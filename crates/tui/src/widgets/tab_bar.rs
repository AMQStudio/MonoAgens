//! Responsive tab bar with progressive short-label collapse and click hit-zones.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::core::hitmap::{HitMap, HitTarget};
use crate::core::theme::ThemePalette;

/// Single tab entry in a [`TabBar`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TabItem {
    pub label: String,
    pub short_label: String,
    pub enabled: bool,
    pub badge: Option<String>,
}

/// Horizontal tab bar state tracking available tabs and the active index.
#[derive(Debug, Clone, Default)]
pub struct TabBar {
    pub items: Vec<TabItem>,
    pub active: usize,
}

impl TabBar {
    /// Creates a new tab bar, selecting the first enabled tab (or `0` if none).
    #[must_use]
    pub fn new(items: Vec<TabItem>) -> Self {
        let active = items.iter().position(|item| item.enabled).unwrap_or(0);
        Self { items, active }
    }

    /// Advances `active` to the next enabled tab (wrapping around).
    pub fn select_next(&mut self) {
        let len = self.items.len();
        if len == 0 {
            return;
        }
        let start = self.active.min(len - 1);
        for step in 1..=len {
            let idx = (start + step) % len;
            if self.items[idx].enabled {
                self.active = idx;
                return;
            }
        }
    }

    /// Moves `active` to the previous enabled tab (wrapping around).
    pub fn select_prev(&mut self) {
        let len = self.items.len();
        if len == 0 {
            return;
        }
        let start = self.active.min(len - 1);
        for step in 1..=len {
            let idx = (start + len - step) % len;
            if self.items[idx].enabled {
                self.active = idx;
                return;
            }
        }
    }

    /// Selects tab `idx` if it exists and is enabled.
    pub fn select(&mut self, idx: usize) -> bool {
        if let Some(item) = self.items.get(idx)
            && item.enabled
        {
            self.active = idx;
            return true;
        }
        false
    }

    /// Returns a reference to the currently active tab item, if any.
    #[must_use]
    pub fn active_item(&self) -> Option<&TabItem> {
        self.items.get(self.active)
    }
}

/// Renders `bar` onto the first row of `area` without wrapping or overflowing `area.width`,
/// and records [`HitTarget::Tab`] rectangles for every visible enabled tab.
pub fn render_tab_bar(
    area: Rect,
    buf: &mut Buffer,
    bar: &TabBar,
    palette: &ThemePalette,
    hitmap: &mut HitMap,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    let base_style = Style::default().fg(palette.fg).bg(palette.modal_bg);
    fill_row(buf, area.x, area.y, area.width, base_style);

    if bar.items.is_empty() {
        return;
    }

    let max_w = usize::from(area.width);
    let active_idx = bar.active.min(bar.items.len() - 1);

    let full_labels: Vec<String> = bar
        .items
        .iter()
        .map(|item| format_tab(item, false, true))
        .collect();
    let short_badge_labels: Vec<String> = bar
        .items
        .iter()
        .map(|item| format_tab(item, true, true))
        .collect();
    let short_labels: Vec<String> = bar
        .items
        .iter()
        .map(|item| format_tab(item, true, false))
        .collect();

    if total_width(&full_labels) <= max_w {
        draw_slice(
            area,
            buf,
            bar,
            0..bar.items.len(),
            &full_labels,
            false,
            false,
            palette,
            hitmap,
        );
        return;
    }

    if total_width(&short_badge_labels) <= max_w {
        draw_slice(
            area,
            buf,
            bar,
            0..bar.items.len(),
            &short_badge_labels,
            false,
            false,
            palette,
            hitmap,
        );
        return;
    }

    if total_width(&short_labels) <= max_w {
        draw_slice(
            area,
            buf,
            bar,
            0..bar.items.len(),
            &short_labels,
            false,
            false,
            palette,
            hitmap,
        );
        return;
    }

    // Tight viewport fallback: center a window around `active_idx` and expand to neighbors
    // while keeping within `max_w`.
    let mut start = active_idx;
    let mut end = active_idx;
    let len = bar.items.len();

    loop {
        let mut expanded = false;
        if end + 1 < len {
            let next_end = end + 1;
            if window_width(&short_labels, start, next_end, len) <= max_w {
                end = next_end;
                expanded = true;
            }
        }
        if start > 0 {
            let prev_start = start - 1;
            if window_width(&short_labels, prev_start, end, len) <= max_w {
                start = prev_start;
                expanded = true;
            }
        }
        if !expanded {
            break;
        }
    }

    let left_arrow = start > 0;
    let right_arrow = end + 1 < len;
    draw_slice(
        area,
        buf,
        bar,
        start..=end,
        &short_labels,
        left_arrow,
        right_arrow,
        palette,
        hitmap,
    );
}

