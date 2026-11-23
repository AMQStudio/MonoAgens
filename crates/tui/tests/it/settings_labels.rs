//! Contract tests for Wave 2 Settings friendly labels (`Friendly Label (tech.id)`),
//! dual label+ID search, `TextInput`/RBAC paste & key isolation, mouse/wheel/horizontal-drift
//! routing, live gateway snapshot sync, and `40x12`/`80x24` footer & backdrop integrity.

use crossterm::event::{KeyCode, KeyModifiers};
use monoagens_gateway::{ResolvedPaths, load_config_file};
use monoagens_protocol::ActorId;
use monoagens_tui::core::hitmap::HitMap;
use monoagens_tui::core::theme::{ThemePalette, ThemePreset};
use monoagens_tui::settings_runtime::SettingsEvent;
use monoagens_tui::{
    HitRegions, LocalChatStore, SecretsUiSummary, SettingsTab, TuiAppState, TuiPreferences,
};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use serde_json::json;
use tempfile::TempDir;

use crate::common::{TuiTestHarness, key_press, mouse_click, mouse_scroll_down, mouse_scroll_left};

fn make_state_with(dir: &TempDir, config: &str) -> TuiAppState {
    let cfg_dir = dir.path().join("config");
    let data_dir = dir.path().join("data");
    std::fs::create_dir_all(&cfg_dir).expect("create config dir");
    std::fs::create_dir_all(&data_dir).expect("create data dir");
    let paths = ResolvedPaths {
        config_path: cfg_dir.join("monoagens.toml"),
        secrets_path: cfg_dir.join("secrets.toml"),
        data_dir,
    };
    std::fs::write(&paths.config_path, config).expect("write monoagens.toml");
    let cfg = load_config_file(&paths.config_path, &paths.secrets_path).expect("load config");
    TuiAppState::new(
        paths,
        cfg,
        TuiPreferences::default(),
        LocalChatStore::default(),
        SecretsUiSummary::default(),
        ActorId::new("per_owner"),
    )
}

fn make_state(dir: &TempDir) -> TuiAppState {
    make_state_with(dir, "version = 1\n")
}

fn fill_background(area: Rect, buf: &mut Buffer, marker: &str) {
    for y in area.y..area.bottom() {
        let mut x = area.x;
        while x < area.right() {
            let rem = usize::from(area.right() - x);
            let slice: String = marker.chars().take(rem).collect();
            let w = u16::try_from(slice.len()).unwrap_or(area.right() - x);
            if w == 0 {
                break;
            }
            buf.set_string(x, y, &slice, Style::default());
            x = x.saturating_add(w);
        }
    }
}

#[test]
fn all_valve_toggles_show_friendly_label_with_canonical_id() {
    let dir = TempDir::new_in(env!("CARGO_TARGET_TMPDIR")).expect("temp dir");
    let mut state = make_state(&dir);
    state.open_settings_at(SettingsTab::Valve);

    let rows = state.visible_settings_rows();
    let labels: Vec<&str> = rows.iter().map(|r| r.label.as_str()).collect();
    let expected = [
        "Allow Guests (valve.guest_access)",
        "Cedar Access Rules (cedar_abac)",
        "Privacy Boundary (line2_scope)",
        "Untrusted Input Shield (taint_blocking)",
        "Require Approval for Risky Actions (pending_approval)",
        "Daily Token Budget Guard (budget_guard)",
        "Auto-Wake Chain Brake (chain_brake)",
        "Quiet Hours (quiet_hours)",
        "Daily Proactive Cap (proactive_cap)",
    ];
    for needle in expected {
        assert!(
            labels.contains(&needle),
            "expected Valve row {needle:?} in {labels:?}"
        );
    }

    // Verify rendered buffer at 100x30 displays the Friendly Label (tech.id) copy verbatim.
    let palette = ThemePalette::for_preset_with_no_color(ThemePreset::Titanium, true);
    let mut harness = TuiTestHarness::new(100, 30);
    let mut hit = HitRegions::default();
    harness.draw_with(|area, buf| {
        hit.clear();
        state.render_settings_overlay(area, buf, &palette, &mut hit);
    });
    harness.assert_text_in_rect(hit.settings_area, "Allow Guests (valve.guest_access)");
    harness.assert_text_in_rect(hit.settings_area, "Cedar Access Rules (cedar_abac)");
    harness.assert_text_in_rect(hit.settings_area, "Privacy Boundary (line2_scope)");
}

