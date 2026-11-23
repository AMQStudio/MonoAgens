//! Self-managed plugin configuration and state directory resolution for `plugin-workspace`.
//!
//! Reads `MONOAGENS_PLUGIN_CONFIG` (auto-generating a commented default `workspace.toml`
//! on first run only when the env var is set and non-empty) and resolves persistent state
//! via `MONOAGENS_PLUGIN_STATE_DIR` with fallback to `MONOAGENS_BODY_STATE_DIR`.

#![forbid(unsafe_code)]

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

/// Environment variable toggled by Gateway when RBAC enforcement is active (`"1"`).
pub const ENV_RBAC_ENABLED: &str = "MONOAGENS_RBAC_ENABLED";

/// Current schema version for `workspace.toml`.
pub const WORKSPACE_CONFIG_VERSION: u32 = 1;

/// Default maximum readable/writable file size in bytes (2 MB).
pub const DEFAULT_MAX_FILE_BYTES: u64 = 2_000_000;

/// Default maximum `workspace.run` timeout in seconds.
pub const DEFAULT_TIMEOUT_SECS: u64 = 60;

/// Default maximum stdout/stderr capture size per stream in bytes (64 KiB).
pub const DEFAULT_MAX_OUTPUT_BYTES: usize = 65_536;

/// Default per-actor disk quota in bytes (100 MiB).
pub const DEFAULT_QUOTA_BYTES: u64 = 104_857_600;

/// Returns the default per-role disk quota mapping in bytes.
#[must_use]
pub fn default_role_quotas() -> BTreeMap<String, u64> {
    BTreeMap::from([
        ("guest".to_owned(), 0),
        ("member".to_owned(), 1_073_741_824),
        ("trusted".to_owned(), 5_368_709_120),
        ("admin".to_owned(), 10_737_418_240),
    ])
}

/// Self-managed configuration loaded from `workspace.toml` (`MONOAGENS_PLUGIN_CONFIG`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct WorkspaceConfig {
    pub version: u32,
    pub root: Option<String>,
    pub max_file_bytes: u64,
    pub timeout_secs: u64,
    pub max_output_bytes: usize,
    pub default_quota_bytes: u64,
    pub role_quotas: BTreeMap<String, u64>,
    pub run_enabled: bool,
}

impl Default for WorkspaceConfig {
    fn default() -> Self {
        Self {
            version: WORKSPACE_CONFIG_VERSION,
            root: None,
            max_file_bytes: DEFAULT_MAX_FILE_BYTES,
            timeout_secs: DEFAULT_TIMEOUT_SECS,
            max_output_bytes: DEFAULT_MAX_OUTPUT_BYTES,
            default_quota_bytes: DEFAULT_QUOTA_BYTES,
            role_quotas: default_role_quotas(),
            run_enabled: true,
        }
    }
}

