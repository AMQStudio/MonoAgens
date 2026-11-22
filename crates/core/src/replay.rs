//! Trace-Pinned deterministic replay from verified snapshots + log tail (`tasks.md` 4.4).
//!
//! Slim Core V2 replay reconstructs [`CoreState`] from a verified snapshot (or empty state) and
//! applies all subsequent log records in [`LogSeq`] order, verifying every recorded
//! [`EventKind::Decision`] trace (`model_blake3`, `feature_schema_version`, `online_adaptation_applied`,
//! finite calibrated neural outputs, `outcome`, and `reason`) without requiring bit-exact
//! floating-point neural re-execution across different CPU/SIMD targets.

use crate::blake3;
use crate::config::CoreConfig;
use crate::event::{DecisionOutcome, EventDetail, EventKind, LogSeq};
use crate::gate::DecisionReason;
use crate::neural::{FEATURE_SCHEMA_VERSION_V1, FEATURE_SCHEMA_VERSION_V2, FEATURE_SCHEMA_VERSION_V3};
use crate::ports::{SnapshotCheckpoint, Store};
use crate::state::CoreState;
use crate::{CoreError, RecoveryReport};

/// Replay mode controlling pinned model digest verification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ReplayMode {
    /// Verifies `model_blake3` pins match `expected_model_blake3` (when both are present).
    #[default]
    Pinned,
    /// Verifies recorded traces without enforcing a single `expected_model_blake3` across model upgrades.
    Unpinned,
}

/// Recorded values extracted from a [`EventKind::Decision`] log event.
#[derive(Debug, Clone, PartialEq)]
pub struct RecordedDecisionValue {
    pub seq: LogSeq,
    pub event_id: monoagens_protocol::EventId,
    pub target_event_id: monoagens_protocol::EventId,
    pub outcome: DecisionOutcome,
    pub probability: f64,
    pub draw: f64,
    pub reason: Option<DecisionReason>,
    pub model_blake3: Option<[u8; 32]>,
    pub feature_schema_version: Option<u16>,
    pub online_adaptation_applied: bool,
}

/// First Trace-Pinned divergence detected during [`replay_from_checkpoint`].
#[derive(Debug, Clone, PartialEq)]
pub struct ReplayMismatch {
    pub seq: LogSeq,
    pub event_id: monoagens_protocol::EventId,
    pub target_event_id: monoagens_protocol::EventId,
    pub field: String,
    pub recorded: RecordedDecisionValue,
    pub detail: String,
}

/// Summary report returned by [`replay_log`] and [`replay_from_checkpoint`].
#[derive(Debug, Clone, PartialEq)]
pub struct ReplayReport {
    pub state: CoreState,
    pub recovery: RecoveryReport,
    pub verified_decisions: usize,
    pub mismatches: Vec<ReplayMismatch>,
    pub active_config: CoreConfig,
}

impl ReplayReport {
    /// Returns `true` when all recorded decisions passed Trace-Pinned verification.
    #[must_use]
    pub fn is_exact(&self) -> bool {
        self.mismatches.is_empty()
    }
}

/// Replays the full log from seq `0` in `mode`.
pub fn replay_log<S: Store>(
    store: &S,
    mode: ReplayMode,
    expected_model_blake3: Option<[u8; 32]>,
) -> Result<ReplayReport, CoreError> {
    replay_from_checkpoint(store, None, mode, expected_model_blake3)
}

