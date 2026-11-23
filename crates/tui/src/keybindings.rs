//! Action table, `keybindings.toml` loader and key-string parsing.
//!
//! `<config_dir>/keybindings.toml` maps action ids to key strings (`"ui.palette" = "ctrl+p"`);
//! defaults live in [`ACTIONS`]. Unknown ids, unparseable keys and conflicts (two actions on one
//! key) are reported as warnings (shown in `/log`) and ignored. The `?` overlay is generated from
//! the same table, so it always shows the key that is really bound.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// One bindable (or documented) action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActionDef {
    pub id: &'static str,
    /// Screen the key works on: `global`, `chat`, `settings`, `list`, `editor`.
    pub screen: &'static str,
    pub default_key: &'static str,
    pub description: &'static str,
    /// Handled through the key map (rebindable). Others are documented fixed keys.
    pub rebindable: bool,
}

const fn a(id: &'static str, screen: &'static str, default_key: &'static str, description: &'static str, rebindable: bool) -> ActionDef {
    ActionDef { id, screen, default_key, description, rebindable }
}

/// The action table.
pub const ACTIONS: &[ActionDef] = &[
    a("ui.help", "global", "f1", "Show this key list (also `?` on an empty composer)", true),
    a("ui.palette", "global", "ctrl+k", "Command palette: commands, settings, chats", true),
    a("ui.sidebar", "global", "ctrl+b", "Toggle the sidebar", true),
    a("app.reset_display", "global", "alt+l", "Clear and repaint the terminal", true),
    a("app.suspend", "global", "ctrl+z", "Suspend to the shell", true),
    a("app.quit", "global", "ctrl+c", "Clear a draft; quit when empty (Ctrl+Q)", false),
    a("chat.send", "chat", "enter", "Send the message", false),
    a("chat.newline", "chat", "shift+enter", "Insert a newline", false),
    a("chat.clear", "chat", "esc", "Clear the composer", false),
    a("chat.paste", "chat", "ctrl+v", "Paste text or an image", false),
    a("chat.slash", "chat", "/", "Open the command menu (Tab completes, PgUp/PgDn page)", false),
    a("chat.unqueue", "chat", "alt+up", "Pull the last queued message back", false),
    a("chat.interrupt", "chat", "esc esc", "Interrupt the running turn (Esc, Esc)", false),
    a("chat.retry", "chat", "f5", "Retry a failed or interrupted turn (F5, Alt+R)", false),
    a("chat.retry_boot", "chat", "ctrl+r", "Config-error banner: retry the boot", false),
    a("chat.history_search", "chat", "ctrl+r", "Search sent-message history (Enter, Esc)", false),
    a("chat.editor", "chat", "ctrl+g", "Edit the draft in $EDITOR", false),
    a("chat.select_message", "chat", "ctrl+up", "Select a message (Ctrl+Up/Down; c q w)", false),
    a("chat.what_happened", "chat", "ctrl+o", "Expand the what-happened summary", false),
    a("chat.find", "chat", "ctrl+f", "Filter this chat's messages", false),
    a("chat.restore_draft", "chat", "up", "Restore a cleared draft (Up when empty)", false),
    a("settings.tabs", "settings", "left", "Previous / next tab (left, right)", false),
    a("settings.sections", "settings", "tab", "Jump to the next section", false),
    a("settings.edit", "settings", "enter", "Edit the selected row (Space toggles)", false),
    a("settings.back", "settings", "esc", "Back one level: editor, sub-page, search, overlay", false),
    a("settings.search", "settings", "type", "Type to filter rows", false),
    a("list.move", "list", "up", "Move the selection (up, down, PgUp, PgDn)", false),
    a("list.pick", "list", "enter", "Pick the highlighted entry", false),
    a("list.search", "list", "type", "Type to search", false),
    a("editor.add", "editor", "a", "Add an item", false),
    a("editor.delete", "editor", "d", "Delete the selected item", false),
    a("editor.reorder", "editor", "alt+up", "Move the item up or down (Alt+Up / Alt+Down, K / J)", false),
    a("editor.reveal", "editor", "ctrl+r", "Reveal or hide a secret while typing", false),
];

/// The fixed (not rebindable) chat shortcut that already uses `key`: global keys must not shadow
/// the composer and conversation shortcuts.
fn fixed_chat_clash(key: Key) -> Option<&'static ActionDef> {
    ACTIONS
        .iter()
        .find(|o| o.screen == "chat" && !o.rebindable && parse_key(o.default_key).is_some_and(|k| k == key))
}

/// A parsed key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Key {
    pub code: KeyCode,
    pub mods: KeyModifiers,
}

