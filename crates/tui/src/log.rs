//! `/log`: the runtime log overlay. Entries come from `monoagens_gateway::runlog` (the ring,
//! rotating file, masking and subscription live there); this module holds the overlay state
//! (filters, search, follow, scrolling) and draws it.

use crossterm::event::{KeyCode, KeyEvent};
use monoagens_gateway::{LogBuffer, LogEntry, RunLogLevel};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Clear, Widget};

use crate::core::hitmap::{HitTarget, ScrollId};
use crate::core::input::{PasteTarget, TextInput};
use crate::core::overlay::{OverlayId, WidgetOutcome};
use crate::core::theme::render_dimmed_backdrop;
use crate::render::scroll_window;
use crate::state::{AppAction, HitRegions};
use crate::theme::ThemePalette;
use crate::width::{chrome_label, fit_prefix, sanitize};
use crate::widgets::scrollbar::render_vertical_scrollbar;
use unicode_width::UnicodeWidthStr;

/// Rows used by the overlay besides the entries (header and footer).
pub const LOG_CHROME_ROWS: u16 = 2;

/// Overlay state: the entries received so far plus filters and scroll position.
#[derive(Debug, Clone)]
pub struct LogView {
    pub open: bool,
    /// Entries received (the gateway ring is bounded to 5 000 as well).
    pub entries: LogBuffer,
    /// Level filter `1..=4`: 1 debug and up, 2 info and up, 3 warn and up, 4 error only.
    pub level: u8,
    /// Source filter (`None` shows every source).
    pub source: Option<String>,
    pub search: String,
    pub search_input: TextInput,
    /// The search box has the keyboard.
    pub searching: bool,
    /// Keep the view at the newest entry.
    pub follow: bool,
    /// Index (in the filtered list) of the first visible entry.
    pub offset: usize,
    /// Cursor row (index in the filtered list).
    pub selected: usize,
    /// Number of entry rows currently on screen (kept in sync with the terminal size).
    pub view_rows: usize,
}

impl Default for LogView {
    fn default() -> Self {
        Self {
            open: false,
            entries: LogBuffer::default(),
            level: 1,
            source: None,
            search: String::new(),
            search_input: TextInput::new(false),
            searching: false,
            follow: true,
            offset: 0,
            selected: 0,
            view_rows: 10,
        }
    }
}

/// Minimum level of filter key `1..=4`.
#[must_use]
pub const fn level_for_filter(filter: u8) -> RunLogLevel {
    match filter {
        0 | 1 => RunLogLevel::Debug,
        2 => RunLogLevel::Info,
        3 => RunLogLevel::Warn,
        _ => RunLogLevel::Error,
    }
}

/// Formats an epoch-millisecond timestamp as `HH:MM:SS.mmm` (UTC).
#[must_use]
pub fn format_time(ts_ms: u64) -> String {
    let secs = ts_ms / 1000 % 86_400;
    format!(
        "{:02}:{:02}:{:02}.{:03}",
        secs / 3600,
        secs / 60 % 60,
        secs % 60,
        ts_ms % 1000
    )
}

/// One entry as displayed and copied.
#[must_use]
pub fn format_entry(e: &LogEntry) -> String {
    format!(
        "{} {:<5} {:<14} {}",
        format_time(e.ts_ms),
        e.level.label(),
        e.source,
        sanitize(&e.message, false)
    )
}

impl LogView {
    /// Stores `entry`; returns the toast text when it is an error.
    pub fn push(&mut self, entry: LogEntry) -> Option<String> {
        let toast = (entry.level == RunLogLevel::Error)
            .then(|| format!("{} — /log for details", sanitize(&entry.message, false)));
        self.entries.push(entry);
        toast
    }

    /// Entries passing the level, source and search filters, oldest first.
    #[must_use]
    pub fn filtered(&self) -> Vec<LogEntry> {
        let min = level_for_filter(self.level);
        let needle = self.search.to_lowercase();
        self.entries
            .snapshot()
            .into_iter()
            .filter(|e| e.level >= min)
            .filter(|e| self.source.as_deref().is_none_or(|s| e.source == s))
            .filter(|e| {
                needle.is_empty()
                    || e.message.to_lowercase().contains(&needle)
                    || e.source.to_lowercase().contains(&needle)
            })
            .collect()
    }

    /// Distinct sources seen so far, sorted.
    #[must_use]
    pub fn sources(&self) -> Vec<String> {
        let mut v: Vec<String> = self.entries.snapshot().into_iter().map(|e| e.source).collect();
        v.sort();
        v.dedup();
        v
    }

