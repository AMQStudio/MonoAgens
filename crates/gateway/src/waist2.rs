//! Waist-2 (`monoagens.body.v1`) NDJSON transport, body routing, and single-receipt
//! action settlement (`R-36`, `R-45`, `R-50`).

use core::fmt;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use monoagens_core::Input;
use monoagens_protocol::{
    Action, ActionCancelParams, ActionCancelResult, ActionExecuteParams, ActionExecuteResult,
    ActionId, ActionReceiptParams, ActionReceiptResult, ApprovalResolveParams,
    ApprovalResolveResult, BlobGetParams, BlobPutParams, BodyDiscoverParams, BodyDiscoverResult,
    BodyHeartbeatParams, BodyHeartbeatResult, BodyId, BodyState, BodyStatus,
    CURRENT_PROTOCOL_VERSION, CapabilityDescriptor, CapabilityId, MAX_INLINE_ATTACHMENT_BYTES,
    PerceptEmitParams, PerceptEmitResult, PlaceId, Receipt, ReceiptOutcome, RpcId,
    StateQueryParams, StateQueryResult, TerminationReason, Time, Waist2Error, Waist2Headers,
    Waist2Method, Waist2Request, Waist2Response, check_version, sort_capabilities,
};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::process::{Child, Command};
use tokio::sync::{Notify, mpsc, oneshot};

use crate::clock::{Clock, run_with_clock_timeout};
use crate::commands::{CommandRegistry, CommandSpec};
use crate::percept_commands::{self, CommandContext};
use crate::cues::{CueRecorder, WaitCueGuard, spawn_wait_cue_monitor};
use crate::logging::{DiagnosticSink, LogLevel};
use crate::runtime::{ControlCommand, IngressQueue};
use crate::seams::GatewaySeams;

/// Capability ID for generic adapter-rendered approval prompts (`approval.prompt` -> `approval/resolve`).
pub const CAPABILITY_APPROVAL_PROMPT: &str = "approval.prompt";

/// Generic payload dispatched via `action/execute` to any adapter declaring capability `approval.prompt`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ApprovalPromptRequest {
    pub action_id: ActionId,
    pub h_act: monoagens_protocol::HAct,
    pub state_epoch: u64,
    pub capability_id: CapabilityId,
    pub target_places: Vec<PlaceId>,
    pub summary: String,
    pub expires_at_ms: Time,
}

const RPC_METHOD_NOT_FOUND: i32 = -32601;
const RPC_INVALID_PARAMS: i32 = -32602;
const RPC_PARSE_ERROR: i32 = -32700;
const RPC_BACKPRESSURE: i32 = -32099;

/// Key used to correlate outbound JSON-RPC request `id`s with responses.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum RpcKey {
    Int(i64),
    Str(String),
}

type PendingRpcTable = Mutex<HashMap<RpcKey, oneshot::Sender<Result<Value, Waist2Error>>>>;
type PendingRpcMap = Arc<PendingRpcTable>;

impl From<&RpcId> for RpcKey {
    fn from(id: &RpcId) -> Self {
        match id {
            RpcId::Int(n) => Self::Int(*n),
            RpcId::Str(s) => Self::Str(s.clone()),
        }
    }
}

/// Errors surfaced by Waist-2 body connection / RPC operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BodyRpcError {
    Transport(String),
    Serialization(String),
    Timeout { timeout_ms: u64 },
    Protocol(Waist2Error),
    Disconnected,
}

impl fmt::Display for BodyRpcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Transport(detail) => write!(f, "body transport error: {detail}"),
            Self::Serialization(detail) => write!(f, "body JSON-RPC serialization error: {detail}"),
            Self::Timeout { timeout_ms } => write!(f, "body RPC timed out after {timeout_ms}ms"),
            Self::Protocol(err) => write!(f, "body protocol error: {err}"),
            Self::Disconnected => f.write_str("body disconnected"),
        }
    }
}

impl std::error::Error for BodyRpcError {}

#[derive(Debug)]
struct PendingActionEntry {
    body_id: BodyId,
    capability_id: CapabilityId,
    h_act: Option<monoagens_protocol::HAct>,
    cue_guard: Option<WaitCueGuard>,
    settled_notify: Arc<Notify>,
}

/// Outcome of trying to settle an action's single terminal/held receipt.
#[derive(Debug, Clone, PartialEq)]
pub enum SettlementOutcome {
    SettledFirst(Receipt),
    AlreadySettledOrUnknown,
}

/// Guarantees that every dispatched `ActionId` settles in Core AT MOST ONCE (`receipt_cardinality == 1`).
#[derive(Debug, Clone, Default)]
pub struct ActionSettlementTracker {
    state: Arc<Mutex<SettlementState>>,
}

#[derive(Debug, Default)]
struct SettlementState {
    pending: BTreeMap<ActionId, PendingActionEntry>,
    settled: BTreeMap<ActionId, Receipt>,
}

impl ActionSettlementTracker {
    /// Creates an empty [`ActionSettlementTracker`].
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers an outbound action as in-flight and returns a [`Notify`] signalled upon settlement.
    pub fn register_pending(
        &self,
        action: &Action,
        body_id: BodyId,
        cue_guard: Option<WaitCueGuard>,
    ) -> Arc<Notify> {
        let notify = Arc::new(Notify::new());
        let mut st = self.state.lock().expect("settlement lock");
        st.pending.insert(
            action.action_id.clone(),
            PendingActionEntry {
                body_id,
                capability_id: action.capability_id.clone(),
                h_act: action.h_act.clone(),
                cue_guard,
                settled_notify: Arc::clone(&notify),
            },
        );
        notify
    }

    /// Attempts to settle `action_id` with `receipt`. Only the first settlement succeeds;
    /// duplicate or post-timeout receipts return [`SettlementOutcome::AlreadySettledOrUnknown`].
    pub fn try_settle(&self, action_id: &ActionId, receipt: Receipt) -> SettlementOutcome {
        let mut st = self.state.lock().expect("settlement lock");
        if st.settled.contains_key(action_id) {
            return SettlementOutcome::AlreadySettledOrUnknown;
        }
        let Some(mut entry) = st.pending.remove(action_id) else {
            return SettlementOutcome::AlreadySettledOrUnknown;
        };
        if let Some(guard) = entry.cue_guard.take() {
            guard.finish();
        }
        st.settled.insert(action_id.clone(), receipt.clone());
        entry.settled_notify.notify_waiters();
        SettlementOutcome::SettledFirst(receipt)
    }

    /// Returns `true` if `action_id` has already settled.
    #[must_use]
    pub fn is_settled(&self, action_id: &ActionId) -> bool {
        self.state
            .lock()
            .expect("settlement lock")
            .settled
            .contains_key(action_id)
    }

    /// Settles all pending actions belonging to `body_id` ONCE as `ReceiptOutcome::Unknown`
    /// when the body crashes or disconnects mid-action.
    pub fn settle_crashed_body(&self, body_id: &BodyId, now: Time, reason: &str) -> Vec<Receipt> {
        let mut st = self.state.lock().expect("settlement lock");
        let target_ids: Vec<ActionId> = st
            .pending
            .iter()
            .filter(|(_, entry)| &entry.body_id == body_id)
            .map(|(id, _)| id.clone())
            .collect();

        let mut receipts = Vec::with_capacity(target_ids.len());
        for action_id in target_ids {
            if let Some(mut entry) = st.pending.remove(&action_id) {
                if let Some(guard) = entry.cue_guard.take() {
                    guard.finish();
                }
                let receipt = make_terminal_receipt(
                    action_id.clone(),
                    entry.capability_id,
                    entry.h_act,
                    ReceiptOutcome::Unknown,
                    Some("ERR_BODY_CRASHED".to_owned()),
                    reason.to_owned(),
                    TerminationReason::Normal,
                    now,
                );
                st.settled.insert(action_id, receipt.clone());
                entry.settled_notify.notify_waiters();
                receipts.push(receipt);
            }
        }
        receipts
    }

