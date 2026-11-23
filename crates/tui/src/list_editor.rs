//! List and map field editors (add / edit / delete / reorder) and the number range check.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::state::HitRegions;
use crate::theme::ThemePalette;
use crate::ui_util::{ListRow, boxed, centered, render_list};

/// Inclusive numeric range of a [`crate::SettingValueKind::Number`] field.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NumberSpec {
    pub min: f64,
    pub max: f64,
    pub integer: bool,
}

impl NumberSpec {
    /// Integer range.
    #[must_use]
    pub const fn int(min: i64, max: i64) -> Self {
        Self {
            min: min as f64,
            max: max as f64,
            integer: true,
        }
    }

    /// Floating-point range.
    #[must_use]
    pub const fn float(min: f64, max: f64) -> Self {
        Self {
            min,
            max,
            integer: false,
        }
    }

    /// Human-readable allowed range.
    #[must_use]
    pub fn describe(&self) -> String {
        if self.integer {
            format!("{}..={}", self.min as i64, self.max as i64)
        } else {
            format!("{}..={}", self.min, self.max)
        }
    }

    /// Parses and range-checks `raw`; the error names the allowed range.
    pub fn validate(&self, raw: &str) -> Result<f64, String> {
        let t = raw.trim();
        let v: f64 = if self.integer {
            t.parse::<i64>()
                .map(|n| n as f64)
                .map_err(|_| format!("'{t}' is not a whole number (allowed {})", self.describe()))?
        } else {
            t.parse::<f64>()
                .map_err(|_| format!("'{t}' is not a number (allowed {})", self.describe()))?
        };
        if !v.is_finite() || v < self.min || v > self.max {
            return Err(format!("{t} is out of range (allowed {})", self.describe()));
        }
        Ok(v)
    }
}

/// One row of an [`ItemEditor`]; lists leave `key` empty.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub key: Option<String>,
    pub value: String,
}

/// Editing step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ItemMode {
    Browse,
    /// Typing the value (or the key first, for maps) of the row at `index` (`None` = new row).
    Edit {
        index: Option<usize>,
        buffer: String,
        /// Map rows: the key typed in the first step.
        key: Option<String>,
        editing_key: bool,
    },
}

/// What a key did to the editor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ItemOutcome {
    None,
    /// The edited rows should be saved.
    Save,
    /// Esc on the browse level.
    Close,
    /// `p`: open the picker for a new item (lists with a known id set).
    Pick,
}

/// List / map editor state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemEditor {
    /// Row id being edited.
    pub target: String,
    pub title: String,
    pub is_map: bool,
    pub items: Vec<Item>,
    pub sel: usize,
    pub off: usize,
    pub mode: ItemMode,
    pub view_rows: usize,
    /// Offer `p` (pick from known ids).
    pub can_pick: bool,
    pub error: Option<String>,
}

impl ItemEditor {
    /// List editor over `values`.
    #[must_use]
    pub fn list(target: &str, title: &str, values: &[String]) -> Self {
        Self::build(target, title, false, values.iter().map(|v| Item { key: None, value: v.clone() }).collect())
    }

    /// Map editor over `(key, value)` pairs.
    #[must_use]
    pub fn map(target: &str, title: &str, entries: &[(String, String)]) -> Self {
        Self::build(
            target,
            title,
            true,
            entries
                .iter()
                .map(|(k, v)| Item { key: Some(k.clone()), value: v.clone() })
                .collect(),
        )
    }

    fn build(target: &str, title: &str, is_map: bool, items: Vec<Item>) -> Self {
        Self {
            target: target.to_owned(),
            title: title.to_owned(),
            is_map,
            items,
            sel: 0,
            off: 0,
            mode: ItemMode::Browse,
            view_rows: 8,
            can_pick: false,
            error: None,
        }
    }

    /// List values in order.
    #[must_use]
    pub fn values(&self) -> Vec<String> {
        self.items.iter().map(|i| i.value.clone()).collect()
    }

    /// Map entries in order.
    #[must_use]
    pub fn entries(&self) -> Vec<(String, String)> {
        self.items
            .iter()
            .map(|i| (i.key.clone().unwrap_or_default(), i.value.clone()))
            .collect()
    }

    /// Adds `value` to a list unless already present (used by the picker).
    pub fn push_unique(&mut self, value: &str) {
        if !self.items.iter().any(|i| i.value == value) {
            self.items.push(Item { key: None, value: value.to_owned() });
            self.sel = self.items.len() - 1;
        }
    }

