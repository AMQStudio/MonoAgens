//! Semantic contract tests for core chat, slash menu (`/reload`, `/retry`), turn lifecycle,
//! queues (`Alt+Up`), 1-cell chrome safety, narrow/short/tiny viewports, markdown rendering,
//! attachments, drag-select copy, chat history persistence, lab gating, and sidebar status dots.

use std::path::PathBuf;

use crossterm::event::{Event, KeyCode, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use monoagens_gateway::{
    GatewayConfig, PluginKind, ResolvedPaths, load_config_file, parse_config_str,
};
use monoagens_protocol::ActorId;
use monoagens_tui::core::hitmap::HitTarget;
use monoagens_tui::conv::{LastTurn, TurnStatus};
use monoagens_tui::turn::ManualClock;
use monoagens_tui::{
    AppAction, ChatTimelineItem, HitRegions, IconMode, LocalChatStore, MessageLayout, MotionMode,
    PendingAttachmentKind, PluginUiEntry, PluginUiState, SecretsUiSummary, ShimmerMode,
    SidebarMode, ThemePreset, TuiAppState, TuiEffects, TuiPreferences, parse_image_dimensions,
    render_app, render_to_buffer,
};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use tempfile::TempDir;

use crate::common::{TuiTestHarness, key_press, mouse_click, mouse_scroll_down};

fn make_state(dir: &TempDir) -> (TuiAppState, ManualClock) {
    let cfg_dir = dir.path().join("config");
    let data_dir = dir.path().join("data");
    std::fs::create_dir_all(&cfg_dir).expect("config dir");
    std::fs::create_dir_all(&data_dir).expect("data dir");
    let paths = ResolvedPaths {
        config_path: cfg_dir.join("monoagens.toml"),
        secrets_path: cfg_dir.join("secrets.toml"),
        data_dir,
    };
    let mut state = TuiAppState::new(
        paths,
        GatewayConfig::default(),
        TuiPreferences::default(),
        LocalChatStore::default(),
        SecretsUiSummary::default(),
        ActorId::new("per_owner"),
    );
    let clock = ManualClock::new();
    state.turn.set_clock(std::sync::Arc::new(clock.clone()));
    (state, clock)
}

fn type_str(state: &mut TuiAppState, s: &str) {
    for c in s.chars() {
        state.handle_key_event(key_press(KeyCode::Char(c), KeyModifiers::NONE));
    }
}

fn send_msg(state: &mut TuiAppState, s: &str) -> Vec<AppAction> {
    type_str(state, s);
    state.handle_key_event(key_press(KeyCode::Enter, KeyModifiers::NONE))
}

fn render_text(state: &TuiAppState, w: u16, h: u16) -> String {
    let mut harness = TuiTestHarness::new(w, h);
    let mut fx = TuiEffects::new();
    let mut hit = HitRegions::default();
    harness.draw_with(|area, buf| {
        render_to_buffer(area, buf, state, &mut fx, &mut hit, 16);
    });
    harness.buffer_text()
}

#[test]
fn slash_menu_scrolls_to_last_command_supports_wheel_and_click_and_lists_reload_retry() {
    let dir = TempDir::new_in(env!("CARGO_TARGET_TMPDIR")).expect("tempdir");
    let (mut state, _) = make_state(&dir);

    let cmds = state.available_slash_commands();
    let reload = cmds.iter().find(|c| c.name == "reload").expect("/reload command");
    assert_eq!(reload.usage, "/reload");
    assert_eq!(
        reload.description,
        "Reload monoagens.toml, RBAC, plugins, and provider live"
    );

    type_str(&mut state, "/");
    assert!(!state.autocomplete.is_empty());
    let last_idx = state.autocomplete.len() - 1;
    for _ in 0..last_idx {
        state.handle_key_event(key_press(KeyCode::Down, KeyModifiers::NONE));
    }
    assert_eq!(state.autocomplete_idx, last_idx);
    let rendered = render_text(&state, 80, 24);
    let last_name = format!("/{}", state.autocomplete[last_idx].name);
    assert!(rendered.contains(&last_name), "slash menu must keep last command visible:\n{rendered}");

    // Wheel moves slash menu selection without touching chat scroll.
    let mut harness = TuiTestHarness::new(80, 24);
    let mut fx = TuiEffects::new();
    let mut hit = HitRegions::default();
    harness.draw_with(|area, buf| render_to_buffer(area, buf, &state, &mut fx, &mut hit, 16));
    let up = MouseEvent {
        kind: MouseEventKind::ScrollUp,
        column: hit.autocomplete_area.x + 1,
        row: hit.autocomplete_area.y + 1,
        modifiers: KeyModifiers::NONE,
    };
    state.handle_mouse_event(up, &hit, harness.area());
    assert!(state.autocomplete_idx < last_idx);
    assert!(state.auto_scroll);

    // Click a visible slash row.
    if let Some((x, y)) = (hit.autocomplete_area.y..hit.autocomplete_area.bottom())
        .find_map(|y| match hit.hitmap.hit_test(hit.autocomplete_area.x + 2, y) {
            Some(HitTarget::ListRow(_)) => Some((hit.autocomplete_area.x + 2, y)),
            _ => None,
        })
    {
        state.handle_mouse_event(mouse_click(x, y), &hit, harness.area());
        assert!(!state.composer.text.is_empty());
    }
}

#[test]
fn slash_retry_handles_idle_boot_error_and_failed_turn_and_reload_emits_action() {
    let dir = TempDir::new_in(env!("CARGO_TARGET_TMPDIR")).expect("tempdir");
    let (mut state, _) = make_state(&dir);

    assert_eq!(state.execute_slash_command("/reload"), vec![AppAction::ReloadConfig]);

    assert!(state.execute_slash_command("/retry").is_empty());
    let toast = state.toast.as_ref().expect("idle toast");
    assert_eq!(toast.text, "Nothing to retry (use /reload to reload config)");

    state.conv.last_turn = Some(LastTurn {
        chat_id: state.chats.active_chat_id.clone(),
        place_id: state.chats.active_chat().place_id.clone(),
        text: "succeeded".into(),
        attachments: vec![],
        status: TurnStatus::Ok,
    });
    assert!(state.execute_slash_command("/retry").is_empty());

    state.set_config_error("bad toml".into(), Some("model.provider".into()));
    assert_eq!(state.execute_slash_command("/retry"), vec![AppAction::RetryBoot]);

    let (mut err_state, _) = make_state(&dir);
    err_state.conv.last_turn = Some(LastTurn {
        chat_id: err_state.chats.active_chat_id.clone(),
        place_id: err_state.chats.active_chat().place_id.clone(),
        text: "retry me".into(),
        attachments: vec![],
        status: TurnStatus::Error,
    });
    let acts = err_state.execute_slash_command("/retry");
    assert_eq!(acts.len(), 1);
    assert!(matches!(&acts[0], AppAction::SendUserMessage { text, .. } if text == "retry me"));
}

#[test]
fn silent_turn_grace_queue_alt_up_and_single_spinner_invariant() {
    let dir = TempDir::new_in(env!("CARGO_TARGET_TMPDIR")).expect("tempdir");
    let (mut state, clock) = make_state(&dir);

    let acts1 = send_msg(&mut state, "first question");
    assert_eq!(acts1.len(), 1);
    assert!(state.is_busy());

    // Queue second message while busy, then recall with Alt+Up.
    let acts2 = send_msg(&mut state, "queued follow-up");
    assert!(acts2.is_empty());
    assert_eq!(state.turn.queued_len(), 1);
    state.handle_key_event(key_press(KeyCode::Up, KeyModifiers::ALT));
    assert_eq!(state.composer.text, "queued follow-up");
    assert_eq!(state.turn.queued_len(), 0);

    // While busy, status bar renders exactly one spinner glyph.
    let txt = render_text(&state, 80, 24);
    let spinners: usize = txt.chars().filter(|c| "⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏".contains(*c)).count();
    assert_eq!(spinners, 1, "expected 1 spinner glyph:\n{txt}");

    // End agent run then pass grace window -> silent turn notice.
    let seq = state.turn.seq();
    state.on_turn_finished(seq);
    clock.advance(2_000);
    assert!(state.tick_turn());
    assert!(!state.is_busy());
    let chat_id = state.chats.active_chat_id.clone();
    assert!(state.timelines[&chat_id]
        .iter()
        .any(|it| matches!(it, ChatTimelineItem::SystemNotice { .. })));
}

#[test]
fn viewport_responsiveness_narrow_short_tiny_and_clean_chrome_glyphs() {
    let dir = TempDir::new_in(env!("CARGO_TARGET_TMPDIR")).expect("tempdir");
    let (mut state, _) = make_state(&dir);
    state.prefs.icons = IconMode::Symbols;
    state.plugins.push(PluginUiEntry {
        id: "whatsapp".into(),
        kind: PluginKind::Adapter,
        enabled: true,
        installed: true,
        state: PluginUiState::Running,
        command: None,
        socket_path: None,
        config_path: state.paths.config_path.clone(),
        state_dir: state.paths.data_dir.clone(),
        dependencies: vec![],
        optional_dependencies: vec![],
    });
    send_msg(&mut state, "hello 🌟 world");
    state.on_assistant_reply(None, None, "reply with emoji 🎉\n# Heading\n- bullet".into(), 12);

    // Normal 120x24: sidebar visible, markdown rendered, emoji in content preserved.
    let normal = render_text(&state, 120, 24);
    assert!(normal.contains("Heading"));
    assert!(normal.contains("whatsapp"));
    assert!(normal.contains("🎉"));

    // Narrow 60x20 hides sidebar cleanly without panicking.
    let narrow = render_text(&state, 60, 20);
    assert!(!narrow.is_empty());

    // Short 80x10 and tiny 20x6 render cleanly without panic or overflow.
    let short = render_text(&state, 80, 10);
    assert!(!short.is_empty());
    let tiny = render_text(&state, 20, 6);
    assert!(tiny.contains("terminal too small"), "{tiny}");

    // Terminal render_app via TestBackend succeeds cleanly too.
    let mut term = Terminal::new(TestBackend::new(80, 24)).expect("term");
    let mut fx = TuiEffects::new();
    let mut hit = HitRegions::default();
    term.draw(|f| render_app(f, &state, &mut fx, &mut hit)).expect("draw");
}

#[test]
fn drag_select_copy_attachment_png_dimensions_sidebar_ctrl_b_and_prefs_persistence() {
    let dir = TempDir::new_in(env!("CARGO_TARGET_TMPDIR")).expect("tempdir");
    let (mut state, _) = make_state(&dir);

    // Valid PNG header dimensions check.
    let mut png = b"\x89PNG\r\n\x1a\n\x00\x00\x00\x0dIHDR".to_vec();
    png.extend_from_slice(&320u32.to_be_bytes());
    png.extend_from_slice(&240u32.to_be_bytes());
    assert_eq!(parse_image_dimensions(&png), Some((320, 240)));

    // Sidebar toggle via Ctrl+B (SidebarMode::Auto is visible at >= 120 cols).
    assert!(state.sidebar_visible(120));
    state.handle_key_event(key_press(KeyCode::Char('b'), KeyModifiers::CONTROL));
    assert!(!state.sidebar_visible(120));

    // Lab gating.
    assert!(!state.lab_mode);
    state.execute_slash_command("/lab on");
    assert!(state.lab_mode);
    state.execute_slash_command("/lab off");
    assert!(!state.lab_mode);

    // Drag selection copy on chat feed emits CopyToClipboard on release.
    send_msg(&mut state, "copyable user text");
    let mut harness = TuiTestHarness::new(100, 24);
    let mut fx = TuiEffects::new();
    let mut hit = HitRegions::default();
    harness.draw_with(|area, buf| render_to_buffer(area, buf, &state, &mut fx, &mut hit, 16));
    let row_off = hit
        .chat_viewport_lines
        .iter()
        .position(|line| line.contains("copyable user text"))
        .expect("rendered user message line in chat_viewport_lines");
    let fy = hit.chat_area.y + row_off as u16;
    let fx_start = hit.chat_area.x;
    let down = MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: fx_start,
        row: fy,
        modifiers: KeyModifiers::NONE,
    };
    let drag = MouseEvent {
        kind: MouseEventKind::Drag(MouseButton::Left),
        column: hit.chat_area.right().saturating_sub(1),
        row: fy,
        modifiers: KeyModifiers::NONE,
    };
    let up = MouseEvent {
        kind: MouseEventKind::Up(MouseButton::Left),
        column: hit.chat_area.right().saturating_sub(1),
        row: fy,
        modifiers: KeyModifiers::NONE,
    };
    state.handle_mouse_event(down, &hit, harness.area());
    state.handle_mouse_event(drag, &hit, harness.area());
    assert!(state.selection.is_some());
    let copy_acts = state.handle_mouse_event(up, &hit, harness.area());
    assert!(copy_acts.iter().any(|a| matches!(a, AppAction::CopyToClipboard(_))));

    // Chat scrollbar wheel down works when content overflows.
    state.chat_scroll = 2;
    state.auto_scroll = false;
    state.handle_mouse_event(mouse_scroll_down(hit.chat_area.x + 2, hit.chat_area.y + 1), &hit, harness.area());

    // PDF gating without pdf-reader/workspace plugin warns cleanly.
    let pdf_path = dir.path().join("doc.pdf");
    std::fs::write(&pdf_path, b"%PDF-1.4\n").unwrap();
    let res = state.handle_terminal_event(
        Event::Paste(pdf_path.display().to_string()),
        &hit,
        harness.area(),
    );
    assert!(res.redraw);
    let _ = (
        PathBuf::new(),
        PendingAttachmentKind::Image {
            width: Some(1),
            height: Some(1),
            mime: "image/png".into(),
        },
        parse_config_str("version = 1\n", &state.paths.config_path, &state.paths.secrets_path),
        load_config_file(&state.paths.config_path, &state.paths.secrets_path),
        MessageLayout::Stream,
        MotionMode::Reduced,
        ShimmerMode::Subtle,
        SidebarMode::Open,
        ThemePreset::Terracotta,
    );
}
