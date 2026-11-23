//! Cross-screen interaction: the `?` / `F1` help overlay, the `Ctrl+K` command palette and the
//! globally bound keys, all driven by the action table in [`crate::keybindings`].

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;

use crate::core::hitmap::{HitTarget, ScrollId};
use crate::core::overlay::{OverlayEntry, OverlayId};
use crate::core::theme::render_dimmed_backdrop;
use crate::core::viewport::scroll_offset_for_row;
use crate::keybindings::{ACTIONS, KeyMap};
use crate::palette::{Palette, PaletteItem, PaletteOutcome, PaletteTarget, render_palette};
use crate::settings::{RowsCtx, all_settings_rows};
use crate::state::{AppAction, HitRegions, TuiAppState};
use crate::theme::ThemePalette;
use crate::ui_util::{ListRow, boxed, centered, render_list, trunc};
use crate::widgets::scrollbar::render_vertical_scrollbar;

/// State of the help overlay.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HelpState {
    pub screen: String,
    pub sel: usize,
    pub off: usize,
}

/// Keymap, help overlay and palette.
#[derive(Debug, Clone, Default)]
pub struct UxState {
    pub keymap: KeyMap,
    pub help: Option<HelpState>,
    pub palette: Option<Palette>,
}

/// `(key, description)` lines of the help overlay for `screen`, from the action table with the
/// keys currently bound.
#[must_use]
pub fn help_entries(screen: &str, keymap: &KeyMap) -> Vec<(String, String)> {
    let mut screens = vec!["global", screen];
    if screen == "settings" {
        screens.extend(["list", "editor"]);
    }
    ACTIONS
        .iter()
        .filter(|d| screens.contains(&d.screen))
        .map(|d| {
            let key = if d.rebindable { KeyMap::pretty(&keymap.key_of(d.id)) } else { KeyMap::pretty(d.default_key) };
            (key, d.description.to_owned())
        })
        .collect()
}

