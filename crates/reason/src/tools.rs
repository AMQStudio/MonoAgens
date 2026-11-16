//! Capability and procedure progressive-disclosure selection types and helpers (`R-38`).
//!
//! Provides deterministic hybrid BM25 + partitioned-vector ranking filtered by
//! `VisibleScope ∩ ActiveCaps` and matching `capability_schema_hash`, without any
//! prompt-rendering logic (rendering lives in `render.rs` in a later dispatch).

use std::collections::{BTreeMap, BTreeSet};

use monoagens_protocol::{
    Audience, CapabilityDescriptor, CapabilityId, CapabilitySchemaHash, EventId, TaggedEmbedding,
    Time, VisibleScope, sort_capabilities,
};
use serde::{Deserialize, Serialize};

use crate::embed::partitioned_cosine;

/// Maximum number of static `OwnerSigned` core capabilities pinned in the cacheable prefix (`R-38`).
pub const MAX_STATIC_CORE_CAPS: usize = 4;
/// Default dynamic capability budget (`R-38`).
pub const DEFAULT_K_CAP: usize = 8;
/// Default active procedure disclosure budget (`R-38`).
pub const DEFAULT_K_PROC: usize = 3;

/// Selection budgets for progressive capability and procedure disclosure (`R-38`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolSelectionBudget {
    pub max_static_core: usize,
    pub k_cap: usize,
    pub k_proc: usize,
}

impl Default for ToolSelectionBudget {
    fn default() -> Self {
        Self {
            max_static_core: MAX_STATIC_CORE_CAPS,
            k_cap: DEFAULT_K_CAP,
            k_proc: DEFAULT_K_PROC,
        }
    }
}

impl ToolSelectionBudget {
    /// Exact neutral ablation budget (`k_cap = |C_avail|, k_proc = 0`, `reasoning/spec.md`).
    #[must_use]
    pub const fn neutral(available_capabilities: usize) -> Self {
        Self {
            max_static_core: MAX_STATIC_CORE_CAPS,
            k_cap: available_capabilities,
            k_proc: 0,
        }
    }
}

/// Capability catalog entry evaluated during progressive tool selection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CapabilityCatalogItem {
    pub descriptor: CapabilityDescriptor,
    pub is_static_core: bool,
    pub owner_signed: bool,
    pub active: bool,
    pub audience: Audience,
    pub embedding: Option<TaggedEmbedding>,
}

/// Consolidated procedure candidate evaluated during progressive tool selection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProcedureCatalogItem {
    pub procedure_id: EventId,
    pub name: String,
    pub steps: Vec<String>,
    pub required_cap_ids: Vec<CapabilityId>,
    pub capability_schema_hash: CapabilitySchemaHash,
    pub audience: Audience,
    pub active: bool,
    pub retired: bool,
    pub embedding: Option<TaggedEmbedding>,
}

/// Output of deterministic tool & procedure selection for `ContextPlan` / gateway callers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SelectedTools {
    pub static_core: Vec<CapabilityDescriptor>,
    pub dynamic_caps: Vec<CapabilityDescriptor>,
    pub procedures: Vec<ProcedureCatalogItem>,
    pub excluded_stale_procedures: usize,
    pub excluded_hidden_items: usize,
}

