//! Contract tests for central paste sanitization, `TextInput` editing/undo/outcomes,
//! `PasteCollapseStore` collapse & submit expansion, and `ClipboardPort` injection.

use crossterm::event::{Event, KeyCode, KeyModifiers};
use monoagens_gateway::{GatewayConfig, ResolvedPaths};
use monoagens_protocol::ActorId;
use monoagens_tui::core::input::{
    ClipboardPort, PasteCollapseStore, PasteTarget, TextInput, sanitize_paste,
};
use monoagens_tui::core::overlay::WidgetOutcome;
use monoagens_tui::settings::{SetupWizardState, WizardStep};
use monoagens_tui::{
    AppAction, HitRegions, LocalChatStore, SecretsUiSummary, TuiAppState, TuiPreferences,
};
use ratatui::layout::Rect;
use tempfile::TempDir;

use crate::common::{FakeClipboard, key_press, paste_event};

fn type_ascii(input: &mut TextInput, s: &str) {
    for ch in s.chars() {
        let outcome = input.handle_key(key_press(KeyCode::Char(ch), KeyModifiers::NONE));
        assert_eq!(outcome, WidgetOutcome::Consumed);
    }
}

#[test]
fn tmux_csi_u_and_xterm_modify_other_keys_never_leak_into_text_input() {
    let csi_u = sanitize_paste("alpha\x1b[118;5ubeta", false);
    assert!(!csi_u.contains('\x1b'));
    assert!(!csi_u.contains("[118"));
    assert!(!csi_u.contains("5u"));
    assert!(csi_u.contains("alpha") && csi_u.contains("beta"));

    let xterm = sanitize_paste("gamma\x1b[27;5;118~delta", false);
    assert!(!xterm.contains('\x1b'));
    assert!(!xterm.contains("[27;5"));
    assert!(!xterm.contains("118~"));
    assert!(xterm.contains("gamma") && xterm.contains("delta"));

    let mut input = TextInput::new(false);
    assert!(input.paste_text("key-\x1b[118;5u\x1b[27;5;118~-val"));
    assert!(!input.as_str().contains('\x1b'));
    assert!(!input.as_str().contains("[118"));
    assert!(!input.as_str().contains("[27;5"));
    assert!(input.as_str().starts_with("key-"));
    assert!(input.as_str().ends_with("-val"));
}

#[test]
fn crlf_and_cr_normalize_differently_for_single_line_and_multiline() {
    let single = sanitize_paste("first\r\nsecond\rthird\n", false);
    assert!(!single.contains('\r'));
    assert!(!single.contains('\n'));
    assert_eq!(single, "first second third");

    let multi = sanitize_paste("first\r\nsecond\rthird", true);
    assert_eq!(multi, "first\nsecond\nthird");
}

#[test]
fn unicode_nfd_normalizes_to_nfc_in_sanitizer_and_text_input() {
    let nfd = "cafe\u{0301}";
    assert_eq!(sanitize_paste(nfd, false), "caf\u{00e9}");

    let mut input = TextInput::new(false);
    assert!(input.paste_text(nfd));
    assert_eq!(input.as_str(), "caf\u{00e9}");
}

#[test]
fn tab_expansion_and_c0_del_stripping() {
    let raw = "col1\tcol2\x00\x07\x1b\x1f\x7fend";
    let sanitized = sanitize_paste(raw, false);
    assert_eq!(sanitized, "col1 col2end");

    let multi = sanitize_paste("a\tb\n\x01c\x7fd", true);
    assert_eq!(multi, "a    b\ncd");
}

