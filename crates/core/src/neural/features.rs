//! Model-agnostic `18d` numeric feature extraction (`x_num`) + optional `10d` `FModal`,
//! computed strictly post-`VisibleScope` (`Delta_NI = 0`) with `NaN`/`Inf` sanitization.

use monoagens_protocol::{FModal, Percept, Time, VisibleScope};

use crate::neural::{DEFAULT_EMB_DIM, DEFAULT_NUM_FEATURES, NeuralModelConfig};
use crate::state::CoreState;

/// Feature schema version emitted by [`extract_x_num`].
pub const FEATURE_SCHEMA_VERSION: u16 = 1;

/// Raw inputs for constructing the bounded `18d` `x_num` feature vector.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct NumericFeatureInputs {
    pub is_named_soul: bool,
    pub is_explicit_addressee: bool,
    pub is_reply_or_expect_link: bool,
    pub is_dyadic_place: bool,
    pub active_participant_count: usize,
    pub dt_since_last_place_event_s: f64,
    pub dt_since_last_soul_act_s: f64,
    pub dt_since_sender_last_act_s: f64,
    pub soul_acts_window: usize,
    pub total_acts_window: usize,
    pub sender_exchange_count: usize,
    pub place_event_count: usize,
    pub budget_used_ratio: f64,
    pub imminent_promise_debt: f64,
    pub max_visible_mem_cos: f64,
    pub max_visible_mem_age_s: f64,
    pub pending_belief_uncertainty: f64,
    pub backlog_queue_depth: usize,
    pub asr_conf_or_one: f64,
}

#[inline]
fn sanitize_unit(val: f64, default: f32) -> f32 {
    if !val.is_finite() {
        return default;
    }
    (val as f32).clamp(0.0, 1.0)
}

#[inline]
fn log1p_norm(val: f64, max_scale: f64) -> f32 {
    if !val.is_finite() || val <= 0.0 {
        return 0.0;
    }
    sanitize_unit(val.ln_1p() / max_scale.ln_1p(), 0.0)
}

/// Computes normalized cosine similarity in `[0.0, 1.0]` (returns `0.0` for zero/mismatched/non-finite vectors).
#[must_use]
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f64 {
    if a.is_empty() || a.len() != b.len() {
        return 0.0;
    }
    let mut dot = 0.0_f64;
    let mut na = 0.0_f64;
    let mut nb = 0.0_f64;
    for (&x, &y) in a.iter().zip(b.iter()) {
        if !x.is_finite() || !y.is_finite() {
            return 0.0;
        }
        let xf = f64::from(x);
        let yf = f64::from(y);
        dot += xf * yf;
        na += xf * xf;
        nb += yf * yf;
    }
    let denom = na.sqrt() * nb.sqrt();
    if denom <= 1e-12 || !denom.is_finite() {
        0.0
    } else {
        (dot / denom).clamp(0.0, 1.0)
    }
}

/// Constructs the finite, `[0.0, 1.0]`-bounded `18d` `x_num` feature vector matching
/// `research/findings/run-5-notes/01_mechanism_and_loc_audit.md` §3 and Python `features.py`.
#[must_use]
pub fn extract_x_num(inp: &NumericFeatureInputs) -> [f32; DEFAULT_NUM_FEATURES] {
    let total_acts = inp.total_acts_window.max(1) as f64;
    let soul_share = sanitize_unit((inp.soul_acts_window as f64) / total_acts, 0.0);
    let sender_fam = sanitize_unit(1.0 - (-(inp.sender_exchange_count as f64) / 5.0).exp(), 0.0);
    let place_fam = sanitize_unit(1.0 - (-(inp.place_event_count as f64) / 20.0).exp(), 0.0);

    [
        if inp.is_named_soul { 1.0 } else { 0.0 },
        if inp.is_explicit_addressee { 1.0 } else { 0.0 },
        if inp.is_reply_or_expect_link { 1.0 } else { 0.0 },
        if inp.is_dyadic_place { 1.0 } else { 0.0 },
        log1p_norm(inp.active_participant_count as f64, 64.0),
        log1p_norm(inp.dt_since_last_place_event_s, 86_400.0),
        log1p_norm(inp.dt_since_last_soul_act_s, 86_400.0),
        log1p_norm(inp.dt_since_sender_last_act_s, 86_400.0),
        soul_share,
        sender_fam,
        place_fam,
        sanitize_unit(inp.budget_used_ratio, 0.0),
        sanitize_unit(inp.imminent_promise_debt, 0.0),
        sanitize_unit(inp.max_visible_mem_cos, 0.0),
        log1p_norm(inp.max_visible_mem_age_s, 2_592_000.0),
        sanitize_unit(inp.pending_belief_uncertainty, 0.0),
        log1p_norm(inp.backlog_queue_depth as f64, 256.0),
        sanitize_unit(inp.asr_conf_or_one, 1.0),
    ]
}

