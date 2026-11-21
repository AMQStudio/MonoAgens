//! Model-agnostic numeric feature extraction (`schema = 1`: `18d` `x_num`; `schema = 2`: `32d`
//! `x_num` = `18d` v1 core + `6d` repair + `4d` thread/actor + `4d` peer-agent) + optional `10d`
//! `FModal`, computed strictly post-`VisibleScope` (`Delta_NI = 0`) with `NaN`/`Inf` sanitization.

use std::collections::BTreeSet;

use monoagens_protocol::{
    ActorId, Audience, DomainId, EventId, ExpectKind, FModal, Percept, PlaceId, TaggedEmbedding,
    Time, VisibleScope,
};

use crate::consolidation::EpisodeState;
use crate::neural::{DEFAULT_EMB_DIM, DEFAULT_NUM_FEATURES, NeuralModelConfig};
use crate::state::CoreState;

/// Legacy schema v1 identifier emitted by [`extract_x_num`].
pub const FEATURE_SCHEMA_VERSION: u16 = 1;

/// Feature schema version 1 (`18d` `x_num`).
pub const FEATURE_SCHEMA_VERSION_V1: u16 = 1;

/// Feature schema version 2 (`32d` `x_num`: `18d` v1 + `6d` repair + `4d` thread/actor + `4d` peer-agent).
pub const FEATURE_SCHEMA_VERSION_V2: u16 = 2;

/// Numeric feature vector width for schema v1 (`18d`).
pub const DEFAULT_NUM_FEATURES_V1: usize = DEFAULT_NUM_FEATURES;

/// Numeric feature vector width for schema v2 (`32d`).
pub const DEFAULT_NUM_FEATURES_V2: usize = 32;

/// Recency window (`ms`) for `has_recent_visible_soul_act` and Loop A repair lookup (`10 min`).
pub const RECENT_SOUL_REPAIR_WINDOW_MS: u64 = 600_000;

/// Computes the canonical deterministic `BLAKE3` hash over `\n`-joined sorted unique actor IDs
/// in `audience`, shared by audience-scoped `h_actor` and multi-slot `EpisodeBook` keys (`Delta_NI = 0`).
#[must_use]
pub fn canonical_audience_hash(audience: &Audience) -> [u8; 32] {
    let mut actors: Vec<&str> = audience.members.iter().map(ActorId::as_str).collect();
    actors.sort_unstable();
    actors.dedup();
    let joined = actors.join("\n");
    crate::blake3::hash(joined.as_bytes())
}

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

/// Raw inputs for constructing the bounded `32d` `schema = 2` `x_num` feature vector.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct NumericFeatureInputsV2 {
    pub base: NumericFeatureInputs,
    // 18..24 (6d repair, R-68)
    pub has_recent_visible_soul_act: bool,
    pub cos_soul_user: f64,
    pub cos_prev_user_rephrase: f64,
    pub cos_expected_reply: f64,
    pub is_reply_to_last_soul: bool,
    pub soul_intervened_since_sender_last: bool,
    // 24..28 (4d thread/actor, R-69)
    pub max_active_thread_cos: f64,
    pub active_thread_count: usize,
    pub sender_in_top_thread: bool,
    pub sender_reliability: f64,
    // 28..32 (4d peer-agent, R-71)
    pub is_sender_automated_peer: bool,
    pub room_has_other_automated_peers: bool,
    pub consecutive_automated_turns: u32,
    pub peer_expertise_margin: Option<f32>,
}

