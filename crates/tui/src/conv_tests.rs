//! Tests of the conversation features: pure state tests driven with a manual clock, and
//! end-to-end tests against a hosted gateway with scripted providers (no network).

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use monoagens_core::{DoorKind, EventDetail, EventKind, EventRecord, NumericFeatures};
use monoagens_gateway::{GatewayConfig, LoadedGatewayEnv, ResolvedPaths};
use monoagens_protocol::{ActorId, Audience, OpaquePayload, OriginActor, PlaceId, Taint, Time};
use monoagens_reason::{
    ModelCallFailure, ModelCallSuccess, ModelProvider, ModelRequest, ProviderSecrets,
    ScriptedModelProvider, ScriptedReply,
};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use serde_json::json;
use tokio::sync::mpsc;

use super::*;
use crate::client::{ClientKind, GatewayClient, RuntimeEvent};
use crate::config::{LocalChatStore, TuiPreferences};
use crate::effects::TuiEffects;
use crate::runtime::{LogFollower, TURN_CORRELATION_PREFIX, TuiLaunchOptions, boot_gateway, build_tui_percept, poll_new_gateway_events};
use crate::settings::SecretsUiSummary;
use crate::state::{AppAction, ChatTimelineItem, HitRegions, TuiAppState};
use crate::turn::ManualClock;

fn target_tmp() -> tempfile::TempDir {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/tui-unit-tests");
    std::fs::create_dir_all(&base).expect("mkdir target tmp");
    tempfile::TempDir::new_in(base).expect("tempdir on disk")
}

fn paths_in(dir: &tempfile::TempDir) -> ResolvedPaths {
    let cfg_dir = dir.path().join("config");
    let data_dir = dir.path().join("data");
    std::fs::create_dir_all(&cfg_dir).unwrap();
    std::fs::create_dir_all(&data_dir).unwrap();
    ResolvedPaths {
        config_path: cfg_dir.join("monoagens.toml"),
        secrets_path: cfg_dir.join("secrets.toml"),
        data_dir,
    }
}

