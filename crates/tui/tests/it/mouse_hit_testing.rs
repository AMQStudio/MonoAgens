//! Contract tests for `HitMap` z-order & exact cell bounds, widget hit recording
//! (`Tab`, `ListRow`, `FooterChip`, `Scrollbar`, `Backdrop`), horizontal scroll drift
//! filtering, and deterministic scrollbar click/drag offset projection.

use crossterm::event::MouseEventKind;
use monoagens_gateway::{GatewayConfig, LogEntry, ResolvedPaths, RunLogLevel};
use monoagens_protocol::ActorId;
use monoagens_tui::catalog_ui::{ModelRow, ModelsEntry, ProviderRow, Screen};
use monoagens_tui::core::hitmap::{HitMap, HitTarget, ScrollId};
use monoagens_tui::core::overlay::OverlayId;
use monoagens_tui::core::theme::{ChromeGlyphs, ThemePalette, ThemePreset};
use monoagens_tui::core::viewport::{
    clamp_scroll_offset, is_horizontal_scroll, max_scroll_offset, offset_from_scrollbar_y,
    scroll_offset_for_row, scrollbar_thumb_range,
};
use monoagens_tui::widgets::hub_frame::{
    FooterChip, FooterMode, HubFrameConfig, compute_hub_areas, render_hub_frame,
};
use monoagens_tui::widgets::menu_list::{MenuItem, MenuRenderConfig, MenuSelection, render_menu_list};
use monoagens_tui::widgets::scrollbar::render_vertical_scrollbar;
use monoagens_tui::widgets::tab_bar::{TabBar, TabItem};
use monoagens_tui::{
    ChatTimelineItem, HitRegions, IconMode, LocalChatStore, SecretsUiSummary, TuiAppState,
    TuiEffects, TuiPreferences, render_to_buffer,
};
use ratatui::layout::Rect;
use ratatui::style::Style;
use tempfile::TempDir;
use unicode_width::UnicodeWidthStr;

use crate::common::{
    TuiTestHarness, mouse_click, mouse_drag, mouse_scroll_down, mouse_scroll_left, mouse_scroll_up,
};

#[test]
fn hitmap_z_order_empty_rects_and_exact_cell_bounds() {
    let mut hitmap = HitMap::default();
    hitmap.record(Rect::new(0, 0, 10, 5), HitTarget::Backdrop(OverlayId::Help));
    hitmap.record(Rect::new(2, 1, 4, 2), HitTarget::Tab(1));

    // Empty-width and empty-height rects must never shadow recorded geometry.
    hitmap.record(Rect::new(3, 1, 0, 2), HitTarget::Tab(9));
    hitmap.record(Rect::new(3, 1, 2, 0), HitTarget::Tab(9));

    // Last recorded non-empty rect wins inside exact half-open bounds [x..x+w, y..y+h).
    assert_eq!(hitmap.hit_test(2, 1), Some(&HitTarget::Tab(1)));
    assert_eq!(hitmap.hit_test(5, 2), Some(&HitTarget::Tab(1)));

    // One cell past right/bottom edges falls back to the underlying Backdrop.
    assert_eq!(
        hitmap.hit_test(6, 2),
        Some(&HitTarget::Backdrop(OverlayId::Help))
    );
    assert_eq!(
        hitmap.hit_test(5, 3),
        Some(&HitTarget::Backdrop(OverlayId::Help))
    );

    // Outside all recorded rects returns None.
    assert_eq!(hitmap.hit_test(10, 0), None);
    assert_eq!(hitmap.hit_test(0, 5), None);
}

