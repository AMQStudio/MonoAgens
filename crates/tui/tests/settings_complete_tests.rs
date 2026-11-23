//! `tui-settings-complete`: scrolling slash menu, typed settings editors, config coverage,
//! keybindings / help / palette, wizard, RBAC editor, plugin management and pairing.


use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use monoagens_gateway::{GatewayConfig, PluginKind, ResolvedPaths};
use monoagens_protocol::ActorId;
use monoagens_tui::settings::{PluginUiEntry, PluginUiState, RowsCtx, all_settings_rows};
use monoagens_tui::settings_fields::{COVERAGE_EXCLUSIONS, EDITOR_COVERAGE, tui_keys};
use monoagens_tui::{
    AppAction, HitRegions, LocalChatStore, SecretsUiSummary, SettingsTab, TuiAppState, TuiEffects, TuiPreferences,
    render_to_buffer,
};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use tempfile::TempDir;
use unicode_width::UnicodeWidthStr;

fn temp_dir() -> TempDir {
    TempDir::new_in(env!("CARGO_TARGET_TMPDIR")).expect("temp dir on disk")
}

fn make_state_with(dir: &TempDir, config: &str) -> TuiAppState {
    let cfg_dir = dir.path().join("config");
    let data_dir = dir.path().join("data");
    std::fs::create_dir_all(&cfg_dir).unwrap();
    std::fs::create_dir_all(&data_dir).unwrap();
    let paths = ResolvedPaths {
        config_path: cfg_dir.join("monoagens.toml"),
        secrets_path: cfg_dir.join("secrets.toml"),
        data_dir,
    };
    std::fs::write(&paths.config_path, config).unwrap();
    let cfg = monoagens_gateway::load_config_file(&paths.config_path, &paths.secrets_path).expect("config");
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

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn press(state: &mut TuiAppState, code: KeyCode) -> Vec<AppAction> {
    state.handle_key_event(key(code))
}

fn type_text(state: &mut TuiAppState, text: &str) {
    for c in text.chars() {
        state.handle_key_event(key(KeyCode::Char(c)));
    }
}

fn render_hit(w: u16, h: u16, state: &TuiAppState) -> (Buffer, HitRegions) {
    let mut effects = TuiEffects::new();
    let mut hit = HitRegions::default();
    let mut term = Terminal::new(TestBackend::new(w, h)).expect("terminal");
    term.draw(|f| render_to_buffer(f.area(), f.buffer_mut(), state, &mut effects, &mut hit, 16))
        .expect("draw");
    (term.backend().buffer().clone(), hit)
}

fn text_of(buf: &Buffer) -> String {
    let mut out = String::new();
    for y in 0..buf.area.height {
        let mut line = String::new();
        let mut skip = 0usize;
        for x in 0..buf.area.width {
            if skip > 0 {
                skip -= 1;
                continue;
            }
            let sym = buf.cell((x, y)).expect("cell").symbol();
            skip = sym.width().saturating_sub(1);
            line.push_str(sym);
        }
        out.push_str(line.trim_end());
        out.push('\n');
    }
    out
}

fn render_text(w: u16, h: u16, state: &TuiAppState) -> String {
    text_of(&render_hit(w, h, state).0)
}

fn open_row(state: &mut TuiAppState, tab: SettingsTab, id: &str) {
    state.open_settings_row(tab, id);
    let rows = state.visible_settings_rows();
    assert_eq!(rows.get(state.settings.selected_index).map(|r| r.id.as_str()), Some(id), "row {id} selected");
}

fn read_config(state: &TuiAppState) -> GatewayConfig {
    monoagens_gateway::load_config_file(&state.paths.config_path, &state.paths.secrets_path).expect("config parses")
}

// ---- 1. slash menu -------------------------------------------------------------------------

#[test]
fn slash_menu_select_last_command_scrolls_and_stays_open() {
    let dir = temp_dir();
    let mut state = make_state(&dir);
    type_text(&mut state, "/");
    let total = state.autocomplete.len();
    assert!(total > 10, "more commands than menu rows: {total}");
    for _ in 0..total - 1 {
        press(&mut state, KeyCode::Down);
    }
    assert_eq!(state.autocomplete_idx, total - 1);
    assert!(state.slash_menu_open(), "still open on the last command");
    let last = state.autocomplete[total - 1].name.clone();
    let (buf, hit) = render_hit(80, 24, &state);
    let text = text_of(&buf);
    assert!(text.contains("❯"), "selection marker:\n{text}");
    assert!(text.contains(&format!("/{last}")), "last command visible:\n{text}");
    assert_eq!(hit.autocomplete_visible, 10, "at most 10 rows");
    assert_eq!(hit.autocomplete_offset, total - 10, "scrolled to the end");
    // The scrollbar thumb is at the bottom of the menu's right edge.
    let area = hit.autocomplete_area;
    let x = area.x + area.width - 1;
    let bottom = buf.cell((x, area.y + area.height - 1)).unwrap().symbol().to_owned();
    let top = buf.cell((x, area.y)).unwrap().symbol().to_owned();
    assert_eq!(bottom, "┃", "thumb at the bottom");
    assert_ne!(top, "┃", "thumb left the top");
}

#[test]
fn slash_menu_sits_above_the_composer_and_wraps_pages_and_completes() {
    let dir = temp_dir();
    let mut state = make_state(&dir);
    type_text(&mut state, "/");
    let total = state.autocomplete.len();
    press(&mut state, KeyCode::Up);
    assert_eq!(state.autocomplete_idx, total - 1, "Up wraps to the last");
    press(&mut state, KeyCode::Down);
    assert_eq!(state.autocomplete_idx, 0, "Down wraps to the first");
    press(&mut state, KeyCode::PageDown);
    assert_eq!(state.autocomplete_idx, 10);
    press(&mut state, KeyCode::PageUp);
    assert_eq!(state.autocomplete_idx, 0);

    let (buf, hit) = render_hit(100, 30, &state);
    let text = text_of(&buf);
    let menu_bottom = hit.autocomplete_area.y + hit.autocomplete_area.height;
    let composer_y = text
        .lines()
        .position(|l| l.contains("esc to cancel") || l.contains("esc to clear"))
        .expect("composer hint line") as u16;
    assert!(menu_bottom <= composer_y, "menu ({menu_bottom}) above composer hint ({composer_y})");

    // Typing narrows the list; Tab completes and closes the menu (space typed).
    type_text(&mut state, "se");
    assert!(state.slash_menu_open());
    press(&mut state, KeyCode::Tab);
    assert!(state.composer.text.starts_with("/settings "), "{:?}", state.composer.text);
    assert!(!state.slash_menu_open(), "closed once a space follows the name");
    // A space typed by hand closes it too; backspacing the space reopens it.
    press(&mut state, KeyCode::Backspace);
    assert!(state.slash_menu_open());
}

#[test]
fn slash_menu_wheel_and_click() {
    let dir = temp_dir();
    let mut state = make_state(&dir);
    type_text(&mut state, "/");
    let (_, hit) = render_hit(100, 30, &state);
    let a = hit.autocomplete_area;
    let wheel = MouseEvent {
        kind: MouseEventKind::ScrollDown,
        column: a.x + 2,
        row: a.y + 1,
        modifiers: KeyModifiers::NONE,
    };
    state.handle_mouse_event(wheel, &hit, Rect::new(0, 0, 100, 30));
    assert_eq!(state.autocomplete_idx, 1, "wheel moves the selection");
    let (rect, idx) = hit.autocomplete_rows[3];
    let click = MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: rect.x + 3,
        row: rect.y,
        modifiers: KeyModifiers::NONE,
    };
    state.handle_mouse_event(click, &hit, Rect::new(0, 0, 100, 30));
    let name = &state.autocomplete.get(idx).map(|c| c.name.clone());
    assert!(
        state.composer.text.starts_with('/') && name.as_ref().is_none_or(|n| state.composer.text.contains(n.as_str()) || state.composer.text.len() > 1),
        "click completed: {:?}",
        state.composer.text
    );
}

