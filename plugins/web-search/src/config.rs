//! Self-managed plugin configuration and state directory resolution for `plugin-web-search`.
//!
//! Reads `MONOAGENS_PLUGIN_CONFIG` (auto-generating a commented default `web-search.toml`
//! on first run only when the env var is set and non-empty) and resolves state persistence
//! via `MONOAGENS_PLUGIN_STATE_DIR` with fallback to `MONOAGENS_BODY_STATE_DIR`.
//!
//! API keys (`TAVILY_API_KEY`, `BRAVE_API_KEY`) are read strictly from process environment
//! variables and are never written to `web-search.toml` or logged.

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use monoagens_body_kit::load_state_dir_from_env;
use serde::{Deserialize, Serialize};

/// Environment variable pointing to the plugin's self-managed TOML config file.
pub const ENV_PLUGIN_CONFIG: &str = "MONOAGENS_PLUGIN_CONFIG";

/// Environment variable pointing to the plugin's self-managed state directory.
pub const ENV_PLUGIN_STATE_DIR: &str = "MONOAGENS_PLUGIN_STATE_DIR";

/// Environment variable for Tavily API key (never persisted to TOML).
pub const ENV_TAVILY_API_KEY: &str = "TAVILY_API_KEY";

/// Environment variable for Brave Search API key (never persisted to TOML).
pub const ENV_BRAVE_API_KEY: &str = "BRAVE_API_KEY";

/// Environment variable fallback for SearXNG base URL.
pub const ENV_SEARXNG_URL: &str = "SEARXNG_URL";

/// Current schema version for `web-search.toml`.
pub const WEB_SEARCH_CONFIG_VERSION: u32 = 1;

/// Default request timeout in seconds.
pub const DEFAULT_TIMEOUT_SECS: u64 = 20;

/// Default maximum redirect hops followed by `web.fetch`.
pub const DEFAULT_MAX_REDIRECTS: usize = 3;

/// Default maximum character limit for `web.fetch`.
pub const DEFAULT_MAX_CHARS: usize = 12_000;

/// Minimum allowed `max_chars` for `web.fetch`.
pub const MIN_MAX_CHARS: usize = 200;

/// Default maximum result count for `web.search`.
pub const DEFAULT_MAX_RESULTS: usize = 5;

/// Maximum allowed result count for `web.search`.
pub const MAX_SEARCH_RESULTS: usize = 10;

/// Default provider fallback chain.
pub fn default_providers() -> Vec<String> {
    vec![
        "searxng".to_owned(),
        "tavily".to_owned(),
        "brave".to_owned(),
        "duckduckgo".to_owned(),
    ]
}

/// Default allowed ports for SSRF-checked `web.fetch`.
pub fn default_allow_ports() -> Vec<u16> {
    vec![80, 443]
}

const fn default_version() -> u32 {
    WEB_SEARCH_CONFIG_VERSION
}

const fn default_timeout_secs() -> u64 {
    DEFAULT_TIMEOUT_SECS
}

const fn default_max_redirects() -> usize {
    DEFAULT_MAX_REDIRECTS
}

const fn default_max_chars_val() -> usize {
    DEFAULT_MAX_CHARS
}

const fn default_max_results_val() -> usize {
    DEFAULT_MAX_RESULTS
}

/// Self-managed configuration loaded from `web-search.toml` (`MONOAGENS_PLUGIN_CONFIG`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct WebSearchConfig {
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default = "default_providers")]
    pub providers: Vec<String>,
    #[serde(default)]
    pub searxng_url: String,
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
    #[serde(default = "default_allow_ports")]
    pub allow_ports: Vec<u16>,
    #[serde(default = "default_max_redirects")]
    pub max_redirects: usize,
    #[serde(default = "default_max_chars_val")]
    pub default_max_chars: usize,
    #[serde(default = "default_max_results_val")]
    pub default_max_results: usize,
}

