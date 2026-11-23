//! Model provider wiring (`scripted` / stateless OpenAI-compatible HTTP `store: false`),
//! daily token budget tracking, and bounded async execution over `crates/reason`.

use core::fmt;
use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use monoagens_core::{ContextPlan, Input, ModelPurpose, Store};
use monoagens_protocol::{EventId, Time};
use monoagens_reason::model::catalog::{self, Dialect};
use monoagens_reason::{
    MessagesModelConfig, MessagesModelProvider,
    CallOutcome, ModelCallFailure, ModelCallRecord, ModelCallSuccess, ModelOutput, ModelProvider, OpenAiModelConfig,
    OpenAiModelProvider, ProviderFailureClass, ProviderSecrets, RenderExtras,
    ScriptedModelProvider, build_model_request_from_plan,
};

use crate::clock::{Clock, run_with_clock_timeout};
use crate::config::{DEFAULT_OPENAI_ENDPOINT, GatewayConfig, ModelProviderKind};
use crate::cues::{CueRecorder, spawn_wait_cue_monitor};
use crate::logging::{DiagnosticSink, LogLevel};
use crate::runtime::IngressQueue;

/// Errors constructing a [`ModelProvider`] from Gateway configuration and secrets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderInitError {
    pub detail: String,
}

impl fmt::Display for ProviderInitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "provider initialization failed: {}", self.detail)
    }
}

impl std::error::Error for ProviderInitError {}

/// UTC-day token budget tracker keyed by `clock.now()`.
#[derive(Debug, Clone)]
pub struct DailyBudgetTracker {
    state: Arc<Mutex<DailyBudgetState>>,
}

#[derive(Debug, Clone)]
struct DailyBudgetState {
    utc_day: u64,
    used_tokens: u64,
    daily_limit: u64,
    usage: BTreeMap<(&'static str, String, String), UsageCounts>,
}

/// Calls and tokens of one `(purpose, provider, model)` today.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UsageCounts {
    pub calls: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

/// One row of [`DailyBudgetTracker::usage_snapshot`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsageRow {
    pub purpose: &'static str,
    /// Catalog provider id (or the legacy provider kind).
    pub provider: String,
    pub model: String,
    pub counts: UsageCounts,
}

/// Snake-case name of a [`ModelPurpose`] (the `[model.purposes.<name>]` key).
#[must_use]
pub const fn purpose_name(purpose: ModelPurpose) -> &'static str {
    match purpose {
        ModelPurpose::Reason => "reason",
        ModelPurpose::Classify => "classify",
        ModelPurpose::Consolidate => "consolidate",
    }
}

impl DailyBudgetTracker {
    /// Creates a new tracker for `daily_limit` starting at `now`.
    #[must_use]
    pub fn new(now: Time, daily_limit: u64) -> Self {
        Self {
            state: Arc::new(Mutex::new(DailyBudgetState {
                utc_day: utc_day_of(now),
                used_tokens: 0,
                daily_limit: daily_limit.max(1),
                usage: BTreeMap::new(),
            })),
        }
    }

    /// Records `tokens` at `now`, rolling over cleanly at UTC midnight, and returns `(used, limit)`.
    pub fn record_tokens(&self, now: Time, tokens: u64) -> (u64, u64) {
        let mut st = self.state.lock().expect("daily budget lock");
        let day = utc_day_of(now);
        if day != st.utc_day {
            st.utc_day = day;
            st.used_tokens = 0;
            st.usage.clear();
        }
        st.used_tokens = st.used_tokens.saturating_add(tokens);
        (st.used_tokens, st.daily_limit)
    }