/// Constructs the finite, `[0.0, 1.0]`-bounded `32d` `schema = 2` `x_num` vector.
#[must_use]
pub fn extract_x_num_v2(inp: &NumericFeatureInputsV2) -> [f32; DEFAULT_NUM_FEATURES_V2] {
    let base = extract_x_num(&inp.base);
    let mut out = [0.0_f32; DEFAULT_NUM_FEATURES_V2];
    out[..DEFAULT_NUM_FEATURES_V1].copy_from_slice(&base);

    // 18..24: 6d repair geometry (R-68)
    out[18] = if inp.has_recent_visible_soul_act { 1.0 } else { 0.0 };
    out[19] = sanitize_unit(inp.cos_soul_user, 0.0);
    out[20] = sanitize_unit(inp.cos_prev_user_rephrase, 0.0);
    out[21] = sanitize_unit(inp.cos_expected_reply, 0.0);
    out[22] = if inp.is_reply_to_last_soul { 1.0 } else { 0.0 };
    out[23] = if inp.soul_intervened_since_sender_last { 1.0 } else { 0.0 };

    // 24..28: 4d thread/actor cues (R-69)
    out[24] = sanitize_unit(inp.max_active_thread_cos, 0.0);
    out[25] = sanitize_unit((inp.active_thread_count.min(6) as f64) / 6.0, 0.0);
    out[26] = if inp.sender_in_top_thread { 1.0 } else { 0.0 };
    out[27] = sanitize_unit(inp.sender_reliability, 0.5);

    // 28..32: 4d peer-agent cues (R-71)
    out[28] = if inp.is_sender_automated_peer { 1.0 } else { 0.0 };
    out[29] = if inp.room_has_other_automated_peers { 1.0 } else { 0.0 };
    out[30] = sanitize_unit((inp.consecutive_automated_turns.min(6) as f64) / 6.0, 0.0);
    out[31] = inp
        .peer_expertise_margin
        .filter(|v| v.is_finite())
        .map_or(0.5_f32, |m| (0.5 * (m.clamp(-1.0, 1.0) + 1.0)).clamp(0.0, 1.0));

    out
}

/// Strictly post-`VisibleScope` visible repair context for `R-68` features and Loop A repair hints.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct VisibleRepairContext {
    pub last_soul_event_id: Option<EventId>,
    pub last_soul_ms: Option<Time>,
    pub last_soul_embedding: Option<TaggedEmbedding>,
    pub last_sender_event_id: Option<EventId>,
    pub last_sender_ms: Option<Time>,
    pub last_sender_embedding: Option<TaggedEmbedding>,
}

