//! Honest `R-47` feedback cue timing (`d_art = 0 ms`, `working_cue` at `1 s`, `progress_note` at `10 s`).

use serde::{Deserialize, Serialize};

/// Artificial delay enforced by `R-47` (`0 ms`, never padded).
pub const DEFAULT_D_ART_MS: u64 = 0;

/// Threshold for emitting `working_cue` (`1_000 ms`).
pub const DEFAULT_WORKING_CUE_MS: u64 = 1_000;

/// Threshold for emitting `progress_note` (`10_000 ms`).
pub const DEFAULT_PROGRESS_NOTE_MS: u64 = 10_000;

/// Honest in-flight feedback cue kinds (`R-47`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HonestCueKind {
    WorkingCue,
    ProgressNote,
}

/// Configured cue thresholds (`d_art_ms = 0`, `working_cue_ms = 1_000`, `progress_note_ms = 10_000`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct HonestCueThresholds {
    pub d_art_ms: u64,
    pub working_cue_ms: u64,
    pub progress_note_ms: u64,
}

impl Default for HonestCueThresholds {
    fn default() -> Self {
        Self {
            d_art_ms: DEFAULT_D_ART_MS,
            working_cue_ms: DEFAULT_WORKING_CUE_MS,
            progress_note_ms: DEFAULT_PROGRESS_NOTE_MS,
        }
    }
}

/// Cue emitted once a genuine wait crosses its threshold.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmittedCue {
    pub kind: HonestCueKind,
    pub wait_id: String,
    pub elapsed_ms: u64,
    pub emitted_at_ms: u64,
}

/// Deterministic, clock-driven cue tracker that never fabricates cues early.
#[derive(Debug, Clone)]
pub struct HonestCueTracker {
    wait_id: String,
    started_at_ms: u64,
    thresholds: HonestCueThresholds,
    working_emitted: bool,
    progress_emitted: bool,
    finished: bool,
}

impl HonestCueTracker {
    /// Starts tracking a genuine wait at `started_at_ms`.
    #[must_use]
    pub fn start(
        wait_id: impl Into<String>,
        started_at_ms: u64,
        thresholds: HonestCueThresholds,
    ) -> Self {
        Self {
            wait_id: wait_id.into(),
            started_at_ms,
            thresholds,
            working_emitted: false,
            progress_emitted: false,
            finished: false,
        }
    }

    /// Marks the wait complete; subsequent `poll` calls emit nothing.
    pub fn finish(&mut self) {
        self.finished = true;
    }

    /// Polls at `now_ms`, returning any newly crossed honest cues in order (`WorkingCue`, then `ProgressNote`).
    pub fn poll(&mut self, now_ms: u64) -> Vec<EmittedCue> {
        if self.finished {
            return Vec::new();
        }
        let elapsed_ms = now_ms.saturating_sub(self.started_at_ms);
        let mut out = Vec::new();

        if !self.working_emitted && elapsed_ms >= self.thresholds.working_cue_ms {
            self.working_emitted = true;
            out.push(EmittedCue {
                kind: HonestCueKind::WorkingCue,
                wait_id: self.wait_id.clone(),
                elapsed_ms,
                emitted_at_ms: now_ms,
            });
        }
        if !self.progress_emitted && elapsed_ms >= self.thresholds.progress_note_ms {
            self.progress_emitted = true;
            out.push(EmittedCue {
                kind: HonestCueKind::ProgressNote,
                wait_id: self.wait_id.clone(),
                elapsed_ms,
                emitted_at_ms: now_ms,
            });
        }
        out
    }

    /// Returns the configured thresholds.
    #[must_use]
    pub const fn thresholds(&self) -> HonestCueThresholds {
        self.thresholds
    }
}
