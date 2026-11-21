//! Compact deterministic structural configuration replacing V1's 184-field `Prior`
//! and 22-mechanism `Switches`/`registry.rs`.

use core::fmt;

use serde::{Deserialize, Serialize};

use crate::neural::{AdaptConfig, NeuralError};

/// Current supported `CoreConfig` schema version.
pub const CURRENT_CORE_CONFIG_VERSION: u16 = 1;

/// Validation error produced by [`CoreConfig::validate`] or [`CoreConfig::from_json_str`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigError {
    UnsupportedVersion {
        found: u16,
        max_supported: u16,
    },
    OutOfRange {
        field: &'static str,
        detail: String,
    },
    InvalidAdapt(NeuralError),
    Parse(String),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedVersion {
                found,
                max_supported,
            } => write!(
                f,
                "unsupported core config format_version {found} (supported: 1..={max_supported})"
            ),
            Self::OutOfRange { field, detail } => {
                write!(f, "invalid core config '{field}': {detail}")
            }
            Self::InvalidAdapt(err) => write!(f, "invalid core adapt config: {err}"),
            Self::Parse(msg) => write!(f, "failed to parse core config: {msg}"),
        }
    }
}

impl std::error::Error for ConfigError {}

impl From<NeuralError> for ConfigError {
    fn from(err: NeuralError) -> Self {
        Self::InvalidAdapt(err)
    }
}

/// Compact V2 deterministic structural & safety configuration (`~23` operational fields).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CoreConfig {
    /// Stored config schema version (`1`).
    pub format_version: u16,
    /// Root PRNG seed for deterministic replay.
    pub seed: u64,
    /// Hard daily reasoning token/cost budget limit (`>= 0.0`).
    pub daily_budget_limit: f64,
    /// Priority Class A ingress capacity (`> 0`).
    pub max_queue_class_a: usize,
    /// Priority Class B ingress capacity (`> 0`).
    pub max_queue_class_b: usize,
    /// Priority Class C ingress capacity (`> 0`).
    pub max_queue_class_c: usize,
    /// Maximum backlog chunks emitted per wake (`>= 1`).
    pub max_backlog_chunks_per_wake: usize,
    /// Coalescing gap threshold for backlog chunking in ms (`> 0`).
    pub backlog_gap_ms: u64,
    /// Hard maximum wait patience horizon in ms (`> 0.0`).
    pub max_wait_ms: f32,
    /// When `true`, sample `gate_4` via `SeededRng` instead of deterministic argmax.
    pub stochastic_gate_sampling: bool,
    /// Debug toggle controlling bounded per-place Warm-Start UCB online adaptation.
    pub online_adaptation: bool,
    /// Bounded adaptation equalizer hyperparameters.
    pub adapt: AdaptConfig,
    /// Optional `.safetensors` model path metadata (IO happens strictly outside `monoagens-core`).
    pub model_path: Option<String>,
    /// Optional expected `BLAKE3` digest for the configured `.safetensors` artifact.
    pub expected_blake3: Option<[u8; 32]>,
    /// Context plan hard byte budget (`> 0`).
    pub max_context_bytes: usize,
    /// Maximum recalled memory items per query (`> 0`).
    pub max_recalled_memories: usize,
    /// Vector candidate retrieval cap (`> 0`).
    pub recall_vector_top_k: usize,
    /// Text candidate retrieval cap (`> 0`).
    pub recall_text_top_k: usize,
    /// Enables automatic retry recall on `need_recall`.
    pub auto_recall_enabled: bool,
    /// Minimum unconsolidated episodes before idle sleep consolidation triggers (`> 0`).
    pub sleep_min_episodes: usize,
    /// Base idle duration in ms before sleep consolidation triggers (`> 0`).
    pub sleep_idle_ms: u64,
    /// Petrov/ACT-R memory fade threshold for vector release (`fade_threshold`, e.g. `-3.29`).
    pub vector_fade_threshold: f64,
    /// Emits `[SELF_GUIDE]` summary slot in `ContextPlan`.
    pub self_guide_enabled: bool,
    /// Minimum certainty bar `[0.0, 0.99]` for thought reporting.
    pub certainty_bar: f64,
}

impl Default for CoreConfig {
    fn default() -> Self {
        Self {
            format_version: CURRENT_CORE_CONFIG_VERSION,
            seed: 1,
            daily_budget_limit: 100_000.0,
            max_queue_class_a: 1024,
            max_queue_class_b: 2048,
            max_queue_class_c: 4096,
            max_backlog_chunks_per_wake: 4,
            backlog_gap_ms: 60_000,
            max_wait_ms: 30_000.0,
            stochastic_gate_sampling: false,
            online_adaptation: true,
            adapt: AdaptConfig::default(),
            model_path: None,
            expected_blake3: None,
            max_context_bytes: 32_768,
            max_recalled_memories: 16,
            recall_vector_top_k: 40,
            recall_text_top_k: 20,
            auto_recall_enabled: true,
            sleep_min_episodes: 4,
            sleep_idle_ms: 300_000,
            vector_fade_threshold: -3.29,
            self_guide_enabled: true,
            certainty_bar: 0.5,
        }
    }
}

