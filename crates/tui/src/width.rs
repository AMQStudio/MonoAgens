//! Width-safe text helpers: control-character sanitising, grapheme-aware fitting and the
//! chrome icon policy (`Appearance -> Icons`).
//!
//! Terminals disagree about the width of emoji (especially with U+FE0F), which leaves stray
//! characters at the right edge. UI chrome therefore drops U+FE0F and, in `Symbols` mode, swaps
//! emoji for single-width symbols. Chat content keeps its emoji.

use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::config::IconMode;

const TAB_WIDTH: usize = 4;

/// Replaces tabs with spaces and drops control characters; `keep_newlines` keeps `\n`.
#[must_use]
pub fn sanitize(s: &str, keep_newlines: bool) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '\t' => out.push_str(&" ".repeat(TAB_WIDTH)),
            '\n' if keep_newlines => out.push('\n'),
            '\n' => out.push(' '),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

/// Display width of a grapheme cluster.
#[must_use]
pub fn grapheme_width(g: &str) -> usize {
    UnicodeWidthStr::width(g)
}

/// Longest prefix of `s` fitting in `max_w` columns, never splitting a grapheme cluster.
#[must_use]
pub fn fit_prefix(s: &str, max_w: usize) -> String {
    let mut out = String::new();
    let mut used = 0usize;
    for g in s.graphemes(true) {
        let w = grapheme_width(g);
        if used + w > max_w {
            break;
        }
        out.push_str(g);
        used += w;
    }
    out
}

/// Single-width symbol standing in for a chrome emoji in `Symbols` mode.
#[must_use]
pub const fn symbol_for(ch: char) -> Option<&'static str> {
    Some(match ch {
        '💬' => "»",
        '🔌' => "¤",
        '🧪' => "λ",
        '✨' => "*",
        '⚙' => "≡",
        '📋' => "§",
        '📎' => "@",
        '🛡' => "◆",
        '🔍' => "?",
        '✏' => "~",
        '✅' => "✓",
        '⚠' => "!",
        '🔒' => "#",
        '🔓' => "o",
        '💡' => "i",
        '⏹' => "■",
        '▶' => "▸",
        '📱' => "▯",
        '🚀' => "^",
        '🔑' => "¬",
        '🔄' => "↻",
        '➕' => "+",
        '🎨' => "◐",
        '🧠' => "Ψ",
        '🤖' => "¤",
        '🔐' => "#",
        '⚡' => "↯",
        '📷' => "▣",
        '📄' => "▤",
        '📝' => "▤",
        '⏱' => "◷",
        _ => return None,
    })
}

/// Rewrites chrome text for the icon mode: always drops variation selectors (U+FE0F/U+FE0E),
/// and in `Symbols` mode replaces known emoji with single-width symbols.
#[must_use]
pub fn chrome_text(s: &str, mode: IconMode) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        if ch == '\u{FE0F}' || ch == '\u{FE0E}' {
            continue;
        }
        match (mode, symbol_for(ch)) {
            (IconMode::Symbols, Some(sym)) => out.push_str(sym),
            _ => out.push(ch),
        }
    }
    out
}

/// Chrome text that is also sanitised for control characters and tabs (single line).
#[must_use]
pub fn chrome_label(s: &str, mode: IconMode) -> String {
    chrome_text(&sanitize(s, false), mode)
}
