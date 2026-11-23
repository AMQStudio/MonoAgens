//! The only way the TUI reaches the gateway: the [`GatewayClient`] trait with two
//! implementations.
//!
//! * [`InProcessClient`]: this TUI won the data-dir lock and hosts the gateway. It calls the
//!   [`GatewayOwner`] directly and also serves the admin socket so another TUI can attach.
//! * [`SocketClient`]: the lock is held by `monoagens serve` (or another TUI); every call goes
//!   over `<data_dir>/gateway.sock`, and the chat body (`body-tui`) is attached through the same
//!   socket with `body.attach`.
//!
//! TUI code never matches on which implementation it has. Pushed information (chat replies,
//! plugin state, log entries, lifecycle cues, owner loss) reaches the event loop through the
//! `tx` channel given at connect time as [`RuntimeEvent`] values.

use std::collections::{BTreeSet, HashMap};
use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

use monoagens_core::EventRecord;
use monoagens_gateway::{
    AdminError, AdminServer, CAPABILITY_APPROVAL_PROMPT, ApprovalPromptRequest, Clock, CommandSpec,
    DataDirLock, DiagnosticSink, GatewayHandle, GatewayOwner, GatewayRuntime, LifecycleCueEvent,
    LogEntry, PluginInfo, PluginState, PluginStateEvent, PreviewEvent, RunLog, RunLogLevel,
    SystemClock, admin_socket_path,
};
use monoagens_protocol::{
    ActionExecuteParams, ActionExecuteResult, ActorId, ApprovalResolveParams, BodyDiscoverParams,
    BodyDiscoverResult, BodyHeartbeatResult, BodyId, BodyShutdownResult, CURRENT_PROTOCOL_VERSION,
    CapabilityDescriptor, CapabilityFlags, CapabilityId, Percept, PlaceId, Receipt, ReceiptOutcome,
    RpcId, TerminationReason, Time, Waist2Error, Waist2Method, Waist2Request, Waist2Response,
    sort_capabilities,
};
use monoagens_reason::{LocalEmbedSidecar, ModelProvider, ProviderSecrets};
use serde_json::{Value, json};
use tokio::io::{
    AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader as TokioBufReader,
};
use tokio::net::UnixStream;
use tokio::net::unix::OwnedWriteHalf;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

use crate::settings::PluginUiState;
use crate::state::PendingApprovalCard;

/// Boxed future returned by every [`GatewayClient`] method (keeps the trait object-safe).
pub type Fut<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;
/// Error of a client call (same shape as an admin JSON-RPC error).
pub type ClientError = AdminError;
/// Result of a client call.
pub type ClientResult<T> = Result<T, ClientError>;

/// Name of the TUI chat body and its place.
pub const TUI_BODY_ID: &str = "body-tui";
/// Place owned by the TUI chat body.
pub const TUI_PLACE: &str = "cli:terminal";

/// Something pushed to the TUI event loop by a client or the chat body.
#[derive(Debug)]
pub enum RuntimeEvent {
    /// `chat.speak` reached the TUI body; `action_id` identifies the action so the reply can be
    /// mapped back to the turn that caused it.
    AssistantReply {
        chat_id: Option<String>,
        text: String,
        duration_ms: u64,
        action_id: Option<String>,
    },
    ApprovalPrompt(PendingApprovalCard),
    TurnError { turn: u64, error: String },
    TurnFinished { turn: u64, effects_spawned: usize },
    PluginState { id: String, state: PluginUiState },
    ChatTitleReady {
        chat_id: String,
        title: String,
        keywords: Vec<String>,
        is_topic_shift: bool,
    },
    /// `/doctor` finished; the report text is ready.
    Doctor(String),
    /// A runtime log entry (gateway, plugin or local).
    Log(LogEntry),
    /// A lifecycle cue (`working_cue` / `progress_note`).
    Cue(LifecycleCueEvent),
    /// The gateway owner went away (socket closed).
    OwnerGone,
    /// Result of a `/model`, `/login` or `/usage` request.
    Catalog(crate::catalog_ui::CatalogEvent),
    /// Result of a governance / observability admin call (`crate::views`).
    Gov(crate::views::GovResponse),
    /// Conversation features: draft preview, trace/explain results, editor, cancel.
    Conv(crate::conv::ConvEvent),
}

/// Page of the owner's event log.
#[derive(Debug, Clone, Default)]
pub struct EventsPage {
    /// Total number of events in the log.
    pub total: usize,
    /// Events at the requested offset and after.
    pub events: Vec<EventRecord>,
    pub budget_used: u64,
    pub budget_limit: u64,
    /// Cost so far in USD; `None` while any price is unknown (the status bar hides it).
    pub cost_usd: Option<f64>,
}

/// Outcome of a gateway slash command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandReply {
    Handled(String),
    Denied(String),
    Forward { body_id: String, command: String, args: String },
    Unknown,
}

/// How this client reaches the gateway (display only).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientKind {
    /// This TUI hosts the gateway.
    Hosting,
    /// Attached to another process over the admin socket.
    Attached,
}