/// Parses `ctrl+k`, `alt+l`, `f1`, `shift+tab`, `?`, `esc`...
#[must_use]
pub fn parse_key(raw: &str) -> Option<Key> {
    let lower = raw.trim().to_lowercase();
    if lower.is_empty() {
        return None;
    }
    let mut mods = KeyModifiers::NONE;
    let parts: Vec<&str> = if lower == "+" { vec!["+"] } else { lower.split('+').collect() };
    let (last, mod_parts) = parts.split_last()?;
    for m in mod_parts {
        mods |= match *m {
            "ctrl" | "control" => KeyModifiers::CONTROL,
            "alt" | "meta" | "option" => KeyModifiers::ALT,
            "shift" => KeyModifiers::SHIFT,
            _ => return None,
        };
    }
    let code = match *last {
        "enter" | "return" => KeyCode::Enter,
        "esc" | "escape" => KeyCode::Esc,
        "tab" => KeyCode::Tab,
        "backspace" => KeyCode::Backspace,
        "up" => KeyCode::Up,
        "down" => KeyCode::Down,
        "left" => KeyCode::Left,
        "right" => KeyCode::Right,
        "pageup" => KeyCode::PageUp,
        "pagedown" => KeyCode::PageDown,
        "home" => KeyCode::Home,
        "end" => KeyCode::End,
        "space" => KeyCode::Char(' '),
        f if f.len() > 1 && f.starts_with('f') && f[1..].chars().all(|c| c.is_ascii_digit()) => {
            let n: u8 = f[1..].parse().ok()?;
            if !(1..=24).contains(&n) {
                return None;
            }
            KeyCode::F(n)
        }
        c if c.chars().count() == 1 => KeyCode::Char(c.chars().next()?),
        _ => return None,
    };
    Some(Key { code, mods })
}

/// Whether `ev` is the key `key` (shift is ignored for plain characters, which already carry it).
#[must_use]
pub fn key_matches(key: Key, ev: &KeyEvent) -> bool {
    let norm = |m: KeyModifiers, code: KeyCode| {
        if matches!(code, KeyCode::Char(_)) {
            m - KeyModifiers::SHIFT
        } else {
            m
        }
    };
    let code_eq = match (key.code, ev.code) {
        (KeyCode::Char(a), KeyCode::Char(b)) => a.eq_ignore_ascii_case(&b),
        (a, b) => a == b,
    };
    code_eq && norm(key.mods, key.code) == norm(ev.modifiers, ev.code)
}

/// The active bindings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyMap {
    bindings: BTreeMap<String, String>,
    /// Problems found while loading (unknown ids, bad keys, conflicts); already ignored.
    pub warnings: Vec<String>,
}

impl Default for KeyMap {
    fn default() -> Self {
        Self {
            bindings: ACTIONS
                .iter()
                .map(|d| (d.id.to_owned(), d.default_key.to_owned()))
                .collect(),
            warnings: Vec::new(),
        }
    }
}

impl KeyMap {
    /// `<config_dir>/keybindings.toml` next to `monoagens.toml`.
    #[must_use]
    pub fn path_for_config(config_path: &Path) -> PathBuf {
        let dir = config_path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        dir.join("keybindings.toml")
    }

    /// Defaults plus the overrides of `raw` (the file text).
    #[must_use]
    pub fn from_toml(raw: &str) -> Self {
        let mut map = Self::default();
        let table = match raw.parse::<toml_edit::DocumentMut>() {
            Ok(t) => t,
            Err(e) => {
                map.warnings.push(format!("keybindings.toml is not valid TOML, using defaults: {e}"));
                return map;
            }
        };
        // Reserved: the first action bound to a key wins; user overrides are applied one by one.
        for (id, item) in table.iter() {
            let val = item.as_str();
            let Some(def) = ACTIONS.iter().find(|d| d.id == id) else {
                map.warnings.push(format!("keybindings.toml: unknown action '{id}' ignored"));
                continue;
            };
            let Some(key_str) = val else {
                map.warnings.push(format!("keybindings.toml: '{id}' must be a key string"));
                continue;
            };
            if !def.rebindable {
                map.warnings.push(format!("keybindings.toml: '{id}' is fixed and cannot be rebound"));
                continue;
            }
            let Some(new_key) = parse_key(key_str) else {
                map.warnings.push(format!("keybindings.toml: '{id}' has an unknown key '{key_str}'"));
                continue;
            };
            // Conflict: another global action already uses this key.
            let clash = ACTIONS.iter().find(|o| {
                o.id != id
                    && o.screen == "global"
                    && map.bindings.get(o.id).and_then(|k| parse_key(k)).is_some_and(|k| k == new_key)
            });
            if let Some(other) = clash.or_else(|| fixed_chat_clash(new_key)) {
                map.warnings.push(format!(
                    "keybindings.toml: '{id}' = '{key_str}' conflicts with '{}'; the binding of '{id}' is ignored",
                    other.id
                ));
                continue;
            }
            map.bindings.insert(id.to_owned(), key_str.trim().to_lowercase());
        }
        map
    }

