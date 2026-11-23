//! Centered 2-pane overlay frame with pinned header, responsive tabs,
//! optional inspector, and pinned 1-row contextual footer strip.

use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Widget};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::core::hitmap::{HitMap, HitTarget};
use crate::core::input::TextInput;
use crate::core::overlay::OverlayId;
use crate::core::theme::{ThemePalette, render_dimmed_backdrop};
use crate::widgets::tab_bar::{TabBar, render_tab_bar};

/// Interactive action chip rendered inside [`FooterMode::Chips`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FooterChip {
    pub label: String,
    pub active: bool,
    pub danger: bool,
}

/// Content mode for the pinned 1-row hub footer strip.
pub enum FooterMode<'a> {
    Hints(&'a [(&'a str, &'a str)]),
    Chips(&'a [FooterChip]),
    InlineInput {
        prompt: &'a str,
        input: &'a TextInput,
        hint: &'a str,
    },
}

/// Sizing and structural layout configuration for a hub modal frame.
#[derive(Debug, Clone)]
pub struct HubFrameConfig<'a> {
    pub overlay_id: OverlayId,
    pub title: String,
    pub max_width: u16,
    pub max_height: u16,
    pub width_pct: u16,
    pub height_pct: u16,
    pub tab_bar: Option<&'a TabBar>,
    pub rail_width: u16,
    pub inspector_height: u16,
}

/// Computed rectangles for every region of a hub modal frame.
#[derive(Debug, Clone, Copy, Default)]
pub struct HubFrameAreas {
    pub outer: Rect,
    pub inner: Rect,
    pub header: Rect,
    pub tab_bar: Option<Rect>,
    pub rail: Option<Rect>,
    pub main: Rect,
    pub inspector: Option<Rect>,
    pub footer: Rect,
}

/// Computes centered modal geometry clamped cleanly to `screen`.
///
/// On compact terminals (including `40x12`), the modal expands to the full `screen`
/// and drops the left rail below `inner.width < 56` so `main` keeps full width.
#[must_use]
pub fn compute_hub_areas(screen: Rect, cfg: &HubFrameConfig<'_>) -> HubFrameAreas {
    if screen.width == 0 || screen.height == 0 {
        return HubFrameAreas::default();
    }

    let outer = if screen.width < 50 || screen.height < 18 {
        screen
    } else {
        let pct_w = (u32::from(screen.width) * u32::from(cfg.width_pct.min(100))) / 100;
        let pct_h = (u32::from(screen.height) * u32::from(cfg.height_pct.min(100))) / 100;
        let max_w = if cfg.max_width == 0 {
            screen.width
        } else {
            cfg.max_width
        };
        let max_h = if cfg.max_height == 0 {
            screen.height
        } else {
            cfg.max_height
        };
        let w = (pct_w as u16).min(max_w).min(screen.width).max(1);
        let h = (pct_h as u16).min(max_h).min(screen.height).max(1);
        Rect::new(
            screen.x + screen.width.saturating_sub(w) / 2,
            screen.y + screen.height.saturating_sub(h) / 2,
            w,
            h,
        )
    };

    let bordered = outer.width >= 4 && outer.height >= 5;
    let inner = if bordered {
        Rect::new(outer.x + 1, outer.y + 1, outer.width - 2, outer.height - 2)
    } else {
        outer
    };

    if inner.height <= 1 {
        return HubFrameAreas {
            outer,
            inner,
            header: inner,
            tab_bar: None,
            rail: None,
            main: Rect::default(),
            inspector: None,
            footer: inner,
        };
    }

    let show_tabs = cfg.tab_bar.is_some() && inner.height >= 4;
    let mut constraints = Vec::with_capacity(4);
    constraints.push(Constraint::Length(1)); // header
    if show_tabs {
        constraints.push(Constraint::Length(1)); // tab_bar
    }
    constraints.push(Constraint::Min(1)); // body
    constraints.push(Constraint::Length(1)); // footer

    let v_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(inner);

    let header = v_chunks[0];
    let (tab_bar, body, footer) = if show_tabs {
        (Some(v_chunks[1]), v_chunks[2], v_chunks[3])
    } else {
        (None, v_chunks[1], v_chunks[2])
    };

    let (body_top, inspector) =
        if cfg.inspector_height > 0 && body.height >= cfg.inspector_height.saturating_add(3) {
            let split = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Min(2),
                    Constraint::Length(cfg.inspector_height),
                ])
                .split(body);
            (split[0], Some(split[1]))
        } else {
            (body, None)
        };

    let (rail, main) = if cfg.rail_width > 0 && inner.width >= 56 {
        let rail_w = cfg.rail_width.min(body_top.width.saturating_sub(20)).max(1);
        let rail_rect = Rect::new(body_top.x, body_top.y, rail_w, body_top.height);
        // Reserve 1 column between `rail` and `main` for the vertical divider line.
        let main_x = body_top.x.saturating_add(rail_w + 1);
        let main_w = body_top.width.saturating_sub(rail_w + 1);
        let main_rect = Rect::new(main_x, body_top.y, main_w, body_top.height);
        (Some(rail_rect), main_rect)
    } else {
        (None, body_top)
    };

    HubFrameAreas {
        outer,
        inner,
        header,
        tab_bar,
        rail,
        main,
        inspector,
        footer,
    }
}

