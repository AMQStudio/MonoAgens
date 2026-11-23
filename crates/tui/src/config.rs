//! Persistent TUI preferences (`tui.toml`), local chat store (`tui_chats.json`),
//! format-preserving `monoagens.toml` edits (with `.bak`), `0600` `secrets.toml` writes,
//! built-in plugin preset discovery, and generic dynamic `plugins/<id>.toml` editor.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use monoagens_gateway::{
    BodyTransportConfig, ConfigError, GatewayConfig, PluginConfig, PluginKind,
    default_config_document, update_config_with_backup, write_default_config,
};
use serde::{Deserialize, Serialize};
use toml_edit::{Array, DocumentMut, Item, Table, Value, value};

use crate::theme::ThemePreset;

/// Message layout mode for the main chat viewport.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageLayout {
    #[default]
    Chat,
    Stream,
}

impl MessageLayout {
    /// Display label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Chat => "Chat (Bubbles)",
            Self::Stream => "Stream (Full Width)",
        }
    }

    /// Cycles to the next layout.
    #[must_use]
    pub const fn next(self) -> Self {
        match self {
            Self::Chat => Self::Stream,
            Self::Stream => Self::Chat,
        }
    }
}

/// Sidebar visibility preference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SidebarMode {
    #[default]
    Auto,
    Open,
    Closed,
}

impl SidebarMode {
    /// Display label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Auto => "Auto (>= 120 cols)",
            Self::Open => "Open",
            Self::Closed => "Closed",
        }
    }

    /// Cycles to the next sidebar mode.
    #[must_use]
    pub const fn next(self) -> Self {
        match self {
            Self::Auto => Self::Open,
            Self::Open => Self::Closed,
            Self::Closed => Self::Auto,
        }
    }
}

/// Motion/animation intensity preference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MotionMode {
    #[default]
    Full,
    Reduced,
    Off,
}

impl MotionMode {
    /// Display label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Full => "Full",
            Self::Reduced => "Reduced",
            Self::Off => "Off",
        }
    }

    /// Cycles to the next motion mode.
    #[must_use]
    pub const fn next(self) -> Self {
        match self {
            Self::Full => Self::Reduced,
            Self::Reduced => Self::Off,
            Self::Off => Self::Full,
        }
    }
}

/// Icon set used in UI chrome (status bar, hints, sidebar); chat content keeps its emoji.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IconMode {
    /// Single-width symbols that every terminal measures the same way.
    #[default]
    Symbols,
    /// Emoji (terminals disagree about their width; stray cells may appear).
    Emoji,
}

impl IconMode {
    /// Display label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Symbols => "Symbols",
            Self::Emoji => "Emoji",
        }
    }

    /// Cycles to the next icon mode.
    #[must_use]
    pub const fn next(self) -> Self {
        match self {
            Self::Symbols => Self::Emoji,
            Self::Emoji => Self::Symbols,
        }
    }
}

/// Status-line thinking shimmer style.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShimmerMode {
    #[default]
    Classic,
    Subtle,
    Off,
}

impl ShimmerMode {
    /// Display label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Classic => "Classic",
            Self::Subtle => "Subtle",
            Self::Off => "Off",
        }
    }

    /// Cycles to the next shimmer mode.
    #[must_use]
    pub const fn next(self) -> Self {
        match self {
            Self::Classic => Self::Subtle,
            Self::Subtle => Self::Off,
            Self::Off => Self::Classic,
        }
    }
}

const fn default_true() -> bool {
    true
}

const fn default_autocomplete_limit() -> usize {
    10
}

/// TUI-only preferences persisted in `<config_dir>/tui.toml` so `monoagens.toml` stays lean.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TuiPreferences {
    #[serde(default)]
    pub theme: ThemePreset,
    #[serde(default)]
    pub message_layout: MessageLayout,
    #[serde(default)]
    pub sidebar: SidebarMode,
    #[serde(default)]
    pub motion: MotionMode,
    #[serde(default)]
    pub shimmer: ShimmerMode,
    #[serde(default)]
    pub icons: IconMode,
    #[serde(default = "default_true")]
    pub mouse_support: bool,
    #[serde(default = "default_true")]
    pub copy_on_select: bool,
    #[serde(default = "default_true")]
    pub auto_chat_title: bool,
    #[serde(default = "default_true")]
    pub show_capability_steps: bool,
    #[serde(default = "default_true")]
    pub show_turn_duration: bool,
    #[serde(default = "default_true")]
    pub show_brain_trace: bool,
    #[serde(default = "default_true")]
    pub show_token_budget: bool,
    #[serde(default = "default_true")]
    pub show_cost_estimate: bool,
    #[serde(default = "default_true")]
    pub show_valve_badge: bool,
    #[serde(default = "default_autocomplete_limit")]
    pub autocomplete_limit: usize,
}

impl Default for TuiPreferences {
    fn default() -> Self {
        Self {
            theme: ThemePreset::Titanium,
            message_layout: MessageLayout::Chat,
            sidebar: SidebarMode::Auto,
            motion: MotionMode::Full,
            shimmer: ShimmerMode::Classic,
            icons: IconMode::Symbols,
            mouse_support: true,
            copy_on_select: true,
            auto_chat_title: true,
            show_capability_steps: true,
            show_turn_duration: true,
            show_brain_trace: true,
            show_token_budget: true,
            show_cost_estimate: true,
            show_valve_badge: true,
            autocomplete_limit: 10,
        }
    }
}

