//! Self-managed plugin configuration and state directory resolution for `plugin-pdf-reader`.
//!
//! Reads `MONOAGENS_PLUGIN_CONFIG` (auto-generating a commented default `pdf-reader.toml`
//! on first run only when the env var is set and non-empty) and resolves state persistence
//! via `MONOAGENS_PLUGIN_STATE_DIR` with fallback to `MONOAGENS_BODY_STATE_DIR`.

#![forbid(unsafe_code)]

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use monoagens_body_kit::load_state_dir_from_env;
use serde::{Deserialize, Serialize};

/// Environment variable pointing to the plugin's self-managed TOML config file.
pub const ENV_PLUGIN_CONFIG: &str = "MONOAGENS_PLUGIN_CONFIG";

/// Environment variable pointing to the plugin's self-managed state directory.
pub const ENV_PLUGIN_STATE_DIR: &str = "MONOAGENS_PLUGIN_STATE_DIR";

/// Environment variable overriding the optional local embedding HTTP sidecar endpoint.
pub const ENV_EMBED_ENDPOINT: &str = "MONOAGENS_EMBED_ENDPOINT";

/// Current schema version for `pdf-reader.toml`.
pub const PDF_READER_CONFIG_VERSION: u32 = 1;

/// Default maximum characters per indexed chunk.
pub const DEFAULT_CHUNK_CHARS: usize = 900;

/// Default character overlap between consecutive chunks on a page.
pub const DEFAULT_CHUNK_OVERLAP_CHARS: usize = 150;

/// Default number of search hits returned when `top_k` is omitted.
pub const DEFAULT_TOP_K: usize = 5;

/// Hard upper bound on `top_k` returned by `pdf.search`.
pub const DEFAULT_MAX_TOP_K: usize = 20;

/// Hard upper bound on pages returned per `pdf.read_page` invocation.
pub const DEFAULT_MAX_PAGES_PER_READ: usize = 5;

/// Default Reciprocal Rank Fusion constant `k`.
pub const DEFAULT_RRF_K: f64 = 60.0;

/// Default BM25 term-frequency saturation parameter `k1`.
pub const DEFAULT_BM25_K1: f64 = 1.5;

/// Default BM25 document-length normalization parameter `b`.
pub const DEFAULT_BM25_B: f64 = 0.75;

/// Self-managed configuration loaded from `pdf-reader.toml` (`MONOAGENS_PLUGIN_CONFIG`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PdfReaderConfig {
    pub version: u32,
    pub chunk_chars: usize,
    pub chunk_overlap_chars: usize,
    pub default_top_k: usize,
    pub max_top_k: usize,
    pub max_pages_per_read: usize,
    pub embed_endpoint: Option<String>,
    pub rrf_k: f64,
    pub bm25_k1: f64,
    pub bm25_b: f64,
}

impl Default for PdfReaderConfig {
    fn default() -> Self {
        Self {
            version: PDF_READER_CONFIG_VERSION,
            chunk_chars: DEFAULT_CHUNK_CHARS,
            chunk_overlap_chars: DEFAULT_CHUNK_OVERLAP_CHARS,
            default_top_k: DEFAULT_TOP_K,
            max_top_k: DEFAULT_MAX_TOP_K,
            max_pages_per_read: DEFAULT_MAX_PAGES_PER_READ,
            embed_endpoint: None,
            rrf_k: DEFAULT_RRF_K,
            bm25_k1: DEFAULT_BM25_K1,
            bm25_b: DEFAULT_BM25_B,
        }
    }
}

