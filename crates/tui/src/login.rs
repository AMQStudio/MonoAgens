//! `/login`: a searchable provider list with key state per row, the masked key entry and the
//! connection test. The same key page opens from the model picker when a model of a provider
//! without a key is chosen; `Esc` returns to where the user came from.
//!
//! The key lives only in the entry buffer, is shown as dots, is sent once (test, then save) and
//! is cleared as soon as it has been saved or the page is left.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use serde_json::Value;

use crate::catalog_ui::{
    CatalogAction, CatalogUi, ModelChoice, ProviderRow, Screen, Secret, footer_bar, put, title_bar,
};
use crate::config::IconMode;
use crate::render::{render_scrollbar, scroll_window};
use crate::state::AppAction;
use crate::theme::ThemePalette;

/// Longest key accepted (real keys are far shorter).
const MAX_KEY_CHARS: usize = 512;

/// Where the key page is.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum KeyPhase {
    #[default]
    Editing,
    Testing,
    Done { ok: bool, message: String },
}

/// The key page for one provider.
#[derive(Debug, Clone, Default)]
pub struct KeyEntry {
    pub provider: String,
    buffer: String,
    pub phase: KeyPhase,
    /// The model to select once the key is saved (picker detour).
    pub then: Option<ModelChoice>,
    pub saved: bool,
}

impl KeyEntry {
    /// A fresh page for `provider`.
    #[must_use]
    pub fn new(provider: String, then: Option<ModelChoice>) -> Self {
        Self {
            provider,
            then,
            ..Self::default()
        }
    }

    /// Number of characters typed (the key itself is never shown).
    #[must_use]
    pub fn typed_len(&self) -> usize {
        self.buffer.chars().count()
    }
}

/// `/login` list state.
#[derive(Debug, Clone, Default)]
pub struct LoginView {
    pub search: String,
    pub sel: usize,
    pub off: usize,
    pub entry: KeyEntry,
}

impl CatalogUi {
    /// `/login`: opens the provider list.
    pub fn open_login(&mut self, provider: Option<String>, model: String) -> Vec<AppAction> {
        self.screen = Screen::LoginList;
        self.return_to = Screen::Closed;
        self.current_provider = provider;
        self.current_model = model;
        self.login = LoginView::default();
        vec![AppAction::Catalog(CatalogAction::LoadProviders)]
    }

    /// Providers matching the search text.
    #[must_use]
    pub fn login_rows(&self) -> Vec<&ProviderRow> {
        let needle = self.login.search.trim().to_lowercase();
        self.providers
            .iter()
            .filter(|p| {
                needle.is_empty()
                    || p.id.to_lowercase().contains(&needle)
                    || p.label.to_lowercase().contains(&needle)
            })
            .collect()
    }