impl TuiAppState {
    /// Screen whose keys the help overlay lists.
    fn help_screen(&self) -> &'static str {
        if self.settings.open {
            "settings"
        } else {
            "chat"
        }
    }

    /// Whether a text field has the keyboard (so `?` is a character, not help).
    fn text_input_active(&self) -> bool {
        if self.settings.open {
            return self.settings.editing.is_some()
                || self.settings.selector.is_some()
                || self.settings.items.is_some()
                || !self.settings.search_query.is_empty();
        }
        !self.composer.text.is_empty()
            || self.log.open
            || self.catalog.is_open()
            || self.gov.is_open()
            || self.wizard.is_some()
    }

    pub(crate) fn help_viewport_rows(&self) -> usize {
        let bx_h = 24u16.min(self.last_viewport.height);
        // boxed() takes 2 border rows, plus 1 bottom hint row.
        bx_h.saturating_sub(3).max(1) as usize
    }

    /// Opens the help overlay.
    pub fn open_help(&mut self) {
        self.ux.help = Some(HelpState { screen: self.help_screen().to_owned(), sel: 0, off: 0 });
        self.overlays.raise_or_push(OverlayEntry {
            id: OverlayId::Help,
            blocks_chat: true,
            dim_backdrop: true,
            sub_input_active: false,
        });
    }

    /// Opens the palette over commands, settings rows and chats.
    pub fn open_palette(&mut self) {
        let mut items = Vec::new();
        for c in self.available_slash_commands() {
            items.push(PaletteItem {
                kind: "cmd ",
                label: format!("/{}", c.name),
                detail: c.description.clone(),
                target: PaletteTarget::Command(c.name.clone()),
            });
        }
        let providers: Vec<crate::settings::ProviderSummary> = Vec::new();
        let bindings = self.ux.keymap.rebindable_entries();
        let ctx = RowsCtx {
            prefs: &self.prefs,
            cfg: &self.gateway_cfg,
            paths: &self.paths,
            secrets: &self.secrets_summary,
            valve_toggles: &self.settings.valve_toggles,
            plugins: &self.plugins,
            providers: &providers,
            bindings: &bindings,
            model_name: &self.model_name,
        };
        for (tab, row) in all_settings_rows(&ctx) {
            if matches!(row.value, crate::settings::SettingValueKind::ReadOnly(_)) {
                continue;
            }
            items.push(PaletteItem {
                kind: "set ",
                label: format!("{} > {}", tab.label(), row.label),
                detail: row.description.clone(),
                target: PaletteTarget::Setting { tab, id: row.id.clone() },
            });
        }
        for chat in &self.chats.chats {
            items.push(PaletteItem {
                kind: "chat",
                label: chat.title.clone(),
                detail: chat.place_id.clone(),
                target: PaletteTarget::Chat(chat.id.clone()),
            });
        }
        let mut palette = Palette::new(items);
        palette.view_rows = (20u16.min(self.last_viewport.height)).saturating_sub(4).max(1) as usize;
        self.ux.palette = Some(palette);
        self.overlays.raise_or_push(OverlayEntry {
            id: OverlayId::Palette,
            blocks_chat: true,
            dim_backdrop: true,
            sub_input_active: true,
        });
    }

    fn run_palette_target(&mut self, target: PaletteTarget) -> Vec<AppAction> {
        match target {
            PaletteTarget::Command(name) => {
                self.settings.close();
                self.composer.text = format!("/{name}");
                self.composer.cursor = self.composer.text.len();
                self.submit_composer()
            }
            PaletteTarget::Setting { tab, id } => self.open_settings_row(tab, &id),
            PaletteTarget::Chat(id) => {
                self.settings.close();
                if self.chats.chats.iter().any(|c| c.id == id) {
                    self.chats.active_chat_id = id;
                    self.persist_chats();
                    self.auto_scroll = true;
                }
                Vec::new()
            }
        }
    }

    /// Globally bound keys and the help / palette overlays. `None` = not handled here.
    pub(crate) fn global_key(&mut self, key: KeyEvent) -> Option<Vec<AppAction>> {
        if self.ux.help.is_some() {
            let visible = self.help_viewport_rows();
            if let Some(h) = &mut self.ux.help {
                let n = help_entries(&h.screen, &self.ux.keymap).len();
                let before = h.sel;
                let mut close = false;
                match key.code {
                    KeyCode::Esc | KeyCode::Enter | KeyCode::Char('?' | 'q') | KeyCode::F(1) => close = true,
                    KeyCode::Up => h.sel = h.sel.saturating_sub(1),
                    KeyCode::Down => h.sel = (h.sel + 1).min(n.saturating_sub(1)),
                    KeyCode::PageUp => h.sel = h.sel.saturating_sub(visible.max(1)),
                    KeyCode::PageDown => h.sel = (h.sel + visible.max(1)).min(n.saturating_sub(1)),
                    _ => {}
                }
                if h.sel != before {
                    h.off = scroll_offset_for_row(h.sel, h.off, visible, n);
                }
                if close {
                    self.ux.help = None;
                    self.overlays.remove(OverlayId::Help);
                }
            }
            return Some(Vec::new());
        }
        if let Some(p) = &mut self.ux.palette {
            return Some(match p.handle_key(key) {
                PaletteOutcome::None => Vec::new(),
                PaletteOutcome::Close => {
                    self.ux.palette = None;
                    self.overlays.remove(OverlayId::Palette);
                    Vec::new()
                }
                PaletteOutcome::Run(target) => {
                    self.ux.palette = None;
                    self.overlays.remove(OverlayId::Palette);
                    self.run_palette_target(target)
                }
            });
        }
        let km = &self.ux.keymap;
        if km.matches("app.suspend", &key) {
            return Some(vec![AppAction::Suspend]);
        }
        if km.matches("app.reset_display", &key) {
            return Some(vec![AppAction::ResetDisplay]);
        }
        if km.matches("ui.palette", &key) {
            self.open_palette();
            return Some(Vec::new());
        }
        if km.matches("ui.help", &key) {
            self.open_help();
            return Some(Vec::new());
        }
        if key.code == KeyCode::Char('?')
            && !key.modifiers.contains(KeyModifiers::CONTROL)
            && !self.text_input_active()
        {
            self.open_help();
            return Some(Vec::new());
        }
        if km.matches("ui.sidebar", &key) {
            self.toggle_sidebar(120);
            return Some(Vec::new());
        }
        None
    }

    /// Mouse on the overlays of this module and the settings sub-views. `None` = not handled.
    pub(crate) fn ux_mouse(&mut self, mouse: MouseEvent, hit: &HitRegions) -> Option<Vec<AppAction>> {
        let inside = |r: Rect| {
            mouse.column >= r.x && mouse.column < r.x + r.width && mouse.row >= r.y && mouse.row < r.y + r.height
        };
        let wheel = match mouse.kind {
            MouseEventKind::ScrollUp => Some(-1isize),
            MouseEventKind::ScrollDown => Some(1),
            _ => None,
        };
        let clicked_row = || {
            if let Some(HitTarget::ListRow(idx)) = hit.hitmap.hit_test(mouse.column, mouse.row) {
                return Some(*idx);
            }
            hit.overlay_rows
                .iter()
                .find(|(r, _)| inside(*r))
                .map(|(_, i)| *i)
        };
        let left = matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left));
        if self.ux.help.is_some() {
            let visible = if hit.overlay_area.height > 0 {
                hit.overlay_area.height as usize
            } else {
                self.help_viewport_rows()
            };
            if let Some(h) = &mut self.ux.help {
                let total = help_entries(&h.screen, &self.ux.keymap).len();
                if let Some(d) = wheel {
                    if total > 0 {
                        h.sel = (h.sel as isize + d * 3).clamp(0, (total - 1) as isize) as usize;
                        h.off = scroll_offset_for_row(h.sel, h.off, visible, total);
                    }
                } else if left && let Some(i) = clicked_row() && total > 0 {
                    h.sel = i.min(total - 1);
                    h.off = scroll_offset_for_row(h.sel, h.off, visible, total);
                }
            }
            return Some(Vec::new());
        }
        if self.ux.palette.is_some() {
            let mut run = None;
            if let Some(p) = &mut self.ux.palette {
                if hit.overlay_area.height > 0 {
                    p.view_rows = hit.overlay_area.height as usize;
                }
                if let Some(d) = wheel {
                    p.move_by(d * 3);
                } else if left && let Some(i) = clicked_row() {
                    p.sel = i;
                    run = p.filtered().get(i).map(|it| it.target.clone());
                }
            }
            if let Some(t) = run {
                self.ux.palette = None;
                self.overlays.remove(OverlayId::Palette);
                return Some(self.run_palette_target(t));
            }
            return Some(Vec::new());
        }
        if !self.settings.open {
            return None;
        }
        if let Some(sel) = &mut self.settings.selector {
            if let Some(d) = wheel {
                sel.move_by(d * 3);
            } else if left && let Some(i) = clicked_row() {
                sel.select_row(i);
                return Some(self.selector_key(KeyEvent::from(KeyCode::Enter)));
            }
            return Some(Vec::new());
        }
        if let Some(ed) = &mut self.settings.items {
            if let Some(d) = wheel {
                ed.sel = (ed.sel as isize + d * 3).clamp(0, ed.items.len().saturating_sub(1) as isize) as usize;
                ed.off = crate::render::scroll_window(ed.sel, ed.off, ed.view_rows, ed.items.len());
            } else if left && let Some(i) = clicked_row() {
                ed.sel = i;
            }
            return Some(Vec::new());
        }
        None
    }
}