impl TuiPreferences {
    /// Resolves `<config_dir>/tui.toml` alongside `monoagens.toml`.
    #[must_use]
    pub fn path_for_config(config_path: &Path) -> PathBuf {
        let dir = config_path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        dir.join("tui.toml")
    }

    /// Loads preferences from `path`, returning defaults when missing.
    pub fn load_from_path(path: &Path) -> Result<Self, String> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = std::fs::read_to_string(path)
            .map_err(|e| format!("failed to read '{}': {e}", path.display()))?;
        if raw.trim().is_empty() {
            return Ok(Self::default());
        }
        toml::from_str(&raw).map_err(|e| format!("invalid TOML in '{}': {e}", path.display()))
    }

    /// Persists preferences atomically to `path`.
    pub fn save_to_path(&self, path: &Path) -> Result<(), String> {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent).map_err(|e| {
                format!("failed to create directory '{}': {e}", parent.display())
            })?;
        }
        let body = toml::to_string_pretty(self)
            .map_err(|e| format!("failed to serialize tui.toml: {e}"))?;
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, &body)
            .map_err(|e| format!("failed to write '{}': {e}", tmp.display()))?;
        std::fs::rename(&tmp, path)
            .map_err(|e| format!("failed to persist '{}': {e}", path.display()))?;
        Ok(())
    }
}

/// Stored message in a local chat persisted across TUI restarts in `tui_chats.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredChatMessagePreview {
    pub role: String,
    pub sender: String,
    pub text_preview: String,
    pub timestamp_ms: u64,
    #[serde(default)]
    pub duration_ms: Option<u64>,
}

/// Metadata for a single local terminal chat (one persistent Mono soul across all local chats).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalChatEntry {
    pub id: String,
    pub place_id: String,
    pub title: String,
    pub title_locked: bool,
    pub turns_since_retitle: u32,
    #[serde(default, alias = "topic_keywords")]
    pub topic_ngrams: Vec<String>,
    #[serde(default)]
    pub last_topic_summary: String,
    #[serde(default)]
    pub messages: Vec<StoredChatMessagePreview>,
}

impl LocalChatEntry {
    /// Default seeded initial chat (`New Chat`) mapped to `cli:terminal`.
    #[must_use]
    pub fn direct_default() -> Self {
        Self {
            id: "direct".to_owned(),
            place_id: "cli:terminal".to_owned(),
            title: "New Chat".to_owned(),
            title_locked: false,
            turns_since_retitle: 0,
            topic_ngrams: Vec::new(),
            last_topic_summary: String::new(),
            messages: Vec::new(),
        }
    }
}

/// Local terminal chat registry persisted to `<data_dir>/tui_chats.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalChatStore {
    pub chats: Vec<LocalChatEntry>,
    #[serde(default)]
    pub active_chat_id: String,
}

impl Default for LocalChatStore {
    fn default() -> Self {
        let direct = LocalChatEntry::direct_default();
        let active_chat_id = direct.id.clone();
        Self {
            chats: vec![direct],
            active_chat_id,
        }
    }
}

impl LocalChatStore {
    /// Resolves `<data_dir>/tui_chats.json`.
    #[must_use]
    pub fn path_for_data_dir(data_dir: &Path) -> PathBuf {
        data_dir.join("tui_chats.json")
    }

    /// Loads `tui_chats.json` and ensures the default `Direct` (`cli:terminal`) chat exists.
    pub fn load_from_path(path: &Path) -> Result<Self, String> {
        let mut store = if path.exists() {
            let raw = std::fs::read_to_string(path)
                .map_err(|e| format!("failed to read '{}': {e}", path.display()))?;
            if raw.trim().is_empty() {
                Self::default()
            } else {
                serde_json::from_str::<Self>(&raw)
                    .map_err(|e| format!("invalid JSON in '{}': {e}", path.display()))?
            }
        } else {
            Self::default()
        };
        store.ensure_direct_seeded();
        Ok(store)
    }

    /// Ensures the initial default chat (`place_id = "cli:terminal"`) is always present.
    pub fn ensure_direct_seeded(&mut self) {
        if !self.chats.iter().any(|c| c.place_id == "cli:terminal") {
            self.chats.insert(0, LocalChatEntry::direct_default());
        }
        for chat in &mut self.chats {
            if !chat.title_locked && chat.title == "Direct" {
                chat.title = "New Chat".to_owned();
            }
        }
        if self.active_chat_id.is_empty()
            || !self.chats.iter().any(|c| c.id == self.active_chat_id)
        {
            self.active_chat_id = self.chats[0].id.clone();
        }
    }