    pub(crate) fn login_list_key(&mut self, key: KeyEvent) -> Vec<AppAction> {
        let total = self.login_rows().len();
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Esc => {
                if self.login.search.is_empty() {
                    self.screen = Screen::Closed;
                } else {
                    self.login.search.clear();
                    self.login.sel = 0;
                    self.login.off = 0;
                }
            }
            KeyCode::Up if total > 0 => {
                self.login.sel = self.login.sel.saturating_sub(1);
                self.login.off = scroll_window(self.login.sel, self.login.off, self.rows(), total);
            }
            KeyCode::Down if total > 0 => {
                self.login.sel = (self.login.sel + 1).min(total - 1);
                self.login.off = scroll_window(self.login.sel, self.login.off, self.rows(), total);
            }
            KeyCode::PageUp if total > 0 => {
                self.login.sel = self.login.sel.saturating_sub(self.rows().saturating_sub(1).max(1));
                self.login.off = scroll_window(self.login.sel, self.login.off, self.rows(), total);
            }
            KeyCode::PageDown if total > 0 => {
                self.login.sel = (self.login.sel + self.rows().saturating_sub(1).max(1)).min(total - 1);
                self.login.off = scroll_window(self.login.sel, self.login.off, self.rows(), total);
            }
            KeyCode::Enter => {
                let id = self.login_rows().get(self.login.sel).map(|p| p.id.clone());
                if let Some(id) = id {
                    self.login.entry = KeyEntry::new(id, None);
                    self.return_to = Screen::LoginList;
                    self.screen = Screen::KeyEntry;
                }
            }
            KeyCode::Backspace => {
                self.login.search.pop();
                self.login.sel = 0;
                self.login.off = 0;
            }
            KeyCode::Char('u') if ctrl => {
                self.login.search.clear();
                self.login.sel = 0;
                self.login.off = 0;
            }
            KeyCode::Char(c) if !ctrl && !key.modifiers.contains(KeyModifiers::ALT) => {
                self.login.search.push(c);
                self.login.sel = 0;
                self.login.off = 0;
            }
            _ => {}
        }
        Vec::new()
    }

    pub(crate) fn key_entry_paste(&mut self, raw: &str) {
        let e = &mut self.login.entry;
        if matches!(e.phase, KeyPhase::Testing) {
            return;
        }
        if matches!(e.phase, KeyPhase::Done { .. }) {
            e.phase = KeyPhase::Editing;
        }
        for c in raw.chars().filter(|c| !c.is_control() && !c.is_whitespace()) {
            if e.buffer.chars().count() < MAX_KEY_CHARS {
                e.buffer.push(c);
            }
        }
    }

    fn leave_key_entry(&mut self) -> Vec<AppAction> {
        let saved = self.login.entry.saved;
        let back = if self.return_to == Screen::Closed {
            Screen::LoginList
        } else {
            self.return_to
        };
        self.login.entry = KeyEntry::default();
        self.screen = back;
        if saved {
            // The key state of every provider and the model lists changed.
            vec![AppAction::Catalog(CatalogAction::LoadProviders)]
        } else {
            Vec::new()
        }
    }

    pub(crate) fn key_entry_key(&mut self, key: KeyEvent) -> Vec<AppAction> {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let keyless = self
            .provider(&self.login.entry.provider)
            .is_some_and(|p| p.keyless);
        let provider = self.login.entry.provider.clone();
        match key.code {
            KeyCode::Esc => return self.leave_key_entry(),
            KeyCode::Enter => {
                let e = &mut self.login.entry;
                if matches!(e.phase, KeyPhase::Testing) {
                    return Vec::new();
                }
                if e.buffer.trim().is_empty() && !keyless {
                    e.phase = KeyPhase::Done {
                        ok: false,
                        message: "Type or paste an API key first.".to_owned(),
                    };
                    return Vec::new();
                }
                e.phase = KeyPhase::Testing;
                return vec![AppAction::Catalog(CatalogAction::TestKey {
                    provider,
                    key: Secret::new(e.buffer.trim()),
                    then: e.then.clone(),
                })];
            }
            KeyCode::Char('s') if ctrl => {
                let e = &mut self.login.entry;
                if e.buffer.trim().is_empty() || matches!(e.phase, KeyPhase::Testing) {
                    return Vec::new();
                }
                e.phase = KeyPhase::Testing;
                return vec![AppAction::Catalog(CatalogAction::SaveKey {
                    provider,
                    key: Secret::new(e.buffer.trim()),
                    then: e.then.clone(),
                })];
            }
            KeyCode::Char('u') if ctrl => self.login.entry.buffer.clear(),
            KeyCode::Backspace => {
                let e = &mut self.login.entry;
                if !matches!(e.phase, KeyPhase::Testing) {
                    e.buffer.pop();
                    if matches!(e.phase, KeyPhase::Done { .. }) {
                        e.phase = KeyPhase::Editing;
                    }
                }
            }
            KeyCode::Char(c) if !ctrl && !key.modifiers.contains(KeyModifiers::ALT) => {
                let e = &mut self.login.entry;
                if !matches!(e.phase, KeyPhase::Testing)
                    && !c.is_whitespace()
                    && e.buffer.chars().count() < MAX_KEY_CHARS
                {
                    if matches!(e.phase, KeyPhase::Done { .. }) {
                        e.phase = KeyPhase::Editing;
                    }
                    e.buffer.push(c);
                }
            }
            _ => {}
        }
        Vec::new()
    }

    pub(crate) fn on_key_tested(&mut self, provider: &str, result: Result<Value, String>) {
        if self.login.entry.provider != provider || self.screen != Screen::KeyEntry {
            return;
        }
        let (ok, message) = match result {
            Ok(v) if v.get("ok").and_then(Value::as_bool) == Some(true) => {
                let n = v.get("model_count").and_then(Value::as_u64).unwrap_or(0);
                let ms = v.get("latency_ms").and_then(Value::as_u64).unwrap_or(0);
                (true, format!("Connected: {n} models, {ms} ms. Saving the key..."))
            }
            Ok(v) => {
                let err = v.get("error").and_then(Value::as_str).unwrap_or("connection failed");
                (false, format!("Test failed: {err}. Ctrl+S saves the key anyway."))
            }
            Err(e) => (false, format!("Test failed: {e}")),
        };
        self.login.entry.phase = KeyPhase::Done { ok, message };
    }

    pub(crate) fn on_key_saved(&mut self, provider: &str, result: Result<(), String>) {
        match result {
            Ok(()) => {
                let typed = self.login.entry.typed_len() > 0;
                if let Some(row) = self.providers.iter_mut().find(|p| p.id == provider)
                    && (!row.keyless || typed)
                {
                    row.key_set = true;
                    row.key_source = Some("secrets".to_owned());
                }
                if self.login.entry.provider == provider {
                    let e = &mut self.login.entry;
                    e.buffer.clear();
                    e.saved = true;
                    let prev = match &e.phase {
                        KeyPhase::Done { message, .. } => message.trim_end_matches("Saving the key...").trim().to_owned(),
                        _ => String::new(),
                    };
                    e.phase = KeyPhase::Done {
                        ok: true,
                        message: if prev.is_empty() {
                            "Key saved.".to_owned()
                        } else {
                            format!("{prev} Key saved.")
                        },
                    };
                }
            }
            Err(e) => {
                if self.login.entry.provider == provider {
                    self.login.entry.phase = KeyPhase::Done {
                        ok: false,
                        message: format!("Could not save the key: {e}"),
                    };
                }
            }
        }
    }
}

