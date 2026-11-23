//! Conversation features of the TUI (change `tui-conversation`): turn control (interrupt,
//! retry), composer history search / external editor / cleared-draft recall, the per-turn
//! "what happened" block and message actions, chat search / pin / delete confirmation /
//! export, notifications and the live draft preview.
//!
//! Everything hangs off one state field, [`ConvState`] (`TuiAppState::conv`), one key hook
//! (`TuiAppState::conv_key`), one slash hook (`conv_slash`), one action (`AppAction::Conv`) and
//! one runtime event (`RuntimeEvent::Conv`), so the shared files only gain small additive hooks.

use std::collections::HashMap;
use std::sync::Arc;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use monoagens_gateway::{PreviewEnd, PreviewEvent};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::Line;
use serde_json::{Value, json};
use tokio::sync::mpsc;

use crate::attachments::PendingAttachment;
use crate::catalog_ui::pad;
use crate::chat_search;
use crate::client::{GatewayClient, RuntimeEvent};
use crate::composer_history::{self, HistorySearch};
use crate::draft_preview::DraftState;
use crate::message_actions::{self, TurnSummary, WhyOverlay};
use crate::notify::{NotifyMode, Notifier};
use crate::state::{AppAction, ChatTimelineItem, TuiAppState};
use crate::theme::ThemePalette;

/// A second Esc within this window interrupts the running turn.
pub const ESC_INTERRUPT_WINDOW_MS: u64 = 1_500;

/// Notice shown under the user message after a successful interrupt.
pub const INTERRUPTED_NOTICE: &str = "Interrupted.";

/// Notice shown when `turn.cancel` found no model call to stop.
pub const NOT_STOPPABLE_NOTICE: &str =
    "The turn could not be stopped (no model call is in flight); it will finish on its own.";

/// How the last turn ended (retry is only offered after an error or an interrupt).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnStatus {
    Running,
    Ok,
    Error,
    Interrupted,
}

/// What `F5` re-submits.
#[derive(Debug, Clone)]
pub struct LastTurn {
    pub chat_id: String,
    pub place_id: String,
    pub text: String,
    pub attachments: Vec<PendingAttachment>,
    pub status: TurnStatus,
}

/// A trace to fetch for the what-happened block (drained by [`pump`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraceRequest {
    pub chat_id: String,
    pub item_idx: usize,
    pub turn: u64,
    pub root_event: Option<u64>,
}

/// All conversation state.
#[derive(Debug, Clone, Default)]
pub struct ConvState {
    /// Turn-clock time of the first Esc while busy.
    pub esc_armed_ms: Option<u64>,
    pub last_turn: Option<LastTurn>,
    pub history_search: Option<HistorySearch>,
    /// Composer text removed by Ctrl+C / Esc, recalled by Up on an empty composer.
    pub cleared_draft: Option<String>,
    /// Selected message (index into the active chat's timeline).
    pub selected: Option<usize>,
    pub why: Option<WhyOverlay>,
    /// What-happened summaries keyed by (chat id, index of the reply in the timeline).
    pub summaries: HashMap<(String, usize), TurnSummary>,
    pub pending_traces: Vec<TraceRequest>,
    /// Sidebar filter text.
    pub chat_filter: Option<String>,
    /// The filter prompt is taking keys.
    pub chat_search_input: bool,
    /// Chat waiting for the `y`/`n` answer of the delete confirmation.
    pub delete_confirm: Option<String>,
    pub notify: Notifier,
    pub preview: DraftState,
    /// `(chat id, index)` of the newest assistant message pushed (reply routing).
    pub last_reply: Option<(String, usize)>,
    /// The editor is running (the next Ctrl+G is ignored until it returns).
    pub editor_open: bool,
    /// `(explain event id)` requested for the open `/why` overlay.
    pub pending_explain: Option<u64>,
}

/// Work for the async runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConvAction {
    /// Esc twice: `turn.cancel`.
    CancelTurn,
    /// Ctrl+G: edit `draft` in `$VISUAL` / `$EDITOR`.
    OpenEditor { draft: String },
    /// Fetch the trace of a finished turn for its what-happened block.
    FetchTrace(TraceRequest),
    /// Fetch the valve explanation of an event for `/why`.
    FetchExplain { event_id: u64 },
    /// Subscribe to / unsubscribe from the owner-only draft preview.
    PreviewSubscribe,
    PreviewUnsubscribe(u64),
}

/// Results coming back into the event loop.
#[derive(Debug)]
pub enum ConvEvent {
    Preview(PreviewEvent),
    PreviewSubscribed(Result<u64, String>),
    PreviewUnsubscribed,
    Trace { request: TraceRequest, result: Result<Vec<Value>, String> },
    Explain { event_id: u64, result: Result<Value, String> },
    EditorDone(Result<String, String>),
    Cancelled(Result<bool, String>),
}

