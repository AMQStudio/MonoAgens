//! Deterministic gate signals (`s_k in [0, 1]`), noisy-OR `deg(e, l)`, and `social_cost`.
//!
//! Every signal evaluates deterministically from opaque IDs, numeric features, and
//! embeddings without word lists or LLM calls (`A-14`, `R-29`, `R-33`, `R-37`).
//! Vector-dependent signals degrade to neutral `0.0` when embeddings are absent while
//! structural cues still decide.

use std::collections::{BTreeMap, BTreeSet};

use monoagens_protocol::{
    ActorId, Audience, EmbeddingTag, EventId, FModal, OrientScore, TaggedEmbedding, Time,
    VisibleScope,
};
use serde::{Deserialize, Serialize};

use crate::activation::{ActivationTrace, closeness_from_trace, sigmoid};
use crate::gate::drives::{PendingBeliefSummary, max_belief_voi};
use crate::prior::Prior;
use crate::privacy::{ScopedCandidate, project_visible_scope};

/// Frozen `A-14` logistic `dialogue_act_gap` head `(w_gap, b_gap)` supplied by `reason`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FrozenDialogueActHead {
    pub w_gap: Vec<f32>,
    pub b_gap: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_tag: Option<EmbeddingTag>,
}

impl FrozenDialogueActHead {
    /// Evaluates `sigma(w_gap . e + b_gap)` against the first matching embedding in `embeddings`.
    /// Returns `None` when no matching-dimension (and matching-tag, if configured) embedding exists.
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

/// Candidate memory record projected through `VisibleScope` prior to FOK/relevance scoring.
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

/// Active expectation cue projected through `VisibleScope` prior to relevance/link scoring.
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

/// Bounded place history entry strictly preceding the current event (`R-29`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlaceHistoryEntry {
    pub event_id: EventId,
    pub actor: ActorId,
    pub timestamp_ms: Time,
    pub deg_by_actor: BTreeMap<ActorId, f64>,
}

/// Inputs required to evaluate all gate signals for an incoming event.
#[derive(Debug, Clone)]
pub struct SignalInputs<'a> {
    pub soul_id: &'a ActorId,
    pub sender: &'a ActorId,
    pub timestamp_ms: Time,
    pub audience: &'a Audience,
    pub addressees: &'a [ActorId],
    pub named: &'a [ActorId],
    pub orient: &'a [OrientScore],
    pub reply_to_actor: Option<&'a ActorId>,
    pub f_modal: Option<FModal>,
    pub embeddings: &'a [TaggedEmbedding],
    pub precomputed_dialogue_act_gap: Option<f32>,
    pub is_dyadic_chat: bool,
    pub answered: f64,
    pub sender_exchange_trace: ActivationTrace,
    pub place_history: &'a [PlaceHistoryEntry],
    pub active_participants: &'a [ActorId],
    pub visible_scope: &'a VisibleScope,
    pub candidate_memories: &'a [CandidateMemory],
    pub expectation_cues: &'a [ExpectationCue],
    pub pending_beliefs: &'a [PendingBeliefSummary],
    pub dialogue_act_head: Option<&'a FrozenDialogueActHead>,
    pub is_idle_probe: bool,
    pub curiosity_override: Option<f64>,
}

/// Listener-wise continuous noisy-OR addressivity trace (`R-29`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct DegEvaluation {
    pub deg_by_actor: BTreeMap<ActorId, f64>,
    pub deg_soul: f64,
    pub max_deg_other: f64,
    pub sum_deg_listeners: f64,
    pub sel_other: f64,
}

/// Complete deterministic signal breakdown stored on every recorded gate decision.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct GateSignals {
    pub own_name: f64,
    pub dialogue_act_gap: f64,
    pub asr_clarification_boost: f64,
    pub info_gap: f64,
    pub own_info_gap: f64,
    pub feeling_of_knowing: f64,
    pub relevance: f64,
    pub commitment: f64,
    pub deg_soul: f64,
    pub max_deg_other: f64,
    pub sum_deg_listeners: f64,
    pub answered: f64,
    pub closeness_sender: f64,
    pub social_cost: f64,
}

