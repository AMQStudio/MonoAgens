//! Data-driven provider catalog (`catalog.toml`, embedded at build time).
//!
//! A catalog entry names a wire dialect, a base URL, the environment variables that may hold
//! the API key, and how the model list is discovered. Dispatch keys on the dialect, never on
//! the provider id.

use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

const CATALOG_TOML: &str = include_str!("catalog.toml");

/// Id of the free-form entry whose base URL comes from the user.
pub const CUSTOM_PROVIDER_ID: &str = "custom";

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

/// Looks up a catalog entry by id (exact match).
#[must_use]
pub fn find(id: &str) -> Option<&'static CatalogEntry> {
    let id = id.trim();
    entries().iter().find(|e| e.id == id)
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