/// Draws the provider list.
pub fn render_list(area: Rect, buf: &mut Buffer, ui: &CatalogUi, palette: &ThemePalette, icons: IconMode) {
    let w = area.width as usize;
    title_bar(area, buf, " Providers   API keys", palette, icons);
    let search = format!(" Search: {}▏", ui.login.search);
    put(
        buf,
        area.x,
        area.y + 1,
        w,
        &search,
        Style::default().fg(palette.accent).bg(palette.bg),
        icons,
    );
    let list_top = area.y + 2;
    let visible = (area.y + area.height - 1).saturating_sub(list_top) as usize;
    let rows = ui.login_rows();
    let offset = scroll_window(ui.login.sel, ui.login.off, visible, rows.len());
    let list_w = w.saturating_sub(1);
    for (i, p) in rows.iter().skip(offset).take(visible).enumerate() {
        let y = list_top + i as u16;
        let (dot, state, fg) = if p.keyless {
            ("◌", "no key needed".to_owned(), palette.dim)
        } else if p.key_set {
            (
                "●",
                match p.key_source.as_deref() {
                    Some("env") => "key set (environment)".to_owned(),
                    Some("legacy") => "key set (legacy)".to_owned(),
                    _ => "key set".to_owned(),
                },
                palette.status_green,
            )
        } else {
            ("○", "not set".to_owned(), palette.dim)
        };
        let current = ui.current_provider.as_deref() == Some(p.id.as_str());
        let label = format!("{}{}", p.label, if current { "  (current)" } else { "" });
        let name_w = list_w.saturating_sub(24).max(10);
        let line = format!(" {dot} {}  {state}", crate::catalog_ui::pad(&label, name_w));
        let mut style = Style::default().fg(fg).bg(palette.bg);
        if i + offset == ui.login.sel {
            style = style
                .fg(palette.fg)
                .bg(palette.selection_bg)
                .add_modifier(Modifier::BOLD);
        }
        put(buf, area.x, y, list_w, &line, style, icons);
    }
    if rows.is_empty() {
        let msg = if ui.providers_loaded {
            " No provider matches."
        } else {
            " Loading providers…"
        };
        put(buf, area.x, list_top, w, msg, Style::default().fg(palette.dim).bg(palette.bg), icons);
    }
    render_scrollbar(
        Rect::new(area.x + area.width - 1, list_top, 1, visible as u16),
        buf,
        rows.len(),
        offset,
        visible,
        palette,
    );
    footer_bar(area, buf, " ⏎ set key · ↑/↓ · type to search · ⎋ close", palette, icons);
}

