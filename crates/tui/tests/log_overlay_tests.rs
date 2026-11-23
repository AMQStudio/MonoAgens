//! `/log` overlay, shared scroll helpers, late-reply routing, config-error startup and `/doctor`
//! (change `tui-foundation`, tasks 3.1, 3.2, 7.1, 7.2 and the late-reply fix).

use std::sync::Arc;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
use monoagens_gateway::{
    BodyTransportConfig, GatewayConfig, LogEntry, PluginConfig, PluginKind, ResolvedPaths,
    RunLogLevel,
};
use monoagens_protocol::ActorId;
use monoagens_tui::doctor::{DoctorStatus, DoctorInput, format_report, run_doctor};
use monoagens_tui::state::ReplyDisposition;
use monoagens_tui::turn::ManualClock;
use monoagens_tui::{
    AppAction, ChatTimelineItem, HitRegions, LocalChatStore, SecretsUiSummary, SettingsTab,
    TuiAppState, TuiEffects, TuiPreferences, render_scrollbar, render_to_buffer, scroll_window,
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

fn make_state(dir: &TempDir) -> TuiAppState {
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

fn key(c: KeyCode) -> KeyEvent {
    KeyEvent::new(c, KeyModifiers::NONE)
}

fn press(state: &mut TuiAppState, code: KeyCode) -> Vec<AppAction> {
    state.handle_key_event(key(code))
}

fn type_text(state: &mut TuiAppState, text: &str) {
    for ch in text.chars() {
        state.handle_key_event(key(KeyCode::Char(ch)));
    }
}

fn entry(n: u64, level: RunLogLevel, source: &str, msg: &str) -> LogEntry {
    LogEntry {
        ts_ms: 3_600_000 + n * 1_000,
        level,
        source: source.to_owned(),
        message: msg.to_owned(),
    }
}

fn render(w: u16, h: u16, state: &TuiAppState) -> Buffer {
    let mut effects = TuiEffects::new();
    let mut hit = HitRegions::default();
    let mut term = Terminal::new(TestBackend::new(w, h)).expect("terminal");
    term.draw(|f| render_to_buffer(f.area(), f.buffer_mut(), state, &mut effects, &mut hit, 16))
        .expect("draw");
    term.backend().buffer().clone()
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

/// A state with a fixed, mixed set of log entries and the overlay open.
fn log_state(dir: &TempDir, n: usize) -> TuiAppState {
    let mut state = make_state(dir);
    for i in 0..n as u64 {
        let (level, source, msg) = match i % 4 {
            0 => (RunLogLevel::Debug, "gateway", format!("debug detail {i}")),
            1 => (RunLogLevel::Info, "plugin:fake", format!("fake says line {i}")),
            2 => (RunLogLevel::Warn, "provider", format!("slow reply {i}")),
            _ => (RunLogLevel::Info, "turn", format!("turn note {i}")),
        };
        state.on_log_entry(entry(i, level, source, &msg));
    }
    state.toast = None;
    state.log.open();
    state
}

// ---- shared helpers -------------------------------------------------------------------------

#[test]
fn scroll_window_edge_cases() {
    // everything fits -> no scrolling
    assert_eq!(scroll_window(3, 5, 10, 8), 0);
    assert_eq!(scroll_window(0, 0, 0, 100), 0);
    // selection above the window pulls it up, below pushes it down
    assert_eq!(scroll_window(2, 10, 5, 100), 2);
    assert_eq!(scroll_window(20, 10, 5, 100), 16);
    // inside the window: unchanged
    assert_eq!(scroll_window(12, 10, 5, 100), 10);
    // never past the last full window
    assert_eq!(scroll_window(99, 0, 5, 100), 95);
    assert_eq!(scroll_window(500, 0, 5, 100), 95);
    assert_eq!(scroll_window(0, 500, 5, 100), 0);
}

#[test]
fn scrollbar_shows_a_thumb_only_when_content_overflows() {
    let palette = monoagens_tui::ThemePalette::for_preset(monoagens_tui::ThemePreset::Titanium);
    let area = Rect::new(0, 0, 1, 10);
    let mut fits = Buffer::empty(area);
    render_scrollbar(area, &mut fits, 10, 0, 10, &palette);
    assert!((0..10).all(|y| fits.cell((0, y)).unwrap().symbol() == " "));

    let mut top = Buffer::empty(area);
    render_scrollbar(area, &mut top, 100, 0, 10, &palette);
    assert_eq!(top.cell((0, 0)).unwrap().symbol(), "┃");
    assert_eq!(top.cell((0, 9)).unwrap().symbol(), "│");

    let mut bottom = Buffer::empty(area);
    render_scrollbar(area, &mut bottom, 100, 90, 10, &palette);
    assert_eq!(bottom.cell((0, 0)).unwrap().symbol(), "│");
    assert_eq!(bottom.cell((0, 9)).unwrap().symbol(), "┃");
}

// ---- /log overlay ---------------------------------------------------------------------------

#[test]
fn slash_log_opens_the_overlay_and_esc_closes_it() {
    let dir = temp_dir();
    let mut state = make_state(&dir);
    type_text(&mut state, "/log");
    press(&mut state, KeyCode::Enter);
    assert!(state.log.open);
    press(&mut state, KeyCode::Esc);
    assert!(!state.log.open);
}

#[test]
fn level_filter_keys_hide_lower_levels() {
    let dir = temp_dir();
    let mut state = log_state(&dir, 16);
    assert_eq!(state.log.filtered().len(), 16);
    press(&mut state, KeyCode::Char('2')); // info and up
    assert!(state.log.filtered().iter().all(|e| e.level >= RunLogLevel::Info));
    assert_eq!(state.log.filtered().len(), 12);
    press(&mut state, KeyCode::Char('3')); // warn and up
    assert_eq!(state.log.filtered().len(), 4);
    press(&mut state, KeyCode::Char('4')); // error only
    assert!(state.log.filtered().is_empty());
    let text = text_of(&render(100, 24, &state));
    assert!(text.contains("No log entries match"), "{text}");
    press(&mut state, KeyCode::Char('1'));
    assert_eq!(state.log.filtered().len(), 16);
}

#[test]
fn source_filter_cycles_and_search_narrows() {
    let dir = temp_dir();
    let mut state = log_state(&dir, 16);
    press(&mut state, KeyCode::Char('s'));
    let first = state.log.source.clone().expect("a source is selected");
    assert!(state.log.filtered().iter().all(|e| e.source == first));
    // cycle through every source and back to "all"
    let n = state.log.sources().len();
    for _ in 0..n {
        press(&mut state, KeyCode::Char('s'));
    }
    assert_eq!(state.log.source, None);

    press(&mut state, KeyCode::Char('/'));
    assert!(state.log.searching);
    type_text(&mut state, "fake says");
    press(&mut state, KeyCode::Enter);
    assert!(!state.log.searching);
    let hits = state.log.filtered();
    assert_eq!(hits.len(), 4);
    assert!(hits.iter().all(|e| e.message.contains("fake says")));
    let text = text_of(&render(100, 24, &state));
    assert!(text.contains("search: fake says"), "{text}");
}

#[test]
fn follow_toggle_wheel_scroll_and_copy() {
    let dir = temp_dir();
    let mut state = log_state(&dir, 100);
    let vp = Rect::new(0, 0, 100, 24);
    let hit = HitRegions::default();
    // view_rows is synced from the terminal size by the event handler
    state.handle_terminal_event(crossterm::event::Event::Resize(100, 24), &hit, vp);
    assert_eq!(state.log.view_rows, 22);
    assert!(state.log.follow);
    let following = text_of(&render(100, 24, &state));
    assert!(following.contains("turn note 99"), "follow shows the newest entry");
    assert!(!following.contains("debug detail 0"));

    // wheel up leaves follow mode and shows older entries
    for _ in 0..10 {
        state.handle_mouse_event(
            MouseEvent {
                kind: MouseEventKind::ScrollUp,
                column: 5,
                row: 5,
                modifiers: KeyModifiers::NONE,
            },
            &hit,
            vp,
        );
    }
    assert!(!state.log.follow);
    let scrolled = text_of(&render(100, 24, &state));
    assert!(!scrolled.contains("turn note 99"), "{scrolled}");
    assert!(scrolled.contains("paused"));

    // f resumes following
    press(&mut state, KeyCode::Char('f'));
    state.log.follow = false;
    press(&mut state, KeyCode::Char('f'));
    assert!(state.log.follow);

    // c copies exactly the visible window
    let actions = press(&mut state, KeyCode::Char('c'));
    match &actions[..] {
        [AppAction::CopyToClipboard(text)] => {
            assert_eq!(text.lines().count(), 22);
            assert!(text.lines().last().unwrap().contains("turn note 99"));
            assert!(text.lines().next().unwrap().contains("INFO") || text.contains("WARN"));
        }
        other => panic!("expected a copy action, got {other:?}"),
    }
}

#[test]
fn overlay_draws_a_scrollbar_only_for_long_logs() {
    let dir = temp_dir();
    let short = text_of(&render(100, 24, &log_state(&dir, 5)));
    assert!(!short.contains('┃'), "{short}");
    let long = text_of(&render(100, 24, &log_state(&dir, 200)));
    assert!(long.contains('┃'), "{long}");
}

#[test]
fn error_entries_raise_a_toast_pointing_at_the_log() {
    let dir = temp_dir();
    let mut state = make_state(&dir);
    state.on_log_entry(entry(1, RunLogLevel::Warn, "provider", "just a warning"));
    assert!(state.toast.is_none());
    state.on_log_entry(entry(2, RunLogLevel::Error, "plugin:fake", "plugin crashed"));
    let toast = state.toast.clone().expect("toast");
    assert!(toast.is_error);
    assert_eq!(toast.text, "plugin crashed — /log for details");
}

#[test]
fn log_secrets_are_never_stored_unmasked_by_the_overlay_state() {
    // Masking happens in the gateway RunLog (covered in its tests); the overlay shows what it gets.
    let dir = temp_dir();
    let mut state = make_state(&dir);
    state.on_log_entry(entry(1, RunLogLevel::Info, "gateway", "key=[REDACTED]"));
    state.log.open();
    let text = text_of(&render(100, 24, &state));
    assert!(text.contains("[REDACTED]"));
}

#[test]
fn snapshot_log_overlay_at_three_sizes() {
    let dir = temp_dir();
    for (w, h) in [(80, 24), (120, 40), (200, 50)] {
        let state = log_state(&dir, 60);
        insta::assert_snapshot!(format!("log_overlay_{w}x{h}"), text_of(&render(w, h, &state)));
    }
}

// ---- late reply -----------------------------------------------------------------------------

#[test]
fn late_reply_from_an_earlier_turn_does_not_end_the_next_turns_busy_state() {
    let dir = temp_dir();
    let mut state = make_state(&dir);
    let clock = ManualClock::new();
    state.turn.set_clock(Arc::new(clock.clone()));

    type_text(&mut state, "one");
    press(&mut state, KeyCode::Enter);
    let first = state.turn.seq();
    // turn one finishes without a reply inside the grace window and ends as silent
    state.on_turn_finished(first);
    clock.advance(2_000);
    assert!(state.tick_turn());
    assert!(!state.is_busy());

    type_text(&mut state, "two");
    press(&mut state, KeyCode::Enter);
    let second = state.turn.seq();
    assert_ne!(first, second);
    assert!(state.is_busy());

    // turn one's reply arrives late: shown, but the running turn stays busy
    let disposition =
        state.on_assistant_reply(Some(first), None, "late answer".to_owned(), 0);
    assert_eq!(disposition, ReplyDisposition::LateReply);
    assert!(state.is_busy(), "the late reply must not end turn two");
    let all_text: Vec<String> = state
        .active_timeline()
        .iter()
        .filter_map(|i| match i {
            ChatTimelineItem::AssistantMessage { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(all_text, ["late answer"]);

    // turn two's own reply ends it
    let disposition = state.on_assistant_reply(Some(second), None, "answer two".to_owned(), 0);
    assert_eq!(disposition, ReplyDisposition::CurrentTurn);
    assert!(!state.is_busy());
}

// ---- 7.1 config error startup ---------------------------------------------------------------

fn config_error_state(dir: &TempDir) -> TuiAppState {
    let mut state = make_state(dir);
    state.set_config_error(
        "config key 'model.provider' value 'nonsense' is invalid (expected a known provider)"
            .to_owned(),
        Some("model.provider".to_owned()),
    );
    state
}

#[test]
fn config_error_banner_disables_chat_and_settings_opens_on_the_key() {
    let dir = temp_dir();
    let mut state = config_error_state(&dir);
    let text = text_of(&render(100, 24, &state));
    assert!(text.contains("Configuration error"), "{text}");
    assert!(text.contains("model.provider"), "{text}");
    assert!(text.contains("/retry"), "{text}");

    // typing a message does not start a turn
    type_text(&mut state, "hello");
    let actions = press(&mut state, KeyCode::Enter);
    assert!(actions.is_empty());
    assert!(!state.is_busy());
    assert!(state.toast.as_ref().is_some_and(|t| t.text.contains("Chat is disabled")));

    // /settings opens the Model tab on the offending row
    type_text(&mut state, "/settings");
    press(&mut state, KeyCode::Enter);
    assert!(state.settings.open);
    assert_eq!(state.settings.tab, SettingsTab::Model);
    assert_eq!(state.settings.selected_index, 0, "model.provider is the first Model row");

    // Ctrl+R (or /retry) asks the runtime to retry the boot
    state.settings.close();
    let retry = state.handle_key_event(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL));
    assert_eq!(retry, vec![AppAction::RetryBoot]);
    type_text(&mut state, "/retry");
    assert_eq!(press(&mut state, KeyCode::Enter), vec![AppAction::RetryBoot]);
}

#[test]
fn snapshot_config_error_banner_at_three_sizes() {
    let dir = temp_dir();
    for (w, h) in [(80, 24), (120, 40), (200, 50)] {
        let state = config_error_state(&dir);
        insta::assert_snapshot!(format!("config_error_{w}x{h}"), text_of(&render(w, h, &state)));
    }
}

// ---- 7.2 /doctor ----------------------------------------------------------------------------

#[test]
fn doctor_flags_a_missing_plugin_binary_and_still_runs_the_other_checks() {
    use std::os::unix::fs::PermissionsExt;
    let dir = temp_dir();
    let cfg_dir = dir.path().join("config");
    let data_dir = dir.path().join("data");
    std::fs::create_dir_all(&cfg_dir).unwrap();
    std::fs::create_dir_all(&data_dir).unwrap();
    let missing = dir.path().join("no-such-plugin-binary");
    let config_path = cfg_dir.join("monoagens.toml");
    let secrets_path = cfg_dir.join("secrets.toml");
    std::fs::write(
        &config_path,
        format!(
            "version = 1\n[model]\nprovider = \"scripted\"\nmodel = \"m\"\n\n[plugins.ghost]\nkind = \"adapter\"\nenabled = true\ncommand = \"{}\"\n",
            missing.display()
        ),
    )
    .unwrap();
    std::fs::write(&secrets_path, "").unwrap();
    std::fs::set_permissions(&secrets_path, std::fs::Permissions::from_mode(0o644)).unwrap();

    let checks = run_doctor(&DoctorInput {
        paths: ResolvedPaths {
            config_path: config_path.clone(),
            secrets_path: secrets_path.clone(),
            data_dir: data_dir.clone(),
        },
        link: "hosting".to_owned(),
    });
    let by = |name: &str| checks.iter().find(|c| c.name == name).unwrap_or_else(|| {
        panic!("no check {name}: {:?}", checks.iter().map(|c| &c.name).collect::<Vec<_>>())
    });
    assert_eq!(by("config").status, DoctorStatus::Pass, "{:?}", by("config"));
    let plugin = by("plugin:ghost");
    assert_eq!(plugin.status, DoctorStatus::Fail);
    assert!(plugin.detail.contains("no-such-plugin-binary"), "{plugin:?}");
    assert!(!plugin.hint.is_empty());
    // loose secrets mode is a failure with a chmod hint, but the other checks still ran
    assert_eq!(by("secrets").status, DoctorStatus::Fail);
    assert!(by("secrets").hint.contains("chmod 600"));
    // the scripted provider skips reachability
    assert_eq!(by("provider").status, DoctorStatus::Skipped);
    assert_eq!(by("data dir").status, DoctorStatus::Pass);
    let report = format_report(&checks);
    assert!(report.contains("[FAIL] plugin:ghost"), "{report}");
    assert!(report.contains("[skip] provider"), "{report}");

    // an invalid config still reports every other check
    std::fs::write(&config_path, "version = 1\n[model]\nprovider = \"nonsense\"\n").unwrap();
    let checks = run_doctor(&DoctorInput {
        paths: ResolvedPaths { config_path, secrets_path, data_dir },
        link: "none".to_owned(),
    });
    assert_eq!(checks.iter().find(|c| c.name == "config").unwrap().status, DoctorStatus::Fail);
    assert!(checks.iter().any(|c| c.name == "data dir"));
    let _ = (
        BodyTransportConfig::Stdio { command: String::new(), args: vec![] },
        PluginConfig {
            id: String::new(),
            enabled: false,
            kind: PluginKind::Adapter,
            transport: BodyTransportConfig::Stdio { command: String::new(), args: vec![] },
            dependencies: vec![],
            optional_dependencies: vec![],
        },
    );
}

#[test]
fn slash_doctor_emits_the_run_doctor_action() {
    let dir = temp_dir();
    let mut state = make_state(&dir);
    type_text(&mut state, "/doctor");
    assert_eq!(press(&mut state, KeyCode::Enter), vec![AppAction::RunDoctor]);
}
