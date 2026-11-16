//! Model provider wiring (`scripted` / stateless OpenAI-compatible HTTP `store: false`),
//! daily token budget tracking, and bounded async execution over `crates/reason`.

use core::fmt;
use std::sync::{Arc, Mutex};

use monoagens_core::{ContextPlan, Input, ModelPurpose, Store};
use monoagens_protocol::{EventId, Time};
use monoagens_reason::{
    CallOutcome, ModelCallRecord, ModelOutput, ModelProvider, OpenAiModelConfig,
    OpenAiModelProvider, ProviderFailureClass, ProviderSecrets, RenderExtras,
    ScriptedModelProvider, build_model_request_from_plan,
};

use crate::clock::{Clock, run_with_clock_timeout};
use crate::config::{DEFAULT_OPENAI_ENDPOINT, GatewayConfig, ModelProviderKind};
use crate::cues::{CueRecorder, spawn_wait_cue_monitor};
use crate::logging::{DiagnosticSink, LogLevel};

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

#[derive(Debug, Clone, Copy)]
struct DailyBudgetState {
    utc_day: u64,
    used_tokens: u64,
    daily_limit: u64,
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
        }
        st.used_tokens = st.used_tokens.saturating_add(tokens);
        (st.used_tokens, st.daily_limit)
    }

    /// Returns current `(used_tokens, daily_limit)` at `now`.
    #[must_use]
    pub fn snapshot(&self, now: Time) -> (u64, u64) {
        let mut st = self.state.lock().expect("daily budget lock");
        let day = utc_day_of(now);
        if day != st.utc_day {
            st.utc_day = day;
            st.used_tokens = 0;
        }
        (st.used_tokens, st.daily_limit)
    }
}

const fn utc_day_of(time: Time) -> u64 {
    time.as_millis() / 86_400_000
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
            let endpoint = secrets
                .model_endpoint
                .clone()
                .unwrap_or_else(|| DEFAULT_OPENAI_ENDPOINT.to_owned());
            let mut oa_cfg = OpenAiModelConfig::new(endpoint, secrets.model_api_key.clone());
            oa_cfg.default_timeout_ms = config.runtime.model_timeout_ms.max(1);
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
    provider: BlockingSafeProvider,
    model_name: String,
    timeout_ms: u64,
    max_output_tokens: Option<u32>,
    temperature: Option<f32>,
    working_cue_ms: u64,
    progress_note_ms: u64,
    budget: DailyBudgetTracker,
    diagnostics: DiagnosticSink,
    cues: CueRecorder,
    clock: Arc<dyn Clock>,
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
        Self {
            provider: BlockingSafeProvider(Arc::new(ProviderSlot(Some(provider)))),
            model_name: config.model.model.clone(),
            timeout_ms: config.runtime.model_timeout_ms,
            max_output_tokens: config.model.max_output_tokens,
            temperature: config.model.temperature,
            working_cue_ms: config.runtime.working_cue_ms,
            progress_note_ms: config.runtime.progress_note_ms,
            budget,
            diagnostics,
            cues,
            clock,
        }
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
        let mut req =
            build_model_request_from_plan(plan, &extras, request_id, purpose, &self.model_name);
        req.timeout_ms = self.timeout_ms;
        req.max_output_tokens = self.max_output_tokens;
        req.temperature = self.temperature;
        req
    }

    /// Executes `req` off the Core owner task under `model_timeout_ms` and returns the
    /// resulting Core [`Input`] (`ModelResult`, `ClassifyResult`, or `ModelFailure`).
    pub async fn execute_prepared(&self, req: monoagens_reason::ModelRequest) -> Input {
        let request_id = req.request_id;
        let purpose = req.purpose;
        let model = req.model.clone();
        let timeout_ms = req.timeout_ms.max(1);
        let provider_ref = self
            .provider
            .0
            .0
            .as_ref()
            .expect("provider present before drop");
        let provider_name = provider_ref.provider_name().to_owned();
        let started_at = self.clock.now();

        let _cue_guard = spawn_wait_cue_monitor(
            Arc::clone(&self.clock),
            self.cues.clone(),
            format!("model:{request_id}"),
            None,
            self.working_cue_ms,
            self.progress_note_ms,
        );

        let provider = Arc::clone(provider_ref);
        let join = tokio::task::spawn_blocking(move || provider.invoke(&req));

        match run_with_clock_timeout(self.clock.as_ref(), timeout_ms, join).await {
            Err(_timed_out) => {
                let elapsed = self.clock.now().saturating_sub_ms(started_at);
                let record = ModelCallRecord {
                    request_id,
                    purpose,
                    provider: provider_name,
                    model: model.clone(),
                    usage: None,
                    latency_ms: elapsed,
                    outcome: CallOutcome::Failed(ProviderFailureClass::Timeout),
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
            Ok(Err(join_err)) => {
                let elapsed = self.clock.now().saturating_sub_ms(started_at);
                let record = ModelCallRecord {
                    request_id,
                    purpose,
                    provider: provider_name,
                    model: model.clone(),
                    usage: None,
                    latency_ms: elapsed,
                    outcome: CallOutcome::Failed(ProviderFailureClass::Server),
                };
                self.diagnostics.record_model_call(record);
                let msg = format!("model worker panicked: {join_err}");
                self.diagnostics
                    .log(self.clock.now(), LogLevel::Error, "gateway.model", &msg);
                Input::ModelFailure {
                    request_id,
                    reason: self.diagnostics.redact(&msg),
                }
            }
            Ok(Ok(Ok(success))) => {
                let tokens = success.record.usage.map_or(0, |u| u.total_tokens);
                let (_used, _limit) = self.budget.record_tokens(self.clock.now(), tokens);
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
                match success.output {
                    ModelOutput::ThoughtActs(parsed) => Input::ModelResult {
                        request_id,
                        acts: parsed.acts,
                    },
                    ModelOutput::Verdict(verdict) => Input::ClassifyResult {
                        request_id,
                        verdict,
                    },
                }
            }
            Ok(Ok(Err(failure))) => {
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
                Input::ModelFailure {
                    request_id,
                    reason: redacted_reason,
                }
            }
        }
    }

    /// Returns the shared [`DailyBudgetTracker`].
    #[must_use]
    pub fn budget(&self) -> &DailyBudgetTracker {
        &self.budget
    }
}
