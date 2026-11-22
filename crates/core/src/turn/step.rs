//! Sans-IO cognitive loop turn execution, record-first enforcement, and crash recovery.

use std::collections::BTreeSet;

use monoagens_protocol::{
    Action, ActionId, ActorId, Audience, CURRENT_PROTOCOL_VERSION, CapabilityId, CatchupPolicy,
    ChannelId, ConsolidateOutput, DomainId, EventId, ExpectKind, OpaquePayload, OriginActor,
    PlaceId, ReceiptOutcome, Taint, TerminationReason, ThoughtAct, Time,
};

use crate::beliefs::{DEFAULT_DOMAIN_ID, parse_basis_str, verify_claimed_basis};
use crate::consolidation::{
    RecordLessonParams, RegisterProcedureParams, join_episode_taint,
};
use crate::context::{
    AutoRecallTriggerState, ModelTask, PrincipleSeverity, calibrate_confidence_0_20,
    compute_report_threshold,
};
use crate::event::{
    DecisionOutcome, DoorKind, EventDetail, EventKind, EventRecord, ExpectationLifecycleState,
    NumericFeatures,
};
use crate::expectations::{
    DeferredDependentAct, ExpectationRecord, PendingClassifyKind, advance_recurrence_schedule,
    is_fulfilled_state, map_resolve_outcome, max_matching_cosine,
};
use crate::gate::{DecisionReason, DecisionTrace};
use crate::neural::{DEFAULT_H_DIM, FEATURE_SCHEMA_VERSION, GateArm, GateClass};
use crate::ports::{DoorDecision, Doors, ModelPurpose, Store};
use crate::recall::{MemoryBasis, MemoryEntry};
use crate::rng::SeededRng;
use crate::state::{encode_actor_facts_from_msg, encode_body_status, encode_identity_payload};
use crate::{Core, CoreError, Effect, Input};

use super::context_emit::{
    append_and_apply, build_classify_plan, build_task_plan, check_citation_lock,
    dispatch_action_and_open_predictions, execute_memory_recall_act, find_linked_expectation_for_act,
    handle_classify_result, join_min_taint, record_act_refused, record_classify_refusal,
    record_withheld, task_segment_for, wake_reasoning_for_event, wake_reasoning_for_event_with_extra,
    wake_reasoning_for_expectation,
};
use super::sleep_and_beliefs::{
    flush_recalibration_if_pending, maybe_extract_procedure_from_goal,
    maybe_run_sleep_consolidation, record_place_adapt_feedback, record_synaptic_feedback,
};

const DEFAULT_SILENCE_SAMPLE_RATE: f64 = 0.20;
const DEFAULT_PATIENCE_MS: u64 = 2_000;
const DEFAULT_RESPONSE_EXPIRY_QUANTILE: f64 = 0.90;
const DEFAULT_GOAL_EXPIRY_MS: u64 = 86_400_000;
const AUTO_RECALL_MAX_ATTEMPTS: u8 = 2;
const DEFAULT_SPEAKING_BAR: u8 = 3;
const DEFAULT_PROMISE_RETRY_MAX: u32 = 3;
const DEFAULT_PROMISE_RETRY_BASE_MS: u64 = 1_000;


use super::*;

