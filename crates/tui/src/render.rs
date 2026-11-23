//! Borderless Ratatui renderer with mouse [`HitRegions`] recording, OMP-style spinner/status bar,
//! 2-column `/` autocomplete, inline `▎` Approval cards, 8-tab `/settings`, and Setup Wizard.

use std::time::Duration;

use monoagens_gateway::PluginKind;
use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Widget};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::config::{IconMode, MessageLayout};
use crate::core::hitmap::{HitTarget, ScrollId};
use crate::core::overlay::OverlayId;
use crate::core::theme::{ChromeGlyphs, render_dimmed_backdrop};
use crate::effects::TuiEffects;
use crate::markdown::render_markdown_lines;
use crate::settings::PluginUiState;
use crate::state::{
    ChatTimelineItem, HitRegions, PendingApprovalCard, TuiAppState, normalize_selection,
};
use crate::theme::ThemePalette;
use crate::width::{chrome_label, fit_prefix, sanitize};
use crate::widgets::scrollbar::render_vertical_scrollbar as widget_vertical_scrollbar;
use unicode_segmentation::UnicodeSegmentation;

/// Below this width the sidebar is hidden and cannot open.
pub const MIN_SIDEBAR_COLS: u16 = 80;
/// Below this height the hint line is hidden and overlays use the full height.
pub const COMPACT_ROWS: u16 = 20;
/// Smallest terminal that is drawn; anything smaller shows a size message.
pub const MIN_COLS: u16 = 40;
pub const MIN_ROWS: u16 = 10;

/// Renders the full MonoAgens TUI into `frame`, updating `hit` rectangles and processing `effects`.
pub fn render_app(
    frame: &mut Frame<'_>,
    state: &TuiAppState,
    effects: &mut TuiEffects,
    hit: &mut HitRegions,
) {
    render_to_buffer(
        frame.area(),
        frame.buffer_mut(),
        state,
        effects,
        hit,
        16,
    );
}

/// Pure buffer renderer (usable directly in headless `TestBackend` tests as well as live frames).
pub fn render_to_buffer(
    area: Rect,
    buf: &mut Buffer,
    state: &TuiAppState,
    effects: &mut TuiEffects,
    hit: &mut HitRegions,
    elapsed_ms: u32,
) {
    hit.clear();
    let palette = ThemePalette::for_preset(state.prefs.theme);
    fill_bg(buf, area, palette.bg, palette.fg);

    if area.width < MIN_COLS || area.height < MIN_ROWS {
        render_too_small(area, buf, &palette);
        return;
    }

    let show_sidebar = state.sidebar_visible(area.width);
    let (sidebar_rect, main_rect) = if show_sidebar {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(30), Constraint::Min(20)])
            .split(area);
        (Some(chunks[0]), chunks[1])
    } else {
        (None, area)
    };

    if let Some(sb_rect) = sidebar_rect {
        hit.sidebar_area = sb_rect;
        render_sidebar(sb_rect, buf, state, &palette, hit);
    }

    let feed_rect = render_main_pane(main_rect, buf, state, effects, &palette, hit);

    let stack = state.effective_overlay_stack();
    let mut modal_rect = None;
    for entry in stack.iter_bottom_to_top() {
        match entry.id {
            OverlayId::ConvModal => {
                if state.conv.why.is_some() {
                    modal_rect = Some(area);
                }
                crate::conv::render_overlays_with_hit(area, feed_rect, buf, state, &palette, hit);
            }
            OverlayId::Wizard => {
                if let Some(wizard) = &state.wizard {
                    modal_rect = Some(area);
                    crate::wizard::render_wizard(area, buf, wizard, &palette, hit);
                }
            }
            OverlayId::Settings => {
                modal_rect = Some(area);
                state.render_settings_overlay(area, buf, &palette, hit);
            }
            OverlayId::Catalog => {
                modal_rect = Some(area);
                crate::catalog_ui::render_overlay_with_hit(
                    area,
                    buf,
                    &state.catalog,
                    &palette,
                    state.prefs.icons,
                    hit,
                );
            }
            OverlayId::Governance => {
                modal_rect = Some(area);
                render_dimmed_backdrop(buf, area, &palette);
                hit.hitmap.record(area, HitTarget::Backdrop(OverlayId::Governance));
                Clear.render(area, buf);
                crate::views::draw(area, buf, &state.gov, &palette, state.prefs.icons);
                record_gov_hitmap(area, buf, &state.gov, &palette, hit);
            }
            OverlayId::Log => {
                modal_rect = Some(area);
                crate::log::render_log_overlay(
                    area,
                    buf,
                    &state.log,
                    &palette,
                    state.prefs.icons,
                    hit,
                );
            }
            OverlayId::Palette => {
                if let Some(p) = &state.ux.palette {
                    modal_rect = Some(area);
                    crate::palette::render_palette(area, buf, p, &palette, hit);
                }
            }
            OverlayId::Help => {
                if let Some(h) = &state.ux.help {
                    modal_rect = Some(area);
                    crate::ui_global::render_help_overlay(area, buf, h, state, &palette, hit);
                }
            }
        }
    }

    if modal_rect.is_none() && !stack.contains(OverlayId::ConvModal) {
        crate::conv::render_overlays_with_hit(area, feed_rect, buf, state, &palette, hit);
    }

    if let Some(toast) = &state.toast {
        render_toast(area, buf, &toast.text, toast.is_error, &palette, state.prefs.icons);
    }

    let fx_area = modal_rect.unwrap_or(main_rect);
    effects.process(elapsed_ms, buf, fx_area);
}

fn record_gov_hitmap(
    area: Rect,
    buf: &mut Buffer,
    gov: &crate::views::GovState,
    palette: &ThemePalette,
    hit: &mut HitRegions,
) {
    if area.width < 10 || area.height < crate::views::GOV_CHROME_ROWS + 1 {
        return;
    }
    hit.hitmap.record(area, HitTarget::OverlayBody(OverlayId::Governance));
    let visible = (area.height - crate::views::GOV_CHROME_ROWS) as usize;
    if let Some(d) = &gov.detail {
        let d_vis = visible.saturating_sub(1);
        if d.lines.len() > d_vis && d_vis > 0 {
            let track = Rect::new(area.x + area.width - 1, area.y + 2, 1, d_vis as u16);
            widget_vertical_scrollbar(
                buf,
                track,
                ScrollId::GovernanceList,
                d.lines.len(),
                d_vis,
                d.offset,
                &mut hit.hitmap,
                Style::default().fg(palette.separator).bg(palette.bg),
                Style::default().fg(palette.accent).bg(palette.bg),
            );
        }
        return;
    }
    let vm = gov.view_model(gov.now());
    let total = vm.rows.len();
    let offset = scroll_window(vm.selected.unwrap_or(0), vm.offset, visible, total);
    let list_w = area.width.saturating_sub(1);
    for (i, _) in vm.rows.iter().skip(offset).take(visible).enumerate() {
        hit.hitmap.record(
            Rect::new(area.x, area.y + 1 + i as u16, list_w, 1),
            HitTarget::ListRow(offset + i),
        );
    }
    if total > visible && visible > 0 {
        let track = Rect::new(area.x + area.width - 1, area.y + 1, 1, visible as u16);
        widget_vertical_scrollbar(
            buf,
            track,
            ScrollId::GovernanceList,
            total,
            visible,
            offset,
            &mut hit.hitmap,
            Style::default().fg(palette.separator).bg(palette.bg),
            Style::default().fg(palette.accent).bg(palette.bg),
        );
    }
}

