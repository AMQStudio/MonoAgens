//! Memory importance scoring (`R-05`, `A-06`).
//!
//! `importance = max(i_reason, i_commit, i_surprise)` clamped to `[0.0, 1.0]`.

use serde::{Deserialize, Serialize};

/// Components contributing to a memory item's importance (`R-05`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub struct ImportanceInputs {
    /// Importance assigned by reasoning when writing (`[0.0, 1.0]`).
    pub i_reason: f64,
    /// `true` when linked to a promise, goal, or expectation (`i_commit = 1.0`).
    pub linked_to_commitment: bool,
    /// Normalized event-boundary surprise at recording time (`[0.0, 1.0]`).
    pub i_surprise: f64,
}

/// Computes `importance = max(i_reason, i_commit, i_surprise)` in `[0.0, 1.0]`.
#[must_use]
pub fn compute_importance(inputs: ImportanceInputs) -> f64 {
    let i_reason = if inputs.i_reason.is_finite() {
        inputs.i_reason.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let i_commit = if inputs.linked_to_commitment {
        1.0
    } else {
        0.0
    };
    let i_surprise = if inputs.i_surprise.is_finite() {
        inputs.i_surprise.clamp(0.0, 1.0)
    } else {
        0.0
    };
    i_reason.max(i_commit).max(i_surprise)
}