pub(crate) fn execute_turn<S: Store>(
    core: &mut Core<S>,
    input: Input,
    now: Time,
    doors: &mut dyn Doors,
) -> Result<Vec<Effect>, CoreError> {
    match input {
        Input::Percept(mut percept) => {
            percept.validate(CURRENT_PROTOCOL_VERSION)?;
            let origin = percept
                .origin_actor
                .clone()
                .unwrap_or_else(|| OriginActor::new(percept.actor.clone()));
            let nf = NumericFeatures {
                f_modal: percept.f_modal,
                orient: percept.orient.clone(),
                deg_soul: None,
                dialogue_act_gap: percept.dialogue_act_gap,
                delta_obs: percept.delta_obs,
                length: percept.length,
                negative_flag: percept.negative_flag,
                repair: percept.repair,
                continued_attention: percept.continued_attention,
                slots: Vec::new(),
            };
            let mut root = EventRecord::new_root(
                EventKind::Percept,
                percept.timestamp_ms,
                percept.actor.clone(),
                percept.place.clone(),
                percept.audience.clone(),
                origin,
                percept.taint,
                nf,
                percept.payload.clone(),
                percept.attachments.clone(),
                Some(EventDetail::PerceptMeta {
                    addressees: percept.addressees.clone(),
                    named: percept.named.clone(),
                    reply_to: percept.reply_to,
                    correlation_id: percept.correlation_id.clone(),
                    embeddings: percept.embeddings.clone(),
                    is_dyadic_chat: false,
                    answered: 0.0,
                    channel: percept.channel.clone(),
                    peer_expertise_margin: percept.peer_expertise_margin,
                }),
            );
            if let Some(reply_to) = percept.reply_to {
                root.caused_by.push(reply_to);
            }

            // Record-first guarantee: append BEFORE touching state or calling doors.
            let stored = append_and_apply(core, root)?;

            // Step 3: inbound door (only reached after append succeeded).
            let decision = doors.inbound(&mut percept);
            let blocked = match decision {
                DoorDecision::Allow => false,
                DoorDecision::Strip {
                    removed_ids: _,
                    rule,
                } => {
                    let verdict_rec = EventRecord::derived_from(
                        &stored,
                        EventKind::DoorVerdict,
                        now,
                        ActorId::new("core"),
                        stored.place.clone(),
                        stored.audience.clone(),
                        None,
                        NumericFeatures::default(),
                        OpaquePayload::default(),
                        Vec::new(),
                        Some(EventDetail::DoorVerdict {
                            door: DoorKind::Inbound,
                            target_event_id: Some(stored.id),
                            rule,
                            verdict: "strip".to_owned(),
                        }),
                    );
                    append_and_apply(core, verdict_rec)?;
                    false
                }
                DoorDecision::Hold { rule, reason } => {
                    let verdict_rec = EventRecord::derived_from(
                        &stored,
                        EventKind::DoorVerdict,
                        now,
                        ActorId::new("core"),
                        stored.place.clone(),
                        stored.audience.clone(),
                        None,
                        NumericFeatures::default(),
                        OpaquePayload::default(),
                        Vec::new(),
                        Some(EventDetail::DoorVerdict {
                            door: DoorKind::Inbound,
                            target_event_id: Some(stored.id),
                            rule,
                            verdict: format!("hold: {reason}"),
                        }),
                    );
                    append_and_apply(core, verdict_rec)?;
                    true
                }
                DoorDecision::Refuse { rule, reason } => {
                    let verdict_rec = EventRecord::derived_from(
                        &stored,
                        EventKind::DoorVerdict,
                        now,
                        ActorId::new("core"),
                        stored.place.clone(),
                        stored.audience.clone(),
                        None,
                        NumericFeatures::default(),
                        OpaquePayload::default(),
                        Vec::new(),
                        Some(EventDetail::DoorVerdict {
                            door: DoorKind::Inbound,
                            target_event_id: Some(stored.id),
                            rule,
                            verdict: format!("refuse: {reason}"),
                        }),
                    );
                    append_and_apply(core, verdict_rec)?;
                    true
                }
            };

            // Cancel any matching open patience `wait`s if a third party answered first (A-08),
            // and check agency (`self_caused`), response predictions, judged silence, and free-text conditions.
            let mut extra_classify_effects: Vec<Effect> = Vec::new();
            let mut matched_resp_id: Option<EventId> = None;
            let mut content_miss_event_id: Option<EventId> = None;
            if !blocked {
                // Agency check (R-09, R-36): self-caused match suppresses appraisal and reasoning.
                if let Some(eff_id) = core
                    .state()
                    .expectation_book
                    .match_self_caused_effect(&stored, &core.state().soul_actor_id)
                {
                    let eff_kept = EventRecord::derived_from(
                        &stored,
                        EventKind::ExpectationChanged,
                        now,
                        ActorId::new("core"),
                        stored.place.clone(),
                        stored.audience.clone(),
                        None,
                        NumericFeatures::default(),
                        OpaquePayload::default(),
                        Vec::new(),
                        Some(EventDetail::ExpectationTransition {
                            expectation_id: eff_id,
                            action_id: None,
                            h_act: None,
                            from_state: Some(ExpectationLifecycleState::Open),
                            to_state: ExpectationLifecycleState::Kept,
                        }),
                    );
                    append_and_apply(core, eff_kept)?;
                    let self_trace = core
                        .state()
                        .step_self_caused_neural(&stored, core.neural_model());
                    let self_dec = EventRecord::derived_from(
                        &stored,
                        EventKind::Decision,
                        now,
                        ActorId::new("core"),
                        stored.place.clone(),
                        stored.audience.clone(),
                        None,
                        NumericFeatures::default(),
                        OpaquePayload::default(),
                        Vec::new(),
                        Some(EventDetail::Decision {
                            target_event_id: stored.id,
                            outcome: DecisionOutcome::Processed,
                            salience: 0.0,
                            threshold: 0.0,
                            probability: 0.0,
                            draw: 0.0,
                            reason: Some(DecisionReason::SelfCaused),
                            trace: self_trace.map(Box::new),
                        }),
                    );
                    append_and_apply(core, self_dec)?;
                    return Ok(Vec::new());
                }

                let matching_waits: Vec<_> = core
                    .state()
                    .open_waits
                    .values()
                    .filter(|w| w.matches_third_party_reply(&stored, &core.state().soul_actor_id))
                    .cloned()
                    .collect();
                for wait in matching_waits {
                    let cancel_exp = EventRecord::derived_from(
                        &stored,
                        EventKind::ExpectationChanged,
                        now,
                        ActorId::new("core"),
                        wait.place.clone(),
                        wait.audience.clone(),
                        None,
                        NumericFeatures::default(),
                        OpaquePayload::default(),
                        Vec::new(),
                        Some(EventDetail::ExpectationTransition {
                            expectation_id: wait.expectation_id,
                            action_id: None,
                            h_act: None,
                            from_state: Some(ExpectationLifecycleState::Open),
                            to_state: ExpectationLifecycleState::Cancelled,
                        }),
                    );
                    append_and_apply(core, cancel_exp)?;

                    let withheld_rec = EventRecord::derived_from(
                        &stored,
                        EventKind::Withheld,
                        now,
                        ActorId::new("core"),
                        wait.place.clone(),
                        wait.audience.clone(),
                        None,
                        NumericFeatures::default(),
                        OpaquePayload::default(),
                        Vec::new(),
                        Some(EventDetail::Withheld {
                            target_event_id: wait.target_event_id,
                            raw_confidence_0_20: 0,
                            calibrated_confidence: 0.0,
                            report_threshold: 0.0,
                            reason: "yielded".to_owned(),
                        }),
                    );
                    append_and_apply(core, withheld_rec)?;

                    let h_dim = core.neural_model().map_or(DEFAULT_H_DIM, |m| m.config.h_dim);
                    let zeros = vec![0.0_f32; h_dim];
                    let yield_trace = DecisionTrace {
                        neural: crate::neural::neutral_outputs(&zeros),
                        model_blake3: core.neural_model().map(|m| m.blake3),
                        feature_schema_version: FEATURE_SCHEMA_VERSION,
                        online_adaptation_applied: false,
                        commitment_bypass: false,
                        self_caused: false,
                        draw: 0.0,
                        probability: 0.0,
                        outcome: DecisionOutcome::Silent,
                        reason: DecisionReason::Yielded,
                    };
                    let yield_dec = EventRecord::derived_from(
                        &stored,
                        EventKind::Decision,
                        now,
                        ActorId::new("core"),
                        wait.place.clone(),
                        wait.audience.clone(),
                        None,
                        NumericFeatures::default(),
                        OpaquePayload::default(),
                        Vec::new(),
                        Some(EventDetail::Decision {
                            target_event_id: wait.target_event_id,
                            outcome: DecisionOutcome::Silent,
                            salience: 0.0,
                            threshold: 0.0,
                            probability: 0.0,
                            draw: 0.0,
                            reason: Some(DecisionReason::Yielded),
                            trace: Some(Box::new(yield_trace)),
                        }),
                    );
                    append_and_apply(core, yield_dec)?;
                    let wait_syn_feats = core
                        .state()
                        .place_adapt
                        .get(&wait.place)
                        .and_then(|ad| {
                            ad.pending_traces
                                .iter()
                                .rev()
                                .find(|t| t.event_id == wait.target_event_id)
                                .or_else(|| {
                                    ad.pending_traces
                                        .iter()
                                        .rev()
                                        .find(|t| t.arm == GateArm::Wait)
                                })
                                .map(|t| t.features)
                        })
                        .unwrap_or([1.0, -1.0, -1.0, 0.0, 0.0, 0.0, -1.0, -1.0]);
                    record_synaptic_feedback(
                        core,
                        Some(&stored),
                        &wait.place,
                        GateArm::Wait,
                        0.35,
                        false,
                        wait_syn_feats,
                        stored.taint,
                        now,
                    )?;
                    record_synaptic_feedback(
                        core,
                        Some(&stored),
                        &wait.place,
                        GateArm::Go,
                        -0.25,
                        false,
                        wait_syn_feats,
                        stored.taint,
                        now,
                    )?;
                }

                // Link incoming percepts that belong to an active Goal chain (reply_to / caused_by).
                if let Some(goal_id) = core.state().expectation_book.matches_goal_chain(&stored) {
                    let link_ev = EventRecord::derived_from(
                        &stored,
                        EventKind::ExpectationChanged,
                        now,
                        ActorId::new("core"),
                        stored.place.clone(),
                        stored.audience.clone(),
                        None,
                        NumericFeatures::default(),
                        OpaquePayload::default(),
                        Vec::new(),
                        Some(EventDetail::ExpectationLinked {
                            expectation_id: goal_id,
                            linked_event_id: stored.id,
                            action_id: None,
                        }),
                    );
                    append_and_apply(core, link_ev)?;
                }

                // Identify matching open non-silence Response prediction BEFORE appraisal so gate sees commitment.
                matched_resp_id = core
                    .state()
                    .expectation_book
                    .matches_any_response_prediction(
                        &stored,
                        &core.state().soul_actor_id,
                        percept.reply_to,
                    );

                // Check existing judged silences (answered by another participant OR re-ask by sender).
                process_judged_silences_on_percept(
                    core,
                    &stored,
                    &percept,
                    now,
                    &mut extra_classify_effects,
                )?;

                // Check open free-text conditions and unlinked Goal similarity.
                process_free_text_conditions_on_percept(
                    core,
                    &stored,
                    &percept,
                    now,
                    &mut extra_classify_effects,
                )?;
            }

            let mut gate_outcome = None;
            let mut audit_withheld: Option<(u8, f64, f64, &'static str)> = None;
            let suppress_peer_turn_alloc = !blocked
                && core
                    .state()
                    .is_peer_turn_allocation_suppressed(&stored.actor, &stored.place);
            let detail = if blocked {
                audit_withheld = Some((0, 0.0, 0.0, "door_blocked"));
                EventDetail::Decision {
                    target_event_id: stored.id,
                    outcome: DecisionOutcome::DoorBlocked,
                    salience: 0.0,
                    threshold: 0.0,
                    probability: 0.0,
                    draw: 0.0,
                    reason: Some(DecisionReason::DoorBlocked),
                    trace: None,
                }
            } else {
                let gate = core.state().appraise_percept_record(
                    &stored,
                    core.config(),
                    core.neural_model(),
                    0,
                );
                if let Some(reason_tag) = gate.trace.silence_audit_reason() {
                    let raw_conf = (gate.probability * 20.0).round().clamp(0.0, 20.0) as u8;
                    audit_withheld = Some((raw_conf, gate.probability, 0.0, reason_tag));
                }
                let d = EventDetail::Decision {
                    target_event_id: stored.id,
                    outcome: gate.outcome,
                    salience: gate.probability,
                    threshold: 0.0,
                    probability: gate.probability,
                    draw: gate.draw,
                    reason: Some(gate.reason),
                    trace: Some(Box::new(gate.trace.clone())),
                };
                gate_outcome = Some(gate);
                d
            };

            let dec = EventRecord::derived_from(
                &stored,
                EventKind::Decision,
                now,
                ActorId::new("core"),
                stored.place.clone(),
                stored.audience.clone(),
                None,
                NumericFeatures::default(),
                OpaquePayload::default(),
                Vec::new(),
                Some(detail),
            );
            append_and_apply(core, dec)?;

            if let Some((raw_conf, cal_conf, thr, reason_tag)) = audit_withheld {
                let withheld_rec = EventRecord::derived_from(
                    &stored,
                    EventKind::Withheld,
                    now,
                    ActorId::new("core"),
                    stored.place.clone(),
                    stored.audience.clone(),
                    None,
                    NumericFeatures::default(),
                    OpaquePayload::default(),
                    Vec::new(),
                    Some(EventDetail::Withheld {
                        target_event_id: stored.id,
                        raw_confidence_0_20: raw_conf,
                        calibrated_confidence: cal_conf,
                        report_threshold: thr,
                        reason: reason_tag.to_owned(),
                    }),
                );
                append_and_apply(core, withheld_rec)?;
            }

            // Now finalize any matched Response prediction (evaluating R-18 content miss).
            if let Some(resp_id) = matched_resp_id {
                let miss_opt = core.state_mut().expectation_book.evaluate_response_content(
                    resp_id,
                    &stored,
                    &percept.embeddings,
                );
                if let Some(miss) = miss_opt {
                    let miss_rec = EventRecord::derived_from(
                        &stored,
                        EventKind::ExpectationChanged,
                        now,
                        ActorId::new("core"),
                        stored.place.clone(),
                        stored.audience.clone(),
                        None,
                        NumericFeatures::default(),
                        OpaquePayload::default(),
                        Vec::new(),
                        Some(EventDetail::ContentMiss {
                            expectation_id: resp_id,
                            reply_event_id: stored.id,
                            who: miss.who,
                            timing_delta_ms: miss.timing_delta_ms,
                            distance: miss.distance,
                            usual_distance: miss.usual_distance,
                            z_score: miss.z_score,
                        }),
                    );
                    let stored_miss = append_and_apply(core, miss_rec)?;
                    content_miss_event_id = Some(stored_miss.id);
                    let fail_rec = EventRecord::derived_from(
                        &stored,
                        EventKind::ExpectationChanged,
                        now,
                        ActorId::new("core"),
                        stored.place.clone(),
                        stored.audience.clone(),
                        None,
                        NumericFeatures::default(),
                        OpaquePayload::default(),
                        Vec::new(),
                        Some(EventDetail::ExpectationTransition {
                            expectation_id: resp_id,
                            action_id: None,
                            h_act: None,
                            from_state: Some(ExpectationLifecycleState::Open),
                            to_state: ExpectationLifecycleState::Failed,
                        }),
                    );
                    append_and_apply(core, fail_rec)?;
                } else {
                    let ans_rec = EventRecord::derived_from(
                        &stored,
                        EventKind::ExpectationChanged,
                        now,
                        ActorId::new("core"),
                        stored.place.clone(),
                        stored.audience.clone(),
                        None,
                        NumericFeatures::default(),
                        OpaquePayload::default(),
                        Vec::new(),
                        Some(EventDetail::ExpectationTransition {
                            expectation_id: resp_id,
                            action_id: None,
                            h_act: None,
                            from_state: Some(ExpectationLifecycleState::Open),
                            to_state: ExpectationLifecycleState::Answered,
                        }),
                    );
                    append_and_apply(core, ans_rec)?;
                }
            }

            if let Some(gate) = gate_outcome {
                // V2.2 p_user_correction Loop B: emit event-sourced propensity-weighted delayed
                // negative AdaptFeedbackApplied on GateArm::Go when calibrated p_user_correction
                // >= 0.65 follows a recent visible soul act, strictly subject to Taint & online_adaptation guards.
                if let Some(p_corr) = gate.neural.p_user_correction
                    && p_corr.is_finite()
                    && p_corr >= crate::neural::USER_CORRECTION_REPAIR_THRESHOLD
                    && stored.actor != core.state().soul_actor_id
                    && core
                        .state()
                        .decision_v22_hints
                        .get(&stored.id)
                        .is_some_and(|h| !h.repair_context_ids.is_empty())
                {
                    let p_go = gate.neural.gate_probs[3].max(0.15);
                    let reward = -0.35 * (p_corr / p_go).clamp(0.0, 1.0);
                    let go_feats = core
                        .state()
                        .place_adapt
                        .get(&stored.place)
                        .and_then(|ad| {
                            ad.pending_traces
                                .iter()
                                .rev()
                                .find(|t| t.arm == GateArm::Go)
                                .map(|t| t.features)
                        })
                        .unwrap_or_else(|| {
                            let x_v3 = crate::neural::extract_features_v3_from_state(
                                core.state(),
                                &percept,
                                now,
                                0,
                            );
                            crate::neural::synaptic_features_from_x_num(&x_v3)
                        });
                    record_place_adapt_feedback(
                        core,
                        Some(&stored),
                        &stored.place,
                        GateArm::Go,
                        reward,
                        true,
                        stored.taint,
                        now,
                    )?;
                    record_synaptic_feedback(
                        core,
                        Some(&stored),
                        &stored.place,
                        GateArm::Go,
                        reward,
                        true,
                        go_feats,
                        stored.taint,
                        now,
                    )?;
                    record_synaptic_feedback(
                        core,
                        Some(&stored),
                        &stored.place,
                        GateArm::Wait,
                        0.20 * (p_corr / p_go).clamp(0.0, 1.0),
                        true,
                        go_feats,
                        stored.taint,
                        now,
                    )?;
                } else if let Some(p_corr) = gate.neural.p_user_correction
                    && p_corr.is_finite()
                    && p_corr < 0.35
                    && stored.actor != core.state().soul_actor_id
                    && percept.reply_to.is_some()
                    && let Some(go_trace) = core.state().place_adapt.get(&stored.place).and_then(|ad| {
                        ad.pending_traces
                            .iter()
                            .rev()
                            .find(|t| t.arm == GateArm::Go)
                            .cloned()
                    })
                    && core
                        .state()
                        .event_authors
                        .get(percept.reply_to.as_ref().expect("is_some"))
                        .is_some_and(|a| a == &core.state().soul_actor_id)
                {
                    record_synaptic_feedback(
                        core,
                        Some(&stored),
                        &stored.place,
                        GateArm::Go,
                        0.25,
                        true,
                        go_trace.features,
                        stored.taint,
                        now,
                    )?;
                }

                if matches!(
                    gate.outcome,
                    DecisionOutcome::Note | DecisionOutcome::Silent
                ) && gate.reason != DecisionReason::Yielded
                    && gate.reason != DecisionReason::DoorBlocked
                    && gate.gate_class != GateClass::Wait
                    && stored.actor != core.state().soul_actor_id
                {
                    let propensity = if gate.outcome == DecisionOutcome::Note {
                        Some(1.0)
                    } else {
                        let rho = DEFAULT_SILENCE_SAMPLE_RATE;
                        let draw = SeededRng::draw_unit(
                            core.seed() ^ 0x517C_E001,
                            core.state().draw_key(stored.id),
                        );
                        if rho > 0.0 && draw < rho {
                            Some(rho)
                        } else {
                            None
                        }
                    };
                    if let Some(prop) = propensity {
                        let medium = percept
                            .channel
                            .clone()
                            .unwrap_or_else(|| ChannelId::new(percept.actor.as_str()));
                        let silence_window_ms = core
                            .state()
                            .expectation_book
                            .response_latency
                            .estimate_window_ms(
                                &stored.place,
                                &medium,
                                DEFAULT_PATIENCE_MS,
                                DEFAULT_RESPONSE_EXPIRY_QUANTILE,
                            );
                        let silence_open = EventRecord::derived_from(
                            &stored,
                            EventKind::ExpectationOpened,
                            now,
                            ActorId::new("core"),
                            stored.place.clone(),
                            stored.audience.clone(),
                            None,
                            NumericFeatures::default(),
                            OpaquePayload::default(),
                            Vec::new(),
                            Some(EventDetail::ExpectationRecordOpened {
                                expectation_id: stored.id,
                                origin_event_id: stored.id,
                                target_event_id: stored.id,
                                kind: ExpectKind::Response,
                                places: vec![stored.place.clone()],
                                people: vec![stored.actor.clone()],
                                domain: None,
                                condition: "judged_silence".to_owned(),
                                condition_embeddings: percept.embeddings.clone(),
                                due_ms: now.saturating_add_ms(silence_window_ms),
                                expiry_ms: now.saturating_add_ms(silence_window_ms),
                                recurrence: None,
                                promise_meta: None,
                                urgency_weight: 0.0,
                                action_id: None,
                                capability_id: None,
                                channel: percept.channel.clone(),
                                lat_min_ms: 0,
                                lat_max_ms: 0,
                                delta_pred: None,
                                sigma_channel: None,
                                correlation_id: None,
                                declared_sensor: false,
                                act_embeddings: Vec::new(),
                                expected_reply_embeddings: Vec::new(),
                                is_judged_silence: true,
                                silence_propensity: Some(prop),
                                silence_sender: Some(stored.actor.clone()),
                                stakes: 0.0,
                                medium: Some(medium),
                                about_ids: vec![stored.id],
                                accept_done_receipt: false,
                                expected_latency_ms: silence_window_ms,
                            }),
                        );
                        append_and_apply(core, silence_open)?;
                    }
                }

                if gate.gate_class == GateClass::Wait && gate.wait_delta_ms > 0 {
                    let medium = percept
                        .channel
                        .clone()
                        .unwrap_or_else(|| ChannelId::new(percept.actor.as_str()));
                    let soul_id = &core.state().soul_actor_id;
                    let non_soul_audience = percept
                        .audience
                        .members
                        .iter()
                        .filter(|a| *a != soul_id)
                        .count();
                    let room_has_automated_peers = percept
                        .audience
                        .members
                        .iter()
                        .filter(|a| *a != soul_id)
                        .any(|a| {
                            core.state()
                                .actor_facts
                                .get(a)
                                .is_some_and(|f| f.automated_peer)
                        })
                        || core
                            .state()
                            .place_participants
                            .get(&stored.place)
                            .is_some_and(|set| {
                                set.iter().filter(|a| *a != soul_id).any(|a| {
                                    core.state()
                                        .actor_facts
                                        .get(a)
                                        .is_some_and(|f| f.automated_peer)
                                })
                            });
                    let jitter_ms = crate::gate::compute_open_wait_jitter_ms(
                        core.seed(),
                        core.state().draw_key(stored.id),
                        non_soul_audience,
                        room_has_automated_peers,
                        percept.peer_expertise_margin,
                    );
                    let due_ms = now.saturating_add_ms(gate.wait_delta_ms.saturating_add(jitter_ms));
                    let wait_rec = EventRecord::derived_from(
                        &stored,
                        EventKind::ExpectationOpened,
                        now,
                        ActorId::new("core"),
                        stored.place.clone(),
                        stored.audience.clone(),
                        None,
                        NumericFeatures::default(),
                        OpaquePayload::default(),
                        Vec::new(),
                        Some(EventDetail::ExpectationOpened {
                            expectation_id: stored.id,
                            target_event_id: stored.id,
                            kind: ExpectKind::Wait,
                            place: stored.place.clone(),
                            sender: stored.actor.clone(),
                            medium,
                            due_ms,
                            expiry_ms: due_ms,
                        }),
                    );
                    append_and_apply(core, wait_rec)?;
                    return Ok(extra_classify_effects);
                }

                if gate.outcome == DecisionOutcome::Go {
                    let deg_soul = percept
                        .delta_obs
                        .map(|_| 0.0_f32)
                        .or_else(|| {
                            percept
                                .orient
                                .iter()
                                .find(|o| o.actor == core.state().soul_actor_id)
                                .map(|o| o.score)
                        })
                        .unwrap_or(0.0);
                    let turn_allocated = (percept.addressees.contains(&core.state().soul_actor_id)
                        || f64::from(deg_soul) >= crate::state::DEFAULT_DEG_ALLOC_THRESHOLD)
                        && !suppress_peer_turn_alloc;
                    core.state_mut()
                        .turn_allocated_by_trigger
                        .insert(stored.id, turn_allocated);

                    let mut wake_effects = wake_reasoning_for_event_with_extra(
                        core,
                        &stored,
                        content_miss_event_id.as_slice(),
                        now,
                        doors,
                    )?;
                    wake_effects.extend(extra_classify_effects);
                    return Ok(wake_effects);
                }
            }

            Ok(extra_classify_effects)
        }

        Input::Receipt(receipt) => {
            receipt.validate()?;
            if core
                .state()
                .terminal_receipts
                .contains_key(&receipt.action_id)
            {
                return Err(CoreError::DuplicateReceipt(receipt.action_id));
            }
            let (place, audience, origin, taint, cause_id) =
                match core.state().in_flight_actions.get(&receipt.action_id) {
                    Some(in_flight) => {
                        let parent = core.store().get(in_flight.event_id)?;
                        match parent {
                            Some(p) => (
                                p.place.clone(),
                                p.audience.clone(),
                                p.origin_actor.clone(),
                                p.taint,
                                Some(p.id),
                            ),
                            None => (
                                in_flight
                                    .places
                                    .first()
                                    .cloned()
                                    .unwrap_or_else(|| PlaceId::new("unknown")),
                                Audience::new([ActorId::new("core")]),
                                OriginActor::new(ActorId::new("core")),
                                Taint::Trusted,
                                None,
                            ),
                        }
                    }
                    None => (
                        PlaceId::new("unknown"),
                        Audience::new([ActorId::new("core")]),
                        OriginActor::new(ActorId::new("core")),
                        Taint::Provisional,
                        None,
                    ),
                };

            let mut rec = EventRecord::new_root(
                EventKind::Receipt,
                receipt.timestamp_ms,
                ActorId::new("body"),
                place.clone(),
                audience.clone(),
                origin,
                taint,
                NumericFeatures::default(),
                OpaquePayload::default(),
                Vec::new(),
                Some(EventDetail::Receipt {
                    action_id: receipt.action_id.clone(),
                    capability_id: receipt.capability_id.clone(),
                    outcome: receipt.outcome,
                    termination_reason: receipt.termination_reason,
                    progress: receipt.progress,
                    delivered_units: receipt.delivered_units,
                    h_act: receipt.h_act.clone(),
                }),
            );
            if let Some(cid) = cause_id {
                rec.caused_by.push(cid);
            }
            let stored = append_and_apply(core, rec)?;

            let verdict = EventRecord::derived_from(
                &stored,
                EventKind::Verdict,
                now,
                ActorId::new("core"),
                place,
                audience,
                None,
                NumericFeatures::default(),
                OpaquePayload::default(),
                Vec::new(),
                Some(EventDetail::Verdict {
                    target_event_id: stored.id,
                    label: "receipt_processed".to_owned(),
                }),
            );
            append_and_apply(core, verdict)?;
            if core.state().recalibration_pending {
                flush_recalibration_if_pending(
                    core,
                    &stored,
                    "capability_receipt_change",
                    now,
                    doors,
                )?;
            }
            match receipt.outcome {
                ReceiptOutcome::Done | ReceiptOutcome::Held | ReceiptOutcome::Unknown => {}
                ReceiptOutcome::Failed | ReceiptOutcome::Refused => {
                    let non_fault = matches!(
                        receipt.termination_reason,
                        Some(
                            TerminationReason::PreemptedExternal
                                | TerminationReason::CanceledByCore
                        )
                    );
                    if !non_fault {
                        let place_c = stored.place.clone();
                        record_place_adapt_feedback(
                            core,
                            Some(&stored),
                            &place_c,
                            GateArm::Go,
                            -0.5,
                            false,
                            Taint::Trusted,
                            now,
                        )?;
                    }
                }
            }
            handle_receipt_expectations(core, &stored, &receipt, now, doors)
        }

        Input::CancelAction {
            action_id,
            termination_reason,
            reason_code,
            detail,
        } => {
            // Single-receipt invariant (R-36): if a terminal receipt already exists, emit nothing more.
            if core.state().terminal_receipts.contains_key(&action_id) {
                return Ok(Vec::new());
            }
            let norm_reason = match termination_reason {
                TerminationReason::CanceledByCore | TerminationReason::PreemptedExternal => {
                    termination_reason
                }
                _ => TerminationReason::CanceledByCore,
            };
            let (capability_id, place, audience, origin, taint, cause_id) =
                if let Some(in_flight) = core.state().in_flight_actions.get(&action_id) {
                    let parent = core.store().get(in_flight.event_id)?;
                    match parent {
                        Some(p) => (
                            in_flight.capability_id.clone(),
                            p.place.clone(),
                            p.audience.clone(),
                            p.origin_actor.clone(),
                            p.taint,
                            Some(p.id),
                        ),
                        None => (
                            in_flight.capability_id.clone(),
                            in_flight
                                .places
                                .first()
                                .cloned()
                                .unwrap_or_else(|| PlaceId::new("unknown")),
                            Audience::new([ActorId::new("core")]),
                            OriginActor::new(ActorId::new("core")),
                            Taint::Trusted,
                            None,
                        ),
                    }
                } else if let Some(pa) = core.state().pending_approvals.get(&action_id) {
                    (
                        pa.capability_id.clone(),
                        PlaceId::new("unknown"),
                        Audience::new([ActorId::new("core")]),
                        OriginActor::new(ActorId::new("core")),
                        Taint::Trusted,
                        None,
                    )
                } else {
                    // Action is not in-flight or held; emit at most zero/no-op receipts.
                    return Ok(Vec::new());
                };

            let mut rec = EventRecord::new_root(
                EventKind::Receipt,
                now,
                ActorId::new("core"),
                place.clone(),
                audience.clone(),
                origin,
                taint,
                NumericFeatures::default(),
                OpaquePayload::from_bytes(
                    detail
                        .as_deref()
                        .or(reason_code.as_deref())
                        .unwrap_or("cancelled")
                        .as_bytes(),
                ),
                Vec::new(),
                Some(EventDetail::Receipt {
                    action_id: action_id.clone(),
                    capability_id: capability_id.clone(),
                    outcome: ReceiptOutcome::Failed,
                    termination_reason: Some(norm_reason),
                    progress: None,
                    delivered_units: None,
                    h_act: None,
                }),
            );
            if let Some(cid) = cause_id {
                rec.caused_by.push(cid);
            }
            let stored = append_and_apply(core, rec)?;

            let verdict = EventRecord::derived_from(
                &stored,
                EventKind::Verdict,
                now,
                ActorId::new("core"),
                place,
                audience,
                None,
                NumericFeatures::default(),
                OpaquePayload::default(),
                Vec::new(),
                Some(EventDetail::Verdict {
                    target_event_id: stored.id,
                    label: "preempted_receipt_processed".to_owned(),
                }),
            );
            append_and_apply(core, verdict)?;
            let synth_receipt = monoagens_protocol::Receipt {
                action_id,
                capability_id,
                outcome: ReceiptOutcome::Failed,
                reason_code,
                detail,
                termination_reason: Some(norm_reason),
                progress: None,
                delivered_units: None,
                h_act: None,
                timestamp_ms: now,
            };
            handle_receipt_expectations(core, &stored, &synth_receipt, now, doors)
        }

        Input::ModelResult { request_id, acts } => {
            for act in &acts {
                act.validate()?;
            }
            let causing = core.store().get(request_id)?;
            let wake_snapshot = core.state().active_wake_snapshots.get(&request_id).cloned();
            let turn_allocated = wake_snapshot
                .as_ref()
                .map(|w| w.turn_allocated)
                .unwrap_or_else(|| {
                    causing
                        .as_ref()
                        .is_none_or(|c| core.state().lookup_turn_allocated(c))
                });

            // Monotonic `min(taint)` across causing event and all cited `used_ids` (7.2).
            let mut turn_taint = causing.as_ref().map_or(Taint::Untrusted, |c| c.taint);
            for act in &acts {
                let ids: &[EventId] = match act {
                    ThoughtAct::Think { used_ids, .. } | ThoughtAct::Act { used_ids, .. } => {
                        used_ids
                    }
                    ThoughtAct::Expect { about_ids, .. } => about_ids,
                    _ => &[],
                };
                for &uid in ids {
                    if let Some(m) = core.state().memory_entries.get(&uid) {
                        turn_taint = join_min_taint(turn_taint, m.taint);
                    } else if let Some(rec) = core.store().get(uid)? {
                        turn_taint = join_min_taint(turn_taint, rec.taint);
                    }
                }
            }

            let mut thought_rec = match &causing {
                Some(cause) => EventRecord::derived_from(
                    cause,
                    EventKind::Thought,
                    now,
                    ActorId::new("soul"),
                    cause.place.clone(),
                    cause.audience.clone(),
                    None,
                    NumericFeatures::default(),
                    OpaquePayload::default(),
                    Vec::new(),
                    None,
                ),
                None => {
                    let mut r = EventRecord::new_root(
                        EventKind::Thought,
                        now,
                        ActorId::new("soul"),
                        PlaceId::new("unknown"),
                        Audience::new([ActorId::new("soul")]),
                        OriginActor::new(core.state().soul_actor_id.clone()),
                        Taint::Untrusted,
                        NumericFeatures::default(),
                        OpaquePayload::default(),
                        Vec::new(),
                        None,
                    );
                    if request_id.as_u64() != 0 {
                        r.caused_by.push(request_id);
                    }
                    r
                }
            };
            // Preserve `origin_actor` from causing chain (unforgeable), propagate monotonic `min(taint)`.
            thought_rec.taint = turn_taint;
            if let Some(thought_text) = acts.iter().find_map(|a| match a {
                ThoughtAct::Think { thought, .. } => Some(thought),
                ThoughtAct::Consolidate {
                    output: ConsolidateOutput::Gist { summary, .. },
                } => Some(summary),
                _ => None,
            }) {
                thought_rec.payload = OpaquePayload::from_bytes(thought_text.as_bytes());
            }
            let stored_thought = append_and_apply(core, thought_rec)?;
            core.state_mut()
                .turn_allocated_by_trigger
                .insert(stored_thought.id, turn_allocated);

            let mut turn_created_ids: BTreeSet<EventId> = BTreeSet::new();
            turn_created_ids.insert(stored_thought.id);
            let mut last_think_stakes: Option<(Option<f32>, Option<f32>, Option<DomainId>)> = None;

            let mut effects = Vec::new();
            for act in acts {
                match act {
                    ThoughtAct::Think {
                        thought,
                        importance,
                        used_ids,
                        basis,
                        value,
                        cost,
                        domain,
                        ..
                    } => {
                        if let Some(bad_id) = check_citation_lock(
                            core,
                            wake_snapshot.as_ref(),
                            &turn_created_ids,
                            &used_ids,
                        )? {
                            let dv = EventRecord::derived_from(
                                &stored_thought,
                                EventKind::DoorVerdict,
                                now,
                                ActorId::new("core"),
                                stored_thought.place.clone(),
                                stored_thought.audience.clone(),
                                None,
                                NumericFeatures::default(),
                                OpaquePayload::default(),
                                Vec::new(),
                                Some(EventDetail::DoorVerdict {
                                    door: DoorKind::Action,
                                    target_event_id: Some(stored_thought.id),
                                    rule: "citation_lock".to_owned(),
                                    verdict: format!(
                                        "refuse: ungrounded citation {}",
                                        bad_id.as_u64()
                                    ),
                                }),
                            );
                            append_and_apply(core, dv)?;
                            record_withheld(
                                core,
                                &stored_thought,
                                0,
                                0.0,
                                0.0,
                                "citation_lock",
                                now,
                            )?;
                            break;
                        }

                        if let Some(imp) = importance {
                            let imp_val = f64::from(imp).clamp(0.0, 1.0);
                            for &uid in &used_ids {
                                if let Some(entry) = core.state_mut().memory_entries.get_mut(&uid) {
                                    entry.importance.i_reason =
                                        entry.importance.i_reason.max(imp_val);
                                }
                            }
                        }

                        let requested_basis = basis
                            .as_deref()
                            .and_then(parse_basis_str)
                            .unwrap_or(MemoryBasis::Inferred);
                        let (verified_basis, basis_downgraded) = verify_claimed_basis(
                            requested_basis,
                            &used_ids,
                            &core.state().memory_entries,
                            &core.state().soul_actor_id,
                            false,
                        );
                        if basis.is_some() {
                            let resolved_domain = domain
                                .clone()
                                .unwrap_or_else(|| DomainId::new(DEFAULT_DOMAIN_ID));
                            let mut belief_rec = EventRecord::derived_from(
                                &stored_thought,
                                EventKind::BeliefChanged,
                                now,
                                ActorId::new("core"),
                                stored_thought.place.clone(),
                                stored_thought.audience.clone(),
                                None,
                                NumericFeatures::default(),
                                OpaquePayload::default(),
                                Vec::new(),
                                Some(EventDetail::BeliefChanged {
                                    belief_id: stored_thought.id,
                                    claim_key: format!("thought:{}", stored_thought.id.as_u64()),
                                    requested_basis,
                                    basis: MemoryBasis::Inferred,
                                    basis_downgraded,
                                    source_actor: if verified_basis == MemoryBasis::Told {
                                        Some(stored_thought.origin_actor.actor_id().clone())
                                    } else {
                                        None
                                    },
                                    domain: resolved_domain,
                                    source_event_ids: used_ids.clone(),
                                    taint: stored_thought.taint,
                                    valid_from_ms: now,
                                    valid_to_ms: None,
                                    replaced_by: None,
                                    alpha_0: 1.0,
                                    beta_0: 1.0,
                                    is_mind_inference: false,
                                }),
                            );
                            belief_rec.caused_by.extend(used_ids.iter().copied());
                            append_and_apply(core, belief_rec)?;
                        }

                        // Index soul's thought into recallable memory with verified basis & monotonic `turn_taint` (7.2, 7.4, 10.1).
                        let mut trace = crate::activation::ActivationTrace::default();
                        trace.record(now);
                        let mut concepts = Vec::new();
                        for tok in thought
                            .split(|c: char| !c.is_alphanumeric() && c != '_' && c != '-')
                            .filter(|s| s.len() >= 2)
                        {
                            if !concepts.iter().any(|c: &String| c == tok) {
                                concepts.push(tok.to_owned());
                            }
                        }
                        let mut imp_inputs = crate::recall::ImportanceInputs::default();
                        if let Some(imp) = importance {
                            imp_inputs.i_reason = f64::from(imp).clamp(0.0, 1.0);
                        }
                        let soul_actor_id = core.state().soul_actor_id.clone();
                        core.state_mut().upsert_memory_entry(MemoryEntry {
                            event_id: stored_thought.id,
                            source_ids: if used_ids.is_empty() {
                                vec![stored_thought.id]
                            } else {
                                used_ids
                            },
                            is_gist: false,
                            timestamp_ms: now,
                            place: stored_thought.place.clone(),
                            actor: soul_actor_id,
                            origin_actor: stored_thought.origin_actor.clone(),
                            basis: MemoryBasis::Inferred,
                            taint: stored_thought.taint,
                            audience: stored_thought.audience.clone(),
                            attachments: Vec::new(),
                            activation: trace,
                            importance: imp_inputs,
                            embeddings: Vec::new(),
                            concepts,
                            superseded_by: None,
                            erased: false,
                        });
                        last_think_stakes = Some((value, cost, domain));
                    }
                    ThoughtAct::Expect {
                        expect_kind,
                        about_ids,
                        places,
                        people,
                        condition,
                        due_ms,
                        expiry_ms,
                        recurrence,
                        promise_meta,
                        domain,
                    } => {
                        for &uid in &about_ids {
                            if let Some(entry) = core.state_mut().memory_entries.get_mut(&uid) {
                                entry.importance.linked_to_commitment = true;
                            }
                        }

                        if expect_kind == ExpectKind::Promise
                            && let Some(conflict) = core.state().expectation_book.check_conflict(
                                &places,
                                promise_meta.as_ref(),
                                None,
                            )
                        {
                            let conflict_rec = EventRecord::derived_from(
                                &stored_thought,
                                EventKind::ExpectationChanged,
                                now,
                                ActorId::new("core"),
                                places
                                    .first()
                                    .cloned()
                                    .unwrap_or_else(|| stored_thought.place.clone()),
                                stored_thought.audience.clone(),
                                None,
                                NumericFeatures::default(),
                                OpaquePayload::default(),
                                Vec::new(),
                                Some(EventDetail::ConflictFlagged {
                                    proposed_expectation_id: None,
                                    conflicting_expectation_id: conflict.conflicting_expectation_id,
                                    conflict_key: conflict.conflict_key,
                                    overlapping_places: conflict.overlapping_places,
                                }),
                            );
                            let stored_conflict = append_and_apply(core, conflict_rec)?;
                            let mut wake_effects =
                                wake_reasoning_for_event(core, &stored_conflict, now, doors)?;
                            effects.append(&mut wake_effects);
                            continue;
                        }

                        let mut cond_embs = Vec::new();
                        for &aid in &about_ids {
                            if let Some(entry) = core.state().memory_entries.get(&aid) {
                                cond_embs.extend(entry.embeddings.clone());
                            }
                        }

                        let effective_due = due_ms.unwrap_or(expiry_ms);
                        let effective_expiry =
                            if expect_kind == ExpectKind::Goal && expiry_ms <= now {
                                now.saturating_add_ms(DEFAULT_GOAL_EXPIRY_MS)
                            } else {
                                expiry_ms
                            };
                        let urgency = match expect_kind {
                            ExpectKind::Monitor => 0.0,
                            ExpectKind::Promise | ExpectKind::Goal => 1.0,
                            _ => 0.5,
                        };
                        let mut open_rec = EventRecord::derived_from(
                            &stored_thought,
                            EventKind::ExpectationOpened,
                            now,
                            ActorId::new("core"),
                            places
                                .first()
                                .cloned()
                                .unwrap_or_else(|| stored_thought.place.clone()),
                            stored_thought.audience.clone(),
                            None,
                            NumericFeatures::default(),
                            OpaquePayload::default(),
                            Vec::new(),
                            None,
                        );
                        let next_exp_id =
                            EventId::new(core.state().last_seq.as_u64().saturating_add(1).max(1));
                        turn_created_ids.insert(next_exp_id);
                        open_rec.detail = Some(EventDetail::ExpectationRecordOpened {
                            expectation_id: next_exp_id,
                            origin_event_id: request_id,
                            target_event_id: stored_thought.id,
                            kind: expect_kind,
                            places,
                            people,
                            domain,
                            condition,
                            condition_embeddings: cond_embs,
                            due_ms: effective_due,
                            expiry_ms: effective_expiry,
                            recurrence,
                            promise_meta,
                            urgency_weight: urgency,
                            action_id: None,
                            capability_id: None,
                            channel: None,
                            lat_min_ms: 0,
                            lat_max_ms: 0,
                            delta_pred: None,
                            sigma_channel: None,
                            correlation_id: None,
                            declared_sensor: false,
                            act_embeddings: Vec::new(),
                            expected_reply_embeddings: Vec::new(),
                            is_judged_silence: false,
                            silence_propensity: None,
                            silence_sender: None,
                            stakes: 1.0,
                            medium: None,
                            about_ids,
                            accept_done_receipt: false,
                            expected_latency_ms: 0,
                        });
                        append_and_apply(core, open_rec)?;
                    }
                    ThoughtAct::Revise {
                        target_id,
                        new_version,
                        condition,
                        due_ms,
                        expiry_ms,
                        places,
                        people,
                        domain,
                    } => {
                        let is_open_expectation = core
                            .state()
                            .expectation_book
                            .expectations
                            .contains_key(&target_id);
                        let immutable_target = !is_open_expectation
                            && core.store().get(target_id)?.is_some_and(|rec| {
                                matches!(
                                    rec.kind,
                                    EventKind::IdentityChanged
                                        | EventKind::Percept
                                        | EventKind::Action
                                        | EventKind::Receipt
                                        | EventKind::Decision
                                        | EventKind::DoorVerdict
                                        | EventKind::Verdict
                                        | EventKind::MemoryUsed
                                        | EventKind::Withheld
                                )
                            });
                        if immutable_target {
                            let dv = EventRecord::derived_from(
                                &stored_thought,
                                EventKind::DoorVerdict,
                                now,
                                ActorId::new("core"),
                                stored_thought.place.clone(),
                                stored_thought.audience.clone(),
                                None,
                                NumericFeatures::default(),
                                OpaquePayload::default(),
                                Vec::new(),
                                Some(EventDetail::DoorVerdict {
                                    door: DoorKind::Action,
                                    target_event_id: Some(stored_thought.id),
                                    rule: "immutable_revise".to_owned(),
                                    verdict: format!(
                                        "refuse: immutable_target {}",
                                        target_id.as_u64()
                                    ),
                                }),
                            );
                            append_and_apply(core, dv)?;
                            record_withheld(
                                core,
                                &stored_thought,
                                0,
                                0.0,
                                0.0,
                                "immutable_revise",
                                now,
                            )?;
                            continue;
                        }

                        let new_id = stored_thought.id;
                        core.state_mut().supersede_memory(target_id, new_id);
                        if is_open_expectation {
                            let rev_rec = EventRecord::derived_from(
                                &stored_thought,
                                EventKind::ExpectationChanged,
                                now,
                                ActorId::new("core"),
                                places
                                    .first()
                                    .cloned()
                                    .unwrap_or_else(|| stored_thought.place.clone()),
                                stored_thought.audience.clone(),
                                None,
                                NumericFeatures::default(),
                                OpaquePayload::default(),
                                Vec::new(),
                                Some(EventDetail::ExpectationRevised {
                                    expectation_id: target_id,
                                    wake_event_id: stored_thought.id,
                                    new_version,
                                    condition,
                                    due_ms,
                                    expiry_ms,
                                    places,
                                    people,
                                    domain,
                                }),
                            );
                            append_and_apply(core, rev_rec)?;
                        }
                    }
                    ThoughtAct::Consolidate { output } => match output {
                        ConsolidateOutput::Gist {
                            about_ids,
                            places,
                            people,
                            ..
                        } => {
                            let place = places
                                .into_iter()
                                .next()
                                .unwrap_or_else(|| stored_thought.place.clone());

                            // Monotonic `min(trust)` across all linked source events (`13.2`).
                            let mut source_taints: Vec<Taint> = vec![stored_thought.taint];
                            let mut source_audiences: Vec<Audience> = Vec::new();
                            for &sid in &about_ids {
                                if let Some(entry) = core.state().memory_entries.get(&sid) {
                                    source_taints.push(entry.taint);
                                    source_audiences.push(entry.audience.clone());
                                } else if let Some(rec) = core.store().get(sid)? {
                                    source_taints.push(rec.taint);
                                    source_audiences.push(rec.audience);
                                }
                            }
                            let min_taint = join_episode_taint(source_taints.iter());

                            // Match pending sleep episode (preventing duplicate gists on resumed runs, `12.1`).
                            let matched_ep_id = {
                                let about_set: BTreeSet<EventId> =
                                    about_ids.iter().copied().collect();
                                core.state()
                                    .episode_book
                                    .episodes
                                    .values()
                                    .find(|ep| ep.event_ids.iter().any(|id| about_set.contains(id)))
                                    .map(|ep| ep.episode_id)
                                    .or_else(|| {
                                        core.state()
                                            .pending_sleep_run
                                            .as_ref()
                                            .and_then(|r| r.pending_episode_ids.first().copied())
                                    })
                            };

                            if let (Some(ep_id), Some(run)) =
                                (matched_ep_id, core.state().pending_sleep_run.as_ref())
                                && run.completed_episode_ids.contains(&ep_id)
                            {
                                // Already consolidated before interruption; skip duplicate gist.
                                continue;
                            }

                            let gist_embeddings: Vec<monoagens_protocol::TaggedEmbedding> =
                                matched_ep_id
                                    .and_then(|ep_id| {
                                        core.state().episode_book.episodes.get(&ep_id)
                                    })
                                    .and_then(|ep| ep.centroid.clone())
                                    .into_iter()
                                    .collect();
                            let mut trace = crate::activation::ActivationTrace::default();
                            trace.record(now);
                            let entry = MemoryEntry {
                                event_id: stored_thought.id,
                                source_ids: about_ids.clone(),
                                is_gist: true,
                                timestamp_ms: now,
                                place: place.clone(),
                                actor: ActorId::new("soul"),
                                origin_actor: stored_thought.origin_actor.clone(),
                                basis: MemoryBasis::Inferred,
                                taint: min_taint,
                                audience: stored_thought.audience.clone(),
                                attachments: Vec::new(),
                                activation: trace,
                                importance: Default::default(),
                                embeddings: gist_embeddings,
                                concepts: people.into_iter().map(|p| p.into_inner()).collect(),
                                superseded_by: None,
                                erased: false,
                            };
                            core.state_mut().upsert_memory_entry(entry);

                            if let Some(ep_id) = matched_ep_id {
                                let (prev_gist, ep_state_label, ep_events) = if let Some(ep) =
                                    core.state().episode_book.episodes.get(&ep_id)
                                {
                                    (
                                        ep.gist_id,
                                        ep.state.as_str().to_owned(),
                                        ep.event_ids.clone(),
                                    )
                                } else {
                                    (None, "sealed".to_owned(), about_ids.clone())
                                };
                                let ep_closed = EventRecord::derived_from(
                                    &stored_thought,
                                    EventKind::EpisodeClosed,
                                    now,
                                    ActorId::new("core"),
                                    place.clone(),
                                    stored_thought.audience.clone(),
                                    Some(min_taint),
                                    NumericFeatures::default(),
                                    OpaquePayload::default(),
                                    Vec::new(),
                                    Some(EventDetail::EpisodeClosed {
                                        episode_id: ep_id,
                                        place: place.clone(),
                                        state_label: ep_state_label,
                                        event_ids: ep_events,
                                        gist_id: Some(stored_thought.id),
                                        superseded_gist_id: prev_gist,
                                        merged_from: Vec::new(),
                                    }),
                                );
                                append_and_apply(core, ep_closed)?;
                            }

                            // Trigger post-sleep `consolidate` verbatim-vs-gist verification (`10.2`).
                            // The verdict is a classification (faithful / conflict) over the
                            // gist and its sources, so it is requested as `Classify`; a
                            // `Consolidate` request would be answered with a new gist instead.
                            if !about_ids.is_empty() {
                                let scope = core.state().visible_scope_for(
                                    stored_thought.audience.clone(),
                                    Some(stored_thought.place.clone()),
                                    now,
                                );
                                let mut sources = Vec::new();
                                let mut material_err = None;
                                for &sid in &about_ids {
                                    match task_segment_for(core, sid, &scope, now) {
                                        Ok(Some(seg)) => sources.push(seg),
                                        Ok(None) => {}
                                        Err(e) => material_err = Some(e),
                                    }
                                }
                                let mut items = vec![stored_thought.id];
                                items.extend(about_ids.iter().copied());
                                let planned = match material_err {
                                    Some(e) => Err(e),
                                    None if sources.is_empty() => {
                                        Err("no source message of the gist is available".to_owned())
                                    }
                                    None => build_task_plan(
                                        core,
                                        &stored_thought,
                                        ModelTask::GistFaithful { sources },
                                        None,
                                        now,
                                        items,
                                    ),
                                };
                                match planned {
                                    Ok(plan) => {
                                        core.state_mut().expectation_book.pending_classify.insert(
                                            stored_thought.id,
                                            PendingClassifyKind::GistVerification {
                                                gist_id: stored_thought.id,
                                                source_ids: about_ids.clone(),
                                            },
                                        );
                                        effects.push(Effect::Model {
                                            purpose: ModelPurpose::Classify,
                                            plan,
                                        });
                                    }
                                    Err(reason) => {
                                        record_classify_refusal(
                                            core,
                                            &stored_thought,
                                            now,
                                            &reason,
                                        )?;
                                    }
                                }
                            }

                            // Update active PendingSleepRun and finalize SleepRun if all episodes finished.
                            let finished_run =
                                if let Some(run) = core.state_mut().pending_sleep_run.as_mut() {
                                    if let Some(ep_id) = matched_ep_id {
                                        run.pending_episode_ids.retain(|id| *id != ep_id);
                                        if !run.completed_episode_ids.contains(&ep_id) {
                                            run.completed_episode_ids.push(ep_id);
                                        }
                                    }
                                    if !run.produced_gist_ids.contains(&stored_thought.id) {
                                        run.produced_gist_ids.push(stored_thought.id);
                                    }
                                    if run.pending_episode_ids.is_empty() {
                                        Some(run.clone())
                                    } else {
                                        None
                                    }
                                } else {
                                    None
                                };
                            if let Some(done_run) = finished_run {
                                let sleep_rec = EventRecord::derived_from(
                                    &stored_thought,
                                    EventKind::SleepRun,
                                    now,
                                    ActorId::new("core"),
                                    stored_thought.place.clone(),
                                    stored_thought.audience.clone(),
                                    None,
                                    NumericFeatures::default(),
                                    OpaquePayload::default(),
                                    Vec::new(),
                                    Some(EventDetail::SleepRun {
                                        snapshot_ms: done_run.snapshot_ms,
                                        snapshot_seq: done_run.snapshot_seq,
                                        covered_event_ids: done_run.covered_event_ids,
                                        produced_ids: done_run.produced_gist_ids,
                                        tokens_used: done_run.tokens_used,
                                        completed: true,
                                    }),
                                );
                                append_and_apply(core, sleep_rec)?;
                            }
                        }
                        ConsolidateOutput::Procedure {
                            name,
                            steps,
                            required_cap_ids,
                            capability_schema_hash,
                            domain,
                        } => {
                            // Write Door (`R-43`): untrusted procedural consolidation is refused.
                            if stored_thought.taint != Taint::Trusted {
                                let dv = EventRecord::derived_from(
                                    &stored_thought,
                                    EventKind::DoorVerdict,
                                    now,
                                    ActorId::new("core"),
                                    stored_thought.place.clone(),
                                    stored_thought.audience.clone(),
                                    None,
                                    NumericFeatures::default(),
                                    OpaquePayload::default(),
                                    Vec::new(),
                                    Some(EventDetail::DoorVerdict {
                                        door: DoorKind::Action,
                                        target_event_id: Some(stored_thought.id),
                                        rule: "write_door_procedure_taint".to_owned(),
                                        verdict: "refuse: untrusted_procedure".to_owned(),
                                    }),
                                );
                                append_and_apply(core, dv)?;
                                continue;
                            }
                            let params = RegisterProcedureParams {
                                procedure_id: Some(stored_thought.id),
                                name: name.clone(),
                                steps: steps.clone(),
                                required_cap_ids: required_cap_ids.clone(),
                                capability_schema_hash: capability_schema_hash.clone(),
                                audience: stored_thought.audience.clone(),
                                domain: domain.clone(),
                                taint: stored_thought.taint,
                                now_ms: now,
                            };
                            let catalog_c = core.state().capability_catalog.clone();
                            if let Some(tp) = core
                                .state_mut()
                                .skill_book
                                .register_consolidated_procedure(params, &catalog_c)
                            {
                                core.state_mut().register_procedure(tp.to_context_entry());
                                let proc_ev = EventRecord::derived_from(
                                    &stored_thought,
                                    EventKind::LearningChange,
                                    now,
                                    ActorId::new("core"),
                                    stored_thought.place.clone(),
                                    stored_thought.audience.clone(),
                                    None,
                                    NumericFeatures::default(),
                                    OpaquePayload::default(),
                                    Vec::new(),
                                    Some(EventDetail::ProcedureUpdated {
                                        procedure_id: tp.procedure_id,
                                        name: tp.name,
                                        steps: tp.steps,
                                        required_cap_ids: tp.required_cap_ids,
                                        capability_schema_hash: tp.capability_schema_hash,
                                        domain: tp.domain,
                                        state_code: tp.state.code(),
                                        reason: tp.retired_reason,
                                    }),
                                );
                                append_and_apply(core, proc_ev)?;
                            }
                        }
                        ConsolidateOutput::Lesson {
                            pattern,
                            failure_signature,
                            contrastive_success_ids,
                            failure_ids,
                            domain,
                        } => {
                            if stored_thought.taint != Taint::Trusted {
                                let dv = EventRecord::derived_from(
                                    &stored_thought,
                                    EventKind::DoorVerdict,
                                    now,
                                    ActorId::new("core"),
                                    stored_thought.place.clone(),
                                    stored_thought.audience.clone(),
                                    None,
                                    NumericFeatures::default(),
                                    OpaquePayload::default(),
                                    Vec::new(),
                                    Some(EventDetail::DoorVerdict {
                                        door: DoorKind::Action,
                                        target_event_id: Some(stored_thought.id),
                                        rule: "write_door_lesson_taint".to_owned(),
                                        verdict: "refuse: untrusted_lesson".to_owned(),
                                    }),
                                );
                                append_and_apply(core, dv)?;
                                continue;
                            }
                            let params = RecordLessonParams {
                                lesson_id: Some(stored_thought.id),
                                pattern,
                                failure_signature,
                                failure_ids,
                                contrastive_success_ids,
                                domain,
                                audience: stored_thought.audience.clone(),
                                taint: stored_thought.taint,
                                now_ms: now,
                            };
                            if let Some(lesson) =
                                core.state_mut().skill_book.record_consolidated_lesson(params)
                            {
                                let l_rec = EventRecord::derived_from(
                                    &stored_thought,
                                    EventKind::LearningChange,
                                    now,
                                    ActorId::new("core"),
                                    stored_thought.place.clone(),
                                    stored_thought.audience.clone(),
                                    None,
                                    NumericFeatures::default(),
                                    OpaquePayload::default(),
                                    Vec::new(),
                                    Some(EventDetail::LessonRecorded {
                                        lesson_id: lesson.lesson_id,
                                        pattern: lesson.pattern,
                                        failure_signature: lesson.failure_signature,
                                        failure_ids: lesson.failure_ids,
                                        contrastive_success_ids: lesson.contrastive_success_ids,
                                        domain: lesson.domain,
                                    }),
                                );
                                append_and_apply(core, l_rec)?;
                            }
                        }
                    },
                    ThoughtAct::Resolve {
                        expectation_id,
                        outcome,
                        places,
                        ..
                    } => {
                        if let Some(exp) = core
                            .state()
                            .expectation_book
                            .expectations
                            .get(&expectation_id)
                            .cloned()
                        {
                            let to_state = map_resolve_outcome(outcome, exp.due_ms, now, exp.kind);
                            let res_rec = EventRecord::derived_from(
                                &stored_thought,
                                EventKind::ExpectationChanged,
                                now,
                                ActorId::new("core"),
                                places
                                    .first()
                                    .cloned()
                                    .or_else(|| exp.places.first().cloned())
                                    .unwrap_or_else(|| stored_thought.place.clone()),
                                stored_thought.audience.clone(),
                                None,
                                NumericFeatures::default(),
                                OpaquePayload::default(),
                                Vec::new(),
                                Some(EventDetail::ExpectationTransition {
                                    expectation_id,
                                    action_id: None,
                                    h_act: None,
                                    from_state: Some(exp.state),
                                    to_state,
                                }),
                            );
                            append_and_apply(core, res_rec)?;
                            if to_state == ExpectationLifecycleState::Reached
                                && exp.kind == ExpectKind::Goal
                            {
                                maybe_extract_procedure_from_goal(
                                    core,
                                    &exp,
                                    &stored_thought,
                                    now,
                                )?;
                            }
                            if is_fulfilled_state(to_state) {
                                let mut unblocked =
                                    flush_deferred_dependents(core, expectation_id, now, doors)?;
                                effects.append(&mut unblocked);
                            }
                        }
                    }
                    ThoughtAct::Act {
                        capability_id,
                        places,
                        people,
                        payload,
                        motivation,
                        confidence_0_20,
                        used_ids,
                        expected,
                        domain,
                    } => {
                        let action_seq = core.state().next_action_seq.max(1);
                        let action_id = ActionId::new(format!("act-{action_seq}"));
                        let mut act_taint = stored_thought.taint;
                        for &uid in &used_ids {
                            if let Some(m) = core.state().memory_entries.get(&uid) {
                                act_taint = join_min_taint(act_taint, m.taint);
                            }
                        }
                        let action = Action {
                            protocol_version: CURRENT_PROTOCOL_VERSION,
                            action_id: action_id.clone(),
                            capability_id: capability_id.clone(),
                            places: places.clone(),
                            audience: stored_thought.audience.clone(),
                            origin_actor: stored_thought.origin_actor.clone(),
                            taint: act_taint,
                            payload: payload.clone(),
                            correlation_id: None,
                            expected_effect: expected,
                            h_act: None,
                            timestamp_ms: now,
                        };
                        action.validate(CURRENT_PROTOCOL_VERSION)?;

                        // Guard 1: CitationLock (7.3)
                        if let Some(bad_id) = check_citation_lock(
                            core,
                            wake_snapshot.as_ref(),
                            &turn_created_ids,
                            &used_ids,
                        )? {
                            record_act_refused(
                                core,
                                &stored_thought,
                                &action,
                                "citation_lock",
                                format!("refuse: ungrounded citation {}", bad_id.as_u64()),
                                now,
                            )?;
                            continue;
                        }

                        // Guard 2: Unknown / non-disclosed capability (7.5)
                        if capability_id.as_str() != "memory.recall"
                            && !core.state().capability_descriptors.is_empty()
                        {
                            let disclosed = wake_snapshot.as_ref().map_or_else(
                                || {
                                    core.state()
                                        .capability_descriptors
                                        .contains_key(&capability_id)
                                },
                                |snap| snap.disclosed_capabilities.contains(&capability_id),
                            );
                            if !disclosed {
                                record_act_refused(
                                    core,
                                    &stored_thought,
                                    &action,
                                    "unknown_capability",
                                    format!(
                                        "refuse: capability '{}' not disclosed",
                                        capability_id.as_str()
                                    ),
                                    now,
                                )?;
                                continue;
                            }
                        }

                        // Guard 3: `[ANCHOR_CHECK]` & `PrincipleAudit` (7.3)
                        let is_identity_changing = core
                            .state()
                            .capability_descriptors
                            .get(&capability_id)
                            .is_some_and(|d| d.flags.identity_changing);
                        let mut anchor_rule: Option<(&str, String)> = None;
                        for p in &core.state().principles {
                            if p.payload.severity == PrincipleSeverity::Advisory {
                                continue;
                            }
                            if p.payload.forbid_identity_change && is_identity_changing {
                                anchor_rule = Some((
                                    "anchor_check",
                                    format!("refuse: anchor violates {}", p.payload.id),
                                ));
                                break;
                            }
                            if p.payload.blocked_capabilities.contains(&capability_id) {
                                anchor_rule = Some((
                                    "principle_audit",
                                    format!(
                                        "refuse: principle {} blocks {}",
                                        p.payload.id,
                                        capability_id.as_str()
                                    ),
                                ));
                                break;
                            }
                            if p.payload.forbid_untrusted_taint && act_taint == Taint::Untrusted {
                                anchor_rule = Some((
                                    "principle_audit_taint",
                                    format!(
                                        "refuse: principle {} rejects untrusted act",
                                        p.payload.id
                                    ),
                                ));
                                break;
                            }
                        }
                        if let Some((rule, msg)) = anchor_rule {
                            core.state_mut().contradiction_flags += 1;
                            core.state_mut().principle_beta += 1.0;
                            if core
                                .state_mut()
                                .ledger
                                .record_principle_verdict(false, true)
                            {
                                core.state_mut().recalibration_pending = true;
                            }
                            record_act_refused(core, &stored_thought, &action, rule, msg, now)?;
                            continue;
                        }

                        let linked_exp_id = find_linked_expectation_for_act(
                            core.state(),
                            request_id,
                            &used_ids,
                            &stored_thought,
                        );

                        // Guard 4: `CommitCheck` against active exclusive/conflicting promises (7.3)
                        let conflicting_promise = core
                            .state()
                            .expectation_book
                            .expectations
                            .values()
                            .find(|exp| {
                                exp.is_active()
                                    && exp.kind == ExpectKind::Promise
                                    && Some(exp.expectation_id) != linked_exp_id
                                    && exp.places.iter().any(|p| places.contains(p))
                                    && exp
                                        .promise_meta
                                        .as_ref()
                                        .is_some_and(|m| m.conflict_key.is_some())
                            })
                            .map(|exp| exp.expectation_id);
                        if let Some(conf_id) = conflicting_promise {
                            core.state_mut().contradiction_flags += 1;
                            core.state_mut().principle_beta += 1.0;
                            if core
                                .state_mut()
                                .ledger
                                .record_principle_verdict(false, true)
                            {
                                core.state_mut().recalibration_pending = true;
                            }
                            record_act_refused(
                                core,
                                &stored_thought,
                                &action,
                                "commit_check",
                                format!("refuse: conflicts with promise {}", conf_id.as_u64()),
                                now,
                            )?;
                            continue;
                        }

                        // Guard 5: `[0, 20]` confidence + `1-5` motivation bars & `P_rc` report control (7.4)
                        if capability_id.as_str() != "memory.recall" {
                            let (think_val, think_cost, think_dom) = match &last_think_stakes {
                                Some((v, c, d)) => (*v, *c, d.clone()),
                                None => (None, None, None),
                            };
                            let eff_domain = domain.clone().or(think_dom);
                            let dom_offset = eff_domain
                                .as_ref()
                                .and_then(|d| {
                                    core.state().domain_calibration_offsets.get(d).copied()
                                })
                                .unwrap_or(0.0);
                            let c_cal = calibrate_confidence_0_20(confidence_0_20, dom_offset);
                            let p_rc = compute_report_threshold(
                                think_val,
                                think_cost,
                                core.config().certainty_bar,
                                true,
                            );

                            if !turn_allocated && motivation < DEFAULT_SPEAKING_BAR {
                                record_withheld(
                                    core,
                                    &stored_thought,
                                    confidence_0_20,
                                    c_cal,
                                    core.config().certainty_bar,
                                    "below_speaking_bar",
                                    now,
                                )?;
                                continue;
                            }

                            let has_explicit_stakes = think_val.is_some() || think_cost.is_some();
                            let existing_ar =
                                core.state().auto_recall_states.get(&request_id).cloned();
                            let should_check_prc =
                                !turn_allocated || has_explicit_stakes || existing_ar.is_some();

                            if should_check_prc && c_cal < p_rc {
                                let is_communicative = core
                                    .state()
                                    .capability_descriptors
                                    .get(&capability_id)
                                    .map_or_else(
                                        || {
                                            let s = capability_id.as_str();
                                            s.contains("chat")
                                                || s.contains("reply")
                                                || s.contains("message")
                                        },
                                        |d| d.flags.communicative,
                                    );
                                let mut ar =
                                    existing_ar.unwrap_or(AutoRecallTriggerState {
                                        active: true,
                                        attempt: 0,
                                        max_attempts: AUTO_RECALL_MAX_ATTEMPTS,
                                        coarser_grain_allowed: false,
                                        last_reason: None,
                                    });
                                if core.config().auto_recall_enabled
                                    && ar.attempt < AUTO_RECALL_MAX_ATTEMPTS
                                {
                                    ar.attempt = ar.attempt.saturating_add(1);
                                    ar.active = true;
                                    ar.max_attempts = AUTO_RECALL_MAX_ATTEMPTS;
                                    ar.last_reason = Some("below_p_rc".to_owned());
                                    core.state_mut()
                                        .auto_recall_states
                                        .insert(request_id, ar.clone());
                                    core.state_mut()
                                        .auto_recall_states
                                        .insert(stored_thought.id, ar.clone());

                                    let recall_action_seq = core.state().next_action_seq.max(1);
                                    let recall_action_id =
                                        ActionId::new(format!("act-{recall_action_seq}"));
                                    let primary_place = places
                                        .first()
                                        .cloned()
                                        .unwrap_or_else(|| stored_thought.place.clone());
                                    let recall_action = Action {
                                        protocol_version: CURRENT_PROTOCOL_VERSION,
                                        action_id: recall_action_id.clone(),
                                        capability_id: CapabilityId::new("memory.recall"),
                                        places: vec![primary_place.clone()],
                                        audience: stored_thought.audience.clone(),
                                        origin_actor: stored_thought.origin_actor.clone(),
                                        taint: stored_thought.taint,
                                        payload: OpaquePayload::from_bytes(b"auto_recall"),
                                        correlation_id: None,
                                        expected_effect: None,
                                        h_act: None,
                                        timestamp_ms: now,
                                    };
                                    let recall_rec = EventRecord::derived_from(
                                        &stored_thought,
                                        EventKind::Action,
                                        now,
                                        ActorId::new("soul"),
                                        primary_place.clone(),
                                        stored_thought.audience.clone(),
                                        None,
                                        NumericFeatures::default(),
                                        recall_action.payload.clone(),
                                        Vec::new(),
                                        Some(EventDetail::Action {
                                            action_id: recall_action_id,
                                            capability_id: CapabilityId::new("memory.recall"),
                                            h_act: None,
                                            correlation_id: None,
                                        }),
                                    );
                                    let stored_recall_act = append_and_apply(core, recall_rec)?;
                                    core.state_mut()
                                        .turn_allocated_by_trigger
                                        .insert(stored_recall_act.id, turn_allocated);
                                    core.state_mut()
                                        .auto_recall_states
                                        .insert(stored_recall_act.id, ar.clone());
                                    let query_ev = used_ids
                                        .first()
                                        .copied()
                                        .or_else(|| causing.as_ref().map(|c| c.id));
                                    if let Some(eff) = execute_memory_recall_act(
                                        core,
                                        &stored_recall_act,
                                        &recall_action,
                                        query_ev,
                                        &primary_place,
                                        &people,
                                        now,
                                        doors,
                                    )? {
                                        if let Effect::Model { plan, .. } = &eff
                                            && let Some(new_tid) = plan.trigger_event_id
                                        {
                                            core.state_mut()
                                                .auto_recall_states
                                                .insert(new_tid, ar.clone());
                                            core.state_mut()
                                                .turn_allocated_by_trigger
                                                .insert(new_tid, turn_allocated);
                                        }
                                        effects.push(eff);
                                    }
                                    continue;
                                } else if is_communicative && !ar.coarser_grain_allowed {
                                    ar.coarser_grain_allowed = true;
                                    ar.active = true;
                                    ar.last_reason = Some("coarser_grain_requested".to_owned());
                                    core.state_mut()
                                        .auto_recall_states
                                        .insert(request_id, ar.clone());
                                    core.state_mut()
                                        .auto_recall_states
                                        .insert(stored_thought.id, ar.clone());
                                    let mut wake_effects = wake_reasoning_for_event_with_extra(
                                        core,
                                        &stored_thought,
                                        &[request_id],
                                        now,
                                        doors,
                                    )?;
                                    effects.append(&mut wake_effects);
                                    continue;
                                } else {
                                    record_withheld(
                                        core,
                                        &stored_thought,
                                        confidence_0_20,
                                        c_cal,
                                        p_rc,
                                        "below_p_rc",
                                        now,
                                    )?;
                                    continue;
                                }
                            } else if !turn_allocated && c_cal < core.config().certainty_bar {
                                record_withheld(
                                    core,
                                    &stored_thought,
                                    confidence_0_20,
                                    c_cal,
                                    core.config().certainty_bar,
                                    "below_certainty_bar",
                                    now,
                                )?;
                                continue;
                            }
                        }

                        core.state_mut().principle_alpha += 1.0;
                        let _ = core
                            .state_mut()
                            .ledger
                            .record_principle_verdict(true, false);

                        let mut act_embeddings = Vec::new();
                        for &uid in &used_ids {
                            if let Some(m) = core.state().memory_entries.get(&uid) {
                                act_embeddings.extend(m.embeddings.clone());
                            }
                        }

                        if let Some(eid) = linked_exp_id
                            && let Some(exp) = core.state().expectation_book.expectations.get(&eid)
                            && let Some(dep_id) =
                                exp.promise_meta.as_ref().and_then(|m| m.depends_on)
                            && core.state().expectation_book.is_dependency_open(dep_id)
                        {
                            core.state_mut().expectation_book.deferred_acts.push(
                                DeferredDependentAct {
                                    expectation_id: eid,
                                    depends_on: dep_id,
                                    thought_event_id: stored_thought.id,
                                    action,
                                    people,
                                    stakes: 1.0,
                                    act_embeddings,
                                    expected_reply_embeddings: Vec::new(),
                                },
                            );
                            let dv = EventRecord::derived_from(
                                &stored_thought,
                                EventKind::DoorVerdict,
                                now,
                                ActorId::new("core"),
                                stored_thought.place.clone(),
                                stored_thought.audience.clone(),
                                None,
                                NumericFeatures::default(),
                                OpaquePayload::default(),
                                Vec::new(),
                                Some(EventDetail::DoorVerdict {
                                    door: DoorKind::Action,
                                    target_event_id: Some(stored_thought.id),
                                    rule: "depends_on".to_owned(),
                                    verdict: format!("deferred_on:{dep_id}"),
                                }),
                            );
                            append_and_apply(core, dv)?;
                            continue;
                        }

                        let mut dispatched = dispatch_action_and_open_predictions(
                            core,
                            &stored_thought,
                            action,
                            &used_ids,
                            &people,
                            linked_exp_id,
                            &act_embeddings,
                            &[],
                            now,
                            doors,
                        )?;
                        effects.append(&mut dispatched);
                    }
                }
            }

            let dec = EventRecord::derived_from(
                &stored_thought,
                EventKind::Decision,
                now,
                ActorId::new("core"),
                stored_thought.place.clone(),
                stored_thought.audience.clone(),
                None,
                NumericFeatures::default(),
                OpaquePayload::default(),
                Vec::new(),
                Some(EventDetail::Decision {
                    target_event_id: stored_thought.id,
                    outcome: DecisionOutcome::Processed,
                    salience: 0.0,
                    threshold: 0.0,
                    probability: 0.0,
                    draw: 0.0,
                    reason: None,
                    trace: None,
                }),
            );
            append_and_apply(core, dec)?;
            Ok(effects)
        }

        Input::ClassifyResult {
            request_id,
            verdict,
        } => handle_classify_result(core, request_id, &verdict.label, now, doors),

        Input::ModelFailure { request_id, reason } => {
            let mut rec = EventRecord::new_root(
                EventKind::Failure,
                now,
                ActorId::new("core"),
                PlaceId::new("unknown"),
                Audience::new([ActorId::new("soul")]),
                OriginActor::new(ActorId::new("soul")),
                Taint::Provisional,
                NumericFeatures::default(),
                OpaquePayload::from_bytes(reason.as_bytes()),
                Vec::new(),
                Some(EventDetail::Failure {
                    target_event_id: Some(request_id),
                    component: "model".to_owned(),
                    detail: reason,
                }),
            );
            if request_id.as_u64() != 0 {
                rec.caused_by.push(request_id);
            }
            append_and_apply(core, rec)?;
            Ok(Vec::new())
        }

        Input::Identity(signed) => {
            let rec = EventRecord::new_root(
                EventKind::IdentityChanged,
                now,
                signed.signer.clone(),
                PlaceId::new("core:identity"),
                Audience::new([signed.signer.clone()]),
                OriginActor::new(signed.signer.clone()),
                Taint::Trusted,
                NumericFeatures::default(),
                OpaquePayload::from_bytes(encode_identity_payload(&signed)),
                Vec::new(),
                None,
            );
            let stored = append_and_apply(core, rec)?;
            let dec = EventRecord::derived_from(
                &stored,
                EventKind::Decision,
                now,
                ActorId::new("core"),
                stored.place.clone(),
                stored.audience.clone(),
                None,
                NumericFeatures::default(),
                OpaquePayload::default(),
                Vec::new(),
                Some(EventDetail::Decision {
                    target_event_id: stored.id,
                    outcome: DecisionOutcome::Processed,
                    salience: 0.0,
                    threshold: 0.0,
                    probability: 0.0,
                    draw: 0.0,
                    reason: None,
                    trace: None,
                }),
            );
            append_and_apply(core, dec)?;
            Ok(Vec::new())
        }

        Input::BodyState(body_state) => {
            let rec = EventRecord::new_root(
                EventKind::BodyState,
                body_state.timestamp_ms,
                ActorId::new(body_state.body_id.as_str()),
                PlaceId::new("core:body_state"),
                Audience::new([ActorId::new(body_state.body_id.as_str())]),
                OriginActor::new(ActorId::new(body_state.body_id.as_str())),
                Taint::Trusted,
                NumericFeatures::default(),
                OpaquePayload::from_bytes(vec![encode_body_status(body_state.status)]),
                Vec::new(),
                Some(EventDetail::BodyCapabilities {
                    body_id: body_state.body_id.clone(),
                    status: body_state.status,
                    capabilities: body_state.capabilities,
                }),
            );
            let stored = append_and_apply(core, rec)?;
            let dec = EventRecord::derived_from(
                &stored,
                EventKind::Decision,
                now,
                ActorId::new("core"),
                stored.place.clone(),
                stored.audience.clone(),
                None,
                NumericFeatures::default(),
                OpaquePayload::default(),
                Vec::new(),
                Some(EventDetail::Decision {
                    target_event_id: stored.id,
                    outcome: DecisionOutcome::Processed,
                    salience: 0.0,
                    threshold: 0.0,
                    probability: 0.0,
                    draw: 0.0,
                    reason: None,
                    trace: None,
                }),
            );
            append_and_apply(core, dec)?;
            flush_recalibration_if_pending(
                core,
                &stored,
                "body_state_capability_change",
                now,
                doors,
            )?;
            Ok(Vec::new())
        }

        Input::ActorFacts(actor_facts) => {
            let rec = EventRecord::new_root(
                EventKind::ActorFacts,
                now,
                actor_facts.actor_id.clone(),
                PlaceId::new("core:actor_facts"),
                Audience::new([actor_facts.actor_id.clone()]),
                OriginActor::new(actor_facts.actor_id.clone()),
                if actor_facts.trusted_origin {
                    Taint::Trusted
                } else {
                    Taint::Provisional
                },
                NumericFeatures::default(),
                OpaquePayload::from_bytes(encode_actor_facts_from_msg(&actor_facts)),
                Vec::new(),
                None,
            );
            let stored = append_and_apply(core, rec)?;
            let dec = EventRecord::derived_from(
                &stored,
                EventKind::Decision,
                now,
                ActorId::new("core"),
                stored.place.clone(),
                stored.audience.clone(),
                None,
                NumericFeatures::default(),
                OpaquePayload::default(),
                Vec::new(),
                Some(EventDetail::Decision {
                    target_event_id: stored.id,
                    outcome: DecisionOutcome::Processed,
                    salience: 0.0,
                    threshold: 0.0,
                    probability: 0.0,
                    draw: 0.0,
                    reason: None,
                    trace: None,
                }),
            );
            append_and_apply(core, dec)?;
            Ok(Vec::new())
        }

        Input::Tick => {
            // Spec: "A tick that changes state SHALL be recorded; a tick that changes nothing SHALL leave no event."
            let needs_recovery = core.state().startup_recovery_pending;
            let has_pending_sleep = core.state().pending_sleep_run.is_some();
            if !needs_recovery && !has_pending_sleep && core.next_due().is_none_or(|due| now < due)
            {
                return Ok(Vec::new());
            }
            let mut effects = Vec::new();
            if needs_recovery {
                let mut rec_effects = run_startup_recovery(core, now, doors)?;
                effects.append(&mut rec_effects);
                if !has_pending_sleep && core.next_due().is_none_or(|due| now < due) {
                    return Ok(effects);
                }
            }

            let rec = EventRecord::new_root(
                EventKind::Tick,
                now,
                ActorId::new("core"),
                PlaceId::new("core:tick"),
                Audience::new([ActorId::new("core")]),
                OriginActor::new(ActorId::new("core")),
                Taint::Trusted,
                NumericFeatures::default(),
                OpaquePayload::default(),
                Vec::new(),
                None,
            );
            let stored = append_and_apply(core, rec)?;
            let dec = EventRecord::derived_from(
                &stored,
                EventKind::Decision,
                now,
                ActorId::new("core"),
                stored.place.clone(),
                stored.audience.clone(),
                None,
                NumericFeatures::default(),
                OpaquePayload::default(),
                Vec::new(),
                Some(EventDetail::Decision {
                    target_event_id: stored.id,
                    outcome: DecisionOutcome::Processed,
                    salience: 0.0,
                    threshold: 0.0,
                    probability: 0.0,
                    draw: 0.0,
                    reason: None,
                    trace: None,
                }),
            );
            append_and_apply(core, dec)?;

            let mut due_waits: Vec<_> = core
                .state()
                .open_waits
                .values()
                .filter(|w| w.due_ms <= now)
                .cloned()
                .collect();
            due_waits.sort_by(|a, b| {
                a.due_ms
                    .cmp(&b.due_ms)
                    .then_with(|| a.expectation_id.cmp(&b.expectation_id))
            });

            for wait in due_waits {
                let reached_rec = EventRecord::derived_from(
                    &stored,
                    EventKind::ExpectationChanged,
                    now,
                    ActorId::new("core"),
                    wait.place.clone(),
                    wait.audience.clone(),
                    None,
                    NumericFeatures::default(),
                    OpaquePayload::default(),
                    Vec::new(),
                    Some(EventDetail::ExpectationTransition {
                        expectation_id: wait.expectation_id,
                        action_id: None,
                        h_act: None,
                        from_state: Some(ExpectationLifecycleState::Open),
                        to_state: ExpectationLifecycleState::Reached,
                    }),
                );
                append_and_apply(core, reached_rec)?;

                if let Some(target_rec) = core.store().get(wait.target_event_id)? {
                    let mut wake_effects = wake_reasoning_for_event(core, &target_rec, now, doors)?;
                    effects.append(&mut wake_effects);
                }
            }

            let mut due_exp_effects = handle_due_expectations(core, &stored, now, doors)?;
            effects.append(&mut due_exp_effects);
            let mut sleep_effects = maybe_run_sleep_consolidation(core, &stored, now, doors)?;
            effects.append(&mut sleep_effects);
            Ok(effects)
        }
    }
}


fn flush_deferred_dependents<S: Store>(
    core: &mut Core<S>,
    resolved_id: EventId,
    now: Time,
    doors: &mut dyn Doors,
) -> Result<Vec<Effect>, CoreError> {
    let mut ready = Vec::new();
    let mut remaining = Vec::new();
    for def in core.state_mut().expectation_book.deferred_acts.drain(..) {
        if def.depends_on == resolved_id {
            ready.push(def);
        } else {
            remaining.push(def);
        }
    }
    core.state_mut().expectation_book.deferred_acts = remaining;

    let mut effects = Vec::new();
    for mut def in ready {
        if let Some(thought) = core.store().get(def.thought_event_id)? {
            let action_seq = core.state().next_action_seq.max(1);
            def.action.action_id = ActionId::new(format!("act-{action_seq}"));
            def.action.timestamp_ms = now;
            let mut dispatched = dispatch_action_and_open_predictions(
                core,
                &thought,
                def.action,
                &[],
                &def.people,
                Some(def.expectation_id),
                &def.act_embeddings,
                &def.expected_reply_embeddings,
                now,
                doors,
            )?;
            effects.append(&mut dispatched);
        }
    }
    Ok(effects)
}

fn handle_receipt_expectations<S: Store>(
    core: &mut Core<S>,
    stored_receipt: &EventRecord,
    receipt: &monoagens_protocol::Receipt,
    now: Time,
    doors: &mut dyn Doors,
) -> Result<Vec<Effect>, CoreError> {
    let mut effects = Vec::new();

    // 1. Update open Response / Effect predictions tied to this action_id.
    let pred_ids = core
        .state()
        .expectation_book
        .find_predictions_for_action(&receipt.action_id);
    for pid in pred_ids {
        let Some(pred) = core
            .state()
            .expectation_book
            .expectations
            .get(&pid)
            .cloned()
        else {
            continue;
        };
        if pred.kind == ExpectKind::Response {
            let next_state = match receipt.outcome {
                ReceiptOutcome::Done
                    if pred
                        .response_spec
                        .as_ref()
                        .is_some_and(|s| s.accept_done_receipt) =>
                {
                    Some(ExpectationLifecycleState::Answered)
                }
                ReceiptOutcome::Failed | ReceiptOutcome::Refused => {
                    Some(ExpectationLifecycleState::Cancelled)
                }
                _ => None,
            };
            if let Some(to_state) = next_state {
                let tr = EventRecord::derived_from(
                    stored_receipt,
                    EventKind::ExpectationChanged,
                    now,
                    ActorId::new("core"),
                    stored_receipt.place.clone(),
                    stored_receipt.audience.clone(),
                    None,
                    NumericFeatures::default(),
                    OpaquePayload::default(),
                    Vec::new(),
                    Some(EventDetail::ExpectationTransition {
                        expectation_id: pid,
                        action_id: Some(receipt.action_id.clone()),
                        h_act: receipt.h_act.clone(),
                        from_state: Some(pred.state),
                        to_state,
                    }),
                );
                append_and_apply(core, tr)?;
            }
        } else if pred.kind == ExpectKind::Effect
            && matches!(
                receipt.outcome,
                ReceiptOutcome::Failed | ReceiptOutcome::Refused
            )
        {
            let tr = EventRecord::derived_from(
                stored_receipt,
                EventKind::ExpectationChanged,
                now,
                ActorId::new("core"),
                stored_receipt.place.clone(),
                stored_receipt.audience.clone(),
                None,
                NumericFeatures::default(),
                OpaquePayload::default(),
                Vec::new(),
                Some(EventDetail::ExpectationTransition {
                    expectation_id: pid,
                    action_id: Some(receipt.action_id.clone()),
                    h_act: receipt.h_act.clone(),
                    from_state: Some(pred.state),
                    to_state: ExpectationLifecycleState::Cancelled,
                }),
            );
            append_and_apply(core, tr)?;
        }
    }

    // 2. Update linked Promise / Goal / PendingApproval expectation.
    let Some(exp_id) = core
        .state()
        .expectation_book
        .find_by_action(&receipt.action_id)
    else {
        return Ok(effects);
    };
    let Some(exp) = core
        .state()
        .expectation_book
        .expectations
        .get(&exp_id)
        .cloned()
    else {
        return Ok(effects);
    };
    if !matches!(
        exp.kind,
        ExpectKind::Promise | ExpectKind::Goal | ExpectKind::PendingApproval
    ) {
        return Ok(effects);
    }

    // Record delivered_units & termination_reason on the expectation record.
    if let Some(mut_exp) = core
        .state_mut()
        .expectation_book
        .expectations
        .get_mut(&exp_id)
    {
        if receipt.delivered_units.is_some() {
            mut_exp.delivered_units = receipt.delivered_units;
        }
        if receipt.termination_reason.is_some() {
            mut_exp.termination_reason = receipt.termination_reason;
        }
    }

    let is_preempted = matches!(
        receipt.termination_reason,
        Some(TerminationReason::PreemptedExternal | TerminationReason::CanceledByCore)
    );

    match receipt.outcome {
        ReceiptOutcome::Held => {
            // Park in non-retrying PendingApproval pinned to H_act; never schedule retry.
            let pa_rec = EventRecord::derived_from(
                stored_receipt,
                EventKind::ExpectationChanged,
                now,
                ActorId::new("core"),
                stored_receipt.place.clone(),
                stored_receipt.audience.clone(),
                None,
                NumericFeatures::default(),
                OpaquePayload::default(),
                Vec::new(),
                Some(EventDetail::ExpectationTransition {
                    expectation_id: exp_id,
                    action_id: Some(receipt.action_id.clone()),
                    h_act: receipt.h_act.clone(),
                    from_state: Some(exp.state),
                    to_state: ExpectationLifecycleState::PendingApproval,
                }),
            );
            append_and_apply(core, pa_rec)?;
        }
        ReceiptOutcome::Done => {
            if exp.state == ExpectationLifecycleState::PendingApproval
                && let Some(pinned) = &exp.last_h_act
                && receipt.h_act.as_ref().is_some_and(|h| h != pinned)
            {
                // Anti-TOCTOU H_act mismatch rejects completion.
                let rej_rec = EventRecord::derived_from(
                    stored_receipt,
                    EventKind::ExpectationChanged,
                    now,
                    ActorId::new("core"),
                    stored_receipt.place.clone(),
                    stored_receipt.audience.clone(),
                    None,
                    NumericFeatures::default(),
                    OpaquePayload::default(),
                    Vec::new(),
                    Some(EventDetail::ExpectationTransition {
                        expectation_id: exp_id,
                        action_id: Some(receipt.action_id.clone()),
                        h_act: receipt.h_act.clone(),
                        from_state: Some(exp.state),
                        to_state: ExpectationLifecycleState::Rejected,
                    }),
                );
                append_and_apply(core, rej_rec)?;
                return Ok(effects);
            }

            let to_state = if exp.kind == ExpectKind::Goal {
                ExpectationLifecycleState::Reached
            } else if now > exp.due_ms {
                ExpectationLifecycleState::KeptLate
            } else {
                ExpectationLifecycleState::Kept
            };
            let done_rec = EventRecord::derived_from(
                stored_receipt,
                EventKind::ExpectationChanged,
                now,
                ActorId::new("core"),
                stored_receipt.place.clone(),
                stored_receipt.audience.clone(),
                None,
                NumericFeatures::default(),
                OpaquePayload::default(),
                Vec::new(),
                Some(EventDetail::ExpectationTransition {
                    expectation_id: exp_id,
                    action_id: Some(receipt.action_id.clone()),
                    h_act: receipt.h_act.clone(),
                    from_state: Some(exp.state),
                    to_state,
                }),
            );
            append_and_apply(core, done_rec)?;
            let mut unblocked = flush_deferred_dependents(core, exp_id, now, doors)?;
            effects.append(&mut unblocked);
        }
        ReceiptOutcome::Failed | ReceiptOutcome::Unknown if is_preempted => {
            // Preempted or core-cancelled: clean stop, zero retries, zero reliability/learning penalty.
            let cancel_rec = EventRecord::derived_from(
                stored_receipt,
                EventKind::ExpectationChanged,
                now,
                ActorId::new("core"),
                stored_receipt.place.clone(),
                stored_receipt.audience.clone(),
                None,
                NumericFeatures::default(),
                OpaquePayload::default(),
                Vec::new(),
                Some(EventDetail::ExpectationTransition {
                    expectation_id: exp_id,
                    action_id: Some(receipt.action_id.clone()),
                    h_act: receipt.h_act.clone(),
                    from_state: Some(exp.state),
                    to_state: ExpectationLifecycleState::Cancelled,
                }),
            );
            append_and_apply(core, cancel_rec)?;
        }
        ReceiptOutcome::Failed | ReceiptOutcome::Unknown => {
            if exp.state != ExpectationLifecycleState::PendingApproval
                && exp.retry_count < DEFAULT_PROMISE_RETRY_MAX
            {
                let next_count = exp.retry_count + 1;
                let delay_ms = DEFAULT_PROMISE_RETRY_BASE_MS
                    .saturating_mul(1u64 << (next_count - 1).min(20));
                let next_retry_at_ms = now.saturating_add_ms(delay_ms);
                let retry_rec = EventRecord::derived_from(
                    stored_receipt,
                    EventKind::ExpectationChanged,
                    now,
                    ActorId::new("core"),
                    stored_receipt.place.clone(),
                    stored_receipt.audience.clone(),
                    None,
                    NumericFeatures::default(),
                    OpaquePayload::default(),
                    Vec::new(),
                    Some(EventDetail::ExpectationRetryScheduled {
                        expectation_id: exp_id,
                        action_id: receipt.action_id.clone(),
                        retry_count: next_count,
                        next_retry_at_ms,
                    }),
                );
                append_and_apply(core, retry_rec)?;
            } else {
                let fail_rec = EventRecord::derived_from(
                    stored_receipt,
                    EventKind::ExpectationChanged,
                    now,
                    ActorId::new("core"),
                    stored_receipt.place.clone(),
                    stored_receipt.audience.clone(),
                    None,
                    NumericFeatures::default(),
                    OpaquePayload::default(),
                    Vec::new(),
                    Some(EventDetail::ExpectationTransition {
                        expectation_id: exp_id,
                        action_id: Some(receipt.action_id.clone()),
                        h_act: receipt.h_act.clone(),
                        from_state: Some(exp.state),
                        to_state: ExpectationLifecycleState::Failed,
                    }),
                );
                append_and_apply(core, fail_rec)?;
                let mut wake_effects = wake_reasoning_for_expectation(core, &exp, now, doors)?;
                effects.append(&mut wake_effects);
            }
        }
        ReceiptOutcome::Refused => {
            let fail_rec = EventRecord::derived_from(
                stored_receipt,
                EventKind::ExpectationChanged,
                now,
                ActorId::new("core"),
                stored_receipt.place.clone(),
                stored_receipt.audience.clone(),
                None,
                NumericFeatures::default(),
                OpaquePayload::default(),
                Vec::new(),
                Some(EventDetail::ExpectationTransition {
                    expectation_id: exp_id,
                    action_id: Some(receipt.action_id.clone()),
                    h_act: receipt.h_act.clone(),
                    from_state: Some(exp.state),
                    to_state: ExpectationLifecycleState::Failed,
                }),
            );
            append_and_apply(core, fail_rec)?;
            let mut wake_effects = wake_reasoning_for_expectation(core, &exp, now, doors)?;
            effects.append(&mut wake_effects);
        }
    }

    Ok(effects)
}


fn handle_due_expectations<S: Store>(
    core: &mut Core<S>,
    tick_rec: &EventRecord,
    now: Time,
    doors: &mut dyn Doors,
) -> Result<Vec<Effect>, CoreError> {
    let mut effects = Vec::new();
    let due_items: Vec<ExpectationRecord> = core
        .state()
        .expectation_book
        .expectations
        .values()
        .filter(|e| {
            e.is_active()
                && e.state != ExpectationLifecycleState::PendingApproval
                && e.next_timer_ms().is_some_and(|t| t <= now)
        })
        .cloned()
        .collect();

    for exp in due_items {
        let place = exp
            .places
            .first()
            .cloned()
            .unwrap_or_else(|| tick_rec.place.clone());
        match exp.kind {
            ExpectKind::Promise => {
                if core.state().expectation_book.has_open_dependency(&exp) {
                    continue;
                }
                if exp.next_retry_at_ms.is_some_and(|t| t <= now) {
                    if let Some(mut_exp) = core
                        .state_mut()
                        .expectation_book
                        .expectations
                        .get_mut(&exp.expectation_id)
                    {
                        mut_exp.next_retry_at_ms = None;
                    }
                    if let Some(mut retry_act) = exp.retry_action.clone() {
                        let action_seq = core.state().next_action_seq.max(1);
                        retry_act.action_id = ActionId::new(format!("act-{action_seq}"));
                        retry_act.timestamp_ms = now;
                        let mut dispatched = dispatch_action_and_open_predictions(
                            core,
                            tick_rec,
                            retry_act,
                            &[],
                            &exp.people,
                            Some(exp.expectation_id),
                            &[],
                            &[],
                            now,
                            doors,
                        )?;
                        effects.append(&mut dispatched);
                    } else {
                        let mut wake_effects =
                            wake_reasoning_for_expectation(core, &exp, now, doors)?;
                        effects.append(&mut wake_effects);
                    }
                } else if !exp.due_woken && exp.due_ms <= now {
                    if let Some(mut_exp) = core
                        .state_mut()
                        .expectation_book
                        .expectations
                        .get_mut(&exp.expectation_id)
                    {
                        mut_exp.due_woken = true;
                        mut_exp.is_overdue = true;
                    }
                    let mut wake_effects = wake_reasoning_for_expectation(core, &exp, now, doors)?;
                    effects.append(&mut wake_effects);
                } else if exp.expiry_ms <= now {
                    let expire_rec = EventRecord::derived_from(
                        tick_rec,
                        EventKind::ExpectationChanged,
                        now,
                        ActorId::new("core"),
                        place,
                        exp.audience.clone(),
                        None,
                        NumericFeatures::default(),
                        OpaquePayload::default(),
                        Vec::new(),
                        Some(EventDetail::ExpectationTransition {
                            expectation_id: exp.expectation_id,
                            action_id: None,
                            h_act: None,
                            from_state: Some(exp.state),
                            to_state: ExpectationLifecycleState::Expired,
                        }),
                    );
                    append_and_apply(core, expire_rec)?;
                }
            }
            ExpectKind::Monitor => {
                if let Some(rec_spec) = &exp.recurrence {
                    let (next_due_ms, slots) =
                        advance_recurrence_schedule(&rec_spec.schedule, exp.due_ms, now);
                    if slots == 0 {
                        continue;
                    }
                    let should_fire = match rec_spec.catchup {
                        CatchupPolicy::CoalesceOne => true,
                        CatchupPolicy::SkipToNext => slots == 1,
                    };
                    let skipped_slots = if should_fire {
                        slots.saturating_sub(1)
                    } else {
                        slots
                    };
                    let fire_rec = EventRecord::derived_from(
                        tick_rec,
                        EventKind::ExpectationChanged,
                        now,
                        ActorId::new("core"),
                        place,
                        exp.audience.clone(),
                        None,
                        NumericFeatures::default(),
                        OpaquePayload::default(),
                        Vec::new(),
                        Some(EventDetail::MonitorFired {
                            expectation_id: exp.expectation_id,
                            fired_at_ms: now,
                            next_due_ms,
                            coalesced: should_fire && slots > 1,
                            skipped_slots,
                        }),
                    );
                    append_and_apply(core, fire_rec)?;
                    if should_fire {
                        let mut wake_effects =
                            wake_reasoning_for_expectation(core, &exp, now, doors)?;
                        effects.append(&mut wake_effects);
                    }
                }
            }
            ExpectKind::Effect => {
                let declared = exp.effect_spec.as_ref().is_some_and(|s| s.declared_sensor);
                let to_state = if declared {
                    ExpectationLifecycleState::Failed
                } else {
                    ExpectationLifecycleState::Expired
                };
                let tr = EventRecord::derived_from(
                    tick_rec,
                    EventKind::ExpectationChanged,
                    now,
                    ActorId::new("core"),
                    place,
                    exp.audience.clone(),
                    None,
                    NumericFeatures::default(),
                    OpaquePayload::default(),
                    Vec::new(),
                    Some(EventDetail::ExpectationTransition {
                        expectation_id: exp.expectation_id,
                        action_id: exp.last_action_id.clone(),
                        h_act: None,
                        from_state: Some(exp.state),
                        to_state,
                    }),
                );
                append_and_apply(core, tr)?;
                if declared {
                    let mut wake_effects = wake_reasoning_for_expectation(core, &exp, now, doors)?;
                    effects.append(&mut wake_effects);
                }
            }
            ExpectKind::Response => {
                if let Some(rs) = &exp.response_spec
                    && rs.is_judged_silence
                {
                    let sj = EventRecord::derived_from(
                        tick_rec,
                        EventKind::ExpectationChanged,
                        now,
                        ActorId::new("core"),
                        place.clone(),
                        exp.audience.clone(),
                        None,
                        NumericFeatures::default(),
                        OpaquePayload::default(),
                        Vec::new(),
                        Some(EventDetail::SilenceJudged {
                            expectation_id: exp.expectation_id,
                            target_event_id: exp.target_event_id,
                            missed: true,
                            reason: "unanswered_timeout".to_owned(),
                            propensity: rs.silence_sample_propensity.unwrap_or(1.0),
                        }),
                    );
                    let sj_stored = append_and_apply(core, sj)?;
                    let place_c = sj_stored.place.clone();
                    let source_taint = sj_stored.taint;
                    record_place_adapt_feedback(
                        core,
                        Some(&sj_stored),
                        &place_c,
                        GateArm::Silent,
                        -0.5,
                        true,
                        source_taint,
                        now,
                    )?;
                    let tr = EventRecord::derived_from(
                        tick_rec,
                        EventKind::ExpectationChanged,
                        now,
                        ActorId::new("core"),
                        place,
                        exp.audience.clone(),
                        None,
                        NumericFeatures::default(),
                        OpaquePayload::default(),
                        Vec::new(),
                        Some(EventDetail::ExpectationTransition {
                            expectation_id: exp.expectation_id,
                            action_id: None,
                            h_act: None,
                            from_state: Some(exp.state),
                            to_state: ExpectationLifecycleState::Failed,
                        }),
                    );
                    append_and_apply(core, tr)?;
                } else {
                    let tr = EventRecord::derived_from(
                        tick_rec,
                        EventKind::ExpectationChanged,
                        now,
                        ActorId::new("core"),
                        place,
                        exp.audience.clone(),
                        None,
                        NumericFeatures::default(),
                        OpaquePayload::default(),
                        Vec::new(),
                        Some(EventDetail::ExpectationTransition {
                            expectation_id: exp.expectation_id,
                            action_id: exp.last_action_id.clone(),
                            h_act: None,
                            from_state: Some(exp.state),
                            to_state: ExpectationLifecycleState::Expired,
                        }),
                    );
                    append_and_apply(core, tr)?;
                    if let Some(parent_id) = exp.linked_expectation_id
                        && let Some(parent) = core
                            .state()
                            .expectation_book
                            .expectations
                            .get(&parent_id)
                            .cloned()
                        && parent.is_active()
                    {
                        let mut wake_effects =
                            wake_reasoning_for_expectation(core, &parent, now, doors)?;
                        effects.append(&mut wake_effects);
                    }
                }
            }
            ExpectKind::Goal => {
                let tr = EventRecord::derived_from(
                    tick_rec,
                    EventKind::ExpectationChanged,
                    now,
                    ActorId::new("core"),
                    place,
                    exp.audience.clone(),
                    None,
                    NumericFeatures::default(),
                    OpaquePayload::default(),
                    Vec::new(),
                    Some(EventDetail::ExpectationTransition {
                        expectation_id: exp.expectation_id,
                        action_id: None,
                        h_act: None,
                        from_state: Some(exp.state),
                        to_state: ExpectationLifecycleState::Expired,
                    }),
                );
                append_and_apply(core, tr)?;
            }
            ExpectKind::Wait | ExpectKind::PendingApproval => {}
        }
    }

    Ok(effects)
}

fn process_judged_silences_on_percept<S: Store>(
    core: &mut Core<S>,
    stored: &EventRecord,
    percept: &monoagens_protocol::Percept,
    now: Time,
    classify_effects: &mut Vec<Effect>,
) -> Result<(), CoreError> {
    let silence_ids: Vec<EventId> = core
        .state()
        .expectation_book
        .expectations
        .values()
        .filter(|e| {
            e.kind == ExpectKind::Response
                && e.is_active()
                && e.response_spec
                    .as_ref()
                    .is_some_and(|s| s.is_judged_silence)
                && e.places.contains(&stored.place)
                && stored.timestamp_ms <= e.expiry_ms
        })
        .map(|e| e.expectation_id)
        .collect();

    for sid in silence_ids {
        let Some(exp) = core
            .state()
            .expectation_book
            .expectations
            .get(&sid)
            .cloned()
        else {
            continue;
        };
        let Some(spec) = &exp.response_spec else {
            continue;
        };
        let Some(sender) = &spec.silence_sender else {
            continue;
        };
        let propensity = spec.silence_sample_propensity.unwrap_or(1.0);

        if stored.actor != core.state().soul_actor_id && &stored.actor != sender {
            let addressed_sender = percept.reply_to == Some(exp.target_event_id)
                || percept.addressees.contains(sender)
                || percept.named.contains(sender);
            if addressed_sender {
                let sj = EventRecord::derived_from(
                    stored,
                    EventKind::ExpectationChanged,
                    now,
                    ActorId::new("core"),
                    stored.place.clone(),
                    exp.audience.clone(),
                    None,
                    NumericFeatures::default(),
                    OpaquePayload::default(),
                    Vec::new(),
                    Some(EventDetail::SilenceJudged {
                        expectation_id: sid,
                        target_event_id: exp.target_event_id,
                        missed: false,
                        reason: "answered_by_participant".to_owned(),
                        propensity,
                    }),
                );
                let sj_stored = append_and_apply(core, sj)?;
                let place_c = sj_stored.place.clone();
                let source_taint = sj_stored.taint;
                record_place_adapt_feedback(
                    core,
                    Some(&sj_stored),
                    &place_c,
                    GateArm::Silent,
                    0.5,
                    true,
                    source_taint,
                    now,
                )?;
                let tr = EventRecord::derived_from(
                    stored,
                    EventKind::ExpectationChanged,
                    now,
                    ActorId::new("core"),
                    stored.place.clone(),
                    exp.audience.clone(),
                    None,
                    NumericFeatures::default(),
                    OpaquePayload::default(),
                    Vec::new(),
                    Some(EventDetail::ExpectationTransition {
                        expectation_id: sid,
                        action_id: None,
                        h_act: None,
                        from_state: Some(exp.state),
                        to_state: ExpectationLifecycleState::Answered,
                    }),
                );
                append_and_apply(core, tr)?;
            }
        } else if &stored.actor == sender
            && stored.id != exp.target_event_id
            && !spec.awaiting_reask_classify
        {
            let lo = core.state().expectation_book.wake_gate.wake_gate_lo;
            let similar = max_matching_cosine(&percept.embeddings, &exp.condition_embeddings)
                .is_some_and(|s| s >= lo);
            if similar {
                match build_classify_plan(
                    core,
                    stored,
                    &exp,
                    now,
                    vec![exp.target_event_id, stored.id],
                ) {
                    Ok(plan) => {
                        if let Some(mut_exp) =
                            core.state_mut().expectation_book.expectations.get_mut(&sid)
                            && let Some(rs) = &mut mut_exp.response_spec
                        {
                            rs.awaiting_reask_classify = true;
                            rs.pending_reask_event_id = Some(stored.id);
                        }
                        core.state_mut().expectation_book.pending_classify.insert(
                            stored.id,
                            PendingClassifyKind::SilenceReask {
                                expectation_id: sid,
                            },
                        );
                        classify_effects.push(Effect::Model {
                            purpose: ModelPurpose::Classify,
                            plan,
                        });
                    }
                    Err(reason) => {
                        record_classify_refusal(core, stored, now, &reason)?;
                    }
                }
            }
        }
    }
    Ok(())
}

fn process_free_text_conditions_on_percept<S: Store>(
    core: &mut Core<S>,
    stored: &EventRecord,
    percept: &monoagens_protocol::Percept,
    now: Time,
    classify_effects: &mut Vec<Effect>,
) -> Result<(), CoreError> {
    if percept.embeddings.is_empty() || stored.actor == core.state().soul_actor_id {
        return Ok(());
    }
    if core
        .state()
        .expectation_book
        .pending_classify
        .contains_key(&stored.id)
    {
        return Ok(());
    }

    let lo = core.state().expectation_book.wake_gate.wake_gate_lo;
    let hi = core.state().expectation_book.wake_gate.wake_gate_hi;

    let candidates: Vec<ExpectationRecord> = core
        .state()
        .expectation_book
        .expectations
        .values()
        .filter(|e| {
            e.is_active()
                && !matches!(
                    e.kind,
                    ExpectKind::Response | ExpectKind::Effect | ExpectKind::Wait
                )
                && !e.condition_embeddings.is_empty()
                && stored.timestamp_ms <= e.expiry_ms
        })
        .cloned()
        .collect();

    for exp in candidates {
        // Already linked to this goal chain via reply_to/caused_by -> handled by commitment bypass.
        if exp.kind == ExpectKind::Goal && exp.goal_chain_ids.contains(&stored.id) {
            continue;
        }
        let Some(sim) = max_matching_cosine(&percept.embeddings, &exp.condition_embeddings) else {
            continue;
        };
        if sim < lo {
            continue;
        }
        if exp.kind != ExpectKind::Goal && hi.is_some_and(|h| sim >= h) {
            // Adopted upper band skips classification; transition condition expectation to Kept.
            let tr = EventRecord::derived_from(
                stored,
                EventKind::ExpectationChanged,
                now,
                ActorId::new("core"),
                stored.place.clone(),
                exp.audience.clone(),
                None,
                NumericFeatures::default(),
                OpaquePayload::default(),
                Vec::new(),
                Some(EventDetail::ExpectationTransition {
                    expectation_id: exp.expectation_id,
                    action_id: None,
                    h_act: None,
                    from_state: Some(exp.state),
                    to_state: ExpectationLifecycleState::Kept,
                }),
            );
            append_and_apply(core, tr)?;
            return Ok(());
        }

        match build_classify_plan(core, stored, &exp, now, vec![exp.expectation_id, stored.id]) {
            Ok(plan) => {
                let pending = if exp.kind == ExpectKind::Goal {
                    PendingClassifyKind::GoalAttribution {
                        expectation_id: exp.expectation_id,
                        similarity: sim,
                    }
                } else {
                    PendingClassifyKind::Condition {
                        expectation_id: exp.expectation_id,
                        similarity: sim,
                    }
                };
                core.state_mut()
                    .expectation_book
                    .pending_classify
                    .insert(stored.id, pending);
                classify_effects.push(Effect::Model {
                    purpose: ModelPurpose::Classify,
                    plan,
                });
            }
            Err(reason) => {
                record_classify_refusal(core, stored, now, &reason)?;
            }
        }
        break;
    }

    Ok(())
}

