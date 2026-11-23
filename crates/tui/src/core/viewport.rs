//! Pure integer viewport, scrollbar, and UTF-8/grapheme cursor-window math.

use crossterm::event::MouseEventKind;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// Returns the maximum valid scroll offset for `total_rows` within `viewport_rows`.
#[must_use]
pub const fn max_scroll_offset(total_rows: usize, viewport_rows: usize) -> usize {
    total_rows.saturating_sub(viewport_rows)
}

/// Clamps `offset` into `0..=max_scroll_offset(total_rows, viewport_rows)`.
#[must_use]
pub const fn clamp_scroll_offset(offset: usize, total_rows: usize, viewport_rows: usize) -> usize {
    let max = max_scroll_offset(total_rows, viewport_rows);
    if offset > max { max } else { offset }
}

/// Adjusts `current_offset` minimally so that `selected` is visible within `viewport_rows`.
#[must_use]
pub const fn scroll_offset_for_row(
    selected: usize,
    current_offset: usize,
    viewport_rows: usize,
    total_rows: usize,
) -> usize {
    if total_rows == 0 || viewport_rows == 0 {
        return 0;
    }
    let max_idx = total_rows - 1;
    let sel = if selected > max_idx {
        max_idx
    } else {
        selected
    };
    let max_off = max_scroll_offset(total_rows, viewport_rows);
    let cur = if current_offset > max_off {
        max_off
    } else {
        current_offset
    };

    if sel < cur {
        sel
    } else if sel >= cur + viewport_rows {
        let next = sel + 1 - viewport_rows;
        if next > max_off { max_off } else { next }
    } else {
        cur
    }
}

/// Computes `(thumb_start_rel, thumb_len)` along a vertical scrollbar track of `track_height`.
///
/// Returns `None` when the content fits in `viewport_rows` or `track_height == 0`.
#[must_use]
pub fn scrollbar_thumb_range(
    total_rows: usize,
    viewport_rows: usize,
    offset: usize,
    track_height: u16,
) -> Option<(u16, u16)> {
    if track_height == 0 || viewport_rows == 0 || total_rows <= viewport_rows {
        return None;
    }

    let track = usize::from(track_height);
    let max_offset = total_rows - viewport_rows;
    let clamped_offset = offset.min(max_offset);

    let thumb_len = ((track.saturating_mul(viewport_rows)) / total_rows).clamp(1, track);
    let max_start = track - thumb_len;
    let thumb_start = if max_start == 0 {
        0
    } else {
        (clamped_offset * max_start + max_offset / 2) / max_offset
    };

    let start_u16 = u16::try_from(thumb_start).unwrap_or(u16::MAX);
    let len_u16 = u16::try_from(thumb_len).unwrap_or(u16::MAX);
    Some((start_u16, len_u16))
}

/// Maps a vertical scrollbar click at `click_y` to a scroll offset in `0..=max_scroll_offset`.
#[must_use]
pub fn offset_from_scrollbar_y(
    track_y: u16,
    track_height: u16,
    click_y: u16,
    total_rows: usize,
    viewport_rows: usize,
) -> usize {
    if track_height == 0 || viewport_rows == 0 || total_rows <= viewport_rows {
        return 0;
    }
    if track_height == 1 {
        return 0;
    }

    let max_offset = total_rows - viewport_rows;
    let max_rel = usize::from(track_height - 1);
    let rel_y = usize::from(click_y.saturating_sub(track_y)).min(max_rel);
    ((rel_y * max_offset) + (max_rel / 2)) / max_rel
}

/// Computes `(byte_start, byte_end)` slice boundaries within `text` such that the display width
/// fits `viewport_cols` and `cursor_byte` remains visible without splitting UTF-8 or graphemes.
#[must_use]
pub fn horizontal_cursor_window(
    text: &str,
    cursor_byte: usize,
    viewport_cols: usize,
) -> (usize, usize) {
    if text.is_empty() {
        return (0, 0);
    }

    let mut graphemes: Vec<(usize, usize, usize)> = Vec::new();
    for (start, g) in text.grapheme_indices(true) {
        let end = start + g.len();
        let width = UnicodeWidthStr::width(g);
        graphemes.push((start, end, width));
    }

    let clamped = cursor_byte.min(text.len());
    // Snap cursor_byte to the containing/preceding grapheme start (or text.len() if at end).
    let c_idx = if clamped == text.len() {
        graphemes.len()
    } else {
        graphemes
            .iter()
            .position(|(start, end, _)| clamped >= *start && clamped < *end)
            .unwrap_or(graphemes.len())
    };
    let snapped_cursor = if c_idx == graphemes.len() {
        text.len()
    } else {
        graphemes[c_idx].0
    };

    if viewport_cols == 0 {
        return (snapped_cursor, snapped_cursor);
    }

    // Include the grapheme under the cursor when cursor is inside the text.
    let target_end_idx = if c_idx < graphemes.len() {
        c_idx + 1
    } else {
        graphemes.len()
    };

    if c_idx < graphemes.len() && graphemes[c_idx].2 > viewport_cols {
        return (snapped_cursor, snapped_cursor);
    }

    let prefix_width: usize = graphemes[..target_end_idx].iter().map(|g| g.2).sum();
    let (start_idx, mut used_width) = if prefix_width <= viewport_cols {
        (0, prefix_width)
    } else {
        let mut s = target_end_idx;
        let mut acc = 0usize;
        while s > 0 {
            let w = graphemes[s - 1].2;
            if acc + w > viewport_cols {
                break;
            }
            acc += w;
            s -= 1;
        }
        (s, acc)
    };

    let mut end_idx = target_end_idx;
    while end_idx < graphemes.len() {
        let w = graphemes[end_idx].2;
        if used_width + w > viewport_cols {
            break;
        }
        used_width += w;
        end_idx += 1;
    }

    let byte_start = if start_idx < graphemes.len() {
        graphemes[start_idx].0
    } else {
        text.len()
    };
    let byte_end = if end_idx == 0 {
        0
    } else {
        graphemes[end_idx - 1].1
    };
    (byte_start, byte_end)
}

