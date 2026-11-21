//! Versioned `monoagens.toml` configuration, XDG/`MONOAGENS_HOME`/`--config` resolution,
//! range validation, and `toml_edit` format-preserving updates after backup.

use core::fmt;
use std::path::{Path, PathBuf};

use monoagens_protocol::{ActorId, BodyId, CURRENT_PROTOCOL_VERSION, ProtocolVersion};
use monoagens_reason::{EMBEDDING_GEMMA_2_DIM, EMBEDDING_GEMMA_2_VER, ProviderSecrets};
use monoagens_core::blake3;
use monoagens_store::{
    ConfigLoadError, LoadedCoreConfig, load_core_config_or_default,
};
use toml_edit::{DocumentMut, Item, Table, TableLike, value};

use crate::cues::{DEFAULT_PROGRESS_NOTE_MS, DEFAULT_WORKING_CUE_MS};
use crate::secrets::{SecretsLoadError, load_provider_secrets, validate_no_inline_secrets_in_toml};

/// Current supported `monoagens.toml` schema version.
pub const CURRENT_CONFIG_VERSION: u32 = 1;

/// Canonical config filename.
pub const CONFIG_FILENAME: &str = "monoagens.toml";

/// Canonical secrets filename stored alongside config (never logged).
pub const SECRETS_FILENAME: &str = "secrets.toml";

/// Default OpenAI-compatible chat completions endpoint used when an API key is supplied without
/// an explicit endpoint override in `secrets.toml` or environment variables.
pub const DEFAULT_OPENAI_ENDPOINT: &str = "https://api.openai.com/v1/chat/completions";

/// Selected model provider backend in `monoagens.toml`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelProviderKind {
    OpenAiCompatible,
    Scripted,
}

impl ModelProviderKind {
    /// Canonical string identifier stored in `monoagens.toml`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::OpenAiCompatible => "openai_compatible",
            Self::Scripted => "scripted",
        }
    }

    fn parse(raw: &str) -> Option<Self> {
        match raw.trim() {
            "openai_compatible" | "openai" => Some(Self::OpenAiCompatible),
            "scripted" => Some(Self::Scripted),
            _ => None,
        }
    }
}

/// Transport kind for an external plugin body declared in `[[bodies]]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BodyTransportConfig {
    Stdio { command: String, args: Vec<String> },
    UnixSocket { path: PathBuf },
}

/// Configured external body plugin entry in `monoagens.toml`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BodyPluginConfig {
    pub id: String,
    pub transport: BodyTransportConfig,
}

/// Runtime bounds and lifecycle cue timings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeConfig {
    pub hook_timeout_ms: u64,
    pub action_timeout_ms: u64,
    pub model_timeout_ms: u64,
    pub working_cue_ms: u64,
    pub progress_note_ms: u64,
    pub body_shutdown_grace_ms: u64,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            hook_timeout_ms: 5_000,
            action_timeout_ms: 30_000,
            model_timeout_ms: 30_000,
            working_cue_ms: DEFAULT_WORKING_CUE_MS,
            progress_note_ms: DEFAULT_PROGRESS_NOTE_MS,
            body_shutdown_grace_ms: 5_000,
        }
    }
}

/// Model provider settings (secrets and endpoints live strictly in `secrets.toml` / env).
#[derive(Debug, Clone, PartialEq)]
pub struct ModelConfig {
    pub provider: ModelProviderKind,
    pub model: String,
    pub daily_token_budget: u64,
    pub max_output_tokens: Option<u32>,
    pub output_limits: monoagens_reason::OutputLimitProfile,
    pub output_limit_param: String,
    pub extra_headers: Vec<(String, String)>,
    pub usage_paths: monoagens_reason::UsageFieldPaths,
    pub temperature: Option<f32>,
}

impl Default for ModelConfig {
    fn default() -> Self {
        Self {
            provider: ModelProviderKind::OpenAiCompatible,
            model: "gpt-4o-mini".to_owned(),
            daily_token_budget: 100_000,
            max_output_tokens: None,
            output_limits: monoagens_reason::OutputLimitProfile::default(),
            output_limit_param: monoagens_reason::DEFAULT_OUTPUT_LIMIT_PARAM.to_owned(),
            extra_headers: Vec::new(),
            usage_paths: monoagens_reason::UsageFieldPaths::default(),
            temperature: None,
        }
    }
}

/// Soul identity, deterministic PRNG seed, and optional prior file path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SoulConfig {
    pub name: String,
    pub seed: u64,
    pub core_config: Option<PathBuf>,
    pub model_path: Option<PathBuf>,
    pub expected_blake3: Option<String>,
    pub online_adaptation: Option<bool>,
}

impl Default for SoulConfig {
    fn default() -> Self {
        Self {
            name: "MonoAgens".to_owned(),
            seed: 1,
            core_config: None,
            model_path: None,
            expected_blake3: None,
            online_adaptation: None,
        }
    }
}

/// Valve mode setting surfaced in `monoagens.toml` (seam for `crates/valve`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValveSectionConfig {
    pub mode: String,
    pub max_chain_wakes: u32,
}

impl Default for ValveSectionConfig {
    fn default() -> Self {
        Self {
            mode: "enforcing".to_owned(),
            max_chain_wakes: 8,
        }
    }
}

/// Platform `(body_id, account_id)` tuple bound to a [`PersonMappingConfig`] in `[[person]]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersonAccountConfig {
    pub body_id: BodyId,
    pub account_id: String,
}

/// Configured `[[person]]` mapping (`C_person`) taking highest precedence over `κ(b, a)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersonMappingConfig {
    pub actor_id: ActorId,
    pub is_owner: bool,
    pub accounts: Vec<PersonAccountConfig>,
}

/// Cross-body identity linking configuration (`4.3`, `R-44`).
///
/// `heuristic_merge_enabled` is invariant-locked to `false` (`T_μ = +∞`) and
/// `owner_escalation_guard` is invariant-locked to `true`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentityConfig {
    pub heuristic_merge_enabled: bool,
    pub link_challenge_ttl_s: u64,
    pub link_max_attempts: u32,
    pub retroactive_merge: bool,
    pub owner_escalation_guard: bool,
}