/// The TUI's single door to the gateway.
pub trait GatewayClient: Send + Sync {
    fn kind(&self) -> ClientKind;
    fn owner_actor(&self) -> ActorId;
    fn now(&self) -> Time;
    /// Key used to sign out-of-band approval resolutions.
    fn approval_mac_key(&self) -> [u8; 32];
    /// Provider and model name for the background chat titler: the classify purpose's route,
    /// swapped live by `config.apply` (`None` when attached).
    fn titler_route(&self) -> Option<(Arc<dyn ModelProvider>, String)>;
    /// Makes simulated (lab) actors known to the valve; a no-op when attached.
    fn register_actors(&self, actors: &[ActorId]);
    /// Why the chat body is unavailable (attached while another TUI owns the terminal place).
    fn chat_unavailable_reason(&self) -> Option<String>;
    /// Writes a local log entry (masked, filed and pushed back as [`RuntimeEvent::Log`]).
    fn log(&self, level: RunLogLevel, source: &str, message: &str);

    fn status(&self) -> Fut<'_, ClientResult<Value>>;
    fn config_get(&self) -> Fut<'_, ClientResult<Value>>;
    fn config_apply(&self, patch: Value) -> Fut<'_, ClientResult<Value>>;
    /// `providers.list`: catalog providers with key state (never key material).
    fn providers_list(&self) -> Fut<'_, ClientResult<Value>>;
    /// `models.discover`: the cached list (`refresh = false`, instant) or a fresh one.
    fn models_discover(&self, provider: String, refresh: bool) -> Fut<'_, ClientResult<Value>>;
    /// `providers.test`: connection test with `api_key` (or the stored key); saves nothing.
    fn providers_test(&self, provider: String, api_key: Option<String>) -> Fut<'_, ClientResult<Value>>;
    /// `secrets.set`: stores a provider key in `secrets.toml` (write-only).
    fn secrets_set(&self, provider: String, api_key: String) -> Fut<'_, ClientResult<Value>>;
    /// `usage.get`: tokens per purpose today, budget left and cost when priced.
    fn usage_get(&self) -> Fut<'_, ClientResult<Value>>;
    fn plugins_list(&self) -> Fut<'_, ClientResult<Vec<PluginInfo>>>;
    fn plugins_start(&self, id: String) -> Fut<'_, ClientResult<()>>;
    fn plugins_stop(&self, id: String) -> Fut<'_, ClientResult<()>>;
    fn plugins_restart(&self, id: String) -> Fut<'_, ClientResult<()>>;
    /// Submits a turn; returns how many effects the owner step spawned.
    fn submit_turn(&self, percept: Percept) -> Fut<'_, ClientResult<usize>>;
    fn cancel_turn(&self) -> Fut<'_, ClientResult<bool>>;
    fn events_since(&self, from: usize) -> Fut<'_, ClientResult<EventsPage>>;
    fn resolve_approval(&self, params: ApprovalResolveParams) -> Fut<'_, ClientResult<()>>;
    fn commands(&self) -> Fut<'_, ClientResult<Vec<CommandSpec>>>;
    fn run_command(
        &self,
        raw: String,
        actor: Option<String>,
        place: Option<String>,
    ) -> Fut<'_, ClientResult<CommandReply>>;
    /// Stops what this client owns (hosting: plugins, socket, gateway; attached: the connection).
    fn shutdown(&self) -> Fut<'_, ()>;
    /// Governance and observability admin methods (`approvals.list`, `people.list`, `links.*`,
    /// `policy.get`, `events.*`, `activity.list`, `lab.*`) and `status`: the same JSON over the
    /// socket or in process. Clients that cannot reach a gateway answer an error.
    fn gov_call(&self, method: String, params: Value) -> Fut<'_, ClientResult<Value>> {
        Box::pin(async move {
            let _ = params;
            Err(ClientError::server(format!("{method}: not supported by this client")))
        })
    }
    /// Subscribes the TUI to the owner-only draft preview of its own turns (`preview.subscribe`);
    /// drafts arrive as `RuntimeEvent::Conv(ConvEvent::Preview(..))`. Returns the subscription id.
    fn preview_subscribe(&self, tx: mpsc::UnboundedSender<RuntimeEvent>) -> Fut<'_, ClientResult<u64>> {
        Box::pin(async move {
            let _ = tx;
            Err(ClientError::server("preview.subscribe: not supported by this client"))
        })
    }
    /// Ends a draft preview subscription.
    fn preview_unsubscribe(&self, id: u64) -> Fut<'_, ClientResult<()>> {
        Box::pin(async move {
            let _ = id;
            Ok(())
        })
    }
    /// Embeds `text` with the gateway's embedder (`None` without one), for chat-title topic shift.
    fn embed_text(&self, text: String) -> Fut<'_, Option<Vec<f32>>> {
        Box::pin(async move {
            let _ = text;
            None
        })
    }
}

// ---------------------------------------------------------------------------------------------
// Shared JSON decoding
// ---------------------------------------------------------------------------------------------

fn decode_events_page(v: &Value) -> ClientResult<EventsPage> {
    let events: Vec<EventRecord> = serde_json::from_value(v.get("events").cloned().unwrap_or(json!([])))
        .map_err(|e| ClientError::server(format!("bad events payload: {e}")))?;
    Ok(EventsPage {
        total: v.get("total").and_then(Value::as_u64).map_or(0, |n| n as usize),
        events,
        budget_used: v.pointer("/budget/used").and_then(Value::as_u64).unwrap_or(0),
        budget_limit: v.pointer("/budget/limit").and_then(Value::as_u64).unwrap_or(0),
        cost_usd: v.pointer("/budget/cost_usd").and_then(Value::as_f64),
    })
}

