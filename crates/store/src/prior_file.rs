//! Host-side `CoreConfig` file loading, strict validation, and canonical BLAKE3 digest computation.
//!
//! Lives outside `monoagens-core` to keep core free of filesystem IO and `serde_json`
//! dependencies (`core-and-lab` Decision 20).

use core::fmt;
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use monoagens_core::{ConfigError, CoreConfig};

/// Validated [`CoreConfig`] paired with its canonical BLAKE3 hex digest.
#[derive(Debug, Clone, PartialEq)]
pub struct LoadedCoreConfig {
    pub config: CoreConfig,
    pub digest: String,
}

impl LoadedCoreConfig {
    /// Validates `config` and computes its canonical JSON BLAKE3 hex digest.
    pub fn from_config(config: CoreConfig) -> Result<Self, ConfigLoadError> {
        config.validate()?;
        let digest = compute_config_digest(&config)?;
        Ok(Self { config, digest })
    }
}

/// Errors produced when reading, parsing, validating, or digesting a config file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigLoadError {
    Read {
        path: String,
        detail: String,
    },
    Parse {
        path: Option<String>,
        detail: String,
    },
    Validation(ConfigError),
    Serialize(String),
}

impl fmt::Display for ConfigLoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read { path, detail } => {
                write!(f, "failed to read config file '{path}': {detail}")
            }
            Self::Parse {
                path: Some(path),
                detail,
            } => write!(f, "failed to parse config file '{path}': {detail}"),
            Self::Parse { path: None, detail } => write!(f, "failed to parse config: {detail}"),
            Self::Validation(err) => write!(f, "{err}"),
            Self::Serialize(detail) => {
                write!(f, "failed to serialize canonical config JSON: {detail}")
            }
        }
    }
}

impl std::error::Error for ConfigLoadError {}

impl From<ConfigError> for ConfigLoadError {
    fn from(err: ConfigError) -> Self {
        Self::Validation(err)
    }
}

/// Computes the canonical BLAKE3 hex digest over `serde_json::to_vec(config)`.
pub fn compute_config_digest(config: &CoreConfig) -> Result<String, ConfigLoadError> {
    let canonical =
        serde_json::to_vec(config).map_err(|e| ConfigLoadError::Serialize(e.to_string()))?;
    Ok(blake3::hash(&canonical).to_hex().to_string())
}

/// Parses and validates a `CoreConfig` JSON string and computes its canonical BLAKE3 digest.
///
/// Rejects unknown top-level keys so typos never silently fall back to defaults.
pub fn load_core_config_from_str(
    raw: &str,
    source_path: Option<&Path>,
) -> Result<LoadedCoreConfig, ConfigLoadError> {
    let val: serde_json::Value = serde_json::from_str(raw).map_err(|e| ConfigLoadError::Parse {
        path: source_path.map(|p| p.display().to_string()),
        detail: e.to_string(),
    })?;
    let obj = val.as_object().ok_or_else(|| ConfigLoadError::Parse {
        path: source_path.map(|p| p.display().to_string()),
        detail: "config root must be a JSON object".to_owned(),
    })?;

    let default_val = serde_json::to_value(CoreConfig::default())
        .map_err(|e| ConfigLoadError::Serialize(e.to_string()))?;
    let known_keys: BTreeSet<&str> = default_val
        .as_object()
        .map(|m| m.keys().map(String::as_str).collect())
        .unwrap_or_default();
    for key in obj.keys() {
        if !known_keys.contains(key.as_str()) {
            return Err(ConfigLoadError::Parse {
                path: source_path.map(|p| p.display().to_string()),
                detail: format!("unknown field `{key}` in CoreConfig"),
            });
        }
    }

    let config: CoreConfig =
        serde_json::from_value(val).map_err(|e| ConfigLoadError::Parse {
            path: source_path.map(|p| p.display().to_string()),
            detail: e.to_string(),
        })?;
    LoadedCoreConfig::from_config(config)
}

/// Strictly loads a `CoreConfig` file from `path`, failing without falling back to the default.
pub fn load_core_config_file(path: &Path) -> Result<LoadedCoreConfig, ConfigLoadError> {
    let raw = fs::read_to_string(path).map_err(|e| ConfigLoadError::Read {
        path: path.display().to_string(),
        detail: e.to_string(),
    })?;
    load_core_config_from_str(&raw, Some(path))
}

/// Returns the validated built-in default [`CoreConfig`] and its canonical BLAKE3 digest.
pub fn default_loaded_core_config() -> Result<LoadedCoreConfig, ConfigLoadError> {
    LoadedCoreConfig::from_config(CoreConfig::default())
}

/// Loads `path` strictly if `Some`, or returns [`default_loaded_core_config`] when `None`.
pub fn load_core_config_or_default(
    path: Option<&Path>,
) -> Result<LoadedCoreConfig, ConfigLoadError> {
    match path {
        Some(p) => load_core_config_file(p),
        None => default_loaded_core_config(),
    }
}