impl Default for IdentityConfig {
    fn default() -> Self {
        Self {
            heuristic_merge_enabled: false,
            link_challenge_ttl_s: 600,
            link_max_attempts: 3,
            retroactive_merge: false,
            owner_escalation_guard: true,
        }
    }
}

/// Selected pre-queue embedder backend kind in `[embed]` (`4.5`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbedBackendKind {
    None,
    Hash,
    EmbeddingGemma2Http,
    EmbeddingGemma2Stdio,
    EmbeddingGemma2Remote,
    OpenAiCompatible,
}

impl EmbedBackendKind {
    /// Canonical string identifier stored in `monoagens.toml`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Hash => "hash",
            Self::EmbeddingGemma2Http => "embeddinggemma2_http",
            Self::EmbeddingGemma2Stdio => "embeddinggemma2_stdio",
            Self::EmbeddingGemma2Remote => "embeddinggemma2_remote",
            Self::OpenAiCompatible => "openai_compatible",
        }
    }

    fn parse(raw: &str) -> Option<Self> {
        match raw.trim() {
            "none" => Some(Self::None),
            "hash" => Some(Self::Hash),
            "embeddinggemma2_http" | "sidecar_http" => Some(Self::EmbeddingGemma2Http),
            "embeddinggemma2_stdio" | "sidecar_stdio" => Some(Self::EmbeddingGemma2Stdio),
            "embeddinggemma2_remote" | "remote_openai" => Some(Self::EmbeddingGemma2Remote),
            "openai_compatible" | "openai" => Some(Self::OpenAiCompatible),
            _ => None,
        }
    }
}

/// Pre-queue embedding configuration in `[embed]` (`4.5`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbedConfig {
    pub backend: EmbedBackendKind,
    pub model_ver: String,
    pub dim_d: u32,
    pub timeout_ms: u64,
    pub model_path: Option<PathBuf>,
    pub endpoint_url: Option<String>,
    pub program: Option<String>,
    pub args: Vec<String>,
    pub remote_model: Option<String>,
}

impl Default for EmbedConfig {
    fn default() -> Self {
        Self {
            backend: EmbedBackendKind::None,
            model_ver: EMBEDDING_GEMMA_2_VER.to_owned(),
            dim_d: EMBEDDING_GEMMA_2_DIM,
            timeout_ms: 2_000,
            model_path: None,
            endpoint_url: None,
            program: None,
            args: Vec::new(),
            remote_model: None,
        }
    }
}

/// Validated `monoagens.toml` configuration (`version = 1`).
#[derive(Debug, Clone, PartialEq)]
pub struct GatewayConfig {
    pub version: u32,
    pub protocol_version: ProtocolVersion,
    pub data_dir: Option<PathBuf>,
    pub soul: SoulConfig,
    pub runtime: RuntimeConfig,
    pub model: ModelConfig,
    pub embed: EmbedConfig,
    pub valve: ValveSectionConfig,
    pub identity: IdentityConfig,
    pub persons: Vec<PersonMappingConfig>,
    pub bodies: Vec<BodyPluginConfig>,
}

impl Default for GatewayConfig {
    fn default() -> Self {
        Self {
            version: CURRENT_CONFIG_VERSION,
            protocol_version: CURRENT_PROTOCOL_VERSION,
            data_dir: None,
            soul: SoulConfig::default(),
            runtime: RuntimeConfig::default(),
            model: ModelConfig::default(),
            embed: EmbedConfig::default(),
            valve: ValveSectionConfig::default(),
            identity: IdentityConfig::default(),
            persons: Vec::new(),
            bodies: Vec::new(),
        }
    }
}

impl GatewayConfig {
    /// Parses a 64-character lowercase/uppercase hex `BLAKE3` digest into `[u8; 32]`.
    pub fn parse_blake3_hex(raw: &str) -> Result<[u8; 32], ConfigError> {
        let s = raw.trim();
        let bytes = s.as_bytes();
        if bytes.len() != 64 {
            return Err(ConfigError::InvalidValue {
                key: "soul.expected_blake3".to_owned(),
                value: raw.to_owned(),
                expected: "64-hex-char BLAKE3 digest",
            });
        }
        let mut out = [0_u8; 32];
        for i in 0..32 {
            let pair = std::str::from_utf8(&bytes[i * 2..i * 2 + 2]).map_err(|_| {
                ConfigError::InvalidValue {
                    key: "soul.expected_blake3".to_owned(),
                    value: raw.to_owned(),
                    expected: "64-hex-char BLAKE3 digest",
                }
            })?;
            out[i] = u8::from_str_radix(pair, 16).map_err(|_| ConfigError::InvalidValue {
                key: "soul.expected_blake3".to_owned(),
                value: raw.to_owned(),
                expected: "64-hex-char BLAKE3 digest",
            })?;
        }
        Ok(out)
    }

    /// Loads and strictly validates `[soul] core_config` (or returns the built-in default when absent)
    /// using the shared `monoagens-store` loader and merges gateway `[soul]` overrides.
    pub fn load_core_config(&self) -> Result<LoadedCoreConfig, ConfigLoadError> {
        let mut loaded = load_core_config_or_default(self.soul.core_config.as_deref())?;
        loaded.config.seed = self.soul.seed;
        if let Some(mp) = &self.soul.model_path {
            loaded.config.model_path = Some(mp.display().to_string());
        }
        if let Some(hex) = &self.soul.expected_blake3 {
            let parsed = Self::parse_blake3_hex(hex).map_err(|e| ConfigLoadError::Parse {
                path: None,
                detail: e.to_string(),
            })?;
            loaded.config.expected_blake3 = Some(parsed);
        }
        if let Some(on) = self.soul.online_adaptation {
            loaded.config.online_adaptation = on;
            loaded.config.adapt.online_adaptation = on;
        }
        loaded.config.validate()?;
        let digest = monoagens_store::compute_config_digest(&loaded.config)?;
        loaded.digest = digest;
        let _ = blake3::hash(b"");
        Ok(loaded)
    }
}

/// Resolved filesystem paths for config, secrets, and state data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedPaths {
    pub config_path: PathBuf,
    pub secrets_path: PathBuf,
    pub data_dir: PathBuf,
}

/// Result of resolving, bootstrapping (on first run), loading, and validating config + secrets.
#[derive(Debug, Clone)]
pub struct LoadedGatewayEnv {
    pub config: GatewayConfig,
    pub secrets: ProviderSecrets,
    pub paths: ResolvedPaths,
    pub created_default: bool,
}