#[test]
fn search_matches_both_friendly_label_and_canonical_id() {
    let dir = TempDir::new_in(env!("CARGO_TARGET_TMPDIR")).expect("temp dir");
    let mut state = make_state(&dir);
    state.open_settings_at(SettingsTab::Valve);

    // Search by friendly label fragment.
    assert!(state.handle_settings_paste("Allow Guests"));
    let by_label = state.visible_settings_rows();
    assert_eq!(by_label.len(), 1);
    assert_eq!(by_label[0].id, "valve.guest_access");
    assert_eq!(by_label[0].label, "Allow Guests (valve.guest_access)");

    // Clear and search by canonical ID fragment.
    state.handle_key_event(key_press(KeyCode::Esc, KeyModifiers::NONE));
    assert!(state.settings.search_query.is_empty());
    assert!(state.settings.search_input.is_empty());

    assert!(state.handle_settings_paste("valve.guest_access"));
    let by_id = state.visible_settings_rows();
    assert_eq!(by_id.len(), 1);
    assert_eq!(by_id[0].id, "valve.guest_access");
    assert_eq!(by_id[0].label, "Allow Guests (valve.guest_access)");
}

#[test]
fn typing_and_pasting_in_inline_editor_selector_or_rbac_never_leaks_to_search() {
    let dir = TempDir::new_in(env!("CARGO_TARGET_TMPDIR")).expect("temp dir");
    let mut state = make_state(&dir);

    // 1. Inline editor isolation (typing, Backspace, and paste with newlines/CSI debris).
    state.open_settings_row(SettingsTab::Valve, "valve.max_chain_wakes");
    state.handle_key_event(key_press(KeyCode::Enter, KeyModifiers::NONE));
    assert!(state.settings.editing.is_some());

    state.handle_key_event(key_press(KeyCode::Backspace, KeyModifiers::NONE));
    state.handle_key_event(key_press(KeyCode::Char('9'), KeyModifiers::NONE));
    assert!(state.handle_settings_paste("\x1b[200~8\r\n"));
    assert!(state.settings.search_query.is_empty());
    assert!(state.settings.search_input.is_empty());
    let edit = state.settings.editing.as_ref().expect("inline editor open");
    assert!(
        edit.buffer.ends_with("98"),
        "unexpected edit buffer: {:?}",
        edit.buffer
    );

    state.handle_key_event(key_press(KeyCode::Esc, KeyModifiers::NONE));
    assert!(state.settings.editing.is_none());

    // 2. Selector isolation (typing, Backspace, and paste go to selector search only).
    state.open_settings_row(SettingsTab::Model, "model.output_limit_param");
    state.handle_key_event(key_press(KeyCode::Enter, KeyModifiers::NONE));
    assert!(state.settings.selector.is_some());
    assert!(state.handle_settings_paste("max_completion\n"));
    state.handle_key_event(key_press(KeyCode::Backspace, KeyModifiers::NONE));
    assert!(state.settings.search_query.is_empty());
    assert!(state.settings.search_input.is_empty());
    let sel = state.settings.selector.as_ref().expect("selector open");
    assert_eq!(sel.search, "max_completio");
    state.handle_key_event(key_press(KeyCode::Esc, KeyModifiers::NONE));

    // 3. RBAC tab isolation (Char, Backspace, and paste never leak into search_query).
    state.open_settings_at(SettingsTab::Rbac);
    for ch in ['a', 'l', 'l', 'o', 'w'] {
        state.handle_key_event(key_press(KeyCode::Char(ch), KeyModifiers::NONE));
    }
    state.handle_key_event(key_press(KeyCode::Backspace, KeyModifiers::NONE));
    let _ = state.handle_settings_paste("chat.speak");
    assert!(
        state.settings.search_query.is_empty(),
        "RBAC typing/paste leaked into search_query: {:?}",
        state.settings.search_query
    );
    assert!(
        state.settings.search_input.is_empty(),
        "RBAC typing/paste leaked into search_input"
    );
}

