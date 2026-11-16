//! Guarded Petrov archive/fade pruning (`B < -3.29`, `A-01`, `13.3`).

use std::collections::BTreeMap;

use monoagens_protocol::{EventId, Time};

use crate::prior::Prior;
use crate::recall::MemoryEntry;
use crate::registry::Switches;

/// Candidate memory selected for vector release via Petrov base-level fade (`B < fade_threshold`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FadeCandidate {
    pub event_id: EventId,
    pub base_level_b: f64,
    pub fade_threshold: f64,
}

/// Identifies non-erased memories with stored vectors whose guarded Petrov base-level
/// activation `B` falls strictly below `prior.fade_threshold` (`-3.29` default, `-inf` when neutral).
///
/// Never evaluates total `A` and never drops `hard_forget` tombstones (`A-01`).
#[must_use]
pub fn select_fade_candidates(
    memories: &BTreeMap<EventId, MemoryEntry>,
    t_snapshot: Time,
    prior: &Prior,
    switches: &Switches,
) -> Vec<FadeCandidate> {
    if !switches.vector_fade {
        return Vec::new();
    }
    let d = prior.petrov_d();
    let threshold = prior.fade_threshold;
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
