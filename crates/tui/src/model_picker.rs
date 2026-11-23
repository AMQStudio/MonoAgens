//! `/model`: the two-pane model picker. Left: providers (configured first with their model
//! count, a separator, then those without a key) under an "All models" row. Right: search line,
//! model list with scrollbar and a detail line for the focused model.
//!
//! The list shows the cached models instantly; a refresh runs in the background (`Ctrl+R`, or
//! `r` while the provider pane has focus). Typing searches; a typed id that is not in the list
//! can be used as is ("custom id" row). Choosing a model of a provider without a key opens that
//! provider's key page first and selects the model once the key is saved.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};

use crate::catalog_ui::{
    CatalogAction, CatalogUi, ModelChoice, ModelRow, Screen, fmt_context, fmt_price, footer_bar, pad,
    put, title_bar,
};
use crate::config::IconMode;
use crate::login::KeyEntry;
use crate::render::{render_scrollbar, scroll_window};
use crate::state::AppAction;
use crate::theme::ThemePalette;

/// Which pane has the keyboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Pane {
    Providers,
    #[default]
    Models,
}

/// Picker state (data lives in [`CatalogUi`]).
#[derive(Debug, Clone, Default)]
pub struct ModelPicker {
    pub focus: Pane,
    /// Index into [`CatalogUi::left_rows`].
    pub left_sel: usize,
    pub left_off: usize,
    pub model_sel: usize,
    pub model_off: usize,
    pub search: String,
    /// Select the current provider's row once the provider list arrives.
    pub auto_select: bool,
}

/// A row of the provider pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeftRow {
    All,
    /// Index into [`CatalogUi::providers`].
    Provider(usize),
    Separator,
}

/// A row of the model pane.
#[derive(Debug, Clone, PartialEq)]
pub enum RightRow {
    Model { provider: String, info: ModelRow },
    /// The typed text used as a model id.
    Custom { provider: String, id: String },
}

impl CatalogUi {
    /// Provider pane rows: All, configured, separator, unconfigured.
    #[must_use]
    pub fn left_rows(&self) -> Vec<LeftRow> {
        let mut rows = vec![LeftRow::All];
        let configured: Vec<usize> = (0..self.providers.len())
            .filter(|&i| self.providers[i].configured())
            .collect();
        let other: Vec<usize> = (0..self.providers.len())
            .filter(|&i| !self.providers[i].configured())
            .collect();
        rows.extend(configured.iter().map(|&i| LeftRow::Provider(i)));
        if !configured.is_empty() && !other.is_empty() {
            rows.push(LeftRow::Separator);
        }
        rows.extend(other.iter().map(|&i| LeftRow::Provider(i)));
        rows
    }

    /// Provider id selected in the left pane (`None` for "All models").
    #[must_use]
    pub fn picker_provider(&self) -> Option<String> {
        match self.left_rows().get(self.picker.left_sel) {
            Some(LeftRow::Provider(i)) => self.providers.get(*i).map(|p| p.id.clone()),
            _ => None,
        }
    }

    /// Model pane rows for the current provider selection and search text.
    #[must_use]
    pub fn right_rows(&self) -> Vec<RightRow> {
        let needle = self.picker.search.trim().to_lowercase();
        let scope: Vec<&str> = match self.picker_provider() {
            Some(p) => self
                .providers
                .iter()
                .filter(|r| r.id == p)
                .map(|r| r.id.as_str())
                .collect(),
            None => self
                .providers
                .iter()
                .filter(|r| r.configured())
                .map(|r| r.id.as_str())
                .collect(),
        };
        let mut rows = Vec::new();
        for id in scope {
            let Some(entry) = self.models.get(id) else { continue };
            for m in &entry.models {
                let hit = needle.is_empty()
                    || m.id.to_lowercase().contains(&needle)
                    || m.display_name.as_deref().is_some_and(|n| n.to_lowercase().contains(&needle))
                    || id.to_lowercase().contains(&needle);
                if hit {
                    rows.push(RightRow::Model {
                        provider: id.to_owned(),
                        info: m.clone(),
                    });
                }
            }
        }
        // A typed id that is not listed can still be used (the provider may not list it).
        let typed = self.picker.search.trim();
        let custom_provider = self.picker_provider().or_else(|| self.current_provider.clone());
        if !typed.is_empty()
            && let Some(provider) = custom_provider
            && !rows.iter().any(|r| matches!(r, RightRow::Model { provider: p, info } if *p == provider && info.id == typed))
        {
            rows.push(RightRow::Custom {
                provider,
                id: typed.to_owned(),
            });
        }
        rows
    }