    /// Persists the chat store to `path`.
    pub fn save_to_path(&self, path: &Path) -> Result<(), String> {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent).map_err(|e| {
                format!("failed to create directory '{}': {e}", parent.display())
            })?;
        }
        let body = serde_json::to_string_pretty(self)
            .map_err(|e| format!("failed to serialize tui_chats.json: {e}"))?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, &body)
            .map_err(|e| format!("failed to write '{}': {e}", tmp.display()))?;
        std::fs::rename(&tmp, path)
            .map_err(|e| format!("failed to persist '{}': {e}", path.display()))?;
        Ok(())
    }

    /// Returns a reference to the active chat.
    #[must_use]
    pub fn active_chat(&self) -> &LocalChatEntry {
        self.chats
            .iter()
            .find(|c| c.id == self.active_chat_id)
            .unwrap_or(&self.chats[0])
    }

    /// Returns a mutable reference to the active chat.
    pub fn active_chat_mut(&mut self) -> &mut LocalChatEntry {
        let idx = self
            .chats
            .iter()
            .position(|c| c.id == self.active_chat_id)
            .unwrap_or(0);
        &mut self.chats[idx]
    }

    /// Creates a new local chat (mapped to `cli:chat:<slug>`) and selects it.
    pub fn create_chat(&mut self, name: Option<&str>) -> &LocalChatEntry {
        let base_title = name
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("New Chat");
        let slug_base = slugify(base_title);
        let mut idx = 1usize;
        let mut id = slug_base.clone();
        while self.chats.iter().any(|c| c.id == id) {
            idx += 1;
            id = format!("{slug_base}-{idx}");
        }
        let locked = name.is_some_and(|s| !s.trim().is_empty());
        let entry = LocalChatEntry {
            id: id.clone(),
            place_id: format!("cli:chat:{id}"),
            title: base_title.to_owned(),
            title_locked: locked,
            turns_since_retitle: 0,
            topic_ngrams: Vec::new(),
            last_topic_summary: String::new(),
            messages: Vec::new(),
        };
        self.chats.push(entry);
        self.active_chat_id = id;
        self.active_chat()
    }

    /// Manually renames a chat and sets `title_locked = true` so auto-titling never overwrites it.
    pub fn rename_chat(&mut self, chat_id: &str, new_title: &str) -> bool {
        let trimmed = new_title.trim();
        if trimmed.is_empty() {
            return false;
        }
        if let Some(chat) = self.chats.iter_mut().find(|c| c.id == chat_id) {
            chat.title = trimmed.to_owned();
            chat.title_locked = true;
            return true;
        }
        false
    }

    /// Appends a message to `chat_id` for persistence across restarts (keeping up to 500 messages).
    pub fn append_message(&mut self, chat_id: &str, msg: StoredChatMessagePreview) {
        const MAX_PERSISTED_MESSAGES_PER_CHAT: usize = 500;
        if let Some(chat) = self.chats.iter_mut().find(|c| c.id == chat_id) {
            chat.messages.push(msg);
            if chat.messages.len() > MAX_PERSISTED_MESSAGES_PER_CHAT {
                let overflow = chat.messages.len() - MAX_PERSISTED_MESSAGES_PER_CHAT;
                chat.messages.drain(..overflow);
            }
        }
    }

    /// Clears all persisted messages for `chat_id`.
    pub fn clear_messages(&mut self, chat_id: &str) {
        if let Some(chat) = self.chats.iter_mut().find(|c| c.id == chat_id) {
            chat.messages.clear();
        }
    }

    /// Deletes a chat when more than one local chat exists, switching `active_chat_id` cleanly.
    pub fn delete_chat(&mut self, chat_id: &str) -> bool {
        if self.chats.len() <= 1 {
            return false;
        }
        let Some(idx) = self.chats.iter().position(|c| c.id == chat_id) else {
            return false;
        };
        self.chats.remove(idx);
        if self.active_chat_id == chat_id {
            let next_idx = idx.min(self.chats.len().saturating_sub(1));
            self.active_chat_id = self.chats[next_idx].id.clone();
        }
        true
    }
}

fn slugify(raw: &str) -> String {
    let mut out = String::new();
    let mut prev_dash = false;
    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            prev_dash = false;
        } else if !prev_dash && !out.is_empty() {
            out.push('-');
            prev_dash = true;
        }
    }
    let trimmed = out.trim_matches('-');
    if trimmed.is_empty() {
        "chat".to_owned()
    } else {
        trimmed.to_owned()
    }
}

/// Ensures `monoagens.toml` exists on disk, then applies `updater` using `toml_edit` with `.bak`.
pub fn mutate_monoagens_toml<F>(
    config_path: &Path,
    secrets_path: &Path,
    updater: F,
) -> Result<(), String>
where
    F: FnOnce(&mut DocumentMut) -> Result<(), ConfigError>,
{
    if !config_path.exists() {
        write_default_config(config_path, &GatewayConfig::default()).map_err(|e| e.to_string())?;
    }
    update_config_with_backup(config_path, secrets_path, updater).map_err(|e| e.to_string())
}

