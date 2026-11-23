//! Secret and endpoint loading with strict isolation from scenario/soul files.
//!
//! Provider keys and endpoints are loaded exclusively from environment variables or
//! a dedicated secrets file (`ARCHITECTURE.md`, "Configuration and secrets"). Any key
//! or credential discovered inside a scenario or soul file fails loudly with a
//! diagnostic explaining where secrets belong.

use core::fmt;
use std::collections::BTreeMap;
use std::path::Path;

use serde_json::Value;

use crate::model::catalog::CatalogEntry;

/// Collects `providers.<id>.api_key` entries from a flattened secrets map.
fn provider_keys_from_flat_map(flat: &BTreeMap<String, String>) -> BTreeMap<String, SecretString> {
    flat.iter()
        .filter_map(|(k, v)| {
            let id = k.strip_prefix("providers.")?.strip_suffix(".api_key")?;
            let v = v.trim();
            (!id.is_empty() && !v.is_empty()).then(|| (id.to_owned(), SecretString::new(v)))
        })
        .collect()
}

/// Redacted secret string wrapper whose `Debug` and `Display` never print secret material.
#[derive(Clone, PartialEq, Eq)]
pub struct SecretString(String);

impl SecretString {
    /// Wraps a secret string.
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Exposes the underlying secret solely for constructing outbound auth headers.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// Returns `true` if the secret is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Debug for SecretString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[REDACTED]")
    }
}

impl fmt::Display for SecretString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[REDACTED]")
    }
}

/// Non-secret configuration source kinds where provider secrets are forbidden.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigSourceKind {
    Scenario,
    Soul,
    Prior,
}

impl ConfigSourceKind {
    /// Human-readable label for diagnostics.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Scenario => "scenario",
            Self::Soul => "soul",
            Self::Prior => "prior",
        }
    }
}

impl fmt::Display for ConfigSourceKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Typed errors returned when loading or validating provider secrets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecretError {
    MissingRequired {
        name: String,
        hint: String,
    },
    Io {
        path: String,
        detail: String,
    },
    Parse {
        detail: String,
    },
    SecretInConfigFile {
        source_kind: ConfigSourceKind,
        field: String,
    },
}

impl fmt::Display for SecretError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingRequired { name, hint } => {
                write!(f, "missing required provider secret '{name}': {hint}")
            }
            Self::Io { path, detail } => {
                write!(f, "failed to read secrets file '{path}': {detail}")
            }
            Self::Parse { detail } => {
                write!(f, "invalid secrets file format: {detail}")
            }
            Self::SecretInConfigFile { source_kind, field } => {
                write!(
                    f,
                    "provider secret found in {source_kind} file at '{field}'; \
                     secrets belong in environment variables or a separate secrets file, \
                     never in scenario or soul files"
                )
            }
        }
    }
}

impl std::error::Error for SecretError {}

/// Canonical environment variable names for model and embedding providers.
pub const ENV_MODEL_API_KEY: &str = "MONOAGENS_MODEL_API_KEY";
pub const ENV_MODEL_ENDPOINT: &str = "MONOAGENS_MODEL_ENDPOINT";
pub const ENV_EMBED_API_KEY: &str = "MONOAGENS_EMBED_API_KEY";
pub const ENV_EMBED_ENDPOINT: &str = "MONOAGENS_EMBED_ENDPOINT";
pub const ENV_OPENAI_API_KEY: &str = "OPENAI_API_KEY";
pub const ENV_OPENAI_BASE_URL: &str = "OPENAI_BASE_URL";

/// Provider keys and endpoints loaded from environment variables or a dedicated secrets file.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct ProviderSecrets {
    pub model_api_key: Option<SecretString>,
    pub model_endpoint: Option<String>,
    pub embed_api_key: Option<SecretString>,
    pub embed_endpoint: Option<String>,
    /// Per-provider keys from `secrets.toml` `[providers.<id>] api_key`, keyed by catalog id.
    pub provider_keys: BTreeMap<String, SecretString>,
}