/// Selects static core capabilities, top-`k_cap` dynamic capabilities, and top-`k_proc`
/// schema-valid active procedures under `VisibleScope ∩ ActiveCaps`.
#[must_use]
pub fn select_tools(
    query_text: &str,
    query_embedding: Option<&TaggedEmbedding>,
    capabilities: &[CapabilityCatalogItem],
    procedures: &[ProcedureCatalogItem],
    visible_scope: &VisibleScope,
    now: Time,
    budget: ToolSelectionBudget,
) -> SelectedTools {
    let mut active_cap_hashes: BTreeMap<&CapabilityId, &CapabilitySchemaHash> = BTreeMap::new();
    let mut static_candidates: Vec<&CapabilityCatalogItem> = Vec::new();
    let mut dynamic_candidates: Vec<&CapabilityCatalogItem> = Vec::new();
    let mut excluded_hidden_items = 0_usize;

    for cap in capabilities {
        if !cap.active {
            continue;
        }
        if !visible_scope.allows(&cap.audience, None, now) {
            excluded_hidden_items += 1;
            continue;
        }
        active_cap_hashes.insert(&cap.descriptor.id, &cap.descriptor.capability_schema_hash);
        if cap.is_static_core && cap.owner_signed {
            static_candidates.push(cap);
        } else {
            dynamic_candidates.push(cap);
        }
    }

    let mut static_core: Vec<CapabilityDescriptor> = static_candidates
        .iter()
        .map(|c| c.descriptor.clone())
        .collect();
    sort_capabilities(&mut static_core);
    if static_core.len() > budget.max_static_core {
        // Overflow static entries move back into dynamic pool.
        let static_kept: BTreeSet<CapabilityId> = static_core
            .iter()
            .take(budget.max_static_core)
            .map(|c| c.id.clone())
            .collect();
        for cap in &static_candidates {
            if !static_kept.contains(&cap.descriptor.id) {
                dynamic_candidates.push(cap);
            }
        }
        static_core.truncate(budget.max_static_core);
    }

    let query_tokens = tokenize(query_text);
    let mut ranked_caps: Vec<(f64, CapabilityDescriptor)> = dynamic_candidates
        .into_iter()
        .map(|cap| {
            let doc_text = format!("{} {}", cap.descriptor.id, cap.descriptor.description);
            let score = hybrid_score(
                &query_tokens,
                query_embedding,
                &doc_text,
                cap.embedding.as_ref(),
            );
            (score, cap.descriptor.clone())
        })
        .collect();

    ranked_caps.sort_by(|a, b| {
        b.0.total_cmp(&a.0)
            .then_with(|| a.1.id.cmp(&b.1.id))
            .then_with(|| a.1.capability_schema_hash.cmp(&b.1.capability_schema_hash))
    });
    let dynamic_caps: Vec<CapabilityDescriptor> = ranked_caps
        .into_iter()
        .take(budget.k_cap)
        .map(|(_, mut desc)| {
            desc.places.sort();
            desc.places.dedup();
            desc
        })
        .collect();

    let mut excluded_stale_procedures = 0_usize;
    let mut ranked_procs: Vec<(f64, ProcedureCatalogItem)> = Vec::new();

    for proc_item in procedures {
        if !proc_item.active || proc_item.retired {
            continue;
        }
        if !visible_scope.allows(&proc_item.audience, Some(proc_item.procedure_id), now) {
            excluded_hidden_items += 1;
            continue;
        }
        let schema_valid = !proc_item.required_cap_ids.is_empty()
            && proc_item.required_cap_ids.iter().all(|req_id| {
                active_cap_hashes
                    .get(req_id)
                    .is_some_and(|active_hash| *active_hash == &proc_item.capability_schema_hash)
            });
        if !schema_valid {
            excluded_stale_procedures += 1;
            continue;
        }
        let doc_text = format!("{} {}", proc_item.name, proc_item.steps.join(" "));
        let score = hybrid_score(
            &query_tokens,
            query_embedding,
            &doc_text,
            proc_item.embedding.as_ref(),
        );
        ranked_procs.push((score, proc_item.clone()));
    }

    ranked_procs.sort_by(|a, b| {
        b.0.total_cmp(&a.0)
            .then_with(|| a.1.procedure_id.cmp(&b.1.procedure_id))
            .then_with(|| a.1.name.cmp(&b.1.name))
    });
    let selected_procs: Vec<ProcedureCatalogItem> = ranked_procs
        .into_iter()
        .take(budget.k_proc)
        .map(|(_, p)| p)
        .collect();

    SelectedTools {
        static_core,
        dynamic_caps,
        procedures: selected_procs,
        excluded_stale_procedures,
        excluded_hidden_items,
    }
}

fn tokenize(text: &str) -> BTreeSet<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|tok| !tok.is_empty())
        .map(|tok| tok.to_ascii_lowercase())
        .collect()
}

fn lexical_bm25_proxy(query_tokens: &BTreeSet<String>, doc_text: &str) -> f64 {
    if query_tokens.is_empty() {
        return 0.0;
    }
    let doc_tokens = tokenize(doc_text);
    if doc_tokens.is_empty() {
        return 0.0;
    }
    let hits = query_tokens.intersection(&doc_tokens).count() as f64;
    let k1 = 1.2_f64;
    let b = 0.75_f64;
    let len_ratio = (doc_tokens.len() as f64) / 16.0;
    let norm_denom = hits + k1 * (1.0 - b + b * len_ratio);
    let tf_part = if norm_denom > 0.0 {
        (hits * (k1 + 1.0)) / norm_denom
    } else {
        0.0
    };
    (tf_part / (query_tokens.len() as f64)).clamp(0.0, 1.0)
}

fn hybrid_score(
    query_tokens: &BTreeSet<String>,
    query_embedding: Option<&TaggedEmbedding>,
    doc_text: &str,
    doc_embedding: Option<&TaggedEmbedding>,
) -> f64 {
    let lex = lexical_bm25_proxy(query_tokens, doc_text);
    let vec_sim = match (query_embedding, doc_embedding) {
        (Some(q), Some(d)) => partitioned_cosine(q, d)
            .map(|c| f64::from(c).max(0.0))
            .unwrap_or(0.0),
        _ => 0.0,
    };
    0.5 * lex + 0.5 * vec_sim
}
