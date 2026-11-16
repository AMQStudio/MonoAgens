//! Pre-queue `EmbeddingGemma 2` (`256d` MRL, `8K` ctx) enrichment, `A-14` `dialogue_act_gap`
//! scoring, and hybrid BM25 + `256d` `Tool RAG` (`R-38`) capability indexing (`4.5`).

use core::fmt;
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Instant;

use monoagens_core::CapabilityCatalogEntry;
use monoagens_protocol::{
    Audience, BodyId, CapabilityDescriptor, CapabilityId, CapabilitySchemaHash, EmbeddingTag,
    ModalitySlot, Percept, TaggedEmbedding, Time,
};
use monoagens_reason::{
    EMBEDDING_GEMMA_2_CTX_TOKENS, EmbedError, EmbedInput, Embedder, EmbeddingGemma2Backend,
    EmbeddingGemma2Config, EmbeddingGemma2Embedder, HashEmbedder, InfoGapClassifier, NoneEmbedder,
    OpenAiEmbedConfig, OpenAiEmbedder, ProviderSecrets, TaskPrefix, default_mime_for_modality,
};

use crate::attachments::AttachmentStore;
use crate::clock::{Clock, run_with_clock_timeout};
use crate::config::{EmbedBackendKind, GatewayConfig};
use crate::logging::{DiagnosticSink, LogLevel};
use crate::seams::{PreQueueIndexSeam, SeamError};

/// Errors constructing a pre-queue [`Embedder`] from Gateway configuration and secrets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbedInitError {
    pub detail: String,
}

impl fmt::Display for EmbedInitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "embedder initialization failed: {}", self.detail)
    }
}

impl std::error::Error for EmbedInitError {}

impl From<EmbedError> for EmbedInitError {
    fn from(err: EmbedError) -> Self {
        Self {
            detail: err.to_string(),
        }
    }
}

/// Guard ensuring any underlying blocking HTTP client inside an [`Embedder`] is never
/// dropped directly on a Tokio async worker thread.
struct EmbedderSlot(Option<Arc<dyn Embedder>>);

impl Drop for EmbedderSlot {
    fn drop(&mut self) {
        if let Some(emb) = self.0.take()
            && tokio::runtime::Handle::try_current().is_ok()
        {
            std::thread::spawn(move || drop(emb));
        }
    }
}

