//! Appraisal gate: signals, salience, opportunity-cost drives, threshold, seeded draw, and backlog.

pub mod backlog;
pub mod decide;
pub mod drives;
pub mod patience;
pub mod salience;
pub mod signals;
pub mod threshold;

pub use backlog::{
    BacklogEventInput, BacklogScheduleResult, MAX_CHUNKS_PER_WAKE, PlaceBacklogChunk,
    chunk_place_backlog, is_backlog_event, schedule_backlog_chunks,
};
pub use decide::{
    CommitmentInputs, DecisionReason, DecisionTrace, GateOutcome, RecomputedTrace, decide_gate,
    recompute_decision_from_trace,
};
pub use drives::{
    EffortControl, PendingBeliefSummary, SwitchingBudget, curiosity_drive,
    effective_idle_trigger_ms, effort_control, max_belief_voi, sleep_pressure, switching_drive,
};
pub use patience::{LatencySketch, OpenWait, PatienceEstimator};
pub use salience::{
    HabituationBook, HabituationEntry, HabituationKey, SalienceEvaluation, SalienceWeights,
    compute_salience,
};
pub use signals::{
    CandidateMemory, DegEvaluation, ExpectationCue, FrozenDialogueActHead, GateSignals,
    PlaceHistoryEntry, SignalInputs, compute_deg, compute_social_cost, cosine_similarity,
    evaluate_signals,
};
pub use threshold::{
    InitHabituationEntry, PromiseSummary, ThresholdInputs, ThresholdTerms, breakpoint_score,
    compute_delta_theta_init, compute_promise_debt, compute_talk_rate, compute_threshold,
    idle_interruption_cost,
};
