//! Slim Core V2 appraisal gate: deterministic commitments, Neural Cognitive Controller step,
//! bounded backlog chunking, and shared helper types.

pub mod backlog;
pub mod decide;
pub mod types;

pub use backlog::{
    BacklogEventInput, BacklogScheduleResult, MAX_CHUNKS_PER_WAKE, PlaceBacklogChunk,
    chunk_place_backlog, is_backlog_event, schedule_backlog_chunks,
};
pub use decide::{
    CommitmentInputs, DecisionReason, DecisionTrace, GateOutcome, OPEN_WAIT_PEER_JITTER_MAX_MS,
    compute_open_wait_jitter_ms, decide_gate,
};
pub use types::{
    CandidateMemory, ExpectationCue, FrozenDialogueActHead, OpenWait, PlaceHistoryEntry,
    PromiseSummary, cosine_similarity,
};