impl CoreConfig {
    /// Returns effective [`AdaptConfig`] synchronized with `self.online_adaptation`.
    #[must_use]
    pub fn effective_adapt_config(&self) -> AdaptConfig {
        AdaptConfig {
            online_adaptation: self.online_adaptation && self.adapt.online_adaptation,
            ..self.adapt
        }
    }

    /// Validates all operational bounds, failing loudly without silent clamping.
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.format_version == 0 || self.format_version > CURRENT_CORE_CONFIG_VERSION {
            return Err(ConfigError::UnsupportedVersion {
                found: self.format_version,
                max_supported: CURRENT_CORE_CONFIG_VERSION,
            });
        }
        if !self.daily_budget_limit.is_finite() || self.daily_budget_limit < 0.0 {
            return Err(ConfigError::OutOfRange {
                field: "daily_budget_limit",
                detail: "must be finite and >= 0.0".to_owned(),
            });
        }
        if self.max_queue_class_a == 0 {
            return Err(ConfigError::OutOfRange {
                field: "max_queue_class_a",
                detail: "must be > 0".to_owned(),
            });
        }
        if self.max_queue_class_b == 0 {
            return Err(ConfigError::OutOfRange {
                field: "max_queue_class_b",
                detail: "must be > 0".to_owned(),
            });
        }
        if self.max_queue_class_c == 0 {
            return Err(ConfigError::OutOfRange {
                field: "max_queue_class_c",
                detail: "must be > 0".to_owned(),
            });
        }
        if self.max_backlog_chunks_per_wake == 0 {
            return Err(ConfigError::OutOfRange {
                field: "max_backlog_chunks_per_wake",
                detail: "must be > 0".to_owned(),
            });
        }
        if self.backlog_gap_ms == 0 {
            return Err(ConfigError::OutOfRange {
                field: "backlog_gap_ms",
                detail: "must be > 0".to_owned(),
            });
        }
        if !self.max_wait_ms.is_finite() || self.max_wait_ms <= 0.0 {
            return Err(ConfigError::OutOfRange {
                field: "max_wait_ms",
                detail: "must be finite and > 0.0".to_owned(),
            });
        }
        self.adapt.validate()?;
        if self.max_context_bytes == 0 {
            return Err(ConfigError::OutOfRange {
                field: "max_context_bytes",
                detail: "must be > 0".to_owned(),
            });
        }
        if self.max_recalled_memories == 0 {
            return Err(ConfigError::OutOfRange {
                field: "max_recalled_memories",
                detail: "must be > 0".to_owned(),
            });
        }
        if self.recall_vector_top_k == 0 {
            return Err(ConfigError::OutOfRange {
                field: "recall_vector_top_k",
                detail: "must be > 0".to_owned(),
            });
        }
        if self.recall_text_top_k == 0 {
            return Err(ConfigError::OutOfRange {
                field: "recall_text_top_k",
                detail: "must be > 0".to_owned(),
            });
        }
        if self.sleep_min_episodes == 0 {
            return Err(ConfigError::OutOfRange {
                field: "sleep_min_episodes",
                detail: "must be > 0".to_owned(),
            });
        }
        if self.sleep_idle_ms == 0 {
            return Err(ConfigError::OutOfRange {
                field: "sleep_idle_ms",
                detail: "must be > 0".to_owned(),
            });
        }
        if self.vector_fade_threshold.is_nan() {
            return Err(ConfigError::OutOfRange {
                field: "vector_fade_threshold",
                detail: "must not be NaN".to_owned(),
            });
        }
        if !self.certainty_bar.is_finite() || !(0.0..=0.99).contains(&self.certainty_bar) {
            return Err(ConfigError::OutOfRange {
                field: "certainty_bar",
                detail: "must be finite and in [0.0, 0.99]".to_owned(),
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_core_config_is_valid() {
        let cfg = CoreConfig::default();
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn invalid_core_config_bounds_fail_loudly() {
        let neg_budget = CoreConfig {
            daily_budget_limit: -1.0,
            ..CoreConfig::default()
        };
        assert_eq!(
            neg_budget.validate(),
            Err(ConfigError::OutOfRange {
                field: "daily_budget_limit",
                detail: "must be finite and >= 0.0".to_owned(),
            })
        );

        let nan_budget = CoreConfig {
            daily_budget_limit: f64::NAN,
            ..CoreConfig::default()
        };
        assert!(matches!(
            nan_budget.validate(),
            Err(ConfigError::OutOfRange {
                field: "daily_budget_limit",
                ..
            })
        ));

        let future_ver = CoreConfig {
            format_version: 99,
            ..CoreConfig::default()
        };
        assert_eq!(
            future_ver.validate(),
            Err(ConfigError::UnsupportedVersion {
                found: 99,
                max_supported: CURRENT_CORE_CONFIG_VERSION,
            })
        );

        let bad_wait = CoreConfig {
            max_wait_ms: 0.0,
            ..CoreConfig::default()
        };
        assert!(matches!(
            bad_wait.validate(),
            Err(ConfigError::OutOfRange {
                field: "max_wait_ms",
                ..
            })
        ));

        let mut bad_adapt = CoreConfig::default();
        bad_adapt.adapt.max_adapt_delta = -0.5;
        assert!(matches!(
            bad_adapt.validate(),
            Err(ConfigError::InvalidAdapt(_))
        ));
    }
}