    /// Records one completed model call (tokens toward the daily budget plus the per-purpose
    /// counters shown by `/usage`) and returns `(used, limit)`.
    pub fn record_call(
        &self,
        now: Time,
        purpose: ModelPurpose,
        provider: &str,
        model: &str,
        usage: Option<monoagens_reason::TokenUsage>,
    ) -> (u64, u64) {
        let out = self.record_tokens(now, usage.map_or(0, |u| u.total_tokens));
        let mut st = self.state.lock().expect("daily budget lock");
        let c = st
            .usage
            .entry((purpose_name(purpose), provider.to_owned(), model.to_owned()))
            .or_default();
        c.calls += 1;
        if let Some(u) = usage {
            c.input_tokens = c.input_tokens.saturating_add(u.input_tokens);
            c.output_tokens = c.output_tokens.saturating_add(u.output_tokens);
        }
        out
    }

    /// Today's usage per `(purpose, provider, model)`.
    #[must_use]
    pub fn usage_snapshot(&self, now: Time) -> Vec<UsageRow> {
        let mut st = self.state.lock().expect("daily budget lock");
        let day = utc_day_of(now);
        if day != st.utc_day {
            st.utc_day = day;
            st.used_tokens = 0;
            st.usage.clear();
        }
        st.usage
            .iter()
            .map(|((purpose, provider, model), counts)| UsageRow {
                purpose,
                provider: provider.clone(),
                model: model.clone(),
                counts: *counts,
            })
            .collect()
    }

    /// Returns current `(used_tokens, daily_limit)` at `now`.
    #[must_use]
    pub fn snapshot(&self, now: Time) -> (u64, u64) {
        let mut st = self.state.lock().expect("daily budget lock");
        let day = utc_day_of(now);
        if day != st.utc_day {
            st.utc_day = day;
            st.used_tokens = 0;
            st.usage.clear();
        }
        (st.used_tokens, st.daily_limit)
    }
}

const fn utc_day_of(time: Time) -> u64 {
    time.as_millis() / 86_400_000
}

/// Shared `openai_chat` config from `[model]` settings.
fn openai_config(
    config: &GatewayConfig,
    endpoint: String,
    key: Option<monoagens_reason::SecretString>,
) -> OpenAiModelConfig {
    let mut oa_cfg = OpenAiModelConfig::new(endpoint, key);
    oa_cfg.default_timeout_ms = config.runtime.model_timeout_ms.max(1);
    oa_cfg.output_limits = config.model.output_limits.clone();
    oa_cfg.output_limit_param = config.model.output_limit_param.clone();
    oa_cfg.usage_paths = config.model.usage_paths.clone();
    oa_cfg.profile_name = Some(config.model.model.clone());
    oa_cfg.extra_headers = config
        .model
        .extra_headers
        .iter()
        .map(|(k, v)| (k.clone(), monoagens_reason::ExtraHeaderValue::parse(v)))
        .collect();
    oa_cfg
}

/// Builds a provider for catalog entry `id`: dialect, endpoint (catalog base URL or
/// `[model.provider_options] base_url`) and key (secrets, then env names, then legacy key).
fn build_catalog_provider(
    config: &GatewayConfig,
    secrets: &ProviderSecrets,
    id: &str,
) -> Result<Arc<dyn ModelProvider>, ProviderInitError> {
    let entry = catalog::find(id).ok_or_else(|| ProviderInitError {
        detail: format!("unknown catalog provider '{id}'"),
    })?;
    let base = config.model.provider_base_url.as_deref();
    if entry.effective_base_url(base).is_empty() {
        return Err(ProviderInitError {
            detail: format!(
                "provider '{id}' needs [model.provider_options] base_url (no catalog default)"
            ),
        });
    }
    let endpoint = entry.chat_endpoint(base);
    let key = secrets.api_key_for(entry, |n| std::env::var(n).ok());
    match entry.dialect {
        Dialect::OpenAiChat => {
            let mut oa_cfg = openai_config(config, endpoint, key);
            // Catalog entry's limit parameter applies unless the user set one explicitly.
            if let Some(param) = &entry.output_limit_param
                && config.model.output_limit_param == monoagens_reason::DEFAULT_OUTPUT_LIMIT_PARAM
            {
                oa_cfg.output_limit_param = param.clone();
            }
            let provider =
                OpenAiModelProvider::new(oa_cfg).map_err(|detail| ProviderInitError { detail })?;
            Ok(Arc::new(provider))
        }
        Dialect::MessagesApi => {
            let mut a_cfg = MessagesModelConfig::new(endpoint, key);
            if let Some(h) = &entry.key_header {
                a_cfg.key_header = h.clone();
            }
            a_cfg.extra_headers = entry
                .extra_headers
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            a_cfg.default_timeout_ms = config.runtime.model_timeout_ms.max(1);
            a_cfg.output_limits = config.model.output_limits.clone();
            a_cfg.profile_name = Some(config.model.model.clone());
            let provider = MessagesModelProvider::new(a_cfg)
                .map_err(|detail| ProviderInitError { detail })?;
            Ok(Arc::new(provider))
        }
    }
}