impl LoadedGatewayEnv {
    /// Verifies that the environment is ready to start the Gateway.
    ///
    /// On first run (when a default `monoagens.toml` was just created) or when the selected
    /// `openai_compatible` provider has neither an API key nor an endpoint in `secrets.toml`/env,
    /// startup fails loudly with a non-zero exit error explaining what must be configured.
    pub fn ensure_startup_ready(&self) -> Result<(), ConfigError> {
        if self.created_default {
            return Err(ConfigError::FirstRunCreated {
                config_path: self.paths.config_path.display().to_string(),
                secrets_path: self.paths.secrets_path.display().to_string(),
            });
        }
        if self.config.model.provider == ModelProviderKind::OpenAiCompatible
            && self.secrets.model_api_key.is_none()
            && self.secrets.model_endpoint.is_none()
        {
            return Err(ConfigError::MissingCredentials {
                provider: self.config.model.provider.as_str().to_owned(),
                secrets_path: self.paths.secrets_path.display().to_string(),
            });
        }
        Ok(())
    }
}

/// Typed configuration and startup errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigError {
    Io {
        path: String,
        detail: String,
    },
    Parse {
        path: String,
        detail: String,
    },
    MissingField {
        key: String,
    },
    UnsupportedVersion {
        found: u32,
        supported: u32,
    },
    InvalidType {
        key: String,
        expected: &'static str,
    },
    OutOfRange {
        key: &'static str,
        value: String,
        range: &'static str,
    },
    InvalidValue {
        key: String,
        value: String,
        expected: &'static str,
    },
    PathResolution(String),
    Secrets(SecretsLoadError),
    FirstRunCreated {
        config_path: String,
        secrets_path: String,
    },
    MissingCredentials {
        provider: String,
        secrets_path: String,
    },
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, detail } => write!(f, "config I/O error at '{path}': {detail}"),
            Self::Parse { path, detail } => write!(f, "invalid TOML in '{path}': {detail}"),
            Self::MissingField { key } => write!(f, "missing required config key '{key}'"),
            Self::UnsupportedVersion { found, supported } => write!(
                f,
                "unsupported monoagens.toml version {found} (supported version is {supported})"
            ),
            Self::InvalidType { key, expected } => {
                write!(f, "config key '{key}' has wrong type (expected {expected})")
            }
            Self::OutOfRange { key, value, range } => write!(
                f,
                "config key '{key}' value {value} is out of valid range {range}"
            ),
            Self::InvalidValue {
                key,
                value,
                expected,
            } => write!(
                f,
                "config key '{key}' value '{value}' is invalid (expected {expected})"
            ),
            Self::PathResolution(detail) => write!(f, "failed to resolve config path: {detail}"),
            Self::Secrets(err) => write!(f, "{err}"),
            Self::FirstRunCreated {
                config_path,
                secrets_path,
            } => write!(
                f,
                "first run: wrote default configuration to '{config_path}' without secrets; \
                 add provider credentials to '{secrets_path}' or environment variables and re-run"
            ),
            Self::MissingCredentials {
                provider,
                secrets_path,
            } => write!(
                f,
                "provider '{provider}' requires credentials or endpoint in '{secrets_path}' \
                 or environment variables (MONOAGENS_MODEL_API_KEY / OPENAI_API_KEY / MONOAGENS_MODEL_ENDPOINT)"
            ),
        }
    }
}

impl std::error::Error for ConfigError {}

impl From<SecretsLoadError> for ConfigError {
    fn from(err: SecretsLoadError) -> Self {
        Self::Secrets(err)
    }
}

/// Resolves config, secrets, and data paths using precedence:
/// `--config` -> `MONOAGENS_HOME` -> XDG (`$XDG_CONFIG_HOME/monoagens` or `$HOME/.config/monoagens`).
pub fn resolve_paths_with<F>(
    cli_config: Option<&Path>,
    env_lookup: F,
) -> Result<ResolvedPaths, ConfigError>
where
    F: Fn(&str) -> Option<String>,
{
    let non_empty = |name: &str| {
        env_lookup(name)
            .map(|v| v.trim().to_owned())
            .filter(|v| !v.is_empty())
    };

    if let Some(explicit) = cli_config {
        let parent = explicit
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let secrets_path = parent.join(SECRETS_FILENAME);
        let data_dir = non_empty("MONOAGENS_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| parent.join("data"));
        return Ok(ResolvedPaths {
            config_path: explicit.to_path_buf(),
            secrets_path,
            data_dir,
        });
    }

    if let Some(home) = non_empty("MONOAGENS_HOME") {
        let base = PathBuf::from(home);
        return Ok(ResolvedPaths {
            config_path: base.join(CONFIG_FILENAME),
            secrets_path: base.join(SECRETS_FILENAME),
            data_dir: base.join("data"),
        });
    }

    let config_root = if let Some(xdg_cfg) = non_empty("XDG_CONFIG_HOME") {
        PathBuf::from(xdg_cfg)
    } else if let Some(home) = non_empty("HOME") {
        PathBuf::from(home).join(".config")
    } else {
        return Err(ConfigError::PathResolution(
            "none of --config, MONOAGENS_HOME, XDG_CONFIG_HOME, or HOME is set".to_owned(),
        ));
    };

    let data_root = if let Some(xdg_data) = non_empty("XDG_DATA_HOME") {
        PathBuf::from(xdg_data)
    } else if let Some(home) = non_empty("HOME") {
        PathBuf::from(home).join(".local").join("share")
    } else {
        config_root.clone()
    };

    let cfg_dir = config_root.join("monoagens");
    Ok(ResolvedPaths {
        config_path: cfg_dir.join(CONFIG_FILENAME),
        secrets_path: cfg_dir.join(SECRETS_FILENAME),
        data_dir: data_root.join("monoagens"),
    })
}

/// Resolves paths using process environment variables.
pub fn resolve_paths(cli_config: Option<&Path>) -> Result<ResolvedPaths, ConfigError> {
    resolve_paths_with(cli_config, |k| std::env::var(k).ok())
}

/// Parses `--config <path>` or `--config=<path>` from an argument iterator.
pub fn extract_cli_config_arg<'a>(
    args: impl IntoIterator<Item = &'a str>,
) -> Result<Option<PathBuf>, ConfigError> {
    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        if arg == "--config" {
            let val = iter.next().ok_or_else(|| {
                ConfigError::PathResolution("--config requires a path argument".to_owned())
            })?;
            if val.trim().is_empty() {
                return Err(ConfigError::PathResolution(
                    "--config path must not be empty".to_owned(),
                ));
            }
            return Ok(Some(PathBuf::from(val)));
        }
        if let Some(rest) = arg.strip_prefix("--config=") {
            if rest.trim().is_empty() {
                return Err(ConfigError::PathResolution(
                    "--config= path must not be empty".to_owned(),
                ));
            }
            return Ok(Some(PathBuf::from(rest)));
        }
    }
    Ok(None)
}

