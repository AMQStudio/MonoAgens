//! Single-owner Core task, bounded 3-class `IngressQueue`, `next_due()` timer driver,
//! and clean lifecycle management.

use core::fmt;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use monoagens_core::neural::{ComputeTier, NeuralModel};
use monoagens_core::{
    Core, CoreConfig, CoreError, DoorDecision, Effect, EventRecord, Input, QueueClass,
    QueueOverflow, Store, StoreError, ThreeClassQueue,
};
use monoagens_protocol::{
    ApprovalResolveParams, ApprovalResolveResult, BodyId, Time, Waist2Error,
};
use monoagens_reason::{InfoGapClassifier, ModelProvider, ReasoningEffort};
use monoagens_store::{ConfigLoadError, SegmentedStore, StoreConfig};
use tokio::sync::{Notify, mpsc, oneshot};

use crate::clock::Clock;
use crate::config::{EmbedBackendKind, GatewayConfig};
use crate::cues::CueRecorder;
use crate::identity::IdentityResolver;
use crate::logging::{DiagnosticSink, LogLevel};
use crate::prequeue::{PreQueueService, build_embedder_from_config_async};
use crate::provider::{DailyBudgetTracker, GatewayModelExecutor, build_provider_from_config_async};
use crate::seams::{DoorController, GatewaySeams};
use crate::waist2::{BodyRouter, BodyRpcError};

/// Outcome returned when submitting an [`Input`] with [`GatewayHandle::submit_and_wait`].
#[derive(Debug, Clone, PartialEq)]
pub struct TurnOutcome {
    pub effects_spawned: usize,
    pub next_due: Option<Time>,
}

/// Typed errors produced by the Gateway runtime and ingress queue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GatewayError {
    QueueOverflow(QueueOverflow),
    Core(String),
    Store(String),
    CoreConfig(String),
    NeuralModel(String),
    OwnerExited,
    ProviderInit(String),
}

impl fmt::Display for GatewayError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::QueueOverflow(ov) => write!(f, "{ov}"),
            Self::Core(msg) => write!(f, "core error: {msg}"),
            Self::Store(msg) => write!(f, "store error: {msg}"),
            Self::CoreConfig(msg) => write!(f, "core config error: {msg}"),
            Self::NeuralModel(msg) => write!(f, "neural model error: {msg}"),
            Self::OwnerExited => f.write_str("core owner task exited"),
            Self::ProviderInit(msg) => write!(f, "provider init error: {msg}"),
        }
    }
}

impl std::error::Error for GatewayError {}

impl From<QueueOverflow> for GatewayError {
    fn from(err: QueueOverflow) -> Self {
        Self::QueueOverflow(err)
    }
}

impl From<CoreError> for GatewayError {
    fn from(err: CoreError) -> Self {
        Self::Core(err.to_string())
    }
}

impl From<StoreError> for GatewayError {
    fn from(err: StoreError) -> Self {
        Self::Store(err.to_string())
    }
}

impl From<ConfigLoadError> for GatewayError {
    fn from(err: ConfigLoadError) -> Self {
        Self::CoreConfig(err.to_string())
    }
}

/// Envelope queued inside [`IngressQueue`].
pub struct IngressEnvelope {
    pub input: Input,
    pub ack: Option<oneshot::Sender<Result<TurnOutcome, GatewayError>>>,
}

/// Single bounded 3-class ingress priority queue (`A=256 > B=512 > C=256`) feeding the
/// dedicated Core owner task. Back-pressure always returns [`GatewayError::QueueOverflow`].
#[derive(Clone)]
pub struct IngressQueue {
    queue: Arc<Mutex<ThreeClassQueue<IngressEnvelope>>>,
    notify: Arc<Notify>,
}

impl Default for IngressQueue {
    fn default() -> Self {
        Self::new()
    }
}

impl IngressQueue {
    /// Creates an empty [`IngressQueue`].
    #[must_use]
    pub fn new() -> Self {
        Self {
            queue: Arc::new(Mutex::new(ThreeClassQueue::new())),
            notify: Arc::new(Notify::new()),
        }
    }

    /// Enqueues `input` into its priority class (`A > B > C`), returning [`GatewayError::QueueOverflow`]
    /// immediately if that class is full (never silently dropping).
    pub fn try_submit(&self, input: Input) -> Result<QueueClass, GatewayError> {
        self.try_submit_with_ack(input, None)
    }