/// Builds the configured pre-queue [`Embedder`] from [`GatewayConfig`] and [`ProviderSecrets`].
pub fn build_embedder_from_config(
    config: &GatewayConfig,
    secrets: &ProviderSecrets,
) -> Result<Arc<dyn Embedder>, EmbedInitError> {
    let e = &config.embed;
    match e.backend {
        EmbedBackendKind::None => Ok(Arc::new(NoneEmbedder)),
        EmbedBackendKind::Hash => Ok(Arc::new(HashEmbedder::with_model_ver(
            &e.model_ver,
            e.dim_d,
        ))),
        EmbedBackendKind::EmbeddingGemma2Http => {
            let endpoint = e
                .endpoint_url
                .clone()
                .or_else(|| secrets.embed_endpoint.clone())
                .ok_or_else(|| EmbedInitError {
                    detail: "embeddinggemma2_http requires embed.endpoint_url or MONOAGENS_EMBED_ENDPOINT"
                        .to_owned(),
                })?;
            let cfg = EmbeddingGemma2Config {
                embed_model_ver: e.model_ver.clone(),
                dim_d: e.dim_d,
                max_context_tokens: EMBEDDING_GEMMA_2_CTX_TOKENS,
                backend: EmbeddingGemma2Backend::SidecarHttp {
                    endpoint_url: endpoint,
                    model_path: e.model_path.clone(),
                    timeout_ms: e.timeout_ms.max(1),
                },
            };
            Ok(Arc::new(EmbeddingGemma2Embedder::new(cfg)?))
        }
        EmbedBackendKind::EmbeddingGemma2Stdio => {
            let program = e.program.clone().ok_or_else(|| EmbedInitError {
                detail: "embeddinggemma2_stdio requires embed.program".to_owned(),
            })?;
            let cfg = EmbeddingGemma2Config {
                embed_model_ver: e.model_ver.clone(),
                dim_d: e.dim_d,
                max_context_tokens: EMBEDDING_GEMMA_2_CTX_TOKENS,
                backend: EmbeddingGemma2Backend::SidecarStdio {
                    program,
                    args: e.args.clone(),
                    model_path: e.model_path.clone(),
                    timeout_ms: e.timeout_ms.max(1),
                },
            };
            Ok(Arc::new(EmbeddingGemma2Embedder::new(cfg)?))
        }
        EmbedBackendKind::EmbeddingGemma2Remote => {
            let endpoint = e
                .endpoint_url
                .clone()
                .or_else(|| secrets.embed_endpoint.clone())
                .ok_or_else(|| EmbedInitError {
                    detail: "embeddinggemma2_remote requires embed.endpoint_url or secrets embed_endpoint"
                        .to_owned(),
                })?;
            let remote_model = e
                .remote_model
                .clone()
                .unwrap_or_else(|| e.model_ver.clone());
            let cfg = EmbeddingGemma2Config {
                embed_model_ver: e.model_ver.clone(),
                dim_d: e.dim_d,
                max_context_tokens: EMBEDDING_GEMMA_2_CTX_TOKENS,
                backend: EmbeddingGemma2Backend::RemoteOpenAi {
                    endpoint_url: endpoint,
                    model: remote_model,
                    api_key: secrets.embed_api_key.clone(),
                    timeout_ms: e.timeout_ms.max(1),
                },
            };
            Ok(Arc::new(EmbeddingGemma2Embedder::new(cfg)?))
        }
        EmbedBackendKind::OpenAiCompatible => {
            let endpoint = e
                .endpoint_url
                .clone()
                .or_else(|| secrets.embed_endpoint.clone())
                .ok_or_else(|| EmbedInitError {
                    detail: "openai_compatible embedder requires embed_endpoint".to_owned(),
                })?;
            let remote_model = e
                .remote_model
                .clone()
                .unwrap_or_else(|| e.model_ver.clone());
            let mut cfg = OpenAiEmbedConfig::new(
                endpoint,
                remote_model,
                e.dim_d,
                secrets.embed_api_key.clone(),
            );
            cfg.embed_model_ver = e.model_ver.clone();
            cfg.timeout_ms = e.timeout_ms.max(1);
            Ok(Arc::new(OpenAiEmbedder::new(cfg)?))
        }
    }
}

/// Async-safe wrapper running `reqwest::blocking::Client` construction on `spawn_blocking`.
pub async fn build_embedder_from_config_async(
    config: &GatewayConfig,
    secrets: &ProviderSecrets,
) -> Result<Arc<dyn Embedder>, EmbedInitError> {
    if matches!(
        config.embed.backend,
        EmbedBackendKind::None | EmbedBackendKind::Hash
    ) {
        return build_embedder_from_config(config, secrets);
    }
    let cfg = config.clone();
    let sec = secrets.clone();
    tokio::task::spawn_blocking(move || build_embedder_from_config(&cfg, &sec))
        .await
        .map_err(|e| EmbedInitError {
            detail: format!("embedder builder task panicked: {e}"),
        })?
}

/// Single capability match returned by [`ToolRagIndex::query`].
#[derive(Debug, Clone, PartialEq)]
pub struct ToolRagHit {
    pub body_id: BodyId,
    pub descriptor: CapabilityDescriptor,
    pub bm25_score: f64,
    pub vector_score: Option<f64>,
    pub hybrid_score: f64,
}

/// Result of querying [`ToolRagIndex`] (`R-38`), including BM25 fallback metadata and latency.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolRagQueryResult {
    pub hits: Vec<ToolRagHit>,
    pub used_vectors: bool,
    pub fallback_reason: Option<String>,
    pub catalog_size: usize,
    pub query_latency_us: u64,
}

#[derive(Debug, Clone)]
struct IndexedCapEntry {
    body_id: BodyId,
    descriptor: CapabilityDescriptor,
    doc_len: usize,
    embedding: Option<TaggedEmbedding>,
}

