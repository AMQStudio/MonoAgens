//! Self-managed plugin configuration (`whatsapp.toml`) and state directory resolution
//! for `body-whatsapp`.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use monoagens_body_kit::load_state_dir_from_env;
use serde::{Deserialize, Serialize};

/// Current schema version for `whatsapp.toml`.
pub const CONFIG_VERSION: u32 = 1;
/// Default clock-skew grace window (in seconds) before boot timestamp.
pub const DEFAULT_HISTORY_GRACE_SECS: u64 = 15;
/// Environment variable injected by Gateway pointing to `<config_dir>/plugins/whatsapp.toml`.
pub const ENV_PLUGIN_CONFIG: &str = "MONOAGENS_PLUGIN_CONFIG";
/// Environment variable injected by Gateway pointing to `<data_dir>/plugins/whatsapp`.
pub const ENV_PLUGIN_STATE_DIR: &str = "MONOAGENS_PLUGIN_STATE_DIR";

/// Self-managed plugin settings owned by `body-whatsapp` (`plugins/whatsapp.toml`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WhatsAppPluginConfig {
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default)]
    pub db_path: Option<String>,
    #[serde(default = "default_auto_typing_on_speak")]
    pub auto_typing_on_speak: bool,
    #[serde(default)]
    pub auto_mark_read: bool,
    #[serde(default = "default_history_grace_secs")]
    pub history_grace_secs: u64,
    #[serde(default = "default_approve_emojis")]
    pub approve_emojis: Vec<String>,
    #[serde(default = "default_deny_emojis")]
    pub deny_emojis: Vec<String>,
}

const fn default_version() -> u32 {
    CONFIG_VERSION
}

const fn default_auto_typing_on_speak() -> bool {
    true
}

const fn default_history_grace_secs() -> u64 {
    DEFAULT_HISTORY_GRACE_SECS
}

fn default_approve_emojis() -> Vec<String> {
    vec!["👍".to_owned(), "✅".to_owned()]
}

fn default_deny_emojis() -> Vec<String> {
    vec!["👎".to_owned(), "❌".to_owned()]
}

impl Default for WhatsAppPluginConfig {
    fn default() -> Self {
        Self {
            version: default_version(),
            db_path: None,
            auto_typing_on_speak: default_auto_typing_on_speak(),
            auto_mark_read: false,
            history_grace_secs: default_history_grace_secs(),
            approve_emojis: default_approve_emojis(),
            deny_emojis: default_deny_emojis(),
        }
    }
}

impl WhatsAppPluginConfig {
    /// Loads plugin configuration from `MONOAGENS_PLUGIN_CONFIG` if set and non-empty.
    ///
    /// - If `MONOAGENS_PLUGIN_CONFIG` is unset or empty, returns `Self::default()` without
    ///   touching the filesystem (preserving clean conformance runs).
    /// - If the file does not exist, creates parent directories, writes a commented default
    ///   `whatsapp.toml`, and returns `Self::default()`.
    /// - If the file exists, parses and returns it.
    pub fn load_or_init_from_env() -> Result<Self, String> {
        let Some(raw) = env::var_os(ENV_PLUGIN_CONFIG) else {
            return Ok(Self::default());
        };
        let trimmed = raw.to_string_lossy();
        if trimmed.trim().is_empty() {
            return Ok(Self::default());
        }
        Self::load_or_init_at(Path::new(trimmed.trim()))
    }

