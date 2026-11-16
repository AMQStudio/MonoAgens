//! Secret loading (`secrets.toml` + environment variables) and inline-secret guards.

use core::fmt;
use std::collections::BTreeMap;
use std::path::Path;

use monoagens_reason::{
    ConfigSourceKind, ENV_EMBED_API_KEY, ENV_EMBED_ENDPOINT, ENV_MODEL_API_KEY, ENV_MODEL_ENDPOINT,
    ENV_OPENAI_API_KEY, ENV_OPENAI_BASE_URL, ProviderSecrets, SecretError, SecretString,
    validate_no_secrets_in_str,
};
use toml_edit::{DocumentMut, Item, Table, Value};

/// Errors produced when loading `secrets.toml` or detecting inline secrets in config files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecretsLoadError {
    Io {
        path: String,
        detail: String,
    },
    Parse {
        path: String,
        detail: String,
    },
    InlineSecretInConfig {
        config_path: String,
        field: String,
        secrets_hint: String,
    },
    CoreSecret(SecretError),
}

impl fmt::Display for SecretsLoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, detail } => {
                write!(f, "failed to read secrets file '{path}': {detail}")
            }
            Self::Parse { path, detail } => {
                write!(f, "invalid secrets file '{path}': {detail}")
            }
            Self::InlineSecretInConfig {
                config_path,
                field,
                secrets_hint,
            } => {
                write!(
                    f,
                    "inline secret found in config file '{config_path}' at '{field}'; \
                     secrets belong in {secrets_hint} or environment variables \
                     (such as {ENV_MODEL_API_KEY}, {ENV_OPENAI_API_KEY}), never in monoagens.toml, \
                     soul, or scenario files"
                )
            }
            Self::CoreSecret(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for SecretsLoadError {}

impl From<SecretError> for SecretsLoadError {
    fn from(err: SecretError) -> Self {
        Self::CoreSecret(err)
    }
}

/// Canonical environment variable for attachment AEAD master key material.
pub const ENV_ATTACHMENT_MASTER_KEY: &str = "MONOAGENS_ATTACHMENT_MASTER_KEY";

/// Canonical forbidden key names that must never appear with values inside `monoagens.toml`.
#[must_use]
pub fn is_forbidden_secret_key(key: &str) -> bool {
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
            | "attachment_master_key"
            | "monoagens_attachment_master_key"
            | "master_key"
            | "aead_key"
            | "encryption_key"
    )
}

fn looks_like_secret_literal(val: &str) -> bool {
    let trimmed = val.trim();
    if trimmed.starts_with("Bearer ") && trimmed.len() > "Bearer ".len() + 6 {
        return true;
    }
    if (trimmed.starts_with("sk-") || trimmed.starts_with("sk-proj-")) && trimmed.len() >= 16 {
        return true;
    }
    false
}

/// Recursively verifies that a parsed `monoagens.toml` [`DocumentMut`] contains zero inline secrets.
pub fn validate_no_inline_secrets_in_toml(
    doc: &DocumentMut,
    config_path: &Path,
    secrets_path: &Path,
) -> Result<(), SecretsLoadError> {
    let config_str = config_path.display().to_string();
    let secrets_hint = format!("'{}'", secrets_path.display());
    scan_table(doc.as_table(), "$", &config_str, &secrets_hint)
}

fn scan_table(
    table: &Table,
    prefix: &str,
    config_path: &str,
    secrets_hint: &str,
) -> Result<(), SecretsLoadError> {
    for (key, item) in table.iter() {
        let field = format!("{prefix}.{key}");
        if is_forbidden_secret_key(key) && item_has_non_empty_value(item) {
            return Err(SecretsLoadError::InlineSecretInConfig {
                config_path: config_path.to_owned(),
                field,
                secrets_hint: secrets_hint.to_owned(),
            });
        }
        scan_item(item, &field, config_path, secrets_hint)?;
    }
    Ok(())
}

fn scan_item(
    item: &Item,
    field: &str,
    config_path: &str,
    secrets_hint: &str,
) -> Result<(), SecretsLoadError> {
    match item {
        Item::Table(t) => scan_table(t, field, config_path, secrets_hint),
        Item::ArrayOfTables(arr) => {
            for (idx, t) in arr.iter().enumerate() {
                let child = format!("{field}[{idx}]");
                scan_table(t, &child, config_path, secrets_hint)?;
            }
            Ok(())
        }
        Item::Value(val) => scan_value(val, field, config_path, secrets_hint),
        Item::None => Ok(()),
    }
}