#[test]
fn text_input_navigation_word_kill_line_kill_undo_and_ignored_keys() {
    let mut input = TextInput::new(false);
    type_ascii(&mut input, "hello brave world");
    assert_eq!(input.as_str(), "hello brave world");

    // Esc and Enter on single-line TextInput must return WidgetOutcome::Ignored.
    assert_eq!(
        input.handle_key(key_press(KeyCode::Esc, KeyModifiers::NONE)),
        WidgetOutcome::Ignored
    );
    assert_eq!(
        input.handle_key(key_press(KeyCode::Enter, KeyModifiers::NONE)),
        WidgetOutcome::Ignored
    );

    // Move left across "world" (5 chars) and delete "brave " with Ctrl+W.
    for _ in 0..5 {
        assert_eq!(
            input.handle_key(key_press(KeyCode::Left, KeyModifiers::NONE)),
            WidgetOutcome::Consumed
        );
    }
    assert_eq!(
        input.handle_key(key_press(KeyCode::Char('w'), KeyModifiers::CONTROL)),
        WidgetOutcome::Consumed
    );
    assert_eq!(input.as_str(), "hello world");

    // Undo restores "hello brave world".
    assert!(input.undo());
    assert_eq!(input.as_str(), "hello brave world");

    // Move Home, then Right 6 times (just after "hello "), then Ctrl+K kills to end.
    assert_eq!(
        input.handle_key(key_press(KeyCode::Home, KeyModifiers::NONE)),
        WidgetOutcome::Consumed
    );
    for _ in 0..6 {
        input.handle_key(key_press(KeyCode::Right, KeyModifiers::NONE));
    }
    assert_eq!(
        input.handle_key(key_press(KeyCode::Char('k'), KeyModifiers::CONTROL)),
        WidgetOutcome::Consumed
    );
    assert_eq!(input.as_str(), "hello ");

    // Undo restores before Ctrl+K, then End + Ctrl+U clears back to start.
    assert!(input.undo());
    assert_eq!(input.as_str(), "hello brave world");
    assert_eq!(
        input.handle_key(key_press(KeyCode::End, KeyModifiers::NONE)),
        WidgetOutcome::Consumed
    );
    assert_eq!(
        input.handle_key(key_press(KeyCode::Char('u'), KeyModifiers::CONTROL)),
        WidgetOutcome::Consumed
    );
    assert_eq!(input.as_str(), "");
    assert!(input.undo());
    assert_eq!(input.as_str(), "hello brave world");
}

#[test]
fn paste_collapse_store_collapses_large_payloads_and_expands_on_submit() {
    let mut store = PasteCollapseStore::default();

    // Small paste stays inline without collapsing.
    let short = "line 1\nline 2";
    let short_out = store.collapse_if_large(short);
    assert_eq!(short_out, short);
    assert_eq!(store.expand(&short_out), short);

    // >10 lines collapses into [Paste #1, +L lines] and expands cleanly on submit.
    let twelve_lines = (1..=12)
        .map(|i| format!("row-{i}"))
        .collect::<Vec<_>>()
        .join("\n");
    let collapsed = store.collapse_if_large(&twelve_lines);
    assert_eq!(collapsed, "[Paste #1, +12 lines]");
    let prompt = format!("please review:\n{collapsed}");
    assert_eq!(
        store.expand(&prompt),
        format!("please review:\n{twelve_lines}")
    );

    // >1000 chars on a single line also collapses and expands cleanly.
    let long_single = "z".repeat(1200);
    let collapsed_long = store.collapse_if_large(&long_single);
    assert_eq!(collapsed_long, "[Paste #2, +1 lines]");
    assert_eq!(store.expand(&collapsed_long), long_single);
}

