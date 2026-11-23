//! Enum selector (omp pattern): name, description, optional live preview, option list with the
//! current value marked, a scrollbar and a "Type to search" line. Enter picks, Esc cancels.

use std::cell::Cell;

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use crate::core::hitmap::{HitMap, HitTarget, ScrollId};
use crate::core::input::{PasteTarget, TextInput};
use crate::core::overlay::{OverlayId, WidgetOutcome, is_key_press};
use crate::core::theme::{ThemePalette, render_dimmed_backdrop};
use crate::core::viewport::scroll_offset_for_row;
use crate::state::HitRegions;
use crate::ui_util::{boxed, centered, fill, trunc};
use crate::widgets::scrollbar::render_vertical_scrollbar;

/// One selectable value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectOption {
    pub value: String,
    pub label: String,
    pub description: String,
}

impl SelectOption {
    /// Option whose label equals its value.
    #[must_use]
    pub fn plain(value: &str, description: &str) -> Self {
        Self {
            value: value.to_owned(),
            label: value.to_owned(),
            description: description.to_owned(),
        }
    }
}

/// What a key did to the selector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectorOutcome {
    None,
    Picked(String),
    Cancel,
}

/// Open selector state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selector {
    /// Row id of the setting being edited (opaque to the widget).
    pub target: String,
    pub name: String,
    pub description: String,
    pub options: Vec<SelectOption>,
    pub current: String,
    pub search: String,
    pub search_input: TextInput,
    pub sel: usize,
    pub off: usize,
    pub view_rows: usize,
    pub rendered_rows: Cell<usize>,
}

impl Selector {
    /// Opens the selector with the current value highlighted.
    #[must_use]
    pub fn new(
        target: &str,
        name: &str,
        description: &str,
        options: Vec<SelectOption>,
        current: &str,
    ) -> Self {
        let sel = options.iter().position(|o| o.value == current).unwrap_or(0);
        Self {
            target: target.to_owned(),
            name: name.to_owned(),
            description: description.to_owned(),
            options,
            current: current.to_owned(),
            search: String::new(),
            search_input: TextInput::new(false).with_placeholder("type to search…"),
            sel,
            off: 0,
            view_rows: 8,
            rendered_rows: Cell::new(0),
        }
    }

    /// Real viewport height observed on the last render frame (falls back to `view_rows`).
    #[must_use]
    pub fn effective_view_rows(&self) -> usize {
        let r = self.rendered_rows.get();
        if r > 0 { r } else { self.view_rows.max(1) }
    }

    fn sync_input_from_search(&mut self) {
        if self.search_input.as_str() != self.search {
            self.search_input.set_value(self.search.clone());
        }
    }

    /// Pastes sanitized text into the selector search box.
    pub fn paste_text(&mut self, raw: &str) -> bool {
        self.sync_input_from_search();
        if self.search_input.paste_text(raw) {
            self.search = self.search_input.value.clone();
            self.sel = 0;
            self.clamp();
            true
        } else {
            false
        }
    }

    /// Indices of the options matching the search text (case-insensitive substring of label,
    /// value or description; prefix matches first).
    #[must_use]
    pub fn filtered(&self) -> Vec<usize> {
        let q = self.search.trim().to_lowercase();
        if q.is_empty() {
            return (0..self.options.len()).collect();
        }
        let hit = |o: &SelectOption| {
            o.label.to_lowercase().contains(&q)
                || o.value.to_lowercase().contains(&q)
                || o.description.to_lowercase().contains(&q)
        };
        let mut idx: Vec<usize> = (0..self.options.len())
            .filter(|i| hit(&self.options[*i]))
            .collect();
        idx.sort_by_key(|i| {
            let o = &self.options[*i];
            let (l, v) = (o.label.to_lowercase(), o.value.to_lowercase());
            if l.starts_with(&q) || v.starts_with(&q) {
                0
            } else if l.contains(&q) || v.contains(&q) {
                1
            } else {
                2
            }
        });
        idx
    }

    /// The option under the cursor.
    #[must_use]
    pub fn highlighted(&self) -> Option<&SelectOption> {
        self.filtered().get(self.sel).map(|i| &self.options[*i])
    }

    fn clamp(&mut self) {
        let n = self.filtered().len();
        self.sel = self.sel.min(n.saturating_sub(1));
        self.off = scroll_offset_for_row(self.sel, self.off, self.effective_view_rows(), n);
    }

    /// Moves the cursor by `delta` rows (no wrap).
    pub fn move_by(&mut self, delta: isize) {
        let n = self.filtered().len() as isize;
        if n == 0 {
            return;
        }
        self.sel = (self.sel as isize + delta).clamp(0, n - 1) as usize;
        self.clamp();
    }