impl fmt::Debug for ProviderSecrets {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProviderSecrets")
            .field(
                "model_api_key",
                &self.model_api_key.as_ref().map(|_| "[REDACTED]"),
            )
            .field("model_endpoint", &self.model_endpoint)
            .field(
                "embed_api_key",
                &self.embed_api_key.as_ref().map(|_| "[REDACTED]"),
            )
            .field("embed_endpoint", &self.embed_endpoint)
            .field(
                "provider_keys",
                &self.provider_keys.keys().collect::<Vec<_>>(),
            )
            .finish()
    }
}

impl ProviderSecrets {
    /// Loads secrets and endpoints from process environment variables.
    #[must_use]
    pub fn from_env() -> Self {
        Self::from_lookup(|key| std::env::var(key).ok())
    }

    /// Loads secrets and endpoints using an injected environment lookup closure (for deterministic tests).
    #[must_use]
    pub fn from_lookup<F>(lookup: F) -> Self
    where
        F: Fn(&str) -> Option<String>,
    {
        let non_empty = |name: &str| {
            lookup(name)
                .map(|v| v.trim().to_owned())
                .filter(|v| !v.is_empty())
        };

        let model_api_key = non_empty(ENV_MODEL_API_KEY)
            .or_else(|| non_empty(ENV_OPENAI_API_KEY))
            .map(SecretString::new);
        let model_endpoint =
            non_empty(ENV_MODEL_ENDPOINT).or_else(|| non_empty(ENV_OPENAI_BASE_URL));
        let embed_api_key = non_empty(ENV_EMBED_API_KEY)
            .or_else(|| non_empty(ENV_MODEL_API_KEY))
            .or_else(|| non_empty(ENV_OPENAI_API_KEY))
            .map(SecretString::new);
        let embed_endpoint = non_empty(ENV_EMBED_ENDPOINT)
            .or_else(|| non_empty(ENV_MODEL_ENDPOINT))
            .or_else(|| non_empty(ENV_OPENAI_BASE_URL));

        Self {
            model_api_key,
            model_endpoint,
            embed_api_key,
            embed_endpoint,
            provider_keys: BTreeMap::new(),
        }
    }

    /// Resolves provider secrets and endpoints from a flat key-value map using canonical key precedence.
    #[must_use]
    pub fn from_flat_map(flat: &BTreeMap<String, String>) -> Self {
        let pick = |keys: &[&str]| -> Option<String> {
            keys.iter().find_map(|k| {
                flat.get(*k)
                    .map(|v| v.trim().to_owned())
                    .filter(|v| !v.is_empty())
            })
        };

        let model_api_key = pick(&[
            "model.api_key",
            "model_api_key",
            ENV_MODEL_API_KEY,
            "openai.api_key",
            "openai_api_key",
            ENV_OPENAI_API_KEY,
            "api_key",
        ])
        .map(SecretString::new);
        let model_endpoint = pick(&[
            "model.endpoint",
            "model.endpoint_url",
            "model_endpoint",
            ENV_MODEL_ENDPOINT,
            "openai.base_url",
            "openai_base_url",
            ENV_OPENAI_BASE_URL,
            "endpoint",
        ]);
        let embed_api_key = pick(&[
            "embed.api_key",
            "embed_api_key",
            ENV_EMBED_API_KEY,
            "model.api_key",
            "model_api_key",
            ENV_MODEL_API_KEY,
            "openai.api_key",
            "openai_api_key",
            ENV_OPENAI_API_KEY,
            "api_key",
        ])
        .map(SecretString::new);
        let embed_endpoint = pick(&[
            "embed.endpoint",
            "embed.endpoint_url",
            "embed_endpoint",
            ENV_EMBED_ENDPOINT,
            "model.endpoint",
            "model.endpoint_url",
            "model_endpoint",
            ENV_MODEL_ENDPOINT,
            "openai.base_url",
            "openai_base_url",
            ENV_OPENAI_BASE_URL,
            "endpoint",
        ]);

        Self {
            model_api_key,
            model_endpoint,
            embed_api_key,
            embed_endpoint,
            provider_keys: provider_keys_from_flat_map(flat),
        }
    }