#[test]
fn rendered_hub_frame_tab_bar_menu_list_and_scrollbar_record_accurate_hit_rects() {
    let palette = ThemePalette::for_preset_with_no_color(ThemePreset::Titanium, true);
    let glyphs = ChromeGlyphs::for_mode(IconMode::Symbols);
    assert_eq!(UnicodeWidthStr::width(glyphs.pointer), 1);

    let mut harness = TuiTestHarness::new(80, 24);
    let mut hitmap = HitMap::default();

    let tabs = vec![
        TabItem {
            label: "Roles".to_owned(),
            short_label: "Roles".to_owned(),
            enabled: true,
            badge: None,
        },
        TabItem {
            label: "Actors".to_owned(),
            short_label: "Actors".to_owned(),
            enabled: true,
            badge: None,
        },
    ];
    let tab_bar = TabBar::new(tabs);
    let chips = vec![
        FooterChip {
            label: "Toggle".to_owned(),
            active: true,
            danger: false,
        },
        FooterChip {
            label: "Close".to_owned(),
            active: false,
            danger: false,
        },
    ];
    let config = HubFrameConfig {
        overlay_id: OverlayId::Settings,
        title: "Access Control".to_owned(),
        max_width: 72,
        max_height: 20,
        width_pct: 90,
        height_pct: 85,
        tab_bar: Some(&tab_bar),
        rail_width: 24,
        inspector_height: 0,
    };
    let items: Vec<MenuItem<String>> = (0..20)
        .map(|i| MenuItem {
            key: format!("role_{i}"),
            label: format!("Role {i}"),
            secondary: None,
            selectable: true,
            data: format!("role_{i}"),
        })
        .collect();
    let mut selection = MenuSelection::new(items);
    let list_cfg = MenuRenderConfig {
        scroll_id: ScrollId::RbacRail,
        show_pointer: true,
        secondary_right_aligned: false,
    };

    let areas = compute_hub_areas(harness.area(), &config);
    let rail_area = areas.rail.expect("rail present at 80x24");
    let tab_area = areas.tab_bar.expect("tab bar present at 80x24");

    harness.draw_with(|screen, buf| {
        hitmap.clear();
        let frame_areas = render_hub_frame(
            screen,
            buf,
            &config,
            FooterMode::Chips(&chips),
            &palette,
            &mut hitmap,
        );
        let frame_rail = frame_areas.rail.expect("rail area");
        render_menu_list(
            frame_rail,
            buf,
            &mut selection,
            &palette,
            &mut hitmap,
            list_cfg,
        );
    });

    // Backdrop outside the modal hits Backdrop.
    assert_eq!(
        hitmap.hit_test(0, 0),
        Some(&HitTarget::Backdrop(OverlayId::Settings))
    );

    // At least Tab(0) and Tab(1) exist on the tab row, and the rendered text contains "Roles"/"Actors".
    let mut found_tab0 = false;
    let mut found_tab1 = false;
    for col in tab_area.x..tab_area.right() {
        match hitmap.hit_test(col, tab_area.y) {
            Some(HitTarget::Tab(0)) => found_tab0 = true,
            Some(HitTarget::Tab(1)) => found_tab1 = true,
            _ => {}
        }
    }
    assert!(
        found_tab0 && found_tab1,
        "expected Tab(0) and Tab(1) hits in {tab_area:?}"
    );
    harness.assert_text_in_rect(tab_area, "Roles");
    harness.assert_text_in_rect(tab_area, "Actors");

    // FooterChip hits exist on the footer row matching rendered chip labels.
    let mut found_chip0 = false;
    let mut found_chip1 = false;
    for col in areas.footer.x..areas.footer.right() {
        match hitmap.hit_test(col, areas.footer.y) {
            Some(HitTarget::FooterChip(0)) => found_chip0 = true,
            Some(HitTarget::FooterChip(1)) => found_chip1 = true,
            _ => {}
        }
    }
    assert!(
        found_chip0 && found_chip1,
        "expected FooterChip(0) and FooterChip(1) hits in {:?}",
        areas.footer
    );
    harness.assert_text_in_rect(areas.footer, "Toggle");
    harness.assert_text_in_rect(areas.footer, "Close");

    // MenuList rows and vertical scrollbar hit rects match rendered buffer coordinates.
    let mut found_row0 = false;
    let mut found_scrollbar = false;
    for row in rail_area.y..rail_area.bottom() {
        for col in rail_area.x..rail_area.right() {
            match hitmap.hit_test(col, row) {
                Some(HitTarget::ListRow(0)) => found_row0 = true,
                Some(HitTarget::Scrollbar {
                    id: ScrollId::RbacRail,
                    ..
                }) => found_scrollbar = true,
                _ => {}
            }
        }
    }
    assert!(found_row0, "expected ListRow(0) hit in {rail_area:?}");
    assert!(
        found_scrollbar,
        "expected Scrollbar hit in {rail_area:?}"
    );
    harness.assert_text_in_rect(rail_area, "Role 0");
}