/// Hybrid BM25 + partitioned `256d` vector index over registered [`CapabilityDescriptor`]s
/// stamped with `capability_schema_hash` (`R-38`, `4.5`).
///
/// Scales cleanly across `50–200+` tools via an inverted BM25 term index and degrades
/// deterministically to pure lexical BM25 when embeddings are unavailable.
pub struct ToolRagIndex {
    embedder: Arc<EmbedderSlot>,
    diagnostics: DiagnosticSink,
    clock: Arc<dyn Clock>,
    entries: Vec<IndexedCapEntry>,
    /// Inverted index: lowercase token -> Vec<(entry_idx, term_freq)>.
    inverted: HashMap<String, Vec<(usize, u32)>>,
    avg_doc_len: f64,
    /// Cache keyed by `(CapabilityId, description, capability_schema_hash, embed_model_ver, dim_d)`
    /// so repeated `body/discover` syncs don't re-embed unchanged descriptors.
    embed_cache:
        HashMap<(CapabilityId, String, CapabilitySchemaHash, String, u32), TaggedEmbedding>,
}

impl fmt::Debug for ToolRagIndex {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ToolRagIndex")
            .field("catalog_size", &self.entries.len())
            .field("vocab_size", &self.inverted.len())
            .finish()
    }
}

impl ToolRagIndex {
    /// Creates a new [`ToolRagIndex`] backed by `embedder`.
    #[must_use]
    pub fn new(
        embedder: Arc<dyn Embedder>,
        diagnostics: DiagnosticSink,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self::from_slot(Arc::new(EmbedderSlot(Some(embedder))), diagnostics, clock)
    }

