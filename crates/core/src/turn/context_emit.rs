//! Sans-IO cognitive loop turn execution, record-first enforcement, and crash recovery.

use std::collections::BTreeSet;

use monoagens_protocol::{
    Action, ActorId, Audience, CURRENT_PROTOCOL_VERSION, CapabilityId, ChannelId, EventId,
    ExpectKind, OpaquePayload, OriginActor, PlaceId, ReceiptOutcome, TaggedEmbedding, Taint,
    TerminationReason, Time, VisibleScope,
};

use crate::activation::{DEFAULT_PETROV_D, closeness_from_trace};
use crate::context::{
    AutoRecallTriggerState, CommitmentSlotItem, ContextSections, ContextSegment, ContextSlotKind,
    CoreSelfSnapshot, ModelTask, ProcedureEntry, SelectedToolSet, ToolSelectionBudget,
    enforce_context_budget, segment_from_event, segment_from_recalled, select_tools_for_context,
};
use crate::event::{
    DecisionOutcome, DoorKind, EventDetail, EventKind, EventRecord, ExpectationLifecycleState,
    LogSeq, NumericFeatures,
};
use crate::expectations::{ExpectationRecord, PendingClassifyKind, is_positive_verdict_label};
use crate::neural::GateArm;
use crate::ports::{ContextPlan, DoorDecision, Doors, ModelPurpose, Store};
use crate::recall::{MemoryBasis, RecallQuery, RecalledItem, run_recall_with_model};
use crate::state::{CoreState, WakeSnapshotRecord};
use crate::{Core, CoreError, Effect};

use super::sleep_and_beliefs::{
    maybe_extract_procedure_from_goal, record_place_adapt_feedback, tr_cause_fallback,
};

const ANCHOR_CHECK_INTERVAL_WAKES: u64 = 32;
const AUTO_RECALL_MAX_ATTEMPTS: u8 = 2;
const WORKING_SET_FULL_OBS_LIMIT: usize = 12;
const PLACE_TAIL_LIMIT: usize = 16;
const DEFAULT_PATIENCE_MS: u64 = 2_000;
const DEFAULT_RESPONSE_EXPIRY_QUANTILE: f64 = 0.90;

pub(super) fn wake_reasoning_for_event<S: Store>(
    core: &mut Core<S>,
    trigger: &EventRecord,
    now: Time,
    doors: &mut dyn Doors,
) -> Result<Vec<Effect>, CoreError> {
    wake_reasoning_for_event_with_extra(core, trigger, &[], now, doors)
}