    /// `/model`: opens the picker; the provider and model lists are requested.
    pub fn open_picker(&mut self, provider: Option<String>, model: String) -> Vec<AppAction> {
        self.screen = Screen::Picker;
        self.return_to = Screen::Closed;
        self.current_provider = provider;
        self.current_model = model;
        self.picker = ModelPicker {
            auto_select: true,
            ..ModelPicker::default()
        };
        if self.providers_loaded {
            self.on_providers_loaded();
        }
        vec![AppAction::Catalog(CatalogAction::LoadProviders)]
    }

    /// Selects the current provider's row once (first time the provider list is known).
    pub(crate) fn on_providers_loaded(&mut self) {
        if self.screen == Screen::Picker && self.picker.auto_select {
            if let Some(cur) = self.current_provider.clone() {
                let rows = self.left_rows();
                if let Some(pos) = rows.iter().position(|r| {
                    matches!(r, LeftRow::Provider(i) if self.providers.get(*i).is_some_and(|p| p.id == cur))
                }) {
                    self.picker.left_sel = pos;
                    self.picker.left_off = scroll_window(pos, 0, self.rows(), rows.len());
                }
            }
            self.picker.auto_select = false;
        }
        self.clamp_picker();
    }

    /// Keeps the selections inside the lists.
    pub(crate) fn clamp_picker(&mut self) {
        let total = self.right_rows().len();
        if total == 0 {
            self.picker.model_sel = 0;
            self.picker.model_off = 0;
        } else {
            self.picker.model_sel = self.picker.model_sel.min(total - 1);
            self.picker.model_off =
                scroll_window(self.picker.model_sel, self.picker.model_off, self.rows(), total);
        }
    }

    /// Requests a background refresh of the selected provider once (cache is already shown).
    fn ensure_refreshed(&mut self) -> Vec<AppAction> {
        let Some(id) = self.picker_provider() else {
            return Vec::new();
        };
        let Some(row) = self.provider(&id) else {
            return Vec::new();
        };
        if !row.configured() || !row.discovery {
            return Vec::new();
        }
        let entry = self.models.entry(id.clone()).or_default();
        if entry.refresh_requested {
            return Vec::new();
        }
        entry.refresh_requested = true;
        entry.refreshing = true;
        vec![AppAction::Catalog(CatalogAction::Discover {
            provider: id,
            refresh: true,
        })]
    }

    fn force_refresh(&mut self) -> Vec<AppAction> {
        let Some(id) = self.picker_provider().or_else(|| self.current_provider.clone()) else {
            return Vec::new();
        };
        let Some(row) = self.provider(&id) else {
            return Vec::new();
        };
        if !row.configured() || !row.discovery {
            self.set_toast_internal("This provider has no model list to refresh", false);
            return Vec::new();
        }
        let entry = self.models.entry(id.clone()).or_default();
        entry.refresh_requested = true;
        entry.refreshing = true;
        vec![AppAction::Catalog(CatalogAction::Discover {
            provider: id,
            refresh: true,
        })]
    }

    fn move_left(&mut self, delta: isize) -> Vec<AppAction> {
        let rows = self.left_rows();
        let mut i = self.picker.left_sel as isize;
        loop {
            i += delta;
            if i < 0 || i as usize >= rows.len() {
                return Vec::new();
            }
            if rows[i as usize] != LeftRow::Separator {
                break;
            }
        }
        self.picker.left_sel = i as usize;
        self.picker.left_off = scroll_window(i as usize, self.picker.left_off, self.rows(), rows.len());
        self.picker.model_sel = 0;
        self.picker.model_off = 0;
        self.ensure_refreshed()
    }

    fn move_models(&mut self, delta: isize) {
        let total = self.right_rows().len();
        if total == 0 {
            return;
        }
        let next = self.picker.model_sel.saturating_add_signed(delta).min(total - 1);
        self.picker.model_sel = next;
        self.picker.model_off = scroll_window(next, self.picker.model_off, self.rows(), total);
    }

