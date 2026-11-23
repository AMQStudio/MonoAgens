//! Panic-safe terminal guard and the TUI event loop. The gateway is reached only through the
//! [`GatewayClient`] trait (`crate::client`): this TUI either hosts the gateway (it won the
//! data-dir lock; plugins are started by the gateway's supervisor) or attaches to the process
//! that holds the lock. Also: non-blocking LLM titler tasks and the interactive pairing handoff.

use std::collections::{BTreeSet, HashMap};
use std::io::{self, IsTerminal};
use std::path::{Path, PathBuf};
use std::process::{Command as StdCommand, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crossterm::event::{
    DisableBracketedPaste, DisableFocusChange, DisableMouseCapture, EnableBracketedPaste,
    EnableFocusChange, EnableMouseCapture, Event,
};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use monoagens_core::{
    ContextPlan, DecisionOutcome, DecisionReason, DecisionTrace, DoorDecision, Doors, EventDetail,
    EventKind, EventRecord, LogSeq,
};
use monoagens_gateway::{
    AdminServer, BodyTransportConfig, CAPABILITY_APPROVAL_PROMPT, Clock, ConfigError, CueRecorder, DiagnosticSink,
    DoorController, EmbedBackendKind, GatewayConfig, GatewayOptions, GatewayOwner, GatewayRuntime,
    GatewaySeams, LoadedGatewayEnv, LockError, OwnerMode, OwnerParams, PluginEnvContext,
    PluginSupervisor, ResolvedPaths, RestartPolicy, RunLog, RunLogLevel, SystemClock,
    ValveDoorController, acquire_data_dir_lock, build_provider_from_config_async,
    load_approval_secrets, load_or_init_gateway_env, load_plugin_env_secrets,
    read_data_dir_lock_pid, resolve_approval_mac_key, resolve_paths, resolve_paths_with,
};
use monoagens_protocol::{
    Action, ActionId, ActorId, ApprovalResolveParams, Audience, CURRENT_PROTOCOL_VERSION,
    CapabilityDescriptor, ChannelId, CorrelationId, OpaquePayload, OriginActor, Percept, PlaceId,
    Receipt, Taint, Time, Waist2Error,
};
use monoagens_reason::{LocalEmbedSidecar, ModelProvider, ProviderSecrets};
use monoagens_valve::{
    ApprovalLogRecord, Valve, ValveConfig, ValveMode, ValveSnapshot, sign_approval_resolution,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use tokio::sync::mpsc;

use crate::attachments::{
    PendingAttachment, PendingAttachmentKind, build_percept_payload_and_attachments,
    parse_image_dimensions,
};
use crate::client::{
    ClientKind, CommandReply, GatewayClient, InProcessClient, InProcessParts, RuntimeEvent,
    SocketClient, plugin_info_state,
};
use crate::clipboard::{ClipboardContent, copy_to_clipboard, read_clipboard_image_or_text};
use crate::config::{
    BUILTIN_PLUGIN_PRESETS, LocalChatStore, TuiPreferences, discover_builtin_binary,
    find_builtin_preset, list_secret_keys, merged_plugin_ids,
};
use crate::doctor::{DoctorInput, format_report, run_doctor};
use crate::effects::TuiEffects;
use crate::render::render_to_buffer;
use crate::settings::{PluginUiState, SecretsUiSummary, SetupWizardState, ValveRuleTogglesState};
use crate::state::{AppAction, ChatTimelineItem, HitRegions, PendingApprovalCard, TuiAppState};
use crate::titler::{
    TitleTrigger, extract_topic_ngrams, generate_chat_title, should_request_title,
};

/// Options controlling [`run_tui`] startup.
#[derive(Debug, Clone, Default)]
pub struct TuiLaunchOptions {
    pub config_path: Option<PathBuf>,
    pub data_dir: Option<PathBuf>,
    pub model_override: Option<PathBuf>,
    pub start_in_lab: bool,
    pub with_plugins: bool,
}

/// Panic-safe terminal guard ensuring raw mode, alternate screen, and mouse capture are always restored.
pub struct TerminalGuard {
    mouse_enabled: bool,
    active: bool,
}

impl TerminalGuard {
    /// Enters raw mode, alternate screen, and optionally mouse capture, installing a panic hook.
    pub fn enter(mouse_enabled: bool) -> Result<Self, String> {
        install_panic_hook();
        enable_raw_mode().map_err(|e| format!("failed to enable raw mode: {e}"))?;
        let mut stdout = io::stdout();
        if let Err(e) = execute!(stdout, EnterAlternateScreen, EnableBracketedPaste, EnableFocusChange) {
            let _ = disable_raw_mode();
            return Err(format!("failed to enter alternate screen: {e}"));
        }
        if mouse_enabled && let Err(e) = execute!(stdout, EnableMouseCapture) {
            let _ = execute!(stdout, DisableFocusChange, DisableBracketedPaste, LeaveAlternateScreen);
            let _ = disable_raw_mode();
            return Err(format!("failed to enable mouse capture: {e}"));
        }
        Ok(Self {
            mouse_enabled,
            active: true,
        })
    }

    /// Toggles mouse capture on the live terminal.
    pub fn set_mouse_capture(&mut self, enabled: bool) {
        if self.mouse_enabled == enabled || !self.active {
            return;
        }
        let mut stdout = io::stdout();
        if enabled {
            let _ = execute!(stdout, EnableMouseCapture);
        } else {
            let _ = execute!(stdout, DisableMouseCapture);
        }
        self.mouse_enabled = enabled;
    }

    /// Temporarily suspends raw mode, mouse capture, and alternate screen for an interactive child.
    pub fn suspend_for_subprocess(&mut self) {
        if !self.active {
            return;
        }
        restore_terminal_raw();
        self.active = false;
    }

    /// Restores raw mode, alternate screen, and mouse capture after a child subprocess exits.
    pub fn resume_after_subprocess(&mut self) -> Result<(), String> {
        if self.active {
            return Ok(());
        }
        enable_raw_mode().map_err(|e| format!("failed to re-enable raw mode: {e}"))?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableBracketedPaste, EnableFocusChange)
            .map_err(|e| format!("failed to re-enter alternate screen: {e}"))?;
        if self.mouse_enabled {
            execute!(stdout, EnableMouseCapture)
                .map_err(|e| format!("failed to re-enable mouse capture: {e}"))?;
        }
        self.active = true;
        Ok(())
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        if self.active {
            restore_terminal_raw();
            self.active = false;
        }
    }
}

fn restore_terminal_raw() {
    let mut stdout = io::stdout();
    let _ = execute!(
        stdout,
        DisableFocusChange,
        DisableBracketedPaste,
        DisableMouseCapture,
        LeaveAlternateScreen
    );
    let _ = disable_raw_mode();
}

fn install_panic_hook() {
    static INSTALLED: std::sync::Once = std::sync::Once::new();
    INSTALLED.call_once(|| {
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            restore_terminal_raw();
            prev(info);
        }));
    });
}

/// Launches the interactive MonoAgens TUI on the active terminal.
pub fn run_tui(options: TuiLaunchOptions) -> Result<u8, String> {
    run_tui_with_provider(options, None)
}

/// Offending config key of a configuration error, when it can be named.
#[must_use]
pub fn config_error_key(err: &ConfigError) -> Option<String> {
    match err {
        ConfigError::MissingField { key }
        | ConfigError::InvalidType { key, .. }
        | ConfigError::InvalidValue { key, .. } => Some(key.clone()),
        ConfigError::OutOfRange { key, .. } => Some((*key).to_owned()),
        _ => None,
    }
}

