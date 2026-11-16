//! Host-side prior file loading, strict validation, and canonical BLAKE3 digest computation.
//!
//! Lives outside `monoagens-core` to keep core free of filesystem IO and `serde_json`
//! dependencies (`core-and-lab` Decision 20).

use core::fmt;
use std::fs;
use std::path::Path;

use monoagens_core::{Prior, PriorError};

/// Validated [`Prior`] paired with its canonical BLAKE3 hex digest.
#[derive(Debug, Clone, PartialEq)]
pub struct LoadedPrior {
    pub prior: Prior,
    pub digest: String,
}

impl LoadedPrior {
    /// Validates `prior` and computes its canonical JSON BLAKE3 hex digest.
    pub fn from_prior(prior: Prior) -> Result<Self, PriorLoadError> {
        prior.validate()?;
        let digest = compute_prior_digest(&prior)?;
        Ok(Self { prior, digest })
    }
}

/// Errors produced when reading, parsing, validating, or digesting a prior file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PriorLoadError {
    Read {
        path: String,
        detail: String,
    },
    Parse {
        path: Option<String>,
        detail: String,
    },
    Validation(PriorError),
    Serialize(String),
}

impl fmt::Display for PriorLoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read { path, detail } => {
                write!(f, "failed to read prior file '{path}': {detail}")
            }
            Self::Parse {
                path: Some(path),
                detail,
            } => write!(f, "failed to parse prior file '{path}': {detail}"),
            Self::Parse { path: None, detail } => write!(f, "failed to parse prior: {detail}"),
            Self::Validation(err) => write!(f, "{err}"),
            Self::Serialize(detail) => {
                write!(f, "failed to serialize canonical prior JSON: {detail}")
            }
        }
    }
}

impl std::error::Error for PriorLoadError {}

impl From<PriorError> for PriorLoadError {
    fn from(err: PriorError) -> Self {
        Self::Validation(err)
    }
}

/// Computes the canonical BLAKE3 hex digest over `serde_json::to_vec(prior)`.
pub fn compute_prior_digest(prior: &Prior) -> Result<String, PriorLoadError> {
    let canonical =
        serde_json::to_vec(prior).map_err(|e| PriorLoadError::Serialize(e.to_string()))?;
    Ok(blake3::hash(&canonical).to_hex().to_string())
}

/// Parses and validates a prior JSON string and computes its canonical BLAKE3 digest.
pub fn load_prior_from_str(
    raw: &str,
    source_path: Option<&Path>,
) -> Result<LoadedPrior, PriorLoadError> {
    let prior: Prior = serde_json::from_str(raw).map_err(|e| PriorLoadError::Parse {
        path: source_path.map(|p| p.display().to_string()),
        detail: e.to_string(),
    })?;
    LoadedPrior::from_prior(prior)
}

/// Strictly loads a prior file from `path`, failing without falling back to the default.
pub fn load_prior_file(path: &Path) -> Result<LoadedPrior, PriorLoadError> {
    let raw = fs::read_to_string(path).map_err(|e| PriorLoadError::Read {
        path: path.display().to_string(),
        detail: e.to_string(),
    })?;
    load_prior_from_str(&raw, Some(path))
}

/// Returns the validated built-in default [`Prior`] and its canonical BLAKE3 digest.
pub fn default_loaded_prior() -> Result<LoadedPrior, PriorLoadError> {
    LoadedPrior::from_prior(Prior::default())
}

/// Loads `path` strictly if `Some`, or returns [`default_loaded_prior`] when `None`.
pub fn load_prior_or_default(path: Option<&Path>) -> Result<LoadedPrior, PriorLoadError> {
    match path {
        Some(p) => load_prior_file(p),
        None => default_loaded_prior(),
    }
}