impl PdfReaderConfig {
    /// Loads configuration from `MONOAGENS_PLUGIN_CONFIG` if set and non-empty.
    ///
    /// - When `MONOAGENS_PLUGIN_CONFIG` is unset or empty, returns [`Self::default()`] without
    ///   creating any files on disk (safe for conformance runs).
    /// - When set to a non-existent path, creates parent directories, writes a commented
    ///   default `pdf-reader.toml`, and returns [`Self::default()`].
    /// - When the file exists, parses and returns the configured settings.
    /// - Also checks `MONOAGENS_EMBED_ENDPOINT` when resolving the embedding sidecar URL.
    pub fn load_or_init_from_env() -> Result<Self, String> {
        let mut cfg = match env::var_os(ENV_PLUGIN_CONFIG) {
            Some(raw) => {
                let path_str = raw.to_string_lossy();
                let trimmed = path_str.trim();
                if trimmed.is_empty() {
                    Self::default()
                } else {
                    Self::load_or_init_from_path(Path::new(trimmed))?
                }
            }
            None => Self::default(),
        };

        if let Some(ep) = load_embed_endpoint_from_env() {
            cfg.embed_endpoint = Some(ep);
        }
        Ok(cfg)
    }

    /// Loads configuration from `path`, creating a commented default file if it does not exist.
    pub fn load_or_init_from_path(path: &Path) -> Result<Self, String> {
        if !path.exists() {
            if let Some(parent) = path.parent()
                && !parent.as_os_str().is_empty()
            {
                fs::create_dir_all(parent).map_err(|e| {
                    format!(
                        "failed to create plugin config parent directory '{}': {e}",
                        parent.display()
                    )
                })?;
            }
            let defaults = Self::default();
            fs::write(path, defaults.to_toml_string()).map_err(|e| {
                format!(
                    "failed to write default pdf-reader plugin config '{}': {e}",
                    path.display()
                )
            })?;
            return Ok(defaults);
        }

        let content = fs::read_to_string(path).map_err(|e| {
            format!(
                "failed to read pdf-reader plugin config '{}': {e}",
                path.display()
            )
        })?;
        Self::from_toml_str(&content).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// Parses a `pdf-reader.toml` document into [`PdfReaderConfig`] and validates bounds.
    pub fn from_toml_str(input: &str) -> Result<Self, String> {
        let mut cfg: Self =
            toml::from_str(input).map_err(|e| format!("invalid pdf-reader TOML config: {e}"))?;

        if cfg.version == 0 || cfg.version > PDF_READER_CONFIG_VERSION {
            return Err(format!(
                "unsupported config version {} (expected 1)",
                cfg.version
            ));
        }
        if cfg.chunk_chars == 0 {
            return Err("chunk_chars must be >= 1".to_owned());
        }
        if cfg.default_top_k == 0 {
            return Err("default_top_k must be >= 1".to_owned());
        }
        if cfg.max_top_k == 0 {
            return Err("max_top_k must be >= 1".to_owned());
        }
        if cfg.max_pages_per_read == 0 {
            return Err("max_pages_per_read must be >= 1".to_owned());
        }
        if !cfg.rrf_k.is_finite() || cfg.rrf_k <= 0.0 {
            return Err("rrf_k must be a positive finite number".to_owned());
        }
        if !cfg.bm25_k1.is_finite() || cfg.bm25_k1 < 0.0 {
            return Err("bm25_k1 must be a non-negative finite number".to_owned());
        }
        if !cfg.bm25_b.is_finite() || !(0.0..=1.0).contains(&cfg.bm25_b) {
            return Err("bm25_b must be between 0.0 and 1.0".to_owned());
        }

        cfg.embed_endpoint = cfg
            .embed_endpoint
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty());

        Ok(cfg)
    }

    /// Renders a clean, well-commented TOML representation of this config.
    #[must_use]
    pub fn to_toml_string(&self) -> String {
        let embed_line = match self.embed_endpoint.as_deref() {
            Some(ep) if !ep.trim().is_empty() => {
                format!(
                    "embed_endpoint = {}",
                    toml::Value::String(ep.to_owned())
                )
            }
            _ => "# embed_endpoint = \"http://127.0.0.1:8080\"".to_owned(),
        };

        format!(
            "# MonoAgens PDF Reader plugin configuration (self-managed, RAG-first)\n\
             version = {}\n\n\
             # Maximum characters per page-aware chunk\n\
             chunk_chars = {}\n\n\
             # Overlapping characters carried between consecutive chunks on a page\n\
             chunk_overlap_chars = {}\n\n\
             # Default and maximum top-K chunks returned by pdf.search\n\
             default_top_k = {}\n\
             max_top_k = {}\n\n\
             # Maximum pages returned per pdf.read_page call\n\
             max_pages_per_read = {}\n\n\
             # Optional local EmbeddingGemma 2 HTTP sidecar URL (also reads MONOAGENS_EMBED_ENDPOINT)\n\
             {}\n\n\
             # Hybrid retrieval parameters (BM25 + Reciprocal Rank Fusion)\n\
             rrf_k = {}\n\
             bm25_k1 = {}\n\
             bm25_b = {}\n",
            self.version,
            self.chunk_chars,
            self.chunk_overlap_chars,
            self.default_top_k,
            self.max_top_k,
            self.max_pages_per_read,
            embed_line,
            self.rrf_k,
            self.bm25_k1,
            self.bm25_b,
        )
    }