/// Launches the interactive MonoAgens TUI with an optional injected [`ModelProvider`].
pub fn run_tui_with_provider(
    options: TuiLaunchOptions,
    provider_override: Option<Arc<dyn ModelProvider>>,
) -> Result<u8, String> {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return Err("monoagens TUI requires an interactive TTY (stdin/stdout)".to_owned());
    }

    // A configuration error no longer exits: the TUI opens on it (banner, /settings, /retry).
    let (mut loaded, config_error) =
        match load_or_init_gateway_env(options.config_path.as_deref(), |k| std::env::var(k).ok()) {
            Ok(l) => (l, None),
            Err(e) => {
                let paths = resolve_paths_with(options.config_path.as_deref(), |k| {
                    std::env::var(k).ok()
                })
                .map_err(|_| e.to_string())?;
                (
                    LoadedGatewayEnv {
                        config: GatewayConfig::default(),
                        secrets: ProviderSecrets::default(),
                        paths,
                        created_default: false,
                    },
                    Some(e),
                )
            }
        };

    if let Some(custom_data) = &options.data_dir {
        loaded.paths.data_dir = custom_data.clone();
    }
    if let Some(model_path) = &options.model_override {
        loaded.config.soul.model_path = Some(model_path.clone());
    }

    let prefs_path = TuiPreferences::path_for_config(&loaded.paths.config_path);
    let prefs = TuiPreferences::load_from_path(&prefs_path)?;

    let chats_path = LocalChatStore::path_for_data_dir(&loaded.paths.data_dir);
    let chats = LocalChatStore::load_from_path(&chats_path)?;

    let (top_keys, env_keys) = list_secret_keys(&loaded.paths.secrets_path).unwrap_or_default();
    let secrets_summary = SecretsUiSummary {
        model_api_key_set: loaded.secrets.model_api_key.is_some(),
        model_endpoint: loaded.secrets.model_endpoint.clone(),
        top_level_keys: top_keys,
        env_keys,
    };

    let owner_actor = owner_actor_from_cfg(Some(&loaded.config));
    let needs_wizard = provider_override.is_none()
        && config_error.is_none()
        && loaded.ensure_startup_ready().is_err();

    let mut state = TuiAppState::new(
        loaded.paths.clone(),
        loaded.config.clone(),
        prefs,
        chats,
        secrets_summary,
        owner_actor.clone(),
    );
    if let Some(err) = &config_error {
        state.set_config_error(err.to_string(), config_error_key(err));
    }
    if options.start_in_lab {
        state.lab_mode = true;
    }
    if needs_wizard {
        let plugin_ids = merged_plugin_ids(&loaded.config.effective_plugins());
        state.wizard = Some(SetupWizardState::new(
            &loaded.config,
            loaded.secrets.model_endpoint.as_deref(),
            plugin_ids,
        ));
    }

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("failed to start tokio runtime: {e}"))?;

    let mut guard = TerminalGuard::enter(state.prefs.mouse_support)?;
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal =
        Terminal::new(backend).map_err(|e| format!("failed to initialize terminal: {e}"))?;

    let exit_res = rt.block_on(async {
        run_tui_event_loop(
            &mut terminal,
            &mut guard,
            &mut state,
            loaded,
            options,
            provider_override,
        )
        .await
    });

    drop(terminal);
    drop(guard);
    exit_res
}

/// How much of the display the event loop must repaint after an action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Redraw {
    None,
    /// Clear and repaint.
    Clear,
    /// Clear, repaint and drop pending input (after the interactive `pair` child used the tty).
    ClearAndDrain,
}

/// Suspends the process like a shell `Ctrl+Z`: leave the alternate screen, stop, restore on resume.
fn suspend_process(guard: &mut TerminalGuard) {
    guard.suspend_for_subprocess();
    #[cfg(unix)]
    {
        use rustix::process::{Signal, getpid, kill_process};
        // Stops here until the shell sends SIGCONT, then execution continues below.
        let _ = kill_process(getpid(), Signal::TSTP);
    }
    let _ = guard.resume_after_subprocess();
}

/// Maps log events and actions back to the TUI turn that caused them, using the correlation id
/// the TUI puts on every percept (`tui-turn-<n>`) and the `caused_by` chain.
#[derive(Debug, Default)]
pub(crate) struct TurnTracker {
    events: HashMap<u64, u64>,
    actions: HashMap<String, u64>,
}

const TURN_CORRELATION_PREFIX: &str = "tui-turn-";

impl TurnTracker {
    fn observe(&mut self, ev: &EventRecord) {
        let turn = match &ev.detail {
            Some(EventDetail::PerceptMeta {
                correlation_id: Some(c),
                ..
            }) => c
                .as_str()
                .strip_prefix(TURN_CORRELATION_PREFIX)
                .and_then(|n| n.parse::<u64>().ok()),
            _ => ev
                .caused_by
                .iter()
                .find_map(|c| self.events.get(&c.0).copied()),
        };
        let Some(turn) = turn else {
            return;
        };
        self.events.insert(ev.id.0, turn);
        if let Some(EventDetail::Action { action_id, .. }) = &ev.detail {
            self.actions.insert(action_id.as_str().to_owned(), turn);
        }
    }

    /// Turn that caused `action_id`, when the log already shows it.
    pub(crate) fn turn_of_action(&self, action_id: &str) -> Option<u64> {
        self.actions.get(action_id).copied()
    }
}

/// Mutable bookkeeping of the event loop that follows the gateway's event log.
#[derive(Default)]
struct LogFollower {
    seen_len: usize,
    resolved_approvals: BTreeSet<ActionId>,
    tracker: TurnTracker,
}

/// Everything the event loop needs to boot (or re-boot) the gateway connection.
struct BootInputs<'a> {
    loaded: &'a mut LoadedGatewayEnv,
    options: &'a TuiLaunchOptions,
    provider_override: &'a Option<Arc<dyn ModelProvider>>,
    tx: &'a mpsc::UnboundedSender<RuntimeEvent>,
}

/// Boots or attaches and syncs the initial state (commands, plugin states, link, notices).
async fn connect_gateway(
    state: &mut TuiAppState,
    inputs: &mut BootInputs<'_>,
    follower: &mut LogFollower,
) -> Option<Arc<dyn GatewayClient>> {
    match boot_gateway(inputs.loaded, inputs.options, inputs.provider_override.clone(), inputs.tx.clone()).await {
        Ok(client) => {
            state.gateway_link = match client.kind() {
                ClientKind::Hosting => "hosting",
                ClientKind::Attached => "attached",
            }
            .to_owned();
            if let Ok(cmds) = client.commands().await {
                state.gateway_commands = cmds;
            }
            if let Ok(plugins) = client.plugins_list().await {
                for p in plugins {
                    state.set_plugin_state(&p.id, plugin_info_state(&p));
                }
            }
            follower.seen_len = client.events_since(usize::MAX).await.map_or(0, |p| p.total);
            state.chat_disabled_reason = client.chat_unavailable_reason();
            if client.kind() == ClientKind::Attached {
                let who = read_data_dir_lock_pid(&state.paths.data_dir)
                    .map_or_else(|| "another process".to_owned(), |p| format!("pid {p}"));
                state.push_timeline_item(
                    &state.chats.active_chat_id.clone(),
                    ChatTimelineItem::SystemNotice {
                        text: format!("Attached to the gateway hosted by {who}. Closing this TUI leaves it running."),
                        is_error: false,
                    },
                );
            }
            Some(client)
        }
        Err(err) => {
            state.set_toast(err.clone(), true);
            state.push_timeline_item(
                &state.chats.active_chat_id.clone(),
                ChatTimelineItem::SystemNotice {
                    text: format!("Gateway boot notice: {err}"),
                    is_error: true,
                },
            );
            None
        }
    }
}