#[test]
fn slash_plugins_row_shows_live_running_total() {
    let dir = temp_dir();
    let mut state = make_state(&dir);
    state.plugins = (0..6)
        .map(|i| PluginUiEntry {
            id: format!("p{i}"),
            kind: PluginKind::Adapter,
            enabled: true,
            installed: true,
            state: if i < 4 { PluginUiState::Running } else { PluginUiState::Stopped },
            command: None,
            socket_path: None,
            config_path: dir.path().join("p.toml"),
            state_dir: dir.path().join("p"),
            dependencies: vec![],
            optional_dependencies: vec![],
        })
        .collect();
    type_text(&mut state, "/plugins");
    let text = render_text(100, 30, &state);
    assert!(text.contains("4/6 running"), "plugins row:\n{text}");
    state.composer.text.clear();
    type_text(&mut state, "/settings");
    assert!(render_text(100, 30, &state).contains("8 tabs"));
}

#[test]
fn snapshot_slash_menu_80x24() {
    let dir = temp_dir();
    let mut state = make_state(&dir);
    type_text(&mut state, "/");
    for _ in 0..12 {
        press(&mut state, KeyCode::Down);
    }
    insta::assert_snapshot!("slash_menu_80x24", render_text(80, 24, &state));
}

// ---- 2. typed settings ---------------------------------------------------------------------

#[test]
fn number_out_of_range_is_rejected_with_the_range_and_nothing_is_saved() {
    let dir = temp_dir();
    let mut state = make_state(&dir);
    open_row(&mut state, SettingsTab::Runtime, "runtime.hook_timeout_ms");
    press(&mut state, KeyCode::Enter);
    assert!(state.settings.editing.is_some());
    // Replace the prefilled value.
    for _ in 0..10 {
        press(&mut state, KeyCode::Backspace);
    }
    type_text(&mut state, "70000");
    press(&mut state, KeyCode::Enter);
    let toast = state.toast.clone().expect("toast");
    assert!(toast.is_error && toast.text.contains("1..=60000"), "{}", toast.text);
    let raw = std::fs::read_to_string(&state.paths.config_path).unwrap();
    assert!(!raw.contains("hook_timeout_ms"), "unchanged: {raw}");
    assert_eq!(read_config(&state).runtime.hook_timeout_ms, GatewayConfig::default().runtime.hook_timeout_ms);
}

#[test]
fn enum_rows_open_a_selector_and_output_limit_param_is_saved() {
    let dir = temp_dir();
    let mut state = make_state(&dir);
    open_row(&mut state, SettingsTab::Model, "model.output_limit_param");
    press(&mut state, KeyCode::Enter);
    assert!(state.settings.selector.is_some(), "Enter on an enum opens the selector");
    let text = render_text(110, 32, &state);
    assert!(text.contains("Type to search"), "{text}");
    assert!(text.contains("max_tokens") && text.contains("max_completion_tokens"), "{text}");
    type_text(&mut state, "comp");
    press(&mut state, KeyCode::Enter);
    assert!(state.settings.selector.is_none());
    assert_eq!(state.gateway_cfg.model.output_limit_param, "max_completion_tokens");
    let raw = std::fs::read_to_string(&state.paths.config_path).unwrap();
    assert!(raw.contains("output_limit_param = \"max_completion_tokens\""), "{raw}");
    // The list shows the new value.
    assert!(render_text(110, 32, &state).contains("max_completion_tokens"));
}

#[test]
fn selector_esc_returns_without_a_change() {
    let dir = temp_dir();
    let mut state = make_state(&dir);
    open_row(&mut state, SettingsTab::Appearance, "tui.theme");
    press(&mut state, KeyCode::Enter);
    press(&mut state, KeyCode::Down);
    press(&mut state, KeyCode::Esc);
    assert!(state.settings.selector.is_none() && state.settings.open);
    assert_eq!(state.prefs, TuiPreferences::default());
}

#[test]
fn valve_toggle_description_footer_and_write() {
    let dir = temp_dir();
    let mut state = make_state_with(&dir, "# keep me\nversion = 1\n");
    open_row(&mut state, SettingsTab::Valve, "valve.quiet_hours");
    let text = render_text(120, 36, &state);
    assert!(text.contains("quiet hours"), "description footer:\n{text}");
    press(&mut state, KeyCode::Enter);
    assert!(state.settings.valve_toggles.quiet_hours);
    let raw = std::fs::read_to_string(&state.paths.config_path).unwrap();
    assert!(raw.contains("# keep me") && raw.contains("quiet_hours = true"), "{raw}");
    assert!(dir.path().join("config/monoagens.toml.bak").exists());
}

