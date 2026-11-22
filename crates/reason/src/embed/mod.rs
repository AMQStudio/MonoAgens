//! Unified embedding trait, mandatory task prefixes, and strict partition isolation.
//!
//! Every emitted vector carries `(embed_model_ver, dim_d, prompt_regime_id)`.
//! Vectors from different partitions are never compared; mismatched partitions
//! are reported explicitly via [`EmbedError::PartitionMismatch`] or [`PartitionedSearchResult`].

use core::fmt;
use std::collections::BTreeSet;

use monoagens_protocol::{EmbeddingTag, EventId, TaggedEmbedding};
use serde::{Deserialize, Serialize};

pub mod embeddinggemma2;
pub mod hash;
pub mod none;
pub mod openai;
pub mod scripted;
pub mod sidecar;

pub use embeddinggemma2::{
    EMBEDDING_GEMMA_2_CTX_TOKENS, EMBEDDING_GEMMA_2_DIM, EMBEDDING_GEMMA_2_VER,
    EMBEDDING_GEMMA_300M_CTX_TOKENS, EMBEDDING_GEMMA_300M_DEFAULT_DIM, EMBEDDING_GEMMA_300M_VER,
    EmbeddingGemma2Backend, EmbeddingGemma2Config, EmbeddingGemma2Embedder,
    EmbeddingGemma300mConfig, EmbeddingGemma300mEmbedder,
};
pub use hash::HashEmbedder;
pub use none::NoneEmbedder;
pub use openai::{OpenAiEmbedConfig, OpenAiEmbedder};
pub use scripted::ScriptedEmbedder;
pub use sidecar::{ENV_ADAPTER_PYTHON, LocalEmbedSidecar, resolve_adapter_python};

/// Mandatory task instruction prefixes (`A-15`, `R-37`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskPrefix {
    /// Classification / dialogue-act gap (`t_clf`).
    Clf,
    /// Semantic textual similarity (`t_sts`).
    Sts,
    /// Retrieval query (`t_q`).
    Query,
    /// Retrieval document / stored event (`t_d`).
    Document,
}

impl TaskPrefix {
    /// Canonical `prompt_regime_id` stored inside [`EmbeddingTag`].
    #[must_use]
    pub const fn regime_id(self) -> &'static str {
        match self {
            Self::Clf => "t_clf",
            Self::Sts => "t_sts",
            Self::Query => "t_q",
            Self::Document => "t_d",
        }
    }

    /// Instruction prefix prepended to text prompts for EmbeddingGemma-family models.
    #[must_use]
    pub const fn instruction_prefix(self) -> &'static str {
        match self {
            Self::Clf => "task: classification | query: ",
            Self::Sts => "task: sentence similarity | query: ",
            Self::Query => "task: search result | query: ",
            Self::Document => "title: none | text: ",
        }
    }

    /// Formats `text` with the mandatory task instruction prefix.
    #[must_use]
    pub fn format_text(self, text: &str) -> String {
        format!("{}{}", self.instruction_prefix(), text)
    }
}

impl fmt::Display for TaskPrefix {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.regime_id())
    }
}

/// Supported embedding input modalities (`R-37`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EmbedModality {
    Text,
    Image,
    AudioVoice,
}

impl fmt::Display for EmbedModality {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Text => "text",
            Self::Image => "image",
            Self::AudioVoice => "audio_voice",
        };
        f.write_str(name)
    }
}

/// Modal payload submitted to an [`Embedder`].
#[derive(Debug, Clone, Copy)]
pub enum EmbedContent<'a> {
    Text(&'a str),
    Image {
        bytes: &'a [u8],
        mime_type: &'a str,
        caption: Option<&'a str>,
    },
    AudioVoice {
        bytes: &'a [u8],
        mime_type: &'a str,
        transcript: Option<&'a str>,
    },
}

