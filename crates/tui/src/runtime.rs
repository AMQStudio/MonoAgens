//! Panic-safe terminal guard, Gateway runtime bridge, live plugin management,
//! non-blocking LLM titler tasks, and interactive pairing subprocess handoff.

use std::collections::{BTreeSet, HashMap};
use std::io::{self, IsTerminal};
use std::path::{Path, PathBuf};
use std::process::{Command as StdCommand, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crossterm::event::{
    DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture, Event,
};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use monoagens_core::{
    ContextPlan, DecisionOutcome, DecisionReason, DecisionTrace, DoorDecision, Doors, EventDetail,
    EventKind, EventRecord, Input, LogSeq,
};
use monoagens_gateway::{
    ApprovalPromptRequest, BodyTransportConfig, CAPABILITY_APPROVAL_PROMPT, Clock,
    CommandOutcome, CueRecorder, DiagnosticSink, DoorController, EmbedBackendKind, GatewayConfig,
    GatewayHandle, GatewayOptions, GatewayRuntime, GatewaySeams, LifecycleCueEvent,
    LifecycleCueKind, LoadedGatewayEnv, PluginConfig, PluginEnvContext, ResolvedPaths, SystemClock,
    ValveDoorController, build_plugin_env, build_provider_from_config_async,
    load_approval_secrets, load_or_init_gateway_env, load_plugin_env_secrets,
    resolve_approval_mac_key, resolve_paths,
};
use monoagens_protocol::{
    Action, ActionExecuteParams, ActionExecuteResult, ActionId, ActorId, ApprovalResolveParams,
    Audience, BodyDiscoverParams, BodyDiscoverResult, BodyHeartbeatResult, BodyId,
    BodyShutdownResult, CURRENT_PROTOCOL_VERSION, CapabilityDescriptor, CapabilityFlags,
    CapabilityId, ChannelId, OpaquePayload, OriginActor, Percept, PlaceId, Receipt,
    ReceiptOutcome, RpcId, Taint, TerminationReason, Time, Waist2Error, Waist2Method,
    Waist2Request, Waist2Response, sort_capabilities,
};
use monoagens_reason::{LocalEmbedSidecar, ModelProvider};
use monoagens_valve::{ApprovalLogRecord, Valve, ValveConfig, ValveMode, ValveSnapshot, sign_approval_resolution};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader as TokioBufReader};
use tokio::process::{Child, Command as TokioCommand};
use tokio::sync::mpsc;