/// Returns `true` when `kind` is a horizontal mouse scroll event.
#[must_use]
pub const fn is_horizontal_scroll(kind: MouseEventKind) -> bool {
    matches!(
        kind,
        MouseEventKind::ScrollLeft | MouseEventKind::ScrollRight
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scroll_offset_bounds_and_selection_tracking() {
        assert_eq!(max_scroll_offset(0, 10), 0);
        assert_eq!(max_scroll_offset(5, 10), 0);
        assert_eq!(max_scroll_offset(10, 10), 0);
        assert_eq!(max_scroll_offset(25, 10), 15);

        assert_eq!(clamp_scroll_offset(99, 25, 10), 15);
        assert_eq!(clamp_scroll_offset(3, 25, 10), 3);

        // Selection tracking within bounds and zero-height edge cases.
        assert_eq!(scroll_offset_for_row(5, 2, 0, 10), 0);
        assert_eq!(scroll_offset_for_row(0, 0, 1, 1), 0);
        assert_eq!(scroll_offset_for_row(4, 0, 5, 10), 0);
        assert_eq!(scroll_offset_for_row(5, 0, 5, 10), 1);
        assert_eq!(scroll_offset_for_row(1, 3, 5, 10), 1);
        assert_eq!(scroll_offset_for_row(99, 0, 5, 10), 5);
    }

    #[test]
    fn scrollbar_thumb_and_click_roundtrip() {
        assert_eq!(scrollbar_thumb_range(10, 10, 0, 10), None);
        assert_eq!(scrollbar_thumb_range(100, 10, 0, 0), None);
        assert_eq!(scrollbar_thumb_range(100, 0, 0, 10), None);

        let (start_top, len) = scrollbar_thumb_range(100, 20, 0, 20).expect("thumb");
        assert_eq!(start_top, 0);
        assert_eq!(len, 4);

        let (start_bot, len_bot) = scrollbar_thumb_range(100, 20, 80, 20).expect("thumb");
        assert_eq!(len_bot, 4);
        assert_eq!(start_bot + len_bot, 20);

        // Single-row track edge case.
        let (s1, l1) = scrollbar_thumb_range(10, 2, 5, 1).expect("single-row thumb");
        assert_eq!((s1, l1), (0, 1));
        assert_eq!(offset_from_scrollbar_y(5, 1, 5, 10, 2), 0);

        // Roundtrip at top, middle, bottom.
        assert_eq!(offset_from_scrollbar_y(5, 20, 5, 100, 20), 0);
        assert_eq!(offset_from_scrollbar_y(5, 20, 24, 100, 20), 80);

        let mid_off = offset_from_scrollbar_y(5, 20, 15, 100, 20);
        let (mid_thumb, _) = scrollbar_thumb_range(100, 20, mid_off, 20).expect("mid thumb");
        assert!((7..=9).contains(&mid_thumb));
    }

    #[test]
    fn horizontal_cursor_window_utf8_and_wide_chars() {
        assert_eq!(horizontal_cursor_window("", 0, 5), (0, 0));
        assert_eq!(horizontal_cursor_window("abcde", 5, 5), (0, 5));
        assert_eq!(horizontal_cursor_window("abcdefgh", 8, 5), (3, 8));
        assert_eq!(horizontal_cursor_window("abcdefgh", 2, 5), (0, 5));

        // Wide CJK characters (2 columns each, 3 bytes each).
        let text = "你好世界";
        // Cursor at '世' (byte 6), viewport 4 cols -> "好世" (bytes 3..9).
        let (s, e) = horizontal_cursor_window(text, 6, 4);
        assert_eq!(&text[s..e], "好世");

        // Mid-UTF-8 cursor byte never panics or slices mid-char.
        let (s2, e2) = horizontal_cursor_window(text, 7, 4);
        assert!(text.is_char_boundary(s2));
        assert!(text.is_char_boundary(e2));
        assert_eq!(&text[s2..e2], "好世");

        // Combining grapheme cluster (e + U+0301) stays intact.
        let combined = "ae\u{0301}bc";
        let (sc, ec) = horizontal_cursor_window(combined, 1, 2);
        assert_eq!(&combined[sc..ec], "ae\u{0301}");
    }

    #[test]
    fn horizontal_scroll_detection() {
        assert!(is_horizontal_scroll(MouseEventKind::ScrollLeft));
        assert!(is_horizontal_scroll(MouseEventKind::ScrollRight));
        assert!(!is_horizontal_scroll(MouseEventKind::ScrollUp));
        assert!(!is_horizontal_scroll(MouseEventKind::ScrollDown));
    }
}
