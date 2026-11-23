//! Self-managed plugin configuration and state directory resolution for `body-telegram`.
//!
//! Reads `MONOAGENS_PLUGIN_CONFIG` (auto-generating a commented default `telegram.toml`
//! on first run only when the env var is set and non-empty) and resolves state persistence
//! via `MONOAGENS_PLUGIN_STATE_DIR` with fallback to `MONOAGENS_BODY_STATE_DIR`.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use monoagens_body_kit::load_state_dir_from_env;

/// Environment variable pointing to the plugin's self-managed TOML config file.
pub const ENV_PLUGIN_CONFIG: &str = "MONOAGENS_PLUGIN_CONFIG";

/// Environment variable pointing to the plugin's self-managed state directory.
pub const ENV_PLUGIN_STATE_DIR: &str = "MONOAGENS_PLUGIN_STATE_DIR";

/// Current schema version for `telegram.toml`.
pub const TELEGRAM_CONFIG_VERSION: u32 = 1;

/// Default Telegram `getUpdates` long-poll timeout in seconds.
pub const DEFAULT_POLL_TIMEOUT_SECS: u64 = 20;

/// Default native inline approval button label.
pub const DEFAULT_APPROVE_BUTTON_LABEL: &str = "✅ Approve";

/// Default native inline denial button label.
pub const DEFAULT_DENY_BUTTON_LABEL: &str = "❌ Deny";

/// Self-managed configuration loaded from `plugins/telegram.toml` (`MONOAGENS_PLUGIN_CONFIG`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TelegramPluginConfig {
    pub version: u32,
    pub default_parse_mode: Option<String>,
    pub poll_timeout_secs: u64,
    pub auto_typing_on_speak: bool,
    pub approve_button_label: String,
    pub deny_button_label: String,
}

impl Default for TelegramPluginConfig {
    fn default() -> Self {
        Self {
            version: TELEGRAM_CONFIG_VERSION,
            default_parse_mode: None,
            poll_timeout_secs: DEFAULT_POLL_TIMEOUT_SECS,
            auto_typing_on_speak: true,
            approve_button_label: DEFAULT_APPROVE_BUTTON_LABEL.to_owned(),
            deny_button_label: DEFAULT_DENY_BUTTON_LABEL.to_owned(),
        }
    }
}