    /// Resolves the API key for catalog provider `entry`.
    ///
    /// Lookup order: `[providers.<id>] api_key` -> the entry's env names (via `env`) -> legacy
    /// `model_api_key`. Keyless entries still honour an explicit key but never fall back to the
    /// legacy one.
    #[must_use]
    pub fn api_key_for<F>(&self, entry: &CatalogEntry, env: F) -> Option<SecretString>
    where
        F: Fn(&str) -> Option<String>,
    {
        if let Some(k) = self.provider_keys.get(&entry.id).filter(|k| !k.is_empty()) {
            return Some(k.clone());
        }
        for name in &entry.api_key_env {
            if let Some(v) = env(name).map(|v| v.trim().to_owned()).filter(|v| !v.is_empty()) {
                return Some(SecretString::new(v));
            }
        }
        if entry.keyless {
            return None;
        }
        self.model_api_key.clone()
    }

    /// Loads secrets and endpoints from a separate dedicated secrets file (JSON object or `KEY=VALUE` lines).
    pub fn from_secrets_file(path: &Path) -> Result<Self, SecretError> {
        let raw = std::fs::read_to_string(path).map_err(|err| SecretError::Io {
            path: path.display().to_string(),
            detail: err.to_string(),
        })?;
        Self::parse_secrets_str(&raw)
    }

    /// Parses a dedicated secrets file body (JSON object or `KEY=VALUE` lines).
    pub fn parse_secrets_str(raw: &str) -> Result<Self, SecretError> {
        let trimmed = raw.trim();
        let mut map: BTreeMap<String, String> = BTreeMap::new();

        if trimmed.starts_with('{') {
            let val: Value = serde_json::from_str(trimmed).map_err(|err| SecretError::Parse {
                detail: format!("invalid JSON secrets file: {err}"),
            })?;
            let obj = val.as_object().ok_or_else(|| SecretError::Parse {
                detail: "secrets JSON root must be an object".to_owned(),
            })?;
            for (k, v) in obj {
                if let Some(s) = v.as_str() {
                    let s = s.trim();
                    if !s.is_empty() {
                        map.insert(k.to_owned(), s.to_owned());
                    }
                } else if !v.is_null() {
                    return Err(SecretError::Parse {
                        detail: format!("secret entry '{k}' must be a string or null"),
                    });
                }
            }
        } else {
            for (line_no, line) in raw.lines().enumerate() {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }
                let (k, v) = line.split_once('=').ok_or_else(|| SecretError::Parse {
                    detail: format!("line {} missing '=' in KEY=VALUE format", line_no + 1),
                })?;
                let key = k.trim();
                let mut val = v.trim();
                if ((val.starts_with('"') && val.ends_with('"'))
                    || (val.starts_with('\'') && val.ends_with('\'')))
                    && val.len() >= 2
                {
                    val = &val[1..val.len() - 1];
                }
                if !key.is_empty() && !val.is_empty() {
                    map.insert(key.to_owned(), val.to_owned());
                }
            }
        }

        Ok(Self::from_flat_map(&map))
    }

    /// Returns all contained secrets for output scrubbing.
    #[must_use]
    pub fn all_secrets(&self) -> Vec<&SecretString> {
        let mut out = Vec::new();
        if let Some(k) = &self.model_api_key {
            out.push(k);
        }
        if let Some(k) = &self.embed_api_key {
            out.push(k);
        }
        out.extend(self.provider_keys.values());
        out
    }

    /// Scrubs all loaded secret values and common token shapes from `text`.
    #[must_use]
    pub fn redact_text(&self, text: &str) -> String {
        redact_secrets(text, &self.all_secrets())
    }
}