fn scan_value(
    val: &Value,
    field: &str,
    config_path: &str,
    secrets_hint: &str,
) -> Result<(), SecretsLoadError> {
    match val {
        Value::String(s) => {
            if looks_like_secret_literal(s.value()) {
                return Err(SecretsLoadError::InlineSecretInConfig {
                    config_path: config_path.to_owned(),
                    field: field.to_owned(),
                    secrets_hint: secrets_hint.to_owned(),
                });
            }
            Ok(())
        }
        Value::InlineTable(t) => {
            for (key, v) in t.iter() {
                let child = format!("{field}.{key}");
                if is_forbidden_secret_key(key) && value_is_non_empty(v) {
                    return Err(SecretsLoadError::InlineSecretInConfig {
                        config_path: config_path.to_owned(),
                        field: child,
                        secrets_hint: secrets_hint.to_owned(),
                    });
                }
                scan_value(v, &child, config_path, secrets_hint)?;
            }
            Ok(())
        }
        Value::Array(arr) => {
            for (idx, v) in arr.iter().enumerate() {
                let child = format!("{field}[{idx}]");
                scan_value(v, &child, config_path, secrets_hint)?;
            }
            Ok(())
        }
        Value::Integer(_) | Value::Float(_) | Value::Boolean(_) | Value::Datetime(_) => Ok(()),
    }
}

fn item_has_non_empty_value(item: &Item) -> bool {
    match item {
        Item::None => false,
        Item::Value(v) => value_is_non_empty(v),
        Item::Table(t) => !t.is_empty(),
        Item::ArrayOfTables(a) => !a.is_empty(),
    }
}

fn value_is_non_empty(v: &Value) -> bool {
    match v {
        Value::String(s) => !s.value().trim().is_empty(),
        _ => true,
    }
}

/// Validates that a soul or scenario file body contains no inline provider secrets.
pub fn validate_non_secret_file(
    source_kind: ConfigSourceKind,
    raw: &str,
) -> Result<(), SecretsLoadError> {
    validate_no_secrets_in_str(source_kind, raw)?;
    Ok(())
}

/// Loads [`ProviderSecrets`] from `secrets.toml` (if it exists) merged with environment lookup
/// (environment variables take precedence over `secrets.toml`).
pub fn load_provider_secrets<F>(
    secrets_path: Option<&Path>,
    env_lookup: F,
) -> Result<ProviderSecrets, SecretsLoadError>
where
    F: Fn(&str) -> Option<String>,
{
    let file_secrets = match secrets_path {
        Some(path) if path.exists() => load_secrets_toml_file(path)?,
        _ => ProviderSecrets::default(),
    };
    let env_secrets = ProviderSecrets::from_lookup(env_lookup);

    Ok(ProviderSecrets {
        model_api_key: env_secrets.model_api_key.or(file_secrets.model_api_key),
        model_endpoint: env_secrets.model_endpoint.or(file_secrets.model_endpoint),
        embed_api_key: env_secrets.embed_api_key.or(file_secrets.embed_api_key),
        embed_endpoint: env_secrets.embed_endpoint.or(file_secrets.embed_endpoint),
    })
}

/// Parses a `secrets.toml` file (TOML tables/keys, or fallback to `KEY=VALUE`/JSON via `reason`).
pub fn load_secrets_toml_file(path: &Path) -> Result<ProviderSecrets, SecretsLoadError> {
    let raw = std::fs::read_to_string(path).map_err(|err| SecretsLoadError::Io {
        path: path.display().to_string(),
        detail: err.to_string(),
    })?;
    parse_secrets_toml_str(&raw, path)
}

/// Parses `secrets.toml` contents into [`ProviderSecrets`].
pub fn parse_secrets_toml_str(raw: &str, path: &Path) -> Result<ProviderSecrets, SecretsLoadError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(ProviderSecrets::default());
    }
    if trimmed.starts_with('{') {
        return ProviderSecrets::parse_secrets_str(trimmed).map_err(SecretsLoadError::from);
    }

    let doc = raw
        .parse::<DocumentMut>()
        .map_err(|err| SecretsLoadError::Parse {
            path: path.display().to_string(),
            detail: err.to_string(),
        })?;

    let mut flat: BTreeMap<String, String> = BTreeMap::new();
    collect_toml_strings(doc.as_table(), "", &mut flat, path)?;

    let pick =
        |keys: &[&str]| -> Option<String> { keys.iter().find_map(|k| flat.get(*k).cloned()) };

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
        "openai_base_url",
        ENV_OPENAI_BASE_URL,
        "endpoint",
    ]);

    Ok(ProviderSecrets {
        model_api_key,
        model_endpoint,
        embed_api_key,
        embed_endpoint,
    })
}

/// Secret key material for AEAD-at-rest attachment custody (`4.4`), loaded exclusively
/// from `secrets.toml` or environment variables (never from `monoagens.toml` or source code).
#[derive(Clone, Default, PartialEq, Eq)]
pub struct AttachmentSecrets {
    pub master_key: Option<SecretString>,
}

impl fmt::Debug for AttachmentSecrets {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AttachmentSecrets")
            .field(
                "master_key",
                &self.master_key.as_ref().map(|_| "[REDACTED]"),
            )
            .finish()
    }
}

