//! Dual-process familiarity (`F`), episodic recollection (`R`), and provenance monitoring (`A-09`, `R-16`, `R-17`).

use monoagens_protocol::{
    ActorId, AttachmentHandle, Audience, EventId, OriginActor, PlaceId, TaggedEmbedding, Taint,
    Time,
};
use serde::{Deserialize, Serialize};

use crate::activation::{ActivationTrace, MS_PER_HOUR, sigmoid};
use crate::privacy::ScopedCandidate;
use crate::recall::importance::{ImportanceInputs, compute_importance};

/// Epistemic source monitoring basis (`R-16`, `R-17`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum MemoryBasis {
    Observed,
    #[default]
    Told,
    Inferred,
    SelfCaused,
}

impl MemoryBasis {
    #[must_use]
    pub const fn code(self) -> u8 {
        match self {
            Self::Observed => 1,
            Self::Told => 2,
            Self::Inferred => 3,
            Self::SelfCaused => 4,
        }
    }

    #[must_use]
    pub const fn from_code(code: u8) -> Option<Self> {
        match code {
            1 => Some(Self::Observed),
            2 => Some(Self::Told),
            3 => Some(Self::Inferred),
            4 => Some(Self::SelfCaused),
            _ => None,
        }
    }
}

/// Conditional context-match tag for `ContextPlan` rendering (`A-09`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "match_kind", rename_all = "snake_case")]
pub enum ContextMatchTag {
    Exact,
    Partial {
        matched_place: bool,
        matched_people: bool,
        matched_time: bool,
    },
    Remote,
    FamiliarOnly,
}

/// Provenance-bearing memory entry stored in Core's memory projection (`R-37`, `A-09`).
///
/// Never stores raw payload bytes; attachments stay opaque [`AttachmentHandle`]s.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryEntry {
    pub event_id: EventId,
    pub source_ids: Vec<EventId>,
    pub is_gist: bool,
    pub timestamp_ms: Time,
    pub place: PlaceId,
    pub actor: ActorId,
    pub origin_actor: OriginActor,
    pub basis: MemoryBasis,
    pub taint: Taint,
    pub audience: Audience,
    pub attachments: Vec<AttachmentHandle>,
    pub activation: ActivationTrace,
    pub importance: ImportanceInputs,
    pub embeddings: Vec<TaggedEmbedding>,
    pub concepts: Vec<String>,
    pub superseded_by: Option<EventId>,
    pub erased: bool,
}

impl MemoryEntry {
    /// Returns the effective importance `max(i_reason, i_commit, i_surprise)`.
    #[must_use]
    pub fn importance_score(&self) -> f64 {
        compute_importance(self.importance)
    }

    /// Returns `true` when not superseded and not erased.
    #[must_use]
    pub const fn is_current(&self) -> bool {
        self.superseded_by.is_none() && !self.erased
    }
}

impl ScopedCandidate for MemoryEntry {
    fn item_audience(&self) -> &Audience {
        &self.audience
    }

    fn item_event_id(&self) -> Option<EventId> {
        Some(self.event_id)
    }
}

/// Lightweight source event facts used to verify recollection `R` without requiring vectors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceEventFact {
    pub event_id: EventId,
    pub actor: ActorId,
    pub audience: Audience,
    pub place: PlaceId,
    pub timestamp_ms: Time,
    pub erased: bool,
}

/// Explicit target constraints for Stage-2 `memory.recall` recollection verification (`A-09`, `R-16`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct RecollectionConstraint {
    pub people: Vec<ActorId>,
    pub place: Option<PlaceId>,
    pub time_range: Option<(Time, Time)>,
}

/// Computes continuous dual-process familiarity `F in [0, 1]` from total activation `A` and
/// optional matching cosine similarity.
#[must_use]
pub fn evaluate_familiarity(total_activation: f64, cosine: Option<f64>) -> f64 {
    let scale = 1.0_f64;
    let fok_threshold = 0.25_f64;
    let a_norm = sigmoid((total_activation - fok_threshold) / scale);
    match cosine {
        Some(c) if c.is_finite() => c.clamp(0.0, 1.0).max(a_norm).clamp(0.0, 1.0),
        _ => a_norm.clamp(0.0, 1.0),
    }
}

/// Verifies episodic recollection `R` (`A-09`) by recovering non-erased source events
/// (even when their vectors have faded and been released) and checking optional `(people, place, time_grain)`.
#[must_use]
pub fn verify_recollection<F>(
    entry: &MemoryEntry,
    constraint: Option<&RecollectionConstraint>,
    mut resolve_source: F,
) -> bool
where
    F: FnMut(EventId) -> Option<SourceEventFact>,
{
    if entry.erased {
        return false;
    }

    let source_ids: &[EventId] = if entry.source_ids.is_empty() {
        std::slice::from_ref(&entry.event_id)
    } else {
        &entry.source_ids
    };

    for &src_id in source_ids {
        let Some(src) = resolve_source(src_id) else {
            continue;
        };
        if src.erased {
            continue;
        }
        let Some(c) = constraint else {
            return true;
        };
        let place_ok = c.place.as_ref().is_none_or(|p| &src.place == p);
        let time_ok = c
            .time_range
            .is_none_or(|(lo, hi)| src.timestamp_ms >= lo && src.timestamp_ms <= hi);
        let people_ok = c
            .people
            .iter()
            .all(|person| &src.actor == person || src.audience.contains(person));
        if place_ok && time_ok && people_ok {
            return true;
        }
    }

    false
}

/// Classifies the conditional context-match tag relative to the wake context (`A-09`).
#[must_use]
pub fn classify_context_match(
    entry: &MemoryEntry,
    recollected: bool,
    wake_place: Option<&PlaceId>,
    wake_people: &[ActorId],
    now_ms: Time,
    recent_window_hours: f64,
) -> ContextMatchTag {
    if !recollected {
        return ContextMatchTag::FamiliarOnly;
    }

    let matched_place = wake_place.is_some_and(|p| p == &entry.place);
    let matched_people = !wake_people.is_empty()
        && wake_people
            .iter()
            .any(|p| p == &entry.actor || entry.audience.contains(p));
    let dt_hours = now_ms.saturating_sub_ms(entry.timestamp_ms) as f64 / MS_PER_HOUR;
    let matched_time = dt_hours <= recent_window_hours.max(0.0);

    if matched_place && matched_people && matched_time {
        ContextMatchTag::Exact
    } else if matched_place || matched_people || matched_time {
        ContextMatchTag::Partial {
            matched_place,
            matched_people,
            matched_time,
        }
    } else {
        ContextMatchTag::Remote
    }
}
