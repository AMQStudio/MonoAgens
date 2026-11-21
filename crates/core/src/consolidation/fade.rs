//! Guarded Petrov archive/fade pruning (`B < fade_threshold`) with per-cycle fraction cap
//! and durable preference anchor protection.

use std::collections::{BTreeMap, BTreeSet};

use monoagens_protocol::{EventId, Time};

use crate::config::CoreConfig;
use crate::recall::MemoryEntry;

/// Default maximum fraction of eligible low-activation memories faded in a single sleep cycle.
pub const DEFAULT_MAX_FADE_FRACTION: f64 = 0.25;

/// Concept tag identifying durable preference anchors exempt from Petrov vector fade.
pub const PREFERENCE_ANCHOR_CONCEPT: &str = "preference_anchor";

/// Candidate memory selected for vector release via Petrov base-level fade (`B < fade_threshold`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FadeCandidate {
    pub event_id: EventId,
    pub base_level_b: f64,
    pub fade_threshold: f64,
}

/// Returns `true` when `entry` is protected as a durable preference anchor (either via explicit
/// `protected_anchors` set or carrying the [`PREFERENCE_ANCHOR_CONCEPT`] tag).
#[must_use]
pub fn is_protected_preference_anchor(
    entry: &MemoryEntry,
    protected_anchors: &BTreeSet<EventId>,
) -> bool {
    protected_anchors.contains(&entry.event_id)
        || entry
            .concepts
            .iter()
            .any(|c| c == PREFERENCE_ANCHOR_CONCEPT)
}

/// Identifies non-erased memories with stored vectors whose guarded Petrov base-level
/// activation `B` falls strictly below `config.vector_fade_threshold`, excluding durable
/// preference anchors and capping per-cycle pruning at [`DEFAULT_MAX_FADE_FRACTION`].
///
/// Never drops `hard_forget` tombstones.
#[must_use]
pub fn select_fade_candidates(
    memories: &BTreeMap<EventId, MemoryEntry>,
    t_snapshot: Time,
    config: &CoreConfig,
) -> Vec<FadeCandidate> {
    select_fade_candidates_guarded(
        memories,
        t_snapshot,
        config,
        &BTreeSet::new(),
        DEFAULT_MAX_FADE_FRACTION,
    )
}

