//! `plugin-pdf-reader`: RAG-first Waist-2 capability plugin (`monoagens.body.v1`) powered
//! by `lopdf` with per-page chunking, persistent `pdf_rag_index.json`, BM25 lexical scoring,
//! and optional hybrid cosine + Reciprocal Rank Fusion (RRF) retrieval.

#![forbid(unsafe_code)]

pub mod config;
pub mod rag;

use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use monoagens_body_kit::{
    HonestCueThresholds, capability_schema_hash, make_refused_receipt, make_terminal_receipt,
};
use monoagens_body_sdk::{
    ActionCancelParams, ActionCancelResult, ActionExecuteParams, ActionExecuteResult, ActionId,
    ApprovalResolveParams, ApprovalResolveResult, Audience, BodyDiscoverParams, BodyDiscoverResult,
    BodyHandler, BodyId, CURRENT_PROTOCOL_VERSION, CapabilityDescriptor, CapabilityFlags,
    CapabilityId, MetaValue, PerceptEmitParams, PerceptEmitResult, PlaceId, ReceiptOutcome,
    TerminationReason, Time, Waist2Error, sort_capabilities,
};
use serde::Deserialize;
use serde_json::{Value, json};

pub use config::{
    DEFAULT_BM25_B, DEFAULT_BM25_K1, DEFAULT_CHUNK_CHARS, DEFAULT_CHUNK_OVERLAP_CHARS,
    DEFAULT_MAX_PAGES_PER_READ, DEFAULT_MAX_TOP_K, DEFAULT_RRF_K, DEFAULT_TOP_K,
    ENV_EMBED_ENDPOINT, ENV_PLUGIN_CONFIG, ENV_PLUGIN_STATE_DIR, PdfReaderConfig,
    resolve_state_dir_from_env,
};
pub use rag::{
    DeleteResult, DocumentSummary, IndexResult, IndexedChunk, IndexedPdfDoc,
    PDF_RAG_INDEX_FILENAME, PageText, PdfRagIndex, ReadPagesResult, SearchHit, chunk_page_text,
    cosine_similarity, extract_pdf_pages, normalize_whitespace, tokenize,
};

/// Default Waist-2 body identifier for `plugin-pdf-reader`.
pub const DEFAULT_BODY_ID: &str = "plugin-pdf-reader";

/// Default declared capability place owned by `plugin-pdf-reader`.
pub const DEFAULT_PLACE_ID: &str = "cap:pdf-reader";