/// Draws the key page.
pub fn render_key_entry(area: Rect, buf: &mut Buffer, ui: &CatalogUi, palette: &ThemePalette, icons: IconMode) {
    let w = area.width as usize;
    let e = &ui.login.entry;
    let label = ui.label_of(&e.provider);
    title_bar(area, buf, &format!(" API key   {label}"), palette, icons);
    let keyless = ui.provider(&e.provider).is_some_and(|p| p.keyless);
    let mut y = area.y + 2;
    let normal = Style::default().fg(palette.fg).bg(palette.bg);
    let dim = Style::default().fg(palette.dim).bg(palette.bg);
    if let Some(choice) = &e.then {
        put(buf, area.x, y, w, &format!(" Model to select afterwards: {}", choice.model), dim, icons);
        y += 1;
    }
    y += 1;
    let dots = "•".repeat(e.typed_len().min(w.saturating_sub(12)));
    let editing = !matches!(e.phase, KeyPhase::Testing);
    put(
        buf,
        area.x,
        y,
        w,
        &format!(" Key: {dots}{}", if editing { "▏" } else { "" }),
        Style::default().fg(palette.accent).bg(palette.bg).add_modifier(Modifier::BOLD),
        icons,
    );
    y += 2;
    let (text, style) = match &e.phase {
        KeyPhase::Editing if keyless => (
            " This provider needs no key. Press Enter to test the connection.".to_owned(),
            dim,
        ),
        KeyPhase::Editing => (
            format!(" Enter tests the connection, then saves the key to secrets.toml (mode 0600). {} typed.", e.typed_len()),
            dim,
        ),
        KeyPhase::Testing => (" Testing the connection…".to_owned(), normal),
        KeyPhase::Done { ok: true, message } => (
            format!(" ✓ {message}"),
            Style::default().fg(palette.status_green).bg(palette.bg),
        ),
        KeyPhase::Done { ok: false, message } => (
            format!(" ✗ {message}"),
            Style::default().fg(palette.status_red).bg(palette.bg),
        ),
    };
    for (i, chunk) in wrap(&text, w).into_iter().take(4).enumerate() {
        put(buf, area.x, y + i as u16, w, &chunk, style, icons);
    }
    footer_bar(area, buf, " ⏎ test & save · ^S save without test · ^U clear · ⎋ back", palette, icons);
}

/// Greedy word wrap on display columns.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut cur = String::new();
    for word in text.split(' ') {
        let add = if cur.is_empty() { word.to_owned() } else { format!("{cur} {word}") };
        if unicode_width::UnicodeWidthStr::width(add.as_str()) > width && !cur.is_empty() {
            lines.push(cur);
            cur = format!(" {word}");
        } else {
            cur = add;
        }
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    lines
}
