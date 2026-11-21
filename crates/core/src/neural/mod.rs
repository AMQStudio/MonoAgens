//! Pure-Rust Neural Cognitive Controller (`V2` "1 Neural Link").
//!
//! Zero-heavy-runtime `f32` inference over frozen `256d` `EmbeddingGemma 2` vectors,
//! compact normalized numeric features (`x_num`), and per-place recurrent working
//! memory (`h_t`), with `BLAKE3` integrity verification, a mandatory neutral
//! fallback (`None`), and bounded per-place Warm-Start UCB online adaptation.
//!
//! # Canonical `safetensors` Contract (Python ↔ Rust 1-to-1 Parity)
//!
//! ## Required `__metadata__` Keys (UTF-8 decimal/string values)
//! - `feature_schema_version`: positive integer (e.g. `"1"` for `256d` emb + `18d` `x_num`)
//! - `d_in_emb`: embedding input width (e.g. `"256"`)
//! - `d_in_num`: numeric feature vector width (e.g. `"18"`)
//! - `d_e`: projected embedding width (e.g. `"64"` or `"128"`)
//! - `d_n`: projected `PLR` numeric feature width (e.g. `"32"` or `"64"`)
//! - `d_model`: trunk hidden width (e.g. `"128"` or `"256"`)
//! - `n_layers`: number of Pre-LN Residual SwiGLU blocks (`>= 1`)
//! - `tabm_k`: `TabM` implicit ensemble member count (`>= 1`, default `8`)
//! - `h_dim`: per-place recurrent working memory width (default `"64"`)
//! - `cell_type`: `"mingru"` or `"gated_deltanet2"`
//! - `temperature`: calibrated softmax temperature `T > 0.0` (e.g. `"1.0"`)
//! - `cic_threshold`: finite-sample CIC confidence threshold in `[0.0, 1.0]` (e.g. `"0.5"`)
//! - `max_wait_ms`: maximum wait horizon in ms `> 0.0` (e.g. `"30000.0"`)
//!
//! ## Optional `__metadata__` Keys
//! - `has_phase2_heads`: `"true"` or `"false"` (default `"false"`)
//! - `has_phase3_heads`: `"true"` or `"false"` (default `"false"`)
//! - `plr_n_frequencies`: if present, must match `stem.plr.freqs.shape[1]`
//! - `d_ff`: if present, must match `trunk.blocks.0.w_gate.weight.shape[0]`
//!
//! ## Canonical `F32` Tensor Names & Shapes (PyTorch row-major `[out_features, in_features]`)
//!
//! ### Modality-Fusion Stem
//! - `stem.emb.linear.weight`: `[d_e, d_in_emb]`
//! - `stem.emb.linear.bias`: `[d_e]`
//! - `stem.emb.norm.weight`: `[d_e]`
//! - `stem.emb.norm.bias`: `[d_e]`
//! - `stem.plr.freqs`: `[d_in_num, n_freq]` (`n_freq >= 1`)
//! - `stem.plr.linear.weight`: `[d_n, d_in_num * 2 * n_freq]`
//! - `stem.plr.linear.bias`: `[d_n]`
//! - `stem.fusion.weight`: `[d_model, d_e + d_n + h_dim]`
//! - `stem.fusion.bias`: `[d_model]`
//!
//! ### `TabM` + Pre-LN Residual SwiGLU Trunk (`l = 0..n_layers`)
//! - `trunk.tabm_in_scale`: `[tabm_k, d_model]`
//! - `trunk.blocks.{l}.norm.weight`: `[d_model]`
//! - `trunk.blocks.{l}.norm.bias`: `[d_model]`
//! - `trunk.blocks.{l}.w_gate.weight`: `[d_ff, d_model]`
//! - `trunk.blocks.{l}.w_up.weight`: `[d_ff, d_model]`
//! - `trunk.blocks.{l}.w_down.weight`: `[d_model, d_ff]`
//! - `trunk.blocks.{l}.tabm_r`: `[tabm_k, d_model]`
//! - `trunk.blocks.{l}.tabm_s`: `[tabm_k, d_model]`
//! - `trunk.final_norm.weight`: `[d_model]`
//! - `trunk.final_norm.bias`: `[d_model]`
//!
//! ### Per-Place Recurrent Working Memory (`cell_type`)
//! - When `cell_type == "mingru"`:
//!   - `memory.mingru.w_z.weight`: `[h_dim, d_model]`
//!   - `memory.mingru.w_z.bias`: `[h_dim]`
//!   - `memory.mingru.w_h.weight`: `[h_dim, d_model]`
//!   - `memory.mingru.w_h.bias`: `[h_dim]`
//! - When `cell_type == "gated_deltanet2"`:
//!   - `memory.gdn2.w_erase.weight`: `[h_dim, d_model]`
//!   - `memory.gdn2.w_erase.bias`: `[h_dim]`
//!   - `memory.gdn2.w_write.weight`: `[h_dim, d_model]`
//!   - `memory.gdn2.w_write.bias`: `[h_dim]`
//!   - `memory.gdn2.w_delta.weight`: `[h_dim, d_model]`
//!   - `memory.gdn2.w_delta.bias`: `[h_dim]`
//!
//! ### Phase 1 Required Output Heads
//! - `head.gate_4.weight`: `[4, d_model]` (class order: `[Silent, Note, Wait, Go]`)
//! - `head.gate_4.bias`: `[4]`
//! - `head.wait_delta_t.weight`: `[1, d_model]`
//! - `head.wait_delta_t.bias`: `[1]`
//!
//! ### Phase 2 Optional Output Heads (`has_phase2_heads == "true"`)
//! - `head.p_uncertainty.weight`: `[1, d_model]`
//! - `head.p_uncertainty.bias`: `[1]`
//! - `head.p_verify.weight`: `[1, d_model]`
//! - `head.p_verify.bias`: `[1]`
//!
//! ### Phase 3 Optional Output Heads (`has_phase3_heads == "true"`)
//! - `head.p_encode_importance.weight`: `[1, d_model]`
//! - `head.p_encode_importance.bias`: `[1]`
//! - `head.p_memory_relevance.weight`: `[1, d_model]`
//! - `head.p_memory_relevance.bias`: `[1]`