fn render_too_small(area: Rect, buf: &mut Buffer, palette: &ThemePalette) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let msg = format!("terminal too small (needs {MIN_COLS}x{MIN_ROWS})");
    let lines = wrap_text_lines(&msg, area.width as usize);
    let top = area.y + area.height.saturating_sub(lines.len() as u16) / 2;
    for (i, line) in lines.iter().enumerate() {
        let y = top + i as u16;
        if y >= area.y + area.height {
            break;
        }
        let w = line.width() as u16;
        let x = area.x + area.width.saturating_sub(w) / 2;
        buf.set_string(x, y, line, Style::default().fg(palette.status_red).bg(palette.bg));
    }
}

fn fill_bg(buf: &mut Buffer, area: Rect, bg: ratatui::style::Color, fg: ratatui::style::Color) {
    let style = Style::default().bg(bg).fg(fg);
    for y in area.y..area.y.saturating_add(area.height) {
        for x in area.x..area.x.saturating_add(area.width) {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_style(style);
            }
        }
    }
}

enum SidebarRow {
    Header(String),
    Spacer,
    Chat { id: String, title: String, active: bool },
    Plugin { id: String, kind: PluginKind, state: PluginUiState },
    Actor { id: String, active: bool },
}

fn render_sidebar(
    area: Rect,
    buf: &mut Buffer,
    state: &TuiAppState,
    palette: &ThemePalette,
    hit: &mut HitRegions,
) {
    if area.width < 4 || area.height < 4 {
        return;
    }
    fill_bg(buf, area, palette.sidebar_bg, palette.fg);

    // Thin right vertical separator `│`
    let sep_x = area.x + area.width - 1;
    let sep_style = Style::default().fg(palette.separator).bg(palette.sidebar_bg);
    for y in area.y..area.y.saturating_add(area.height) {
        if let Some(cell) = buf.cell_mut((sep_x, y)) {
            cell.set_symbol("│");
            cell.set_style(sep_style);
        }
    }

    let inner_w = area.width.saturating_sub(2) as usize;
    let max_y = area.y.saturating_add(area.height);

    let put_line = |buf: &mut Buffer, row_y: u16, spans: Vec<Span<'static>>| {
        if row_y >= max_y {
            return;
        }
        let line = Line::from(spans);
        buf.set_line(area.x + 1, row_y, &line, inner_w as u16);
    };

    // Brand header (strictly 1-cell glyphs)
    let glyphs = ChromeGlyphs::for_mode(state.prefs.icons);
    put_line(
        buf,
        area.y,
        vec![
            Span::styled(
                format!("{} Mono", glyphs.tab_plugins),
                Style::default()
                    .fg(palette.accent)
                    .bg(palette.sidebar_bg)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("  {}", glyphs.bullet),
                Style::default().fg(palette.muted).bg(palette.sidebar_bg),
            ),
        ],
    );
    if area.y + 1 < max_y {
        put_line(
            buf,
            area.y + 1,
            vec![Span::styled(
                "─".repeat(inner_w),
                Style::default().fg(palette.separator).bg(palette.sidebar_bg),
            )],
        );
    }

    let body_y = area.y + 2;
    let avail_h = max_y.saturating_sub(body_y) as usize;
    if avail_h == 0 {
        return;
    }

    let mut rows: Vec<SidebarRow> = Vec::new();
    rows.push(SidebarRow::Header(format!("{} CHATS", glyphs.arrow_right)));
    for chat in crate::chat_search::visible_chats(
        &state.chats,
        &state.timelines,
        state.conv.chat_filter.as_deref(),
    ) {
        rows.push(SidebarRow::Chat {
            id: chat.id.clone(),
            title: if chat.pinned { format!("* {}", chat.title) } else { chat.title.clone() },
            active: chat.id == state.chats.active_chat_id,
        });
    }
    rows.push(SidebarRow::Spacer);
    rows.push(SidebarRow::Header(format!("{} PLUGINS", glyphs.tab_model)));

    let visible_plugins: Vec<_> = {
        let installed: Vec<_> = state.plugins.iter().filter(|p| p.installed).collect();
        if installed.is_empty() {
            state.plugins.iter().take(4).collect()
        } else {
            installed
        }
    };
    for plugin in visible_plugins {
        rows.push(SidebarRow::Plugin {
            id: plugin.id.clone(),
            kind: plugin.kind,
            state: plugin.state.clone(),
        });
    }

    if state.lab_mode {
        rows.push(SidebarRow::Spacer);
        rows.push(SidebarRow::Header(format!("{} SPEAK AS", glyphs.tab_soul)));
        for actor in &state.lab_session.known_actors {
            rows.push(SidebarRow::Actor {
                id: actor.clone(),
                active: state.lab_session.current_actor.as_str() == actor,
            });
        }
    }

    let total_rows = rows.len();
    let max_scroll = total_rows.saturating_sub(avail_h);
    hit.sidebar_max_scroll = max_scroll;
    let scroll = state.sidebar_scroll.min(max_scroll);

    for (vis_idx, row) in rows.iter().skip(scroll).take(avail_h).enumerate() {
        let y = body_y + vis_idx as u16;
        match row {
            SidebarRow::Header(label) => {
                put_line(
                    buf,
                    y,
                    vec![Span::styled(
                        label.clone(),
                        Style::default()
                            .fg(palette.dim)
                            .bg(palette.sidebar_bg)
                            .add_modifier(Modifier::BOLD),
                    )],
                );
            }
            SidebarRow::Spacer => {}
            SidebarRow::Chat { id, title, active } => {
                let row_rect = Rect::new(area.x, y, area.width.saturating_sub(1), 1);
                hit.sidebar_chats.push((row_rect, id.clone()));
                hit.hitmap.record(row_rect, HitTarget::SidebarChat(id.clone()));
                let active_prefix = format!("{} ", glyphs.pointer);
                let prefix = if *active { active_prefix.as_str() } else { "  " };
                let trunc_title = truncate_to_width(title, inner_w.saturating_sub(2));
                let row_bg = if *active {
                    palette.selection_bg
                } else {
                    palette.sidebar_bg
                };
                let row_fg = if *active {
                    palette.accent
                } else {
                    palette.fg
                };
                fill_bg(buf, row_rect, row_bg, row_fg);
                put_line(
                    buf,
                    y,
                    vec![
                        Span::styled(prefix.to_owned(), Style::default().fg(palette.accent).bg(row_bg)),
                        Span::styled(trunc_title, Style::default().fg(row_fg).bg(row_bg)),
                    ],
                );
            }
            SidebarRow::Plugin { id, kind, state: pstate } => {
                let row_rect = Rect::new(area.x, y, area.width.saturating_sub(1), 1);
                hit.sidebar_plugins.push((row_rect, id.clone()));
                hit.hitmap.record(row_rect, HitTarget::SidebarPlugin(id.clone()));
                let dot_color = match pstate {
                    PluginUiState::Running | PluginUiState::RunningElsewhere => {
                        palette.status_green
                    }
                    PluginUiState::Starting | PluginUiState::Stopping | PluginUiState::NeedsSetup(_) => {
                        palette.status_yellow
                    }
                    PluginUiState::Stopped | PluginUiState::Failed(_) => palette.status_red,
                };
                let dot_glyph = pstate.dot_glyph();
                let kind_label = match kind {
                    PluginKind::Adapter => "(Adapter)",
                    PluginKind::Capability => "(Capabilities)",
                };
                let name_budget = inner_w.saturating_sub(kind_label.len() + 3);
                let name = truncate_to_width(id, name_budget);
                put_line(
                    buf,
                    y,
                    vec![
                        Span::styled(
                            format!("{dot_glyph} "),
                            Style::default().fg(dot_color).bg(palette.sidebar_bg),
                        ),
                        Span::styled(
                            format!("{name} "),
                            Style::default().fg(palette.fg).bg(palette.sidebar_bg),
                        ),
                        Span::styled(
                            kind_label,
                            Style::default().fg(palette.dim).bg(palette.sidebar_bg),
                        ),
                    ],
                );
            }
            SidebarRow::Actor { id, active } => {
                let row_rect = Rect::new(area.x, y, area.width.saturating_sub(1), 1);
                hit.sidebar_actors.push((row_rect, id.clone()));
                hit.hitmap.record(row_rect, HitTarget::SidebarActor(id.clone()));
                let prefix = if *active {
                    format!("{} ", glyphs.dot_on)
                } else {
                    format!("{} ", glyphs.dot_off)
                };
                put_line(
                    buf,
                    y,
                    vec![
                        Span::styled(prefix, Style::default().fg(palette.accent).bg(palette.sidebar_bg)),
                        Span::styled(
                            truncate_to_width(id, inner_w.saturating_sub(2)),
                            Style::default().fg(palette.fg).bg(palette.sidebar_bg),
                        ),
                    ],
                );
            }
        }
    }

    if total_rows > avail_h {
        let sb_rect = Rect::new(sep_x, body_y, 1, avail_h as u16);
        widget_vertical_scrollbar(
            buf,
            sb_rect,
            ScrollId::Sidebar,
            total_rows,
            avail_h,
            scroll,
            &mut hit.hitmap,
            Style::default().fg(palette.separator).bg(palette.sidebar_bg),
            Style::default().fg(palette.accent).bg(palette.sidebar_bg),
        );
    }
}