impl EmbedContent<'_> {
    /// Returns the modality kind of this payload.
    #[must_use]
    pub const fn modality(&self) -> EmbedModality {
        match self {
            Self::Text(_) => EmbedModality::Text,
            Self::Image { .. } => EmbedModality::Image,
            Self::AudioVoice { .. } => EmbedModality::AudioVoice,
        }
    }
}

/// Input submitted to [`Embedder::embed`].
#[derive(Debug, Clone, Copy)]
pub struct EmbedInput<'a> {
    pub task: TaskPrefix,
    pub content: EmbedContent<'a>,
}

impl<'a> EmbedInput<'a> {
    /// Constructs a text embedding input with an explicit [`TaskPrefix`].
    #[must_use]
    pub const fn text(task: TaskPrefix, content: &'a str) -> Self {
        Self {
            task,
            content: EmbedContent::Text(content),
        }
    }

    /// Constructs a `t_q` query embedding input.
    #[must_use]
    pub const fn query(content: &'a str) -> Self {
        Self::text(TaskPrefix::Query, content)
    }

    /// Constructs a `t_d` document embedding input.
    #[must_use]
    pub const fn document(content: &'a str) -> Self {
        Self::text(TaskPrefix::Document, content)
    }

    /// Constructs a `t_clf` classification embedding input.
    #[must_use]
    pub const fn classify(content: &'a str) -> Self {
        Self::text(TaskPrefix::Clf, content)
    }

    /// Constructs a `t_sts` similarity embedding input.
    #[must_use]
    pub const fn similarity(content: &'a str) -> Self {
        Self::text(TaskPrefix::Sts, content)
    }

    /// Constructs an image embedding input.
    #[must_use]
    pub const fn image(
        task: TaskPrefix,
        bytes: &'a [u8],
        mime_type: &'a str,
        caption: Option<&'a str>,
    ) -> Self {
        Self {
            task,
            content: EmbedContent::Image {
                bytes,
                mime_type,
                caption,
            },
        }
    }

    /// Constructs an `audio_voice` embedding input.
    #[must_use]
    pub const fn audio_voice(
        task: TaskPrefix,
        bytes: &'a [u8],
        mime_type: &'a str,
        transcript: Option<&'a str>,
    ) -> Self {
        Self {
            task,
            content: EmbedContent::AudioVoice {
                bytes,
                mime_type,
                transcript,
            },
        }
    }
}

/// Typed errors returned by [`Embedder`] implementations and partition checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EmbedError {
    UnsupportedModality {
        backend: String,
        modality: EmbedModality,
    },
    MissingModelPath {
        backend: String,
    },
    ModelPathNotFound {
        backend: String,
        path: String,
    },
    ContextLimitExceeded {
        backend: String,
        estimated_tokens: usize,
        max_tokens: usize,
    },
    DimensionMismatch {
        expected: u32,
        actual: usize,
    },
    NonFiniteVector,
    ZeroNormVector,
    PartitionMismatch {
        expected: Box<EmbeddingTag>,
        actual: Box<EmbeddingTag>,
        reason: String,
    },
    Authentication {
        detail: String,
    },
    RateLimit {
        detail: String,
    },
    Timeout {
        timeout_ms: u64,
        detail: String,
    },
    Server {
        status: Option<u16>,
        detail: String,
    },
    Sidecar {
        detail: String,
    },
    Transport {
        detail: String,
    },
    InvalidResponse {
        detail: String,
    },
    ScriptedFailure {
        detail: String,
    },
}