    fn from_slot(
        embedder: Arc<EmbedderSlot>,
        diagnostics: DiagnosticSink,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            embedder,
            diagnostics,
            clock,
            entries: Vec::new(),
            inverted: HashMap::new(),
            avg_doc_len: 1.0,
            embed_cache: HashMap::new(),
        }
    }

    /// Number of currently indexed capabilities across all connected bodies.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns `true` if no capabilities are indexed.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Indexes or replaces all capabilities owned by `body_id`, computing `t_d` embeddings
    /// when available and falling back cleanly to BM25-only indexing on embedder outage.
    pub fn index_capabilities(&mut self, body_id: &BodyId, capabilities: &[CapabilityDescriptor]) {
        self.entries.retain(|e| &e.body_id != body_id);

        let emb_ref = self.embedder.0.as_ref().expect("embedder present");
        let model_ver = emb_ref.model_ver().map(str::to_owned);
        let dim_d = emb_ref.dim_d();
        let mut outage_logged = false;

        for raw_desc in capabilities {
            let mut desc = raw_desc.clone();
            desc.places.sort();
            desc.places.dedup();

            let doc_text = capability_doc_text(&desc);
            let embedding = match (&model_ver, dim_d) {
                (Some(mv), Some(dim)) => {
                    let cache_key = (
                        desc.id.clone(),
                        doc_text.clone(),
                        desc.capability_schema_hash.clone(),
                        mv.clone(),
                        dim,
                    );
                    if let Some(cached) = self.embed_cache.get(&cache_key) {
                        Some(cached.clone())
                    } else if outage_logged {
                        None
                    } else {
                        match emb_ref.embed(&EmbedInput::document(&doc_text)) {
                            Ok(Some(te)) => {
                                self.embed_cache.insert(cache_key, te.clone());
                                Some(te)
                            }
                            Ok(None) => None,
                            Err(err) => {
                                outage_logged = true;
                                self.diagnostics.log(
                                    self.clock.now(),
                                    LogLevel::Warn,
                                    "gateway.tool_rag",
                                    &format!(
                                        "capability '{}' embedding failed (falling back to BM25): {err}",
                                        desc.id
                                    ),
                                );
                                None
                            }
                        }
                    }
                }
                _ => None,
            };

            self.entries.push(IndexedCapEntry {
                body_id: body_id.clone(),
                descriptor: desc,
                doc_len: 0,
                embedding,
            });
        }

        self.rebuild_inverted_index();
    }

    /// Removes all capabilities belonging to `body_id`.
    pub fn remove_body_capabilities(&mut self, body_id: &BodyId) {
        let before = self.entries.len();
        self.entries.retain(|e| &e.body_id != body_id);
        if self.entries.len() != before {
            self.rebuild_inverted_index();
        }
    }

    fn rebuild_inverted_index(&mut self) {
        self.entries.sort_by(|a, b| {
            a.descriptor
                .id
                .cmp(&b.descriptor.id)
                .then_with(|| {
                    a.descriptor
                        .capability_schema_hash
                        .cmp(&b.descriptor.capability_schema_hash)
                })
                .then_with(|| a.body_id.cmp(&b.body_id))
        });

        self.inverted.clear();
        let mut total_len = 0usize;

        for (idx, entry) in self.entries.iter_mut().enumerate() {
            let doc_text = capability_doc_text(&entry.descriptor);
            let mut tf_map: BTreeMap<String, u32> = BTreeMap::new();
            let mut doc_len = 0usize;
            for tok in tokenize(&doc_text) {
                *tf_map.entry(tok).or_insert(0) += 1;
                doc_len += 1;
            }
            entry.doc_len = doc_len;
            total_len += doc_len;

            for (tok, tf) in tf_map {
                self.inverted.entry(tok).or_default().push((idx, tf));
            }
        }

        self.avg_doc_len = if self.entries.is_empty() {
            1.0
        } else {
            (total_len as f64 / self.entries.len() as f64).max(1.0)
        };
    }

    /// Queries the capability index using hybrid BM25 + partitioned `t_q -> t_d` cosine
    /// similarity, degrading deterministically to pure lexical BM25 on embedder outage.
    pub fn query(&self, query_text: &str, top_k: usize) -> ToolRagQueryResult {
        let started = Instant::now();
        let n_docs = self.entries.len();
        if n_docs == 0 || top_k == 0 {
            return ToolRagQueryResult {
                hits: Vec::new(),
                used_vectors: false,
                fallback_reason: None,
                catalog_size: n_docs,
                query_latency_us: started.elapsed().as_micros() as u64,
            };
        }

        // 1. Inverted-index BM25 scoring.
        let mut bm25_raw = vec![0.0_f64; n_docs];
        let query_tokens = unique_tokens(query_text);
        let k1 = 1.2_f64;
        let b = 0.75_f64;
        let n_f = n_docs as f64;

        for tok in &query_tokens {
            if let Some(postings) = self.inverted.get(tok) {
                let df = postings.len() as f64;
                let idf = ((n_f - df + 0.5) / (df + 0.5) + 1.0).ln().max(0.0);
                for &(idx, tf) in postings {
                    let dl = self.entries[idx].doc_len as f64;
                    let tf_f = f64::from(tf);
                    let denom = tf_f + k1 * (1.0 - b + b * (dl / self.avg_doc_len));
                    if denom > 0.0 {
                        bm25_raw[idx] += idf * (tf_f * (k1 + 1.0)) / denom;
                    }
                }
            }
        }

        // 2. Query embedding (`t_q`) with clean BM25 fallback on None or error.
        let emb_ref = self.embedder.0.as_ref().expect("embedder present");
        let (query_emb, fallback_reason) = match emb_ref.embed(&EmbedInput::query(query_text)) {
            Ok(Some(te)) => (Some(te), None),
            Ok(None) => (None, Some("none embedder active; using BM25".to_owned())),
            Err(err) => {
                let msg = format!("query embedding failed (falling back to BM25): {err}");
                self.diagnostics
                    .log(self.clock.now(), LogLevel::Warn, "gateway.tool_rag", &msg);
                (None, Some(msg))
            }
        };

        let mut any_vector_matched = false;
        let mut scored: Vec<(usize, f64, Option<f64>, f64)> = Vec::with_capacity(n_docs);

        for (idx, entry) in self.entries.iter().enumerate() {
            let bm25_s = bm25_raw[idx];
            let bm25_norm = if bm25_s > 0.0 {
                bm25_s / (bm25_s + 1.0)
            } else {
                0.0
            };

            let vec_s = match (&query_emb, &entry.embedding) {
                (Some(q), Some(d)) => asymmetric_q_to_d_cosine(q, d).map(f64::from),
                _ => None,
            };

            let hybrid = if let Some(vs) = vec_s {
                any_vector_matched = true;
                0.5 * bm25_norm + 0.5 * vs.max(0.0)
            } else {
                bm25_norm
            };
            scored.push((idx, bm25_s, vec_s, hybrid));
        }

        scored.sort_by(|a, b| {
            b.3.total_cmp(&a.3)
                .then_with(|| b.1.total_cmp(&a.1))
                .then_with(|| {
                    self.entries[a.0]
                        .descriptor
                        .id
                        .cmp(&self.entries[b.0].descriptor.id)
                })
                .then_with(|| {
                    self.entries[a.0]
                        .descriptor
                        .capability_schema_hash
                        .cmp(&self.entries[b.0].descriptor.capability_schema_hash)
                })
        });

        let hits = scored
            .into_iter()
            .take(top_k)
            .map(|(idx, bm25_score, vector_score, hybrid_score)| {
                let entry = &self.entries[idx];
                ToolRagHit {
                    body_id: entry.body_id.clone(),
                    descriptor: entry.descriptor.clone(),
                    bm25_score,
                    vector_score,
                    hybrid_score,
                }
            })
            .collect();

        ToolRagQueryResult {
            hits,
            used_vectors: any_vector_matched,
            fallback_reason: if any_vector_matched {
                None
            } else {
                fallback_reason.or_else(|| {
                    Some("no matching capability embedding partition; using BM25".to_owned())
                })
            },
            catalog_size: n_docs,
            query_latency_us: started.elapsed().as_micros() as u64,
        }
    }

    /// Exports indexed entries as [`CapabilityCatalogEntry`] items for Core's `Tool RAG`
    /// (`Core::register_capability_item`), stamped with `capability_schema_hash` and `t_d` vectors.
    #[must_use]
    pub fn to_core_catalog(&self) -> Vec<CapabilityCatalogEntry> {
        self.entries
            .iter()
            .map(|e| CapabilityCatalogEntry {
                descriptor: e.descriptor.clone(),
                is_static_core: false,
                owner_signed: false,
                active: true,
                audience: Audience::default(),
                embedding: e.embedding.clone(),
            })
            .collect()
    }
}