fn render_main_pane(
    area: Rect,
    buf: &mut Buffer,
    state: &TuiAppState,
    effects: &TuiEffects,
    palette: &ThemePalette,
    hit: &mut HitRegions,
) -> Rect {
    let compact_rows = area.height < COMPACT_ROWS;
    let approval_height = if state.pending_approval.is_some() { 4 } else { 0 };
    let busy_height = if state.is_busy() { 1 } else { 0 };
    let attachment_height = if state.composer.pending_attachments.is_empty() {
        0
    } else {
        4
    };
    let max_composer_lines = if compact_rows { 3 } else { 5 };
    let composer_lines = state
        .composer
        .text
        .split('\n')
        .count()
        .clamp(1, max_composer_lines) as u16;
    // top rule + lines + bottom rule (+ footer hint unless the terminal is short)
    let composer_height = composer_lines + if compact_rows { 2 } else { 3 };
    let fixed_rows = approval_height + busy_height + 1 + attachment_height + composer_height;
    let autocomplete_height = if state.slash_menu_open() {
        (state.autocomplete_visible_rows() as u16).min(area.height.saturating_sub(fixed_rows + 1))
    } else {
        0
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),
            Constraint::Length(approval_height),
            Constraint::Length(busy_height),
            Constraint::Length(1), // status line
            Constraint::Length(attachment_height),
            Constraint::Length(autocomplete_height),
            Constraint::Length(composer_height),
        ])
        .split(area);

    let mut feed_rect = chunks[0];
    if let Some(banner) = banner_lines(state, feed_rect.width as usize) {
        let bh = (banner.len() as u16).min(feed_rect.height.saturating_sub(1));
        render_banner(Rect::new(feed_rect.x, feed_rect.y, feed_rect.width, bh), buf, &banner, palette);
        feed_rect = Rect::new(
            feed_rect.x,
            feed_rect.y + bh,
            feed_rect.width,
            feed_rect.height - bh,
        );
    }
    let approval_rect = chunks[1];
    let busy_rect = chunks[2];
    let status_rect = chunks[3];
    let attachment_rect = chunks[4];
    let ac_rect = chunks[5];
    let composer_rect = chunks[6];

    hit.chat_area = feed_rect;
    hit.hitmap.record(feed_rect, HitTarget::ChatFeed);
    render_chat_feed(feed_rect, buf, state, palette, hit);

    if let Some(card) = &state.pending_approval
        && approval_rect.height >= 3
    {
        render_approval_card(approval_rect, buf, card, palette, hit);
    }

    if state.is_busy() && busy_rect.height >= 1 {
        render_busy_phase_line(busy_rect, buf, state, effects, palette);
    }

    render_status_bar(status_rect, buf, state, effects, palette);
    if attachment_height > 0 {
        render_attachment_strip(attachment_rect, buf, state, palette, hit);
    }
    render_composer(composer_rect, buf, state, palette, !compact_rows, hit);

    if autocomplete_height > 0 {
        render_autocomplete(ac_rect, buf, state, palette, hit);
    }
    feed_rect
}

/// Lines of the top banner (config error, or why chat is unavailable), if any.
fn banner_lines(state: &TuiAppState, width: usize) -> Option<Vec<String>> {
    let inner = width.saturating_sub(2).max(1);
    if let Some(err) = &state.config_error {
        let mut lines: Vec<String> = wrap_text_lines(&format!("Configuration error: {}", err.message), inner)
            .into_iter()
            .take(2)
            .collect();
        let fix = match &err.key {
            Some(key) => format!("Fix '{key}' in /settings, then /retry (Ctrl+R) to boot the gateway."),
            None => "Fix the configuration in /settings, then /retry (Ctrl+R) to boot the gateway."
                .to_owned(),
        };
        lines.push(fix);
        return Some(lines);
    }
    state
        .chat_disabled_reason
        .as_ref()
        .map(|r| vec![format!("Chat is disabled: {r}")])
}

fn render_banner(rect: Rect, buf: &mut Buffer, lines: &[String], palette: &ThemePalette) {
    if rect.height == 0 {
        return;
    }
    let style = Style::default().fg(palette.status_red).bg(palette.status_bg);
    for (i, line) in lines.iter().enumerate().take(rect.height as usize) {
        let y = rect.y + i as u16;
        for x in rect.x..rect.x + rect.width {
            if let Some(c) = buf.cell_mut((x, y)) {
                c.reset();
                c.set_style(style);
            }
        }
        buf.set_string(rect.x + 1, y, truncate_to_width(line, rect.width.saturating_sub(2) as usize), style);
    }
}

