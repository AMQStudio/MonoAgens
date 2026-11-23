//! Friendly capability labels and best-effort on-disk capability metadata cache.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use monoagens_protocol::{CapabilityDescriptor, sort_capabilities};
use serde::{Deserialize, Serialize};

/// File name of the capability descriptor cache inside `<data_dir>`.
pub const CAPABILITY_CACHE_FILENAME: &str = "capability_cache.json";

static TEMP_SEQ: AtomicU64 = AtomicU64::new(0);

/// Returns a human-friendly display label for a capability identifier.
///
/// Built-in capabilities use a curated label table; unknown capabilities fall back to a
/// deterministic Title-Case rendering of their trailing dot-separated segment.
#[must_use]
pub fn friendly_capability_label(id: &str) -> String {
    if let Some(label) = curated_capability_label(id) {
        return label.to_owned();
    }
    humanize_suffix(id)
}

pub(crate) fn friendly_plugin_label(id: &str) -> String {
    match id {
        "whatsapp" => "WhatsApp".to_owned(),
        "telegram" => "Telegram".to_owned(),
        "discord" => "Discord".to_owned(),
        "slack" => "Slack".to_owned(),
        "workspace" => "Workspace".to_owned(),
        "web-fetch" => "Web Fetch".to_owned(),
        "pdf-reader" => "PDF Reader".to_owned(),
        "soul-tuning" => "Soul Tuning".to_owned(),
        _ => humanize_words(id),
    }
}

fn curated_capability_label(id: &str) -> Option<&'static str> {
    Some(match id {
        "agent.converse" => "Allow Conversation",
        "chat.speak" => "Send Messages",
        "chat.react" => "React to Messages",
        "chat.delete" => "Delete Messages",
        "chat.edit" => "Edit Messages",
        "chat.mark_read" => "Mark as Read",
        "chat.send_media" => "Send Media",
        "chat.send_poll" => "Send Polls",
        "chat.send_sticker" => "Send Stickers",
        "chat.typing" => "Typing Indicator",
        "approval.prompt" => "Approval Prompts",
        "web.search" => "Web Search",
        "web.fetch" => "Read Web Pages",
        "workspace.read" => "Read Files",
        "workspace.write" => "Write Files",
        "workspace.edit" => "Edit Files",
        "workspace.glob" => "Find Files",
        "workspace.grep" => "Search Files",
        "workspace.run" => "Run Commands",
        "pdf.index" => "Index PDFs",
        "pdf.list" => "List PDFs",
        "pdf.read_page" => "Read PDF Pages",
        "pdf.search" => "Search PDFs",
        "pdf.delete" => "Delete PDFs",
        "soul_tuning.get_mode" => "View Soul Mode",
        "soul_tuning.list_modes" => "List Soul Modes",
        "soul_tuning.set_mode" => "Change Soul Mode",
        _ => return None,
    })
}

fn humanize_suffix(id: &str) -> String {
    let trimmed = id.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let suffix = trimmed.rsplit('.').find(|s| !s.is_empty()).unwrap_or(trimmed);
    let label = humanize_words(suffix);
    if label.is_empty() {
        trimmed.to_owned()
    } else {
        label
    }
}

fn humanize_words(raw: &str) -> String {
    let words: Vec<String> = raw
        .split(|c: char| !c.is_alphanumeric())
        .filter(|part| !part.is_empty())
        .map(title_case_word)
        .collect();
    if words.is_empty() {
        raw.trim().to_owned()
    } else {
        words.join(" ")
    }
}

fn title_case_word(word: &str) -> String {
    let mut chars = word.chars();
    let Some(first) = chars.next() else {
        return String::new();
    };
    let mut out = String::with_capacity(word.len());
    out.extend(first.to_uppercase());
    out.extend(chars.flat_map(char::to_lowercase));
    out
}

/// Per-plugin cache of last-discovered [`CapabilityDescriptor`]s persisted to
/// `<data_dir>/capability_cache.json`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CapabilityCache {
    pub plugins: BTreeMap<String, Vec<CapabilityDescriptor>>,
}