/// Returns `true` if a configuration field name looks like a provider credential/secret key.
fn is_forbidden_secret_key_name(key: &str) -> bool {
    let norm = key.trim().to_ascii_lowercase();
    matches!(
        norm.as_str(),
        "api_key"
            | "apikey"
            | "provider_key"
            | "provider_api_key"
            | "model_api_key"
            | "embed_api_key"
            | "openai_api_key"
            | "secret"
            | "secret_key"
            | "client_secret"
            | "api_secret"
            | "auth_token"
            | "bearer_token"
            | "access_token"
            | "private_key"
            | "authorization"
            | "password"
            | "monoagens_model_api_key"
            | "monoagens_embed_api_key"
    )
}

fn looks_like_embedded_secret_value(value: &str) -> bool {
    let trimmed = value.trim();
    if trimmed.starts_with("Bearer ") && trimmed.len() > "Bearer ".len() + 6 {
        return true;
    }
    if (trimmed.starts_with("sk-") || trimmed.starts_with("sk-proj-")) && trimmed.len() >= 16 {
        return true;
    }
    false
}

/// Validates that a parsed scenario, soul, or prior JSON value contains no provider secrets.
pub fn validate_no_secrets_in_json(
    source_kind: ConfigSourceKind,
    value: &Value,
) -> Result<(), SecretError> {
    validate_json_recursive(source_kind, value, "$")
}

fn validate_json_recursive(
    source_kind: ConfigSourceKind,
    value: &Value,
    path: &str,
) -> Result<(), SecretError> {
    match value {
        Value::Object(map) => {
            for (k, v) in map {
                let child_path = format!("{path}.{k}");
                if is_forbidden_secret_key_name(k) {
                    let has_value = match v {
                        Value::Null => false,
                        Value::String(s) => !s.trim().is_empty(),
                        _ => true,
                    };
                    if has_value {
                        return Err(SecretError::SecretInConfigFile {
                            source_kind,
                            field: child_path,
                        });
                    }
                }
                validate_json_recursive(source_kind, v, &child_path)?;
            }
            Ok(())
        }
        Value::Array(items) => {
            for (idx, item) in items.iter().enumerate() {
                let child_path = format!("{path}[{idx}]");
                validate_json_recursive(source_kind, item, &child_path)?;
            }
            Ok(())
        }
        Value::String(s) => {
            if looks_like_embedded_secret_value(s) {
                return Err(SecretError::SecretInConfigFile {
                    source_kind,
                    field: path.to_owned(),
                });
            }
            Ok(())
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => Ok(()),
    }
}

/// Validates that raw scenario or soul text (JSON or key/value text) contains no provider secrets.
pub fn validate_no_secrets_in_str(
    source_kind: ConfigSourceKind,
    raw: &str,
) -> Result<(), SecretError> {
    if let Ok(val) = serde_json::from_str::<Value>(raw) {
        return validate_no_secrets_in_json(source_kind, &val);
    }

    for (idx, line) in raw.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if let Some((k, v)) = trimmed.split_once('=').or_else(|| trimmed.split_once(':')) {
            let key = k.trim().trim_matches('"').trim_matches('\'');
            let val = v.trim().trim_matches('"').trim_matches('\'');
            if is_forbidden_secret_key_name(key) && !val.is_empty() && val != "null" {
                return Err(SecretError::SecretInConfigFile {
                    source_kind,
                    field: format!("line {}:{}", idx + 1, key),
                });
            }
            if looks_like_embedded_secret_value(val) {
                return Err(SecretError::SecretInConfigFile {
                    source_kind,
                    field: format!("line {}:{}", idx + 1, key),
                });
            }
        } else if looks_like_embedded_secret_value(trimmed) {
            return Err(SecretError::SecretInConfigFile {
                source_kind,
                field: format!("line {}", idx + 1),
            });
        }
    }
    Ok(())
}