/// Outcome of enriching a single ingress [`Percept`] before enqueueing.
#[derive(Debug, Clone, PartialEq)]
pub struct PreQueueEnrichReport {
    pub embedded_tag: Option<EmbeddingTag>,
    pub dialogue_act_gap: Option<f32>,
    pub degraded_reason: Option<String>,
}

/// Pre-queue `EmbeddingGemma 2` + `A-14` `dialogue_act_gap` + hybrid `Tool RAG` service (`4.5`).
///
/// Holds the `Embedder` outside the `ToolRagIndex` lock so concurrent percept enrichment
/// never contends with capability discovery, and enforces `timeout_ms` via `spawn_blocking`
/// + `Clock` without retry storms.
#[derive(Clone)]
pub struct PreQueueService {
    embedder: Arc<EmbedderSlot>,
    info_gap: Option<InfoGapClassifier>,
    tool_rag: Arc<RwLock<ToolRagIndex>>,
    attachments: Option<Arc<Mutex<AttachmentStore>>>,
    diagnostics: DiagnosticSink,
    clock: Arc<dyn Clock>,
    timeout_ms: u64,
}

impl fmt::Debug for PreQueueService {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PreQueueService")
            .field("timeout_ms", &self.timeout_ms)
            .field("has_info_gap", &self.info_gap.is_some())
            .field("has_attachments", &self.attachments.is_some())
            .finish()
    }
}