/// Sanitizes optional `10d` `FModal` sensor channels (`NaN`/`Inf` -> `0.0`, except channel `9` -> `1.0`).
#[must_use]
pub fn extract_fmodal(fmodal: Option<&FModal>) -> [f32; 10] {
    let mut out = [0.0_f32; 10];
    out[9] = 1.0;
    let Some(fm) = fmodal else {
        return out;
    };
    for (i, &v) in fm.0.iter().enumerate() {
        let default = if i == 9 { 1.0 } else { 0.0 };
        out[i] = if v.is_finite() { v } else { default };
    }
    out
}

/// Concatenates `18d` `x_num` with `10d` sanitized `FModal` (`28d` total).
#[must_use]
pub fn extract_x_num_with_fmodal(
    inp: &NumericFeatureInputs,
    fmodal: Option<&FModal>,
) -> [f32; 28] {
    let base = extract_x_num(inp);
    let modal = extract_fmodal(fmodal);
    let mut out = [0.0_f32; 28];
    out[..DEFAULT_NUM_FEATURES].copy_from_slice(&base);
    out[DEFAULT_NUM_FEATURES..].copy_from_slice(&modal);
    out
}

/// Extracts a finite embedding vector of length `d_in_emb` from `percept.embeddings`,
/// degrading cleanly to zeros (`[0.0; d_in_emb]`) when no matching embedding is present.
#[must_use]
pub fn extract_embedding_for_model(percept: &Percept, d_in_emb: usize) -> Vec<f32> {
    extract_embedding_matching(percept, d_in_emb, None, None)
}

/// Extracts a finite embedding vector matching `config.d_in_emb` and preferring embeddings whose
/// `(embed_model_ver, prompt_regime_id)` match `config`'s declared contract (`C-1`), falling back
/// to dimension-matching / first embedding or zeros when absent.
#[must_use]
pub fn extract_embedding_for_model_config(
    percept: &Percept,
    config: &NeuralModelConfig,
) -> Vec<f32> {
    extract_embedding_matching(
        percept,
        config.d_in_emb,
        config.expected_embed_model_ver.as_deref(),
        config.expected_prompt_regime_id.as_deref(),
    )
}

fn extract_embedding_matching(
    percept: &Percept,
    d_in_emb: usize,
    expected_ver: Option<&str>,
    expected_regime: Option<&str>,
) -> Vec<f32> {
    if d_in_emb == 0 {
        return Vec::new();
    }
    let exact_match = if expected_ver.is_some() || expected_regime.is_some() {
        percept.embeddings.iter().find(|e| {
            e.values.len() == d_in_emb
                && expected_ver.is_none_or(|v| e.tag.embed_model_ver == v)
                && expected_regime.is_none_or(|r| e.tag.prompt_regime_id == r)
        })
    } else {
        None
    };
    let candidate = exact_match
        .or_else(|| percept.embeddings.iter().find(|e| e.values.len() == d_in_emb))
        .or_else(|| percept.embeddings.first());
    let mut out = vec![0.0_f32; d_in_emb];
    if let Some(emb) = candidate {
        for (dst, &src) in out.iter_mut().zip(emb.values.iter()) {
            *dst = if src.is_finite() { src } else { 0.0 };
        }
    }
    out
}