/// Guarded Petrov fade selector excluding `protected_anchors` (plus concept-tagged durable
/// preference anchors) and capping released candidates at `ceil(eligible_count * max_fade_fraction)`
/// (at least `1` when `eligible_count > 0 && max_fade_fraction > 0.0`, `0` when fraction `<= 0.0`).
///
/// Candidates are sorted deterministically by lowest `base_level_b` first, tie-breaking by `EventId`.
#[must_use]
pub fn select_fade_candidates_guarded(
    memories: &BTreeMap<EventId, MemoryEntry>,
    t_snapshot: Time,
    config: &CoreConfig,
    protected_anchors: &BTreeSet<EventId>,
    max_fade_fraction: f64,
) -> Vec<FadeCandidate> {
    let d = 0.5_f64;
    let threshold = config.vector_fade_threshold;
    if !threshold.is_finite() || !max_fade_fraction.is_finite() || max_fade_fraction <= 0.0 {
        return Vec::new();
    }
    let frac = max_fade_fraction.clamp(0.0, 1.0);
    let mut out = Vec::new();

    for (&eid, entry) in memories {
        if entry.erased
            || entry.embeddings.is_empty()
            || is_protected_preference_anchor(entry, protected_anchors)
        {
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

    if out.is_empty() {
        return out;
    }

    out.sort_by(|a, b| {
        a.base_level_b
            .total_cmp(&b.base_level_b)
            .then_with(|| a.event_id.cmp(&b.event_id))
    });

    let eligible_count = out.len();
    let cap = ((eligible_count as f64) * frac).ceil() as usize;
    let cap = cap.max(1).min(eligible_count);
    out.truncate(cap);
    out
}

#[cfg(test)]
mod tests {
    use monoagens_protocol::{
        ActorId, Audience, EmbeddingTag, OriginActor, PlaceId, TaggedEmbedding, Taint,
    };

    use super::*;
    use crate::activation::ActivationTrace;
    use crate::recall::{ImportanceInputs, MemoryBasis};

    fn make_stale_memory(id: u64, ts_ms: u64, concepts: Vec<&str>) -> MemoryEntry {
        let mut trace = ActivationTrace::default();
        trace.record(Time::from_millis(ts_ms));
        MemoryEntry {
            event_id: EventId::new(id),
            source_ids: vec![EventId::new(id)],
            is_gist: false,
            timestamp_ms: Time::from_millis(ts_ms),
            place: PlaceId::new("room:main"),
            actor: ActorId::new("alice"),
            origin_actor: OriginActor::new(ActorId::new("alice")),
            basis: MemoryBasis::Observed,
            taint: Taint::Trusted,
            audience: Audience::new([ActorId::new("alice"), ActorId::new("soul")]),
            attachments: Vec::new(),
            activation: trace,
            importance: ImportanceInputs::default(),
            embeddings: vec![TaggedEmbedding {
                tag: EmbeddingTag {
                    embed_model_ver: "egemma2-256".to_owned(),
                    dim_d: 4,
                    prompt_regime_id: "t_d".to_owned(),
                },
                values: vec![1.0, 0.0, 0.0, 0.0],
            }],
            concepts: concepts.into_iter().map(str::to_owned).collect(),
            superseded_by: None,
            erased: false,
        }
    }

    #[test]
    fn fade_guard_caps_per_cycle_pruning_and_protects_preference_anchors() {
        let cfg = CoreConfig::default();
        let mut memories = BTreeMap::new();
        // 12 very stale regular memories (ids 1..=12, older ids have lower B) + 2 anchors.
        for id in 1..=12u64 {
            memories.insert(
                EventId::new(id),
                make_stale_memory(id, id * 1_000, Vec::new()),
            );
        }
        // Concept-tagged durable preference anchor (id=100, oldest of all!)
        memories.insert(
            EventId::new(100),
            make_stale_memory(100, 100, vec![PREFERENCE_ANCHOR_CONCEPT]),
        );
        // Explicitly protected anchor (id=101, oldest of all!)
        memories.insert(EventId::new(101), make_stale_memory(101, 50, Vec::new()));

        let t_snapshot = Time::from_millis(45 * 86_400_000);
        let protected = BTreeSet::from([EventId::new(101)]);

        // 12 eligible -> ceil(12 * 0.25) = 3 lowest-B candidates; neither anchor 100 nor 101 fades.
        let cands = select_fade_candidates_guarded(
            &memories,
            t_snapshot,
            &cfg,
            &protected,
            DEFAULT_MAX_FADE_FRACTION,
        );
        assert_eq!(cands.len(), 3);
        assert!(cands.iter().all(|c| c.event_id != EventId::new(100)));
        assert!(cands.iter().all(|c| c.event_id != EventId::new(101)));
        // Lowest activation B (oldest timestamps: ids 1, 2, 3) chosen deterministically first.
        assert_eq!(
            cands.iter().map(|c| c.event_id).collect::<Vec<_>>(),
            vec![EventId::new(1), EventId::new(2), EventId::new(3)]
        );

        // Zero fraction releases nothing.
        let zero = select_fade_candidates_guarded(&memories, t_snapshot, &cfg, &protected, 0.0);
        assert!(zero.is_empty());

        // Single eligible item with 0.25 fraction still releases at least 1.
        let mut single = BTreeMap::new();
        single.insert(EventId::new(1), make_stale_memory(1, 1_000, Vec::new()));
        let single_cands = select_fade_candidates(&single, t_snapshot, &cfg);
        assert_eq!(single_cands.len(), 1);
        assert_eq!(single_cands[0].event_id, EventId::new(1));
    }
}
