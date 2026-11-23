//! Enum selector (omp pattern): name, description, optional live preview, option list with the
//! current value marked, a scrollbar and a "Type to search" line. Enter picks, Esc cancels.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::state::HitRegions;
use crate::theme::ThemePalette;
use crate::ui_util::{ListRow, boxed, centered, render_list, trunc};

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
    pub sel: usize,
    pub off: usize,
    pub view_rows: usize,
}

impl Selector {
    /// Opens the selector with the current value highlighted.
    #[must_use]
    pub fn new(target: &str, name: &str, description: &str, options: Vec<SelectOption>, current: &str) -> Self {
        let sel = options.iter().position(|o| o.value == current).unwrap_or(0);
        Self {
            target: target.to_owned(),
            name: name.to_owned(),
            description: description.to_owned(),
            options,
            current: current.to_owned(),
            search: String::new(),
            sel,
            off: 0,
            view_rows: 8,
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
        let mut idx: Vec<usize> = (0..self.options.len()).filter(|i| hit(&self.options[*i])).collect();
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
        self.off = crate::render::scroll_window(self.sel, self.off, self.view_rows, n);
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
        match key.code {
            KeyCode::Esc => return SelectorOutcome::Cancel,
            KeyCode::Enter => {
                return self
                    .highlighted()
                    .map_or(SelectorOutcome::Cancel, |o| SelectorOutcome::Picked(o.value.clone()));
            }
            KeyCode::Up => self.move_by(-1),
            KeyCode::Down => self.move_by(1),
            KeyCode::PageUp => self.move_by(-(self.view_rows as isize)),
            KeyCode::PageDown => self.move_by(self.view_rows as isize),
            KeyCode::Home => self.move_by(isize::MIN / 2),
            KeyCode::End => self.move_by(isize::MAX / 2),
            KeyCode::Backspace => {
                self.search.pop();
                self.sel = 0;
                self.clamp();
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.search.push(c);
                self.sel = 0;
                self.clamp();
            }
            _ => {}
        }
        SelectorOutcome::None
    }
}

/// Draws the selector over `area` and records the row hit rects.
pub fn render_selector(area: Rect, buf: &mut Buffer, sel: &Selector, palette: &ThemePalette, hit: &mut HitRegions) {
    let box_area = centered(72, 22.min(area.height), area);
    let inner = boxed(box_area, buf, &format!("Select {}", sel.name), palette);
    if inner.height < 6 || inner.width < 20 {
        return;
    }
    let muted = Style::default().fg(palette.muted).bg(palette.modal_bg);
    buf.set_string(
        inner.x + 1,
        inner.y,
        trunc(&sel.description, inner.width.saturating_sub(2) as usize),
        muted,
    );
    // Preview / highlighted option description.
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
        trunc(&format!("Preview: {preview}"), inner.width.saturating_sub(2) as usize),
        Style::default().fg(palette.accent).bg(palette.modal_bg),
    );
    let list_h = inner.height.saturating_sub(4);
    let list_area = Rect::new(inner.x, inner.y + 3, inner.width, list_h);
    let filtered = sel.filtered();
    let rows: Vec<ListRow> = filtered
        .iter()
        .map(|i| {
            let o = &sel.options[*i];
            let cur = if o.value == sel.current { " (current)" } else { "" };
            ListRow {
                left: format!("{}{cur}", o.label),
                right: o.description.clone(),
                dim: false,
            }
        })
        .collect();
    hit.overlay_area = list_area;
    hit.overlay_rows.clear();
    let mut rows_hit = Vec::new();
    let off = render_list(list_area, buf, &rows, sel.sel, sel.off, palette, &mut rows_hit);
    hit.overlay_rows = rows_hit;
    hit.overlay_total = rows.len();
    hit.overlay_offset = off;
    if rows.is_empty() {
        buf.set_string(list_area.x + 2, list_area.y, "No match", muted);
    }
    let search = Line::from(vec![
        Span::styled(
            " Type to search: ",
            Style::default()
                .fg(palette.accent)
                .bg(palette.modal_bg)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!("{}▏", sel.search), Style::default().fg(palette.fg).bg(palette.modal_bg)),
    ]);
    buf.set_line(inner.x, inner.y + inner.height - 1, &search, inner.width);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts() -> Vec<SelectOption> {
        ["alpha", "beta", "gamma", "max_tokens", "max_completion_tokens"]
            .iter()
            .map(|v| SelectOption::plain(v, ""))
            .collect()
    }

    fn k(c: KeyCode) -> KeyEvent {
        KeyEvent::new(c, KeyModifiers::NONE)
    }

    #[test]
    fn search_and_pick() {
        let mut s = Selector::new("x", "X", "", opts(), "beta");
        assert_eq!(s.sel, 1, "current value is highlighted first");
        s.handle_key(k(KeyCode::Char('m')));
        s.handle_key(k(KeyCode::Char('a')));
        assert_eq!(s.filtered().len(), 3, "gamma also contains \"ma\"; prefix matches sort first");
        assert_eq!(s.handle_key(k(KeyCode::Enter)), SelectorOutcome::Picked("max_tokens".into()));
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