    /// Enqueues `input` with an optional completion ack sender.
    pub(crate) fn try_submit_with_ack(
        &self,
        input: Input,
        ack: Option<oneshot::Sender<Result<TurnOutcome, GatewayError>>>,
    ) -> Result<QueueClass, GatewayError> {
        let class = input.queue_class();
        let mut q = self.queue.lock().expect("ingress queue lock");
        q.push(class, IngressEnvelope { input, ack })?;
        drop(q);
        self.notify.notify_one();
        Ok(class)
    }

    /// Pops the highest-priority item (`A > B > C`).
    pub fn try_pop(&self) -> Option<(QueueClass, IngressEnvelope)> {
        self.queue.lock().expect("ingress queue lock").pop()
    }

    /// Returns total queued items across all 3 classes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.queue.lock().expect("ingress queue lock").len()
    }

    /// Returns `true` if all 3 classes are empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.queue.lock().expect("ingress queue lock").is_empty()
    }

    /// Returns current length of `class`.
    #[must_use]
    pub fn class_len(&self, class: QueueClass) -> usize {
        self.queue
            .lock()
            .expect("ingress queue lock")
            .class_len(class)
    }

    /// Future resolving when new ingress may be available.
    pub fn notified(&self) -> tokio::sync::futures::Notified<'_> {
        self.notify.notified()
    }
}

pub(crate) enum ControlCommand {
    ReadEvents {
        reply: oneshot::Sender<Result<Vec<EventRecord>, GatewayError>>,
    },
    NextDue {
        reply: oneshot::Sender<Option<Time>>,
    },
    ResolveApproval {
        params: Box<ApprovalResolveParams>,
        from_in_band_body: Option<BodyId>,
        reply: oneshot::Sender<Result<ApprovalResolveResult, Waist2Error>>,
    },
    Shutdown {
        reply: oneshot::Sender<()>,
    },
}

/// Statistics asserting single-owner non-concurrent execution of `Core::step`.
#[derive(Debug, Clone, Default)]
pub struct OwnershipStats {
    active_calls: Arc<AtomicUsize>,
    max_concurrent_calls: Arc<AtomicUsize>,
    total_steps: Arc<AtomicUsize>,
}

impl OwnershipStats {
    fn enter_step(&self) {
        let prev = self.active_calls.fetch_add(1, Ordering::SeqCst);
        let concurrent = prev + 1;
        self.max_concurrent_calls
            .fetch_max(concurrent, Ordering::SeqCst);
        assert_eq!(
            prev, 0,
            "Core::step invoked concurrently by {concurrent} tasks — ownership invariant violated"
        );
    }

    fn exit_step(&self) {
        self.active_calls.fetch_sub(1, Ordering::SeqCst);
        self.total_steps.fetch_add(1, Ordering::SeqCst);
    }

    /// Maximum observed concurrent `Core::step` calls (MUST always be `<= 1`).
    #[must_use]
    pub fn max_concurrent_calls(&self) -> usize {
        self.max_concurrent_calls.load(Ordering::SeqCst)
    }

    /// Total completed `Core::step` calls.
    #[must_use]
    pub fn total_steps(&self) -> usize {
        self.total_steps.load(Ordering::SeqCst)
    }
}

/// Cloneable handle to interact with a running [`GatewayRuntime`].
#[derive(Clone)]
pub struct GatewayHandle {
    ingress: IngressQueue,
    control_tx: mpsc::UnboundedSender<ControlCommand>,
    router: BodyRouter,
    diagnostics: DiagnosticSink,
    cues: CueRecorder,
    budget: DailyBudgetTracker,
    clock: Arc<dyn Clock>,
    seams: GatewaySeams,
    ownership_stats: OwnershipStats,
    shutdown_grace_ms: u64,
}

impl GatewayHandle {
    /// Submits an [`Input`] into the single bounded ingress queue (non-blocking).
    pub fn try_submit(&self, input: Input) -> Result<QueueClass, GatewayError> {
        self.ingress.try_submit(input)
    }

    /// Submits an [`Input`] into the ingress queue and awaits its turn outcome.
    pub async fn submit_and_wait(&self, input: Input) -> Result<TurnOutcome, GatewayError> {
        let (tx, rx) = oneshot::channel();
        self.ingress.try_submit_with_ack(input, Some(tx))?;
        rx.await.map_err(|_| GatewayError::OwnerExited)?
    }