    /// Moves the cursor to the filtered row `row`.
    pub fn select_row(&mut self, row: usize) {
        self.sel = row;
        self.clamp();
    }

    /// Handles one key.
    pub fn handle_key(&mut self, key: KeyEvent) -> SelectorOutcome {
        if !is_key_press(key) {
            return SelectorOutcome::None;
        }
        let page = self.effective_view_rows() as isize;
        match key.code {
            KeyCode::Esc => return SelectorOutcome::Cancel,
            KeyCode::Enter => {
                return self.highlighted().map_or(SelectorOutcome::Cancel, |o| {
                    SelectorOutcome::Picked(o.value.clone())
                });
            }
            KeyCode::Up => self.move_by(-1),
            KeyCode::Down => self.move_by(1),
            KeyCode::PageUp => self.move_by(-page),
            KeyCode::PageDown => self.move_by(page),
            KeyCode::Home if self.search.is_empty() => self.move_by(isize::MIN / 2),
            KeyCode::End if self.search.is_empty() => self.move_by(isize::MAX / 2),
            _ => {
                self.sync_input_from_search();
                let before = self.search_input.value.clone();
                if self.search_input.handle_key(key) == WidgetOutcome::Consumed {
                    self.search = self.search_input.value.clone();
                    if self.search != before {
                        self.sel = 0;
                        self.clamp();
                    }
                }
            }
        }
        SelectorOutcome::None
    }
}

/// Draws the selector over `area` and records hit rects in both `hit` and `hitmap`.
pub fn render_selector_with_hitmap(
    area: Rect,
    buf: &mut Buffer,
    sel: &Selector,
    palette: &ThemePalette,
    hit: &mut HitRegions,
    hitmap: &mut HitMap,
) {
    render_dimmed_backdrop(buf, area, palette);
    hitmap.record(area, HitTarget::Backdrop(OverlayId::Settings));

    let box_area = centered(72.min(area.width), 22.min(area.height), area);
    let inner = boxed(box_area, buf, &format!("Select {}", sel.name), palette);
    hitmap.record(box_area, HitTarget::OverlayBody(OverlayId::Settings));
    if inner.height < 4 || inner.width < 16 {
        return;
    }

    let muted = Style::default().fg(palette.muted).bg(palette.modal_bg);
    let dim = Style::default().fg(palette.dim).bg(palette.modal_bg);
    let has_preview = inner.height >= 7;
    let header_rows: u16 = if has_preview { 3 } else { 1 };

    buf.set_string(
        inner.x + 1,
        inner.y,
        trunc(&sel.description, inner.width.saturating_sub(2) as usize),
        muted,
    );
    if has_preview {
        let preview = sel.highlighted().map_or_else(String::new, |o| {
            if o.description.is_empty() {
                format!("Value: {}", o.value)
            } else {
                format!("{} - {}", o.label, o.description)
            }
        });
        buf.set_string(
            inner.x + 1,
            inner.y + 1,
            trunc(
                &format!("Preview: {preview}"),
                inner.width.saturating_sub(2) as usize,
            ),
            Style::default().fg(palette.accent).bg(palette.modal_bg),
        );
    }

    // Reserve 2 bottom rows when space allows: 1 for Search, 1 for bounded footer hint.
    let bottom_rows: u16 = if inner.height >= header_rows + 3 {
        2
    } else {
        1
    };
    let list_h = inner.height.saturating_sub(header_rows + bottom_rows);
    let list_area = Rect::new(inner.x, inner.y + header_rows, inner.width, list_h);
    sel.rendered_rows.set(usize::from(list_h).max(1));

    let filtered = sel.filtered();
    let total = filtered.len();
    let visible = usize::from(list_area.height);
    let off = scroll_offset_for_row(sel.sel, sel.off, visible.max(1), total);
    let has_sb = total > visible && list_area.width > 3;
    let content_w = if has_sb {
        list_area.width - 1
    } else {
        list_area.width
    };
    let left_w = ((content_w as usize) * 2 / 5)
        .max(10)
        .min(content_w as usize);

    hit.overlay_area = list_area;
    hit.overlay_rows.clear();
    hit.overlay_total = total;
    hit.overlay_offset = off;

    for (vis_idx, &opt_idx) in filtered.iter().skip(off).take(visible).enumerate() {
        let abs_idx = off + vis_idx;
        let y = list_area.y + vis_idx as u16;
        let row_rect = Rect::new(list_area.x, y, content_w, 1);
        hit.overlay_rows.push((row_rect, abs_idx));
        hitmap.record(row_rect, HitTarget::ListRow(abs_idx));

        let o = &sel.options[opt_idx];
        let cur = if o.value == sel.current {
            " (current)"
        } else {
            ""
        };
        let is_sel = abs_idx == sel.sel;
        let bg = if is_sel {
            palette.selection_bg
        } else {
            palette.modal_bg
        };
        fill(buf, row_rect, bg, palette.fg);

        let marker = if is_sel { "› " } else { "  " };
        let left_raw = format!("{}{cur}", o.label);
        let left = format!("{marker}{}", trunc(&left_raw, left_w.saturating_sub(3)));
        let left = format!("{left:<left_w$}");
        let fg = if is_sel { palette.accent } else { palette.fg };
        let right = trunc(
            &o.description,
            (content_w as usize).saturating_sub(left_w + 1),
        );
        let line = Line::from(vec![
            Span::styled(left, Style::default().fg(fg).bg(bg)),
            Span::styled(right, Style::default().fg(palette.muted).bg(bg)),
        ]);
        buf.set_line(list_area.x, y, &line, content_w);
    }

    if has_sb && list_area.height > 0 {
        let track = Rect::new(
            list_area.x + list_area.width - 1,
            list_area.y,
            1,
            list_area.height,
        );
        render_vertical_scrollbar(
            buf,
            track,
            ScrollId::SettingsList,
            total,
            visible,
            off,
            hitmap,
            Style::default().fg(palette.dim).bg(palette.modal_bg),
            Style::default().fg(palette.accent).bg(palette.modal_bg),
        );
    }

    if filtered.is_empty() && list_area.height > 0 {
        buf.set_string(list_area.x + 2, list_area.y, "No match", muted);
    }

    let search_y = if bottom_rows == 2 {
        inner.y + inner.height - 2
    } else {
        inner.y + inner.height - 1
    };
    let label_prefix = " ? Search: ";
    let prefix_w = UnicodeWidthStr::width(label_prefix);
    let input_w = usize::from(inner.width).saturating_sub(prefix_w + 1).max(1);
    let (vis_search, _) = if sel.search_input.as_str() == sel.search {
        sel.search_input.visible_slice(input_w)
    } else {
        let tmp = TextInput::new(false).with_value(sel.search.clone());
        tmp.visible_slice(input_w)
    };
    let search_line = Line::from(vec![
        Span::styled(
            label_prefix,
            Style::default()
                .fg(palette.accent)
                .bg(palette.modal_bg)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("{vis_search}▏"),
            Style::default().fg(palette.fg).bg(palette.modal_bg),
        ),
    ]);
    buf.set_line(inner.x, search_y, &search_line, inner.width);

    if bottom_rows == 2 {
        let hint_y = inner.y + inner.height - 1;
        let hint = trunc(
            " ↑/↓ select · Enter pick · Ctrl+V paste · Esc cancel",
            usize::from(inner.width),
        );
        buf.set_string(inner.x, hint_y, hint, dim);
    }
}