/// Builds the configured [`ModelProvider`] (`scripted` or stateless `OpenAiModelProvider`).
///
/// For `scripted`, an empty [`ScriptedModelProvider`] is returned when callers don't inject one
/// directly via [`GatewayModelExecutor::with_provider`].
pub fn build_provider_from_config(
    config: &GatewayConfig,
    secrets: &ProviderSecrets,
) -> Result<Arc<dyn ModelProvider>, ProviderInitError> {
    match config.model.provider {
        ModelProviderKind::Scripted => Ok(Arc::new(ScriptedModelProvider::new())),
        ModelProviderKind::OpenAiCompatible => {
            if let Some(id) = config.model.catalog_provider.as_deref() {
                return build_catalog_provider(config, secrets, id);
            }
            let endpoint = secrets
                .model_endpoint
                .clone()
                .unwrap_or_else(|| DEFAULT_OPENAI_ENDPOINT.to_owned());
            let oa_cfg = openai_config(config, endpoint, secrets.model_api_key.clone());
            let provider =
                OpenAiModelProvider::new(oa_cfg).map_err(|detail| ProviderInitError { detail })?;
            Ok(Arc::new(provider))
        }
    }
}

/// Async-safe builder running `reqwest::blocking::Client` construction on `spawn_blocking`
/// so it never creates/drops an inner blocking runtime on an async worker thread.
pub async fn build_provider_from_config_async(
    config: &GatewayConfig,
    secrets: &ProviderSecrets,
) -> Result<Arc<dyn ModelProvider>, ProviderInitError> {
    if config.model.provider == ModelProviderKind::Scripted {
        return Ok(Arc::new(ScriptedModelProvider::new()));
    }
    let cfg = config.clone();
    let sec = secrets.clone();
    tokio::task::spawn_blocking(move || build_provider_from_config(&cfg, &sec))
        .await
        .map_err(|e| ProviderInitError {
            detail: format!("provider builder task panicked: {e}"),
        })?
}

/// Provider and model name for the main `[model]` pair and for each `[model.purposes.*]`
/// override. Built from config, swapped live into a [`LiveModel`] by `config.apply`.
#[derive(Clone)]
pub struct ModelRoutes {
    main: RouteSpec,
    purposes: HashMap<ModelPurpose, RouteSpec>,
}

/// Provider, model name and provider label (catalog id) of one route.
#[derive(Clone)]
struct RouteSpec {
    provider: Arc<dyn ModelProvider>,
    model: String,
    label: String,
}

/// Label of the main provider: catalog id, else the legacy provider kind.
fn main_label(config: &GatewayConfig) -> String {
    config
        .model
        .catalog_provider
        .clone()
        .unwrap_or_else(|| config.model.provider.as_str().to_owned())
}

impl ModelRoutes {
    /// Routes where every purpose uses `provider` and the configured main model (unless
    /// `[model.purposes.<purpose>]` names another model on the same provider).
    #[must_use]
    pub fn single(provider: Arc<dyn ModelProvider>, config: &GatewayConfig) -> Self {
        let mut purposes = HashMap::new();
        for (name, p) in &config.model.purposes {
            if p.catalog_provider.is_none()
                && let Some(purpose) = purpose_from_name(name)
            {
                purposes.insert(
                    purpose,
                    RouteSpec {
                        provider: Arc::clone(&provider),
                        model: p.model.clone(),
                        label: main_label(config),
                    },
                );
            }
        }
        Self {
            main: RouteSpec {
                provider,
                model: config.model.model.clone(),
                label: main_label(config),
            },
            purposes,
        }
    }
}