    /// Builds a `Receipt(Unknown, TimedOut)` for an in-flight action if it has not yet settled.
    pub fn try_settle_timeout(
        &self,
        action: &Action,
        timeout_ms: u64,
        now: Time,
    ) -> Option<Receipt> {
        let receipt = make_terminal_receipt(
            action.action_id.clone(),
            action.capability_id.clone(),
            action.h_act.clone(),
            ReceiptOutcome::Unknown,
            Some("timed_out".to_owned()),
            format!("action timed out after {timeout_ms}ms"),
            TerminationReason::TimedOut,
            now,
        );
        match self.try_settle(&action.action_id, receipt) {
            SettlementOutcome::SettledFirst(r) => Some(r),
            SettlementOutcome::AlreadySettledOrUnknown => None,
        }
    }
}

/// Live handle to a connected Waist-2 body over NDJSON transport.
#[derive(Clone)]
pub struct BodyConnection {
    body_id: BodyId,
    places: Vec<PlaceId>,
    capabilities: Vec<CapabilityDescriptor>,
    outbound_tx: mpsc::UnboundedSender<String>,
    pending_rpcs: PendingRpcMap,
    next_rpc_id: Arc<AtomicI64>,
    alive: Arc<AtomicBool>,
}

impl BodyConnection {
    /// Returns the discovered [`BodyId`].
    #[must_use]
    pub fn body_id(&self) -> &BodyId {
        &self.body_id
    }

    /// Returns the places owned by this body.
    #[must_use]
    pub fn places(&self) -> &[PlaceId] {
        &self.places
    }

    /// Returns the sorted capabilities advertised by this body.
    #[must_use]
    pub fn capabilities(&self) -> &[CapabilityDescriptor] {
        &self.capabilities
    }

    /// Returns `true` if the underlying NDJSON stream has not disconnected or crashed.
    #[must_use]
    pub fn is_alive(&self) -> bool {
        self.alive.load(Ordering::SeqCst)
    }

    /// Sends a typed Waist-2 RPC request and awaits its response under `timeout_ms`.
    pub async fn call<P: Serialize, R: DeserializeOwned>(
        &self,
        method: Waist2Method,
        params: P,
        timeout_ms: u64,
        clock: &dyn Clock,
    ) -> Result<R, BodyRpcError> {
        if !self.is_alive() {
            return Err(BodyRpcError::Disconnected);
        }
        let id_num = self.next_rpc_id.fetch_add(1, Ordering::SeqCst);
        let rpc_id = RpcId::Int(id_num);
        let req = Waist2Request::new(Some(rpc_id.clone()), method, params);
        let line =
            serde_json::to_string(&req).map_err(|e| BodyRpcError::Serialization(e.to_string()))?;

        let (tx, rx) = oneshot::channel();
        {
            let mut pending = self.pending_rpcs.lock().expect("pending_rpcs lock");
            pending.insert(RpcKey::Int(id_num), tx);
        }

        if self.outbound_tx.send(line).is_err() {
            self.pending_rpcs
                .lock()
                .expect("pending_rpcs lock")
                .remove(&RpcKey::Int(id_num));
            return Err(BodyRpcError::Disconnected);
        }

        let res = match run_with_clock_timeout(clock, timeout_ms.max(1), rx).await {
            Err(_timed_out) => {
                self.pending_rpcs
                    .lock()
                    .expect("pending_rpcs lock")
                    .remove(&RpcKey::Int(id_num));
                return Err(BodyRpcError::Timeout { timeout_ms });
            }
            Ok(Err(_closed)) => return Err(BodyRpcError::Disconnected),
            Ok(Ok(val_res)) => val_res,
        };

        match res {
            Ok(val) => serde_json::from_value::<R>(val)
                .map_err(|e| BodyRpcError::Serialization(e.to_string())),
            Err(err) => Err(BodyRpcError::Protocol(err)),
        }
    }

    /// Sends `body/shutdown` best-effort and closes the outbound channel.
    pub async fn shutdown(&self, reason: Option<String>, timeout_ms: u64, clock: &dyn Clock) {
        if self.is_alive() {
            let params = monoagens_protocol::BodyShutdownParams {
                headers: Waist2Headers::new(format!("shutdown-{}", self.body_id)),
                body_id: self.body_id.clone(),
                reason,
            };
            let _: Result<monoagens_protocol::BodyShutdownResult, _> = self
                .call(Waist2Method::BodyShutdown, params, timeout_ms, clock)
                .await;
        }
        self.alive.store(false, Ordering::SeqCst);
    }
}

/// Gateway registry routing `PlaceId -> BodyConnection` and dispatching bounded actions.
#[derive(Clone)]
pub struct BodyRouter {
    inner: Arc<Mutex<RouterState>>,
    settlement: ActionSettlementTracker,
    commands: Arc<Mutex<CommandRegistry>>,
    command_ctx: Arc<RwLock<CommandContext>>,
    ingress: IngressQueue,
    seams: GatewaySeams,
    diagnostics: DiagnosticSink,
    cues: CueRecorder,
    clock: Arc<dyn Clock>,
    control_tx: Arc<Mutex<Option<mpsc::UnboundedSender<ControlCommand>>>>,
    action_timeout_ms: u64,
    hook_timeout_ms: u64,
    working_cue_ms: u64,
    progress_note_ms: u64,
}

#[derive(Default)]
struct RouterState {
    bodies: BTreeMap<BodyId, BodyConnection>,
    place_to_body: BTreeMap<PlaceId, BodyId>,
    attested_oob_bodies: BTreeSet<BodyId>,
    /// Bodies asked to shut down on purpose: their disconnect is logged as a stop, not a crash.
    stopping: BTreeSet<BodyId>,
    children: Vec<Child>,
}

