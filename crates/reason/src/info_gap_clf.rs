//! Frozen `A-14` logistic `dialogue_act_gap` head `(w_gap, b_gap)` operating at `O(d)`.
//!
//! Evaluates `sigma(w_gap . e + b_gap)` purely from numeric vectors without word lists,
//! and produces a bounded `[0.0, 1.0]` probability even when main embeddings are `none`.

use core::fmt;

use monoagens_core::{FrozenDialogueActHead, sigmoid};
use monoagens_protocol::{EmbeddingTag, TaggedEmbedding};
use serde::{Deserialize, Serialize};

use crate::embed::{EmbedInput, Embedder, check_partition_match};

/// Errors produced when constructing or strictly evaluating [`InfoGapClassifier`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InfoGapError {
    EmptyWeights,
    NonFiniteWeights,
    DimensionMismatch { expected: usize, actual: usize },
    NonFiniteFeature,
    PartitionMismatch(String),
}

impl fmt::Display for InfoGapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyWeights => f.write_str("w_gap must not be empty"),
            Self::NonFiniteWeights => f.write_str("w_gap and b_gap must be finite"),
            Self::DimensionMismatch { expected, actual } => {
                write!(
                    f,
                    "feature dimension mismatch: expected {expected}, got {actual}"
                )
            }
            Self::NonFiniteFeature => f.write_str("input feature vector contains NaN or Inf"),
            Self::PartitionMismatch(detail) => write!(f, "tag mismatch: {detail}"),
        }
    }
}

impl std::error::Error for InfoGapError {}

/// Result of scoring `dialogue_act_gap` via an [`Embedder`] with graceful `none`/error fallback.
#[derive(Debug, Clone, PartialEq)]
pub struct InfoGapEvaluation {
    pub score: f64,
    pub used_vector: bool,
    pub fallback_reason: Option<String>,
}

/// Frozen `A-14` logistic `dialogue_act_gap` scorer `(w_gap, b_gap)`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InfoGapClassifier {
    w_gap: Vec<f32>,
    b_gap: f32,
    expected_tag: Option<EmbeddingTag>,
}

impl InfoGapClassifier {
    /// Constructs and validates a frozen logistic head from numeric weights `(w_gap, b_gap)`.
    pub fn from_weights(
        w_gap: Vec<f32>,
        b_gap: f32,
        expected_tag: Option<EmbeddingTag>,
    ) -> Result<Self, InfoGapError> {
        if w_gap.is_empty() {
            return Err(InfoGapError::EmptyWeights);
        }
        if !b_gap.is_finite() || w_gap.iter().any(|w| !w.is_finite()) {
            return Err(InfoGapError::NonFiniteWeights);
        }
        Ok(Self {
            w_gap,
            b_gap,
            expected_tag,
        })
    }

    /// Returns the weight vector slice.
    #[must_use]
    pub fn w_gap(&self) -> &[f32] {
        &self.w_gap
    }

    /// Returns the logistic bias term.
    #[must_use]
    pub const fn b_gap(&self) -> f32 {
        self.b_gap
    }

    /// Returns the optional expected embedding partition tag.
    #[must_use]
    pub fn expected_tag(&self) -> Option<&EmbeddingTag> {
        self.expected_tag.as_ref()
    }

    /// Converts this head into Core's [`FrozenDialogueActHead`] representation.
    #[must_use]
    pub fn to_core_head(&self) -> FrozenDialogueActHead {
        FrozenDialogueActHead {
            w_gap: self.w_gap.clone(),
            b_gap: self.b_gap,
            expected_tag: self.expected_tag.clone(),
        }
    }

    /// Baseline intercept probability `sigma(b_gap)` in `[0.0, 1.0]` used when embeddings are `none`.
    #[must_use]
    pub fn neutral_prior_score(&self) -> f64 {
        sigmoid(f64::from(self.b_gap)).clamp(0.0, 1.0)
    }

    /// Evaluates `sigma(w_gap . features + b_gap)` in `O(d)` strictly from numbers.
    pub fn score_slice(&self, features: &[f32]) -> Result<f64, InfoGapError> {
        if features.len() != self.w_gap.len() {
            return Err(InfoGapError::DimensionMismatch {
                expected: self.w_gap.len(),
                actual: features.len(),
            });
        }
        let mut dot = 0.0_f64;
        for (&w, &x) in self.w_gap.iter().zip(features.iter()) {
            if !x.is_finite() {
                return Err(InfoGapError::NonFiniteFeature);
            }
            dot += f64::from(w) * f64::from(x);
        }
        let z = dot + f64::from(self.b_gap);
        Ok(sigmoid(z).clamp(0.0, 1.0))
    }

    /// Evaluates `sigma(w_gap . emb.values + b_gap)` in `O(d)` after verifying partition compatibility.
    pub fn score_embedding(&self, emb: &TaggedEmbedding) -> Result<f64, InfoGapError> {
        if let Some(expected) = &self.expected_tag {
            check_partition_match(expected, &emb.tag)
                .map_err(|err| InfoGapError::PartitionMismatch(err.to_string()))?;
        }
        self.score_slice(&emb.values)
    }

    /// Evaluates `dialogue_act_gap` in `[0.0, 1.0]` even when `maybe_emb` is `None` (`none` embeddings).
    #[must_use]
    pub fn score_optional_embedding(&self, maybe_emb: Option<&TaggedEmbedding>) -> f64 {
        match maybe_emb {
            Some(emb) => self
                .score_embedding(emb)
                .unwrap_or_else(|_| self.neutral_prior_score()),
            None => self.neutral_prior_score(),
        }
    }

    /// Scores `text` through `embedder` under the mandatory `t_clf` prefix, returning a bounded
    /// `[0.0, 1.0]` probability even when `embedder` is `NoneEmbedder` or fails for this event.
    #[must_use]
    pub fn score_with_embedder(&self, text: &str, embedder: &dyn Embedder) -> InfoGapEvaluation {
        let input = EmbedInput::classify(text);
        match embedder.embed(&input) {
            Ok(Some(emb)) => match self.score_embedding(&emb) {
                Ok(score) => InfoGapEvaluation {
                    score,
                    used_vector: true,
                    fallback_reason: None,
                },
                Err(err) => InfoGapEvaluation {
                    score: self.neutral_prior_score(),
                    used_vector: false,
                    fallback_reason: Some(err.to_string()),
                },
            },
            Ok(None) => InfoGapEvaluation {
                score: self.neutral_prior_score(),
                used_vector: false,
                fallback_reason: Some(
                    "none embedder active; using frozen logistic prior".to_owned(),
                ),
            },
            Err(err) => InfoGapEvaluation {
                score: self.neutral_prior_score(),
                used_vector: false,
                fallback_reason: Some(format!(
                    "embedder failed ({err}); using frozen logistic prior"
                )),
            },
        }
    }
}
