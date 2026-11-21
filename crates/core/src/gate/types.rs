//! Deterministic gate helper types preserved in Slim Core V2 (`FrozenDialogueActHead`,
//! `CandidateMemory`, `ExpectationCue`, `PlaceHistoryEntry`, `OpenWait`, `PromiseSummary`).

use std::collections::BTreeMap;

use monoagens_protocol::{
    ActorId, Audience, ChannelId, EmbeddingTag, EventId, PlaceId, TaggedEmbedding, Time,
};
use serde::{Deserialize, Serialize};

use crate::activation::{ActivationTrace, sigmoid};
use crate::event::{EventDetail, EventRecord};
use crate::privacy::ScopedCandidate;

/// Optional frozen logistic head over embeddings for dialogue-act / info-gap estimation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FrozenDialogueActHead {
    pub w_gap: Vec<f32>,
    pub b_gap: f32,
    pub expected_tag: Option<EmbeddingTag>,
}

impl FrozenDialogueActHead {
    /// Evaluates `sigma(w_gap . e + b_gap)` against the first matching embedding in `embeddings`.
    #[must_use]
    pub fn score(&self, embeddings: &[TaggedEmbedding]) -> Option<f64> {
        if self.w_gap.is_empty() || !self.b_gap.is_finite() {
            return None;
        }
        let target = embeddings.iter().find(|emb| {
            emb.values.len() == self.w_gap.len()
                && self.expected_tag.as_ref().is_none_or(|tag| &emb.tag == tag)
        })?;

        let mut dot = 0.0_f64;
        for (&w, &x) in self.w_gap.iter().zip(target.values.iter()) {
            let wf = f64::from(w);
            let xf = f64::from(x);
            if !wf.is_finite() || !xf.is_finite() {
                return None;
            }
            dot += wf * xf;
        }
        let z = dot + f64::from(self.b_gap);
        Some(sigmoid(z).clamp(0.0, 1.0))
    }
}

/// Candidate memory entry stored for `VisibleScope` gating and context projections.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CandidateMemory {
    pub event_id: EventId,
    pub audience: Audience,
    pub activation: ActivationTrace,
    pub importance: f64,
    pub embeddings: Vec<TaggedEmbedding>,
}

impl ScopedCandidate for CandidateMemory {
    fn item_audience(&self) -> &Audience {
        &self.audience
    }

    fn item_event_id(&self) -> Option<EventId> {
        Some(self.event_id)
    }
}

/// Open expectation cue used to project visible expectations (`Delta_NI = 0`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExpectationCue {
    pub expectation_id: Option<EventId>,
    pub audience: Audience,
    pub counterparty: Option<ActorId>,
    pub answers_event: bool,
    pub embeddings: Vec<TaggedEmbedding>,
}

impl ScopedCandidate for ExpectationCue {
    fn item_audience(&self) -> &Audience {
        &self.audience
    }

    fn item_event_id(&self) -> Option<EventId> {
        self.expectation_id
    }
}

/// Recent place event summary kept in bounded `place_history` per place.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlaceHistoryEntry {
    pub event_id: EventId,
    pub actor: ActorId,
    pub audience: Audience,
    pub timestamp_ms: Time,
    pub deg_by_actor: BTreeMap<ActorId, f64>,
}

/// Compact summary of an active promise / monitor used for commitment / debt projections.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PromiseSummary {
    pub due_ms: Time,
    pub urgency_weight: f64,
    pub is_recurring_monitor: bool,
    pub active: bool,
}

/// Active `wait` expectation opened when the Neural Controller chooses `Wait` (or unallocated `Go` yield).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenWait {
    pub expectation_id: EventId,
    pub target_event_id: EventId,
    pub place: PlaceId,
    pub sender: ActorId,
    pub medium: ChannelId,
    pub audience: Audience,
    pub opened_at_ms: Time,
    pub due_ms: Time,
}

impl OpenWait {
    /// Returns `true` if `percept` is a third-party answer to this open `wait` within the window.
    #[must_use]
    pub fn matches_third_party_reply(&self, percept: &EventRecord, soul_id: &ActorId) -> bool {
        if percept.place != self.place {
            return false;
        }
        if &percept.actor == soul_id || percept.actor == self.sender {
            return false;
        }
        if percept.timestamp_ms > self.due_ms {
            return false;
        }
        match &percept.detail {
            Some(EventDetail::PerceptMeta {
                addressees,
                reply_to,
                ..
            }) => {
                *reply_to == Some(self.target_event_id)
                    || addressees.contains(&self.sender)
                    || (reply_to.is_none() && addressees.is_empty())
            }
            _ => percept.caused_by.contains(&self.target_event_id) || percept.caused_by.is_empty(),
        }
    }
}

/// Safe cosine similarity over two equal-length `f32` vectors (`None` if empty, mismatched, zero-norm, or non-finite).
#[must_use]
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> Option<f64> {
    if a.is_empty() || a.len() != b.len() {
        return None;
    }
    let mut dot = 0.0_f64;
    let mut na = 0.0_f64;
    let mut nb = 0.0_f64;
    for (&x, &y) in a.iter().zip(b.iter()) {
        let xf = f64::from(x);
        let yf = f64::from(y);
        if !xf.is_finite() || !yf.is_finite() {
            return None;
        }
        dot += xf * yf;
        na += xf * xf;
        nb += yf * yf;
    }
    let denom = (na * nb).sqrt();
    if denom <= 1e-12 || !denom.is_finite() {
        None
    } else {
        Some((dot / denom).clamp(-1.0, 1.0))
    }
}
