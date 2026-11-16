//! Sans-IO cognitive loop turn execution, record-first enforcement, and crash recovery.

use std::collections::{BTreeMap, BTreeSet};

use monoagens_protocol::{
    Action, ActionId, ActorId, Audience, CURRENT_PROTOCOL_VERSION, CapabilityId, CatchupPolicy,
    ChannelId, ConsolidateOutput, DomainId, EventId, ExpectKind, OpaquePayload, OriginActor,
    PlaceId, ReceiptOutcome, TaggedEmbedding, Taint, TerminationReason, ThoughtAct, Time,
    VisibleScope,
};

use crate::beliefs::{DEFAULT_DOMAIN_ID, parse_basis_str, verify_claimed_basis};
use crate::blake3;
use crate::consolidation::{
    EpisodeState, ExtractProcedureParams, PendingSleepRun, RecordLessonParams,
    RegisterProcedureParams, compute_procedure_schema_hash, evaluate_sleep_trigger,
    join_episode_taint, remaining_unconsolidated_episodes, select_fade_candidates,
};
use crate::context::{
    AutoRecallTriggerState, CommitmentSlotItem, ContextSections, ContextSegment, ContextSlotKind,
    CoreSelfSnapshot, PrincipleSeverity, ProcedureEntry, SelectedToolSet, ToolSelectionBudget,
    calibrate_confidence_0_20, compute_report_threshold, enforce_context_budget,
    segment_from_event, segment_from_recalled, select_tools_for_context,
};
use crate::event::{
    DecisionOutcome, DoorKind, EventDetail, EventKind, EventRecord, ExpectationLifecycleState,
    LogSeq, NumericFeatures,
};
use crate::expectations::{
    DeferredDependentAct, ExpectationRecord, PendingClassifyKind, advance_recurrence_schedule,
    is_fulfilled_state, is_positive_verdict_label, map_resolve_outcome, max_matching_cosine,
};
use crate::gate::{DecisionReason, DecisionTrace, GateSignals, HabituationBook};
use crate::learn::{LearningOutcomeKind, LearningSample};
use crate::r#loop::QueueClass;
use crate::ports::{ContextPlan, DoorDecision, Doors, ModelPurpose, SnapshotCheckpoint, Store};
use crate::prior::Prior;
use crate::recall::{MemoryBasis, MemoryEntry, RecallQuery, RecalledItem, run_recall};
use crate::registry::Switches;
use crate::rng::SeededRng;
use crate::state::{
    CoreState, WakeSnapshotRecord, encode_actor_facts_from_msg, encode_body_status,
    encode_identity_payload,
};
use crate::{Core, CoreError, Effect, Input, RecoveryReport};

/// Opens a [`Core`] over `store`:
/// 1. Validates `prior`;
/// 2. Loads the latest snapshot checkpoint and verifies its `blake3` checksum (failing loudly naming it);
/// 3. Replays all log events after the snapshot cursor in [`LogSeq`] order;
/// 4. Resumes any trailing unconsumed inputs in deterministic `(timestamp_ms, LogSeq)` order
///    without calling a model or dispatching any action.
pub(crate) fn open_core<S: Store>(
    mut store: S,
    prior: Prior,
    switches: Switches,
    seed: u64,
) -> Result<Core<S>, CoreError> {
    prior.validate()?;

    let checkpoint = store.latest_checkpoint()?;
    let (mut state, snapshot_cursor) = match checkpoint {
        Some(ckpt) => {
            let actual = blake3::hash(&ckpt.bytes);
            if actual != ckpt.blake3 {
                return Err(CoreError::SnapshotChecksumMismatch {
                    snapshot_id: ckpt.snapshot_id,
                    expected: blake3::to_hex(&ckpt.blake3),
                    actual: blake3::to_hex(&actual),
                });
            }
            let mut decoded = CoreState::decode_snapshot(&ckpt.bytes)?;
            decoded.map_switches = switches.clone();
            if let Some(ref recorded) = decoded.recorded_prior.clone() {
                decoded.sync_prior(recorded, &switches);
            }
            (decoded, Some(ckpt.cursor))
        }
        None => {
            let mut st = CoreState::new();
            st.sync_prior(&Prior::default(), &switches);
            (st, None)
        }
    };

    let mut cursor = snapshot_cursor;
    let mut replayed_events = 0usize;
    loop {
        let batch = store.read_after_seq(cursor, 256)?;
        if batch.is_empty() {
            break;
        }
        for rec in batch {
            cursor = Some(rec.seq);
            state.apply_event(&rec);
            replayed_events += 1;
        }
    }

    // Step 3: resume trailing recorded inputs lacking a terminal completion marker
    // in deterministic timestamp-then-LogSeq order, never calling a model or dispatching,
    // and using only the prior recorded in the log so far (never the caller's new prior).
    let mut pending: Vec<EventRecord> = state.unconsumed_inputs.values().cloned().collect();
    pending.sort_by(|a, b| {
        a.timestamp_ms
            .cmp(&b.timestamp_ms)
            .then_with(|| a.seq.cmp(&b.seq))
    });

    let mut resumed_inputs = Vec::with_capacity(pending.len());
    for interrupted in pending {
        let marker_kind = match interrupted.kind {
            EventKind::Receipt => EventKind::Verdict,
            _ => EventKind::Decision,
        };
        let detail = match marker_kind {
            EventKind::Verdict => Some(EventDetail::Verdict {
                target_event_id: interrupted.id,
                label: "recovered_receipt_processed".to_owned(),
            }),
            _ => Some(EventDetail::Decision {
                target_event_id: interrupted.id,
                outcome: DecisionOutcome::Processed,
                salience: 0.0,
                threshold: 0.0,
                probability: 0.0,
                draw: 0.0,
                reason: Some(DecisionReason::Recovered),
                trace: None,
            }),
        };
        let mut marker = EventRecord::derived_from(
            &interrupted,
            marker_kind,
            interrupted.timestamp_ms,
            ActorId::new("core"),
            interrupted.place.clone(),
            interrupted.audience.clone(),
            None,
            NumericFeatures::default(),
            OpaquePayload::default(),
            Vec::new(),
            detail,
        );
        let marker_id = store.append(marker.clone())?;
        marker.id = marker_id;
        marker.seq = LogSeq::new(state.last_seq.as_u64().saturating_add(1));
        if let Some(persisted) = store.get(marker_id)? {
            state.apply_event(&persisted);
        } else {
            state.apply_event(&marker);
        }
        resumed_inputs.push(interrupted.id);
    }

    // Step 4 (Decision 20): record `prior_loaded` before admitting new input whenever the log
    // holds none yet or the given prior differs from the most recently recorded one.
    if state.recorded_prior.as_ref() != Some(&prior) {
        let ts = state
            .last_timestamp_ms
            .unwrap_or_else(|| Time::from_millis(0));
        let mut rec = EventRecord::new_root(
            EventKind::PriorLoaded,
            ts,
            ActorId::new("core"),
            PlaceId::new("core:prior"),
            Audience::new([ActorId::new("core")]),
            OriginActor::new(ActorId::new("core")),
            Taint::Trusted,
            NumericFeatures::default(),
            OpaquePayload::default(),
            Vec::new(),
            Some(EventDetail::PriorLoaded {
                prior: Box::new(prior.clone()),
            }),
        );
        let prior_id = store.append(rec.clone())?;
        rec.id = prior_id;
        rec.seq = LogSeq::new(state.last_seq.as_u64().saturating_add(1));
        if let Some(persisted) = store.get(prior_id)? {
            state.apply_event(&persisted);
        } else {
            state.apply_event(&rec);
        }
    } else {
        state.sync_prior(&prior, &switches);
    }

    if !state.expectation_book.expectations.is_empty() {
        state.startup_recovery_pending = true;
    }

    let report = RecoveryReport {
        snapshot_cursor,
        replayed_events,
        resumed_inputs,
    };

    Ok(Core::from_parts(
        store, prior, switches, seed, state, report,
    ))
}

