//! Chat search, pin ordering and Markdown export.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::config::{LocalChatEntry, LocalChatStore};
use crate::message_actions::message_text;
use crate::state::ChatTimelineItem;

/// Whether `needle` (case-insensitive) occurs in the chat's title or in any message text.
#[must_use]
pub fn chat_matches(
    chat: &LocalChatEntry,
    timeline: Option<&[ChatTimelineItem]>,
    needle: &str,
) -> bool {
    let needle = needle.trim().to_lowercase();
    if needle.is_empty() || chat.title.to_lowercase().contains(&needle) {
        return true;
    }
    match timeline {
        Some(items) if !items.is_empty() => items
            .iter()
            .filter_map(message_text)
            .any(|t| t.to_lowercase().contains(&needle)),
        _ => chat
            .messages
            .iter()
            .any(|m| m.text_preview.to_lowercase().contains(&needle)),
    }
}

/// The chats of the sidebar in display order: filtered by `filter` (title and message text),
/// pinned chats first, otherwise in their stored order.
#[must_use]
pub fn visible_chats<'a>(
    chats: &'a LocalChatStore,
    timelines: &HashMap<String, Vec<ChatTimelineItem>>,
    filter: Option<&str>,
) -> Vec<&'a LocalChatEntry> {
    let mut out: Vec<&LocalChatEntry> = chats
        .chats
        .iter()
        .filter(|c| {
            filter.is_none_or(|f| chat_matches(c, timelines.get(&c.id).map(Vec::as_slice), f))
        })
        .collect();
    out.sort_by_key(|c| !c.pinned); // stable: pinned first, order kept inside each group
    out
}

/// Masks things that look like credentials before text leaves the TUI in an export.
#[must_use]
pub fn mask_secrets(text: &str) -> String {
    text.split_inclusive(char::is_whitespace)
        .map(|tok| {
            let word = tok.trim_end();
            let tail = &tok[word.len()..];
            let looks_secret = (word.starts_with("sk-") && word.len() >= 12)
                || (word.len() >= 32
                    && word.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'))
                    && word.chars().any(|c| c.is_ascii_digit())
                    && word.chars().any(|c| c.is_ascii_alphabetic()));
            if looks_secret {
                format!("[redacted]{tail}")
            } else {
                tok.to_owned()
            }
        })
        .collect()
}

/// The chat as Markdown: a heading, then each message in order (text only; attachments appear
/// as the filenames already in the message text).
#[must_use]
pub fn export_markdown(title: &str, soul_name: &str, items: &[ChatTimelineItem]) -> String {
    let mut out = format!("# {}\n", title.trim());
    for item in items {
        match item {
            ChatTimelineItem::UserMessage { text, .. } => {
                out.push_str("\n**You**\n\n");
                out.push_str(&mask_secrets(text));
                out.push('\n');
            }
            ChatTimelineItem::AssistantMessage { text, .. } => {
                out.push_str(&format!("\n**{soul_name}**\n\n"));
                out.push_str(&mask_secrets(text));
                out.push('\n');
            }
            _ => {}
        }
    }
    out
}

/// Civil date (UTC) of a unix timestamp.
#[must_use]
pub fn ymd_of(unix_secs: u64) -> (u64, u64, u64) {
    let z = (unix_secs / 86_400) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y as u64, m as u64, d as u64)
}

