//! Borderless Ratatui renderer with mouse [`HitRegions`] recording, OMP-style spinner/status bar,
//! 2-column `/` autocomplete, inline `▎` Approval cards, 8-tab `/settings`, and Setup Wizard.

use std::time::Duration;

use monoagens_gateway::PluginKind;
use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph, Widget, Wrap};
use unicode_width::UnicodeWidthStr;

use crate::config::MessageLayout;
use crate::effects::TuiEffects;
use crate::settings::{
    PluginDetailState, PluginUiState, SettingValueKind, SettingsTab, SetupWizardState, WizardStep,
    build_settings_rows,
};
use crate::state::{
    ChatTimelineItem, HitRegions, PendingApprovalCard, TuiAppState, normalize_selection,
};
use crate::theme::ThemePalette;

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

    if area.width < 10 || area.height < 6 {
        return;
    }

    let show_sidebar = state.sidebar_visible(area.width);
    let (sidebar_rect, main_rect) = if show_sidebar {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(28), Constraint::Min(20)])
            .split(area);
        (Some(chunks[0]), chunks[1])
    } else {
        (None, area)
    };

    if let Some(sb_rect) = sidebar_rect {
        hit.sidebar_area = sb_rect;
        render_sidebar(sb_rect, buf, state, &palette, hit);
    }

    hit.chat_area = main_rect;
    render_main_pane(main_rect, buf, state, effects, &palette, hit);

    let mut modal_rect = None;
    if state.settings.open {
        let mr = centered_rect(88, 86, area);
        modal_rect = Some(mr);
        hit.settings_area = mr;
        render_settings_modal(mr, buf, state, &palette, hit);
    } else if let Some(wizard) = &state.wizard {
        let wr = centered_fixed_rect(72, 20, area);
        modal_rect = Some(wr);
        render_setup_wizard(wr, buf, wizard, &palette, hit);
    }

    if let Some(toast) = &state.toast {
        render_toast(area, buf, &toast.text, toast.is_error, &palette);
    }

    let fx_area = modal_rect.unwrap_or(main_rect);
    effects.process(elapsed_ms, buf, fx_area);
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