    /// Picks `provider`/`model`: selects it, or detours through the provider's key page.
    fn choose(&mut self, provider: String, model: String) -> Vec<AppAction> {
        let choice = ModelChoice {
            provider_changed: self.current_provider.as_deref() != Some(provider.as_str()),
            provider: provider.clone(),
            model,
        };
        let configured = self.provider(&provider).is_none_or(|p| p.configured());
        if configured {
            self.set_toast_internal(&format!("Switching to {}...", choice.model), false);
            self.screen = Screen::Closed;
            return vec![AppAction::Catalog(CatalogAction::SelectModel(choice))];
        }
        self.login.entry = KeyEntry::new(provider, Some(choice));
        self.return_to = Screen::Picker;
        self.screen = Screen::KeyEntry;
        Vec::new()
    }

    /// Enter on the provider pane for an unconfigured provider: just the key page.
    fn key_page_for_selected(&mut self) -> bool {
        let Some(id) = self.picker_provider() else {
            return false;
        };
        if self.provider(&id).is_some_and(|p| !p.configured()) {
            self.login.entry = KeyEntry::new(id, None);
            self.return_to = Screen::Picker;
            self.screen = Screen::KeyEntry;
            return true;
        }
        false
    }

    pub(crate) fn picker_key(&mut self, key: KeyEvent) -> Vec<AppAction> {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let page = self.rows().saturating_sub(1).max(1) as isize;
        match key.code {
            KeyCode::Esc => {
                if self.picker.search.is_empty() {
                    self.screen = Screen::Closed;
                } else {
                    self.picker.search.clear();
                    self.clamp_picker();
                }
            }
            KeyCode::Tab | KeyCode::BackTab => {
                self.picker.focus = match self.picker.focus {
                    Pane::Providers => Pane::Models,
                    Pane::Models => Pane::Providers,
                };
            }
            KeyCode::Left => self.picker.focus = Pane::Providers,
            KeyCode::Right => self.picker.focus = Pane::Models,
            KeyCode::Up => {
                return match self.picker.focus {
                    Pane::Providers => self.move_left(-1),
                    Pane::Models => {
                        self.move_models(-1);
                        Vec::new()
                    }
                };
            }
            KeyCode::Down => {
                return match self.picker.focus {
                    Pane::Providers => self.move_left(1),
                    Pane::Models => {
                        self.move_models(1);
                        Vec::new()
                    }
                };
            }
            KeyCode::PageUp => self.move_models(-page),
            KeyCode::PageDown => self.move_models(page),
            KeyCode::Home => self.move_models(-(self.picker.model_sel as isize)),
            KeyCode::End => self.move_models(isize::MAX / 2),
            KeyCode::Enter => return self.picker_enter(),
            KeyCode::Backspace => {
                self.picker.search.pop();
                self.picker.model_sel = 0;
                self.picker.model_off = 0;
                self.clamp_picker();
            }
            KeyCode::Char('r') if ctrl => return self.force_refresh(),
            KeyCode::Char('u') if ctrl => {
                self.picker.search.clear();
                self.clamp_picker();
            }
            KeyCode::Char('r')
                if !ctrl && self.picker.focus == Pane::Providers && self.picker.search.is_empty() =>
            {
                return self.force_refresh();
            }
            KeyCode::Char(c) if !ctrl && !key.modifiers.contains(KeyModifiers::ALT) => {
                self.picker.focus = Pane::Models;
                self.picker.search.push(c);
                self.picker.model_sel = 0;
                self.picker.model_off = 0;
                self.clamp_picker();
            }
            _ => {}
        }
        Vec::new()
    }

    fn picker_enter(&mut self) -> Vec<AppAction> {
        match self.picker.focus {
            Pane::Providers => {
                if self.key_page_for_selected() {
                    return Vec::new();
                }
                self.picker.focus = Pane::Models;
                Vec::new()
            }
            Pane::Models => {
                let rows = self.right_rows();
                match rows.get(self.picker.model_sel).cloned() {
                    Some(RightRow::Model { provider, info }) => self.choose(provider, info.id),
                    Some(RightRow::Custom { provider, id }) => self.choose(provider, id),
                    None => {
                        // Nothing listed: an unconfigured provider asks for its key.
                        self.key_page_for_selected();
                        Vec::new()
                    }
                }
            }
        }
    }
}