/// Extracts the `18d` `x_num` vector from [`CoreState`] strictly post-[`VisibleScope`] (`Delta_NI = 0`).
///
/// Any memory entry, belief, or event outside `VisibleScope::new(percept.audience, Some(percept.place))`
/// is excluded before computing similarity, recency, cadence, familiarity, or uncertainty.
#[must_use]
pub fn extract_features_from_state(
    state: &CoreState,
    percept: &Percept,
    now: Time,
    backlog_queue_depth: usize,
) -> [f32; DEFAULT_NUM_FEATURES] {
    let scope = VisibleScope::new(percept.audience.clone(), Some(percept.place.clone()));
    let soul_id = &state.soul_actor_id;

    let is_named_soul = percept.named.iter().any(|a| a == soul_id);
    let is_explicit_addressee = percept.addressees.iter().any(|a| a == soul_id);

    // Check reply / expectation link strictly among expectations visible in `scope`.
    let is_reply_or_expect_link = percept.reply_to.is_some()
        || state.expectation_book.expectations.values().any(|exp| {
            exp.is_active()
                && exp.places.contains(&percept.place)
                && scope.allows(&exp.audience, Some(exp.expectation_id), now)
        });

    let non_soul_audience = percept
        .audience
        .members
        .iter()
        .filter(|a| *a != soul_id)
        .count();
    let is_dyadic_place = non_soul_audience <= 1;

    // Place-scoped cadence & counts (strictly keyed to percept.place + visible history).
    let place_history = state.place_history.get(&percept.place);
    let mut visible_place_events = 0usize;
    let mut visible_soul_acts = 0usize;
    let mut visible_sender_acts = 0usize;
    let mut last_visible_place_ms: Option<Time> = None;
    let mut last_visible_soul_ms: Option<Time> = None;
    let mut last_visible_sender_ms: Option<Time> = None;
    let mut visible_participants = std::collections::BTreeSet::new();

    if let Some(entries) = place_history {
        for entry in entries {
            if !scope.allows(&entry.audience, Some(entry.event_id), now) {
                continue;
            }
            visible_place_events += 1;
            visible_participants.insert(entry.actor.clone());
            last_visible_place_ms = Some(
                last_visible_place_ms.map_or(entry.timestamp_ms, |m| m.max(entry.timestamp_ms)),
            );
            if &entry.actor == soul_id {
                visible_soul_acts += 1;
                last_visible_soul_ms = Some(
                    last_visible_soul_ms.map_or(entry.timestamp_ms, |m| m.max(entry.timestamp_ms)),
                );
            }
            if entry.actor == percept.actor {
                visible_sender_acts += 1;
                last_visible_sender_ms = Some(
                    last_visible_sender_ms
                        .map_or(entry.timestamp_ms, |m| m.max(entry.timestamp_ms)),
                );
            }
        }
    }
    visible_participants.insert(percept.actor.clone());

    let dt_place_s = last_visible_place_ms
        .map_or(0.0, |t| now.saturating_sub_ms(t) as f64 / 1000.0);
    let dt_soul_s = last_visible_soul_ms
        .map_or(86_400.0, |t| now.saturating_sub_ms(t) as f64 / 1000.0);
    let dt_sender_s = last_visible_sender_ms
        .map_or(86_400.0, |t| now.saturating_sub_ms(t) as f64 / 1000.0);

    let budget_used_ratio = if state.daily_budget_limit > 0.0 && state.daily_budget_limit.is_finite()
    {
        (state.daily_budget_used / state.daily_budget_limit).clamp(0.0, 1.0)
    } else {
        0.0
    };

    // Imminent promise debt strictly across visible active promises at this place.
    let mut imminent_debt = 0.0_f64;
    for exp in state.expectation_book.expectations.values() {
        if !exp.is_active()
            || !matches!(exp.kind, monoagens_protocol::ExpectKind::Promise)
            || !scope.allows(&exp.audience, Some(exp.expectation_id), now)
        {
            continue;
        }
        let rem_ms = exp.due_ms.saturating_sub_ms(now) as f64;
        let urgency = (1.0 / (1.0 + rem_ms / 60_000.0)).clamp(0.0, 1.0);
        imminent_debt = imminent_debt.max(urgency);
    }

    // Strictly VisibleScope-filtered memory cosine & recency.
    let mut max_vis_cos = 0.0_f64;
    let mut max_vis_age_s = 0.0_f64;
    for mem in state.memory_entries.values() {
        if mem.superseded_by.is_some() {
            continue;
        }
        if !scope.allows(&mem.audience, Some(mem.event_id), now) {
            continue;
        }
        let age_s = now.saturating_sub_ms(mem.timestamp_ms) as f64 / 1000.0;
        if let Some(q_emb) = percept.embeddings.first() {
            for m_emb in &mem.embeddings {
                if q_emb.tag == m_emb.tag {
                    let c = cosine_similarity(&q_emb.values, &m_emb.values);
                    if c > max_vis_cos {
                        max_vis_cos = c;
                        max_vis_age_s = age_s;
                    }
                }
            }
        } else if max_vis_age_s == 0.0 || age_s < max_vis_age_s {
            max_vis_age_s = age_s;
        }
    }

    // Strictly VisibleScope-filtered pending belief uncertainty.
    let mut max_voi = 0.0_f64;
    for belief in state.belief_book.beliefs.values() {
        if !scope.allows(&belief.audience, Some(belief.belief_id), now) {
            continue;
        }
        let var = belief
            .credence
            .as_ref()
            .map_or(0.0, |c| c.variance().clamp(0.0, 0.25) * 4.0);
        max_voi = max_voi.max(var);
    }

    let asr_conf = percept
        .f_modal
        .as_ref()
        .map_or(1.0, |fm| {
            let v = f64::from(fm.0[9]);
            if v.is_finite() { v.clamp(0.0, 1.0) } else { 1.0 }
        });

    let inputs = NumericFeatureInputs {
        is_named_soul,
        is_explicit_addressee,
        is_reply_or_expect_link,
        is_dyadic_place,
        active_participant_count: visible_participants.len().max(non_soul_audience + 1),
        dt_since_last_place_event_s: dt_place_s,
        dt_since_last_soul_act_s: dt_soul_s,
        dt_since_sender_last_act_s: dt_sender_s,
        soul_acts_window: visible_soul_acts,
        total_acts_window: visible_place_events.max(1),
        sender_exchange_count: visible_sender_acts,
        place_event_count: visible_place_events,
        budget_used_ratio,
        imminent_promise_debt: imminent_debt,
        max_visible_mem_cos: max_vis_cos,
        max_visible_mem_age_s: max_vis_age_s,
        pending_belief_uncertainty: max_voi,
        backlog_queue_depth,
        asr_conf_or_one: asr_conf,
    };
    extract_x_num(&inputs)
}