impl PreQueueService {
    /// Creates a new [`PreQueueService`] wrapping `embedder`, optional [`InfoGapClassifier`],
    /// and bounded `timeout_ms`.
    #[must_use]
    pub fn new(
        embedder: Arc<dyn Embedder>,
        info_gap: Option<InfoGapClassifier>,
        attachments: Option<Arc<Mutex<AttachmentStore>>>,
        diagnostics: DiagnosticSink,
        clock: Arc<dyn Clock>,
        timeout_ms: u64,
    ) -> Self {
        let slot = Arc::new(EmbedderSlot(Some(embedder)));
        let tool_rag =
            ToolRagIndex::from_slot(Arc::clone(&slot), diagnostics.clone(), Arc::clone(&clock));
        Self {
            embedder: slot,
            info_gap,
            tool_rag: Arc::new(RwLock::new(tool_rag)),
            attachments,
            diagnostics,
            clock,
            timeout_ms: timeout_ms.max(1),
        }
    }

    /// Queries the hybrid BM25 + `256d` `Tool RAG` capability index.
    #[must_use]
    pub fn query_tools(&self, query_text: &str, top_k: usize) -> ToolRagQueryResult {
        self.tool_rag
            .read()
            .expect("tool_rag lock")
            .query(query_text, top_k)
    }

    /// Exports Core-compatible [`CapabilityCatalogEntry`] records stamped with
    /// `capability_schema_hash` and `t_d` embeddings.
    #[must_use]
    pub fn core_capability_catalog(&self) -> Vec<CapabilityCatalogEntry> {
        self.tool_rag
            .read()
            .expect("tool_rag lock")
            .to_core_catalog()
    }

    /// Indexes capabilities for `body_id` in the `Tool RAG` index.
    pub fn index_capabilities(&self, body_id: &BodyId, capabilities: &[CapabilityDescriptor]) {
        self.tool_rag
            .write()
            .expect("tool_rag lock")
            .index_capabilities(body_id, capabilities);
    }

    /// Removes capabilities owned by `body_id` from the `Tool RAG` index.
    pub fn remove_body_capabilities(&self, body_id: &BodyId) {
        self.tool_rag
            .write()
            .expect("tool_rag lock")
            .remove_body_capabilities(body_id);
    }

    /// Synchronously enriches `percept` (used by [`PreQueueIndexSeam`] and sync callers).
    ///
    /// On embedder outage/error, logs a diagnostic and leaves `percept.embeddings` empty
    /// (vector-less percepts enter Core cleanly).
    pub fn enrich_percept_sync(&self, percept: &mut Percept) -> PreQueueEnrichReport {
        let emb_ref = self.embedder.0.as_ref().expect("embedder present");
        let now = self.clock.now();
        enrich_percept_with_embedder(
            emb_ref.as_ref(),
            self.info_gap.as_ref(),
            self.attachments.as_ref(),
            &self.diagnostics,
            now,
            percept,
        )
    }

    /// Asynchronously enriches `percept` on `spawn_blocking` bounded strictly by `timeout_ms`
    /// (`run_with_clock_timeout`), never stalling ingress and never retrying on outage.
    pub async fn enrich_percept_async(&self, percept: &mut Percept) -> PreQueueEnrichReport {
        let emb_slot = Arc::clone(&self.embedder);
        let info_gap = self.info_gap.clone();
        let attachments = self.attachments.clone();
        let diagnostics = self.diagnostics.clone();
        let now = self.clock.now();
        let timeout_ms = self.timeout_ms;
        let mut work_percept = percept.clone();

        let join = tokio::task::spawn_blocking(move || {
            let emb_ref = emb_slot.0.as_ref().expect("embedder present");
            let report = enrich_percept_with_embedder(
                emb_ref.as_ref(),
                info_gap.as_ref(),
                attachments.as_ref(),
                &diagnostics,
                now,
                &mut work_percept,
            );
            (work_percept, report)
        });

        match run_with_clock_timeout(self.clock.as_ref(), timeout_ms, join).await {
            Ok(Ok((enriched, report))) => {
                *percept = enriched;
                report
            }
            Ok(Err(join_err)) => {
                let reason = format!("pre-queue embedder worker panicked: {join_err}");
                self.diagnostics.log(
                    self.clock.now(),
                    LogLevel::Error,
                    "gateway.embedder",
                    &reason,
                );
                if let Some(head) = &self.info_gap {
                    percept.dialogue_act_gap = Some(head.neutral_prior_score() as f32);
                }
                PreQueueEnrichReport {
                    embedded_tag: None,
                    dialogue_act_gap: percept.dialogue_act_gap,
                    degraded_reason: Some(reason),
                }
            }
            Err(_timed_out) => {
                let reason = format!(
                    "pre-queue embedder timed out after {timeout_ms}ms (continuing vector-less)"
                );
                self.diagnostics.log(
                    self.clock.now(),
                    LogLevel::Warn,
                    "gateway.embedder",
                    &reason,
                );
                if let Some(head) = &self.info_gap {
                    percept.dialogue_act_gap = Some(head.neutral_prior_score() as f32);
                }
                PreQueueEnrichReport {
                    embedded_tag: None,
                    dialogue_act_gap: percept.dialogue_act_gap,
                    degraded_reason: Some(reason),
                }
            }
        }
    }
}