impl CapabilityCache {
    /// Path of the capability cache file inside `data_dir`.
    #[must_use]
    pub fn cache_path(data_dir: &Path) -> PathBuf {
        data_dir.join(CAPABILITY_CACHE_FILENAME)
    }

    /// Loads `<data_dir>/capability_cache.json`, returning an empty cache on missing or invalid data.
    #[must_use]
    pub fn load(data_dir: &Path) -> Self {
        let path = Self::cache_path(data_dir);
        let Ok(raw) = std::fs::read_to_string(path) else {
            return Self::default();
        };
        serde_json::from_str(&raw).unwrap_or_default()
    }

    /// Returns cached capabilities for `plugin_id`, if any.
    #[must_use]
    pub fn get(&self, plugin_id: &str) -> Option<&[CapabilityDescriptor]> {
        self.plugins.get(plugin_id).map(Vec::as_slice)
    }

    /// Replaces the cached capabilities for `plugin_id` (sorted deterministically) and returns
    /// `true` when the entry changed.
    pub fn update_plugin(&mut self, plugin_id: &str, mut caps: Vec<CapabilityDescriptor>) -> bool {
        sort_capabilities(&mut caps);
        if self.plugins.get(plugin_id) == Some(&caps) {
            return false;
        }
        self.plugins.insert(plugin_id.to_owned(), caps);
        true
    }

    /// Best-effort atomic write to `<data_dir>/capability_cache.json` (temp file + rename).
    /// Never panics or fails runtime operations on I/O errors.
    pub fn save(&self, data_dir: &Path) {
        let Ok(json) = serde_json::to_vec_pretty(self) else {
            return;
        };
        if std::fs::create_dir_all(data_dir).is_err() {
            return;
        }
        let seq = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
        let tmp = data_dir.join(format!(
            ".capability_cache.{}.{seq}.tmp",
            std::process::id()
        ));
        let write_ok = (|| -> std::io::Result<()> {
            let mut file = std::fs::File::create(&tmp)?;
            file.write_all(&json)?;
            file.sync_all()?;
            drop(file);
            std::fs::rename(&tmp, Self::cache_path(data_dir))
        })();
        if write_ok.is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use monoagens_protocol::{CapabilityFlags, CapabilityId, CapabilitySchemaHash, PlaceId};

    #[test]
    fn curated_and_fallback_capability_labels() {
        assert_eq!(friendly_capability_label("agent.converse"), "Allow Conversation");
        assert_eq!(friendly_capability_label("chat.speak"), "Send Messages");
        assert_eq!(friendly_capability_label("approval.prompt"), "Approval Prompts");
        assert_eq!(friendly_capability_label("soul_tuning.set_mode"), "Change Soul Mode");
        assert_eq!(friendly_capability_label("mcp.github.create_issue"), "Create Issue");
        assert_eq!(friendly_capability_label("custom.foo_bar"), "Foo Bar");
        assert_eq!(friendly_capability_label("single"), "Single");
    }

    #[test]
    fn capability_cache_roundtrip_and_invalid_resilience() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(CapabilityCache::load(dir.path()).plugins.is_empty());

        let mut cache = CapabilityCache::default();
        let cap = CapabilityDescriptor {
            id: CapabilityId::new("chat.speak"),
            description: "Speak".to_owned(),
            places: vec![PlaceId::new("tg:1")],
            flags: CapabilityFlags {
                communicative: true,
                identity_changing: false,
                irreversible: false,
            },
            typical_latency_ms: 10,
            capability_schema_hash: CapabilitySchemaHash::new("h1"),
            sensed_effect: None,
        };
        assert!(cache.update_plugin("telegram", vec![cap.clone()]));
        assert!(!cache.update_plugin("telegram", vec![cap.clone()]));
        cache.save(dir.path());

        let reloaded = CapabilityCache::load(dir.path());
        assert_eq!(reloaded.get("telegram"), Some(std::slice::from_ref(&cap)));

        // Corrupted JSON falls back cleanly without panicking.
        std::fs::write(CapabilityCache::cache_path(dir.path()), b"{not valid json").unwrap();
        assert!(CapabilityCache::load(dir.path()).plugins.is_empty());
    }
}
