//! Generic key-stabilized selectable menu list with dynamic viewport sizing,
//! heading skipping, 2-step confirmation, hit-testing, and vertical scrollbar.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::core::hitmap::{HitMap, HitTarget, ScrollId};
use crate::core::theme::ThemePalette;
use crate::core::viewport::{clamp_scroll_offset, scroll_offset_for_row};
use crate::widgets::scrollbar::render_vertical_scrollbar;

/// Single row inside a [`MenuSelection`] list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuItem<T> {
    pub key: String,
    pub label: String,
    pub secondary: Option<String>,
    pub selectable: bool,
    pub data: T,
}

/// Key-stabilized selection and scroll state for a vertical menu list.
#[derive(Debug, Clone)]
pub struct MenuSelection<T> {
    pub items: Vec<MenuItem<T>>,
    pub selected_key: Option<String>,
    pub offset: usize,
    pub viewport_rows: usize,
    pub pending_confirm: Option<String>,
}

impl<T> MenuSelection<T> {
    /// Creates a new menu selection, pre-selecting the first selectable item if one exists.
    #[must_use]
    pub fn new(items: Vec<MenuItem<T>>) -> Self {
        let selected_key = items
            .iter()
            .find(|item| item.selectable)
            .map(|item| item.key.clone());
        Self {
            items,
            selected_key,
            offset: 0,
            viewport_rows: 0,
            pending_confirm: None,
        }
    }

    /// Replaces items while keeping `selected_key` if still present and selectable;
    /// otherwise falls back to the first selectable item and clamps `offset`.
    pub fn update_items(&mut self, items: Vec<MenuItem<T>>) {
        let preserved = self.selected_key.as_ref().is_some_and(|key| {
            items
                .iter()
                .any(|item| item.selectable && &item.key == key)
        });
        if !preserved {
            self.selected_key = items
                .iter()
                .find(|item| item.selectable)
                .map(|item| item.key.clone());
        }

        if let Some(pending) = &self.pending_confirm
            && !items
                .iter()
                .any(|item| item.selectable && &item.key == pending)
        {
            self.pending_confirm = None;
        }

        self.items = items;
        self.offset = clamp_scroll_offset(self.offset, self.items.len(), self.viewport_rows);
    }

    /// Returns the index of the currently selected item, if present and selectable.
    #[must_use]
    pub fn selected_index(&self) -> Option<usize> {
        let key = self.selected_key.as_ref()?;
        self.items
            .iter()
            .position(|item| item.selectable && &item.key == key)
    }

    /// Returns a reference to the currently selected item, if any.
    #[must_use]
    pub fn selected_item(&self) -> Option<&MenuItem<T>> {
        self.selected_index().and_then(|idx| self.items.get(idx))
    }

    /// Moves selection forward to the next selectable item, skipping headings and clearing
    /// any armed confirmation when selection changes.
    pub fn select_next(&mut self) {
        let next_idx = match self.selected_index() {
            Some(cur) => ((cur + 1)..self.items.len()).find(|&i| self.items[i].selectable),
            None => (0..self.items.len()).find(|&i| self.items[i].selectable),
        };
        if let Some(idx) = next_idx {
            self.selected_key = Some(self.items[idx].key.clone());
            self.pending_confirm = None;
            self.offset =
                scroll_offset_for_row(idx, self.offset, self.viewport_rows, self.items.len());
        }
    }

    /// Moves selection backward to the previous selectable item, skipping headings and clearing
    /// any armed confirmation when selection changes.
    pub fn select_prev(&mut self) {
        let prev_idx = match self.selected_index() {
            Some(cur) => (0..cur).rev().find(|&i| self.items[i].selectable),
            None => (0..self.items.len())
                .rev()
                .find(|&i| self.items[i].selectable),
        };
        if let Some(idx) = prev_idx {
            self.selected_key = Some(self.items[idx].key.clone());
            self.pending_confirm = None;
            self.offset =
                scroll_offset_for_row(idx, self.offset, self.viewport_rows, self.items.len());
        }
    }