    /// Connects a bidirectional NDJSON Waist-2 body stream and registers it.
    pub async fn connect_ndjson_body<R, W>(
        &self,
        reader: R,
        writer: W,
        expected_body_id: Option<BodyId>,
    ) -> Result<BodyId, BodyRpcError>
    where
        R: tokio::io::AsyncRead + Unpin + Send + 'static,
        W: tokio::io::AsyncWrite + Unpin + Send + 'static,
    {
        self.router
            .connect_ndjson_stream(reader, writer, expected_body_id)
            .await
    }

    /// Connects a configured stdio child process body.
    pub async fn connect_stdio_body(
        &self,
        command: &str,
        args: &[String],
    ) -> Result<BodyId, BodyRpcError> {
        self.router.connect_stdio_process(command, args).await
    }

    /// Connects a configured Unix socket body.
    pub async fn connect_unix_body(&self, path: &std::path::Path) -> Result<BodyId, BodyRpcError> {
        self.router.connect_unix_socket(path).await
    }

    /// Reads all events currently persisted in Core's store via the owner task.
    pub async fn read_events(&self) -> Result<Vec<EventRecord>, GatewayError> {
        let (tx, rx) = oneshot::channel();
        self.control_tx
            .send(ControlCommand::ReadEvents { reply: tx })
            .map_err(|_| GatewayError::OwnerExited)?;
        rx.await.map_err(|_| GatewayError::OwnerExited)?
    }

    /// Queries `Core::next_due()` via the owner task.
    pub async fn next_due(&self) -> Result<Option<Time>, GatewayError> {
        let (tx, rx) = oneshot::channel();
        self.control_tx
            .send(ControlCommand::NextDue { reply: tx })
            .map_err(|_| GatewayError::OwnerExited)?;
        rx.await.map_err(|_| GatewayError::OwnerExited)
    }

    /// Registers a `BodyId` as an attested out-of-band owner control channel for `approval/resolve`.
    pub fn register_out_of_band_control_body(&self, body_id: BodyId) {
        self.router.register_out_of_band_control_body(body_id);
    }

    /// Submits an attested out-of-band `approval/resolve` command directly into `OwnerTaskContext`.
    pub async fn resolve_approval(
        &self,
        params: ApprovalResolveParams,
    ) -> Result<ApprovalResolveResult, Waist2Error> {
        let (tx, rx) = oneshot::channel();
        self.control_tx
            .send(ControlCommand::ResolveApproval {
                params: Box::new(params),
                from_in_band_body: None,
                reply: tx,
            })
            .map_err(|_| Waist2Error::invalid_receipt_transition("owner task exited"))?;
        rx.await
            .map_err(|_| Waist2Error::invalid_receipt_transition("owner task exited"))?
    }

    /// Returns the shared [`IngressQueue`].
    #[must_use]
    pub fn ingress(&self) -> &IngressQueue {
        &self.ingress
    }

    /// Returns the [`BodyRouter`].
    #[must_use]
    pub fn router(&self) -> &BodyRouter {
        &self.router
    }

    /// Returns the [`DiagnosticSink`].
    #[must_use]
    pub fn diagnostics(&self) -> &DiagnosticSink {
        &self.diagnostics
    }

    /// Returns the honest [`CueRecorder`].
    #[must_use]
    pub fn cues(&self) -> &CueRecorder {
        &self.cues
    }

    /// Returns the [`DailyBudgetTracker`].
    #[must_use]
    pub fn budget(&self) -> &DailyBudgetTracker {
        &self.budget
    }

    /// Returns single-owner execution statistics (`max_concurrent_calls` MUST be `<= 1`).
    #[must_use]
    pub fn ownership_stats(&self) -> &OwnershipStats {
        &self.ownership_stats
    }

    /// Returns the injected [`Clock`].
    #[must_use]
    pub fn clock(&self) -> &Arc<dyn Clock> {
        &self.clock
    }

    /// Returns the wired [`GatewaySeams`].
    #[must_use]
    pub fn seams(&self) -> &GatewaySeams {
        &self.seams
    }

    /// Returns the wired [`PreQueueService`], if configured.
    #[must_use]
    pub fn pre_queue_service(&self) -> Option<&Arc<PreQueueService>> {
        self.seams.pre_queue_service.as_ref()
    }