#[test]
fn fake_clipboard_injection_via_clipboard_port_and_paste_target() {
    fn paste_from_port(target: &mut dyn PasteTarget, clipboard: &dyn ClipboardPort) {
        if let Ok(Some(text)) = clipboard.read_text() {
            target.paste_text(&text);
        }
    }

    let clipboard = FakeClipboard::new(Some("sk-live-token\r\n"));
    let mut input = TextInput::new(false);
    paste_from_port(&mut input, &clipboard);
    assert_eq!(input.as_str(), "sk-live-token");

    let before = input.as_str().to_owned();
    clipboard.set_text(None);
    paste_from_port(&mut input, &clipboard);
    assert_eq!(input.as_str(), before);

    clipboard.write_text("copied-via-port").expect("write_text");
    let mut third = TextInput::new(false);
    paste_from_port(&mut third, &clipboard);
    assert_eq!(third.as_str(), "copied-via-port");

    // Bracketed Event::Paste payload routes through the same sanitizer path.
    if let Event::Paste(raw) = paste_event("e\u{0301}\t1") {
        let mut second = TextInput::new(false);
        assert!(second.paste_text(&raw));
        assert_eq!(second.as_str(), "\u{00e9} 1");
    } else {
        panic!("expected Event::Paste");
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
fn event_paste_works_in_wizard_step_key_and_step_endpoint() {
    let dir = TempDir::new_in(env!("CARGO_TARGET_TMPDIR")).expect("tempdir");
    let mut state = make_app_state(&dir);
    let hit = HitRegions::default();
    let viewport = Rect::new(0, 0, 100, 30);

    // WizardStep::StepEndpoint (even with w.editing == false) accepts Event::Paste.
    let mut wizard = SetupWizardState::new(&state.gateway_cfg, None, vec![]);
    wizard.editing = false;
    wizard.endpoint.clear();
    wizard.step = WizardStep::StepEndpoint;
    state.wizard = Some(wizard);

    state.handle_terminal_event(
        paste_event("https://api.example.com/v1\r\n"),
        &hit,
        viewport,
    );
    assert_eq!(
        state.wizard.as_ref().expect("wizard").endpoint,
        "https://api.example.com/v1"
    );
    assert!(state.composer.text.is_empty());

    // WizardStep::StepKey (even with w.editing == false) accepts Event::Paste and strips escapes/newlines.
    if let Some(w) = &mut state.wizard {
        w.step = WizardStep::StepKey;
        w.api_key.clear();
    }
    state.handle_terminal_event(
        paste_event("sk-\x1b[118;5ulive-secret\r\n"),
        &hit,
        viewport,
    );
    assert_eq!(
        state.wizard.as_ref().expect("wizard").api_key,
        "sk-live-secret"
    );
    assert!(state.composer.text.is_empty());
}

#[test]
fn event_paste_works_in_log_search_and_ctrl_k_palette() {
    let dir = TempDir::new_in(env!("CARGO_TARGET_TMPDIR")).expect("tempdir");
    let mut state = make_app_state(&dir);
    let hit = HitRegions::default();
    let viewport = Rect::new(0, 0, 100, 30);

    // Log search paste only inserts when searching == true.
    state.log.open();
    state.handle_terminal_event(paste_event("ignored"), &hit, viewport);
    assert!(state.log.search.is_empty());

    state.log.searching = true;
    state.handle_terminal_event(paste_event("timeout\x1b[27;5;118~\r\n"), &hit, viewport);
    assert_eq!(state.log.search, "timeout");
    assert!(state.composer.text.is_empty());
    state.log.open = false;

    // Ctrl+K Palette paste sanitizes and updates query without leaking to composer.
    state.open_palette();
    state.handle_terminal_event(paste_event("valve\te\u{0301}\n"), &hit, viewport);
    assert_eq!(
        state.ux.palette.as_ref().expect("palette").query,
        "valve \u{00e9}"
    );
    assert!(state.composer.text.is_empty());
}

#[test]
fn composer_large_paste_collapses_to_token_and_expands_on_send() {
    let dir = TempDir::new_in(env!("CARGO_TARGET_TMPDIR")).expect("tempdir");
    let mut state = make_app_state(&dir);
    let hit = HitRegions::default();
    let viewport = Rect::new(0, 0, 100, 30);

    let twelve_lines = (1..=12)
        .map(|i| format!("line-{i}: payload"))
        .collect::<Vec<_>>()
        .join("\n");
    state.handle_terminal_event(paste_event(&twelve_lines), &hit, viewport);
    assert_eq!(state.composer.text, "[Paste #1, +12 lines]");

    let actions = state.handle_key_event(key_press(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(actions.len(), 1);
    match &actions[0] {
        AppAction::SendUserMessage { text, .. } => {
            assert_eq!(text, &twelve_lines);
            assert!(!text.contains("[Paste #1"));
        }
        other => panic!("expected SendUserMessage, got {other:?}"),
    }
    assert!(state.composer.text.is_empty());
}