fn decode_commands(v: &Value) -> ClientResult<Vec<CommandSpec>> {
    serde_json::from_value(v.get("commands").cloned().unwrap_or(json!([])))
        .map_err(|e| ClientError::server(format!("bad commands payload: {e}")))
}

fn decode_plugins(v: &Value) -> ClientResult<Vec<PluginInfo>> {
    serde_json::from_value(v.get("plugins").cloned().unwrap_or(json!([])))
        .map_err(|e| ClientError::server(format!("bad plugins payload: {e}")))
}

fn decode_command_reply(v: &Value) -> CommandReply {
    let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or_default().to_owned();
    match v.get("outcome").and_then(Value::as_str) {
        Some("handled") => CommandReply::Handled(s("reply_text")),
        Some("denied") => CommandReply::Denied(s("reason")),
        Some("forward") => CommandReply::Forward {
            body_id: s("body_id"),
            command: s("command"),
            args: s("args"),
        },
        _ => CommandReply::Unknown,
    }
}

/// Maps a supervisor state event to the sidebar state.
#[must_use]
pub fn plugin_ui_state(state: PluginState, reason: Option<&str>, retrying: bool) -> PluginUiState {
    match state {
        PluginState::Stopped => PluginUiState::Stopped,
        PluginState::Starting => PluginUiState::Starting,
        PluginState::Running => PluginUiState::Running,
        PluginState::RunningElsewhere => PluginUiState::RunningElsewhere,
        PluginState::Failed => {
            let r = reason.unwrap_or("failed");
            PluginUiState::Failed(if retrying {
                format!("{r} (restarting)")
            } else {
                r.to_owned()
            })
        }
    }
}

fn plugin_event(ev: &PluginStateEvent) -> RuntimeEvent {
    RuntimeEvent::PluginState {
        id: ev.id.clone(),
        state: plugin_ui_state(ev.state, ev.reason.as_deref(), ev.retry_at_ms.is_some()),
    }
}

/// Initial sidebar states for [`PluginInfo`] rows.
#[must_use]
pub fn plugin_info_state(info: &PluginInfo) -> PluginUiState {
    plugin_ui_state(info.state, info.reason.as_deref(), info.retry_at_ms.is_some())
}

// ---------------------------------------------------------------------------------------------
// TUI chat body (Waist 2)
// ---------------------------------------------------------------------------------------------

/// Serves the TUI chat body over an NDJSON stream: answers `body/discover`, heartbeat,
/// shutdown and `action/execute` (`chat.speak`, approval prompts) and forwards what the user
/// must see to `tx`.
pub async fn serve_tui_body<R, W>(
    reader: R,
    mut writer: W,
    tx: mpsc::UnboundedSender<RuntimeEvent>,
    clock: Arc<dyn Clock>,
) where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut reader = TokioBufReader::new(reader);
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line).await {
            Ok(0) | Err(_) => break,
            Ok(_) => {
                if let Some(resp) = handle_tui_waist2_line(&line, &tx, clock.now())
                    && (writer.write_all(resp.as_bytes()).await.is_err()
                        || writer.write_all(b"\n").await.is_err()
                        || writer.flush().await.is_err())
                {
                    break;
                }
            }
        }
    }
}

fn tui_body_capabilities() -> Vec<CapabilityDescriptor> {
    let place = || vec![PlaceId::new(TUI_PLACE)];
    let flags = CapabilityFlags {
        communicative: true,
        identity_changing: false,
        irreversible: false,
    };
    let mut caps = vec![
        CapabilityDescriptor {
            id: CapabilityId::new("chat.speak"),
            description: "Render chat reply in interactive TUI".to_owned(),
            places: place(),
            flags,
            typical_latency_ms: 0,
            capability_schema_hash: monoagens_protocol::CapabilitySchemaHash::new("tui-chat-speak"),
            sensed_effect: None,
        },
        CapabilityDescriptor {
            id: CapabilityId::new(CAPABILITY_APPROVAL_PROMPT),
            description: "Render interactive Valve approval card in TUI".to_owned(),
            places: place(),
            flags,
            typical_latency_ms: 0,
            capability_schema_hash: monoagens_protocol::CapabilitySchemaHash::new(
                "tui-approval-prompt",
            ),
            sensed_effect: None,
        },
    ];
    sort_capabilities(&mut caps);
    caps
}