    /// Gracefully shuts down connected bodies and stops the Core owner task.
    pub async fn shutdown(&self) {
        self.router.shutdown_all(self.shutdown_grace_ms).await;
        let (tx, rx) = oneshot::channel();
        if self
            .control_tx
            .send(ControlCommand::Shutdown { reply: tx })
            .is_ok()
        {
            let _ = rx.await;
        }
    }
}

/// Optional runtime builders for [`GatewayRuntime::start`].
pub struct GatewayOptions {
    pub clock: Arc<dyn Clock>,
    pub provider: Option<Arc<dyn ModelProvider>>,
    pub diagnostics: DiagnosticSink,
    pub cues: CueRecorder,
    pub seams: GatewaySeams,
}

/// Running Gateway instance owning the handle and background Core owner task.
pub struct GatewayRuntime {
    handle: GatewayHandle,
    owner_join: Option<tokio::task::JoinHandle<()>>,
}

impl GatewayRuntime {
    /// Starts the Gateway with one dedicated Core owner task that calls `Core::open` and `Core::step`.
    pub async fn start<S, F, D>(
        config: GatewayConfig,
        secrets: &monoagens_reason::ProviderSecrets,
        open_store: F,
        core_config: CoreConfig,
        doors: D,
        mut options: GatewayOptions,
    ) -> Result<Self, GatewayError>
    where
        S: Store + Send + 'static,
        F: FnOnce() -> Result<S, StoreError> + Send + 'static,
        D: DoorController + 'static,
    {
        core_config
            .validate()
            .map_err(|e| GatewayError::CoreConfig(e.to_string()))?;
        let neural_model = load_neural_model_at_edge(&config, &core_config)?;
        options.diagnostics.register_secrets(secrets);

        if options.seams.identity.is_none() && !config.persons.is_empty() {
            let log_path = config
                .data_dir
                .as_ref()
                .map(|d| d.join("identity_events.jsonl"));
            let resolver = IdentityResolver::from_gateway_config(&config, log_path)
                .map_err(|e| GatewayError::ProviderInit(e.to_string()))?;
            options.seams.identity = Some(Arc::new(resolver));
        }

        if options.seams.pre_queue_service.is_none()
            && options.seams.pre_queue_index.is_none()
            && config.embed.backend != EmbedBackendKind::None
        {
            let embedder = build_embedder_from_config_async(&config, secrets)
                .await
                .map_err(|e| GatewayError::ProviderInit(e.to_string()))?;
            let info_gap = neural_model.as_ref().and_then(|m| {
                m.info_gap.as_ref().and_then(|ig| {
                    InfoGapClassifier::from_weights(
                        ig.w_gap.clone(),
                        ig.b_gap,
                        ig.expected_tag.clone(),
                    )
                    .ok()
                })
            });
            let service = Arc::new(PreQueueService::new(
                embedder,
                info_gap,
                None,
                options.diagnostics.clone(),
                Arc::clone(&options.clock),
                config.embed.timeout_ms,
            ));
            options.seams.pre_queue_service = Some(service);
        }

        let provider = match options.provider {
            Some(p) => p,
            None => build_provider_from_config_async(&config, secrets)
                .await
                .map_err(|e| GatewayError::ProviderInit(e.to_string()))?,
        };

        let budget = DailyBudgetTracker::new(options.clock.now(), config.model.daily_token_budget);
        let model_exec = GatewayModelExecutor::with_provider(
            provider,
            &config,
            budget.clone(),
            options.diagnostics.clone(),
            options.cues.clone(),
            Arc::clone(&options.clock),
        );

        let seams_for_handle = options.seams.clone();
        let pre_queue_for_owner = options.seams.pre_queue_service.clone();
        let ingress = IngressQueue::new();
        let router = BodyRouter::new(
            ingress.clone(),
            options.seams,
            options.diagnostics.clone(),
            options.cues.clone(),
            Arc::clone(&options.clock),
            config.runtime.action_timeout_ms,
            config.runtime.hook_timeout_ms,
            config.runtime.working_cue_ms,
            config.runtime.progress_note_ms,
        );

        let (control_tx, control_rx) = mpsc::unbounded_channel();
        router.set_control_tx(control_tx.clone());
        let (ready_tx, ready_rx) = oneshot::channel();
        let ownership_stats = OwnershipStats::default();

        let owner_ctx = OwnerTaskContext {
            doors,
            ingress: ingress.clone(),
            control_rx,
            router: router.clone(),
            model_exec,
            budget: budget.clone(),
            diagnostics: options.diagnostics.clone(),
            clock: Arc::clone(&options.clock),
            stats: ownership_stats.clone(),
            pre_queue_service: pre_queue_for_owner,
        };

        let owner_join = tokio::spawn(async move {
            run_core_owner_task(open_store, core_config, neural_model, owner_ctx, ready_tx).await;
        });

        ready_rx.await.map_err(|_| GatewayError::OwnerExited)??;

        let handle = GatewayHandle {
            ingress,
            control_tx,
            router,
            diagnostics: options.diagnostics,
            cues: options.cues,
            budget,
            clock: options.clock,
            seams: seams_for_handle,
            ownership_stats,
            shutdown_grace_ms: config.runtime.body_shutdown_grace_ms,
        };

        Ok(Self {
            handle,
            owner_join: Some(owner_join),
        })
    }