impl fmt::Display for EmbedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedModality { backend, modality } => {
                write!(
                    f,
                    "embedder '{backend}' does not support modality '{modality}'"
                )
            }
            Self::MissingModelPath { backend } => {
                write!(
                    f,
                    "local embedder '{backend}' requires a configured model_path (weights are not vendored)"
                )
            }
            Self::ModelPathNotFound { backend, path } => {
                write!(
                    f,
                    "local embedder '{backend}' model_path '{path}' does not exist"
                )
            }
            Self::ContextLimitExceeded {
                backend,
                estimated_tokens,
                max_tokens,
            } => write!(
                f,
                "embedder '{backend}' input (~{estimated_tokens} tokens) exceeds context limit {max_tokens}"
            ),
            Self::DimensionMismatch { expected, actual } => {
                write!(
                    f,
                    "embedding dimension mismatch: expected {expected}, got {actual}"
                )
            }
            Self::NonFiniteVector => f.write_str("embedding vector contains NaN or Inf"),
            Self::ZeroNormVector => f.write_str("embedding vector has zero L2 norm"),
            Self::PartitionMismatch {
                expected,
                actual,
                reason,
            } => write!(
                f,
                "embedding partition mismatch (cannot compare ({}, {}, {}) with ({}, {}, {})): {}",
                expected.embed_model_ver,
                expected.dim_d,
                expected.prompt_regime_id,
                actual.embed_model_ver,
                actual.dim_d,
                actual.prompt_regime_id,
                reason
            ),
            Self::Authentication { detail } => {
                write!(f, "embedder authentication failed: {detail}")
            }
            Self::RateLimit { detail } => write!(f, "embedder rate limited: {detail}"),
            Self::Timeout { timeout_ms, detail } => {
                write!(f, "embedder timed out after {timeout_ms}ms: {detail}")
            }
            Self::Server { status, detail } => match status {
                Some(code) => write!(f, "embedder server error ({code}): {detail}"),
                None => write!(f, "embedder server error: {detail}"),
            },
            Self::Sidecar { detail } => write!(f, "embedder sidecar error: {detail}"),
            Self::Transport { detail } => write!(f, "embedder transport error: {detail}"),
            Self::InvalidResponse { detail } => write!(f, "invalid embedder response: {detail}"),
            Self::ScriptedFailure { detail } => write!(f, "scripted embedder failure: {detail}"),
        }
    }
}

impl std::error::Error for EmbedError {}

/// Unified embedding provider trait across `none`, `hash`, `scripted`, `openai`, and `EmbeddingGemma 2`.
pub trait Embedder: Send + Sync {
    /// Human-readable backend identifier.
    fn backend_name(&self) -> &str;

    /// Model version identifier (`None` for [`NoneEmbedder`]).
    fn model_ver(&self) -> Option<&str>;

    /// Output vector dimension (`None` for [`NoneEmbedder`]).
    fn dim_d(&self) -> Option<u32>;

    /// Maximum supported context tokens, if bounded.
    fn max_context_tokens(&self) -> Option<usize>;

    /// Supported input modalities.
    fn supported_modalities(&self) -> &'static [EmbedModality];

    /// Constructs the partition [`EmbeddingTag`] for `task` (`None` for [`NoneEmbedder`]).
    fn tag_for(&self, task: TaskPrefix) -> Option<EmbeddingTag>;