async fn run_tui_event_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    guard: &mut TerminalGuard,
    state: &mut TuiAppState,
    mut loaded: LoadedGatewayEnv,
    options: TuiLaunchOptions,
    provider_override: Option<Arc<dyn ModelProvider>>,
) -> Result<u8, String> {
    let (tx, mut rx) = mpsc::unbounded_channel::<RuntimeEvent>();
    let mut effects = TuiEffects::new();
    let mut hit = HitRegions::default();
    let mut client: Option<Arc<dyn GatewayClient>> = None;
    let mut follower = LogFollower::default();

    if state.wizard.is_none() && state.config_error.is_none() {
        let mut inputs = BootInputs {
            loaded: &mut loaded,
            options: &options,
            provider_override: &provider_override,
            tx: &tx,
        };
        client = connect_gateway(state, &mut inputs, &mut follower).await;
    }

    let (key_tx, mut key_rx) = mpsc::unbounded_channel::<Event>();
    std::thread::spawn(move || {
        loop {
            match crossterm::event::poll(Duration::from_millis(50)) {
                Ok(true) => {
                    if let Ok(ev) = crossterm::event::read()
                        && key_tx.send(ev).is_err()
                    {
                        break;
                    }
                }
                Ok(false) => {}
                Err(_) => break,
            }
        }
    });

    let mut tick_interval = tokio::time::interval(Duration::from_millis(80));
    tick_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut last_draw = Instant::now();
    let mut active_turn_chat_id: Option<String> = None;
    let mut needs_redraw = true;

    loop {
        if needs_redraw || effects.is_running() || state.is_busy() {
            let now = Instant::now();
            let elapsed_ms = now.duration_since(last_draw).as_millis().clamp(1, 250) as u32;
            last_draw = now;
            if state.is_busy() {
                state.thinking_elapsed_ms = state.turn.elapsed_ms();
            }
            terminal
                .draw(|frame| {
                    render_to_buffer(
                        frame.area(),
                        frame.buffer_mut(),
                        state,
                        &mut effects,
                        &mut hit,
                        elapsed_ms,
                    );
                })
                .map_err(|e| format!("terminal draw failed: {e}"))?;
            needs_redraw = false;
        }

        if state.should_quit {
            break;
        }

        tokio::select! {
            biased;
            maybe_ev = key_rx.recv() => {
                let Some(ev) = maybe_ev else { break; };
                let viewport = terminal.size().map(|s| ratatui::layout::Rect::new(0, 0, s.width, s.height)).unwrap_or_default();
                let outcome = state.handle_terminal_event(ev, &hit, viewport);
                needs_redraw = outcome.redraw;
                let mut redraw = if outcome.clear { Redraw::Clear } else { Redraw::None };
                for action in outcome.actions {
                    let r = handle_app_action(
                        action,
                        state,
                        guard,
                        &mut loaded,
                        &options,
                        &provider_override,
                        &mut client,
                        &mut follower,
                        &mut active_turn_chat_id,
                        &tx,
                        &mut effects,
                    )
                    .await;
                    redraw = redraw.max(r);
                }
                if redraw >= Redraw::Clear {
                    if redraw == Redraw::ClearAndDrain {
                        // The interactive child consumed the terminal; drop whatever it left behind.
                        while key_rx.try_recv().is_ok() {}
                        while crossterm::event::poll(Duration::ZERO).unwrap_or(false) {
                            let _ = crossterm::event::read();
                        }
                    }
                    let _ = terminal.clear();
                    needs_redraw = true;
                }
            }
            maybe_rt = rx.recv() => {
                if let Some(rt_ev) = maybe_rt {
                    // A reply is routed by the turn that caused its action: bring the log up to
                    // date first so the action is known.
                    if matches!(&rt_ev, RuntimeEvent::AssistantReply { action_id: Some(_), .. })
                        && let Some(c) = &client
                    {
                        poll_new_gateway_events(c, state, &mut follower, &mut active_turn_chat_id).await;
                    }
                    handle_runtime_event(
                        rt_ev,
                        state,
                        &mut active_turn_chat_id,
                        &mut effects,
                        &follower.tracker,
                        &mut client,
                    );
                    needs_redraw = true;
                }
            }
            _ = tick_interval.tick() => {
                if let Some(toast) = &mut state.toast {
                    toast.ticks_remaining = toast.ticks_remaining.saturating_sub(1);
                    if toast.ticks_remaining == 0 {
                        state.toast = None;
                        needs_redraw = true;
                    }
                }
                if state.is_busy() {
                    effects.advance_tick();
                    let turn_seq = state.turn.seq();
                    if state.tick_turn() {
                        if let Some(c) = &client {
                            c.log(
                                RunLogLevel::Info,
                                "turn",
                                &format!(
                                    "turn.no_reply: turn {turn_seq} finished without a reply (effects_spawned={})",
                                    state.turn_effects
                                ),
                            );
                        }
                        active_turn_chat_id = None;
                    }
                    if let Some(c) = &client {
                        poll_new_gateway_events(c, state, &mut follower, &mut active_turn_chat_id).await;
                    }
                    needs_redraw = true;
                } else if effects.is_running() {
                    needs_redraw = true;
                }
            }
        }

        // Submit queued messages (typed while busy) in order once the running turn ended.
        while let Some(action) = state.next_queued_action() {
            needs_redraw = true;
            handle_app_action(
                action,
                state,
                guard,
                &mut loaded,
                &options,
                &provider_override,
                &mut client,
                &mut follower,
                &mut active_turn_chat_id,
                &tx,
                &mut effects,
            )
            .await;
        }
    }

    if let Some(c) = client {
        c.shutdown().await;
    }
    Ok(0)
}