    /// Convenience starter opening a durable [`SegmentedStore`] rooted at `store_dir`.
    pub async fn start_segmented<D>(
        config: GatewayConfig,
        secrets: &monoagens_reason::ProviderSecrets,
        store_dir: std::path::PathBuf,
        core_config: CoreConfig,
        doors: D,
        options: GatewayOptions,
    ) -> Result<Self, GatewayError>
    where
        D: DoorController + 'static,
    {
        Self::start(
            config,
            secrets,
            move || SegmentedStore::open(StoreConfig::new(store_dir)),
            core_config,
            doors,
            options,
        )
        .await
    }

    /// Starts the Gateway loading `[soul] core_config` and `[soul] model_path` at the edge,
    /// failing startup on invalid config, missing `.safetensors`, or `BLAKE3` mismatch.
    pub async fn start_with_configured_core<S, F, D>(
        config: GatewayConfig,
        secrets: &monoagens_reason::ProviderSecrets,
        open_store: F,
        doors: D,
        options: GatewayOptions,
    ) -> Result<Self, GatewayError>
    where
        S: Store + Send + 'static,
        F: FnOnce() -> Result<S, StoreError> + Send + 'static,
        D: DoorController + 'static,
    {
        let loaded_cfg = config.load_core_config()?;
        Self::start(config, secrets, open_store, loaded_cfg.config, doors, options).await
    }

    /// Convenience starter opening a durable [`SegmentedStore`] rooted at `store_dir` and
    /// loading `[soul] core_config` + optional `.safetensors` weights at the edge.
    pub async fn start_segmented_with_configured_core<D>(
        config: GatewayConfig,
        secrets: &monoagens_reason::ProviderSecrets,
        store_dir: std::path::PathBuf,
        doors: D,
        options: GatewayOptions,
    ) -> Result<Self, GatewayError>
    where
        D: DoorController + 'static,
    {
        let loaded_cfg = config.load_core_config()?;
        Self::start_segmented(config, secrets, store_dir, loaded_cfg.config, doors, options).await
    }

    /// Returns a cloneable [`GatewayHandle`].
    #[must_use]
    pub fn handle(&self) -> GatewayHandle {
        self.handle.clone()
    }

    /// Gracefully shuts down the Gateway and awaits the Core owner task exit.
    pub async fn shutdown(mut self) {
        self.handle.shutdown().await;
        if let Some(join) = self.owner_join.take() {
            let _ = join.await;
        }
    }
}

struct OwnerTaskContext<D: DoorController> {
    doors: D,
    ingress: IngressQueue,
    control_rx: mpsc::UnboundedReceiver<ControlCommand>,
    router: BodyRouter,
    model_exec: GatewayModelExecutor,
    budget: DailyBudgetTracker,
    diagnostics: DiagnosticSink,
    clock: Arc<dyn Clock>,
    stats: OwnershipStats,
    pre_queue_service: Option<Arc<PreQueueService>>,
}

fn load_neural_model_at_edge(
    gateway_cfg: &GatewayConfig,
    core_cfg: &CoreConfig,
) -> Result<Option<Arc<NeuralModel>>, GatewayError> {
    let model_path = gateway_cfg
        .soul
        .model_path
        .clone()
        .or_else(|| core_cfg.model_path.as_ref().map(std::path::PathBuf::from));
    let Some(path) = model_path else {
        return Ok(None);
    };
    let expected = match &gateway_cfg.soul.expected_blake3 {
        Some(hex) => Some(
            GatewayConfig::parse_blake3_hex(hex)
                .map_err(|e| GatewayError::NeuralModel(e.to_string()))?,
        ),
        None => core_cfg.expected_blake3,
    };
    let bytes = std::fs::read(&path).map_err(|e| {
        GatewayError::NeuralModel(format!(
            "failed to read safetensors model '{}': {e}",
            path.display()
        ))
    })?;
    let model = NeuralModel::from_safetensors_bytes(&bytes, expected)
        .map_err(|e| GatewayError::NeuralModel(e.to_string()))?;
    Ok(Some(Arc::new(model)))
}