/// Answers one Waist-2 request line from the gateway; returns the response line to write.
#[must_use]
pub fn handle_tui_waist2_line(
    line: &str,
    tx: &mpsc::UnboundedSender<RuntimeEvent>,
    now: Time,
) -> Option<String> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    let req: Waist2Request<Value> = serde_json::from_str(trimmed).ok()?;
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
            let res = BodyDiscoverResult {
                body_id: params.body_id.unwrap_or_else(|| BodyId::new(TUI_BODY_ID)),
                protocol_version: CURRENT_PROTOCOL_VERSION,
                capabilities: tui_body_capabilities(),
                places: vec![PlaceId::new(TUI_PLACE)],
            };
            serde_json::to_value(res).ok()?
        }
        Waist2Method::BodyHeartbeat => serde_json::to_value(BodyHeartbeatResult {
            acknowledged_at_ms: now,
        })
        .ok()?,
        Waist2Method::BodyShutdown => {
            serde_json::to_value(BodyShutdownResult { accepted: true }).ok()?
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
                    chat_id: None,
                    text,
                    duration_ms: 0,
                    action_id: Some(params.action_id.as_str().to_owned()),
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
            serde_json::to_value(ActionExecuteResult {
                accepted: true,
                receipt: Some(receipt),
            })
            .ok()?
        }
        _ => {
            return serialize_waist2_err(
                id,
                Waist2Error::invalid_receipt_transition("unsupported method"),
            );
        }
    };

    serde_json::to_string(&Waist2Response::ok(Some(id), result_val)).ok()
}

fn serialize_waist2_err(id: RpcId, err: Waist2Error) -> Option<String> {
    serde_json::to_string(&Waist2Response::<Value>::err(Some(id), err)).ok()
}

fn extract_chat_speak_text(bytes: &[u8]) -> String {
    if let Ok(val) = serde_json::from_slice::<Value>(bytes)
        && let Some(t) = val.get("text").and_then(|v| v.as_str())
    {
        return t.to_owned();
    }
    String::from_utf8_lossy(bytes).to_string()
}

// ---------------------------------------------------------------------------------------------
// In-process client
// ---------------------------------------------------------------------------------------------

/// Everything [`InProcessClient::new`] needs (built by the boot code in `runtime.rs`).
pub struct InProcessParts {
    pub owner: Arc<GatewayOwner>,
    pub runtime: GatewayRuntime,
    pub lock: DataDirLock,
    pub admin: Option<AdminServer>,
    pub owner_actor: ActorId,
    pub mac_key: [u8; 32],
    pub clock: Arc<SystemClock>,
    pub dynamic_actors: Arc<Mutex<BTreeSet<ActorId>>>,
    pub embed_sidecar: Option<LocalEmbedSidecar>,
}

/// The TUI hosts the gateway: direct calls into the [`GatewayOwner`].
pub struct InProcessClient {
    owner: Arc<GatewayOwner>,
    handle: GatewayHandle,
    runtime: tokio::sync::Mutex<Option<GatewayRuntime>>,
    admin: Mutex<Option<AdminServer>>,
    _lock: DataDirLock,
    owner_actor: ActorId,
    mac_key: [u8; 32],
    clock: Arc<SystemClock>,
    dynamic_actors: Arc<Mutex<BTreeSet<ActorId>>>,
    tasks: Mutex<Vec<JoinHandle<()>>>,
    /// Forwarders of draft preview subscriptions, by subscription id.
    preview_tasks: Mutex<HashMap<u64, JoinHandle<()>>>,
    _embed_sidecar: Option<LocalEmbedSidecar>,
}

