//! Full interactive TUI application state, slash commands, mouse & keyboard reducers.

use std::collections::{BTreeSet, HashMap};
use std::path::Path;

use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use monoagens_gateway::{
    CommandSpec, GatewayConfig, PluginConfig, ResolvedPaths,
    RunLogLevel,
};
use monoagens_protocol::{ActionId, ActorId, Audience, HAct, PlaceId};
use ratatui::layout::Rect;
use unicode_width::UnicodeWidthChar;

use crate::attachments::{
    FileAttachmentOutcome, PasteItem, PendingAttachment, PendingAttachmentKind,
    classify_paste_input, import_host_file_to_workspace, inspect_file_for_attachment,
};

use crate::config::{
    BUILTIN_PLUGIN_PRESETS, LocalChatStore, SidebarMode, StoredChatMessagePreview, TuiPreferences,
    config_dir_from_path, discover_builtin_binary, list_secret_keys,
};
use crate::settings::{
    PluginUiEntry, PluginUiState, SecretsUiSummary, SettingsModalState, SettingsTab, SetupWizardState,
    ValveRuleTogglesState, WizardStep,
};
use crate::theme::ThemePreset;
use crate::turn::{NO_REPLY_NOTICE, QueuedMessage, TurnState};

/// A single item rendered inside a chat timeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChatTimelineItem {
    UserMessage {
        sender: String,
        text: String,
        timestamp_ms: u64,
        is_group: bool,
    },
    AssistantMessage {
        text: String,
        duration_ms: Option<u64>,
        timestamp_ms: u64,
    },
    CapabilityStep {
        capability: String,
        status: String,
        detail: String,
    },
    SystemNotice {
        text: String,
        is_error: bool,
    },
    BrainTrace {
        summary: String,
    },
}

/// Generic inline Valve `PendingApproval` card rendered in the chat viewport.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingApprovalCard {
    pub action_id: ActionId,
    pub h_act: HAct,
    pub state_epoch: u64,
    pub capability_id: String,
    pub summary: String,
    pub selected_approve: bool,
}

/// Slash command entry shown in the 2-column `/` autocomplete dropdown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlashCommandItem {
    pub name: String,
    pub usage: String,
    pub description: String,
    pub current_value: Option<String>,
    pub icon: &'static str,
    pub lab_only: bool,
    pub from_plugin: Option<String>,
}

/// Multi-line composer state with sent-message history and staged attachments.
#[derive(Debug, Clone)]
pub struct ComposerState {
    pub text: String,
    pub cursor: usize,
    pub history: Vec<String>,
    pub history_idx: Option<usize>,
    pub pending_attachments: Vec<PendingAttachment>,
    pub next_attachment_id: usize,
}

impl Default for ComposerState {
    fn default() -> Self {
        Self {
            text: String::new(),
            cursor: 0,
            history: Vec::new(),
            history_idx: None,
            pending_attachments: Vec::new(),
            next_attachment_id: 1,
        }
    }
}

impl ComposerState {
    /// Inserts a character at the byte cursor.
    pub fn insert_char(&mut self, ch: char) {
        let ch = if ch == '\t' { ' ' } else { ch };
        if ch.is_control() && ch != '\n' {
            return;
        }
        let idx = self.cursor.min(self.text.len());
        self.text.insert(idx, ch);
        self.cursor = idx + ch.len_utf8();
    }

    /// Inserts a string slice at the byte cursor.
    pub fn insert_str(&mut self, s: &str) {
        let s = crate::width::sanitize(s, true);
        let s = s.as_str();
        if s.is_empty() {
            return;
        }
        let mut idx = self.cursor.min(self.text.len());
        while idx > 0 && !self.text.is_char_boundary(idx) {
            idx -= 1;
        }
        self.text.insert_str(idx, s);
        self.cursor = idx + s.len();
    }

    /// Assigns a monotonic draft ID, inserts the inline token (`📷 #1`, etc.),
    /// and stages `att` in `pending_attachments`.
    pub fn add_attachment(&mut self, mut att: PendingAttachment) -> usize {
        let id = self.next_attachment_id.max(1);
        self.next_attachment_id = id + 1;
        att.id = id;
        let token = format!("{} ", att.inline_token());
        self.insert_str(&token);
        self.pending_attachments.push(att);
        id
    }

    /// Removes attachment `id` and strips its inline token from the composer text.
    pub fn remove_attachment(&mut self, id: usize) -> Option<PendingAttachment> {
        let pos = self.pending_attachments.iter().position(|a| a.id == id)?;
        let removed = self.pending_attachments.remove(pos);
        let token = removed.inline_token();
        let with_space = format!("{token} ");
        if let Some(idx) = self.text.find(&with_space) {
            self.text.replace_range(idx..idx + with_space.len(), "");
        } else if let Some(idx) = self.text.find(&token) {
            self.text.replace_range(idx..idx + token.len(), "");
        }
        self.clamp_cursor();
        if self.pending_attachments.is_empty() && self.text.trim().is_empty() {
            self.next_attachment_id = 1;
        }
        Some(removed)
    }

    /// Removes all pending attachments, strips their inline tokens, and resets ID counter to `1`.
    pub fn clear_attachments(&mut self) {
        let removed = std::mem::take(&mut self.pending_attachments);
        for att in removed {
            let token = att.inline_token();
            let with_space = format!("{token} ");
            if let Some(idx) = self.text.find(&with_space) {
                self.text.replace_range(idx..idx + with_space.len(), "");
            } else if let Some(idx) = self.text.find(&token) {
                self.text.replace_range(idx..idx + token.len(), "");
            }
        }
        self.clamp_cursor();
        self.next_attachment_id = 1;
    }

    fn clamp_cursor(&mut self) {
        if self.cursor > self.text.len() {
            self.cursor = self.text.len();
        }
        while self.cursor > 0 && !self.text.is_char_boundary(self.cursor) {
            self.cursor -= 1;
        }
    }

    /// Inserts a newline at the byte cursor.
    pub fn insert_newline(&mut self) {
        self.insert_char('\n');
    }

    /// Deletes the character immediately before the cursor.
    pub fn backspace(&mut self) {
        let idx = self.cursor.min(self.text.len());
        if idx == 0 {
            return;
        }
        if let Some((prev_idx, _)) = self.text[..idx].char_indices().next_back() {
            self.text.drain(prev_idx..idx);
            self.cursor = prev_idx;
        }
    }

    /// Moves cursor one char left.
    pub fn move_left(&mut self) {
        let idx = self.cursor.min(self.text.len());
        if let Some((prev_idx, _)) = self.text[..idx].char_indices().next_back() {
            self.cursor = prev_idx;
        }
    }

    /// Moves cursor one char right.
    pub fn move_right(&mut self) {
        let idx = self.cursor.min(self.text.len());
        if let Some(ch) = self.text[idx..].chars().next() {
            self.cursor = idx + ch.len_utf8();
        }
    }

    /// Navigates backward through sent history.
    pub fn history_up(&mut self) {
        if self.history.is_empty() {
            return;
        }
        let next = match self.history_idx {
            None => self.history.len() - 1,
            Some(0) => 0,
            Some(i) => i - 1,
        };
        self.history_idx = Some(next);
        self.text = self.history[next].clone();
        self.cursor = self.text.len();
    }

    /// Navigates forward through sent history.
    pub fn history_down(&mut self) {
        let Some(cur) = self.history_idx else {
            return;
        };
        if cur + 1 >= self.history.len() {
            self.history_idx = None;
            self.text.clear();
            self.cursor = 0;
        } else {
            let next = cur + 1;
            self.history_idx = Some(next);
            self.text = self.history[next].clone();
            self.cursor = self.text.len();
        }
    }

    /// Clears composer and records non-empty input into history.
    pub fn take_submitted(&mut self) -> String {
        let taken = std::mem::take(&mut self.text);
        self.cursor = 0;
        self.history_idx = None;
        let trimmed = taken.trim().to_owned();
        if !trimmed.is_empty() && self.history.last() != Some(&trimmed) {
            self.history.push(trimmed.clone());
        }
        taken
    }

    /// Returns `true` when the composer begins with `/` on a single line (triggers autocomplete).
    #[must_use]
    pub fn is_slash_mode(&self) -> bool {
        let trimmed = self.text.trim_start();
        trimmed.starts_with('/') && !trimmed.contains('\n')
    }
}

/// `/lab` simulation state (hidden unless `lab_mode == true`).
#[derive(Debug, Clone)]
pub struct LabSimulationState {
    pub owner_actor: ActorId,
    pub current_actor: ActorId,
    pub place: PlaceId,
    pub is_group: bool,
    pub audience: Audience,
    pub known_actors: BTreeSet<String>,
}

impl LabSimulationState {
    /// Initializes simulation state for `owner_actor`.
    #[must_use]
    pub fn new(owner_actor: ActorId) -> Self {
        let mut known_actors = BTreeSet::new();
        known_actors.insert(owner_actor.as_str().to_owned());
        Self {
            current_actor: owner_actor.clone(),
            audience: Audience::new([owner_actor.clone(), ActorId::new("soul")]),
            owner_actor,
            place: PlaceId::new("cli:terminal"),
            is_group: false,
            known_actors,
        }
    }
}

/// A configuration error that kept the gateway from booting: the TUI still opens and shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigErrorState {
    /// Full error text.
    pub message: String,
    /// Offending config key (`model.provider`) when it can be named.
    pub key: Option<String>,
}

/// Settings tab that owns a config key.
#[must_use]
pub fn settings_tab_for_key(key: &str) -> SettingsTab {
    match key.split('.').next().unwrap_or("") {
        "soul" | "embed" => SettingsTab::SoulBrain,
        "model" => SettingsTab::Model,
        "valve" => SettingsTab::Valve,
        "rbac" | "person" | "persons" | "identity" => SettingsTab::Rbac,
        "runtime" => SettingsTab::Runtime,
        "plugins" | "plugin" | "bodies" => SettingsTab::Plugins,
        "secrets" | "api_key" | "endpoint" => SettingsTab::Providers,
        _ => SettingsTab::Appearance,
    }
}

/// Result of routing an assistant reply (see [`TuiAppState::on_assistant_reply`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplyDisposition {
    /// The reply belongs to the running (or latest) turn and ended its busy state.
    CurrentTurn,
    /// The reply belongs to an earlier turn; the running turn stays busy.
    LateReply,
}

/// Brief transient status toast.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToastMessage {
    pub text: String,
    pub is_error: bool,
    pub ticks_remaining: u16,
}

/// Active drag-selection range in chat viewport screen coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChatSelection {
    pub anchor: (u16, u16),
    pub current: (u16, u16),
    pub dragging: bool,
}