async fn run_core_owner_task<S, F, D>(
    open_store: F,
    core_config: CoreConfig,
    neural_model: Option<Arc<NeuralModel>>,
    mut ctx: OwnerTaskContext<D>,
    ready_tx: oneshot::Sender<Result<(), GatewayError>>,
) where
    S: Store + 'static,
    F: FnOnce() -> Result<S, StoreError>,
    D: DoorController,
{
    let store = match open_store() {
        Ok(s) => s,
        Err(err) => {
            let _ = ready_tx.send(Err(GatewayError::Store(err.to_string())));
            return;
        }
    };

    let mut core = match Core::open(store, core_config, neural_model) {
        Ok(c) => c,
        Err(err) => {
            let _ = ready_tx.send(Err(GatewayError::Core(err.to_string())));
            return;
        }
    };

    let (used, limit) = ctx.budget.snapshot(ctx.clock.now());
    core.set_daily_budget(used as f64, limit as f64);
    if let Some(ig) = core.neural_model().and_then(|m| m.info_gap.as_ref()) {
        core.set_dialogue_act_head(Some(ig.to_frozen_head()));
    }

    if let Ok(events) = core.store().read_in_order(None, usize::MAX) {
        ctx.doors.replay_from_store_events(&events);
    }
    ctx.doors.sync_state_epoch(core.state().epoch);

    if core.state().startup_recovery_pending {
        let now = ctx.clock.now();
        ctx.doors.sync_runtime_state(
            core.state().last_seq,
            now,
            used,
            &ctx.router.all_capabilities(),
        );
        ctx.doors.sync_state_epoch(core.state().epoch);
        ctx.stats.enter_step();
        let recovery_res = core.recover_startup(now, &mut ctx.doors);
        ctx.stats.exit_step();
        persist_door_approval_events(&mut core, &mut ctx);
        match recovery_res {
            Ok(effects) => spawn_effects(&core, &ctx, effects),
            Err(err) => {
                ctx.diagnostics.log(
                    now,
                    LogLevel::Error,
                    "gateway.core",
                    &format!("startup recovery failed: {err}"),
                );
            }
        }
    }

    let _ = ready_tx.send(Ok(()));

    let mut last_fired_due: Option<Time> = None;

    loop {
        // 1. Drain all pending ingress items in strict A > B > C order.
        while let Some((_class, env)) = ctx.ingress.try_pop() {
            let now = ctx.clock.now();
            let (used, limit) = ctx.budget.snapshot(now);
            core.set_daily_budget(used as f64, limit as f64);
            ctx.doors.sync_runtime_state(
                core.state().last_seq,
                now,
                used,
                &ctx.router.all_capabilities(),
            );
            ctx.doors.sync_state_epoch(core.state().epoch);

            let prev_seq = core.state().last_seq;
            let is_tick = matches!(env.input, Input::Tick);
            let is_body_state = matches!(env.input, Input::BodyState(_));
            ctx.stats.enter_step();
            let step_res = core.step(env.input, now, &mut ctx.doors);
            ctx.stats.exit_step();
            persist_door_approval_events(&mut core, &mut ctx);
            if core.state().last_seq > prev_seq
                && let Ok(new_events) = core.store().read_after_seq(Some(prev_seq), usize::MAX)
            {
                ctx.doors.sync_item_audiences(&new_events);
            }

            match step_res {
                Ok(effects) => {
                    if is_body_state && let Some(pq) = &ctx.pre_queue_service {
                        for entry in pq.core_capability_catalog() {
                            core.register_capability_item(entry);
                        }
                    }
                    let effects_spawned = effects.len();
                    spawn_effects(&core, &ctx, effects);
                    if !is_tick {
                        last_fired_due = None;
                    }
                    if let Some(ack) = env.ack {
                        let _ = ack.send(Ok(TurnOutcome {
                            effects_spawned,
                            next_due: core.next_due(),
                        }));
                    }
                }
                Err(err) => {
                    let msg = ctx.diagnostics.redact(&err.to_string());
                    ctx.diagnostics.log(
                        now,
                        LogLevel::Error,
                        "gateway.core",
                        &format!("Core::step failed: {msg}"),
                    );
                    if let Some(ack) = env.ack {
                        let _ = ack.send(Err(GatewayError::Core(msg)));
                    }
                }
            }
        }

        // 2. Drain non-blocking control commands before sleeping.
        while let Ok(cmd) = ctx.control_rx.try_recv() {
            if handle_control_command(cmd, &mut core, &mut ctx) {
                return;
            }
        }

        // 3. Compute nearest due time from Core::next_due().
        let next_due = core.next_due();
        let active_due = next_due.filter(|due| last_fired_due != Some(*due));
        if next_due != last_fired_due {
            last_fired_due = None;
        }

        // 4. Wait for next ingress, control command, or Core::next_due() timer.
        let due_fut = async {
            match active_due {
                Some(deadline) => ctx.clock.sleep_until(deadline).await,
                None => std::future::pending::<()>().await,
            }
        };

        tokio::select! {
            biased;
            cmd = ctx.control_rx.recv() => {
                match cmd {
                    Some(c) => {
                        if handle_control_command(c, &mut core, &mut ctx) {
                            return;
                        }
                    }
                    None => {
                        return;
                    }
                }
            }
            _ = ctx.ingress.notified() => {}
            _ = due_fut => {
                if let Some(due_time) = active_due {
                    last_fired_due = Some(due_time);
                    if let Err(err) = ctx.ingress.try_submit(Input::Tick) {
                        ctx.diagnostics.log(
                            ctx.clock.now(),
                            LogLevel::Warn,
                            "gateway.timer",
                            &format!("failed to enqueue scheduled Tick: {err}"),
                        );
                    }
                }
            }
        }
    }
}