#[allow(clippy::too_many_arguments)]
async fn handle_app_action(
    action: AppAction,
    state: &mut TuiAppState,
    guard: &mut TerminalGuard,
    loaded: &mut LoadedGatewayEnv,
    options: &TuiLaunchOptions,
    provider_override: &Option<Arc<dyn ModelProvider>>,
    client: &mut Option<Arc<dyn GatewayClient>>,
    follower: &mut LogFollower,
    active_turn_chat_id: &mut Option<String>,
    tx: &mpsc::UnboundedSender<RuntimeEvent>,
    effects: &mut TuiEffects,
) -> Redraw {
    let mut needs_full_redraw = Redraw::None;
    match action {
        AppAction::ToggleMouseCapture(enabled) => {
            guard.set_mouse_capture(enabled);
        }
        AppAction::CompleteSetupWizard | AppAction::RetryBoot => {
            if client.is_none() && reload_config(state, loaded, options) {
                let mut inputs = BootInputs {
                    loaded,
                    options,
                    provider_override,
                    tx,
                };
                *client = connect_gateway(state, &mut inputs, follower).await;
                if client.is_some() {
                    state.set_toast("Gateway booted", false);
                }
            }
        }
        AppAction::RunDoctor => {
            let input = DoctorInput {
                paths: state.paths.clone(),
                link: if state.gateway_link.is_empty() {
                    "none".to_owned()
                } else {
                    state.gateway_link.clone()
                },
            };
            let tx_d = tx.clone();
            tokio::task::spawn_blocking(move || {
                let _ = tx_d.send(RuntimeEvent::Doctor(format_report(&run_doctor(&input))));
            });
            state.set_toast("Running /doctor checks…", false);
        }
        AppAction::SendUserMessage {
            chat_id,
            place_id: _,
            text,
            attachments,
        } => {
            *active_turn_chat_id = Some(chat_id.clone());
            // Check non-blocking background auto-titler
            let trigger = {
                let chat = state.chats.active_chat_mut();
                let trig = should_request_title(chat, &text, state.prefs.auto_chat_title);
                chat.turns_since_retitle = chat.turns_since_retitle.saturating_add(1);
                let kw = extract_topic_ngrams(&text);
                if chat.topic_ngrams.is_empty() {
                    chat.topic_ngrams = kw;
                } else {
                    for k in kw {
                        if !chat.topic_ngrams.contains(&k) {
                            chat.topic_ngrams.push(k);
                        }
                    }
                }
                trig
            };
            state.persist_chats();

            if trigger != TitleTrigger::None
                && let Some(c) = client.as_ref()
                && let Some(tp) = c.titler_provider()
            {
                let model = state.model_name.clone();
                let cur_title = if trigger == TitleTrigger::TopicShift {
                    Some(state.chats.active_chat().title.clone())
                } else {
                    None
                };
                let recent = vec![text.clone()];
                let new_kw = extract_topic_ngrams(&text);
                let tx_title = tx.clone();
                let cid = chat_id.clone();
                tokio::task::spawn_blocking(move || {
                    if let Some(title) =
                        generate_chat_title(&tp, &model, cur_title.as_deref(), &recent)
                    {
                        let _ = tx_title.send(RuntimeEvent::ChatTitleReady {
                            chat_id: cid,
                            title,
                            keywords: new_kw,
                            is_topic_shift: trigger == TitleTrigger::TopicShift,
                        });
                    }
                });
            }

            let Some(c) = client.clone() else {
                state.finish_busy();
                *active_turn_chat_id = None;
                state.push_timeline_item(
                    &chat_id,
                    ChatTimelineItem::SystemNotice {
                        text: "Provider not configured yet. Open /settings -> Providers to set model_api_key."
                            .to_owned(),
                        is_error: true,
                    },
                );
                return needs_full_redraw;
            };

            let known: Vec<ActorId> = state
                .lab_session
                .known_actors
                .iter()
                .filter(|a| a.as_str() != "soul")
                .map(ActorId::new)
                .collect();
            c.register_actors(&known);
            let turn = state.turn.seq();
            let now = c.now();
            let mut percept = build_tui_percept(state, &c.owner_actor(), &text, &attachments, now);
            percept.correlation_id = Some(CorrelationId::new(format!("{TURN_CORRELATION_PREFIX}{turn}")));
            let tx_msg = tx.clone();
            tokio::spawn(async move {
                match c.submit_turn(percept).await {
                    Err(err) => {
                        let _ = tx_msg.send(RuntimeEvent::TurnError {
                            turn,
                            error: err.message,
                        });
                    }
                    Ok(effects_spawned) => {
                        let _ = tx_msg.send(RuntimeEvent::TurnFinished {
                            turn,
                            effects_spawned,
                        });
                    }
                }
            });
        }
        AppAction::ResolveApproval {
            action_id,
            h_act,
            state_epoch,
            approved,
        } => {
            if let Some(c) = client.as_ref() {
                let now = c.now();
                let sig = sign_approval_resolution(
                    &c.approval_mac_key(),
                    &action_id,
                    &h_act,
                    state_epoch,
                    approved,
                    now,
                );
                let params = ApprovalResolveParams {
                    headers: monoagens_protocol::Waist2Headers::new(format!(
                        "tui-approval-{}",
                        action_id.as_str()
                    )),
                    action_id,
                    h_act,
                    state_epoch,
                    approved,
                    resolver: c.owner_actor(),
                    resolved_at_ms: now,
                    signature: Some(sig),
                    out_of_band_channel: true,
                };
                match c.resolve_approval(params).await {
                    Ok(()) => {
                        state.set_toast(
                            if approved {
                                "Approval granted"
                            } else {
                                "Action denied"
                            },
                            false,
                        );
                    }
                    Err(e) => {
                        state.set_toast(format!("Approval failed: {e}"), true);
                    }
                }
            }
        }
        AppAction::StartPlugin(pid) => {
            if let Some(c) = client.clone() {
                let tx_p = tx.clone();
                tokio::spawn(async move {
                    let mut res = c.plugins_start(pid.clone()).await;
                    if res.is_err() {
                        // A plugin installed in the UI is not known to the gateway yet: let the
                        // owner reload the config file (starts enabled new plugins), then retry.
                        let _ = c.config_apply(serde_json::json!({})).await;
                        res = c.plugins_start(pid.clone()).await;
                    }
                    if let Err(e) = res {
                        let _ = tx_p.send(RuntimeEvent::PluginState {
                            id: pid,
                            state: PluginUiState::Failed(e.message),
                        });
                    }
                });
            } else {
                state.set_plugin_state(&pid, PluginUiState::Stopped);
            }
        }
        AppAction::StopPlugin(pid) => {
            if let Some(c) = client.clone() {
                let tx_p = tx.clone();
                let id = pid.clone();
                tokio::spawn(async move {
                    if let Err(e) = c.plugins_stop(id.clone()).await {
                        let _ = tx_p.send(RuntimeEvent::PluginState {
                            id,
                            state: PluginUiState::Failed(e.message),
                        });
                    }
                });
            }
            state.set_plugin_state(&pid, PluginUiState::Stopped);
            state.set_toast(format!("Stopped plugin '{pid}'"), false);
        }
        AppAction::PairPlugin(pid) => {
            let res = run_interactive_pair_subprocess(guard, &loaded.paths, &state.gateway_cfg, &pid);
            match res {
                Ok(0) => state.set_toast(format!("Paired '{pid}' successfully"), false),
                Ok(130) => state.set_toast(format!("Pairing '{pid}' cancelled"), false),
                Ok(124) => state.set_toast(
                    format!("Pairing '{pid}' timed out — try again or use phone pairing"),
                    true,
                ),
                Ok(code) => state.set_toast(format!("Pairing '{pid}' exited with code {code}"), true),
                Err(e) => state.set_toast(e, true),
            }
            let palette = crate::theme::ThemePalette::for_preset(state.prefs.theme);
            effects.trigger_modal_dissolve(state.prefs.motion, &palette);
            needs_full_redraw = Redraw::ClearAndDrain;
        }
        AppAction::SubmitGatewayCommand(raw) => {
            if let Some(c) = client.as_ref() {
                let actor = state.lab_session.current_actor.as_str().to_owned();
                let place = state.chats.active_chat().place_id.clone();
                let reply = c.run_command(raw.clone(), Some(actor), Some(place)).await;
                let active_id = state.chats.active_chat_id.clone();
                let (text, is_error) = match reply {
                    Ok(CommandReply::Handled(reply_text)) => (reply_text, false),
                    Ok(CommandReply::Denied(reason)) => (reason, true),
                    Ok(CommandReply::Forward { body_id, command, args }) => {
                        (format!("Forwarded /{command} {args} to {body_id}"), false)
                    }
                    Ok(CommandReply::Unknown) => {
                        (format!("Unknown command '{raw}'. Type /help for commands."), true)
                    }
                    Err(e) => (format!("Command failed: {e}"), true),
                };
                state.push_timeline_item(
                    &active_id,
                    ChatTimelineItem::SystemNotice { text, is_error },
                );
            }
        }
        AppAction::CopyToClipboard(text) => {
            if let Err(err) = copy_to_clipboard(&text) {
                state.set_toast(format!("Clipboard copy failed: {err}"), true);
            }
        }
        AppAction::ResetDisplay => {
            needs_full_redraw = Redraw::Clear;
        }
        AppAction::Suspend => {
            suspend_process(guard);
            needs_full_redraw = Redraw::Clear;
        }
        AppAction::PasteFromClipboard => match read_clipboard_image_or_text() {
            Some(ClipboardContent::ImagePng(bytes)) => {
                let dims = parse_image_dimensions(&bytes);
                let byte_len = bytes.len();
                let att = PendingAttachment {
                    id: 0,
                    kind: PendingAttachmentKind::Image {
                        width: dims.map(|d| d.0),
                        height: dims.map(|d| d.1),
                        mime: "image/png".to_owned(),
                    },
                    label: "clipboard.png".to_owned(),
                    source_path: None,
                    bytes: Some(bytes),
                    byte_len,
                };
                let id = state.composer.add_attachment(att);
                state.filter_autocomplete();
                state.set_toast(format!("Attached clipboard image #{id}"), false);
            }
            Some(ClipboardContent::Text(text)) => {
                let _ = state.handle_paste(&text);
            }
            None => {
                state.set_toast("Clipboard is empty or unavailable", false);
            }
        },
    }
    needs_full_redraw
}

/// Reloads the config for a wizard completion or `/retry`. Returns `true` when the gateway
/// should be booted now, `false` when the config is still not usable (the banner is updated).
fn reload_config(
    state: &mut TuiAppState,
    loaded: &mut LoadedGatewayEnv,
    options: &TuiLaunchOptions,
) -> bool {
    match load_or_init_gateway_env(options.config_path.as_deref(), |k| std::env::var(k).ok()) {
        Ok(reloaded) => {
            *loaded = reloaded;
            if let Some(custom_data) = &options.data_dir {
                loaded.paths.data_dir = custom_data.clone();
            }
            state.gateway_cfg = loaded.config.clone();
            state.model_name = loaded.config.model.model.clone();
            state.valve_mode = loaded.config.valve.mode.clone();
            state.refresh_secrets_summary();
            state.config_error = None;
            loaded.ensure_startup_ready().is_ok()
        }
        Err(e) => {
            state.set_config_error(e.to_string(), config_error_key(&e));
            state.set_toast("Configuration is still invalid", true);
            false
        }
    }
}

