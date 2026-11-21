//! Snapshot-isolated sleep consolidation (`episodes`, `fade`, `skills`).

pub mod episodes;
pub mod fade;
pub mod skills;

use std::collections::{BTreeMap, BTreeSet};

use monoagens_protocol::{EventId, Taint, Time};
use serde::{Deserialize, Serialize};

use crate::config::CoreConfig;
use crate::event::LogSeq;

pub use episodes::{
    EpisodeAdmitParams, EpisodeBook, EpisodeEventOutcome, EpisodeRecord, EpisodeState,
    MAX_ACTIVE_THREADS_PER_SCOPE, canonical_audience_hash, participant_jaccard,
};
pub use fade::{
    DEFAULT_MAX_FADE_FRACTION, FadeCandidate, PREFERENCE_ANCHOR_CONCEPT,
    is_protected_preference_anchor, select_fade_candidates, select_fade_candidates_guarded,
};
pub use skills::{
    DEFAULT_PROC_UCB_C, ExtractProcedureParams, FailureEpisodeRecord, LessonEntry,
    ProcedureLifecycleState, ProcedureMonitorOutcome, RecordLessonParams, RegisterProcedureParams,
    SkillBook, SuccessEpisodeRecord, TrackedProcedure, compute_procedure_schema_hash,
    schema_matches_catalog,
};

/// Evaluated sleep trigger decision at time `now_ms`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SleepTriggerStatus {
    pub should_trigger: bool,
    pub unconsolidated_count: usize,
    pub idle_elapsed_ms: u64,
    pub effective_t_idle_ms: u64,
}

/// Active or interrupted snapshot-isolated sleep consolidation run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingSleepRun {
    pub run_id: EventId,
    pub snapshot_ms: Time,
    pub snapshot_seq: LogSeq,
    pub covered_event_ids: Vec<EventId>,
    pub pending_episode_ids: Vec<EventId>,
    pub completed_episode_ids: Vec<EventId>,
    pub produced_gist_ids: Vec<EventId>,
    pub episode_taints: BTreeMap<EventId, Taint>,
    pub tokens_used: u64,
}

/// Evaluates whether snapshot-isolated sleep consolidation should trigger (`>= sleep_min_episodes` AND `idle >= sleep_idle_ms`).
#[must_use]
pub fn evaluate_sleep_trigger(
    unconsolidated_count: usize,
    last_activity_ms: Option<Time>,
    _last_sleep_ms: Option<Time>,
    now_ms: Time,
    config: &CoreConfig,
) -> SleepTriggerStatus {
    let idle_elapsed_ms = last_activity_ms.map_or(0, |last| now_ms.saturating_sub_ms(last));
    let effective_t_idle_ms = config.sleep_idle_ms;
    let should_trigger = unconsolidated_count >= config.sleep_min_episodes
        && last_activity_ms.is_some()
        && idle_elapsed_ms >= effective_t_idle_ms;

    SleepTriggerStatus {
        should_trigger,
        unconsolidated_count,
        idle_elapsed_ms,
        effective_t_idle_ms,
    }
}

/// Computes monotonic `min(trust)` taint across a set of source events.
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
