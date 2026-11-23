//! Contract tests for core conversation flows: `Esc Esc` interrupt, `/retry`,
//! `/reload`, `Ctrl+O` what-happened summary toggle, `/why` modal backdrop & scroll,
//! and `Ctrl+R` composer history search.

use std::sync::Arc;

use crossterm::event::{KeyCode, KeyModifiers};
use monoagens_gateway::{GatewayConfig, ResolvedPaths};
use monoagens_protocol::ActorId;
use monoagens_tui::conv::{ConvAction, ConvEvent};
use monoagens_tui::core::hitmap::HitTarget;
use monoagens_tui::core::overlay::OverlayId;
use monoagens_tui::turn::{ManualClock, TurnClock};
use monoagens_tui::{
    AppAction, ChatTimelineItem, HitRegions, LocalChatStore, SecretsUiSummary, TuiAppState,
    TuiEffects, TuiPreferences, render_to_buffer,
};
use ratatui::layout::Rect;
use serde_json::json;
use tempfile::TempDir;

use crate::common::{TuiTestHarness, key_press, mouse_scroll_down, paste_event};

fn make_app_state(dir: &TempDir) -> (TuiAppState, ManualClock) {
    let cfg_dir = dir.path().join("config");
    let data_dir = dir.path().join("data");
    std::fs::create_dir_all(&cfg_dir).expect("cfg dir");
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
    state.turn.set_clock(Arc::new(clock.clone()));
    (state, clock)
}

fn type_and_send(state: &mut TuiAppState, text: &str) -> Vec<AppAction> {
    for ch in text.chars() {
        state.handle_key_event(key_press(KeyCode::Char(ch), KeyModifiers::NONE));
    }
    state.handle_key_event(key_press(KeyCode::Enter, KeyModifiers::NONE))
}