#[test]
fn settings_overlay_renders_intact_footer_and_zero_bleed_at_40x12_and_80x24() {
    let dir = TempDir::new_in(env!("CARGO_TARGET_TMPDIR")).expect("temp dir");
    let mut state = make_state(&dir);
    state.open_settings_at(SettingsTab::Valve);
    let palette = ThemePalette::for_preset_with_no_color(ThemePreset::Titanium, true);

    // Narrow 40x12 viewport: full-screen modal, zero background bleed, pinned footer inside border.
    let mut narrow = TuiTestHarness::new(40, 12);
    let mut hit_narrow = HitRegions::default();
    narrow.draw_with(|area, buf| {
        fill_background(area, buf, "BleedMarker40x12 ");
        hit_narrow.clear();
        state.render_settings_overlay(area, buf, &palette, &mut hit_narrow);
    });
    narrow.assert_text_absent_in_rect(narrow.area(), "BleedMarker40x12");
    narrow.assert_text_in_rect(narrow.area(), "Settings");
    narrow.assert_footer_intact(narrow.area(), 10, &["←/→", "tab"]);

    // Standard 80x24 viewport: centered modal, zero background bleed inside modal, intact footer.
    let mut standard = TuiTestHarness::new(80, 24);
    let mut hit_std = HitRegions::default();
    standard.draw_with(|area, buf| {
        fill_background(area, buf, "BleedMarker80x24 ");
        hit_std.clear();
        state.render_settings_overlay(area, buf, &palette, &mut hit_std);
    });
    let modal = hit_std.settings_area;
    assert!(modal.width > 0 && modal.height > 0);
    standard.assert_text_absent_in_rect(modal, "BleedMarker80x24");
    standard.assert_no_bleed_outside(modal, "Allow Guests");
    standard.assert_footer_intact(modal, modal.bottom() - 2, &["←/→", "Esc"]);
}

#[test]
fn live_config_snapshot_and_mouse_scroll_and_tab_clicks_work_cleanly() {
    let dir = TempDir::new_in(env!("CARGO_TARGET_TMPDIR")).expect("temp dir");
    let mut state = make_state(&dir);
    state.gateway_link = "attached".to_owned();
    state.open_settings_at(SettingsTab::Valve);

    // Live ConfigLoaded snapshot updates Valve & RBAC without touching local disk.
    state.on_settings_event(SettingsEvent::ConfigLoaded(json!({
        "path": "/remote/monoagens.toml",
        "toml": "version = 1\n[valve]\nguest_access = true\nquiet_hours = true\n"
    })));
    assert!(state.gateway_cfg.valve.guest_access);
    assert!(state.settings.valve_toggles.quiet_hours);

    // Render to populate HitMap, verify horizontal scroll drift is ignored, wheel scrolls,
    // and clicking a Tab switches tabs.
    let palette = ThemePalette::for_preset_with_no_color(ThemePreset::Titanium, true);
    let mut harness = TuiTestHarness::new(90, 28);
    let mut hit = HitRegions::default();
    let mut hitmap = HitMap::default();
    harness.draw_with(|area, buf| {
        hit.clear();
        hitmap.clear();
        state.render_settings_overlay_with_hitmap(area, buf, &palette, &mut hit, &mut hitmap);
    });

    assert_eq!(state.settings.selected_index, 0);
    state.handle_settings_mouse(mouse_scroll_left(40, 10), &hitmap);
    assert_eq!(
        state.settings.selected_index, 0,
        "horizontal scroll drift must be ignored"
    );

    state.handle_settings_mouse(mouse_scroll_down(40, 10), &hitmap);
    assert_eq!(
        state.settings.selected_index, 1,
        "vertical wheel must advance selection"
    );

    // Locate the Runtime tab in the rendered tab row and click it.
    let tab_row = hit.settings_area.y + 2;
    let runtime_col = (hit.settings_area.x..hit.settings_area.right())
        .find(|&x| {
            matches!(
                hitmap.hit_test(x, tab_row),
                Some(monoagens_tui::core::hitmap::HitTarget::Tab(7))
            )
        })
        .expect("Runtime tab hit rect");
    let _ = state.handle_settings_mouse(mouse_click(runtime_col, tab_row), &hitmap);
    assert_eq!(state.settings.tab, SettingsTab::Runtime);
}

