//! Contract tests for `OverlayStack` focus isolation, key-release filtering,
//! `HubFrame` backdrop clearing, footer integrity at `40x12`/`80x24`, and `TabBar` hits.

use crossterm::event::{KeyCode, KeyModifiers};
use monoagens_gateway::{GatewayConfig, ResolvedPaths};
use monoagens_protocol::ActorId;
use monoagens_tui::core::hitmap::{HitMap, HitTarget};
use monoagens_tui::core::overlay::{
    FocusScope, OverlayEntry, OverlayId, OverlayStack, is_key_press,
};
use monoagens_tui::core::theme::{ChromeGlyphs, ThemePalette, ThemePreset};
use monoagens_tui::message_actions::WhyOverlay;
use monoagens_tui::settings::{SettingsTab, SetupWizardState, WizardStep};
use monoagens_tui::widgets::hub_frame::{
    FooterChip, FooterMode, HubFrameConfig, compute_hub_areas, render_hub_frame,
};
use monoagens_tui::widgets::tab_bar::{TabBar, TabItem, render_tab_bar};
use monoagens_tui::{
    AppAction, HitRegions, IconMode, LocalChatStore, SecretsUiSummary, TuiAppState, TuiEffects,
    TuiPreferences, render_to_buffer,
};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use tempfile::TempDir;
use unicode_width::UnicodeWidthStr;

use crate::common::{
    TuiTestHarness, key_press, key_release, key_repeat, mouse_scroll_down, mouse_scroll_up,
};

fn overlay_entry(id: OverlayId) -> OverlayEntry {
    OverlayEntry {
        id,
        blocks_chat: true,
        dim_backdrop: true,
        sub_input_active: false,
    }
}

fn fill_background_with_leak_marker(area: Rect, buf: &mut Buffer, marker: &str) {
    for y in area.y..area.bottom() {
        let mut x = area.x;
        while x < area.right() {
            let remaining = usize::from(area.right() - x);
            let slice: String = marker.chars().take(remaining).collect();
            let width = u16::try_from(slice.len()).unwrap_or(area.right() - x);
            buf.set_string(x, y, &slice, Style::default());
            if width == 0 {
                break;
            }
            x = x.saturating_add(width);
        }
    }
}

#[test]
fn overlay_stack_push_raise_remove_pop_and_focus_scope() {
    let mut stack = OverlayStack::default();
    assert!(stack.is_empty());
    let base_scope = stack.focus_scope();
    assert_eq!(base_scope, FocusScope::ChatComposer);

    stack.push(overlay_entry(OverlayId::Settings));
    assert_eq!(stack.len(), 1);
    assert_eq!(stack.top().map(|entry| entry.id), Some(OverlayId::Settings));
    assert_eq!(stack.focus_scope(), FocusScope::Overlay(OverlayId::Settings));

    stack.push(overlay_entry(OverlayId::Help));
    assert_eq!(stack.len(), 2);
    assert_eq!(stack.top().map(|entry| entry.id), Some(OverlayId::Help));
    assert_eq!(stack.focus_scope(), FocusScope::Overlay(OverlayId::Help));

    // Raising an already-open overlay moves it to top without duplicating it.
    stack.raise_or_push(overlay_entry(OverlayId::Settings));
    assert_eq!(stack.len(), 2);
    assert_eq!(stack.top().map(|entry| entry.id), Some(OverlayId::Settings));
    assert_eq!(stack.focus_scope(), FocusScope::Overlay(OverlayId::Settings));

    assert!(stack.remove(OverlayId::Help));
    assert_eq!(stack.len(), 1);
    assert_eq!(stack.top().map(|entry| entry.id), Some(OverlayId::Settings));

    let popped = stack.pop();
    assert_eq!(popped.map(|entry| entry.id), Some(OverlayId::Settings));
    assert!(stack.is_empty());
    assert_eq!(stack.focus_scope(), base_scope);
    assert_ne!(stack.focus_scope(), FocusScope::Overlay(OverlayId::Settings));
}

#[test]
fn key_event_release_is_rejected_while_press_and_repeat_pass() {
    assert!(is_key_press(key_press(KeyCode::Char('a'), KeyModifiers::NONE)));
    assert!(is_key_press(key_repeat(KeyCode::Char('a'), KeyModifiers::NONE)));
    assert!(!is_key_press(key_release(KeyCode::Char('a'), KeyModifiers::NONE)));

    assert!(is_key_press(key_press(KeyCode::Enter, KeyModifiers::NONE)));
    assert!(is_key_press(key_repeat(KeyCode::Esc, KeyModifiers::NONE)));
    assert!(!is_key_press(key_release(KeyCode::Enter, KeyModifiers::NONE)));
}

