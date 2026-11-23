//! Data-driven provider catalog (`catalog.toml`, embedded at build time).
//!
//! A catalog entry names a wire dialect, a base URL, the environment variables that may hold
//! the API key, and how the model list is discovered. Dispatch keys on the dialect, never on
//! the provider id.

use std::sync::{OnceLock, RwLock};

use serde::{Deserialize, Serialize};

const CATALOG_TOML: &str = include_str!("catalog.toml");

/// Wire protocol spoken by a provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Dialect {
    /// `POST {base}/chat/completions`, Bearer auth.
    #[serde(rename = "openai_chat")]
    OpenAiChat,
    /// `POST {base}/messages`, `x-api-key` auth.
    #[serde(rename = "messages_api")]
    MessagesApi,
}

/// How a provider's model list is fetched (named by mechanism).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiscoveryMode {
    /// `GET {base}/models`, optional bearer key, `data[].id`.
    ModelsList,
    /// `GET {base}/models` with the key in `key_header` plus `extra_headers`; cursor paged.
    ModelsListHeaderKey,
    /// `GET {discovery_url}/models?key=...`, `models[].name`; paged by token.
    NativeModelsListQueryKey,
    None,
}

/// One provider preset.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogEntry {
    pub id: String,
    pub label: String,
    pub dialect: Dialect,
    pub base_url: String,
    #[serde(default)]
    pub api_key_env: Vec<String>,
    #[serde(default)]
    pub keyless: bool,
    pub discovery: DiscoveryMode,
    #[serde(default)]
    pub default_model: String,
    #[serde(default)]
    pub docs_url: String,
    #[serde(default)]
    pub seed_models: Vec<String>,
    /// Header carrying the API key for the messages dialect and header-key discovery.
    #[serde(default)]
    pub key_header: Option<String>,
    /// Fixed extra request headers (for example a protocol version header).
    #[serde(default)]
    pub extra_headers: std::collections::BTreeMap<String, String>,
    /// Native list URL root for `native_models_list_query_key`.
    #[serde(default)]
    pub discovery_url: Option<String>,
    /// Prefix stripped from discovered model ids.
    #[serde(default)]
    pub model_id_prefix: Option<String>,
    /// Optional JSON parameter name for the output-token limit (dialect default when absent).
    #[serde(default)]
    pub output_limit_param: Option<String>,
}

#[derive(Deserialize)]
struct CatalogFile {
    provider: Vec<CatalogEntry>,
}

/// Returns every embedded catalog entry in file order.
///
/// # Panics
/// Panics if the embedded `catalog.toml` is malformed (guarded by a unit test).
#[must_use]
pub fn entries() -> &'static [CatalogEntry] {
    static CATALOG: OnceLock<Vec<CatalogEntry>> = OnceLock::new();
    CATALOG.get_or_init(|| {
        toml::from_str::<CatalogFile>(CATALOG_TOML)
            .expect("embedded catalog.toml must parse")
            .provider
    })
}

fn custom_registry() -> &'static RwLock<Vec<&'static CatalogEntry>> {
    static CUSTOM: OnceLock<RwLock<Vec<&'static CatalogEntry>>> = OnceLock::new();
    CUSTOM.get_or_init(|| RwLock::new(Vec::new()))
}

/// Registers (or updates) a named custom provider so [`find`] resolves it. Ids that collide
/// with a built-in catalog id are ignored. Entries are leaked so lookups can hand out
/// `&'static` references; this only happens when a provider is added or edited.
pub fn register_custom(entry: CatalogEntry) {
    if entries().iter().any(|b| b.id == entry.id) {
        return;
    }
    let mut guard = custom_registry().write().expect("custom registry lock");
    if let Some(pos) = guard.iter().position(|c| c.id == entry.id) {
        if *guard[pos] == entry {
            return;
        }
        guard[pos] = Box::leak(Box::new(entry));
    } else {
        guard.push(Box::leak(Box::new(entry)));
    }
}

/// Named custom providers registered in this process, in registration order.
#[must_use]
pub fn custom_entries() -> Vec<&'static CatalogEntry> {
    custom_registry().read().expect("custom registry lock").clone()
}

/// Built-in entries followed by registered custom providers.
#[must_use]
pub fn all_entries() -> Vec<&'static CatalogEntry> {
    entries().iter().chain(custom_entries()).collect()
}

/// Looks up a catalog entry by id (exact match), built-in first, then named custom.
#[must_use]
pub fn find(id: &str) -> Option<&'static CatalogEntry> {
    let id = id.trim();
    entries()
        .iter()
        .find(|e| e.id == id)
        .or_else(|| custom_entries().into_iter().find(|e| e.id == id))
}

/// Derives a custom provider name from a base URL host (`gw.example` -> `gw-example`).
#[must_use]
pub fn custom_name_from_url(url: &str) -> String {
    let host = url
        .split("://")
        .nth(1)
        .unwrap_or(url)
        .split(['/', ':', '?'])
        .next()
        .unwrap_or("")
        .trim();
    let name: String = host
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
        .collect();
    let name = name.trim_matches('-').to_owned();
    if name.is_empty() { "custom-provider".to_owned() } else { name }
}

/// Builds a named custom provider entry (key-requiring unless `keyless`).
#[must_use]
pub fn custom_entry(name: &str, base_url: &str, dialect: Dialect, keyless: bool) -> CatalogEntry {
    CatalogEntry {
        id: name.to_owned(),
        label: format!("{name} (custom)"),
        dialect,
        base_url: base_url.trim().trim_end_matches('/').to_owned(),
        api_key_env: Vec::new(),
        keyless,
        discovery: DiscoveryMode::ModelsList,
        default_model: String::new(),
        docs_url: String::new(),
        seed_models: Vec::new(),
        key_header: None,
        extra_headers: std::collections::BTreeMap::new(),
        discovery_url: None,
        model_id_prefix: None,
        output_limit_param: None,
    }
}

/// `true` when `base_url` (ignoring a trailing slash and the legacy `/chat/completions`
/// suffix) equals the entry's base URL.
#[must_use]
pub fn find_by_endpoint(endpoint: &str) -> Option<&'static CatalogEntry> {
    let norm = |u: &str| {
        let u = u.trim().trim_end_matches('/');
        let u = u.strip_suffix("/chat/completions").unwrap_or(u);
        let u = u.strip_suffix("/messages").unwrap_or(u);
        u.trim_end_matches('/').to_ascii_lowercase()
    };
    let want = norm(endpoint);
    entries()
        .iter()
        .find(|e| !e.base_url.is_empty() && norm(&e.base_url) == want)
}

impl CatalogEntry {
    /// Effective base URL: `override_url` (when non-empty) or the catalog default, without a
    /// trailing slash.
    #[must_use]
    pub fn effective_base_url(&self, override_url: Option<&str>) -> String {
        let raw = override_url
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(&self.base_url);
        raw.trim_end_matches('/').to_owned()
    }

    /// Full chat/messages endpoint for this entry's dialect.
    #[must_use]
    pub fn chat_endpoint(&self, override_url: Option<&str>) -> String {
        let base = self.effective_base_url(override_url);
        match self.dialect {
            Dialect::OpenAiChat => format!("{base}/chat/completions"),
            Dialect::MessagesApi => format!("{base}/messages"),
        }
    }
}