/// `<title-slug>-<YYYYMMDD>.md`.
#[must_use]
pub fn default_export_name(title: &str, unix_secs: u64) -> String {
    let slug: String = title
        .chars()
        .map(|c| if c.is_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    let slug = if slug.is_empty() { "chat".to_owned() } else { slug };
    let (y, m, d) = ymd_of(unix_secs);
    format!("{}-{y:04}{m:02}{d:02}.md", slug.chars().take(48).collect::<String>())
}

/// Resolves the export target: the given path (a directory gets the default file name) or
/// `<data_dir>/exports/<default name>`.
#[must_use]
pub fn export_target(data_dir: &Path, arg: Option<&str>, title: &str, unix_secs: u64) -> PathBuf {
    let name = default_export_name(title, unix_secs);
    match arg.map(str::trim).filter(|a| !a.is_empty()) {
        Some(a) => {
            let p = PathBuf::from(a);
            if p.is_dir() || a.ends_with('/') {
                p.join(name)
            } else {
                p
            }
        }
        None => data_dir.join("exports").join(name),
    }
}

/// Writes `markdown` to `path` (creating parents), mode 0600.
///
/// # Errors
/// I/O failures as text.
pub fn write_export(path: &Path, markdown: &str) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    if let Some(dir) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| format!("cannot create '{}': {e}", dir.display()))?;
    }
    std::fs::write(path, markdown).map_err(|e| format!("cannot write '{}': {e}", path.display()))?;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::StoredChatMessagePreview;

    fn msg(text: &str) -> StoredChatMessagePreview {
        StoredChatMessagePreview {
            role: "user".into(),
            sender: "me".into(),
            text_preview: text.into(),
            timestamp_ms: 0,
            duration_ms: None,
        }
    }

    #[test]
    fn search_finds_title_and_message_text_and_pin_sorts_first() {
        let mut store = LocalChatStore::default();
        store.chats[0].title = "Greetings".into();
        store.chats[0].messages.push(msg("nothing special"));
        store.create_chat(Some("Recipes"));
        store.chats[1].messages.push(msg("how to bake a zanzibarcake"));
        store.create_chat(Some("Newest"));
        let tl = HashMap::new();

        let hits = visible_chats(&store, &tl, Some("zanzibar"));
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].title, "Recipes");
        assert_eq!(visible_chats(&store, &tl, Some("greet")).len(), 1, "title match");
        assert_eq!(visible_chats(&store, &tl, None).len(), 3);

        store.chats[0].pinned = true; // the oldest chat
        let order: Vec<_> = visible_chats(&store, &tl, None).iter().map(|c| c.title.clone()).collect();
        assert_eq!(order, ["Greetings", "Recipes", "Newest"]);
        store.chats[2].pinned = true;
        let order: Vec<_> = visible_chats(&store, &tl, None).iter().map(|c| c.title.clone()).collect();
        assert_eq!(order, ["Greetings", "Newest", "Recipes"], "pinned keep their relative order");
    }

    #[test]
    fn pinned_flag_persists_and_old_files_load() {
        let dir = { let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/tui-unit-tests"); std::fs::create_dir_all(&base).unwrap(); base.join(format!("pin-{}", std::process::id())) };
        let path = dir.join("tui_chats.json");
        let mut store = LocalChatStore::default();
        store.chats[0].pinned = true;
        store.save_to_path(&path).unwrap();
        assert!(LocalChatStore::load_from_path(&path).unwrap().chats[0].pinned);
        let old = r#"{"chats":[{"id":"direct","place_id":"cli:terminal","title":"New Chat","title_locked":false,"turns_since_retitle":0}],"active_chat_id":"direct"}"#;
        std::fs::write(&path, old).unwrap();
        assert!(!LocalChatStore::load_from_path(&path).unwrap().chats[0].pinned);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn export_markdown_keeps_order_and_masks_keys() {
        let items = vec![
            ChatTimelineItem::UserMessage { sender: "me".into(), text: "my key is sk-abcdefghijklmnop ok [file] a.png".into(), timestamp_ms: 0, is_group: false },
            ChatTimelineItem::SystemNotice { text: "ignore me".into(), is_error: false },
            ChatTimelineItem::AssistantMessage { text: "hello there".into(), duration_ms: None, timestamp_ms: 0 },
        ];
        let md = export_markdown("My Chat", "Mono", &items);
        let user = md.find("**You**").unwrap();
        let bot = md.find("**Mono**").unwrap();
        assert!(user < bot && md.starts_with("# My Chat"));
        assert!(!md.contains("sk-abcdefghijklmnop") && md.contains("[redacted]") && md.contains("a.png"));
        assert!(!md.contains("ignore me"));
    }

    #[test]
    fn export_names_and_dates() {
        assert_eq!(ymd_of(0), (1970, 1, 1));
        assert_eq!(ymd_of(1_700_000_000), (2023, 11, 14));
        assert_eq!(default_export_name("Tolong Jelaskan Valve!", 1_700_000_000), "tolong-jelaskan-valve-20231114.md");
        assert_eq!(default_export_name("???", 0), "chat-19700101.md");
        let t = export_target(Path::new("/d"), None, "A b", 0);
        assert_eq!(t, Path::new("/d/exports/a-b-19700101.md"));
        assert_eq!(export_target(Path::new("/d"), Some("/x/out.md"), "A", 0), Path::new("/x/out.md"));
    }
}