/// Combined provider and attachment custody secrets loaded by the Gateway secrets layer.
#[derive(Clone, Default, Debug, PartialEq, Eq)]
pub struct GatewaySecrets {
    pub providers: ProviderSecrets,
    pub attachments: AttachmentSecrets,
}

/// Loads [`AttachmentSecrets`] from `secrets.toml` (`[attachments].master_key`,
/// `attachment_master_key`, or `MONOAGENS_ATTACHMENT_MASTER_KEY`) and environment variables,
/// with environment variables taking precedence.
pub fn load_attachment_secrets<F>(
    secrets_path: Option<&Path>,
    env_lookup: F,
) -> Result<AttachmentSecrets, SecretsLoadError>
where
    F: Fn(&str) -> Option<String>,
{
    let file_key = match secrets_path {
        Some(path) if path.exists() => {
            let raw = std::fs::read_to_string(path).map_err(|err| SecretsLoadError::Io {
                path: path.display().to_string(),
                detail: err.to_string(),
            })?;
            parse_attachment_master_key_str(&raw, path)?
        }
        _ => None,
    };

    let env_key = env_lookup(ENV_ATTACHMENT_MASTER_KEY)
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty())
        .map(SecretString::new);

    Ok(AttachmentSecrets {
        master_key: env_key.or(file_key),
    })
}

/// Loads both [`ProviderSecrets`] and [`AttachmentSecrets`] from `secrets.toml` and `env_lookup`.
pub fn load_gateway_secrets<F>(
    secrets_path: Option<&Path>,
    env_lookup: F,
) -> Result<GatewaySecrets, SecretsLoadError>
where
    F: Fn(&str) -> Option<String>,
{
    let providers = load_provider_secrets(secrets_path, &env_lookup)?;
    let attachments = load_attachment_secrets(secrets_path, env_lookup)?;
    Ok(GatewaySecrets {
        providers,
        attachments,
    })
}

fn parse_attachment_master_key_str(
    raw: &str,
    path: &Path,
) -> Result<Option<SecretString>, SecretsLoadError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    if trimmed.starts_with('{') {
        let val: serde_json::Value =
            serde_json::from_str(trimmed).map_err(|err| SecretsLoadError::Parse {
                path: path.display().to_string(),
                detail: format!("invalid JSON secrets file: {err}"),
            })?;
        for key in [
            "attachment_master_key",
            ENV_ATTACHMENT_MASTER_KEY,
            "master_key",
        ] {
            if let Some(s) = val.get(key).and_then(serde_json::Value::as_str) {
                let t = s.trim();
                if !t.is_empty() {
                    return Ok(Some(SecretString::new(t.to_owned())));
                }
            }
        }
        return Ok(None);
    }

    let doc = raw
        .parse::<DocumentMut>()
        .map_err(|err| SecretsLoadError::Parse {
            path: path.display().to_string(),
            detail: err.to_string(),
        })?;
    let mut flat: BTreeMap<String, String> = BTreeMap::new();
    collect_toml_strings(doc.as_table(), "", &mut flat, path)?;

    let found = [
        "attachments.master_key",
        "attachments.attachment_master_key",
        "attachment_master_key",
        ENV_ATTACHMENT_MASTER_KEY,
        "master_key",
    ]
    .iter()
    .find_map(|k| flat.get(*k).cloned());

    Ok(found.map(SecretString::new))
}

fn collect_toml_strings(
    table: &Table,
    prefix: &str,
    out: &mut BTreeMap<String, String>,
    path: &Path,
) -> Result<(), SecretsLoadError> {
    for (k, item) in table.iter() {
        let full_key = if prefix.is_empty() {
            k.to_owned()
        } else {
            format!("{prefix}.{k}")
        };
        match item {
            Item::Table(sub) => collect_toml_strings(sub, &full_key, out, path)?,
            Item::Value(Value::String(s)) => {
                let v = s.value().trim();
                if !v.is_empty() {
                    out.insert(full_key, v.to_owned());
                }
            }
            Item::Value(Value::InlineTable(it)) => {
                for (ik, iv) in it.iter() {
                    let sub_key = format!("{full_key}.{ik}");
                    if let Value::String(s) = iv {
                        let v = s.value().trim();
                        if !v.is_empty() {
                            out.insert(sub_key, v.to_owned());
                        }
                    } else {
                        return Err(SecretsLoadError::Parse {
                            path: path.display().to_string(),
                            detail: format!("secret entry '{sub_key}' must be a string"),
                        });
                    }
                }
            }
            Item::None => {}
            _ => {
                return Err(SecretsLoadError::Parse {
                    path: path.display().to_string(),
                    detail: format!("secret entry '{full_key}' must be a string or table"),
                });
            }
        }
    }
    Ok(())
}