impl ConvState {
    /// The "press Esc again" prompt is visible at turn-clock time `now_ms`.
    #[must_use]
    pub fn esc_prompt_visible(&self, now_ms: u64) -> bool {
        self.esc_armed_ms
            .is_some_and(|t| now_ms.saturating_sub(t) < ESC_INTERRUPT_WINDOW_MS)
    }

    /// Whether any modal input of this module is active (it owns the keyboard).
    #[must_use]
    pub fn modal(&self) -> bool {
        self.history_search.is_some()
            || self.chat_search_input
            || self.delete_confirm.is_some()
            || self.why.is_some()
    }
}

fn ctrl(key: KeyEvent, c: char) -> bool {
    key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char(c)
}

impl TuiAppState {
    /// True while one of the big overlays (not the chat) owns the keyboard.
    fn conv_chat_blocked(&self) -> bool {
        self.log.open
            || self.catalog.is_open()
            || self.gov.is_open()
            || self.wizard.is_some()
            || self.settings.open
    }

    /// Key hook, called first by `handle_key_event`. `Some(actions)` consumes the key.
    pub fn conv_key(&mut self, key: KeyEvent) -> Option<Vec<AppAction>> {
        if self.conv_chat_blocked() {
            return None;
        }
        if let Some(r) = self.conv_modal_key(key) {
            return Some(r);
        }
        // Ctrl+C: clear a non-empty draft and remember it; an empty composer still quits.
        if ctrl(key, 'c') && !self.composer.text.is_empty() {
            self.conv_clear_draft();
            return Some(Vec::new());
        }
        let busy = self.is_busy();
        match key.code {
            KeyCode::Esc if busy => return Some(self.conv_esc_while_busy()),
            KeyCode::Esc if self.conv.selected.is_some() => {
                self.conv.selected = None;
                return Some(Vec::new());
            }
            KeyCode::Esc if !self.composer.text.is_empty() && !self.composer.is_slash_mode() => {
                // The composer is about to be cleared: remember it (no consume).
                self.conv.cleared_draft = Some(self.composer.text.clone());
            }
            KeyCode::F(5) => return Some(self.conv_retry()),
            KeyCode::Char('r') | KeyCode::Char('R') if key.modifiers.contains(KeyModifiers::ALT) => {
                return Some(self.conv_retry());
            }
            KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) && self.config_error.is_none() => {
                self.conv_start_history_search();
                return Some(Vec::new());
            }
            KeyCode::Char('g') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if self.conv.editor_open {
                    return Some(Vec::new());
                }
                self.conv.editor_open = true;
                return Some(vec![AppAction::Conv(ConvAction::OpenEditor {
                    draft: self.composer.text.clone(),
                })]);
            }
            KeyCode::Char('o') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return Some(self.conv_toggle_summary());
            }
            KeyCode::Char('f') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.conv.chat_search_input = true;
                return Some(Vec::new());
            }
            KeyCode::Up if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.conv_move_selection(-1);
                return Some(Vec::new());
            }
            KeyCode::Down if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.conv_move_selection(1);
                return Some(Vec::new());
            }
            KeyCode::Up
                if key.modifiers.is_empty()
                    && self.composer.text.is_empty()
                    && self.composer.history_idx.is_none()
                    && self.conv.cleared_draft.is_some()
                    && self.conv.selected.is_none() =>
            {
                if let Some(d) = self.conv.cleared_draft.take() {
                    self.composer.cursor = d.len();
                    self.composer.text = d;
                    self.filter_autocomplete();
                }
                return Some(Vec::new());
            }
            _ => {}
        }
        if self.conv.selected.is_some()
            && let Some(r) = self.conv_selection_key(key)
        {
            return Some(r);
        }
        None
    }

    fn conv_clear_draft(&mut self) {
        let text = std::mem::take(&mut self.composer.text);
        self.composer.cursor = 0;
        self.composer.history_idx = None;
        if !text.trim().is_empty() {
            self.conv.cleared_draft = Some(text);
        }
        self.filter_autocomplete();
    }

    /// Keys while one of this module's modal states is active.
    fn conv_modal_key(&mut self, key: KeyEvent) -> Option<Vec<AppAction>> {
        if self.conv.delete_confirm.is_some() {
            match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => {
                    let id = self.conv.delete_confirm.take().unwrap_or_default();
                    self.conv_delete_chat(&id);
                }
                KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                    self.conv.delete_confirm = None;
                    self.set_toast("Delete cancelled", false);
                }
                _ => {}
            }
            return Some(Vec::new());
        }
        if let Some(why) = &mut self.conv.why {
            match key.code {
                KeyCode::Esc | KeyCode::Char('q') => self.conv.why = None,
                KeyCode::Up => why.scroll = why.scroll.saturating_sub(1),
                KeyCode::Down => why.scroll += 1,
                KeyCode::PageUp => why.scroll = why.scroll.saturating_sub(8),
                KeyCode::PageDown => why.scroll += 8,
                _ => {}
            }
            return Some(Vec::new());
        }
        if self.conv.history_search.is_some() {
            return Some(self.conv_history_key(key));
        }
        if self.conv.chat_search_input {
            match key.code {
                KeyCode::Enter => self.conv.chat_search_input = false,
                KeyCode::Esc => {
                    self.conv.chat_search_input = false;
                    self.conv.chat_filter = None;
                }
                KeyCode::Backspace => {
                    if let Some(f) = &mut self.conv.chat_filter {
                        f.pop();
                        if f.is_empty() {
                            self.conv.chat_filter = None;
                        }
                    }
                }
                KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.conv.chat_filter.get_or_insert_with(String::new).push(c);
                }
                _ => {}
            }
            return Some(Vec::new());
        }
        None
    }

    // ----- turn control ------------------------------------------------------------------

    fn conv_esc_while_busy(&mut self) -> Vec<AppAction> {
        let now = self.turn.now_ms();
        if self.conv.esc_prompt_visible(now) {
            self.conv.esc_armed_ms = None;
            return vec![AppAction::Conv(ConvAction::CancelTurn)];
        }
        self.conv.esc_armed_ms = Some(now);
        Vec::new()
    }

    /// Result of `turn.cancel`.
    pub fn conv_on_cancel_result(&mut self, result: Result<bool, String>) {
        match result {
            Ok(true) => {
                let chat = self
                    .turn
                    .chat_id()
                    .map_or_else(|| self.chats.active_chat_id.clone(), str::to_owned);
                self.finish_busy();
                self.push_timeline_item(
                    &chat,
                    ChatTimelineItem::SystemNotice {
                        text: INTERRUPTED_NOTICE.to_owned(),
                        is_error: false,
                    },
                );
                if let Some(t) = &mut self.conv.last_turn {
                    t.status = TurnStatus::Interrupted;
                }
            }
            Ok(false) => {
                let chat = self.chats.active_chat_id.clone();
                self.push_timeline_item(
                    &chat,
                    ChatTimelineItem::SystemNotice {
                        text: NOT_STOPPABLE_NOTICE.to_owned(),
                        is_error: false,
                    },
                );
            }
            Err(e) => self.set_toast(format!("Interrupt failed: {e}"), true),
        }
    }

    fn conv_retry(&mut self) -> Vec<AppAction> {
        if self.is_busy() {
            return Vec::new();
        }
        let Some(last) = self.conv.last_turn.clone() else {
            return Vec::new();
        };
        if !matches!(last.status, TurnStatus::Error | TurnStatus::Interrupted) {
            return Vec::new();
        }
        vec![self.begin_user_turn(last.chat_id, last.place_id, last.text, last.attachments)]
    }

    /// Records the turn that is starting (for retry) and resets turn-control state.
    pub(crate) fn conv_on_turn_begin(
        &mut self,
        chat_id: &str,
        place_id: &str,
        text: &str,
        attachments: &[PendingAttachment],
    ) {
        self.conv.esc_armed_ms = None;
        self.conv.last_turn = Some(LastTurn {
            chat_id: chat_id.to_owned(),
            place_id: place_id.to_owned(),
            text: text.to_owned(),
            attachments: attachments.to_vec(),
            status: TurnStatus::Running,
        });
    }

    /// The busy state ended.
    pub(crate) fn conv_on_busy_end(&mut self) {
        self.conv.esc_armed_ms = None;
        self.conv.preview.resolve();
    }

    /// The running turn ended with an error.
    pub fn conv_mark_turn_error(&mut self) {
        if let Some(t) = &mut self.conv.last_turn
            && t.status == TurnStatus::Running
        {
            t.status = TurnStatus::Error;
        }
    }

    pub(crate) fn conv_mark_turn_ok(&mut self) {
        if let Some(t) = &mut self.conv.last_turn
            && t.status == TurnStatus::Running
        {
            t.status = TurnStatus::Ok;
        }
    }

    // ----- composer ------------------------------------------------------------------------

    fn conv_start_history_search(&mut self) {
        let s = HistorySearch::start(&self.composer.text);
        self.conv.history_search = Some(s);
        self.conv_show_history_match();
    }

    fn conv_show_history_match(&mut self) {
        let Some(s) = &self.conv.history_search else { return };
        if let Some(text) = s.preview(&self.composer.history) {
            let text = text.to_owned();
            self.composer.cursor = text.len();
            self.composer.text = text;
        }
        self.filter_autocomplete();
    }

    fn conv_history_key(&mut self, key: KeyEvent) -> Vec<AppAction> {
        let Some(s) = &mut self.conv.history_search else {
            return Vec::new();
        };
        match key.code {
            KeyCode::Enter => {
                // Accept: the match stays in the composer for editing.
                self.conv.history_search = None;
            }
            KeyCode::Esc => {
                let saved = s.saved_text.clone();
                self.conv.history_search = None;
                self.composer.cursor = saved.len();
                self.composer.text = saved;
                self.filter_autocomplete();
            }
            KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                s.older(&self.composer.history);
                self.conv_show_history_match();
            }
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                let saved = s.saved_text.clone();
                self.conv.history_search = None;
                self.composer.cursor = saved.len();
                self.composer.text = saved;
            }
            KeyCode::Backspace => {
                s.backspace();
                self.conv_show_history_match();
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                s.push(c);
                self.conv_show_history_match();
            }
            _ => {}
        }
        Vec::new()
    }

    // ----- what happened / message actions --------------------------------------------------

    fn conv_toggle_summary(&mut self) -> Vec<AppAction> {
        let chat = self.chats.active_chat_id.clone();
        let key = match self.conv.selected {
            Some(i) => self.conv_summary_key_for(&chat, i),
            None => self
                .conv
                .summaries
                .keys()
                .filter(|(c, _)| *c == chat)
                .max_by_key(|(_, i)| *i)
                .cloned(),
        };
        let Some(key) = key else {
            self.set_toast("No turn summary to expand yet", false);
            return Vec::new();
        };
        let Some(s) = self.conv.summaries.get_mut(&key) else {
            return Vec::new();
        };
        s.expanded = !s.expanded;
        if s.expanded && (s.error.is_some() || !s.loaded) {
            let req = TraceRequest {
                chat_id: key.0.clone(),
                item_idx: key.1,
                turn: s.turn,
                root_event: s.root_event,
            };
            s.error = None;
            return vec![AppAction::Conv(ConvAction::FetchTrace(req))];
        }
        Vec::new()
    }

    /// Summary key that belongs to timeline item `idx` (the item itself, or for a user message
    /// the reply that follows it).
    fn conv_summary_key_for(&self, chat: &str, idx: usize) -> Option<(String, usize)> {
        let tl = self.timelines.get(chat)?;
        (idx..tl.len().min(idx + 6))
            .find(|i| self.conv.summaries.contains_key(&(chat.to_owned(), *i)))
            .map(|i| (chat.to_owned(), i))
    }

    fn conv_move_selection(&mut self, delta: i32) {
        let tl = self.active_timeline();
        let idx = message_actions::message_indices(tl);
        self.conv.selected = message_actions::move_selection(self.conv.selected, &idx, delta);
        self.auto_scroll = self.conv.selected.is_none();
    }

    fn conv_selection_key(&mut self, key: KeyEvent) -> Option<Vec<AppAction>> {
        if key.modifiers.contains(KeyModifiers::CONTROL) || key.modifiers.contains(KeyModifiers::ALT) {
            return None;
        }
        match key.code {
            KeyCode::Up => {
                self.conv_move_selection(-1);
                Some(Vec::new())
            }
            KeyCode::Down => {
                self.conv_move_selection(1);
                Some(Vec::new())
            }
            KeyCode::Char('c') => Some(self.conv_copy_selected()),
            KeyCode::Char('q') => {
                self.conv_quote_selected();
                Some(Vec::new())
            }
            KeyCode::Char('w') => {
                let sel = self.conv.selected;
                Some(self.conv_why(sel))
            }
            _ => {
                // Any other key leaves selection mode and is handled as usual.
                self.conv.selected = None;
                None
            }
        }
    }

    fn selected_text(&self) -> Option<String> {
        let i = self.conv.selected?;
        message_actions::message_text(self.active_timeline().get(i)?).map(str::to_owned)
    }

    /// A click (no drag) at screen cell `(x, y)`: selects the message drawn there.
    pub fn conv_click_message(&mut self, hit: &crate::state::HitRegions, x: u16, y: u16) {
        if self.conv_chat_blocked() || self.conv.modal() {
            return;
        }
        let found = hit.message_rows.iter().find(|(r, _)| {
            x >= r.x && x < r.x.saturating_add(r.width) && y >= r.y && y < r.y.saturating_add(r.height)
        });
        if let Some((_, idx)) = found {
            self.conv.selected = Some(*idx);
            self.auto_scroll = false;
        }
    }

    /// `c`: copy the selected message.
    pub fn conv_copy_selected(&mut self) -> Vec<AppAction> {
        let Some(text) = self.selected_text() else {
            return Vec::new();
        };
        self.conv.selected = None;
        self.set_toast("Copied message to clipboard", false);
        vec![AppAction::CopyToClipboard(text)]
    }

    /// `q`: quote the selected message into the composer.
    pub fn conv_quote_selected(&mut self) {
        let Some(text) = self.selected_text() else {
            return;
        };
        let quoted = message_actions::quote_text(&text);
        let rest = std::mem::take(&mut self.composer.text);
        self.composer.text = if rest.is_empty() {
            format!("{quoted}\n")
        } else {
            format!("{quoted}\n{rest}")
        };
        self.composer.cursor = self.composer.text.len();
        self.conv.selected = None;
        self.filter_autocomplete();
    }

    /// `w` / `/why`: the valve explanation for a message (the latest reply when `idx` is `None`).
    pub fn conv_why(&mut self, idx: Option<usize>) -> Vec<AppAction> {
        let chat = self.chats.active_chat_id.clone();
        let idx = idx.or_else(|| {
            self.active_timeline()
                .iter()
                .rposition(|i| matches!(i, ChatTimelineItem::AssistantMessage { .. }))
        });
        self.conv.selected = None;
        let root = idx
            .and_then(|i| self.conv_summary_key_for(&chat, i))
            .and_then(|k| self.conv.summaries.get(&k))
            .and_then(|s| s.root_event);
        match root {
            Some(event_id) => {
                self.conv.why = Some(WhyOverlay::loading(format!("event #{event_id}")));
                self.conv.pending_explain = Some(event_id);
                vec![AppAction::Conv(ConvAction::FetchExplain { event_id })]
            }
            None => {
                let mut w = WhyOverlay::loading("this message");
                w.set_error("no recorded turn for it in this session");
                self.conv.why = Some(w);
                Vec::new()
            }
        }
    }

    /// Reply routed into the timeline: notify, remember its index and ask for its trace.
    pub fn conv_after_reply(&mut self, turn: u64, root_event: Option<u64>, text: &str) {
        self.conv.notify.mode = self.prefs.conv.notifications;
        self.conv.notify.reply(text);
        self.conv_mark_turn_ok();
        if let Some((chat, idx)) = self.conv.last_reply.clone() {
            self.conv
                .summaries
                .insert((chat.clone(), idx), TurnSummary::pending(turn, root_event));
            self.conv.pending_traces.push(TraceRequest {
                chat_id: chat,
                item_idx: idx,
                turn,
                root_event,
            });
        }
    }

    /// An approval prompt arrived.
    pub fn conv_on_approval(&mut self, summary: &str) {
        self.conv.notify.mode = self.prefs.conv.notifications;
        self.conv.notify.approval(summary);
    }

    /// Focus reporting.
    pub fn conv_focus(&mut self, gained: bool) {
        self.conv.notify.mode = self.prefs.conv.notifications;
        if gained {
            self.conv.notify.focus_gained();
        } else {
            self.conv.notify.focus_lost();
        }
    }

    /// The valve did not let the action through: the draft is replaced by the verdict and the
    /// drafted text appears nowhere else.
    pub fn conv_on_valve_verdict(&mut self, rule: &str, verdict: &str) {
        if self.conv.preview.draft.is_none() {
            return;
        }
        self.conv.preview.resolve();
        let chat = self
            .turn
            .chat_id()
            .map_or_else(|| self.chats.active_chat_id.clone(), str::to_owned);
        self.push_timeline_item(
            &chat,
            ChatTimelineItem::SystemNotice {
                text: format!("The valve did not deliver this reply ({rule}: {verdict})."),
                is_error: false,
            },
        );
    }

    // ----- chats ------------------------------------------------------------------------------

    fn conv_delete_chat(&mut self, id: &str) {
        let active = self.chats.active_chat_id.clone();
        self.conv.summaries.retain(|(c, _), _| c != id);
        if self.chats.delete_chat(id) {
            self.timelines.remove(id);
            let next = self.chats.active_chat_id.clone();
            self.timelines.entry(next).or_default();
            self.auto_scroll = true;
            self.chat_scroll = usize::MAX;
            self.persist_chats();
            self.set_toast("Deleted chat", false);
        } else {
            self.timelines.entry(active.clone()).or_default().clear();
            self.chats.clear_messages(&active);
            self.chat_scroll = 0;
            self.auto_scroll = true;
            self.persist_chats();
            self.set_toast("Cleared last remaining chat", false);
        }
    }

    /// Slash hook, called first by `execute_slash_command`. `Some(actions)` handled it.
    pub fn conv_slash(&mut self, cmd: &str, rest: &str) -> Option<Vec<AppAction>> {
        let active = self.chats.active_chat_id.clone();
        match cmd {
            "/delete" => {
                self.conv.delete_confirm = Some(active);
                self.filter_autocomplete();
                Some(Vec::new())
            }
            "/clear" => {
                self.conv.summaries.retain(|(c, _), _| *c != active);
                self.conv.selected = None;
                None
            }
            "/why" => {
                self.filter_autocomplete();
                Some(self.conv_why(None))
            }
            "/export" => {
                self.conv_export(rest);
                Some(Vec::new())
            }
            "/chat" => {
                let (sub, arg) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
                match sub {
                    "pin" | "unpin" => {
                        let chat = self.chats.active_chat_mut();
                        chat.pinned = !chat.pinned;
                        let pinned = chat.pinned;
                        self.persist_chats();
                        self.set_toast(if pinned { "Pinned chat" } else { "Unpinned chat" }, false);
                    }
                    "search" => {
                        let q = arg.trim();
                        self.conv.chat_filter = (!q.is_empty()).then(|| q.to_owned());
                        let n = chat_search::visible_chats(
                            &self.chats,
                            &self.timelines,
                            self.conv.chat_filter.as_deref(),
                        )
                        .len();
                        self.set_toast(
                            if q.is_empty() {
                                "Chat search cleared".to_owned()
                            } else {
                                format!("{n} chat(s) match '{q}'")
                            },
                            false,
                        );
                    }
                    _ => self.set_toast("Usage: /chat pin | /chat search <text>", true),
                }
                Some(Vec::new())
            }
            "/notify" => {
                match NotifyMode::parse(rest) {
                    Some(m) => {
                        self.prefs.conv.notifications = m;
                        self.persist_prefs();
                        self.set_toast(format!("Notifications: {}", m.label()), false);
                    }
                    None => self.set_toast(
                        format!(
                            "Notifications: {} (off | bell | desktop | both)",
                            self.prefs.conv.notifications.label()
                        ),
                        false,
                    ),
                }
                Some(Vec::new())
            }
            "/preview" => {
                match rest.trim() {
                    "on" => self.prefs.conv.live_draft_preview = true,
                    "off" => self.prefs.conv.live_draft_preview = false,
                    _ => {
                        self.set_toast(
                            format!(
                                "Live draft preview: {} (/preview on | off)",
                                if self.prefs.conv.live_draft_preview { "on" } else { "off" }
                            ),
                            false,
                        );
                        return Some(Vec::new());
                    }
                }
                self.persist_prefs();
                self.set_toast(
                    format!(
                        "Live draft preview {}",
                        if self.prefs.conv.live_draft_preview { "on" } else { "off" }
                    ),
                    false,
                );
                Some(Vec::new())
            }
            _ => None,
        }
    }

    /// `/export [path]`: the active chat as Markdown.
    pub fn conv_export(&mut self, arg: &str) {
        let chat = self.chats.active_chat();
        let (title, id) = (chat.title.clone(), chat.id.clone());
        let md = chat_search::export_markdown(
            &title,
            &self.gateway_cfg.soul.name,
            self.timelines.get(&id).map_or(&[][..], Vec::as_slice),
        );
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let target = chat_search::export_target(&self.paths.data_dir, Some(arg), &title, now);
        match chat_search::write_export(&target, &md) {
            Ok(()) => self.set_toast(format!("Exported to {}", target.display()), false),
            Err(e) => self.set_toast(format!("Export failed: {e}"), true),
        }
    }

    // ----- results coming back ------------------------------------------------------------------

    /// Applies a result of the async work.
    pub fn conv_event(&mut self, ev: ConvEvent) {
        match ev {
            ConvEvent::Preview(PreviewEvent::Draft { request_id, text }) => {
                let busy = self.is_busy();
                self.conv.preview.update(request_id, text, busy);
            }
            ConvEvent::Preview(PreviewEvent::End { request_id, outcome }) => {
                self.conv
                    .preview
                    .end(request_id, matches!(outcome, PreviewEnd::Completed));
            }
            ConvEvent::PreviewSubscribed(r) => {
                self.conv.preview.busy = false;
                match r {
                    Ok(id) => self.conv.preview.subscription = Some(id),
                    Err(e) => self.set_toast(format!("Draft preview unavailable: {e}"), true),
                }
            }
            ConvEvent::PreviewUnsubscribed => {
                self.conv.preview.busy = false;
                self.conv.preview.subscription = None;
                self.conv.preview.resolve();
            }
            ConvEvent::Trace { request, result } => {
                let key = (request.chat_id.clone(), request.item_idx);
                let expanded = self.conv.summaries.get(&key).is_some_and(|s| s.expanded);
                let summary = match result {
                    Ok(rows) => TurnSummary::from_rows(request.turn, request.root_event, &rows, expanded),
                    Err(e) => TurnSummary {
                        turn: request.turn,
                        root_event: request.root_event,
                        expanded,
                        error: Some(e),
                        ..TurnSummary::default()
                    },
                };
                if self.conv.summaries.contains_key(&key) {
                    self.conv.summaries.insert(key, summary);
                }
            }
            ConvEvent::Explain { event_id, result } => {
                if self.conv.pending_explain == Some(event_id)
                    && let Some(w) = &mut self.conv.why
                {
                    match result {
                        Ok(v) => w.set_explain(&v),
                        Err(e) => w.set_error(&e),
                    }
                }
            }
            ConvEvent::EditorDone(r) => {
                self.conv.editor_open = false;
                match r {
                    Ok(text) => {
                        self.composer.cursor = text.len();
                        self.composer.text = text;
                        self.filter_autocomplete();
                    }
                    Err(e) => self.set_toast(e, true),
                }
            }
            ConvEvent::Cancelled(r) => self.conv_on_cancel_result(r),
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Runtime glue
// ---------------------------------------------------------------------------------------------

/// Runs one [`ConvAction`]. Returns `true` when the terminal must be fully repainted (the
/// external editor used it).
pub async fn run_action(
    action: ConvAction,
    state: &mut TuiAppState,
    guard: &mut crate::runtime::TerminalGuard,
    client: &Option<Arc<dyn GatewayClient>>,
    tx: &mpsc::UnboundedSender<RuntimeEvent>,
) -> bool {
    match action {
        ConvAction::CancelTurn => {
            let Some(c) = client.clone() else {
                state.set_toast("The gateway is not running", true);
                return false;
            };
            let tx = tx.clone();
            tokio::spawn(async move {
                let r = c.cancel_turn().await.map_err(|e| e.message);
                let _ = tx.send(RuntimeEvent::Conv(ConvEvent::Cancelled(r)));
            });
            false
        }
        ConvAction::OpenEditor { draft } => {
            let editor = composer_history::editor_command(
                std::env::var("VISUAL").ok().as_deref(),
                std::env::var("EDITOR").ok().as_deref(),
            );
            let Some(editor) = editor else {
                state.conv.editor_open = false;
                state.set_toast("No editor: set $VISUAL or $EDITOR", true);
                return false;
            };
            let path = composer_history::editor_draft_path(&state.paths.data_dir);
            guard.suspend_for_subprocess();
            let result = composer_history::run_external_editor(&editor, &path, &draft);
            let resumed = guard.resume_after_subprocess();
            let result = match (result, resumed) {
                (Ok(t), Ok(())) => Ok(t),
                (Err(e), _) | (_, Err(e)) => Err(e),
            };
            state.conv_event(ConvEvent::EditorDone(result));
            true
        }
        ConvAction::FetchTrace(req) => {
            spawn_trace(client, tx, req);
            false
        }
        ConvAction::FetchExplain { event_id } => {
            let Some(c) = client.clone() else {
                state.conv_event(ConvEvent::Explain {
                    event_id,
                    result: Err("the gateway is not running".to_owned()),
                });
                return false;
            };
            let tx = tx.clone();
            tokio::spawn(async move {
                let result = c
                    .gov_call("events.explain".to_owned(), json!({ "id": event_id }))
                    .await
                    .map_err(|e| e.message);
                let _ = tx.send(RuntimeEvent::Conv(ConvEvent::Explain { event_id, result }));
            });
            false
        }
        ConvAction::PreviewSubscribe => {
            if let Some(c) = client.clone() {
                let tx2 = tx.clone();
                tokio::spawn(async move {
                    let r = c.preview_subscribe(tx2.clone()).await.map_err(|e| e.message);
                    let _ = tx2.send(RuntimeEvent::Conv(ConvEvent::PreviewSubscribed(r)));
                });
            }
            false
        }
        ConvAction::PreviewUnsubscribe(id) => {
            if let Some(c) = client.clone() {
                let tx2 = tx.clone();
                tokio::spawn(async move {
                    let _ = c.preview_unsubscribe(id).await;
                    let _ = tx2.send(RuntimeEvent::Conv(ConvEvent::PreviewUnsubscribed));
                });
            }
            false
        }
    }
}

fn spawn_trace(
    client: &Option<Arc<dyn GatewayClient>>,
    tx: &mpsc::UnboundedSender<RuntimeEvent>,
    request: TraceRequest,
) {
    let Some(c) = client.clone() else { return };
    let Some(root) = request.root_event else {
        let _ = tx.send(RuntimeEvent::Conv(ConvEvent::Trace {
            request,
            result: Err("the turn's event is not known".to_owned()),
        }));
        return;
    };
    let tx = tx.clone();
    tokio::spawn(async move {
        let result = c
            .gov_call("events.trace".to_owned(), json!({ "trace": root.to_string() }))
            .await
            .map(|v| v.get("events").and_then(Value::as_array).cloned().unwrap_or_default())
            .map_err(|e| e.message);
        let _ = tx.send(RuntimeEvent::Conv(ConvEvent::Trace { request, result }));
    });
}

/// Called once per event-loop iteration: starts queued trace fetches, keeps the preview
/// subscription in line with the setting and writes pending notification bytes.
pub fn pump(
    state: &mut TuiAppState,
    client: &Option<Arc<dyn GatewayClient>>,
    tx: &mpsc::UnboundedSender<RuntimeEvent>,
) {
    for req in std::mem::take(&mut state.conv.pending_traces) {
        spawn_trace(client, tx, req);
    }
    if client.is_some() && !state.conv.preview.busy {
        let want = state.prefs.conv.live_draft_preview;
        match (want, state.conv.preview.subscription) {
            (true, None) => {
                state.conv.preview.busy = true;
                let c = client.clone();
                let tx2 = tx.clone();
                if let Some(c) = c {
                    tokio::spawn(async move {
                        let r = c.preview_subscribe(tx2.clone()).await.map_err(|e| e.message);
                        let _ = tx2.send(RuntimeEvent::Conv(ConvEvent::PreviewSubscribed(r)));
                    });
                }
            }
            (false, Some(id)) => {
                state.conv.preview.busy = true;
                if let Some(c) = client.clone() {
                    let tx2 = tx.clone();
                    tokio::spawn(async move {
                        let _ = c.preview_unsubscribe(id).await;
                        let _ = tx2.send(RuntimeEvent::Conv(ConvEvent::PreviewUnsubscribed));
                    });
                }
            }
            _ => {}
        }
    }
    let out = state.conv.notify.take_output();
    if !out.is_empty() {
        use std::io::Write;
        let mut so = std::io::stdout();
        let _ = so.write_all(&out);
        let _ = so.flush();
    }
}

// ---------------------------------------------------------------------------------------------
// Rendering hooks
// ---------------------------------------------------------------------------------------------

/// Marks the lines of timeline item `idx` as selected.
pub fn mark_selected(lines: &mut [Line<'static>], state: &TuiAppState, idx: usize, palette: &ThemePalette) {
    if state.conv.selected != Some(idx) {
        return;
    }
    for l in lines.iter_mut() {
        l.style = if palette.no_color {
            l.style.add_modifier(Modifier::REVERSED)
        } else {
            l.style.bg(palette.selection_bg)
        };
    }
}

/// The what-happened lines that belong under timeline item `idx` (empty when none).
#[must_use]
pub fn summary_lines(state: &TuiAppState, idx: usize, width: usize, palette: &ThemePalette) -> Vec<Line<'static>> {
    let key = (state.chats.active_chat_id.clone(), idx);
    let Some(s) = state.conv.summaries.get(&key) else {
        return Vec::new();
    };
    let style = Style::default().fg(palette.dim);
    let mut out: Vec<Line<'static>> = s
        .lines()
        .into_iter()
        .map(|l| Line::styled(crate::width::fit_prefix(&l, width), style))
        .collect();
    out.push(Line::default());
    out
}

/// Draws the one-line prompts at the bottom of the chat feed and the `/why` overlay.
pub fn render_overlays(area: Rect, feed: Rect, buf: &mut Buffer, state: &TuiAppState, palette: &ThemePalette) {
    let c = &state.conv;
    let prompt: Option<String> = if let Some(id) = &c.delete_confirm {
        let title = state
            .chats
            .chats
            .iter()
            .find(|x| x.id == *id)
            .map_or(id.as_str(), |x| x.title.as_str());
        Some(format!("Delete chat '{title}'? y = delete, n = keep"))
    } else if let Some(s) = &c.history_search {
        Some(format!(
            "(reverse-i-search) `{}`{}  Ctrl+R older, Enter accept, Esc cancel",
            s.query,
            if s.matched(&state.composer.history).is_none() { " (no match)" } else { "" }
        ))
    } else if c.chat_search_input {
        Some(format!(
            "chat search: {}▏  Enter keep filter, Esc clear",
            c.chat_filter.as_deref().unwrap_or("")
        ))
    } else if c.esc_prompt_visible(state.turn.now_ms()) && state.is_busy() {
        Some("Press Esc again to interrupt".to_owned())
    } else if c.selected.is_some() {
        Some("message: c copy · q quote · w why · ↑/↓ move · Esc leave".to_owned())
    } else {
        None
    };
    if let Some(p) = prompt
        && feed.height >= 1
        && feed.width > 8
    {
        let y = feed.y + feed.height - 1;
        let style = Style::default().fg(palette.accent).bg(palette.status_bg);
        buf.set_string(feed.x + 1, y, pad(&p, feed.width.saturating_sub(2) as usize), style);
    }
    if let Some(why) = &c.why {
        message_actions::render_why(area, buf, why, palette);
    }
}

#[cfg(test)]
#[path = "conv_tests.rs"]
mod conv_tests;
