//! Hybrid memory recall (`vector top-40 ∪ text top-20 ∪ bounded SR map`),
//! pre-Step-5 `VisibleScope` projection (`R-33`), `w_B = 0` neutral ranking (`A-01`),
//! query partitioning by `(embed_model_ver, dim_d, prompt_regime_id)`, and `memory.recall`.

use std::collections::{BTreeMap, BTreeSet};

use monoagens_protocol::{
    ActorId, AttachmentHandle, Audience, EmbeddingTag, EventId, OriginActor, PlaceId,
    TaggedEmbedding, Taint, Time, VisibleScope,
};
use serde::{Deserialize, Serialize};

use crate::CoreError;
use crate::activation::{ActivationTrace, DEFAULT_PETROV_D};
use crate::config::CoreConfig;
use crate::event::{EventDetail, EventKind, EventRecord};
use crate::gate::cosine_similarity;
use crate::neural::{
    NeuralModel, NumericFeatureInputs, extract_x_num, forward_step,
};
use crate::ports::Store;
use crate::recall::importance::ImportanceInputs;
use crate::recall::recollection::{
    ContextMatchTag, MemoryBasis, MemoryEntry, RecollectionConstraint, SourceEventFact,
    classify_context_match, evaluate_familiarity, verify_recollection,
};
use crate::state::CoreState;

const RECALL_WEIGHT_BASE: f64 = 0.35;
const RECALL_WEIGHT_CONTEXT: f64 = 0.50;
const RECALL_WEIGHT_IMPORTANCE: f64 = 0.15;
const RECALL_NEURAL_BLEND_WEIGHT: f64 = 0.35;

/// Default vector candidate retrieval limit (`design.md` §7).
pub const DEFAULT_VECTOR_TOP_K: usize = 40;

/// Default text candidate retrieval limit (`design.md` §7).
pub const DEFAULT_TEXT_TOP_K: usize = 20;

/// Temporal visibility mode for [`run_recall_with_model`] (`RD-Forget`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecallTimeMode {
    #[default]
    CurrentOnly,
    IncludeSuperseded,
}

/// Query specification for [`run_recall`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecallQuery {
    pub query_event_id: Option<EventId>,
    pub query_embeddings: Vec<TaggedEmbedding>,
    /// Optional partition key `(embed_model_ver, dim_d, prompt_regime_id)` enforced across
    /// both vector and store-reported text candidate sets (`5.4`).
    pub partition_tag: Option<EmbeddingTag>,
    pub visible_scope: VisibleScope,
    pub wake_place: Option<PlaceId>,
    pub wake_people: Vec<ActorId>,
    pub seed_concepts: Vec<String>,
    pub recollection_constraint: Option<RecollectionConstraint>,
    pub limit: usize,
    pub record_uses: bool,
    pub hop_budget_override: Option<usize>,
    /// Temporal visibility mode controlling whether superseded memories are returned (`RD-Forget`).
    #[serde(default)]
    pub time_mode: RecallTimeMode,
}

impl RecallQuery {
    /// Creates a minimal [`RecallQuery`] scoped to `visible_scope`.
    ///
    /// Defaults to [`RecallTimeMode::IncludeSuperseded`] for backward compatibility with existing
    /// history-inspecting callers; live chat wakes use [`Self::current_only`] (`RD-Forget`).
    #[must_use]
    pub fn new(visible_scope: VisibleScope) -> Self {
        Self {
            query_event_id: None,
            query_embeddings: Vec::new(),
            partition_tag: None,
            visible_scope,
            wake_place: None,
            wake_people: Vec::new(),
            seed_concepts: Vec::new(),
            recollection_constraint: None,
            limit: 10,
            record_uses: false,
            hop_budget_override: None,
            time_mode: RecallTimeMode::IncludeSuperseded,
        }
    }

    /// Creates a [`RecallQuery`] scoped to `visible_scope` that excludes superseded memories (`RD-Forget`).
    #[must_use]
    pub fn current_only(visible_scope: VisibleScope) -> Self {
        let mut q = Self::new(visible_scope);
        q.time_mode = RecallTimeMode::CurrentOnly;
        q
    }
}