/// Sets or clears a field at `key_path` (e.g. `["valve", "mode"]`) in `monoagens.toml` via `toml_edit` + `.bak`.
pub fn save_monoagens_toml_field(
    config_path: &Path,
    secrets_path: &Path,
    key_path: &[&str],
    new_item: Option<Item>,
) -> Result<(), String> {
    if key_path.is_empty() {
        return Err("empty key path".to_owned());
    }
    mutate_monoagens_toml(config_path, secrets_path, |doc| {
        if key_path.len() == 1 {
            match new_item {
                Some(item) => {
                    doc.insert(key_path[0], item);
                }
                None => {
                    doc.remove(key_path[0]);
                }
            }
            return Ok(());
        }

        let mut cursor = doc.as_table_mut();
        for &seg in &key_path[..key_path.len() - 1] {
            let entry = cursor.entry(seg).or_insert_with(|| {
                let mut t = Table::new();
                t.set_implicit(false);
                Item::Table(t)
            });
            cursor = entry.as_table_mut().ok_or_else(|| ConfigError::InvalidType {
                key: seg.to_owned(),
                expected: "TOML table",
            })?;
        }
        let leaf = key_path[key_path.len() - 1];
        match new_item {
            Some(item) => {
                cursor.insert(leaf, item);
            }
            None => {
                cursor.remove(leaf);
            }
        }
        Ok(())
    })
}

/// Writes `default_config_document` if missing, or updates `[model]`/`[plugins]` during setup wizard.
pub fn write_or_update_gateway_config(
    config_path: &Path,
    secrets_path: &Path,
    cfg: &GatewayConfig,
) -> Result<(), String> {
    if !config_path.exists() {
        return write_default_config(config_path, cfg).map_err(|e| e.to_string());
    }
    let doc_text = default_config_document(cfg).to_string();
    let _parsed: DocumentMut = doc_text
        .parse()
        .map_err(|e| format!("failed to build config document: {e}"))?;
    update_config_with_backup(config_path, secrets_path, |doc| {
        let model_tbl = doc
            .entry("model")
            .or_insert_with(|| Item::Table(Table::new()))
            .as_table_mut()
            .ok_or_else(|| ConfigError::InvalidType {
                key: "model".to_owned(),
                expected: "table",
            })?;
        model_tbl.insert("provider", value(cfg.model.provider.as_str()));
        model_tbl.insert("model", value(cfg.model.model.as_str()));
        model_tbl.insert("daily_token_budget", value(cfg.model.daily_token_budget as i64));
        Ok(())
    })
    .map_err(|e| e.to_string())
}

fn read_or_init_secrets_doc(path: &Path) -> Result<DocumentMut, String> {
    if !path.exists() {
        return Ok(DocumentMut::new());
    }
    let raw = std::fs::read_to_string(path)
        .map_err(|e| format!("failed to read secrets file '{}': {e}", path.display()))?;
    if raw.trim().is_empty() {
        return Ok(DocumentMut::new());
    }
    if raw.trim_start().starts_with('{') {
        return Err(format!(
            "secrets file '{}' is JSON-formatted; convert it to TOML to manage keys",
            path.display()
        ));
    }
    raw.parse::<DocumentMut>()
        .map_err(|e| format!("invalid TOML in secrets file '{}': {e}", path.display()))
}

fn write_secrets_doc(path: &Path, doc: &DocumentMut) -> Result<(), String> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).map_err(|e| {
            format!(
                "failed to create directory '{}' for secrets file: {e}",
                parent.display()
            )
        })?;
    }
    std::fs::write(path, doc.to_string())
        .map_err(|e| format!("failed to write secrets file '{}': {e}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).map_err(|e| {
            format!(
                "failed to set 0600 permissions on '{}': {e}",
                path.display()
            )
        })?;
    }
    Ok(())
}

fn validate_secret_key(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("secret key must not be empty".to_owned());
    }
    if trimmed.contains(char::is_whitespace) || trimmed.contains('.') {
        return Err(format!(
            "invalid secret key '{trimmed}' (must not contain whitespace or '.')"
        ));
    }
    Ok(trimmed.to_owned())
}

/// Saves a top-level key into `secrets.toml` with `0600` Unix permissions (never prints values).
pub fn save_secret_key(secrets_path: &Path, key: &str, secret_val: &str) -> Result<(), String> {
    let key = validate_secret_key(key)?;
    if key == "env" {
        return Err("top-level key 'env' is reserved for [env] plugin secrets".to_owned());
    }
    let val = secret_val.trim();
    if val.is_empty() {
        return Err("secret value must not be empty".to_owned());
    }
    let mut doc = read_or_init_secrets_doc(secrets_path)?;
    doc[&key] = value(val);
    write_secrets_doc(secrets_path, &doc)
}

/// Saves an `[env]` secret key into `secrets.toml` with `0600` Unix permissions (never prints values).
pub fn save_secret_env(secrets_path: &Path, key: &str, secret_val: &str) -> Result<(), String> {
    let key = validate_secret_key(key)?;
    let val = secret_val.trim();
    if val.is_empty() {
        return Err("secret value must not be empty".to_owned());
    }
    let mut doc = read_or_init_secrets_doc(secrets_path)?;
    let env_item = doc.entry("env").or_insert_with(|| Item::Table(Table::new()));
    let env_tbl = env_item
        .as_table_like_mut()
        .ok_or_else(|| "secret section '[env]' must be a TOML table".to_owned())?;
    env_tbl.insert(&key, value(val));
    write_secrets_doc(secrets_path, &doc)
}