    /// Selects item at `idx` if it exists and is selectable.
    pub fn select_index(&mut self, idx: usize) -> bool {
        let Some(item) = self.items.get(idx) else {
            return false;
        };
        if !item.selectable {
            return false;
        }
        if self.selected_key.as_deref() != Some(item.key.as_str()) {
            self.pending_confirm = None;
        }
        self.selected_key = Some(item.key.clone());
        self.offset =
            scroll_offset_for_row(idx, self.offset, self.viewport_rows, self.items.len());
        true
    }

    /// Scrolls the viewport offset by `delta` rows, clamping safely without overflow.
    pub fn scroll_by(&mut self, delta: isize) {
        let candidate = self.offset.saturating_add_signed(delta);
        self.offset = clamp_scroll_offset(candidate, self.items.len(), self.viewport_rows);
    }

    /// Two-step confirmation for destructive actions: the first call arms `key` and returns
    /// `false`; a second call with the same `key` clears `pending_confirm` and returns `true`.
    pub fn arm_or_confirm(&mut self, key: &str) -> bool {
        if self.pending_confirm.as_deref() == Some(key) {
            self.pending_confirm = None;
            true
        } else {
            self.pending_confirm = Some(key.to_owned());
            false
        }
    }

    /// Clears any pending 2-step confirmation.
    pub fn cancel_confirm(&mut self) {
        self.pending_confirm = None;
    }
}

/// Visual and hit-tracking configuration for [`render_menu_list`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MenuRenderConfig {
    pub scroll_id: ScrollId,
    pub show_pointer: bool,
    pub secondary_right_aligned: bool,
}

/// Renders `state` inside `area`, updates `state.viewport_rows` dynamically from `area.height`,
/// records [`HitTarget::ListRow`] for selectable rows, and draws a vertical scrollbar when needed.
pub fn render_menu_list<T>(
    area: Rect,
    buf: &mut Buffer,
    state: &mut MenuSelection<T>,
    palette: &ThemePalette,
    hitmap: &mut HitMap,
    cfg: MenuRenderConfig,
) {
    state.viewport_rows = usize::from(area.height);
    if area.width == 0 || area.height == 0 {
        return;
    }

    state.offset = clamp_scroll_offset(state.offset, state.items.len(), state.viewport_rows);

    let has_scrollbar = state.items.len() > state.viewport_rows && area.width > 4;
    let content_w = if has_scrollbar {
        area.width - 1
    } else {
        area.width
    };
    let sel_idx = state.selected_index();

    for vis_row in 0..state.viewport_rows {
        let abs_idx = state.offset + vis_row;
        let y = area.y + vis_row as u16;
        let row_rect = Rect::new(area.x, y, content_w, 1);

        let Some(item) = state.items.get(abs_idx) else {
            fill_row(
                buf,
                row_rect,
                Style::default().fg(palette.fg).bg(palette.modal_bg),
            );
            continue;
        };

        if !item.selectable {
            let heading_style = Style::default()
                .fg(palette.accent)
                .bg(palette.modal_bg)
                .add_modifier(Modifier::BOLD);
            fill_row(buf, row_rect, heading_style);
            let text = truncate_to_width(&item.label, usize::from(content_w));
            buf.set_line(
                area.x,
                y,
                &Line::from(Span::styled(text, heading_style)),
                content_w,
            );
            continue;
        }

        let is_sel = sel_idx == Some(abs_idx);
        let is_pending = state.pending_confirm.as_deref() == Some(item.key.as_str());
        let row_bg = if is_sel {
            palette.selection_bg
        } else {
            palette.modal_bg
        };
        let label_fg = if is_pending {
            palette.status_red
        } else if is_sel {
            palette.accent
        } else {
            palette.fg
        };
        let label_style = if is_sel {
            Style::default()
                .fg(label_fg)
                .bg(row_bg)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(label_fg).bg(row_bg)
        };
        let sec_style = Style::default()
            .fg(if is_sel { palette.fg } else { palette.muted })
            .bg(row_bg);

        fill_row(buf, row_rect, Style::default().fg(label_fg).bg(row_bg));
        hitmap.record(row_rect, HitTarget::ListRow(abs_idx));

        let pointer = if cfg.show_pointer {
            if is_sel { "❯ " } else { "  " }
        } else {
            ""
        };
        let pointer_w = display_width(pointer);
        let max_cols = usize::from(content_w);
        let body_cols = max_cols.saturating_sub(pointer_w);

        let line = build_row_line(
            pointer,
            &item.label,
            item.secondary.as_deref(),
            body_cols,
            cfg.secondary_right_aligned,
            label_style,
            sec_style,
        );
        buf.set_line(area.x, y, &line, content_w);
    }

    if has_scrollbar {
        let track = Rect::new(area.x + content_w, area.y, 1, area.height);
        render_vertical_scrollbar(
            buf,
            track,
            cfg.scroll_id,
            state.items.len(),
            state.viewport_rows,
            state.offset,
            hitmap,
            Style::default()
                .fg(palette.separator)
                .bg(palette.modal_bg),
            Style::default().fg(palette.accent).bg(palette.modal_bg),
        );
    }
}