/// Renders a clean default `monoagens.toml` (`version = 1`) using `toml_edit` with zero secrets.
#[must_use]
pub fn default_config_document(cfg: &GatewayConfig) -> DocumentMut {
    let mut doc = DocumentMut::new();
    doc["version"] = value(i64::from(cfg.version));

    let mut soul = Table::new();
    soul["name"] = value(cfg.soul.name.clone());
    soul["seed"] = value(cfg.soul.seed as i64);
    if let Some(cfg_path) = &cfg.soul.core_config {
        soul["core_config"] = value(cfg_path.display().to_string());
    }
    if let Some(model_path) = &cfg.soul.model_path {
        soul["model_path"] = value(model_path.display().to_string());
    }
    if let Some(hex) = &cfg.soul.expected_blake3 {
        soul["expected_blake3"] = value(hex.clone());
    }
    if let Some(adapt) = cfg.soul.online_adaptation {
        soul["online_adaptation"] = value(adapt);
    }
    doc["soul"] = Item::Table(soul);

    let mut runtime = Table::new();
    runtime["hook_timeout_ms"] = value(cfg.runtime.hook_timeout_ms as i64);
    runtime["action_timeout_ms"] = value(cfg.runtime.action_timeout_ms as i64);
    runtime["model_timeout_ms"] = value(cfg.runtime.model_timeout_ms as i64);
    runtime["working_cue_ms"] = value(cfg.runtime.working_cue_ms as i64);
    runtime["progress_note_ms"] = value(cfg.runtime.progress_note_ms as i64);
    runtime["body_shutdown_grace_ms"] = value(cfg.runtime.body_shutdown_grace_ms as i64);
    doc["runtime"] = Item::Table(runtime);

    let mut model = Table::new();
    model["provider"] = value(cfg.model.provider.as_str());
    model["model"] = value(cfg.model.model.clone());
    model["daily_token_budget"] = value(cfg.model.daily_token_budget as i64);
    doc["model"] = Item::Table(model);

    let mut valve = Table::new();
    valve["mode"] = value(cfg.valve.mode.clone());
    valve["max_chain_wakes"] = value(i64::from(cfg.valve.max_chain_wakes));
    doc["valve"] = Item::Table(valve);

    doc
}

/// Writes the default `monoagens.toml` (`version = 1`, no secrets) using `toml_edit`.
pub fn write_default_config(path: &Path, cfg: &GatewayConfig) -> Result<(), ConfigError> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).map_err(|e| ConfigError::Io {
            path: parent.display().to_string(),
            detail: e.to_string(),
        })?;
    }
    let doc = default_config_document(cfg);
    std::fs::write(path, doc.to_string()).map_err(|e| ConfigError::Io {
        path: path.display().to_string(),
        detail: e.to_string(),
    })
}

/// Safely updates an existing `monoagens.toml` with `toml_edit` after writing a `.bak` backup,
/// preserving comments, formatting, and unknown keys, and rejecting inline secrets.
pub fn update_config_with_backup(
    path: &Path,
    secrets_path: &Path,
    updater: impl FnOnce(&mut DocumentMut) -> Result<(), ConfigError>,
) -> Result<(), ConfigError> {
    let raw = std::fs::read_to_string(path).map_err(|e| ConfigError::Io {
        path: path.display().to_string(),
        detail: e.to_string(),
    })?;
    let mut doc = raw.parse::<DocumentMut>().map_err(|e| ConfigError::Parse {
        path: path.display().to_string(),
        detail: e.to_string(),
    })?;

    let backup_path = PathBuf::from(format!("{}.bak", path.display()));
    std::fs::write(&backup_path, &raw).map_err(|e| ConfigError::Io {
        path: backup_path.display().to_string(),
        detail: e.to_string(),
    })?;

    updater(&mut doc)?;
    validate_no_inline_secrets_in_toml(&doc, path, secrets_path)?;
    let _parsed = parse_config_document(&doc)?;

    std::fs::write(path, doc.to_string()).map_err(|e| ConfigError::Io {
        path: path.display().to_string(),
        detail: e.to_string(),
    })
}

/// Loads `monoagens.toml` (creating a default `version = 1` file on first run without secrets)
/// and loads isolated `ProviderSecrets` from `secrets.toml` / environment variables.
pub fn load_or_init_gateway_env<F>(
    cli_config: Option<&Path>,
    env_lookup: F,
) -> Result<LoadedGatewayEnv, ConfigError>
where
    F: Fn(&str) -> Option<String>,
{
    let mut paths = resolve_paths_with(cli_config, &env_lookup)?;
    let mut created_default = false;
    let config = if paths.config_path.exists() {
        load_config_file(&paths.config_path, &paths.secrets_path)?
    } else {
        let default_cfg = GatewayConfig::default();
        write_default_config(&paths.config_path, &default_cfg)?;
        created_default = true;
        default_cfg
    };

    if let Some(custom_data_dir) = &config.data_dir {
        paths.data_dir = custom_data_dir.clone();
    }

    let secrets = load_provider_secrets(Some(&paths.secrets_path), env_lookup)?;

    Ok(LoadedGatewayEnv {
        config,
        secrets,
        paths,
        created_default,
    })
}

/// Loads and strictly validates an existing `monoagens.toml` file.
pub fn load_config_file(
    config_path: &Path,
    secrets_path: &Path,
) -> Result<GatewayConfig, ConfigError> {
    let raw = std::fs::read_to_string(config_path).map_err(|e| ConfigError::Io {
        path: config_path.display().to_string(),
        detail: e.to_string(),
    })?;
    parse_config_str(&raw, config_path, secrets_path)
}