/// Dim bubble for a message typed while a turn runs; it moves into the timeline when submitted.
fn push_queued_bubble(
    lines: &mut Vec<Line<'static>>,
    text: &str,
    content_w: usize,
    state: &TuiAppState,
    palette: &ThemePalette,
) {
    let dim = Style::default().fg(palette.dim).bg(palette.bubble_bg);
    let tag = " queued · alt+↑ to edit";
    if state.prefs.message_layout == MessageLayout::Chat {
        let max_bubble_w = ((content_w * 72) / 100).max(14).min(content_w);
        let inner_max = max_bubble_w.saturating_sub(4).max(4);
        let wrapped = wrap_text_lines(text, inner_max);
        let text_w = wrapped
            .iter()
            .map(|l| l.width())
            .max()
            .unwrap_or(1)
            .min(inner_max);
        let left_pad = content_w.saturating_sub(text_w + 4);
        for wline in wrapped {
            let trail = text_w.saturating_sub(wline.width());
            lines.push(Line::from(vec![
                Span::raw(" ".repeat(left_pad)),
                Span::styled(format!("  {wline}{}  ", " ".repeat(trail)), dim),
            ]));
        }
    } else {
        for wline in wrap_text_lines(text, content_w.saturating_sub(2)) {
            lines.push(Line::from(vec![
                Span::styled("▎ ", dim),
                Span::styled(wline, dim),
            ]));
        }
    }
    let tag = truncate_to_width(tag, content_w);
    let pad = content_w.saturating_sub(tag.width());
    lines.push(Line::from(vec![
        Span::raw(" ".repeat(pad)),
        Span::styled(
            tag,
            Style::default()
                .fg(palette.dim)
                .add_modifier(Modifier::ITALIC),
        ),
    ]));
    lines.push(Line::default());
}