impl InProcessClient {
    /// Wires the chat body (over an in-memory duplex stream) and the forwarders for plugin
    /// state, log entries and cues into `tx`.
    pub async fn new(
        parts: InProcessParts,
        tx: mpsc::UnboundedSender<RuntimeEvent>,
    ) -> Result<Self, String> {
        let handle = parts.runtime.handle();
        let (gw_stream, body_stream) = tokio::io::duplex(64 * 1024);
        let (gw_read, gw_write) = tokio::io::split(gw_stream);
        let (body_read, body_write) = tokio::io::split(body_stream);
        let clock_dyn: Arc<dyn Clock> = parts.clock.clone();
        let mut tasks = vec![tokio::spawn(serve_tui_body(
            body_read,
            body_write,
            tx.clone(),
            clock_dyn,
        ))];
        handle
            .connect_ndjson_body(gw_read, gw_write, Some(BodyId::new(TUI_BODY_ID)))
            .await
            .map_err(|e| format!("failed to connect body-tui over Waist 2: {e}"))?;

        // Forward plugin state, log entries and cues.
        let mut plugin_rx = parts.owner.supervisor().subscribe();
        let txp = tx.clone();
        tasks.push(tokio::spawn(async move {
            loop {
                match plugin_rx.recv().await {
                    Ok(ev) => {
                        if txp.send(plugin_event(&ev)).is_err() {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        }));
        let mut log_rx = parts.owner.runlog().subscribe();
        let txl = tx.clone();
        tasks.push(tokio::spawn(async move {
            loop {
                match log_rx.recv().await {
                    Ok(entry) => {
                        if txl.send(RuntimeEvent::Log(entry)).is_err() {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        }));
        let mut cue_rx = handle.cues().subscribe();
        let txc = tx;
        tasks.push(tokio::spawn(async move {
            while let Some(cue) = cue_rx.recv().await {
                if txc.send(RuntimeEvent::Cue(cue)).is_err() {
                    break;
                }
            }
        }));

        Ok(Self {
            owner: parts.owner,
            handle,
            runtime: tokio::sync::Mutex::new(Some(parts.runtime)),
            admin: Mutex::new(parts.admin),
            _lock: parts.lock,
            owner_actor: parts.owner_actor,
            mac_key: parts.mac_key,
            clock: parts.clock,
            dynamic_actors: parts.dynamic_actors,
            tasks: Mutex::new(tasks),
            preview_tasks: Mutex::new(HashMap::new()),
            _embed_sidecar: parts.embed_sidecar,
        })
    }

    /// The owner behind this client (tests and boot code).
    #[must_use]
    pub fn owner(&self) -> &Arc<GatewayOwner> {
        &self.owner
    }

    /// The gateway handle (tests).
    #[must_use]
    pub fn handle(&self) -> &GatewayHandle {
        &self.handle
    }
}

impl GatewayClient for InProcessClient {
    fn kind(&self) -> ClientKind {
        ClientKind::Hosting
    }

    fn owner_actor(&self) -> ActorId {
        self.owner_actor.clone()
    }

    fn now(&self) -> Time {
        self.clock.now()
    }

    fn approval_mac_key(&self) -> [u8; 32] {
        self.mac_key
    }

    fn titler_route(&self) -> Option<(Arc<dyn ModelProvider>, String)> {
        Some(self.handle.live_model().route_for(monoagens_core::ModelPurpose::Classify))
    }

    fn register_actors(&self, actors: &[ActorId]) {
        let mut guard = self.dynamic_actors.lock().expect("dynamic actors lock");
        for a in actors {
            guard.insert(a.clone());
        }
    }

    fn chat_unavailable_reason(&self) -> Option<String> {
        None
    }

    fn log(&self, level: RunLogLevel, source: &str, message: &str) {
        self.owner.runlog().log(level, source, message);
    }

    fn status(&self) -> Fut<'_, ClientResult<Value>> {
        Box::pin(async move { Ok(self.owner.status()) })
    }

    fn config_get(&self) -> Fut<'_, ClientResult<Value>> {
        Box::pin(async move { self.owner.config_get() })
    }

    fn config_apply(&self, patch: Value) -> Fut<'_, ClientResult<Value>> {
        Box::pin(async move {
            let report = self.owner.config_apply(&patch).await?;
            Ok(json!({ "ok": true, "reloaded": report }))
        })
    }

    fn providers_list(&self) -> Fut<'_, ClientResult<Value>> {
        Box::pin(async move { self.owner.providers_list() })
    }

    fn models_discover(&self, provider: String, refresh: bool) -> Fut<'_, ClientResult<Value>> {
        Box::pin(async move { self.owner.models_discover(&provider, refresh).await })
    }

    fn providers_test(&self, provider: String, api_key: Option<String>) -> Fut<'_, ClientResult<Value>> {
        Box::pin(async move { self.owner.providers_test(&provider, api_key.as_deref()).await })
    }

    fn secrets_set(&self, provider: String, api_key: String) -> Fut<'_, ClientResult<Value>> {
        Box::pin(async move { self.owner.secrets_set(&provider, &api_key).await })
    }

    fn usage_get(&self) -> Fut<'_, ClientResult<Value>> {
        Box::pin(async move { Ok(self.owner.usage_get()) })
    }

    fn plugins_list(&self) -> Fut<'_, ClientResult<Vec<PluginInfo>>> {
        Box::pin(async move { Ok(self.owner.supervisor().list()) })
    }

    fn plugins_start(&self, id: String) -> Fut<'_, ClientResult<()>> {
        Box::pin(async move { self.owner.plugins_start(&id).await })
    }

    fn plugins_stop(&self, id: String) -> Fut<'_, ClientResult<()>> {
        Box::pin(async move { self.owner.plugins_stop(&id).await })
    }

    fn plugins_restart(&self, id: String) -> Fut<'_, ClientResult<()>> {
        Box::pin(async move { self.owner.plugins_restart(&id).await })
    }

    fn submit_turn(&self, percept: Percept) -> Fut<'_, ClientResult<usize>> {
        Box::pin(async move {
            let v = self.owner.turn_submit_percept(percept).await?;
            Ok(v.get("effects_spawned").and_then(Value::as_u64).map_or(0, |n| n as usize))
        })
    }

    fn cancel_turn(&self) -> Fut<'_, ClientResult<bool>> {
        Box::pin(async move {
            Ok(self
                .owner
                .turn_cancel()
                .get("cancelled")
                .and_then(Value::as_bool)
                .unwrap_or(false))
        })
    }

    fn events_since(&self, from: usize) -> Fut<'_, ClientResult<EventsPage>> {
        Box::pin(async move { decode_events_page(&self.owner.events_since(from).await?) })
    }

    fn resolve_approval(&self, params: ApprovalResolveParams) -> Fut<'_, ClientResult<()>> {
        Box::pin(async move { self.owner.approvals_resolve(params).await.map(|_| ()) })
    }

    fn commands(&self) -> Fut<'_, ClientResult<Vec<CommandSpec>>> {
        Box::pin(async move { decode_commands(&self.owner.commands_list()) })
    }

    fn run_command(
        &self,
        raw: String,
        actor: Option<String>,
        place: Option<String>,
    ) -> Fut<'_, ClientResult<CommandReply>> {
        Box::pin(async move {
            Ok(decode_command_reply(&self.owner.command_run(
                &raw,
                actor.as_deref(),
                place.as_deref(),
            )))
        })
    }

    fn gov_call(&self, method: String, params: Value) -> Fut<'_, ClientResult<Value>> {
        Box::pin(async move {
            if method == "status" {
                return Ok(self.owner.status());
            }
            monoagens_gateway::governance::governance_call(&self.owner, &method, &params).await
        })
    }

    fn preview_subscribe(&self, tx: mpsc::UnboundedSender<RuntimeEvent>) -> Fut<'_, ClientResult<u64>> {
        Box::pin(async move {
            let mut sub = self.handle.preview().subscribe();
            let id = sub.id();
            let task = tokio::spawn(async move {
                while let Some(ev) = sub.recv().await {
                    if tx.send(RuntimeEvent::Conv(crate::conv::ConvEvent::Preview(ev))).is_err() {
                        break;
                    }
                }
            });
            self.preview_tasks.lock().expect("preview tasks lock").insert(id, task);
            Ok(id)
        })
    }

    fn preview_unsubscribe(&self, id: u64) -> Fut<'_, ClientResult<()>> {
        Box::pin(async move {
            // Aborting the forwarder drops the subscription, which removes it from the hub.
            if let Some(t) = self.preview_tasks.lock().expect("preview tasks lock").remove(&id) {
                t.abort();
            }
            self.handle.preview().unsubscribe(id);
            Ok(())
        })
    }

    fn embed_text(&self, text: String) -> Fut<'_, Option<Vec<f32>>> {
        Box::pin(async move {
            let pq = self.handle.pre_queue_service()?.clone();
            tokio::task::spawn_blocking(move || pq.embed_text_blocking(&text))
                .await
                .ok()
                .flatten()
        })
    }

    fn shutdown(&self) -> Fut<'_, ()> {
        Box::pin(async move {
            self.owner.supervisor().shutdown().await;
            if let Some(admin) = self.admin.lock().expect("admin lock").take() {
                admin.shutdown();
            }
            for t in self.tasks.lock().expect("tasks lock").drain(..) {
                t.abort();
            }
            if let Some(rt) = self.runtime.lock().await.take() {
                rt.shutdown().await;
            }
        })
    }
}