/// Parses and validates `monoagens.toml` text, rejecting inline secrets, newer versions, and
/// out-of-range values while tolerating unknown keys.
pub fn parse_config_str(
    raw: &str,
    config_path: &Path,
    secrets_path: &Path,
) -> Result<GatewayConfig, ConfigError> {
    let doc = raw.parse::<DocumentMut>().map_err(|e| ConfigError::Parse {
        path: config_path.display().to_string(),
        detail: e.to_string(),
    })?;
    validate_no_inline_secrets_in_toml(&doc, config_path, secrets_path)?;
    parse_config_document(&doc)
}

fn parse_config_document(doc: &DocumentMut) -> Result<GatewayConfig, ConfigError> {
    let root = doc.as_table();

    let version_i64 = get_required_i64(root, "version")?;
    let version = u32::try_from(version_i64).map_err(|_| ConfigError::UnsupportedVersion {
        found: if version_i64 < 0 { 0 } else { u32::MAX },
        supported: CURRENT_CONFIG_VERSION,
    })?;
    if version != CURRENT_CONFIG_VERSION {
        return Err(ConfigError::UnsupportedVersion {
            found: version,
            supported: CURRENT_CONFIG_VERSION,
        });
    }

    let mut cfg = GatewayConfig::default();

    if let Some(data_dir) = get_opt_str(root, "data_dir", "data_dir")? {
        if data_dir.trim().is_empty() {
            return Err(ConfigError::InvalidValue {
                key: "data_dir".to_owned(),
                value: data_dir,
                expected: "non-empty directory path",
            });
        }
        cfg.data_dir = Some(PathBuf::from(data_dir));
    }

    if let Some(soul_tbl) = get_opt_table(root, "soul")? {
        if let Some(name) = get_opt_str(soul_tbl, "name", "soul.name")? {
            if name.trim().is_empty() {
                return Err(ConfigError::InvalidValue {
                    key: "soul.name".to_owned(),
                    value: name,
                    expected: "non-empty string",
                });
            }
            cfg.soul.name = name;
        }
        if let Some(seed_i64) = get_opt_i64(soul_tbl, "seed", "soul.seed")? {
            cfg.soul.seed = u64::try_from(seed_i64).map_err(|_| ConfigError::OutOfRange {
                key: "soul.seed",
                value: seed_i64.to_string(),
                range: "0..=9223372036854775807",
            })?;
        }
        if let Some(cfg_str) = get_opt_str(soul_tbl, "core_config", "soul.core_config")? {
            if cfg_str.trim().is_empty() {
                return Err(ConfigError::InvalidValue {
                    key: "soul.core_config".to_owned(),
                    value: cfg_str,
                    expected: "non-empty CoreConfig JSON file path",
                });
            }
            cfg.soul.core_config = Some(PathBuf::from(cfg_str));
        }
        if let Some(model_str) = get_opt_str(soul_tbl, "model_path", "soul.model_path")? {
            if model_str.trim().is_empty() {
                return Err(ConfigError::InvalidValue {
                    key: "soul.model_path".to_owned(),
                    value: model_str,
                    expected: "non-empty safetensors model path",
                });
            }
            cfg.soul.model_path = Some(PathBuf::from(model_str));
        }
        if let Some(hex_str) = get_opt_str(soul_tbl, "expected_blake3", "soul.expected_blake3")? {
            let _ = GatewayConfig::parse_blake3_hex(&hex_str)?;
            cfg.soul.expected_blake3 = Some(hex_str);
        }
        if let Some(adapt) = get_opt_bool(soul_tbl, "online_adaptation", "soul.online_adaptation")? {
            cfg.soul.online_adaptation = Some(adapt);
        }
    }

    if let Some(rt_tbl) = get_opt_table(root, "runtime")? {
        if let Some(v) = get_opt_i64(rt_tbl, "hook_timeout_ms", "runtime.hook_timeout_ms")? {
            cfg.runtime.hook_timeout_ms =
                check_u64_range("runtime.hook_timeout_ms", v, 1, 60_000, "1..=60000")?;
        }
        if let Some(v) = get_opt_i64(rt_tbl, "action_timeout_ms", "runtime.action_timeout_ms")? {
            cfg.runtime.action_timeout_ms =
                check_u64_range("runtime.action_timeout_ms", v, 1, 600_000, "1..=600000")?;
        }
        if let Some(v) = get_opt_i64(rt_tbl, "model_timeout_ms", "runtime.model_timeout_ms")? {
            cfg.runtime.model_timeout_ms =
                check_u64_range("runtime.model_timeout_ms", v, 1, 600_000, "1..=600000")?;
        }
        if let Some(v) = get_opt_i64(rt_tbl, "working_cue_ms", "runtime.working_cue_ms")? {
            cfg.runtime.working_cue_ms =
                check_u64_range("runtime.working_cue_ms", v, 1, 60_000, "1..=60000")?;
        }
        if let Some(v) = get_opt_i64(rt_tbl, "progress_note_ms", "runtime.progress_note_ms")? {
            cfg.runtime.progress_note_ms =
                check_u64_range("runtime.progress_note_ms", v, 1, 600_000, "1..=600000")?;
        }
        if let Some(v) = get_opt_i64(
            rt_tbl,
            "body_shutdown_grace_ms",
            "runtime.body_shutdown_grace_ms",
        )? {
            cfg.runtime.body_shutdown_grace_ms =
                check_u64_range("runtime.body_shutdown_grace_ms", v, 1, 60_000, "1..=60000")?;
        }
    }

    if cfg.runtime.progress_note_ms < cfg.runtime.working_cue_ms {
        return Err(ConfigError::OutOfRange {
            key: "runtime.progress_note_ms",
            value: cfg.runtime.progress_note_ms.to_string(),
            range: ">= runtime.working_cue_ms",
        });
    }

    if let Some(model_tbl) = get_opt_table(root, "model")? {
        if let Some(prov) = get_opt_str(model_tbl, "provider", "model.provider")? {
            cfg.model.provider =
                ModelProviderKind::parse(&prov).ok_or_else(|| ConfigError::InvalidValue {
                    key: "model.provider".to_owned(),
                    value: prov,
                    expected: "'openai_compatible' or 'scripted'",
                })?;
        }
        if let Some(model_name) = get_opt_str(model_tbl, "model", "model.model")? {
            if model_name.trim().is_empty() {
                return Err(ConfigError::InvalidValue {
                    key: "model.model".to_owned(),
                    value: model_name,
                    expected: "non-empty model name",
                });
            }
            cfg.model.model = model_name;
        }
        if let Some(budget) =
            get_opt_i64(model_tbl, "daily_token_budget", "model.daily_token_budget")?
        {
            cfg.model.daily_token_budget = check_u64_range(
                "model.daily_token_budget",
                budget,
                1,
                100_000_000,
                "1..=100000000",
            )?;
        }
        if let Some(max_tok) =
            get_opt_i64(model_tbl, "max_output_tokens", "model.max_output_tokens")?
        {
            let v = check_u64_range(
                "model.max_output_tokens",
                max_tok,
                1,
                1_000_000,
                "1..=1000000",
            )?;
            cfg.model.max_output_tokens = Some(v as u32);
        }
        if let Some(temp) = get_opt_f64(model_tbl, "temperature", "model.temperature")? {
            if !temp.is_finite() || !(0.0..=2.0).contains(&temp) {
                return Err(ConfigError::OutOfRange {
                    key: "model.temperature",
                    value: temp.to_string(),
                    range: "0.0..=2.0",
                });
            }
            cfg.model.temperature = Some(temp as f32);
        }
        if let Some(param) =
            get_opt_str(model_tbl, "output_limit_param", "model.output_limit_param")?
        {
            if param.trim().is_empty() {
                return Err(ConfigError::InvalidValue {
                    key: "model.output_limit_param".to_owned(),
                    value: param,
                    expected: "non-empty parameter name",
                });
            }
            cfg.model.output_limit_param = param.trim().to_owned();
        }
        if let Some(lim_tbl) = get_opt_table(model_tbl, "output_limits")? {
            if let Some(v) = get_opt_i64(lim_tbl, "reason", "model.output_limits.reason")? {
                cfg.model.output_limits.reason =
                    check_u64_range("model.output_limits.reason", v, 1, 1_000_000, "1..=1000000")?
                        as u32;
            }
            if let Some(v) = get_opt_i64(lim_tbl, "classify", "model.output_limits.classify")? {
                cfg.model.output_limits.classify = check_u64_range(
                    "model.output_limits.classify",
                    v,
                    1,
                    1_000_000,
                    "1..=1000000",
                )? as u32;
            }
            if let Some(v) = get_opt_i64(lim_tbl, "consolidate", "model.output_limits.consolidate")?
            {
                cfg.model.output_limits.consolidate = check_u64_range(
                    "model.output_limits.consolidate",
                    v,
                    1,
                    1_000_000,
                    "1..=1000000",
                )? as u32;
            }
        }
        if let Some(hdr_tbl) = get_opt_table(model_tbl, "extra_headers")? {
            for (k, val) in hdr_tbl.iter() {
                let valid_name =
                    monoagens_reason::validate_extra_header_name(k).map_err(|_| {
                        ConfigError::InvalidValue {
                            key: format!("model.extra_headers.{k}"),
                            value: k.to_owned(),
                            expected: "non-protocol HTTP header name (cannot override Authorization, Content-Type, Content-Length, or Host)",
                        }
                    })?;
                let s = val.as_str().ok_or_else(|| ConfigError::InvalidType {
                    key: format!("model.extra_headers.{k}"),
                    expected: "string",
                })?;
                cfg.model.extra_headers.push((valid_name, s.to_owned()));
            }
        }
        if let Some(up_tbl) = get_opt_table(model_tbl, "usage_paths")? {
            if let Some(v) = get_opt_str(up_tbl, "input_tokens", "model.usage_paths.input_tokens")?
            {
                cfg.model.usage_paths.input_tokens = v;
            }
            if let Some(v) =
                get_opt_str(up_tbl, "output_tokens", "model.usage_paths.output_tokens")?
            {
                cfg.model.usage_paths.output_tokens = v;
            }
            if let Some(v) =
                get_opt_str(up_tbl, "cached_tokens", "model.usage_paths.cached_tokens")?
            {
                cfg.model.usage_paths.cached_tokens = v;
            }
            if let Some(v) = get_opt_str(
                up_tbl,
                "reasoning_tokens",
                "model.usage_paths.reasoning_tokens",
            )? {
                cfg.model.usage_paths.reasoning_tokens = v;
            }
            if let Some(v) = get_opt_str(up_tbl, "total_tokens", "model.usage_paths.total_tokens")?
            {
                cfg.model.usage_paths.total_tokens = v;
            }
            if let Some(v) = get_opt_str(up_tbl, "cost", "model.usage_paths.cost")? {
                cfg.model.usage_paths.cost = if v.trim().is_empty() { None } else { Some(v) };
            }
        }
    }

    if let Some(valve_tbl) = get_opt_table(root, "valve")? {
        if let Some(mode) = get_opt_str(valve_tbl, "mode", "valve.mode")? {
            match mode.trim() {
                "enforcing" | "partial" | "off" => cfg.valve.mode = mode.trim().to_owned(),
                _ => {
                    return Err(ConfigError::InvalidValue {
                        key: "valve.mode".to_owned(),
                        value: mode,
                        expected: "'enforcing', 'partial', or 'off'",
                    });
                }
            }
        }
        if let Some(mcw) = get_opt_i64(valve_tbl, "max_chain_wakes", "valve.max_chain_wakes")? {
            let v = check_u64_range("valve.max_chain_wakes", mcw, 1, 1_000, "1..=1000")?;
            cfg.valve.max_chain_wakes = v as u32;
        }
    }

    if let Some(bodies_item) = root.get("bodies") {
        let arr = bodies_item
            .as_array_of_tables()
            .ok_or(ConfigError::InvalidType {
                key: "bodies".to_owned(),
                expected: "array of tables [[bodies]]",
            })?;
        for (idx, tbl) in arr.iter().enumerate() {
            let prefix = format!("bodies[{idx}]");
            let id = get_opt_str(tbl, "id", &format!("{prefix}.id"))?.ok_or_else(|| {
                ConfigError::MissingField {
                    key: format!("{prefix}.id"),
                }
            })?;
            if id.trim().is_empty() {
                return Err(ConfigError::InvalidValue {
                    key: format!("{prefix}.id"),
                    value: id,
                    expected: "non-empty body id",
                });
            }
            let transport_kind = get_opt_str(tbl, "transport", &format!("{prefix}.transport"))?
                .unwrap_or_else(|| "stdio".to_owned());
            let transport = match transport_kind.trim() {
                "stdio" => {
                    let command = get_opt_str(tbl, "command", &format!("{prefix}.command"))?
                        .ok_or_else(|| ConfigError::MissingField {
                            key: format!("{prefix}.command"),
                        })?;
                    let args = get_opt_str_array(tbl, "args", &format!("{prefix}.args"))?;
                    BodyTransportConfig::Stdio { command, args }
                }
                "unix" | "unix_socket" => {
                    let socket_path =
                        get_opt_str(tbl, "socket_path", &format!("{prefix}.socket_path"))?
                            .ok_or_else(|| ConfigError::MissingField {
                                key: format!("{prefix}.socket_path"),
                            })?;
                    BodyTransportConfig::UnixSocket {
                        path: PathBuf::from(socket_path),
                    }
                }
                other => {
                    return Err(ConfigError::InvalidValue {
                        key: format!("{prefix}.transport"),
                        value: other.to_owned(),
                        expected: "'stdio' or 'unix_socket'",
                    });
                }
            };
            cfg.bodies.push(BodyPluginConfig { id, transport });
        }
    }

    if let Some(embed_tbl) = get_opt_table(root, "embed")? {
        if let Some(backend) = get_opt_str(embed_tbl, "backend", "embed.backend")? {
            cfg.embed.backend = EmbedBackendKind::parse(&backend).ok_or_else(|| {
                ConfigError::InvalidValue {
                    key: "embed.backend".to_owned(),
                    value: backend,
                    expected: "'none', 'hash', 'embeddinggemma2_http', 'embeddinggemma2_stdio', 'embeddinggemma2_remote', or 'openai_compatible'",
                }
            })?;
        }
        if let Some(mv) = get_opt_str(embed_tbl, "model_ver", "embed.model_ver")? {
            if mv.trim().is_empty() {
                return Err(ConfigError::InvalidValue {
                    key: "embed.model_ver".to_owned(),
                    value: mv,
                    expected: "non-empty model version",
                });
            }
            cfg.embed.model_ver = mv;
        }
        if let Some(dim) = get_opt_i64(embed_tbl, "dim_d", "embed.dim_d")? {
            let v = check_u64_range("embed.dim_d", dim, 1, 8192, "1..=8192")?;
            cfg.embed.dim_d = v as u32;
        }
        if let Some(timeout_ms) = get_opt_i64(embed_tbl, "timeout_ms", "embed.timeout_ms")? {
            cfg.embed.timeout_ms =
                check_u64_range("embed.timeout_ms", timeout_ms, 1, 600_000, "1..=600000")?;
        }
        if let Some(mp) = get_opt_str(embed_tbl, "model_path", "embed.model_path")?
            && !mp.trim().is_empty()
        {
            cfg.embed.model_path = Some(PathBuf::from(mp));
        }
        if let Some(ep) = get_opt_str(embed_tbl, "endpoint_url", "embed.endpoint_url")?
            && !ep.trim().is_empty()
        {
            cfg.embed.endpoint_url = Some(ep);
        }
        if let Some(prog) = get_opt_str(embed_tbl, "program", "embed.program")?
            && !prog.trim().is_empty()
        {
            cfg.embed.program = Some(prog);
        }
        if embed_tbl.get("args").is_some() {
            cfg.embed.args = get_opt_str_array(embed_tbl, "args", "embed.args")?;
        }
        if let Some(rm) = get_opt_str(embed_tbl, "remote_model", "embed.remote_model")?
            && !rm.trim().is_empty()
        {
            cfg.embed.remote_model = Some(rm);
        }
    }

    if let Some(id_tbl) = get_opt_table(root, "identity")? {
        if let Some(heur) = get_opt_bool(
            id_tbl,
            "heuristic_merge_enabled",
            "identity.heuristic_merge_enabled",
        )? {
            if heur {
                return Err(ConfigError::InvalidValue {
                    key: "identity.heuristic_merge_enabled".to_owned(),
                    value: "true".to_owned(),
                    expected: "false (invariant-locked T_mu = +inf)",
                });
            }
            cfg.identity.heuristic_merge_enabled = false;
        }
        if let Some(ttl_s) = get_opt_i64(
            id_tbl,
            "link_challenge_ttl_s",
            "identity.link_challenge_ttl_s",
        )? {
            cfg.identity.link_challenge_ttl_s = check_u64_range(
                "identity.link_challenge_ttl_s",
                ttl_s,
                1,
                86_400,
                "1..=86400",
            )?;
        }
        if let Some(attempts) =
            get_opt_i64(id_tbl, "link_max_attempts", "identity.link_max_attempts")?
        {
            let v = check_u64_range("identity.link_max_attempts", attempts, 1, 20, "1..=20")?;
            cfg.identity.link_max_attempts = v as u32;
        }
        if let Some(retro) =
            get_opt_bool(id_tbl, "retroactive_merge", "identity.retroactive_merge")?
        {
            cfg.identity.retroactive_merge = retro;
        }
        if let Some(guard) = get_opt_bool(
            id_tbl,
            "owner_escalation_guard",
            "identity.owner_escalation_guard",
        )? {
            if !guard {
                return Err(ConfigError::InvalidValue {
                    key: "identity.owner_escalation_guard".to_owned(),
                    value: "false".to_owned(),
                    expected: "true (invariant-locked Absolute Line 1 guard)",
                });
            }
            cfg.identity.owner_escalation_guard = true;
        }
    }

    if let Some(person_item) = root.get("person") {
        let arr = person_item
            .as_array_of_tables()
            .ok_or(ConfigError::InvalidType {
                key: "person".to_owned(),
                expected: "array of tables [[person]]",
            })?;
        for (idx, tbl) in arr.iter().enumerate() {
            let prefix = format!("person[{idx}]");
            let actor_raw = get_opt_str(tbl, "actor_id", &format!("{prefix}.actor_id"))?
                .or(get_opt_str(tbl, "id", &format!("{prefix}.id"))?)
                .ok_or_else(|| ConfigError::MissingField {
                    key: format!("{prefix}.actor_id"),
                })?;
            if actor_raw.trim().is_empty() {
                return Err(ConfigError::InvalidValue {
                    key: format!("{prefix}.actor_id"),
                    value: actor_raw,
                    expected: "non-empty actor_id",
                });
            }
            let is_owner = get_opt_bool(tbl, "owner", &format!("{prefix}.owner"))?
                .or(get_opt_bool(
                    tbl,
                    "is_owner",
                    &format!("{prefix}.is_owner"),
                )?)
                .unwrap_or(false);

            let mut accounts = Vec::new();
            if let (Some(bid), Some(aid)) = (
                get_opt_str(tbl, "body_id", &format!("{prefix}.body_id"))?,
                get_opt_str(tbl, "account_id", &format!("{prefix}.account_id"))?,
            ) {
                accounts.push(PersonAccountConfig {
                    body_id: BodyId::new(bid),
                    account_id: aid,
                });
            }
            if let Some(accts_item) = tbl.get("accounts") {
                let accts_arr = accts_item
                    .as_array()
                    .ok_or_else(|| ConfigError::InvalidType {
                        key: format!("{prefix}.accounts"),
                        expected: "array of { body_id, account_id } tables",
                    })?;
                for (aidx, val) in accts_arr.iter().enumerate() {
                    let it = val
                        .as_inline_table()
                        .ok_or_else(|| ConfigError::InvalidType {
                            key: format!("{prefix}.accounts[{aidx}]"),
                            expected: "inline table { body_id = ..., account_id = ... }",
                        })?;
                    let bid = it.get("body_id").and_then(|v| v.as_str()).ok_or_else(|| {
                        ConfigError::MissingField {
                            key: format!("{prefix}.accounts[{aidx}].body_id"),
                        }
                    })?;
                    let aid = it
                        .get("account_id")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| ConfigError::MissingField {
                            key: format!("{prefix}.accounts[{aidx}].account_id"),
                        })?;
                    accounts.push(PersonAccountConfig {
                        body_id: BodyId::new(bid),
                        account_id: aid.to_owned(),
                    });
                }
            }
            cfg.persons.push(PersonMappingConfig {
                actor_id: ActorId::new(actor_raw),
                is_owner,
                accounts,
            });
        }
    }

    Ok(cfg)
}

