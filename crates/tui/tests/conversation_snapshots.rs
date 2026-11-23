//! Snapshots of the conversation views (`tui-conversation`): what-happened block with a selected
//! message, the draft bubble with the interrupt prompt, and the `/why` overlay.

use std::sync::Arc;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use monoagens_gateway::{GatewayConfig, PreviewEvent, ResolvedPaths};
use monoagens_protocol::ActorId;
use monoagens_tui::conv::ConvEvent;
use monoagens_tui::message_actions::{TurnSummary, WhyOverlay};
use monoagens_tui::turn::ManualClock;
use monoagens_tui::{
    ChatTimelineItem, HitRegions, LocalChatStore, SecretsUiSummary, TuiAppState, TuiEffects,
    TuiPreferences, render_to_buffer,
};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use serde_json::json;
use tempfile::TempDir;
use unicode_width::UnicodeWidthStr;

fn make_state(dir: &TempDir) -> TuiAppState {
    let cfg_dir = dir.path().join("config");
    let data_dir = dir.path().join("data");
    std::fs::create_dir_all(&cfg_dir).unwrap();
    std::fs::create_dir_all(&data_dir).unwrap();
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
    state.turn.set_clock(Arc::new(ManualClock::new()));
    state
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

fn chat_with_reply(dir: &TempDir) -> TuiAppState {
    let mut state = make_state(dir);
    let id = state.chats.active_chat_id.clone();
    state.push_timeline_item(
        &id,
        ChatTimelineItem::UserMessage {
            sender: "owner".into(),
            text: "Explain the valve".into(),
            timestamp_ms: 0,
            is_group: false,
        },
    );
    state.push_timeline_item(
        &id,
        ChatTimelineItem::AssistantMessage {
            text: "The valve checks every action.".into(),
            duration_ms: None,
            timestamp_ms: 0,
        },
    );
    state
}

fn what_happened(state: &mut TuiAppState, expanded: bool) {
    let rows = vec![
        json!({"kind":"wake_started","summary":"wake_started"}),
        json!({"kind":"action","summary":"action chat.speak (act-1)"}),
    ];
    let id = state.chats.active_chat_id.clone();
    state
        .conv
        .summaries
        .insert((id, 1), TurnSummary::from_rows(1, Some(6), &rows, expanded));
}

fn snap(state: &TuiAppState) -> (String, String) {
    (text_of(&render(80, 24, state)), text_of(&render(120, 40, state)))
}

#[test]
fn snapshot_what_happened_expanded_with_selection() {
    let dir = TempDir::new_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    let mut state = chat_with_reply(&dir);
    what_happened(&mut state, true);
    state.handle_key_event(KeyEvent::new(KeyCode::Up, KeyModifiers::CONTROL));
    let (a, b) = snap(&state);
    insta::assert_snapshot!("what_happened_80x24", a);
    insta::assert_snapshot!("what_happened_120x40", b);
}

#[test]
fn snapshot_draft_bubble_and_interrupt_prompt() {
    let dir = TempDir::new_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    let mut state = chat_with_reply(&dir);
    state.composer.text = "next question".into();
    state.handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    state.conv_event(ConvEvent::Preview(PreviewEvent::Draft {
        request_id: 3,
        text: "Halo, ini draft balasan yang sedang ditulis model".into(),
    }));
    state.handle_key_event(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    let (a, b) = snap(&state);
    insta::assert_snapshot!("draft_80x24", a);
    insta::assert_snapshot!("draft_120x40", b);
}

#[test]
fn snapshot_why_overlay() {
    let dir = TempDir::new_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    let mut state = chat_with_reply(&dir);
    let mut why = WhyOverlay::loading("event #6");
    why.set_explain(&json!({"lines": [
        "decision event=7 target=6 outcome=Go salience=0.900000 threshold=0.500000 probability=0.950000 draw=0.100000 reason=Some(Addressed)",
        "  door_verdict door=action rule=allow reason_tag=ok"
    ]}));
    state.conv.why = Some(why);
    let (a, b) = snap(&state);
    insta::assert_snapshot!("why_80x24", a);
    insta::assert_snapshot!("why_120x40", b);
}