    /// `s`: cycles through all sources then back to "all".
    pub fn cycle_source(&mut self) {
        let sources = self.sources();
        self.source = match &self.source {
            None => sources.first().cloned(),
            Some(cur) => sources
                .iter()
                .position(|s| s == cur)
                .and_then(|i| sources.get(i + 1))
                .cloned(),
        };
        self.reset_view();
    }

    fn reset_view(&mut self) {
        self.selected = 0;
        self.offset = 0;
        self.follow = true;
    }

    /// Opens the overlay following the newest entry.
    pub fn open(&mut self) {
        self.open = true;
        self.searching = false;
        self.follow = true;
    }

    /// Offset actually shown for `total` filtered entries and `visible` rows.
    #[must_use]
    pub fn effective_offset(&self, total: usize, visible: usize) -> usize {
        let max = total.saturating_sub(visible);
        if self.follow {
            max
        } else {
            self.offset.min(max)
        }
    }

    fn move_cursor(&mut self, delta: isize) {
        let total = self.filtered().len();
        if total == 0 {
            return;
        }
        let visible = self.view_rows.max(1);
        let current = if self.follow {
            total - 1
        } else {
            self.selected.min(total - 1)
        };
        let next = current.saturating_add_signed(delta).min(total - 1);
        self.follow = false;
        let offset = self.effective_offset(total, visible);
        let from = if current >= offset { offset } else { current };
        self.selected = next;
        self.offset = scroll_window(next, from, visible, total);
        if next == total - 1 && delta > 0 {
            self.follow = true;
        }
    }

    /// Mouse wheel: scrolls the window by `delta` rows.
    pub fn scroll_by(&mut self, delta: isize) {
        let total = self.filtered().len();
        let visible = self.view_rows.max(1);
        let max = total.saturating_sub(visible);
        let cur = self.effective_offset(total, visible);
        let next = cur.saturating_add_signed(delta).min(max);
        self.offset = next;
        self.selected = next;
        self.follow = next >= max && delta > 0;
    }