    /// Embeds `input`, returning `Ok(None)` when embeddings are disabled (`NoneEmbedder`).
    fn embed(&self, input: &EmbedInput<'_>) -> Result<Option<TaggedEmbedding>, EmbedError>;
}

/// L2-normalizes `values` in place, rejecting empty, non-finite, or zero-norm vectors.
pub fn l2_normalize(values: &mut [f32]) -> Result<(), EmbedError> {
    if values.is_empty() {
        return Err(EmbedError::ZeroNormVector);
    }
    let mut sum_sq = 0.0_f64;
    for &v in values.iter() {
        if !v.is_finite() {
            return Err(EmbedError::NonFiniteVector);
        }
        let vf = f64::from(v);
        sum_sq += vf * vf;
    }
    let norm = sum_sq.sqrt();
    if norm <= 0.0 || !norm.is_finite() {
        return Err(EmbedError::ZeroNormVector);
    }
    for v in values.iter_mut() {
        *v = (f64::from(*v) / norm) as f32;
    }
    Ok(())
}

/// Returns `true` if `values` is non-empty, finite, and unit L2-normalized within `tolerance`.
#[must_use]
pub fn is_l2_normalized(values: &[f32], tolerance: f32) -> bool {
    if values.is_empty() || !tolerance.is_finite() || tolerance < 0.0 {
        return false;
    }
    let mut sum_sq = 0.0_f64;
    for &v in values {
        if !v.is_finite() {
            return false;
        }
        let vf = f64::from(v);
        sum_sq += vf * vf;
    }
    (sum_sq.sqrt() - 1.0).abs() <= f64::from(tolerance)
}

/// Returns `true` iff all three partition tuple components `(embed_model_ver, dim_d, prompt_regime_id)` match.
#[must_use]
pub fn same_partition(a: &EmbeddingTag, b: &EmbeddingTag) -> bool {
    a.embed_model_ver == b.embed_model_ver
        && a.dim_d == b.dim_d
        && a.prompt_regime_id == b.prompt_regime_id
}

/// Verifies that `expected` and `actual` share the exact `(embed_model_ver, dim_d, prompt_regime_id)` partition.
pub fn check_partition_match(
    expected: &EmbeddingTag,
    actual: &EmbeddingTag,
) -> Result<(), EmbedError> {
    let mut diffs = Vec::new();
    if expected.embed_model_ver != actual.embed_model_ver {
        diffs.push(format!(
            "embed_model_ver '{}' != '{}'",
            expected.embed_model_ver, actual.embed_model_ver
        ));
    }
    if expected.dim_d != actual.dim_d {
        diffs.push(format!("dim_d {} != {}", expected.dim_d, actual.dim_d));
    }
    if expected.prompt_regime_id != actual.prompt_regime_id {
        diffs.push(format!(
            "prompt_regime_id '{}' != '{}'",
            expected.prompt_regime_id, actual.prompt_regime_id
        ));
    }
    if diffs.is_empty() {
        Ok(())
    } else {
        Err(EmbedError::PartitionMismatch {
            expected: Box::new(expected.clone()),
            actual: Box::new(actual.clone()),
            reason: diffs.join(", "),
        })
    }
}

/// Computes cosine similarity only if `a` and `b` belong to the exact same partition.
pub fn partitioned_cosine(a: &TaggedEmbedding, b: &TaggedEmbedding) -> Result<f32, EmbedError> {
    check_partition_match(&a.tag, &b.tag)?;
    if a.values.len() != b.values.len() || a.values.len() != a.tag.dim_d as usize {
        return Err(EmbedError::DimensionMismatch {
            expected: a.tag.dim_d,
            actual: b.values.len(),
        });
    }
    let mut dot = 0.0_f64;
    let mut norm_a = 0.0_f64;
    let mut norm_b = 0.0_f64;
    for (&x, &y) in a.values.iter().zip(b.values.iter()) {
        if !x.is_finite() || !y.is_finite() {
            return Err(EmbedError::NonFiniteVector);
        }
        let xf = f64::from(x);
        let yf = f64::from(y);
        dot += xf * yf;
        norm_a += xf * xf;
        norm_b += yf * yf;
    }
    let denom = norm_a.sqrt() * norm_b.sqrt();
    if denom <= 0.0 || !denom.is_finite() {
        return Err(EmbedError::ZeroNormVector);
    }
    Ok((dot / denom).clamp(-1.0, 1.0) as f32)
}

/// Result of a partition-isolated similarity search reporting any skipped incompatible partitions.
#[derive(Debug, Clone, PartialEq)]
pub struct PartitionedSearchResult {
    pub matches: Vec<(EventId, f32)>,
    pub skipped_mismatches: Vec<EmbedError>,
}

impl PartitionedSearchResult {
    /// Returns `true` if any stored vector belonged to a different `(embed_model_ver, dim_d, prompt_regime_id)` partition.
    #[must_use]
    pub fn has_partition_mismatch(&self) -> bool {
        !self.skipped_mismatches.is_empty()
    }
}

/// Searches `corpus` strictly within `query.tag`'s `(embed_model_ver, dim_d, prompt_regime_id)` partition,
/// never comparing incompatible vectors and reporting every mismatched partition encountered.
#[must_use]
pub fn search_partitioned(
    query: &TaggedEmbedding,
    corpus: &[(EventId, TaggedEmbedding)],
    top_k: usize,
) -> PartitionedSearchResult {
    search_against_partition(query, &query.tag, corpus, top_k)
}

/// Searches stored `t_d` vectors for an asymmetric `t_q` query strictly within
/// `(query.tag.embed_model_ver, query.tag.dim_d, "t_d")`, reporting any mismatched partitions.
#[must_use]
pub fn search_asymmetric_q_to_d(
    query: &TaggedEmbedding,
    corpus: &[(EventId, TaggedEmbedding)],
    top_k: usize,
) -> PartitionedSearchResult {
    let target_tag = EmbeddingTag {
        embed_model_ver: query.tag.embed_model_ver.clone(),
        dim_d: query.tag.dim_d,
        prompt_regime_id: TaskPrefix::Document.regime_id().to_owned(),
    };
    search_against_partition(query, &target_tag, corpus, top_k)
}

fn search_against_partition(
    query: &TaggedEmbedding,
    target_tag: &EmbeddingTag,
    corpus: &[(EventId, TaggedEmbedding)],
    top_k: usize,
) -> PartitionedSearchResult {
    let mut matches = Vec::new();
    let mut skipped_mismatches = Vec::new();
    let mut reported_tags: BTreeSet<(String, u32, String)> = BTreeSet::new();

    for (event_id, item) in corpus {
        if let Err(err) = check_partition_match(target_tag, &item.tag) {
            let key = (
                item.tag.embed_model_ver.clone(),
                item.tag.dim_d,
                item.tag.prompt_regime_id.clone(),
            );
            if reported_tags.insert(key) {
                skipped_mismatches.push(err);
            }
            continue;
        }
        if item.values.len() != query.values.len() {
            skipped_mismatches.push(EmbedError::DimensionMismatch {
                expected: target_tag.dim_d,
                actual: item.values.len(),
            });
            continue;
        }
        if let Ok(sim) = raw_cosine(&query.values, &item.values) {
            matches.push((*event_id, sim));
        }
    }

    matches.sort_by(|a, b| {
        b.1.total_cmp(&a.1)
            .then_with(|| a.0.as_u64().cmp(&b.0.as_u64()))
    });
    matches.truncate(top_k);

    PartitionedSearchResult {
        matches,
        skipped_mismatches,
    }
}

fn raw_cosine(a: &[f32], b: &[f32]) -> Result<f32, EmbedError> {
    let mut dot = 0.0_f64;
    let mut norm_a = 0.0_f64;
    let mut norm_b = 0.0_f64;
    for (&x, &y) in a.iter().zip(b.iter()) {
        if !x.is_finite() || !y.is_finite() {
            return Err(EmbedError::NonFiniteVector);
        }
        let xf = f64::from(x);
        let yf = f64::from(y);
        dot += xf * yf;
        norm_a += xf * xf;
        norm_b += yf * yf;
    }
    let denom = norm_a.sqrt() * norm_b.sqrt();
    if denom <= 0.0 || !denom.is_finite() {
        return Err(EmbedError::ZeroNormVector);
    }
    Ok((dot / denom).clamp(-1.0, 1.0) as f32)
}

/// Conservative token-count estimator (`~4` bytes per token, minimum 1 for non-empty text).
pub(crate) fn estimate_tokens(text: &str) -> usize {
    if text.is_empty() {
        0
    } else {
        text.len().div_ceil(4).max(1)
    }
}