    /// Moves the selected row up (`-1`) or down (`1`), swapping with its neighbour.
    pub fn move_item(&mut self, delta: isize) {
        let n = self.items.len();
        let to = self.sel as isize + delta;
        if n < 2 || to < 0 || to >= n as isize {
            return;
        }
        self.items.swap(self.sel, to as usize);
        self.sel = to as usize;
    }

    fn clamp(&mut self) {
        self.sel = self.sel.min(self.items.len().saturating_sub(1));
        self.off = crate::render::scroll_window(self.sel, self.off, self.view_rows, self.items.len());
    }

    /// Handles one key.
    pub fn handle_key(&mut self, key: KeyEvent) -> ItemOutcome {
        self.error = None;
        if let ItemMode::Edit { .. } = self.mode {
            return self.edit_key(key);
        }
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        match key.code {
            KeyCode::Esc => return ItemOutcome::Close,
            KeyCode::Up if alt => {
                self.move_item(-1);
                return ItemOutcome::Save;
            }
            KeyCode::Down if alt => {
                self.move_item(1);
                return ItemOutcome::Save;
            }
            KeyCode::Char('K') => {
                self.move_item(-1);
                return ItemOutcome::Save;
            }
            KeyCode::Char('J') => {
                self.move_item(1);
                return ItemOutcome::Save;
            }
            KeyCode::Up => self.sel = self.sel.saturating_sub(1),
            KeyCode::Down => self.sel = (self.sel + 1).min(self.items.len().saturating_sub(1)),
            KeyCode::PageUp => self.sel = self.sel.saturating_sub(self.view_rows),
            KeyCode::PageDown => self.sel = (self.sel + self.view_rows).min(self.items.len().saturating_sub(1)),
            KeyCode::Char('a' | 'n') | KeyCode::Insert => {
                self.mode = ItemMode::Edit {
                    index: None,
                    buffer: String::new(),
                    key: None,
                    editing_key: self.is_map,
                };
            }
            KeyCode::Char('p') if self.can_pick => return ItemOutcome::Pick,
            KeyCode::Enter | KeyCode::Char('e') => {
                if let Some(item) = self.items.get(self.sel) {
                    self.mode = ItemMode::Edit {
                        index: Some(self.sel),
                        buffer: if self.is_map { item.key.clone().unwrap_or_default() } else { item.value.clone() },
                        key: None,
                        editing_key: self.is_map,
                    };
                }
            }
            KeyCode::Char('d') | KeyCode::Delete if self.sel < self.items.len() => {
                self.items.remove(self.sel);
                self.clamp();
                return ItemOutcome::Save;
            }
            _ => {}
        }
        self.clamp();
        ItemOutcome::None
    }