fn handle_runtime_event(
    rt_ev: RuntimeEvent,
    state: &mut TuiAppState,
    active_turn_chat_id: &mut Option<String>,
    effects: &mut TuiEffects,
    tracker: &TurnTracker,
    client: &mut Option<Arc<dyn GatewayClient>>,
) {
    let palette = crate::theme::ThemePalette::for_preset(state.prefs.theme);
    match rt_ev {
        RuntimeEvent::AssistantReply {
            chat_id,
            text,
            duration_ms,
            action_id,
        } => {
            let turn = action_id.as_deref().and_then(|a| tracker.turn_of_action(a));
            let disposition = state.on_assistant_reply(turn, chat_id, text, duration_ms);
            if disposition == crate::state::ReplyDisposition::CurrentTurn {
                *active_turn_chat_id = None;
            }
            effects.trigger_message_fade_in(state.prefs.motion, &palette);
        }
        RuntimeEvent::ApprovalPrompt(card) => {
            state.pending_approval = Some(card);
        }
        RuntimeEvent::TurnError { turn, error } => {
            let cid = match state.on_turn_error(turn) {
                Some(chat) => {
                    *active_turn_chat_id = None;
                    chat
                }
                None => state.chats.active_chat_id.clone(),
            };
            state.push_timeline_item(
                &cid,
                ChatTimelineItem::SystemNotice {
                    text: format!("Turn error: {error}"),
                    is_error: true,
                },
            );
        }
        RuntimeEvent::TurnFinished {
            turn,
            effects_spawned,
        } => {
            // Replies arrive asynchronously via chat.speak; arm the grace window, `tick_turn`
            // ends a silent turn once it passes.
            if state.turn.is_current(turn) {
                state.turn_effects = effects_spawned;
            }
            state.on_turn_finished(turn);
        }
        RuntimeEvent::PluginState { id, state: pstate } => {
            state.set_plugin_state(&id, pstate);
        }
        RuntimeEvent::ChatTitleReady {
            chat_id,
            title,
            keywords,
            is_topic_shift,
        } => {
            if let Some(chat) = state.chats.chats.iter_mut().find(|c| c.id == chat_id)
                && !chat.title_locked
            {
                chat.title = title;
                chat.turns_since_retitle = 0;
                if is_topic_shift {
                    chat.topic_ngrams = keywords;
                }
                state.persist_chats();
            }
        }
        RuntimeEvent::Log(entry) => state.on_log_entry(entry),
        RuntimeEvent::Cue(cue) => {
            state.thinking_phase = match cue.kind {
                monoagens_gateway::LifecycleCueKind::WorkingCue => "Working…".to_owned(),
                monoagens_gateway::LifecycleCueKind::ProgressNote => "Still working…".to_owned(),
            };
        }
        RuntimeEvent::Doctor(report) => {
            let active = state.chats.active_chat_id.clone();
            state.push_timeline_item(
                &active,
                ChatTimelineItem::SystemNotice {
                    text: report,
                    is_error: false,
                },
            );
        }
        RuntimeEvent::OwnerGone => {
            if client.take().is_some() {
                state.finish_busy();
                *active_turn_chat_id = None;
                state.chat_disabled_reason =
                    Some("the gateway owner went away; restart the TUI to host or re-attach".to_owned());
                let active = state.chats.active_chat_id.clone();
                state.push_timeline_item(
                    &active,
                    ChatTimelineItem::SystemNotice {
                        text: "The gateway owner went away (owner went away). Restart the TUI to host a gateway or attach again."
                            .to_owned(),
                        is_error: true,
                    },
                );
                state.set_toast("Gateway owner went away", true);
            }
        }
    }
}

async fn poll_new_gateway_events(
    client: &Arc<dyn GatewayClient>,
    state: &mut TuiAppState,
    follower: &mut LogFollower,
    active_turn_chat_id: &mut Option<String>,
) {
    let Ok(page) = client.events_since(follower.seen_len).await else {
        return;
    };
    state.daily_tokens_used = page.budget_used;
    state.daily_token_budget = page.budget_limit.max(1);
    if page.events.is_empty() {
        follower.seen_len = page.total;
        return;
    }
    let active_id = active_turn_chat_id
        .clone()
        .unwrap_or_else(|| state.chats.active_chat_id.clone());
    for ev in &page.events {
        follower.tracker.observe(ev);
        process_event(ev, state, &mut follower.resolved_approvals, active_turn_chat_id, &active_id);
    }
    follower.seen_len = page.total;
}

fn process_event(
    ev: &EventRecord,
    state: &mut TuiAppState,
    resolved_approvals: &mut BTreeSet<ActionId>,
    active_turn_chat_id: &mut Option<String>,
    active_id: &str,
) {
    let active_id = active_id.to_owned();
    if let Some(detail) = &ev.detail {
        match detail {
            EventDetail::Decision {
                outcome, trace, ..
            } => {
                if let Some(tr) = trace {
                    let line = format_decision_trace(tr);
                    state.last_decision_trace = Some(line.clone());
                    state.push_timeline_item(&active_id, ChatTimelineItem::BrainTrace { summary: line });
                }
                if *outcome == DecisionOutcome::Silent || *outcome == DecisionOutcome::DoorBlocked {
                    state.finish_busy();
                    *active_turn_chat_id = None;
                }
            }
            EventDetail::Action {
                capability_id, ..
            } if capability_id.as_str() != "chat.speak"
                && capability_id.as_str() != CAPABILITY_APPROVAL_PROMPT =>
            {
                state.thinking_phase = format!("Running {}…", capability_id.as_str());
                state.push_timeline_item(
                    &active_id,
                    ChatTimelineItem::CapabilityStep {
                        capability: capability_id.as_str().to_owned(),
                        status: "running".to_owned(),
                        detail: String::new(),
                    },
                );
            }
            EventDetail::Receipt {
                capability_id,
                outcome,
                ..
            } if capability_id.as_str() != "chat.speak"
                && capability_id.as_str() != CAPABILITY_APPROVAL_PROMPT =>
            {
                state.push_timeline_item(
                    &active_id,
                    ChatTimelineItem::CapabilityStep {
                        capability: capability_id.as_str().to_owned(),
                        status: format!("{outcome:?}").to_lowercase(),
                        detail: String::new(),
                    },
                );
            }
            EventDetail::Failure { component, detail, .. } => {
                state.finish_busy();
                *active_turn_chat_id = None;
                state.push_timeline_item(
                    &active_id,
                    ChatTimelineItem::SystemNotice {
                        text: format!("[{component}] {detail}"),
                        is_error: true,
                    },
                );
            }
            _ => {}
        }
    }
    if ev.kind == EventKind::DoorVerdict
        && let Ok(rec) = serde_json::from_slice::<ApprovalLogRecord>(ev.payload.as_bytes())
    {
        match rec {
            ApprovalLogRecord::Held { pending, .. } => {
                if !resolved_approvals.contains(&pending.action_id) {
                    state.pending_approval = Some(PendingApprovalCard {
                        action_id: pending.action_id,
                        h_act: pending.h_act,
                        state_epoch: pending.state_epoch,
                        capability_id: pending.capability_id.as_str().to_owned(),
                        summary: pending.reason.unwrap_or_else(|| {
                            format!(
                                "Place(s): {}",
                                pending
                                    .places
                                    .iter()
                                    .map(PlaceId::as_str)
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            )
                        }),
                        selected_approve: true,
                    });
                    state.finish_busy();
                }
            }
            ApprovalLogRecord::Resolved { action_id, .. } => {
                resolved_approvals.insert(action_id.clone());
                if state
                    .pending_approval
                    .as_ref()
                    .is_some_and(|c| c.action_id == action_id)
                {
                    state.pending_approval = None;
                }
            }
            ApprovalLogRecord::GrantConsumed { .. } => {}
        }
    }
}

