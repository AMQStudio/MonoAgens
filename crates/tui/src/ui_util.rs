//! Small drawing helpers shared by the settings widgets (selector, palette, help, editors).

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Widget};
use unicode_width::UnicodeWidthStr;

use crate::render::{render_scrollbar, scroll_window};
use crate::theme::ThemePalette;
use crate::width::{fit_prefix, sanitize};

/// Fills `area` with the given colours.
pub fn fill(buf: &mut Buffer, area: Rect, bg: ratatui::style::Color, fg: ratatui::style::Color) {
    let style = Style::default().bg(bg).fg(fg);
    for y in area.y..area.y.saturating_add(area.height) {
        for x in area.x..area.x.saturating_add(area.width) {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_style(style);
            }
        }
    }
}

/// Truncates to `max_w` display columns with an ellipsis.
#[must_use]
pub fn trunc(s: &str, max_w: usize) -> String {
    if max_w == 0 {
        return String::new();
    }
    let s = sanitize(s, false);
    if s.width() <= max_w {
        return s;
    }
    let mut out = fit_prefix(&s, max_w.saturating_sub(1));
    out.push('…');
    out
}

/// A rect of at most `w` x `h` centred in `area`.
#[must_use]
pub fn centered(w: u16, h: u16, area: Rect) -> Rect {
    let w = w.min(area.width);
    let h = h.min(area.height);
    Rect::new(
        area.x + area.width.saturating_sub(w) / 2,
        area.y + area.height.saturating_sub(h) / 2,
        w,
        h,
    )
}

/// Draws a bordered, cleared box and returns its inner rect.
pub fn boxed(area: Rect, buf: &mut Buffer, title: &str, palette: &ThemePalette) -> Rect {
    Clear.render(area, buf);
    fill(buf, area, palette.modal_bg, palette.fg);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(palette.accent))
        .title(Span::styled(
            format!(" {title} "),
            Style::default()
                .fg(palette.fg)
                .bg(palette.modal_bg)
                .add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(area);
    block.render(area, buf);
    inner
}

/// One row of a [`render_list`].
pub struct ListRow {
    pub left: String,
    pub right: String,
    /// Dim style (for example a heading or a disabled row).
    pub dim: bool,
}

/// Draws `rows` with a `❯` marker on `selected`, scrolling so it stays visible, and a
/// scrollbar when the rows exceed the area. Returns the first visible row and pushes one hit
/// rect per visible row into `hits`.
pub fn render_list(
    area: Rect,
    buf: &mut Buffer,
    rows: &[ListRow],
    selected: usize,
    offset: usize,
    palette: &ThemePalette,
    hits: &mut Vec<(Rect, usize)>,
) -> usize {
    let total = rows.len();
    let visible = (area.height as usize).min(total);
    let off = scroll_window(selected, offset, visible, total);
    let bar = u16::from(total > visible && area.width > 2);
    let w = area.width.saturating_sub(bar);
    let left_w = ((w as usize) * 2 / 5).max(10).min(w as usize);
    for (i, row) in rows.iter().enumerate().skip(off).take(visible) {
        let y = area.y + (i - off) as u16;
        let rect = Rect::new(area.x, y, w, 1);
        hits.push((rect, i));
        let sel = i == selected;
        let bg = if sel { palette.selection_bg } else { palette.modal_bg };
        fill(buf, rect, bg, palette.fg);
        let marker = if sel { "❯ " } else { "  " };
        let left = format!("{marker}{}", trunc(&row.left, left_w.saturating_sub(3)));
        let left = format!("{left:<left_w$}");
        let fg = if sel {
            palette.accent
        } else if row.dim {
            palette.dim
        } else {
            palette.fg
        };
        let line = Line::from(vec![
            Span::styled(left, Style::default().fg(fg).bg(bg)),
            Span::styled(
                trunc(&row.right, (w as usize).saturating_sub(left_w + 1)),
                Style::default().fg(palette.muted).bg(bg),
            ),
        ]);
        buf.set_line(area.x, y, &line, w);
    }
    if bar == 1 {
        render_scrollbar(
            Rect::new(area.x + area.width - 1, area.y, 1, area.height),
            buf,
            total,
            off,
            visible,
            palette,
        );
    }
    off
}