#[test]
fn nested_escape_closes_edit_then_detail_then_overlay() {
    let dir = temp_dir();
    let mut state = make_state(&dir);
    state.plugins = vec![PluginUiEntry {
        id: "fake".to_owned(),
        kind: PluginKind::Adapter,
        enabled: true,
        installed: true,
        state: PluginUiState::Running,
        command: Some("x".into()),
        socket_path: None,
        config_path: dir.path().join("config/plugins/fake.toml"),
        state_dir: dir.path().join("data/plugins/fake"),
        dependencies: vec![],
        optional_dependencies: vec![],
    }];
    state.settings.open_tab(SettingsTab::Plugins);
    press(&mut state, KeyCode::Enter);
    assert!(state.settings.plugin_detail.is_some());
    // Select "[ + Add value ]" (last row) and start an edit.
    let rows = state.plugin_detail_rows();
    state.settings.plugin_detail.as_mut().unwrap().selected_row = rows.len() - 1;
    press(&mut state, KeyCode::Enter);
    assert!(state.settings.editing.is_some());
    press(&mut state, KeyCode::Esc);
    assert!(state.settings.editing.is_none() && state.settings.plugin_detail.is_some());
    press(&mut state, KeyCode::Esc);
    assert!(state.settings.plugin_detail.is_none() && state.settings.open);
    press(&mut state, KeyCode::Esc);
    assert!(!state.settings.open);
}

#[test]
fn settings_list_shows_a_scrollbar_that_follows_the_selection() {
    let dir = temp_dir();
    let mut state = make_state(&dir);
    state.settings.open_tab(SettingsTab::Appearance);
    let (buf, hit) = render_hit(100, 22, &state);
    let rows = state.visible_settings_rows().len();
    assert!(hit.settings_options.len() < rows, "more rows than the viewport");
    let first_opt = hit.settings_options.first().unwrap().0;
    let last_opt = hit.settings_options.last().unwrap().0;
    let x = last_opt.x + last_opt.width;
    let thumb_at = |buf: &Buffer| {
        (first_opt.y..=last_opt.y).position(|y| buf.cell((x, y)).unwrap().symbol() == "┃")
    };
    let first = thumb_at(&buf).expect("scrollbar thumb drawn");
    for _ in 0..rows {
        press(&mut state, KeyCode::Down);
    }
    let (buf2, _) = render_hit(100, 22, &state);
    let later = thumb_at(&buf2).expect("thumb");
    assert!(later > first, "thumb moved down: {first} -> {later}");
}

// ---- 3. coverage -----------------------------------------------------------------------------

#[test]
fn every_config_key_has_a_row_an_editor_or_a_documented_exclusion() {
    let dir = temp_dir();
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
    let mut missing = Vec::new();
    let mut keys: Vec<String> = monoagens_gateway::config_keys::CONFIG_KEYS.iter().map(|k| (*k).to_owned()).collect();
    keys.extend(monoagens_gateway::config_keys::SECRET_KEYS.iter().map(|k| (*k).to_owned()));
    keys.extend(tui_keys());
    for key in &keys {
        let expanded: Vec<String> = if key.contains("<purpose>") {
            ["reason", "classify", "consolidate"].iter().map(|p| key.replace("<purpose>", p)).collect()
        } else {
            vec![key.clone()]
        };
        let covered = expanded.iter().all(|k| {
            row_ids.iter().any(|id| id == k || id == &k.replace("secrets.", ""))
                || row_ids.iter().any(|id| id == &format!("secrets.{k}"))
        }) || EDITOR_COVERAGE.iter().any(|(k, _)| k == key)
            || COVERAGE_EXCLUSIONS.iter().any(|(k, _)| k == key)
            // Map rows cover their `<name>` children: model.extra_headers / model.prices.
            || row_ids.iter().any(|id| key.starts_with(&format!("{id}.")));
        if !covered {
            missing.push(key.clone());
        }
    }
    assert!(missing.is_empty(), "config keys without a settings row, editor or exclusion: {missing:?}");
    for (k, why) in COVERAGE_EXCLUSIONS {
        assert!(!why.trim().is_empty(), "exclusion '{k}' needs a reason");
    }
}

// ---- 4. keybindings, help, palette ---------------------------------------------------------

fn make_state_with_keys(dir: &TempDir, keys: &str) -> TuiAppState {
    std::fs::create_dir_all(dir.path().join("config")).unwrap();
    std::fs::write(dir.path().join("config/keybindings.toml"), keys).unwrap();
    make_state(dir)
}

fn ctrl(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
}

#[test]
fn keybindings_conflict_is_ignored_and_logged_naming_both_actions() {
    let dir = temp_dir();
    let state = make_state_with_keys(&dir, "\"ui.sidebar\" = \"ctrl+k\"\n\"nope\" = \"f9\"\n");
    assert_eq!(state.ux.keymap.key_of("ui.sidebar"), "ctrl+b");
    let w = state.ux.keymap.warnings.join("\n");
    assert!(w.contains("ui.sidebar") && w.contains("ui.palette"), "{w}");
    assert!(w.contains("nope"), "unknown id reported: {w}");
}

#[test]
fn help_overlay_lists_a_rebound_key_and_the_rebound_key_works() {
    let dir = temp_dir();
    let mut state = make_state_with_keys(&dir, "\"ui.palette\" = \"ctrl+p\"\n");
    press(&mut state, KeyCode::F(1));
    assert!(state.ux.help.is_some());
    let text = render_text(110, 34, &state);
    assert!(text.contains("Ctrl+P"), "help shows the new key:\n{text}");
    assert!(!text.contains("Ctrl+K"), "and not the old one:\n{text}");
    press(&mut state, KeyCode::Esc);
    assert!(state.ux.help.is_none());
    state.handle_key_event(ctrl('k'));
    assert!(state.ux.palette.is_none(), "old key no longer opens the palette");
    state.handle_key_event(ctrl('p'));
    assert!(state.ux.palette.is_some(), "rebound key opens the palette");
}

#[test]
fn question_mark_opens_help_only_on_an_empty_composer() {
    let dir = temp_dir();
    let mut state = make_state(&dir);
    type_text(&mut state, "hi ");
    press(&mut state, KeyCode::Char('?'));
    assert!(state.ux.help.is_none());
    assert!(state.composer.text.ends_with('?'));
    state.composer.text.clear();
    press(&mut state, KeyCode::Char('?'));
    assert!(state.ux.help.is_some());
}

#[test]
fn palette_jumps_to_a_setting() {
    let dir = temp_dir();
    let mut state = make_state(&dir);
    state.handle_key_event(ctrl('k'));
    assert!(state.ux.palette.is_some());
    type_text(&mut state, "quiet");
    let text = render_text(110, 34, &state);
    assert!(text.contains("quiet_hours"), "{text}");
    press(&mut state, KeyCode::Enter);
    assert!(state.ux.palette.is_none());
    assert!(state.settings.open && state.settings.tab == SettingsTab::Valve);
    let rows = state.visible_settings_rows();
    assert_eq!(rows[state.settings.selected_index].id, "valve.quiet_hours");
}