/// One visible recalled item carrying full provenance, dual-process `F`/`R`, and blind handles (`R-37`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecalledItem {
    pub event_id: EventId,
    pub source_ids: Vec<EventId>,
    pub timestamp_ms: Time,
    pub place: PlaceId,
    pub actor: ActorId,
    pub origin_actor: OriginActor,
    pub basis: MemoryBasis,
    pub taint: Taint,
    pub audience: Audience,
    pub attachments: Vec<AttachmentHandle>,
    pub base_level_b: f64,
    pub cosine: f64,
    pub importance: f64,
    pub map_score: f64,
    pub total_score: f64,
    pub familiarity: f64,
    pub recollected: bool,
    pub context_match: ContextMatchTag,
    pub superseded_by: Option<EventId>,
    pub is_current: bool,
    /// Optional Phase 3 post-`VisibleScope` neural memory relevance score in `[0.0, 1.0]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub neural_relevance: Option<f64>,
}

/// Result of [`run_recall`]; contains strictly post-`VisibleScope` visible items (`Δ_NI = 0`)
/// and separates surfaced `recalled_event_ids` from downstream `cited_event_ids`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct RecallResult {
    pub items: Vec<RecalledItem>,
    pub used_event_ids: Vec<EventId>,
    /// All visible event IDs surfaced by this recall pass.
    #[serde(default)]
    pub recalled_event_ids: Vec<EventId>,
    /// Subset of `recalled_event_ids` explicitly cited in reasoning/acts.
    #[serde(default)]
    pub cited_event_ids: Vec<EventId>,
}

impl RecallResult {
    /// Records downstream cited event IDs, strictly restricting to recalled items and preserving order.
    pub fn mark_cited(&mut self, cited: &[EventId]) {
        let (_, cited_from_recalled) =
            partition_recalled_and_cited(&self.recalled_event_ids, cited);
        for id in cited_from_recalled {
            if !self.cited_event_ids.contains(&id) {
                self.cited_event_ids.push(id);
            }
        }
    }
}

/// Separates surfaced `recalled_ids` into `(recalled_only, cited_from_recalled)` preserving order.
#[must_use]
pub fn partition_recalled_and_cited(
    recalled_ids: &[EventId],
    cited_ids: &[EventId],
) -> (Vec<EventId>, Vec<EventId>) {
    let cited_set: BTreeSet<EventId> = cited_ids.iter().copied().collect();
    let mut recalled_only = Vec::new();
    let mut cited_from_recalled = Vec::new();
    for &id in recalled_ids {
        if cited_set.contains(&id) {
            if !cited_from_recalled.contains(&id) {
                cited_from_recalled.push(id);
            }
        } else if !recalled_only.contains(&id) {
            recalled_only.push(id);
        }
    }
    (recalled_only, cited_from_recalled)
}

/// Executes hybrid memory recall strictly through the [`Store`] port and `CoreState` projections,
/// enforcing pre-Step-5 `VisibleScope` projection BEFORE FOK, ranking, top-k, or use recording.
pub fn run_recall<S: Store>(
    store: &S,
    state: &CoreState,
    query: &RecallQuery,
    now_ms: Time,
    config: &CoreConfig,
) -> Result<RecallResult, CoreError> {
    run_recall_with_model(store, state, query, now_ms, config, None)
}