    /// Loads plugin configuration from `path`, writing a commented default `whatsapp.toml`
    /// on first run if `path` does not yet exist.
    pub fn load_or_init_at(path: &Path) -> Result<Self, String> {
        if path.exists() {
            let contents = fs::read_to_string(path).map_err(|e| {
                format!(
                    "failed to read WhatsApp plugin config '{}': {e}",
                    path.display()
                )
            })?;
            return toml::from_str::<Self>(&contents).map_err(|e| {
                format!(
                    "failed to parse WhatsApp plugin config '{}': {e}",
                    path.display()
                )
            });
        }

        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent).map_err(|e| {
                format!(
                    "failed to create WhatsApp plugin config directory '{}': {e}",
                    parent.display()
                )
            })?;
        }
        let template = Self::default_commented_toml();
        fs::write(path, template).map_err(|e| {
            format!(
                "failed to write default WhatsApp plugin config '{}': {e}",
                path.display()
            )
        })?;
        Ok(Self::default())
    }

    /// Renders a clean, well-commented default `whatsapp.toml` template.
    #[must_use]
    pub fn default_commented_toml() -> String {
        concat!(
            "# body-whatsapp self-managed plugin configuration\n",
            "# Generated automatically on first run. Restart body-whatsapp after editing.\n\n",
            "version = 1\n\n",
            "# Optional override for the WhatsApp SQLite session database path.\n",
            "# Defaults to $MONOAGENS_PLUGIN_STATE_DIR/session.db if set,\n",
            "# otherwise $XDG_DATA_HOME/monoagens/whatsapp/session.db (~/.local/share/monoagens/whatsapp/session.db).\n",
            "# db_path = \"/path/to/session.db\"\n\n",
            "# Send composing/paused chatstate indicators around chat.speak delivery.\n",
            "auto_typing_on_speak = true\n\n",
            "# Automatically send read receipts for translated inbound WhatsApp messages.\n",
            "auto_mark_read = false\n\n",
            "# Clock-skew grace window (seconds) before boot timestamp for dropping history messages.\n",
            "history_grace_secs = 15\n\n",
            "# Emoji reactions on an approval.prompt message that resolve the action as approved.\n",
            "approve_emojis = [\"👍\", \"✅\"]\n\n",
            "# Emoji reactions on an approval.prompt message that resolve the action as denied.\n",
            "deny_emojis = [\"👎\", \"❌\"]\n",
        )
        .to_owned()
    }

    /// Resolves the SQLite `session.db` path honoring `self.db_path`,
    /// `MONOAGENS_PLUGIN_STATE_DIR`, `XDG_DATA_HOME`, and `~/.local/share`.
    #[must_use]
    pub fn resolve_db_path(&self) -> PathBuf {
        if let Some(custom) = self.db_path.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            return expand_tilde(custom);
        }
        default_session_db_path()
    }

    /// Returns effective approve emojis, preferring a non-empty per-request override,
    /// then `self.approve_emojis`, then built-in defaults.
    #[must_use]
    pub fn effective_approve_emojis(&self, override_list: Option<&[String]>) -> Vec<String> {
        pick_non_empty_emojis(override_list, &self.approve_emojis, default_approve_emojis)
    }

    /// Returns effective deny emojis, preferring a non-empty per-request override,
    /// then `self.deny_emojis`, then built-in defaults.
    #[must_use]
    pub fn effective_deny_emojis(&self, override_list: Option<&[String]>) -> Vec<String> {
        pick_non_empty_emojis(override_list, &self.deny_emojis, default_deny_emojis)
    }
}

fn pick_non_empty_emojis(
    override_list: Option<&[String]>,
    configured: &[String],
    fallback: fn() -> Vec<String>,
) -> Vec<String> {
    if let Some(items) = override_list {
        let filtered: Vec<String> = items
            .iter()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(ToOwned::to_owned)
            .collect();
        if !filtered.is_empty() {
            return filtered;
        }
    }
    let configured_filtered: Vec<String> = configured
        .iter()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned)
        .collect();
    if !configured_filtered.is_empty() {
        return configured_filtered;
    }
    fallback()
}

fn expand_tilde(raw: &str) -> PathBuf {
    if let Some(rest) = raw.strip_prefix("~/")
        && let Some(home) = env::var_os("HOME").filter(|v| !v.is_empty())
    {
        return PathBuf::from(home).join(rest);
    }
    PathBuf::from(raw)
}

/// Resolves the Waist-2 persistence directory, checking `MONOAGENS_PLUGIN_STATE_DIR`
/// before falling back to `MONOAGENS_BODY_STATE_DIR`.
#[must_use]
pub fn resolve_state_dir_from_env() -> Option<PathBuf> {
    if let Some(dir) = env::var_os(ENV_PLUGIN_STATE_DIR).filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(dir));
    }
    load_state_dir_from_env()
}

/// Resolves the default WhatsApp SQLite `session.db` path, checking
/// `MONOAGENS_PLUGIN_STATE_DIR` first, then `XDG_DATA_HOME`, then `~/.local/share`.
#[must_use]
pub fn default_session_db_path() -> PathBuf {
    if let Some(plugin_state_dir) = env::var_os(ENV_PLUGIN_STATE_DIR).filter(|v| !v.is_empty()) {
        return PathBuf::from(plugin_state_dir).join("session.db");
    }
    let base = env::var_os("XDG_DATA_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            env::var_os("HOME")
                .filter(|v| !v.is_empty())
                .map(|h| PathBuf::from(h).join(".local/share"))
        })
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("monoagens/whatsapp/session.db")
}