#[test]
fn horizontal_scroll_drift_is_filtered_from_vertical_scroll() {
    assert!(is_horizontal_scroll(MouseEventKind::ScrollLeft));
    assert!(is_horizontal_scroll(MouseEventKind::ScrollRight));
    assert!(!is_horizontal_scroll(MouseEventKind::ScrollUp));
    assert!(!is_horizontal_scroll(MouseEventKind::ScrollDown));

    let left_ev = mouse_scroll_left(10, 5);
    let up_ev = mouse_scroll_up(10, 5);
    let down_ev = mouse_scroll_down(10, 5);

    let mut scroll_offset = 12usize;
    let max_offset = max_scroll_offset(40, 10);
    for ev in [left_ev, up_ev, down_ev] {
        if is_horizontal_scroll(ev.kind) {
            continue;
        }
        match ev.kind {
            MouseEventKind::ScrollUp => {
                scroll_offset = clamp_scroll_offset(scroll_offset.saturating_sub(1), 40, 10);
            }
            MouseEventKind::ScrollDown => {
                scroll_offset = clamp_scroll_offset(scroll_offset + 1, 40, 10);
            }
            _ => {}
        }
    }
    // Left scroll was ignored; Up (-1) then Down (+1) leaves offset unchanged and bounded.
    assert_eq!(scroll_offset, 12);
    assert!(scroll_offset <= max_offset);
}

#[test]
fn scrollbar_click_and_drag_project_deterministically_across_scroll_range() {
    let total = 50usize;
    let visible = 10usize;
    let track_y = 2u16;
    let track_height = 10u16;
    let max_offset = max_scroll_offset(total, visible);
    assert_eq!(max_offset, 40);

    // Top click maps to 0; bottom click maps to max_offset.
    let top_click = mouse_click(20, track_y);
    let bottom_click = mouse_click(20, track_y + track_height - 1);
    assert_eq!(
        offset_from_scrollbar_y(track_y, track_height, top_click.row, total, visible),
        0
    );
    assert_eq!(
        offset_from_scrollbar_y(track_y, track_height, bottom_click.row, total, visible),
        max_offset
    );

    // Middle click lands strictly inside (0, max_offset).
    let mid_click = mouse_click(20, track_y + track_height / 2);
    let mid_offset =
        offset_from_scrollbar_y(track_y, track_height, mid_click.row, total, visible);
    assert!(
        mid_offset > 0 && mid_offset < max_offset,
        "mid_offset={mid_offset}"
    );

    // Drag across every row in the track produces a monotonically non-decreasing projection.
    let mut prev = 0usize;
    for y in track_y..track_y + track_height {
        let drag_ev = mouse_drag(20, y);
        let projected =
            offset_from_scrollbar_y(track_y, track_height, drag_ev.row, total, visible);
        assert!(projected >= prev && projected <= max_offset);
        prev = projected;
    }
    assert_eq!(prev, max_offset);

    // Thumb range and row-scroll helpers stay within track/scroll bounds, and widget renders cleanly.
    let (thumb_start, thumb_len) =
        scrollbar_thumb_range(total, visible, mid_offset, track_height).expect("thumb range");
    assert!(thumb_len >= 1 && thumb_start + thumb_len <= track_height);
    assert_eq!(scroll_offset_for_row(49, 0, visible, total), max_offset);

    let palette = ThemePalette::for_preset_with_no_color(ThemePreset::Titanium, true);
    let mut harness = TuiTestHarness::new(10, 14);
    let mut hitmap = HitMap::default();
    let track_rect = Rect::new(9, track_y, 1, track_height);
    harness.draw_with(|_area, buf| {
        hitmap.clear();
        render_vertical_scrollbar(
            buf,
            track_rect,
            ScrollId::Palette,
            total,
            visible,
            mid_offset,
            &mut hitmap,
            Style::default().fg(palette.separator).bg(palette.modal_bg),
            Style::default().fg(palette.accent).bg(palette.modal_bg),
        );
    });
    assert_eq!(
        hitmap.hit_test(9, track_y + 1),
        Some(&HitTarget::Scrollbar {
            id: ScrollId::Palette,
            track: track_rect,
            total_rows: total,
            viewport_rows: visible,
        })
    );
}

