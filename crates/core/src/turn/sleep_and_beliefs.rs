//! Sans-IO cognitive loop turn execution, record-first enforcement, and crash recovery.

use std::collections::{BTreeMap, BTreeSet};

use monoagens_protocol::{
    ActorId, Audience, DomainId, EventId, ExpectKind, OpaquePayload, OriginActor, PlaceId, Taint,
    Time,
};

use crate::beliefs::{DEFAULT_DOMAIN_ID, verify_claimed_basis};
use crate::consolidation::{
    EpisodeState, ExtractProcedureParams, PendingSleepRun, compute_procedure_schema_hash,
    evaluate_sleep_trigger, remaining_unconsolidated_episodes, select_fade_candidates,
};
use crate::context::{ContextSections, ModelTask};
use crate::event::{DoorKind, EventDetail, EventKind, EventRecord, NumericFeatures};
use crate::expectations::ExpectationRecord;
use crate::neural::GateArm;
use crate::ports::{ContextPlan, DoorDecision, Doors, ModelPurpose, Store};
use crate::recall::MemoryBasis;
use crate::{Core, CoreError, Effect};

use super::context_emit::{
    append_and_apply, build_consolidate_plan, join_min_taint, task_segment_for,
};
use super::open::take_snapshot;

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
    let resolved_domain = domain.unwrap_or_else(|| DomainId::new(DEFAULT_DOMAIN_ID));
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
            .is_some_and(|rec| rec.kind == EventKind::ConfigLoaded)
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

pub(super) fn maybe_run_sleep_consolidation<S: Store>(
    core: &mut Core<S>,
    trigger_anchor: &EventRecord,
    now: Time,
    doors: &mut dyn Doors,
) -> Result<Vec<Effect>, CoreError> {
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
        core.config(),
    );
    if !trigger_status.should_trigger {
        return Ok(Vec::new());
    }

    let t_snapshot = now;
    let snapshot_seq = core.state().last_seq;

    let cfg_snap = core.config().clone();
    // 1. Soft-close qualifying open idle tails into `Provisional` (`A-05`, `12.2`).
    let soft_closed = core
        .state_mut()
        .episode_book
        .soft_close_idle_tails(t_snapshot, &cfg_snap);
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
    let merges = core.state_mut().episode_book.merge_eligible_episodes();
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

    // 3. Guarded Petrov vector fade (`A-01`, `13.3`).
    let fade_cands = select_fade_candidates(
        &core.state().memory_entries,
        t_snapshot,
        core.config(),
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
        let mut plan = if let Some(trigger_rec) =
            core.store().get(ep.episode_id).ok().flatten().or_else(|| {
                ep.event_ids
                    .first()
                    .and_then(|id| core.store().get(*id).ok().flatten())
            }) {
            let mut events = Vec::new();
            for &eid in &ep.event_ids {
                if let Ok(Some(seg)) = task_segment_for(core, eid, &scope, now) {
                    events.push(seg);
                }
            }
            let mut p = build_consolidate_plan(
                core,
                &trigger_rec,
                ep.place.clone(),
                scope,
                now,
                ep.event_ids.clone(),
            );
            p.sections.task = Some(ModelTask::ConsolidateEpisode { events });
            p.trigger_event_id = Some(ep.episode_id);
            p
        } else {
            ContextPlan {
                trigger_event_id: Some(ep.episode_id),
                place: Some(ep.place.clone()),
                visible_scope: Some(scope),
                items: ep.event_ids.clone(),
                ..ContextPlan::default()
            }
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

/// Records a bounded Warm-Start UCB bandit feedback update on `place_adapt[place]`
/// strictly via an event-sourced `AdaptFeedbackApplied` log record (`C-3`, `C-4`, `C-10`).
/// No-op when `online_adaptation` is disabled or when negative/delayed feedback comes
/// strictly from `Taint::Untrusted`.
#[allow(clippy::too_many_arguments)]
pub(super) fn record_place_adapt_feedback<S: Store>(
    core: &mut Core<S>,
    cause: Option<&EventRecord>,
    place: &PlaceId,
    arm: GateArm,
    reward: f32,
    is_delayed_one_sided: bool,
    source_taint: Taint,
    now: Time,
) -> Result<(), CoreError> {
    let adapt_cfg = core.config().effective_adapt_config();
    if !adapt_cfg.online_adaptation {
        return Ok(());
    }
    if source_taint == Taint::Untrusted && (reward < 0.0 || is_delayed_one_sided) {
        return Ok(());
    }
    let context_hint = core.state().compute_place_context_hint(place);
    let detail = Some(EventDetail::AdaptFeedbackApplied {
        place: place.clone(),
        arm,
        reward,
        is_delayed_one_sided,
        context_hint,
        source_taint,
    });
    let ev = match cause {
        Some(c) => EventRecord::derived_from(
            c,
            EventKind::Verdict,
            now,
            ActorId::new("core"),
            place.clone(),
            c.audience.clone(),
            Some(source_taint),
            NumericFeatures::default(),
            OpaquePayload::default(),
            Vec::new(),
            detail,
        ),
        None => EventRecord::new_root(
            EventKind::Verdict,
            now,
            ActorId::new("core"),
            place.clone(),
            Audience::new([core.state().soul_actor_id.clone()]),
            OriginActor::new(core.state().soul_actor_id.clone()),
            source_taint,
            NumericFeatures::default(),
            OpaquePayload::default(),
            Vec::new(),
            detail,
        ),
    };
    let _ = append_and_apply(core, ev)?;
    Ok(())
}

pub(super) fn tr_cause_fallback(exp: &ExpectationRecord) -> EventRecord {
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

pub(super) fn maybe_extract_procedure_from_goal<S: Store>(
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
    if let Some(tp) = core
        .state_mut()
        .skill_book
        .extract_procedure_from_goal_chain(params, &catalog)
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

pub(super) fn flush_recalibration_if_pending<S: Store>(
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recall::partition_recalled_and_cited;

    #[test]
    fn partition_recalled_and_cited_separates_surfaced_from_cited_deterministically() {
        let recalled = vec![EventId::new(10), EventId::new(20), EventId::new(30)];
        let cited = vec![EventId::new(20), EventId::new(99)];
        let (recalled_only, cited_from_recalled) = partition_recalled_and_cited(&recalled, &cited);
        assert_eq!(recalled_only, vec![EventId::new(10), EventId::new(30)]);
        assert_eq!(cited_from_recalled, vec![EventId::new(20)]);
    }
}
