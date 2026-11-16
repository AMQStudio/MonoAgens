//! Deterministic `Two-Epoch Replay` and bit-for-bit decision verification (`R-34`).
//!
//! Replays a log from a snapshot cursor (or from the beginning when `None`), recomputing
//! gate decisions and comparing them bit-for-bit (`to_bits()`) against recorded decisions,
//! reporting the first divergence with both recorded and recomputed values.
//!
//! Post `hard_forget`, pre-forget adaptive weights remain frozen at the surviving epoch
//! snapshot anchor unless [`ReplayMode::FullReplay`] is selected.

use std::collections::BTreeMap;

use monoagens_protocol::EventId;
use serde::{Deserialize, Serialize};

use crate::CoreError;
use crate::blake3;
use crate::event::{DecisionOutcome, EventDetail, EventKind, LogSeq};
use crate::gate::{DecisionReason, GateOutcome};
use crate::ports::{SnapshotCheckpoint, Store};
use crate::prior::Prior;
use crate::registry::Switches;
use crate::state::CoreState;

/// Controls how `Two-Epoch Replay` treats pre-forget adaptive weights after `hard_forget`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ReplayMode {
    /// Post `hard_forget`, freeze pre-forget adaptive weights at the surviving epoch snapshot anchor.
    #[default]
    TwoEpochFrozen,
    /// Re-evaluate using current `Prior`/`Switches` across the entire replayed span without freezing.
    FullReplay,
}

/// Comparable snapshot of a decision's numeric and categorical outputs.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RecomputedDecisionValue {
    pub outcome: DecisionOutcome,
    pub salience: f64,
    pub threshold: f64,
    pub probability: f64,
    pub draw: f64,
    pub reason: Option<DecisionReason>,
}

/// First bit-for-bit divergence encountered during log replay.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReplayDivergence {
    pub decision_event_id: EventId,
    pub target_event_id: EventId,
    pub seq: LogSeq,
    pub field: String,
    pub recorded: RecomputedDecisionValue,
    pub recomputed: RecomputedDecisionValue,
}

/// Outcome summary of a [`replay_from_checkpoint`] or [`replay_log`] run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReplayReport {
    pub snapshot_cursor: Option<LogSeq>,
    pub epoch: u64,
    pub anchor_preserved: bool,
    pub replayed_events: usize,
    pub matched_decisions: usize,
    pub first_divergence: Option<ReplayDivergence>,
}

impl ReplayReport {
    /// Returns `true` if every replayed decision matched bit-for-bit with zero divergences.
    #[must_use]
    pub const fn is_bit_identical(&self) -> bool {
        self.first_divergence.is_none()
    }
}

/// Replays `store` starting from `store.latest_checkpoint()` (if present) using [`ReplayMode::TwoEpochFrozen`].
///
/// The prior in effect is taken strictly from the log's `prior_loaded` events (or `Prior::default()`
/// prior to the first `prior_loaded` event), never from the caller.
pub fn replay_log<S: Store>(
    store: &S,
    switches: &Switches,
    seed: u64,
) -> Result<ReplayReport, CoreError> {
    let ckpt = store.latest_checkpoint()?;
    replay_from_checkpoint(
        store,
        ckpt.as_ref(),
        switches,
        seed,
        ReplayMode::TwoEpochFrozen,
    )
}