#[test]
fn every_config_key_has_a_settings_row_editor_or_documented_exclusion() {
    use monoagens_tui::settings::{RowsCtx, all_settings_rows};
    use monoagens_tui::settings_fields::{COVERAGE_EXCLUSIONS, EDITOR_COVERAGE, tui_keys};
    let dir = TempDir::new().expect("temp dir");
    let state = make_state(&dir);
    let providers: Vec<monoagens_tui::settings::ProviderSummary> = Vec::new();
    let bindings = Vec::new();
    let ctx = RowsCtx {
        prefs: &state.prefs,
        cfg: &state.gateway_cfg,
        paths: &state.paths,
        secrets: &state.secrets_summary,
        valve_toggles: &state.settings.valve_toggles,
        plugins: &state.plugins,
        providers: &providers,
        bindings: &bindings,
        model_name: &state.model_name,
    };
    let row_ids: Vec<String> = all_settings_rows(&ctx).into_iter().map(|(_, r)| r.id).collect();
    let mut keys: Vec<String> = monoagens_gateway::config_keys::CONFIG_KEYS
        .iter()
        .map(|k| (*k).to_owned())
        .collect();
    keys.extend(monoagens_gateway::config_keys::SECRET_KEYS.iter().map(|k| (*k).to_owned()));
    keys.extend(tui_keys());

    let mut missing = Vec::new();
    for key in &keys {
        let expanded: Vec<String> = if key.contains("<purpose>") {
            ["reason", "classify", "consolidate"]
                .iter()
                .map(|p| key.replace("<purpose>", p))
                .collect()
        } else {
            vec![key.clone()]
        };
        let covered = expanded.iter().all(|k| {
            row_ids.iter().any(|id| id == k || id == &k.replace("secrets.", ""))
                || row_ids.iter().any(|id| id == &format!("secrets.{k}"))
        }) || EDITOR_COVERAGE.iter().any(|(k, _)| k == key)
            || COVERAGE_EXCLUSIONS.iter().any(|(k, _)| k == key)
            || row_ids.iter().any(|id| key.starts_with(&format!("{id}.")));
        if !covered {
            missing.push(key.clone());
        }
    }
    assert!(missing.is_empty(), "config keys lacking row/editor/exclusion: {missing:?}");
}

#[test]
fn patch_to_file_preserves_comments_writes_bak_and_keybindings_help_and_conv_prefs_work() {
    let dir = TempDir::new().expect("temp dir");
    let mut state = make_state_with(&dir, "# user comment\nversion = 1\n");

    monoagens_gateway::apply_config_patch_to_file(
        &state.paths.config_path,
        &state.paths.secrets_path,
        &json!({ "valve": { "quiet_hours": true } }),
    )
    .expect("patch applied");
    let raw = std::fs::read_to_string(&state.paths.config_path).unwrap();
    assert!(raw.contains("# user comment") && raw.contains("quiet_hours = true"), "{raw}");
    assert!(state.paths.config_path.with_extension("toml.bak").exists());

    // Invalid range is rejected cleanly without corrupting file.
    assert!(
        monoagens_gateway::apply_config_patch_to_file(
            &state.paths.config_path,
            &state.paths.secrets_path,
            &json!({ "runtime": { "hook_timeout_ms": 999_999 } }),
        )
        .is_err()
    );

    // Conv preference toggle persists to tui.toml `[conv]`.
    state.open_settings_at(SettingsTab::Appearance);
    let conv_idx = state
        .visible_settings_rows()
        .iter()
        .position(|r| r.id == "tui.conv.live_draft_preview")
        .expect("conv row");
    state.settings.selected_index = conv_idx;
    state.handle_key_event(key_press(KeyCode::Char(' '), KeyModifiers::NONE));
    assert!(!state.prefs.conv.live_draft_preview);

    // Ctrl+K command palette jumps to a setting.
    state.settings.close();
    state.handle_key_event(key_press(KeyCode::Char('k'), KeyModifiers::CONTROL));
    assert!(state.ux.palette.is_some());
    for c in "Theme".chars() {
        state.handle_key_event(key_press(KeyCode::Char(c), KeyModifiers::NONE));
    }
    state.handle_key_event(key_press(KeyCode::Enter, KeyModifiers::NONE));
    assert!(state.settings.open);
}
