//! Blocking model discovery per provider, with a per-provider JSON cache.
//!
//! Modes are named by mechanism (see [`DiscoveryMode`]); header names, URLs and id prefixes
//! come from the catalog entry.

use core::fmt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use reqwest::blocking::{Client, RequestBuilder};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::model::catalog::{CatalogEntry, DiscoveryMode};
use crate::secrets::{SecretString, redact_secrets};

/// Whole-request timeout for discovery calls.
pub const DISCOVERY_TIMEOUT: Duration = Duration::from_secs(10);

const MAX_PAGES: usize = 10;

/// One discovered model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelInfo {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_length: Option<u64>,
    /// USD per million input tokens, when the provider lists prices (when listed).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub price_in_per_mtok: Option<f64>,
    /// USD per million output tokens, when the provider lists prices (when listed).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub price_out_per_mtok: Option<f64>,
}

impl ModelInfo {
    fn bare(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            display_name: None,
            context_length: None,
            price_in_per_mtok: None,
            price_out_per_mtok: None,
        }
    }
}

/// A model list plus when it was fetched.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelList {
    pub models: Vec<ModelInfo>,
    /// Unix epoch milliseconds of the successful fetch that produced `models`.
    pub fetched_at_ms: u64,
    /// `true` when this came from the cache because a refresh failed.
    #[serde(default)]
    pub stale: bool,
}

/// Discovery failure (never contains the API key).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiscoveryError {
    /// The entry has `discovery = "none"`.
    Unsupported,
    /// A key is required but none was supplied.
    MissingKey,
    /// The entry has no base URL (e.g. `custom` without an override).
    NoBaseUrl,
    /// Transport failure (connect, timeout, ...).
    Network(String),
    /// Non-success HTTP status.
    Http { status: u16, detail: String },
    /// Body was not the expected JSON shape.
    Parse(String),
}

impl fmt::Display for DiscoveryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported => f.write_str("provider does not support model discovery"),
            Self::MissingKey => f.write_str("an API key is required to list models"),
            Self::NoBaseUrl => f.write_str("no base URL configured"),
            Self::Network(d) => write!(f, "network error: {d}"),
            Self::Http { status, detail } => write!(f, "HTTP {status}: {detail}"),
            Self::Parse(d) => write!(f, "unexpected model list: {d}"),
        }
    }
}

impl std::error::Error for DiscoveryError {}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

fn scrub(text: &str, key: Option<&SecretString>) -> String {
    let secrets: Vec<&SecretString> = key.into_iter().collect();
    redact_secrets(text, &secrets)
}

fn send(
    req: RequestBuilder,
    key: Option<&SecretString>,
) -> Result<(u16, String), DiscoveryError> {
    let resp = req.send().map_err(|e| {
        // `without_url` keeps a query-string key out of the message.
        DiscoveryError::Network(scrub(&e.without_url().to_string(), key))
    })?;
    let status = resp.status().as_u16();
    let body = resp
        .text()
        .map_err(|e| DiscoveryError::Network(scrub(&e.without_url().to_string(), key)))?;
    if !(200..300).contains(&status) {
        let mut detail = scrub(&body, key);
        detail.truncate(detail.char_indices().nth(300).map_or(detail.len(), |(i, _)| i));
        return Err(DiscoveryError::Http { status, detail });
    }
    Ok((status, body))
}

fn json_f64(v: Option<&Value>) -> Option<f64> {
    match v? {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse::<f64>().ok(),
        _ => None,
    }
}

fn parse_openai_models(body: &str) -> Result<Vec<ModelInfo>, DiscoveryError> {
    let root: Value =
        serde_json::from_str(body).map_err(|e| DiscoveryError::Parse(e.to_string()))?;
    let data = root
        .get("data")
        .and_then(Value::as_array)
        .or_else(|| root.as_array())
        .ok_or_else(|| DiscoveryError::Parse("missing 'data' array".to_owned()))?;
    let mut out = Vec::with_capacity(data.len());
    for item in data {
        let Some(id) = item.get("id").and_then(Value::as_str) else {
            continue;
        };
        let mut info = ModelInfo::bare(id);
        info.display_name = item
            .get("name")
            .and_then(Value::as_str)
            .filter(|n| *n != id)
            .map(str::to_owned);
        info.context_length = item.get("context_length").and_then(Value::as_u64);
        if let Some(p) = item.get("pricing") {
            // Listed prices are USD per token (strings); store per million tokens.
            info.price_in_per_mtok = json_f64(p.get("prompt")).map(|v| v * 1_000_000.0);
            info.price_out_per_mtok = json_f64(p.get("completion")).map(|v| v * 1_000_000.0);
        }
        out.push(info);
    }
    Ok(out)
}