fn build_row_line<'a>(
    pointer: &'a str,
    label: &str,
    secondary: Option<&str>,
    body_cols: usize,
    right_aligned: bool,
    label_style: Style,
    sec_style: Style,
) -> Line<'a> {
    let mut spans = Vec::with_capacity(4);
    if !pointer.is_empty() {
        spans.push(Span::styled(pointer, label_style));
    }

    let sec_clean = secondary.map(str::trim).filter(|s| !s.is_empty());
    match sec_clean {
        Some(sec) if body_cols >= 6 => {
            let full_sec_w = display_width(sec);
            let full_label_w = display_width(label);

            if right_aligned {
                let max_sec_w = (body_cols / 2).max(4);
                let sec_text = truncate_to_width(sec, full_sec_w.min(max_sec_w));
                let sec_w = display_width(&sec_text);
                let max_label_w = body_cols.saturating_sub(sec_w + 1);
                let label_text = truncate_to_width(label, max_label_w);
                let label_w = display_width(&label_text);
                let pad = body_cols.saturating_sub(label_w + sec_w);
                spans.push(Span::styled(label_text, label_style));
                if pad > 0 {
                    spans.push(Span::styled(" ".repeat(pad), label_style));
                }
                spans.push(Span::styled(sec_text, sec_style));
            } else {
                let label_text = truncate_to_width(
                    label,
                    full_label_w.min(body_cols.saturating_sub(2)),
                );
                let label_w = display_width(&label_text);
                let rem = body_cols.saturating_sub(label_w);
                spans.push(Span::styled(label_text, label_style));
                if rem >= 2 {
                    spans.push(Span::styled(" ", label_style));
                    spans.push(Span::styled(truncate_to_width(sec, rem - 1), sec_style));
                }
            }
        }
        _ => {
            spans.push(Span::styled(
                truncate_to_width(label, body_cols),
                label_style,
            ));
        }
    }

    Line::from(spans)
}

fn fill_row(buf: &mut Buffer, rect: Rect, style: Style) {
    for y in rect.top()..rect.bottom() {
        for x in rect.left()..rect.right() {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_symbol(" ");
                cell.set_style(style);
            }
        }
    }
}

fn display_width(s: &str) -> usize {
    UnicodeWidthStr::width(s)
}