/// Executes hybrid memory recall with optional Phase 3 post-`VisibleScope` neural reranking.
///
/// When `model` is `None` or lacks Phase 3 heads (`model.phase3.is_none()`), ranking falls back
/// deterministically to `0.35 * B + 0.50 * cos + 0.15 * imp`. Neural scoring runs strictly
/// AFTER `VisibleScope` projection so private/out-of-scope entries never influence visible items
/// (`Delta_NI = 0`).
pub fn run_recall_with_model<S: Store>(
    store: &S,
    state: &CoreState,
    query: &RecallQuery,
    now_ms: Time,
    config: &CoreConfig,
    model: Option<&NeuralModel>,
) -> Result<RecallResult, CoreError> {
    let active_tag = query
        .partition_tag
        .as_ref()
        .or_else(|| query.query_embeddings.first().map(|e| &e.tag));

    let vec_limit = config.recall_vector_top_k.max(1);
    let text_limit = config.recall_text_top_k.max(1);

    let mut candidate_ids: BTreeSet<EventId> = BTreeSet::new();
    let mut store_cosines: BTreeMap<EventId, f64> = BTreeMap::new();

    // 1. Vector candidates (top-40 per partitioned tag, never mixing incompatible tags).
    for q_emb in &query.query_embeddings {
        if let Some(req_tag) = active_tag
            && &q_emb.tag != req_tag
        {
            continue;
        }
        let vec_hits = store.candidates_by_vector(&q_emb.tag, &q_emb.values, vec_limit)?;
        for (eid, score) in vec_hits {
            if Some(eid) == query.query_event_id {
                continue;
            }
            candidate_ids.insert(eid);
            let sf = f64::from(score);
            if sf.is_finite() {
                store_cosines
                    .entry(eid)
                    .and_modify(|prev| *prev = prev.max(sf))
                    .or_insert(sf);
            }
        }
    }

    // 2. Text candidates (top-20, partitioned by active_tag where the store reports it).
    if let Some(q_id) = query.query_event_id {
        let text_hits = store.candidates_by_text_partitioned(q_id, active_tag, text_limit)?;
        for eid in text_hits {
            if eid != q_id {
                candidate_ids.insert(eid);
            }
        }
    }

    // Step 3 (V1 cognitive map) pruned in Slim Core V2.

    // Include in-memory indexed entries whose embeddings match the active partition or
    // when no query vectors exist (in case store vector index wasn't separately populated).
    for (eid, entry) in &state.memory_entries {
        if Some(*eid) == query.query_event_id || entry.erased {
            continue;
        }
        if let Some(req_tag) = active_tag {
            if entry.embeddings.iter().any(|e| &e.tag == req_tag) {
                candidate_ids.insert(*eid);
            }
        } else if query.query_embeddings.is_empty() && query.query_event_id.is_none() {
            candidate_ids.insert(*eid);
        }
    }

    // 4. PRE-STEP-5 VisibleScope projection BEFORE any FOK, activation, ranking, or use-count update.
    let mut visible_entries: Vec<MemoryEntry> = Vec::new();
    for eid in candidate_ids {
        let Some(entry) = resolve_memory_entry(store, state, eid)? else {
            continue;
        };
        if entry.erased {
            continue;
        }
        if !query
            .visible_scope
            .allows(&entry.audience, Some(entry.event_id), now_ms)
        {
            continue;
        }
        // RD-Forget temporal visibility filter runs strictly AFTER VisibleScope projection.
        if query.time_mode == RecallTimeMode::CurrentOnly && entry.superseded_by.is_some() {
            continue;
        }
        visible_entries.push(entry);
    }

    // 5. Rank strictly visible candidates using A = w_B * B + w_context * cos + w_importance * imp.
    let d = DEFAULT_PETROV_D;

    let mut ranked: Vec<RecalledItem> = Vec::with_capacity(visible_entries.len());
    for entry in &visible_entries {
        // Partition-isolated cosine computation: never mix vectors across incompatible tags.
        let mut best_cos: Option<f64> = store_cosines.get(&entry.event_id).copied();
        for q_emb in &query.query_embeddings {
            if let Some(req_tag) = active_tag
                && &q_emb.tag != req_tag
            {
                continue;
            }
            for c_emb in &entry.embeddings {
                if c_emb.tag == q_emb.tag
                    && let Some(sim) = cosine_similarity(&q_emb.values, &c_emb.values)
                {
                    best_cos = Some(best_cos.map_or(sim, |prev| prev.max(sim)));
                }
            }
        }
        let cos_val = best_cos.unwrap_or(0.0);

        let b_val = entry.activation.base_level(now_ms, d);
        let imp_val = entry.importance_score();
        let r_map = 0.0_f64;

        let base_total_score =
            RECALL_WEIGHT_BASE * b_val + RECALL_WEIGHT_CONTEXT * cos_val + RECALL_WEIGHT_IMPORTANCE * imp_val;

        let neural_relevance = score_visible_candidate_relevance(
            model,
            entry,
            active_tag,
            cos_val,
            query,
            now_ms,
        );
        let total_score = match neural_relevance {
            Some(rel) => {
                (1.0 - RECALL_NEURAL_BLEND_WEIGHT) * base_total_score
                    + RECALL_NEURAL_BLEND_WEIGHT * rel
            }
            None => base_total_score,
        };

        let familiarity = evaluate_familiarity(total_score, best_cos);
        let recollected =
            verify_recollection(entry, query.recollection_constraint.as_ref(), |src_id| {
                resolve_source_fact(store, state, src_id)
            });
        let context_match = classify_context_match(
            entry,
            recollected,
            query.wake_place.as_ref(),
            &query.wake_people,
            now_ms,
            24.0,
        );

        let is_current = entry.superseded_by.is_none();

        ranked.push(RecalledItem {
            event_id: entry.event_id,
            source_ids: if entry.source_ids.is_empty() {
                vec![entry.event_id]
            } else {
                entry.source_ids.clone()
            },
            timestamp_ms: entry.timestamp_ms,
            place: entry.place.clone(),
            actor: entry.actor.clone(),
            origin_actor: entry.origin_actor.clone(),
            basis: entry.basis,
            taint: entry.taint,
            audience: entry.audience.clone(),
            attachments: entry.attachments.clone(),
            base_level_b: b_val,
            cosine: cos_val,
            importance: imp_val,
            map_score: r_map,
            total_score,
            familiarity,
            recollected,
            context_match,
            superseded_by: entry.superseded_by,
            is_current,
            neural_relevance,
        });
    }

    ranked.sort_by(|a, b| {
        b.is_current
            .cmp(&a.is_current)
            .then_with(|| b.total_score.total_cmp(&a.total_score))
            .then_with(|| a.event_id.cmp(&b.event_id))
    });
    if query.limit > 0 {
        ranked.truncate(query.limit);
    } else {
        ranked.clear();
    }

    let used_event_ids: Vec<EventId> = ranked.iter().map(|item| item.event_id).collect();
    let recalled_event_ids = used_event_ids.clone();
    Ok(RecallResult {
        items: ranked,
        used_event_ids,
        recalled_event_ids,
        cited_event_ids: Vec::new(),
    })
}