// ---------------------------------------------------------------------------------------------
// Socket client
// ---------------------------------------------------------------------------------------------

type Pending = Arc<Mutex<HashMap<i64, oneshot::Sender<ClientResult<Value>>>>>;

/// Attached to the lock holder over `<data_dir>/gateway.sock`.
pub struct SocketClient {
    writer: tokio::sync::Mutex<OwnedWriteHalf>,
    pending: Pending,
    next_id: AtomicI64,
    owner_actor: ActorId,
    mac_key: [u8; 32],
    clock: Arc<SystemClock>,
    local_log: RunLog,
    chat_error: Option<String>,
    tasks: Mutex<Vec<JoinHandle<()>>>,
}

impl SocketClient {
    /// Connects to the admin socket in `data_dir`, subscribes to logs, plugin state and cues
    /// and attaches the chat body. A refused body attach does not fail the connection; it is
    /// reported by [`GatewayClient::chat_unavailable_reason`].
    pub async fn connect(
        data_dir: &Path,
        owner_actor: ActorId,
        mac_key: [u8; 32],
        secrets: &ProviderSecrets,
        tx: mpsc::UnboundedSender<RuntimeEvent>,
    ) -> Result<Self, String> {
        let path = admin_socket_path(data_dir);
        let stream = UnixStream::connect(&path)
            .await
            .map_err(|e| format!("cannot reach the gateway owner at '{}': {e}", path.display()))?;
        let (rd, wr) = stream.into_split();
        let pending: Pending = Arc::new(Mutex::new(HashMap::new()));
        let clock = Arc::new(SystemClock::new());
        let clock_dyn: Arc<dyn Clock> = clock.clone();

        let local_log = RunLog::new(
            Arc::clone(&clock_dyn),
            DiagnosticSink::new(secrets),
            Some(data_dir.join("logs").join("tui.log")),
        );

        let mut tasks = Vec::new();
        // Reader: demultiplex responses and pushed notifications.
        let pending_r = Arc::clone(&pending);
        let txr = tx.clone();
        tasks.push(tokio::spawn(async move {
            let mut reader = TokioBufReader::new(rd);
            let mut line = String::new();
            loop {
                line.clear();
                match (&mut reader).take(4 * 1024 * 1024).read_line(&mut line).await {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {}
                }
                let Ok(v) = serde_json::from_str::<Value>(line.trim()) else {
                    continue;
                };
                if let Some(method) = v.get("method").and_then(Value::as_str) {
                    let params = v.get("params").cloned().unwrap_or(Value::Null);
                    let ev = match method {
                        "logs.entry" => serde_json::from_value(params).ok().map(RuntimeEvent::Log),
                        "plugins.state" => serde_json::from_value::<PluginStateEvent>(params)
                            .ok()
                            .map(|e| plugin_event(&e)),
                        "cues.event" => serde_json::from_value(params).ok().map(RuntimeEvent::Cue),
                        "preview.event" => serde_json::from_value::<PreviewEvent>(params)
                            .ok()
                            .map(|e| RuntimeEvent::Conv(crate::conv::ConvEvent::Preview(e))),
                        _ => None,
                    };
                    if let Some(ev) = ev {
                        let _ = txr.send(ev);
                    }
                    continue;
                }
                let Some(id) = v.get("id").and_then(Value::as_i64) else {
                    continue;
                };
                let result = if let Some(err) = v.get("error") {
                    Err(ClientError::new(
                        err.get("code")
                            .and_then(Value::as_i64)
                            .and_then(|c| i32::try_from(c).ok())
                            .unwrap_or(-32000),
                        err.get("message").and_then(Value::as_str).unwrap_or(""),
                    ))
                } else {
                    Ok(v.get("result").cloned().unwrap_or(Value::Null))
                };
                if let Some(tx) = pending_r.lock().expect("pending lock").remove(&id) {
                    let _ = tx.send(result);
                }
            }
            // Owner gone: fail everything still waiting and tell the event loop.
            let drained: Vec<_> = pending_r.lock().expect("pending lock").drain().collect();
            for (_, tx) in drained {
                let _ = tx.send(Err(ClientError::server("gateway owner went away")));
            }
            let _ = txr.send(RuntimeEvent::OwnerGone);
        }));

        // Local log entries are pushed to the overlay like remote ones.
        let mut local_rx = local_log.subscribe();
        let txl = tx.clone();
        tasks.push(tokio::spawn(async move {
            loop {
                match local_rx.recv().await {
                    Ok(entry) => {
                        if txl.send(RuntimeEvent::Log(entry)).is_err() {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        }));

        let client = Self {
            writer: tokio::sync::Mutex::new(wr),
            pending,
            next_id: AtomicI64::new(1),
            owner_actor,
            mac_key,
            clock: clock.clone(),
            local_log,
            chat_error: None,
            tasks: Mutex::new(tasks),
        };

        // Subscriptions: the response arrives first; entries stream afterwards.
        client.call("logs.subscribe", json!({ "backlog": 500 })).await.map_err(|e| e.to_string())?;
        client.call("plugins.subscribe", json!({})).await.map_err(|e| e.to_string())?;
        client.call("cues.subscribe", json!({})).await.map_err(|e| e.to_string())?;

        // Attach the chat body on its own connection.
        let chat_error = match Self::attach_body(&path, tx, clock_dyn).await {
            Ok(task) => {
                client.tasks.lock().expect("tasks lock").push(task);
                None
            }
            Err(e) => Some(e),
        };
        Ok(Self { chat_error, ..client })
    }

    async fn attach_body(
        path: &Path,
        tx: mpsc::UnboundedSender<RuntimeEvent>,
        clock: Arc<dyn Clock>,
    ) -> Result<JoinHandle<()>, String> {
        let stream = UnixStream::connect(path)
            .await
            .map_err(|e| format!("body attach: {e}"))?;
        let (rd, mut wr) = stream.into_split();
        let req = json!({
            "jsonrpc": "2.0", "id": 1, "method": "body.attach",
            "params": { "body_id": TUI_BODY_ID, "places": [TUI_PLACE] },
        });
        wr.write_all(format!("{req}\n").as_bytes())
            .await
            .map_err(|e| format!("body attach: {e}"))?;
        let mut reader = TokioBufReader::new(rd);
        let mut line = String::new();
        reader
            .read_line(&mut line)
            .await
            .map_err(|e| format!("body attach: {e}"))?;
        let v: Value = serde_json::from_str(line.trim()).map_err(|e| format!("body attach: {e}"))?;
        if let Some(err) = v.get("error") {
            return Err(err
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("body attach refused")
                .to_owned());
        }
        Ok(tokio::spawn(async move {
            serve_tui_body(reader, wr, tx.clone(), clock).await;
            let _ = tx.send(RuntimeEvent::OwnerGone);
        }))
    }

    /// Calls an admin method.
    pub async fn call(&self, method: &str, params: Value) -> ClientResult<Value> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().expect("pending lock").insert(id, tx);
        let req = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        {
            let mut w = self.writer.lock().await;
            let io = |e: std::io::Error| ClientError::server(format!("admin io: {e}"));
            w.write_all(format!("{req}\n").as_bytes()).await.map_err(io)?;
            w.flush().await.map_err(io)?;
        }
        rx.await
            .unwrap_or_else(|_| Err(ClientError::server("gateway owner went away")))
    }
}

impl GatewayClient for SocketClient {
    fn kind(&self) -> ClientKind {
        ClientKind::Attached
    }

    fn owner_actor(&self) -> ActorId {
        self.owner_actor.clone()
    }

    fn now(&self) -> Time {
        self.clock.now()
    }

    fn approval_mac_key(&self) -> [u8; 32] {
        self.mac_key
    }

    fn titler_route(&self) -> Option<(Arc<dyn ModelProvider>, String)> {
        None
    }

    fn register_actors(&self, _actors: &[ActorId]) {}

    fn chat_unavailable_reason(&self) -> Option<String> {
        self.chat_error.clone()
    }

    fn log(&self, level: RunLogLevel, source: &str, message: &str) {
        self.local_log.log(level, source, message);
    }

    fn status(&self) -> Fut<'_, ClientResult<Value>> {
        Box::pin(async move { self.call("status", json!({})).await })
    }

    fn config_get(&self) -> Fut<'_, ClientResult<Value>> {
        Box::pin(async move { self.call("config.get", json!({})).await })
    }

    fn config_apply(&self, patch: Value) -> Fut<'_, ClientResult<Value>> {
        Box::pin(async move { self.call("config.apply", json!({ "patch": patch })).await })
    }