#[test]
fn esc_esc_interrupts_busy_turn_and_retry_resubmits_message() {
    let dir = TempDir::new_in(env!("CARGO_TARGET_TMPDIR")).expect("tempdir");
    let (mut state, clock) = make_app_state(&dir);

    let send_acts = type_and_send(&mut state, "explain valve rules");
    assert_eq!(send_acts.len(), 1);
    assert!(state.is_busy());

    // First Esc arms the interrupt prompt without cancelling yet.
    let first_esc = state.handle_key_event(key_press(KeyCode::Esc, KeyModifiers::NONE));
    assert!(first_esc.is_empty());
    assert!(state.conv.esc_prompt_visible(clock.now_ms()));

    // Second Esc within window emits CancelTurn.
    clock.advance(400);
    let second_esc = state.handle_key_event(key_press(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(second_esc, vec![AppAction::Conv(ConvAction::CancelTurn)]);

    // Cancelled response marks turn interrupted so /retry can resubmit it.
    state.conv_on_cancel_result(Ok(true));
    assert!(!state.is_busy());

    let retry_acts = type_and_send(&mut state, "/retry");
    assert_eq!(retry_acts.len(), 1);
    match &retry_acts[0] {
        AppAction::SendUserMessage { text, .. } => assert_eq!(text, "explain valve rules"),
        other => panic!("expected SendUserMessage on /retry, got {other:?}"),
    }
}

#[test]
fn slash_reload_emits_reload_config_action() {
    let dir = TempDir::new_in(env!("CARGO_TARGET_TMPDIR")).expect("tempdir");
    let (mut state, _clock) = make_app_state(&dir);

    let acts = type_and_send(&mut state, "/reload");
    assert_eq!(acts, vec![AppAction::ReloadConfig]);
}

#[test]
fn ctrl_o_toggles_summary_and_why_renders_cleared_modal_with_backdrop() {
    let dir = TempDir::new_in(env!("CARGO_TARGET_TMPDIR")).expect("tempdir");
    let (mut state, _clock) = make_app_state(&dir);

    let _ = type_and_send(&mut state, "hello");
    state.on_assistant_reply(None, None, "hi there".to_owned(), 42);
    state.conv_after_reply(1, Some(77), "hi there");

    // Ctrl+O expands summary and requests FetchTrace.
    let acts = state.handle_key_event(key_press(KeyCode::Char('o'), KeyModifiers::CONTROL));
    assert_eq!(acts.len(), 1);
    assert!(matches!(acts[0], AppAction::Conv(ConvAction::FetchTrace(_))));

    // /why requests FetchExplain for root event #77.
    let why_acts = type_and_send(&mut state, "/why");
    assert_eq!(
        why_acts,
        vec![AppAction::Conv(ConvAction::FetchExplain { event_id: 77 })]
    );

    let trace_req = match &acts[0] {
        AppAction::Conv(ConvAction::FetchTrace(r)) => r.clone(),
        _ => unreachable!(),
    };
    state.conv_event(ConvEvent::Trace {
        request: trace_req,
        result: Ok(vec![json!({"kind": "model_call", "summary": "gpt-4o"})]),
    });
    state.conv_event(ConvEvent::Explain {
        event_id: 77,
        result: Ok(json!({"lines": (0..25).map(|i| format!("verdict line {i:02}")).collect::<Vec<_>>()})),
    });

    let mut harness = TuiTestHarness::new(80, 24);
    let mut effects = TuiEffects::new();
    let mut hit = HitRegions::default();
    harness.draw_with(|area, buf| {
        render_to_buffer(area, buf, &state, &mut effects, &mut hit, 16);
    });

    assert_eq!(
        hit.hitmap.hit_test(0, 0),
        Some(&HitTarget::Backdrop(OverlayId::ConvModal))
    );
    harness.assert_text_in_rect(harness.area(), "Why: event #77");
    harness.assert_text_in_rect(harness.area(), "verdict line 00");

    // Wheel inside /why scrolls without affecting chat_scroll.
    state.chat_scroll = 5;
    state.handle_mouse_event(mouse_scroll_down(40, 12), &hit, harness.area());
    assert_eq!(state.chat_scroll, 5);
    assert!(state.conv.why.as_ref().expect("why").scroll > 0);

    // Esc closes /why.
    state.handle_key_event(key_press(KeyCode::Esc, KeyModifiers::NONE));
    assert!(state.conv.why.is_none());
    assert!(!state.has_open_overlay());
}

#[test]
fn ctrl_r_history_search_supports_paste_and_esc_restores_draft() {
    let dir = TempDir::new_in(env!("CARGO_TARGET_TMPDIR")).expect("tempdir");
    let (mut state, _clock) = make_app_state(&dir);

    let _ = type_and_send(&mut state, "deploy staging cluster");
    let active_id = state.chats.active_chat_id.clone();
    state.finish_busy();
    state.push_timeline_item(
        &active_id,
        ChatTimelineItem::AssistantMessage {
            text: "ok".to_owned(),
            duration_ms: Some(10),
            timestamp_ms: 1,
        },
    );

    // Type a partial draft, then Ctrl+R for reverse-i-search.
    for ch in "draft".chars() {
        state.handle_key_event(key_press(KeyCode::Char(ch), KeyModifiers::NONE));
    }
    state.handle_key_event(key_press(KeyCode::Char('r'), KeyModifiers::CONTROL));
    assert!(state.conv.history_search.is_some());
    assert!(state.has_open_overlay());

    state.handle_terminal_event(
        paste_event("staging\r\n"),
        &HitRegions::default(),
        Rect::new(0, 0, 80, 24),
    );
    assert_eq!(state.composer.text, "deploy staging cluster");

    // Esc cancels history search and restores the prior draft.
    state.handle_key_event(key_press(KeyCode::Esc, KeyModifiers::NONE));
    assert!(state.conv.history_search.is_none());
    assert_eq!(state.composer.text, "draft");
}