    /// Text of the entries currently on screen (what `c` copies).
    #[must_use]
    pub fn visible_text(&self) -> String {
        let all = self.filtered();
        let visible = self.view_rows.max(1);
        let off = self.effective_offset(all.len(), visible);
        all.iter()
            .skip(off)
            .take(visible)
            .map(format_entry)
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn sync_search_input(&mut self) {
        if self.search_input.value != self.search {
            self.search_input.set_value(self.search.clone());
        }
    }

    /// Handles a key while the overlay is open.
    pub fn handle_key(&mut self, key: KeyEvent) -> Vec<AppAction> {
        if self.searching {
            match key.code {
                KeyCode::Esc | KeyCode::Enter => self.searching = false,
                _ => {
                    self.sync_search_input();
                    if self.search_input.handle_key(key) == WidgetOutcome::Consumed {
                        self.search.clone_from(&self.search_input.value);
                        self.reset_view();
                    }
                }
            }
            return Vec::new();
        }
        let page = self.view_rows.max(1).saturating_sub(1).max(1) as isize;
        match key.code {
            KeyCode::Esc => self.open = false,
            KeyCode::Char(c @ '1'..='4') => {
                self.level = c as u8 - b'0';
                self.reset_view();
            }
            KeyCode::Char('s') => self.cycle_source(),
            KeyCode::Char('/') => self.searching = true,
            KeyCode::Char('f') => {
                self.follow = !self.follow;
                if !self.follow {
                    let total = self.filtered().len();
                    self.offset = total.saturating_sub(self.view_rows.max(1));
                    self.selected = total.saturating_sub(1);
                }
            }
            KeyCode::Char('c') => {
                let text = self.visible_text();
                if !text.is_empty() {
                    return vec![AppAction::CopyToClipboard(text)];
                }
            }
            KeyCode::Up => self.move_cursor(-1),
            KeyCode::Down => self.move_cursor(1),
            KeyCode::PageUp => self.move_cursor(-page),
            KeyCode::PageDown => self.move_cursor(page),
            KeyCode::Home => {
                self.follow = false;
                self.selected = 0;
                self.offset = 0;
            }
            KeyCode::End => {
                self.follow = true;
            }
            _ => {}
        }
        Vec::new()
    }
}

impl PasteTarget for LogView {
    fn paste_text(&mut self, raw: &str) -> bool {
        if !self.searching {
            return false;
        }
        self.sync_search_input();
        if self.search_input.paste_text(raw) {
            self.search.clone_from(&self.search_input.value);
            self.reset_view();
            true
        } else {
            false
        }
    }
}

/// Draws the overlay over the whole `area`.
pub fn render_log_overlay(
    area: Rect,
    buf: &mut Buffer,
    view: &LogView,
    palette: &ThemePalette,
    icons: crate::config::IconMode,
    hit: &mut HitRegions,
) {
    if area.width < 10 || area.height < LOG_CHROME_ROWS + 1 {
        return;
    }
    render_dimmed_backdrop(buf, area, palette);
    hit.hitmap.record(area, HitTarget::Backdrop(OverlayId::Log));
    Clear.render(area, buf);
    let bg = Style::default().fg(palette.fg).bg(palette.bg);
    for y in area.y..area.y + area.height {
        for x in area.x..area.x + area.width {
            if let Some(c) = buf.cell_mut((x, y)) {
                c.reset();
                c.set_style(bg);
            }
        }
    }
    hit.hitmap.record(area, HitTarget::OverlayBody(OverlayId::Log));
    let all = view.filtered();
    let total = all.len();
    let visible = (area.height - LOG_CHROME_ROWS) as usize;
    let offset = view.effective_offset(total, visible);
    let list_w = area.width.saturating_sub(1) as usize;

    // Header: title + active filters.
    let level_names = ["all", "info+", "warn+", "error"];
    let header = format!(
        " Runtime log  level:{} ({}/{})  source:{}  {}",
        level_names[usize::from(view.level.clamp(1, 4) - 1)],
        total,
        view.entries.len(),
        view.source.as_deref().unwrap_or("all"),
        if view.follow { "follow" } else { "paused" },
    );
    buf.set_string(
        area.x,
        area.y,
        pad(&chrome_label(&header, icons), area.width as usize),
        Style::default()
            .fg(palette.accent)
            .bg(palette.status_bg)
            .add_modifier(Modifier::BOLD),
    );

    // Entries.
    for (row, entry) in all.iter().skip(offset).take(visible).enumerate() {
        let y = area.y + 1 + row as u16;
        let fg = match entry.level {
            RunLogLevel::Error => palette.status_red,
            RunLogLevel::Warn => palette.status_yellow,
            RunLogLevel::Info => palette.fg,
            RunLogLevel::Debug => palette.dim,
        };
        let is_cursor = !view.follow && offset + row == view.selected;
        let line = pad(&chrome_label(&format_entry(entry), icons), list_w);
        let mut style = Style::default().fg(fg).bg(palette.bg);
        if is_cursor {
            style = style.bg(palette.status_bg).add_modifier(Modifier::BOLD);
        }
        buf.set_string(area.x, y, line, style);
        hit.hitmap.record(
            Rect::new(area.x, y, list_w as u16, 1),
            HitTarget::ListRow(offset + row),
        );
    }
    if total == 0 {
        buf.set_string(
            area.x + 1,
            area.y + 1,
            "No log entries match the current filters.",
            Style::default().fg(palette.dim).bg(palette.bg),
        );
    }

    // Scrollbar on the right edge of the entry rows.
    let sb = Rect::new(area.x + area.width - 1, area.y + 1, 1, visible as u16);
    render_vertical_scrollbar(
        buf,
        sb,
        ScrollId::Log,
        total,
        visible,
        offset,
        &mut hit.hitmap,
        Style::default().fg(palette.separator).bg(palette.bg),
        Style::default().fg(palette.accent).bg(palette.bg),
    );
    hit.log_area = Rect::new(area.x, area.y + 1, area.width, visible as u16);
    hit.log_total = total;
    hit.log_visible = visible;

    // Footer: search box or key hints.
    let footer = if view.searching || !view.search.is_empty() {
        format!(
            " search: {}{}   (Enter accept, Esc stop)",
            view.search,
            if view.searching { "▏" } else { "" }
        )
    } else {
        " 1-4 level  s source  / search  f follow  c copy  PgUp/PgDn  Esc close".to_owned()
    };
    buf.set_string(
        area.x,
        area.y + area.height - 1,
        pad(&chrome_label(&footer, icons), area.width as usize),
        Style::default().fg(palette.dim).bg(palette.status_bg),
    );
}

fn pad(s: &str, width: usize) -> String {
    let fitted = fit_prefix(s, width);
    let used = fitted.width();
    format!("{fitted}{}", " ".repeat(width.saturating_sub(used)))
}