impl PreQueueIndexSeam for PreQueueService {
    fn enrich_percept(&self, percept: &mut Percept) -> Result<(), SeamError> {
        let report = self.enrich_percept_sync(percept);
        if let Some(reason) = report.degraded_reason {
            // Returning Err causes `waist2.rs` to log a diagnostic and forward the vector-less percept.
            return Err(SeamError::new("pre_queue_index", reason));
        }
        Ok(())
    }

    fn index_capabilities(&mut self, body_id: &BodyId, capabilities: &[CapabilityDescriptor]) {
        PreQueueService::index_capabilities(self, body_id, capabilities);
    }

    fn remove_body_capabilities(&mut self, body_id: &BodyId) {
        PreQueueService::remove_body_capabilities(self, body_id);
    }
}

fn enrich_percept_with_embedder(
    embedder: &dyn Embedder,
    info_gap: Option<&InfoGapClassifier>,
    attachments: Option<&Arc<Mutex<AttachmentStore>>>,
    diagnostics: &DiagnosticSink,
    now: Time,
    percept: &mut Percept,
) -> PreQueueEnrichReport {
    let mut degraded_reason: Option<String> = None;
    let mut embedded_tag: Option<EmbeddingTag> = None;

    // 1. Primary document (`t_d`) embedding from text payload or attached media handle.
    let text_opt = if !percept.payload.is_empty() {
        std::str::from_utf8(percept.payload.as_bytes()).ok()
    } else {
        None
    };

    if let Some(text) = text_opt {
        if !text.trim().is_empty() {
            match embedder.embed(&EmbedInput::document(text)) {
                Ok(Some(te)) => {
                    embedded_tag = Some(te.tag.clone());
                    push_unique_tagged_embedding(&mut percept.embeddings, te);
                }
                Ok(None) => {}
                Err(err) => {
                    let msg = format!("text embedding failed (vector-less fallback): {err}");
                    diagnostics.log(now, LogLevel::Warn, "gateway.embedder", &msg);
                    degraded_reason = Some(msg);
                }
            }
        }
    } else if !percept.attachments.is_empty()
        && let Some(att_store) = attachments
    {
        let scope_id = percept.actor.as_str();
        for handle in &percept.attachments {
            let fetched = {
                let guard = att_store.lock().expect("attachment store lock");
                guard.fetch_scoped_bytes_uncached(scope_id, &handle.handle_id, now)
            };
            let bytes = match fetched {
                Ok(b) => b,
                Err(err) => {
                    let msg = format!(
                        "failed reading attachment '{}' for embedding: {err}",
                        handle.handle_id
                    );
                    diagnostics.log(now, LogLevel::Warn, "gateway.embedder", &msg);
                    degraded_reason = Some(msg);
                    continue;
                }
            };
            let mime = default_mime_for_modality(handle.modality_slot);
            let embed_res = match handle.modality_slot {
                ModalitySlot::Image => {
                    embedder.embed(&EmbedInput::image(TaskPrefix::Document, &bytes, mime, None))
                }
                ModalitySlot::AudioVoice => embedder.embed(&EmbedInput::audio_voice(
                    TaskPrefix::Document,
                    &bytes,
                    mime,
                    None,
                )),
                ModalitySlot::Video | ModalitySlot::SensorBlob => continue,
            };
            match embed_res {
                Ok(Some(te)) => {
                    embedded_tag = Some(te.tag.clone());
                    push_unique_tagged_embedding(&mut percept.embeddings, te);
                    break;
                }
                Ok(None) => {}
                Err(err) => {
                    let msg = format!("multimodal embedding failed (vector-less fallback): {err}");
                    diagnostics.log(now, LogLevel::Warn, "gateway.embedder", &msg);
                    degraded_reason = Some(msg);
                }
            }
        }
    }

    // 2. Frozen A-14 `dialogue_act_gap` scoring (`t_clf`, falling back to neutral prior on None/error).
    if let Some(head) = info_gap {
        if let Some(text) = text_opt {
            let eval = head.score_with_embedder(text, embedder);
            percept.dialogue_act_gap = Some(eval.score as f32);
            if !eval.used_vector
                && let Some(reason) = &eval.fallback_reason
                && embedder.model_ver().is_some()
                && degraded_reason.is_none()
            {
                diagnostics.log(now, LogLevel::Warn, "gateway.embedder", reason);
                degraded_reason = Some(reason.clone());
            }
        } else {
            percept.dialogue_act_gap = Some(head.neutral_prior_score() as f32);
        }
    }

    PreQueueEnrichReport {
        embedded_tag,
        dialogue_act_gap: percept.dialogue_act_gap,
        degraded_reason,
    }
}

