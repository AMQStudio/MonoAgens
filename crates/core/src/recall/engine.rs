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
use crate::activation::ActivationTrace;
use crate::event::{EventDetail, EventKind, EventRecord};
use crate::gate::cosine_similarity;
use crate::map::MapVertex;
use crate::ports::Store;
use crate::prior::Prior;
use crate::recall::importance::ImportanceInputs;
use crate::recall::recollection::{
    ContextMatchTag, MemoryBasis, MemoryEntry, RecollectionConstraint, SourceEventFact,
    classify_context_match, evaluate_familiarity, verify_recollection,
};
use crate::registry::Switches;
use crate::state::CoreState;

/// Default vector candidate retrieval limit (`design.md` §7).
pub const DEFAULT_VECTOR_TOP_K: usize = 40;

/// Default text candidate retrieval limit (`design.md` §7).
pub const DEFAULT_TEXT_TOP_K: usize = 20;

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
}

impl RecallQuery {
    /// Creates a minimal [`RecallQuery`] scoped to `visible_scope`.
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
        }
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
}

/// Result of [`run_recall`]; contains strictly post-`VisibleScope` visible items (`Δ_NI = 0`).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct RecallResult {
    pub items: Vec<RecalledItem>,
    pub used_event_ids: Vec<EventId>,
}

/// Executes hybrid memory recall strictly through the [`Store`] port and `CoreState` projections,
/// enforcing pre-Step-5 `VisibleScope` projection BEFORE FOK, ranking, top-k, or use recording.
pub fn run_recall<S: Store>(
    store: &S,
    state: &CoreState,
    query: &RecallQuery,
    now_ms: Time,
    prior: &Prior,
    switches: &Switches,
) -> Result<RecallResult, CoreError> {
    let active_tag = query
        .partition_tag
        .as_ref()
        .or_else(|| query.query_embeddings.first().map(|e| &e.tag));

    let vec_limit = prior.recall_vector_top_k.max(1);
    let text_limit = prior.recall_text_top_k.max(1);

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

    // 3. Bounded Successor-Representation / Co-occurrence cognitive map candidates.
    let mut seeds: Vec<MapVertex> = Vec::new();
    if let Some(place) = &query.wake_place {
        seeds.push(MapVertex::Place(place.clone()));
    }
    for actor in &query.wake_people {
        seeds.push(MapVertex::Actor(actor.clone()));
    }
    for concept in &query.seed_concepts {
        seeds.push(MapVertex::Concept(concept.clone()));
    }
    let hop_budget = query
        .hop_budget_override
        .unwrap_or(prior.map_hop_budget)
        .min(
            prior
                .map_hop_budget
                .max(query.hop_budget_override.unwrap_or(0)),
        );
    let map_scores = state
        .cognitive_map
        .spread_activation(&seeds, hop_budget, prior);
    for (eid, _) in state
        .cognitive_map
        .candidate_events_for_activation(&map_scores, vec_limit)
    {
        if Some(eid) != query.query_event_id {
            candidate_ids.insert(eid);
        }
    }

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
        visible_entries.push(entry);
    }

    // 5. Rank strictly visible candidates using A = w_B * B + context_weight * cos + importance_weight * imp + w_map * r_map.
    let d = prior.petrov_d();
    let w_b = if switches.activation_recall {
        prior.w_b
    } else {
        0.0
    };
    let w_map = prior.map_w_map;

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
        let r_map = state.cognitive_map.score_event(entry.event_id, &map_scores);

        let total_score = w_b * b_val
            + prior.context_weight * cos_val
            + prior.importance_weight * imp_val
            + w_map * r_map;

        let familiarity = evaluate_familiarity(total_score, best_cos, prior);
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

        let is_current = if switches.memory_truth {
            entry.superseded_by.is_none()
        } else {
            true
        };

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

    let used_event_ids = ranked.iter().map(|item| item.event_id).collect();
    Ok(RecallResult {
        items: ranked,
        used_event_ids,
    })
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