/// Hit-test rectangles recorded during every render frame for mouse wheel and click dispatch.
#[derive(Debug, Clone, Default)]
pub struct HitRegions {
    pub sidebar_area: Rect,
    pub sidebar_max_scroll: usize,
    pub chat_area: Rect,
    pub chat_scrollbar: Option<Rect>,
    pub chat_total_lines: usize,
    pub chat_visible_lines: usize,
    pub chat_viewport_lines: Vec<String>,
    pub attachment_cards: Vec<(Rect, usize)>,
    pub sidebar_chats: Vec<(Rect, String)>,
    pub sidebar_plugins: Vec<(Rect, String)>,
    pub sidebar_actors: Vec<(Rect, String)>,
    pub approval_approve: Option<Rect>,
    pub approval_deny: Option<Rect>,
    pub autocomplete_rows: Vec<(Rect, usize)>,
    pub autocomplete_area: Rect,
    pub autocomplete_total: usize,
    pub autocomplete_visible: usize,
    pub autocomplete_offset: usize,
    /// Generic list overlay (selector, item editor, palette, help): rows and scroll geometry.
    pub overlay_area: Rect,
    pub overlay_rows: Vec<(Rect, usize)>,
    pub overlay_total: usize,
    pub overlay_offset: usize,
    pub settings_area: Rect,
    pub log_area: Rect,
    pub log_total: usize,
    pub log_visible: usize,
    pub settings_back: Option<Rect>,
    pub settings_tabs: Vec<(Rect, SettingsTab)>,
    pub settings_sections: Vec<(Rect, usize)>,
    pub settings_options: Vec<(Rect, usize)>,
    pub wizard_buttons: Vec<(Rect, usize)>,
    /// Visible rows of each selectable message of the chat feed (timeline index).
    pub message_rows: Vec<(Rect, usize)>,
}

impl HitRegions {
    /// Clears all per-frame hit rectangles before rendering.
    pub fn clear(&mut self) {
        self.sidebar_area = Rect::default();
        self.sidebar_max_scroll = 0;
        self.chat_area = Rect::default();
        self.chat_scrollbar = None;
        self.chat_total_lines = 0;
        self.chat_visible_lines = 0;
        self.chat_viewport_lines.clear();
        self.attachment_cards.clear();
        self.sidebar_chats.clear();
        self.sidebar_plugins.clear();
        self.sidebar_actors.clear();
        self.approval_approve = None;
        self.approval_deny = None;
        self.autocomplete_rows.clear();
        self.autocomplete_area = Rect::default();
        self.autocomplete_total = 0;
        self.autocomplete_visible = 0;
        self.autocomplete_offset = 0;
        self.overlay_area = Rect::default();
        self.overlay_rows.clear();
        self.overlay_total = 0;
        self.overlay_offset = 0;
        self.settings_area = Rect::default();
        self.log_area = Rect::default();
        self.log_total = 0;
        self.log_visible = 0;
        self.settings_back = None;
        self.settings_tabs.clear();
        self.settings_sections.clear();
        self.settings_options.clear();
        self.wizard_buttons.clear();
        self.message_rows.clear();
    }
}

const fn rect_contains(r: Rect, x: u16, y: u16) -> bool {
    x >= r.x && x < r.x.saturating_add(r.width) && y >= r.y && y < r.y.saturating_add(r.height)
}

/// Side-effect actions emitted by state reducers for the async runtime to execute.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppAction {
    SendUserMessage {
        chat_id: String,
        place_id: String,
        text: String,
        attachments: Vec<PendingAttachment>,
    },
    ResolveApproval {
        action_id: ActionId,
        h_act: HAct,
        state_epoch: u64,
        approved: bool,
    },
    StartPlugin(String),
    StopPlugin(String),
    PairPlugin(String),
    ToggleMouseCapture(bool),
    CompleteSetupWizard,
    SubmitGatewayCommand(String),
    CopyToClipboard(String),
    PasteFromClipboard,
    /// `/doctor`: run the diagnostic checks.
    RunDoctor,
    /// `/retry` or `Ctrl+R` in the config-error state: reload the config and boot the gateway.
    RetryBoot,
    /// `Alt+L`: clear the terminal and repaint everything.
    ResetDisplay,
    /// `Ctrl+Z`: leave the alternate screen, stop the process, restore on `SIGCONT`.
    Suspend,
    /// `/model`, `/login` and `/usage` requests to the gateway (see `catalog_ui`).
    Catalog(crate::catalog_ui::CatalogAction),
    /// A governance / observability admin call for the panels in `crate::views`.
    Gov(crate::views::GovRequest),
    /// Conversation work for the async runtime (interrupt, editor, trace, preview).
    Conv(crate::conv::ConvAction),
    /// A settings write or plugin action (see `crate::settings_runtime`).
    Settings(crate::settings_runtime::SettingsAction),
}

/// Result of feeding one terminal event to [`TuiAppState::handle_terminal_event`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EventOutcome {
    pub actions: Vec<AppAction>,
    /// The screen must be redrawn.
    pub redraw: bool,
    /// The terminal must be cleared first (full display reset).
    pub clear: bool,
}

/// Root state for the interactive MonoAgens TUI.
#[derive(Debug, Clone)]
pub struct TuiAppState {
    pub prefs: TuiPreferences,
    pub chats: LocalChatStore,
    pub timelines: HashMap<String, Vec<ChatTimelineItem>>,
    pub plugins: Vec<PluginUiEntry>,
    pub pending_approval: Option<PendingApprovalCard>,
    pub selection: Option<ChatSelection>,
    pub lab_mode: bool,
    pub lab_session: LabSimulationState,
    pub composer: ComposerState,
    pub autocomplete: Vec<SlashCommandItem>,
    pub autocomplete_idx: usize,
    /// First visible row of the slash menu (kept so the selection stays on screen).
    pub autocomplete_off: usize,
    pub settings: SettingsModalState,
    pub wizard: Option<SetupWizardState>,
    pub sidebar_scroll: usize,
    pub chat_scroll: usize,
    pub auto_scroll: bool,
    pub dragging_chat_scrollbar: bool,
    pub thinking: bool,
    pub thinking_phase: String,
    pub thinking_elapsed_ms: u64,
    pub turn: TurnState,
    pub last_decision_trace: Option<String>,
    pub daily_tokens_used: u64,
    pub daily_token_budget: u64,
    /// Cost so far in USD; `None` while any price is unknown (then the status bar hides it).
    pub estimated_cost_usd: Option<f64>,
    pub valve_mode: String,
    pub model_name: String,
    pub gateway_commands: Vec<CommandSpec>,
    pub toast: Option<ToastMessage>,
    pub should_quit: bool,
    pub paths: ResolvedPaths,
    pub gateway_cfg: GatewayConfig,
    pub secrets_summary: SecretsUiSummary,
    /// `/log` overlay state and received runtime log entries.
    pub log: crate::log::LogView,
    /// `/model` picker, `/login` and `/usage` overlays plus the provider/model data behind them.
    pub catalog: crate::catalog_ui::CatalogUi,
    /// Set while the gateway could not boot because of a configuration error.
    pub config_error: Option<ConfigErrorState>,
    /// Set when chat cannot be used although the gateway is reachable (attached while another
    /// TUI owns the terminal place).
    pub chat_disabled_reason: Option<String>,
    /// How this TUI reaches the gateway (`hosting`, `attached`), for the status line and `/doctor`.
    pub gateway_link: String,
    /// Effects the owner step of the latest finished turn spawned (for `turn.no_reply`).
    pub turn_effects: usize,
    /// `/approvals`, `/people`, `/policy`, `/activity`, `/events` and `/status` panels.
    pub gov: crate::views::GovState,
    /// Conversation features: interrupt/retry, history search, what-happened, notifications,
    /// draft preview (see `crate::conv`).
    pub conv: crate::conv::ConvState,
    /// Key map, help overlay and command palette.
    pub ux: crate::ui_global::UxState,
}

impl TuiAppState {
    /// Constructs a [`TuiAppState`] from resolved paths, config, preferences, and local chats.
    #[must_use]
    pub fn new(
        paths: ResolvedPaths,
        gateway_cfg: GatewayConfig,
        prefs: TuiPreferences,
        chats: LocalChatStore,
        secrets_summary: SecretsUiSummary,
        owner_actor: ActorId,
    ) -> Self {
        let mut timelines = HashMap::new();
        for chat in &chats.chats {
            let items: Vec<ChatTimelineItem> = chat
                .messages
                .iter()
                .map(|m| {
                    if m.role == "user" {
                        ChatTimelineItem::UserMessage {
                            sender: m.sender.clone(),
                            text: m.text_preview.clone(),
                            timestamp_ms: m.timestamp_ms,
                            is_group: false,
                        }
                    } else {
                        ChatTimelineItem::AssistantMessage {
                            text: m.text_preview.clone(),
                            duration_ms: m.duration_ms,
                            timestamp_ms: m.timestamp_ms,
                        }
                    }
                })
                .collect();
            timelines.insert(chat.id.clone(), items);
        }

        let keymap = crate::keybindings::KeyMap::load(&crate::keybindings::KeyMap::path_for_config(&paths.config_path));
        let valve_toggles = ValveRuleTogglesState::load_from_config(&paths.config_path);
        let plugins = build_plugin_entries(&gateway_cfg, &paths);
        let settings = SettingsModalState {
            valve_toggles,
            ..SettingsModalState::default()
        };

        let mut app = Self {
            prefs,
            chats,
            timelines,
            plugins,
            pending_approval: None,
            selection: None,
            lab_mode: false,
            lab_session: LabSimulationState::new(owner_actor),
            composer: ComposerState::default(),
            autocomplete: Vec::new(),
            autocomplete_idx: 0,
            autocomplete_off: 0,
            settings,
            wizard: None,
            sidebar_scroll: 0,
            chat_scroll: 0,
            auto_scroll: true,
            dragging_chat_scrollbar: false,
            thinking: false,
            thinking_phase: "Reasoning…".to_owned(),
            thinking_elapsed_ms: 0,
            turn: TurnState::default(),
            last_decision_trace: None,
            daily_tokens_used: 0,
            daily_token_budget: gateway_cfg.model.daily_token_budget.max(1),
            estimated_cost_usd: None,
            valve_mode: gateway_cfg.valve.mode.clone(),
            model_name: gateway_cfg.model.model.clone(),
            gateway_commands: Vec::new(),
            toast: None,
            should_quit: false,
            paths,
            gateway_cfg,
            secrets_summary,
            log: crate::log::LogView::default(),
            catalog: crate::catalog_ui::CatalogUi::default(),
            config_error: None,
            chat_disabled_reason: None,
            gateway_link: String::new(),
            turn_effects: 0,
            gov: crate::views::GovState::default(),
            conv: crate::conv::ConvState::default(),
            ux: crate::ui_global::UxState { keymap, ..crate::ui_global::UxState::default() },
        };
        app.filter_autocomplete();
        app
    }

    /// Returns whether the sidebar is currently visible for `viewport_width`.
    #[must_use]
    pub fn sidebar_visible(&self, viewport_width: u16) -> bool {
        if viewport_width < crate::render::MIN_SIDEBAR_COLS {
            return false;
        }
        match self.prefs.sidebar {
            SidebarMode::Open => true,
            SidebarMode::Closed => false,
            SidebarMode::Auto => viewport_width >= 120,
        }
    }

    /// Toggles sidebar between Open and Closed (`Ctrl+B`), persisting `tui.toml`.
    pub fn toggle_sidebar(&mut self, viewport_width: u16) {
        let currently_visible = self.sidebar_visible(viewport_width);
        self.prefs.sidebar = if currently_visible {
            SidebarMode::Closed
        } else {
            SidebarMode::Open
        };
        self.persist_prefs();
    }