#[test]
fn key_bindings_can_be_edited_from_settings() {
    let dir = temp_dir();
    let mut state = make_state(&dir);
    open_row(&mut state, SettingsTab::Appearance, "keys.edit");
    press(&mut state, KeyCode::Enter);
    let ed = state.settings.items.as_mut().expect("map editor");
    let at = ed.items.iter().position(|i| i.key.as_deref() == Some("ui.palette")).unwrap();
    ed.sel = at;
    press(&mut state, KeyCode::Enter); // edit key name
    press(&mut state, KeyCode::Enter); // keep it, go to the value
    for _ in 0..10 {
        press(&mut state, KeyCode::Backspace);
    }
    type_text(&mut state, "ctrl+y");
    press(&mut state, KeyCode::Enter);
    assert_eq!(state.ux.keymap.key_of("ui.palette"), "ctrl+y");
    let raw = std::fs::read_to_string(dir.path().join("config/keybindings.toml")).unwrap();
    assert!(raw.contains("ui.palette") && raw.contains("ctrl+y"), "{raw}");
}

// ---- 5. RBAC editor -----------------------------------------------------------------------

fn select_row(state: &mut TuiAppState, id: &str) {
    let rows = state.visible_settings_rows();
    let at = rows.iter().position(|r| r.id == id).unwrap_or_else(|| {
        panic!("row {id} not in {:?}", rows.iter().map(|r| r.id.clone()).collect::<Vec<_>>())
    });
    state.settings.selected_index = at;
}

fn create_role(state: &mut TuiAppState, name: &str) {
    state.open_settings_row(SettingsTab::Rbac, "rbac.new_role");
    press(state, KeyCode::Enter);
    type_text(state, name);
    press(state, KeyCode::Enter);
    assert!(state.settings.rbac_page.is_some(), "role page open");
}

#[test]
fn create_a_role_with_base_and_allow_and_the_toml_parses_back() {
    let dir = temp_dir();
    let mut state = make_state(&dir);
    create_role(&mut state, "family");
    select_row(&mut state, "rbac.role.family.base");
    press(&mut state, KeyCode::Enter);
    type_text(&mut state, "trusted");
    press(&mut state, KeyCode::Enter);
    select_row(&mut state, "rbac.role.family.allow");
    press(&mut state, KeyCode::Enter);
    press(&mut state, KeyCode::Char('a'));
    type_text(&mut state, "chat.speak");
    press(&mut state, KeyCode::Enter);
    let raw = std::fs::read_to_string(&state.paths.config_path).unwrap();
    assert!(raw.contains("[roles.family]") && raw.contains("chat.speak"), "{raw}");
    let rbac = read_config(&state).rbac.expect("[roles] created on first edit");
    let role = &rbac.roles["family"];
    assert_eq!(role.base.as_str(), "trusted");
    assert_eq!(role.allow, vec!["chat.speak".to_owned()]);
    // The same tables round-trip through the valve's own type.
    let json = serde_json::to_value(&rbac).unwrap();
    let back: monoagens_valve::rbac::RbacConfig = serde_json::from_value(json).unwrap();
    assert_eq!(back, rbac);
}

#[test]
fn deleting_the_fallback_role_is_refused_with_the_reason() {
    let dir = temp_dir();
    let mut state = make_state(&dir);
    create_role(&mut state, "guest-plus");
    press(&mut state, KeyCode::Esc); // back to the list
    open_row(&mut state, SettingsTab::Rbac, "rbac.fallback");
    press(&mut state, KeyCode::Enter);
    type_text(&mut state, "guest-plus");
    press(&mut state, KeyCode::Enter);
    assert_eq!(read_config(&state).rbac.unwrap().fallback_role.as_deref(), Some("guest-plus"));
    select_row(&mut state, "rbac.role.guest-plus");
    press(&mut state, KeyCode::Enter);
    select_row(&mut state, "rbac.role.guest-plus.delete");
    press(&mut state, KeyCode::Enter);
    assert!(state.settings.confirm.is_none(), "no confirmation for a refused delete");
    let toast = state.toast.clone().unwrap();
    assert!(toast.is_error && toast.text.contains("fallback role"), "{}", toast.text);
    assert!(read_config(&state).rbac.unwrap().roles.contains_key("guest-plus"), "role remains");
}

#[test]
fn allow_picker_lists_capabilities_of_connected_bodies() {
    let dir = temp_dir();
    let mut state = make_state(&dir);
    state.settings.hints.capabilities = vec!["chat.speak".to_owned(), "tool.shell".to_owned()];
    create_role(&mut state, "family");
    select_row(&mut state, "rbac.role.family.allow");
    press(&mut state, KeyCode::Enter);
    press(&mut state, KeyCode::Char('p'));
    assert!(state.settings.selector.is_some());
    assert!(render_text(110, 34, &state).contains("chat.speak"));
    type_text(&mut state, "chat.sp");
    press(&mut state, KeyCode::Enter);
    assert_eq!(read_config(&state).rbac.unwrap().roles["family"].allow, vec!["chat.speak".to_owned()]);
}

#[test]
fn actors_places_and_permission_bases() {
    let dir = temp_dir();
    let mut state = make_state(&dir);
    create_role(&mut state, "family");
    press(&mut state, KeyCode::Esc);
    // Assign a role to an actor seen in the event log.
    state.settings.hints.actors = vec!["per_friend".to_owned()];
    open_row(&mut state, SettingsTab::Rbac, "rbac.new_actor");
    press(&mut state, KeyCode::Enter);
    type_text(&mut state, "per_f");
    press(&mut state, KeyCode::Enter);
    select_row(&mut state, "rbac.actor.per_friend.role.family");
    press(&mut state, KeyCode::Char(' '));
    let cfg = read_config(&state);
    assert_eq!(cfg.rbac.as_ref().unwrap().actor_roles[&ActorId::new("per_friend")], vec!["family".to_owned()]);
    press(&mut state, KeyCode::Esc);
    // A place rule.
    open_row(&mut state, SettingsTab::Rbac, "rbac.new_place");
    press(&mut state, KeyCode::Enter);
    type_text(&mut state, "telegram:1");
    press(&mut state, KeyCode::Enter);
    select_row(&mut state, "rbac.place.telegram:1.deny");
    press(&mut state, KeyCode::Enter);
    press(&mut state, KeyCode::Char('a'));
    type_text(&mut state, "tool.*");
    press(&mut state, KeyCode::Enter);
    let cfg = read_config(&state);
    let rule = &cfg.rbac.as_ref().unwrap().places[&monoagens_protocol::PlaceId::new("telegram:1")];
    assert_eq!(rule.deny, vec!["tool.*".to_owned()]);
    press(&mut state, KeyCode::Esc);
    press(&mut state, KeyCode::Esc);
    // Owner-only permission base.
    open_row(&mut state, SettingsTab::Rbac, "rbac.new_base");
    press(&mut state, KeyCode::Enter);
    type_text(&mut state, "tool.shell");
    press(&mut state, KeyCode::Enter);
    assert_eq!(read_config(&state).rbac.unwrap().permission_bases["tool.shell"], None, "owner-only");
    select_row(&mut state, "rbac.base.tool.shell.tier");
    press(&mut state, KeyCode::Enter);
    type_text(&mut state, "member");
    press(&mut state, KeyCode::Enter);
    assert_eq!(
        read_config(&state).rbac.unwrap().permission_bases["tool.shell"],
        Some(monoagens_valve::rbac::RoleTier::Member)
    );
}