impl TelegramPluginConfig {
    /// Loads configuration from `MONOAGENS_PLUGIN_CONFIG` if set and non-empty.
    ///
    /// - When `MONOAGENS_PLUGIN_CONFIG` is unset or empty, returns [`Self::default()`] without
    ///   creating any files on disk (safe for conformance runs).
    /// - When set to a non-existent path, creates parent directories, writes a commented
    ///   default `telegram.toml`, and returns [`Self::default()`].
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
                    "failed to write default Telegram plugin config '{}': {e}",
                    path.display()
                )
            })?;
            return Ok(defaults);
        }

        let content = fs::read_to_string(path).map_err(|e| {
            format!(
                "failed to read Telegram plugin config '{}': {e}",
                path.display()
            )
        })?;
        Self::from_toml_str(&content).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// Renders a clean, well-commented TOML representation of this config.
    #[must_use]
    pub fn to_toml_string(&self) -> String {
        let parse_mode_line = match self.default_parse_mode.as_deref() {
            Some(mode) if !mode.trim().is_empty() => {
                format!("default_parse_mode = {}", format_toml_string(mode))
            }
            _ => "# default_parse_mode = \"HTML\"".to_owned(),
        };
        format!(
            "# MonoAgens Telegram plugin configuration (self-managed)\n\
             version = {}\n\n\
             # Optional default Telegram parse_mode when not specified per-action (\"HTML\", \"MarkdownV2\", \"Markdown\")\n\
             {}\n\n\
             # Long-polling timeout passed to Telegram getUpdates (seconds)\n\
             poll_timeout_secs = {}\n\n\
             # Automatically send a 'typing' chat action before chat.speak / chat.send_buttons\n\
             auto_typing_on_speak = {}\n\n\
             # Native inline_keyboard button labels used by approval.prompt\n\
             approve_button_label = {}\n\
             deny_button_label = {}\n",
            self.version,
            parse_mode_line,
            self.poll_timeout_secs,
            self.auto_typing_on_speak,
            format_toml_string(&self.approve_button_label),
            format_toml_string(&self.deny_button_label),
        )
    }

    /// Parses a `telegram.toml` document into [`TelegramPluginConfig`].
    pub fn from_toml_str(input: &str) -> Result<Self, String> {
        let mut cfg = Self::default();
        for (idx, raw_line) in input.lines().enumerate() {
            let line_no = idx + 1;
            let stripped = strip_toml_comment(raw_line).trim();
            if stripped.is_empty() {
                continue;
            }
            if stripped.starts_with('[') && stripped.ends_with(']') {
                continue;
            }
            let Some((raw_key, raw_val)) = stripped.split_once('=') else {
                return Err(format!("line {line_no}: expected 'key = value'"));
            };
            let key = raw_key.trim();
            let val = raw_val.trim();

            match key {
                "version" => {
                    let parsed = val.parse::<u32>().map_err(|e| {
                        format!("line {line_no}: invalid integer for 'version': {e}")
                    })?;
                    if parsed == 0 || parsed > TELEGRAM_CONFIG_VERSION {
                        return Err(format!(
                            "line {line_no}: unsupported config version {parsed} (expected 1)"
                        ));
                    }
                    cfg.version = parsed;
                }
                "default_parse_mode" => {
                    let parsed = parse_toml_string(val)
                        .map_err(|e| format!("line {line_no}: invalid 'default_parse_mode': {e}"))?;
                    cfg.default_parse_mode = if parsed.trim().is_empty() {
                        None
                    } else {
                        Some(parsed)
                    };
                }
                "poll_timeout_secs" => {
                    cfg.poll_timeout_secs = val.parse::<u64>().map_err(|e| {
                        format!("line {line_no}: invalid integer for 'poll_timeout_secs': {e}")
                    })?;
                }
                "auto_typing_on_speak" => {
                    cfg.auto_typing_on_speak = parse_toml_bool(val).map_err(|e| {
                        format!("line {line_no}: invalid boolean for 'auto_typing_on_speak': {e}")
                    })?;
                }
                "approve_button_label" => {
                    let parsed = parse_toml_string(val).map_err(|e| {
                        format!("line {line_no}: invalid 'approve_button_label': {e}")
                    })?;
                    if !parsed.trim().is_empty() {
                        cfg.approve_button_label = parsed;
                    }
                }
                "deny_button_label" => {
                    let parsed = parse_toml_string(val)
                        .map_err(|e| format!("line {line_no}: invalid 'deny_button_label': {e}"))?;
                    if !parsed.trim().is_empty() {
                        cfg.deny_button_label = parsed;
                    }
                }
                _ => {
                    // Forward-compatible: ignore unrecognized keys cleanly.
                }
            }
        }
        Ok(cfg)
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

fn strip_toml_comment(line: &str) -> &str {
    let mut in_double = false;
    let mut in_single = false;
    let mut escaped = false;
    for (idx, ch) in line.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if in_double {
            match ch {
                '\\' => escaped = true,
                '"' => in_double = false,
                _ => {}
            }
        } else if in_single {
            if ch == '\'' {
                in_single = false;
            }
        } else {
            match ch {
                '"' => in_double = true,
                '\'' => in_single = true,
                '#' => return &line[..idx],
                _ => {}
            }
        }
    }
    line
}

fn parse_toml_bool(raw: &str) -> Result<bool, String> {
    match raw.trim() {
        "true" => Ok(true),
        "false" => Ok(false),
        other => Err(format!("expected 'true' or 'false', got '{other}'")),
    }
}

fn parse_toml_string(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if let Some(inner) = trimmed.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
        let mut out = String::with_capacity(inner.len());
        let mut chars = inner.chars();
        while let Some(ch) = chars.next() {
            if ch == '\\' {
                match chars.next() {
                    Some('"') => out.push('"'),
                    Some('\\') => out.push('\\'),
                    Some('n') => out.push('\n'),
                    Some('r') => out.push('\r'),
                    Some('t') => out.push('\t'),
                    Some(other) => {
                        return Err(format!("unsupported escape sequence '\\{other}'"));
                    }
                    None => return Err("dangling escape at end of string".to_owned()),
                }
            } else {
                out.push(ch);
            }
        }
        return Ok(out);
    }
    if let Some(inner) = trimmed.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')) {
        return Ok(inner.to_owned());
    }
    Err("expected quoted string".to_owned())
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

    #[test]
    fn toml_roundtrip_escapes_and_comments() {
        let cfg = TelegramPluginConfig {
            version: 1,
            default_parse_mode: Some("MarkdownV2".to_owned()),
            poll_timeout_secs: 45,
            auto_typing_on_speak: false,
            approve_button_label: "Approve #1 \"Now\"".to_owned(),
            deny_button_label: "Deny\\Cancel".to_owned(),
        };
        let rendered = cfg.to_toml_string();
        let parsed = TelegramPluginConfig::from_toml_str(&rendered).unwrap();
        assert_eq!(parsed, cfg);
    }

    #[test]
    fn toml_rejects_unsupported_version() {
        let err = TelegramPluginConfig::from_toml_str("version = 99\n").unwrap_err();
        assert!(err.contains("unsupported config version 99"));
    }
}