    /// Displays a transient toast notification.
    pub fn set_toast(&mut self, text: impl Into<String>, is_error: bool) {
        self.toast = Some(ToastMessage {
            text: text.into(),
            is_error,
            ticks_remaining: 75,
        });
    }

    /// Appends a timeline item to the chat and persists user/assistant messages to `tui_chats.json`.
    pub fn push_timeline_item(&mut self, chat_id: &str, item: ChatTimelineItem) {
        let persist_msg = match &item {
            ChatTimelineItem::UserMessage {
                sender,
                text,
                timestamp_ms,
                ..
            } => Some(StoredChatMessagePreview {
                role: "user".to_owned(),
                sender: sender.clone(),
                text_preview: text.clone(),
                timestamp_ms: *timestamp_ms,
                duration_ms: None,
            }),
            ChatTimelineItem::AssistantMessage {
                text,
                duration_ms,
                timestamp_ms,
            } => Some(StoredChatMessagePreview {
                role: "assistant".to_owned(),
                sender: self.gateway_cfg.soul.name.clone(),
                text_preview: text.clone(),
                timestamp_ms: *timestamp_ms,
                duration_ms: *duration_ms,
            }),
            _ => None,
        };
        let is_reply = matches!(item, ChatTimelineItem::AssistantMessage { .. });
        let entry = self.timelines.entry(chat_id.to_owned()).or_default();
        entry.push(item);
        if is_reply {
            self.conv.last_reply = Some((chat_id.to_owned(), entry.len() - 1));
        }
        if let Some(msg) = persist_msg {
            self.chats.append_message(chat_id, msg);
            self.persist_chats();
        }
        if self.chats.active_chat_id == chat_id && self.auto_scroll {
            self.chat_scroll = usize::MAX;
        }
    }