/// Computes cosine similarity between two equal-length vectors, returning `None` if empty,
/// mismatched in length, zero-norm, or non-finite.
#[must_use]
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> Option<f64> {
    if a.is_empty() || a.len() != b.len() {
        return None;
    }
    let mut dot = 0.0_f64;
    let mut norm_a = 0.0_f64;
    let mut norm_b = 0.0_f64;
    for (&x, &y) in a.iter().zip(b.iter()) {
        let xf = f64::from(x);
        let yf = f64::from(y);
        if !xf.is_finite() || !yf.is_finite() {
            return None;
        }
        dot += xf * yf;
        norm_a += xf * xf;
        norm_b += yf * yf;
    }
    let denom = norm_a.sqrt() * norm_b.sqrt();
    if denom <= 0.0 || !denom.is_finite() {
        return None;
    }
    Some((dot / denom).clamp(-1.0, 1.0))
}

fn max_matching_cosine(a: &[TaggedEmbedding], b: &[TaggedEmbedding]) -> Option<f64> {
    let mut best: Option<f64> = None;
    for ea in a {
        for eb in b {
            if ea.tag == eb.tag
                && let Some(sim) = cosine_similarity(&ea.values, &eb.values)
            {
                best = Some(best.map_or(sim, |prev| prev.max(sim)));
            }
        }
    }
    best
}

/// Computes continuous noisy-OR addressivity `deg(e, l) in [0, 1]` for every active participant
/// `l != sender` (`R-29`).
#[must_use]
#[allow(clippy::too_many_arguments)]
pub fn compute_deg(
    soul_id: &ActorId,
    sender: &ActorId,
    timestamp_ms: Time,
    audience: &Audience,
    addressees: &[ActorId],
    named: &[ActorId],
    orient: &[OrientScore],
    reply_to_actor: Option<&ActorId>,
    visible_expectations: &[&ExpectationCue],
    place_history: &[PlaceHistoryEntry],
    active_participants: &[ActorId],
    is_dyadic_chat: bool,
    prior: &Prior,
) -> DegEvaluation {
    let mut listeners: BTreeSet<ActorId> = BTreeSet::new();
    listeners.insert(soul_id.clone());
    for a in active_participants {
        listeners.insert(a.clone());
    }
    for a in &audience.members {
        listeners.insert(a.clone());
    }
    for a in addressees {
        listeners.insert(a.clone());
    }
    for a in named {
        listeners.insert(a.clone());
    }
    for o in orient {
        listeners.insert(o.actor.clone());
    }
    for entry in place_history {
        listeners.insert(entry.actor.clone());
    }
    listeners.remove(sender);

    let total_active = listeners.len() + 1;
    let window_start = place_history.len().saturating_sub(prior.w_c_window);
    let history_slice = &place_history[window_start..];

    // Precompute C_p(sender, v) for every listener v in the active set.
    let mut c_map: BTreeMap<ActorId, f64> = BTreeMap::new();
    let other_denom = (total_active.saturating_sub(1)).max(1) as f64;
    for v in &listeners {
        let mut c_val = 0.0_f64;
        for (rev_idx, entry) in history_slice.iter().rev().enumerate() {
            let delta_n = (rev_idx + 1) as i32;
            let delta_t_s = timestamp_ms.saturating_sub_ms(entry.timestamp_ms) as f64 / 1000.0;
            let decay = prior.gamma_c.powi(delta_n) * (-delta_t_s / prior.tau_c_s).exp();

            let r = if &entry.actor == sender {
                let d_l = entry.deg_by_actor.get(v).copied().unwrap_or(0.0);
                if d_l >= prior.theta_trace { d_l } else { 0.0 }
            } else if &entry.actor == v {
                let d_s = entry.deg_by_actor.get(sender).copied().unwrap_or(0.0);
                if d_s >= prior.theta_trace {
                    d_s
                } else {
                    let max_deg = entry.deg_by_actor.values().copied().fold(0.0_f64, f64::max);
                    if delta_n <= 2 && max_deg < prior.theta_trace {
                        1.0 / other_denom
                    } else {
                        0.0
                    }
                }
            } else {
                0.0
            };

            c_val += decay * r;
        }
        c_map.insert(v.clone(), c_val);
    }

    let total_c: f64 = c_map.values().sum();
    let mut deg_by_actor = BTreeMap::new();

    for l in &listeners {
        let explicit = if addressees.contains(l) || (is_dyadic_chat && total_active == 2) {
            1.0
        } else {
            0.0
        };

        let reply_link = reply_to_actor == Some(l);
        let expect_link = visible_expectations
            .iter()
            .any(|exp| exp.answers_event && exp.counterparty.as_ref() == Some(l));
        let link = if reply_link || expect_link { 1.0 } else { 0.0 };

        let c_sl = c_map.get(l).copied().unwrap_or(0.0);
        let cont = if c_sl <= 0.0 {
            0.0
        } else {
            let c_others = (total_c - c_sl).max(0.0);
            let denom = prior.k_c + c_sl + prior.beta_fan * c_others;
            if denom > 0.0 {
                (c_sl / denom).clamp(0.0, 1.0)
            } else {
                0.0
            }
        };

        let orient_val = orient.iter().find(|o| &o.actor == l).map_or(0.0, |o| {
            let v = f64::from(o.score);
            if v.is_finite() {
                v.clamp(0.0, 1.0)
            } else {
                0.0
            }
        });

        let deg = 1.0
            - (1.0 - explicit)
                * (1.0 - link)
                * (1.0 - prior.w_c * cont)
                * (1.0 - prior.w_o * orient_val);
        deg_by_actor.insert(l.clone(), deg.clamp(0.0, 1.0));
    }

    let deg_soul = deg_by_actor.get(soul_id).copied().unwrap_or(0.0);
    let max_deg_other = deg_by_actor
        .iter()
        .filter(|(actor, _)| *actor != soul_id)
        .map(|(_, &d)| d)
        .fold(0.0_f64, f64::max);
    let sum_deg_listeners: f64 = deg_by_actor.values().sum();
    let sel_other = (max_deg_other - deg_soul).clamp(0.0, 1.0);

    DegEvaluation {
        deg_by_actor,
        deg_soul,
        max_deg_other,
        sum_deg_listeners,
        sel_other,
    }
}