fn check_u64_range(
    key: &'static str,
    val: i64,
    min: u64,
    max: u64,
    range: &'static str,
) -> Result<u64, ConfigError> {
    if val < 0 {
        return Err(ConfigError::OutOfRange {
            key,
            value: val.to_string(),
            range,
        });
    }
    let u = val as u64;
    if u < min || u > max {
        return Err(ConfigError::OutOfRange {
            key,
            value: val.to_string(),
            range,
        });
    }
    Ok(u)
}

fn get_required_i64(table: &dyn TableLike, key: &str) -> Result<i64, ConfigError> {
    let item = table.get(key).ok_or_else(|| ConfigError::MissingField {
        key: key.to_owned(),
    })?;
    item.as_integer().ok_or_else(|| ConfigError::InvalidType {
        key: key.to_owned(),
        expected: "integer",
    })
}

fn get_opt_i64(
    table: &dyn TableLike,
    key: &str,
    full_key: &str,
) -> Result<Option<i64>, ConfigError> {
    match table.get(key) {
        None | Some(Item::None) => Ok(None),
        Some(item) => item
            .as_integer()
            .map(Some)
            .ok_or_else(|| ConfigError::InvalidType {
                key: full_key.to_owned(),
                expected: "integer",
            }),
    }
}

fn get_opt_f64(
    table: &dyn TableLike,
    key: &str,
    full_key: &str,
) -> Result<Option<f64>, ConfigError> {
    match table.get(key) {
        None | Some(Item::None) => Ok(None),
        Some(item) => {
            if let Some(f) = item.as_float() {
                Ok(Some(f))
            } else if let Some(i) = item.as_integer() {
                Ok(Some(i as f64))
            } else {
                Err(ConfigError::InvalidType {
                    key: full_key.to_owned(),
                    expected: "number",
                })
            }
        }
    }
}