/// Maps a `[model.purposes.<name>]` key to the Core's [`ModelPurpose`].
#[must_use]
pub fn purpose_from_name(name: &str) -> Option<ModelPurpose> {
    match name {
        "reason" => Some(ModelPurpose::Reason),
        "classify" => Some(ModelPurpose::Classify),
        "consolidate" => Some(ModelPurpose::Consolidate),
        _ => None,
    }
}

/// Builds the main provider (or reuses `main`) and one provider per distinct
/// `[model.purposes.<purpose>] provider`. Blocking: call from `spawn_blocking`.
pub fn build_model_routes(
    config: &GatewayConfig,
    secrets: &ProviderSecrets,
    main: Option<Arc<dyn ModelProvider>>,
) -> Result<ModelRoutes, ProviderInitError> {
    let main = match main {
        Some(p) => p,
        None => build_provider_from_config(config, secrets)?,
    };
    let mut routes = ModelRoutes::single(main, config);
    for (name, p) in &config.model.purposes {
        let (Some(purpose), Some(id)) = (purpose_from_name(name), p.catalog_provider.as_deref())
        else {
            continue;
        };
        let mut sub = config.clone();
        sub.model.model.clone_from(&p.model);
        sub.model.provider_base_url = None;
        let provider = build_catalog_provider(&sub, secrets, id)?;
        routes.purposes.insert(
            purpose,
            RouteSpec {
                provider,
                model: p.model.clone(),
                label: id.to_owned(),
            },
        );
    }
    Ok(routes)
}

/// Async-safe [`build_model_routes`] (blocking client construction on `spawn_blocking`).
pub async fn build_model_routes_async(
    config: &GatewayConfig,
    secrets: &ProviderSecrets,
    main: Option<Arc<dyn ModelProvider>>,
) -> Result<ModelRoutes, ProviderInitError> {
    let cfg = config.clone();
    let sec = secrets.clone();
    tokio::task::spawn_blocking(move || build_model_routes(&cfg, &sec, main))
        .await
        .map_err(|e| ProviderInitError {
            detail: format!("provider builder task panicked: {e}"),
        })?
}

#[derive(Clone)]
struct Route {
    provider: BlockingSafeProvider,
    model: String,
    label: String,
}

struct LiveRoutes {
    main: Route,
    purposes: HashMap<ModelPurpose, Route>,
}

impl LiveRoutes {
    fn from_routes(r: ModelRoutes) -> Self {
        let wrap = |r: RouteSpec| Route {
            provider: BlockingSafeProvider(Arc::new(ProviderSlot(Some(r.provider)))),
            model: r.model,
            label: r.label,
        };
        Self {
            main: wrap(r.main),
            purposes: r.purposes.into_iter().map(|(k, v)| (k, wrap(v))).collect(),
        }
    }

    fn route(&self, purpose: ModelPurpose) -> &Route {
        self.purposes.get(&purpose).unwrap_or(&self.main)
    }
}

/// The swappable provider routes shared by the model executor and the gateway handle.
/// `config.apply` replaces them in place; the next model request uses the new ones.
#[derive(Clone)]
pub struct LiveModel {
    routes: Arc<Mutex<Arc<LiveRoutes>>>,
}

impl LiveModel {
    /// Creates a live model holding `routes`.
    #[must_use]
    pub fn new(routes: ModelRoutes) -> Self {
        Self {
            routes: Arc::new(Mutex::new(Arc::new(LiveRoutes::from_routes(routes)))),
        }
    }

    /// Swaps in new routes; in-flight calls keep the provider they started with.
    pub fn replace(&self, routes: ModelRoutes) {
        *self.routes.lock().expect("live model lock") = Arc::new(LiveRoutes::from_routes(routes));
    }