#[allow(clippy::too_many_arguments)]
fn draw_slice(
    area: Rect,
    buf: &mut Buffer,
    bar: &TabBar,
    range: impl IntoIterator<Item = usize>,
    labels: &[String],
    left_arrow: bool,
    right_arrow: bool,
    palette: &ThemePalette,
    hitmap: &mut HitMap,
) {
    let right_edge = area.x.saturating_add(area.width);
    let mut cur_x = area.x;
    let arrow_style = Style::default().fg(palette.dim).bg(palette.modal_bg);

    // Reserve 1 column on the right for the overflow chevron when tabs are hidden right.
    let reserve_right: u16 = if right_arrow && area.width >= 4 { 1 } else { 0 };

    if left_arrow && cur_x + 1 < right_edge.saturating_sub(reserve_right) {
        buf.set_line(cur_x, area.y, &Line::from(Span::styled("‹", arrow_style)), 1);
        cur_x += 1;
    }

    let mut first = true;
    for idx in range {
        let limit_x = right_edge.saturating_sub(reserve_right);
        if cur_x >= limit_x {
            break;
        }
        if !first {
            if cur_x + 1 >= limit_x {
                break;
            }
            cur_x += 1;
        }
        first = false;

        let avail = usize::from(limit_x.saturating_sub(cur_x));
        if avail == 0 {
            break;
        }

        let raw = &labels[idx];
        let text = truncate_to_width(raw, avail);
        let drawn_w = u16::try_from(display_width(&text)).unwrap_or(u16::MAX).min(limit_x - cur_x);
        if drawn_w == 0 {
            continue;
        }

        let item = &bar.items[idx];
        let is_active = idx == bar.active;
        let style = if is_active {
            Style::default()
                .fg(palette.accent_fg)
                .bg(palette.accent)
                .add_modifier(Modifier::BOLD)
        } else if item.enabled {
            Style::default().fg(palette.fg).bg(palette.modal_bg)
        } else {
            Style::default().fg(palette.dim).bg(palette.modal_bg)
        };

        buf.set_line(cur_x, area.y, &Line::from(Span::styled(text, style)), drawn_w);
        if item.enabled {
            hitmap.record(Rect::new(cur_x, area.y, drawn_w, 1), HitTarget::Tab(idx));
        }
        cur_x = cur_x.saturating_add(drawn_w);
    }

    if reserve_right == 1 && right_edge > area.x {
        buf.set_line(
            right_edge - 1,
            area.y,
            &Line::from(Span::styled("›", arrow_style)),
            1,
        );
    }
}

fn format_tab(item: &TabItem, use_short: bool, include_badge: bool) -> String {
    let base = if use_short && !item.short_label.is_empty() {
        item.short_label.as_str()
    } else {
        item.label.as_str()
    };
    if include_badge && let Some(badge) = item.badge.as_deref().filter(|b| !b.is_empty()) {
        format!(" {base} [{badge}] ")
    } else {
        format!(" {base} ")
    }
}

fn total_width(labels: &[String]) -> usize {
    if labels.is_empty() {
        return 0;
    }
    let text_w: usize = labels.iter().map(|s| display_width(s)).sum();
    text_w + labels.len().saturating_sub(1)
}

fn window_width(labels: &[String], start: usize, end: usize, total: usize) -> usize {
    let count = end - start + 1;
    let text_w: usize = labels[start..=end].iter().map(|s| display_width(s)).sum();
    let sep_w = count.saturating_sub(1);
    let left_w = usize::from(start > 0);
    let right_w = usize::from(end + 1 < total);
    left_w + text_w + sep_w + right_w
}