/// Evaluates Phase 3 `p_memory_relevance` on a single post-`VisibleScope` candidate memory.
///
/// Strictly read-only and scoped solely to the visible `entry` + `query`, using zero initial
/// hidden state so private/invisible memories can never alter any visible candidate's score.
fn score_visible_candidate_relevance(
    model: Option<&NeuralModel>,
    entry: &MemoryEntry,
    active_tag: Option<&EmbeddingTag>,
    cos_val: f64,
    query: &RecallQuery,
    now_ms: Time,
) -> Option<f64> {
    let m = model?;
    m.phase3.as_ref()?;

    let d_emb = m.config.d_in_emb;
    let mut cand_emb = vec![0.0_f32; d_emb];
    let matched_emb = active_tag
        .and_then(|tag| entry.embeddings.iter().find(|e| &e.tag == tag))
        .or_else(|| entry.embeddings.first());
    if let Some(te) = matched_emb {
        let take = d_emb.min(te.values.len());
        for (dst, &src) in cand_emb[..take].iter_mut().zip(&te.values[..take]) {
            if src.is_finite() {
                *dst = src;
            }
        }
    }

    let age_s = now_ms
        .as_millis()
        .saturating_sub(entry.timestamp_ms.as_millis()) as f64
        / 1000.0;
    let same_place_or_person = query.wake_place.as_ref() == Some(&entry.place)
        || query.wake_people.contains(&entry.actor);
    let x_num = extract_x_num(&NumericFeatureInputs {
        is_named_soul: false,
        is_explicit_addressee: false,
        is_reply_or_expect_link: same_place_or_person,
        is_dyadic_place: entry.audience.len() <= 2,
        active_participant_count: entry.audience.len().max(1),
        dt_since_last_place_event_s: age_s,
        dt_since_last_soul_act_s: 86_400.0,
        dt_since_sender_last_act_s: age_s,
        soul_acts_window: 0,
        total_acts_window: 1,
        sender_exchange_count: 1,
        place_event_count: 1,
        budget_used_ratio: 0.0,
        imminent_promise_debt: 0.0,
        max_visible_mem_cos: cos_val.clamp(0.0, 1.0),
        max_visible_mem_age_s: age_s,
        pending_belief_uncertainty: if entry.superseded_by.is_some() { 1.0 } else { 0.0 },
        backlog_queue_depth: 0,
        asr_conf_or_one: 1.0,
    });
    let zero_h = vec![0.0_f32; m.config.h_dim];
    let out = forward_step(Some(m), &cand_emb, &x_num, &zero_h, None).ok()?;
    out.p_memory_relevance
        .filter(|v| v.is_finite())
        .map(|v| f64::from(v.clamp(0.0, 1.0)))
}