    fn edit_key(&mut self, key: KeyEvent) -> ItemOutcome {
        let ItemMode::Edit { index, buffer, key: map_key, editing_key } = &mut self.mode else {
            return ItemOutcome::None;
        };
        match key.code {
            KeyCode::Esc => self.mode = ItemMode::Browse,
            KeyCode::Backspace => {
                buffer.pop();
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => buffer.push(c),
            KeyCode::Enter => {
                let text = buffer.trim().to_owned();
                if *editing_key {
                    if text.is_empty() {
                        self.error = Some("key must not be empty".to_owned());
                        return ItemOutcome::None;
                    }
                    let existing = index.and_then(|i| self.items.get(i)).map(|it| it.value.clone());
                    *map_key = Some(text);
                    *editing_key = false;
                    *buffer = existing.unwrap_or_default();
                    return ItemOutcome::None;
                }
                if text.is_empty() && !self.is_map {
                    self.error = Some("value must not be empty".to_owned());
                    return ItemOutcome::None;
                }
                let new = Item { key: map_key.take(), value: text };
                match *index {
                    Some(i) if i < self.items.len() => self.items[i] = new,
                    _ => {
                        self.items.push(new);
                        self.sel = self.items.len() - 1;
                    }
                }
                self.mode = ItemMode::Browse;
                self.clamp();
                return ItemOutcome::Save;
            }
            _ => {}
        }
        ItemOutcome::None
    }
}

/// Draws the editor.
pub fn render_item_editor(area: Rect, buf: &mut Buffer, ed: &ItemEditor, palette: &ThemePalette, hit: &mut HitRegions) {
    let box_area = centered(72, 20.min(area.height), area);
    let inner = boxed(box_area, buf, &ed.title, palette);
    if inner.height < 5 || inner.width < 20 {
        return;
    }
    let hint = if ed.can_pick {
        " a add · p pick · Enter edit · d delete · Alt+↑/↓ (or K/J) reorder · Esc back"
    } else {
        " a add · Enter edit · d delete · Alt+↑/↓ (or K/J) reorder · Esc back"
    };
    buf.set_string(
        inner.x,
        inner.y,
        crate::ui_util::trunc(hint, inner.width as usize),
        Style::default().fg(palette.dim).bg(palette.modal_bg),
    );
    let list_area = Rect::new(inner.x, inner.y + 1, inner.width, inner.height.saturating_sub(3));
    let rows: Vec<ListRow> = ed
        .items
        .iter()
        .map(|i| match &i.key {
            Some(k) => ListRow { left: k.clone(), right: i.value.clone(), dim: false },
            None => ListRow { left: i.value.clone(), right: String::new(), dim: false },
        })
        .collect();
    hit.overlay_area = list_area;
    let mut rows_hit = Vec::new();
    let off = render_list(list_area, buf, &rows, ed.sel, ed.off, palette, &mut rows_hit);
    hit.overlay_rows = rows_hit;
    hit.overlay_total = rows.len();
    hit.overlay_offset = off;
    if rows.is_empty() {
        buf.set_string(
            list_area.x + 2,
            list_area.y,
            "(empty) press a to add",
            Style::default().fg(palette.dim).bg(palette.modal_bg),
        );
    }
    let bottom = inner.y + inner.height - 1;
    if let ItemMode::Edit { buffer, editing_key, .. } = &ed.mode {
        let label = if ed.is_map && *editing_key { " Key: " } else { " Value: " };
        let line = Line::from(vec![
            Span::styled(label, Style::default().fg(palette.accent).bg(palette.selection_bg).add_modifier(Modifier::BOLD)),
            Span::styled(format!("{buffer}▏"), Style::default().fg(palette.fg).bg(palette.selection_bg)),
        ]);
        crate::ui_util::fill(buf, Rect::new(inner.x, bottom, inner.width, 1), palette.selection_bg, palette.fg);
        buf.set_line(inner.x, bottom, &line, inner.width);
    } else if let Some(err) = &ed.error {
        buf.set_string(inner.x + 1, bottom, err, Style::default().fg(palette.status_red).bg(palette.modal_bg));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn k(c: KeyCode) -> KeyEvent {
        KeyEvent::new(c, KeyModifiers::NONE)
    }

    #[test]
    fn number_range_check_names_the_range() {
        let spec = NumberSpec::int(1, 60_000);
        assert_eq!(spec.validate(" 500 "), Ok(500.0));
        let err = spec.validate("70000").unwrap_err();
        assert!(err.contains("1..=60000"), "{err}");
        assert!(spec.validate("1.5").is_err());
        assert!(NumberSpec::float(0.0, 2.0).validate("2.5").is_err());
        assert_eq!(NumberSpec::float(0.0, 2.0).validate("0.7"), Ok(0.7));
    }

    #[test]
    fn list_reorder_swaps_neighbours() {
        let mut ed = ItemEditor::list("t", "T", &["a".into(), "b".into(), "c".into()]);
        ed.sel = 1;
        assert_eq!(ed.handle_key(k(KeyCode::Char('K'))), ItemOutcome::Save);
        assert_eq!(ed.values(), ["b", "a", "c"]);
        assert_eq!(ed.sel, 0);
    }

    #[test]
    fn list_add_edit_delete() {
        let mut ed = ItemEditor::list("t", "T", &["a".into()]);
        ed.handle_key(k(KeyCode::Char('a')));
        ed.handle_key(k(KeyCode::Char('z')));
        assert_eq!(ed.handle_key(k(KeyCode::Enter)), ItemOutcome::Save);
        assert_eq!(ed.values(), ["a", "z"]);
        ed.handle_key(k(KeyCode::Enter)); // edit "z"
        ed.handle_key(k(KeyCode::Backspace));
        ed.handle_key(k(KeyCode::Char('y')));
        ed.handle_key(k(KeyCode::Enter));
        assert_eq!(ed.values(), ["a", "y"]);
        assert_eq!(ed.handle_key(k(KeyCode::Char('d'))), ItemOutcome::Save);
        assert_eq!(ed.values(), ["a"]);
    }

    #[test]
    fn map_add_takes_key_then_value() {
        let mut ed = ItemEditor::map("t", "T", &[]);
        ed.handle_key(k(KeyCode::Char('a')));
        for c in "X-Org".chars() {
            ed.handle_key(k(KeyCode::Char(c)));
        }
        ed.handle_key(k(KeyCode::Enter));
        for c in "acme".chars() {
            ed.handle_key(k(KeyCode::Char(c)));
        }
        assert_eq!(ed.handle_key(k(KeyCode::Enter)), ItemOutcome::Save);
        assert_eq!(ed.entries(), [("X-Org".to_owned(), "acme".to_owned())]);
    }
}