/// Helper returning default `256d` zero embedding for vectorless inputs.
#[must_use]
pub fn zero_embedding() -> Vec<f32> {
    vec![0.0_f32; DEFAULT_EMB_DIM]
}

#[cfg(test)]
mod tests {
    use monoagens_protocol::{
        ActorId, Audience, EmbeddingTag, OpaquePayload, OriginActor, PlaceId, TaggedEmbedding,
        Taint,
    };

    use super::*;
    use crate::activation::ActivationTrace;
    use crate::recall::{MemoryBasis, MemoryEntry};

    #[test]
    fn sanitizes_non_finite_and_bounds_x_num_and_fmodal() {
        let vec = extract_x_num(&NumericFeatureInputs {
            is_named_soul: true,
            dt_since_last_place_event_s: f64::NAN,
            budget_used_ratio: f64::INFINITY,
            imminent_promise_debt: -3.0,
            max_visible_mem_cos: 2.5,
            asr_conf_or_one: f64::NEG_INFINITY,
            ..NumericFeatureInputs::default()
        });
        assert_eq!(vec[0], 1.0);
        assert_eq!(vec[17], 1.0);
        for v in vec {
            assert!(v.is_finite() && (0.0..=1.0).contains(&v));
        }

        let fm = FModal([
            f32::NAN,
            f32::INFINITY,
            0.25,
            -0.5,
            0.0,
            0.1,
            0.2,
            0.3,
            0.4,
            f32::NAN,
        ]);
        let full = extract_x_num_with_fmodal(&NumericFeatureInputs::default(), Some(&fm));
        assert_eq!(full[18], 0.0);
        assert_eq!(full[19], 0.0);
        assert_eq!(full[20], 0.25);
        assert_eq!(full[27], 1.0);
    }

    #[test]
    fn vectorless_percept_degrades_to_zero_cosine_and_zero_embedding() {
        let mut p = Percept::minimal(
            ActorId::new("alice"),
            PlaceId::new("room:a"),
            Audience::new([ActorId::new("alice"), ActorId::new("soul")]),
            Time::from_millis(1000),
            OpaquePayload::from_bytes(b"hi"),
        );
        p.embeddings.clear();
        let emb = extract_embedding_for_model(&p, 256);
        assert_eq!(emb.len(), 256);
        assert!(emb.iter().all(|&x| x == 0.0));

        let state = CoreState::new();
        let x = extract_features_from_state(&state, &p, Time::from_millis(1000), 0);
        assert_eq!(x[13], 0.0); // max_visible_mem_cos
    }