fn resolve_memory_entry<S: Store>(
    store: &S,
    state: &CoreState,
    event_id: EventId,
) -> Result<Option<MemoryEntry>, CoreError> {
    if let Some(entry) = state.memory_entries.get(&event_id) {
        return Ok(Some(entry.clone()));
    }
    let Some(rec) = store.get(event_id)? else {
        return Ok(None);
    };
    Ok(memory_entry_from_record(&rec, state))
}

fn resolve_source_fact<S: Store>(
    store: &S,
    state: &CoreState,
    event_id: EventId,
) -> Option<SourceEventFact> {
    if let Some(entry) = state.memory_entries.get(&event_id) {
        return Some(SourceEventFact {
            event_id: entry.event_id,
            actor: entry.actor.clone(),
            audience: entry.audience.clone(),
            place: entry.place.clone(),
            timestamp_ms: entry.timestamp_ms,
            erased: entry.erased,
        });
    }
    let rec = store.get(event_id).ok()??;
    Some(SourceEventFact {
        event_id: rec.id,
        actor: rec.actor,
        audience: rec.audience,
        place: rec.place,
        timestamp_ms: rec.timestamp_ms,
        erased: false,
    })
}

pub(crate) fn memory_entry_from_record(
    rec: &EventRecord,
    state: &CoreState,
) -> Option<MemoryEntry> {
    if rec.kind != EventKind::Percept && rec.kind != EventKind::Thought {
        return None;
    }
    let embeddings = match &rec.detail {
        Some(EventDetail::PerceptMeta { embeddings, .. }) => embeddings.clone(),
        _ => Vec::new(),
    };
    let mut trace = ActivationTrace::default();
    trace.record(rec.timestamp_ms);
    let basis = if rec.actor == state.soul_actor_id {
        MemoryBasis::SelfCaused
    } else if rec.taint == Taint::Trusted {
        MemoryBasis::Observed
    } else {
        MemoryBasis::Told
    };
    Some(MemoryEntry {
        event_id: rec.id,
        source_ids: vec![rec.id],
        is_gist: false,
        timestamp_ms: rec.timestamp_ms,
        place: rec.place.clone(),
        actor: rec.actor.clone(),
        origin_actor: rec.origin_actor.clone(),
        basis,
        taint: rec.taint,
        audience: rec.audience.clone(),
        attachments: rec.attachments.clone(),
        activation: trace,
        importance: ImportanceInputs::default(),
        embeddings,
        concepts: Vec::new(),
        superseded_by: None,
        erased: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MemStore;
    use monoagens_protocol::{CURRENT_PROTOCOL_VERSION, EmbeddingTag};
    use safetensors::tensor::{Dtype, TensorView, serialize};
    use std::collections::HashMap;

    fn build_synthetic_phase3_model(d_in_emb: usize, d_model: usize) -> NeuralModel {
        let d_in_num = 18;
        let n_freq = 4;
        let d_e = 16;
        let d_n = 8;
        let d_ff = 16;
        let k = 2;
        let h_dim = 8;
        let plr_in = d_in_num * 2 * n_freq;
        let fusion_in = d_e + d_n + h_dim;

        let mut meta = HashMap::new();
        meta.insert("feature_schema_version".to_owned(), "1".to_owned());
        meta.insert("d_in_emb".to_owned(), d_in_emb.to_string());
        meta.insert("d_in_num".to_owned(), d_in_num.to_string());
        meta.insert("plr_n_frequencies".to_owned(), n_freq.to_string());
        meta.insert("d_e".to_owned(), d_e.to_string());
        meta.insert("d_n".to_owned(), d_n.to_string());
        meta.insert("d_model".to_owned(), d_model.to_string());
        meta.insert("d_ff".to_owned(), d_ff.to_string());
        meta.insert("n_layers".to_owned(), "1".to_owned());
        meta.insert("tabm_k".to_owned(), k.to_string());
        meta.insert("h_dim".to_owned(), h_dim.to_string());
        meta.insert("cell_type".to_owned(), "mingru".to_owned());
        meta.insert("temperature".to_owned(), "1.0".to_owned());
        meta.insert("cic_threshold".to_owned(), "0.5".to_owned());
        meta.insert("max_wait_ms".to_owned(), "30000.0".to_owned());
        meta.insert("has_phase2_heads".to_owned(), "true".to_owned());
        meta.insert("has_phase3_heads".to_owned(), "true".to_owned());

        let mut raw: Vec<(String, Vec<usize>, Vec<u8>)> = Vec::new();
        let mut push = |name: &str, shape: Vec<usize>, fill: f32| {
            let n: usize = shape.iter().product();
            let mut bytes = Vec::with_capacity(n * 4);
            for i in 0..n {
                let v = fill + (i as f32) * 0.001;
                bytes.extend_from_slice(&v.to_le_bytes());
            }
            raw.push((name.to_owned(), shape, bytes));
        };

        push("stem.emb.linear.weight", vec![d_e, d_in_emb], 0.01);
        push("stem.emb.linear.bias", vec![d_e], 0.0);
        push("stem.emb.norm.weight", vec![d_e], 1.0);
        push("stem.emb.norm.bias", vec![d_e], 0.0);
        push("stem.plr.freqs", vec![d_in_num, n_freq], 0.1);
        push("stem.plr.linear.weight", vec![d_n, plr_in], 0.01);
        push("stem.plr.linear.bias", vec![d_n], 0.0);
        push("stem.fusion.weight", vec![d_model, fusion_in], 0.01);
        push("stem.fusion.bias", vec![d_model], 0.0);
        push("trunk.tabm_in_scale", vec![k, d_model], 1.0);
        push("trunk.blocks.0.norm.weight", vec![d_model], 1.0);
        push("trunk.blocks.0.norm.bias", vec![d_model], 0.0);
        push("trunk.blocks.0.w_gate.weight", vec![d_ff, d_model], 0.01);
        push("trunk.blocks.0.w_up.weight", vec![d_ff, d_model], 0.01);
        push("trunk.blocks.0.w_down.weight", vec![d_model, d_ff], 0.01);
        push("trunk.blocks.0.tabm_r", vec![k, d_model], 1.0);
        push("trunk.blocks.0.tabm_s", vec![k, d_model], 1.0);
        push("trunk.final_norm.weight", vec![d_model], 1.0);
        push("trunk.final_norm.bias", vec![d_model], 0.0);
        push("memory.mingru.w_z.weight", vec![h_dim, d_model], 0.01);
        push("memory.mingru.w_z.bias", vec![h_dim], 0.0);
        push("memory.mingru.w_h.weight", vec![h_dim, d_model], 0.01);
        push("memory.mingru.w_h.bias", vec![h_dim], 0.0);
        push("head.gate_4.weight", vec![4, d_model], 0.01);
        push("head.gate_4.bias", vec![4], 0.0);
        push("head.wait_delta_t.weight", vec![1, d_model], 0.01);
        push("head.wait_delta_t.bias", vec![1], 0.0);
        push("head.p_uncertainty.weight", vec![1, d_model], 0.02);
        push("head.p_uncertainty.bias", vec![1], 0.0);
        push("head.p_verify.weight", vec![1, d_model], 0.02);
        push("head.p_verify.bias", vec![1], 0.0);
        push("head.p_encode_importance.weight", vec![1, d_model], 0.03);
        push("head.p_encode_importance.bias", vec![1], 0.1);
        push("head.p_memory_relevance.weight", vec![1, d_model], 0.05);
        push("head.p_memory_relevance.bias", vec![1], 0.25);

        let views: Vec<(&str, TensorView<'_>)> = raw
            .iter()
            .map(|(n, s, b)| (n.as_str(), TensorView::new(Dtype::F32, s.clone(), b).unwrap()))
            .collect();
        let bytes = serialize(views, Some(meta)).unwrap();
        NeuralModel::from_safetensors_bytes(&bytes, None).unwrap()
    }

    fn make_entry(id: u64, aud: &[&str], emb_val: f32) -> MemoryEntry {
        let mut trace = ActivationTrace::default();
        trace.record(Time::from_millis(1_000));
        MemoryEntry {
            event_id: EventId::new(id),
            source_ids: vec![EventId::new(id)],
            is_gist: false,
            timestamp_ms: Time::from_millis(1_000),
            place: PlaceId::new("room:main"),
            actor: ActorId::new(aud[0]),
            origin_actor: OriginActor::new(ActorId::new(aud[0])),
            basis: MemoryBasis::Observed,
            taint: Taint::Trusted,
            audience: Audience::new(aud.iter().map(|s| ActorId::new(*s))),
            attachments: Vec::new(),
            activation: trace,
            importance: ImportanceInputs::default(),
            embeddings: vec![TaggedEmbedding {
                tag: EmbeddingTag {
                    embed_model_ver: "gemma2".to_owned(),
                    dim_d: 256,
                    prompt_regime_id: "default".to_owned(),
                },
                values: vec![emb_val; 256],
            }],
            concepts: Vec::new(),
            superseded_by: None,
            erased: false,
        }
    }

    #[test]
    fn phase3_recall_reranks_strictly_post_visible_scope_with_zero_interference() {
        let store = MemStore::new();
        let mut state_only_pub = CoreState::new();
        state_only_pub.upsert_memory_entry(make_entry(10, &["alice", "soul"], 0.1));

        let mut state_with_priv = state_only_pub.clone();
        state_with_priv.upsert_memory_entry(make_entry(20, &["bob_private", "soul"], 0.9));

        let model = build_synthetic_phase3_model(256, 16);
        let scope = VisibleScope::new(
            Audience::new([ActorId::new("alice"), ActorId::new("soul")]),
            Some(PlaceId::new("room:main")),
        );
        let mut query = RecallQuery::new(scope);
        query.query_embeddings = vec![TaggedEmbedding {
            tag: EmbeddingTag {
                embed_model_ver: "gemma2".to_owned(),
                dim_d: 256,
                prompt_regime_id: "default".to_owned(),
            },
            values: vec![0.1; 256],
        }];
        query.limit = 10;

        let now = Time::from_millis(5_000);
        let cfg = CoreConfig::default();

        // Neutral fallback (None) leaves neural_relevance = None
        let neutral = run_recall_with_model(&store, &state_only_pub, &query, now, &cfg, None).unwrap();
        assert_eq!(neutral.items.len(), 1);
        assert!(neutral.items[0].neural_relevance.is_none());

        // Phase 3 model scores visible items identically whether private item 20 exists or not (Delta_NI == 0)
        let r_pub =
            run_recall_with_model(&store, &state_only_pub, &query, now, &cfg, Some(&model)).unwrap();
        let r_priv =
            run_recall_with_model(&store, &state_with_priv, &query, now, &cfg, Some(&model)).unwrap();
        assert_eq!(r_pub, r_priv, "private entries must cause zero interference (Delta_NI = 0)");
        assert!(r_pub.items[0].neural_relevance.is_some());
        let rel = r_pub.items[0].neural_relevance.unwrap();
        assert!((0.0..=1.0).contains(&rel) && rel.is_finite());
        let _ = CURRENT_PROTOCOL_VERSION;
    }

    #[test]
    fn recall_time_mode_controls_superseded_visibility_and_separates_recalled_vs_cited() {
        let store = MemStore::new();
        let mut state = CoreState::new();
        let mut old_entry = make_entry(10, &["alice", "soul"], 0.8);
        old_entry.superseded_by = Some(EventId::new(11));
        state.upsert_memory_entry(old_entry);
        state.upsert_memory_entry(make_entry(11, &["alice", "soul"], 0.8));
        // Private superseded entry outside alice's scope must never interfere.
        let mut priv_entry = make_entry(99, &["bob_private", "soul"], 0.8);
        priv_entry.superseded_by = Some(EventId::new(100));
        state.upsert_memory_entry(priv_entry);

        let scope = VisibleScope::new(
            Audience::new([ActorId::new("alice"), ActorId::new("soul")]),
            Some(PlaceId::new("room:main")),
        );
        let emb = vec![TaggedEmbedding {
            tag: EmbeddingTag {
                embed_model_ver: "gemma2".to_owned(),
                dim_d: 256,
                prompt_regime_id: "default".to_owned(),
            },
            values: vec![0.8; 256],
        }];
        let now = Time::from_millis(5_000);
        let cfg = CoreConfig::default();

        // CurrentOnly excludes superseded entry 10.
        let mut q_curr = RecallQuery::current_only(scope.clone());
        assert_eq!(q_curr.time_mode, RecallTimeMode::CurrentOnly);
        assert_eq!(RecallTimeMode::default(), RecallTimeMode::CurrentOnly);
        q_curr.query_embeddings = emb.clone();
        let mut res_curr = run_recall(&store, &state, &q_curr, now, &cfg).unwrap();
        assert_eq!(res_curr.recalled_event_ids, vec![EventId::new(11)]);
        assert!(res_curr.cited_event_ids.is_empty());
        assert_eq!(res_curr.items.len(), 1);
        assert!(res_curr.items[0].is_current);

        // IncludeSuperseded returns both 11 (current) and 10 (is_current = false), strictly post-VisibleScope.
        let mut q_all = RecallQuery::new(scope);
        q_all.time_mode = RecallTimeMode::IncludeSuperseded;
        q_all.query_embeddings = emb;
        let mut res_all = run_recall(&store, &state, &q_all, now, &cfg).unwrap();
        assert_eq!(
            res_all.recalled_event_ids,
            vec![EventId::new(11), EventId::new(10)]
        );
        assert!(res_all.items[0].is_current);
        assert!(!res_all.items[1].is_current);
        assert_eq!(res_all.items[1].superseded_by, Some(EventId::new(11)));
        assert!(res_all.items.iter().all(|i| i.event_id != EventId::new(99)));

        // Recalled vs cited tracking filters strictly to recalled_event_ids.
        res_all.mark_cited(&[EventId::new(11), EventId::new(999)]);
        assert_eq!(res_all.cited_event_ids, vec![EventId::new(11)]);
        assert_eq!(
            res_all.recalled_event_ids,
            vec![EventId::new(11), EventId::new(10)]
        );
        res_curr.mark_cited(&[EventId::new(10)]);
        assert!(res_curr.cited_event_ids.is_empty());
    }
}