    /// Returns the effective embedding sidecar endpoint URL, checking `MONOAGENS_EMBED_ENDPOINT`
    /// first and falling back to `self.embed_endpoint`.
    #[must_use]
    pub fn resolved_embed_endpoint(&self) -> Option<String> {
        load_embed_endpoint_from_env().or_else(|| {
            self.embed_endpoint
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(ToOwned::to_owned)
        })
    }
}

/// Reads `MONOAGENS_EMBED_ENDPOINT` if set and non-empty.
#[must_use]
pub fn load_embed_endpoint_from_env() -> Option<String> {
    env::var_os(ENV_EMBED_ENDPOINT)
        .map(|v| v.to_string_lossy().trim().to_owned())
        .filter(|s| !s.is_empty())
}

/// Resolves the state directory from `MONOAGENS_PLUGIN_STATE_DIR` first,
/// falling back to `MONOAGENS_BODY_STATE_DIR`.
#[must_use]
pub fn resolve_state_dir_from_env() -> Option<PathBuf> {
    env::var_os(ENV_PLUGIN_STATE_DIR)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(load_state_dir_from_env)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn toml_roundtrip_and_defaults() {
        let cfg = PdfReaderConfig {
            version: 1,
            chunk_chars: 1200,
            chunk_overlap_chars: 200,
            default_top_k: 7,
            max_top_k: 30,
            max_pages_per_read: 4,
            embed_endpoint: Some("http://127.0.0.1:9090".to_owned()),
            rrf_k: 50.0,
            bm25_k1: 1.2,
            bm25_b: 0.8,
        };
        let rendered = cfg.to_toml_string();
        let parsed = PdfReaderConfig::from_toml_str(&rendered).unwrap();
        assert_eq!(parsed, cfg);
    }

    #[test]
    fn load_or_init_from_path_creates_default_and_reloads() {
        let dir = TempDir::new().unwrap();
        let cfg_path = dir.path().join("nested/pdf-reader.toml");

        let created = PdfReaderConfig::load_or_init_from_path(&cfg_path).unwrap();
        assert_eq!(created, PdfReaderConfig::default());
        assert!(cfg_path.exists());

        let contents = fs::read_to_string(&cfg_path).unwrap();
        assert!(contents.contains("chunk_chars = 900"));
        assert!(contents.contains("max_pages_per_read = 5"));

        // Overwrite with custom config and reload.
        fs::write(
            &cfg_path,
            "version = 1\nchunk_chars = 600\nmax_pages_per_read = 2\n",
        )
        .unwrap();
        let reloaded = PdfReaderConfig::load_or_init_from_path(&cfg_path).unwrap();
        assert_eq!(reloaded.chunk_chars, 600);
        assert_eq!(reloaded.max_pages_per_read, 2);
        assert_eq!(reloaded.chunk_overlap_chars, DEFAULT_CHUNK_OVERLAP_CHARS);
    }

    #[test]
    fn toml_rejects_invalid_version_and_bounds() {
        let err = PdfReaderConfig::from_toml_str("version = 99\n").unwrap_err();
        assert!(err.contains("unsupported config version 99"));

        let err2 = PdfReaderConfig::from_toml_str("bm25_b = 1.5\n").unwrap_err();
        assert!(err2.contains("bm25_b"));
    }
}