fn render_chat_feed(
    area: Rect,
    buf: &mut Buffer,
    state: &TuiAppState,
    palette: &ThemePalette,
    hit: &mut HitRegions,
) {
    if area.width < 5 || area.height == 0 {
        return;
    }
    let items = state.active_timeline();
    let pad_x = 1u16;
    let sb_x = area.x + area.width - 1;
    let content_w = area.width.saturating_sub(pad_x * 2 + 1) as usize;
    if content_w < 4 {
        return;
    }

    let mut rendered_lines: Vec<Line<'static>> = Vec::new();
    // (first line, end line, timeline index) of each selectable message, for mouse clicks.
    let mut message_spans: Vec<(usize, usize, usize)> = Vec::new();
    for (item_idx, item) in items.iter().enumerate() {
        let item_start = rendered_lines.len();
        match item {
            ChatTimelineItem::UserMessage {
                sender,
                text,
                is_group,
                ..
            } => {
                if state.prefs.message_layout == MessageLayout::Chat {
                    let max_bubble_w = ((content_w * 72) / 100).max(14).min(content_w);
                    let inner_max = max_bubble_w.saturating_sub(4).max(4);
                    let wrapped = wrap_text_lines(text, inner_max);
                    let text_w = wrapped
                        .iter()
                        .map(|l| l.width())
                        .max()
                        .unwrap_or(1)
                        .min(inner_max);
                    let bubble_w = text_w + 4;
                    let left_pad = content_w.saturating_sub(bubble_w);

                    if state.lab_mode && *is_group {
                        let header_pad = content_w.saturating_sub(sender.width());
                        rendered_lines.push(Line::from(vec![
                            Span::raw(" ".repeat(header_pad)),
                            Span::styled(sender.clone(), Style::default().fg(palette.dim)),
                        ]));
                    }
                    for wline in wrapped {
                        let trail = text_w.saturating_sub(wline.width());
                        rendered_lines.push(Line::from(vec![
                            Span::raw(" ".repeat(left_pad)),
                            Span::styled(
                                format!("  {wline}{}  ", " ".repeat(trail)),
                                Style::default().bg(palette.bubble_bg).fg(palette.bubble_fg),
                            ),
                        ]));
                    }
                    rendered_lines.push(Line::default());
                } else {
                    for wline in wrap_text_lines(text, content_w.saturating_sub(2)) {
                        rendered_lines.push(Line::from(vec![
                            Span::styled(
                                "▎ ",
                                Style::default().fg(palette.accent).bg(palette.bubble_bg),
                            ),
                            Span::styled(
                                wline,
                                Style::default().fg(palette.bubble_fg).bg(palette.bubble_bg),
                            ),
                        ]));
                    }
                    rendered_lines.push(Line::default());
                }
            }
            ChatTimelineItem::AssistantMessage {
                text, duration_ms, ..
            } => {
                rendered_lines.extend(render_markdown_lines(&sanitize(text, true), content_w, palette));
                if state.prefs.show_turn_duration
                    && let Some(ms) = duration_ms
                {
                    let glyphs = ChromeGlyphs::for_mode(state.prefs.icons);
                    let label = format!("  {} {}", glyphs.bullet, format_turn_duration(*ms));
                    rendered_lines.push(Line::from(Span::styled(
                        label,
                        Style::default().fg(palette.dim),
                    )));
                }
                rendered_lines.push(Line::default());
            }
            ChatTimelineItem::CapabilityStep {
                capability,
                status,
                detail,
            } => {
                if state.prefs.show_capability_steps {
                    rendered_lines.push(Line::from(Span::styled(
                        format!("┌─ {capability} ({status})"),
                        Style::default().fg(palette.dim),
                    )));
                    if !detail.is_empty() {
                        for dline in wrap_text_lines(detail, content_w.saturating_sub(3)) {
                            rendered_lines.push(Line::from(Span::styled(
                                format!("│  {dline}"),
                                Style::default().fg(palette.muted),
                            )));
                        }
                    }
                    rendered_lines.push(Line::from(Span::styled(
                        "└─",
                        Style::default().fg(palette.dim),
                    )));
                }
            }
            ChatTimelineItem::SystemNotice { text, is_error } => {
                let color = if *is_error {
                    palette.status_red
                } else {
                    palette.muted
                };
                for wline in wrap_text_lines(text, content_w) {
                    rendered_lines.push(Line::from(Span::styled(
                        wline,
                        Style::default().fg(color),
                    )));
                }
                rendered_lines.push(Line::default());
            }
            ChatTimelineItem::BrainTrace { summary } => {
                if state.lab_mode && state.prefs.show_brain_trace {
                    for wline in wrap_text_lines(summary, content_w) {
                        rendered_lines.push(Line::from(Span::styled(
                            wline,
                            Style::default().fg(palette.dim),
                        )));
                    }
                }
            }
        }
        if crate::message_actions::message_text(item).is_some() {
            message_spans.push((item_start, rendered_lines.len(), item_idx));
        }
        crate::conv::mark_selected(&mut rendered_lines[item_start..], state, item_idx, palette);
        rendered_lines.extend(crate::conv::summary_lines(state, item_idx, content_w, palette));
    }

    // Live draft preview of the running turn (dim, never part of the timeline).
    rendered_lines.extend(state.conv.preview.lines(content_w, palette));

    for queued in state.turn.queued() {
        push_queued_bubble(&mut rendered_lines, &queued.text, content_w, state, palette);
    }

    if rendered_lines.is_empty() {
        let glyphs = ChromeGlyphs::for_mode(state.prefs.icons);
        let top_pad = (area.height as usize).saturating_sub(6) / 3;
        for _ in 0..top_pad {
            rendered_lines.push(Line::default());
        }
        let header = format!(
            "  {} {} · ready",
            glyphs.tab_plugins, state.gateway_cfg.soul.name
        );
        rendered_lines.push(Line::from(Span::styled(
            truncate_to_width(&header, content_w),
            Style::default()
                .fg(palette.accent)
                .add_modifier(Modifier::BOLD),
        )));
        for wline in wrap_text_lines(
            "  Ready to chat — type a message, drop a file, or try /settings",
            content_w,
        ) {
            rendered_lines.push(Line::from(Span::styled(
                wline,
                Style::default().fg(palette.fg),
            )));
        }
        rendered_lines.push(Line::default());
        let tips = format!(
            "  {} Start chatting · {} /new · {} /attach · {} /settings · {} drag to copy · Ctrl+B sidebar",
            glyphs.arrow_right, glyphs.tab_plugins, glyphs.bullet, glyphs.tab_runtime, glyphs.info
        );
        for wline in wrap_text_lines(&tips, content_w) {
            rendered_lines.push(Line::from(Span::styled(
                wline,
                Style::default().fg(palette.dim),
            )));
        }
    }

    let visible_h = area.height as usize;
    let total = rendered_lines.len();
    let max_scroll = total.saturating_sub(visible_h);
    let scroll = if state.auto_scroll {
        max_scroll
    } else {
        state.chat_scroll.min(max_scroll)
    };

    for (start, end, idx) in message_spans {
        let (from, to) = (start.max(scroll), end.min(scroll + visible_h));
        if from < to {
            let msg_rect = Rect::new(
                area.x,
                area.y + (from - scroll) as u16,
                area.width.saturating_sub(1),
                (to - from) as u16,
            );
            hit.message_rows.push((msg_rect, idx));
            hit.hitmap.record(msg_rect, HitTarget::ChatMessage(idx));
        }
    }

    for (row_idx, line) in rendered_lines.iter().skip(scroll).take(visible_h).enumerate() {
        buf.set_line(
            area.x + pad_x,
            area.y + row_idx as u16,
            line,
            content_w as u16,
        );
    }

    // Record exact rendered viewport lines for mouse drag-select extraction (excluding scrollbar col).
    for row_idx in 0..area.height {
        let y = area.y + row_idx;
        let mut row_text = String::new();
        let mut skip = 0usize;
        for x in area.x..sb_x {
            if skip > 0 {
                skip -= 1;
                continue;
            }
            if let Some(cell) = buf.cell((x, y)) {
                let sym = cell.symbol();
                let w = sym.width();
                if w > 1 {
                    skip = w - 1;
                }
                row_text.push_str(sym);
            }
        }
        hit.chat_viewport_lines.push(row_text.trim_end().to_owned());
    }

    // Sleek vertical scrollbar on rightmost column of feed_rect
    let sb_rect = Rect::new(sb_x, area.y, 1, area.height);
    hit.chat_scrollbar = Some(sb_rect);
    hit.chat_total_lines = total;
    hit.chat_visible_lines = visible_h;
    if total > visible_h {
        widget_vertical_scrollbar(
            buf,
            sb_rect,
            ScrollId::Chat,
            total,
            visible_h,
            scroll,
            &mut hit.hitmap,
            Style::default().fg(palette.separator).bg(palette.bg),
            Style::default().fg(palette.accent).bg(palette.bg),
        );
    } else {
        render_vertical_scrollbar(
            buf,
            sb_rect,
            total,
            visible_h,
            scroll,
            palette.bg,
            palette,
            true,
        );
    }

    // Apply selection highlight overlay across normalized (start..=end) cell range.
    if let Some(sel) = state.selection
        && sel.anchor != sel.current
    {
        let (start, end) = normalize_selection(sel.anchor, sel.current);
        let max_x = sb_x.saturating_sub(1);
        let max_y = area.y.saturating_add(area.height.saturating_sub(1));
        let start_y = start.1.clamp(area.y, max_y);
        let end_y = end.1.clamp(area.y, max_y);
        for y in start_y..=end_y {
            let x_from = if y == start_y {
                start.0.clamp(area.x, max_x)
            } else {
                area.x
            };
            let x_to = if y == end_y {
                end.0.clamp(area.x, max_x)
            } else {
                max_x
            };
            for x in x_from..=x_to {
                if let Some(cell) = buf.cell_mut((x, y)) {
                    if palette.no_color {
                        cell.set_style(Style::default().add_modifier(Modifier::REVERSED));
                    } else {
                        cell.set_style(Style::default().bg(palette.selection_bg).fg(palette.fg));
                    }
                }
            }
        }
    }
}

fn render_approval_card(
    area: Rect,
    buf: &mut Buffer,
    card: &PendingApprovalCard,
    palette: &ThemePalette,
    hit: &mut HitRegions,
) {
    let bar_style = Style::default().fg(palette.approval_bar);
    let title_line = Line::from(vec![
        Span::styled("▎ ", bar_style),
        Span::styled(
            format!("Approval Required: {}", card.capability_id),
            Style::default()
                .fg(palette.approval_bar)
                .add_modifier(Modifier::BOLD),
        ),
    ]);
    buf.set_line(area.x + 1, area.y, &title_line, area.width.saturating_sub(2));

    if area.height >= 2 {
        let summary_line = Line::from(vec![
            Span::styled("▎ ", bar_style),
            Span::styled(
                truncate_to_width(&card.summary, area.width.saturating_sub(5) as usize),
                Style::default().fg(palette.fg),
            ),
        ]);
        buf.set_line(area.x + 1, area.y + 1, &summary_line, area.width.saturating_sub(2));
    }

    if area.height >= 3 {
        let btn_y = area.y + 2;
        let approve_style = if card.selected_approve {
            Style::default()
                .bg(palette.status_green)
                .fg(palette.accent_fg)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(palette.status_green)
        };
        let deny_style = if !card.selected_approve {
            Style::default()
                .bg(palette.status_red)
                .fg(palette.accent_fg)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(palette.status_red)
        };

        let approve_lbl = " [ Approve ] ";
        let deny_lbl = " [ Deny ] ";
        let line = Line::from(vec![
            Span::styled("▎ ", bar_style),
            Span::styled(approve_lbl, approve_style),
            Span::raw("  "),
            Span::styled(deny_lbl, deny_style),
            Span::styled("  (←/→ Enter, y/n, or click)", Style::default().fg(palette.dim)),
        ]);
        buf.set_line(area.x + 1, btn_y, &line, area.width.saturating_sub(2));

        let approve_x = area.x + 3;
        let deny_x = approve_x + approve_lbl.len() as u16 + 2;
        let approve_rect = Rect::new(approve_x, btn_y, approve_lbl.len() as u16, 1);
        let deny_rect = Rect::new(deny_x, btn_y, deny_lbl.len() as u16, 1);
        hit.approval_approve = Some(approve_rect);
        hit.approval_deny = Some(deny_rect);
        hit.hitmap
            .record(approve_rect, HitTarget::Button("approval.approve".to_owned()));
        hit.hitmap
            .record(deny_rect, HitTarget::Button("approval.deny".to_owned()));
    }
}

