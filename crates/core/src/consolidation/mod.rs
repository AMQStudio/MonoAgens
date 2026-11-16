//! Snapshot-isolated sleep consolidation (`A-04`, `A-05`, `A-06`, `A-12`, `R-24`, `R-35`, `R-43`).

pub mod boundaries;
pub mod episodes;
pub mod fade;
pub mod skills;

use std::collections::{BTreeMap, BTreeSet};

use monoagens_protocol::{EventId, Taint, Time};
use serde::{Deserialize, Serialize};

use crate::event::LogSeq;
use crate::gate::{effective_idle_trigger_ms, sleep_pressure};
use crate::prior::Prior;
use crate::registry::Switches;

pub use boundaries::{
    BoundaryEvaluation, BoundaryInputs, PlaceBoundaryState, empirical_bayes_moments,
    participant_jaccard,
};
pub use episodes::{
    EpisodeAdmitParams, EpisodeBook, EpisodeEventOutcome, EpisodeRecord, EpisodeState,
};
pub use fade::{FadeCandidate, select_fade_candidates};
pub use skills::{
    ExtractProcedureParams, FailureEpisodeRecord, LessonEntry, ProcedureLifecycleState,
    ProcedureMonitorOutcome, RecordLessonParams, RegisterProcedureParams, SkillBook,
    SuccessEpisodeRecord, TrackedProcedure, compute_procedure_schema_hash, schema_matches_catalog,
};

/// Evaluated sleep trigger decision at time `now_ms`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SleepTriggerStatus {
    pub should_trigger: bool,
    pub unconsolidated_count: usize,
    pub idle_elapsed_ms: u64,
    pub effective_t_idle_ms: u64,
}

/// Active or interrupted snapshot-isolated sleep consolidation run (`12.1`).
///
/// Captures an immutable `snapshot_ms` / `snapshot_seq` boundary so live events arriving
/// during sleep never wait (`Delta theta_sleep = 0`) and interrupted runs resume only the
/// unconsolidated remainder without duplicating gists.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingSleepRun {
    pub run_id: EventId,
    pub snapshot_ms: Time,
    pub snapshot_seq: LogSeq,
    pub covered_event_ids: Vec<EventId>,
    pub pending_episode_ids: Vec<EventId>,
    pub completed_episode_ids: Vec<EventId>,
    pub produced_gist_ids: Vec<EventId>,
    /// Maps each episode's request/trigger id to `(episode_id, min_trust_taint)`.
    pub episode_taints: BTreeMap<EventId, Taint>,
    pub tokens_used: u64,
}

/// Evaluates whether snapshot-isolated sleep consolidation should trigger (`>= N_min` AND `idle >= T_idle_eff`).
#[must_use]
pub fn evaluate_sleep_trigger(
    unconsolidated_count: usize,
    last_activity_ms: Option<Time>,
    last_sleep_ms: Option<Time>,
    now_ms: Time,
    prior: &Prior,
    switches: &Switches,
) -> SleepTriggerStatus {
    if !switches.consolidation_and_merge {
        return SleepTriggerStatus {
            should_trigger: false,
            unconsolidated_count,
            idle_elapsed_ms: 0,
            effective_t_idle_ms: prior.t_idle_sleep_ms,
        };
    }
    let idle_elapsed_ms = last_activity_ms.map_or(0, |last| now_ms.saturating_sub_ms(last));
    let wake_elapsed_ms = last_sleep_ms.map_or(0, |s| now_ms.saturating_sub_ms(s));
    let d_sleep = sleep_pressure(unconsolidated_count, wake_elapsed_ms, prior);
    let effective_t_idle_ms = effective_idle_trigger_ms(prior.t_idle_sleep_ms, d_sleep);

    // Spec (`A-01`, `12.1`): `>= N_min` (not `>`) AND global idle `>= T_idle_eff`.
    let should_trigger = unconsolidated_count >= prior.n_min_sleep
        && last_activity_ms.is_some()
        && idle_elapsed_ms >= effective_t_idle_ms;

    SleepTriggerStatus {
        should_trigger,
        unconsolidated_count,
        idle_elapsed_ms,
        effective_t_idle_ms,
    }
}

/// Computes monotonic `min(trust)` taint across a set of source events (`13.2`).
#[must_use]
pub fn join_episode_taint<'a>(taints: impl IntoIterator<Item = &'a Taint>) -> Taint {
    let mut acc: Option<Taint> = None;
    for &t in taints {
        acc = Some(match acc {
            None => t,
            Some(prev) => prev.combine(t),
        });
    }
    acc.unwrap_or(Taint::Untrusted)
}

/// Deduplicates episode IDs preserving deterministic order while skipping already-completed ones.
#[must_use]
pub fn remaining_unconsolidated_episodes(
    candidates: &[EventId],
    completed: &BTreeSet<EventId>,
) -> Vec<EventId> {
    candidates
        .iter()
        .copied()
        .filter(|id| !completed.contains(id))
        .collect()
}
