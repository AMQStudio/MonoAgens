//! Core startup, snapshot verification, log tail replay, config recording, and crash recovery.

use std::collections::BTreeSet;
use std::sync::Arc;

use monoagens_protocol::{
    ActorId, Audience, CatchupPolicy, ExpectKind, OpaquePayload, OriginActor, PlaceId, Taint,
    TerminationReason, Time,
};

use crate::blake3;
use crate::config::CoreConfig;
use crate::event::{
    DecisionOutcome, EventDetail, EventKind, EventRecord, ExpectationLifecycleState, LogSeq,
    NumericFeatures, encode_core_config,
};
use crate::expectations::{ExpectationRecord, advance_recurrence_schedule};
use crate::gate::DecisionReason;
use crate::r#loop::QueueClass;
use crate::neural::NeuralModel;
use crate::ports::{Doors, SnapshotCheckpoint, Store};
use crate::state::CoreState;
use crate::{Core, CoreError, Effect, Input, RecoveryReport};

use super::context_emit::{append_and_apply, wake_reasoning_for_expectation};

/// Opens a [`Core`] over `store`:
/// 1. Validates `config`;
/// 2. Loads the latest snapshot checkpoint and verifies its `blake3` checksum (failing loudly);
/// 3. Replays all log events after the snapshot cursor in [`LogSeq`] order;
/// 4. Resumes any trailing unconsumed inputs in deterministic `(timestamp_ms, LogSeq)` order
///    without calling a model or dispatching any action;
/// 5. Records `ConfigLoaded` if the log has no config yet or `config` changed.
pub(crate) fn open_core<S: Store>(
    mut store: S,
    config: CoreConfig,
    model: Option<Arc<NeuralModel>>,
) -> Result<Core<S>, CoreError> {
    config.validate()?;

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
            if let Some(ref recorded) = decoded.recorded_config.clone() {
                decoded.sync_config(recorded);
            }
            (decoded, Some(ckpt.cursor))
        }
        None => {
            let mut st = CoreState::new();
            st.sync_config(&config);
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

    if state.recorded_config.as_ref() != Some(&config) {
        let ts = state
            .last_timestamp_ms
            .unwrap_or_else(|| Time::from_millis(0));
        let mut cfg_w = crate::event::BinWriter::new();
        encode_core_config(&mut cfg_w, &config);
        let config_blake3 = blake3::hash(&cfg_w.finish());
        let mut rec = EventRecord::new_root(
            EventKind::ConfigLoaded,
            ts,
            ActorId::new("core"),
            PlaceId::new("core:config"),
            Audience::new([ActorId::new("core")]),
            OriginActor::new(ActorId::new("core")),
            Taint::Trusted,
            NumericFeatures::default(),
            OpaquePayload::default(),
            Vec::new(),
            Some(EventDetail::ConfigLoaded {
                config: Box::new(config.clone()),
                config_blake3,
            }),
        );
        let cfg_id = store.append(rec.clone())?;
        rec.id = cfg_id;
        rec.seq = LogSeq::new(state.last_seq.as_u64().saturating_add(1));
        if let Some(persisted) = store.get(cfg_id)? {
            state.apply_event(&persisted);
        } else {
            state.apply_event(&rec);
        }
    } else {
        state.sync_config(&config);
    }

    if !state.expectation_book.expectations.is_empty() {
        state.startup_recovery_pending = true;
    }

    let report = RecoveryReport {
        snapshot_cursor,
        replayed_events,
        resumed_inputs,
    };

    Ok(Core::from_parts(store, config, model, state, report))
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