fn format_decision_trace(trace: &DecisionTrace) -> String {
    let outcome = match trace.outcome {
        DecisionOutcome::Go => "Go",
        DecisionOutcome::Silent => "Silent",
        DecisionOutcome::Note => "Note",
        DecisionOutcome::DoorBlocked => "DoorBlocked",
        DecisionOutcome::Processed => "Processed",
    };
    let tier = trace.neural.compute_tier.map_or("—", |t| match t {
        monoagens_core::ComputeTier::Fast => "Fast",
        monoagens_core::ComputeTier::Standard => "Standard",
        monoagens_core::ComputeTier::Deep => "Deep",
    });
    let mut s = format!(
        "[brain: {outcome} | reason={:?} | tier={tier} | p_go={:.2}",
        trace.reason, trace.neural.gate_probs[3]
    );
    if trace.reason == DecisionReason::WaitDeferral && trace.neural.wait_delta_ms > 0.0 {
        s.push_str(&format!(" | wait={:.0}ms", trace.neural.wait_delta_ms));
    }
    s.push(']');
    s
}

/// Hosts the gateway when this process wins the data-dir lock, otherwise attaches to the holder.
async fn boot_gateway(
    loaded: &LoadedGatewayEnv,
    options: &TuiLaunchOptions,
    provider_override: Option<Arc<dyn ModelProvider>>,
    tx: mpsc::UnboundedSender<RuntimeEvent>,
) -> Result<Arc<dyn GatewayClient>, String> {
    let owner_actor = owner_actor_from_cfg(Some(&loaded.config));
    let approval_secrets =
        load_approval_secrets(Some(&loaded.paths.secrets_path), |k| std::env::var(k).ok())
            .map_err(|e| e.to_string())?;
    let mac_key = resolve_approval_mac_key(&approval_secrets);

    let store_dir = options
        .data_dir
        .clone()
        .or_else(|| loaded.config.data_dir.clone())
        .unwrap_or_else(|| loaded.paths.data_dir.clone());

    match acquire_data_dir_lock(&store_dir) {
        Ok(lock) => {
            let client = host_gateway(
                loaded,
                options,
                provider_override,
                tx,
                lock,
                store_dir,
                owner_actor,
                mac_key,
            )
            .await?;
            Ok(Arc::new(client))
        }
        Err(LockError::Held(held)) => {
            let who = held
                .pid
                .map_or_else(|| "another process".to_owned(), |p| format!("pid {p}"));
            let client = SocketClient::connect(&store_dir, owner_actor, mac_key, &loaded.secrets, tx)
                .await
                .map_err(|e| {
                    format!(
                        "data dir '{}' is hosted by {who} but attaching failed: {e}",
                        store_dir.display()
                    )
                })?;
            Ok(Arc::new(client))
        }
        Err(other) => Err(format!("failed to take the data-dir lock: {other}")),
    }
}

#[allow(clippy::too_many_arguments)]
async fn host_gateway(
    loaded: &LoadedGatewayEnv,
    options: &TuiLaunchOptions,
    provider_override: Option<Arc<dyn ModelProvider>>,
    tx: mpsc::UnboundedSender<RuntimeEvent>,
    lock: monoagens_gateway::DataDirLock,
    store_dir: PathBuf,
    owner_actor: ActorId,
    mac_key: [u8; 32],
) -> Result<InProcessClient, String> {
    let toggles = ValveRuleTogglesState::load_from_config(&loaded.paths.config_path);
    let valve_cfg = build_valve_config(&loaded.config, &toggles);
    let mut snapshot = ValveSnapshot {
        owner_actor_id: Some(owner_actor.clone()),
        t_now: SystemClock::new().now(),
        ..ValveSnapshot::default()
    };
    snapshot.known_actors.insert(owner_actor.clone());

    let mut valve =
        tokio::task::spawn_blocking(move || Valve::with_default_policies(valve_cfg, snapshot))
            .await
            .map_err(|e| format!("valve task failed: {e}"))?
            .map_err(|e| format!("valve verification failed: {e}"))?;
    valve.set_approval_attestation_key(mac_key);

    let dynamic_actors = Arc::new(Mutex::new(BTreeSet::new()));
    let door_controller = TuiDoorController {
        inner: ValveDoorController::new(valve),
        dynamic_actors: Arc::clone(&dynamic_actors),
    };

    let mut cfg = loaded.config.clone();
    cfg.data_dir = Some(store_dir.clone());
    if let Some(model_path) = &options.model_override {
        cfg.soul.model_path = Some(model_path.clone());
    }

    let embed_sidecar = if provider_override.is_none() {
        maybe_start_embed_sidecar(&mut cfg)?
    } else {
        None
    };

    let titler_provider = match &provider_override {
        Some(p) => Some(Arc::clone(p)),
        None => build_provider_from_config_async(&cfg, &loaded.secrets)
            .await
            .ok(),
    };

    let clock = Arc::new(SystemClock::new());
    let diagnostics = DiagnosticSink::new(&loaded.secrets);
    let runtime = GatewayRuntime::start_segmented_with_configured_core(
        cfg.clone(),
        &loaded.secrets,
        store_dir.clone(),
        door_controller,
        GatewayOptions {
            clock: clock.clone(),
            provider: provider_override,
            diagnostics,
            cues: CueRecorder::new(),
            seams: GatewaySeams::default(),
        },
    )
    .await
    .map_err(|e| format!("gateway startup failed: {e}"))?;

    let handle = runtime.handle();
    let clock_dyn: Arc<dyn Clock> = clock.clone();
    let runlog = RunLog::new(
        Arc::clone(&clock_dyn),
        handle.diagnostics().clone(),
        Some(store_dir.join("logs").join("tui.log")),
    );
    runlog.attach_diagnostics(handle.diagnostics());
    let supervisor = PluginSupervisor::new(
        handle.clone(),
        PluginEnvContext {
            config_dir: Some(crate::config::config_dir_from_path(&loaded.paths.config_path)),
            data_dir: Some(store_dir.clone()),
            plugin_env: load_plugin_env_secrets(&loaded.paths.secrets_path).unwrap_or_default(),
            rbac_enabled: cfg.rbac.is_some(),
        },
        clock_dyn,
        runlog.clone(),
        RestartPolicy::default(),
        cfg.effective_plugins(),
    );
    let owner = Arc::new(GatewayOwner::new(OwnerParams {
        handle: handle.clone(),
        supervisor,
        runlog: runlog.clone(),
        mode: OwnerMode::Tui,
        config: cfg,
        config_path: loaded.paths.config_path.clone(),
        secrets_path: loaded.paths.secrets_path.clone(),
        data_dir: store_dir.clone(),
        owner_actor: owner_actor.clone(),
    }));
    // This TUI also serves the admin socket so another TUI can attach to it.
    let admin = match AdminServer::start(&lock, &store_dir, Arc::clone(&owner)).await {
        Ok(a) => Some(a),
        Err(e) => {
            runlog.warn("gateway", &format!("admin socket not served: {e}"));
            None
        }
    };

    let client = InProcessClient::new(
        InProcessParts {
            owner: Arc::clone(&owner),
            runtime,
            lock,
            admin,
            owner_actor,
            mac_key,
            clock,
            dynamic_actors,
            titler_provider,
            embed_sidecar,
        },
        tx,
    )
    .await?;

    // Enabled plugins start now (dependency order, crash backoff); states arrive as events.
    let supervisor = owner.supervisor().clone();
    tokio::spawn(async move { supervisor.start_enabled().await });
    Ok(client)
}

fn build_valve_config(cfg: &GatewayConfig, toggles: &ValveRuleTogglesState) -> ValveConfig {
    ValveConfig {
        mode: match cfg.valve.mode.as_str() {
            "partial" => ValveMode::Partial,
            "off" => ValveMode::Off,
            _ => ValveMode::Enforcing,
        },
        cedar_enabled: toggles.cedar_abac,
        line2_enabled: toggles.line2_scope,
        taint_blocking_enabled: toggles.taint_blocking,
        pending_approval_enabled: toggles.pending_approval,
        budget_guard_enabled: toggles.budget_guard,
        chain_brake_enabled: toggles.chain_brake,
        quiet_hours_enabled: toggles.quiet_hours,
        proactive_cap_enabled: toggles.proactive_cap,
        max_chain_wakes: cfg.valve.max_chain_wakes,
        daily_token_budget: cfg.model.daily_token_budget,
        rbac: cfg.rbac.clone(),
        ..ValveConfig::default()
    }
}

