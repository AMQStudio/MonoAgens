//! Chrome glyph set (strictly single-cell width), palette re-exports, and backdrop helpers.

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::Clear;

use crate::config::IconMode;
pub use crate::theme::{ThemePalette, ThemePreset, no_color_active};

/// Single-width chrome glyphs across all UI icon modes (never 2-cell emoji).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChromeGlyphs {
    pub tab_appearance: &'static str,
    pub tab_soul: &'static str,
    pub tab_model: &'static str,
    pub tab_valve: &'static str,
    pub tab_rbac: &'static str,
    pub tab_providers: &'static str,
    pub tab_plugins: &'static str,
    pub tab_runtime: &'static str,
    pub dot_on: &'static str,
    pub dot_starting: &'static str,
    pub dot_off: &'static str,
    pub check_on: &'static str,
    pub check_off: &'static str,
    pub check_partial: &'static str,
    pub lock_owner: &'static str,
    pub pointer: &'static str,
    pub arrow_right: &'static str,
    pub info: &'static str,
    pub search: &'static str,
    pub bullet: &'static str,
}

impl ChromeGlyphs {
    /// Returns strictly single-width glyphs for `mode`.
    #[must_use]
    pub const fn for_mode(mode: IconMode) -> Self {
        match mode {
            IconMode::Symbols => Self {
                tab_appearance: "◐",
                tab_soul: "Ψ",
                tab_model: "¤",
                tab_valve: "◆",
                tab_rbac: "#",
                tab_providers: "¬",
                tab_plugins: "+",
                tab_runtime: "≡",
                dot_on: "●",
                dot_starting: "◐",
                dot_off: "○",
                check_on: "✓",
                check_off: " ",
                check_partial: "–",
                lock_owner: "*",
                pointer: "›",
                arrow_right: "»",
                info: "i",
                search: "?",
                bullet: "•",
            },
            IconMode::Emoji => Self {
                tab_appearance: "◈",
                tab_soul: "Ψ",
                tab_model: "◎",
                tab_valve: "◆",
                tab_rbac: "❖",
                tab_providers: "◇",
                tab_plugins: "✦",
                tab_runtime: "≡",
                dot_on: "●",
                dot_starting: "◐",
                dot_off: "○",
                check_on: "✓",
                check_off: "□",
                check_partial: "–",
                lock_owner: "×",
                pointer: "▸",
                arrow_right: "»",
                info: "i",
                search: "?",
                bullet: "•",
            },
        }
    }
}

/// Clears `rect` inside `frame` before drawing a floating overlay.
pub fn clear_overlay_area(frame: &mut Frame<'_>, rect: Rect) {
    frame.render_widget(Clear, rect);
}

/// Dims an existing buffer region as a modal backdrop.
pub fn render_dimmed_backdrop(buf: &mut Buffer, area: Rect, palette: &ThemePalette) {
    let clipped = area.intersection(buf.area);
    let dim_style = if palette.no_color {
        Style::default().add_modifier(Modifier::DIM)
    } else {
        Style::default()
            .fg(palette.dim)
            .bg(palette.bg)
            .add_modifier(Modifier::DIM)
    };

    for y in clipped.top()..clipped.bottom() {
        for x in clipped.left()..clipped.right() {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_style(dim_style);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use unicode_width::UnicodeWidthStr;

    use super::*;

    #[test]
    fn all_chrome_glyphs_are_strictly_single_cell() {
        for mode in [IconMode::Symbols, IconMode::Emoji] {
            let g = ChromeGlyphs::for_mode(mode);
            let fields = [
                g.tab_appearance,
                g.tab_soul,
                g.tab_model,
                g.tab_valve,
                g.tab_rbac,
                g.tab_providers,
                g.tab_plugins,
                g.tab_runtime,
                g.dot_on,
                g.dot_starting,
                g.dot_off,
                g.check_on,
                g.check_off,
                g.check_partial,
                g.lock_owner,
                g.pointer,
                g.arrow_right,
                g.info,
                g.search,
                g.bullet,
            ];
            for glyph in fields {
                assert_eq!(
                    UnicodeWidthStr::width(glyph),
                    1,
                    "glyph {glyph:?} in {mode:?} must have display width 1"
                );
            }
        }
    }

    #[test]
    fn dimmed_backdrop_applies_dim_modifier() {
        let area = Rect::new(0, 0, 4, 2);
        let mut buf = Buffer::empty(area);
        let palette = ThemePalette::for_preset_with_no_color(ThemePreset::Titanium, false);
        render_dimmed_backdrop(&mut buf, area, &palette);
        let cell = buf.cell((1, 1)).expect("cell in bounds");
        assert!(cell.modifier.contains(Modifier::DIM));
        assert_eq!(cell.fg, palette.dim);
        assert_eq!(cell.bg, palette.bg);
    }
}
