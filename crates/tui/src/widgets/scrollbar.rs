//! Vertical scrollbar widget integrated with `viewport` math and `HitMap` recording.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;

use crate::core::hitmap::{HitMap, HitTarget, ScrollId};
use crate::core::viewport::scrollbar_thumb_range;

/// Renders a 1-column vertical scrollbar along `track` and records a [`HitTarget::Scrollbar`]
/// entry in `hitmap` whenever `total_rows > viewport_rows`.
#[allow(clippy::too_many_arguments)]
pub fn render_vertical_scrollbar(
    buf: &mut Buffer,
    track: Rect,
    scroll_id: ScrollId,
    total_rows: usize,
    viewport_rows: usize,
    offset: usize,
    hitmap: &mut HitMap,
    track_style: Style,
    thumb_style: Style,
) {
    if track.width == 0 || track.height == 0 || total_rows <= viewport_rows {
        return;
    }

    let Some((thumb_start, thumb_len)) =
        scrollbar_thumb_range(total_rows, viewport_rows, offset, track.height)
    else {
        return;
    };

    let thumb_end = thumb_start.saturating_add(thumb_len);
    for row in 0..track.height {
        let is_thumb = row >= thumb_start && row < thumb_end;
        if let Some(cell) = buf.cell_mut((track.x, track.y + row)) {
            cell.set_symbol(if is_thumb { "█" } else { "│" });
            cell.set_style(if is_thumb { thumb_style } else { track_style });
        }
    }

    hitmap.record(
        track,
        HitTarget::Scrollbar {
            id: scroll_id,
            track,
            total_rows,
            viewport_rows,
        },
    );
}