fn owner_actor_from_cfg(cfg: Option<&GatewayConfig>) -> ActorId {
    cfg.and_then(|c| c.persons.iter().find(|p| p.is_owner))
        .map(|p| p.actor_id.clone())
        .unwrap_or_else(|| ActorId::new("per_owner"))
}

fn build_tui_percept(
    state: &TuiAppState,
    owner_actor: &ActorId,
    text: &str,
    attachments: &[PendingAttachment],
    now: Time,
) -> Percept {
    let actor = if state.lab_mode {
        state.lab_session.current_actor.clone()
    } else {
        owner_actor.clone()
    };
    let audience = if state.lab_mode && state.lab_session.is_group {
        state.lab_session.audience.clone()
    } else {
        Audience::new([actor.clone(), ActorId::new("soul")])
    };
    let taint = if &actor == owner_actor {
        Taint::Trusted
    } else {
        Taint::Untrusted
    };
    let (expanded_text, attachment_handles) = build_percept_payload_and_attachments(
        text,
        attachments,
        &state.paths.config_path,
        &state.paths.data_dir,
        actor.as_str(),
        state.is_workspace_plugin_active(),
        now,
    );
    let lower = expanded_text.to_ascii_lowercase();
    let mentions_soul = lower.contains("@mono") || lower.contains("@soul");
    let (addressees, named) = if state.lab_mode && state.lab_session.is_group {
        if mentions_soul {
            (vec![ActorId::new("soul")], vec![ActorId::new("soul")])
        } else {
            (Vec::new(), Vec::new())
        }
    } else {
        (vec![ActorId::new("soul")], Vec::new())
    };

    Percept {
        protocol_version: CURRENT_PROTOCOL_VERSION,
        timestamp_ms: now,
        actor: actor.clone(),
        place: PlaceId::new("cli:terminal"),
        audience,
        origin_actor: Some(OriginActor::new(actor)),
        taint,
        payload: OpaquePayload::from_bytes(expanded_text.as_bytes()),
        addressees,
        named,
        orient: Vec::new(),
        reply_to: None,
        correlation_id: None,
        channel: Some(ChannelId::new(state.chats.active_chat().place_id.as_str())),
        delta_obs: None,
        channel_prediction: None,
        length: Some(expanded_text.len() as u64),
        negative_flag: None,
        repair: None,
        continued_attention: None,
        f_modal: None,
        embeddings: Vec::new(),
        attachments: attachment_handles,
        dialogue_act_gap: None,
        peer_expertise_margin: None,
    }
}

fn run_interactive_pair_subprocess(
    guard: &mut TerminalGuard,
    paths: &ResolvedPaths,
    cfg: &GatewayConfig,
    plugin_id: &str,
) -> Result<u8, String> {
    let configured = cfg.effective_plugins().into_iter().find(|p| p.id == plugin_id);
    let binary = if let Some(p) = configured {
        match p.transport {
            BodyTransportConfig::Stdio { command, .. } => command,
            BodyTransportConfig::UnixSocket { path } => {
                return Err(format!(
                    "plugin '{plugin_id}' uses unix_socket ({}) and has no pair binary",
                    path.display()
                ));
            }
        }
    } else if let Some(preset) = find_builtin_preset(plugin_id) {
        discover_builtin_binary(preset)
            .ok_or_else(|| format!("binary '{}' for '{plugin_id}' not found", preset.binary))?
            .display()
            .to_string()
    } else {
        return Err(format!("plugin '{plugin_id}' not configured or built-in"));
    };

    let cfg_dir = crate::config::config_dir_from_path(&paths.config_path);
    let plugin_config = cfg_dir.join("plugins").join(format!("{plugin_id}.toml"));
    let state_dir = paths.data_dir.join("plugins").join(plugin_id);
    std::fs::create_dir_all(&state_dir)
        .map_err(|e| format!("failed to create plugin state dir: {e}"))?;
    let db_path = state_dir.join("session.db");
    let plugin_env = load_plugin_env_secrets(&paths.secrets_path).unwrap_or_default();

    guard.suspend_for_subprocess();

    let mut cmd = StdCommand::new(&binary);
    cmd.arg("pair").arg("--db").arg(&db_path);
    cmd.env("MONOAGENS_PLUGIN_CONFIG", &plugin_config);
    cmd.env("MONOAGENS_PLUGIN_STATE_DIR", &state_dir);
    cmd.env("MONOAGENS_BODY_STATE_DIR", &state_dir);
    for (k, secret) in &plugin_env {
        let v = secret.expose();
        if !v.is_empty() && std::env::var_os(k).is_none() {
            cmd.env(k, v);
        }
    }
    cmd.stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());

    let res = cmd
        .status()
        .map_err(|e| format!("failed to launch '{binary}': {e}"));

    let resume_res = guard.resume_after_subprocess();
    let status = res?;
    resume_res?;

    Ok(match status.code() {
        Some(c) => u8::try_from(c.clamp(0, 255)).unwrap_or(1),
        None => 1,
    })
}

fn maybe_start_embed_sidecar(cfg: &mut GatewayConfig) -> Result<Option<LocalEmbedSidecar>, String> {
    if cfg.embed.backend != EmbedBackendKind::EmbeddingGemma2Http {
        return Ok(None);
    }
    if cfg
        .embed
        .endpoint_url
        .as_deref()
        .is_some_and(|s| !s.trim().is_empty())
        || std::env::var("MONOAGENS_EMBED_ENDPOINT").is_ok_and(|s| !s.trim().is_empty())
    {
        return Ok(None);
    }

    let ws_root = find_workspace_root_for_tui();
    let script = ws_root.join("lab/adapters/eval_helpers/embeddinggemma2_sidecar.py");
    let model_dir = cfg
        .embed
        .model_path
        .clone()
        .unwrap_or_else(|| ws_root.join("lab/datasets/embedder_models/embeddinggemma-2"));
    if !script.is_file() || !model_dir.is_dir() {
        return Ok(None);
    }
    cfg.embed.model_path = Some(model_dir.clone());
    let sidecar = LocalEmbedSidecar::start(&script, &model_dir, &ws_root)?;
    cfg.embed.endpoint_url = Some(sidecar.url().to_owned());
    Ok(Some(sidecar))
}

fn find_workspace_root_for_tui() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    if manifest
        .join("lab/adapters/eval_helpers/embeddinggemma2_sidecar.py")
        .is_file()
    {
        return manifest;
    }
    std::env::current_dir().unwrap_or(manifest)
}

struct TuiDoorController {
    inner: ValveDoorController,
    dynamic_actors: Arc<Mutex<BTreeSet<ActorId>>>,
}

impl TuiDoorController {
    fn sync_dynamic_actors(&mut self) {
        let guard = self.dynamic_actors.lock().expect("dynamic actors lock");
        let known = &mut self.inner.valve_mut().snapshot_mut().known_actors;
        for actor in guard.iter() {
            known.insert(actor.clone());
        }
    }
}

impl Doors for TuiDoorController {
    fn inbound(&mut self, percept: &mut Percept) -> DoorDecision {
        self.sync_dynamic_actors();
        self.inner.inbound(percept)
    }

    fn context(&mut self, plan: &mut ContextPlan) -> DoorDecision {
        self.sync_dynamic_actors();
        self.inner.context(plan)
    }

    fn action(&mut self, action: &mut Action) -> DoorDecision {
        self.inner.action(action)
    }
}

impl DoorController for TuiDoorController {
    fn reload_valve(&mut self, config: &GatewayConfig) -> bool {
        self.inner.reload_valve(config)
    }