impl WorkspaceConfig {
    /// Loads configuration from `MONOAGENS_PLUGIN_CONFIG` if set and non-empty.
    ///
    /// - When `MONOAGENS_PLUGIN_CONFIG` is unset or empty, returns [`Self::default()`] without
    ///   creating any files on disk (safe for offline conformance runs).
    /// - When set to a non-existent path, creates parent directories, writes a commented
    ///   default `workspace.toml`, and returns [`Self::default()`].
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
                    "failed to write default workspace plugin config '{}': {e}",
                    path.display()
                )
            })?;
            return Ok(defaults);
        }

        let content = fs::read_to_string(path).map_err(|e| {
            format!(
                "failed to read workspace plugin config '{}': {e}",
                path.display()
            )
        })?;
        Self::from_toml_str(&content).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// Parses a `workspace.toml` document into [`WorkspaceConfig`].
    pub fn from_toml_str(input: &str) -> Result<Self, String> {
        let mut cfg: Self =
            toml::from_str(input).map_err(|e| format!("invalid TOML config: {e}"))?;
        if cfg.version == 0 || cfg.version > WORKSPACE_CONFIG_VERSION {
            return Err(format!(
                "unsupported config version {} (expected 1)",
                cfg.version
            ));
        }
        if let Some(ref root) = cfg.root
            && root.trim().is_empty()
        {
            cfg.root = None;
        }
        if cfg.timeout_secs == 0 {
            cfg.timeout_secs = 1;
        }
        if cfg.max_output_bytes == 0 {
            cfg.max_output_bytes = DEFAULT_MAX_OUTPUT_BYTES;
        }
        Ok(cfg)
    }

    /// Renders a clean, well-commented TOML representation of this config.
    #[must_use]
    pub fn to_toml_string(&self) -> String {
        let root_line = match self.root.as_deref() {
            Some(r) if !r.trim().is_empty() => format!("root = {}", format_toml_string(r)),
            _ => "# root = \"/var/lib/monoagens/workspaces\"".to_owned(),
        };
        let mut role_lines = String::new();
        for (role, quota) in &self.role_quotas {
            role_lines.push_str(&format!("{role} = {quota}\n"));
        }
        format!(
            "# MonoAgens Workspace plugin configuration (self-managed)\n\
             version = {}\n\n\
             # Optional override for the workspace root directory\n\
             # (defaults to <MONOAGENS_PLUGIN_STATE_DIR>/workspaces or temp dir)\n\
             {}\n\n\
             # Maximum file size allowed for workspace.read / write / edit / grep (bytes)\n\
             max_file_bytes = {}\n\n\
             # Maximum command timeout allowed for workspace.run (seconds)\n\
             timeout_secs = {}\n\n\
             # Maximum captured stdout/stderr bytes per stream for workspace.run\n\
             max_output_bytes = {}\n\n\
             # Default per-actor recursive directory disk quota when RBAC is inactive (bytes)\n\
             default_quota_bytes = {}\n\n\
             # Enable or disable workspace.run command execution\n\
             run_enabled = {}\n\n\
             # Per-role disk quotas enforced when MONOAGENS_RBAC_ENABLED=1 (bytes)\n\
             [role_quotas]\n\
             {}",
            self.version,
            root_line,
            self.max_file_bytes,
            self.timeout_secs,
            self.max_output_bytes,
            self.default_quota_bytes,
            self.run_enabled,
            role_lines,
        )
    }

    /// Resolves the root directory under which per-actor sandboxes live.
    #[must_use]
    pub fn resolve_workspace_root(&self) -> PathBuf {
        if let Some(ref custom) = self.root {
            let trimmed = custom.trim();
            if !trimmed.is_empty() {
                return PathBuf::from(trimmed);
            }
        }
        if let Some(state_dir) = resolve_state_dir_from_env() {
            return state_dir.join("workspaces");
        }
        env::temp_dir().join("monoagens-workspaces")
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

/// Returns `true` if `MONOAGENS_RBAC_ENABLED` is `"1"`.
#[must_use]
pub fn is_rbac_enabled_from_env() -> bool {
    env::var(ENV_RBAC_ENABLED).ok().as_deref() == Some("1")
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn toml_roundtrip_with_comments_and_role_quotas() {
        let mut cfg = WorkspaceConfig {
            version: 1,
            root: Some("/tmp/ws-test".to_owned()),
            max_file_bytes: 512_000,
            timeout_secs: 30,
            max_output_bytes: 16_384,
            default_quota_bytes: 2_000_000,
            role_quotas: default_role_quotas(),
            run_enabled: false,
        };
        cfg.role_quotas.insert("vip".to_owned(), 99_999);
        let rendered = cfg.to_toml_string();
        let parsed = WorkspaceConfig::from_toml_str(&rendered).unwrap();
        assert_eq!(parsed, cfg);
    }

    #[test]
    fn toml_rejects_unsupported_version() {
        let err = WorkspaceConfig::from_toml_str("version = 99\n").unwrap_err();
        assert!(err.contains("unsupported config version 99"));
    }

    #[test]
    fn load_or_init_from_path_creates_default_then_reloads_edits() {
        let tmp = TempDir::new().unwrap();
        let cfg_path = tmp.path().join("nested").join("workspace.toml");
        assert!(!cfg_path.exists());

        let initial = WorkspaceConfig::load_or_init_from_path(&cfg_path).unwrap();
        assert_eq!(initial, WorkspaceConfig::default());
        assert!(cfg_path.exists());

        let custom = "version = 1\nmax_file_bytes = 4096\nrun_enabled = false\n[role_quotas]\nguest = 0\nmember = 8192\n";
        fs::write(&cfg_path, custom).unwrap();

        let reloaded = WorkspaceConfig::load_or_init_from_path(&cfg_path).unwrap();
        assert_eq!(reloaded.max_file_bytes, 4096);
        assert!(!reloaded.run_enabled);
        assert_eq!(reloaded.role_quotas.get("guest"), Some(&0));
        assert_eq!(reloaded.role_quotas.get("member"), Some(&8192));
    }
}