/// Renders the dimmed backdrop, cleared modal surface, border, header title, optional tab bar,
/// rail/main divider, and pinned 1-row footer, recording backdrop, overlay body, tab, and footer
/// chip regions in `hitmap`.
pub fn render_hub_frame<'a>(
    frame_area: Rect,
    buf: &mut Buffer,
    cfg: &HubFrameConfig<'a>,
    footer: FooterMode<'a>,
    palette: &ThemePalette,
    hitmap: &mut HitMap,
) -> HubFrameAreas {
    render_dimmed_backdrop(buf, frame_area, palette);
    hitmap.record(frame_area, HitTarget::Backdrop(cfg.overlay_id));

    let areas = compute_hub_areas(frame_area, cfg);
    if areas.outer.width == 0 || areas.outer.height == 0 {
        return areas;
    }

    // Clear underlying cells completely so background text/glyphs never bleed through.
    Clear.render(areas.outer, buf);
    let modal_style = if palette.no_color {
        Style::default()
    } else {
        Style::default().fg(palette.fg).bg(palette.modal_bg)
    };
    fill_rect(buf, areas.outer, modal_style);

    // Record OverlayBody before tabs and footer chips so reverse hit-testing resolves
    // specific interactive controls on top of the modal body.
    hitmap.record(areas.outer, HitTarget::OverlayBody(cfg.overlay_id));

    if areas.outer.width >= 4 && areas.outer.height >= 5 {
        let block = Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(palette.accent).bg(palette.modal_bg))
            .style(modal_style);
        block.render(areas.outer, buf);
    }

    // Render title on row 0 (`areas.header`).
    let header_style = Style::default()
        .fg(palette.fg)
        .bg(palette.modal_bg)
        .add_modifier(Modifier::BOLD);
    fill_rect(buf, areas.header, header_style);
    if areas.header.width > 0 && areas.header.height > 0 {
        let title_text = truncate_to_width(&cfg.title, usize::from(areas.header.width));
        buf.set_line(
            areas.header.x,
            areas.header.y,
            &Line::from(Span::styled(title_text, header_style)),
            areas.header.width,
        );
    }

    if let (Some(tab_rect), Some(bar)) = (areas.tab_bar, cfg.tab_bar) {
        render_tab_bar(tab_rect, buf, bar, palette, hitmap);
    }

    if let Some(rail_rect) = areas.rail {
        let sep_x = rail_rect.x.saturating_add(rail_rect.width);
        let sep_style = Style::default().fg(palette.separator).bg(palette.modal_bg);
        for y in rail_rect.top()..rail_rect.bottom() {
            if let Some(cell) = buf.cell_mut((sep_x, y)) {
                cell.set_symbol("│");
                cell.set_style(sep_style);
            }
        }
    }

    render_footer(areas.footer, buf, footer, palette, hitmap);
    areas
}