pub(super) fn wake_reasoning_for_event_with_extra<S: Store>(
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
    query.limit = core.config().max_recalled_memories.max(1);
    query.record_uses = true;

    let recall = run_recall_with_model(
        core.store(),
        core.state(),
        &query,
        now,
        core.config(),
        core.neural_model(),
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
    let tool_budget = ToolSelectionBudget::from_config(core.config(), catalog.len());
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
    let interval = ANCHOR_CHECK_INTERVAL_WAKES;
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
        .unwrap_or(AutoRecallTriggerState {
            active: false,
            attempt: 0,
            max_attempts: AUTO_RECALL_MAX_ATTEMPTS,
            coarser_grain_allowed: false,
            last_reason: None,
        });
    let self_guide_snapshot = if core.config().self_guide_enabled {
        let model_blake3 = core.neural_model().map(|m| &m.blake3);
        Some(CoreSelfSnapshot::build(
            CURRENT_PROTOCOL_VERSION,
            model_blake3,
            core.config().online_adaptation,
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
    let source_cards = core.state().ledger.build_source_cards_in_scope(
        &participants,
        &core.state().expectation_book,
        &soul_id,
        Some((&scope, now)),
        extra_items,
        |actor| {
            core.state()
                .actor_exchanges
                .get(actor)
                .map(|tr| closeness_from_trace(tr, now, DEFAULT_PETROV_D, 4.0))
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
            condition: e.condition.clone(),
            estimated_bytes: 96 + e.condition.len(),
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

        // The place tail is what was said and done at the place: percepts and the soul's own
        // actions. Core bookkeeping (decisions, withholds, expectation records, model calls,
        // receipts) is never conversation and must not displace it or leak gate internals.
        if &rec.place == place
            && !recalled_set.contains(&rec.id)
            && matches!(rec.kind, EventKind::Percept | EventKind::Action)
            && !rec.payload.is_empty()
        {
            place_tail_candidates.push(rec);
        }
    }

    let full_obs_limit = WORKING_SET_FULL_OBS_LIMIT;
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

    let tail_limit = PLACE_TAIL_LIMIT;
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
        task: None,
        budget_bytes: core.config().max_context_bytes,
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

    let (neural_p_verify, neural_p_uncertainty) = core
        .state()
        .decision_neural_hints
        .get(&trigger_event_id)
        .copied()
        .or_else(|| {
            trigger
                .caused_by
                .iter()
                .find_map(|cid| core.state().decision_neural_hints.get(cid).copied())
        })
        .unwrap_or((None, None));
    let mut plan = ContextPlan {
        trigger_event_id: Some(trigger_event_id),
        place: Some(place.clone()),
        visible_scope: Some(scope),
        items,
        sections,
        dropped_items,
        static_prefix_digest: [0u8; 32],
        valve_self_snapshot: None,
        neural_p_verify,
        neural_p_uncertainty,
    };
    plan.refresh_static_prefix_digest();
    Ok(plan)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn execute_memory_recall_act<S: Store>(
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
    query.limit = core.config().max_recalled_memories.max(1);
    query.record_uses = true;

    let recall = run_recall_with_model(
        core.store(),
        core.state(),
        &query,
        now,
        core.config(),
        core.neural_model(),
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


pub(super) fn append_and_apply<S: Store>(
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

pub(super) fn find_linked_expectation_for_act(
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
pub(super) fn dispatch_action_and_open_predictions<S: Store>(
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
                    .map_or(DEFAULT_PATIENCE_MS, |d| {
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

                // Open Response prediction.
                {
                    let medium = ChannelId::new(action.capability_id.as_str());
                    let resp_window_ms = core
                        .state()
                        .expectation_book
                        .response_latency
                        .estimate_window_ms(
                            &primary_place,
                            &medium,
                            body_declared_ms,
                            DEFAULT_RESPONSE_EXPIRY_QUANTILE,
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


fn select_caps_for_scope<S: Store>(
    core: &Core<S>,
    scope: &VisibleScope,
    now: Time,
) -> SelectedToolSet {
    let catalog = core.state().effective_capability_catalog();
    let procs: Vec<ProcedureEntry> = core.state().procedures.values().cloned().collect();
    let tool_budget = ToolSelectionBudget::from_config(core.config(), catalog.len());
    select_tools_for_context(&[], &[], &catalog, &procs, scope, now, tool_budget)
}

/// Segment for task material (an earlier message, a gist source, an episode event), or `None`
/// when the record is not conversation (bookkeeping, empty payload) or is outside `scope`.
pub(super) fn task_segment_for<S: Store>(
    core: &Core<S>,
    id: EventId,
    scope: &VisibleScope,
    now: Time,
) -> Result<Option<ContextSegment>, String> {
    let rec = core
        .store()
        .get(id)
        .map_err(|e| format!("task material {id} unreadable: {e}"))?;
    let Some(rec) = rec else {
        return Ok(None);
    };
    let erased = core
        .state()
        .memory_entries
        .get(&rec.id)
        .is_some_and(|m| m.erased);
    if erased
        || rec.payload.is_empty()
        || !matches!(
            rec.kind,
            EventKind::Percept | EventKind::Action | EventKind::Thought
        )
        || !scope.allows(&rec.audience, Some(rec.id), now)
    {
        return Ok(None);
    }
    let soul_id = core.state().soul_actor_id.clone();
    let basis = core.state().memory_entries.get(&rec.id).map(|m| m.basis);
    Ok(Some(segment_from_event(
        ContextSlotKind::Task,
        &rec,
        &soul_id,
        basis,
        false,
        "task_material",
    )))
}

/// Builds the classify task for an expectation and its triggering event.
fn classify_task_for<S: Store>(
    core: &Core<S>,
    trigger: &EventRecord,
    exp: &ExpectationRecord,
    now: Time,
) -> Result<ModelTask, String> {
    let is_silence = exp
        .response_spec
        .as_ref()
        .is_some_and(|s| s.is_judged_silence);
    if is_silence {
        let scope = core.state().visible_scope_for(
            trigger.audience.clone(),
            Some(trigger.place.clone()),
            now,
        );
        return match task_segment_for(core, exp.target_event_id, &scope, now)? {
            Some(earlier) => Ok(ModelTask::SilenceReask { earlier }),
            None => Err("the message the soul stayed silent on is not available".to_owned()),
        };
    }
    if exp.condition.trim().is_empty() {
        return Err("classification condition text is empty".to_owned());
    }
    if exp.kind == ExpectKind::Goal {
        Ok(ModelTask::GoalProgress {
            condition: exp.condition.clone(),
        })
    } else {
        Ok(ModelTask::ConditionMet {
            condition: exp.condition.clone(),
        })
    }
}

pub(super) fn build_classify_plan<S: Store>(
    core: &Core<S>,
    trigger: &EventRecord,
    exp: &ExpectationRecord,
    now: Time,
    items: Vec<EventId>,
) -> Result<ContextPlan, String> {
    let task = classify_task_for(core, trigger, exp, now)?;
    let commitment = (!matches!(task, ModelTask::SilenceReask { .. })).then_some(exp);
    build_task_plan(core, trigger, task, commitment, now, items)
}

/// Builds a non-reasoning plan carrying `task`, the trigger and (optionally) one commitment.
pub(super) fn build_task_plan<S: Store>(
    core: &Core<S>,
    trigger: &EventRecord,
    task: ModelTask,
    commitment: Option<&ExpectationRecord>,
    now: Time,
    items: Vec<EventId>,
) -> Result<ContextPlan, String> {
    if trigger.payload.as_bytes().is_empty() {
        return Err("classification trigger event has empty content".to_owned());
    }
    let scope =
        core.state()
            .visible_scope_for(trigger.audience.clone(), Some(trigger.place.clone()), now);
    if !scope.allows(&trigger.audience, Some(trigger.id), now) {
        return Err("classification trigger event is not visible in scope".to_owned());
    }
    let soul_id = core.state().soul_actor_id.clone();
    let selected_tools = select_caps_for_scope(core, &scope, now);
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
    let commitments = commitment
        .map(|exp| CommitmentSlotItem {
            expectation_id: exp.expectation_id,
            kind: exp.kind,
            due_ms: exp.due_ms,
            expiry_ms: exp.expiry_ms,
            places: exp.places.clone(),
            people: exp.people.clone(),
            is_fired_or_overdue: exp.due_ms <= now || exp.is_overdue || exp.due_woken,
            condition: exp.condition.clone(),
            estimated_bytes: 96 + exp.condition.len(),
        })
        .into_iter()
        .collect();
    let sections = ContextSections {
        slot_0_anchor: core.state().identity_anchor.clone(),
        slot_0_principles: core.state().principles.clone(),
        static_core_caps: selected_tools.static_core,
        dynamic_caps: selected_tools.dynamic_caps,
        commitments,
        now_ms: now,
        knowledge_cutoff: core.state().knowledge_cutoff.clone(),
        trigger: trigger_seg,
        task: Some(task),
        budget_bytes: core.config().max_context_bytes,
        ..ContextSections::default()
    };
    Ok(ContextPlan {
        trigger_event_id: Some(trigger.id),
        place: Some(trigger.place.clone()),
        visible_scope: Some(scope),
        items,
        sections,
        ..ContextPlan::default()
    })
}

pub(super) fn record_classify_refusal<S: Store>(
    core: &mut Core<S>,
    trigger: &EventRecord,
    now: Time,
    reason: &str,
) -> Result<(), CoreError> {
    let dv = EventRecord::derived_from(
        trigger,
        EventKind::DoorVerdict,
        now,
        ActorId::new("core"),
        trigger.place.clone(),
        trigger.audience.clone(),
        None,
        NumericFeatures::default(),
        OpaquePayload::default(),
        Vec::new(),
        Some(EventDetail::DoorVerdict {
            door: DoorKind::Context,
            target_event_id: Some(trigger.id),
            rule: "classify_missing_inputs".to_owned(),
            verdict: format!("refused: {reason}"),
        }),
    );
    append_and_apply(core, dv)?;
    Ok(())
}

pub(super) fn build_consolidate_plan<S: Store>(
    core: &Core<S>,
    trigger: &EventRecord,
    place: PlaceId,
    scope: VisibleScope,
    now: Time,
    items: Vec<EventId>,
) -> ContextPlan {
    let soul_id = core.state().soul_actor_id.clone();
    let selected_tools = select_caps_for_scope(core, &scope, now);
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
    let sections = ContextSections {
        slot_0_anchor: core.state().identity_anchor.clone(),
        slot_0_principles: core.state().principles.clone(),
        static_core_caps: selected_tools.static_core,
        dynamic_caps: selected_tools.dynamic_caps,
        now_ms: now,
        knowledge_cutoff: core.state().knowledge_cutoff.clone(),
        trigger: trigger_seg,
        budget_bytes: core.config().max_context_bytes,
        ..ContextSections::default()
    };
    ContextPlan {
        trigger_event_id: Some(trigger.id),
        place: Some(place),
        visible_scope: Some(scope),
        items,
        sections,
        ..ContextPlan::default()
    }
}

pub(super) fn handle_classify_result<S: Store>(
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
                let feedback_place = sj_stored.place.clone();
                let source_taint = trigger_ev.as_ref().map_or(Taint::Trusted, |ev| ev.taint);
                record_place_adapt_feedback(
                    core,
                    Some(&sj_stored),
                    &feedback_place,
                    GateArm::Silent,
                    -0.5,
                    true,
                    source_taint,
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
                        core.config(),
                        core.neural_model(),
                        0,
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

pub(super) fn wake_reasoning_for_expectation<S: Store>(
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

pub(super) const fn join_min_taint(a: Taint, b: Taint) -> Taint {
    if taint_rank(a) <= taint_rank(b) { a } else { b }
}

pub(super) fn check_citation_lock<S: Store>(
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

pub(super) fn record_withheld<S: Store>(
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

pub(super) fn record_act_refused<S: Store>(
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
