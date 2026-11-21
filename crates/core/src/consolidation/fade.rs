//! Guarded Petrov archive/fade pruning (`B < fade_threshold`).

use std::collections::BTreeMap;

use monoagens_protocol::{EventId, Time};

use crate::config::CoreConfig;
use crate::recall::MemoryEntry;

/// Candidate memory selected for vector release via Petrov base-level fade (`B < fade_threshold`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FadeCandidate {
    pub event_id: EventId,
    pub base_level_b: f64,
    pub fade_threshold: f64,
}

/// Identifies non-erased memories with stored vectors whose guarded Petrov base-level
/// activation `B` falls strictly below `config.vector_fade_threshold`.
///
/// Never drops `hard_forget` tombstones.
#[must_use]
pub fn select_fade_candidates(
    memories: &BTreeMap<EventId, MemoryEntry>,
    t_snapshot: Time,
    config: &CoreConfig,
) -> Vec<FadeCandidate> {
    let d = 0.5_f64;
    let threshold = config.vector_fade_threshold;
    if !threshold.is_finite() {
        return Vec::new();
    }
    let mut out = Vec::new();

    for (&eid, entry) in memories {
        if entry.erased || entry.embeddings.is_empty() {
            continue;
        }
        let b = entry.activation.base_level(t_snapshot, d);
        if b.is_finite() && b < threshold {
            out.push(FadeCandidate {
                event_id: eid,
                base_level_b: b,
                fade_threshold: threshold,
            });
        }
    }
    out.sort_by_key(|c| c.event_id);
    out
}