    fn providers_list(&self) -> Fut<'_, ClientResult<Value>> {
        Box::pin(async move { self.call("providers.list", json!({})).await })
    }

    fn models_discover(&self, provider: String, refresh: bool) -> Fut<'_, ClientResult<Value>> {
        Box::pin(async move {
            self.call("models.discover", json!({ "provider": provider, "refresh": refresh }))
                .await
        })
    }

    fn providers_test(&self, provider: String, api_key: Option<String>) -> Fut<'_, ClientResult<Value>> {
        Box::pin(async move {
            self.call("providers.test", json!({ "provider": provider, "api_key": api_key }))
                .await
        })
    }

    fn secrets_set(&self, provider: String, api_key: String) -> Fut<'_, ClientResult<Value>> {
        Box::pin(async move {
            self.call("secrets.set", json!({ "provider": provider, "api_key": api_key }))
                .await
        })
    }

    fn usage_get(&self) -> Fut<'_, ClientResult<Value>> {
        Box::pin(async move { self.call("usage.get", json!({})).await })
    }

    fn plugins_list(&self) -> Fut<'_, ClientResult<Vec<PluginInfo>>> {
        Box::pin(async move { decode_plugins(&self.call("plugins.list", json!({})).await?) })
    }

    fn plugins_start(&self, id: String) -> Fut<'_, ClientResult<()>> {
        Box::pin(async move { self.call("plugins.start", json!({ "id": id })).await.map(|_| ()) })
    }

    fn plugins_stop(&self, id: String) -> Fut<'_, ClientResult<()>> {
        Box::pin(async move { self.call("plugins.stop", json!({ "id": id })).await.map(|_| ()) })
    }

    fn plugins_restart(&self, id: String) -> Fut<'_, ClientResult<()>> {
        Box::pin(async move {
            self.call("plugins.restart", json!({ "id": id })).await.map(|_| ())
        })
    }

    fn submit_turn(&self, percept: Percept) -> Fut<'_, ClientResult<usize>> {
        Box::pin(async move {
            let p = serde_json::to_value(&percept)
                .map_err(|e| ClientError::server(format!("failed to encode percept: {e}")))?;
            let v = self.call("turn.submit", json!({ "percept": p })).await?;
            Ok(v.get("effects_spawned").and_then(Value::as_u64).map_or(0, |n| n as usize))
        })
    }

    fn cancel_turn(&self) -> Fut<'_, ClientResult<bool>> {
        Box::pin(async move {
            Ok(self
                .call("turn.cancel", json!({}))
                .await?
                .get("cancelled")
                .and_then(Value::as_bool)
                .unwrap_or(false))
        })
    }

    fn events_since(&self, from: usize) -> Fut<'_, ClientResult<EventsPage>> {
        Box::pin(async move {
            decode_events_page(&self.call("events.since", json!({ "from": from })).await?)
        })
    }

    fn resolve_approval(&self, params: ApprovalResolveParams) -> Fut<'_, ClientResult<()>> {
        Box::pin(async move {
            let p = serde_json::to_value(&params)
                .map_err(|e| ClientError::server(format!("failed to encode approval: {e}")))?;
            self.call("approvals.resolve", p).await.map(|_| ())
        })
    }

    fn commands(&self) -> Fut<'_, ClientResult<Vec<CommandSpec>>> {
        Box::pin(async move { decode_commands(&self.call("commands.list", json!({})).await?) })
    }

    fn run_command(
        &self,
        raw: String,
        actor: Option<String>,
        place: Option<String>,
    ) -> Fut<'_, ClientResult<CommandReply>> {
        Box::pin(async move {
            Ok(decode_command_reply(
                &self
                    .call("commands.run", json!({ "text": raw, "actor": actor, "place": place }))
                    .await?,
            ))
        })
    }

    fn gov_call(&self, method: String, params: Value) -> Fut<'_, ClientResult<Value>> {
        Box::pin(async move { self.call(&method, params).await })
    }

    fn preview_subscribe(&self, tx: mpsc::UnboundedSender<RuntimeEvent>) -> Fut<'_, ClientResult<u64>> {
        Box::pin(async move {
            // Drafts arrive on this connection as `preview.event` notifications (see the reader).
            let _ = tx;
            let v = self.call("preview.subscribe", json!({})).await?;
            v.get("subscription")
                .and_then(Value::as_u64)
                .ok_or_else(|| ClientError::server("preview.subscribe: no subscription id"))
        })
    }

    fn preview_unsubscribe(&self, id: u64) -> Fut<'_, ClientResult<()>> {
        Box::pin(async move {
            self.call("preview.unsubscribe", json!({ "subscription": id })).await.map(|_| ())
        })
    }

    fn shutdown(&self) -> Fut<'_, ()> {
        Box::pin(async move {
            for t in self.tasks.lock().expect("tasks lock").drain(..) {
                t.abort();
            }
        })
    }
}