fn base_for(entry: &CatalogEntry, base_override: Option<&str>) -> Result<String, DiscoveryError> {
    let base = entry.effective_base_url(base_override);
    if base.is_empty() {
        return Err(DiscoveryError::NoBaseUrl);
    }
    Ok(base)
}

fn require_key<'a>(
    entry: &CatalogEntry,
    key: Option<&'a SecretString>,
) -> Result<Option<&'a SecretString>, DiscoveryError> {
    match key {
        Some(k) if !k.is_empty() => Ok(Some(k)),
        _ if entry.keyless => Ok(None),
        _ => Err(DiscoveryError::MissingKey),
    }
}

/// Lists models for `entry` over the network (blocking, 10 s timeout).
///
/// `base_url_override` replaces the catalog base URL (as in `[model.provider_options]`).
pub fn list_models(
    entry: &CatalogEntry,
    key: Option<&SecretString>,
    base_url_override: Option<&str>,
) -> Result<Vec<ModelInfo>, DiscoveryError> {
    let client = Client::builder()
        .timeout(DISCOVERY_TIMEOUT)
        .build()
        .map_err(|e| DiscoveryError::Network(e.to_string()))?;
    match entry.discovery {
        DiscoveryMode::None => Err(DiscoveryError::Unsupported),
        DiscoveryMode::ModelsList => {
            let base = base_for(entry, base_url_override)?;
            // Keyless local servers and public lists (when listed) work without a key.
            let key = key.filter(|k| !k.is_empty());
            let mut req = client.get(format!("{base}/models"));
            if let Some(k) = key {
                req = req.bearer_auth(k.expose());
            }
            let (_, body) = send(req, key)?;
            parse_openai_models(&body)
        }
        DiscoveryMode::ModelsListHeaderKey => {
            let base = base_for(entry, base_url_override)?;
            let key = require_key(entry, key)?;
            let mut out: Vec<ModelInfo> = Vec::new();
            let mut after: Option<String> = None;
            for _ in 0..MAX_PAGES {
                let mut url = format!("{base}/models?limit=1000");
                if let Some(a) = &after {
                    url.push_str("&after_id=");
                    url.push_str(a);
                }
                let mut req = client.get(url);
                for (hk, hv) in &entry.extra_headers {
                    req = req.header(hk.as_str(), hv.as_str());
                }
                if let (Some(k), Some(h)) = (key, entry.key_header.as_deref()) {
                    req = req.header(h, k.expose());
                }
                let (_, body) = send(req, key)?;
                let root: Value = serde_json::from_str(&body)
                    .map_err(|e| DiscoveryError::Parse(e.to_string()))?;
                let data = root
                    .get("data")
                    .and_then(Value::as_array)
                    .ok_or_else(|| DiscoveryError::Parse("missing 'data' array".to_owned()))?;
                for item in data {
                    let Some(id) = item.get("id").and_then(Value::as_str) else {
                        continue;
                    };
                    let mut info = ModelInfo::bare(id);
                    info.display_name = item
                        .get("display_name")
                        .and_then(Value::as_str)
                        .map(str::to_owned);
                    info.context_length = item.get("max_input_tokens").and_then(Value::as_u64);
                    out.push(info);
                }
                let more = root.get("has_more").and_then(Value::as_bool).unwrap_or(false);
                after = root.get("last_id").and_then(Value::as_str).map(str::to_owned);
                if !more || after.is_none() {
                    break;
                }
            }
            Ok(out)
        }
        DiscoveryMode::NativeModelsListQueryKey => {
            let key = require_key(entry, key)?;
            // The catalog base is the compat path; the native list lives one level up.
            let override_base = entry.effective_base_url(base_url_override);
            let base = if base_url_override.is_some_and(|b| !b.trim().is_empty()) {
                override_base
                    .strip_suffix("/openai")
                    .unwrap_or(&override_base)
                    .to_owned()
            } else {
                entry
                    .discovery_url
                    .as_deref()
                    .map_or(override_base, |u| u.trim_end_matches('/').to_owned())
            };
            if base.is_empty() {
                return Err(DiscoveryError::NoBaseUrl);
            }
            let mut out: Vec<ModelInfo> = Vec::new();
            let mut token: Option<String> = None;
            for _ in 0..MAX_PAGES {
                let mut url = reqwest::Url::parse(&format!("{base}/models"))
                    .map_err(|e| DiscoveryError::Parse(format!("invalid base URL: {e}")))?;
                {
                    let mut q = url.query_pairs_mut();
                    q.append_pair("pageSize", "1000");
                    if let Some(k) = key {
                        q.append_pair("key", k.expose());
                    }
                    if let Some(t) = &token {
                        q.append_pair("pageToken", t);
                    }
                }
                let req = client.get(url);
                let (_, body) = send(req, key)?;
                let root: Value = serde_json::from_str(&body)
                    .map_err(|e| DiscoveryError::Parse(e.to_string()))?;
                let data = root
                    .get("models")
                    .and_then(Value::as_array)
                    .ok_or_else(|| DiscoveryError::Parse("missing 'models' array".to_owned()))?;
                for item in data {
                    let Some(name) = item.get("name").and_then(Value::as_str) else {
                        continue;
                    };
                    // Keep chat-capable models only when the method list is present.
                    if let Some(methods) =
                        item.get("supportedGenerationMethods").and_then(Value::as_array)
                        && !methods.iter().any(|m| m.as_str() == Some("generateContent"))
                    {
                        continue;
                    }
                    let id = entry
                        .model_id_prefix
                        .as_deref()
                        .and_then(|p| name.strip_prefix(p))
                        .unwrap_or(name);
                    let mut info = ModelInfo::bare(id);
                    info.display_name = item
                        .get("displayName")
                        .and_then(Value::as_str)
                        .map(str::to_owned);
                    info.context_length = item.get("inputTokenLimit").and_then(Value::as_u64);
                    out.push(info);
                }
                token = root
                    .get("nextPageToken")
                    .and_then(Value::as_str)
                    .filter(|t| !t.is_empty())
                    .map(str::to_owned);
                if token.is_none() {
                    break;
                }
            }
            Ok(out)
        }
    }
}