// ---- 6. plugin management -----------------------------------------------------------------

const TELEGRAM_CFG: &str = "version = 1\n[plugins.telegram]\nenabled = true\nkind = \"adapter\"\ntransport = \"stdio\"\ncommand = \"/bin/true\"\nargs = [\"--token-env\", \"TELEGRAM_BOT_TOKEN\"]\n";

fn detail_row(state: &mut TuiAppState, id: &str) -> usize {
    let rows = state.plugin_detail_rows();
    let at = rows.iter().position(|r| r.id == id).unwrap_or_else(|| {
        panic!("detail row {id} not in {:?}", rows.iter().map(|r| r.id.clone()).collect::<Vec<_>>())
    });
    state.settings.plugin_detail.as_mut().unwrap().selected_row = at;
    at
}

fn push_log(state: &mut TuiAppState, source: &str, message: &str) {
    state.log.push(monoagens_gateway::LogEntry {
        ts_ms: 1,
        level: monoagens_gateway::RunLogLevel::Error,
        source: source.to_owned(),
        message: message.to_owned(),
    });
}

#[test]
fn missing_secret_warning_stderr_tail_and_set_action() {
    let dir = temp_dir();
    let mut state = make_state_with(&dir, TELEGRAM_CFG);
    state.set_plugin_state("telegram", PluginUiState::Failed("TELEGRAM_BOT_TOKEN is not set".into()));
    for i in 0..7 {
        push_log(&mut state, "plugin:telegram", &format!("stderr line {i}"));
    }
    push_log(&mut state, "plugin:other", "not mine");
    state.open_plugin_detail("telegram");
    let rows = state.plugin_detail_rows();
    let status = rows.iter().find(|r| r.id == "pd:status").unwrap();
    assert!(matches!(&status.value, monoagens_tui::settings::SettingValueKind::ReadOnly(t) if t.contains("TELEGRAM_BOT_TOKEN is not set")));
    assert_eq!(rows.iter().filter(|r| r.id.starts_with("pd:err:")).count(), 5, "last 5 stderr lines");
    assert!(rows.iter().any(|r| r.id == "pd:err:4" && matches!(&r.value, monoagens_tui::settings::SettingValueKind::ReadOnly(t) if t == "stderr line 6")));
    assert!(rows.iter().all(|r| !matches!(&r.value, monoagens_tui::settings::SettingValueKind::ReadOnly(t) if t == "not mine")));
    let warn = rows.iter().find(|r| r.id == "pd:secret:TELEGRAM_BOT_TOKEN").expect("secret row");
    assert!(warn.label.contains("Missing") && warn.label.contains("TELEGRAM_BOT_TOKEN"));
    state.settings.open = true;
    let text = render_text(120, 36, &state);
    assert!(text.contains("TELEGRAM_BOT_TOKEN") && text.contains("NOT SET"), "{text}");
    // The set action writes a masked value to secrets.toml [env] and clears the warning.
    let at = detail_row(&mut state, "pd:secret:TELEGRAM_BOT_TOKEN");
    state.activate_plugin_detail_row(at);
    assert!(state.settings.editing.as_ref().is_some_and(|e| e.masked));
    type_text(&mut state, "123:abc");
    press(&mut state, KeyCode::Enter);
    let secrets = std::fs::read_to_string(&state.paths.secrets_path).unwrap();
    assert!(secrets.contains("TELEGRAM_BOT_TOKEN") && secrets.contains("123:abc"), "{secrets}");
    let row = state.plugin_detail_rows().into_iter().find(|r| r.id == "pd:secret:TELEGRAM_BOT_TOKEN").unwrap();
    assert!(!row.label.contains("Missing"), "{}", row.label);
    assert!(!render_text(120, 36, &state).contains("123:abc"), "the secret is never drawn");
}

#[test]
fn restart_and_view_log_from_the_detail_page() {
    let dir = temp_dir();
    let mut state = make_state_with(&dir, TELEGRAM_CFG);
    state.set_plugin_state("telegram", PluginUiState::Running);
    state.open_plugin_detail("telegram");
    let at = detail_row(&mut state, "pd:restart");
    let actions = state.activate_plugin_detail_row(at);
    assert_eq!(
        actions,
        vec![AppAction::Settings(monoagens_tui::settings_runtime::SettingsAction::RestartPlugin("telegram".into()))]
    );
    assert_eq!(state.plugins[0].state, PluginUiState::Starting, "passes through starting");
    state.set_plugin_state("telegram", PluginUiState::Running);
    let at = detail_row(&mut state, "pd:log");
    state.activate_plugin_detail_row(at);
    assert!(state.log.open);
    assert_eq!(state.log.source.as_deref(), Some("plugin:telegram"));
}

#[test]
fn array_value_gets_a_new_item_and_comments_survive() {
    let dir = temp_dir();
    let mut state = make_state_with(&dir, TELEGRAM_CFG);
    let plugin_toml = dir.path().join("config/plugins/telegram.toml");
    std::fs::create_dir_all(plugin_toml.parent().unwrap()).unwrap();
    std::fs::write(&plugin_toml, "# keep this comment\nallowed_chats = [\"a\"]\nretries = 3\n").unwrap();
    state.settings.open_tab(SettingsTab::Plugins);
    state.open_plugin_detail("telegram");
    let rows = state.plugin_detail_rows();
    assert!(matches!(&rows.iter().find(|r| r.id == "pv:retries").unwrap().value, monoagens_tui::settings::SettingValueKind::Number(_)));
    let at = detail_row(&mut state, "pv:allowed_chats");
    state.activate_plugin_detail_row(at);
    assert!(state.settings.items.is_some(), "array opens a list editor");
    press(&mut state, KeyCode::Char('a'));
    type_text(&mut state, "b");
    press(&mut state, KeyCode::Enter);
    let raw = std::fs::read_to_string(&plugin_toml).unwrap();
    assert!(raw.contains("# keep this comment"), "{raw}");
    assert!(raw.contains("\"a\"") && raw.contains("\"b\""), "{raw}");
}