fn render_busy_phase_line(
    area: Rect,
    buf: &mut Buffer,
    state: &TuiAppState,
    effects: &TuiEffects,
    palette: &ThemePalette,
) {
    // The status bar owns the spinner and elapsed time; this line only names the phase.
    let mut spans = vec![Span::styled(
        "   ",
        Style::default().fg(palette.accent),
    )];
    let phase = chrome_label(&state.thinking_phase, state.prefs.icons);
    for (idx, g) in phase.graphemes(true).enumerate() {
        let style = effects.shimmer_style(idx, state.prefs.shimmer, state.prefs.motion, palette);
        spans.push(Span::styled(g.to_owned(), style));
    }
    let line = Line::from(spans);
    buf.set_line(area.x, area.y, &line, area.width);
}

fn render_status_bar(
    area: Rect,
    buf: &mut Buffer,
    state: &TuiAppState,
    effects: &TuiEffects,
    palette: &ThemePalette,
) {
    fill_bg(buf, area, palette.status_bg, palette.muted);
    let glyphs = ChromeGlyphs::for_mode(state.prefs.icons);
    let mut spans: Vec<Span<'static>> = Vec::new();
    if state.is_busy() {
        let spinner = effects.spinner_glyph(state.prefs.motion);
        let elapsed = TuiAppState::format_elapsed(Duration::from_millis(state.thinking_elapsed_ms));
        spans.push(Span::styled(
            format!(" {spinner} {elapsed} {} ", glyphs.pointer),
            Style::default()
                .fg(palette.accent)
                .bg(palette.status_bg)
                .add_modifier(Modifier::BOLD),
        ));
    } else {
        spans.push(Span::styled(
            format!(" {} ", glyphs.dot_on),
            Style::default().fg(palette.status_green).bg(palette.status_bg),
        ));
    }

    spans.push(Span::styled(
        state.model_name.clone(),
        Style::default().fg(palette.fg).bg(palette.status_bg),
    ));

    let compact = area.width < MIN_SIDEBAR_COLS;
    if state.prefs.show_valve_badge && !compact {
        spans.push(Span::styled(
            format!(" · {} Valve: {}", glyphs.tab_valve, state.valve_mode),
            Style::default().fg(palette.muted).bg(palette.status_bg),
        ));
    }

    if state.prefs.show_cost_estimate
        && !compact
        && let Some(cost) = state.estimated_cost_usd
    {
        let text = if cost > 0.0 && cost < 0.01 {
            format!(" · ${cost:.4}")
        } else {
            format!(" · ${cost:.2}")
        };
        spans.push(Span::styled(
            text,
            Style::default().fg(palette.muted).bg(palette.status_bg),
        ));
    }

    if state.prefs.show_token_budget && !compact {
        let pct = ((state.daily_tokens_used as f64 / state.daily_token_budget.max(1) as f64)
            * 100.0)
            .round()
            .min(100.0) as u64;
        spans.push(Span::styled(
            format!(" ── {pct}%"),
            Style::default().fg(palette.dim).bg(palette.status_bg),
        ));
    }

    if !compact {
        spans.push(Span::styled(
            format!(
                " · {} {}",
                glyphs.arrow_right,
                sanitize(&state.chats.active_chat().title, false)
            ),
            Style::default().fg(palette.dim).bg(palette.status_bg),
        ));
    }

    if state.gov.pending_count > 0 {
        spans.push(Span::styled(
            format!(" · ! {}", state.gov.pending_count),
            Style::default()
                .fg(palette.status_yellow)
                .bg(palette.status_bg)
                .add_modifier(Modifier::BOLD),
        ));
    }

    if state.lab_mode {
        spans.push(Span::raw(" "));
        spans.push(Span::styled(
            format!(" {} lab ", glyphs.tab_soul),
            Style::default()
                .bg(palette.lab_badge_bg)
                .fg(palette.lab_badge_fg)
                .add_modifier(Modifier::BOLD),
        ));
    }

    let line = Line::from(spans);
    buf.set_line(area.x, area.y, &line, area.width);
}

fn render_attachment_strip(
    area: Rect,
    buf: &mut Buffer,
    state: &TuiAppState,
    palette: &ThemePalette,
    hit: &mut HitRegions,
) {
    if area.height < 4 || area.width < 12 || state.composer.pending_attachments.is_empty() {
        return;
    }

    let mut cur_x = area.x + 1;
    let right_limit = area.x.saturating_add(area.width.saturating_sub(1));
    let border_style = Style::default().fg(palette.accent);
    let body_style = Style::default().fg(palette.fg);
    let cue_style = Style::default().fg(palette.dim);

    for att in &state.composer.pending_attachments {
        let top_lbl = format!(" {} #{} ", att.icon(), att.id);
        let footer_lbl = format!(" {} ", att.card_footer_label());
        let body_raw = att.card_body_label();

        let inner_w = top_lbl
            .width()
            .max(footer_lbl.width())
            .max(body_raw.width() + 4)
            .clamp(12, 24);
        let card_w = (inner_w + 2) as u16;
        if cur_x + card_w > right_limit {
            break;
        }

        let card_rect = Rect::new(cur_x, area.y, card_w, 4);
        hit.attachment_cards.push((card_rect, att.id));

        // Top border: ╭─ 📷 #1 ─╮
        let top_lbl_fit = truncate_to_width(&top_lbl, inner_w.saturating_sub(1));
        let top_dash_after = inner_w.saturating_sub(1 + top_lbl_fit.width());
        let top_line = Line::from(vec![
            Span::styled(format!("╭─{top_lbl_fit}"), border_style),
            Span::styled(format!("{}╮", "─".repeat(top_dash_after)), border_style),
        ]);
        buf.set_line(cur_x, area.y, &top_line, card_w);

        // Middle row 1: centered body label + subtle × click cue
        let max_body = inner_w.saturating_sub(3);
        let body_fit = truncate_to_width(&body_raw, max_body);
        let avail_for_pad = inner_w.saturating_sub(body_fit.width() + 2);
        let pad_left = avail_for_pad / 2;
        let pad_right = avail_for_pad.saturating_sub(pad_left);
        let mid1_line = Line::from(vec![
            Span::styled("│", border_style),
            Span::raw(" ".repeat(pad_left)),
            Span::styled(body_fit, body_style),
            Span::raw(" ".repeat(pad_right)),
            Span::styled("× ", cue_style),
            Span::styled("│", border_style),
        ]);
        buf.set_line(cur_x, area.y + 1, &mid1_line, card_w);

        // Middle row 2: blank interior row for clean card proportions
        let mid2_line = Line::from(vec![
            Span::styled("│", border_style),
            Span::raw(" ".repeat(inner_w)),
            Span::styled("│", border_style),
        ]);
        buf.set_line(cur_x, area.y + 2, &mid2_line, card_w);

        // Bottom border: ╰─ 640x640 ─╯
        let bot_lbl_fit = truncate_to_width(&footer_lbl, inner_w.saturating_sub(1));
        let bot_dash_after = inner_w.saturating_sub(1 + bot_lbl_fit.width());
        let bot_line = Line::from(vec![
            Span::styled(format!("╰─{bot_lbl_fit}"), border_style),
            Span::styled(format!("{}╯", "─".repeat(bot_dash_after)), border_style),
        ]);
        buf.set_line(cur_x, area.y + 3, &bot_line, card_w);

        cur_x += card_w + 1;
    }
}

