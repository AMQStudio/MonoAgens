//! Shared Waist-2 (`monoagens.body.v1`) body runtime, honest `R-47` cue tracker,
//! and split-bubble chat delivery with mid-stream barge-in and `R-09` echo correlation.

#![forbid(unsafe_code)]

pub mod cues;
pub mod runtime;
pub mod split_bubble;

pub use cues::{
    DEFAULT_D_ART_MS, DEFAULT_PROGRESS_NOTE_MS, DEFAULT_WORKING_CUE_MS, EmittedCue, HonestCueKind,
    HonestCueThresholds, HonestCueTracker,
};
pub use runtime::{
    PersistentBodyRuntime, capability_schema_hash, load_state_dir_from_env, make_refused_receipt,
    make_terminal_receipt,
};
pub use split_bubble::{
    BargeInToken, BubbleSender, EchoRecord, EchoRegistry, SplitBubbleOutcome, SplitError,
    deliver_split_bubbles, split_text_chunks,
};