/// Removes an `[env]` key from `secrets.toml` with `0600` Unix permissions.
pub fn remove_secret_env(secrets_path: &Path, key: &str) -> Result<(), String> {
    let key = validate_secret_key(key)?;
    if !secrets_path.exists() {
        return Ok(());
    }
    let mut doc = read_or_init_secrets_doc(secrets_path)?;
    if let Some(env_tbl) = doc.get_mut("env").and_then(Item::as_table_like_mut) {
        env_tbl.remove(&key);
        if env_tbl.is_empty() {
            doc.as_table_mut().remove("env");
        }
    }
    write_secrets_doc(secrets_path, &doc)
}

/// Lists non-secret key names `(top_level_keys, env_keys)` from `secrets.toml` without exposing values.
pub fn list_secret_keys(secrets_path: &Path) -> Result<(Vec<String>, Vec<String>), String> {
    if !secrets_path.exists() {
        return Ok((Vec::new(), Vec::new()));
    }
    let doc = read_or_init_secrets_doc(secrets_path)?;
    let mut top = Vec::new();
    for (k, item) in doc.as_table().iter() {
        if k != "env" && !item.is_none() {
            top.push(k.to_owned());
        }
    }
    let mut env_keys = Vec::new();
    if let Some(env_tbl) = doc.get("env").and_then(Item::as_table_like) {
        for (k, item) in env_tbl.iter() {
            if !item.is_none() {
                env_keys.push(k.to_owned());
            }
        }
    }
    Ok((top, env_keys))
}

/// Supported scalar/array value variants in [`DynamicTomlDoc`] for `<config_dir>/plugins/<id>.toml`.
#[derive(Debug, Clone, PartialEq)]
pub enum DynamicTomlValue {
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    StrArray(Vec<String>),
}

impl DynamicTomlValue {
    /// Human-readable short representation for settings rows.
    #[must_use]
    pub fn display_compact(&self) -> String {
        match self {
            Self::Bool(b) => b.to_string(),
            Self::Int(i) => i.to_string(),
            Self::Float(f) => format!("{f:.3}"),
            Self::Str(s) => s.clone(),
            Self::StrArray(arr) => format!("[{}]", arr.join(", ")),
        }
    }

    fn to_toml_value(&self) -> Value {
        match self {
            Self::Bool(b) => Value::from(*b),
            Self::Int(i) => Value::from(*i),
            Self::Float(f) => Value::from(*f),
            Self::Str(s) => Value::from(s.as_str()),
            Self::StrArray(items) => {
                let mut arr = Array::new();
                for item in items {
                    arr.push(item.as_str());
                }
                Value::Array(arr)
            }
        }
    }
}

/// Flattened key/value entry inside [`DynamicTomlDoc`].
#[derive(Debug, Clone, PartialEq)]
pub struct DynamicTomlEntry {
    pub key_path: String,
    pub value: DynamicTomlValue,
}

/// Generic format-preserving TOML editor for arbitrary `<config_dir>/plugins/<id>.toml` files.
#[derive(Debug, Clone)]
pub struct DynamicTomlDoc {
    pub path: PathBuf,
    doc: DocumentMut,
}

impl DynamicTomlDoc {
    /// Loads an existing plugin TOML file or initializes an empty one if not created yet.
    pub fn load_or_init(path: &Path) -> Result<Self, String> {
        let doc = if path.exists() {
            let raw = std::fs::read_to_string(path)
                .map_err(|e| format!("failed to read '{}': {e}", path.display()))?;
            if raw.trim().is_empty() {
                DocumentMut::new()
            } else {
                raw.parse::<DocumentMut>()
                    .map_err(|e| format!("invalid TOML in '{}': {e}", path.display()))?
            }
        } else {
            DocumentMut::new()
        };
        Ok(Self {
            path: path.to_path_buf(),
            doc,
        })
    }

    /// Lists all flattened editable leaf entries in deterministic order.
    #[must_use]
    pub fn entries(&self) -> Vec<DynamicTomlEntry> {
        let mut out = Vec::new();
        collect_dynamic_entries(self.doc.as_table(), "", &mut out);
        out
    }

    /// Sets or inserts a value at a dot-separated `key_path` (e.g. `session.auto_reconnect` or `enabled`).
    pub fn set_entry(&mut self, key_path: &str, new_val: DynamicTomlValue) -> Result<(), String> {
        let segments: Vec<&str> = key_path
            .split('.')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect();
        if segments.is_empty() {
            return Err("key path must not be empty".to_owned());
        }
        let mut tbl = self.doc.as_table_mut();
        for &seg in &segments[..segments.len() - 1] {
            let entry = tbl.entry(seg).or_insert_with(|| Item::Table(Table::new()));
            tbl = entry
                .as_table_mut()
                .ok_or_else(|| format!("'{seg}' is not a TOML table"))?;
        }
        let leaf = segments[segments.len() - 1];
        let toml_val = new_val.to_toml_value();
        if let Some(existing) = tbl.get_mut(leaf).and_then(Item::as_value_mut) {
            let decor = existing.decor().clone();
            let mut updated = toml_val;
            *updated.decor_mut() = decor;
            *existing = updated;
        } else {
            tbl.insert(leaf, Item::Value(toml_val));
        }
        Ok(())
    }