    fn snapshot(&self) -> Arc<LiveRoutes> {
        Arc::clone(&self.routes.lock().expect("live model lock"))
    }

    /// Provider label (catalog id) currently used for `purpose`.
    #[must_use]
    pub fn label_for(&self, purpose: ModelPurpose) -> String {
        self.snapshot().route(purpose).label.clone()
    }

    /// Provider and model name currently used for `purpose` (the titler uses `Classify`).
    #[must_use]
    pub fn route_for(&self, purpose: ModelPurpose) -> (Arc<dyn ModelProvider>, String) {
        let snap = self.snapshot();
        let r = snap.route(purpose);
        (
            Arc::clone(r.provider.0.0.as_ref().expect("provider present before drop")),
            r.model.clone(),
        )
    }
}


/// Reason code recorded in `Input::ModelFailure` when the owner cancels an in-flight turn.
pub const CANCELLED_BY_OWNER: &str = "cancelled_by_owner";

/// One in-flight model call. Exactly one of {result, cancel, timeout} may *claim* it and
/// submit an `Input` for its request id (atomic compare-and-set).
#[derive(Debug)]
struct InFlightCall {
    request_id: EventId,
    purpose: ModelPurpose,
    claimed: AtomicBool,
    cancelled: tokio::sync::Notify,
}

impl InFlightCall {
    fn try_claim(&self) -> bool {
        self.claimed
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }
}

/// Registry of in-flight model calls, shared by the executor and the owner-facing handle.
#[derive(Debug, Clone, Default)]
pub struct TurnRegistry {
    calls: Arc<Mutex<BTreeMap<u64, Arc<InFlightCall>>>>,
    next_key: Arc<AtomicU64>,
}

struct InFlightGuard {
    registry: TurnRegistry,
    key: u64,
}

impl Drop for InFlightGuard {
    fn drop(&mut self) {
        self.registry
            .calls
            .lock()
            .expect("turn registry lock")
            .remove(&self.key);
    }
}

impl TurnRegistry {
    fn register(&self, request_id: EventId, purpose: ModelPurpose) -> (Arc<InFlightCall>, InFlightGuard) {
        let key = self.next_key.fetch_add(1, Ordering::Relaxed);
        let call = Arc::new(InFlightCall {
            request_id,
            purpose,
            claimed: AtomicBool::new(false),
            cancelled: tokio::sync::Notify::new(),
        });
        self.calls
            .lock()
            .expect("turn registry lock")
            .insert(key, Arc::clone(&call));
        (
            call,
            InFlightGuard {
                registry: self.clone(),
                key,
            },
        )
    }

    /// Number of model calls currently in flight.
    #[must_use]
    pub fn in_flight(&self) -> usize {
        self.calls.lock().expect("turn registry lock").len()
    }

    /// Cancels every in-flight `Reason` model call that has not been settled yet: each one
    /// wins its claim, gets `Input::ModelFailure { reason: "cancelled_by_owner" }` submitted
    /// into `ingress`, and stops being awaited. Returns the cancelled request ids.
    pub fn cancel_reasoning(&self, ingress: &IngressQueue, diagnostics: &DiagnosticSink, now: Time) -> Vec<EventId> {
        let candidates: Vec<Arc<InFlightCall>> = self
            .calls
            .lock()
            .expect("turn registry lock")
            .values()
            .filter(|c| c.purpose == ModelPurpose::Reason)
            .cloned()
            .collect();
        let mut cancelled = Vec::new();
        for call in candidates {
            if !call.try_claim() {
                continue;
            }
            match ingress.try_submit(Input::ModelFailure {
                request_id: call.request_id,
                reason: CANCELLED_BY_OWNER.to_owned(),
            }) {
                Ok(_) => {
                    diagnostics.log(
                        now,
                        LogLevel::Info,
                        "gateway.model",
                        &format!("model call {} cancelled by owner", call.request_id),
                    );
                    cancelled.push(call.request_id);
                }
                Err(err) => {
                    diagnostics.log(
                        now,
                        LogLevel::Error,
                        "gateway.model",
                        &format!("failed to enqueue cancel for {}: {err}", call.request_id),
                    );
                }
            }
            call.cancelled.notify_one();
        }
        cancelled
    }
}