#[test]
fn enable_on_startup_goes_through_config_apply_patch() {
    let dir = temp_dir();
    let mut state = make_state_with(&dir, TELEGRAM_CFG);
    state.open_plugin_detail("telegram");
    let at = detail_row(&mut state, "pd:enabled");
    state.activate_plugin_detail_row(at);
    assert!(!read_config(&state).plugins[0].enabled);
    assert!(dir.path().join("config/monoagens.toml.bak").exists());
}

#[test]
fn install_flow_shows_expected_path_and_build_hint_when_binary_is_missing() {
    let dir = temp_dir();
    let mut state = make_state(&dir);
    if let Some(p) = monoagens_tui::config::find_builtin_preset("voice").and_then(monoagens_tui::config::discover_builtin_binary) {
        eprintln!("body-voice exists at {}; skipping the missing-binary scenario", p.display());
        return;
    }
    state.settings.open_tab(SettingsTab::Plugins);
    select_row(&mut state, "plugin.voice");
    press(&mut state, KeyCode::Char('i'));
    assert!(state.settings.install.is_some(), "guided flow, not a bare install");
    let text = render_text(120, 36, &state);
    assert!(text.contains("body-voice") && text.contains("NOT FOUND") && text.contains("cargo build"), "{text}");
    press(&mut state, KeyCode::Enter);
    assert!(read_config(&state).plugins.is_empty(), "nothing enabled without the binary");
}

#[test]
fn remove_asks_first_and_declining_keeps_the_plugin() {
    let dir = temp_dir();
    let mut state = make_state_with(&dir, TELEGRAM_CFG);
    state.settings.open_tab(SettingsTab::Plugins);
    select_row(&mut state, "plugin.telegram");
    press(&mut state, KeyCode::Char('d'));
    assert!(state.settings.confirm.is_some());
    assert!(render_text(120, 36, &state).contains("Remove plugin 'telegram'"));
    press(&mut state, KeyCode::Char('n'));
    assert!(state.settings.confirm.is_none());
    assert_eq!(read_config(&state).plugins.len(), 1, "declined: plugin remains");
    press(&mut state, KeyCode::Char('d'));
    press(&mut state, KeyCode::Char('y'));
    assert!(read_config(&state).plugins.is_empty());
}

// ---- pairing -------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pairing_page_shows_the_code_then_paired_without_leaving_the_tui() {
    use std::os::unix::fs::PermissionsExt;
    let dir = temp_dir();
    let script = dir.path().join("body-whatsapp");
    std::fs::write(
        &script,
        "#!/bin/sh\necho '=== WhatsApp Pair Code (valid for 120s) ===' >&2\necho '  >>> ABCD-1234 <<<' >&2\nsleep 0.5\necho '✅ Device linked! Completing handshake' >&2\nexit 0\n",
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    let cfg = format!(
        "version = 1\n[plugins.whatsapp]\nenabled = true\nkind = \"adapter\"\ntransport = \"stdio\"\ncommand = \"{}\"\nargs = [\"--live\"]\n",
        script.display()
    );
    let mut state = make_state_with(&dir, &cfg);
    state.settings.open_tab(SettingsTab::Plugins);
    state.open_plugin_detail("whatsapp");
    let at = detail_row(&mut state, "pd:pair");
    let actions = state.activate_plugin_detail_row(at);
    assert!(state.settings.pairing.is_some());
    assert!(!actions.iter().any(|a| matches!(a, AppAction::PairPlugin(_))), "no subprocess takeover of the terminal");
    let [AppAction::Settings(action)] = &actions[..] else { panic!("one settings action: {actions:?}") };

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let runner = tokio::spawn(monoagens_tui::settings_runtime::run_settings_action_without_client(action.clone(), tx));
    let mut saw_code = false;
    loop {
        let ev = tokio::time::timeout(Duration::from_secs(10), rx.recv())
            .await
            .expect("pairing output in time")
            .expect("channel open");
        let monoagens_tui::RuntimeEvent::Settings(ev) = ev else { continue };
        let done = matches!(&ev, monoagens_tui::settings_runtime::SettingsEvent::Pairing(monoagens_tui::pairing::PairingEvent::Exit(_)));
        state.on_settings_event(ev);
        if !saw_code && render_text(110, 36, &state).contains("ABCD-1234") {
            saw_code = true;
            assert!(render_text(110, 36, &state).contains("waiting"), "status waiting while the code shows");
        }
        if done {
            break;
        }
    }
    tokio::time::timeout(Duration::from_secs(10), runner).await.expect("runner ends").unwrap();
    assert!(saw_code, "the page showed the code");
    assert_eq!(state.settings.pairing.as_ref().unwrap().status, monoagens_tui::pairing::PairingStatus::Paired);
    assert!(render_text(110, 36, &state).contains("paired"));
}

// ---- first-run wizard ----------------------------------------------------------------------

fn fake_provider_server() -> String {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut s) = stream else { break };
            let mut buf = [0u8; 4096];
            let _ = s.read(&mut buf);
            let body = r#"{"data":[{"id":"llama-fake-1"},{"id":"llama-fake-2"}]}"#;
            let _ = write!(
                s,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });
    format!("http://{addr}/v1")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wizard_walks_provider_key_test_model_picker_without_an_endpoint_field() {
    let dir = temp_dir();
    let server = fake_provider_server();
    let mut state = make_state(&dir);
    state.gateway_cfg.model.provider_base_url = Some(server);
    state.wizard = Some(monoagens_tui::SetupWizardState::new(&state.gateway_cfg, None, vec!["voice".to_owned()]));
    // Step 1: pick groq from the catalog list.
    let text = render_text(100, 30, &state);
    assert!(text.contains("Provider") && text.contains("groq"), "{text}");
    type_text(&mut state, "groq");
    press(&mut state, KeyCode::Enter);
    // Step 2: key, no endpoint field.
    let text = render_text(100, 30, &state);
    assert!(text.contains("API key") && !text.to_lowercase().contains("endpoint"), "{text}");
    type_text(&mut state, "gsk-test-key-123");
    assert!(!render_text(100, 30, &state).contains("gsk-test-key-123"), "key is masked");
    let actions = press(&mut state, KeyCode::Enter);
    let [AppAction::Settings(action)] = &actions[..] else { panic!("test action: {actions:?}") };
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    tokio::spawn(monoagens_tui::settings_runtime::run_settings_action_without_client(action.clone(), tx));
    let ev = tokio::time::timeout(Duration::from_secs(10), rx.recv()).await.expect("test result in time").unwrap();
    let monoagens_tui::RuntimeEvent::Settings(ev) = ev else { panic!("settings event") };
    state.on_settings_event(ev);
    // Step 3: the discovered models in a picker with search.
    let text = render_text(100, 30, &state);
    assert!(text.contains("llama-fake-1") && text.contains("llama-fake-2"), "{text}");
    type_text(&mut state, "fake-2");
    press(&mut state, KeyCode::Enter);
    // Step 4: plugins, then finish.
    let actions = press(&mut state, KeyCode::Enter);
    assert!(actions.contains(&AppAction::CompleteSetupWizard));
    let cfg = read_config(&state);
    assert_eq!(cfg.model.catalog_provider.as_deref(), Some("groq"));
    assert_eq!(cfg.model.model, "llama-fake-2");
    let secrets = std::fs::read_to_string(&state.paths.secrets_path).unwrap();
    assert!(secrets.contains("[providers.groq]") && secrets.contains("gsk-test-key-123"), "{secrets}");
    assert!(!std::fs::read_to_string(&state.paths.config_path).unwrap().contains("gsk-test-key-123"));
}