    /// Removes a key at `key_path`.
    pub fn remove_entry(&mut self, key_path: &str) -> bool {
        let segments: Vec<&str> = key_path
            .split('.')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect();
        if segments.is_empty() {
            return false;
        }
        let mut tbl = self.doc.as_table_mut();
        for &seg in &segments[..segments.len() - 1] {
            let Some(next) = tbl.get_mut(seg).and_then(Item::as_table_mut) else {
                return false;
            };
            tbl = next;
        }
        tbl.remove(segments[segments.len() - 1]).is_some()
    }

    /// Persists the TOML document atomically, creating parent directories if needed.
    pub fn save(&self) -> Result<(), String> {
        if let Some(parent) = self.path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent).map_err(|e| {
                format!("failed to create directory '{}': {e}", parent.display())
            })?;
        }
        let body = self.doc.to_string();
        let tmp = self.path.with_extension("toml.tmp");
        std::fs::write(&tmp, &body)
            .map_err(|e| format!("failed to write '{}': {e}", tmp.display()))?;
        std::fs::rename(&tmp, &self.path)
            .map_err(|e| format!("failed to persist '{}': {e}", self.path.display()))?;
        Ok(())
    }
}

fn collect_dynamic_entries(
    table: &Table,
    prefix: &str,
    out: &mut Vec<DynamicTomlEntry>,
) {
    for (k, item) in table.iter() {
        let full = if prefix.is_empty() {
            k.to_owned()
        } else {
            format!("{prefix}.{k}")
        };
        match item {
            Item::Table(sub) => collect_dynamic_entries(sub, &full, out),
            Item::Value(v) => {
                if let Some(parsed) = parse_toml_value(v) {
                    out.push(DynamicTomlEntry {
                        key_path: full,
                        value: parsed,
                    });
                } else if let Value::InlineTable(it) = v {
                    for (ik, iv) in it.iter() {
                        if let Some(parsed) = parse_toml_value(iv) {
                            out.push(DynamicTomlEntry {
                                key_path: format!("{full}.{ik}"),
                                value: parsed,
                            });
                        }
                    }
                }
            }
            Item::ArrayOfTables(_) | Item::None => {}
        }
    }
}

fn parse_toml_value(v: &Value) -> Option<DynamicTomlValue> {
    match v {
        Value::Boolean(b) => Some(DynamicTomlValue::Bool(*b.value())),
        Value::Integer(i) => Some(DynamicTomlValue::Int(*i.value())),
        Value::Float(f) => Some(DynamicTomlValue::Float(*f.value())),
        Value::String(s) => Some(DynamicTomlValue::Str(s.value().clone())),
        Value::Array(arr) => {
            let items: Vec<String> = arr
                .iter()
                .map(|elem| match elem {
                    Value::String(s) => s.value().clone(),
                    other => other.to_string().trim().to_owned(),
                })
                .collect();
            Some(DynamicTomlValue::StrArray(items))
        }
        _ => None,
    }
}

/// Parses a user-entered string into a [`DynamicTomlValue`] matching `hint` when possible.
#[must_use]
pub fn parse_dynamic_input(raw: &str, hint: Option<&DynamicTomlValue>) -> DynamicTomlValue {
    let trimmed = raw.trim();
    match hint {
        Some(DynamicTomlValue::Bool(_)) => {
            let b = trimmed.eq_ignore_ascii_case("true")
                || trimmed == "1"
                || trimmed.eq_ignore_ascii_case("yes");
            DynamicTomlValue::Bool(b)
        }
        Some(DynamicTomlValue::Int(_)) => trimmed
            .parse::<i64>()
            .map_or_else(|_| DynamicTomlValue::Str(trimmed.to_owned()), DynamicTomlValue::Int),
        Some(DynamicTomlValue::Float(_)) => trimmed
            .parse::<f64>()
            .map_or_else(|_| DynamicTomlValue::Str(trimmed.to_owned()), DynamicTomlValue::Float),
        Some(DynamicTomlValue::StrArray(_)) => {
            let inner = trimmed
                .strip_prefix('[')
                .and_then(|s| s.strip_suffix(']'))
                .unwrap_or(trimmed);
            let items = inner
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(ToOwned::to_owned)
                .collect();
            DynamicTomlValue::StrArray(items)
        }
        Some(DynamicTomlValue::Str(_)) => DynamicTomlValue::Str(trimmed.to_owned()),
        None => {
            if trimmed.eq_ignore_ascii_case("true") {
                DynamicTomlValue::Bool(true)
            } else if trimmed.eq_ignore_ascii_case("false") {
                DynamicTomlValue::Bool(false)
            } else if let Ok(i) = trimmed.parse::<i64>() {
                DynamicTomlValue::Int(i)
            } else if let Ok(f) = trimmed.parse::<f64>() {
                DynamicTomlValue::Float(f)
            } else if trimmed.starts_with('[') && trimmed.ends_with(']') {
                let inner = &trimmed[1..trimmed.len() - 1];
                let items = inner
                    .split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(ToOwned::to_owned)
                    .collect();
                DynamicTomlValue::StrArray(items)
            } else {
                DynamicTomlValue::Str(trimmed.to_owned())
            }
        }
    }
}