impl BodyRouter {
    /// Creates a new [`BodyRouter`] wired to `ingress`, `diagnostics`, `cues`, and `clock`.
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        ingress: IngressQueue,
        seams: GatewaySeams,
        diagnostics: DiagnosticSink,
        cues: CueRecorder,
        clock: Arc<dyn Clock>,
        action_timeout_ms: u64,
        hook_timeout_ms: u64,
        working_cue_ms: u64,
        progress_note_ms: u64,
    ) -> Self {
        Self {
            inner: Arc::new(Mutex::new(RouterState::default())),
            settlement: ActionSettlementTracker::new(),
            commands: Arc::new(Mutex::new(CommandRegistry::new())),
            command_ctx: Arc::new(RwLock::new(CommandContext::default())),
            ingress,
            seams,
            diagnostics,
            cues,
            clock,
            control_tx: Arc::new(Mutex::new(None)),
            action_timeout_ms,
            hook_timeout_ms,
            working_cue_ms,
            progress_note_ms,
        }
    }

    /// Returns the shared [`CommandRegistry`] populated from built-ins and `body/discover` `_meta.commands`.
    #[must_use]
    pub fn command_registry(&self) -> Arc<Mutex<CommandRegistry>> {
        Arc::clone(&self.commands)
    }

    /// Replaces the facts adapter slash commands are answered from.
    pub fn set_command_context(&self, ctx: CommandContext) {
        *self.command_ctx.write().expect("command ctx lock") = ctx;
    }

    /// Applies an edited configuration live: `[[person]]` account mappings (identity) and the
    /// command facts (owner, RBAC, guest access).
    pub fn apply_config(&self, config: &crate::config::GatewayConfig) {
        if let Some(identity) = &self.seams.identity {
            identity.reload_persons(&config.persons);
        }
        self.set_command_context(CommandContext::from_config(config));
    }

    /// Sends `text` to `actor` at `place` through the body owning the place, bypassing the core
    /// (no model call, no event). Used for gateway-answered slash commands.
    fn spawn_direct_reply(&self, place: PlaceId, actor: monoagens_protocol::ActorId, text: String) {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let conn = {
            let st = self.inner.lock().expect("router lock");
            st.place_to_body
                .get(&place)
                .and_then(|bid| st.bodies.get(bid))
                .cloned()
        };
        let Some(conn) = conn else { return };
        let Some(capability_id) = conn
            .capabilities()
            .iter()
            .find(|c| c.id.as_str() == "chat.speak")
            .map(|c| c.id.clone())
        else {
            return;
        };
        let n = NEXT.fetch_add(1, Ordering::SeqCst);
        let params = ActionExecuteParams {
            headers: Waist2Headers::new(format!("cmd-{n}")),
            action_id: ActionId::new(format!("cmd-{n}")),
            capability_id,
            places: vec![place],
            audience: monoagens_protocol::Audience::new([actor]),
            payload: monoagens_protocol::OpaquePayload::from_bytes(
                serde_json::json!({ "text": text }).to_string().as_bytes(),
            ),
            timeout_ms: Some(self.action_timeout_ms.max(1)),
            h_act: None,
        };
        let clock = Arc::clone(&self.clock);
        let timeout_ms = self.action_timeout_ms.max(1);
        let diagnostics = self.diagnostics.clone();
        tokio::spawn(async move {
            if let Err(err) = conn
                .call::<ActionExecuteParams, ActionExecuteResult>(
                    Waist2Method::ActionExecute,
                    params,
                    timeout_ms,
                    clock.as_ref(),
                )
                .await
            {
                diagnostics.log(
                    clock.now(),
                    LogLevel::Warn,
                    "gateway.commands",
                    &format!("slash command reply failed: {err}"),
                );
            }
        });
    }

    pub(crate) fn set_control_tx(&self, tx: mpsc::UnboundedSender<ControlCommand>) {
        *self.control_tx.lock().expect("control_tx lock") = Some(tx);
    }

    /// Registers a `BodyId` as an attested out-of-band owner control channel permitted to
    /// submit `approval/resolve` (`I-HITL`). Unregistered chat bodies fail closed.
    pub fn register_out_of_band_control_body(&self, body_id: BodyId) {
        self.inner
            .lock()
            .expect("router lock")
            .attested_oob_bodies
            .insert(body_id);
    }

    /// Revokes out-of-band control status for `body_id`.
    pub fn unregister_out_of_band_control_body(&self, body_id: &BodyId) {
        self.inner
            .lock()
            .expect("router lock")
            .attested_oob_bodies
            .remove(body_id);
    }

    /// Returns the shared [`ActionSettlementTracker`].
    #[must_use]
    pub fn settlement(&self) -> &ActionSettlementTracker {
        &self.settlement
    }

    /// Returns currently registered [`BodyId`]s.
    #[must_use]
    pub fn registered_bodies(&self) -> Vec<BodyId> {
        self.inner
            .lock()
            .expect("router lock")
            .bodies
            .keys()
            .cloned()
            .collect()
    }

    /// Returns `true` while `body_id` is registered and its stream is alive.
    #[must_use]
    pub fn is_body_connected(&self, body_id: &BodyId) -> bool {
        self.inner
            .lock()
            .expect("router lock")
            .bodies
            .get(body_id)
            .is_some_and(BodyConnection::is_alive)
    }

    /// Sends `body/shutdown` to one body (best effort, bounded by `timeout_ms`) and marks it dead.
    pub async fn shutdown_body(&self, body_id: &BodyId, reason: &str, timeout_ms: u64) {
        let conn = {
            let mut st = self.inner.lock().expect("router lock");
            let conn = st.bodies.get(body_id).cloned();
            if conn.is_some() {
                st.stopping.insert(body_id.clone());
            }
            conn
        };
        if let Some(conn) = conn {
            conn.shutdown(Some(reason.to_owned()), timeout_ms, self.clock.as_ref())
                .await;
        }
    }

    /// Returns currently connected [`BodyId`]s.
    #[must_use]
    pub fn connected_bodies(&self) -> Vec<BodyId> {
        self.registered_bodies()
    }

    /// Returns all [`PlaceId`]s currently owned by `body_id` (static + dynamically registered).
    #[must_use]
    pub fn owned_places(&self, body_id: &BodyId) -> Vec<PlaceId> {
        let st = self.inner.lock().expect("router lock");
        st.place_to_body
            .iter()
            .filter_map(|(place, owner)| (owner == body_id).then_some(place.clone()))
            .collect()
    }

    /// The body that owns `place`, if one is connected.
    #[must_use]
    pub fn body_of_place(&self, place: &PlaceId) -> Option<BodyId> {
        self.inner
            .lock()
            .expect("router lock")
            .place_to_body
            .get(place)
            .cloned()
    }

    /// Returns all currently registered [`CapabilityDescriptor`]s across live bodies,
    /// deduplicated by [`CapabilityId`].
    #[must_use]
    pub fn all_capabilities(&self) -> Vec<CapabilityDescriptor> {
        let st = self.inner.lock().expect("router lock");
        let mut by_id: BTreeMap<CapabilityId, CapabilityDescriptor> = BTreeMap::new();
        for conn in st.bodies.values() {
            for cap in conn.capabilities() {
                match by_id.get_mut(&cap.id) {
                    Some(existing) => {
                        existing.places.extend(cap.places.iter().cloned());
                        existing.flags.communicative |= cap.flags.communicative;
                        existing.flags.identity_changing |= cap.flags.identity_changing;
                        existing.flags.irreversible |= cap.flags.irreversible;
                        if existing.description.trim().is_empty()
                            && !cap.description.trim().is_empty()
                        {
                            existing.description = cap.description.clone();
                        }
                    }
                    None => {
                        by_id.insert(cap.id.clone(), cap.clone());
                    }
                }
            }
        }
        let mut out: Vec<CapabilityDescriptor> = by_id.into_values().collect();
        sort_capabilities(&mut out);
        out
    }

    /// Returns currently registered [`CapabilityDescriptor`]s grouped by connected [`BodyId`].
    #[must_use]
    pub fn capabilities_by_body(&self) -> BTreeMap<BodyId, Vec<CapabilityDescriptor>> {
        let st = self.inner.lock().expect("router lock");
        st.bodies
            .iter()
            .map(|(body_id, conn)| {
                let mut caps = conn.capabilities().to_vec();
                sort_capabilities(&mut caps);
                (body_id.clone(), caps)
            })
            .collect()
    }

    /// Connects a child process speaking Waist-2 NDJSON over `stdin`/`stdout`, runs
    /// `body/discover` under `hook_timeout_ms`, and registers it into routing and Core.
    pub async fn connect_stdio_process(
        &self,
        command: &str,
        args: &[String],
    ) -> Result<BodyId, BodyRpcError> {
        self.connect_stdio_process_with_env(command, args, &[]).await
    }

    /// Connects a child process speaking Waist-2 NDJSON over `stdin`/`stdout` with extra
    /// injected environment variables (e.g. `MONOAGENS_PLUGIN_CONFIG`), runs `body/discover`
    /// under `hook_timeout_ms`, and registers it into routing and Core.
    pub async fn connect_stdio_process_with_env(
        &self,
        command: &str,
        args: &[String],
        extra_envs: &[(String, String)],
    ) -> Result<BodyId, BodyRpcError> {
        let mut cmd = Command::new(command);
        cmd.args(args)
            .envs(extra_envs.iter().map(|(k, v)| (k.as_str(), v.as_str())))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut child = cmd
            .spawn()
            .map_err(|e| BodyRpcError::Transport(format!("failed to spawn '{command}': {e}")))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| BodyRpcError::Transport("child missing piped stdin".to_owned()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| BodyRpcError::Transport("child missing piped stdout".to_owned()))?;
        self.inner.lock().expect("router lock").children.push(child);
        self.connect_ndjson_stream(stdout, stdin, None).await
    }

    /// Connects a Unix domain socket body speaking Waist-2 NDJSON, runs `body/discover`,
    /// and registers it into routing and Core.
    pub async fn connect_unix_socket(&self, path: &Path) -> Result<BodyId, BodyRpcError> {
        let stream = UnixStream::connect(path).await.map_err(|e| {
            BodyRpcError::Transport(format!("failed to connect '{}': {e}", path.display()))
        })?;
        let (read_half, write_half) = stream.into_split();
        self.connect_ndjson_stream(read_half, write_half, None)
            .await
    }

    /// Connects any bidirectional async NDJSON stream (`AsyncRead` + `AsyncWrite`), runs
    /// `body/discover` under `hook_timeout_ms`, registers places/capabilities, and submits
    /// `Input::BodyState` to Core.
    pub async fn connect_ndjson_stream<R, W>(
        &self,
        reader: R,
        mut writer: W,
        expected_body_id: Option<BodyId>,
    ) -> Result<BodyId, BodyRpcError>
    where
        R: AsyncRead + Unpin + Send + 'static,
        W: AsyncWrite + Unpin + Send + 'static,
    {
        let (outbound_tx, mut outbound_rx) = mpsc::unbounded_channel::<String>();
        let pending_rpcs: PendingRpcMap = Arc::new(Mutex::new(HashMap::new()));
        let alive = Arc::new(AtomicBool::new(true));
        let next_rpc_id = Arc::new(AtomicI64::new(1));

        // Spawn writer task for NDJSON lines.
        let alive_w = Arc::clone(&alive);
        tokio::spawn(async move {
            while let Some(line) = outbound_rx.recv().await {
                if writer.write_all(line.as_bytes()).await.is_err()
                    || writer.write_all(b"\n").await.is_err()
                    || writer.flush().await.is_err()
                {
                    alive_w.store(false, Ordering::SeqCst);
                    break;
                }
            }
        });

        let body_slot: Arc<Mutex<Option<BodyId>>> = Arc::new(Mutex::new(expected_body_id.clone()));

        // Spawn reader task for NDJSON lines (responses + inbound body requests).
        let router_r = self.clone();
        let pending_r = Arc::clone(&pending_rpcs);
        let outbound_r = outbound_tx.clone();
        let alive_r = Arc::clone(&alive);
        let body_slot_r = Arc::clone(&body_slot);
        tokio::spawn(async move {
            let mut buf_reader = BufReader::new(reader);
            let mut line = String::new();
            loop {
                line.clear();
                match buf_reader.read_line(&mut line).await {
                    Ok(0) => {
                        // EOF -> body exited or crashed.
                        break;
                    }
                    Ok(_) => {
                        let trimmed = line.trim();
                        if trimmed.is_empty() {
                            continue;
                        }
                        let current_body_id = body_slot_r.lock().expect("body_slot lock").clone();
                        router_r
                            .handle_inbound_ndjson_line(
                                trimmed,
                                current_body_id.as_ref(),
                                &pending_r,
                                &outbound_r,
                            )
                            .await;
                    }
                    Err(err) => {
                        let bid = body_slot_r.lock().expect("body_slot lock").clone();
                        router_r.diagnostics.log(
                            router_r.clock.now(),
                            LogLevel::Warn,
                            "gateway.body",
                            &format!(
                                "body {} read error: {err}",
                                bid.as_ref().map_or("<unknown>", BodyId::as_str)
                            ),
                        );
                        break;
                    }
                }
            }

            alive_r.store(false, Ordering::SeqCst);
            // Fail all waiting outbound RPCs immediately.
            let drained: Vec<_> = {
                let mut map = pending_r.lock().expect("pending_rpcs lock");
                map.drain().map(|(_, tx)| tx).collect()
            };
            for tx in drained {
                let _ = tx.send(Err(Waist2Error::invalid_receipt_transition(
                    "body stream closed",
                )));
            }
            if let Some(bid) = body_slot_r.lock().expect("body_slot lock").clone() {
                router_r.handle_body_disconnect(&bid, "body stream closed unexpectedly");
            }
        });

        let probe_conn = BodyConnection {
            body_id: expected_body_id
                .clone()
                .unwrap_or_else(|| BodyId::new("discovering")),
            places: Vec::new(),
            capabilities: Vec::new(),
            outbound_tx: outbound_tx.clone(),
            pending_rpcs: Arc::clone(&pending_rpcs),
            next_rpc_id: Arc::clone(&next_rpc_id),
            alive: Arc::clone(&alive),
        };

        let discover_val: Value = probe_conn
            .call(
                Waist2Method::BodyDiscover,
                BodyDiscoverParams {
                    headers: Waist2Headers::new("discover"),
                    body_id: expected_body_id,
                },
                self.hook_timeout_ms,
                self.clock.as_ref(),
            )
            .await?;
        let discover: BodyDiscoverResult = serde_json::from_value(discover_val.clone())
            .map_err(|e| BodyRpcError::Serialization(e.to_string()))?;

        if let Err(ver_err) = check_version(discover.protocol_version, CURRENT_PROTOCOL_VERSION) {
            alive.store(false, Ordering::SeqCst);
            return Err(BodyRpcError::Protocol(Waist2Error::unsupported_version(
                ver_err.received,
                ver_err.supported,
            )));
        }

        let mut caps = discover.capabilities;
        sort_capabilities(&mut caps);
        let body_id = discover.body_id.clone();
        *body_slot.lock().expect("body_slot lock") = Some(body_id.clone());

        if let Some(cmds_val) = discover_val.get("_meta").and_then(|m| m.get("commands")) {
            match serde_json::from_value::<Vec<CommandSpec>>(cmds_val.clone()) {
                Ok(cmds) => {
                    self.commands
                        .lock()
                        .expect("commands lock")
                        .register_plugin_commands(&body_id, cmds);
                }
                Err(err) => {
                    self.diagnostics.log(
                        self.clock.now(),
                        LogLevel::Warn,
                        "gateway.commands",
                        &format!("failed to parse _meta.commands for body '{body_id}': {err}"),
                    );
                }
            }
        }

        let conn = BodyConnection {
            body_id: body_id.clone(),
            places: discover.places.clone(),
            capabilities: caps.clone(),
            outbound_tx,
            pending_rpcs,
            next_rpc_id,
            alive,
        };

        {
            let mut st = self.inner.lock().expect("router lock");
            for place in &discover.places {
                st.place_to_body.insert(place.clone(), body_id.clone());
            }
            st.bodies.insert(body_id.clone(), conn);
        }

        if let Some(service) = &self.seams.pre_queue_service {
            service.index_capabilities(&body_id, &caps);
        }
        if let Some(index) = &self.seams.pre_queue_index {
            index
                .lock()
                .expect("pre_queue_index lock")
                .index_capabilities(&body_id, &caps);
        }

        let body_state = BodyState {
            protocol_version: discover.protocol_version,
            body_id: body_id.clone(),
            status: BodyStatus::Ready,
            capabilities: caps,
            timestamp_ms: self.clock.now(),
        };
        if let Err(err) = self.ingress.try_submit(Input::BodyState(body_state)) {
            self.diagnostics.log(
                self.clock.now(),
                LogLevel::Warn,
                "gateway.body",
                &format!("back-pressure submitting BodyState for '{body_id}': {err}"),
            );
        }

        Ok(body_id)
    }

    async fn handle_inbound_ndjson_line(
        &self,
        line: &str,
        body_id: Option<&BodyId>,
        pending_rpcs: &PendingRpcTable,
        outbound_tx: &mpsc::UnboundedSender<String>,
    ) {
        let val: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(err) => {
                self.diagnostics.log(
                    self.clock.now(),
                    LogLevel::Warn,
                    "gateway.waist2",
                    &format!("invalid JSON-RPC line from body: {err}"),
                );
                let resp: Waist2Response<Value> = Waist2Response::err(
                    None,
                    Waist2Error {
                        code: RPC_PARSE_ERROR,
                        message: format!("parse error: {err}"),
                        data: None,
                    },
                );
                if let Ok(s) = serde_json::to_string(&resp) {
                    let _ = outbound_tx.send(s);
                }
                return;
            }
        };

        // Response to an outbound request?
        if val.get("method").is_none() {
            let resp: Waist2Response<Value> = match serde_json::from_value(val) {
                Ok(r) => r,
                Err(err) => {
                    self.diagnostics.log(
                        self.clock.now(),
                        LogLevel::Warn,
                        "gateway.waist2",
                        &format!("malformed JSON-RPC response from body: {err}"),
                    );
                    return;
                }
            };
            if let Some(id) = resp.id {
                let key = RpcKey::from(&id);
                let sender = pending_rpcs.lock().expect("pending_rpcs lock").remove(&key);
                if let Some(tx) = sender {
                    let payload = match (resp.result, resp.error) {
                        (_, Some(err)) => Err(err),
                        (Some(res), None) => Ok(res),
                        (None, None) => Ok(Value::Null),
                    };
                    let _ = tx.send(payload);
                }
            }
            return;
        }

        // Inbound RPC request/notification from body.
        let rpc_id: Option<RpcId> = val
            .get("id")
            .and_then(|v| serde_json::from_value::<RpcId>(v.clone()).ok());
        let method = val
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        let params = val.get("params").cloned().unwrap_or(Value::Null);

        let reply_json = match method.as_str() {
            "percept/emit" => {
                self.handle_percept_emit(rpc_id.clone(), params, body_id)
                    .await
            }
            "action/receipt" => self.handle_action_receipt(rpc_id.clone(), params),
            "body/heartbeat" => self.handle_body_heartbeat(rpc_id.clone(), params),
            "blob/put" => self.handle_blob_put(rpc_id.clone(), params),
            "blob/get" => self.handle_blob_get(rpc_id.clone(), params),
            "approval/resolve" => {
                self.handle_approval_resolve(rpc_id.clone(), params, body_id)
                    .await
            }
            other => serialize_err_response(
                rpc_id.clone(),
                RPC_METHOD_NOT_FOUND,
                &format!("method '{other}' not supported on Gateway inbound"),
            ),
        };

        if rpc_id.is_some()
            && let Some(line) = reply_json
        {
            let _ = outbound_tx.send(line);
        }
    }

    async fn handle_percept_emit(
        &self,
        rpc_id: Option<RpcId>,
        params: Value,
        body_id: Option<&BodyId>,
    ) -> Option<String> {
        let parsed: PerceptEmitParams = match serde_json::from_value(params) {
            Ok(p) => p,
            Err(err) => {
                return serialize_err_response(
                    rpc_id,
                    RPC_INVALID_PARAMS,
                    &format!("invalid percept/emit params: {err}"),
                );
            }
        };
        let mut percept = match parsed.into_percept(CURRENT_PROTOCOL_VERSION) {
            Ok(p) => p,
            Err(werr) => {
                return serde_json::to_string(&Waist2Response::<PerceptEmitResult>::err(
                    rpc_id, werr,
                ))
                .ok();
            }
        };

        // Dynamic place ownership registration + anti-hijack guard for authenticated bodies.
        if let Some(bid) = body_id {
            let mut st = self.inner.lock().expect("router lock");
            match st.place_to_body.get(&percept.place) {
                None => {
                    st.place_to_body.insert(percept.place.clone(), bid.clone());
                    if let Some(conn) = st.bodies.get_mut(bid)
                        && !conn.places.contains(&percept.place)
                    {
                        conn.places.push(percept.place.clone());
                    }
                }
                Some(existing_owner) if existing_owner != bid => {
                    let owner = existing_owner.clone();
                    drop(st);
                    self.diagnostics.log(
                        self.clock.now(),
                        LogLevel::Warn,
                        "gateway.waist2",
                        &format!(
                            "ERR_PLACE_OWNED_BY_OTHER_BODY: body '{bid}' attempted to claim place '{}' owned by '{owner}'",
                            percept.place
                        ),
                    );
                    return serialize_err_response(
                        rpc_id,
                        RPC_INVALID_PARAMS,
                        &format!(
                            "ERR_PLACE_OWNED_BY_OTHER_BODY: place '{}' is already owned by body '{owner}'",
                            percept.place
                        ),
                    );
                }
                Some(_) => {}
            }
        }

        // Seam 4.3: canonical κ(b, a) identity resolution if wired.
        if let (Some(id_seam), Some(bid)) = (&self.seams.identity, body_id) {
            let raw_actor = percept.actor.clone();
            // Registered people resolve to their canonical actor; an unregistered sender is a
            // guest and keeps the account's own actor id (readable, and pickable in /people).
            let resolved = id_seam
                .resolve_registered(bid, &raw_actor)
                .unwrap_or_else(|| raw_actor.clone());
            if resolved != raw_actor {
                percept.actor = resolved.clone();
                if let Some(origin) = &percept.origin_actor
                    && origin.actor_id() == &raw_actor
                {
                    percept.origin_actor =
                        Some(monoagens_protocol::OriginActor::new(resolved.clone()));
                }
                if percept.audience.members.remove(&raw_actor) {
                    percept.audience.members.insert(resolved);
                }
            }
        }

        // Slash commands from an adapter are answered here, with no model call, for every
        // admitted sender (guests included) and filtered by their permissions.
        if body_id.is_some()
            && let Some(text) = percept_commands::percept_text(&percept)
            && text.trim_start().starts_with('/')
        {
            let capabilities = self.all_capabilities();
            let reply = {
                let ctx = self.command_ctx.read().expect("command ctx lock");
                let registry = self.commands.lock().expect("commands lock");
                percept_commands::answer_with_descriptors(
                    &registry,
                    &ctx,
                    &percept,
                    &text,
                    &capabilities,
                )
            };
            if let Some(reply) = reply {
                self.spawn_direct_reply(percept.place.clone(), percept.actor.clone(), reply);
                return serde_json::to_string(&Waist2Response::ok(
                    rpc_id,
                    PerceptEmitResult { accepted: true },
                ))
                .ok();
            }
        }

        // Seam 4.4: inline attachment custody if wired; reject oversized inline payloads cleanly.
        if let (Some(att_seam), Some(bid)) = (&self.seams.attachments, body_id) {
            if let Err(err) = att_seam
                .lock()
                .expect("attachments seam lock")
                .ingest_percept_attachments(bid, &mut percept)
            {
                return serialize_err_response(rpc_id, RPC_INVALID_PARAMS, &err.to_string());
            }
        } else if percept.payload.len() > MAX_INLINE_ATTACHMENT_BYTES {
            return serialize_err_response(
                rpc_id,
                RPC_INVALID_PARAMS,
                &format!(
                    "inline payload {} bytes exceeds {} KiB limit and no attachment store seam is wired",
                    percept.payload.len(),
                    MAX_INLINE_ATTACHMENT_BYTES / 1024
                ),
            );
        }

        // Seam 4.5: pre-queue embedder if wired; outages/timeouts degrade cleanly to vector-less percepts.
        if let Some(service) = &self.seams.pre_queue_service {
            let _ = service.enrich_percept_async(&mut percept).await;
        } else if let Some(embed_seam) = &self.seams.pre_queue_index
            && let Err(err) = embed_seam
                .lock()
                .expect("pre_queue_index seam lock")
                .enrich_percept(&mut percept)
        {
            self.diagnostics.log(
                self.clock.now(),
                LogLevel::Warn,
                "gateway.embedder",
                &format!("pre-queue embedder failed (continuing vector-less): {err}"),
            );
        }

        // Bounded async gateway hooks (`hook_timeout_ms`).
        for hook in &self.seams.hooks {
            let fut = hook.on_percept(&percept);
            match run_with_clock_timeout(self.clock.as_ref(), self.hook_timeout_ms, fut).await {
                Ok(Ok(())) => {}
                Ok(Err(reason)) => {
                    self.diagnostics.log(
                        self.clock.now(),
                        LogLevel::Warn,
                        "gateway.hook",
                        &format!("hook '{}' failed: {reason}", hook.name()),
                    );
                }
                Err(_timed_out) => {
                    self.diagnostics.log(
                        self.clock.now(),
                        LogLevel::Warn,
                        "gateway.hook",
                        &format!(
                            "hook '{}' timed out after {}ms",
                            hook.name(),
                            self.hook_timeout_ms
                        ),
                    );
                }
            }
        }

        if let Err(err) = self.ingress.try_submit(Input::Percept(Box::new(percept))) {
            return serialize_err_response(
                rpc_id,
                RPC_BACKPRESSURE,
                &format!("ingress queue back-pressure: {err}"),
            );
        }

        serde_json::to_string(&Waist2Response::ok(
            rpc_id,
            PerceptEmitResult { accepted: true },
        ))
        .ok()
    }

    fn handle_action_receipt(&self, rpc_id: Option<RpcId>, params: Value) -> Option<String> {
        let parsed: ActionReceiptParams = match serde_json::from_value(params) {
            Ok(p) => p,
            Err(err) => {
                return serialize_err_response(
                    rpc_id,
                    RPC_INVALID_PARAMS,
                    &format!("invalid action/receipt params: {err}"),
                );
            }
        };
        if let Err(err) = check_version(parsed.headers.protocol_version, CURRENT_PROTOCOL_VERSION) {
            return serde_json::to_string(&Waist2Response::<ActionReceiptResult>::err(
                rpc_id,
                Waist2Error::unsupported_version(err.received, err.supported),
            ))
            .ok();
        }
        if let Err(err) = parsed.receipt.validate() {
            return serde_json::to_string(&Waist2Response::<ActionReceiptResult>::err(
                rpc_id,
                err.into(),
            ))
            .ok();
        }

        let recorded = match self
            .settlement
            .try_settle(&parsed.receipt.action_id, parsed.receipt.clone())
        {
            SettlementOutcome::SettledFirst(receipt) => {
                if let Err(err) = self.ingress.try_submit(Input::Receipt(receipt)) {
                    self.diagnostics.log(
                        self.clock.now(),
                        LogLevel::Error,
                        "gateway.receipt",
                        &format!("failed to enqueue receipt: {err}"),
                    );
                    false
                } else {
                    true
                }
            }
            SettlementOutcome::AlreadySettledOrUnknown => {
                self.diagnostics.log(
                    self.clock.now(),
                    LogLevel::Warn,
                    "gateway.receipt",
                    &format!(
                        "ignored duplicate or post-timeout receipt for action '{}'",
                        parsed.receipt.action_id
                    ),
                );
                false
            }
        };

        serde_json::to_string(&Waist2Response::ok(
            rpc_id,
            ActionReceiptResult { recorded },
        ))
        .ok()
    }

    fn handle_body_heartbeat(&self, rpc_id: Option<RpcId>, params: Value) -> Option<String> {
        let parsed: BodyHeartbeatParams = match serde_json::from_value(params) {
            Ok(p) => p,
            Err(err) => {
                return serialize_err_response(
                    rpc_id,
                    RPC_INVALID_PARAMS,
                    &format!("invalid body/heartbeat params: {err}"),
                );
            }
        };
        if let Err(err) = check_version(parsed.headers.protocol_version, CURRENT_PROTOCOL_VERSION) {
            return serde_json::to_string(&Waist2Response::<BodyHeartbeatResult>::err(
                rpc_id,
                Waist2Error::unsupported_version(err.received, err.supported),
            ))
            .ok();
        }
        let now = self.clock.now();
        serde_json::to_string(&Waist2Response::ok(
            rpc_id,
            BodyHeartbeatResult {
                acknowledged_at_ms: now,
            },
        ))
        .ok()
    }

    async fn handle_approval_resolve(
        &self,
        rpc_id: Option<RpcId>,
        params: Value,
        body_id: Option<&BodyId>,
    ) -> Option<String> {
        let mut parsed: ApprovalResolveParams = match serde_json::from_value(params) {
            Ok(p) => p,
            Err(err) => {
                return serialize_err_response(
                    rpc_id,
                    RPC_INVALID_PARAMS,
                    &format!("invalid approval/resolve params: {err}"),
                );
            }
        };
        if let Err(err) = check_version(parsed.headers.protocol_version, CURRENT_PROTOCOL_VERSION) {
            return serde_json::to_string(&Waist2Response::<ApprovalResolveResult>::err(
                rpc_id,
                Waist2Error::unsupported_version(err.received, err.supported),
            ))
            .ok();
        }
        let Some(control_tx) = self.control_tx.lock().expect("control_tx lock").clone() else {
            return serialize_err_response(
                rpc_id,
                RPC_METHOD_NOT_FOUND,
                "approval/resolve capability is not wired in this Gateway",
            );
        };

        if let (Some(id_seam), Some(bid)) = (&self.seams.identity, body_id)
            && let Some(resolved) = id_seam.resolve_registered(bid, &parsed.resolver)
            && resolved != parsed.resolver
        {
            parsed.resolver = resolved;
        }

        let is_attested_oob = body_id.is_some_and(|bid| {
            self.inner
                .lock()
                .expect("router lock")
                .attested_oob_bodies
                .contains(bid)
        });
        let from_in_band_body = if is_attested_oob {
            None
        } else {
            Some(
                body_id
                    .cloned()
                    .unwrap_or_else(|| BodyId::new("unknown_in_band_body")),
            )
        };

        let (tx, rx) = oneshot::channel();
        if control_tx
            .send(ControlCommand::ResolveApproval {
                params: Box::new(parsed),
                from_in_band_body,
                reply: tx,
            })
            .is_err()
        {
            return serialize_err_response(rpc_id, RPC_BACKPRESSURE, "owner task exited");
        }
        match rx.await {
            Ok(Ok(res)) => serde_json::to_string(&Waist2Response::ok(rpc_id, res)).ok(),
            Ok(Err(werr)) => {
                serde_json::to_string(&Waist2Response::<ApprovalResolveResult>::err(rpc_id, werr))
                    .ok()
            }
            Err(_) => serialize_err_response(rpc_id, RPC_BACKPRESSURE, "owner task exited"),
        }
    }

    fn handle_blob_put(&self, rpc_id: Option<RpcId>, params: Value) -> Option<String> {
        let Some(att_seam) = &self.seams.attachments else {
            return serialize_err_response(
                rpc_id,
                RPC_METHOD_NOT_FOUND,
                "blob/put capability is not wired in this Gateway",
            );
        };
        let parsed: BlobPutParams = match serde_json::from_value(params) {
            Ok(p) => p,
            Err(err) => {
                return serialize_err_response(rpc_id, RPC_INVALID_PARAMS, &err.to_string());
            }
        };
        match att_seam
            .lock()
            .expect("attachments seam lock")
            .handle_blob_put(parsed)
        {
            Ok(res) => serde_json::to_string(&Waist2Response::ok(rpc_id, res)).ok(),
            Err(werr) => serde_json::to_string(&Waist2Response::<Value>::err(rpc_id, werr)).ok(),
        }
    }

    fn handle_blob_get(&self, rpc_id: Option<RpcId>, params: Value) -> Option<String> {
        let Some(att_seam) = &self.seams.attachments else {
            return serialize_err_response(
                rpc_id,
                RPC_METHOD_NOT_FOUND,
                "blob/get capability is not wired in this Gateway",
            );
        };
        let parsed: BlobGetParams = match serde_json::from_value(params) {
            Ok(p) => p,
            Err(err) => {
                return serialize_err_response(rpc_id, RPC_INVALID_PARAMS, &err.to_string());
            }
        };
        match att_seam
            .lock()
            .expect("attachments seam lock")
            .handle_blob_get(parsed)
        {
            Ok(res) => serde_json::to_string(&Waist2Response::ok(rpc_id, res)).ok(),
            Err(werr) => serde_json::to_string(&Waist2Response::<Value>::err(rpc_id, werr)).ok(),
        }
    }

    /// Removes a crashed/disconnected body from routing, logs the failure for the owner,
    /// settles all its pending actions ONCE as `unknown`, and notifies Core (`BodyStatus::Stopped`).
    pub fn handle_body_disconnect(&self, body_id: &BodyId, reason: &str) {
        let (removed, intentional) = {
            let mut st = self.inner.lock().expect("router lock");
            let conn = st.bodies.remove(body_id);
            st.place_to_body.retain(|_, owner| owner != body_id);
            (conn.is_some(), st.stopping.remove(body_id))
        };

        if removed {
            self.commands
                .lock()
                .expect("commands lock")
                .unregister_plugin_commands(body_id);
            if let Some(service) = &self.seams.pre_queue_service {
                service.remove_body_capabilities(body_id);
            }
            if let Some(index) = &self.seams.pre_queue_index {
                index
                    .lock()
                    .expect("pre_queue_index lock")
                    .remove_body_capabilities(body_id);
            }

            let now = self.clock.now();
            if !intentional {
                // An intentional stop is reported once by the supervisor (info), not here.
                self.diagnostics.log(
                    now,
                    LogLevel::Error,
                    "gateway.body",
                    &format!("body '{body_id}' disconnected/crashed: {reason}"),
                );
            }

            let unknown_receipts = self.settlement.settle_crashed_body(body_id, now, reason);
            for receipt in unknown_receipts {
                if let Err(err) = self.ingress.try_submit(Input::Receipt(receipt)) {
                    self.diagnostics.log(
                        self.clock.now(),
                        LogLevel::Error,
                        "gateway.receipt",
                        &format!("failed to submit crash unknown receipt: {err}"),
                    );
                }
            }

            let stopped = BodyState {
                protocol_version: CURRENT_PROTOCOL_VERSION,
                body_id: body_id.clone(),
                status: BodyStatus::Stopped,
                capabilities: Vec::new(),
                timestamp_ms: now,
            };
            let _ = self.ingress.try_submit(Input::BodyState(stopped));
        }
    }

    /// Spawns a bounded async task to dispatch `action` to the body owning its primary place,
    /// settling in Core AT MOST ONCE under `action_timeout_ms`.
    pub fn spawn_dispatch_action(&self, action: Action) {
        let router = self.clone();
        tokio::spawn(async move {
            router.dispatch_action(action).await;
        });
    }

    /// Dispatches `action` to its target body under `action_timeout_ms` and settles once.
    pub async fn dispatch_action(&self, action: Action) {
        let primary_place = action.places.first().cloned();
        let target_conn = {
            let st = self.inner.lock().expect("router lock");
            primary_place
                .as_ref()
                .and_then(|p| st.place_to_body.get(p))
                .and_then(|bid| st.bodies.get(bid))
                .cloned()
        };

        let Some(conn) = target_conn else {
            let receipt = make_terminal_receipt(
                action.action_id.clone(),
                action.capability_id.clone(),
                action.h_act.clone(),
                ReceiptOutcome::Refused,
                Some("ERR_UNKNOWN_PLACE".to_owned()),
                "no connected body owns target place".to_owned(),
                TerminationReason::Normal,
                self.clock.now(),
            );
            let _ = self.ingress.try_submit(Input::Receipt(receipt));
            return;
        };

        let body_id = conn.body_id().clone();
        let cap_limit_ms = conn
            .capabilities()
            .iter()
            .find(|c| c.id == action.capability_id)
            .map(|c| c.typical_latency_ms);
        // Spec D2: T_act = min(action_timeout_ms, body_limit) when body specifies a bound,
        // otherwise action_timeout_ms. Wait: typical_latency_ms is an estimate, not a hard timeout,
        // so keep action_timeout_ms as the hard cap unless explicitly overridden.
        let _ = cap_limit_ms;
        let timeout_ms = self.action_timeout_ms.max(1);

        let cue_guard = spawn_wait_cue_monitor(
            Arc::clone(&self.clock),
            self.cues.clone(),
            action.action_id.to_string(),
            primary_place,
            self.working_cue_ms,
            self.progress_note_ms,
        );
        let settled_notify =
            self.settlement
                .register_pending(&action, body_id.clone(), Some(cue_guard));

        let params = ActionExecuteParams {
            headers: Waist2Headers::new(format!("exec-{}", action.action_id)),
            action_id: action.action_id.clone(),
            capability_id: action.capability_id.clone(),
            places: action.places.clone(),
            audience: action.audience.clone(),
            payload: action.payload.clone(),
            timeout_ms: Some(timeout_ms),
            h_act: action.h_act.clone(),
        };

        let started_at = self.clock.now();
        let deadline = started_at.saturating_add_ms(timeout_ms);

        match conn
            .call::<ActionExecuteParams, ActionExecuteResult>(
                Waist2Method::ActionExecute,
                params,
                timeout_ms,
                self.clock.as_ref(),
            )
            .await
        {
            Ok(exec_res) => {
                if let Some(receipt) = exec_res.receipt {
                    self.settle_and_submit(&action.action_id, receipt);
                    return;
                }
                if !exec_res.accepted {
                    let receipt = make_terminal_receipt(
                        action.action_id.clone(),
                        action.capability_id.clone(),
                        action.h_act.clone(),
                        ReceiptOutcome::Failed,
                        Some("action_rejected".to_owned()),
                        "body returned accepted=false without receipt".to_owned(),
                        TerminationReason::Normal,
                        self.clock.now(),
                    );
                    self.settle_and_submit(&action.action_id, receipt);
                    return;
                }

                // Async action accepted: wait for `action/receipt`, crash, or `T_act` timeout.
                loop {
                    let notified = settled_notify.notified();
                    if self.settlement.is_settled(&action.action_id) {
                        return;
                    }
                    tokio::select! {
                        biased;
                        _ = notified => {
                            if self.settlement.is_settled(&action.action_id) {
                                return;
                            }
                        }
                        _ = self.clock.sleep_until(deadline) => {
                            if let Some(r) = self.settlement.try_settle_timeout(&action, timeout_ms, self.clock.now()) {
                                self.diagnostics.log(
                                    self.clock.now(),
                                    LogLevel::Warn,
                                    "gateway.action",
                                    &format!("action '{}' timed out after {}ms on body '{}'", action.action_id, timeout_ms, body_id),
                                );
                                let _ = self.ingress.try_submit(Input::Receipt(r));
                            }
                            return;
                        }
                    }
                }
            }
            Err(BodyRpcError::Timeout { .. }) => {
                if let Some(r) =
                    self.settlement
                        .try_settle_timeout(&action, timeout_ms, self.clock.now())
                {
                    self.diagnostics.log(
                        self.clock.now(),
                        LogLevel::Warn,
                        "gateway.action",
                        &format!(
                            "action '{}' execute RPC timed out after {}ms on body '{}'",
                            action.action_id, timeout_ms, body_id
                        ),
                    );
                    let _ = self.ingress.try_submit(Input::Receipt(r));
                }
            }
            Err(BodyRpcError::Disconnected) | Err(BodyRpcError::Transport(_)) => {
                self.handle_body_disconnect(&body_id, "body disconnected during action/execute");
            }
            Err(BodyRpcError::Protocol(werr)) => {
                let outcome = if werr.code == -32001 {
                    ReceiptOutcome::Refused
                } else {
                    ReceiptOutcome::Failed
                };
                let receipt = make_terminal_receipt(
                    action.action_id.clone(),
                    action.capability_id.clone(),
                    action.h_act.clone(),
                    outcome,
                    werr.data.map(|d| d.symbol),
                    werr.message,
                    TerminationReason::Normal,
                    self.clock.now(),
                );
                self.settle_and_submit(&action.action_id, receipt);
            }
            Err(BodyRpcError::Serialization(detail)) => {
                let receipt = make_terminal_receipt(
                    action.action_id.clone(),
                    action.capability_id.clone(),
                    action.h_act.clone(),
                    ReceiptOutcome::Failed,
                    Some("ERR_SERIALIZATION".to_owned()),
                    detail,
                    TerminationReason::Normal,
                    self.clock.now(),
                );
                self.settle_and_submit(&action.action_id, receipt);
            }
        }
    }

    fn settle_and_submit(&self, action_id: &ActionId, receipt: Receipt) {
        if let SettlementOutcome::SettledFirst(r) = self.settlement.try_settle(action_id, receipt) {
            let _ = self.ingress.try_submit(Input::Receipt(r));
        }
    }

    /// Dispatches a generic [`CAPABILITY_APPROVAL_PROMPT`] (`"approval.prompt"`) action to the
    /// adapter owning `prompt_place` if and only if that adapter declared `"approval.prompt"` in
    /// `body/discover`.
    ///
    /// Returns `Ok(false)` cleanly when no body owns `prompt_place` or the owning body does not
    /// declare `"approval.prompt"` (so callers fall back to CLI/text prompts), and `Ok(true)` when
    /// the adapter accepted the prompt dispatch.
    pub async fn dispatch_approval_prompt(
        &self,
        prompt_place: &PlaceId,
        audience: monoagens_protocol::Audience,
        req: &ApprovalPromptRequest,
    ) -> Result<bool, BodyRpcError> {
        let target_conn = {
            let st = self.inner.lock().expect("router lock");
            st.place_to_body
                .get(prompt_place)
                .and_then(|bid| st.bodies.get(bid))
                .cloned()
        };
        let Some(conn) = target_conn else {
            return Ok(false);
        };
        let declares_prompt = conn
            .capabilities()
            .iter()
            .any(|cap| cap.id.as_str() == CAPABILITY_APPROVAL_PROMPT);
        if !declares_prompt {
            return Ok(false);
        }

        let payload_bytes =
            serde_json::to_vec(req).map_err(|e| BodyRpcError::Serialization(e.to_string()))?;
        let timeout_ms = self.action_timeout_ms.max(1);
        let params = ActionExecuteParams {
            headers: Waist2Headers::new(format!("approval-prompt-{}", req.action_id)),
            action_id: ActionId::new(format!("approval-prompt:{}", req.action_id)),
            capability_id: CapabilityId::new(CAPABILITY_APPROVAL_PROMPT),
            places: vec![prompt_place.clone()],
            audience,
            payload: monoagens_protocol::OpaquePayload::from_bytes(payload_bytes),
            timeout_ms: Some(timeout_ms),
            h_act: Some(req.h_act.clone()),
        };

        let res: ActionExecuteResult = conn
            .call(
                Waist2Method::ActionExecute,
                params,
                timeout_ms,
                self.clock.as_ref(),
            )
            .await?;
        Ok(res.accepted)
    }

    /// Sends `action/cancel` to the body owning `places.first()` when Core cancels an action.
    pub async fn cancel_action_on_body(
        &self,
        action_id: ActionId,
        place: Option<&PlaceId>,
        reason: Option<String>,
    ) -> Result<ActionCancelResult, BodyRpcError> {
        let target_conn = {
            let st = self.inner.lock().expect("router lock");
            place
                .and_then(|p| st.place_to_body.get(p))
                .and_then(|bid| st.bodies.get(bid))
                .cloned()
        };
        let Some(conn) = target_conn else {
            return Err(BodyRpcError::Disconnected);
        };
        conn.call(
            Waist2Method::ActionCancel,
            ActionCancelParams {
                headers: Waist2Headers::new(format!("cancel-{action_id}")),
                action_id,
                reason,
            },
            self.hook_timeout_ms,
            self.clock.as_ref(),
        )
        .await
    }

    /// Queries crash-recovery state (`state/query`) on a connected body (`<= 5000 ms`).
    pub async fn query_body_state(
        &self,
        body_id: &BodyId,
        action_id: ActionId,
    ) -> Result<StateQueryResult, BodyRpcError> {
        let conn = self
            .inner
            .lock()
            .expect("router lock")
            .bodies
            .get(body_id)
            .cloned()
            .ok_or(BodyRpcError::Disconnected)?;
        conn.call(
            Waist2Method::StateQuery,
            StateQueryParams {
                headers: Waist2Headers::new(format!("query-{action_id}")),
                action_id,
            },
            5_000,
            self.clock.as_ref(),
        )
        .await
    }

    /// Gracefully shuts down all connected bodies and reaps any spawned child processes.
    pub async fn shutdown_all(&self, grace_ms: u64) {
        let conns: Vec<BodyConnection> = self
            .inner
            .lock()
            .expect("router lock")
            .bodies
            .values()
            .cloned()
            .collect();
        for conn in conns {
            conn.shutdown(
                Some("gateway_shutdown".to_owned()),
                grace_ms,
                self.clock.as_ref(),
            )
            .await;
        }
        let mut children: Vec<Child> = {
            let mut st = self.inner.lock().expect("router lock");
            st.bodies.clear();
            st.place_to_body.clear();
            std::mem::take(&mut st.children)
        };
        for child in &mut children {
            let _ = child.kill().await;
            let _ = child.wait().await;
        }
    }
}

fn serialize_err_response(rpc_id: Option<RpcId>, code: i32, message: &str) -> Option<String> {
    serde_json::to_string(&Waist2Response::<Value>::err(
        rpc_id,
        Waist2Error {
            code,
            message: message.to_owned(),
            data: None,
        },
    ))
    .ok()
}

/// Constructs a terminal/unknown [`Receipt`]; `Unknown` receipts omit progress and delivered units.
#[allow(clippy::too_many_arguments)]
fn make_terminal_receipt(
    action_id: ActionId,
    capability_id: CapabilityId,
    h_act: Option<monoagens_protocol::HAct>,
    outcome: ReceiptOutcome,
    reason_code: Option<String>,
    detail: String,
    termination_reason: TerminationReason,
    timestamp_ms: Time,
) -> Receipt {
    let (progress, delivered_units) = if outcome == ReceiptOutcome::Unknown {
        (None, None)
    } else {
        (Some(0.0), Some(0))
    };
    Receipt {
        action_id,
        capability_id,
        outcome,
        reason_code,
        detail: Some(detail),
        termination_reason: Some(termination_reason),
        progress,
        delivered_units,
        h_act,
        timestamp_ms,
    }
}