/// Computes `social_cost` per `R-02` + `R-29`:
/// `max(clamp(max_{l != soul} deg(e, l) - deg(e, soul), 0, 1), answered) * (1 - lambda_c * closeness(sender))`.
#[must_use]
pub fn compute_social_cost(
    sel_other: f64,
    answered: f64,
    closeness_sender: f64,
    lambda_c: f64,
) -> f64 {
    let sel = if sel_other.is_finite() {
        sel_other.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let ans = if answered.is_finite() {
        answered.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let close = if closeness_sender.is_finite() {
        closeness_sender.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let lc = if lambda_c.is_finite() {
        lambda_c.clamp(0.0, 1.0)
    } else {
        0.0
    };
    (sel.max(ans) * (1.0 - lc * close)).clamp(0.0, 1.0)
}

/// Evaluates all gate signals deterministically in `[0, 1]`.
///
/// Enforces pre-Step-5 `VisibleScope` projection BEFORE computing `feeling_of_knowing`
/// and `relevance` (`R-33`, `Δ_NI = 0`).
#[must_use]
pub fn evaluate_signals(inputs: &SignalInputs<'_>, prior: &Prior) -> (GateSignals, DegEvaluation) {
    let d = prior.petrov_d();

    // 1. Pre-Step-5 VisibleScope projection BEFORE any FOK / relevance / link inspection.
    let visible_memories = project_visible_scope(
        inputs.visible_scope,
        inputs.candidate_memories,
        inputs.timestamp_ms,
    );
    let visible_expectations = project_visible_scope(
        inputs.visible_scope,
        inputs.expectation_cues,
        inputs.timestamp_ms,
    );

    // 2. Graded addressivity deg(e, l) and sender closeness.
    let deg_eval = compute_deg(
        inputs.soul_id,
        inputs.sender,
        inputs.timestamp_ms,
        inputs.audience,
        inputs.addressees,
        inputs.named,
        inputs.orient,
        inputs.reply_to_actor,
        &visible_expectations,
        inputs.place_history,
        inputs.active_participants,
        inputs.is_dyadic_chat,
        prior,
    );
    let closeness_sender = closeness_from_trace(
        &inputs.sender_exchange_trace,
        inputs.timestamp_ms,
        d,
        prior.closeness_prior_count,
    );
    let answered = if inputs.answered.is_finite() {
        inputs.answered.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let social_cost = compute_social_cost(
        deg_eval.sel_other,
        answered,
        closeness_sender,
        prior.lambda_c,
    );

    // 3. own_name
    let own_name = if inputs.named.contains(inputs.soul_id) {
        1.0
    } else {
        0.0
    };

    // 4. dialogue_act_gap (A-14 frozen logistic head) + ASR clarification boost (R-37).
    let dialogue_act_gap = inputs
        .dialogue_act_head
        .and_then(|head| head.score(inputs.embeddings))
        .or_else(|| {
            inputs.precomputed_dialogue_act_gap.and_then(|v| {
                let vf = f64::from(v);
                if vf.is_finite() {
                    Some(vf.clamp(0.0, 1.0))
                } else {
                    None
                }
            })
        })
        .unwrap_or(0.0);

    let asr_clarification_boost = match inputs.f_modal {
        Some(FModal(vals)) => {
            let speech_active = vals[1];
            let asr_conf = vals[9];
            let has_speech_or_asr = speech_active > 0.0 || asr_conf > 0.0;
            if has_speech_or_asr
                && asr_conf.is_finite()
                && f64::from(asr_conf) < prior.asr_conf_threshold
            {
                prior.asr_clarification_boost
            } else {
                0.0
            }
        }
        None => 0.0,
    };

    // 5. feeling_of_knowing (A* over VisibleScope-filtered memories only) and relevance.
    let mut best_a: Option<f64> = None;
    let mut best_rel: f64 = 0.0;

    if !inputs.embeddings.is_empty() {
        for mem in &visible_memories {
            if let Some(cos) = max_matching_cosine(inputs.embeddings, &mem.embeddings) {
                let pos_cos = cos.max(0.0);
                if pos_cos > best_rel {
                    best_rel = pos_cos;
                }
                let b = mem.activation.base_level(inputs.timestamp_ms, d);
                let imp = if mem.importance.is_finite() {
                    mem.importance.clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let act =
                    prior.w_b * b + prior.context_weight * cos + prior.importance_weight * imp;
                best_a = Some(best_a.map_or(act, |prev| prev.max(act)));
            }
        }

        for exp in &visible_expectations {
            if let Some(cos) = max_matching_cosine(inputs.embeddings, &exp.embeddings) {
                let pos_cos = cos.max(0.0);
                if pos_cos > best_rel {
                    best_rel = pos_cos;
                }
            }
        }
    }

    let feeling_of_knowing = match best_a {
        Some(a_star) => {
            sigmoid((a_star - prior.fok_threshold) / prior.fok_scale_norm).clamp(0.0, 1.0)
        }
        None => 0.0,
    };
    let relevance = best_rel.clamp(0.0, 1.0);

    // 6. own_info_gap (low Beta credence on pending beliefs, R-31 / A-14).
    let own_info_gap = inputs
        .curiosity_override
        .unwrap_or_else(|| max_belief_voi(inputs.pending_beliefs))
        .clamp(0.0, 1.0);

    let info_gap = if inputs.is_idle_probe {
        own_info_gap
    } else {
        (dialogue_act_gap + asr_clarification_boost).clamp(0.0, 1.0)
    };

    // 7. unallocated commitment cue (R-29 L96).
    let expectation_hit = visible_expectations.iter().any(|e| e.answers_event);
    let commitment = if expectation_hit {
        1.0
    } else {
        deg_eval.deg_soul.clamp(0.0, 1.0)
    };

    let signals = GateSignals {
        own_name,
        dialogue_act_gap,
        asr_clarification_boost,
        info_gap,
        own_info_gap,
        feeling_of_knowing,
        relevance,
        commitment,
        deg_soul: deg_eval.deg_soul,
        max_deg_other: deg_eval.max_deg_other,
        sum_deg_listeners: deg_eval.sum_deg_listeners,
        answered,
        closeness_sender,
        social_cost,
    };

    (signals, deg_eval)
}