fn render_composer(
    area: Rect,
    buf: &mut Buffer,
    state: &TuiAppState,
    palette: &ThemePalette,
    show_hint: bool,
    hit: &mut HitRegions,
) {
    hit.hitmap.record(area, HitTarget::Composer);
    if area.height < 3 || area.width < 6 {
        return;
    }
    let has_footer = show_hint && area.height >= 4;
    let bottom_rule_y = if has_footer {
        area.y + area.height - 2
    } else {
        area.y + area.height - 1
    };
    let text_rows = bottom_rule_y.saturating_sub(area.y + 1);

    let rule = "─".repeat(area.width as usize);
    let rule_style = Style::default().fg(palette.separator);
    buf.set_string(area.x, area.y, &rule, rule_style);

    let composer_active = !state.has_open_overlay();
    let cursor_style = if palette.no_color {
        Style::default().add_modifier(Modifier::REVERSED)
    } else {
        Style::default().bg(Color::White).fg(Color::Black)
    };

    let mut cursor_byte = state.composer.cursor.min(state.composer.text.len());
    while cursor_byte > 0 && !state.composer.text.is_char_boundary(cursor_byte) {
        cursor_byte -= 1;
    }
    let before_cursor = &state.composer.text[..cursor_byte];
    let cursor_row = before_cursor.matches('\n').count();
    let line_start_byte = before_cursor.rfind('\n').map_or(0, |i| i + 1);
    let cursor_col_in_line_bytes = cursor_byte - line_start_byte;
    let cursor_col_display = before_cursor[line_start_byte..].width();

    let lines: Vec<&str> = if state.composer.text.is_empty() {
        vec![""]
    } else {
        state.composer.text.split('\n').collect()
    };

    let v_scroll = if text_rows == 0 {
        0
    } else {
        cursor_row.saturating_sub(text_rows as usize - 1)
    };
    let text_avail_w = area.width.saturating_sub(4) as usize;
    let line_render_w = area.width.saturating_sub(2);

    for row in 0..text_rows {
        let abs_row = v_scroll + row as usize;
        let y = area.y + 1 + row;
        let prefix = if abs_row == 0 { "> " } else { "  " };
        let prefix_style = Style::default()
            .fg(palette.accent)
            .add_modifier(Modifier::BOLD);
        let content = lines.get(abs_row).copied().unwrap_or("");

        if state.composer.text.is_empty() && abs_row == 0 {
            let mut spans = vec![Span::styled(prefix, prefix_style)];
            if composer_active {
                spans.push(Span::styled(" ", cursor_style));
            }
            spans.push(Span::styled(
                "Message Mono or type / for commands…",
                Style::default().fg(palette.dim),
            ));
            buf.set_line(area.x + 1, y, &Line::from(spans), line_render_w);
        } else if composer_active && abs_row == cursor_row && text_avail_w > 0 {
            let local_cursor = cursor_col_in_line_bytes.min(content.len());
            let before = &content[..local_cursor];
            let (cursor_sym, after) = if let Some(ch) = content[local_cursor..].chars().next() {
                let next_idx = local_cursor + ch.len_utf8();
                (ch.to_string(), &content[next_idx..])
            } else {
                (" ".to_owned(), "")
            };
            let cursor_w = cursor_sym.width().max(1);
            let h_scroll = if cursor_col_display + cursor_w > text_avail_w {
                (cursor_col_display + cursor_w).saturating_sub(text_avail_w)
            } else {
                0
            };
            let before_vis = slice_display_window(before, h_scroll, text_avail_w);
            let used_w = before_vis.width();
            let rem_after_cursor = text_avail_w.saturating_sub(used_w + cursor_sym.width());
            let after_vis = truncate_to_width_no_ellipsis(after, rem_after_cursor);

            let line = Line::from(vec![
                Span::styled(prefix, prefix_style),
                Span::styled(before_vis, Style::default().fg(palette.fg)),
                Span::styled(cursor_sym, cursor_style),
                Span::styled(after_vis, Style::default().fg(palette.fg)),
            ]);
            buf.set_line(area.x + 1, y, &line, line_render_w);
        } else {
            let line = Line::from(vec![
                Span::styled(prefix, prefix_style),
                Span::styled(
                    truncate_to_width_no_ellipsis(content, text_avail_w),
                    Style::default().fg(palette.fg),
                ),
            ]);
            buf.set_line(area.x + 1, y, &line, line_render_w);
        }
    }

    buf.set_string(area.x, bottom_rule_y, &rule, rule_style);

    if has_footer {
        let hint = if state.is_busy()
            || state.slash_menu_open()
        {
            " esc to cancel · enter send · shift+enter newline · / commands · ctrl+b sidebar · ctrl+v paste"
        } else {
            " esc to clear · enter send · shift+enter newline · / commands · ctrl+b sidebar · ctrl+v paste"
        };
        buf.set_string(
            area.x,
            area.y + area.height - 1,
            truncate_to_width(hint, area.width as usize),
            Style::default().fg(palette.dim),
        );
    }
}