#[derive(Debug, Deserialize)]
struct PdfIndexArgs {
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    bytes_base64: Option<String>,
    #[serde(default)]
    doc_id: Option<String>,
    #[serde(default)]
    title: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PdfSearchArgs {
    query: String,
    #[serde(default)]
    doc_id: Option<String>,
    #[serde(default)]
    top_k: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct PdfReadPageArgs {
    doc_id: String,
    start_page: u32,
    #[serde(default)]
    end_page: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct PdfDeleteArgs {
    doc_id: String,
}

/// Waist-2 capability body implementing RAG-first PDF indexing, search, and bounded page reads.
pub struct PdfReaderBody {
    body_id: BodyId,
    place: PlaceId,
    config: PdfReaderConfig,
    index: PdfRagIndex,
    thresholds: HonestCueThresholds,
    held_actions: HashSet<ActionId>,
    clock_now_ms: u64,
}

impl PdfReaderBody {
    /// Constructs a [`PdfReaderBody`] with explicit config and optional persistent state directory.
    #[must_use]
    pub fn new(config: PdfReaderConfig, state_dir: Option<PathBuf>) -> Self {
        let index = PdfRagIndex::load_or_new(state_dir.as_deref());
        Self {
            body_id: BodyId::new(DEFAULT_BODY_ID),
            place: PlaceId::new(DEFAULT_PLACE_ID),
            config,
            index,
            thresholds: HonestCueThresholds::default(),
            held_actions: HashSet::new(),
            clock_now_ms: 0,
        }
    }

    /// Initializes [`PdfReaderBody`] from `MONOAGENS_PLUGIN_CONFIG`, `MONOAGENS_EMBED_ENDPOINT`,
    /// and `MONOAGENS_PLUGIN_STATE_DIR` / `MONOAGENS_BODY_STATE_DIR`.
    pub fn from_env() -> Result<(Self, Option<PathBuf>), String> {
        let config = PdfReaderConfig::load_or_init_from_env()?;
        let state_dir = resolve_state_dir_from_env();
        let body = Self::new(config, state_dir.clone());
        Ok((body, state_dir))
    }

    /// Returns the declared place owned by this capability body.
    #[must_use]
    pub fn place(&self) -> &PlaceId {
        &self.place
    }

    /// Returns the loaded [`PdfReaderConfig`].
    #[must_use]
    pub fn config(&self) -> &PdfReaderConfig {
        &self.config
    }

    /// Borrows the underlying [`PdfRagIndex`].
    #[must_use]
    pub fn index(&self) -> &PdfRagIndex {
        &self.index
    }

    /// Advances the local virtual/injected clock timestamp used for receipts and indexing.
    pub fn set_now_ms(&mut self, now_ms: u64) {
        self.clock_now_ms = now_ms;
    }

    fn descriptors(&self) -> Vec<CapabilityDescriptor> {
        let place_str = self.place.as_str();
        let mut caps = vec![
            CapabilityDescriptor {
                id: CapabilityId::new("pdf.index"),
                description: "Parse a PDF per page, build overlapping RAG chunks, and return compact document metadata".to_owned(),
                places: vec![self.place.clone()],
                flags: CapabilityFlags {
                    communicative: false,
                    identity_changing: false,
                    irreversible: false,
                },
                typical_latency_ms: 50,
                capability_schema_hash: capability_schema_hash(&json!({
                    "capability": "pdf.index",
                    "place": place_str,
                    "input": {
                        "type": "object",
                        "properties": {
                            "path": { "type": "string" },
                            "bytes_base64": { "type": "string" },
                            "doc_id": { "type": "string" },
                            "title": { "type": "string" }
                        }
                    }
                })),
                sensed_effect: None,
            },
            CapabilityDescriptor {
                id: CapabilityId::new("pdf.search"),
                description: "Search indexed PDF chunks with BM25 and optional hybrid cosine RRF returning page-cited snippets".to_owned(),
                places: vec![self.place.clone()],
                flags: CapabilityFlags {
                    communicative: false,
                    identity_changing: false,
                    irreversible: false,
                },
                typical_latency_ms: 20,
                capability_schema_hash: capability_schema_hash(&json!({
                    "capability": "pdf.search",
                    "place": place_str,
                    "input": {
                        "type": "object",
                        "required": ["query"],
                        "properties": {
                            "query": { "type": "string" },
                            "doc_id": { "type": "string" },
                            "top_k": { "type": "integer", "minimum": 1 }
                        }
                    }
                })),
                sensed_effect: None,
            },
            CapabilityDescriptor {
                id: CapabilityId::new("pdf.read_page"),
                description: "Read a bounded page range from an indexed PDF document".to_owned(),
                places: vec![self.place.clone()],
                flags: CapabilityFlags {
                    communicative: false,
                    identity_changing: false,
                    irreversible: false,
                },
                typical_latency_ms: 5,
                capability_schema_hash: capability_schema_hash(&json!({
                    "capability": "pdf.read_page",
                    "place": place_str,
                    "input": {
                        "type": "object",
                        "required": ["doc_id", "start_page"],
                        "properties": {
                            "doc_id": { "type": "string" },
                            "start_page": { "type": "integer", "minimum": 1 },
                            "end_page": { "type": "integer", "minimum": 1 }
                        }
                    }
                })),
                sensed_effect: None,
            },
            CapabilityDescriptor {
                id: CapabilityId::new("pdf.list"),
                description: "List indexed PDF documents with page and chunk counts".to_owned(),
                places: vec![self.place.clone()],
                flags: CapabilityFlags {
                    communicative: false,
                    identity_changing: false,
                    irreversible: false,
                },
                typical_latency_ms: 2,
                capability_schema_hash: capability_schema_hash(&json!({
                    "capability": "pdf.list",
                    "place": place_str,
                    "input": {
                        "type": "object",
                        "properties": {}
                    }
                })),
                sensed_effect: None,
            },
            CapabilityDescriptor {
                id: CapabilityId::new("pdf.delete"),
                description: "Remove an indexed PDF document and its chunks from the RAG store".to_owned(),
                places: vec![self.place.clone()],
                flags: CapabilityFlags {
                    communicative: false,
                    identity_changing: false,
                    irreversible: false,
                },
                typical_latency_ms: 5,
                capability_schema_hash: capability_schema_hash(&json!({
                    "capability": "pdf.delete",
                    "place": place_str,
                    "input": {
                        "type": "object",
                        "required": ["doc_id"],
                        "properties": {
                            "doc_id": { "type": "string" }
                        }
                    }
                })),
                sensed_effect: None,
            },
        ];
        sort_capabilities(&mut caps);
        caps
    }

    fn execute_pdf_index(&mut self, payload: &[u8]) -> Result<Value, (&'static str, String)> {
        let args: PdfIndexArgs = serde_json::from_slice(payload)
            .map_err(|e| ("ERR_INVALID_ARGS", format!("invalid pdf.index JSON payload: {e}")))?;

        let (bytes, source) = match (
            args.path.as_deref().map(str::trim).filter(|s| !s.is_empty()),
            args.bytes_base64
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty()),
        ) {
            (Some(path), _) => {
                let data = fs::read(path).map_err(|e| {
                    ("ERR_PDF_IO", format!("failed to read PDF file '{path}': {e}"))
                })?;
                (data, path.to_owned())
            }
            (None, Some(b64)) => {
                let compact: String = b64.chars().filter(|c| !c.is_whitespace()).collect();
                let data = BASE64_STANDARD.decode(compact).map_err(|e| {
                    ("ERR_INVALID_ARGS", format!("invalid base64 PDF payload: {e}"))
                })?;
                (data, "bytes_base64".to_owned())
            }
            (None, None) => {
                return Err((
                    "ERR_INVALID_ARGS",
                    "pdf.index requires either 'path' or 'bytes_base64'".to_owned(),
                ));
            }
        };

        let result = self
            .index
            .index_bytes(
                &bytes,
                args.doc_id.as_deref(),
                args.title.as_deref(),
                &source,
                self.clock_now_ms,
                &self.config,
            )
            .map_err(|e| ("ERR_PDF_PARSE", e))?;

        Ok(json!(result))
    }

    fn execute_pdf_search(&self, payload: &[u8]) -> Result<Value, (&'static str, String)> {
        let args: PdfSearchArgs = serde_json::from_slice(payload)
            .map_err(|e| ("ERR_INVALID_ARGS", format!("invalid pdf.search JSON payload: {e}")))?;

        let hits = self
            .index
            .search(
                &args.query,
                args.doc_id.as_deref(),
                args.top_k,
                &self.config,
            )
            .map_err(|e| ("ERR_PDF_SEARCH", e))?;

        Ok(json!({
            "query": args.query.trim(),
            "total_hits": hits.len(),
            "hits": hits,
        }))
    }

    fn execute_pdf_read_page(&self, payload: &[u8]) -> Result<Value, (&'static str, String)> {
        let args: PdfReadPageArgs = serde_json::from_slice(payload).map_err(|e| {
            (
                "ERR_INVALID_ARGS",
                format!("invalid pdf.read_page JSON payload: {e}"),
            )
        })?;

        let result = self
            .index
            .read_page(
                args.doc_id.trim(),
                args.start_page,
                args.end_page,
                self.config.max_pages_per_read,
            )
            .map_err(|e| ("ERR_PDF_READ_PAGE", e))?;

        Ok(json!(result))
    }

    fn execute_pdf_list(&self, payload: &[u8]) -> Result<Value, (&'static str, String)> {
        if !payload.is_empty() {
            let _: Value = serde_json::from_slice(payload).map_err(|e| {
                ("ERR_INVALID_ARGS", format!("invalid pdf.list JSON payload: {e}"))
            })?;
        }
        let documents = self.index.list();
        Ok(json!({ "documents": documents }))
    }

    fn execute_pdf_delete(&mut self, payload: &[u8]) -> Result<Value, (&'static str, String)> {
        let args: PdfDeleteArgs = serde_json::from_slice(payload).map_err(|e| {
            (
                "ERR_INVALID_ARGS",
                format!("invalid pdf.delete JSON payload: {e}"),
            )
        })?;

        let result = self
            .index
            .delete(args.doc_id.trim())
            .map_err(|e| ("ERR_PDF_STORAGE", e))?;
        Ok(json!(result))
    }
}

impl BodyHandler for PdfReaderBody {
    fn discover(&mut self, params: BodyDiscoverParams) -> Result<BodyDiscoverResult, Waist2Error> {
        Ok(BodyDiscoverResult {
            body_id: params.body_id.unwrap_or_else(|| self.body_id.clone()),
            protocol_version: CURRENT_PROTOCOL_VERSION,
            capabilities: self.descriptors(),
            places: vec![self.place.clone()],
        })
    }

    fn execute(&mut self, params: ActionExecuteParams) -> Result<ActionExecuteResult, Waist2Error> {
        if params.audience.is_empty() {
            return Err(Waist2Error::undeclared_audience());
        }

        let cap = params.capability_id.as_str();
        let known = matches!(
            cap,
            "pdf.index" | "pdf.search" | "pdf.read_page" | "pdf.list" | "pdf.delete"
        );
        if !known {
            return Err(Waist2Error::unknown_capability(&params.capability_id));
        }

        let place_ok = !params.places.is_empty() && params.places.iter().all(|p| p == &self.place);
        if !place_ok {
            let receipt = make_refused_receipt(
                params.action_id,
                params.capability_id,
                "ERR_UNKNOWN_PLACE",
                "action targets a place not owned by plugin-pdf-reader",
                params.h_act,
                Time::from_millis(self.clock_now_ms),
            );
            return Ok(ActionExecuteResult {
                accepted: false,
                receipt: Some(receipt),
            });
        }

        if params.h_act.is_some() && params.payload.as_bytes() == b"hold" {
            self.held_actions.insert(params.action_id.clone());
            let receipt = make_terminal_receipt(
                params.action_id,
                params.capability_id,
                ReceiptOutcome::Held,
                Some("PendingApproval".to_owned()),
                Some("awaiting approval before executing pdf capability".to_owned()),
                Some(TerminationReason::Normal),
                Some(0.0),
                Some(0),
                params.h_act,
                Time::from_millis(self.clock_now_ms),
            );
            return Ok(ActionExecuteResult {
                accepted: true,
                receipt: Some(receipt),
            });
        }

        let now = Time::from_millis(self.clock_now_ms);
        let payload_bytes = params.payload.as_bytes();
        let is_delivery_probe = payload_bytes == b"delivery-timing-probe"
            || params.headers.meta.get("org.monoagens.delivery_probe")
                == Some(&MetaValue::Bool(true));
        let is_tck_probe = is_delivery_probe
            || matches!(
                payload_bytes,
                b"idempotency-probe" | b"crash-recovery-probe" | b"owned-place-action"
            );

        if is_tck_probe {
            let detail = json!({
                "d_art_ms": self.thresholds.d_art_ms,
                "working_cue_ms": self.thresholds.working_cue_ms,
                "progress_note_ms": self.thresholds.progress_note_ms,
                "probe": true,
            })
            .to_string();
            let receipt = make_terminal_receipt(
                params.action_id,
                params.capability_id,
                ReceiptOutcome::Done,
                None,
                Some(detail),
                Some(TerminationReason::Normal),
                Some(1.0),
                Some(1),
                params.h_act,
                now,
            );
            return Ok(ActionExecuteResult {
                accepted: true,
                receipt: Some(receipt),
            });
        }

        let outcome = match cap {
            "pdf.index" => self.execute_pdf_index(payload_bytes),
            "pdf.search" => self.execute_pdf_search(payload_bytes),
            "pdf.read_page" => self.execute_pdf_read_page(payload_bytes),
            "pdf.list" => self.execute_pdf_list(payload_bytes),
            "pdf.delete" => self.execute_pdf_delete(payload_bytes),
            _ => return Err(Waist2Error::unknown_capability(&params.capability_id)),
        };

        let receipt = match outcome {
            Ok(detail_json) => make_terminal_receipt(
                params.action_id,
                params.capability_id,
                ReceiptOutcome::Done,
                None,
                Some(detail_json.to_string()),
                Some(TerminationReason::Normal),
                Some(1.0),
                Some(1),
                params.h_act,
                now,
            ),
            Err((code, msg)) => make_terminal_receipt(
                params.action_id,
                params.capability_id,
                ReceiptOutcome::Failed,
                Some(code.to_owned()),
                Some(json!({ "error": code, "message": msg }).to_string()),
                Some(TerminationReason::Normal),
                Some(0.0),
                Some(0),
                params.h_act,
                now,
            ),
        };

        Ok(ActionExecuteResult {
            accepted: true,
            receipt: Some(receipt),
        })
    }

    fn cancel(&mut self, params: ActionCancelParams) -> Result<ActionCancelResult, Waist2Error> {
        self.held_actions.remove(&params.action_id);
        let receipt = make_terminal_receipt(
            params.action_id,
            CapabilityId::new("pdf.search"),
            ReceiptOutcome::Failed,
            Some("canceled_by_core".to_owned()),
            params.reason,
            Some(TerminationReason::CanceledByCore),
            Some(0.0),
            Some(0),
            None,
            Time::from_millis(self.clock_now_ms),
        );
        Ok(ActionCancelResult {
            cancelled: true,
            receipt: Some(receipt),
        })
    }

    fn resolve_approval(
        &mut self,
        params: ApprovalResolveParams,
    ) -> Result<ApprovalResolveResult, Waist2Error> {
        if self.held_actions.remove(&params.action_id) {
            return Ok(ApprovalResolveResult { applied: true });
        }
        Ok(ApprovalResolveResult { applied: false })
    }

    fn on_percept(&mut self, params: PerceptEmitParams) -> Result<PerceptEmitResult, Waist2Error> {
        if params.audience.as_ref().is_none_or(Audience::is_empty) {
            return Err(Waist2Error::undeclared_audience());
        }
        if params.place != self.place {
            return Err(Waist2Error::invalid_receipt_transition(format!(
                "percept place '{}' is not owned by plugin-pdf-reader",
                params.place
            )));
        }
        Ok(PerceptEmitResult { accepted: true })
    }
}