    /// Active chat timeline items.
    #[must_use]
    pub fn active_timeline(&self) -> &[ChatTimelineItem] {
        self.timelines
            .get(&self.chats.active_chat_id)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// Refreshes plugin entries from `gateway_cfg` while preserving live [`PluginUiState`].
    pub fn refresh_plugins_from_config(&mut self) {
        let prev_states: HashMap<String, PluginUiState> = self
            .plugins
            .iter()
            .map(|p| (p.id.clone(), p.state.clone()))
            .collect();
        let mut next = build_plugin_entries(&self.gateway_cfg, &self.paths);
        for item in &mut next {
            if let Some(prev) = prev_states.get(&item.id) {
                item.state = prev.clone();
            }
        }
        self.plugins = next;
    }

    /// Sets live [`PluginUiState`] for `plugin_id`.
    pub fn set_plugin_state(&mut self, plugin_id: &str, state: PluginUiState) {
        if let Some(p) = self.plugins.iter_mut().find(|p| p.id == plugin_id) {
            p.state = state;
        }
    }

    /// Persists `TuiPreferences` to `<config_dir>/tui.toml`.
    pub fn persist_prefs(&self) {
        let path = TuiPreferences::path_for_config(&self.paths.config_path);
        let _ = self.prefs.save_to_path(&path);
    }

    /// Persists `LocalChatStore` to `<data_dir>/tui_chats.json`.
    pub fn persist_chats(&self) {
        let path = LocalChatStore::path_for_data_dir(&self.paths.data_dir);
        let _ = self.chats.save_to_path(&path);
    }

    /// Refreshes `SecretsUiSummary` keys from `secrets.toml` (without reading plaintext values).
    pub fn refresh_secrets_summary(&mut self) {
        if let Ok((top, env_keys)) = list_secret_keys(&self.paths.secrets_path) {
            self.secrets_summary.top_level_keys = top;
            self.secrets_summary.env_keys = env_keys;
        }
    }

    /// Builds the full list of slash commands for the current mode (`lab_mode` + Gateway commands).
    #[must_use]
    pub fn available_slash_commands(&self) -> Vec<SlashCommandItem> {
        let mut cmds = vec![
            SlashCommandItem {
                name: "new".to_owned(),
                usage: "/new [name]".to_owned(),
                description: "Create a new local chat (same persistent soul)".to_owned(),
                current_value: None,
                icon: "+",
                lab_only: false,
                from_plugin: None,
            },
            SlashCommandItem {
                name: "rename".to_owned(),
                usage: "/rename <title>".to_owned(),
                description: "Rename active chat and lock against auto-retitle".to_owned(),
                current_value: Some(self.chats.active_chat().title.clone()),
                icon: "✎",
                lab_only: false,
                from_plugin: None,
            },
            SlashCommandItem {
                name: "delete".to_owned(),
                usage: "/delete".to_owned(),
                description: "Delete active chat (or clear if it is the last chat)".to_owned(),
                current_value: None,
                icon: "✕",
                lab_only: false,
                from_plugin: None,
            },
            SlashCommandItem {
                name: "clear".to_owned(),
                usage: "/clear".to_owned(),
                description: "Clear active chat viewport".to_owned(),
                current_value: None,
                icon: "⌫",
                lab_only: false,
                from_plugin: None,
            },
            SlashCommandItem {
                name: "attach".to_owned(),
                usage: "/attach <path>".to_owned(),
                description: "Attach image/PDF card or import file via workspace".to_owned(),
                current_value: if self.composer.pending_attachments.is_empty() {
                    None
                } else {
                    Some(format!("{} attached", self.composer.pending_attachments.len()))
                },
                icon: "⊕",
                lab_only: false,
                from_plugin: None,
            },
            SlashCommandItem {
                name: "detach".to_owned(),
                usage: "/detach [id]".to_owned(),
                description: "Remove attachment #id or clear all pending cards".to_owned(),
                current_value: None,
                icon: "✕",
                lab_only: false,
                from_plugin: None,
            },
            SlashCommandItem {
                name: "copy".to_owned(),
                usage: "/copy".to_owned(),
                description: "Copy latest assistant reply to clipboard".to_owned(),
                current_value: None,
                icon: "⎘",
                lab_only: false,
                from_plugin: None,
            },
            SlashCommandItem {
                name: "settings".to_owned(),
                usage: "/settings".to_owned(),
                description: "Open 8-tab Settings overlay".to_owned(),
                current_value: Some(format!("{} tabs", SettingsTab::ALL.len())),
                icon: "⚙",
                lab_only: false,
                from_plugin: None,
            },
            SlashCommandItem {
                name: "log".to_owned(),
                usage: "/log".to_owned(),
                description: "Open the runtime log (levels, sources, search)".to_owned(),
                current_value: Some(self.log_error_summary()),
                icon: "§",
                lab_only: false,
                from_plugin: None,
            },
            SlashCommandItem {
                name: "doctor".to_owned(),
                usage: "/doctor".to_owned(),
                description: "Check config, secrets, provider, plugins, embedder and data dir".to_owned(),
                current_value: None,
                icon: "+",
                lab_only: false,
                from_plugin: None,
            },
            SlashCommandItem {
                name: "retry".to_owned(),
                usage: "/retry".to_owned(),
                description: "Reload the configuration and boot the gateway again".to_owned(),
                current_value: None,
                icon: "↻",
                lab_only: false,
                from_plugin: None,
            },
            SlashCommandItem {
                name: "plugins".to_owned(),
                usage: "/plugins".to_owned(),
                description: "Open Plugins settings & TOML editor".to_owned(),
                current_value: Some(self.plugins_live_summary()),
                icon: "◈",
                lab_only: false,
                from_plugin: None,
            },
            SlashCommandItem {
                name: "model".to_owned(),
                usage: "/model".to_owned(),
                description: format!("Pick provider and model (now {})", self.model_name),
                current_value: Some(self.model_name.clone()),
                icon: "◆",
                lab_only: false,
                from_plugin: None,
            },
            SlashCommandItem {
                name: "valve".to_owned(),
                usage: "/valve".to_owned(),
                description: "Open Valve policy & toggles".to_owned(),
                current_value: Some(self.valve_mode.clone()),
                icon: "⛨",
                lab_only: false,
                from_plugin: None,
            },
            SlashCommandItem {
                name: "help".to_owned(),
                usage: "/help".to_owned(),
                description: "Show available commands".to_owned(),
                current_value: None,
                icon: "?",
                lab_only: false,
                from_plugin: None,
            },
            SlashCommandItem {
                name: "lab".to_owned(),
                usage: "/lab".to_owned(),
                description: if self.lab_mode {
                    "Lab simulation active (use /exit to leave)".to_owned()
                } else {
                    "Enter Lab multi-actor & brain trace mode".to_owned()
                },
                current_value: Some(if self.lab_mode { "on" } else { "off" }.to_owned()),
                icon: "⚗",
                lab_only: false,
                from_plugin: None,
            },
            SlashCommandItem {
                name: "exit".to_owned(),
                usage: "/exit".to_owned(),
                description: if self.lab_mode {
                    "Exit Lab mode back to normal chat".to_owned()
                } else {
                    "Quit MonoAgens TUI".to_owned()
                },
                current_value: None,
                icon: "⏻",
                lab_only: false,
                from_plugin: None,
            },
        ];

        if self.lab_mode {
            cmds.push(SlashCommandItem {
                name: "as".to_owned(),
                usage: "/as <actor>".to_owned(),
                description: "Switch simulated sender actor".to_owned(),
                current_value: Some(self.lab_session.current_actor.as_str().to_owned()),
                icon: "☺",
                lab_only: true,
                from_plugin: None,
            });
            cmds.push(SlashCommandItem {
                name: "group".to_owned(),
                usage: "/group <name> <members...>".to_owned(),
                description: "Enter simulated multi-actor group".to_owned(),
                current_value: if self.lab_session.is_group {
                    Some(self.lab_session.place.as_str().to_owned())
                } else {
                    None
                },
                icon: "◎",
                lab_only: true,
                from_plugin: None,
            });
            cmds.push(SlashCommandItem {
                name: "dm".to_owned(),
                usage: "/dm [actor]".to_owned(),
                description: "Return to 1:1 DM simulation".to_owned(),
                current_value: None,
                icon: "✉",
                lab_only: true,
                from_plugin: None,
            });
            cmds.push(SlashCommandItem {
                name: "brain".to_owned(),
                usage: "/brain".to_owned(),
                description: "Toggle brain telemetry lines in Lab mode".to_owned(),
                current_value: Some(if self.prefs.show_brain_trace { "on" } else { "off" }.to_owned()),
                icon: "⌘",
                lab_only: true,
                from_plugin: None,
            });
        }

        cmds.push(SlashCommandItem {
            name: "login".to_owned(),
            usage: "/login".to_owned(),
            description: "Set or change provider API keys".to_owned(),
            current_value: self.catalog.login_status(self.gateway_cfg.model.catalog_provider.as_deref()),
            icon: "⚿",
            lab_only: false,
            from_plugin: None,
        });
        cmds.push(SlashCommandItem {
            name: "usage".to_owned(),
            usage: "/usage".to_owned(),
            description: "Tokens per purpose today and budget left".to_owned(),
            current_value: Some(format!(
                "{}/{}",
                self.daily_tokens_used, self.daily_token_budget
            )),
            icon: "Σ",
            lab_only: false,
            from_plugin: None,
        });
        for (name, description, icon) in [
            ("approvals", "Pending approvals (grant or deny, including adapter holds)", "⚑"),
            ("people", "Persons, accounts, roles and account links", "☺"),
            ("policy", "Read-only valve toggles, absolute lines, Cedar rules and RBAC", "§"),
            ("activity", "Read-only feed of adapter messages and what the soul did", "≡"),
            ("events", "Event log with filters, explain and traces", "▤"),
            ("status", "Gateway owner, data dir, model, plugins, queue and budget", "ℹ"),
        ] {
            cmds.push(SlashCommandItem {
                name: name.to_owned(),
                usage: format!("/{name}"),
                description: description.to_owned(),
                current_value: None,
                icon,
                lab_only: false,
                from_plugin: None,
            });
        }

        for (name, usage, description, icon) in [
            ("why", "/why", "Explain the valve decision behind the latest reply", "?"),
            ("export", "/export [path]", "Write this chat as Markdown (default <data_dir>/exports/)", "⇩"),
            ("chat", "/chat pin | search <text>", "Pin the chat, or filter the chat list by title and text", "≡"),
            ("notify", "/notify off|bell|desktop|both", "Notify when unfocused and a reply arrives", "♪"),
            ("preview", "/preview on|off", "Show the reply draft while the model writes", "…"),
        ] {
            cmds.push(SlashCommandItem {
                name: name.to_owned(),
                usage: usage.to_owned(),
                description: description.to_owned(),
                current_value: match name {
                    "notify" => Some(self.prefs.conv.notifications.label().to_owned()),
                    "preview" => Some(if self.prefs.conv.live_draft_preview { "on" } else { "off" }.to_owned()),
                    _ => None,
                },
                icon,
                lab_only: false,
                from_plugin: None,
            });
        }

        for spec in &self.gateway_commands {
            if cmds.iter().any(|c| c.name == spec.name) {
                continue;
            }
            cmds.push(SlashCommandItem {
                name: spec.name.clone(),
                usage: spec
                    .usage
                    .clone()
                    .unwrap_or_else(|| format!("/{}", spec.name)),
                description: spec.summary.clone(),
                current_value: spec
                    .owner_plugin
                    .as_ref()
                    .map(|b| b.as_str().to_owned()),
                icon: "›",
                lab_only: false,
                from_plugin: spec.owner_plugin.as_ref().map(|b| b.as_str().to_owned()),
            });
        }

        cmds
    }

    /// Updates `self.autocomplete` based on current composer `/` prefix.
    pub fn filter_autocomplete(&mut self) {
        if !self.composer.is_slash_mode() {
            self.autocomplete.clear();
            self.autocomplete_idx = 0;
            return;
        }
        let query = self
            .composer
            .text
            .trim_start()
            .trim_start_matches('/')
            .to_ascii_lowercase();
        let mut matches: Vec<SlashCommandItem> = self
            .available_slash_commands()
            .into_iter()
            .filter(|cmd| {
                query.is_empty()
                    || cmd.name.to_ascii_lowercase().starts_with(&query)
                    || cmd.name.to_ascii_lowercase().contains(&query)
                    || cmd.description.to_ascii_lowercase().contains(&query)
            })
            .collect();
        if matches.len() <= self.autocomplete_idx {
            self.autocomplete_idx = 0;
        }
        self.autocomplete.clear();
        self.autocomplete.append(&mut matches);
        self.sync_autocomplete_window();
    }

    /// The `/` menu is open while the composer starts with `/`, has no space yet and some command
    /// matches (it closes on Esc, Enter, or when the text stops fitting).
    #[must_use]
    pub fn slash_menu_open(&self) -> bool {
        let t = self.composer.text.trim_start();
        self.composer.is_slash_mode()
            && !t.contains(char::is_whitespace)
            && !self.autocomplete.is_empty()
    }

    /// Rows the slash menu wants (at most 10, or `autocomplete_limit` when smaller).
    #[must_use]
    pub fn autocomplete_visible_rows(&self) -> usize {
        self.autocomplete
            .len()
            .min(self.prefs.autocomplete_limit.clamp(1, 10))
    }

    fn sync_autocomplete_window(&mut self) {
        let visible = self.autocomplete_visible_rows();
        self.autocomplete_off = crate::render::scroll_window(
            self.autocomplete_idx,
            self.autocomplete_off,
            visible,
            self.autocomplete.len(),
        );
    }

    /// Moves the slash selection by `delta` rows (wrapping), keeping it on screen.
    pub fn autocomplete_move(&mut self, delta: isize) {
        let n = self.autocomplete.len();
        if n == 0 {
            return;
        }
        let cur = self.autocomplete_idx as isize;
        self.autocomplete_idx = (cur + delta).rem_euclid(n as isize) as usize;
        self.sync_autocomplete_window();
    }

    /// Moves the slash selection by one page without wrapping.
    pub fn autocomplete_page(&mut self, down: bool) {
        let n = self.autocomplete.len();
        if n == 0 {
            return;
        }
        let page = self.autocomplete_visible_rows().max(1);
        self.autocomplete_idx = if down {
            (self.autocomplete_idx + page).min(n - 1)
        } else {
            self.autocomplete_idx.saturating_sub(page)
        };
        self.sync_autocomplete_window();
    }

    fn log_error_summary(&self) -> String {
        let errors = self
            .log
            .entries
            .snapshot()
            .iter()
            .filter(|e| e.level == RunLogLevel::Error)
            .count();
        match errors {
            1 => "1 error".to_owned(),
            n => format!("{n} errors"),
        }
    }

    fn plugins_live_summary(&self) -> String {
        let installed: Vec<_> = self.plugins.iter().filter(|p| p.installed).collect();
        let running = installed
            .iter()
            .filter(|p| p.state == crate::settings::PluginUiState::Running)
            .count();
        format!("{running}/{} running", installed.len())
    }

    /// Returns `true` when the `workspace` plugin is currently running.
    #[must_use]
    pub fn is_workspace_plugin_active(&self) -> bool {
        self.plugins
            .iter()
            .any(|p| p.id == "workspace" && matches!(p.state, PluginUiState::Running))
    }

    /// Returns `true` when the `pdf-reader` plugin is currently running.
    #[must_use]
    pub fn is_pdf_reader_plugin_active(&self) -> bool {
        self.plugins
            .iter()
            .any(|p| p.id == "pdf-reader" && matches!(p.state, PluginUiState::Running))
    }

    fn handle_supported_attachment(&mut self, att: PendingAttachment) {
        let is_pdf = matches!(
            &att.kind,
            PendingAttachmentKind::Document { mime, .. }
                if mime.eq_ignore_ascii_case("application/pdf")
        );
        if is_pdf {
            if self.is_pdf_reader_plugin_active() {
                let label = att.label.clone();
                let id = self.composer.add_attachment(att);
                self.set_toast(format!("Attached #{id}: {label}"), false);
            } else if self.is_workspace_plugin_active() {
                if let Some(src) = att.source_path {
                    self.handle_host_file_path_import(&src);
                }
            } else {
                self.set_toast(
                    "PDF requires 'pdf-reader' (or 'workspace') plugin — start it from PLUGINS or /settings",
                    true,
                );
            }
            return;
        }
        let label = att.label.clone();
        let id = self.composer.add_attachment(att);
        self.set_toast(format!("Attached #{id}: {label}"), false);
    }

    #[must_use]
    fn current_actor_id_str(&self) -> &str {
        if self.lab_mode {
            self.lab_session.current_actor.as_str()
        } else {
            self.lab_session.owner_actor.as_str()
        }
    }

    fn handle_host_file_path_import(&mut self, path: &Path) {
        let fname = path
            .file_name()
            .and_then(|n| n.to_str())
            .filter(|s| !s.is_empty())
            .unwrap_or("file")
            .to_owned();
        if !self.is_workspace_plugin_active() {
            self.set_toast(
                format!(
                    "Workspace plugin inactive — start 'workspace' plugin to import '{fname}' into Mono's workspace"
                ),
                true,
            );
            return;
        }
        match import_host_file_to_workspace(
            path,
            &self.paths.config_path,
            &self.paths.data_dir,
            self.current_actor_id_str(),
        ) {
            Ok(rel_path) => {
                let prefix_space = self
                    .composer
                    .text
                    .chars()
                    .last()
                    .is_some_and(|c| !c.is_whitespace());
                if prefix_space {
                    self.composer.insert_char(' ');
                }
                self.composer.insert_str(&rel_path);
                self.set_toast(format!("Imported to workspace: {rel_path}"), false);
            }
            Err(err) => {
                self.set_toast(err, true);
            }
        }
    }

    /// Handles bracketed paste (`Event::Paste`) or clipboard text paste across Wizard,
    /// Settings modal, or Chat composer (smart media/PDF cards, workspace-gated host file
    /// imports, and long-text collapse).
    pub fn handle_paste(&mut self, raw: &str) -> Vec<AppAction> {
        self.selection = None;
        if self.catalog.is_open() {
            self.catalog.handle_paste(raw);
            return Vec::new();
        }
        if let Some(w) = &mut self.wizard {
            if w.step == WizardStep::Step1Provider && w.editing {
                let cleaned = raw.trim_matches(['\r', '\n']);
                match w.selected_field {
                    0 => w.endpoint.push_str(cleaned),
                    1 => w.api_key.push_str(cleaned),
                    2 => w.model.push_str(cleaned),
                    _ => {}
                }
            }
            return Vec::new();
        }

        if self.settings.open {
            let cleaned = raw.trim_matches(['\r', '\n']);
            if let Some(edit) = &mut self.settings.editing {
                edit.buffer.push_str(cleaned);
            } else if self.settings.plugin_detail.is_none() {
                self.settings.search_query.push_str(cleaned);
                self.settings.selected_index = 0;
                self.settings.scroll_offset = 0;
            }
            return Vec::new();
        }

        let items = classify_paste_input(raw);
        for item in items {
            match item {
                PasteItem::Attach(att) => {
                    self.handle_supported_attachment(att);
                }
                PasteItem::PlainPath(path_str) => {
                    self.handle_host_file_path_import(Path::new(&path_str));
                }
                PasteItem::PlainText(text) => {
                    self.composer.insert_str(&text);
                }
            }
        }
        self.filter_autocomplete();
        Vec::new()
    }

    /// Routes one terminal event. Resize and focus-gain only force a redraw (focus-gain also
    /// clears, since another program may have painted over the screen); no state changes.
    pub fn handle_terminal_event(
        &mut self,
        ev: Event,
        hit: &HitRegions,
        viewport: Rect,
    ) -> EventOutcome {
        self.log.view_rows = viewport
            .height
            .saturating_sub(crate::log::LOG_CHROME_ROWS)
            .max(1) as usize;
        self.catalog.view_rows = viewport.height.saturating_sub(5).max(1) as usize;
        self.gov.view_rows = viewport
            .height
            .saturating_sub(crate::views::GOV_CHROME_ROWS)
            .max(1) as usize;
        match ev {
            Event::Key(k) => EventOutcome {
                actions: self.handle_key_event(k),
                redraw: true,
                clear: false,
            },
            Event::Mouse(m) => EventOutcome {
                actions: self.handle_mouse_event(m, hit, viewport),
                redraw: true,
                clear: false,
            },
            Event::Paste(text) => EventOutcome {
                actions: self.handle_paste(&text),
                redraw: true,
                clear: false,
            },
            Event::Resize(..) => EventOutcome {
                actions: Vec::new(),
                redraw: true,
                clear: false,
            },
            Event::FocusGained => {
                self.conv_focus(true);
                EventOutcome {
                    actions: Vec::new(),
                    redraw: true,
                    clear: true,
                }
            }
            Event::FocusLost => {
                self.conv_focus(false);
                EventOutcome::default()
            }
        }
    }

    /// Handles a key event across Wizard, Settings modal, Approval card, or Chat composer.
    pub fn handle_key_event(&mut self, key: KeyEvent) -> Vec<AppAction> {
        let mut actions = Vec::new();
        self.selection = None;

        // The help overlay and the command palette own the keyboard while open (before any
        // conversation shortcut, so Esc closes them instead of interrupting a turn).
        if (self.ux.help.is_some() || self.ux.palette.is_some())
            && let Some(handled) = self.global_key(key)
        {
            return handled;
        }

        // Conversation keys (interrupt, history search, editor, retry, message actions). Not
        // active while a settings overlay, selector, wizard or log/catalog/governance panel is open.
        if let Some(handled) = self.conv_key(key) {
            return handled;
        }

        // Global Ctrl+C / Ctrl+Q exit
        if key.modifiers.contains(KeyModifiers::CONTROL)
            && matches!(key.code, KeyCode::Char('c') | KeyCode::Char('q'))
        {
            self.should_quit = true;
            return actions;
        }

        // Help overlay, command palette and the rebindable global keys (suspend, display reset,
        // sidebar) come from the action table.
        if let Some(actions) = self.global_key(key) {
            return actions;
        }

        // The `/log` overlay owns the keyboard while open
        if self.log.open {
            return self.log.handle_key(key);
        }

        // The `/model`, `/login` and `/usage` overlays own the keyboard while open
        if self.catalog.is_open() {
            return self.catalog.handle_key(key);
        }
        if self.gov.is_open() {
            self.gov.lab_mode = self.lab_mode;
            return self.gov.handle_key(key);
        }

        // First-run Setup Wizard intercepts input until completed or skipped
        if self.wizard.is_some() {
            return self.handle_wizard_key(key);
        }

        // Config-error state: Ctrl+R retries the gateway boot (history search is off in this state)
        if self.config_error.is_some()
            && key.modifiers.contains(KeyModifiers::CONTROL)
            && key.code == KeyCode::Char('r')
            && !self.settings.open
        {
            return vec![AppAction::RetryBoot];
        }

        // Alt+Up pulls the last queued message back into the composer
        if key.modifiers.contains(KeyModifiers::ALT)
            && key.code == KeyCode::Up
            && !self.settings.open
        {
            self.unqueue_last();
            return actions;
        }

        // Settings overlay intercepts input while open
        if self.settings.open {
            return self.settings_key(key);
        }

        // Composer Ctrl+V clipboard paste (image or text)
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('v') {
            actions.push(AppAction::PasteFromClipboard);
            return actions;
        }

        // Pending Approval inline card shortcuts when composer is empty
        if self.composer.text.is_empty()
            && let Some(card) = &mut self.pending_approval
        {
            match key.code {
                KeyCode::Left => {
                    card.selected_approve = true;
                    return actions;
                }
                KeyCode::Right => {
                    card.selected_approve = false;
                    return actions;
                }
                KeyCode::Char('y') | KeyCode::Char('Y') => {
                    let c = self.pending_approval.take().expect("checked");
                    actions.push(AppAction::ResolveApproval {
                        action_id: c.action_id,
                        h_act: c.h_act,
                        state_epoch: c.state_epoch,
                        approved: true,
                    });
                    return actions;
                }
                KeyCode::Char('n') | KeyCode::Char('N') => {
                    let c = self.pending_approval.take().expect("checked");
                    actions.push(AppAction::ResolveApproval {
                        action_id: c.action_id,
                        h_act: c.h_act,
                        state_epoch: c.state_epoch,
                        approved: false,
                    });
                    return actions;
                }
                KeyCode::Enter => {
                    let c = self.pending_approval.take().expect("checked");
                    actions.push(AppAction::ResolveApproval {
                        action_id: c.action_id,
                        h_act: c.h_act,
                        state_epoch: c.state_epoch,
                        approved: c.selected_approve,
                    });
                    return actions;
                }
                _ => {}
            }
        }

        // Slash autocomplete navigation
        if self.slash_menu_open() {
            match key.code {
                KeyCode::Tab => {
                    if let Some(item) = self.autocomplete.get(self.autocomplete_idx) {
                        self.composer.text = format!("/{} ", item.name);
                        self.composer.cursor = self.composer.text.len();
                        self.filter_autocomplete();
                    }
                    return actions;
                }
                KeyCode::Up => {
                    self.autocomplete_move(-1);
                    return actions;
                }
                KeyCode::Down => {
                    self.autocomplete_move(1);
                    return actions;
                }
                KeyCode::PageUp => {
                    self.autocomplete_page(false);
                    return actions;
                }
                KeyCode::PageDown => {
                    self.autocomplete_page(true);
                    return actions;
                }
                KeyCode::Enter if !key.modifiers.contains(KeyModifiers::SHIFT) => {
                    // Enter runs the highlighted command (typed prefixes are completed first).
                    if let Some(item) = self.autocomplete.get(self.autocomplete_idx).cloned() {
                        let typed = self.composer.text.trim().trim_start_matches('/').to_owned();
                        if !self.autocomplete.iter().any(|c| c.name == typed) {
                            self.composer.text = format!("/{}", item.name);
                            self.composer.cursor = self.composer.text.len();
                        }
                    }
                }
                _ => {}
            }
        }

        match key.code {
            KeyCode::Enter if key.modifiers.contains(KeyModifiers::SHIFT) => {
                self.composer.insert_newline();
                self.filter_autocomplete();
            }
            KeyCode::Enter => {
                actions.extend(self.submit_composer());
            }
            KeyCode::Backspace => {
                self.composer.backspace();
                self.filter_autocomplete();
            }
            KeyCode::Left => self.composer.move_left(),
            KeyCode::Right => self.composer.move_right(),
            KeyCode::Up => self.composer.history_up(),
            KeyCode::Down => self.composer.history_down(),
            KeyCode::PageUp => {
                self.auto_scroll = false;
                self.chat_scroll = self.chat_scroll.saturating_sub(5);
            }
            KeyCode::PageDown => {
                self.chat_scroll = self.chat_scroll.saturating_add(5);
            }
            KeyCode::Esc => {
                if !self.composer.text.is_empty() || !self.composer.pending_attachments.is_empty() {
                    self.composer.text.clear();
                    self.composer.cursor = 0;
                    self.composer.clear_attachments();
                    self.filter_autocomplete();
                }
            }
            KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.composer.insert_char(ch);
                self.filter_autocomplete();
            }
            _ => {}
        }

        actions
    }