/// Extracts the most recent visible soul event and prior visible sender event at `place`
/// strictly post-[`VisibleScope`] (`Delta_NI = 0`), optionally excluding `exclude_event_id`.
#[must_use]
pub fn extract_visible_repair_context(
    state: &CoreState,
    place: &PlaceId,
    scope: &VisibleScope,
    sender: &ActorId,
    now: Time,
    exclude_event_id: Option<EventId>,
) -> VisibleRepairContext {
    let soul_id = &state.soul_actor_id;
    let mut ctx = VisibleRepairContext::default();

    let is_newer = |cand_ms: Time, cand_id: EventId, cur_ms: Option<Time>, cur_id: Option<EventId>| {
        match (cur_ms, cur_id) {
            (None, _) => true,
            (Some(cm), Some(cid)) => {
                cand_ms > cm || (cand_ms == cm && cand_id.as_u64() >= cid.as_u64())
            }
            (Some(cm), None) => cand_ms >= cm,
        }
    };

    if let Some(entries) = state.place_history.get(place) {
        for entry in entries {
            if Some(entry.event_id) == exclude_event_id {
                continue;
            }
            if !scope.allows(&entry.audience, Some(entry.event_id), now) {
                continue;
            }
            let emb = state
                .memory_entries
                .get(&entry.event_id)
                .filter(|m| !m.erased && scope.allows(&m.audience, Some(m.event_id), now))
                .and_then(|m| m.embeddings.first().cloned());

            if &entry.actor == soul_id
                && is_newer(entry.timestamp_ms, entry.event_id, ctx.last_soul_ms, ctx.last_soul_event_id)
            {
                ctx.last_soul_event_id = Some(entry.event_id);
                ctx.last_soul_ms = Some(entry.timestamp_ms);
                if emb.is_some() {
                    ctx.last_soul_embedding = emb.clone();
                }
            }
            if &entry.actor == sender
                && is_newer(
                    entry.timestamp_ms,
                    entry.event_id,
                    ctx.last_sender_ms,
                    ctx.last_sender_event_id,
                )
            {
                ctx.last_sender_event_id = Some(entry.event_id);
                ctx.last_sender_ms = Some(entry.timestamp_ms);
                if emb.is_some() {
                    ctx.last_sender_embedding = emb;
                }
            }
        }
    }

    for mem in state.memory_entries.values() {
        if mem.erased || mem.superseded_by.is_some() || &mem.place != place {
            continue;
        }
        if Some(mem.event_id) == exclude_event_id {
            continue;
        }
        if !scope.allows(&mem.audience, Some(mem.event_id), now) {
            continue;
        }
        let emb = mem.embeddings.first().cloned();
        if &mem.actor == soul_id {
            if is_newer(mem.timestamp_ms, mem.event_id, ctx.last_soul_ms, ctx.last_soul_event_id) {
                ctx.last_soul_event_id = Some(mem.event_id);
                ctx.last_soul_ms = Some(mem.timestamp_ms);
                if emb.is_some() {
                    ctx.last_soul_embedding = emb.clone();
                }
            } else if ctx.last_soul_event_id == Some(mem.event_id)
                && ctx.last_soul_embedding.is_none()
                && emb.is_some()
            {
                ctx.last_soul_embedding = emb.clone();
            }
        }
        if &mem.actor == sender {
            if is_newer(
                mem.timestamp_ms,
                mem.event_id,
                ctx.last_sender_ms,
                ctx.last_sender_event_id,
            ) {
                ctx.last_sender_event_id = Some(mem.event_id);
                ctx.last_sender_ms = Some(mem.timestamp_ms);
                if emb.is_some() {
                    ctx.last_sender_embedding = emb;
                }
            } else if ctx.last_sender_event_id == Some(mem.event_id)
                && ctx.last_sender_embedding.is_none()
                && emb.is_some()
            {
                ctx.last_sender_embedding = emb;
            }
        }
    }

    // Also inspect visible non-judged-silence Response predictions opened by the soul's own acts.
    for exp in state.expectation_book.expectations.values() {
        if exp.kind != ExpectKind::Response || !exp.places.contains(place) {
            continue;
        }
        if !scope.allows(&exp.audience, Some(exp.expectation_id), now) {
            continue;
        }
        let Some(spec) = &exp.response_spec else {
            continue;
        };
        if spec.is_judged_silence {
            continue;
        }
        let soul_ev_id = if exp.origin_event_id.as_u64() != 0 {
            exp.origin_event_id
        } else {
            exp.expectation_id
        };
        if Some(soul_ev_id) == exclude_event_id {
            continue;
        }
        let emb = spec.act_embeddings.first().cloned();
        if is_newer(exp.created_at_ms, soul_ev_id, ctx.last_soul_ms, ctx.last_soul_event_id) {
            ctx.last_soul_event_id = Some(soul_ev_id);
            ctx.last_soul_ms = Some(exp.created_at_ms);
            if emb.is_some() {
                ctx.last_soul_embedding = emb;
            }
        } else if ctx.last_soul_event_id == Some(soul_ev_id)
            && ctx.last_soul_embedding.is_none()
            && emb.is_some()
        {
            ctx.last_soul_embedding = emb;
        }
    }

    ctx
}

fn matching_cosine(percept_embs: &[TaggedEmbedding], target: Option<&TaggedEmbedding>) -> f64 {
    let Some(t) = target else {
        return 0.0;
    };
    let mut best = 0.0_f64;
    for q in percept_embs {
        if q.tag == t.tag {
            best = best.max(cosine_similarity(&q.values, &t.values));
        }
    }
    best
}