/// Draws the selector over `area` and records the row hit rects.
pub fn render_selector(
    area: Rect,
    buf: &mut Buffer,
    sel: &Selector,
    palette: &ThemePalette,
    hit: &mut HitRegions,
) {
    let mut hitmap = HitMap::default();
    render_selector_with_hitmap(area, buf, sel, palette, hit, &mut hitmap);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts() -> Vec<SelectOption> {
        [
            "alpha",
            "beta",
            "gamma",
            "max_tokens",
            "max_completion_tokens",
        ]
        .iter()
        .map(|v| SelectOption::plain(v, ""))
        .collect()
    }

    fn k(c: KeyCode) -> KeyEvent {
        KeyEvent::new(c, crossterm::event::KeyModifiers::NONE)
    }

    #[test]
    fn search_and_pick() {
        let mut s = Selector::new("x", "X", "", opts(), "beta");
        assert_eq!(s.sel, 1, "current value is highlighted first");
        s.handle_key(k(KeyCode::Char('m')));
        s.handle_key(k(KeyCode::Char('a')));
        assert_eq!(
            s.filtered().len(),
            3,
            "gamma also contains \"ma\"; prefix matches sort first"
        );
        assert_eq!(
            s.handle_key(k(KeyCode::Enter)),
            SelectorOutcome::Picked("max_tokens".into())
        );
    }

    #[test]
    fn esc_cancels_and_navigation_clamps() {
        let mut s = Selector::new("x", "X", "", opts(), "alpha");
        s.move_by(-3);
        assert_eq!(s.sel, 0);
        s.move_by(99);
        assert_eq!(s.sel, 4);
        assert_eq!(s.handle_key(k(KeyCode::Esc)), SelectorOutcome::Cancel);
    }
}