fn truncate_to_width(s: &str, max_w: usize) -> String {
    if max_w == 0 {
        return String::new();
    }
    if display_width(s) <= max_w {
        return s.to_owned();
    }
    if max_w == 1 {
        return "…".to_owned();
    }
    let target = max_w - 1;
    let mut out = String::new();
    let mut used = 0usize;
    for g in s.graphemes(true) {
        let w = display_width(g);
        if used + w > target {
            break;
        }
        out.push_str(g);
        used += w;
    }
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::theme::ThemePreset;

    fn make_item(key: &str, label: &str, selectable: bool) -> MenuItem<&'static str> {
        MenuItem {
            key: key.to_owned(),
            label: label.to_owned(),
            secondary: if selectable {
                Some("meta".to_owned())
            } else {
                None
            },
            selectable,
            data: "ok",
        }
    }

    #[test]
    fn selection_stability_and_heading_skipping() {
        let items = vec![
            make_item("h1", "General", false),
            make_item("a", "Alpha", true),
            make_item("h2", "Advanced", false),
            make_item("b", "Beta", true),
            make_item("c", "Gamma", true),
        ];
        let mut sel = MenuSelection::new(items);
        sel.viewport_rows = 3;
        assert_eq!(sel.selected_index(), Some(1));
        assert_eq!(sel.selected_key.as_deref(), Some("a"));

        // Heading is never selectable.
        assert!(!sel.select_index(2));
        assert_eq!(sel.selected_index(), Some(1));

        // Moving next skips heading at index 2.
        sel.select_next();
        assert_eq!(sel.selected_index(), Some(3));
        assert_eq!(sel.selected_key.as_deref(), Some("b"));

        // Reordering items preserves selected_key ("b") at its new index.
        let reordered = vec![
            make_item("c", "Gamma", true),
            make_item("b", "Beta", true),
            make_item("a", "Alpha", true),
        ];
        sel.update_items(reordered);
        assert_eq!(sel.selected_key.as_deref(), Some("b"));
        assert_eq!(sel.selected_index(), Some(1));

        // Removing selected item falls back cleanly to first selectable item.
        let shrunk = vec![
            make_item("h1", "Section", false),
            make_item("c", "Gamma", true),
        ];
        sel.update_items(shrunk);
        assert_eq!(sel.selected_key.as_deref(), Some("c"));
        assert_eq!(sel.selected_index(), Some(1));
    }

    #[test]
    fn arm_or_confirm_clears_on_move_and_scroll_clamps_safely() {
        let items = (0..12)
            .map(|i| make_item(&format!("k{i}"), &format!("Item {i}"), true))
            .collect();
        let mut sel = MenuSelection::new(items);
        sel.viewport_rows = 4;

        assert!(!sel.arm_or_confirm("k0"));
        assert_eq!(sel.pending_confirm.as_deref(), Some("k0"));
        sel.select_next();
        assert_eq!(sel.pending_confirm, None);

        assert!(!sel.arm_or_confirm("k1"));
        assert!(sel.arm_or_confirm("k1"));
        assert_eq!(sel.pending_confirm, None);

        sel.scroll_by(isize::MAX);
        assert_eq!(sel.offset, 8);
        sel.scroll_by(isize::MIN);
        assert_eq!(sel.offset, 0);
    }

    #[test]
    fn render_updates_viewport_rows_and_records_hits_and_scrollbar() {
        let items = vec![
            make_item("h1", "Section", false),
            make_item("r1", "First row", true),
            make_item("r2", "Second row", true),
            make_item("r3", "Third row", true),
            make_item("r4", "Fourth row", true),
            make_item("r5", "Fifth row", true),
        ];
        let mut sel = MenuSelection::new(items);
        let area = Rect::new(0, 0, 36, 4);
        let mut buf = Buffer::empty(area);
        let palette = ThemePalette::for_preset_with_no_color(ThemePreset::Titanium, false);
        let mut hitmap = HitMap::default();

        render_menu_list(
            area,
            &mut buf,
            &mut sel,
            &palette,
            &mut hitmap,
            MenuRenderConfig {
                scroll_id: ScrollId::SettingsList,
                show_pointer: true,
                secondary_right_aligned: true,
            },
        );

        assert_eq!(sel.viewport_rows, 4);
        // Row 0 is a non-selectable heading -> must not produce a ListRow hit.
        assert_eq!(hitmap.hit_test(2, 0), None);
        assert_eq!(hitmap.hit_test(2, 1), Some(&HitTarget::ListRow(1)));
        assert_eq!(hitmap.hit_test(2, 3), Some(&HitTarget::ListRow(3)));
        assert_eq!(
            hitmap.scrollbar_at(35, 1),
            Some((ScrollId::SettingsList, Rect::new(35, 0, 1, 4), 6, 4))
        );
    }
}