fn fill_row(buf: &mut Buffer, x: u16, y: u16, width: u16, style: Style) {
    for col in x..x.saturating_add(width) {
        if let Some(cell) = buf.cell_mut((col, y)) {
            cell.set_symbol(" ");
            cell.set_style(style);
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

    fn sample_tabs() -> Vec<TabItem> {
        vec![
            TabItem {
                label: "Appearance".to_owned(),
                short_label: "Look".to_owned(),
                enabled: true,
                badge: None,
            },
            TabItem {
                label: "Providers".to_owned(),
                short_label: "Prov".to_owned(),
                enabled: false,
                badge: Some("0".to_owned()),
            },
            TabItem {
                label: "Permissions".to_owned(),
                short_label: "RBAC".to_owned(),
                enabled: true,
                badge: Some("3".to_owned()),
            },
            TabItem {
                label: "Plugins".to_owned(),
                short_label: "Plug".to_owned(),
                enabled: true,
                badge: None,
            },
            TabItem {
                label: "Runtime".to_owned(),
                short_label: "Run".to_owned(),
                enabled: true,
                badge: None,
            },
        ]
    }

    fn row_text(buf: &Buffer, area: Rect) -> String {
        (area.x..area.x + area.width)
            .map(|x| buf.cell((x, area.y)).map_or(" ", |c| c.symbol()))
            .collect()
    }

    #[test]
    fn navigation_skips_disabled_tabs() {
        let mut bar = TabBar::new(sample_tabs());
        assert_eq!(bar.active, 0);
        assert!(!bar.select(1));
        assert_eq!(bar.active, 0);

        bar.select_next();
        assert_eq!(bar.active, 2);
        bar.select_prev();
        assert_eq!(bar.active, 0);
        bar.select_prev();
        assert_eq!(bar.active, 4);
        assert_eq!(bar.active_item().map(|t| t.label.as_str()), Some("Runtime"));
    }

    #[test]
    fn wide_viewport_renders_full_labels_and_records_enabled_hits() {
        let area = Rect::new(0, 0, 80, 1);
        let mut buf = Buffer::empty(area);
        let palette = ThemePalette::for_preset_with_no_color(ThemePreset::Titanium, false);
        let mut hitmap = HitMap::default();
        let bar = TabBar::new(sample_tabs());

        render_tab_bar(area, &mut buf, &bar, &palette, &mut hitmap);
        let rendered = row_text(&buf, area);
        assert!(rendered.contains("Appearance"));
        assert!(rendered.contains("Permissions [3]"));

        let mut hit_tabs = Vec::new();
        for x in 0..80 {
            if let Some(HitTarget::Tab(idx)) = hitmap.hit_test(x, 0)
                && !hit_tabs.contains(idx)
            {
                hit_tabs.push(*idx);
            }
        }
        assert_eq!(hit_tabs, vec![0, 2, 3, 4]);
    }

    #[test]
    fn narrow_40x12_viewport_collapses_without_overflow() {
        let area = Rect::new(0, 0, 26, 1);
        let mut buf = Buffer::empty(area);
        let palette = ThemePalette::for_preset_with_no_color(ThemePreset::Titanium, false);
        let mut hitmap = HitMap::default();
        let mut bar = TabBar::new(sample_tabs());
        assert!(bar.select(2));

        render_tab_bar(area, &mut buf, &bar, &palette, &mut hitmap);
        let rendered = row_text(&buf, area);
        assert!(rendered.contains("RBAC"));
        assert!(display_width(&rendered) <= 26);

        let active_hit = (0..26).any(|x| hitmap.hit_test(x, 0) == Some(&HitTarget::Tab(2)));
        assert!(active_hit);
    }

    #[test]
    fn ultra_narrow_width_truncates_active_tab_safely() {
        let area = Rect::new(0, 0, 8, 1);
        let mut buf = Buffer::empty(area);
        let palette = ThemePalette::for_preset_with_no_color(ThemePreset::Titanium, false);
        let mut hitmap = HitMap::default();
        let mut bar = TabBar::new(sample_tabs());
        assert!(bar.select(2));

        render_tab_bar(area, &mut buf, &bar, &palette, &mut hitmap);
        assert!(hitmap.hit_test(1, 0) == Some(&HitTarget::Tab(2)));
    }
}