    /// Submits the current composer text (either slash command or chat message).
    pub fn submit_composer(&mut self) -> Vec<AppAction> {
        let raw = self.composer.take_submitted();
        self.filter_autocomplete();
        let trimmed = raw.trim();
        if trimmed.starts_with('/') {
            return self.execute_slash_command(trimmed);
        }

        let attachments = std::mem::take(&mut self.composer.pending_attachments);
        self.composer.next_attachment_id = 1;

        if trimmed.is_empty() && attachments.is_empty() {
            return Vec::new();
        }

        if self.config_error.is_some() {
            self.set_toast(
                "Chat is disabled: fix the configuration in /settings, then /retry",
                true,
            );
            return Vec::new();
        }
        if let Some(reason) = self.chat_disabled_reason.clone() {
            self.set_toast(format!("Chat is disabled: {reason}"), true);
            return Vec::new();
        }

        let mut display_text = trimmed.to_owned();
        for att in &attachments {
            let tok = att.inline_token();
            if !display_text.contains(&tok) {
                if !display_text.is_empty() {
                    display_text.push(' ');
                }
                display_text.push_str(&tok);
            }
        }

        if self.is_busy() {
            let active = self.chats.active_chat();
            let msg = QueuedMessage {
                chat_id: active.id.clone(),
                place_id: active.place_id.clone(),
                text: display_text,
                attachments,
            };
            self.turn.push_queued(msg);
            return Vec::new();
        }

        let active = self.chats.active_chat();
        let (chat_id, place_id) = (active.id.clone(), active.place_id.clone());
        vec![self.begin_user_turn(chat_id, place_id, display_text, attachments)]
    }

    /// Pushes the user bubble, marks the turn busy and returns the action that submits it.
    pub(crate) fn begin_user_turn(
        &mut self,
        chat_id: String,
        place_id: String,
        text: String,
        attachments: Vec<PendingAttachment>,
    ) -> AppAction {
        let sender = if self.lab_mode && self.lab_session.is_group {
            self.lab_session.current_actor.as_str().to_owned()
        } else {
            self.lab_session.owner_actor.as_str().to_owned()
        };
        let is_group = self.lab_mode && self.lab_session.is_group;
        self.push_timeline_item(
            &chat_id,
            ChatTimelineItem::UserMessage {
                sender,
                text: text.clone(),
                timestamp_ms: 0,
                is_group,
            },
        );
        self.thinking = true;
        self.thinking_phase = "Reasoning…".to_owned();
        self.thinking_elapsed_ms = 0;
        self.turn.begin(&chat_id);
        self.conv_on_turn_begin(&chat_id, &place_id, &text, &attachments);

        AppAction::SendUserMessage {
            chat_id,
            place_id,
            text,
            attachments,
        }
    }

    /// Routes an assistant reply. `turn` is the turn the reply belongs to when known: a reply of
    /// an earlier turn is shown in that turn's chat without touching the running turn's busy state.
    pub fn on_assistant_reply(
        &mut self,
        turn: Option<u64>,
        chat_id: Option<String>,
        text: String,
        duration_ms: u64,
    ) -> ReplyDisposition {
        let late = turn.is_some_and(|t| t != self.turn.seq());
        if late {
            let target = turn
                .and_then(|t| self.turn.chat_of(t).map(str::to_owned))
                .or(chat_id)
                .unwrap_or_else(|| self.chats.active_chat_id.clone());
            self.push_timeline_item(
                &target,
                ChatTimelineItem::AssistantMessage {
                    text,
                    duration_ms: (duration_ms > 0).then_some(duration_ms),
                    timestamp_ms: 0,
                },
            );
            return ReplyDisposition::LateReply;
        }
        let elapsed_ms = if duration_ms > 0 {
            duration_ms
        } else {
            self.turn.elapsed_ms()
        };
        let target = self
            .turn
            .chat_id()
            .map(str::to_owned)
            .or(chat_id)
            .unwrap_or_else(|| self.chats.active_chat_id.clone());
        self.finish_busy();
        self.thinking_elapsed_ms = elapsed_ms;
        self.push_timeline_item(
            &target,
            ChatTimelineItem::AssistantMessage {
                text,
                duration_ms: Some(elapsed_ms),
                timestamp_ms: 0,
            },
        );
        ReplyDisposition::CurrentTurn
    }