fn render_autocomplete(
    area: Rect,
    buf: &mut Buffer,
    state: &TuiAppState,
    palette: &ThemePalette,
    hit: &mut HitRegions,
) {
    fill_bg(buf, area, palette.sidebar_bg, palette.fg);
    hit.autocomplete_area = area;
    let total = state.autocomplete.len();
    let visible = (area.height as usize).min(total);
    let offset = scroll_window(state.autocomplete_idx, state.autocomplete_off, visible, total);
    hit.autocomplete_total = total;
    hit.autocomplete_visible = visible;
    hit.autocomplete_offset = offset;
    let bar_w = u16::from(total > visible);
    let text_w = area.width.saturating_sub(bar_w);
    let col_left_w = 24usize.min((text_w as usize) / 2);
    for (row, item) in state.autocomplete.iter().enumerate().skip(offset).take(visible) {
        let y = area.y + (row - offset) as u16;
        let row_rect = Rect::new(area.x, y, text_w, 1);
        hit.autocomplete_rows.push((row_rect, row));
        hit.hitmap.record(row_rect, HitTarget::ListRow(row));

        let selected = row == state.autocomplete_idx;
        let bg = if selected {
            palette.selection_bg
        } else {
            palette.sidebar_bg
        };
        let left_fg = if selected { palette.accent } else { palette.fg };
        fill_bg(buf, row_rect, bg, left_fg);

        let marker = if selected { "❯" } else { " " };
        let icon = chrome_label(item.icon, state.prefs.icons);
        let left_text = format!("{marker}{icon} /{}", item.name);
        let padded_left = format!("{left_text:<width$}", width = col_left_w);
        let right_text = match &item.current_value {
            Some(v) => format!("{}  [{v}]", item.description),
            None => item.description.clone(),
        };
        let line = Line::from(vec![
            Span::styled(padded_left, Style::default().fg(left_fg).bg(bg)),
            Span::styled(
                truncate_to_width(&right_text, (text_w as usize).saturating_sub(col_left_w + 2)),
                Style::default().fg(palette.muted).bg(bg),
            ),
        ]);
        buf.set_line(area.x, y, &line, text_w);
    }
    if bar_w == 1 {
        let bar = Rect::new(area.x + area.width - 1, area.y, 1, area.height);
        render_scrollbar(bar, buf, total, offset, visible, palette);
    }
}

fn render_toast(
    area: Rect,
    buf: &mut Buffer,
    text: &str,
    is_error: bool,
    palette: &ThemePalette,
    icons: IconMode,
) {
    let glyphs = ChromeGlyphs::for_mode(icons);
    let prefixed = if is_error {
        format!("! {}", chrome_label(text, icons))
    } else if text.starts_with("Copied") {
        format!("{} {}", glyphs.info, chrome_label(text, icons))
    } else {
        format!("{} {}", glyphs.check_on, chrome_label(text, icons))
    };
    let w = (prefixed.width() as u16 + 4).min(area.width.saturating_sub(4)).max(12);
    let x = area.x + area.width.saturating_sub(w + 2);
    let y = area.y + 1;
    let rect = Rect::new(x, y, w, 1);
    let fg = if is_error {
        palette.status_red
    } else {
        palette.accent
    };
    fill_bg(buf, rect, palette.status_bg, fg);
    buf.set_string(
        x + 1,
        y,
        truncate_to_width(&prefixed, w.saturating_sub(2) as usize),
        Style::default().fg(fg).bg(palette.status_bg),
    );
}

fn truncate_to_width(s: &str, max_w: usize) -> String {
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

fn wrap_text_lines(text: &str, max_w: usize) -> Vec<String> {
    if max_w == 0 {
        return vec![String::new()];
    }
    let text = sanitize(text, true);
    let mut lines = Vec::new();
    for raw_line in text.split('\n') {
        if raw_line.is_empty() {
            lines.push(String::new());
            continue;
        }
        let mut current = String::new();
        let mut current_w = 0usize;
        for word in raw_line.split_inclusive(' ') {
            let ww = word.width();
            if current_w > 0 && current_w + ww > max_w {
                lines.push(current.trim_end().to_owned());
                current.clear();
                current_w = 0;
            }
            if word.width() > max_w {
                for g in word.graphemes(true) {
                    let cw = crate::width::grapheme_width(g);
                    if current_w + cw > max_w && !current.is_empty() {
                        lines.push(current.clone());
                        current.clear();
                        current_w = 0;
                    }
                    current.push_str(g);
                    current_w += cw;
                }
            } else {
                current.push_str(word);
                current_w += ww;
            }
        }
        if !current.is_empty() || raw_line.is_empty() {
            lines.push(current.trim_end().to_owned());
        }
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

/// Shared list helper: returns the new first-visible index so that `selected` is on screen
/// (`visible` rows of `total`); the result never leaves `0..=total-visible`.
#[must_use]
pub fn scroll_window(selected: usize, offset: usize, visible: usize, total: usize) -> usize {
    if visible == 0 || total <= visible {
        return 0;
    }
    let max = total - visible;
    let selected = selected.min(total - 1);
    let mut offset = offset.min(max);
    if selected < offset {
        offset = selected;
    } else if selected >= offset + visible {
        offset = selected + 1 - visible;
    }
    offset.min(max)
}

/// Shared list helper: draws a vertical scrollbar in `area` (one column) for `total` rows with
/// `visible` on screen starting at `offset`; nothing is drawn when everything fits.
pub fn render_scrollbar(
    area: Rect,
    buf: &mut Buffer,
    total: usize,
    offset: usize,
    visible: usize,
    palette: &ThemePalette,
) {
    render_vertical_scrollbar(buf, area, total, visible, offset, palette.bg, palette, false);
}

#[allow(clippy::too_many_arguments)]
fn render_vertical_scrollbar(
    buf: &mut Buffer,
    rect: Rect,
    total: usize,
    visible: usize,
    scroll: usize,
    track_bg: Color,
    palette: &ThemePalette,
    always_show_track: bool,
) {
    if rect.width == 0 || rect.height == 0 {
        return;
    }
    let h = rect.height as usize;
    if total <= visible {
        if !always_show_track {
            return;
        }
        let track_style = Style::default().fg(palette.separator).bg(track_bg);
        for r in 0..h {
            if let Some(cell) = buf.cell_mut((rect.x, rect.y + r as u16)) {
                cell.set_symbol("│");
                cell.set_style(track_style);
            }
        }
        return;
    }

    let thumb_h = ((visible * h) / total.max(1)).max(1).min(h);
    let max_scroll = total.saturating_sub(visible);
    let clamped_scroll = scroll.min(max_scroll);
    let max_thumb_top = h.saturating_sub(thumb_h);
    let thumb_top = if max_scroll == 0 || max_thumb_top == 0 {
        0
    } else {
        (clamped_scroll * max_thumb_top + max_scroll / 2) / max_scroll
    };

    let track_style = Style::default().fg(palette.separator).bg(track_bg);
    let thumb_style = Style::default().fg(palette.accent).bg(track_bg);

    for r in 0..h {
        let is_thumb = r >= thumb_top && r < thumb_top + thumb_h;
        if let Some(cell) = buf.cell_mut((rect.x, rect.y + r as u16)) {
            cell.set_symbol(if is_thumb { "┃" } else { "│" });
            cell.set_style(if is_thumb { thumb_style } else { track_style });
        }
    }
}

fn format_turn_duration(ms: u64) -> String {
    if ms >= 60_000 {
        TuiAppState::format_elapsed(Duration::from_millis(ms))
    } else {
        format!("{:.1}s", ms as f64 / 1000.0)
    }
}

fn slice_display_window(s: &str, h_scroll: usize, max_w: usize) -> String {
    if max_w == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut col = 0usize;
    let mut out_w = 0usize;
    for ch in s.chars() {
        let cw = ch.width().unwrap_or(0);
        if col < h_scroll {
            col += cw;
            continue;
        }
        if out_w + cw > max_w {
            break;
        }
        out.push(ch);
        out_w += cw;
        col += cw;
    }
    out
}

fn truncate_to_width_no_ellipsis(s: &str, max_w: usize) -> String {
    fit_prefix(&sanitize(s, false), max_w)
}