/// Extracts the `32d` `schema = 2` `x_num` vector from [`CoreState`] strictly post-[`VisibleScope`]
/// (`Delta_NI = 0`).
///
/// Indices `0..18` match [`extract_features_from_state`] bit-for-bit; indices `18..32` append
/// `6d` repair (`R-68`), `4d` thread/actor (`R-69`), and `4d` peer-agent (`R-71`) channels.
#[must_use]
pub fn extract_features_v2_from_state(
    state: &CoreState,
    percept: &Percept,
    now: Time,
    backlog_queue_depth: usize,
) -> [f32; DEFAULT_NUM_FEATURES_V2] {
    let base = extract_features_from_state(state, percept, now, backlog_queue_depth);
    let scope = VisibleScope::new(percept.audience.clone(), Some(percept.place.clone()));
    let soul_id = &state.soul_actor_id;

    // 18..24: 6d repair geometry (strictly post-VisibleScope)
    let repair_ctx = extract_visible_repair_context(
        state,
        &percept.place,
        &scope,
        &percept.actor,
        now,
        None,
    );
    let has_recent_visible_soul_act = repair_ctx
        .last_soul_ms
        .is_some_and(|ts| now.saturating_sub_ms(ts) <= RECENT_SOUL_REPAIR_WINDOW_MS);
    let cos_soul_user = matching_cosine(&percept.embeddings, repair_ctx.last_soul_embedding.as_ref());
    let cos_prev_user_rephrase =
        matching_cosine(&percept.embeddings, repair_ctx.last_sender_embedding.as_ref());

    let mut cos_expected_reply = 0.0_f64;
    if !percept.embeddings.is_empty() {
        for exp in state.expectation_book.expectations.values() {
            if !exp.is_active()
                || exp.kind != ExpectKind::Response
                || !exp.places.contains(&percept.place)
                || !scope.allows(&exp.audience, Some(exp.expectation_id), now)
            {
                continue;
            }
            if let Some(spec) = &exp.response_spec {
                for expected_emb in &spec.expected_reply_embeddings {
                    cos_expected_reply =
                        cos_expected_reply.max(matching_cosine(&percept.embeddings, Some(expected_emb)));
                }
            }
        }
    }

    let is_reply_to_last_soul = percept.reply_to.is_some()
        && percept.reply_to == repair_ctx.last_soul_event_id;
    let soul_intervened_since_sender_last = match (repair_ctx.last_soul_ms, repair_ctx.last_sender_ms) {
        (Some(ts_soul), Some(ts_sender)) => {
            ts_soul >= ts_sender && repair_ctx.last_soul_event_id != repair_ctx.last_sender_event_id
        }
        _ => false,
    };

    // 24..28: 4d thread/actor features (strictly post-VisibleScope over active/open threads in scope)
    let mut visible_active_threads = 0usize;
    let mut max_active_thread_cos = 0.0_f64;
    let mut top_thread_key: Option<(f64, u64, u64)> = None;
    let mut sender_in_top_thread = false;

    let scoped_threads = state
        .episode_book
        .active_threads_for_scope(&percept.place, &percept.audience);
    let candidate_episodes: Vec<&crate::consolidation::EpisodeRecord> = if !scoped_threads.is_empty()
    {
        scoped_threads
    } else {
        state.episode_book.episodes.values().collect()
    };

    for ep in candidate_episodes {
        if ep.state != EpisodeState::Open || ep.place != percept.place {
            continue;
        }
        // Verify episode visibility strictly under VisibleScope (Delta_NI = 0).
        let mut saw_entry = false;
        let mut all_visible = true;
        for &eid in &ep.event_ids {
            if let Some(mem) = state.memory_entries.get(&eid) {
                saw_entry = true;
                if mem.erased || !scope.allows(&mem.audience, Some(mem.event_id), now) {
                    all_visible = false;
                    break;
                }
            }
        }
        if !all_visible {
            continue;
        }
        if !saw_entry {
            let mut ep_aud_members = ep.participants.clone();
            ep_aud_members.insert(soul_id.clone());
            let ep_aud = Audience::new(ep_aud_members);
            if !scope.allows(&ep_aud, Some(ep.episode_id), now) {
                continue;
            }
        }

        visible_active_threads += 1;
        let cos = matching_cosine(&percept.embeddings, ep.centroid.as_ref());
        if cos > max_active_thread_cos {
            max_active_thread_cos = cos;
        }
        let cand_key = (cos, ep.end_ms.as_millis(), ep.episode_id.as_u64());
        let is_better = match top_thread_key {
            None => true,
            Some((best_cos, best_end, best_id)) => {
                (cand_key.0 - best_cos) > 1e-12
                    || ((cand_key.0 - best_cos).abs() <= 1e-12
                        && (cand_key.1 > best_end
                            || (cand_key.1 == best_end && cand_key.2 > best_id)))
            }
        };
        if is_better {
            top_thread_key = Some(cand_key);
            sender_in_top_thread = ep.participants.contains(&percept.actor);
        }
    }

    let sender_reliability = state
        .belief_book
        .source_reliability_at(&percept.actor, &DomainId::new("general"), now)
        .expected();

    // 28..32: 4d peer-agent features (strictly post-VisibleScope)
    let is_sender_automated_peer = state
        .actor_facts
        .get(&percept.actor)
        .is_some_and(|f| f.automated_peer);

    let mut visible_other_actors = BTreeSet::new();
    for m in &percept.audience.members {
        if m != soul_id && m != &percept.actor {
            visible_other_actors.insert(m.clone());
        }
    }
    if let Some(entries) = state.place_history.get(&percept.place) {
        for entry in entries {
            if scope.allows(&entry.audience, Some(entry.event_id), now)
                && &entry.actor != soul_id
                && entry.actor != percept.actor
            {
                visible_other_actors.insert(entry.actor.clone());
            }
        }
    }
    let room_has_other_automated_peers = visible_other_actors.iter().any(|a| {
        state
            .actor_facts
            .get(a)
            .is_some_and(|f| f.automated_peer)
    });

    let consecutive_automated_turns = state
        .consecutive_automated_turns_by_place
        .get(&percept.place)
        .copied()
        .unwrap_or(0);

    let mut out = [0.0_f32; DEFAULT_NUM_FEATURES_V2];
    out[..DEFAULT_NUM_FEATURES_V1].copy_from_slice(&base);
    out[18] = if has_recent_visible_soul_act { 1.0 } else { 0.0 };
    out[19] = sanitize_unit(cos_soul_user, 0.0);
    out[20] = sanitize_unit(cos_prev_user_rephrase, 0.0);
    out[21] = sanitize_unit(cos_expected_reply, 0.0);
    out[22] = if is_reply_to_last_soul { 1.0 } else { 0.0 };
    out[23] = if soul_intervened_since_sender_last { 1.0 } else { 0.0 };
    out[24] = sanitize_unit(max_active_thread_cos, 0.0);
    out[25] = sanitize_unit((visible_active_threads.min(6) as f64) / 6.0, 0.0);
    out[26] = if sender_in_top_thread { 1.0 } else { 0.0 };
    out[27] = sanitize_unit(sender_reliability, 0.5);
    out[28] = if is_sender_automated_peer { 1.0 } else { 0.0 };
    out[29] = if room_has_other_automated_peers { 1.0 } else { 0.0 };
    out[30] = sanitize_unit((consecutive_automated_turns.min(6) as f64) / 6.0, 0.0);
    out[31] = percept
        .peer_expertise_margin
        .filter(|v| v.is_finite())
        .map_or(0.5_f32, |m| (0.5 * (m.clamp(-1.0, 1.0) + 1.0)).clamp(0.0, 1.0));
    out
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
            h_actor_dim: 0,
            cell_type: crate::neural::CellType::MinGru,
            temperature: 1.0,
            cic_threshold: 0.5,
            max_wait_ms: 30_000.0,
            has_phase2_heads: false,
            has_phase3_heads: false,
            has_v22_heads: false,
            expected_embed_model_ver: Some("egemma2-256".to_owned()),
            expected_prompt_regime_id: Some("t_d".to_owned()),
        };
        let picked = extract_embedding_for_model_config(&p, &cfg);
        assert_eq!(picked, vec![0.9, 0.8, 0.7, 0.6]);
    }

    #[test]
    fn canonical_audience_hash_is_order_independent_and_isolates_distinct_audiences() {
        let a1 = Audience::new([ActorId::new("alice"), ActorId::new("soul")]);
        let a2 = Audience::new([ActorId::new("soul"), ActorId::new("alice")]);
        let group = Audience::new([
            ActorId::new("alice"),
            ActorId::new("bob"),
            ActorId::new("soul"),
        ]);
        assert_eq!(canonical_audience_hash(&a1), canonical_audience_hash(&a2));
        assert_ne!(canonical_audience_hash(&a1), canonical_audience_hash(&group));
    }

    #[test]
    fn extract_features_v2_populates_repair_thread_peer_and_preserves_delta_ni_zero() {
        use crate::consolidation::EpisodeAdmitParams;
        use crate::expectations::{ExpectationRecord, ResponsePredictionSpec};
        use crate::gate::PlaceHistoryEntry;
        use crate::state::ActorFactsEntry;
        use std::collections::{BTreeMap, BTreeSet};

        let tag = EmbeddingTag {
            embed_model_ver: "egemma2-256".to_owned(),
            dim_d: 4,
            prompt_regime_id: "t_d".to_owned(),
        };
        let pub_place = PlaceId::new("room:public");
        let priv_place = PlaceId::new("dm:alice");
        let pub_aud = Audience::new([
            ActorId::new("bob"),
            ActorId::new("peer_bot"),
            ActorId::new("soul"),
        ]);
        let priv_aud = Audience::new([ActorId::new("alice"), ActorId::new("soul")]);
        let now = Time::from_millis(10_000);

        let build_base_state = || {
            let mut st = CoreState::new();
            st.actor_facts.insert(
                ActorId::new("peer_bot"),
                ActorFactsEntry {
                    alpha_0: 2.0,
                    beta_0: 1.0,
                    teacher_weight: 0.5,
                    trusted_origin: true,
                    automated_peer: true,
                },
            );
            st.consecutive_automated_turns_by_place
                .insert(pub_place.clone(), 3);

            // Prior visible sender message (event 10)
            st.place_history.entry(pub_place.clone()).or_default().push(
                PlaceHistoryEntry {
                    event_id: EventId::new(10),
                    actor: ActorId::new("bob"),
                    audience: pub_aud.clone(),
                    timestamp_ms: Time::from_millis(5_000),
                    deg_by_actor: BTreeMap::new(),
                },
            );
            st.upsert_memory_entry(MemoryEntry {
                event_id: EventId::new(10),
                source_ids: vec![EventId::new(10)],
                timestamp_ms: Time::from_millis(5_000),
                place: pub_place.clone(),
                actor: ActorId::new("bob"),
                origin_actor: OriginActor::new(ActorId::new("bob")),
                basis: MemoryBasis::Observed,
                taint: Taint::Trusted,
                audience: pub_aud.clone(),
                embeddings: vec![TaggedEmbedding {
                    tag: tag.clone(),
                    values: vec![0.8, 0.6, 0.0, 0.0],
                }],
                attachments: Vec::new(),
                activation: ActivationTrace::default(),
                superseded_by: None,
                is_gist: false,
                importance: crate::recall::ImportanceInputs::default(),
                concepts: Vec::new(),
                erased: false,
            });

            // Visible recent soul act (event 11) after sender's prior message
            st.place_history.entry(pub_place.clone()).or_default().push(
                PlaceHistoryEntry {
                    event_id: EventId::new(11),
                    actor: ActorId::new("soul"),
                    audience: pub_aud.clone(),
                    timestamp_ms: Time::from_millis(7_000),
                    deg_by_actor: BTreeMap::new(),
                },
            );
            st.upsert_memory_entry(MemoryEntry {
                event_id: EventId::new(11),
                source_ids: vec![EventId::new(11)],
                timestamp_ms: Time::from_millis(7_000),
                place: pub_place.clone(),
                actor: ActorId::new("soul"),
                origin_actor: OriginActor::new(ActorId::new("soul")),
                basis: MemoryBasis::SelfCaused,
                taint: Taint::Trusted,
                audience: pub_aud.clone(),
                embeddings: vec![TaggedEmbedding {
                    tag: tag.clone(),
                    values: vec![1.0, 0.0, 0.0, 0.0],
                }],
                attachments: Vec::new(),
                activation: ActivationTrace::default(),
                superseded_by: None,
                is_gist: false,
                importance: crate::recall::ImportanceInputs::default(),
                concepts: Vec::new(),
                erased: false,
            });

            // Active visible Response prediction with expected_reply_embeddings
            st.expectation_book.expectations.insert(
                EventId::new(12),
                ExpectationRecord {
                    expectation_id: EventId::new(12),
                    origin_event_id: EventId::new(11),
                    target_event_id: EventId::new(10),
                    kind: ExpectKind::Response,
                    state: crate::event::ExpectationLifecycleState::Open,
                    version: 1,
                    places: vec![pub_place.clone()],
                    people: vec![ActorId::new("bob")],
                    audience: pub_aud.clone(),
                    domain: None,
                    condition: "reply".to_owned(),
                    condition_embeddings: Vec::new(),
                    about_ids: Vec::new(),
                    created_at_ms: Time::from_millis(7_000),
                    updated_at_ms: Time::from_millis(7_000),
                    due_ms: Time::from_millis(20_000),
                    expiry_ms: Time::from_millis(30_000),
                    recurrence: None,
                    promise_meta: None,
                    urgency_weight: 0.0,
                    is_overdue: false,
                    due_woken: false,
                    retry_count: 0,
                    next_retry_at_ms: None,
                    retry_action: None,
                    last_action_id: None,
                    last_capability_id: None,
                    last_h_act: None,
                    delivered_units: None,
                    termination_reason: None,
                    approval_prompt_emitted: false,
                    effect_spec: None,
                    response_spec: Some(ResponsePredictionSpec {
                        action_id: None,
                        medium: monoagens_protocol::ChannelId::new("chat"),
                        addressees: vec![ActorId::new("bob")],
                        accept_done_receipt: false,
                        act_embeddings: vec![TaggedEmbedding {
                            tag: tag.clone(),
                            values: vec![1.0, 0.0, 0.0, 0.0],
                        }],
                        expected_reply_embeddings: vec![TaggedEmbedding {
                            tag: tag.clone(),
                            values: vec![0.9, 0.435_89, 0.0, 0.0],
                        }],
                        is_judged_silence: false,
                        silence_sample_propensity: None,
                        silence_sender: None,
                        awaiting_reask_classify: false,
                        pending_reask_event_id: None,
                        stakes: 0.0,
                        expected_latency_ms: 5_000,
                    }),
                    monitor_spec: None,
                    goal_chain_ids: Vec::new(),
                    linked_action_ids: Vec::new(),
                    linked_expectation_id: None,
                },
            );

            // Visible active episode at pub_place containing bob
            let ep_emb = TaggedEmbedding {
                tag: tag.clone(),
                values: vec![1.0, 0.0, 0.0, 0.0],
            };
            st.episode_book.admit_event(
                EpisodeAdmitParams {
                    event_id: EventId::new(10),
                    place: &pub_place,
                    timestamp_ms: Time::from_millis(5_000),
                    participants: BTreeSet::from([ActorId::new("bob")]),
                    taint: Taint::Trusted,
                    embeddings: std::slice::from_ref(&ep_emb),
                    missed_expectation: false,
                },
                &crate::config::CoreConfig::default(),
            );
            st
        };

        let st_clean = build_base_state();
        let mut st_with_private = build_base_state();

        // Inject private DM memory, soul reply, and private episode (must NOT leak!)
        st_with_private.upsert_memory_entry(MemoryEntry {
            event_id: EventId::new(901),
            source_ids: vec![EventId::new(901)],
            timestamp_ms: Time::from_millis(9_500),
            place: priv_place.clone(),
            actor: ActorId::new("soul"),
            origin_actor: OriginActor::new(ActorId::new("soul")),
            basis: MemoryBasis::SelfCaused,
            taint: Taint::Trusted,
            audience: priv_aud.clone(),
            embeddings: vec![TaggedEmbedding {
                tag: tag.clone(),
                values: vec![0.0, 0.0, 1.0, 0.0],
            }],
            attachments: Vec::new(),
            activation: ActivationTrace::default(),
            superseded_by: None,
            is_gist: false,
            importance: crate::recall::ImportanceInputs::default(),
            concepts: Vec::new(),
            erased: false,
        });
        // Also inject a hidden sub-audience private episode at the SAME pub_place!
        st_with_private.upsert_memory_entry(MemoryEntry {
            event_id: EventId::new(902),
            source_ids: vec![EventId::new(902)],
            timestamp_ms: Time::from_millis(9_600),
            place: pub_place.clone(),
            actor: ActorId::new("alice"),
            origin_actor: OriginActor::new(ActorId::new("alice")),
            basis: MemoryBasis::Observed,
            taint: Taint::Trusted,
            audience: priv_aud.clone(),
            embeddings: vec![TaggedEmbedding {
                tag: tag.clone(),
                values: vec![1.0, 0.0, 0.0, 0.0],
            }],
            attachments: Vec::new(),
            activation: ActivationTrace::default(),
            superseded_by: None,
            is_gist: false,
            importance: crate::recall::ImportanceInputs::default(),
            concepts: Vec::new(),
            erased: false,
        });
        st_with_private.episode_book.episodes.insert(
            EventId::new(902),
            crate::consolidation::EpisodeRecord {
                episode_id: EventId::new(902),
                place: pub_place.clone(),
                state: EpisodeState::Open,
                event_ids: vec![EventId::new(902)],
                participants: BTreeSet::from([ActorId::new("alice")]),
                start_ms: Time::from_millis(9_600),
                end_ms: Time::from_millis(9_600),
                centroid: Some(TaggedEmbedding {
                    tag: tag.clone(),
                    values: vec![1.0, 0.0, 0.0, 0.0],
                }),
                vector_count: 1,
                taint: Taint::Trusted,
                gist_id: None,
                dirty: false,
                consolidated: false,
            },
        );

        let mut percept = Percept::minimal(
            ActorId::new("bob"),
            pub_place,
            pub_aud,
            now,
            OpaquePayload::from_bytes(b"actually no"),
        );
        percept.reply_to = Some(EventId::new(11));
        percept.peer_expertise_margin = Some(0.6);
        percept.embeddings.push(TaggedEmbedding {
            tag,
            values: vec![1.0, 0.0, 0.0, 0.0],
        });

        let v1 = extract_features_from_state(&st_clean, &percept, now, 0);
        let v2_clean = extract_features_v2_from_state(&st_clean, &percept, now, 0);
        let v2_priv = extract_features_v2_from_state(&st_with_private, &percept, now, 0);

        // Delta_NI = 0: bit-for-bit identical across all 32 features!
        assert_eq!(v2_clean, v2_priv);
        // 0..18 exact match with v1
        assert_eq!(&v2_clean[..18], &v1[..]);
        // All 32 bounded in [0, 1] and finite
        for &val in &v2_clean {
            assert!(val.is_finite() && (0.0..=1.0).contains(&val));
        }
        // Repair features (18..24)
        assert_eq!(v2_clean[18], 1.0); // has_recent_visible_soul_act
        assert!((v2_clean[19] - 1.0).abs() < 1e-5); // cos_soul_user
        assert!((v2_clean[20] - 0.8).abs() < 1e-5); // cos_prev_user_rephrase
        assert!((v2_clean[21] - 0.9).abs() < 1e-4); // cos_expected_reply
        assert_eq!(v2_clean[22], 1.0); // is_reply_to_last_soul
        assert_eq!(v2_clean[23], 1.0); // soul_intervened_since_sender_last
        // Thread/actor features (24..28)
        assert!((v2_clean[24] - 1.0).abs() < 1e-5); // max_active_thread_cos
        assert!((v2_clean[25] - (1.0 / 6.0)).abs() < 1e-5); // active_thread_count_norm
        assert_eq!(v2_clean[26], 1.0); // sender_in_top_thread
        assert!((v2_clean[27] - 0.5).abs() < 1e-5); // sender_reliability (uniform Beta(1,1))
        // Peer-agent features (28..32)
        assert_eq!(v2_clean[28], 0.0); // is_sender_automated_peer (bob is human)
        assert_eq!(v2_clean[29], 1.0); // room_has_other_automated_peers (peer_bot)
        assert!((v2_clean[30] - 0.5).abs() < 1e-5); // consecutive_automated_turns_norm = 3/6
        assert!((v2_clean[31] - 0.8).abs() < 1e-5); // peer_expertise_margin = 0.5*(0.6+1)
    }
}