    /// Stores a runtime log entry; an error entry raises a toast pointing at `/log`.
    pub fn on_log_entry(&mut self, entry: monoagens_gateway::LogEntry) {
        if let Some(toast) = self.log.push(entry) {
            self.set_toast(toast, true);
        }
    }

    /// Enters the "config error" state: banner, chat disabled, `/settings` on the offending key.
    pub fn set_config_error(&mut self, message: String, key: Option<String>) {
        self.config_error = Some(ConfigErrorState { message, key });
    }

    /// Opens `/settings` on the tab and row of the offending key when it is known.
    pub fn open_settings_on_error_key(&mut self) {
        let key = self.config_error.as_ref().and_then(|e| e.key.clone());
        match key {
            Some(key) => {
                self.settings.open_tab(settings_tab_for_key(&key));
                let rows = self.current_settings_rows();
                if let Some(pos) = rows.iter().position(|r| r.id == key) {
                    self.settings.selected_index = pos;
                    self.settings.scroll_offset = pos;
                }
            }
            None => self.settings.open_tab(SettingsTab::Appearance),
        }
    }

    /// Ends the busy state (reply, error, silent turn, door block, ...).
    pub fn finish_busy(&mut self) {
        self.thinking = false;
        self.turn.end();
        self.conv_on_busy_end();
    }

    /// Records that the owner step of turn `seq` returned; a reply may still follow within the
    /// grace window.
    pub fn on_turn_finished(&mut self, seq: u64) {
        if self.thinking {
            self.turn.mark_finished(seq);
        }
    }

    /// Ends busy because turn `seq` failed; stale turn numbers are ignored. Returns the chat to
    /// show the error in.
    pub fn on_turn_error(&mut self, seq: u64) -> Option<String> {
        if !self.turn.is_current(seq) {
            return None;
        }
        let chat = self.turn.chat_id().map(str::to_owned);
        self.finish_busy();
        self.conv_mark_turn_error();
        chat
    }

    /// Refreshes the elapsed counter and ends a silent turn once the grace window passed.
    /// Returns `true` when the silent-turn notice was just pushed (caller logs `turn.no_reply`).
    pub fn tick_turn(&mut self) -> bool {
        if !self.thinking {
            return false;
        }
        self.thinking_elapsed_ms = self.turn.elapsed_ms();
        if self.turn.grace_expired() {
            let chat = self
                .turn
                .chat_id()
                .map_or_else(|| self.chats.active_chat_id.clone(), str::to_owned);
            self.finish_busy();
            self.push_timeline_item(
                &chat,
                ChatTimelineItem::SystemNotice {
                    text: NO_REPLY_NOTICE.to_owned(),
                    is_error: false,
                },
            );
            return true;
        }
        false
    }

    /// When idle, pops the oldest queued message and starts its turn.
    pub fn next_queued_action(&mut self) -> Option<AppAction> {
        if self.is_busy() {
            return None;
        }
        let msg = self.turn.pop_front_queued()?;
        Some(self.begin_user_turn(msg.chat_id, msg.place_id, msg.text, msg.attachments))
    }

    /// `Alt+Up`: moves the newest queued message back into the composer (ahead of any draft).
    pub fn unqueue_last(&mut self) {
        let Some(msg) = self.turn.pop_back_queued() else {
            return;
        };
        let draft = std::mem::take(&mut self.composer.text);
        self.composer.text = if draft.is_empty() {
            msg.text
        } else {
            format!("{}\n{draft}", msg.text)
        };
        self.composer.cursor = self.composer.text.len();
        // The text already carries the inline tokens, so restore attachments without re-inserting them.
        self.composer.pending_attachments.extend(msg.attachments);
        self.composer.next_attachment_id = self
            .composer
            .pending_attachments
            .iter()
            .map(|a| a.id)
            .max()
            .map_or(1, |m| m + 1);
        self.filter_autocomplete();
    }

    /// Opens a governance panel (with its initial requests).
    pub fn open_gov(&mut self, panel: crate::views::Panel) -> Vec<AppAction> {
        self.gov.lab_mode = self.lab_mode;
        self.gov.status.link = self.gateway_link.clone();
        if panel == crate::views::Panel::Policy {
            let toggles = ValveRuleTogglesState::load_from_config(&self.paths.config_path);
            self.gov.policy.toggles = crate::views::policy::toggle_rows(&toggles);
        }
        self.gov.open(panel)
    }

    /// Executes a local TUI slash command or forwards a Gateway/plugin command.
    pub fn execute_slash_command(&mut self, raw: &str) -> Vec<AppAction> {
        let mut parts = raw.split_whitespace();
        let cmd = parts.next().unwrap_or("");
        let rest = parts.collect::<Vec<_>>().join(" ");
        let active_id = self.chats.active_chat_id.clone();

        if let Some(handled) = self.conv_slash(cmd, &rest) {
            return handled;
        }

        match cmd {
            "/exit" | "/quit" | "/q" => {
                if self.lab_mode {
                    self.lab_mode = false;
                    self.lab_session.is_group = false;
                    self.lab_session.current_actor = self.lab_session.owner_actor.clone();
                    self.set_toast("Exited Lab mode", false);
                    self.push_timeline_item(
                        &active_id,
                        ChatTimelineItem::SystemNotice {
                            text: "Exited Lab mode back to normal chat.".to_owned(),
                            is_error: false,
                        },
                    );
                } else {
                    self.should_quit = true;
                }
                self.filter_autocomplete();
                Vec::new()
            }
            "/lab" => {
                if rest.eq_ignore_ascii_case("exit") || rest.eq_ignore_ascii_case("off") {
                    self.lab_mode = false;
                    self.lab_session.is_group = false;
                    self.lab_session.current_actor = self.lab_session.owner_actor.clone();
                    self.set_toast("Exited Lab mode", false);
                } else {
                    self.lab_mode = true;
                    self.set_toast("Entered Lab mode (/exit returns to chat)", false);
                    self.push_timeline_item(
                        &active_id,
                        ChatTimelineItem::SystemNotice {
                            text: "Lab mode active: /as, /group, /dm, /brain unlocked. Use /exit to leave Lab mode."
                                .to_owned(),
                            is_error: false,
                        },
                    );
                }
                self.filter_autocomplete();
                Vec::new()
            }
            "/settings" => {
                if self.config_error.is_some() {
                    self.open_settings_on_error_key();
                } else {
                    self.settings.open_tab(SettingsTab::Appearance);
                }
                Vec::new()
            }
            "/log" => {
                self.log.open();
                Vec::new()
            }
            "/approvals" => self.open_gov(crate::views::Panel::Approvals),
            "/people" => self.open_gov(crate::views::Panel::People),
            "/policy" => self.open_gov(crate::views::Panel::Policy),
            "/activity" => self.open_gov(crate::views::Panel::Activity),
            "/events" => self.open_gov(crate::views::Panel::Events),
            "/status" => self.open_gov(crate::views::Panel::Status),
            "/doctor" => vec![AppAction::RunDoctor],
            "/retry" => {
                if self.config_error.is_some() {
                    vec![AppAction::RetryBoot]
                } else {
                    self.set_toast("Nothing to retry: the gateway is running", false);
                    Vec::new()
                }
            }
            "/plugins" => {
                self.settings.open_tab(SettingsTab::Plugins);
                Vec::new()
            }
            "/model" => {
                let provider = self.gateway_cfg.model.catalog_provider.clone();
                self.catalog.open_picker(provider, self.model_name.clone())
            }
            "/login" => {
                let provider = self.gateway_cfg.model.catalog_provider.clone();
                self.catalog.open_login(provider, self.model_name.clone())
            }
            "/usage" => self.catalog.open_usage(),
            "/valve" => {
                self.settings.open_tab(SettingsTab::Valve);
                Vec::new()
            }
            "/clear" => {
                self.timelines.entry(active_id.clone()).or_default().clear();
                self.chats.clear_messages(&active_id);
                self.persist_chats();
                self.chat_scroll = 0;
                self.auto_scroll = true;
                self.set_toast("Cleared active chat", false);
                Vec::new()
            }
            "/attach" => {
                let target = rest.trim();
                if target.is_empty() {
                    self.set_toast("Usage: /attach <path>", true);
                    return Vec::new();
                }
                match inspect_file_for_attachment(target) {
                    FileAttachmentOutcome::Supported(att) => {
                        self.handle_supported_attachment(att);
                        self.filter_autocomplete();
                    }
                    FileAttachmentOutcome::UnsupportedPath(p) => {
                        self.handle_host_file_path_import(Path::new(&p));
                        self.filter_autocomplete();
                    }
                    FileAttachmentOutcome::NotFound(p) => {
                        self.set_toast(format!("File not found: {p}"), true);
                    }
                }
                Vec::new()
            }
            "/detach" => {
                let arg = rest.trim();
                if arg.is_empty() {
                    let count = self.composer.pending_attachments.len();
                    self.composer.clear_attachments();
                    self.filter_autocomplete();
                    if count == 0 {
                        self.set_toast("No pending attachments", false);
                    } else {
                        self.set_toast(format!("Cleared {count} pending attachment(s)"), false);
                    }
                } else {
                    match arg.trim_start_matches('#').parse::<usize>() {
                        Ok(id) => {
                            if self.composer.remove_attachment(id).is_some() {
                                self.filter_autocomplete();
                                self.set_toast(format!("Detached #{id}"), false);
                            } else {
                                self.set_toast(format!("No pending attachment #{id}"), true);
                            }
                        }
                        Err(_) => {
                            self.set_toast("Usage: /detach [id]", true);
                        }
                    }
                }
                Vec::new()
            }
            "/copy" => {
                let latest = self.active_timeline().iter().rev().find_map(|item| {
                    if let ChatTimelineItem::AssistantMessage { text, .. } = item {
                        Some(text.clone())
                    } else {
                        None
                    }
                });
                if let Some(reply) = latest {
                    self.set_toast("Copied latest reply to clipboard", false);
                    vec![AppAction::CopyToClipboard(reply)]
                } else {
                    self.set_toast("No assistant reply to copy", true);
                    Vec::new()
                }
            }
            "/new" => {
                let arg = rest.trim();
                let name = (!arg.is_empty()).then_some(arg);
                let created_id = self.chats.create_chat(name).id.clone();
                self.timelines.entry(created_id).or_default();
                self.auto_scroll = true;
                self.chat_scroll = 0;
                self.persist_chats();
                self.set_toast("Created new chat", false);
                Vec::new()
            }
            "/rename" => {
                let arg = rest.trim();
                if arg.is_empty() {
                    self.set_toast("Usage: /rename <title>", true);
                } else if self.chats.rename_chat(&active_id, arg) {
                    self.persist_chats();
                    self.set_toast(format!("Renamed & locked chat: {arg}"), false);
                }
                Vec::new()
            }
            "/delete" => {
                if self.chats.delete_chat(&active_id) {
                    self.timelines.remove(&active_id);
                    let next_id = self.chats.active_chat_id.clone();
                    self.timelines.entry(next_id).or_default();
                    self.auto_scroll = true;
                    self.chat_scroll = usize::MAX;
                    self.persist_chats();
                    self.set_toast("Deleted chat", false);
                } else {
                    self.timelines.entry(active_id.clone()).or_default().clear();
                    self.chats.clear_messages(&active_id);
                    self.chat_scroll = 0;
                    self.auto_scroll = true;
                    self.persist_chats();
                    self.set_toast("Cleared last remaining chat", false);
                }
                Vec::new()
            }
            "/as" => {
                if !self.lab_mode {
                    self.push_timeline_item(
                        &active_id,
                        ChatTimelineItem::SystemNotice {
                            text: "/as is only available in Lab mode. Run /lab first.".to_owned(),
                            is_error: true,
                        },
                    );
                    return Vec::new();
                }
                let who = rest.trim();
                if who.is_empty() {
                    self.set_toast("Usage: /as <actor>", true);
                    return Vec::new();
                }
                let actor = if who.eq_ignore_ascii_case("owner") {
                    self.lab_session.owner_actor.clone()
                } else {
                    ActorId::new(who)
                };
                self.lab_session
                    .known_actors
                    .insert(actor.as_str().to_owned());
                if !self.lab_session.audience.members.contains(&actor) {
                    let mut members = self.lab_session.audience.members.clone();
                    members.insert(actor.clone());
                    self.lab_session.audience = Audience::new(members);
                }
                self.lab_session.current_actor = actor.clone();
                self.set_toast(format!("Speaking as {}", actor.as_str()), false);
                Vec::new()
            }
            "/group" => {
                if !self.lab_mode {
                    self.push_timeline_item(
                        &active_id,
                        ChatTimelineItem::SystemNotice {
                            text: "/group is only available in Lab mode. Run /lab first.".to_owned(),
                            is_error: true,
                        },
                    );
                    return Vec::new();
                }
                let mut tokens = rest.split_whitespace();
                let Some(room) = tokens.next() else {
                    self.set_toast("Usage: /group <name> <members...>", true);
                    return Vec::new();
                };
                let members_raw = tokens.collect::<Vec<_>>().join(",");
                let mut members: Vec<ActorId> = members_raw
                    .split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(|m| {
                        if m.eq_ignore_ascii_case("owner") {
                            self.lab_session.owner_actor.clone()
                        } else {
                            ActorId::new(m)
                        }
                    })
                    .collect();
                if members.is_empty() {
                    members.push(self.lab_session.owner_actor.clone());
                    members.push(ActorId::new("alice"));
                    members.push(ActorId::new("bob"));
                }
                for m in &members {
                    self.lab_session.known_actors.insert(m.as_str().to_owned());
                }
                if !members.contains(&self.lab_session.current_actor) {
                    members.push(self.lab_session.current_actor.clone());
                }
                members.push(ActorId::new("soul"));
                self.lab_session.place = PlaceId::new(room);
                self.lab_session.audience = Audience::new(members);
                self.lab_session.is_group = true;
                self.set_toast(format!("Joined Lab group '{room}'"), false);
                Vec::new()
            }
            "/dm" => {
                if !self.lab_mode {
                    self.push_timeline_item(
                        &active_id,
                        ChatTimelineItem::SystemNotice {
                            text: "/dm is only available in Lab mode. Run /lab first.".to_owned(),
                            is_error: true,
                        },
                    );
                    return Vec::new();
                }
                let target = rest.trim();
                self.lab_session.is_group = false;
                self.lab_session.place = PlaceId::new("cli:terminal");
                self.lab_session.current_actor = if target.is_empty() || target.eq_ignore_ascii_case("owner") {
                    self.lab_session.owner_actor.clone()
                } else {
                    let a = ActorId::new(target);
                    self.lab_session.known_actors.insert(a.as_str().to_owned());
                    a
                };
                self.lab_session.audience = Audience::new([
                    self.lab_session.current_actor.clone(),
                    ActorId::new("soul"),
                ]);
                self.set_toast(
                    format!("Switched to 1:1 DM as {}", self.lab_session.current_actor.as_str()),
                    false,
                );
                Vec::new()
            }
            "/brain" => {
                if !self.lab_mode {
                    self.push_timeline_item(
                        &active_id,
                        ChatTimelineItem::SystemNotice {
                            text: "/brain is only available in Lab mode. Run /lab first.".to_owned(),
                            is_error: true,
                        },
                    );
                    return Vec::new();
                }
                self.prefs.show_brain_trace = !self.prefs.show_brain_trace;
                self.persist_prefs();
                self.set_toast(
                    format!(
                        "Brain telemetry {}",
                        if self.prefs.show_brain_trace { "on" } else { "off" }
                    ),
                    false,
                );
                Vec::new()
            }
            "/help" => {
                let listing = self
                    .available_slash_commands()
                    .into_iter()
                    .map(|c| format!("  {:<24} {}", c.usage, c.description))
                    .collect::<Vec<_>>()
                    .join("\n");
                self.push_timeline_item(
                    &active_id,
                    ChatTimelineItem::SystemNotice {
                        text: format!("Commands:\n{listing}"),
                        is_error: false,
                    },
                );
                Vec::new()
            }
            _ => vec![AppAction::SubmitGatewayCommand(raw.trim().to_owned())],
        }
    }