/// Draws the help overlay with backdrop clearing and `HitMap` recording.
pub fn render_help_overlay(
    area: Rect,
    buf: &mut Buffer,
    h: &HelpState,
    state: &TuiAppState,
    palette: &ThemePalette,
    hit: &mut HitRegions,
) {
    render_dimmed_backdrop(buf, area, palette);
    hit.hitmap.record(area, HitTarget::Backdrop(OverlayId::Help));
    let bx = centered(78.min(area.width), 24.min(area.height), area);
    let inner = boxed(bx, buf, &format!("Keys: {} (Esc closes)", h.screen), palette);
    hit.hitmap.record(bx, HitTarget::OverlayBody(OverlayId::Help));
    if inner.height < 2 || inner.width < 4 {
        return;
    }
    let entries = help_entries(&h.screen, &state.ux.keymap);
    let rows: Vec<ListRow> = entries
        .iter()
        .map(|(k, d)| ListRow { left: k.clone(), right: d.clone(), dim: false })
        .collect();
    let list = Rect::new(inner.x, inner.y, inner.width, inner.height.saturating_sub(1));
    let mut hits = Vec::new();
    let sel = h.sel.min(rows.len().saturating_sub(1));
    let off = render_list(list, buf, &rows, sel, h.off, palette, &mut hits);
    for (rect, idx) in &hits {
        hit.hitmap.record(*rect, HitTarget::ListRow(*idx));
    }
    let visible = list.height as usize;
    if rows.len() > visible && list.width > 2 {
        let track = Rect::new(list.x + list.width - 1, list.y, 1, list.height);
        render_vertical_scrollbar(
            buf,
            track,
            ScrollId::Help,
            rows.len(),
            visible,
            off,
            &mut hit.hitmap,
            Style::default().fg(palette.separator).bg(palette.modal_bg),
            Style::default().fg(palette.accent).bg(palette.modal_bg),
        );
    }
    hit.overlay_area = list;
    hit.overlay_rows = hits;
    hit.overlay_total = rows.len();
    hit.overlay_offset = off;
    buf.set_string(
        inner.x + 1,
        inner.y + inner.height - 1,
        trunc("Rebind keys in /settings > Appearance > Key bindings (keybindings.toml)", inner.width.saturating_sub(2) as usize),
        Style::default().fg(palette.dim).bg(palette.modal_bg),
    );
}

/// Draws the help overlay and the palette.
pub fn render_ux_overlays(area: Rect, buf: &mut Buffer, state: &TuiAppState, palette: &ThemePalette, hit: &mut HitRegions) {
    if let Some(h) = &state.ux.help {
        render_help_overlay(area, buf, h, state, palette, hit);
    }
    if let Some(p) = &state.ux.palette {
        render_palette(area, buf, p, palette, hit);
    }
}