pub mod adapt;
pub mod features;
pub mod forward;
pub mod weights;

use core::fmt;

use serde::{Deserialize, Serialize};

pub use adapt::{AdaptConfig, AdaptFeedback, GateArm, PlaceAdaptState};
pub use features::{
    FEATURE_SCHEMA_VERSION, NumericFeatureInputs, cosine_similarity, extract_embedding_for_model,
    extract_features_from_state, extract_fmodal, extract_x_num, extract_x_num_with_fmodal,
    zero_embedding,
};
pub use forward::{NeuralOutputs, forward_step, neutral_outputs};
pub use weights::{
    CellType, NeuralModel, NeuralModelConfig, Phase2HeadWeights, Phase3HeadWeights,
    RecurrentCellWeights, SwiGluBlockWeights,
};

/// Default per-place recurrent working memory dimension.
pub const DEFAULT_H_DIM: usize = 64;

/// Default `EmbeddingGemma 2` frozen input width.
pub const DEFAULT_EMB_DIM: usize = 256;

/// Default V2 model-agnostic numeric feature count (`x_num`).
pub const DEFAULT_NUM_FEATURES: usize = 18;

/// Errors produced during neural weight loading, validation, or inference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NeuralError {
    /// Failed to parse the `safetensors` container or header.
    SafeTensorParse(String),
    /// Expected `BLAKE3` digest did not match the artifact bytes.
    Blake3Mismatch { expected: String, actual: String },
    /// Missing `__metadata__` section or required metadata key.
    MissingMetadata(&'static str),
    /// Metadata key could not be parsed or violated invariants.
    InvalidMetadata { key: &'static str, detail: String },
    /// Required tensor was absent from the `safetensors` artifact.
    MissingTensor(String),
    /// Tensor dtype was not `F32` or byte length was malformed.
    InvalidDtype { tensor: String, detail: String },
    /// Tensor shape did not match declared architecture dimensions.
    ShapeMismatch {
        tensor: String,
        expected: Vec<usize>,
        actual: Vec<usize>,
    },
    /// Tensor contained `NaN` or `Inf`.
    NonFiniteWeight { tensor: String, index: usize },
    /// Inference input slice did not match model input dimension.
    InputDimensionMismatch {
        field: &'static str,
        expected: usize,
        actual: usize,
    },
    /// Inference input contained `NaN` or `Inf`.
    NonFiniteInput { field: &'static str, index: usize },
    /// Invalid adaptation configuration.
    InvalidAdaptConfig(&'static str),
}

impl fmt::Display for NeuralError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SafeTensorParse(msg) => write!(f, "safetensors parse error: {msg}"),
            Self::Blake3Mismatch { expected, actual } => {
                write!(f, "neural artifact blake3 mismatch: expected {expected}, got {actual}")
            }
            Self::MissingMetadata(key) => {
                write!(f, "missing required neural metadata key '{key}'")
            }
            Self::InvalidMetadata { key, detail } => {
                write!(f, "invalid neural metadata '{key}': {detail}")
            }
            Self::MissingTensor(name) => write!(f, "missing required tensor '{name}'"),
            Self::InvalidDtype { tensor, detail } => {
                write!(f, "invalid dtype for tensor '{tensor}': {detail}")
            }
            Self::ShapeMismatch {
                tensor,
                expected,
                actual,
            } => write!(
                f,
                "shape mismatch for tensor '{tensor}': expected {expected:?}, got {actual:?}"
            ),
            Self::NonFiniteWeight { tensor, index } => {
                write!(f, "non-finite weight in tensor '{tensor}' at flat index {index}")
            }
            Self::InputDimensionMismatch {
                field,
                expected,
                actual,
            } => write!(
                f,
                "input dimension mismatch for '{field}': expected {expected}, got {actual}"
            ),
            Self::NonFiniteInput { field, index } => {
                write!(f, "non-finite input in '{field}' at index {index}")
            }
            Self::InvalidAdaptConfig(msg) => write!(f, "invalid adapt config: {msg}"),
        }
    }
}

impl std::error::Error for NeuralError {}

/// Gate class index mapping for `gate_4` output logits `[Silent, Note, Wait, Go]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GateClass {
    Silent = 0,
    Note = 1,
    Wait = 2,
    Go = 3,
}

impl GateClass {
    /// Returns all 4 gate classes in canonical logit index order.
    pub const ALL: [Self; 4] = [Self::Silent, Self::Note, Self::Wait, Self::Go];

    /// Converts an index `0..4` into a [`GateClass`].
    #[must_use]
    pub const fn from_index(index: usize) -> Option<Self> {
        match index {
            0 => Some(Self::Silent),
            1 => Some(Self::Note),
            2 => Some(Self::Wait),
            3 => Some(Self::Go),
            _ => None,
        }
    }
}