/// Scrubs any occurrences of explicit `secrets` and `Bearer` / `sk-` token patterns from `text`.
#[must_use]
pub fn redact_secrets(text: &str, secrets: &[&SecretString]) -> String {
    let mut out = text.to_owned();
    for secret in secrets {
        let raw = secret.expose();
        if !raw.is_empty() {
            out = out.replace(raw, "[REDACTED]");
        }
    }
    scrub_bearer_tokens(&out)
}

fn scrub_bearer_tokens(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(pos) = rest.find("Bearer ") {
        out.push_str(&rest[..pos]);
        out.push_str("Bearer [REDACTED]");
        let after = &rest[pos + "Bearer ".len()..];
        let token_len = after
            .find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | ',' | '}' | ']'))
            .unwrap_or(after.len());
        rest = &after[token_len..];
    }
    out.push_str(rest);

    // Also scrub inline `sk-...` tokens of length >= 12.
    let mut final_out = String::with_capacity(out.len());
    let mut rem = out.as_str();
    while let Some(pos) = rem.find("sk-") {
        final_out.push_str(&rem[..pos]);
        let after = &rem[pos..];
        let tok_len = after
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_'))
            .unwrap_or(after.len());
        if tok_len >= 12 {
            final_out.push_str("[REDACTED]");
        } else {
            final_out.push_str(&after[..tok_len]);
        }
        rem = &after[tok_len..];
    }
    final_out.push_str(rem);
    final_out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    #[test]
    fn provider_key_lookup_order_secrets_then_env_then_legacy() {
        let entry = crate::model::catalog::find("groq").expect("entry");
        let env = |name: &str| (name == "GROQ_API_KEY").then(|| "env-key".to_owned());
        let no_env = |_: &str| None;

        let all = ProviderSecrets::from_flat_map(&flat(&[
            ("providers.groq.api_key", "file-key"),
            ("model_api_key", "legacy-key"),
        ]));
        assert_eq!(all.api_key_for(entry, env).unwrap().expose(), "file-key");

        let legacy_only = ProviderSecrets::from_flat_map(&flat(&[("model_api_key", "legacy-key")]));
        assert_eq!(legacy_only.api_key_for(entry, env).unwrap().expose(), "env-key");
        assert_eq!(
            legacy_only.api_key_for(entry, no_env).unwrap().expose(),
            "legacy-key"
        );
        assert!(ProviderSecrets::default().api_key_for(entry, no_env).is_none());

        let local = crate::model::catalog::find("lm-studio").expect("lm-studio");
        assert!(legacy_only.api_key_for(local, no_env).is_none());
    }

    #[test]
    fn provider_keys_never_appear_in_debug_or_redacted_text() {
        let secrets = ProviderSecrets::from_flat_map(&flat(&[(
            "providers.openai.api_key",
            "sk-super-secret-value-123",
        )]));
        let dbg = format!("{secrets:?}");
        assert!(!dbg.contains("sk-super-secret-value-123"), "{dbg}");
        assert!(dbg.contains("openai"));
        let scrubbed = secrets.redact_text("error with sk-super-secret-value-123 inside");
        assert!(!scrubbed.contains("sk-super-secret-value-123"), "{scrubbed}");
    }

    #[test]
    fn from_flat_map_respects_precedence_and_skips_blank_values() {
        let mut map = BTreeMap::new();
        map.insert("api_key".to_owned(), "sk-bare".to_owned());
        map.insert("endpoint".to_owned(), "https://bare.invalid/v1".to_owned());
        let bare = ProviderSecrets::from_flat_map(&map);
        assert_eq!(
            bare.model_api_key.as_ref().map(SecretString::expose),
            Some("sk-bare")
        );
        assert_eq!(bare.model_endpoint.as_deref(), Some("https://bare.invalid/v1"));
        assert_eq!(
            bare.embed_api_key.as_ref().map(SecretString::expose),
            Some("sk-bare")
        );
        assert_eq!(bare.embed_endpoint.as_deref(), Some("https://bare.invalid/v1"));

        // Higher-precedence keys override lower ones, while blank values are ignored.
        map.insert(ENV_OPENAI_API_KEY.to_owned(), "sk-env-openai".to_owned());
        map.insert("openai.api_key".to_owned(), "  sk-dot-openai  ".to_owned());
        map.insert("model.api_key".to_owned(), "   ".to_owned());
        map.insert("openai.base_url".to_owned(), "https://openai.invalid/v1".to_owned());
        map.insert("model.endpoint_url".to_owned(), "  https://model.invalid/v1 ".to_owned());
        map.insert("model.endpoint".to_owned(), "\t\n".to_owned());
        let model_override = ProviderSecrets::from_flat_map(&map);
        assert_eq!(
            model_override.model_api_key.as_ref().map(SecretString::expose),
            Some("sk-dot-openai")
        );
        assert_eq!(
            model_override.model_endpoint.as_deref(),
            Some("https://model.invalid/v1")
        );
        // Embed falls back to model.endpoint_url and openai.api_key when embed.* are absent.
        assert_eq!(
            model_override.embed_api_key.as_ref().map(SecretString::expose),
            Some("sk-dot-openai")
        );
        assert_eq!(
            model_override.embed_endpoint.as_deref(),
            Some("https://model.invalid/v1")
        );

        // Explicit embed keys win for embed_* without disturbing model_*.
        map.insert("model.api_key".to_owned(), "sk-model-top".to_owned());
        map.insert("embed.api_key".to_owned(), "sk-embed-top".to_owned());
        map.insert("embed.endpoint".to_owned(), "https://embed.invalid/v1".to_owned());
        let full = ProviderSecrets::from_flat_map(&map);
        assert_eq!(
            full.model_api_key.as_ref().map(SecretString::expose),
            Some("sk-model-top")
        );
        assert_eq!(
            full.embed_api_key.as_ref().map(SecretString::expose),
            Some("sk-embed-top")
        );
        assert_eq!(
            full.embed_endpoint.as_deref(),
            Some("https://embed.invalid/v1")
        );
    }

    #[test]
    fn parse_secrets_str_delegates_to_from_flat_map_for_json_and_kv() {
        let json_body = r#"{
            "model.api_key": " sk-json-model ",
            "openai.base_url": "https://json-model.invalid/v1",
            "embed_api_key": "sk-json-embed",
            "empty_key": "   "
        }"#;
        let parsed_json = ProviderSecrets::parse_secrets_str(json_body).expect("parse json");
        assert_eq!(
            parsed_json.model_api_key.as_ref().map(SecretString::expose),
            Some("sk-json-model")
        );
        assert_eq!(
            parsed_json.model_endpoint.as_deref(),
            Some("https://json-model.invalid/v1")
        );
        assert_eq!(
            parsed_json.embed_api_key.as_ref().map(SecretString::expose),
            Some("sk-json-embed")
        );
        assert_eq!(
            parsed_json.embed_endpoint.as_deref(),
            Some("https://json-model.invalid/v1")
        );

        let kv_body = "# comment\nmodel_api_key='sk-kv-model'\nembed.endpoint=\"https://kv-embed.invalid/v1\"\n";
        let parsed_kv = ProviderSecrets::parse_secrets_str(kv_body).expect("parse kv");
        assert_eq!(
            parsed_kv.model_api_key.as_ref().map(SecretString::expose),
            Some("sk-kv-model")
        );
        assert_eq!(
            parsed_kv.embed_endpoint.as_deref(),
            Some("https://kv-embed.invalid/v1")
        );
    }
}