fn render_footer(
    area: Rect,
    buf: &mut Buffer,
    footer: FooterMode<'_>,
    palette: &ThemePalette,
    hitmap: &mut HitMap,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    let base_style = Style::default().fg(palette.muted).bg(palette.modal_bg);
    fill_rect(buf, area, base_style);
    let max_w = usize::from(area.width);

    match footer {
        FooterMode::Hints(hints) => {
            let key_style = Style::default()
                .fg(palette.accent)
                .bg(palette.modal_bg)
                .add_modifier(Modifier::BOLD);
            let label_style = Style::default().fg(palette.muted).bg(palette.modal_bg);

            let mut cur_x = area.x;
            let mut used_w = 0usize;
            for (idx, (key, label)) in hints.iter().enumerate() {
                let sep_w = if idx == 0 { 0 } else { 2 };
                let pair_text = if label.is_empty() {
                    (*key).to_owned()
                } else {
                    format!("{key} {label}")
                };
                let item_w = display_width(&pair_text);

                if used_w + sep_w + item_w > max_w {
                    if idx == 0 {
                        let clipped = truncate_to_width(&pair_text, max_w);
                        let clipped_w = u16::try_from(display_width(&clipped)).unwrap_or(area.width);
                        buf.set_line(
                            cur_x,
                            area.y,
                            &Line::from(Span::styled(clipped, label_style)),
                            clipped_w.min(area.width),
                        );
                    }
                    break;
                }

                if sep_w > 0 {
                    cur_x = cur_x.saturating_add(sep_w as u16);
                    used_w += sep_w;
                }

                let key_w = display_width(key);
                buf.set_line(
                    cur_x,
                    area.y,
                    &Line::from(Span::styled(*key, key_style)),
                    key_w as u16,
                );
                cur_x = cur_x.saturating_add(key_w as u16);

                if !label.is_empty() {
                    let rest = format!(" {label}");
                    let rest_w = display_width(&rest);
                    buf.set_line(
                        cur_x,
                        area.y,
                        &Line::from(Span::styled(rest, label_style)),
                        rest_w as u16,
                    );
                    cur_x = cur_x.saturating_add(rest_w as u16);
                }

                used_w += item_w;
            }
        }
        FooterMode::Chips(chips) => {
            let mut cur_x = area.x;
            let mut used_w = 0usize;
            for (idx, chip) in chips.iter().enumerate() {
                let sep_w = usize::from(idx > 0);
                let text = format!("[{}]", chip.label);
                let chip_w = display_width(&text);

                if used_w + sep_w + chip_w > max_w {
                    if idx == 0 {
                        let clipped = truncate_to_width(&text, max_w);
                        let w = u16::try_from(display_width(&clipped))
                            .unwrap_or(area.width)
                            .min(area.width);
                        if w > 0 {
                            let style = chip_style(chip, palette);
                            buf.set_line(cur_x, area.y, &Line::from(Span::styled(clipped, style)), w);
                            hitmap.record(Rect::new(cur_x, area.y, w, 1), HitTarget::FooterChip(idx));
                        }
                    }
                    break;
                }

                if sep_w > 0 {
                    cur_x = cur_x.saturating_add(1);
                    used_w += 1;
                }

                let w = u16::try_from(chip_w).unwrap_or(area.width);
                let style = chip_style(chip, palette);
                buf.set_line(cur_x, area.y, &Line::from(Span::styled(text, style)), w);
                hitmap.record(Rect::new(cur_x, area.y, w, 1), HitTarget::FooterChip(idx));
                cur_x = cur_x.saturating_add(w);
                used_w += chip_w;
            }
        }
        FooterMode::InlineInput {
            prompt,
            input,
            hint,
        } => {
            let prompt_text = truncate_to_width(prompt, max_w);
            let prompt_w = display_width(&prompt_text);
            let prompt_style = Style::default()
                .fg(palette.accent)
                .bg(palette.modal_bg)
                .add_modifier(Modifier::BOLD);
            if prompt_w > 0 {
                buf.set_line(
                    area.x,
                    area.y,
                    &Line::from(Span::styled(prompt_text, prompt_style)),
                    prompt_w as u16,
                );
            }

            let rem_w = max_w.saturating_sub(prompt_w);
            if rem_w == 0 {
                return;
            }

            let hint_full = if hint.is_empty() {
                String::new()
            } else {
                format!("  {hint}")
            };
            let hint_w = display_width(&hint_full);
            let show_hint = !hint_full.is_empty() && rem_w >= hint_w.saturating_add(8);
            let avail_w = if show_hint {
                rem_w - hint_w
            } else {
                rem_w
            };

            let (visible, cursor_col) = input.visible_slice(avail_w);
            let input_x = area.x.saturating_add(prompt_w as u16);
            let input_style = Style::default().fg(palette.fg).bg(palette.modal_bg);
            if !visible.is_empty() && avail_w > 0 {
                buf.set_line(
                    input_x,
                    area.y,
                    &Line::from(Span::styled(visible, input_style)),
                    avail_w as u16,
                );
            }

            if avail_w > 0 {
                let clamped_col = cursor_col.min(avail_w.saturating_sub(1));
                let cursor_x = input_x.saturating_add(clamped_col as u16);
                if let Some(cell) = buf.cell_mut((cursor_x, area.y)) {
                    let cursor_style = if palette.no_color {
                        Style::default().add_modifier(Modifier::REVERSED)
                    } else {
                        Style::default()
                            .fg(palette.accent_fg)
                            .bg(palette.accent)
                            .add_modifier(Modifier::BOLD)
                    };
                    cell.set_style(cursor_style);
                }
            }

            if show_hint {
                let hint_x = input_x.saturating_add(avail_w as u16);
                let hint_style = Style::default().fg(palette.dim).bg(palette.modal_bg);
                buf.set_line(
                    hint_x,
                    area.y,
                    &Line::from(Span::styled(hint_full, hint_style)),
                    hint_w as u16,
                );
            }
        }
    }
}