/// Guard ensuring any underlying blocking HTTP client (e.g. `reqwest::blocking::Client`)
/// is never dropped directly inside a Tokio async runtime context.
#[derive(Clone)]
struct BlockingSafeProvider(Arc<ProviderSlot>);

struct ProviderSlot(Option<Arc<dyn ModelProvider>>);

impl Drop for ProviderSlot {
    fn drop(&mut self) {
        if let Some(provider) = self.0.take()
            && tokio::runtime::Handle::try_current().is_ok()
        {
            std::thread::spawn(move || drop(provider));
        }
    }
}

/// Executes `Effect::Model` requests on `spawn_blocking` under `model_timeout_ms`, records
/// every call (model, purpose, tokens incl. cached, latency, outcome) and updates daily budget.
#[derive(Clone)]
pub struct GatewayModelExecutor {
    live: LiveModel,
    timeout_ms: u64,
    max_output_tokens: Option<u32>,
    output_limits: monoagens_reason::OutputLimitProfile,
    temperature: Option<f32>,
    working_cue_ms: u64,
    progress_note_ms: u64,
    budget: DailyBudgetTracker,
    diagnostics: DiagnosticSink,
    cues: CueRecorder,
    clock: Arc<dyn Clock>,
    turns: TurnRegistry,
}

impl GatewayModelExecutor {
    /// Creates a model executor wrapping `provider`.
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn with_provider(
        provider: Arc<dyn ModelProvider>,
        config: &GatewayConfig,
        budget: DailyBudgetTracker,
        diagnostics: DiagnosticSink,
        cues: CueRecorder,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self::with_routes(ModelRoutes::single(provider, config), config, budget, diagnostics, cues, clock)
    }