fn push_unique_tagged_embedding(dest: &mut Vec<TaggedEmbedding>, item: TaggedEmbedding) {
    if let Some(existing) = dest.iter_mut().find(|e| e.tag == item.tag) {
        *existing = item;
    } else {
        dest.push(item);
    }
}

fn capability_doc_text(desc: &CapabilityDescriptor) -> String {
    if desc.places.is_empty() {
        format!("{} {}", desc.id, desc.description)
    } else {
        let places: Vec<&str> = desc.places.iter().map(|p| p.as_str()).collect();
        format!("{} {} {}", desc.id, desc.description, places.join(" "))
    }
}

fn tokenize(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|tok| !tok.is_empty())
        .map(|tok| tok.to_ascii_lowercase())
        .collect()
}

fn unique_tokens(text: &str) -> Vec<String> {
    let mut seen = BTreeMap::<String, ()>::new();
    for tok in tokenize(text) {
        seen.insert(tok, ());
    }
    seen.into_keys().collect()
}

/// Computes cosine similarity strictly for asymmetric `t_q` query against `t_d` document
/// sharing identical `(embed_model_ver, dim_d)`.
fn asymmetric_q_to_d_cosine(query: &TaggedEmbedding, doc: &TaggedEmbedding) -> Option<f32> {
    if query.tag.embed_model_ver != doc.tag.embed_model_ver
        || query.tag.dim_d != doc.tag.dim_d
        || query.tag.prompt_regime_id != TaskPrefix::Query.regime_id()
        || doc.tag.prompt_regime_id != TaskPrefix::Document.regime_id()
        || query.values.len() != doc.values.len()
        || query.values.len() != query.tag.dim_d as usize
    {
        return None;
    }

    let mut dot = 0.0_f64;
    let mut na = 0.0_f64;
    let mut nb = 0.0_f64;
    for (&x, &y) in query.values.iter().zip(doc.values.iter()) {
        if !x.is_finite() || !y.is_finite() {
            return None;
        }
        let xf = f64::from(x);
        let yf = f64::from(y);
        dot += xf * yf;
        na += xf * xf;
        nb += yf * yf;
    }
    let denom = na.sqrt() * nb.sqrt();
    if denom <= 0.0 || !denom.is_finite() {
        return None;
    }
    Some((dot / denom).clamp(-1.0, 1.0) as f32)
}