/// Replays from optional verified `checkpoint` + subsequent log tail in [`LogSeq`] order,
/// performing Trace-Pinned validation on every recorded `Decision` event.
pub fn replay_from_checkpoint<S: Store>(
    store: &S,
    checkpoint: Option<&SnapshotCheckpoint>,
    mode: ReplayMode,
    expected_model_blake3: Option<[u8; 32]>,
) -> Result<ReplayReport, CoreError> {
    let (mut state, snapshot_cursor, mut active_config) = match checkpoint {
        Some(ckpt) => {
            let actual = blake3::hash(&ckpt.bytes);
            if actual != ckpt.blake3 {
                return Err(CoreError::SnapshotChecksumMismatch {
                    snapshot_id: ckpt.snapshot_id.clone(),
                    expected: blake3::to_hex(&ckpt.blake3),
                    actual: blake3::to_hex(&actual),
                });
            }
            let decoded = CoreState::decode_snapshot(&ckpt.bytes)?;
            let resolved_cfg = match decoded.recorded_config.clone() {
                Some(cfg) => cfg,
                None => find_latest_config_up_to(store, ckpt.cursor)?,
            };
            resolved_cfg.validate()?;
            (decoded, Some(ckpt.cursor), resolved_cfg)
        }
        None => {
            let default_cfg = CoreConfig::default();
            let mut st = CoreState::new();
            st.sync_config(&default_cfg);
            (st, None, default_cfg)
        }
    };

    let mut cursor = snapshot_cursor;
    let mut replayed_events = 0usize;
    let mut verified_decisions = 0usize;
    let mut mismatches = Vec::new();

    loop {
        let batch = store.read_after_seq(cursor, 256)?;
        if batch.is_empty() {
            break;
        }
        for rec in batch {
            cursor = Some(rec.seq);

            match rec.kind {
                EventKind::ConfigLoaded => {
                    if let Some(EventDetail::ConfigLoaded { config, .. }) = &rec.detail {
                        config.validate()?;
                        active_config = (**config).clone();
                    }
                    state.apply_event(&rec);
                    replayed_events += 1;
                    continue;
                }
                EventKind::Decision => {
                    if let Some(EventDetail::Decision {
                        target_event_id,
                        outcome,
                        salience,
                        probability,
                        draw,
                        reason,
                        trace: Some(tr),
                        ..
                    }) = &rec.detail
                        && *outcome != DecisionOutcome::DoorBlocked
                        && *outcome != DecisionOutcome::Processed
                    {
                        verified_decisions += 1;
                        let recorded_val = RecordedDecisionValue {
                            seq: rec.seq,
                            event_id: rec.id,
                            target_event_id: *target_event_id,
                            outcome: *outcome,
                            probability: *probability,
                            draw: *draw,
                            reason: *reason,
                            model_blake3: tr.model_blake3,
                            feature_schema_version: Some(tr.feature_schema_version),
                            online_adaptation_applied: tr.online_adaptation_applied,
                        };

                        if (*salience - tr.probability).abs() > 1e-12 && mismatches.is_empty() {
                            mismatches.push(ReplayMismatch {
                                seq: rec.seq,
                                event_id: rec.id,
                                target_event_id: *target_event_id,
                                field: "salience".to_owned(),
                                recorded: recorded_val,
                                detail: format!(
                                    "event salience={} != trace probability={}",
                                    salience, tr.probability
                                ),
                            });
                        } else if let Some((field, detail)) = verify_trace_pinned(
                            &recorded_val,
                            tr,
                            mode,
                            expected_model_blake3,
                            &active_config,
                        ) && mismatches.is_empty()
                        {
                            mismatches.push(ReplayMismatch {
                                seq: rec.seq,
                                event_id: rec.id,
                                target_event_id: *target_event_id,
                                field,
                                recorded: recorded_val,
                                detail,
                            });
                        }
                    }
                }
                _ => {}
            }

            state.apply_event(&rec);
            replayed_events += 1;
        }
    }

    Ok(ReplayReport {
        state,
        recovery: RecoveryReport {
            snapshot_cursor,
            replayed_events,
            resumed_inputs: Vec::new(),
        },
        verified_decisions,
        mismatches,
        active_config,
    })
}