/// Built-in plugin catalog preset matching `monoagens plugin`.
#[derive(Debug, Clone, Copy)]
pub struct BuiltinPluginPreset {
    pub id: &'static str,
    pub binary: &'static str,
    pub kind: PluginKind,
    pub default_args: &'static [&'static str],
    pub default_optional_deps: &'static [&'static str],
}

/// Built-in MonoAgens plugins discoverable in `/settings -> Plugins` and Setup Wizard.
pub const BUILTIN_PLUGIN_PRESETS: &[BuiltinPluginPreset] = &[
    BuiltinPluginPreset {
        id: "telegram",
        binary: "body-telegram",
        kind: PluginKind::Adapter,
        default_args: &["--token-env", "TELEGRAM_BOT_TOKEN"],
        default_optional_deps: &[],
    },
    BuiltinPluginPreset {
        id: "whatsapp",
        binary: "body-whatsapp",
        kind: PluginKind::Adapter,
        default_args: &["--live"],
        default_optional_deps: &[],
    },
    BuiltinPluginPreset {
        id: "cli",
        binary: "body-cli",
        kind: PluginKind::Adapter,
        default_args: &[],
        default_optional_deps: &[],
    },
    BuiltinPluginPreset {
        id: "voice",
        binary: "body-voice",
        kind: PluginKind::Adapter,
        default_args: &[],
        default_optional_deps: &[],
    },
    BuiltinPluginPreset {
        id: "sim-actuator",
        binary: "body-sim-actuator",
        kind: PluginKind::Adapter,
        default_args: &[],
        default_optional_deps: &[],
    },
    BuiltinPluginPreset {
        id: "mcp-bridge",
        binary: "body-mcp-bridge",
        kind: PluginKind::Capability,
        default_args: &[],
        default_optional_deps: &[],
    },
    BuiltinPluginPreset {
        id: "a2a-bridge",
        binary: "plugin-a2a-bridge",
        kind: PluginKind::Capability,
        default_args: &[],
        default_optional_deps: &[],
    },
    BuiltinPluginPreset {
        id: "ros2-bridge",
        binary: "plugin-ros2-bridge",
        kind: PluginKind::Adapter,
        default_args: &[],
        default_optional_deps: &[],
    },
    BuiltinPluginPreset {
        id: "web-search",
        binary: "plugin-web-search",
        kind: PluginKind::Capability,
        default_args: &[],
        default_optional_deps: &[],
    },
    BuiltinPluginPreset {
        id: "workspace",
        binary: "plugin-workspace",
        kind: PluginKind::Capability,
        default_args: &[],
        default_optional_deps: &["rbac"],
    },
    BuiltinPluginPreset {
        id: "pdf-reader",
        binary: "plugin-pdf-reader",
        kind: PluginKind::Capability,
        default_args: &[],
        default_optional_deps: &[],
    },
    BuiltinPluginPreset {
        id: "soul-tuning",
        binary: "plugin-soul-tuning",
        kind: PluginKind::Capability,
        default_args: &[],
        default_optional_deps: &[],
    },
];

/// Looks up a built-in preset by id.
#[must_use]
pub fn find_builtin_preset(id: &str) -> Option<&'static BuiltinPluginPreset> {
    BUILTIN_PLUGIN_PRESETS.iter().find(|p| p.id == id)
}

fn find_workspace_root() -> Option<PathBuf> {
    if let Ok(cwd) = std::env::current_dir() {
        for dir in cwd.ancestors() {
            let cargo_toml = dir.join("Cargo.toml");
            if cargo_toml.is_file()
                && std::fs::read_to_string(&cargo_toml)
                    .is_ok_and(|contents| contents.contains("[workspace]"))
            {
                return Some(dir.to_path_buf());
            }
        }
    }
    let manifest_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    if manifest_root.join("Cargo.toml").is_file() {
        return manifest_root.canonicalize().ok().or(Some(manifest_root));
    }
    None
}

/// Discovers the binary path for a built-in plugin preset near `current_exe`, `target/`, or `PATH`.
#[must_use]
pub fn discover_builtin_binary(preset: &BuiltinPluginPreset) -> Option<PathBuf> {
    let file_name = format!("{}{}", preset.binary, std::env::consts::EXE_SUFFIX);
    let mut candidates = Vec::new();

    if let Ok(current_exe) = std::env::current_exe()
        && let Some(exe_dir) = current_exe.parent()
    {
        candidates.push(exe_dir.join(&file_name));
        if exe_dir.file_name().is_some_and(|n| n == "deps")
            && let Some(parent_dir) = exe_dir.parent()
        {
            candidates.push(parent_dir.join(&file_name));
        }
    }

    if let Some(ws) = find_workspace_root() {
        candidates.push(ws.join("target/release").join(&file_name));
        candidates.push(ws.join("target/debug").join(&file_name));
        if preset.id == "whatsapp" {
            candidates.push(ws.join("plugins/whatsapp/target/release").join(&file_name));
            candidates.push(ws.join("plugins/whatsapp/target/debug").join(&file_name));
        }
    }

    if let Some(path_env) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path_env) {
            candidates.push(dir.join(&file_name));
        }
    }

    candidates.into_iter().find(|p| p.is_file())
}