#[test]
fn wizard_no_longer_offers_an_unnamed_custom_provider() {
    let dir = temp_dir();
    let mut state = make_state(&dir);
    state.wizard = Some(monoagens_tui::SetupWizardState::new(&state.gateway_cfg, None, vec![]));
    type_text(&mut state, "custom");
    let text = render_text(100, 30, &state);
    assert!(!text.contains("Custom (OpenAI-compatible)"), "{text}");
}

// ---- model / providers tabs reuse the /model and /provider screens -----------------------------

#[test]
fn model_and_providers_tabs_open_the_catalog_screens() {
    let dir = temp_dir();
    let mut state = make_state(&dir);
    open_row(&mut state, SettingsTab::Model, "model.model");
    let actions = press(&mut state, KeyCode::Enter);
    assert!(state.catalog.is_open() && state.catalog.screen == monoagens_tui::catalog_ui::Screen::Picker);
    assert!(actions.iter().any(|a| matches!(a, AppAction::Catalog(_))));
    press(&mut state, KeyCode::Esc);
    assert!(!state.catalog.is_open() && state.settings.open, "Esc returns to settings");
    state.catalog.apply(monoagens_tui::CatalogEvent::Providers(Ok(serde_json::json!({
        "current": {"provider": "groq", "model": "x"},
        "providers": [{"id": "groq", "label": "Groq", "keyless": false, "key_set": true, "key_source": "secrets", "discovery": true}]
    }))));
    open_row(&mut state, SettingsTab::Providers, "provider.groq");
    press(&mut state, KeyCode::Enter);
    assert_eq!(state.catalog.screen, monoagens_tui::catalog_ui::Screen::LoginList);
}

// ---- writes go through the gateway (attached TUI) ------------------------------------------

mod attached {
    use std::sync::Arc;

    use monoagens_core::{CoreConfig, MemStore, NoopDoors};
    use monoagens_gateway::{
        AdminServer, Clock, CueRecorder, DiagnosticSink, GatewayOptions, GatewayOwner, GatewayRuntime, GatewaySeams,
        ModelProviderKind, OwnerMode, OwnerParams, PluginEnvContext, PluginSupervisor, RestartPolicy, RunLog,
        SystemClock, acquire_data_dir_lock,
    };
    use monoagens_reason::ProviderSecrets;
    use monoagens_tui::client::GatewayClient;
    use monoagens_tui::settings_runtime::{SettingsEvent, run_settings_action};
    use monoagens_tui::{RuntimeEvent, SocketClient};

    use super::*;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn valve_toggle_and_secret_reach_the_daemon_through_the_admin_socket() {
        let dir = temp_dir();
        let mut state = make_state_with(&dir, "# daemon config\nversion = 1\n[model]\nprovider = \"scripted\"\nmodel = \"mock\"\n");
        let mut cfg = GatewayConfig::default();
        cfg.model.provider = ModelProviderKind::Scripted;
        cfg.model.model = "mock".to_owned();
        let secrets = ProviderSecrets::default();
        let rt = GatewayRuntime::start(
            cfg.clone(),
            &secrets,
            || Ok(MemStore::new()),
            CoreConfig::default(),
            NoopDoors,
            GatewayOptions {
                clock: Arc::new(SystemClock::new()),
                provider: None,
                diagnostics: DiagnosticSink::new(&secrets),
                cues: CueRecorder::new(),
                seams: GatewaySeams::default(),
            },
        )
        .await
        .expect("gateway");
        let handle = rt.handle();
        let clock: Arc<dyn Clock> = Arc::new(SystemClock::new());
        let log = RunLog::new(Arc::clone(&clock), handle.diagnostics().clone(), None);
        let supervisor = PluginSupervisor::new(
            handle.clone(),
            PluginEnvContext {
                config_dir: None,
                data_dir: Some(state.paths.data_dir.clone()),
                rbac_enabled: false,
                plugin_env: Default::default(),
            },
            clock,
            log.clone(),
            RestartPolicy::default(),
            vec![],
        );
        let owner = Arc::new(GatewayOwner::new(OwnerParams {
            handle: handle.clone(),
            supervisor,
            runlog: log,
            mode: OwnerMode::Serve,
            config: cfg,
            config_path: state.paths.config_path.clone(),
            secrets_path: state.paths.secrets_path.clone(),
            data_dir: state.paths.data_dir.clone(),
            owner_actor: ActorId::new("per_owner"),
        }));
        let lock = acquire_data_dir_lock(&state.paths.data_dir).expect("lock");
        let server = AdminServer::start(&lock, &state.paths.data_dir, owner).await.expect("admin socket");

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let client: Arc<dyn GatewayClient> = Arc::new(
            tokio::time::timeout(
                Duration::from_secs(10),
                SocketClient::connect(&state.paths.data_dir, ActorId::new("per_owner"), [0u8; 32], &secrets, tx.clone()),
            )
            .await
            .expect("connect in time")
            .expect("attached"),
        );
        state.gateway_link = "attached".to_owned();

        // The valve toggle becomes a config.apply action, never a local file write.
        open_row(&mut state, SettingsTab::Valve, "valve.quiet_hours");
        let actions = press(&mut state, KeyCode::Enter);
        let [AppAction::Settings(action)] = &actions[..] else { panic!("{actions:?}") };
        let raw_before = std::fs::read_to_string(&state.paths.config_path).unwrap();
        assert!(!raw_before.contains("quiet_hours"), "the TUI did not write the file itself");
        tokio::time::timeout(Duration::from_secs(10), run_settings_action(action.clone(), Arc::clone(&client), tx.clone()))
            .await
            .expect("apply in time");
        let done = loop {
            let ev = tokio::time::timeout(Duration::from_secs(10), rx.recv()).await.expect("event").expect("open");
            if let RuntimeEvent::Settings(e @ SettingsEvent::Done { .. }) = ev {
                break e;
            }
        };
        assert!(matches!(&done, SettingsEvent::Done { result: Ok(()), .. }), "{done:?}");
        state.on_settings_event(done);
        let raw = std::fs::read_to_string(&state.paths.config_path).unwrap();
        assert!(raw.contains("# daemon config") && raw.contains("quiet_hours = true"), "{raw}");
        assert!(dir.path().join("config/monoagens.toml.bak").exists(), ".bak written by the daemon");
        assert!(state.settings.valve_toggles.quiet_hours, "the list shows the saved value");

        // A secret is written without being echoed back.
        let actions = state.secret_action("env", "TELEGRAM_BOT_TOKEN", Some("tok-123".to_owned()), "saved".to_owned());
        let [AppAction::Settings(action)] = &actions[..] else { panic!("{actions:?}") };
        assert!(!format!("{action:?}").contains("tok-123"), "the value never prints");
        run_settings_action(action.clone(), Arc::clone(&client), tx.clone()).await;
        loop {
            let ev = tokio::time::timeout(Duration::from_secs(10), rx.recv()).await.expect("event").expect("open");
            if let RuntimeEvent::Settings(SettingsEvent::Done { result, .. }) = ev {
                assert_eq!(result, Ok(()));
                break;
            }
        }
        let secrets_raw = std::fs::read_to_string(&state.paths.secrets_path).unwrap();
        assert!(secrets_raw.contains("TELEGRAM_BOT_TOKEN") && secrets_raw.contains("tok-123"), "{secrets_raw}");
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&state.paths.secrets_path).unwrap().permissions().mode() & 0o777, 0o600);

        // Capability ids for the RBAC picker come over the same socket.
        let caps = client.gov_call("capabilities.list".to_owned(), serde_json::json!({})).await.expect("caps");
        assert!(caps["capabilities"].is_array());

        client.shutdown().await;
        server.shutdown();
        rt.shutdown().await;
    }
}