fn verify_trace_pinned(
    rec: &RecordedDecisionValue,
    tr: &crate::gate::DecisionTrace,
    mode: ReplayMode,
    expected_model_blake3: Option<[u8; 32]>,
    active_config: &CoreConfig,
) -> Option<(String, String)> {
    if rec.outcome != tr.outcome {
        return Some((
            "outcome".to_owned(),
            format!("event outcome {:?} != trace outcome {:?}", rec.outcome, tr.outcome),
        ));
    }
    if rec.reason.is_some_and(|r| r != tr.reason) {
        return Some((
            "reason".to_owned(),
            format!("event reason {:?} != trace reason {:?}", rec.reason, tr.reason),
        ));
    }
    if (rec.probability - tr.probability).abs() > 1e-12 {
        return Some((
            "probability".to_owned(),
            format!("event p={} != trace p={}", rec.probability, tr.probability),
        ));
    }
    if (rec.draw - tr.draw).abs() > 1e-12 {
        return Some((
            "draw".to_owned(),
            format!("event draw={} != trace draw={}", rec.draw, tr.draw),
        ));
    }
    if tr.feature_schema_version != FEATURE_SCHEMA_VERSION_V1
        && tr.feature_schema_version != FEATURE_SCHEMA_VERSION_V2
        && tr.feature_schema_version != FEATURE_SCHEMA_VERSION_V3
    {
        return Some((
            "feature_schema_version".to_owned(),
            format!(
                "trace schema {} not in [{}, {}, {}]",
                tr.feature_schema_version,
                FEATURE_SCHEMA_VERSION_V1,
                FEATURE_SCHEMA_VERSION_V2,
                FEATURE_SCHEMA_VERSION_V3
            ),
        ));
    }
    if !tr.neural.gate_logits.iter().all(|v| v.is_finite())
        || !tr
            .neural
            .gate_probs
            .iter()
            .all(|v| v.is_finite() && *v >= 0.0)
    {
        return Some((
            "neural.gate_probs".to_owned(),
            "non-finite or negative neural gate outputs in trace".to_owned(),
        ));
    }
    let sum: f32 = tr.neural.gate_probs.iter().sum();
    if (sum - 1.0).abs() > 1e-3 {
        return Some((
            "neural.gate_probs".to_owned(),
            format!("gate_probs sum {sum} not normalized"),
        ));
    }
    if tr.reason == DecisionReason::Commitment
        && (!tr.commitment_bypass || tr.outcome != DecisionOutcome::Go)
    {
        return Some((
            "commitment_bypass".to_owned(),
            "commitment decision must have commitment_bypass=true and outcome=Go".to_owned(),
        ));
    }
    if tr.reason == DecisionReason::NeutralFallback
        && (!tr.neural.is_neutral_fallback || tr.model_blake3.is_some())
    {
        return Some((
            "neutral_fallback".to_owned(),
            "neutral fallback must have is_neutral_fallback=true and model_blake3=None".to_owned(),
        ));
    }
    if tr.online_adaptation_applied && !active_config.online_adaptation {
        return Some((
            "online_adaptation_applied".to_owned(),
            "trace applied online adaptation while active CoreConfig disabled it".to_owned(),
        ));
    }
    if mode == ReplayMode::Pinned
        && let Some(expected) = expected_model_blake3
        && let Some(actual) = tr.model_blake3
        && actual != expected
    {
        return Some((
            "model_blake3".to_owned(),
            format!(
                "pinned digest mismatch: recorded={}, expected={}",
                blake3::to_hex(&actual),
                blake3::to_hex(&expected)
            ),
        ));
    }
    None
}

fn find_latest_config_up_to<S: Store>(
    store: &S,
    max_seq: LogSeq,
) -> Result<CoreConfig, CoreError> {
    let mut found: Option<CoreConfig> = None;
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
            if rec.kind == EventKind::ConfigLoaded
                && let Some(EventDetail::ConfigLoaded { config, .. }) = &rec.detail
            {
                found = Some((**config).clone());
            }
        }
    }
    Ok(found.unwrap_or_default())
}