#[test]
fn hub_frame_clears_background_keeps_footer_intact_and_switches_tabs() {
    let palette = ThemePalette::for_preset_with_no_color(ThemePreset::Titanium, true);
    let glyphs = ChromeGlyphs::for_mode(IconMode::Symbols);
    assert_eq!(UnicodeWidthStr::width(glyphs.check_on), 1);

    for (width, height) in [(40_u16, 12_u16), (80_u16, 24_u16)] {
        let mut harness = TuiTestHarness::new(width, height);
        let mut hitmap = HitMap::default();
        let tabs = vec![
            TabItem {
                label: "Appearance & Theme".to_owned(),
                short_label: "App".to_owned(),
                enabled: true,
                badge: None,
            },
            TabItem {
                label: "Plugins & Channels".to_owned(),
                short_label: "Plg".to_owned(),
                enabled: true,
                badge: None,
            },
            TabItem {
                label: "Access Control".to_owned(),
                short_label: "RBAC".to_owned(),
                enabled: true,
                badge: None,
            },
        ];
        let chips = vec![
            FooterChip {
                label: "Esc Close".to_owned(),
                active: false,
                danger: false,
            },
            FooterChip {
                label: "Tab Next".to_owned(),
                active: true,
                danger: false,
            },
        ];
        let mut tab_bar = TabBar::new(tabs);
        let config = HubFrameConfig {
            overlay_id: OverlayId::Settings,
            title: "Settings".to_owned(),
            max_width: 72,
            max_height: 20,
            width_pct: 90,
            height_pct: 85,
            tab_bar: Some(&tab_bar),
            rail_width: 0,
            inspector_height: 0,
        };
        let screen = harness.area();
        let expected_areas = compute_hub_areas(screen, &config);

        harness.draw_with(|area, buf| {
            fill_background_with_leak_marker(area, buf, "SidebarTextLeakCheck ");
            hitmap.clear();
            let areas = render_hub_frame(
                area,
                buf,
                &config,
                FooterMode::Chips(&chips),
                &palette,
                &mut hitmap,
            );
            assert_eq!(areas.footer, expected_areas.footer);
        });

        // No background sidebar text may bleed inside the cleared modal rect.
        harness.assert_text_absent_in_rect(expected_areas.outer, "SidebarTextLeakCheck");
        // Modal title text must not bleed outside the modal rect.
        harness.assert_no_bleed_outside(expected_areas.outer, "Settings");
        harness.assert_footer_intact(
            expected_areas.outer,
            expected_areas.footer.y,
            &["Esc Close", "Tab Next"],
        );

        // Locate tab 1 hit region in the rendered TabBar and switch tabs.
        let tab_area = expected_areas.tab_bar.expect("tab bar area present");
        let mut tab1_coord = None;
        for col in tab_area.x..tab_area.right() {
            if matches!(hitmap.hit_test(col, tab_area.y), Some(HitTarget::Tab(1))) {
                tab1_coord = Some((col, tab_area.y));
                break;
            }
        }
        let (col, row) = tab1_coord.expect("Tab 1 hit rect recorded in TabBar");
        if let Some(HitTarget::Tab(idx)) = hitmap.hit_test(col, row).cloned() {
            assert!(tab_bar.select(idx));
        }
        assert_eq!(tab_bar.active, 1);

        // Re-render standalone TabBar after switching and verify active tab text at viewport width.
        harness.draw_with(|area, buf| {
            hitmap.clear();
            let bar_rect = Rect::new(area.x, area.y, area.width, 1);
            render_tab_bar(bar_rect, buf, &tab_bar, &palette, &mut hitmap);
        });
        harness.assert_text_in_rect(
            Rect::new(0, 0, width, 1),
            if width == 40 { "Plg" } else { "Plugins" },
        );
    }
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

#[test]
fn key_release_events_are_ignored_in_tui_app_state() {
    let dir = TempDir::new_in(env!("CARGO_TARGET_TMPDIR")).expect("tempdir");
    let mut state = make_app_state(&dir);

    let actions = state.handle_key_event(key_release(KeyCode::Char('x'), KeyModifiers::NONE));
    assert!(actions.is_empty());
    assert!(state.composer.text.is_empty());

    // Enter release must not submit composer text.
    state.handle_key_event(key_press(KeyCode::Char('h'), KeyModifiers::NONE));
    let rel_enter = state.handle_key_event(key_release(KeyCode::Enter, KeyModifiers::NONE));
    assert!(rel_enter.is_empty());
    assert_eq!(state.composer.text, "h");

    // Release inside palette or settings must also be ignored.
    state.open_palette();
    state.handle_key_event(key_release(KeyCode::Char('q'), KeyModifiers::NONE));
    assert_eq!(
        state.ux.palette.as_ref().expect("palette open").query,
        ""
    );
}

#[test]
fn ctrl_v_inside_wizard_settings_palette_and_help_never_leaks_to_composer() {
    let dir = TempDir::new_in(env!("CARGO_TARGET_TMPDIR")).expect("tempdir");
    let mut state = make_app_state(&dir);
    assert!(!state.has_open_overlay());

    // 1. Wizard StepKey: Ctrl+V emits PasteFromClipboard (text-only path in runtime) and does not leak to composer.
    let mut wizard = SetupWizardState::new(&state.gateway_cfg, None, vec![]);
    wizard.step = WizardStep::StepKey;
    state.wizard = Some(wizard);
    assert!(state.has_open_overlay());
    let acts = state.handle_key_event(key_press(KeyCode::Char('v'), KeyModifiers::CONTROL));
    assert_eq!(acts, vec![AppAction::PasteFromClipboard]);
    assert!(state.composer.text.is_empty());
    state.wizard = None;

    // 2. Settings open: Ctrl+V triggers PasteFromClipboard without leaking to composer.
    state.settings.open_tab(SettingsTab::Appearance);
    assert!(state.has_open_overlay());
    let acts = state.handle_key_event(key_press(KeyCode::Char('v'), KeyModifiers::CONTROL));
    assert_eq!(acts, vec![AppAction::PasteFromClipboard]);
    assert!(state.composer.text.is_empty());
    state.settings.close();

    // 3. Palette open: Ctrl+V triggers PasteFromClipboard without leaking to composer.
    state.open_palette();
    assert!(state.has_open_overlay());
    let acts = state.handle_key_event(key_press(KeyCode::Char('v'), KeyModifiers::CONTROL));
    assert_eq!(acts, vec![AppAction::PasteFromClipboard]);
    assert!(state.composer.text.is_empty());
    state.ux.palette = None;

    // 4. Help open (no text input): Ctrl+V is swallowed and emits NO clipboard paste action.
    state.open_help();
    assert!(state.has_open_overlay());
    let acts = state.handle_key_event(key_press(KeyCode::Char('v'), KeyModifiers::CONTROL));
    assert!(acts.is_empty());
    assert!(state.composer.text.is_empty());
}

#[test]
fn wizard_and_why_isolate_mouse_wheel_from_chat_behind_them() {
    let dir = TempDir::new_in(env!("CARGO_TARGET_TMPDIR")).expect("tempdir");
    let mut state = make_app_state(&dir);
    state.chat_scroll = 15;
    state.auto_scroll = false;

    let viewport = Rect::new(0, 0, 100, 30);
    let mut effects = TuiEffects::new();
    let mut hit = HitRegions::default();

    // Wizard isolates wheel events from chat_scroll behind it.
    let mut wizard = SetupWizardState::new(&state.gateway_cfg, None, vec![]);
    wizard.step = WizardStep::Step1Provider;
    state.wizard = Some(wizard);
    let mut harness = TuiTestHarness::new(100, 30);
    harness.draw_with(|area, buf| {
        render_to_buffer(area, buf, &state, &mut effects, &mut hit, 16);
    });

    state.handle_mouse_event(mouse_scroll_up(50, 12), &hit, viewport);
    state.handle_mouse_event(mouse_scroll_down(50, 12), &hit, viewport);
    assert_eq!(state.chat_scroll, 15, "wizard must not leak wheel to chat");
    assert!(
        state.wizard.as_ref().expect("wizard").sel > 0,
        "wizard provider list scrolled down"
    );
    state.wizard = None;

    // /why isolates wheel events from chat_scroll behind it.
    let mut why = WhyOverlay::loading("event #1");
    why.lines = (0..30).map(|i| format!("reason line {i}")).collect();
    why.loading = false;
    state.conv.why = Some(why);
    harness.draw_with(|area, buf| {
        render_to_buffer(area, buf, &state, &mut effects, &mut hit, 16);
    });

    state.handle_mouse_event(mouse_scroll_down(50, 12), &hit, viewport);
    assert_eq!(state.chat_scroll, 15, "/why must not leak wheel to chat");
    assert!(
        state.conv.why.as_ref().expect("why").scroll > 0,
        "/why scrolled internally"
    );
    state.handle_mouse_event(mouse_scroll_up(50, 12), &hit, viewport);
    assert_eq!(state.conv.why.as_ref().expect("why").scroll, 0);
}