    /// Handles mouse wheel scrolling and left-clicks using [`HitRegions`].
    pub fn handle_mouse_event(
        &mut self,
        mouse: MouseEvent,
        hit: &HitRegions,
        viewport: Rect,
    ) -> Vec<AppAction> {
        let mut actions = Vec::new();
        let x = mouse.column;
        let y = mouse.row;

        if self.gov.is_open() {
            match mouse.kind {
                MouseEventKind::ScrollUp => self.gov.scroll_by(-3),
                MouseEventKind::ScrollDown => self.gov.scroll_by(3),
                _ => {}
            }
            return actions;
        }

        if self.log.open {
            match mouse.kind {
                MouseEventKind::ScrollUp => self.log.scroll_by(-3),
                MouseEventKind::ScrollDown => self.log.scroll_by(3),
                _ => {}
            }
            return actions;
        }
        if self.catalog.is_open() {
            match mouse.kind {
                MouseEventKind::ScrollUp => self.catalog.scroll_by(-3),
                MouseEventKind::ScrollDown => self.catalog.scroll_by(3),
                _ => {}
            }
            return actions;
        }

        if let Some(actions) = self.ux_mouse(mouse, hit) {
            return actions;
        }

        // The slash menu takes the wheel while the pointer is over it.
        if self.slash_menu_open()
            && rect_contains(hit.autocomplete_area, x, y)
            && matches!(mouse.kind, MouseEventKind::ScrollUp | MouseEventKind::ScrollDown)
        {
            let delta = if matches!(mouse.kind, MouseEventKind::ScrollUp) { -1 } else { 1 };
            let n = self.autocomplete.len() as isize;
            // The wheel scrolls without wrapping.
            let idx = (self.autocomplete_idx as isize + delta).clamp(0, n - 1);
            self.autocomplete_idx = idx as usize;
            self.sync_autocomplete_window();
            return actions;
        }

        match mouse.kind {
            MouseEventKind::ScrollUp => {
                self.selection = None;
                if self.settings.open {
                    self.settings_wheel(-1);
                } else if self.sidebar_visible(viewport.width) && rect_contains(hit.sidebar_area, x, y) {
                    self.sidebar_scroll = self.sidebar_scroll.saturating_sub(2);
                } else if rect_contains(hit.chat_area, x, y) || !rect_contains(hit.sidebar_area, x, y) {
                    self.auto_scroll = false;
                    self.chat_scroll = self.chat_scroll.saturating_sub(3);
                }
            }
            MouseEventKind::ScrollDown => {
                self.selection = None;
                if self.settings.open {
                    self.settings_wheel(1);
                } else if self.sidebar_visible(viewport.width) && rect_contains(hit.sidebar_area, x, y) {
                    self.sidebar_scroll = (self.sidebar_scroll + 2).min(hit.sidebar_max_scroll);
                } else if rect_contains(hit.chat_area, x, y) || !rect_contains(hit.sidebar_area, x, y) {
                    let max_scroll = hit.chat_total_lines.saturating_sub(hit.chat_visible_lines);
                    self.chat_scroll = self.chat_scroll.saturating_add(3).min(max_scroll);
                    if self.chat_scroll >= max_scroll {
                        self.auto_scroll = true;
                    }
                }
            }
            MouseEventKind::Down(MouseButton::Left) => {
                if self.wizard.is_some() {
                    self.selection = None;
                    for (rect, btn_idx) in &hit.wizard_buttons {
                        if rect_contains(*rect, x, y) {
                            return self.activate_wizard_button(*btn_idx);
                        }
                    }
                    return actions;
                }

                if self.settings.open {
                    self.selection = None;
                    if self.settings.has_subview() && self.settings.plugin_detail.is_none() {
                        return actions; // a dialog or editor owns the pointer
                    }
                    if self.settings.plugin_detail.is_some()
                        && let Some(back_rect) = hit.settings_back
                        && rect_contains(back_rect, x, y)
                    {
                        self.settings.plugin_detail = None;
                        self.settings.editing = None;
                        return actions;
                    }
                    for (rect, tab) in &hit.settings_tabs {
                        if rect_contains(*rect, x, y) {
                            return self.switch_settings_tab(*tab);
                        }
                    }
                    for (rect, sec_idx) in &hit.settings_sections {
                        if rect_contains(*rect, x, y) {
                            self.settings.section_index = *sec_idx;
                            self.jump_to_section(*sec_idx);
                            return actions;
                        }
                    }
                    for (rect, opt_idx) in &hit.settings_options {
                        if rect_contains(*rect, x, y) {
                            if let Some(detail) = &mut self.settings.plugin_detail {
                                detail.selected_row = *opt_idx;
                                return self.activate_plugin_detail_row(*opt_idx);
                            }
                            self.settings.selected_index = *opt_idx;
                            return self.activate_settings_row(*opt_idx);
                        }
                    }
                    return actions;
                }

                // Inline Approval buttons
                if let Some(approve_rect) = hit.approval_approve
                    && rect_contains(approve_rect, x, y)
                    && let Some(card) = self.pending_approval.take()
                {
                    self.selection = None;
                    actions.push(AppAction::ResolveApproval {
                        action_id: card.action_id,
                        h_act: card.h_act,
                        state_epoch: card.state_epoch,
                        approved: true,
                    });
                    return actions;
                }
                if let Some(deny_rect) = hit.approval_deny
                    && rect_contains(deny_rect, x, y)
                    && let Some(card) = self.pending_approval.take()
                {
                    self.selection = None;
                    actions.push(AppAction::ResolveApproval {
                        action_id: card.action_id,
                        h_act: card.h_act,
                        state_epoch: card.state_epoch,
                        approved: false,
                    });
                    return actions;
                }

                // Slash autocomplete click
                for (rect, idx) in &hit.autocomplete_rows {
                    if rect_contains(*rect, x, y) {
                        self.selection = None;
                        if let Some(item) = self.autocomplete.get(*idx) {
                            self.composer.text = format!("/{} ", item.name);
                            self.composer.cursor = self.composer.text.len();
                            self.autocomplete_idx = *idx;
                            self.filter_autocomplete();
                        }
                        return actions;
                    }
                }

                // Attachment card click-to-detach
                for (rect, att_id) in &hit.attachment_cards {
                    if rect_contains(*rect, x, y) {
                        self.selection = None;
                        if self.composer.remove_attachment(*att_id).is_some() {
                            self.filter_autocomplete();
                            self.set_toast(format!("Detached #{att_id}"), false);
                        }
                        return actions;
                    }
                }

                // Sidebar CHATS click
                if self.sidebar_visible(viewport.width) {
                    for (rect, chat_id) in &hit.sidebar_chats {
                        if rect_contains(*rect, x, y) {
                            self.selection = None;
                            self.chats.active_chat_id = chat_id.clone();
                            self.timelines.entry(chat_id.clone()).or_default();
                            self.auto_scroll = true;
                            self.chat_scroll = usize::MAX;
                            self.persist_chats();
                            return actions;
                        }
                    }
                    for (rect, plugin_id) in &hit.sidebar_plugins {
                        if rect_contains(*rect, x, y) {
                            self.selection = None;
                            self.settings.open_tab(SettingsTab::Plugins);
                            self.open_plugin_detail(plugin_id);
                            return actions;
                        }
                    }
                    for (rect, actor_id) in &hit.sidebar_actors {
                        if rect_contains(*rect, x, y) && self.lab_mode {
                            self.selection = None;
                            self.lab_session.current_actor = ActorId::new(actor_id.as_str());
                            self.set_toast(format!("Speaking as {actor_id}"), false);
                            return actions;
                        }
                    }
                }

                // Chat vertical scrollbar click/drag
                if let Some(sb_rect) = hit.chat_scrollbar
                    && rect_contains(sb_rect, x, y)
                {
                    self.selection = None;
                    self.dragging_chat_scrollbar = true;
                    self.scroll_chat_from_scrollbar_y(hit, y);
                    return actions;
                }

                // Chat viewport drag-select start
                if rect_contains(hit.chat_area, x, y) {
                    self.selection = Some(ChatSelection {
                        anchor: (x, y),
                        current: (x, y),
                        dragging: true,
                    });
                } else {
                    self.selection = None;
                }
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                if self.dragging_chat_scrollbar {
                    self.scroll_chat_from_scrollbar_y(hit, y);
                    return actions;
                }
                if let Some(sel) = &mut self.selection
                    && sel.dragging
                    && hit.chat_area.width > 0
                    && hit.chat_area.height > 0
                {
                    let clamped_x = x.clamp(
                        hit.chat_area.x,
                        hit.chat_area.x.saturating_add(hit.chat_area.width.saturating_sub(1)),
                    );
                    let clamped_y = y.clamp(
                        hit.chat_area.y,
                        hit.chat_area.y.saturating_add(hit.chat_area.height.saturating_sub(1)),
                    );
                    sel.current = (clamped_x, clamped_y);
                }
            }
            MouseEventKind::Up(MouseButton::Left) => {
                if self.dragging_chat_scrollbar {
                    self.dragging_chat_scrollbar = false;
                    return actions;
                }
                if let Some(mut sel) = self.selection.take()
                    && sel.dragging
                {
                    sel.dragging = false;
                    if sel.anchor == sel.current {
                        // A click without a drag selects the message under the pointer.
                        self.conv_click_message(hit, sel.current.0, sel.current.1);
                    }
                    if sel.anchor != sel.current {
                        let selected = extract_selected_text(hit, sel.anchor, sel.current);
                        if !selected.is_empty() {
                            if self.prefs.copy_on_select {
                                self.set_toast("Copied to clipboard", false);
                                actions.push(AppAction::CopyToClipboard(selected));
                            } else {
                                self.selection = Some(sel);
                            }
                        }
                    }
                }
            }
            _ => {}
        }

        actions
    }

