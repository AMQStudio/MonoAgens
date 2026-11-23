//! Turn lifecycle, input, terminal-robustness and width-safety tests for `tui-foundation`.

use std::sync::Arc;

use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use monoagens_gateway::{GatewayConfig, ResolvedPaths};
use monoagens_protocol::ActorId;
use monoagens_tui::turn::{ManualClock, NO_REPLY_GRACE_MS, NO_REPLY_NOTICE};
use monoagens_tui::width::{chrome_text, sanitize};
use monoagens_tui::{
    AppAction, ChatTimelineItem, HitRegions, IconMode, LocalChatStore, SecretsUiSummary,
    SidebarMode, TuiAppState, TuiEffects, TuiPreferences, render_to_buffer,
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

fn clocked_state(dir: &TempDir) -> (TuiAppState, ManualClock) {
    let mut state = make_state(dir);
    let clock = ManualClock::new();
    state.turn.set_clock(Arc::new(clock.clone()));
    (state, clock)
}

fn key(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
    KeyEvent::new(code, mods)
}

fn type_text(state: &mut TuiAppState, text: &str) {
    for ch in text.chars() {
        state.handle_key_event(key(KeyCode::Char(ch), KeyModifiers::NONE));
    }
}

fn send(state: &mut TuiAppState, text: &str) -> Vec<AppAction> {
    type_text(state, text);
    state.handle_key_event(key(KeyCode::Enter, KeyModifiers::NONE))
}

fn notices(state: &TuiAppState) -> Vec<String> {
    state
        .active_timeline()
        .iter()
        .filter_map(|i| match i {
            ChatTimelineItem::SystemNotice { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect()
}

fn user_texts(state: &TuiAppState) -> Vec<String> {
    state
        .active_timeline()
        .iter()
        .filter_map(|i| match i {
            ChatTimelineItem::UserMessage { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect()
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

// ---- 5.1 turn lifecycle -------------------------------------------------------------------

#[test]
fn silent_turn_ends_busy_after_grace_with_notice() {
    let dir = temp_dir();
    let (mut state, clock) = clocked_state(&dir);
    let actions = send(&mut state, "hello");
    assert!(matches!(actions[..], [AppAction::SendUserMessage { .. }]));
    assert!(state.is_busy());

    clock.advance(2_000);
    assert!(!state.tick_turn(), "no grace window before TurnFinished");
    assert!(state.is_busy());
    assert_eq!(state.thinking_elapsed_ms, 2_000);

    let seq = state.turn.seq();
    state.on_turn_finished(seq);
    clock.advance(NO_REPLY_GRACE_MS - 1);
    assert!(!state.tick_turn());
    assert!(state.is_busy(), "still inside the grace window");

    clock.advance(1);
    assert!(state.tick_turn(), "grace expired -> no_reply");
    assert!(!state.is_busy());
    assert_eq!(notices(&state), [NO_REPLY_NOTICE]);

    // the next message starts its own turn
    let actions = send(&mut state, "again");
    assert!(matches!(actions[..], [AppAction::SendUserMessage { .. }]));
    assert!(state.is_busy());
}

#[test]
fn reply_inside_grace_window_ends_busy_without_notice() {
    let dir = temp_dir();
    let (mut state, clock) = clocked_state(&dir);
    send(&mut state, "hello");
    let seq = state.turn.seq();
    state.on_turn_finished(seq);
    clock.advance(500);
    state.finish_busy(); // AssistantReply arrives
    clock.advance(5_000);
    assert!(!state.tick_turn());
    assert!(notices(&state).is_empty());
}

#[test]
fn stale_turn_events_do_not_touch_the_next_turn() {
    let dir = temp_dir();
    let (mut state, clock) = clocked_state(&dir);
    send(&mut state, "one");
    let first = state.turn.seq();
    state.finish_busy();
    send(&mut state, "two");
    assert_ne!(first, state.turn.seq());

    state.on_turn_finished(first);
    clock.advance(10_000);
    assert!(!state.tick_turn(), "stale TurnFinished must not arm the grace window");
    assert!(state.is_busy());
    assert_eq!(state.on_turn_error(first), None);
    assert!(state.is_busy());
    assert!(state.on_turn_error(state.turn.seq()).is_some());
    assert!(!state.is_busy());
}

// ---- 5.2 queue ----------------------------------------------------------------------------

#[test]
fn messages_sent_while_busy_are_queued_and_submitted_in_order() {
    let dir = temp_dir();
    let (mut state, _clock) = clocked_state(&dir);
    send(&mut state, "first");
    assert!(send(&mut state, "second").is_empty());
    assert!(send(&mut state, "third").is_empty());
    assert_eq!(state.turn.queued_len(), 2);
    assert_eq!(user_texts(&state), ["first"]);
    assert!(state.next_queued_action().is_none(), "busy: nothing is submitted");

    state.finish_busy();
    let Some(AppAction::SendUserMessage { text, .. }) = state.next_queued_action() else {
        panic!("expected the oldest queued message");
    };
    assert_eq!(text, "second");
    assert!(state.next_queued_action().is_none(), "second turn is busy again");
    state.finish_busy();
    let Some(AppAction::SendUserMessage { text, .. }) = state.next_queued_action() else {
        panic!("expected the next queued message");
    };
    assert_eq!(text, "third");
    assert_eq!(user_texts(&state), ["first", "second", "third"]);
}

#[test]
fn resize_while_queued_keeps_queue_and_asks_for_redraw() {
    let dir = temp_dir();
    let (mut state, _clock) = clocked_state(&dir);
    send(&mut state, "first");
    send(&mut state, "queued text");
    let out = state.handle_terminal_event(Event::Resize(60, 20), &HitRegions::default(), Rect::new(0, 0, 60, 20));
    assert!(out.redraw && !out.clear && out.actions.is_empty());
    assert_eq!(state.turn.queued_len(), 1);

    let buf = render(60, 20, &state);
    let text = text_of(&buf);
    assert!(text.contains("queued text") && text.contains("queued"), "{text}");
}

#[test]
fn alt_up_pulls_last_queued_message_back_into_the_composer() {
    let dir = temp_dir();
    let (mut state, _clock) = clocked_state(&dir);
    send(&mut state, "first");
    send(&mut state, "alpha");
    send(&mut state, "beta");
    assert_eq!(state.turn.queued_len(), 2);

    state.handle_key_event(key(KeyCode::Up, KeyModifiers::ALT));
    assert_eq!(state.composer.text, "beta");
    assert_eq!(state.turn.queued_len(), 1);
    state.handle_key_event(key(KeyCode::Up, KeyModifiers::ALT));
    assert_eq!(state.composer.text, "alpha\nbeta");
    assert_eq!(state.turn.queued_len(), 0);
    // nothing left: no-op
    state.handle_key_event(key(KeyCode::Up, KeyModifiers::ALT));
    assert_eq!(state.composer.text, "alpha\nbeta");
}

// ---- 5.3 single busy indicator --------------------------------------------------------------

#[test]
fn exactly_one_spinner_glyph_while_busy() {
    let dir = temp_dir();
    let (mut state, clock) = clocked_state(&dir);
    send(&mut state, "hello");
    clock.advance(3_000);
    state.tick_turn();
    for size in [(80, 24), (120, 40)] {
        let text = text_of(&render(size.0, size.1, &state));
        let spinners = text.chars().filter(|c| ('\u{2800}'..='\u{28FF}').contains(c)).count();
        assert_eq!(spinners, 1, "{size:?}:\n{text}");
        assert!(text.contains("3s"), "elapsed in the status bar:\n{text}");
        assert!(text.contains("Reasoning"), "phase text stays:\n{text}");
    }
}

// ---- 6.1 events ---------------------------------------------------------------------------

#[test]
fn terminal_events_resize_focus_reset_and_suspend() {
    let dir = temp_dir();
    let mut state = make_state(&dir);
    let hit = HitRegions::default();
    let vp = Rect::new(0, 0, 80, 24);

    let out = state.handle_terminal_event(Event::Resize(100, 30), &hit, vp);
    assert!(out.redraw && !out.clear);
    let out = state.handle_terminal_event(Event::FocusGained, &hit, vp);
    assert!(out.redraw && out.clear);
    let out = state.handle_terminal_event(Event::FocusLost, &hit, vp);
    assert!(!out.redraw);

    let out = state.handle_terminal_event(
        Event::Key(key(KeyCode::Char('l'), KeyModifiers::ALT)),
        &hit,
        vp,
    );
    assert_eq!(out.actions, [AppAction::ResetDisplay]);
    let out = state.handle_terminal_event(
        Event::Key(key(KeyCode::Char('z'), KeyModifiers::CONTROL)),
        &hit,
        vp,
    );
    assert_eq!(out.actions, [AppAction::Suspend]);
    assert!(!state.should_quit);
}

// ---- 6.2 width safety ---------------------------------------------------------------------

#[test]
fn chrome_text_strips_variation_selectors_and_maps_symbols() {
    assert_eq!(chrome_text("🛡️ Valve", IconMode::Symbols), "◆ Valve");
    assert_eq!(chrome_text("⚙️ Settings", IconMode::Symbols), "≡ Settings");
    assert_eq!(chrome_text("🛡️ Valve", IconMode::Emoji), "🛡 Valve");
    assert_eq!(chrome_text("⚙️", IconMode::Emoji), "⚙");
    assert_eq!(sanitize("a\tb\u{1b}[31mc\r\nd", true), "a    b[31mc\nd");
    assert_eq!(sanitize("a\nb", false), "a b");
}

#[test]
fn emoji_with_variation_selector_never_reaches_the_chrome_at_80_columns() {
    let dir = temp_dir();
    let mut state = make_state(&dir);
    state.prefs.show_valve_badge = true;
    state.set_toast("saved", false);
    for mode in [IconMode::Symbols, IconMode::Emoji] {
        state.prefs.icons = mode;
        let buf = render(80, 24, &state);
        let text = text_of(&buf);
        // the welcome hint line carries "⚙️ /settings" in the feed region
        assert!(!text.contains('\u{FE0F}'), "{mode:?}:\n{text}");
        assert!(text.contains("Valve:"), "{mode:?}:\n{text}");
        for line in text.lines() {
            assert!(line.width() <= 80, "row wider than the terminal: {line:?}");
        }
        if mode == IconMode::Symbols {
            // the wide emoji cell keeps its width, so one pad cell follows the symbol
            assert!(text.contains("◆  Valve:"), "{text}");
            assert!(text.contains("≡ /settings"), "{text}");
            assert!(!text.contains('🛡') && !text.contains('⚙'), "{text}");
        }
    }
}

#[test]
fn chat_content_keeps_emoji_and_strips_control_characters() {
    let dir = temp_dir();
    let mut state = make_state(&dir);
    let id = state.chats.active_chat_id.clone();
    state.push_timeline_item(
        &id,
        ChatTimelineItem::AssistantMessage {
            text: "done ✨ with\ttab \u{1b}[2Jgone".to_owned(),
            duration_ms: None,
            timestamp_ms: 0,
        },
    );
    let buf = render(100, 24, &state);
    let text = text_of(&buf);
    assert!(text.contains("done ✨ with"), "{text}");
    for y in 0..buf.area.height {
        for x in 0..buf.area.width {
            let sym = buf.cell((x, y)).expect("cell").symbol();
            assert!(!sym.chars().any(char::is_control), "control char in cell ({x},{y}): {sym:?}");
        }
    }
}

#[test]
fn pasted_tabs_and_control_chars_are_sanitised_in_the_composer() {
    let dir = temp_dir();
    let mut state = make_state(&dir);
    state.handle_paste("a\tb\u{7}c");
    assert_eq!(state.composer.text, "a    bc");
}

// ---- 6.3 small terminals ------------------------------------------------------------------

#[test]
fn narrow_terminal_hides_sidebar_and_compacts_status_bar() {
    let dir = temp_dir();
    let mut state = make_state(&dir);
    state.prefs.sidebar = SidebarMode::Open;
    assert!(!state.sidebar_visible(79));
    assert!(state.sidebar_visible(80));
    let text = text_of(&render(60, 15, &state));
    assert!(!text.contains("CHATS") && !text.contains("PLUGINS"), "{text}");
    assert!(!text.contains("Valve:"), "{text}");
    assert!(text.contains(&state.model_name), "{text}");
    let wide = text_of(&render(130, 30, &state));
    assert!(wide.contains("CHATS") && wide.contains("Valve:"), "{wide}");
}

#[test]
fn short_terminal_hides_hint_line_and_caps_composer() {
    let dir = temp_dir();
    let mut state = make_state(&dir);
    let tall = text_of(&render(100, 30, &state));
    assert!(tall.contains("esc to clear"), "{tall}");
    let short = text_of(&render(100, 15, &state));
    assert!(!short.contains("esc to clear"), "{short}");
    assert!(short.contains("Message Mono"), "composer still drawn:\n{short}");
    state.composer.text = "1\n2\n3\n4\n5\n6".to_owned();
    state.composer.cursor = state.composer.text.len();
    let short = text_of(&render(100, 15, &state));
    assert!(short.contains("> 4") || short.contains("  4"), "{short}");
}

#[test]
fn tiny_terminal_shows_only_the_size_message() {
    let dir = temp_dir();
    let state = make_state(&dir);
    for (w, h) in [(30u16, 8u16), (39, 24), (100, 9)] {
        let text = text_of(&render(w, h, &state));
        assert!(
            text.replace(['\n', ' '], "").contains("terminaltoosmall(needs40x10)"),
            "{w}x{h}:\n{text}"
        );
        assert!(!text.contains("Message Mono"));
    }
    let ok = text_of(&render(40, 10, &state));
    assert!(!ok.contains("too small"), "{ok}");
}

// ---- snapshots at three sizes -------------------------------------------------------------

fn snapshot_state(dir: &TempDir) -> TuiAppState {
    let mut state = make_state(dir);
    let id = state.chats.active_chat_id.clone();
    state.push_timeline_item(
        &id,
        ChatTimelineItem::UserMessage {
            sender: "owner".to_owned(),
            text: "Hello Mono".to_owned(),
            timestamp_ms: 0,
            is_group: false,
        },
    );
    state.push_timeline_item(
        &id,
        ChatTimelineItem::AssistantMessage {
            text: "Hello! How can I help today?".to_owned(),
            duration_ms: Some(1200),
            timestamp_ms: 0,
        },
    );
    state
}

#[test]
fn snapshot_chat_80x24() {
    let dir = temp_dir();
    insta::assert_snapshot!(text_of(&render(80, 24, &snapshot_state(&dir))));
}

#[test]
fn snapshot_chat_120x40() {
    let dir = temp_dir();
    insta::assert_snapshot!(text_of(&render(120, 40, &snapshot_state(&dir))));
}

#[test]
fn snapshot_chat_200x50() {
    let dir = temp_dir();
    insta::assert_snapshot!(text_of(&render(200, 50, &snapshot_state(&dir))));
}