impl Default for WebSearchConfig {
    fn default() -> Self {
        Self {
            version: WEB_SEARCH_CONFIG_VERSION,
            providers: default_providers(),
            searxng_url: String::new(),
            timeout_secs: DEFAULT_TIMEOUT_SECS,
            allow_ports: default_allow_ports(),
            max_redirects: DEFAULT_MAX_REDIRECTS,
            default_max_chars: DEFAULT_MAX_CHARS,
            default_max_results: DEFAULT_MAX_RESULTS,
        }
    }
}

impl WebSearchConfig {
    /// Loads configuration from `MONOAGENS_PLUGIN_CONFIG` if set and non-empty.
    ///
    /// - When `MONOAGENS_PLUGIN_CONFIG` is unset or empty, returns [`Self::default()`] without
    ///   creating any files on disk (safe for conformance runs).
    /// - When set to a non-existent path, creates parent directories, writes a commented
    ///   default `web-search.toml`, and returns [`Self::default()`].
    /// - When the file exists, parses and returns the configured settings.
    pub fn load_or_init_from_env() -> Result<Self, String> {
        let Some(raw) = env::var_os(ENV_PLUGIN_CONFIG) else {
            return Ok(Self::default());
        };
        let path_str = raw.to_string_lossy();
        let trimmed = path_str.trim();
        if trimmed.is_empty() {
            return Ok(Self::default());
        }
        Self::load_or_init_from_path(Path::new(trimmed))
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
                    "failed to write default web-search plugin config '{}': {e}",
                    path.display()
                )
            })?;
            return Ok(defaults);
        }

        let content = fs::read_to_string(path).map_err(|e| {
            format!(
                "failed to read web-search plugin config '{}': {e}",
                path.display()
            )
        })?;
        Self::from_toml_str(&content).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// Parses and validates a `web-search.toml` document into [`WebSearchConfig`].
    pub fn from_toml_str(input: &str) -> Result<Self, String> {
        let mut cfg: Self =
            toml::from_str(input).map_err(|e| format!("invalid web-search TOML: {e}"))?;
        if cfg.version == 0 || cfg.version > WEB_SEARCH_CONFIG_VERSION {
            return Err(format!(
                "unsupported config version {} (expected {})",
                cfg.version, WEB_SEARCH_CONFIG_VERSION
            ));
        }
        if cfg.providers.is_empty() {
            cfg.providers = default_providers();
        }
        if cfg.allow_ports.is_empty() {
            cfg.allow_ports = default_allow_ports();
        }
        cfg.timeout_secs = cfg.timeout_secs.max(1);
        cfg.default_max_chars = cfg.default_max_chars.max(MIN_MAX_CHARS);
        cfg.default_max_results = cfg.default_max_results.clamp(1, MAX_SEARCH_RESULTS);
        Ok(cfg)
    }

    /// Renders a clean, well-commented TOML representation of this config.
    ///
    /// API keys are intentionally excluded — they are read strictly from environment variables.
    #[must_use]
    pub fn to_toml_string(&self) -> String {
        let providers_toml = format_string_array(&self.providers);
        let ports_toml = format_u16_array(&self.allow_ports);
        let searxng_toml = format_toml_string(&self.searxng_url);
        format!(
            "# MonoAgens Web Search plugin configuration (self-managed)\n\
             # API keys (TAVILY_API_KEY, BRAVE_API_KEY) are read strictly from process env vars.\n\
             version = {}\n\n\
             # Ordered search provider fallback chain\n\
             providers = {}\n\n\
             # Optional self-hosted SearXNG base URL (e.g. \"http://127.0.0.1:8888\")\n\
             searxng_url = {}\n\n\
             # Outbound HTTP request timeout (seconds)\n\
             timeout_secs = {}\n\n\
             # Ports allowed by the fail-closed SSRF guard for web.fetch\n\
             allow_ports = {}\n\n\
             # Maximum manual SSRF-checked redirect hops for web.fetch\n\
             max_redirects = {}\n\n\
             # Default maximum extracted text characters returned by web.fetch (minimum 200)\n\
             default_max_chars = {}\n\n\
             # Default maximum search results returned by web.search (1..=10)\n\
             default_max_results = {}\n",
            self.version,
            providers_toml,
            searxng_toml,
            self.timeout_secs,
            ports_toml,
            self.max_redirects,
            self.default_max_chars,
            self.default_max_results,
        )
    }
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