fn make_app_state(dir: &TempDir) -> TuiAppState {
    let cfg_dir = dir.path().join("config");
    let data_dir = dir.path().join("data");
    std::fs::create_dir_all(&cfg_dir).expect("cfg dir");
    std::fs::create_dir_all(&data_dir).expect("data dir");
    let paths = ResolvedPaths {
        config_path: cfg_dir.join("monoagens.toml"),
        secrets_path: cfg_dir.join("secrets.toml"),
        data_dir,
    };
    TuiAppState::new(
        paths,
        GatewayConfig::default(),
        TuiPreferences::default(),
        LocalChatStore::default(),
        SecretsUiSummary::default(),
        ActorId::new("per_owner"),
    )
}

fn find_scrollbar(hit: &HitRegions, target_id: ScrollId, area: Rect) -> (Rect, usize, usize) {
    for row in area.y..area.bottom() {
        for col in area.x..area.right() {
            if let Some((id, track, total, visible)) = hit.hitmap.scrollbar_at(col, row)
                && id == target_id
            {
                return (track, total, visible);
            }
        }
    }
    panic!("expected scrollbar {target_id:?} in {area:?}");
}

fn find_first_target(hit: &HitRegions, target: HitTarget, area: Rect) -> (u16, u16) {
    for row in area.y..area.bottom() {
        for col in area.x..area.right() {
            if hit.hitmap.hit_test(col, row) == Some(&target) {
                return (col, row);
            }
        }
    }
    panic!("expected {target:?} in {area:?}");
}

#[test]
fn help_overlay_wheel_clamps_selection_and_updates_offset() {
    let dir = TempDir::new_in(env!("CARGO_TARGET_TMPDIR")).expect("tempdir");
    let mut state = make_app_state(&dir);
    state.open_help();

    // Small height so Help entries exceed the visible viewport and require scrolling `off`.
    let viewport = Rect::new(0, 0, 80, 14);
    let mut harness = TuiTestHarness::new(80, 14);
    let mut effects = TuiEffects::new();
    let mut hit = HitRegions::default();
    harness.draw_with(|area, buf| {
        render_to_buffer(area, buf, &state, &mut effects, &mut hit, 16);
    });

    for _ in 0..8 {
        state.handle_mouse_event(mouse_scroll_down(40, 6), &hit, viewport);
    }
    let h = state.ux.help.as_ref().expect("help open");
    assert!(h.sel > 0);
    assert!(h.off > 0, "help scroll offset must advance on wheel down: {h:?}");
    let max_sel = h.sel;
    // Further wheel down clamps instead of overflowing past entries length.
    for _ in 0..20 {
        state.handle_mouse_event(mouse_scroll_down(40, 6), &hit, viewport);
    }
    assert_eq!(state.ux.help.as_ref().expect("help").sel, max_sel);
}

#[test]
fn clicking_composer_never_starts_chat_drag_selection() {
    let dir = TempDir::new_in(env!("CARGO_TARGET_TMPDIR")).expect("tempdir");
    let mut state = make_app_state(&dir);
    let active_id = state.chats.active_chat_id.clone();
    state.push_timeline_item(
        &active_id,
        ChatTimelineItem::UserMessage {
            sender: "me".to_owned(),
            text: "Hello chat feed".to_owned(),
            timestamp_ms: 1,
            is_group: false,
        },
    );

    let viewport = Rect::new(0, 0, 80, 24);
    let mut harness = TuiTestHarness::new(80, 24);
    let mut effects = TuiEffects::new();
    let mut hit = HitRegions::default();
    harness.draw_with(|area, buf| {
        render_to_buffer(area, buf, &state, &mut effects, &mut hit, 16);
    });

    let (comp_x, comp_y) = find_first_target(&hit, HitTarget::Composer, viewport);
    assert!(comp_y >= hit.chat_area.bottom());

    state.handle_mouse_event(mouse_click(comp_x, comp_y), &hit, viewport);
    assert!(
        state.selection.is_none(),
        "clicking composer must never start chat selection"
    );
    state.handle_mouse_event(mouse_drag(comp_x + 4, comp_y), &hit, viewport);
    assert!(state.selection.is_none());
}