fn get_opt_bool(
    table: &dyn TableLike,
    key: &str,
    full_key: &str,
) -> Result<Option<bool>, ConfigError> {
    match table.get(key) {
        None | Some(Item::None) => Ok(None),
        Some(item) => item
            .as_bool()
            .map(Some)
            .ok_or_else(|| ConfigError::InvalidType {
                key: full_key.to_owned(),
                expected: "boolean",
            }),
    }
}

fn get_opt_str(
    table: &dyn TableLike,
    key: &str,
    full_key: &str,
) -> Result<Option<String>, ConfigError> {
    match table.get(key) {
        None | Some(Item::None) => Ok(None),
        Some(item) => {
            item.as_str()
                .map(|s| Some(s.to_owned()))
                .ok_or_else(|| ConfigError::InvalidType {
                    key: full_key.to_owned(),
                    expected: "string",
                })
        }
    }
}

fn get_opt_str_array(
    table: &dyn TableLike,
    key: &str,
    full_key: &str,
) -> Result<Vec<String>, ConfigError> {
    match table.get(key) {
        None | Some(Item::None) => Ok(Vec::new()),
        Some(item) => {
            let arr = item.as_array().ok_or_else(|| ConfigError::InvalidType {
                key: full_key.to_owned(),
                expected: "array of strings",
            })?;
            let mut out = Vec::with_capacity(arr.len());
            for (idx, v) in arr.iter().enumerate() {
                let s = v.as_str().ok_or_else(|| ConfigError::InvalidType {
                    key: format!("{full_key}[{idx}]"),
                    expected: "string",
                })?;
                out.push(s.to_owned());
            }
            Ok(out)
        }
    }
}

fn get_opt_table<'a>(
    table: &'a dyn TableLike,
    key: &str,
) -> Result<Option<&'a dyn TableLike>, ConfigError> {
    match table.get(key) {
        None | Some(Item::None) => Ok(None),
        Some(item) => item
            .as_table_like()
            .map(Some)
            .ok_or_else(|| ConfigError::InvalidType {
                key: key.to_owned(),
                expected: "table",
            }),
    }
}