fn persist_door_approval_events<S: Store, D: DoorController>(
    core: &mut Core<S>,
    ctx: &mut OwnerTaskContext<D>,
) {
    let events = ctx.doors.take_pending_approval_events();
    for ev in events {
        if let Err(err) = core.store_mut().append(ev) {
            ctx.diagnostics.log(
                ctx.clock.now(),
                LogLevel::Error,
                "gateway.approval",
                &format!("failed to persist approval event: {err}"),
            );
        }
    }
}

fn handle_control_command<S: Store, D: DoorController>(
    cmd: ControlCommand,
    core: &mut Core<S>,
    ctx: &mut OwnerTaskContext<D>,
) -> bool {
    match cmd {
        ControlCommand::ReadEvents { reply } => {
            let res = core
                .store()
                .read_in_order(None, usize::MAX)
                .map_err(GatewayError::from);
            let _ = reply.send(res);
            false
        }
        ControlCommand::NextDue { reply } => {
            let _ = reply.send(core.next_due());
            false
        }
        ControlCommand::ResolveApproval {
            params,
            from_in_band_body,
            reply,
        } => {
            let params = *params;
            let now = ctx.clock.now();
            let (used, _limit) = ctx.budget.snapshot(now);
            ctx.doors.sync_runtime_state(
                core.state().last_seq,
                now,
                used,
                &ctx.router.all_capabilities(),
            );
            ctx.doors.sync_state_epoch(core.state().epoch);

            if let Some(bad_body) = from_in_band_body {
                ctx.doors.record_in_band_rejection_audit(
                    &params,
                    &format!(
                        "approval/resolve rejected from untrusted or in-band body '{}'",
                        bad_body.as_str()
                    ),
                );
                persist_door_approval_events(core, ctx);
                let _ = reply.send(Err(Waist2Error::invalid_receipt_transition(
                    "approval/resolve rejected from untrusted or in-band chat body",
                )));
                return false;
            }

            let Some(resolve_res) = ctx.doors.resolve_approval(&params) else {
                let _ = reply.send(Err(Waist2Error {
                    code: -32601,
                    message: "approval/resolve capability is not wired in this Gateway".to_owned(),
                    data: None,
                }));
                return false;
            };

            // Record-first: persist resolution / rejection audit events before any dispatch.
            persist_door_approval_events(core, ctx);

            let res = match resolve_res {
                Err(werr) => Err(werr),
                Ok(ok_res) => {
                    if params.approved {
                        if let Some(mut action) = ctx.doors.take_approved_action(&params.action_id)
                        {
                            let door_dec = ctx.doors.action(&mut action);
                            persist_door_approval_events(core, ctx);
                            match door_dec {
                                DoorDecision::Allow | DoorDecision::Strip { .. } => {
                                    ctx.router.spawn_dispatch_action(action);
                                    Ok(ok_res)
                                }
                                DoorDecision::Refuse { reason, .. }
                                | DoorDecision::Hold { reason, .. } => {
                                    Err(Waist2Error::invalid_receipt_transition(reason))
                                }
                            }
                        } else {
                            Ok(ok_res)
                        }
                    } else {
                        if let Some(receipt) =
                            ctx.doors.take_pending_denial_receipt(&params.action_id)
                        {
                            ctx.stats.enter_step();
                            let step_res = core.step(Input::Receipt(receipt), now, &mut ctx.doors);
                            ctx.stats.exit_step();
                            persist_door_approval_events(core, ctx);
                            if let Ok(effects) = step_res {
                                spawn_effects(core, ctx, effects);
                            }
                        }
                        Ok(ok_res)
                    }
                }
            };
            let _ = reply.send(res);
            false
        }
        ControlCommand::Shutdown { reply } => {
            let _ = reply.send(());
            true
        }
    }
}