    fn sync_runtime_state(
        &mut self,
        log_seq: LogSeq,
        now: Time,
        daily_tokens_used: u64,
        capabilities: &[CapabilityDescriptor],
    ) {
        self.inner
            .sync_runtime_state(log_seq, now, daily_tokens_used, capabilities);
    }

    fn sync_state_epoch(&mut self, state_epoch: u64) {
        self.inner.sync_state_epoch(state_epoch);
    }

    fn replay_from_store_events(&mut self, events: &[EventRecord]) {
        self.inner.replay_from_store_events(events);
    }

    fn sync_item_audiences(&mut self, events: &[EventRecord]) {
        self.inner.sync_item_audiences(events);
    }

    fn resolve_approval(
        &mut self,
        params: &ApprovalResolveParams,
    ) -> Option<Result<monoagens_protocol::ApprovalResolveResult, Waist2Error>> {
        self.inner.resolve_approval(params)
    }

    fn record_in_band_rejection_audit(&mut self, params: &ApprovalResolveParams, detail: &str) {
        self.inner.record_in_band_rejection_audit(params, detail);
    }

    fn take_pending_approval_events(&mut self) -> Vec<EventRecord> {
        self.inner.take_pending_approval_events()
    }

    fn take_approved_action(&mut self, action_id: &ActionId) -> Option<Action> {
        self.inner.take_approved_action(action_id)
    }

    fn take_pending_denial_receipt(&mut self, action_id: &ActionId) -> Option<Receipt> {
        self.inner.take_pending_denial_receipt(action_id)
    }
}

/// Resolves config, secrets, and data paths cleanly for tests/callers.
pub fn resolve_tui_paths(config_path: Option<&Path>) -> Result<ResolvedPaths, String> {
    resolve_paths(config_path).map_err(|e| e.to_string())
}

/// Ensures built-in plugin presets count matches catalog.
#[must_use]
pub const fn builtin_plugin_count() -> usize {
    BUILTIN_PLUGIN_PRESETS.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::LocalChatStore;
    use monoagens_reason::{ScriptedModelProvider, ScriptedReply};
    use serde_json::json;

    fn target_tmp() -> tempfile::TempDir {
        let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/tui-unit-tests");
        std::fs::create_dir_all(&base).expect("mkdir target tmp");
        tempfile::TempDir::new_in(base).expect("tempdir on disk")
    }

    fn speak_reply(text: &str) -> String {
        json!({
            "protocol_version": { "major": 1, "minor": 0 },
            "acts": [
                {
                    "act": "think", "thought": "answer the owner", "motivation": 4,
                    "confidence_0_20": 18, "used_ids": [], "importance": 0.6,
                    "basis": "observed", "value": null, "cost": null, "domain": null
                },
                {
                    "act": "act", "capability_id": "chat.speak", "places": ["cli:terminal"],
                    "people": ["per_owner"], "payload": text.as_bytes(), "motivation": 4,
                    "confidence_0_20": 18, "used_ids": [], "expected": null, "domain": null
                }
            ]
        })
        .to_string()
    }

    /// Two real turns through a hosted gateway: each reply's `chat.speak` action maps back to the
    /// turn that submitted it, so a late reply cannot be taken for the next turn's reply.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn chat_speak_actions_map_back_to_their_submitting_turn() {
        let dir = target_tmp();
        let cfg_dir = dir.path().join("config");
        let data_dir = dir.path().join("data");
        std::fs::create_dir_all(&cfg_dir).unwrap();
        std::fs::create_dir_all(&data_dir).unwrap();
        let mut config = GatewayConfig::default();
        config.model.provider = monoagens_gateway::ModelProviderKind::Scripted;
        config.runtime.model_timeout_ms = 20_000;
        let paths = ResolvedPaths {
            config_path: cfg_dir.join("monoagens.toml"),
            secrets_path: cfg_dir.join("secrets.toml"),
            data_dir: data_dir.clone(),
        };
        let loaded = LoadedGatewayEnv {
            config: config.clone(),
            secrets: ProviderSecrets::default(),
            paths: paths.clone(),
            created_default: false,
        };
        let mut provider = ScriptedModelProvider::new();
        provider.set_fallback(ScriptedReply::text(speak_reply("pong")));
        let options = TuiLaunchOptions {
            data_dir: Some(data_dir),
            ..TuiLaunchOptions::default()
        };
        let (tx, mut rx) = mpsc::unbounded_channel();
        let client = boot_gateway(&loaded, &options, Some(Arc::new(provider)), tx)
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
        let mut follower = LogFollower::default();
        let mut active = None;
        let mut seen: Vec<(u64, String)> = Vec::new();
        for text in ["ping one", "ping two"] {
            state.finish_busy();
            let chat_id = state.chats.active_chat_id.clone();
            let turn = state.turn.begin(&chat_id);
            let mut percept =
                build_tui_percept(&state, &client.owner_actor(), text, &[], client.now());
            percept.correlation_id =
                Some(CorrelationId::new(format!("{TURN_CORRELATION_PREFIX}{turn}")));
            client.submit_turn(percept).await.expect("submit turn");
            let action_id = loop {
                let ev = tokio::time::timeout(Duration::from_secs(20), rx.recv())
                    .await
                    .expect("reply in time")
                    .expect("channel open");
                if let RuntimeEvent::AssistantReply {
                    action_id: Some(a), ..
                } = ev
                {
                    break a;
                }
            };
            poll_new_gateway_events(&client, &mut state, &mut follower, &mut active).await;
            seen.push((turn, action_id));
        }
        for (turn, action_id) in &seen {
            assert_eq!(
                follower.tracker.turn_of_action(action_id),
                Some(*turn),
                "action {action_id} must map to turn {turn}"
            );
        }
        assert_ne!(seen[0].1, seen[1].1);
        client.shutdown().await;
    }

    /// The same call through `SocketClient` (attached) and `InProcessClient` (hosting) answers
    /// alike: the TUI code cannot tell the implementations apart.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn socket_client_status_matches_the_in_process_client() {
        let dir = target_tmp();
        let cfg_dir = dir.path().join("config");
        let data_dir = dir.path().join("data");
        std::fs::create_dir_all(&cfg_dir).unwrap();
        std::fs::create_dir_all(&data_dir).unwrap();
        let mut config = GatewayConfig::default();
        config.model.provider = monoagens_gateway::ModelProviderKind::Scripted;
        let paths = ResolvedPaths {
            config_path: cfg_dir.join("monoagens.toml"),
            secrets_path: cfg_dir.join("secrets.toml"),
            data_dir: data_dir.clone(),
        };
        let loaded = LoadedGatewayEnv {
            config,
            secrets: ProviderSecrets::default(),
            paths,
            created_default: false,
        };
        let options = TuiLaunchOptions {
            data_dir: Some(data_dir.clone()),
            ..TuiLaunchOptions::default()
        };
        let provider: Arc<dyn ModelProvider> = Arc::new(ScriptedModelProvider::new());
        let (tx_host, _rx_host) = mpsc::unbounded_channel();
        let host = boot_gateway(&loaded, &options, Some(Arc::clone(&provider)), tx_host)
            .await
            .expect("host");
        assert_eq!(host.kind(), ClientKind::Hosting);

        // A second boot on the same data dir cannot take the lock and attaches instead.
        let (tx_att, _rx_att) = mpsc::unbounded_channel();
        let attached = boot_gateway(&loaded, &options, Some(provider), tx_att)
            .await
            .expect("attach");
        assert_eq!(attached.kind(), ClientKind::Attached);

        let local = host.status().await.expect("in-process status");
        let remote = attached.status().await.expect("socket status");
        assert_eq!(local, remote);
        assert_eq!(remote["owner"]["pid"], serde_json::json!(std::process::id()));
        assert_eq!(remote["owner"]["mode"], serde_json::json!("tui"));
        assert_eq!(
            host.plugins_list().await.unwrap(),
            attached.plugins_list().await.unwrap()
        );
        assert_eq!(
            host.commands().await.unwrap(),
            attached.commands().await.unwrap()
        );
        attached.shutdown().await;
        host.shutdown().await;
    }
}