    /// Creates a model executor over explicit per-purpose `routes`.
    #[must_use]
    pub fn with_routes(
        routes: ModelRoutes,
        config: &GatewayConfig,
        budget: DailyBudgetTracker,
        diagnostics: DiagnosticSink,
        cues: CueRecorder,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            live: LiveModel::new(routes),
            timeout_ms: config.runtime.model_timeout_ms,
            max_output_tokens: config.model.max_output_tokens,
            output_limits: config.model.output_limits.clone(),
            temperature: config.model.temperature,
            working_cue_ms: config.runtime.working_cue_ms,
            progress_note_ms: config.runtime.progress_note_ms,
            budget,
            diagnostics,
            cues,
            clock,
            turns: TurnRegistry::default(),
        }
    }

    /// Returns the swappable provider routes (shared with the gateway handle).
    #[must_use]
    pub fn live_model(&self) -> &LiveModel {
        &self.live
    }

    /// Returns the registry of in-flight model calls (used by `turn.cancel`).
    #[must_use]
    pub fn turns(&self) -> &TurnRegistry {
        &self.turns
    }

    /// Renders `plan` synchronously against `store` inside the Core owner task, returning a
    /// self-contained [`monoagens_reason::ModelRequest`] ready to run asynchronously.
    pub fn prepare_request(
        &self,
        purpose: ModelPurpose,
        plan: &ContextPlan,
        store: &dyn Store,
    ) -> monoagens_reason::ModelRequest {
        let request_id = plan.trigger_event_id.unwrap_or(EventId::new(0));
        let mut extras = RenderExtras::default();
        if let Err(err) = extras.populate_payloads_from_store(plan, store) {
            self.diagnostics.log(
                self.clock.now(),
                LogLevel::Warn,
                "gateway.render",
                &format!("failed to populate ContextPlan payloads for {request_id}: {err}"),
            );
        }
        let model_name = self.live.snapshot().route(purpose).model.clone();
        let mut req =
            build_model_request_from_plan(plan, &extras, request_id, purpose, &model_name);
        req.timeout_ms = self.timeout_ms;
        req.max_output_tokens = Some(
            self.max_output_tokens
                .unwrap_or_else(|| self.output_limits.for_purpose(purpose)),
        );
        req.conversation_id = plan
            .place
            .as_ref()
            .or_else(|| {
                plan.visible_scope
                    .as_ref()
                    .and_then(|s| s.target_place.as_ref())
            })
            .map(|p| {
                format!(
                    "conv_{}",
                    &blake3::hash(p.as_str().as_bytes()).to_hex()[..32]
                )
            });
        req.temperature = self.temperature;
        req
    }

    /// Executes `req` off the Core owner task under `model_timeout_ms` and returns the
    /// resulting Core [`Input`] (`ModelResult`, `ClassifyResult`, or `ModelFailure`).
    ///
    /// If the call is cancelled by the owner meanwhile (see [`TurnRegistry::cancel_reasoning`],
    /// which submits the `ModelFailure` itself) this returns the same cancel failure; callers
    /// that submit the returned input must use [`Self::execute_prepared_cancellable`] instead
    /// to avoid submitting it twice.
    pub async fn execute_prepared(&self, req: monoagens_reason::ModelRequest) -> Input {
        let request_id = req.request_id;
        match self.execute_prepared_cancellable(req).await {
            Some(input) => input,
            None => Input::ModelFailure {
                request_id,
                reason: CANCELLED_BY_OWNER.to_owned(),
            },
        }
    }

    /// Like [`Self::execute_prepared`] but returns `None` when the owner cancelled the call:
    /// the cancel path already submitted `Input::ModelFailure { cancelled_by_owner }`, and a
    /// result that still arrives from the abandoned blocking call is discarded (logged
    /// "late result discarded"). Cancel, completion and timeout race through one atomic
    /// compare-and-set, so exactly one of them submits an input for the request.
    pub async fn execute_prepared_cancellable(
        &self,
        req: monoagens_reason::ModelRequest,
    ) -> Option<Input> {
        let request_id = req.request_id;
        let purpose = req.purpose;
        let model = req.model.clone();
        let timeout_ms = req.timeout_ms.max(1);
        let routes = self.live.snapshot();
        let provider_ref = routes
            .route(purpose)
            .provider
            .0
            .0
            .as_ref()
            .expect("provider present before drop");
        let provider_name = provider_ref.provider_name().to_owned();
        let route_label = routes.route(purpose).label.clone();
        let started_at = self.clock.now();

        let _cue_guard = spawn_wait_cue_monitor(
            Arc::clone(&self.clock),
            self.cues.clone(),
            format!("model:{request_id}"),
            None,
            self.working_cue_ms,
            self.progress_note_ms,
        );

        let (call, _in_flight_guard) = self.turns.register(request_id, purpose);
        let (res_tx, mut res_rx) =
            tokio::sync::oneshot::channel::<Result<ModelCallSuccess, ModelCallFailure>>();
        let provider = Arc::clone(provider_ref);
        let call_for_worker = Arc::clone(&call);
        let diagnostics = self.diagnostics.clone();
        let clock = Arc::clone(&self.clock);
        // The blocking HTTP call cannot be interrupted; on cancel/timeout it is abandoned and
        // finishes on its own thread, where losing the claim means its result is dropped.
        let _detached = tokio::task::spawn_blocking(move || {
            let outcome = provider.invoke(&req);
            if call_for_worker.try_claim() {
                let _ = res_tx.send(outcome);
            } else {
                diagnostics.log(
                    clock.now(),
                    LogLevel::Info,
                    "gateway.model",
                    &format!("late result discarded (request {})", call_for_worker.request_id),
                );
            }
        });

        let waited = tokio::select! {
            biased;
            () = call.cancelled.notified() => return None,
            res = run_with_clock_timeout(self.clock.as_ref(), timeout_ms, &mut res_rx) => res,
        };

        let waited = match waited {
            Err(_timed_out) => {
                if call.try_claim() {
                    return Some(self.timeout_failure(
                        request_id, purpose, provider_name, model, timeout_ms, started_at,
                    ));
                }
                // Lost the claim: either the owner cancelled, or the worker just delivered.
                tokio::select! {
                    biased;
                    () = call.cancelled.notified() => return None,
                    res = &mut res_rx => res,
                }
            }
            Ok(res) => res,
        };

        match waited {
            Err(_worker_dropped) => {
                if !call.try_claim() {
                    return None;
                }
                let elapsed = self.clock.now().saturating_sub_ms(started_at);
                let record = ModelCallRecord {
                    request_id,
                    purpose,
                    provider: provider_name,
                    model: model.clone(),
                    usage: None,
                    latency_ms: elapsed,
                    outcome: CallOutcome::Failed(ProviderFailureClass::Server),
                    strict_schema: true,
                };
                self.diagnostics.record_model_call(record);
                let msg = "model worker panicked".to_owned();
                self.diagnostics
                    .log(self.clock.now(), LogLevel::Error, "gateway.model", &msg);
                Some(Input::ModelFailure {
                    request_id,
                    reason: self.diagnostics.redact(&msg),
                })
            }
            Ok(Ok(success)) => {
                let tokens = success.record.usage.map_or(0, |u| u.total_tokens);
                let (_used, _limit) = self.budget.record_call(
                    self.clock.now(),
                    purpose,
                    &route_label,
                    &model,
                    success.record.usage,
                );
                self.diagnostics.record_model_call(success.record.clone());
                self.diagnostics.log(
                    self.clock.now(),
                    LogLevel::Info,
                    "gateway.model",
                    &format!(
                        "model call ok provider={} model={} purpose={:?} tokens={} cached={:?} latency_ms={}",
                        success.record.provider,
                        success.record.model,
                        success.record.purpose,
                        tokens,
                        success.record.usage.and_then(|u| u.cached_tokens),
                        success.record.latency_ms
                    ),
                );
                Some(match success.output {
                    ModelOutput::ThoughtActs(parsed) => Input::ModelResult {
                        request_id,
                        acts: parsed.acts,
                    },
                    ModelOutput::Verdict(verdict) => Input::ClassifyResult {
                        request_id,
                        verdict,
                    },
                })
            }
            Ok(Err(failure)) => {
                let mut record = *failure.record.clone();
                record.model = self.diagnostics.redact(&record.model);
                self.diagnostics.record_model_call(record);
                let redacted_reason = self.diagnostics.redact(&failure.to_string());
                self.diagnostics.log(
                    self.clock.now(),
                    LogLevel::Warn,
                    "gateway.model",
                    &redacted_reason,
                );
                Some(Input::ModelFailure {
                    request_id,
                    reason: redacted_reason,
                })
            }
        }
    }

    fn timeout_failure(
        &self,
        request_id: EventId,
        purpose: ModelPurpose,
        provider_name: String,
        model: String,
        timeout_ms: u64,
        started_at: Time,
    ) -> Input {
        let elapsed = self.clock.now().saturating_sub_ms(started_at);
        let record = ModelCallRecord {
            request_id,
            purpose,
            provider: provider_name,
            model: model.clone(),
            usage: None,
            latency_ms: elapsed,
            outcome: CallOutcome::Failed(ProviderFailureClass::Timeout),
            strict_schema: true,
        };
        self.diagnostics.record_model_call(record);
        let msg = format!(
            "model call timed out after {timeout_ms}ms (model '{model}', purpose {purpose:?})"
        );
        self.diagnostics
            .log(self.clock.now(), LogLevel::Warn, "gateway.model", &msg);
        Input::ModelFailure {
            request_id,
            reason: self.diagnostics.redact(&msg),
        }
    }

    /// Returns the shared [`DailyBudgetTracker`].
    #[must_use]
    pub fn budget(&self) -> &DailyBudgetTracker {
        &self.budget
    }
}