    fn scroll_chat_from_scrollbar_y(&mut self, hit: &HitRegions, y: u16) {
        let Some(sb) = hit.chat_scrollbar else {
            return;
        };
        let max_scroll = hit.chat_total_lines.saturating_sub(hit.chat_visible_lines);
        if max_scroll == 0 || sb.height <= 1 {
            self.chat_scroll = max_scroll;
            self.auto_scroll = true;
            return;
        }
        let rel_y = y.saturating_sub(sb.y).min(sb.height - 1) as usize;
        let denom = (sb.height - 1) as usize;
        let new_scroll = (rel_y * max_scroll + denom / 2) / denom;
        self.chat_scroll = new_scroll.min(max_scroll);
        self.auto_scroll = self.chat_scroll >= max_scroll;
    }

    pub(crate) fn jump_to_section(&mut self, sec_idx: usize) {
        let sections = self.settings.tab.sections();
        let Some(target_sec) = sections.get(sec_idx) else {
            return;
        };
        let rows = self.current_settings_rows();
        if let Some(pos) = rows.iter().position(|r| r.section == *target_sec) {
            self.settings.selected_index = pos;
            self.settings.scroll_offset = pos;
        }
    }

    pub(crate) fn toggle_plugin_running(&mut self, pid: &str) -> Vec<AppAction> {
        let Some(p) = self.plugins.iter_mut().find(|p| p.id == pid) else {
            return Vec::new();
        };
        if !p.installed {
            self.begin_install(pid);
            return Vec::new();
        }
        let state = self
            .plugins
            .iter()
            .find(|p| p.id == pid)
            .map(|p| p.state.clone());
        match state {
            Some(PluginUiState::Running | PluginUiState::Starting) => {
                self.set_plugin_state(pid, PluginUiState::Stopping);
                vec![AppAction::StopPlugin(pid.to_owned())]
            }
            Some(_) => {
                self.set_plugin_state(pid, PluginUiState::Starting);
                vec![AppAction::StartPlugin(pid.to_owned())]
            }
            None => Vec::new(),
        }
    }

}

/// Builds the merged list of configured plugins and built-in catalog presets.
#[must_use]
pub fn build_plugin_entries(cfg: &GatewayConfig, paths: &ResolvedPaths) -> Vec<PluginUiEntry> {
    let cfg_dir = config_dir_from_path(&paths.config_path);
    let configured: Vec<PluginConfig> = cfg.effective_plugins();
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();

    for p in configured {
        seen.insert(p.id.clone());
        let (command, socket_path) = match &p.transport {
            monoagens_gateway::BodyTransportConfig::Stdio { command, .. } => {
                (Some(command.clone()), None)
            }
            monoagens_gateway::BodyTransportConfig::UnixSocket { path } => {
                (None, Some(path.display().to_string()))
            }
        };
        out.push(PluginUiEntry {
            id: p.id.clone(),
            kind: p.kind,
            enabled: p.enabled,
            installed: true,
            state: PluginUiState::Stopped,
            command,
            socket_path,
            config_path: cfg_dir.join("plugins").join(format!("{}.toml", p.id)),
            state_dir: paths.data_dir.join("plugins").join(&p.id),
            dependencies: p.dependencies,
            optional_dependencies: p.optional_dependencies,
        });
    }

    for preset in BUILTIN_PLUGIN_PRESETS {
        if seen.contains(preset.id) {
            continue;
        }
        let discovered = discover_builtin_binary(preset).map(|p| p.display().to_string());
        out.push(PluginUiEntry {
            id: preset.id.to_owned(),
            kind: preset.kind,
            enabled: false,
            installed: false,
            state: PluginUiState::Stopped,
            command: discovered,
            socket_path: None,
            config_path: cfg_dir.join("plugins").join(format!("{}.toml", preset.id)),
            state_dir: paths.data_dir.join("plugins").join(preset.id),
            dependencies: Vec::new(),
            optional_dependencies: preset
                .default_optional_deps
                .iter()
                .map(|s| (*s).to_owned())
                .collect(),
        });
    }

    out
}

impl TuiAppState {
    /// Returns `true` while a turn, model call, or capability action is in flight.
    #[must_use]
    pub const fn is_busy(&self) -> bool {
        self.thinking
    }

    /// Formats an elapsed [`std::time::Duration`] concisely (`0s`, `3s`, `45s`, `1m 05s`, `2m 12s`).
    #[must_use]
    pub fn format_elapsed(elapsed: std::time::Duration) -> String {
        let total_secs = elapsed.as_secs();
        if total_secs < 60 {
            format!("{total_secs}s")
        } else {
            let mins = total_secs / 60;
            let secs = total_secs % 60;
            format!("{mins}m {secs:02}s")
        }
    }
}

/// Builds a [`ThemePreset`] check helper used by appearance preview.
#[must_use]
pub const fn all_theme_presets() -> &'static [ThemePreset] {
    &ThemePreset::ALL
}

/// Normalizes `(anchor, current)` selection coordinates into `(start, end)` in `(row, col)` order.
#[must_use]
pub const fn normalize_selection(a: (u16, u16), b: (u16, u16)) -> ((u16, u16), (u16, u16)) {
    if a.1 < b.1 || (a.1 == b.1 && a.0 <= b.0) {
        (a, b)
    } else {
        (b, a)
    }
}

/// Extracts exact selected text across `hit.chat_viewport_lines` for screen coordinates `(anchor, current)`.
#[must_use]
pub fn extract_selected_text(hit: &HitRegions, anchor: (u16, u16), current: (u16, u16)) -> String {
    let area = hit.chat_area;
    if area.width == 0 || area.height == 0 || hit.chat_viewport_lines.is_empty() {
        return String::new();
    }
    let (start, end) = normalize_selection(anchor, current);
    let start_row = start.1.saturating_sub(area.y) as usize;
    let end_row = end
        .1
        .saturating_sub(area.y)
        .min(area.height.saturating_sub(1)) as usize;
    let start_col = start.0.saturating_sub(area.x) as usize;
    let end_col = end.0.saturating_sub(area.x) as usize;

    let mut out_lines = Vec::new();
    for row in start_row..=end_row {
        let Some(line) = hit.chat_viewport_lines.get(row) else {
            continue;
        };
        let c_from = if row == start_row { start_col } else { 0 };
        let c_to = if row == end_row { end_col } else { usize::MAX };
        let mut col = 0usize;
        let mut slice = String::new();
        for ch in line.chars() {
            let cw = ch.width().unwrap_or(0);
            let ch_end = if cw == 0 { col } else { col + cw - 1 };
            if (col >= c_from && col <= c_to) || (cw > 1 && ch_end >= c_from && col <= c_to) {
                slice.push(ch);
            }
            col += cw;
            if col > c_to {
                break;
            }
        }
        out_lines.push(slice.trim_end().to_owned());
    }

    while out_lines.last().is_some_and(String::is_empty) {
        out_lines.pop();
    }
    while out_lines.first().is_some_and(String::is_empty) {
        out_lines.remove(0);
    }
    out_lines.join("\n")
}