fn chip_style(chip: &FooterChip, palette: &ThemePalette) -> Style {
    if chip.danger {
        if chip.active {
            Style::default()
                .fg(palette.accent_fg)
                .bg(palette.status_red)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
                .fg(palette.status_red)
                .bg(palette.modal_bg)
                .add_modifier(Modifier::BOLD)
        }
    } else if chip.active {
        Style::default()
            .fg(palette.accent_fg)
            .bg(palette.accent)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
            .fg(palette.fg)
            .bg(palette.selection_bg)
    }
}

fn fill_rect(buf: &mut Buffer, rect: Rect, style: Style) {
    let clipped = rect.intersection(buf.area);
    for y in clipped.top()..clipped.bottom() {
        for x in clipped.left()..clipped.right() {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_symbol(" ");
                cell.set_style(style);
            }
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
    use crate::widgets::tab_bar::TabItem;

    fn sample_tabs() -> TabBar {
        TabBar::new(vec![
            TabItem {
                label: "Appearance".to_owned(),
                short_label: "Look".to_owned(),
                enabled: true,
                badge: None,
            },
            TabItem {
                label: "Permissions".to_owned(),
                short_label: "RBAC".to_owned(),
                enabled: true,
                badge: None,
            },
        ])
    }

    fn row_text(buf: &Buffer, area: Rect, y: u16) -> String {
        (area.x..area.x + area.width)
            .map(|x| buf.cell((x, y)).map_or(" ", |c| c.symbol()))
            .collect()
    }

    #[test]
    fn geometry_at_40x12_drops_rail_and_pins_footer() {
        let screen = Rect::new(0, 0, 40, 12);
        let tabs = sample_tabs();
        let cfg = HubFrameConfig {
            overlay_id: OverlayId::Settings,
            title: "Settings".to_owned(),
            max_width: 96,
            max_height: 36,
            width_pct: 90,
            height_pct: 85,
            tab_bar: Some(&tabs),
            rail_width: 24,
            inspector_height: 3,
        };

        let areas = compute_hub_areas(screen, &cfg);
        assert_eq!(areas.outer, screen);
        assert_eq!(areas.rail, None);
        assert_eq!(areas.footer.y, 10);
        assert_eq!(areas.footer.height, 1);
        assert!(areas.main.width >= 38);
        assert!(areas.main.height >= 4);
    }

    #[test]
    fn geometry_at_80x24_includes_rail_and_inspector() {
        let screen = Rect::new(0, 0, 80, 24);
        let tabs = sample_tabs();
        let cfg = HubFrameConfig {
            overlay_id: OverlayId::Settings,
            title: "RBAC Matrix".to_owned(),
            max_width: 96,
            max_height: 36,
            width_pct: 90,
            height_pct: 85,
            tab_bar: Some(&tabs),
            rail_width: 22,
            inspector_height: 3,
        };

        let areas = compute_hub_areas(screen, &cfg);
        assert!(areas.outer.width >= 60);
        assert!(areas.rail.is_some());
        assert!(areas.inspector.is_some());
        assert_eq!(areas.footer.y, areas.inner.bottom() - 1);
    }

    #[test]
    fn render_clears_bleed_and_drops_excess_hints_without_overflow() {
        let screen = Rect::new(0, 0, 40, 12);
        let mut buf = Buffer::empty(screen);
        // Populate background with noise that must not bleed into the modal interior.
        for y in 0..screen.height {
            buf.set_line(
                0,
                y,
                &Line::from("BACKGROUND_CHAT_BLEED_SHOULD_CLEAR_12345"),
                screen.width,
            );
        }

        let palette = ThemePalette::for_preset_with_no_color(ThemePreset::Titanium, false);
        let mut hitmap = HitMap::default();
        let cfg = HubFrameConfig {
            overlay_id: OverlayId::Settings,
            title: "Settings Hub".to_owned(),
            max_width: 90,
            max_height: 30,
            width_pct: 90,
            height_pct: 85,
            tab_bar: None,
            rail_width: 22,
            inspector_height: 0,
        };
        let hints = [
            ("Enter", "Edit"),
            ("Esc", "Close"),
            ("Tab", "Switch"),
            ("Ctrl+R", "Reload"),
        ];

        let areas = render_hub_frame(
            screen,
            &mut buf,
            &cfg,
            FooterMode::Hints(&hints),
            &palette,
            &mut hitmap,
        );

        let main_row = row_text(&buf, areas.main, areas.main.y);
        assert!(!main_row.contains("BLEED"));
        let footer_row = row_text(&buf, areas.footer, areas.footer.y);
        assert!(footer_row.contains("Enter Edit"));
        assert!(footer_row.contains("Esc Close"));
        assert!(!footer_row.contains("Reload"));
        assert_eq!(
            hitmap.hit_test(areas.main.x, areas.main.y),
            Some(&HitTarget::OverlayBody(OverlayId::Settings))
        );
    }

    #[test]
    fn render_footer_chips_and_inline_input_record_hits_and_cursor() {
        let screen = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(screen);
        let palette = ThemePalette::for_preset_with_no_color(ThemePreset::Titanium, false);
        let mut hitmap = HitMap::default();
        let cfg = HubFrameConfig {
            overlay_id: OverlayId::Settings,
            title: "Actions".to_owned(),
            max_width: 72,
            max_height: 20,
            width_pct: 90,
            height_pct: 85,
            tab_bar: None,
            rail_width: 0,
            inspector_height: 0,
        };

        let chips = [
            FooterChip {
                label: "Save".to_owned(),
                active: true,
                danger: false,
            },
            FooterChip {
                label: "Delete".to_owned(),
                active: false,
                danger: true,
            },
        ];
        let areas = render_hub_frame(
            screen,
            &mut buf,
            &cfg,
            FooterMode::Chips(&chips),
            &palette,
            &mut hitmap,
        );
        assert_eq!(
            hitmap.hit_test(areas.footer.x + 1, areas.footer.y),
            Some(&HitTarget::FooterChip(0))
        );
        assert_eq!(
            hitmap.hit_test(areas.footer.x + 8, areas.footer.y),
            Some(&HitTarget::FooterChip(1))
        );

        hitmap.clear();
        let mut input = TextInput::new(false);
        input.set_value("hello");
        let areas = render_hub_frame(
            screen,
            &mut buf,
            &cfg,
            FooterMode::InlineInput {
                prompt: "? Filter: ",
                input: &input,
                hint: "Esc cancel",
            },
            &palette,
            &mut hitmap,
        );
        let footer_row = row_text(&buf, areas.footer, areas.footer.y);
        assert!(footer_row.contains("? Filter: hello"));
        assert!(footer_row.contains("Esc cancel"));
    }
}