/// Trait abstracting environment variable reads so tests never mutate global process env.
pub trait EnvSource: Send + Sync {
    fn get_nonempty(&self, name: &str) -> Option<String>;
}

/// Reads environment variables strictly from the current process environment.
#[derive(Debug, Default, Clone, Copy)]
pub struct ProcessEnv;

impl EnvSource for ProcessEnv {
    fn get_nonempty(&self, name: &str) -> Option<String> {
        env::var(name).ok().filter(|v| !v.trim().is_empty())
    }
}

/// In-memory [`EnvSource`] for offline unit and integration tests.
#[derive(Debug, Default, Clone)]
pub struct MemoryEnv {
    vars: BTreeMap<String, String>,
}

impl MemoryEnv {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, key: impl Into<String>, val: impl Into<String>) {
        self.vars.insert(key.into(), val.into());
    }
}

impl EnvSource for MemoryEnv {
    fn get_nonempty(&self, name: &str) -> Option<String> {
        self.vars.get(name).filter(|v| !v.trim().is_empty()).cloned()
    }
}

fn format_toml_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn format_string_array(items: &[String]) -> String {
    let inner: Vec<String> = items.iter().map(|s| format_toml_string(s)).collect();
    format!("[{}]", inner.join(", "))
}

fn format_u16_array(items: &[u16]) -> String {
    let inner: Vec<String> = items.iter().map(u16::to_string).collect();
    format!("[{}]", inner.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn toml_roundtrip_preserves_fields_and_excludes_api_keys() {
        let cfg = WebSearchConfig {
            version: 1,
            providers: vec!["tavily".to_owned(), "duckduckgo".to_owned()],
            searxng_url: "https://search.example.org".to_owned(),
            timeout_secs: 15,
            allow_ports: vec![80, 443, 8443],
            max_redirects: 5,
            default_max_chars: 8_000,
            default_max_results: 7,
        };
        let rendered = cfg.to_toml_string();
        assert!(!rendered.contains("TAVILY_API_KEY ="));
        assert!(!rendered.contains("BRAVE_API_KEY ="));
        let parsed = WebSearchConfig::from_toml_str(&rendered).unwrap();
        assert_eq!(parsed, cfg);
    }

    #[test]
    fn toml_rejects_unsupported_version() {
        let err = WebSearchConfig::from_toml_str("version = 99\n").unwrap_err();
        assert!(err.contains("unsupported config version 99"));
    }

    #[test]
    fn load_or_init_from_path_writes_default_and_reloads_edits() {
        let dir = TempDir::new().unwrap();
        let cfg_path = dir.path().join("nested/web-search.toml");
        assert!(!cfg_path.exists());

        let initial = WebSearchConfig::load_or_init_from_path(&cfg_path).unwrap();
        assert_eq!(initial, WebSearchConfig::default());
        assert!(cfg_path.exists());

        let written = fs::read_to_string(&cfg_path).unwrap();
        assert!(written.contains("version = 1"));
        assert!(written.contains("allow_ports = [80, 443]"));

        // Modify config on disk and reload.
        fs::write(
            &cfg_path,
            "version = 1\ntimeout_secs = 30\ndefault_max_chars = 6000\ndefault_max_results = 3\n",
        )
        .unwrap();
        let reloaded = WebSearchConfig::load_or_init_from_path(&cfg_path).unwrap();
        assert_eq!(reloaded.timeout_secs, 30);
        assert_eq!(reloaded.default_max_chars, 6_000);
        assert_eq!(reloaded.default_max_results, 3);
        assert_eq!(reloaded.allow_ports, vec![80, 443]);
    }
}