impl Input {
    /// Classifies this input into the 3-class ingress priority queue (`A > B > C`).
    #[must_use]
    pub fn queue_class(&self) -> QueueClass {
        match self {
            Self::CancelAction { .. } => QueueClass::A,
            Self::Receipt(r)
                if matches!(
                    r.termination_reason,
                    Some(TerminationReason::CanceledByCore | TerminationReason::PreemptedExternal)
                ) =>
            {
                QueueClass::A
            }
            Self::Tick => QueueClass::C,
            Self::Percept(_)
            | Self::Receipt(_)
            | Self::ModelResult { .. }
            | Self::ClassifyResult { .. }
            | Self::ModelFailure { .. }
            | Self::Identity(_)
            | Self::BodyState(_)
            | Self::ActorFacts(_) => QueueClass::B,
        }
    }
}

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
                if let Some(eff_id) = core.state().expectation_book.match_self_caused_effect(
                    &stored,
                    &core.state().soul_actor_id,
                    core.prior(),
                    core.switches(),
                ) {
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
                            trace: None,
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

                    let mut yield_trace = DecisionTrace {
                        outcome: DecisionOutcome::Silent,
                        reason: DecisionReason::Yielded,
                        signals: Default::default(),
                        weights: Default::default(),
                        raw_salience: 0.0,
                        h_pre: 0.0,
                        kappa: 0.0,
                        habituation_factor: 1.0,
                        effective_salience: 0.0,
                        threshold_terms: Default::default(),
                        tau: core.prior().tau(),
                        note_band: core.prior().note_band,
                        stochastic_enabled: core.switches().stochastic_go_no_go,
                        commitment_bypass: false,
                        probability: 0.0,
                        draw: 0.0,
                    };
                    yield_trace.reason = DecisionReason::Yielded;
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
                    core.prior(),
                    core.switches(),
                    core.seed(),
                    true,
                );
                if let Some(reason_tag) = gate.trace.silence_audit_reason() {
                    let raw_conf = (gate.probability * 20.0).round().clamp(0.0, 20.0) as u8;
                    audit_withheld = Some((raw_conf, gate.probability, gate.threshold, reason_tag));
                }
                let d = EventDetail::Decision {
                    target_event_id: stored.id,
                    outcome: gate.outcome,
                    salience: gate.effective_salience,
                    threshold: gate.threshold,
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
                let prior_snapshot = core.prior().clone();
                let miss_opt = core.state_mut().expectation_book.evaluate_response_content(
                    resp_id,
                    &stored,
                    &percept.embeddings,
                    &prior_snapshot,
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
                if matches!(
                    gate.outcome,
                    DecisionOutcome::Note | DecisionOutcome::Silent
                ) && gate.reason != DecisionReason::Yielded
                    && gate.reason != DecisionReason::DoorBlocked
                    && core.switches().prediction_and_grounding
                    && stored.actor != core.state().soul_actor_id
                {
                    let propensity = if gate.outcome == DecisionOutcome::Note {
                        Some(1.0)
                    } else {
                        let rho = core.prior().silence_sample_rate.clamp(0.0, 1.0);
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
                        let medium = HabituationBook::source_channel_for(
                            percept.channel.as_ref(),
                            &percept.actor,
                        );
                        let silence_window_ms = core
                            .state()
                            .expectation_book
                            .response_latency
                            .estimate_window_ms(
                                &stored.place,
                                &medium,
                                core.prior().patience_default_ms,
                                core.prior().response_expiry_quantile,
                                core.prior(),
                                core.switches(),
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

                if gate.outcome == DecisionOutcome::Go {
                    let turn_allocated = gate.trace.signals.deg_soul >= core.prior().theta_alloc
                        || percept.addressees.contains(&core.state().soul_actor_id)
                        || gate.trace.commitment_bypass;
                    core.state_mut()
                        .turn_allocated_by_trigger
                        .insert(stored.id, turn_allocated);
                    let medium = HabituationBook::source_channel_for(
                        percept.channel.as_ref(),
                        &percept.actor,
                    );
                    let wait_ms = if core.switches().patience
                        && !turn_allocated
                        && !gate.trace.commitment_bypass
                    {
                        core.state().patience.estimate_patience_ms(
                            &stored.place,
                            &medium,
                            core.prior(),
                            core.switches(),
                        )
                    } else {
                        0
                    };

                    if wait_ms > 0 {
                        let due_ms = now.saturating_add_ms(wait_ms);
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
                ReceiptOutcome::Done => {
                    maybe_apply_learning_step(
                        core,
                        &stored,
                        stored.id,
                        LearningOutcomeCtx {
                            outcome_kind: LearningOutcomeKind::GoodAct,
                            sample_propensity: None,
                            termination_reason: receipt.termination_reason,
                            teacher_reliability: 1.0,
                        },
                        now,
                    )?;
                }
                ReceiptOutcome::Failed | ReceiptOutcome::Refused => {
                    maybe_apply_learning_step(
                        core,
                        &stored,
                        stored.id,
                        LearningOutcomeCtx {
                            outcome_kind: LearningOutcomeKind::BadAct,
                            sample_propensity: None,
                            termination_reason: receipt.termination_reason,
                            teacher_reliability: 1.0,
                        },
                        now,
                    )?;
                }
                ReceiptOutcome::Held | ReceiptOutcome::Unknown => {}
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
                                now.saturating_add_ms(core.prior().default_goal_expiry_ms)
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
                                embeddings: Vec::new(),
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
                            if !about_ids.is_empty() {
                                core.state_mut().expectation_book.pending_classify.insert(
                                    stored_thought.id,
                                    PendingClassifyKind::GistVerification {
                                        gist_id: stored_thought.id,
                                        source_ids: about_ids.clone(),
                                    },
                                );
                                let mut items = vec![stored_thought.id];
                                items.extend(about_ids.iter().copied());
                                let scope = core.state().visible_scope_for(
                                    stored_thought.audience.clone(),
                                    Some(place.clone()),
                                    now,
                                );
                                effects.push(Effect::Model {
                                    purpose: ModelPurpose::Consolidate,
                                    plan: ContextPlan {
                                        trigger_event_id: Some(stored_thought.id),
                                        place: Some(place),
                                        visible_scope: Some(scope),
                                        items,
                                        ..ContextPlan::default()
                                    },
                                });
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
                            let (prior_c, sw_c, catalog_c) = (
                                core.prior().clone(),
                                core.switches().clone(),
                                core.state().capability_catalog.clone(),
                            );
                            if let Some(tp) =
                                core.state_mut().skill_book.register_consolidated_procedure(
                                    params, &catalog_c, &prior_c, &sw_c,
                                )
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
                                taint: stored_thought.taint,
                                now_ms: now,
                            };
                            let (prior_c, sw_c) = (core.prior().clone(), core.switches().clone());
                            if let Some(lesson) = core
                                .state_mut()
                                .skill_book
                                .record_consolidated_lesson(params, &prior_c, &sw_c)
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
                                core.prior().certainty_bar,
                                core.switches().report_control,
                            );

                            if !turn_allocated
                                && core.switches().report_bar_on_thought
                                && motivation < core.prior().speaking_bar
                            {
                                record_withheld(
                                    core,
                                    &stored_thought,
                                    confidence_0_20,
                                    c_cal,
                                    core.prior().certainty_bar,
                                    "below_speaking_bar",
                                    now,
                                )?;
                                continue;
                            }

                            let has_explicit_stakes = think_val.is_some() || think_cost.is_some();
                            let existing_ar =
                                core.state().auto_recall_states.get(&request_id).cloned();
                            let should_check_prc = core.switches().report_control
                                && (!turn_allocated
                                    || has_explicit_stakes
                                    || existing_ar.is_some());

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
                                    existing_ar.unwrap_or_else(|| AutoRecallTriggerState {
                                        active: true,
                                        attempt: 0,
                                        max_attempts: core.prior().auto_recall_max,
                                        coarser_grain_allowed: false,
                                        last_reason: None,
                                    });
                                if ar.attempt < core.prior().auto_recall_max {
                                    ar.attempt = ar.attempt.saturating_add(1);
                                    ar.active = true;
                                    ar.max_attempts = core.prior().auto_recall_max;
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
                            } else if !turn_allocated
                                && core.switches().report_bar_on_thought
                                && c_cal < core.prior().certainty_bar
                            {
                                record_withheld(
                                    core,
                                    &stored_thought,
                                    confidence_0_20,
                                    c_cal,
                                    core.prior().certainty_bar,
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

fn wake_reasoning_for_event<S: Store>(
    core: &mut Core<S>,
    trigger: &EventRecord,
    now: Time,
    doors: &mut dyn Doors,
) -> Result<Vec<Effect>, CoreError> {
    wake_reasoning_for_event_with_extra(core, trigger, &[], now, doors)
}

fn wake_reasoning_for_event_with_extra<S: Store>(
    core: &mut Core<S>,
    trigger: &EventRecord,
    extra_items: &[EventId],
    now: Time,
    doors: &mut dyn Doors,
) -> Result<Vec<Effect>, CoreError> {
    let scope =
        core.state()
            .visible_scope_for(trigger.audience.clone(), Some(trigger.place.clone()), now);
    let query_embeddings = match &trigger.detail {
        Some(EventDetail::PerceptMeta { embeddings, .. }) => embeddings.clone(),
        _ => Vec::new(),
    };
    let mut wake_people: Vec<ActorId> = vec![trigger.actor.clone()];
    for m in &trigger.audience.members {
        if m != &core.state().soul_actor_id && !wake_people.contains(m) {
            wake_people.push(m.clone());
        }
    }
    let mut query = RecallQuery::new(scope.clone());
    query.query_event_id = Some(trigger.id);
    query.partition_tag = query_embeddings.first().map(|e| e.tag.clone());
    query.query_embeddings = query_embeddings;
    query.wake_place = Some(trigger.place.clone());
    query.wake_people = wake_people;
    query.limit = core.prior().n_rec_max.max(1);
    query.record_uses = true;

    let recall = run_recall(
        core.store(),
        core.state(),
        &query,
        now,
        core.prior(),
        core.switches(),
    )?;

    let mut plan = assemble_full_context_plan(
        core,
        trigger,
        trigger.id,
        &trigger.place,
        scope,
        &query.query_embeddings,
        &recall.items,
        extra_items,
        now,
    )?;

    run_context_door_and_emit(core, trigger, &mut plan, now, doors)
}

#[allow(clippy::too_many_arguments)]
fn assemble_full_context_plan<S: Store>(
    core: &mut Core<S>,
    trigger: &EventRecord,
    trigger_event_id: EventId,
    place: &PlaceId,
    scope: VisibleScope,
    query_embeddings: &[TaggedEmbedding],
    recalled_items: &[RecalledItem],
    extra_items: &[EventId],
    now: Time,
) -> Result<ContextPlan, CoreError> {
    let soul_id = core.state().soul_actor_id.clone();

    // 1. Progressive Tool RAG disclosure (Slot 2 & 3).
    let mut query_terms: Vec<String> =
        vec![place.as_str().to_owned(), trigger.actor.as_str().to_owned()];
    let raw_text = String::from_utf8_lossy(trigger.payload.as_bytes());
    for word in raw_text.split(|c: char| !c.is_alphanumeric() && c != '_' && c != '-' && c != '.') {
        let w = word.trim();
        if w.len() >= 2 {
            query_terms.push(w.to_owned());
        }
    }
    let catalog = core.state().effective_capability_catalog();
    let procs: Vec<ProcedureEntry> = core.state().procedures.values().cloned().collect();
    let tool_budget = ToolSelectionBudget::from_prior(core.prior(), catalog.len());
    let selected_tools: SelectedToolSet = select_tools_for_context(
        &query_terms,
        query_embeddings,
        &catalog,
        &procs,
        &scope,
        now,
        tool_budget,
    );

    // 2. Anchor-check cadence & optional [SELF_GUIDE] CoreSelfSnapshot.
    let interval = core.prior().anchor_check_interval_wakes;
    let anchor_check_due = core.state().anchor_check_pending_post_sleep
        || (interval > 0
            && core.state().wake_count_since_anchor_check.saturating_add(1) >= interval);
    {
        let st = core.state_mut();
        st.wake_count_since_anchor_check = st.wake_count_since_anchor_check.saturating_add(1);
        if anchor_check_due {
            st.wake_count_since_anchor_check = 0;
            st.anchor_check_pending_post_sleep = false;
        }
    }
    let auto_recall_state = core
        .state()
        .auto_recall_states
        .get(&trigger_event_id)
        .or_else(|| {
            trigger
                .caused_by
                .iter()
                .find_map(|cid| core.state().auto_recall_states.get(cid))
        })
        .cloned()
        .unwrap_or_else(|| AutoRecallTriggerState {
            active: false,
            attempt: 0,
            max_attempts: core.prior().auto_recall_max,
            coarser_grain_allowed: false,
            last_reason: None,
        });
    let self_guide_snapshot = if core.switches().self_mechanics_guide {
        Some(CoreSelfSnapshot::build(
            CURRENT_PROTOCOL_VERSION,
            core.switches(),
            &scope,
            auto_recall_state,
            core.state().recent_receipt_cues.clone(),
        ))
    } else {
        None
    };

    // 3. Derived 4-pillar self_card & promisee-aware participant source_cards (`15.1`, `15.2`).
    let self_card = core.state().build_self_card();
    let mut participants: Vec<ActorId> = vec![trigger.actor.clone()];
    participants.extend(scope.audience.members.iter().cloned());
    let source_cards = core.state().ledger.build_source_cards(
        &participants,
        &core.state().expectation_book,
        &soul_id,
        core.switches(),
        |actor| {
            core.state()
                .actor_exchanges
                .get(actor)
                .map(|tr| {
                    crate::activation::closeness_from_trace(
                        tr,
                        now,
                        core.prior().petrov_d(),
                        core.prior().closeness_prior_count,
                    )
                })
                .unwrap_or(0.0)
        },
    );

    // 4. Active commitments (promises & goals), sorted deterministically.
    let mut commitments: Vec<CommitmentSlotItem> = core
        .state()
        .expectation_book
        .expectations
        .values()
        .filter(|e| e.is_active() && matches!(e.kind, ExpectKind::Promise | ExpectKind::Goal))
        .map(|e| CommitmentSlotItem {
            expectation_id: e.expectation_id,
            kind: e.kind,
            due_ms: e.due_ms,
            expiry_ms: e.expiry_ms,
            places: e.places.clone(),
            people: e.people.clone(),
            is_fired_or_overdue: e.due_ms <= now || e.is_overdue || e.due_woken,
            estimated_bytes: 96,
        })
        .collect();
    commitments.sort_by_key(|c| (c.due_ms.as_millis(), c.expectation_id.as_u64()));

    // 5. Working set & Place tail from visible non-erased log records.
    let all_records = core.store().read_after_seq(None, usize::MAX)?;
    let recalled_set: BTreeSet<EventId> = recalled_items.iter().map(|r| r.event_id).collect();
    let trigger_causes: BTreeSet<EventId> = trigger.caused_by.iter().copied().collect();

    let mut chain_thoughts_actions: Vec<ContextSegment> = Vec::new();
    let mut obs_candidates: Vec<&EventRecord> = Vec::new();
    let mut place_tail_candidates: Vec<&EventRecord> = Vec::new();

    for rec in &all_records {
        if rec.id == trigger.id || rec.id == trigger_event_id {
            continue;
        }
        let erased = core
            .state()
            .memory_entries
            .get(&rec.id)
            .is_some_and(|m| m.erased);
        if erased || !scope.allows(&rec.audience, Some(rec.id), now) {
            continue;
        }
        let in_chain = core.state().active_chains.contains(&rec.id)
            || trigger_causes.contains(&rec.id)
            || extra_items.contains(&rec.id);
        let is_soul_step =
            rec.actor == soul_id && matches!(rec.kind, EventKind::Thought | EventKind::Action);
        if (in_chain || (is_soul_step && &rec.place == place))
            && matches!(rec.kind, EventKind::Thought | EventKind::Action)
        {
            chain_thoughts_actions.push(segment_from_event(
                ContextSlotKind::WorkingSet,
                rec,
                &soul_id,
                Some(MemoryBasis::SelfCaused),
                false,
                "working_set_soul",
            ));
        } else if in_chain && rec.kind == EventKind::Percept {
            obs_candidates.push(rec);
        }

        if &rec.place == place && !recalled_set.contains(&rec.id) {
            place_tail_candidates.push(rec);
        }
    }

    let full_obs_limit = core.prior().working_set_full_obs_limit;
    let total_obs = obs_candidates.len();
    let mut working_set = chain_thoughts_actions;
    for (idx, rec) in obs_candidates.into_iter().enumerate() {
        let is_recent = total_obs.saturating_sub(idx) <= full_obs_limit;
        working_set.push(segment_from_event(
            ContextSlotKind::WorkingSet,
            rec,
            &soul_id,
            Some(MemoryBasis::Observed),
            !is_recent,
            if is_recent {
                "working_set_obs_full"
            } else {
                "working_set_obs_ref"
            },
        ));
    }

    let tail_limit = core.prior().place_tail_limit;
    let tail_start = place_tail_candidates.len().saturating_sub(tail_limit);
    let place_tail: Vec<ContextSegment> = place_tail_candidates[tail_start..]
        .iter()
        .map(|rec| {
            let basis = core.state().memory_entries.get(&rec.id).map(|m| m.basis);
            segment_from_event(
                ContextSlotKind::PlaceTail,
                rec,
                &soul_id,
                basis,
                false,
                "place_tail",
            )
        })
        .collect();

    // 6. Recalled & Trigger segments.
    let recalled: Vec<ContextSegment> = recalled_items
        .iter()
        .map(|item| segment_from_recalled(item, &soul_id))
        .collect();
    let trigger_seg = Some(segment_from_event(
        ContextSlotKind::Trigger,
        trigger,
        &soul_id,
        core.state()
            .memory_entries
            .get(&trigger.id)
            .map(|m| m.basis),
        false,
        "trigger",
    ));

    let mut sections = ContextSections {
        slot_0_anchor: core.state().identity_anchor.clone(),
        slot_0_principles: core.state().principles.clone(),
        static_core_caps: selected_tools.static_core,
        self_guide_snapshot,
        dynamic_caps: selected_tools.dynamic_caps,
        procedures: selected_tools.procedures,
        excluded_stale_procedures: selected_tools.excluded_stale_procedures,
        self_card,
        source_cards,
        commitments,
        working_set,
        working_set_cues: core.state().recent_receipt_cues.clone(),
        recalled,
        place_tail,
        now_ms: now,
        knowledge_cutoff: core.state().knowledge_cutoff.clone(),
        anchor_check_due,
        trigger: trigger_seg,
        budget_bytes: core.prior().context_budget_bytes,
        total_estimated_bytes: 0,
    };

    let dropped_items = enforce_context_budget(&mut sections);

    // Preserve recalled item IDs minus any dropped from recalled, plus explicit extra items.
    let surviving_recalled: BTreeSet<EventId> = sections
        .recalled
        .iter()
        .filter_map(|s| s.event_id)
        .collect();
    let mut items: Vec<EventId> = recalled_items
        .iter()
        .map(|r| r.event_id)
        .filter(|id| surviving_recalled.contains(id))
        .collect();
    for &id in extra_items {
        if !items.contains(&id) {
            items.push(id);
        }
    }

    let mut visible_ids: BTreeSet<EventId> = items.iter().copied().collect();
    visible_ids.insert(trigger_event_id);
    if trigger.id.as_u64() != 0 {
        visible_ids.insert(trigger.id);
    }
    for &cid in &trigger.caused_by {
        visible_ids.insert(cid);
    }
    for seg in sections
        .working_set
        .iter()
        .chain(sections.recalled.iter())
        .chain(sections.place_tail.iter())
    {
        if let Some(eid) = seg.event_id {
            visible_ids.insert(eid);
        }
    }
    for c in &sections.commitments {
        visible_ids.insert(c.expectation_id);
    }

    let mut disclosed_capabilities: BTreeSet<CapabilityId> = sections
        .static_core_caps
        .iter()
        .chain(sections.dynamic_caps.iter())
        .map(|c| c.id.clone())
        .collect();
    disclosed_capabilities.insert(CapabilityId::new("memory.recall"));

    let turn_allocated = core.state().lookup_turn_allocated(trigger);
    core.state_mut().active_wake_snapshots.insert(
        trigger_event_id,
        WakeSnapshotRecord {
            visible_ids,
            visible_scope: scope.clone(),
            disclosed_capabilities,
            turn_allocated,
            wake_place: place.clone(),
            anchor_check_due,
        },
    );

    let mut plan = ContextPlan {
        trigger_event_id: Some(trigger_event_id),
        place: Some(place.clone()),
        visible_scope: Some(scope),
        items,
        sections,
        dropped_items,
        static_prefix_digest: [0u8; 32],
        valve_self_snapshot: None,
    };
    plan.refresh_static_prefix_digest();
    Ok(plan)
}

#[allow(clippy::too_many_arguments)]
fn execute_memory_recall_act<S: Store>(
    core: &mut Core<S>,
    action_rec: &EventRecord,
    action: &Action,
    query_event_id: Option<EventId>,
    primary_place: &PlaceId,
    people: &[ActorId],
    now: Time,
    doors: &mut dyn Doors,
) -> Result<Option<Effect>, CoreError> {
    let scope =
        core.state()
            .visible_scope_for(action.audience.clone(), Some(primary_place.clone()), now);
    let query_embeddings = if let Some(qid) = query_event_id {
        core.state()
            .memory_entries
            .get(&qid)
            .map(|m| m.embeddings.clone())
            .unwrap_or_default()
    } else {
        Vec::new()
    };

    let mut query = RecallQuery::new(scope.clone());
    query.query_event_id = query_event_id;
    query.partition_tag = query_embeddings.first().map(|e| e.tag.clone());
    query.query_embeddings = query_embeddings;
    query.wake_place = Some(primary_place.clone());
    query.wake_people = people.to_vec();
    query.limit = core.prior().n_rec_max.max(1);
    query.record_uses = true;

    let recall = run_recall(
        core.store(),
        core.state(),
        &query,
        now,
        core.prior(),
        core.switches(),
    )?;

    let done_receipt = EventRecord::derived_from(
        action_rec,
        EventKind::Receipt,
        now,
        ActorId::new("core"),
        primary_place.clone(),
        action.audience.clone(),
        None,
        NumericFeatures::default(),
        OpaquePayload::default(),
        Vec::new(),
        Some(EventDetail::Receipt {
            action_id: action.action_id.clone(),
            capability_id: action.capability_id.clone(),
            outcome: ReceiptOutcome::Done,
            termination_reason: Some(TerminationReason::Normal),
            progress: Some(1.0),
            delivered_units: Some(recall.items.len() as u64),
            h_act: action.h_act.clone(),
        }),
    );
    let stored_receipt = append_and_apply(core, done_receipt)?;

    let query_embs = query.query_embeddings.clone();
    let mut plan = assemble_full_context_plan(
        core,
        &stored_receipt,
        stored_receipt.id,
        primary_place,
        scope,
        &query_embs,
        &recall.items,
        &[],
        now,
    )?;

    let mut effects = run_context_door_and_emit(core, &stored_receipt, &mut plan, now, doors)?;
    Ok(effects.pop())
}

fn run_context_door_and_emit<S: Store>(
    core: &mut Core<S>,
    cause: &EventRecord,
    plan: &mut ContextPlan,
    now: Time,
    doors: &mut dyn Doors,
) -> Result<Vec<Effect>, CoreError> {
    let door_decision = doors.context(plan);
    plan.refresh_static_prefix_digest();
    let blocked = match door_decision {
        DoorDecision::Allow => false,
        DoorDecision::Strip { removed_ids, rule } => {
            plan.items.retain(|id| !removed_ids.contains(id));
            plan.sections
                .recalled
                .retain(|s| s.event_id.is_none_or(|id| !removed_ids.contains(&id)));
            plan.sections
                .working_set
                .retain(|s| s.event_id.is_none_or(|id| !removed_ids.contains(&id)));
            if let Some(tid) = plan.trigger_event_id
                && let Some(snap) = core.state_mut().active_wake_snapshots.get_mut(&tid)
            {
                for rid in &removed_ids {
                    snap.visible_ids.remove(rid);
                }
            }
            let dv = EventRecord::derived_from(
                cause,
                EventKind::DoorVerdict,
                now,
                ActorId::new("core"),
                cause.place.clone(),
                cause.audience.clone(),
                None,
                NumericFeatures::default(),
                OpaquePayload::default(),
                Vec::new(),
                Some(EventDetail::DoorVerdict {
                    door: DoorKind::Context,
                    target_event_id: Some(cause.id),
                    rule,
                    verdict: "strip".to_owned(),
                }),
            );
            append_and_apply(core, dv)?;
            false
        }
        DoorDecision::Hold { rule, reason } => {
            let dv = EventRecord::derived_from(
                cause,
                EventKind::DoorVerdict,
                now,
                ActorId::new("core"),
                cause.place.clone(),
                cause.audience.clone(),
                None,
                NumericFeatures::default(),
                OpaquePayload::default(),
                Vec::new(),
                Some(EventDetail::DoorVerdict {
                    door: DoorKind::Context,
                    target_event_id: Some(cause.id),
                    rule,
                    verdict: format!("hold: {reason}"),
                }),
            );
            append_and_apply(core, dv)?;
            true
        }
        DoorDecision::Refuse { rule, reason } => {
            let dv = EventRecord::derived_from(
                cause,
                EventKind::DoorVerdict,
                now,
                ActorId::new("core"),
                cause.place.clone(),
                cause.audience.clone(),
                None,
                NumericFeatures::default(),
                OpaquePayload::default(),
                Vec::new(),
                Some(EventDetail::DoorVerdict {
                    door: DoorKind::Context,
                    target_event_id: Some(cause.id),
                    rule,
                    verdict: format!("refuse: {reason}"),
                }),
            );
            append_and_apply(core, dv)?;
            true
        }
    };

    if blocked {
        let withheld = EventRecord::derived_from(
            cause,
            EventKind::Withheld,
            now,
            ActorId::new("core"),
            cause.place.clone(),
            cause.audience.clone(),
            None,
            NumericFeatures::default(),
            OpaquePayload::default(),
            Vec::new(),
            Some(EventDetail::Withheld {
                target_event_id: cause.id,
                raw_confidence_0_20: 0,
                calibrated_confidence: 0.0,
                report_threshold: 0.0,
                reason: "context_door_blocked".to_owned(),
            }),
        );
        append_and_apply(core, withheld)?;
        return Ok(Vec::new());
    }

    if !plan.items.is_empty() {
        let mut mem_used = EventRecord::derived_from(
            cause,
            EventKind::MemoryUsed,
            now,
            ActorId::new("core"),
            cause.place.clone(),
            cause.audience.clone(),
            None,
            NumericFeatures::default(),
            OpaquePayload::default(),
            Vec::new(),
            Some(EventDetail::MemoryUsed {
                used_ids: plan.items.clone(),
            }),
        );
        for &id in &plan.items {
            if !mem_used.caused_by.contains(&id) {
                mem_used.caused_by.push(id);
            }
        }
        append_and_apply(core, mem_used)?;
    }

    let wake_rec = EventRecord::derived_from(
        cause,
        EventKind::WakeStarted,
        now,
        ActorId::new("core"),
        cause.place.clone(),
        cause.audience.clone(),
        None,
        NumericFeatures::default(),
        OpaquePayload::default(),
        Vec::new(),
        None,
    );
    append_and_apply(core, wake_rec)?;

    Ok(vec![Effect::Model {
        purpose: ModelPurpose::Reason,
        plan: plan.clone(),
    }])
}

pub(crate) fn take_snapshot<S: Store>(
    core: &mut Core<S>,
    snapshot_id: impl Into<String>,
) -> Result<SnapshotCheckpoint, CoreError> {
    let bytes = core.state().encode_snapshot();
    let digest = blake3::hash(&bytes);
    let checkpoint = SnapshotCheckpoint {
        snapshot_id: snapshot_id.into(),
        cursor: core.state().last_seq,
        epoch: core.state().epoch,
        bytes,
        blake3: digest,
    };
    core.store_mut().snapshot(checkpoint.clone())?;
    Ok(checkpoint)
}

fn append_and_apply<S: Store>(
    core: &mut Core<S>,
    mut record: EventRecord,
) -> Result<EventRecord, CoreError> {
    let id = core.store_mut().append(record.clone())?;
    if let Some(persisted) = core.store().get(id)? {
        core.state_mut().apply_event(&persisted);
        Ok(persisted)
    } else {
        record.id = id;
        record.seq = LogSeq::new(core.state().last_seq.as_u64().saturating_add(1));
        core.state_mut().apply_event(&record);
        Ok(record)
    }
}

fn find_linked_expectation_for_act(
    state: &CoreState,
    request_id: EventId,
    used_ids: &[EventId],
    thought: &EventRecord,
) -> Option<EventId> {
    for &uid in used_ids {
        if let Some(exp) = state.expectation_book.expectations.get(&uid)
            && exp.is_active()
            && matches!(
                exp.kind,
                ExpectKind::Promise | ExpectKind::Goal | ExpectKind::PendingApproval
            )
        {
            return Some(uid);
        }
    }
    if let Some(exp) = state.expectation_book.expectations.get(&request_id)
        && exp.is_active()
        && matches!(
            exp.kind,
            ExpectKind::Promise | ExpectKind::Goal | ExpectKind::PendingApproval
        )
    {
        return Some(request_id);
    }
    for &cid in &thought.caused_by {
        if let Some(exp) = state.expectation_book.expectations.get(&cid)
            && exp.is_active()
            && matches!(
                exp.kind,
                ExpectKind::Promise | ExpectKind::Goal | ExpectKind::PendingApproval
            )
        {
            return Some(cid);
        }
    }
    for exp in state.expectation_book.expectations.values() {
        if exp.is_active()
            && matches!(exp.kind, ExpectKind::Promise | ExpectKind::Goal)
            && (exp.target_event_id == request_id
                || exp.target_event_id == thought.id
                || exp.origin_event_id == request_id
                || exp.goal_chain_ids.contains(&request_id)
                || used_ids.iter().any(|u| exp.goal_chain_ids.contains(u)))
        {
            return Some(exp.expectation_id);
        }
    }
    None
}

#[allow(clippy::too_many_arguments)]
fn dispatch_action_and_open_predictions<S: Store>(
    core: &mut Core<S>,
    causing_thought: &EventRecord,
    mut action: Action,
    used_ids: &[EventId],
    people: &[ActorId],
    linked_exp_id: Option<EventId>,
    act_embeddings: &[TaggedEmbedding],
    expected_reply_embeddings: &[TaggedEmbedding],
    now: Time,
    doors: &mut dyn Doors,
) -> Result<Vec<Effect>, CoreError> {
    let mut effects = Vec::new();
    let capability_id = action.capability_id.clone();
    let action_id = action.action_id.clone();

    match doors.action(&mut action) {
        DoorDecision::Allow | DoorDecision::Strip { .. } => {
            let primary_place = action
                .places
                .first()
                .cloned()
                .unwrap_or_else(|| causing_thought.place.clone());
            let action_rec = EventRecord::derived_from(
                causing_thought,
                EventKind::Action,
                now,
                ActorId::new("soul"),
                primary_place.clone(),
                action.audience.clone(),
                None,
                NumericFeatures::default(),
                action.payload.clone(),
                Vec::new(),
                Some(EventDetail::Action {
                    action_id: action.action_id.clone(),
                    capability_id: action.capability_id.clone(),
                    h_act: action.h_act.clone(),
                    correlation_id: action.correlation_id.clone(),
                }),
            );
            let stored_action = append_and_apply(core, action_rec)?;

            if let Some(eid) = linked_exp_id {
                let link_rec = EventRecord::derived_from(
                    &stored_action,
                    EventKind::ExpectationChanged,
                    now,
                    ActorId::new("core"),
                    primary_place.clone(),
                    action.audience.clone(),
                    None,
                    NumericFeatures::default(),
                    OpaquePayload::default(),
                    Vec::new(),
                    Some(EventDetail::ExpectationLinked {
                        expectation_id: eid,
                        linked_event_id: stored_action.id,
                        action_id: Some(action.action_id.clone()),
                    }),
                );
                append_and_apply(core, link_rec)?;
                if let Some(exp) = core.state_mut().expectation_book.expectations.get_mut(&eid) {
                    exp.last_action_id = Some(action.action_id.clone());
                    exp.last_capability_id = Some(action.capability_id.clone());
                    exp.retry_action = Some(action.clone());
                }
            }

            if action.capability_id.as_str() == "memory.recall" {
                let query_ev = used_ids
                    .first()
                    .copied()
                    .or_else(|| causing_thought.caused_by.first().copied());
                if let Some(eff) = execute_memory_recall_act(
                    core,
                    &stored_action,
                    &action,
                    query_ev,
                    &primary_place,
                    people,
                    now,
                    doors,
                )? {
                    effects.push(eff);
                }
            } else {
                let cap_desc = core
                    .state()
                    .capability_descriptors
                    .get(&action.capability_id)
                    .cloned();
                let body_declared_ms = cap_desc
                    .as_ref()
                    .map_or(core.prior().patience_default_ms, |d| {
                        d.typical_latency_ms.max(1)
                    });

                // Open Effect prediction (declared sensor effect or echo tracker).
                let sensed = action
                    .expected_effect
                    .clone()
                    .or_else(|| cap_desc.as_ref().and_then(|d| d.sensed_effect.clone()));
                let (declared_sensor, eff_channel, eff_window_ms, delta_pred, sigma_ch) =
                    if let Some(se) = sensed {
                        (
                            true,
                            se.channel,
                            se.window_ms.max(1),
                            Some(se.delta_pred),
                            Some(se.noise),
                        )
                    } else {
                        (
                            false,
                            ChannelId::new(action.capability_id.as_str()),
                            body_declared_ms.max(5_000),
                            None,
                            None,
                        )
                    };
                let eff_exp_id =
                    EventId::new(core.state().last_seq.as_u64().saturating_add(1).max(1));
                let eff_open = EventRecord::derived_from(
                    &stored_action,
                    EventKind::ExpectationOpened,
                    now,
                    ActorId::new("core"),
                    primary_place.clone(),
                    action.audience.clone(),
                    None,
                    NumericFeatures::default(),
                    OpaquePayload::default(),
                    Vec::new(),
                    Some(EventDetail::ExpectationRecordOpened {
                        expectation_id: eff_exp_id,
                        origin_event_id: stored_action.id,
                        target_event_id: stored_action.id,
                        kind: ExpectKind::Effect,
                        places: action.places.clone(),
                        people: people.to_vec(),
                        domain: None,
                        condition: "effect".to_owned(),
                        condition_embeddings: Vec::new(),
                        due_ms: now.saturating_add_ms(eff_window_ms),
                        expiry_ms: now.saturating_add_ms(eff_window_ms),
                        recurrence: None,
                        promise_meta: None,
                        urgency_weight: 0.0,
                        action_id: Some(action.action_id.clone()),
                        capability_id: Some(action.capability_id.clone()),
                        channel: Some(eff_channel),
                        lat_min_ms: 0,
                        lat_max_ms: eff_window_ms,
                        delta_pred,
                        sigma_channel: sigma_ch,
                        correlation_id: action.correlation_id.clone(),
                        declared_sensor,
                        act_embeddings: Vec::new(),
                        expected_reply_embeddings: Vec::new(),
                        is_judged_silence: false,
                        silence_propensity: None,
                        silence_sender: None,
                        stakes: 0.5,
                        medium: None,
                        about_ids: vec![stored_action.id],
                        accept_done_receipt: false,
                        expected_latency_ms: eff_window_ms,
                    }),
                );
                append_and_apply(core, eff_open)?;
                if let Some(eff_rec) = core
                    .state_mut()
                    .expectation_book
                    .expectations
                    .get_mut(&eff_exp_id)
                {
                    eff_rec.linked_expectation_id = linked_exp_id;
                }

                // Open Response prediction when prediction_and_grounding is enabled.
                if core.switches().prediction_and_grounding {
                    let medium = ChannelId::new(action.capability_id.as_str());
                    let resp_window_ms = core
                        .state()
                        .expectation_book
                        .response_latency
                        .estimate_window_ms(
                            &primary_place,
                            &medium,
                            body_declared_ms,
                            core.prior().response_expiry_quantile,
                            core.prior(),
                            core.switches(),
                        );
                    let accept_done_receipt = cap_desc
                        .as_ref()
                        .map_or_else(|| people.is_empty(), |d| !d.flags.communicative);
                    let resp_exp_id =
                        EventId::new(core.state().last_seq.as_u64().saturating_add(1).max(1));
                    let resp_open = EventRecord::derived_from(
                        &stored_action,
                        EventKind::ExpectationOpened,
                        now,
                        ActorId::new("core"),
                        primary_place.clone(),
                        action.audience.clone(),
                        None,
                        NumericFeatures::default(),
                        OpaquePayload::default(),
                        Vec::new(),
                        Some(EventDetail::ExpectationRecordOpened {
                            expectation_id: resp_exp_id,
                            origin_event_id: stored_action.id,
                            target_event_id: stored_action.id,
                            kind: ExpectKind::Response,
                            places: action.places.clone(),
                            people: people.to_vec(),
                            domain: None,
                            condition: "response".to_owned(),
                            condition_embeddings: Vec::new(),
                            due_ms: now.saturating_add_ms(resp_window_ms),
                            expiry_ms: now.saturating_add_ms(resp_window_ms),
                            recurrence: None,
                            promise_meta: None,
                            urgency_weight: 0.0,
                            action_id: Some(action.action_id.clone()),
                            capability_id: Some(action.capability_id.clone()),
                            channel: None,
                            lat_min_ms: 0,
                            lat_max_ms: 0,
                            delta_pred: None,
                            sigma_channel: None,
                            correlation_id: action.correlation_id.clone(),
                            declared_sensor: false,
                            act_embeddings: act_embeddings.to_vec(),
                            expected_reply_embeddings: expected_reply_embeddings.to_vec(),
                            is_judged_silence: false,
                            silence_propensity: None,
                            silence_sender: None,
                            stakes: if linked_exp_id.is_some() { 1.0 } else { 0.5 },
                            medium: Some(medium),
                            about_ids: vec![stored_action.id],
                            accept_done_receipt,
                            expected_latency_ms: body_declared_ms,
                        }),
                    );
                    append_and_apply(core, resp_open)?;
                    if let Some(resp_rec) = core
                        .state_mut()
                        .expectation_book
                        .expectations
                        .get_mut(&resp_exp_id)
                    {
                        resp_rec.linked_expectation_id = linked_exp_id;
                    }
                }

                effects.push(Effect::Dispatch(action));
            }
        }
        DoorDecision::Hold { rule, reason } => {
            let dv = EventRecord::derived_from(
                causing_thought,
                EventKind::DoorVerdict,
                now,
                ActorId::new("core"),
                causing_thought.place.clone(),
                causing_thought.audience.clone(),
                None,
                NumericFeatures::default(),
                OpaquePayload::default(),
                Vec::new(),
                Some(EventDetail::DoorVerdict {
                    door: DoorKind::Action,
                    target_event_id: Some(causing_thought.id),
                    rule,
                    verdict: format!("hold: {reason}"),
                }),
            );
            append_and_apply(core, dv)?;
            let held_receipt = EventRecord::derived_from(
                causing_thought,
                EventKind::Receipt,
                now,
                ActorId::new("core"),
                causing_thought.place.clone(),
                causing_thought.audience.clone(),
                None,
                NumericFeatures::default(),
                OpaquePayload::default(),
                Vec::new(),
                Some(EventDetail::Receipt {
                    action_id: action_id.clone(),
                    capability_id,
                    outcome: ReceiptOutcome::Held,
                    termination_reason: None,
                    progress: None,
                    delivered_units: None,
                    h_act: action.h_act.clone(),
                }),
            );
            append_and_apply(core, held_receipt)?;
            if let Some(eid) = linked_exp_id {
                let pa_rec = EventRecord::derived_from(
                    causing_thought,
                    EventKind::ExpectationChanged,
                    now,
                    ActorId::new("core"),
                    causing_thought.place.clone(),
                    causing_thought.audience.clone(),
                    None,
                    NumericFeatures::default(),
                    OpaquePayload::default(),
                    Vec::new(),
                    Some(EventDetail::ExpectationTransition {
                        expectation_id: eid,
                        action_id: Some(action_id),
                        h_act: action.h_act,
                        from_state: Some(ExpectationLifecycleState::Open),
                        to_state: ExpectationLifecycleState::PendingApproval,
                    }),
                );
                append_and_apply(core, pa_rec)?;
            }
        }
        DoorDecision::Refuse { rule, reason } => {
            let dv = EventRecord::derived_from(
                causing_thought,
                EventKind::DoorVerdict,
                now,
                ActorId::new("core"),
                causing_thought.place.clone(),
                causing_thought.audience.clone(),
                None,
                NumericFeatures::default(),
                OpaquePayload::default(),
                Vec::new(),
                Some(EventDetail::DoorVerdict {
                    door: DoorKind::Action,
                    target_event_id: Some(causing_thought.id),
                    rule,
                    verdict: format!("refuse: {reason}"),
                }),
            );
            append_and_apply(core, dv)?;
            let refused_receipt = EventRecord::derived_from(
                causing_thought,
                EventKind::Receipt,
                now,
                ActorId::new("core"),
                causing_thought.place.clone(),
                causing_thought.audience.clone(),
                None,
                NumericFeatures::default(),
                OpaquePayload::default(),
                Vec::new(),
                Some(EventDetail::Receipt {
                    action_id,
                    capability_id,
                    outcome: ReceiptOutcome::Refused,
                    termination_reason: Some(TerminationReason::Normal),
                    progress: None,
                    delivered_units: None,
                    h_act: action.h_act,
                }),
            );
            append_and_apply(core, refused_receipt)?;
        }
    }
    Ok(effects)
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
                && exp.retry_count < core.prior().promise_retry_max
            {
                let next_count = exp.retry_count + 1;
                let delay_ms = core
                    .prior()
                    .promise_retry_base_ms
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

pub(crate) fn run_startup_recovery<S: Store>(
    core: &mut Core<S>,
    now: Time,
    doors: &mut dyn Doors,
) -> Result<Vec<Effect>, CoreError> {
    core.state_mut().startup_recovery_pending = false;
    let mut effects = Vec::new();

    let snapshot: Vec<ExpectationRecord> = core
        .state()
        .expectation_book
        .expectations
        .values()
        .filter(|e| e.is_active() && e.due_ms <= now)
        .cloned()
        .collect();

    // 1. Overdue single-shot promises: wake at most once per place.
    let mut woken_promise_places: BTreeSet<PlaceId> = BTreeSet::new();
    for exp in &snapshot {
        if exp.kind != ExpectKind::Promise || exp.state != ExpectationLifecycleState::Open {
            continue;
        }
        if core.state().expectation_book.has_open_dependency(exp) {
            continue;
        }
        let place = exp
            .places
            .first()
            .cloned()
            .unwrap_or_else(|| PlaceId::new("core:promise"));
        if let Some(mut_exp) = core
            .state_mut()
            .expectation_book
            .expectations
            .get_mut(&exp.expectation_id)
        {
            mut_exp.is_overdue = true;
            mut_exp.due_woken = true;
        }
        core.state_mut().sync_expectation_entry(exp.expectation_id);

        if woken_promise_places.insert(place) {
            let mut wake_effects = wake_reasoning_for_expectation(core, exp, now, doors)?;
            effects.append(&mut wake_effects);
        }
    }

    // 2. Overdue RecurrenceSpec monitors: coalesce into at most one catch-up per place (or skip_to_next).
    let mut fired_monitor_places: BTreeSet<PlaceId> = BTreeSet::new();
    for exp in &snapshot {
        if exp.kind != ExpectKind::Monitor || exp.state != ExpectationLifecycleState::Open {
            continue;
        }
        let Some(rec_spec) = &exp.recurrence else {
            continue;
        };
        let (next_due_ms, slots) = advance_recurrence_schedule(&rec_spec.schedule, exp.due_ms, now);
        if slots == 0 {
            continue;
        }
        let place = exp
            .places
            .first()
            .cloned()
            .unwrap_or_else(|| PlaceId::new("core:monitor"));
        let should_fire = match rec_spec.catchup {
            CatchupPolicy::CoalesceOne => fired_monitor_places.insert(place.clone()),
            CatchupPolicy::SkipToNext => false,
        };
        let skipped_slots = if should_fire {
            slots.saturating_sub(1)
        } else {
            slots
        };

        let fire_rec = EventRecord::new_root(
            EventKind::ExpectationChanged,
            now,
            ActorId::new("core"),
            place,
            exp.audience.clone(),
            OriginActor::new(core.state().soul_actor_id.clone()),
            Taint::Trusted,
            NumericFeatures::default(),
            OpaquePayload::default(),
            Vec::new(),
            Some(EventDetail::MonitorFired {
                expectation_id: exp.expectation_id,
                fired_at_ms: now,
                next_due_ms,
                coalesced: should_fire,
                skipped_slots,
            }),
        );
        append_and_apply(core, fire_rec)?;

        if should_fire {
            let mut wake_effects = wake_reasoning_for_expectation(core, exp, now, doors)?;
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
                    maybe_apply_learning_step(
                        core,
                        &sj_stored,
                        exp.target_event_id,
                        LearningOutcomeCtx {
                            outcome_kind: LearningOutcomeKind::MissedSilence,
                            sample_propensity: rs.silence_sample_propensity,
                            termination_reason: None,
                            teacher_reliability: 1.0,
                        },
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
                maybe_apply_learning_step(
                    core,
                    &sj_stored,
                    exp.target_event_id,
                    LearningOutcomeCtx {
                        outcome_kind: LearningOutcomeKind::AnsweredSilence,
                        sample_propensity: Some(propensity),
                        termination_reason: None,
                        teacher_reliability: 1.0,
                    },
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
                if let Some(mut_exp) = core.state_mut().expectation_book.expectations.get_mut(&sid)
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
                let scope = core.state().visible_scope_for(
                    stored.audience.clone(),
                    Some(stored.place.clone()),
                    now,
                );
                classify_effects.push(Effect::Model {
                    purpose: ModelPurpose::Classify,
                    plan: ContextPlan {
                        trigger_event_id: Some(stored.id),
                        place: Some(stored.place.clone()),
                        visible_scope: Some(scope),
                        items: vec![exp.target_event_id, stored.id],
                        ..ContextPlan::default()
                    },
                });
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
        let scope = core.state().visible_scope_for(
            stored.audience.clone(),
            Some(stored.place.clone()),
            now,
        );
        classify_effects.push(Effect::Model {
            purpose: ModelPurpose::Classify,
            plan: ContextPlan {
                trigger_event_id: Some(stored.id),
                place: Some(stored.place.clone()),
                visible_scope: Some(scope),
                items: vec![exp.expectation_id, stored.id],
                ..ContextPlan::default()
            },
        });
        break;
    }

    Ok(())
}

fn handle_classify_result<S: Store>(
    core: &mut Core<S>,
    request_id: EventId,
    label: &str,
    now: Time,
    doors: &mut dyn Doors,
) -> Result<Vec<Effect>, CoreError> {
    let Some(pending) = core
        .state_mut()
        .expectation_book
        .pending_classify
        .remove(&request_id)
    else {
        return Ok(Vec::new());
    };
    let positive = is_positive_verdict_label(label);
    let mut effects = Vec::new();
    let trigger_ev = core.store().get(request_id)?;

    match pending {
        PendingClassifyKind::Condition {
            expectation_id,
            similarity,
        } => {
            let verdict_rec = match &trigger_ev {
                Some(ev) => EventRecord::derived_from(
                    ev,
                    EventKind::Verdict,
                    now,
                    ActorId::new("core"),
                    ev.place.clone(),
                    ev.audience.clone(),
                    None,
                    NumericFeatures::default(),
                    OpaquePayload::default(),
                    Vec::new(),
                    Some(EventDetail::ConditionVerdict {
                        expectation_id,
                        target_event_id: request_id,
                        similarity,
                        met: positive,
                        label: label.to_owned(),
                    }),
                ),
                None => EventRecord::new_root(
                    EventKind::Verdict,
                    now,
                    ActorId::new("core"),
                    PlaceId::new("core:verdict"),
                    Audience::new([core.state().soul_actor_id.clone()]),
                    OriginActor::new(core.state().soul_actor_id.clone()),
                    Taint::Trusted,
                    NumericFeatures::default(),
                    OpaquePayload::default(),
                    Vec::new(),
                    Some(EventDetail::ConditionVerdict {
                        expectation_id,
                        target_event_id: request_id,
                        similarity,
                        met: positive,
                        label: label.to_owned(),
                    }),
                ),
            };
            append_and_apply(core, verdict_rec)?;

            if positive
                && let Some(exp) = core
                    .state()
                    .expectation_book
                    .expectations
                    .get(&expectation_id)
                    .cloned()
                && exp.is_active()
            {
                let to_state = if exp.kind == ExpectKind::Goal {
                    ExpectationLifecycleState::Reached
                } else if exp.kind == ExpectKind::Promise && now > exp.due_ms {
                    ExpectationLifecycleState::KeptLate
                } else {
                    ExpectationLifecycleState::Kept
                };
                let tr = EventRecord::new_root(
                    EventKind::ExpectationChanged,
                    now,
                    ActorId::new("core"),
                    exp.places
                        .first()
                        .cloned()
                        .unwrap_or_else(|| PlaceId::new("core:expectation")),
                    exp.audience.clone(),
                    OriginActor::new(core.state().soul_actor_id.clone()),
                    Taint::Trusted,
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
                append_and_apply(core, tr)?;
                if to_state == ExpectationLifecycleState::Reached {
                    maybe_extract_procedure_from_goal(core, &exp, &tr_cause_fallback(&exp), now)?;
                }
                if let Some(ev) = &trigger_ev {
                    let mut wake_effects = wake_reasoning_for_event_with_extra(
                        core,
                        ev,
                        &[expectation_id],
                        now,
                        doors,
                    )?;
                    effects.append(&mut wake_effects);
                } else {
                    let mut wake_effects = wake_reasoning_for_expectation(core, &exp, now, doors)?;
                    effects.append(&mut wake_effects);
                }
            }
        }
        PendingClassifyKind::SilenceReask { expectation_id } => {
            if let Some(exp) = core
                .state_mut()
                .expectation_book
                .expectations
                .get_mut(&expectation_id)
                && let Some(rs) = &mut exp.response_spec
            {
                rs.awaiting_reask_classify = false;
                rs.pending_reask_event_id = None;
            }
            if positive
                && let Some(exp) = core
                    .state()
                    .expectation_book
                    .expectations
                    .get(&expectation_id)
                    .cloned()
                && exp.is_active()
            {
                let prop = exp
                    .response_spec
                    .as_ref()
                    .and_then(|s| s.silence_sample_propensity)
                    .unwrap_or(1.0);
                let place = exp
                    .places
                    .first()
                    .cloned()
                    .unwrap_or_else(|| PlaceId::new("core:silence"));
                let sj = EventRecord::new_root(
                    EventKind::ExpectationChanged,
                    now,
                    ActorId::new("core"),
                    place.clone(),
                    exp.audience.clone(),
                    OriginActor::new(core.state().soul_actor_id.clone()),
                    Taint::Trusted,
                    NumericFeatures::default(),
                    OpaquePayload::default(),
                    Vec::new(),
                    Some(EventDetail::SilenceJudged {
                        expectation_id,
                        target_event_id: exp.target_event_id,
                        missed: true,
                        reason: "reask_confirmed".to_owned(),
                        propensity: prop,
                    }),
                );
                let sj_stored = append_and_apply(core, sj)?;
                maybe_apply_learning_step(
                    core,
                    &sj_stored,
                    exp.target_event_id,
                    LearningOutcomeCtx {
                        outcome_kind: LearningOutcomeKind::MissedSilence,
                        sample_propensity: Some(prop),
                        termination_reason: None,
                        teacher_reliability: 1.0,
                    },
                    now,
                )?;
                let tr = EventRecord::new_root(
                    EventKind::ExpectationChanged,
                    now,
                    ActorId::new("core"),
                    place,
                    exp.audience.clone(),
                    OriginActor::new(core.state().soul_actor_id.clone()),
                    Taint::Trusted,
                    NumericFeatures::default(),
                    OpaquePayload::default(),
                    Vec::new(),
                    Some(EventDetail::ExpectationTransition {
                        expectation_id,
                        action_id: None,
                        h_act: None,
                        from_state: Some(exp.state),
                        to_state: ExpectationLifecycleState::Failed,
                    }),
                );
                append_and_apply(core, tr)?;
            }
        }
        PendingClassifyKind::GoalAttribution {
            expectation_id,
            similarity,
        } => {
            let verdict_rec = match &trigger_ev {
                Some(ev) => EventRecord::derived_from(
                    ev,
                    EventKind::Verdict,
                    now,
                    ActorId::new("core"),
                    ev.place.clone(),
                    ev.audience.clone(),
                    None,
                    NumericFeatures::default(),
                    OpaquePayload::default(),
                    Vec::new(),
                    Some(EventDetail::ConditionVerdict {
                        expectation_id,
                        target_event_id: request_id,
                        similarity,
                        met: positive,
                        label: label.to_owned(),
                    }),
                ),
                None => EventRecord::new_root(
                    EventKind::Verdict,
                    now,
                    ActorId::new("core"),
                    PlaceId::new("core:verdict"),
                    Audience::new([core.state().soul_actor_id.clone()]),
                    OriginActor::new(core.state().soul_actor_id.clone()),
                    Taint::Trusted,
                    NumericFeatures::default(),
                    OpaquePayload::default(),
                    Vec::new(),
                    Some(EventDetail::ConditionVerdict {
                        expectation_id,
                        target_event_id: request_id,
                        similarity,
                        met: positive,
                        label: label.to_owned(),
                    }),
                ),
            };
            append_and_apply(core, verdict_rec)?;

            if positive
                && let Some(exp) = core
                    .state()
                    .expectation_book
                    .expectations
                    .get(&expectation_id)
                    .cloned()
                && exp.is_active()
            {
                let link_rec = EventRecord::new_root(
                    EventKind::ExpectationChanged,
                    now,
                    ActorId::new("core"),
                    exp.places
                        .first()
                        .cloned()
                        .unwrap_or_else(|| PlaceId::new("core:goal")),
                    exp.audience.clone(),
                    OriginActor::new(core.state().soul_actor_id.clone()),
                    Taint::Trusted,
                    NumericFeatures::default(),
                    OpaquePayload::default(),
                    Vec::new(),
                    Some(EventDetail::ExpectationLinked {
                        expectation_id,
                        linked_event_id: request_id,
                        action_id: None,
                    }),
                );
                append_and_apply(core, link_rec)?;

                // Appraise through the gate WITHOUT bypassing it (R-07 Step 3).
                if let Some(ev) = &trigger_ev {
                    let gate = core.state().appraise_percept_record(
                        ev,
                        core.prior(),
                        core.switches(),
                        core.seed(),
                        true,
                    );
                    if gate.outcome == DecisionOutcome::Go {
                        let mut wake_effects = wake_reasoning_for_event_with_extra(
                            core,
                            ev,
                            &[expectation_id],
                            now,
                            doors,
                        )?;
                        effects.append(&mut wake_effects);
                    }
                }
            }
        }
        PendingClassifyKind::GistVerification {
            gist_id,
            source_ids,
        } => {
            let norm = label.trim().to_ascii_lowercase();
            let faithful = matches!(
                norm.as_str(),
                "faithful"
                    | "verified"
                    | "supported"
                    | "consistent"
                    | "valid"
                    | "ok"
                    | "true"
                    | "yes"
                    | "confirmed"
            );
            let verdict_rec = EventRecord::new_root(
                EventKind::Verdict,
                now,
                ActorId::new("core"),
                PlaceId::new("core:sleep"),
                Audience::new([core.state().soul_actor_id.clone()]),
                OriginActor::new(core.state().soul_actor_id.clone()),
                Taint::Trusted,
                NumericFeatures::default(),
                OpaquePayload::default(),
                Vec::new(),
                Some(EventDetail::Verdict {
                    target_event_id: gist_id,
                    label: if faithful {
                        format!("gist_faithful:{norm}")
                    } else {
                        format!("gist_conflict:{norm}")
                    },
                }),
            );
            let stored_verdict = append_and_apply(core, verdict_rec)?;
            if !faithful {
                let replacement = source_ids.first().copied().unwrap_or(stored_verdict.id);
                core.state_mut().supersede_memory(gist_id, replacement);
            }
        }
    }

    Ok(effects)
}

fn wake_reasoning_for_expectation<S: Store>(
    core: &mut Core<S>,
    exp: &ExpectationRecord,
    now: Time,
    doors: &mut dyn Doors,
) -> Result<Vec<Effect>, CoreError> {
    if let Some(target_rec) = core.store().get(exp.target_event_id)? {
        return wake_reasoning_for_event_with_extra(
            core,
            &target_rec,
            &[exp.expectation_id],
            now,
            doors,
        );
    }
    let place = exp
        .places
        .first()
        .cloned()
        .unwrap_or_else(|| PlaceId::new("core:expectation"));
    let scope = core
        .state()
        .visible_scope_for(exp.audience.clone(), Some(place.clone()), now);
    let synth = EventRecord::new_root(
        EventKind::ExpectationChanged,
        now,
        ActorId::new("core"),
        place.clone(),
        exp.audience.clone(),
        OriginActor::new(core.state().soul_actor_id.clone()),
        Taint::Trusted,
        NumericFeatures::default(),
        OpaquePayload::default(),
        Vec::new(),
        None,
    );
    let mut plan = assemble_full_context_plan(
        core,
        &synth,
        exp.expectation_id,
        &place,
        scope,
        &exp.condition_embeddings,
        &[],
        &[exp.expectation_id],
        now,
    )?;
    run_context_door_and_emit(core, &synth, &mut plan, now, doors)
}

const fn taint_rank(t: Taint) -> u8 {
    match t {
        Taint::Untrusted => 0,
        Taint::Provisional => 1,
        Taint::Trusted => 2,
    }
}

const fn join_min_taint(a: Taint, b: Taint) -> Taint {
    if taint_rank(a) <= taint_rank(b) { a } else { b }
}

fn check_citation_lock<S: Store>(
    core: &Core<S>,
    wake_snapshot: Option<&WakeSnapshotRecord>,
    turn_created_ids: &BTreeSet<EventId>,
    used_ids: &[EventId],
) -> Result<Option<EventId>, CoreError> {
    for &uid in used_ids {
        if turn_created_ids.contains(&uid) {
            continue;
        }
        if let Some(snap) = wake_snapshot {
            if !snap.visible_ids.contains(&uid) {
                return Ok(Some(uid));
            }
        } else {
            let exists = core.store().get(uid)?.is_some()
                || core.state().memory_entries.contains_key(&uid)
                || core
                    .state()
                    .expectation_book
                    .expectations
                    .contains_key(&uid);
            if !exists {
                return Ok(Some(uid));
            }
        }
    }
    Ok(None)
}

fn record_withheld<S: Store>(
    core: &mut Core<S>,
    stored_thought: &EventRecord,
    raw_confidence_0_20: u8,
    calibrated_confidence: f64,
    report_threshold: f64,
    reason: &str,
    now: Time,
) -> Result<(), CoreError> {
    let rec = EventRecord::derived_from(
        stored_thought,
        EventKind::Withheld,
        now,
        ActorId::new("core"),
        stored_thought.place.clone(),
        stored_thought.audience.clone(),
        None,
        NumericFeatures::default(),
        OpaquePayload::default(),
        Vec::new(),
        Some(EventDetail::Withheld {
            target_event_id: stored_thought.id,
            raw_confidence_0_20,
            calibrated_confidence,
            report_threshold,
            reason: reason.to_owned(),
        }),
    );
    append_and_apply(core, rec)?;
    Ok(())
}

fn record_act_refused<S: Store>(
    core: &mut Core<S>,
    stored_thought: &EventRecord,
    action: &Action,
    rule: &str,
    verdict: String,
    now: Time,
) -> Result<(), CoreError> {
    let dv = EventRecord::derived_from(
        stored_thought,
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
            rule: rule.to_owned(),
            verdict,
        }),
    );
    append_and_apply(core, dv)?;

    let refused = EventRecord::derived_from(
        stored_thought,
        EventKind::Receipt,
        now,
        ActorId::new("core"),
        stored_thought.place.clone(),
        stored_thought.audience.clone(),
        None,
        NumericFeatures::default(),
        OpaquePayload::from_bytes(rule.as_bytes()),
        Vec::new(),
        Some(EventDetail::Receipt {
            action_id: action.action_id.clone(),
            capability_id: action.capability_id.clone(),
            outcome: ReceiptOutcome::Refused,
            termination_reason: Some(TerminationReason::Normal),
            progress: None,
            delivered_units: None,
            h_act: action.h_act.clone(),
        }),
    );
    append_and_apply(core, refused)?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn record_belief_event<S: Store>(
    core: &mut Core<S>,
    claim_key: String,
    requested_basis: MemoryBasis,
    source_actor: Option<ActorId>,
    domain: Option<DomainId>,
    source_event_ids: Vec<EventId>,
    mut taint: Taint,
    is_mind_inference: bool,
    now: Time,
) -> Result<EventId, CoreError> {
    let (verified_basis, basis_downgraded) = verify_claimed_basis(
        requested_basis,
        &source_event_ids,
        &core.state().memory_entries,
        &core.state().soul_actor_id,
        is_mind_inference,
    );
    for &sid in &source_event_ids {
        if let Some(m) = core.state().memory_entries.get(&sid) {
            taint = join_min_taint(taint, m.taint);
        } else if let Some(rec) = core.store().get(sid)? {
            taint = join_min_taint(taint, rec.taint);
        }
    }
    let resolved_domain = domain.unwrap_or_else(|| {
        let mut embs = Vec::new();
        for &sid in &source_event_ids {
            if let Some(m) = core.state().memory_entries.get(&sid) {
                embs.extend(m.embeddings.clone());
            }
        }
        core.state().domain_clusters.assign_domain(&embs)
    });
    let (alpha_0, beta_0) = source_actor
        .as_ref()
        .map_or((1.0, 1.0), |a| core.state().belief_book.actor_prior(a));
    let origin_actor = source_actor.as_ref().map_or_else(
        || OriginActor::new(core.state().soul_actor_id.clone()),
        |a| OriginActor::new(a.clone()),
    );
    let mut rec = EventRecord::new_root(
        EventKind::BeliefChanged,
        now,
        ActorId::new("core"),
        PlaceId::new("core:belief"),
        Audience::new([core.state().soul_actor_id.clone()]),
        origin_actor,
        taint,
        NumericFeatures::default(),
        OpaquePayload::default(),
        Vec::new(),
        Some(EventDetail::BeliefChanged {
            belief_id: EventId::new(0),
            claim_key,
            requested_basis,
            basis: verified_basis,
            basis_downgraded,
            source_actor,
            domain: resolved_domain,
            source_event_ids: source_event_ids.clone(),
            taint,
            valid_from_ms: now,
            valid_to_ms: None,
            replaced_by: None,
            alpha_0,
            beta_0,
            is_mind_inference,
        }),
    );
    rec.caused_by = source_event_ids;
    let stored = append_and_apply(core, rec)?;
    Ok(stored.id)
}

pub(crate) fn record_cited_outcome_events<S: Store>(
    core: &mut Core<S>,
    cited_ids: &[EventId],
    positive: bool,
    weight: f64,
    source_actor: Option<ActorId>,
    domain: Option<DomainId>,
    now: Time,
) -> Result<Vec<EventId>, CoreError> {
    let w = if weight.is_finite() && weight >= 0.0 {
        weight
    } else {
        1.0
    };
    let (pos, neg) = if positive { (w, 0.0) } else { (0.0, w) };
    let mut emitted = Vec::with_capacity(cited_ids.len());
    for &cid in cited_ids {
        let inferred_actor = source_actor.clone().or_else(|| {
            core.state()
                .belief_book
                .beliefs
                .get(&cid)
                .and_then(|b| b.source_actor.clone())
        });
        let resolved_domain = domain.clone().unwrap_or_else(|| {
            core.state()
                .belief_book
                .beliefs
                .get(&cid)
                .map_or_else(|| DomainId::new(DEFAULT_DOMAIN_ID), |b| b.domain.clone())
        });
        let mut rec = EventRecord::new_root(
            EventKind::Evidence,
            now,
            ActorId::new("core"),
            PlaceId::new("core:evidence"),
            Audience::new([core.state().soul_actor_id.clone()]),
            OriginActor::new(core.state().soul_actor_id.clone()),
            Taint::Trusted,
            NumericFeatures::default(),
            OpaquePayload::default(),
            Vec::new(),
            Some(EventDetail::Evidence {
                target_id: cid,
                source_actor: inferred_actor,
                domain: resolved_domain,
                positive: pos,
                negative: neg,
                is_convention: false,
                place: None,
                pattern_id: None,
                cited_ids: vec![cid],
            }),
        );
        rec.caused_by.push(cid);
        let stored = append_and_apply(core, rec)?;
        emitted.push(stored.id);
    }
    Ok(emitted)
}

pub(crate) fn record_convention_event<S: Store>(
    core: &mut Core<S>,
    place: PlaceId,
    pattern_id: String,
    peer_actor: ActorId,
    domain: Option<DomainId>,
    observed_score: f64,
    now: Time,
) -> Result<EventId, CoreError> {
    let score = if observed_score.is_finite() {
        observed_score.clamp(0.0, 1.0)
    } else {
        0.5
    };
    let resolved_domain = domain.unwrap_or_else(|| DomainId::new(DEFAULT_DOMAIN_ID));
    let rec = EventRecord::new_root(
        EventKind::Evidence,
        now,
        peer_actor.clone(),
        place.clone(),
        Audience::new([core.state().soul_actor_id.clone()]),
        OriginActor::new(peer_actor.clone()),
        Taint::Provisional,
        NumericFeatures::default(),
        OpaquePayload::default(),
        Vec::new(),
        Some(EventDetail::Evidence {
            target_id: EventId::new(0),
            source_actor: Some(peer_actor),
            domain: resolved_domain,
            positive: score,
            negative: 1.0 - score,
            is_convention: true,
            place: Some(place),
            pattern_id: Some(pattern_id),
            cited_ids: Vec::new(),
        }),
    );
    let stored = append_and_apply(core, rec)?;
    Ok(stored.id)
}

pub(crate) fn apply_gist_verification_verdict<S: Store>(
    core: &mut Core<S>,
    gist_id: EventId,
    conflict_detected: bool,
    now: Time,
) -> Result<EventId, CoreError> {
    core.state_mut()
        .expectation_book
        .pending_classify
        .remove(&gist_id);
    let source_first = core
        .state()
        .memory_entries
        .get(&gist_id)
        .and_then(|e| e.source_ids.first().copied());
    let verdict_rec = EventRecord::new_root(
        EventKind::Verdict,
        now,
        ActorId::new("core"),
        PlaceId::new("core:sleep"),
        Audience::new([core.state().soul_actor_id.clone()]),
        OriginActor::new(core.state().soul_actor_id.clone()),
        Taint::Trusted,
        NumericFeatures::default(),
        OpaquePayload::default(),
        Vec::new(),
        Some(EventDetail::Verdict {
            target_event_id: gist_id,
            label: if conflict_detected {
                "gist_conflict:overstated".to_owned()
            } else {
                "gist_faithful:verified".to_owned()
            },
        }),
    );
    let stored = append_and_apply(core, verdict_rec)?;
    if conflict_detected {
        let replacement = source_first.unwrap_or(stored.id);
        core.state_mut().supersede_memory(gist_id, replacement);
    }
    Ok(stored.id)
}

pub(crate) fn execute_hard_forget_in_core<S: Store>(
    core: &mut Core<S>,
    target_event_ids: Vec<EventId>,
    target_actor_ids: Vec<ActorId>,
    reason: String,
    now: Time,
) -> Result<EventId, CoreError> {
    let mut filtered_event_ids = Vec::with_capacity(target_event_ids.len());
    for eid in target_event_ids {
        if core
            .store()
            .get(eid)?
            .is_some_and(|rec| rec.kind == EventKind::PriorLoaded)
        {
            continue;
        }
        filtered_event_ids.push(eid);
    }

    let mut all_erased: BTreeSet<EventId> = filtered_event_ids.iter().copied().collect();
    for (&eid, entry) in &core.state().memory_entries {
        if target_actor_ids
            .iter()
            .any(|a| &entry.actor == a || entry.audience.contains(a))
        {
            all_erased.insert(eid);
        }
    }
    for eid in &all_erased {
        core.store_mut().release_vector(*eid)?;
    }

    let next_epoch = core.state().epoch.saturating_add(1);
    let rec = EventRecord::new_root(
        EventKind::HardForget,
        now,
        ActorId::new("core"),
        PlaceId::new("core:hard_forget"),
        Audience::new([ActorId::new("core")]),
        OriginActor::new(ActorId::new("core")),
        Taint::Trusted,
        NumericFeatures::default(),
        OpaquePayload::default(),
        Vec::new(),
        Some(EventDetail::HardForget {
            target_event_ids: filtered_event_ids,
            target_actor_ids,
            epoch: next_epoch,
            reason,
        }),
    );
    let stored = append_and_apply(core, rec)?;
    let snap_id = format!(
        "hard_forget_epoch_{}_seq_{}",
        core.state().epoch,
        core.state().last_seq.as_u64()
    );
    let _ = take_snapshot(core, snap_id)?;
    Ok(stored.id)
}

pub(crate) fn run_sleep_if_due<S: Store>(
    core: &mut Core<S>,
    now: Time,
    doors: &mut dyn Doors,
) -> Result<Vec<Effect>, CoreError> {
    let anchor = EventRecord::new_root(
        EventKind::Tick,
        now,
        ActorId::new("core"),
        PlaceId::new("core:sleep"),
        Audience::new([core.state().soul_actor_id.clone()]),
        OriginActor::new(core.state().soul_actor_id.clone()),
        Taint::Trusted,
        NumericFeatures::default(),
        OpaquePayload::default(),
        Vec::new(),
        None,
    );
    maybe_run_sleep_consolidation(core, &anchor, now, doors)
}

fn maybe_run_sleep_consolidation<S: Store>(
    core: &mut Core<S>,
    trigger_anchor: &EventRecord,
    now: Time,
    doors: &mut dyn Doors,
) -> Result<Vec<Effect>, CoreError> {
    if !core.switches().consolidation_and_merge {
        return Ok(Vec::new());
    }

    // Resume an interrupted sleep run without duplicating already-consolidated gists (`12.1`).
    if let Some(run) = core.state().pending_sleep_run.clone() {
        let completed: BTreeSet<EventId> = run.completed_episode_ids.iter().copied().collect();
        let remaining = remaining_unconsolidated_episodes(&run.pending_episode_ids, &completed);
        if remaining.is_empty() {
            let done_rec = EventRecord::new_root(
                EventKind::SleepRun,
                now,
                ActorId::new("core"),
                PlaceId::new("core:sleep"),
                Audience::new([core.state().soul_actor_id.clone()]),
                OriginActor::new(core.state().soul_actor_id.clone()),
                Taint::Trusted,
                NumericFeatures::default(),
                OpaquePayload::default(),
                Vec::new(),
                Some(EventDetail::SleepRun {
                    snapshot_ms: run.snapshot_ms,
                    snapshot_seq: run.snapshot_seq,
                    covered_event_ids: run.covered_event_ids,
                    produced_ids: run.produced_gist_ids,
                    tokens_used: run.tokens_used,
                    completed: true,
                }),
            );
            append_and_apply(core, done_rec)?;
            return Ok(Vec::new());
        }
        return emit_consolidate_effects_for_episodes(core, &remaining, run.snapshot_ms, doors);
    }

    let trigger_status = evaluate_sleep_trigger(
        core.state().unconsolidated_count,
        core.state().last_global_activity_ms,
        core.state().last_sleep_ms,
        now,
        core.prior(),
        core.switches(),
    );
    if !trigger_status.should_trigger {
        return Ok(Vec::new());
    }

    let t_snapshot = now;
    let snapshot_seq = core.state().last_seq;

    let prior_snap = core.prior().clone();
    // 1. Soft-close qualifying open idle tails into `Provisional` (`A-05`, `12.2`).
    let soft_closed = core
        .state_mut()
        .episode_book
        .soft_close_idle_tails(t_snapshot, &prior_snap);
    for ep_id in soft_closed {
        if let Some(ep) = core.state().episode_book.episodes.get(&ep_id).cloned() {
            let ep_ev = EventRecord::derived_from(
                trigger_anchor,
                EventKind::EpisodeClosed,
                now,
                ActorId::new("core"),
                ep.place.clone(),
                Audience::new([core.state().soul_actor_id.clone()]),
                Some(ep.taint),
                NumericFeatures::default(),
                OpaquePayload::default(),
                Vec::new(),
                Some(EventDetail::EpisodeClosed {
                    episode_id: ep.episode_id,
                    place: ep.place,
                    state_label: EpisodeState::Provisional.as_str().to_owned(),
                    event_ids: ep.event_ids,
                    gist_id: None,
                    superseded_gist_id: None,
                    merged_from: Vec::new(),
                }),
            );
            append_and_apply(core, ep_ev)?;
        }
    }

    // 2. Merge adjacent/overlapping qualifying episodes per place (`R-24`).
    let merges = core
        .state_mut()
        .episode_book
        .merge_eligible_episodes(&prior_snap);
    for (merged_into, absorbed) in merges {
        if let Some(ep) = core
            .state()
            .episode_book
            .episodes
            .get(&merged_into)
            .cloned()
        {
            let merge_ev = EventRecord::derived_from(
                trigger_anchor,
                EventKind::EpisodeClosed,
                now,
                ActorId::new("core"),
                ep.place.clone(),
                Audience::new([core.state().soul_actor_id.clone()]),
                Some(ep.taint),
                NumericFeatures::default(),
                OpaquePayload::default(),
                Vec::new(),
                Some(EventDetail::EpisodeClosed {
                    episode_id: ep.episode_id,
                    place: ep.place,
                    state_label: ep.state.as_str().to_owned(),
                    event_ids: ep.event_ids,
                    gist_id: ep.gist_id,
                    superseded_gist_id: None,
                    merged_from: vec![absorbed],
                }),
            );
            append_and_apply(core, merge_ev)?;
        }
    }

    // 3. Domain-partitioned spherical k-means + Hungarian alignment (`A-12`, `13.1`).
    let active_vectors: Vec<(EventId, TaggedEmbedding)> = core
        .state()
        .memory_entries
        .iter()
        .filter(|(_, m)| !m.erased && m.timestamp_ms <= t_snapshot)
        .filter_map(|(&eid, m)| m.embeddings.first().map(|emb| (eid, emb.clone())))
        .collect();
    if !active_vectors.is_empty() {
        let mut next_clusters = core.state().domain_clusters.clone();
        let sleep_draw_seq = core
            .state()
            .draw_key(EventId::new(snapshot_seq.as_u64()))
            .as_u64();
        let update = next_clusters.update_clusters(
            &active_vectors,
            core.seed(),
            sleep_draw_seq,
            core.prior(),
        );
        if update.changed {
            let credence_rebased = core.state().belief_book.compute_rebase_records(
                &update.transitions,
                t_snapshot,
                core.prior(),
            );
            let centroids = update
                .centroids
                .into_iter()
                .map(|c| (c.domain_id, c.centroid))
                .collect();
            let dom_ev = EventRecord::derived_from(
                trigger_anchor,
                EventKind::DomainsUpdated,
                now,
                ActorId::new("core"),
                PlaceId::new("core:sleep"),
                Audience::new([core.state().soul_actor_id.clone()]),
                None,
                NumericFeatures::default(),
                OpaquePayload::default(),
                Vec::new(),
                Some(EventDetail::DomainsUpdated {
                    centroids,
                    transitions: update.transitions,
                    credence_rebased,
                }),
            );
            append_and_apply(core, dom_ev)?;
        }
    }

    // 4. Guarded Petrov vector fade (`B < -3.29`, `A-01`, `13.3`).
    let fade_cands = select_fade_candidates(
        &core.state().memory_entries,
        t_snapshot,
        core.prior(),
        core.switches(),
    );
    for cand in fade_cands {
        core.store_mut().release_vector(cand.event_id)?;
        let mut fade_ev = EventRecord::derived_from(
            trigger_anchor,
            EventKind::VectorReleased,
            now,
            ActorId::new("core"),
            PlaceId::new("core:sleep"),
            Audience::new([core.state().soul_actor_id.clone()]),
            None,
            NumericFeatures::default(),
            OpaquePayload::default(),
            Vec::new(),
            Some(EventDetail::VectorReleased {
                event_id: cand.event_id,
                base_level_b: cand.base_level_b,
                fade_threshold: cand.fade_threshold,
            }),
        );
        fade_ev.caused_by.push(cand.event_id);
        append_and_apply(core, fade_ev)?;
    }

    // 5. Consolidate all `Sealed` and `Provisional` unconsolidated episodes up to `t_snapshot`.
    let pending_eps = core
        .state()
        .episode_book
        .unconsolidated_episodes_at(t_snapshot);
    let mut covered_event_ids = Vec::new();
    let mut episode_taints = BTreeMap::new();
    for &ep_id in &pending_eps {
        if let Some(ep) = core.state().episode_book.episodes.get(&ep_id) {
            for &eid in &ep.event_ids {
                if !covered_event_ids.contains(&eid) {
                    covered_event_ids.push(eid);
                }
            }
            episode_taints.insert(ep_id, ep.taint);
        }
    }

    if pending_eps.is_empty() {
        let done_ev = EventRecord::derived_from(
            trigger_anchor,
            EventKind::SleepRun,
            now,
            ActorId::new("core"),
            PlaceId::new("core:sleep"),
            Audience::new([core.state().soul_actor_id.clone()]),
            None,
            NumericFeatures::default(),
            OpaquePayload::default(),
            Vec::new(),
            Some(EventDetail::SleepRun {
                snapshot_ms: t_snapshot,
                snapshot_seq,
                covered_event_ids,
                produced_ids: Vec::new(),
                tokens_used: 0,
                completed: true,
            }),
        );
        append_and_apply(core, done_ev)?;
        return Ok(Vec::new());
    }

    let checkpoint_ev = EventRecord::derived_from(
        trigger_anchor,
        EventKind::SleepRun,
        now,
        ActorId::new("core"),
        PlaceId::new("core:sleep"),
        Audience::new([core.state().soul_actor_id.clone()]),
        None,
        NumericFeatures::default(),
        OpaquePayload::default(),
        Vec::new(),
        Some(EventDetail::SleepRun {
            snapshot_ms: t_snapshot,
            snapshot_seq,
            covered_event_ids: covered_event_ids.clone(),
            produced_ids: Vec::new(),
            tokens_used: 0,
            completed: false,
        }),
    );
    let stored_ckpt = append_and_apply(core, checkpoint_ev)?;
    core.state_mut().pending_sleep_run = Some(PendingSleepRun {
        run_id: stored_ckpt.id,
        snapshot_ms: t_snapshot,
        snapshot_seq,
        covered_event_ids,
        pending_episode_ids: pending_eps.clone(),
        completed_episode_ids: Vec::new(),
        produced_gist_ids: Vec::new(),
        episode_taints,
        tokens_used: 0,
    });

    emit_consolidate_effects_for_episodes(core, &pending_eps, t_snapshot, doors)
}

fn emit_consolidate_effects_for_episodes<S: Store>(
    core: &mut Core<S>,
    episode_ids: &[EventId],
    now: Time,
    doors: &mut dyn Doors,
) -> Result<Vec<Effect>, CoreError> {
    let mut effects = Vec::new();
    for &ep_id in episode_ids {
        let Some(ep) = core.state().episode_book.episodes.get(&ep_id).cloned() else {
            continue;
        };
        let mut aud_members = BTreeSet::new();
        aud_members.insert(core.state().soul_actor_id.clone());
        for p in &ep.participants {
            aud_members.insert(p.clone());
        }
        let scope =
            core.state()
                .visible_scope_for(Audience::new(aud_members), Some(ep.place.clone()), now);
        let mut plan = ContextPlan {
            trigger_event_id: Some(ep.episode_id),
            place: Some(ep.place.clone()),
            visible_scope: Some(scope),
            items: ep.event_ids.clone(),
            ..ContextPlan::default()
        };
        match doors.context(&mut plan) {
            DoorDecision::Allow | DoorDecision::Strip { .. } => {
                effects.push(Effect::Model {
                    purpose: ModelPurpose::Consolidate,
                    plan,
                });
            }
            DoorDecision::Hold { .. } | DoorDecision::Refuse { .. } => {}
        }
    }
    Ok(effects)
}

fn lookup_gate_trace_for_target<S: Store>(
    core: &Core<S>,
    target_event_id: EventId,
) -> Result<(f64, GateSignals), CoreError> {
    let records = core.store().read_after_seq(None, usize::MAX)?;
    for rec in &records {
        if let Some(EventDetail::Decision {
            target_event_id: tid,
            probability,
            trace: Some(tr),
            ..
        }) = &rec.detail
            && *tid == target_event_id
        {
            return Ok((*probability, tr.signals.clone()));
        }
    }
    Ok((0.5, GateSignals::default()))
}

struct LearningOutcomeCtx {
    outcome_kind: LearningOutcomeKind,
    sample_propensity: Option<f64>,
    termination_reason: Option<TerminationReason>,
    teacher_reliability: f64,
}

fn maybe_apply_learning_step<S: Store>(
    core: &mut Core<S>,
    cause_rec: &EventRecord,
    target_event_id: EventId,
    ctx: LearningOutcomeCtx,
    now: Time,
) -> Result<(), CoreError> {
    let (prob_go, signals) = lookup_gate_trace_for_target(core, target_event_id)?;
    let prior_c = core.prior().clone();
    let sw_c = core.switches().clone();
    let sample = LearningSample {
        source_event_id: cause_rec.id,
        outcome_kind: ctx.outcome_kind,
        prob_go,
        signals,
        sample_propensity: ctx.sample_propensity,
        teacher_reliability: ctx.teacher_reliability,
        termination_reason: ctx.termination_reason,
        timestamp_ms: now,
    };
    let mut weights = core.state().adaptive_weights;
    let res = core
        .state_mut()
        .learner
        .apply_outcome(&mut weights, &sample, &prior_c, &sw_c);
    if res.applied {
        core.state_mut().adaptive_weights = weights;
        if let Some(epoch_id) = res.epoch_id {
            let dw = res.delta_weights;
            let lrec = EventRecord::derived_from(
                cause_rec,
                EventKind::LearningChange,
                now,
                ActorId::new("core"),
                cause_rec.place.clone(),
                cause_rec.audience.clone(),
                None,
                NumericFeatures::default(),
                OpaquePayload::default(),
                Vec::new(),
                Some(EventDetail::LearningUpdate {
                    epoch_id,
                    source_event_id: cause_rec.id,
                    outcome_code: ctx.outcome_kind.code(),
                    delta_w: [
                        dw.w_own_name,
                        dw.w_info,
                        dw.w_fok,
                        dw.w_rel,
                        dw.w_commitment,
                        dw.w_sc,
                    ],
                    delta_theta_0: res.delta_theta_0,
                    ips_weight: res.ips_weight,
                    signed_error: res.signed_error,
                }),
            );
            append_and_apply(core, lrec)?;
        }
    }
    Ok(())
}

fn tr_cause_fallback(exp: &ExpectationRecord) -> EventRecord {
    EventRecord::new_root(
        EventKind::ExpectationChanged,
        exp.updated_at_ms,
        ActorId::new("core"),
        exp.places
            .first()
            .cloned()
            .unwrap_or_else(|| PlaceId::new("core:goal")),
        exp.audience.clone(),
        OriginActor::new(ActorId::new("soul")),
        Taint::Trusted,
        NumericFeatures::default(),
        OpaquePayload::default(),
        Vec::new(),
        None,
    )
}

fn maybe_extract_procedure_from_goal<S: Store>(
    core: &mut Core<S>,
    exp: &ExpectationRecord,
    cause_rec: &EventRecord,
    now: Time,
) -> Result<(), CoreError> {
    if exp.kind != ExpectKind::Goal || cause_rec.taint != Taint::Trusted {
        return Ok(());
    }
    let mut places = exp.places.clone();
    let mut steps = Vec::new();
    for &cid in &exp.goal_chain_ids {
        if let Some(rec) = core.store().get(cid)? {
            if !places.contains(&rec.place) {
                places.push(rec.place.clone());
            }
            steps.push(format!("step:{}", rec.id.as_u64()));
        }
    }
    if steps.is_empty() && !exp.condition.trim().is_empty() {
        steps.push(exp.condition.clone());
    }
    let mut required_cap_ids = Vec::new();
    for aid in &exp.linked_action_ids {
        if let Some(tr) = core.state().terminal_receipts.get(aid) {
            if !required_cap_ids.contains(&tr.capability_id) {
                required_cap_ids.push(tr.capability_id.clone());
            }
        } else if let Some(inf) = core.state().in_flight_actions.get(aid)
            && !required_cap_ids.contains(&inf.capability_id)
        {
            required_cap_ids.push(inf.capability_id.clone());
        }
    }
    let catalog = core.state().capability_catalog.clone();
    let schema_hash = compute_procedure_schema_hash(required_cap_ids.iter().filter_map(|cid| {
        catalog
            .get(cid)
            .map(|entry| (cid, &entry.descriptor.capability_schema_hash))
    }));
    let params = ExtractProcedureParams {
        procedure_id: None,
        source_goal_id: Some(exp.expectation_id),
        name: format!("goal_proc_{}", exp.expectation_id.as_u64()),
        steps,
        required_cap_ids,
        capability_schema_hash: schema_hash,
        places,
        actors: exp.people.clone(),
        audience: exp.audience.clone(),
        domain: exp.domain.clone(),
        source_event_ids: exp.goal_chain_ids.clone(),
        taint: cause_rec.taint,
        now_ms: now,
    };
    let (prior_c, sw_c) = (core.prior().clone(), core.switches().clone());
    if let Some(tp) = core
        .state_mut()
        .skill_book
        .extract_procedure_from_goal_chain(params, &catalog, &prior_c, &sw_c)
    {
        core.state_mut().register_procedure(tp.to_context_entry());
        let ev = EventRecord::derived_from(
            cause_rec,
            EventKind::LearningChange,
            now,
            ActorId::new("core"),
            cause_rec.place.clone(),
            cause_rec.audience.clone(),
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
        append_and_apply(core, ev)?;
    }
    Ok(())
}

fn flush_recalibration_if_pending<S: Store>(
    core: &mut Core<S>,
    cause_rec: &EventRecord,
    reason: &str,
    now: Time,
    doors: &mut dyn Doors,
) -> Result<(), CoreError> {
    if !core.state().recalibration_pending {
        return Ok(());
    }
    let self_card = core.state().build_self_card();
    let mut plan = ContextPlan {
        trigger_event_id: Some(cause_rec.id),
        place: Some(cause_rec.place.clone()),
        sections: ContextSections {
            self_card: self_card.clone(),
            ..ContextSections::default()
        },
        ..ContextPlan::default()
    };
    let verdict_label = match doors.context(&mut plan) {
        DoorDecision::Allow => "recalibrated: allow",
        DoorDecision::Strip { .. } => "recalibrated: strip",
        DoorDecision::Hold { .. } => "recalibration_held",
        DoorDecision::Refuse { .. } => "recalibration_refused",
    };
    let dv = EventRecord::derived_from(
        cause_rec,
        EventKind::DoorVerdict,
        now,
        ActorId::new("core"),
        cause_rec.place.clone(),
        cause_rec.audience.clone(),
        None,
        NumericFeatures::default(),
        OpaquePayload::default(),
        Vec::new(),
        Some(EventDetail::DoorVerdict {
            door: DoorKind::Context,
            target_event_id: Some(cause_rec.id),
            rule: "self_card_recalibration".to_owned(),
            verdict: verdict_label.to_owned(),
        }),
    );
    append_and_apply(core, dv)?;
    let rc = EventRecord::derived_from(
        cause_rec,
        EventKind::DoorVerdict,
        now,
        ActorId::new("core"),
        cause_rec.place.clone(),
        cause_rec.audience.clone(),
        None,
        NumericFeatures::default(),
        OpaquePayload::default(),
        Vec::new(),
        Some(EventDetail::SelfCardRecalibrated {
            degraded_capabilities: self_card.degraded_capabilities,
            calibration_samples: self_card.calibration_samples,
            budget_headroom_ratio: self_card.budget_headroom_ratio,
            contradiction_flags: self_card.contradiction_flags,
            reason: reason.to_owned(),
        }),
    );
    append_and_apply(core, rc)?;
    Ok(())
}