/// Maps a V2.2 neural [`ComputeTier`] to a per-request OpenAI-compatible [`ReasoningEffort`].
#[must_use]
pub const fn map_compute_tier_to_reasoning_effort(tier: ComputeTier) -> ReasoningEffort {
    match tier {
        ComputeTier::Fast => ReasoningEffort::Low,
        ComputeTier::Standard => ReasoningEffort::Medium,
        ComputeTier::Deep => ReasoningEffort::High,
    }
}

fn spawn_effects<S: Store, D: DoorController>(
    core: &Core<S>,
    ctx: &OwnerTaskContext<D>,
    effects: Vec<Effect>,
) {
    for effect in effects {
        match effect {
            Effect::Model { purpose, plan } => {
                let mut req = ctx.model_exec.prepare_request(purpose, &plan, core.store());
                if let Some(tier) = plan.compute_tier {
                    req.reasoning_effort = Some(map_compute_tier_to_reasoning_effort(tier));
                }
                let exec = ctx.model_exec.clone();
                let ingress = ctx.ingress.clone();
                let diagnostics = ctx.diagnostics.clone();
                let clock = Arc::clone(&ctx.clock);
                tokio::spawn(async move {
                    let input = exec.execute_prepared(req).await;
                    if let Err(err) = ingress.try_submit(input) {
                        diagnostics.log(
                            clock.now(),
                            LogLevel::Error,
                            "gateway.model",
                            &format!("failed to enqueue model result into ingress: {err}"),
                        );
                    }
                });
            }
            Effect::Dispatch(action) => {
                ctx.router.spawn_dispatch_action(action);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compute_tier_maps_cleanly_to_reasoning_effort() {
        assert_eq!(
            map_compute_tier_to_reasoning_effort(ComputeTier::Fast),
            ReasoningEffort::Low
        );
        assert_eq!(
            map_compute_tier_to_reasoning_effort(ComputeTier::Standard),
            ReasoningEffort::Medium
        );
        assert_eq!(
            map_compute_tier_to_reasoning_effort(ComputeTier::Deep),
            ReasoningEffort::High
        );
    }

    #[test]
    fn info_gap_head_converts_cleanly_to_reason_classifier_and_core_head() {
        let head = monoagens_core::neural::InfoGapHeadWeights {
            w_gap: vec![0.5, -0.25, 0.1],
            b_gap: -0.4,
            expected_tag: Some(monoagens_protocol::EmbeddingTag {
                embed_model_ver: "egemma2-256".to_owned(),
                dim_d: 3,
                prompt_regime_id: "t_clf".to_owned(),
            }),
        };
        let clf = InfoGapClassifier::from_weights(
            head.w_gap.clone(),
            head.b_gap,
            head.expected_tag.clone(),
        )
        .expect("valid info gap head");
        let core_head = head.to_frozen_head();
        assert_eq!(core_head.w_gap, clf.w_gap());
        assert_eq!(core_head.b_gap, clf.b_gap());
        assert!((clf.neutral_prior_score() - 0.401_312_34).abs() < 1e-5);
    }
}