fn render_sidebar(
    area: Rect,
    buf: &mut Buffer,
    state: &TuiAppState,
    palette: &ThemePalette,
    hit: &mut HitRegions,
) {
    if area.width < 4 || area.height < 3 {
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
    let mut y = area.y;
    let max_y = area.y.saturating_add(area.height);

    let put_line = |buf: &mut Buffer, row_y: u16, spans: Vec<Span<'static>>| {
        if row_y >= max_y {
            return;
        }
        let line = Line::from(spans);
        buf.set_line(area.x + 1, row_y, &line, inner_w as u16);
    };

    // CHATS section
    put_line(
        buf,
        y,
        vec![Span::styled(
            "CHATS",
            Style::default()
                .fg(palette.dim)
                .bg(palette.sidebar_bg)
                .add_modifier(Modifier::BOLD),
        )],
    );
    y += 1;

    for chat in &state.chats.chats {
        if y >= max_y {
            break;
        }
        let is_active = chat.id == state.chats.active_chat_id;
        let row_rect = Rect::new(area.x, y, area.width.saturating_sub(1), 1);
        hit.sidebar_chats.push((row_rect, chat.id.clone()));

        let prefix = if is_active { "▸ " } else { "  " };
        let title = truncate_to_width(&chat.title, inner_w.saturating_sub(2));
        let row_bg = if is_active {
            palette.selection_bg
        } else {
            palette.sidebar_bg
        };
        let row_fg = if is_active {
            palette.accent
        } else {
            palette.fg
        };
        fill_bg(buf, row_rect, row_bg, row_fg);
        put_line(
            buf,
            y,
            vec![
                Span::styled(prefix, Style::default().fg(palette.accent).bg(row_bg)),
                Span::styled(title, Style::default().fg(row_fg).bg(row_bg)),
            ],
        );
        y += 1;
    }

    if y < max_y {
        y += 1;
    }

    // PLUGINS section (only installed or enabled plugins, or first few catalog entries if none)
    if y < max_y {
        put_line(
            buf,
            y,
            vec![Span::styled(
                "PLUGINS",
                Style::default()
                    .fg(palette.dim)
                    .bg(palette.sidebar_bg)
                    .add_modifier(Modifier::BOLD),
            )],
        );
        y += 1;
    }

    let visible_plugins: Vec<_> = {
        let installed: Vec<_> = state.plugins.iter().filter(|p| p.installed).collect();
        if installed.is_empty() {
            state.plugins.iter().take(4).collect()
        } else {
            installed
        }
    };

    for plugin in visible_plugins {
        if y >= max_y {
            break;
        }
        let row_rect = Rect::new(area.x, y, area.width.saturating_sub(1), 1);
        hit.sidebar_plugins.push((row_rect, plugin.id.clone()));

        let dot_color = match &plugin.state {
            PluginUiState::Running => palette.status_green,
            PluginUiState::Starting | PluginUiState::Stopping => palette.status_yellow,
            PluginUiState::Stopped | PluginUiState::Failed(_) => palette.status_red,
        };
        let kind_label = match plugin.kind {
            PluginKind::Adapter => "(Adapter)",
            PluginKind::Capability => "(Capabilities)",
        };
        let name_budget = inner_w.saturating_sub(kind_label.len() + 3);
        let name = truncate_to_width(&plugin.id, name_budget);
        put_line(
            buf,
            y,
            vec![
                Span::styled(
                    format!("{} ", plugin.state.dot_glyph()),
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
        y += 1;
    }

    // SPEAK AS section ONLY when lab_mode == true
    if state.lab_mode && y + 2 < max_y {
        y += 1;
        put_line(
            buf,
            y,
            vec![Span::styled(
                "SPEAK AS",
                Style::default()
                    .fg(palette.dim)
                    .bg(palette.sidebar_bg)
                    .add_modifier(Modifier::BOLD),
            )],
        );
        y += 1;
        for actor in &state.lab_session.known_actors {
            if y >= max_y {
                break;
            }
            let row_rect = Rect::new(area.x, y, area.width.saturating_sub(1), 1);
            hit.sidebar_actors.push((row_rect, actor.clone()));
            let is_cur = state.lab_session.current_actor.as_str() == actor;
            let prefix = if is_cur { "● " } else { "○ " };
            put_line(
                buf,
                y,
                vec![
                    Span::styled(prefix, Style::default().fg(palette.accent).bg(palette.sidebar_bg)),
                    Span::styled(
                        truncate_to_width(actor, inner_w.saturating_sub(2)),
                        Style::default().fg(palette.fg).bg(palette.sidebar_bg),
                    ),
                ],
            );
            y += 1;
        }
    }
}

fn render_main_pane(
    area: Rect,
    buf: &mut Buffer,
    state: &TuiAppState,
    effects: &TuiEffects,
    palette: &ThemePalette,
    hit: &mut HitRegions,
) {
    let approval_height = if state.pending_approval.is_some() { 4 } else { 0 };
    let busy_height = if state.is_busy() { 1 } else { 0 };
    let attachment_height = if state.composer.pending_attachments.is_empty() {
        0
    } else {
        4
    };
    let autocomplete_height = if state.composer.is_slash_mode() && !state.autocomplete.is_empty() {
        (state.autocomplete.len() as u16).min(8)
    } else {
        0
    };
    let composer_lines = state.composer.text.lines().count().clamp(1, 5) as u16;
    let composer_height = composer_lines + 2; // top rule + lines + bottom rule

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),
            Constraint::Length(approval_height),
            Constraint::Length(busy_height),
            Constraint::Length(1), // status line
            Constraint::Length(attachment_height),
            Constraint::Length(composer_height),
            Constraint::Length(autocomplete_height),
        ])
        .split(area);

    let feed_rect = chunks[0];
    let approval_rect = chunks[1];
    let busy_rect = chunks[2];
    let status_rect = chunks[3];
    let attachment_rect = chunks[4];
    let composer_rect = chunks[5];
    let ac_rect = chunks[6];

    hit.chat_area = feed_rect;
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
    render_composer(composer_rect, buf, state, palette);

    if autocomplete_height > 0 {
        render_autocomplete(ac_rect, buf, state, palette, hit);
    }
}