fn build_plugin_table(spec: &PluginConfig) -> Table {
    let mut tbl = Table::new();
    tbl["enabled"] = value(spec.enabled);
    tbl["kind"] = value(spec.kind.as_str());
    match &spec.transport {
        BodyTransportConfig::Stdio { command, args } => {
            tbl["transport"] = value("stdio");
            tbl["command"] = value(command.as_str());
            if !args.is_empty() {
                let mut arr = Array::new();
                for arg in args {
                    arr.push(arg.as_str());
                }
                tbl["args"] = value(arr);
            }
        }
        BodyTransportConfig::UnixSocket { path } => {
            tbl["transport"] = value("unix_socket");
            tbl["socket_path"] = value(path.display().to_string());
        }
    }
    if !spec.dependencies.is_empty() {
        let mut arr = Array::new();
        for dep in &spec.dependencies {
            arr.push(dep.as_str());
        }
        tbl["dependencies"] = value(arr);
    }
    if !spec.optional_dependencies.is_empty() {
        let mut arr = Array::new();
        for dep in &spec.optional_dependencies {
            arr.push(dep.as_str());
        }
        tbl["optional_dependencies"] = value(arr);
    }
    tbl
}

/// Installs a built-in plugin preset into `[plugins.<id>]` in `monoagens.toml` (with `.bak`).
pub fn install_builtin_plugin(
    config_path: &Path,
    secrets_path: &Path,
    plugin_id: &str,
) -> Result<PluginConfig, String> {
    let preset = find_builtin_preset(plugin_id)
        .ok_or_else(|| format!("unknown built-in plugin '{plugin_id}'"))?;
    let cmd = discover_builtin_binary(preset)
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| preset.binary.to_owned());
    let spec = PluginConfig {
        id: preset.id.to_owned(),
        enabled: true,
        kind: preset.kind,
        transport: BodyTransportConfig::Stdio {
            command: cmd,
            args: preset.default_args.iter().map(|s| (*s).to_owned()).collect(),
        },
        dependencies: Vec::new(),
        optional_dependencies: preset
            .default_optional_deps
            .iter()
            .map(|s| (*s).to_owned())
            .collect(),
    };
    let spec_clone = spec.clone();
    mutate_monoagens_toml(config_path, secrets_path, |doc| {
        let plugins_item = doc.entry("plugins").or_insert_with(|| {
            let mut t = Table::new();
            t.set_implicit(true);
            Item::Table(t)
        });
        let plugins_tbl = plugins_item
            .as_table_mut()
            .ok_or_else(|| ConfigError::InvalidType {
                key: "plugins".to_owned(),
                expected: "table [plugins]",
            })?;
        plugins_tbl.insert(&spec_clone.id, Item::Table(build_plugin_table(&spec_clone)));
        Ok(())
    })?;
    Ok(spec)
}

/// Sets `enabled = true/false` on `[plugins.<id>]` in `monoagens.toml` (with `.bak`).
pub fn set_plugin_enabled(
    config_path: &Path,
    secrets_path: &Path,
    plugin_id: &str,
    enabled: bool,
) -> Result<(), String> {
    let id = plugin_id.to_owned();
    mutate_monoagens_toml(config_path, secrets_path, |doc| {
        let plugins_tbl = doc
            .get_mut("plugins")
            .and_then(Item::as_table_mut)
            .ok_or_else(|| ConfigError::MissingField {
                key: format!("plugins.{id}"),
            })?;
        let tbl = plugins_tbl
            .get_mut(&id)
            .and_then(Item::as_table_like_mut)
            .ok_or_else(|| ConfigError::MissingField {
                key: format!("plugins.{id}"),
            })?;
        tbl.insert("enabled", value(enabled));
        Ok(())
    })
}

/// Removes `[plugins.<id>]` from `monoagens.toml` (with `.bak`).
pub fn remove_plugin_from_config(
    config_path: &Path,
    secrets_path: &Path,
    plugin_id: &str,
) -> Result<(), String> {
    let id = plugin_id.to_owned();
    mutate_monoagens_toml(config_path, secrets_path, |doc| {
        if let Some(plugins_tbl) = doc.get_mut("plugins").and_then(Item::as_table_mut) {
            plugins_tbl.remove(&id);
            if plugins_tbl.is_empty() {
                plugins_tbl.set_implicit(true);
            }
        }
        Ok(())
    })
}

/// Resolves config directory containing `monoagens.toml`.
#[must_use]
pub fn config_dir_from_path(config_path: &Path) -> PathBuf {
    config_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
}

/// Collects all unique plugin IDs across configured plugins and built-in presets.
#[must_use]
pub fn merged_plugin_ids(configured: &[PluginConfig]) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut ids = Vec::new();
    for p in configured {
        if seen.insert(p.id.clone()) {
            ids.push(p.id.clone());
        }
    }
    for preset in BUILTIN_PLUGIN_PRESETS {
        if seen.insert(preset.id.to_owned()) {
            ids.push(preset.id.to_owned());
        }
    }
    ids
}