    #[test]
    fn private_memory_never_leaks_into_extracted_features_delta_ni_zero() {
        let tag = EmbeddingTag {
            embed_model_ver: "egemma2-256".to_owned(),
            dim_d: 256,
            prompt_regime_id: "t_d".to_owned(),
        };
        let mut state_with_priv = CoreState::new();
        let state_without_priv = CoreState::new();

        state_with_priv.upsert_memory_entry(MemoryEntry {
            event_id: monoagens_protocol::EventId::new(99),
            source_ids: vec![monoagens_protocol::EventId::new(99)],
            timestamp_ms: Time::from_millis(500),
            place: PlaceId::new("dm:alice"),
            actor: ActorId::new("alice"),
            origin_actor: OriginActor::new(ActorId::new("alice")),
            basis: MemoryBasis::Observed,
            taint: Taint::Untrusted,
            audience: Audience::new([ActorId::new("alice"), ActorId::new("soul")]),
            embeddings: vec![TaggedEmbedding {
                tag: tag.clone(),
                values: vec![1.0; 256],
            }],
            attachments: Vec::new(),
            activation: ActivationTrace::default(),
            superseded_by: None,
            is_gist: false,
            importance: crate::recall::ImportanceInputs {
                i_reason: 0.5,
                ..crate::recall::ImportanceInputs::default()
            },
            concepts: Vec::new(),
            erased: false,
        });

        let mut pub_percept = Percept::minimal(
            ActorId::new("bob"),
            PlaceId::new("room:public"),
            Audience::new([ActorId::new("bob"), ActorId::new("soul")]),
            Time::from_millis(2000),
            OpaquePayload::from_bytes(b"hello"),
        );
        pub_percept.embeddings.push(TaggedEmbedding {
            tag,
            values: vec![1.0; 256],
        });

        let f_with = extract_features_from_state(
            &state_with_priv,
            &pub_percept,
            Time::from_millis(2000),
            0,
        );
        let f_without = extract_features_from_state(
            &state_without_priv,
            &pub_percept,
            Time::from_millis(2000),
            0,
        );
        assert_eq!(f_with, f_without);
        assert_eq!(f_with[13], 0.0);
    }

    #[test]
    fn extract_embedding_for_model_config_prefers_matching_regime_and_version() {
        let mut p = Percept::minimal(
            ActorId::new("alice"),
            PlaceId::new("room:a"),
            Audience::new([ActorId::new("alice"), ActorId::new("soul")]),
            Time::from_millis(1_000),
            OpaquePayload::from_bytes(b"halo"),
        );
        p.embeddings = vec![
            TaggedEmbedding {
                tag: EmbeddingTag {
                    embed_model_ver: "egemma2-256".to_owned(),
                    dim_d: 4,
                    prompt_regime_id: "t_q".to_owned(),
                },
                values: vec![0.1, 0.2, 0.3, 0.4],
            },
            TaggedEmbedding {
                tag: EmbeddingTag {
                    embed_model_ver: "egemma2-256".to_owned(),
                    dim_d: 4,
                    prompt_regime_id: "t_d".to_owned(),
                },
                values: vec![0.9, 0.8, 0.7, 0.6],
            },
        ];
        let cfg = NeuralModelConfig {
            feature_schema_version: 1,
            d_in_emb: 4,
            d_in_num: 18,
            plr_n_frequencies: 8,
            d_e: 8,
            d_n: 8,
            d_model: 16,
            d_ff: 32,
            n_layers: 1,
            tabm_k: 2,
            h_dim: 8,
            cell_type: crate::neural::CellType::MinGru,
            temperature: 1.0,
            cic_threshold: 0.5,
            max_wait_ms: 30_000.0,
            has_phase2_heads: false,
            has_phase3_heads: false,
            expected_embed_model_ver: Some("egemma2-256".to_owned()),
            expected_prompt_regime_id: Some("t_d".to_owned()),
        };
        let picked = extract_embedding_for_model_config(&p, &cfg);
        assert_eq!(picked, vec![0.9, 0.8, 0.7, 0.6]);
    }
}