// ---- snapshots -----------------------------------------------------------------------------

#[test]
fn snapshot_settings_valve_tab_100x30() {
    let dir = temp_dir();
    let mut state = make_state(&dir);
    open_row(&mut state, SettingsTab::Valve, "valve.chain_brake");
    insta::assert_snapshot!("settings_valve_100x30", render_text(100, 30, &state));
}

#[test]
fn snapshot_selector_overlay_100x30() {
    let dir = temp_dir();
    let mut state = make_state(&dir);
    open_row(&mut state, SettingsTab::Model, "model.output_limit_param");
    press(&mut state, KeyCode::Enter);
    insta::assert_snapshot!("selector_100x30", render_text(100, 30, &state));
}

#[test]
fn snapshot_help_overlay_100x30() {
    let dir = temp_dir();
    let mut state = make_state(&dir);
    press(&mut state, KeyCode::F(1));
    insta::assert_snapshot!("help_100x30", render_text(100, 30, &state));
}

#[test]
fn snapshot_wizard_providers_100x30() {
    let dir = temp_dir();
    let mut state = make_state(&dir);
    state.wizard = Some(monoagens_tui::SetupWizardState::new(&state.gateway_cfg, None, vec!["voice".to_owned()]));
    insta::assert_snapshot!("wizard_providers_100x30", render_text(100, 30, &state));
}

#[test]
fn snapshot_rbac_role_page_100x30() {
    let dir = temp_dir();
    let mut state = make_state(&dir);
    create_role(&mut state, "family");
    insta::assert_snapshot!("rbac_role_page_100x30", render_text(100, 30, &state));
}

#[test]
fn patch_to_file_keeps_comments_writes_bak_and_rejects_invalid_values() {
    let dir = temp_dir();
    let path = dir.path().join("monoagens.toml");
    let secrets = dir.path().join("secrets.toml");
    std::fs::write(&path, "# hi\nversion = 1\n").unwrap();
    monoagens_gateway::apply_config_patch_to_file(&path, &secrets, &serde_json::json!({"valve": {"quiet_hours": true}}))
        .expect("applied");
    let raw = std::fs::read_to_string(&path).unwrap();
    assert!(raw.contains("# hi") && raw.contains("quiet_hours = true"), "{raw}");
    assert!(dir.path().join("monoagens.toml.bak").exists());
    let err = monoagens_gateway::apply_config_patch_to_file(
        &path,
        &secrets,
        &serde_json::json!({"runtime": {"hook_timeout_ms": 999_999}}),
    );
    assert!(err.is_err(), "out-of-range value is refused");
    assert!(!std::fs::read_to_string(&path).unwrap().contains("999999"), "file unchanged");
}

// ---- 5. merge of tui-conversation: [conv] rows, key ownership, help ---------------------------

#[test]
fn conv_preferences_have_settings_rows_and_write_tui_toml() {
    let dir = temp_dir();
    let mut state = make_state(&dir);
    open_row(&mut state, SettingsTab::Appearance, "tui.conv.live_draft_preview");
    assert!(state.prefs.conv.live_draft_preview);
    press(&mut state, KeyCode::Char(' '));
    assert!(!state.prefs.conv.live_draft_preview, "Space toggles the preview off");
    open_row(&mut state, SettingsTab::Appearance, "tui.conv.notifications");
    press(&mut state, KeyCode::Enter);
    assert!(state.settings.selector.is_some());
    type_text(&mut state, "bell");
    press(&mut state, KeyCode::Enter);
    assert_eq!(state.prefs.conv.notifications.label(), "bell");
    let raw = std::fs::read_to_string(state.paths.config_path.with_file_name("tui.toml")).unwrap_or_default();
    assert!(raw.contains("[conv]") && raw.contains("notifications = \"bell\""), "{raw}");
    assert!(tui_keys().contains(&"tui.conv.notifications".to_owned()), "coverage sees the [conv] keys");
}

#[test]
fn help_lists_the_conversation_shortcuts_and_esc_closes_it_without_interrupting() {
    let dir = temp_dir();
    let mut state = make_state(&dir);
    press(&mut state, KeyCode::F(1));
    let text = render_text(100, 40, &state);
    for needle in ["Interrupt", "Retry", "history", "$EDITOR", "Select a message", "what-happened"] {
        assert!(text.contains(needle), "help lists '{needle}':\n{text}");
    }
    state.thinking = true;
    let actions = press(&mut state, KeyCode::Esc);
    assert!(state.ux.help.is_none(), "Esc closes the help");
    assert!(actions.is_empty() && state.conv.esc_armed_ms.is_none(), "and does not arm the interrupt");
}