fn age_text(now_ms: u64, fetched_ms: u64) -> String {
    if fetched_ms == 0 {
        return "no cache".to_owned();
    }
    let secs = now_ms.saturating_sub(fetched_ms) / 1000;
    match secs {
        0..=59 => "cached just now".to_owned(),
        60..=3599 => format!("cached {} min ago", secs / 60),
        3600..=86399 => format!("cached {} h ago", secs / 3600),
        _ => format!("cached {} d ago", secs / 86400),
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

/// Draws the picker.
pub fn render(area: Rect, buf: &mut Buffer, ui: &CatalogUi, palette: &ThemePalette, icons: IconMode) {
    let w = area.width as usize;
    let cur_label = ui
        .current_provider
        .as_deref()
        .map(|p| ui.label_of(p))
        .unwrap_or_default();
    let now = if cur_label.is_empty() {
        ui.current_model.clone()
    } else {
        format!("{cur_label} / {}", ui.current_model)
    };
    title_bar(area, buf, &format!(" Select model   now: {now}"), palette, icons);

    let left_w: u16 = if area.width >= 100 {
        30
    } else if area.width >= 60 {
        24
    } else {
        18
    };
    let body_top = area.y + 1;
    let footer_y = area.y + area.height - 1;
    let detail_y = footer_y - 1;
    let sep_x = area.x + left_w;
    let right_x = sep_x + 1;
    let right_w = (area.x + area.width).saturating_sub(right_x) as usize;
    let list_top = body_top + 1;
    let visible = detail_y.saturating_sub(list_top) as usize;
    let dim = Style::default().fg(palette.dim).bg(palette.bg);
    let normal = Style::default().fg(palette.fg).bg(palette.bg);

    // Separator column.
    for y in body_top..detail_y {
        buf.set_string(sep_x, y, "│", Style::default().fg(palette.separator).bg(palette.bg));
    }

    // Left pane.
    let left_rows = ui.left_rows();
    let left_visible = detail_y.saturating_sub(body_top) as usize;
    let left_off = scroll_window(ui.picker.left_sel, ui.picker.left_off, left_visible, left_rows.len());
    for (i, row) in left_rows.iter().skip(left_off).take(left_visible).enumerate() {
        let y = body_top + i as u16;
        let idx = left_off + i;
        let selected = idx == ui.picker.left_sel;
        let (text, fg) = match row {
            LeftRow::All => (" All models".to_owned(), palette.fg),
            LeftRow::Separator => (" ─────────────".to_owned(), palette.separator),
            LeftRow::Provider(pi) => {
                let p = &ui.providers[*pi];
                let count = ui
                    .models
                    .get(&p.id)
                    .filter(|e| !e.models.is_empty())
                    .map(|e| format!(" ({})", e.models.len()))
                    .unwrap_or_default();
                if p.configured() {
                    (format!(" ● {}{count}", p.label), palette.fg)
                } else {
                    (format!(" ○ {}", p.label), palette.dim)
                }
            }
        };
        let mut style = Style::default().fg(fg).bg(palette.bg);
        if selected && ui.picker.focus == Pane::Providers {
            style = style.bg(palette.selection_bg).add_modifier(Modifier::BOLD);
        } else if selected {
            style = style.add_modifier(Modifier::BOLD).fg(palette.accent);
        }
        put(buf, area.x, y, left_w as usize, &text, style, icons);
    }
    if ui.providers.is_empty() {
        let msg = if ui.providers_loaded { "no providers" } else { " loading…" };
        put(buf, area.x, body_top + 1, left_w as usize, msg, dim, icons);
    }

    // Search line with the refresh state of the shown provider.
    let sel_provider = ui.picker_provider();
    let status = match sel_provider.as_deref().and_then(|p| ui.models.get(p)) {
        Some(e) if e.refreshing => "refreshing…".to_owned(),
        Some(e) if e.error.is_some() => format!("offline, {}", age_text(now_ms(), e.fetched_at_ms)),
        Some(e) => age_text(now_ms(), e.fetched_at_ms),
        None => String::new(),
    };
    let search = format!(
        " Search: {}{}",
        ui.picker.search,
        if ui.picker.focus == Pane::Models { "▏" } else { "" }
    );
    let status_w = status.chars().count() + 2;
    let search_w = right_w.saturating_sub(status_w);
    put(
        buf,
        right_x,
        body_top,
        search_w,
        &search,
        if ui.picker.focus == Pane::Models {
            Style::default().fg(palette.accent).bg(palette.bg)
        } else {
            normal
        },
        icons,
    );
    put(buf, right_x + search_w as u16, body_top, right_w - search_w, &format!(" {status}"), dim, icons);

    // Model list.
    let rows = ui.right_rows();
    let offset = scroll_window(ui.picker.model_sel, ui.picker.model_off, visible, rows.len());
    let list_w = right_w.saturating_sub(1);
    let all = sel_provider.is_none();
    for (i, row) in rows.iter().skip(offset).take(visible).enumerate() {
        let y = list_top + i as u16;
        let idx = offset + i;
        let (name, meta, is_current) = match row {
            RightRow::Model { provider, info } => {
                let name = if all {
                    format!("{provider}/{}", info.id)
                } else {
                    info.id.clone()
                };
                let mut meta = Vec::new();
                if let Some(c) = info.context_length {
                    meta.push(format!("ctx {}", fmt_context(c)));
                }
                if let (Some(a), Some(b)) = (info.price_in_per_mtok, info.price_out_per_mtok) {
                    meta.push(format!("{}/{}", fmt_price(a), fmt_price(b)));
                }
                let cur = ui.current_provider.as_deref() == Some(provider.as_str())
                    && ui.current_model == info.id;
                (name, meta.join("  "), cur)
            }
            RightRow::Custom { id, .. } => (format!("Use \"{id}\" as a custom model id"), String::new(), false),
        };
        let selected = idx == ui.picker.model_sel;
        let mut style = normal;
        if matches!(row, RightRow::Custom { .. }) {
            style = Style::default().fg(palette.accent).bg(palette.bg);
        }
        if selected && ui.picker.focus == Pane::Models {
            style = style.bg(palette.selection_bg).add_modifier(Modifier::BOLD);
        } else if selected {
            style = style.add_modifier(Modifier::UNDERLINED);
        }
        let marker = if is_current { "✓ " } else { "  " };
        let meta_w = meta.chars().count();
        let name_w = list_w.saturating_sub(meta_w + 1);
        let line = format!("{}{}", pad(&format!("{marker}{name}"), name_w), if meta.is_empty() { String::new() } else { format!(" {meta}") });
        put(buf, right_x, y, list_w, &line, style, icons);
    }
    if rows.is_empty() {
        let msg = empty_message(ui);
        put(buf, right_x, list_top, right_w, &format!(" {msg}"), dim, icons);
    }
    render_scrollbar(
        Rect::new(right_x + right_w as u16 - 1, list_top, 1, visible as u16),
        buf,
        rows.len(),
        offset,
        visible,
        palette,
    );

    // Detail line for the focused model.
    let detail = match rows.get(ui.picker.model_sel) {
        Some(RightRow::Model { provider, info }) => {
            let mut parts = vec![format!("{}/{}", ui.label_of(provider), info.id)];
            if let Some(n) = &info.display_name {
                parts.push(n.clone());
            }
            if let Some(c) = info.context_length {
                parts.push(format!("context {} tokens", fmt_context(c)));
            }
            if let (Some(a), Some(b)) = (info.price_in_per_mtok, info.price_out_per_mtok) {
                parts.push(format!("{} in / {} out per 1M tokens", fmt_price(a), fmt_price(b)));
            }
            if parts.len() == 1 {
                parts.push("no further metadata listed".to_owned());
            }
            parts.join(" · ")
        }
        Some(RightRow::Custom { provider, id }) => {
            format!("Use \"{id}\" on {} without checking the provider's list", ui.label_of(provider))
        }
        None => String::new(),
    };
    put(buf, area.x, detail_y, w, &format!(" {detail}"), Style::default().fg(palette.muted).bg(palette.bg), icons);

    let footer = if w >= 78 {
        " ⏎ select · ↑/↓ · type to search · Tab providers · ^R refresh · ⎋ close"
    } else {
        " ⏎ select · ↑/↓ · type to search · ⎋ close"
    };
    footer_bar(area, buf, footer, palette, icons);
}

fn empty_message(ui: &CatalogUi) -> String {
    let Some(id) = ui.picker_provider() else {
        return if ui.providers.iter().any(|p| p.configured()) {
            "No models yet: pick a provider on the left or type a model id.".to_owned()
        } else {
            "No provider has a key yet: pick one on the left to add it.".to_owned()
        };
    };
    let label = ui.label_of(&id);
    match ui.provider(&id) {
        Some(p) if !p.configured() => format!("No API key for {label}. Press ⏎ on the provider to add one."),
        _ => match ui.models.get(&id) {
            Some(e) if e.refreshing => "Loading models…".to_owned(),
            Some(e) => match &e.error {
                Some(err) => format!("Could not list models: {err}. Type an id to use it anyway."),
                None => "No models listed. Type an id to use it anyway.".to_owned(),
            },
            None => "Loading models…".to_owned(),
        },
    }
}