#[test]
fn model_picker_and_log_support_row_click_wheel_and_scrollbar_drag() {
    let dir = TempDir::new_in(env!("CARGO_TARGET_TMPDIR")).expect("tempdir");
    let mut state = make_app_state(&dir);
    let viewport = Rect::new(0, 0, 80, 20);
    let mut harness = TuiTestHarness::new(80, 20);
    let mut effects = TuiEffects::new();
    let mut hit = HitRegions::default();

    // 1. /model picker row click + wheel + scrollbar drag
    state.catalog.providers = vec![ProviderRow {
        id: "openai".to_owned(),
        label: "OpenAI".to_owned(),
        keyless: false,
        key_set: true,
        key_source: Some("secrets".to_owned()),
        discovery: true,
        logged_in: true,
        custom: false,
        base_url: None,
        enabled: true,
    }];
    state.catalog.providers_loaded = true;
    state.catalog.models.insert(
        "openai".to_owned(),
        ModelsEntry {
            models: (0..40)
                .map(|i| ModelRow {
                    id: format!("gpt-model-{i:02}"),
                    display_name: None,
                    context_length: Some(128_000),
                    price_in_per_mtok: None,
                    price_out_per_mtok: None,
                })
                .collect(),
            ..ModelsEntry::default()
        },
    );
    let _ = state.catalog.open_picker(Some("openai".to_owned()), "gpt-model-00".to_owned());
    harness.draw_with(|area, buf| {
        render_to_buffer(area, buf, &state, &mut effects, &mut hit, 16);
    });

    // Click row 2 in model list
    let (rx, ry) = find_first_target(&hit, HitTarget::ListRow(2), viewport);
    state.handle_mouse_event(mouse_click(rx, ry), &hit, viewport);
    assert_eq!(state.catalog.picker.model_sel, 2);

    // Wheel down moves selection
    state.handle_mouse_event(mouse_scroll_down(rx, ry), &hit, viewport);
    assert_eq!(state.catalog.picker.model_sel, 5);

    // Scrollbar drag jumps to bottom
    let (sb_track, _, _) = find_scrollbar(&hit, ScrollId::CatalogList, viewport);
    state.handle_mouse_event(mouse_click(sb_track.x, sb_track.y), &hit, viewport);
    state.handle_mouse_event(
        mouse_drag(sb_track.x, sb_track.bottom() - 1),
        &hit,
        viewport,
    );
    assert!(state.catalog.picker.model_off > 10);
    state.catalog.screen = Screen::Closed;

    // 2. /log row click + wheel + scrollbar drag
    for i in 0..40 {
        state.log.push(LogEntry {
            ts_ms: 1_000 + i,
            level: RunLogLevel::Info,
            source: "gateway".to_owned(),
            message: format!("event line {i:02}"),
        });
    }
    state.log.open();
    harness.draw_with(|area, buf| {
        render_to_buffer(area, buf, &state, &mut effects, &mut hit, 16);
    });

    // Click a visible log row -> pauses follow and selects that row
    let first_row = state.log.effective_offset(40, hit.log_visible) + 1;
    let (lx, ly) = find_first_target(&hit, HitTarget::ListRow(first_row), viewport);
    state.handle_mouse_event(mouse_click(lx, ly), &hit, viewport);
    assert!(!state.log.follow);
    assert_eq!(state.log.selected, first_row);

    // Wheel up scrolls back
    let before_off = state.log.offset;
    state.handle_mouse_event(mouse_scroll_up(lx, ly), &hit, viewport);
    assert!(state.log.offset < before_off);

    // Scrollbar click + drag to top (0)
    let (log_sb, _, _) = find_scrollbar(&hit, ScrollId::Log, viewport);
    state.handle_mouse_event(mouse_click(log_sb.x, log_sb.bottom() - 1), &hit, viewport);
    state.handle_mouse_event(mouse_drag(log_sb.x, log_sb.y), &hit, viewport);
    assert_eq!(state.log.offset, 0);
}

#[test]
fn horizontal_scroll_is_ignored_by_tui_app_state() {
    let dir = TempDir::new_in(env!("CARGO_TARGET_TMPDIR")).expect("tempdir");
    let mut state = make_app_state(&dir);
    state.chat_scroll = 9;
    state.auto_scroll = false;

    let viewport = Rect::new(0, 0, 80, 24);
    let hit = HitRegions::default();
    let acts = state.handle_mouse_event(mouse_scroll_left(30, 10), &hit, viewport);
    assert!(acts.is_empty());
    assert_eq!(state.chat_scroll, 9);
    assert!(!state.auto_scroll);
}