fn render_chat_feed(
    area: Rect,
    buf: &mut Buffer,
    state: &TuiAppState,
    palette: &ThemePalette,
    hit: &mut HitRegions,
) {
    if area.width < 4 || area.height == 0 {
        return;
    }
    let items = state.active_timeline();
    let pad_x = 1u16;
    let content_w = area.width.saturating_sub(pad_x * 2) as usize;
    if content_w < 4 {
        return;
    }

    let mut rendered_lines: Vec<Line<'static>> = Vec::new();
    for item in items {
        match item {
            ChatTimelineItem::UserMessage {
                sender,
                text,
                is_group,
                ..
            } => {
                if state.prefs.message_layout == MessageLayout::Chat {
                    let max_bubble_w = ((content_w * 70) / 100).max(12).min(content_w);
                    let inner_max = max_bubble_w.saturating_sub(2);
                    let wrapped = wrap_text_lines(text, inner_max);
                    let text_w = wrapped
                        .iter()
                        .map(|l| l.width())
                        .max()
                        .unwrap_or(1)
                        .min(inner_max);
                    let bubble_w = text_w + 2;
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
                                format!(" {wline}{} ", " ".repeat(trail)),
                                Style::default().bg(palette.bubble_bg).fg(palette.bubble_fg),
                            ),
                        ]));
                    }
                    rendered_lines.push(Line::default());
                } else {
                    for wline in wrap_text_lines(text, content_w.saturating_sub(2)) {
                        rendered_lines.push(Line::from(Span::styled(
                            format!("› {wline}"),
                            Style::default().fg(palette.fg).bg(palette.bubble_bg),
                        )));
                    }
                    rendered_lines.push(Line::default());
                }
            }
            ChatTimelineItem::AssistantMessage {
                text, duration_ms, ..
            } => {
                for wline in wrap_text_lines(text, content_w) {
                    rendered_lines.push(Line::from(Span::styled(
                        wline,
                        Style::default().fg(palette.fg),
                    )));
                }
                if state.prefs.show_turn_duration
                    && let Some(ms) = duration_ms
                {
                    let label = format!("  {:.1}s", *ms as f64 / 1000.0);
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
    }

    if rendered_lines.is_empty() {
        let hint = format!(
            "  {} · ready. Type a message or /settings",
            state.gateway_cfg.soul.name
        );
        rendered_lines.push(Line::from(Span::styled(
            hint,
            Style::default().fg(palette.dim),
        )));
    }

    let visible_h = area.height as usize;
    let total = rendered_lines.len();
    let max_scroll = total.saturating_sub(visible_h);
    let scroll = if state.auto_scroll {
        max_scroll
    } else {
        state.chat_scroll.min(max_scroll)
    };

    for (row_idx, line) in rendered_lines.iter().skip(scroll).take(visible_h).enumerate() {
        buf.set_line(
            area.x + pad_x,
            area.y + row_idx as u16,
            line,
            content_w as u16,
        );
    }

    // Record exact rendered viewport lines for mouse drag-select extraction.
    for row_idx in 0..area.height {
        let y = area.y + row_idx;
        let mut row_text = String::new();
        let mut skip = 0usize;
        for x in area.x..area.x.saturating_add(area.width) {
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

    // Apply selection highlight overlay across normalized (start..=end) cell range.
    if let Some(sel) = state.selection
        && sel.anchor != sel.current
    {
        let (start, end) = normalize_selection(sel.anchor, sel.current);
        let max_x = area.x.saturating_add(area.width.saturating_sub(1));
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
        hit.approval_approve = Some(Rect::new(approve_x, btn_y, approve_lbl.len() as u16, 1));
        hit.approval_deny = Some(Rect::new(deny_x, btn_y, deny_lbl.len() as u16, 1));
    }
}

fn render_busy_phase_line(
    area: Rect,
    buf: &mut Buffer,
    state: &TuiAppState,
    effects: &TuiEffects,
    palette: &ThemePalette,
) {
    let spinner = effects.spinner_glyph(state.prefs.motion);
    let mut spans = vec![Span::styled(
        format!(" {spinner} "),
        Style::default().fg(palette.accent),
    )];
    for (idx, ch) in state.thinking_phase.chars().enumerate() {
        let style = effects.shimmer_style(idx, state.prefs.shimmer, state.prefs.motion, palette);
        spans.push(Span::styled(ch.to_string(), style));
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
    let mut spans: Vec<Span<'static>> = Vec::new();
    if state.is_busy() {
        let spinner = effects.spinner_glyph(state.prefs.motion);
        let elapsed = TuiAppState::format_elapsed(Duration::from_millis(state.thinking_elapsed_ms));
        spans.push(Span::styled(
            format!(" {spinner} {elapsed} › "),
            Style::default()
                .fg(palette.accent)
                .bg(palette.status_bg)
                .add_modifier(Modifier::BOLD),
        ));
    } else {
        spans.push(Span::styled(
            " ● ",
            Style::default().fg(palette.status_green).bg(palette.status_bg),
        ));
    }

    spans.push(Span::styled(
        state.model_name.clone(),
        Style::default().fg(palette.fg).bg(palette.status_bg),
    ));

    if state.prefs.show_valve_badge {
        spans.push(Span::styled(
            format!(" · Valve: {}", state.valve_mode),
            Style::default().fg(palette.muted).bg(palette.status_bg),
        ));
    }

    if state.prefs.show_cost_estimate {
        spans.push(Span::styled(
            format!(" · ${:.2}", state.estimated_cost_usd),
            Style::default().fg(palette.muted).bg(palette.status_bg),
        ));
    }

    if state.prefs.show_token_budget {
        let pct = ((state.daily_tokens_used as f64 / state.daily_token_budget.max(1) as f64)
            * 100.0)
            .round()
            .min(100.0) as u64;
        spans.push(Span::styled(
            format!(" ── {pct}%"),
            Style::default().fg(palette.dim).bg(palette.status_bg),
        ));
    }

    spans.push(Span::styled(
        format!(" · {}", state.chats.active_chat().title),
        Style::default().fg(palette.dim).bg(palette.status_bg),
    ));

    if state.lab_mode {
        spans.push(Span::raw(" "));
        spans.push(Span::styled(
            " lab ",
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
) {
    if area.height < 3 {
        return;
    }
    let rule = "─".repeat(area.width as usize);
    let rule_style = Style::default().fg(palette.separator);
    buf.set_string(area.x, area.y, &rule, rule_style);

    let text_rows = area.height - 2;
    let lines: Vec<&str> = if state.composer.text.is_empty() {
        vec![""]
    } else {
        state.composer.text.split('\n').collect()
    };

    for row in 0..text_rows {
        let y = area.y + 1 + row;
        let prefix = if row == 0 { "› " } else { "  " };
        let content = lines.get(row as usize).copied().unwrap_or("");
        if state.composer.text.is_empty() && row == 0 {
            let line = Line::from(vec![
                Span::styled(prefix, Style::default().fg(palette.accent)),
                Span::styled(
                    "Message Mono or type / for commands…",
                    Style::default().fg(palette.dim),
                ),
            ]);
            buf.set_line(area.x + 1, y, &line, area.width.saturating_sub(2));
        } else {
            let line = Line::from(vec![
                Span::styled(prefix, Style::default().fg(palette.accent)),
                Span::styled(content.to_owned(), Style::default().fg(palette.fg)),
            ]);
            buf.set_line(area.x + 1, y, &line, area.width.saturating_sub(2));
        }
    }

    buf.set_string(area.x, area.y + area.height - 1, &rule, rule_style);
}

fn render_autocomplete(
    area: Rect,
    buf: &mut Buffer,
    state: &TuiAppState,
    palette: &ThemePalette,
    hit: &mut HitRegions,
) {
    fill_bg(buf, area, palette.sidebar_bg, palette.fg);
    let col_left_w = 24usize.min((area.width as usize) / 2);
    for (idx, item) in state.autocomplete.iter().take(area.height as usize).enumerate() {
        let y = area.y + idx as u16;
        let row_rect = Rect::new(area.x, y, area.width, 1);
        hit.autocomplete_rows.push((row_rect, idx));

        let selected = idx == state.autocomplete_idx;
        let bg = if selected {
            palette.selection_bg
        } else {
            palette.sidebar_bg
        };
        let left_fg = if selected { palette.accent } else { palette.fg };
        fill_bg(buf, row_rect, bg, left_fg);

        let left_text = format!(" {} /{}", item.icon, item.name);
        let padded_left = format!("{left_text:<width$}", width = col_left_w);
        let right_text = match &item.current_value {
            Some(v) => format!("{}  [{v}]", item.description),
            None => item.description.clone(),
        };
        let line = Line::from(vec![
            Span::styled(padded_left, Style::default().fg(left_fg).bg(bg)),
            Span::styled(
                truncate_to_width(&right_text, (area.width as usize).saturating_sub(col_left_w + 2)),
                Style::default().fg(palette.muted).bg(bg),
            ),
        ]);
        buf.set_line(area.x, y, &line, area.width);
    }
}

fn render_settings_modal(
    area: Rect,
    buf: &mut Buffer,
    state: &TuiAppState,
    palette: &ThemePalette,
    hit: &mut HitRegions,
) {
    Clear.render(area, buf);
    fill_bg(buf, area, palette.modal_bg, palette.fg);
    let block = Block::bordered()
        .border_style(Style::default().fg(palette.separator))
        .title(" Settings  (←/→ Tab · ↑/↓ Select · Enter/Space Edit · Esc Back/Close) ");
    let inner = block.inner(area);
    block.render(area, buf);

    if inner.width < 24 || inner.height < 8 {
        return;
    }

    // Tab bar + search bar
    let mut tab_x = inner.x + 1;
    let tab_y = inner.y;
    for tab in SettingsTab::ALL {
        let label = format!(" {} ", tab.label());
        let w = label.width() as u16;
        if tab_x + w >= inner.x + inner.width {
            break;
        }
        let active = tab == state.settings.tab;
        let style = if active {
            Style::default()
                .bg(palette.accent)
                .fg(palette.accent_fg)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(palette.muted).bg(palette.modal_bg)
        };
        buf.set_string(tab_x, tab_y, &label, style);
        hit.settings_tabs.push((Rect::new(tab_x, tab_y, w, 1), tab));
        tab_x += w + 1;
    }

    let search_y = inner.y + 1;
    let search_line = Line::from(vec![
        Span::styled(" Search: ", Style::default().fg(palette.dim)),
        Span::styled(
            if state.settings.search_query.is_empty() {
                "(type to filter…)".to_owned()
            } else {
                state.settings.search_query.clone()
            },
            Style::default().fg(if state.settings.search_query.is_empty() {
                palette.dim
            } else {
                palette.accent
            }),
        ),
    ]);
    buf.set_line(inner.x, search_y, &search_line, inner.width);

    let rule = "─".repeat(inner.width as usize);
    buf.set_string(inner.x, inner.y + 2, &rule, Style::default().fg(palette.separator));

    let body_area = Rect::new(
        inner.x,
        inner.y + 3,
        inner.width,
        inner.height.saturating_sub(4),
    );

    if let Some(detail) = &state.settings.plugin_detail {
        render_plugin_detail(body_area, buf, state, detail, palette, hit);
    } else {
        let panes = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(22), Constraint::Min(20)])
            .split(body_area);

        render_settings_sections(panes[0], buf, state, palette, hit);
        render_settings_options(panes[1], buf, state, palette, hit);
    }

    if let Some(edit) = &state.settings.editing {
        let prompt_area = Rect::new(
            inner.x + 1,
            inner.y + inner.height.saturating_sub(1),
            inner.width.saturating_sub(2),
            1,
        );
        let display_buf = if edit.masked {
            "●".repeat(edit.buffer.chars().count())
        } else {
            edit.buffer.clone()
        };
        let line = Line::from(vec![
            Span::styled(
                format!("{}: ", edit.prompt),
                Style::default()
                    .fg(palette.accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!("{display_buf}▏"), Style::default().fg(palette.fg)),
        ]);
        fill_bg(buf, prompt_area, palette.selection_bg, palette.fg);
        buf.set_line(prompt_area.x, prompt_area.y, &line, prompt_area.width);
    }
}

fn render_settings_sections(
    area: Rect,
    buf: &mut Buffer,
    state: &TuiAppState,
    palette: &ThemePalette,
    hit: &mut HitRegions,
) {
    let sep_x = area.x + area.width.saturating_sub(1);
    for y in area.y..area.y + area.height {
        if let Some(cell) = buf.cell_mut((sep_x, y)) {
            cell.set_symbol("│");
            cell.set_style(Style::default().fg(palette.separator).bg(palette.modal_bg));
        }
    }

    for (idx, sec) in state.settings.tab.sections().iter().enumerate() {
        let y = area.y + idx as u16;
        if y >= area.y + area.height {
            break;
        }
        let rect = Rect::new(area.x, y, area.width.saturating_sub(1), 1);
        hit.settings_sections.push((rect, idx));
        let active = idx == state.settings.section_index;
        let style = if active {
            Style::default()
                .bg(palette.selection_bg)
                .fg(palette.accent)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(palette.muted).bg(palette.modal_bg)
        };
        fill_bg(buf, rect, if active { palette.selection_bg } else { palette.modal_bg }, palette.fg);
        buf.set_string(area.x + 1, y, *sec, style);
    }
}

fn render_settings_options(
    area: Rect,
    buf: &mut Buffer,
    state: &TuiAppState,
    palette: &ThemePalette,
    hit: &mut HitRegions,
) {
    let rows = build_settings_rows(
        state.settings.tab,
        &state.settings.search_query,
        &state.prefs,
        &state.gateway_cfg,
        &state.paths,
        &state.secrets_summary,
        &state.settings.valve_toggles,
        &state.plugins,
    );

    let preview_h = if state.settings.tab == SettingsTab::Appearance && area.height >= 10 {
        4
    } else {
        0
    };
    let list_h = area.height.saturating_sub(preview_h) as usize;
    let selected = state.settings.selected_index.min(rows.len().saturating_sub(1));
    let scroll = if selected >= state.settings.scroll_offset + list_h {
        selected.saturating_sub(list_h.saturating_sub(1))
    } else {
        state.settings.scroll_offset.min(selected)
    };

    for (vis_idx, row) in rows.iter().skip(scroll).take(list_h).enumerate() {
        let abs_idx = scroll + vis_idx;
        let y = area.y + vis_idx as u16;
        let rect = Rect::new(area.x, y, area.width, 1);
        hit.settings_options.push((rect, abs_idx));

        let is_sel = abs_idx == selected;
        let bg = if is_sel {
            palette.selection_bg
        } else {
            palette.modal_bg
        };
        fill_bg(buf, rect, bg, palette.fg);

        let val_str = match &row.value {
            SettingValueKind::Bool(b) => {
                if *b { "[x] on".to_owned() } else { "[ ] off".to_owned() }
            }
            SettingValueKind::Cycle(s)
            | SettingValueKind::Text(s)
            | SettingValueKind::Number(s)
            | SettingValueKind::Action(s)
            | SettingValueKind::ReadOnly(s) => s.clone(),
            SettingValueKind::MaskedSecret { is_set } => {
                if *is_set { "●●●● (set)".to_owned() } else { "(unset)".to_owned() }
            }
        };

        let left_w = ((area.width as usize) / 2).max(18);
        let right_w = (area.width as usize).saturating_sub(left_w + 3);
        let label_padded = format!(
            " {:<width$}",
            truncate_to_width(&row.label, left_w.saturating_sub(2)),
            width = left_w.saturating_sub(1)
        );
        let val_trunc = truncate_to_width(&val_str, right_w);

        let line = Line::from(vec![
            Span::styled(
                label_padded,
                Style::default()
                    .fg(if is_sel { palette.accent } else { palette.fg })
                    .bg(bg),
            ),
            Span::styled(val_trunc, Style::default().fg(palette.muted).bg(bg)),
        ]);
        buf.set_line(area.x, y, &line, area.width);
    }

    if preview_h > 0 {
        let py = area.y + list_h as u16;
        let preview_area = Rect::new(area.x + 1, py, area.width.saturating_sub(2), preview_h);
        buf.set_string(
            preview_area.x,
            preview_area.y,
            "Live Preview:",
            Style::default().fg(palette.dim),
        );
        let bubble = Span::styled(
            " Hello Mono! ",
            Style::default().bg(palette.bubble_bg).fg(palette.bubble_fg),
        );
        buf.set_line(preview_area.x, preview_area.y + 1, &Line::from(vec![bubble]), preview_area.width);
        let status_preview = format!(
            " ● {} · Valve: {} · $0.00 ── 0%",
            state.model_name, state.valve_mode
        );
        buf.set_string(
            preview_area.x,
            preview_area.y + 2,
            truncate_to_width(&status_preview, preview_area.width as usize),
            Style::default().bg(palette.status_bg).fg(palette.fg),
        );
    }
}

fn render_plugin_detail(
    area: Rect,
    buf: &mut Buffer,
    state: &TuiAppState,
    detail: &PluginDetailState,
    palette: &ThemePalette,
    hit: &mut HitRegions,
) {
    let plugin = state.plugins.iter().find(|p| p.id == detail.plugin_id);
    let header_status = plugin
        .map(|p| format!("{} {}", p.state.dot_glyph(), p.state.label()))
        .unwrap_or_else(|| "unknown".to_owned());
    let header = Line::from(vec![
        Span::styled(
            format!(" Plugins › {} ", detail.plugin_id),
            Style::default()
                .fg(palette.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!("({header_status})"), Style::default().fg(palette.muted)),
    ]);
    buf.set_line(area.x + 1, area.y, &header, area.width.saturating_sub(2));

    let running = plugin.is_some_and(|p| matches!(p.state, PluginUiState::Running | PluginUiState::Starting));
    let enabled = plugin.is_some_and(|p| p.enabled);
    let mut rows: Vec<(String, String)> = vec![
        (
            if running {
                "[ Stop Plugin ]".to_owned()
            } else {
                "[ Start Plugin ]".to_owned()
            },
            "Spawn / stop Waist-2 body".to_owned(),
        ),
        (
            "[ Pair Device ]".to_owned(),
            "Run interactive `pair` in TTY".to_owned(),
        ),
        (
            "[ Enable on Startup ]".to_owned(),
            format!("{}", enabled),
        ),
        (
            "[ Manage Env Secrets ]".to_owned(),
            "Add/update [env] secret in secrets.toml (0600)".to_owned(),
        ),
        (
            format!("[ Reload {} ]", detail.toml_path.file_name().and_then(|n| n.to_str()).unwrap_or("plugin.toml")),
            format!("{} keys", detail.toml_entries.len()),
        ),
    ];

    for entry in &detail.toml_entries {
        rows.push((entry.key_path.clone(), entry.value.display_compact()));
    }
    rows.push(("[ + Add TOML Key ]".to_owned(), "Insert key in plugins/<id>.toml".to_owned()));

    let start_y = area.y + 2;
    let avail_h = area.height.saturating_sub(2) as usize;
    for (idx, (k, v)) in rows.iter().take(avail_h).enumerate() {
        let y = start_y + idx as u16;
        let rect = Rect::new(area.x + 1, y, area.width.saturating_sub(2), 1);
        hit.settings_options.push((rect, idx));
        let sel = idx == detail.selected_row;
        let bg = if sel {
            palette.selection_bg
        } else {
            palette.modal_bg
        };
        fill_bg(buf, rect, bg, palette.fg);
        let left_w = ((rect.width as usize) / 2).max(20);
        let line = Line::from(vec![
            Span::styled(
                format!(" {:<w$}", truncate_to_width(k, left_w - 2), w = left_w - 1),
                Style::default()
                    .fg(if sel { palette.accent } else { palette.fg })
                    .bg(bg),
            ),
            Span::styled(
                truncate_to_width(v, (rect.width as usize).saturating_sub(left_w + 1)),
                Style::default().fg(palette.muted).bg(bg),
            ),
        ]);
        buf.set_line(rect.x, rect.y, &line, rect.width);
    }
}

fn render_setup_wizard(
    area: Rect,
    buf: &mut Buffer,
    wizard: &SetupWizardState,
    palette: &ThemePalette,
    hit: &mut HitRegions,
) {
    Clear.render(area, buf);
    fill_bg(buf, area, palette.modal_bg, palette.fg);
    let title = match wizard.step {
        WizardStep::Step1Provider => " MonoAgens Setup (Step 1/2: Provider & Key) ",
        WizardStep::Step2Plugins => " MonoAgens Setup (Step 2/2: Enable Plugins) ",
    };
    let block = Block::bordered()
        .border_style(Style::default().fg(palette.accent))
        .title(title);
    let inner = block.inner(area);
    block.render(area, buf);

    if inner.width < 20 || inner.height < 8 {
        return;
    }

    match wizard.step {
        WizardStep::Step1Provider => {
            let fields = [
                ("Endpoint", wizard.endpoint.clone()),
                ("API Key (0600)", wizard.masked_api_key()),
                ("Model", wizard.model.clone()),
                ("[ Next: Plugins → ]", String::new()),
                ("[ Skip Setup ]", String::new()),
            ];
            for (idx, (lbl, val)) in fields.iter().enumerate() {
                let y = inner.y + 1 + (idx as u16) * 2;
                if y >= inner.y + inner.height {
                    break;
                }
                let rect = Rect::new(inner.x + 1, y, inner.width.saturating_sub(2), 1);
                hit.wizard_buttons.push((rect, idx));
                let sel = wizard.selected_field == idx;
                let bg = if sel {
                    palette.selection_bg
                } else {
                    palette.modal_bg
                };
                fill_bg(buf, rect, bg, palette.fg);
                let suffix = if sel && wizard.editing { " ▏" } else { "" };
                let text = if val.is_empty() {
                    format!(" {lbl}")
                } else {
                    format!(" {lbl:<16} {val}{suffix}")
                };
                buf.set_string(
                    rect.x,
                    rect.y,
                    truncate_to_width(&text, rect.width as usize),
                    Style::default()
                        .fg(if sel { palette.accent } else { palette.fg })
                        .bg(bg),
                );
            }
        }
        WizardStep::Step2Plugins => {
            buf.set_string(
                inner.x + 1,
                inner.y + 1,
                "Space: toggle plugin · Enter: Save & Launch Chat · ←: Back",
                Style::default().fg(palette.dim),
            );
            let max_rows = inner.height.saturating_sub(5) as usize;
            for (idx, pid) in wizard.plugin_ids.iter().take(max_rows).enumerate() {
                let y = inner.y + 3 + idx as u16;
                let rect = Rect::new(inner.x + 1, y, inner.width.saturating_sub(2), 1);
                hit.wizard_buttons.push((rect, idx));
                let checked = wizard.plugin_enabled.get(idx).copied().unwrap_or(false);
                let sel = wizard.plugin_cursor == idx;
                let bg = if sel {
                    palette.selection_bg
                } else {
                    palette.modal_bg
                };
                fill_bg(buf, rect, bg, palette.fg);
                let box_str = if checked { "[x]" } else { "[ ]" };
                buf.set_string(
                    rect.x,
                    rect.y,
                    format!(" {box_str} {pid}"),
                    Style::default()
                        .fg(if sel { palette.accent } else { palette.fg })
                        .bg(bg),
                );
            }
            let finish_y = inner.y + inner.height.saturating_sub(2);
            let back_rect = Rect::new(inner.x + 2, finish_y, 12, 1);
            let save_rect = Rect::new(inner.x + 16, finish_y, 20, 1);
            hit.wizard_buttons.push((back_rect, 998));
            hit.wizard_buttons.push((save_rect, 999));
            buf.set_string(
                back_rect.x,
                back_rect.y,
                "[ ← Back ]",
                Style::default().fg(palette.muted),
            );
            buf.set_string(
                save_rect.x,
                save_rect.y,
                "[ Save & Finish ]",
                Style::default()
                    .bg(palette.accent)
                    .fg(palette.accent_fg)
                    .add_modifier(Modifier::BOLD),
            );
        }
    }

    if let Some(err) = &wizard.error {
        let p = Paragraph::new(err.as_str())
            .style(Style::default().fg(palette.status_red))
            .wrap(Wrap { trim: true });
        p.render(
            Rect::new(
                inner.x + 1,
                inner.y + inner.height.saturating_sub(1),
                inner.width.saturating_sub(2),
                1,
            ),
            buf,
        );
    }
}

fn render_toast(area: Rect, buf: &mut Buffer, text: &str, is_error: bool, palette: &ThemePalette) {
    let w = (text.width() as u16 + 4).min(area.width.saturating_sub(4)).max(12);
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
        truncate_to_width(text, w.saturating_sub(2) as usize),
        Style::default().fg(fg).bg(palette.status_bg),
    );
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let w = ((u32::from(area.width) * u32::from(percent_x)) / 100)
        .max(24)
        .min(u32::from(area.width)) as u16;
    let h = ((u32::from(area.height) * u32::from(percent_y)) / 100)
        .max(10)
        .min(u32::from(area.height)) as u16;
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    Rect::new(x, y, w, h)
}

fn centered_fixed_rect(max_w: u16, max_h: u16, area: Rect) -> Rect {
    let w = max_w.min(area.width);
    let h = max_h.min(area.height);
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    Rect::new(x, y, w, h)
}

fn truncate_to_width(s: &str, max_w: usize) -> String {
    if max_w == 0 {
        return String::new();
    }
    if s.width() <= max_w {
        return s.to_owned();
    }
    let mut out = String::new();
    let mut cur_w = 0usize;
    for ch in s.chars() {
        let cw = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
        if cur_w + cw + 1 > max_w {
            out.push('…');
            break;
        }
        out.push(ch);
        cur_w += cw;
    }
    out
}

fn wrap_text_lines(text: &str, max_w: usize) -> Vec<String> {
    if max_w == 0 {
        return vec![String::new()];
    }
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
                for ch in word.chars() {
                    let cw = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
                    if current_w + cw > max_w && !current.is_empty() {
                        lines.push(current.clone());
                        current.clear();
                        current_w = 0;
                    }
                    current.push(ch);
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