/// Replays `store` from an explicit `checkpoint` cursor (or full log when `None`) under `mode`,
/// comparing every replayed `Percept` decision bit-for-bit against the recorded `Decision` event.
///
/// The prior in effect at every log position comes strictly from `prior_loaded` events (and
/// snapshot state for checkpointed prefixes), never from the caller.
pub fn replay_from_checkpoint<S: Store>(
    store: &S,
    checkpoint: Option<&SnapshotCheckpoint>,
    switches: &Switches,
    seed: u64,
    mode: ReplayMode,
) -> Result<ReplayReport, CoreError> {
    let (mut state, snapshot_cursor, mut active_prior) = match checkpoint {
        Some(ckpt) => {
            let actual = blake3::hash(&ckpt.bytes);
            if actual != ckpt.blake3 {
                return Err(CoreError::SnapshotChecksumMismatch {
                    snapshot_id: ckpt.snapshot_id.clone(),
                    expected: blake3::to_hex(&ckpt.blake3),
                    actual: blake3::to_hex(&actual),
                });
            }
            let mut decoded = CoreState::decode_snapshot(&ckpt.bytes)?;
            decoded.map_switches = switches.clone();
            let resolved_prior = match decoded.recorded_prior.clone() {
                Some(p) => p,
                None => find_latest_prior_up_to(store, ckpt.cursor)?,
            };
            resolved_prior.validate()?;
            decoded.theta_ex = resolved_prior.theta_ex;
            decoded.m_ex = resolved_prior.m_ex;
            decoded.habituation_gamma = resolved_prior.habituation_gamma();
            decoded.patience_decay_rho = resolved_prior.patience_decay_rho;
            decoded.map_prior = resolved_prior.clone();
            (decoded, Some(ckpt.cursor), resolved_prior)
        }
        None => {
            let default_prior = Prior::default();
            let mut st = CoreState::new();
            st.sync_prior(&default_prior, switches);
            (st, None, default_prior)
        }
    };

    let use_frozen_anchor = matches!(mode, ReplayMode::TwoEpochFrozen);
    let mut cursor = snapshot_cursor;
    let mut replayed_events = 0usize;
    let mut matched_decisions = 0usize;
    let mut first_divergence: Option<ReplayDivergence> = None;
    let mut pending_expected: BTreeMap<EventId, GateOutcome> = BTreeMap::new();

    loop {
        let batch = store.read_after_seq(cursor, 256)?;
        if batch.is_empty() {
            break;
        }
        for rec in batch {
            cursor = Some(rec.seq);
            replayed_events += 1;

            match rec.kind {
                EventKind::PriorLoaded => {
                    if let Some(EventDetail::PriorLoaded {
                        prior: loaded_prior,
                    }) = &rec.detail
                    {
                        loaded_prior.validate()?;
                        active_prior = (**loaded_prior).clone();
                    }
                    state.apply_event(&rec);
                }
                EventKind::Percept => {
                    let expected = state.appraise_percept_record(
                        &rec,
                        &active_prior,
                        switches,
                        seed,
                        use_frozen_anchor,
                    );
                    pending_expected.insert(rec.id, expected);
                    state.apply_event(&rec);
                }
                EventKind::Decision => {
                    if let Some(EventDetail::Decision {
                        target_event_id,
                        outcome,
                        salience,
                        threshold,
                        probability,
                        draw,
                        reason,
                        ..
                    }) = &rec.detail
                        && *outcome != DecisionOutcome::DoorBlocked
                        && *outcome != DecisionOutcome::Processed
                        && let Some(recomp) = pending_expected.remove(target_event_id)
                    {
                        let recorded_val = RecomputedDecisionValue {
                            outcome: *outcome,
                            salience: *salience,
                            threshold: *threshold,
                            probability: *probability,
                            draw: *draw,
                            reason: *reason,
                        };
                        let recomputed_val = RecomputedDecisionValue {
                            outcome: recomp.outcome,
                            salience: recomp.effective_salience,
                            threshold: recomp.threshold,
                            probability: recomp.probability,
                            draw: recomp.draw,
                            reason: Some(recomp.reason),
                        };

                        if let Some(field) = find_first_mismatch(&recorded_val, &recomputed_val) {
                            if first_divergence.is_none() {
                                first_divergence = Some(ReplayDivergence {
                                    decision_event_id: rec.id,
                                    target_event_id: *target_event_id,
                                    seq: rec.seq,
                                    field: field.to_owned(),
                                    recorded: recorded_val,
                                    recomputed: recomputed_val,
                                });
                            }
                        } else {
                            matched_decisions += 1;
                        }
                    }
                    state.apply_event(&rec);
                }
                _ => {
                    state.apply_event(&rec);
                }
            }
        }
    }

    let anchor_preserved = state.epoch == 0
        || (state.epoch_anchor.frozen && state.epoch_anchor.frozen_weights.is_some());

    Ok(ReplayReport {
        snapshot_cursor,
        epoch: state.epoch,
        anchor_preserved,
        replayed_events,
        matched_decisions,
        first_divergence,
    })
}

fn find_first_mismatch(
    recorded: &RecomputedDecisionValue,
    recomputed: &RecomputedDecisionValue,
) -> Option<&'static str> {
    if recorded.outcome != recomputed.outcome {
        return Some("outcome");
    }
    if recorded.salience.to_bits() != recomputed.salience.to_bits() {
        return Some("salience");
    }
    if recorded.threshold.to_bits() != recomputed.threshold.to_bits() {
        return Some("threshold");
    }
    if recorded.probability.to_bits() != recomputed.probability.to_bits() {
        return Some("probability");
    }
    if recorded.draw.to_bits() != recomputed.draw.to_bits() {
        return Some("draw");
    }
    if recorded.reason.is_some() && recorded.reason != recomputed.reason {
        return Some("reason");
    }
    None
}

fn find_latest_prior_up_to<S: Store>(store: &S, max_seq: LogSeq) -> Result<Prior, CoreError> {
    let mut found: Option<Prior> = None;
    let mut cursor: Option<LogSeq> = None;
    loop {
        let batch = store.read_after_seq(cursor, 256)?;
        if batch.is_empty() {
            break;
        }
        for rec in batch {
            if rec.seq > max_seq {
                return Ok(found.unwrap_or_default());
            }
            cursor = Some(rec.seq);
            if rec.kind == EventKind::PriorLoaded
                && let Some(EventDetail::PriorLoaded { prior }) = &rec.detail
            {
                found = Some((**prior).clone());
            }
        }
    }
    Ok(found.unwrap_or_default())
}