use crate::attachments::{
    PendingAttachment, PendingAttachmentKind, build_percept_payload_and_attachments,
    parse_image_dimensions,
};
use crate::clipboard::{ClipboardContent, copy_to_clipboard, read_clipboard_image_or_text};
use crate::config::{
    BUILTIN_PLUGIN_PRESETS, LocalChatStore, TuiPreferences, discover_builtin_binary,
    find_builtin_preset, list_secret_keys, merged_plugin_ids,
};
use crate::effects::TuiEffects;
use crate::render::render_to_buffer;
use crate::settings::{PluginUiState, SecretsUiSummary, SetupWizardState, ValveRuleTogglesState};
use crate::state::{
    AppAction, ChatTimelineItem, HitRegions, PendingApprovalCard, TuiAppState,
};
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
        if let Err(e) = execute!(stdout, EnterAlternateScreen, EnableBracketedPaste) {
            let _ = disable_raw_mode();
            return Err(format!("failed to enter alternate screen: {e}"));
        }
        if mouse_enabled && let Err(e) = execute!(stdout, EnableMouseCapture) {
            let _ = execute!(stdout, DisableBracketedPaste, LeaveAlternateScreen);
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
        execute!(stdout, EnterAlternateScreen, EnableBracketedPaste)
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

/// Launches the interactive MonoAgens TUI with an optional injected [`ModelProvider`].
pub fn run_tui_with_provider(
    options: TuiLaunchOptions,
    provider_override: Option<Arc<dyn ModelProvider>>,
) -> Result<u8, String> {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return Err("monoagens TUI requires an interactive TTY (stdin/stdout)".to_owned());
    }

    let mut loaded = load_or_init_gateway_env(options.config_path.as_deref(), |k| {
        std::env::var(k).ok()
    })
    .map_err(|e| e.to_string())?;

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
    let needs_wizard =
        provider_override.is_none() && loaded.ensure_startup_ready().is_err();

    let mut state = TuiAppState::new(
        loaded.paths.clone(),
        loaded.config.clone(),
        prefs,
        chats,
        secrets_summary,
        owner_actor.clone(),
    );
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

enum RuntimeEvent {
    AssistantReply {
        chat_id: String,
        text: String,
        duration_ms: u64,
    },
    ApprovalPrompt(PendingApprovalCard),
    TurnError(String),
    TurnFinished,
    PluginState {
        id: String,
        state: PluginUiState,
    },
    ChatTitleReady {
        chat_id: String,
        title: String,
        keywords: Vec<String>,
        is_topic_shift: bool,
    },
}

struct BootedTuiGateway {
    runtime: GatewayRuntime,
    handle: GatewayHandle,
    owner_actor: ActorId,
    mac_key: [u8; 32],
    clock: Arc<SystemClock>,
    dynamic_actors: Arc<Mutex<BTreeSet<ActorId>>>,
    titler_provider: Option<Arc<dyn ModelProvider>>,
    body_task: tokio::task::JoinHandle<()>,
    _embed_sidecar: Option<LocalEmbedSidecar>,
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
    let mut booted: Option<BootedTuiGateway> = None;
    let mut plugin_children: HashMap<String, (Child, Option<BodyId>)> = HashMap::new();
    let mut seen_events_len: usize = 0;
    let mut resolved_approvals: BTreeSet<ActionId> = BTreeSet::new();

    if state.wizard.is_none() {
        match boot_tui_gateway(&loaded, &options, provider_override.clone(), tx.clone()).await {
            Ok(b) => {
                sync_gateway_commands(state, &b.handle);
                seen_events_len = b.handle.read_events().await.map_or(0, |e| e.len());
                if options.with_plugins {
                    spawn_auto_plugins(state, &b.handle, &loaded.paths, &tx, &mut plugin_children);
                }
                booted = Some(b);
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
            }
        }
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

    let mut cue_rx = booted.as_ref().map(|b| b.handle.cues().subscribe());
    let mut tick_interval = tokio::time::interval(Duration::from_millis(80));
    tick_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut last_draw = Instant::now();
    let mut last_busy_start: Option<Instant> = None;
    let mut needs_redraw = true;

    loop {
        if needs_redraw || effects.is_running() || state.is_busy() {
            let now = Instant::now();
            let elapsed_ms = now.duration_since(last_draw).as_millis().clamp(1, 250) as u32;
            last_draw = now;
            if let Some(started) = last_busy_start {
                state.thinking_elapsed_ms = started.elapsed().as_millis() as u64;
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
                let actions = match ev {
                    Event::Key(k) => state.handle_key_event(k),
                    Event::Mouse(m) => state.handle_mouse_event(m, &hit, viewport),
                    Event::Paste(text) => state.handle_paste(&text),
                    _ => Vec::new(),
                };
                needs_redraw = true;

                for action in actions {
                    handle_app_action(
                        action,
                        state,
                        guard,
                        &mut loaded,
                        &options,
                        &provider_override,
                        &mut booted,
                        &mut cue_rx,
                        &mut seen_events_len,
                        &mut plugin_children,
                        &mut last_busy_start,
                        &tx,
                        &mut effects,
                    )
                    .await;
                }
            }
            maybe_rt = rx.recv() => {
                if let Some(rt_ev) = maybe_rt {
                    handle_runtime_event(
                        rt_ev,
                        state,
                        &mut last_busy_start,
                        &mut effects,
                    );
                    needs_redraw = true;
                }
            }
            cue_opt = async {
                match &mut cue_rx {
                    Some(rx_cues) => rx_cues.recv().await,
                    None => std::future::pending().await,
                }
            } => {
                if let Some(cue) = cue_opt {
                    state.thinking_phase = match cue.kind {
                        LifecycleCueKind::WorkingCue => "Working…".to_owned(),
                        LifecycleCueKind::ProgressNote => "Still working…".to_owned(),
                    };
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
                    if let Some(b) = &booted {
                        poll_new_gateway_events(
                            &b.handle,
                            &b.clock,
                            state,
                            &mut seen_events_len,
                            &mut resolved_approvals,
                        )
                        .await;
                    }
                    needs_redraw = true;
                } else if effects.is_running() {
                    needs_redraw = true;
                }
            }
        }
    }

    for (_, (mut child, _)) in plugin_children.drain() {
        let _ = child.kill().await;
    }
    if let Some(b) = booted {
        b.body_task.abort();
        b.runtime.shutdown().await;
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
    booted: &mut Option<BootedTuiGateway>,
    cue_rx: &mut Option<mpsc::UnboundedReceiver<LifecycleCueEvent>>,
    seen_events_len: &mut usize,
    plugin_children: &mut HashMap<String, (Child, Option<BodyId>)>,
    last_busy_start: &mut Option<Instant>,
    tx: &mpsc::UnboundedSender<RuntimeEvent>,
    effects: &mut TuiEffects,
) {
    match action {
        AppAction::ToggleMouseCapture(enabled) => {
            guard.set_mouse_capture(enabled);
        }
        AppAction::CompleteSetupWizard => {
            if booted.is_none()
                && let Ok(reloaded) = load_or_init_gateway_env(options.config_path.as_deref(), |k| {
                    std::env::var(k).ok()
                })
            {
                *loaded = reloaded;
                if let Some(custom_data) = &options.data_dir {
                    loaded.paths.data_dir = custom_data.clone();
                }
                if loaded.ensure_startup_ready().is_ok() || provider_override.is_some() {
                    match boot_tui_gateway(loaded, options, provider_override.clone(), tx.clone())
                        .await
                    {
                        Ok(b) => {
                            sync_gateway_commands(state, &b.handle);
                            *cue_rx = Some(b.handle.cues().subscribe());
                            *seen_events_len = b.handle.read_events().await.map_or(0, |e| e.len());
                            *booted = Some(b);
                        }
                        Err(err) => {
                            state.set_toast(err, true);
                        }
                    }
                }
            }
        }
        AppAction::SendUserMessage {
            chat_id,
            place_id: _,
            text,
            attachments,
        } => {
            *last_busy_start = Some(Instant::now());
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
                && let Some(b) = booted.as_ref()
                && let Some(tp) = &b.titler_provider
            {
                let tp = Arc::clone(tp);
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

            let Some(b) = booted.as_ref() else {
                state.thinking = false;
                *last_busy_start = None;
                state.push_timeline_item(
                    &chat_id,
                    ChatTimelineItem::SystemNotice {
                        text: "Provider not configured yet. Open /settings -> Providers to set model_api_key."
                            .to_owned(),
                        is_error: true,
                    },
                );
                return;
            };

            sync_dynamic_actors(&b.dynamic_actors, state);
            let now = b.clock.now();
            let mut percept = build_tui_percept(state, &b.owner_actor, &text, &attachments, now);
            let handle = b.handle.clone();
            let tx_msg = tx.clone();
            tokio::spawn(async move {
                if let Some(pq) = handle.pre_queue_service() {
                    let _ = pq.enrich_percept_async(&mut percept).await;
                }
                if let Err(err) = handle.submit_and_wait(Input::Percept(Box::new(percept))).await {
                    let _ = tx_msg.send(RuntimeEvent::TurnError(err.to_string()));
                } else {
                    let _ = tx_msg.send(RuntimeEvent::TurnFinished);
                }
            });
        }
        AppAction::ResolveApproval {
            action_id,
            h_act,
            state_epoch,
            approved,
        } => {
            if let Some(b) = booted.as_ref() {
                let now = b.clock.now();
                let sig = sign_approval_resolution(
                    &b.mac_key,
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
                    resolver: b.owner_actor.clone(),
                    resolved_at_ms: now,
                    signature: Some(sig),
                    out_of_band_channel: true,
                };
                match b.handle.resolve_approval(params).await {
                    Ok(_) => {
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
            if let Some(b) = booted.as_ref()
                && let Some(p) = state.plugins.iter().find(|p| p.id == pid).cloned()
            {
                let spec = state
                    .gateway_cfg
                    .effective_plugins()
                    .into_iter()
                    .find(|c| c.id == pid);
                if let Some(cfg) = spec {
                    let handle = b.handle.clone();
                    let tx_p = tx.clone();
                    let paths = loaded.paths.clone();
                    let rbac_enabled = state.gateway_cfg.rbac.is_some();
                    spawn_single_plugin(cfg, handle, paths, rbac_enabled, tx_p, plugin_children);
                } else {
                    state.set_plugin_state(
                        &pid,
                        PluginUiState::Failed(format!("plugin '{}' not configured", p.id)),
                    );
                }
            } else {
                state.set_plugin_state(&pid, PluginUiState::Stopped);
            }
        }
        AppAction::StopPlugin(pid) => {
            if let Some((mut child, body_id)) = plugin_children.remove(&pid) {
                let _ = child.kill().await;
                if let (Some(b), Some(bid)) = (booted.as_ref(), body_id) {
                    b.handle
                        .router()
                        .handle_body_disconnect(&bid, "stopped by user");
                }
            }
            state.set_plugin_state(&pid, PluginUiState::Stopped);
            state.set_toast(format!("Stopped plugin '{pid}'"), false);
        }
        AppAction::PairPlugin(pid) => {
            let res = run_interactive_pair_subprocess(guard, &loaded.paths, &state.gateway_cfg, &pid);
            match res {
                Ok(0) => state.set_toast(format!("Paired '{pid}' successfully"), false),
                Ok(code) => state.set_toast(format!("Pairing '{pid}' exited with code {code}"), true),
                Err(e) => state.set_toast(e, true),
            }
            let palette = crate::theme::ThemePalette::for_preset(state.prefs.theme);
            effects.trigger_modal_dissolve(state.prefs.motion, &palette);
        }
        AppAction::SubmitGatewayCommand(raw) => {
            if let Some(b) = booted.as_ref() {
                let reg = b.handle.command_registry();
                let caps: Vec<CapabilityId> = b
                    .handle
                    .router()
                    .all_capabilities()
                    .into_iter()
                    .map(|c| c.id)
                    .collect();
                let outcome = reg.lock().expect("cmd reg lock").try_handle_slash_command(
                    &raw,
                    &state.lab_session.current_actor,
                    &b.owner_actor,
                    Some(&PlaceId::new(&state.chats.active_chat().place_id)),
                    state.gateway_cfg.rbac.as_ref(),
                    &caps,
                );
                let active_id = state.chats.active_chat_id.clone();
                match outcome {
                    Some(CommandOutcome::Handled { reply_text }) => {
                        state.push_timeline_item(
                            &active_id,
                            ChatTimelineItem::SystemNotice {
                                text: reply_text,
                                is_error: false,
                            },
                        );
                    }
                    Some(CommandOutcome::Denied { reason }) => {
                        state.push_timeline_item(
                            &active_id,
                            ChatTimelineItem::SystemNotice {
                                text: reason,
                                is_error: true,
                            },
                        );
                    }
                    Some(CommandOutcome::ForwardToPlugin { body_id, command, args }) => {
                        state.push_timeline_item(
                            &active_id,
                            ChatTimelineItem::SystemNotice {
                                text: format!("Forwarded /{command} {args} to {}", body_id.as_str()),
                                is_error: false,
                            },
                        );
                    }
                    None => {
                        state.push_timeline_item(
                            &active_id,
                            ChatTimelineItem::SystemNotice {
                                text: format!("Unknown command '{raw}'. Type /help for commands."),
                                is_error: true,
                            },
                        );
                    }
                }
            }
        }
        AppAction::CopyToClipboard(text) => {
            if let Err(err) = copy_to_clipboard(&text) {
                state.set_toast(format!("Clipboard copy failed: {err}"), true);
            }
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
}

fn handle_runtime_event(
    rt_ev: RuntimeEvent,
    state: &mut TuiAppState,
    last_busy_start: &mut Option<Instant>,
    effects: &mut TuiEffects,
) {
    let palette = crate::theme::ThemePalette::for_preset(state.prefs.theme);
    match rt_ev {
        RuntimeEvent::AssistantReply {
            chat_id,
            text,
            duration_ms,
        } => {
            state.thinking = false;
            *last_busy_start = None;
            state.push_timeline_item(
                &chat_id,
                ChatTimelineItem::AssistantMessage {
                    text,
                    duration_ms: Some(duration_ms),
                    timestamp_ms: 0,
                },
            );
            effects.trigger_message_fade_in(state.prefs.motion, &palette);
        }
        RuntimeEvent::ApprovalPrompt(card) => {
            state.pending_approval = Some(card);
        }
        RuntimeEvent::TurnError(err) => {
            state.thinking = false;
            *last_busy_start = None;
            let cid = state.chats.active_chat_id.clone();
            state.push_timeline_item(
                &cid,
                ChatTimelineItem::SystemNotice {
                    text: format!("Turn error: {err}"),
                    is_error: true,
                },
            );
        }
        RuntimeEvent::TurnFinished => {
            // Replies arrive asynchronously via chat.speak; only clear if no ModelCall pending.
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
    }
}

async fn poll_new_gateway_events(
    handle: &GatewayHandle,
    clock: &Arc<SystemClock>,
    state: &mut TuiAppState,
    seen_len: &mut usize,
    resolved_approvals: &mut BTreeSet<ActionId>,
) {
    let (used, limit) = handle.budget().snapshot(clock.now());
    state.daily_tokens_used = used;
    state.daily_token_budget = limit.max(1);

    let Ok(events) = handle.read_events().await else {
        return;
    };
    if events.len() <= *seen_len {
        return;
    }
    let active_id = state.chats.active_chat_id.clone();
    for ev in &events[*seen_len..] {
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
                        state.thinking = false;
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
                    state.thinking = false;
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
                        state.thinking = false;
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
    *seen_len = events.len();
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

async fn boot_tui_gateway(
    loaded: &LoadedGatewayEnv,
    options: &TuiLaunchOptions,
    provider_override: Option<Arc<dyn ModelProvider>>,
    tx: mpsc::UnboundedSender<RuntimeEvent>,
) -> Result<BootedTuiGateway, String> {
    let owner_actor = owner_actor_from_cfg(Some(&loaded.config));
    let toggles = ValveRuleTogglesState::load_from_config(&loaded.paths.config_path);
    let valve_cfg = build_valve_config(&loaded.config, &toggles);
    let mut snapshot = ValveSnapshot {
        owner_actor_id: Some(owner_actor.clone()),
        t_now: SystemClock::new().now(),
        ..ValveSnapshot::default()
    };
    snapshot.known_actors.insert(owner_actor.clone());

    let approval_secrets =
        load_approval_secrets(Some(&loaded.paths.secrets_path), |k| std::env::var(k).ok())
            .map_err(|e| e.to_string())?;
    let mac_key = resolve_approval_mac_key(&approval_secrets);

    let mut valve =
        tokio::task::spawn_blocking(move || Valve::with_default_policies(valve_cfg, snapshot))
            .await
            .map_err(|e| format!("valve task failed: {e}"))?
            .map_err(|e| format!("valve verification failed: {e}"))?;
    valve.set_approval_attestation_key(mac_key);

    let dynamic_actors = Arc::new(Mutex::new(BTreeSet::new()));
    let actors_for_doors = Arc::clone(&dynamic_actors);
    let door_controller = TuiDoorController {
        inner: ValveDoorController::new(valve),
        dynamic_actors: actors_for_doors,
    };

    let mut cfg = loaded.config.clone();
    let store_dir = options
        .data_dir
        .clone()
        .or_else(|| cfg.data_dir.clone())
        .unwrap_or_else(|| loaded.paths.data_dir.clone());
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
    let cues = CueRecorder::new();

    let runtime = GatewayRuntime::start_segmented_with_configured_core(
        cfg.clone(),
        &loaded.secrets,
        store_dir,
        door_controller,
        GatewayOptions {
            clock: clock.clone(),
            provider: provider_override,
            diagnostics,
            cues,
            seams: GatewaySeams::default(),
        },
    )
    .await
    .map_err(|e| format!("gateway startup failed: {e}"))?;

    let handle = runtime.handle();
    let (gw_stream, body_stream) = tokio::io::duplex(64 * 1024);
    let (gw_read, gw_write) = tokio::io::split(gw_stream);
    let (body_read, mut body_write) = tokio::io::split(body_stream);

    let tx_body = tx.clone();
    let clock_body = Arc::clone(&clock);
    let body_task = tokio::spawn(async move {
        let mut reader = TokioBufReader::new(body_read);
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line).await {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    if let Some(resp) = handle_tui_waist2_line(&line, &tx_body, clock_body.now())
                        && (body_write.write_all(resp.as_bytes()).await.is_err()
                            || body_write.write_all(b"\n").await.is_err()
                            || body_write.flush().await.is_err())
                    {
                        break;
                    }
                }
            }
        }
    });

    handle
        .connect_ndjson_body(gw_read, gw_write, Some(BodyId::new("body-tui")))
        .await
        .map_err(|e| format!("failed to connect body-tui over Waist 2: {e}"))?;

    Ok(BootedTuiGateway {
        runtime,
        handle,
        owner_actor,
        mac_key,
        clock,
        dynamic_actors,
        titler_provider,
        body_task,
        _embed_sidecar: embed_sidecar,
    })
}

fn handle_tui_waist2_line(
    line: &str,
    tx: &mpsc::UnboundedSender<RuntimeEvent>,
    now: Time,
) -> Option<String> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    let req: Waist2Request<serde_json::Value> = serde_json::from_str(trimmed).ok()?;
    let id = req.id?;

    let result_val = match req.method {
        Waist2Method::BodyDiscover => {
            let params: BodyDiscoverParams = match serde_json::from_value(req.params) {
                Ok(p) => p,
                Err(e) => {
                    return serialize_waist2_err(
                        id,
                        Waist2Error::invalid_receipt_transition(e.to_string()),
                    );
                }
            };
            let mut caps = vec![
                CapabilityDescriptor {
                    id: CapabilityId::new("chat.speak"),
                    description: "Render chat reply in interactive TUI".to_owned(),
                    places: vec![PlaceId::new("cli:terminal")],
                    flags: CapabilityFlags {
                        communicative: true,
                        identity_changing: false,
                        irreversible: false,
                    },
                    typical_latency_ms: 0,
                    capability_schema_hash: monoagens_protocol::CapabilitySchemaHash::new(
                        "tui-chat-speak",
                    ),
                    sensed_effect: None,
                },
                CapabilityDescriptor {
                    id: CapabilityId::new(CAPABILITY_APPROVAL_PROMPT),
                    description: "Render interactive Valve approval card in TUI".to_owned(),
                    places: vec![PlaceId::new("cli:terminal")],
                    flags: CapabilityFlags {
                        communicative: true,
                        identity_changing: false,
                        irreversible: false,
                    },
                    typical_latency_ms: 0,
                    capability_schema_hash: monoagens_protocol::CapabilitySchemaHash::new(
                        "tui-approval-prompt",
                    ),
                    sensed_effect: None,
                },
            ];
            sort_capabilities(&mut caps);
            let res = BodyDiscoverResult {
                body_id: params.body_id.unwrap_or_else(|| BodyId::new("body-tui")),
                protocol_version: CURRENT_PROTOCOL_VERSION,
                capabilities: caps,
                places: vec![PlaceId::new("cli:terminal")],
            };
            serde_json::to_value(res).ok()?
        }
        Waist2Method::BodyHeartbeat => {
            let res = BodyHeartbeatResult {
                acknowledged_at_ms: now,
            };
            serde_json::to_value(res).ok()?
        }
        Waist2Method::BodyShutdown => {
            let res = BodyShutdownResult { accepted: true };
            serde_json::to_value(res).ok()?
        }
        Waist2Method::ActionExecute => {
            let params: ActionExecuteParams = match serde_json::from_value(req.params) {
                Ok(p) => p,
                Err(e) => {
                    return serialize_waist2_err(
                        id,
                        Waist2Error::invalid_receipt_transition(e.to_string()),
                    );
                }
            };
            if params.capability_id.as_str() == "chat.speak" {
                let text = extract_chat_speak_text(params.payload.as_bytes());
                let _ = tx.send(RuntimeEvent::AssistantReply {
                    chat_id: "direct".to_owned(),
                    text,
                    duration_ms: 0,
                });
            } else if params.capability_id.as_str() == CAPABILITY_APPROVAL_PROMPT
                && let Ok(prompt) =
                    serde_json::from_slice::<ApprovalPromptRequest>(params.payload.as_bytes())
            {
                let _ = tx.send(RuntimeEvent::ApprovalPrompt(PendingApprovalCard {
                    action_id: prompt.action_id,
                    h_act: prompt.h_act,
                    state_epoch: prompt.state_epoch,
                    capability_id: prompt.capability_id.as_str().to_owned(),
                    summary: prompt.summary,
                    selected_approve: true,
                }));
            }
            let receipt = Receipt {
                action_id: params.action_id,
                capability_id: params.capability_id,
                outcome: ReceiptOutcome::Done,
                reason_code: None,
                detail: None,
                termination_reason: Some(TerminationReason::Normal),
                progress: Some(1.0),
                delivered_units: Some(1),
                h_act: params.h_act,
                timestamp_ms: now,
            };
            let res = ActionExecuteResult {
                accepted: true,
                receipt: Some(receipt),
            };
            serde_json::to_value(res).ok()?
        }
        _ => {
            return serialize_waist2_err(
                id,
                Waist2Error::invalid_receipt_transition("unsupported method"),
            );
        }
    };

    let resp = Waist2Response::ok(Some(id), result_val);
    serde_json::to_string(&resp).ok()
}

fn serialize_waist2_err(id: RpcId, err: Waist2Error) -> Option<String> {
    let resp = Waist2Response::<serde_json::Value>::err(Some(id), err);
    serde_json::to_string(&resp).ok()
}

fn extract_chat_speak_text(bytes: &[u8]) -> String {
    if let Ok(val) = serde_json::from_slice::<serde_json::Value>(bytes)
        && let Some(t) = val.get("text").and_then(|v| v.as_str())
    {
        return t.to_owned();
    }
    String::from_utf8_lossy(bytes).to_string()
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

fn sync_dynamic_actors(dynamic_actors: &Arc<Mutex<BTreeSet<ActorId>>>, state: &TuiAppState) {
    let mut guard = dynamic_actors.lock().expect("dynamic actors lock");
    for a in &state.lab_session.known_actors {
        if a != "soul" {
            guard.insert(ActorId::new(a));
        }
    }
}

fn sync_gateway_commands(state: &mut TuiAppState, handle: &GatewayHandle) {
    let reg = handle.command_registry();
    let specs = reg
        .lock()
        .expect("cmd reg lock")
        .all_commands()
        .into_iter()
        .cloned()
        .collect();
    state.gateway_commands = specs;
}

fn spawn_auto_plugins(
    state: &mut TuiAppState,
    handle: &GatewayHandle,
    paths: &ResolvedPaths,
    tx: &mpsc::UnboundedSender<RuntimeEvent>,
    children: &mut HashMap<String, (Child, Option<BodyId>)>,
) {
    let rbac_enabled = state.gateway_cfg.rbac.is_some();
    for cfg in state.gateway_cfg.effective_plugins() {
        if cfg.enabled {
            state.set_plugin_state(&cfg.id, PluginUiState::Starting);
            spawn_single_plugin(
                cfg,
                handle.clone(),
                paths.clone(),
                rbac_enabled,
                tx.clone(),
                children,
            );
        }
    }
}

fn spawn_single_plugin(
    cfg: PluginConfig,
    handle: GatewayHandle,
    paths: ResolvedPaths,
    rbac_enabled: bool,
    tx: mpsc::UnboundedSender<RuntimeEvent>,
    children: &mut HashMap<String, (Child, Option<BodyId>)>,
) {
    let pid = cfg.id.clone();
    match cfg.transport {
        BodyTransportConfig::Stdio { command, args } => {
            let env_ctx = PluginEnvContext {
                config_dir: Some(crate::config::config_dir_from_path(&paths.config_path)),
                data_dir: Some(paths.data_dir.clone()),
                plugin_env: load_plugin_env_secrets(&paths.secrets_path).unwrap_or_default(),
                rbac_enabled,
            };
            let extra_envs = build_plugin_env(&pid, &env_ctx);
            let mut cmd = TokioCommand::new(&command);
            cmd.args(&args)
                .envs(extra_envs)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null());
            match cmd.spawn() {
                Ok(mut child) => {
                    let stdin = child.stdin.take();
                    let stdout = child.stdout.take();
                    children.insert(pid.clone(), (child, None));
                    if let (Some(stdin), Some(stdout)) = (stdin, stdout) {
                        tokio::spawn(async move {
                            match handle.connect_ndjson_body(stdout, stdin, None).await {
                                Ok(_body_id) => {
                                    let _ = tx.send(RuntimeEvent::PluginState {
                                        id: pid,
                                        state: PluginUiState::Running,
                                    });
                                }
                                Err(err) => {
                                    let _ = tx.send(RuntimeEvent::PluginState {
                                        id: pid,
                                        state: PluginUiState::Failed(err.to_string()),
                                    });
                                }
                            }
                        });
                    } else {
                        let _ = tx.send(RuntimeEvent::PluginState {
                            id: pid,
                            state: PluginUiState::Failed("missing stdio pipes".to_owned()),
                        });
                    }
                }
                Err(e) => {
                    let _ = tx.send(RuntimeEvent::PluginState {
                        id: pid,
                        state: PluginUiState::Failed(format!("spawn '{command}' failed: {e}")),
                    });
                }
            }
        }
        BodyTransportConfig::UnixSocket { path } => {
            tokio::spawn(async move {
                match handle.connect_unix_body(&path).await {
                    Ok(_) => {
                        let _ = tx.send(RuntimeEvent::PluginState {
                            id: pid,
                            state: PluginUiState::Running,
                        });
                    }
                    Err(err) => {
                        let _ = tx.send(RuntimeEvent::PluginState {
                            id: pid,
                            state: PluginUiState::Failed(err.to_string()),
                        });
                    }
                }
            });
        }
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