fn cache_path(dir: &Path, provider_id: &str) -> PathBuf {
    let safe: String = provider_id
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect();
    dir.join(format!("{safe}.json"))
}

/// Reads the cached list for `provider_id` from `cache_dir`, if any (`stale` is always `false`).
#[must_use]
pub fn read_cache(cache_dir: &Path, provider_id: &str) -> Option<ModelList> {
    let raw = std::fs::read_to_string(cache_path(cache_dir, provider_id)).ok()?;
    serde_json::from_str(&raw).ok()
}

fn write_cache(cache_dir: &Path, provider_id: &str, list: &ModelList) {
    if std::fs::create_dir_all(cache_dir).is_err() {
        return;
    }
    if let Ok(raw) = serde_json::to_string(list) {
        let _ = std::fs::write(cache_path(cache_dir, provider_id), raw);
    }
}

/// Fetches the model list and refreshes the cache; on failure returns the cached list
/// (marked `stale`) together with the error. With no cache the list is empty.
pub fn list_models_cached(
    entry: &CatalogEntry,
    key: Option<&SecretString>,
    base_url_override: Option<&str>,
    cache_dir: &Path,
) -> (ModelList, Option<DiscoveryError>) {
    match list_models(entry, key, base_url_override) {
        Ok(models) => {
            let list = ModelList {
                models,
                fetched_at_ms: now_ms(),
                stale: false,
            };
            write_cache(cache_dir, &entry.id, &list);
            (list, None)
        }
        Err(err) => {
            let mut list = read_cache(cache_dir, &entry.id).unwrap_or(ModelList {
                models: Vec::new(),
                fetched_at_ms: 0,
                stale: true,
            });
            list.stale = true;
            (list, Some(err))
        }
    }
}
