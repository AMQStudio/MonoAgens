//! `Ctrl+K` command palette: fuzzy search over commands, settings rows and chats.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::settings::SettingsTab;
use crate::state::HitRegions;
use crate::theme::ThemePalette;
use crate::ui_util::{ListRow, boxed, centered, render_list};

/// What Enter does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaletteTarget {
    /// Runs `/name`.
    Command(String),
    /// Opens `/settings` on the tab at the row with this id.
    Setting { tab: SettingsTab, id: String },
    /// Switches to the chat.
    Chat(String),
}

/// One palette entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaletteItem {
    pub kind: &'static str,
    pub label: String,
    pub detail: String,
    pub target: PaletteTarget,
}

/// Fuzzy score of `query` in `text` (subsequence match; consecutive and word-start hits score
/// higher); `None` when `query` is not a subsequence.
#[must_use]
pub fn fuzzy_score(query: &str, text: &str) -> Option<i32> {
    let q: Vec<char> = query.to_lowercase().chars().filter(|c| !c.is_whitespace()).collect();
    if q.is_empty() {
        return Some(0);
    }
    let t: Vec<char> = text.to_lowercase().chars().collect();
    let mut score = 0;
    let mut ti = 0;
    let mut prev_hit: Option<usize> = None;
    for qc in &q {
        let pos = (ti..t.len()).find(|i| t[*i] == *qc)?;
        score += 10;
        if prev_hit == Some(pos.wrapping_sub(1)) {
            score += 15;
        }
        if pos == 0 || !t[pos - 1].is_alphanumeric() {
            score += 12;
        }
        score -= i32::try_from(pos.saturating_sub(ti)).unwrap_or(0).min(8);
        prev_hit = Some(pos);
        ti = pos + 1;
    }
    if t.iter().collect::<String>().starts_with(&q.iter().collect::<String>()) {
        score += 30;
    }
    Some(score)
}

/// Palette state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Palette {
    pub query: String,
    pub items: Vec<PaletteItem>,
    pub sel: usize,
    pub off: usize,
    pub view_rows: usize,
}

/// What a key did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaletteOutcome {
    None,
    Close,
    Run(PaletteTarget),
}

impl Palette {
    /// Opens the palette over `items`.
    #[must_use]
    pub fn new(items: Vec<PaletteItem>) -> Self {
        Self { query: String::new(), items, sel: 0, off: 0, view_rows: 10 }
    }

    /// Items matching the query, best first.
    #[must_use]
    pub fn filtered(&self) -> Vec<&PaletteItem> {
        if self.query.trim().is_empty() {
            return self.items.iter().collect();
        }
        let mut scored: Vec<(i32, usize, &PaletteItem)> = self
            .items
            .iter()
            .enumerate()
            .filter_map(|(i, it)| {
                let hay = format!("{} {}", it.label, it.detail);
                let s = fuzzy_score(&self.query, &it.label)
                    .map(|s| s + 20)
                    .or_else(|| fuzzy_score(&self.query, &hay))?;
                Some((s, i, it))
            })
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        scored.into_iter().map(|(_, _, it)| it).collect()
    }

    fn clamp(&mut self) {
        let n = self.filtered().len();
        self.sel = self.sel.min(n.saturating_sub(1));
        self.off = crate::render::scroll_window(self.sel, self.off, self.view_rows, n);
    }

    /// Moves the selection by `delta` (no wrap).
    pub fn move_by(&mut self, delta: isize) {
        let n = self.filtered().len() as isize;
        if n > 0 {
            self.sel = (self.sel as isize + delta).clamp(0, n - 1) as usize;
        }
        self.clamp();
    }

    /// Handles one key.
    pub fn handle_key(&mut self, key: KeyEvent) -> PaletteOutcome {
        match key.code {
            KeyCode::Esc => return PaletteOutcome::Close,
            KeyCode::Enter => {
                return self
                    .filtered()
                    .get(self.sel)
                    .map_or(PaletteOutcome::Close, |it| PaletteOutcome::Run(it.target.clone()));
            }
            KeyCode::Up => self.move_by(-1),
            KeyCode::Down => self.move_by(1),
            KeyCode::PageUp => self.move_by(-(self.view_rows as isize)),
            KeyCode::PageDown => self.move_by(self.view_rows as isize),
            KeyCode::Backspace => {
                self.query.pop();
                self.sel = 0;
                self.clamp();
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.query.push(c);
                self.sel = 0;
                self.clamp();
            }
            _ => {}
        }
        PaletteOutcome::None
    }
}

/// Draws the palette.
pub fn render_palette(area: Rect, buf: &mut Buffer, p: &Palette, theme: &ThemePalette, hit: &mut HitRegions) {
    let bx = centered(80.min(area.width), 20.min(area.height), area);
    let inner = boxed(bx, buf, "Command palette", theme);
    if inner.height < 4 {
        return;
    }
    let line = Line::from(vec![
        Span::styled(" > ", Style::default().fg(theme.accent).bg(theme.modal_bg).add_modifier(Modifier::BOLD)),
        Span::styled(format!("{}▏", p.query), Style::default().fg(theme.fg).bg(theme.modal_bg)),
    ]);
    buf.set_line(inner.x, inner.y, &line, inner.width);
    let list = Rect::new(inner.x, inner.y + 2, inner.width, inner.height.saturating_sub(2));
    let filtered = p.filtered();
    let rows: Vec<ListRow> = filtered
        .iter()
        .map(|it| ListRow { left: format!("{}  {}", it.kind, it.label), right: it.detail.clone(), dim: false })
        .collect();
    hit.overlay_area = list;
    let mut hits = Vec::new();
    let off = render_list(list, buf, &rows, p.sel, p.off, theme, &mut hits);
    hit.overlay_rows = hits;
    hit.overlay_total = rows.len();
    hit.overlay_offset = off;
    if rows.is_empty() {
        buf.set_string(list.x + 2, list.y, "No match", Style::default().fg(theme.dim).bg(theme.modal_bg));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(label: &str) -> PaletteItem {
        PaletteItem { kind: "cmd", label: label.to_owned(), detail: String::new(), target: PaletteTarget::Command(label.to_owned()) }
    }

    #[test]
    fn fuzzy_matches_subsequences_and_prefers_prefixes() {
        assert!(fuzzy_score("qh", "quiet_hours").is_some());
        assert!(fuzzy_score("xyz", "quiet_hours").is_none());
        assert!(fuzzy_score("quiet", "quiet_hours") > fuzzy_score("quiet", "a quite quiet one"));
        let mut p = Palette::new(vec![item("plugins"), item("quiet_hours"), item("quit")]);
        p.query = "qui".into();
        let labels: Vec<_> = p.filtered().iter().map(|i| i.label.clone()).collect();
        assert_eq!(labels.len(), 2);
        assert!(!labels.contains(&"plugins".to_owned()));
    }
}