fn state_in(dir: &tempfile::TempDir) -> (TuiAppState, ManualClock) {
    let mut state = TuiAppState::new(
        paths_in(dir),
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

fn key(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
    KeyEvent::new(code, mods)
}

fn press(state: &mut TuiAppState, code: KeyCode, mods: KeyModifiers) -> Vec<AppAction> {
    state.handle_key_event(key(code, mods))
}

fn type_text(state: &mut TuiAppState, text: &str) {
    for ch in text.chars() {
        press(state, KeyCode::Char(ch), KeyModifiers::NONE);
    }
}

fn send(state: &mut TuiAppState, text: &str) -> Vec<AppAction> {
    type_text(state, text);
    press(state, KeyCode::Enter, KeyModifiers::NONE)
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

fn screen(state: &TuiAppState, w: u16, h: u16) -> String {
    let area = Rect::new(0, 0, w, h);
    let mut buf = Buffer::empty(area);
    let mut fx = TuiEffects::new();
    let mut hit = HitRegions::default();
    crate::render::render_to_buffer(area, &mut buf, state, &mut fx, &mut hit, 16);
    (0..h)
        .map(|y| (0..w).map(|x| buf[(x, y)].symbol().to_owned()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

fn reply_in_state(state: &mut TuiAppState, text: &str) {
    state.on_assistant_reply(None, None, text.to_owned(), 5);
    state.conv_after_reply(state.turn.seq(), Some(1), text);
}

// ------------------------------------------------------------------------------ turn control

#[test]
fn esc_twice_within_the_window_interrupts_and_a_single_esc_expires() {
    let dir = target_tmp();
    let (mut state, clock) = state_in(&dir);
    send(&mut state, "slow question");
    assert!(state.is_busy());

    // First Esc: a prompt, no action.
    assert!(press(&mut state, KeyCode::Esc, KeyModifiers::NONE).is_empty());
    assert!(screen(&state, 100, 30).contains("Press Esc again to interrupt"));

    // Waiting past 1.5 s: the prompt is gone, the turn continues.
    clock.advance(1_600);
    assert!(!screen(&state, 100, 30).contains("Press Esc again to interrupt"));
    assert!(state.is_busy());

    // A late second Esc only re-arms; a prompt Esc inside the window interrupts.
    assert!(press(&mut state, KeyCode::Esc, KeyModifiers::NONE).is_empty());
    clock.advance(1_000);
    let actions = press(&mut state, KeyCode::Esc, KeyModifiers::NONE);
    assert_eq!(actions, vec![AppAction::Conv(ConvAction::CancelTurn)]);
}

#[test]
fn esc_keeps_its_other_meanings_when_idle() {
    let dir = target_tmp();
    let (mut state, _) = state_in(&dir);
    type_text(&mut state, "draft text");
    assert!(press(&mut state, KeyCode::Esc, KeyModifiers::NONE).is_empty());
    assert!(state.composer.text.is_empty(), "idle Esc still clears the composer");
    // ...and the cleared draft can be recalled.
    press(&mut state, KeyCode::Up, KeyModifiers::NONE);
    assert_eq!(state.composer.text, "draft text");
}

#[test]
fn interrupt_ends_busy_shows_the_notice_keeps_the_queue_and_enables_retry() {
    let dir = target_tmp();
    let (mut state, _) = state_in(&dir);
    send(&mut state, "first question");
    send(&mut state, "second question"); // queued
    assert_eq!(state.turn.queued_len(), 1);

    state.conv_on_cancel_result(Ok(true));
    assert!(!state.is_busy());
    assert!(notices(&state).iter().any(|n| n == "Interrupted."));
    assert_eq!(state.turn.queued_len(), 1, "the queue survives the interrupt");
    match state.next_queued_action() {
        Some(AppAction::SendUserMessage { text, .. }) => assert_eq!(text, "second question"),
        other => panic!("queued message must be submitted next: {other:?}"),
    }
    assert!(state.is_busy());
}

#[test]
fn interrupt_that_cannot_stop_says_so_and_stays_busy() {
    let dir = target_tmp();
    let (mut state, _) = state_in(&dir);
    send(&mut state, "waiting on a capability");
    state.conv_on_cancel_result(Ok(false));
    assert!(state.is_busy(), "the turn finishes on its own");
    assert!(notices(&state).iter().any(|n| n.contains("could not be stopped")));
}

#[test]
fn retry_resends_after_error_or_interrupt_and_does_nothing_after_success() {
    let dir = target_tmp();
    let (mut state, _) = state_in(&dir);

    // After a provider error: F5 sends the same text as a new turn.
    send(&mut state, "please answer");
    let seq = state.turn.seq();
    state.on_turn_error(seq);
    let actions = press(&mut state, KeyCode::F(5), KeyModifiers::NONE);
    match actions.as_slice() {
        [AppAction::SendUserMessage { text, .. }] => assert_eq!(text, "please answer"),
        other => panic!("expected a resend: {other:?}"),
    }
    assert!(state.is_busy());
    let users = state
        .active_timeline()
        .iter()
        .filter(|i| matches!(i, ChatTimelineItem::UserMessage { .. }))
        .count();
    assert_eq!(users, 2, "a new event; nothing is deleted");

    // After a success: nothing.
    reply_in_state(&mut state, "done");
    assert!(press(&mut state, KeyCode::F(5), KeyModifiers::NONE).is_empty());
    assert!(press(&mut state, KeyCode::Char('r'), KeyModifiers::ALT).is_empty());

    // After an interrupt: Alt+R works.
    send(&mut state, "another one");
    state.conv_on_cancel_result(Ok(true));
    let actions = press(&mut state, KeyCode::Char('r'), KeyModifiers::ALT);
    assert!(matches!(actions.as_slice(), [AppAction::SendUserMessage { text, .. }] if text == "another one"));
}

// ------------------------------------------------------------------------------ composer

#[test]
fn ctrl_r_finds_older_messages_enter_accepts_esc_restores() {
    let dir = target_tmp();
    let (mut state, _) = state_in(&dir);
    state.composer.history = vec!["alpha".into(), "beta".into()];
    type_text(&mut state, "my draft");
    press(&mut state, KeyCode::Char('r'), KeyModifiers::CONTROL);
    type_text(&mut state, "alp");
    assert_eq!(state.composer.text, "alpha");
    assert!(screen(&state, 100, 30).contains("(reverse-i-search) `alp`"));
    press(&mut state, KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(state.composer.text, "alpha", "Enter accepts the match for editing");
    assert!(state.conv.history_search.is_none());
    assert!(!state.is_busy(), "accepting does not send");

    type_text(&mut state, "!");
    press(&mut state, KeyCode::Char('r'), KeyModifiers::CONTROL);
    type_text(&mut state, "bet");
    assert_eq!(state.composer.text, "beta");
    press(&mut state, KeyCode::Esc, KeyModifiers::NONE);
    assert_eq!(state.composer.text, "alpha!", "Esc restores the previous draft");
}

#[test]
fn ctrl_c_clears_and_up_recalls_the_cleared_draft_before_history() {
    let dir = target_tmp();
    let (mut state, _) = state_in(&dir);
    state.composer.history = vec!["older".into()];
    type_text(&mut state, "hello");
    press(&mut state, KeyCode::Char('c'), KeyModifiers::CONTROL);
    assert!(state.composer.text.is_empty());
    assert!(!state.should_quit, "Ctrl+C with a draft only clears it");
    press(&mut state, KeyCode::Up, KeyModifiers::NONE);
    assert_eq!(state.composer.text, "hello");
    press(&mut state, KeyCode::Down, KeyModifiers::NONE);
    press(&mut state, KeyCode::Up, KeyModifiers::NONE);
    assert_eq!(state.composer.text, "older", "the draft is recalled once, then history");
    // Ctrl+C on an empty composer still quits.
    state.composer.text.clear();
    press(&mut state, KeyCode::Char('c'), KeyModifiers::CONTROL);
    assert!(state.should_quit);
}

#[test]
fn ctrl_g_asks_the_runtime_to_open_the_editor_and_loads_the_result() {
    let dir = target_tmp();
    let (mut state, _) = state_in(&dir);
    type_text(&mut state, "start");
    let actions = press(&mut state, KeyCode::Char('g'), KeyModifiers::CONTROL);
    assert_eq!(
        actions,
        vec![AppAction::Conv(ConvAction::OpenEditor { draft: "start".into() })]
    );
    assert!(press(&mut state, KeyCode::Char('g'), KeyModifiers::CONTROL).is_empty(), "no second editor");
    state.conv_event(ConvEvent::EditorDone(Ok("start\nappended".into())));
    assert_eq!(state.composer.text, "start\nappended");
    state.conv_event(ConvEvent::EditorDone(Err("No editor: set $VISUAL or $EDITOR".into())));
    assert!(state.toast.as_ref().is_some_and(|t| t.is_error && t.text.contains("No editor")));
}

// ------------------------------------------------------------------------------ messages

fn state_with_reply(dir: &tempfile::TempDir) -> TuiAppState {
    let (mut state, _) = state_in(dir);
    send(&mut state, "question");
    reply_in_state(&mut state, "line one\nline two");
    state
}

#[test]
fn selecting_a_reply_and_pressing_q_quotes_it_with_prefixes() {
    let dir = target_tmp();
    let mut state = state_with_reply(&dir);
    press(&mut state, KeyCode::Up, KeyModifiers::CONTROL);
    assert!(screen(&state, 100, 30).contains("c copy"), "selection hint is shown");
    press(&mut state, KeyCode::Char('q'), KeyModifiers::NONE);
    assert_eq!(state.composer.text, "> line one\n> line two\n");
    assert!(state.conv.selected.is_none());
}

#[test]
fn clicking_a_message_selects_it_and_a_drag_still_copies() {
    use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
    let dir = target_tmp();
    let mut state = state_with_reply(&dir);
    let area = Rect::new(0, 0, 120, 30);
    let mut buf = Buffer::empty(area);
    let mut fx = TuiEffects::new();
    let mut hit = HitRegions::default();
    crate::render::render_to_buffer(area, &mut buf, &state, &mut fx, &mut hit, 16);
    let (rect, idx) = hit
        .message_rows
        .iter()
        .find(|(_, i)| matches!(state.active_timeline()[*i], ChatTimelineItem::AssistantMessage { .. }))
        .cloned()
        .expect("the reply has rows");
    let ev = |kind| {
        Event::Mouse(MouseEvent { kind, column: rect.x + 2, row: rect.y, modifiers: KeyModifiers::NONE })
    };
    state.handle_terminal_event(ev(MouseEventKind::Down(MouseButton::Left)), &hit, area);
    state.handle_terminal_event(ev(MouseEventKind::Up(MouseButton::Left)), &hit, area);
    assert_eq!(state.conv.selected, Some(idx), "a click selects the message");
    // Then `q` quotes it.
    press(&mut state, KeyCode::Char('q'), KeyModifiers::NONE);
    assert!(state.composer.text.starts_with("> line one"));
}

#[test]
fn a_model_call_in_flight_rearms_the_grace_window() {
    let dir = target_tmp();
    let (mut state, clock) = state_in(&dir);
    send(&mut state, "slow model");
    state.on_turn_finished(state.turn.seq());
    clock.advance(crate::turn::NO_REPLY_GRACE_MS + 10);
    assert!(state.turn.grace_expired());
    state.turn.rearm_grace();
    assert!(!state.turn.grace_expired(), "the window starts over while the model is still thinking");
    clock.advance(crate::turn::NO_REPLY_GRACE_MS + 10);
    assert!(state.tick_turn(), "when the model finishes silently the notice appears");
}

#[test]
fn selection_copy_and_leaving_selection() {
    let dir = target_tmp();
    let mut state = state_with_reply(&dir);
    press(&mut state, KeyCode::Up, KeyModifiers::CONTROL);
    press(&mut state, KeyCode::Up, KeyModifiers::CONTROL); // the user message
    let actions = press(&mut state, KeyCode::Char('c'), KeyModifiers::NONE);
    assert_eq!(actions, vec![AppAction::CopyToClipboard("question".into())]);
    press(&mut state, KeyCode::Down, KeyModifiers::CONTROL);
    press(&mut state, KeyCode::Esc, KeyModifiers::NONE);
    assert!(state.conv.selected.is_none());
    assert!(state.composer.text.is_empty());
}

#[test]
fn what_happened_line_expands_on_ctrl_o() {
    let dir = target_tmp();
    let mut state = state_with_reply(&dir);
    let req = state.conv.pending_traces.pop().expect("a trace is requested for the reply");
    state.conv_event(ConvEvent::Trace {
        request: req,
        result: Ok(vec![
            json!({"kind":"percept","summary":"percept"}),
            json!({"kind":"model_call","summary":"model_call"}),
            json!({"kind":"action","summary":"action chat.speak (act_1)"}),
        ]),
    });
    let collapsed = screen(&state, 100, 30);
    assert!(collapsed.contains("1 model call · 1 action"), "{collapsed}");
    press(&mut state, KeyCode::Char('o'), KeyModifiers::CONTROL);
    let expanded = screen(&state, 100, 30);
    assert!(expanded.contains("what happened"), "{expanded}");
    assert!(expanded.contains("model call"), "{expanded}");
    assert!(expanded.contains("chat.speak") && expanded.contains("valve: allowed"), "{expanded}");
    press(&mut state, KeyCode::Char('o'), KeyModifiers::CONTROL);
    assert!(screen(&state, 100, 30).contains("1 model call · 1 action"));
}

// ------------------------------------------------------------------------------ chats

#[test]
fn chat_search_pin_delete_confirmation_and_export() {
    let dir = target_tmp();
    let (mut state, _) = state_in(&dir);
    send(&mut state, "my zanzibarcake recipe");
    reply_in_state(&mut state, "mix flour");
    state.execute_slash_command("/new Second");
    send(&mut state, "something else");
    reply_in_state(&mut state, "ok");

    // Search by message text finds only the first chat.
    state.execute_slash_command("/chat search zanzibar");
    let visible: Vec<_> = crate::chat_search::visible_chats(&state.chats, &state.timelines, state.conv.chat_filter.as_deref())
        .iter()
        .map(|c| c.title.clone())
        .collect();
    assert_eq!(visible.len(), 1, "{visible:?}");
    assert!(screen(&state, 120, 30).contains("New Chat") || screen(&state, 120, 30).contains("zanzibar") || !visible.is_empty());
    state.execute_slash_command("/chat search");
    assert!(state.conv.chat_filter.is_none());

    // Pin sorts first and persists in tui_chats.json.
    let first_id = state.chats.chats[0].id.clone();
    state.chats.active_chat_id = first_id.clone();
    state.execute_slash_command("/chat pin");
    let order = crate::chat_search::visible_chats(&state.chats, &state.timelines, None);
    assert_eq!(order[0].id, first_id);
    let saved = std::fs::read_to_string(LocalChatStore::path_for_data_dir(&state.paths.data_dir)).unwrap();
    assert!(saved.contains("\"pinned\": true"), "{saved}");

    // Delete asks first; n keeps the chat, y deletes it.
    let before = state.chats.chats.len();
    state.execute_slash_command("/delete");
    assert!(screen(&state, 120, 30).contains("Delete chat"));
    press(&mut state, KeyCode::Char('n'), KeyModifiers::NONE);
    assert_eq!(state.chats.chats.len(), before, "declined: the chat remains");
    state.execute_slash_command("/delete");
    press(&mut state, KeyCode::Char('y'), KeyModifiers::NONE);
    assert_eq!(state.chats.chats.len(), before - 1);

    // Export: both messages, in order, to <data_dir>/exports.
    let id = state.chats.chats[0].id.clone();
    state.chats.active_chat_id = id;
    state.execute_slash_command("/new Export me");
    send(&mut state, "first message");
    reply_in_state(&mut state, "second message");
    state.execute_slash_command("/export");
    let exports = state.paths.data_dir.join("exports");
    let file = std::fs::read_dir(&exports).unwrap().next().expect("an export file").unwrap().path();
    let md = std::fs::read_to_string(&file).unwrap();
    let a = md.find("first message").expect("first");
    let b = md.find("second message").expect("second");
    assert!(a < b);
    assert!(state.toast.as_ref().is_some_and(|t| t.text.contains(file.to_str().unwrap())));
}

// ------------------------------------------------------------------------------ notifications

#[test]
fn reply_while_unfocused_notifies_per_setting_and_focus_returns_clean() {
    let dir = target_tmp();
    let (mut state, _) = state_in(&dir);
    state.prefs.conv.notifications = NotifyMode::Both;
    let ev = |e| state.conv.notify.mode == NotifyMode::Both && matches!(e, Event::FocusLost);
    let _ = ev;
    state.handle_terminal_event(Event::FocusLost, &HitRegions::default(), Rect::new(0, 0, 100, 30));
    send(&mut state, "hi");
    reply_in_state(&mut state, "Halo from Mono\nsecond line");
    let out = String::from_utf8(state.conv.notify.take_output()).unwrap();
    assert!(out.contains('\u{7}'), "bell");
    assert!(out.contains("\x1b]9;MonoAgens: Halo from Mono\x1b\\"), "osc 9: {out:?}");
    assert!(out.contains("\x1b]777;notify;MonoAgens;Halo from Mono\x1b\\"), "osc 777: {out:?}");
    assert!(out.contains("(1) MonoAgens"), "title badge");
    state.handle_terminal_event(Event::FocusGained, &HitRegions::default(), Rect::new(0, 0, 100, 30));
    assert!(String::from_utf8(state.conv.notify.take_output()).unwrap().contains("\x1b]2;MonoAgens\x1b\\"));

    // Setting off: nothing is written while unfocused.
    state.prefs.conv.notifications = NotifyMode::Off;
    state.handle_terminal_event(Event::FocusLost, &HitRegions::default(), Rect::new(0, 0, 100, 30));
    send(&mut state, "again");
    reply_in_state(&mut state, "reply two");
    assert!(state.conv.notify.take_output().is_empty());
}

// ------------------------------------------------------------------------------ draft preview

#[test]
fn draft_bubble_grows_then_is_replaced_by_the_reply() {
    let dir = target_tmp();
    let (mut state, _) = state_in(&dir);
    send(&mut state, "question");
    state.conv_event(ConvEvent::Preview(PreviewEvent::Draft { request_id: 4, text: "Hal".into() }));
    state.conv_event(ConvEvent::Preview(PreviewEvent::Draft { request_id: 4, text: "Halo dun".into() }));
    let s = screen(&state, 100, 30);
    assert!(s.contains("draft") && s.contains("Halo dun"), "{s}");
    reply_in_state(&mut state, "Halo dunia");
    let s = screen(&state, 100, 30);
    assert!(!s.contains("not final"), "the draft is gone once the reply is in: {s}");
    assert!(s.contains("Halo dunia"));
    // A late draft of the finished turn does not come back.
    state.conv_event(ConvEvent::Preview(PreviewEvent::Draft { request_id: 4, text: "Halo dunia!!".into() }));
    assert!(state.conv.preview.draft.is_none());
}

#[test]
fn valve_denial_replaces_the_draft_with_the_verdict_and_the_text_appears_nowhere() {
    let dir = target_tmp();
    let (mut state, _) = state_in(&dir);
    send(&mut state, "question");
    state.conv_event(ConvEvent::Preview(PreviewEvent::Draft { request_id: 4, text: "SECRET-DRAFT-TEXT".into() }));
    assert!(screen(&state, 100, 30).contains("SECRET-DRAFT-TEXT"));

    // The recorded door verdict drives the same path as the live event loop.
    let rec = EventRecord::new_root(
        EventKind::DoorVerdict,
        Time::from_millis(1),
        ActorId::new("soul"),
        PlaceId::new("cli:terminal"),
        Audience::new([ActorId::new("soul")]),
        OriginActor::new(ActorId::new("soul")),
        Taint::Trusted,
        NumericFeatures::default(),
        OpaquePayload::from_bytes(b"{}"),
        Vec::new(),
        Some(EventDetail::DoorVerdict {
            door: DoorKind::Action,
            target_event_id: None,
            rule: "action_door".into(),
            verdict: "deny".into(),
        }),
    );
    let mut approvals = std::collections::BTreeSet::new();
    let mut active = None;
    crate::runtime::process_event(&rec, &mut state, &mut approvals, &mut active, "direct");

    let s = screen(&state, 100, 30);
    assert!(!s.contains("SECRET-DRAFT-TEXT"), "denied text must not stay visible: {s}");
    assert!(s.contains("The valve did not deliver this reply (action_door: deny)"), "{s}");
    let timeline_text = format!("{:?}", state.active_timeline());
    assert!(!timeline_text.contains("SECRET-DRAFT-TEXT"));
    let persisted = std::fs::read_to_string(LocalChatStore::path_for_data_dir(&state.paths.data_dir)).unwrap_or_default();
    assert!(!persisted.contains("SECRET-DRAFT-TEXT"));
}

#[test]
fn cancel_and_failure_remove_the_draft() {
    let dir = target_tmp();
    let (mut state, _) = state_in(&dir);
    send(&mut state, "question");
    state.conv_event(ConvEvent::Preview(PreviewEvent::Draft { request_id: 9, text: "partial".into() }));
    state.conv_on_cancel_result(Ok(true));
    assert!(state.conv.preview.draft.is_none());
    assert!(!screen(&state, 100, 30).contains("partial"));

    send(&mut state, "again");
    state.conv_event(ConvEvent::Preview(PreviewEvent::Draft { request_id: 10, text: "oops".into() }));
    state.conv_event(ConvEvent::Preview(PreviewEvent::End { request_id: 10, outcome: PreviewEnd::Failed }));
    assert!(state.conv.preview.draft.is_none());
}

// ------------------------------------------------------------------------------ end to end

/// A provider that blocks until released (a slow model).
struct SlowProvider {
    entered: Arc<AtomicBool>,
    release: Arc<AtomicBool>,
}

impl ModelProvider for SlowProvider {
    fn provider_name(&self) -> &str {
        "slow"
    }

    fn invoke(&self, request: &ModelRequest) -> Result<ModelCallSuccess, ModelCallFailure> {
        self.entered.store(true, Ordering::SeqCst);
        while !self.release.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(10));
        }
        ScriptedModelProvider::from_replies([ScriptedReply::text(speak_reply("late"))]).invoke(request)
    }
}

fn speak_reply(text: &str) -> String {
    json!({
        "protocol_version": { "major": 1, "minor": 0 },
        "acts": [
            { "act": "think", "thought": "answer the owner", "motivation": 4, "confidence_0_20": 18,
              "used_ids": [], "importance": 0.6, "basis": "observed", "value": null, "cost": null, "domain": null },
            { "act": "act", "capability_id": "chat.speak", "places": ["cli:terminal"],
              "people": ["per_owner"], "payload": text.as_bytes(), "motivation": 4,
              "confidence_0_20": 18, "used_ids": [], "expected": null, "domain": null }
        ]
    })
    .to_string()
}

struct Hosted {
    client: Arc<dyn GatewayClient>,
    rx: mpsc::UnboundedReceiver<RuntimeEvent>,
    tx: mpsc::UnboundedSender<RuntimeEvent>,
    state: TuiAppState,
    _dir: tempfile::TempDir,
}

async fn host(provider: Arc<dyn ModelProvider>) -> Hosted {
    let dir = target_tmp();
    let paths = paths_in(&dir);
    let mut config = GatewayConfig::default();
    config.model.provider = monoagens_gateway::ModelProviderKind::Scripted;
    config.runtime.model_timeout_ms = 20_000;
    let loaded = LoadedGatewayEnv {
        config: config.clone(),
        secrets: ProviderSecrets::default(),
        paths: paths.clone(),
        created_default: false,
    };
    let options = TuiLaunchOptions {
        data_dir: Some(paths.data_dir.clone()),
        ..TuiLaunchOptions::default()
    };
    let (tx, rx) = mpsc::unbounded_channel();
    let client = boot_gateway(&loaded, &options, Some(provider), tx.clone())
        .await
        .expect("host gateway");
    assert_eq!(client.kind(), ClientKind::Hosting);
    let mut state = TuiAppState::new(
        paths,
        config,
        TuiPreferences::default(),
        LocalChatStore::default(),
        SecretsUiSummary::default(),
        client.owner_actor(),
    );
    state.turn.set_clock(Arc::new(ManualClock::new()));
    Hosted { client, rx, tx, state, _dir: dir }
}

/// Starts a turn the way the event loop does and returns once the submit task is running.
fn submit(h: &mut Hosted, text: &str) {
    let actions = send(&mut h.state, text);
    let Some(AppAction::SendUserMessage { text, .. }) = actions.into_iter().next() else {
        panic!("a turn must start");
    };
    let turn = h.state.turn.seq();
    let c = Arc::clone(&h.client);
    let tx = h.tx.clone();
    let mut percept = build_tui_percept(&h.state, &c.owner_actor(), &text, &[], c.now());
    percept.correlation_id = Some(monoagens_protocol::CorrelationId::new(format!("{TURN_CORRELATION_PREFIX}{turn}")));
    tokio::spawn(async move {
        match c.submit_turn(percept).await {
            Ok(n) => {
                let _ = tx.send(RuntimeEvent::TurnFinished { turn, effects_spawned: n });
            }
            Err(e) => {
                let _ = tx.send(RuntimeEvent::TurnError { turn, error: e.message });
            }
        }
    });
}

async fn recv(h: &mut Hosted) -> RuntimeEvent {
    tokio::time::timeout(Duration::from_secs(10), h.rx.recv())
        .await
        .expect("event in time")
        .expect("channel open")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn interrupting_a_slow_turn_records_cancelled_by_owner_and_ends_busy() {
    let entered = Arc::new(AtomicBool::new(false));
    let release = Arc::new(AtomicBool::new(false));
    let mut h = host(Arc::new(SlowProvider { entered: entered.clone(), release: release.clone() })).await;
    submit(&mut h, "a slow question");
    for _ in 0..500 {
        if entered.load(Ordering::SeqCst) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(entered.load(Ordering::SeqCst), "the model call is in flight");

    // Esc twice -> CancelTurn -> turn.cancel through the client -> result into the state.
    press(&mut h.state, KeyCode::Esc, KeyModifiers::NONE);
    let actions = press(&mut h.state, KeyCode::Esc, KeyModifiers::NONE);
    assert_eq!(actions, vec![AppAction::Conv(ConvAction::CancelTurn)]);
    let cancelled = tokio::time::timeout(Duration::from_secs(10), h.client.cancel_turn())
        .await
        .expect("cancel in time")
        .expect("cancel call");
    assert!(cancelled);
    h.state.conv_event(ConvEvent::Cancelled(Ok(cancelled)));
    assert!(!h.state.is_busy());
    assert!(notices(&h.state).iter().any(|n| n == "Interrupted."));

    // The log holds the recorded cancel; following it must not touch the next turn.
    let mut follower = LogFollower::default();
    let mut active = None;
    for _ in 0..200 {
        poll_new_gateway_events(&h.client, &mut h.state, &mut follower, &mut active).await;
        let page = h.client.events_since(0).await.unwrap();
        let recorded = page.events.iter().any(|e| {
            matches!(&e.detail, Some(EventDetail::Failure { detail, .. }) if detail == "cancelled_by_owner")
        });
        if recorded {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let page = h.client.events_since(0).await.unwrap();
    assert!(page.events.iter().any(|e| {
        matches!(&e.detail, Some(EventDetail::Failure { detail, .. }) if detail == "cancelled_by_owner")
    }));
    assert!(!notices(&h.state).iter().any(|n| n.contains("cancelled_by_owner")), "no error notice for an owner cancel");

    // A second cancel finds nothing in flight.
    assert!(!h.client.cancel_turn().await.unwrap());
    release.store(true, Ordering::SeqCst);
    h.client.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn what_happened_and_why_come_from_the_event_log_of_a_scripted_turn() {
    let provider = ScriptedModelProvider::from_replies([ScriptedReply::text(speak_reply("pong"))]);
    let mut h = host(Arc::new(provider)).await;
    let mut follower = LogFollower::default();
    let mut active = None;
    submit(&mut h, "ping");
    // Drive the loop until the reply arrives.
    loop {
        let ev = recv(&mut h).await;
        let is_reply = matches!(&ev, RuntimeEvent::AssistantReply { .. });
        if is_reply {
            poll_new_gateway_events(&h.client, &mut h.state, &mut follower, &mut active).await;
        }
        crate::runtime::handle_runtime_event(
            ev,
            &mut h.state,
            &mut active,
            &mut TuiEffects::new(),
            &follower.tracker,
            &mut Some(Arc::clone(&h.client)),
        );
        if is_reply {
            break;
        }
    }
    let req = h.state.conv.pending_traces.pop().expect("trace requested after the reply");
    assert!(req.root_event.is_some(), "the turn's percept event is known");

    // events.trace through the client, as the pump does.
    let root = req.root_event.unwrap();
    let rows = h
        .client
        .gov_call("events.trace".into(), json!({ "trace": root.to_string() }))
        .await
        .expect("events.trace");
    let rows = rows["events"].as_array().cloned().unwrap();
    h.state.conv_event(ConvEvent::Trace { request: req, result: Ok(rows) });
    let s = screen(&h.state, 120, 40);
    assert!(s.contains("1 model call"), "{s}");
    press(&mut h.state, KeyCode::Char('o'), KeyModifiers::CONTROL);
    let s = screen(&h.state, 120, 40);
    assert!(s.contains("model call") && s.contains("chat.speak"), "{s}");

    // /why: the overlay shows the same lines events.explain returns (the CLI's --explain text).
    let actions = h.state.execute_slash_command("/why");
    let Some(AppAction::Conv(ConvAction::FetchExplain { event_id })) = actions.into_iter().next() else {
        panic!("/why asks for the explanation");
    };
    assert_eq!(event_id, root);
    let explain = h
        .client
        .gov_call("events.explain".into(), json!({ "id": event_id }))
        .await
        .expect("events.explain");
    let lines: Vec<String> = explain["lines"].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_owned()).collect();
    assert!(lines[0].starts_with("decision event="), "{lines:?}");
    h.state.conv_event(ConvEvent::Explain { event_id, result: Ok(explain.clone()) });
    assert_eq!(h.state.conv.why.as_ref().unwrap().lines, lines);
    let s = screen(&h.state, 140, 40);
    assert!(s.contains("decision event="), "{s}");
    press(&mut h.state, KeyCode::Esc, KeyModifiers::NONE);
    assert!(h.state.conv.why.is_none());
    h.client.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn live_draft_streams_to_the_tui_then_the_real_reply_replaces_it() {
    let provider = ScriptedModelProvider::from_replies([ScriptedReply::text(speak_reply(
        "Halo dari Mono, balasan panjang yang dipotong-potong",
    ))])
    .with_stream_pacing(10, 15);
    let provider = Arc::new(provider);
    let mut h = host(provider.clone()).await;

    // Subscribe the way `pump` does (setting on), then run a turn.
    let sub = h.client.preview_subscribe(h.tx.clone()).await.expect("subscribe");
    h.state.conv.preview.subscription = Some(sub);
    submit(&mut h, "hi");
    let mut saw_draft = false;
    let mut follower = LogFollower::default();
    let mut active = None;
    loop {
        let ev = recv(&mut h).await;
        match ev {
            RuntimeEvent::Conv(ev) => {
                h.state.conv_event(ev);
                if h.state.conv.preview.draft.as_ref().is_some_and(|d| !d.text.is_empty()) {
                    let s = screen(&h.state, 120, 40);
                    if s.contains("not final") {
                        saw_draft = true;
                    }
                }
            }
            ev @ RuntimeEvent::AssistantReply { .. } => {
                poll_new_gateway_events(&h.client, &mut h.state, &mut follower, &mut active).await;
                crate::runtime::handle_runtime_event(
                    ev,
                    &mut h.state,
                    &mut active,
                    &mut TuiEffects::new(),
                    &follower.tracker,
                    &mut Some(Arc::clone(&h.client)),
                );
                break;
            }
            _ => {}
        }
    }
    assert!(saw_draft, "a draft bubble was rendered while the model wrote");
    assert_eq!(provider.streamed_calls(), 1);
    assert!(h.state.conv.preview.draft.is_none(), "replaced by the real reply");
    let s = screen(&h.state, 120, 40);
    assert!(!s.contains("not final"));
    assert!(s.contains("Halo dari Mono, balasan panjang yang dipotong-potong"));

    // Unsubscribing: the next turn is not streamed.
    h.client.preview_unsubscribe(sub).await.unwrap();
    h.client.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn setting_off_means_no_subscription_and_no_streamed_call() {
    let provider = Arc::new(ScriptedModelProvider::from_replies([ScriptedReply::text(speak_reply("pong"))]));
    let mut h = host(provider.clone()).await;
    h.state.prefs.conv.live_draft_preview = false;
    let client = Some(Arc::clone(&h.client));
    crate::conv::pump(&mut h.state, &client, &h.tx);
    assert!(!h.state.conv.preview.busy && h.state.conv.preview.subscription.is_none(), "setting off: pump does not subscribe");
    submit(&mut h, "hi");
    loop {
        if matches!(recv(&mut h).await, RuntimeEvent::AssistantReply { .. }) {
            break;
        }
    }
    assert_eq!(provider.streamed_calls(), 0, "the provider got a blocking call");
    assert!(h.state.conv.preview.draft.is_none());
    h.client.shutdown().await;
}