    /// Loads the file (missing file = defaults).
    #[must_use]
    pub fn load(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(raw) => Self::from_toml(&raw),
            Err(_) => Self::default(),
        }
    }

    /// The key string bound to `id`.
    #[must_use]
    pub fn key_of(&self, id: &str) -> String {
        self.bindings
            .get(id)
            .cloned()
            .or_else(|| ACTIONS.iter().find(|d| d.id == id).map(|d| d.default_key.to_owned()))
            .unwrap_or_default()
    }

    /// Whether `ev` triggers action `id`.
    #[must_use]
    pub fn matches(&self, id: &str, ev: &KeyEvent) -> bool {
        parse_key(&self.key_of(id)).is_some_and(|k| key_matches(k, ev))
    }

    /// Rebinds `id`; refuses unknown/fixed actions, unparseable keys and keys another global
    /// action uses.
    pub fn set(&mut self, id: &str, key: &str) -> Result<(), String> {
        let def = ACTIONS.iter().find(|d| d.id == id).ok_or_else(|| format!("unknown action '{id}'"))?;
        if !def.rebindable {
            return Err(format!("'{id}' is fixed and cannot be rebound"));
        }
        let parsed = parse_key(key).ok_or_else(|| format!("'{key}' is not a key (examples: ctrl+k, alt+l, f2)"))?;
        if let Some(other) = ACTIONS
            .iter()
            .find(|o| {
                o.id != id
                    && o.screen == "global"
                    && parse_key(&self.key_of(o.id)).is_some_and(|k| k == parsed)
            })
            .or_else(|| fixed_chat_clash(parsed))
        {
            return Err(format!("'{key}' is already bound to '{}'", other.id));
        }
        self.bindings.insert(id.to_owned(), key.trim().to_lowercase());
        Ok(())
    }

    /// Restores the default key of `id`.
    pub fn reset(&mut self, id: &str) {
        if let Some(d) = ACTIONS.iter().find(|d| d.id == id) {
            self.bindings.insert(id.to_owned(), d.default_key.to_owned());
        }
    }

    /// `(action id, key)` of every rebindable action (the settings editor rows).
    #[must_use]
    pub fn rebindable_entries(&self) -> Vec<(String, String)> {
        ACTIONS
            .iter()
            .filter(|d| d.rebindable)
            .map(|d| (d.id.to_owned(), self.key_of(d.id)))
            .collect()
    }

    /// Writes the overrides (keys that differ from the defaults) to `path`.
    pub fn save(&self, path: &Path) -> Result<(), String> {
        let mut out = String::from("# MonoAgens key bindings: action id = key. Delete a line to restore the default.\n");
        for d in ACTIONS.iter().filter(|d| d.rebindable) {
            let k = self.key_of(d.id);
            if k != d.default_key {
                out.push_str(&format!("\"{}\" = \"{k}\"\n", d.id));
            }
        }
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        std::fs::write(path, out).map_err(|e| format!("failed to write '{}': {e}", path.display()))
    }

    /// Display form of a key string (`ctrl+k` -> `Ctrl+K`).
    #[must_use]
    pub fn pretty(key: &str) -> String {
        key.split('+')
            .map(|p| match p {
                "ctrl" => "Ctrl".to_owned(),
                "alt" => "Alt".to_owned(),
                "shift" => "Shift".to_owned(),
                other if other.chars().count() == 1 => other.to_uppercase(),
                other => {
                    let mut c = other.chars();
                    c.next().map_or_else(String::new, |f| f.to_uppercase().collect::<String>() + c.as_str())
                }
            })
            .collect::<Vec<_>>()
            .join("+")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_keys() {
        assert_eq!(parse_key("ctrl+k"), Some(Key { code: KeyCode::Char('k'), mods: KeyModifiers::CONTROL }));
        assert_eq!(parse_key("F1").map(|k| k.code), Some(KeyCode::F(1)));
        assert_eq!(parse_key("shift+tab").map(|k| k.code), Some(KeyCode::Tab));
        assert!(parse_key("ctrl+").is_none());
        assert!(parse_key("hyper+x").is_none());
    }

    #[test]
    fn conflict_is_ignored_and_names_both_actions() {
        let m = KeyMap::from_toml("\"ui.palette\" = \"ctrl+b\"\n");
        assert_eq!(m.key_of("ui.palette"), "ctrl+k");
        assert_eq!(m.warnings.len(), 1);
        assert!(m.warnings[0].contains("ui.palette") && m.warnings[0].contains("ui.sidebar"), "{:?}", m.warnings);
    }

    #[test]
    fn rebinding_and_unknown_ids() {
        let m = KeyMap::from_toml("\"ui.palette\" = \"ctrl+p\"\n\"nope\" = \"f5\"\n\"app.quit\" = \"f9\"\n");
        assert_eq!(m.key_of("ui.palette"), "ctrl+p");
        assert!(m.matches("ui.palette", &KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL)));
        assert!(!m.matches("ui.palette", &KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL)));
        assert_eq!(m.warnings.len(), 2, "{:?}", m.warnings);
    }

    #[test]
    fn two_user_bindings_on_one_key_keep_the_first() {
        let m = KeyMap::from_toml("\"ui.palette\" = \"f7\"\n\"ui.help\" = \"f7\"\n");
        // File order: "ui.palette" binds first, the later "ui.help" is ignored.
        assert_eq!(m.key_of("ui.palette"), "f7");
        assert_eq!(m.key_of("ui.help"), "f1");
        assert!(m.warnings[0].contains("ui.help") && m.warnings[0].contains("ui.palette"));
    }
}
