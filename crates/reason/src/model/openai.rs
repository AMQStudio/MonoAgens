//! OpenAI-compatible stateless HTTP (`store: false`) model provider.

use core::fmt;
use std::str::FromStr;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use monoagens_protocol::ModalitySlot;
use reqwest::blocking::Client;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::model::{
    CallOutcome, MessagePart, ModelCallFailure, ModelCallRecord, ModelCallSuccess, ModelProvider,
    ModelRequest, ProviderFailureClass, ResponseSchemaMode, TokenUsage, decode_model_response,
};
use crate::parse::{strict_classification_verdict_schema, strict_thought_batch_schema};
use crate::secrets::{SecretString, redact_secrets};

/// Reasoning effort level sent as `reasoning_effort` on OpenAI-compatible requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReasoningEffort {
    Low,
    Medium,
    High,
    Max,
}

impl ReasoningEffort {
    /// Canonical lowercase wire value for OpenAI-compatible `reasoning_effort`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Max => "max",
        }
    }
}

impl fmt::Display for ReasoningEffort {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ReasoningEffort {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "low" => Ok(Self::Low),
            "medium" => Ok(Self::Medium),
            "high" => Ok(Self::High),
            "max" => Ok(Self::Max),
            other => Err(format!(
                "invalid reasoning effort '{other}' (expected low|medium|high|max)"
            )),
        }
    }
}

/// Protocol headers that extra headers must never override (`model-providers` spec).
const PROTOCOL_FORBIDDEN_HEADERS: &[&str] =
    &["authorization", "content-type", "content-length", "host"];

/// Validates that an extra HTTP header name is well-formed and does not override a protocol header.
pub fn validate_extra_header_name(name: &str) -> Result<String, String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("extra header name must not be empty".to_owned());
    }
    let lower = trimmed.to_ascii_lowercase();
    if PROTOCOL_FORBIDDEN_HEADERS.contains(&lower.as_str()) {
        return Err(format!(
            "extra header '{trimmed}' cannot override protocol header '{lower}'"
        ));
    }
    HeaderName::from_bytes(trimmed.as_bytes())
        .map_err(|e| format!("invalid extra header name '{trimmed}': {e}"))?;
    Ok(trimmed.to_owned())
}

/// Configured value source for an extra HTTP header.
#[derive(Clone, Debug)]
pub enum ExtraHeaderValue {
    /// Static literal value (redacted in logs/debug).
    Literal(SecretString),
    /// Value read from an environment variable at provider construction time.
    Env(String),
    /// Replaced at call time with the request's stable `conversation_id`.
    ConversationId,
}

impl ExtraHeaderValue {
    /// Parses a config string (`"{conversation_id}"`, `"env:VAR"`, or literal).
    #[must_use]
    pub fn parse(raw: &str) -> Self {
        let trimmed = raw.trim();
        if trimmed == "{conversation_id}" {
            Self::ConversationId
        } else if let Some(var) = trimmed.strip_prefix("env:") {
            Self::Env(var.trim().to_owned())
        } else {
            Self::Literal(SecretString::new(raw.to_owned()))
        }
    }
}

/// Configurable JSON dot-paths for reading token usage from provider responses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageFieldPaths {
    pub input_tokens: String,
    pub output_tokens: String,
    pub cached_tokens: String,
    pub reasoning_tokens: String,
    pub total_tokens: String,
    pub cost: Option<String>,
}

impl Default for UsageFieldPaths {
    fn default() -> Self {
        Self {
            input_tokens: "prompt_tokens".to_owned(),
            output_tokens: "completion_tokens".to_owned(),
            cached_tokens: "prompt_tokens_details.cached_tokens".to_owned(),
            reasoning_tokens: "completion_tokens_details.reasoning_tokens".to_owned(),
            total_tokens: "total_tokens".to_owned(),
            cost: Some("cost".to_owned()),
        }
    }
}

fn lookup_json_path<'a>(root: &'a Value, path: &str) -> Option<&'a Value> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return None;
    }
    let mut cur = root;
    for seg in trimmed.split('.') {
        cur = cur.as_object()?.get(seg)?;
    }
    Some(cur)
}

fn lookup_u64_path(root: &Value, path: &str) -> Option<u64> {
    lookup_json_path(root, path).and_then(Value::as_u64)
}

fn lookup_f64_path(root: &Value, path: &str) -> Option<f64> {
    lookup_json_path(root, path).and_then(Value::as_f64)
}

/// Default JSON parameter name for output-token limits.
pub const DEFAULT_OUTPUT_LIMIT_PARAM: &str = "max_completion_tokens";

/// Product default output-token limit per purpose (`model-providers` spec).
pub const PRODUCT_DEFAULT_MAX_OUTPUT_TOKENS: u32 = 16_384;

/// Per-purpose output-token limits for a model profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputLimitProfile {
    pub reason: u32,
    pub classify: u32,
    pub consolidate: u32,
}

impl Default for OutputLimitProfile {
    fn default() -> Self {
        Self {
            reason: PRODUCT_DEFAULT_MAX_OUTPUT_TOKENS,
            classify: PRODUCT_DEFAULT_MAX_OUTPUT_TOKENS,
            consolidate: PRODUCT_DEFAULT_MAX_OUTPUT_TOKENS,
        }
    }
}

impl OutputLimitProfile {
    /// Returns the configured output-token limit for `purpose`.
    #[must_use]
    pub const fn for_purpose(&self, purpose: monoagens_core::ModelPurpose) -> u32 {
        match purpose {
            monoagens_core::ModelPurpose::Reason => self.reason,
            monoagens_core::ModelPurpose::Classify => self.classify,
            monoagens_core::ModelPurpose::Consolidate => self.consolidate,
        }
    }

    /// Returns the configuration setting name for `purpose`'s limit.
    #[must_use]
    pub const fn setting_name(purpose: monoagens_core::ModelPurpose) -> &'static str {
        match purpose {
            monoagens_core::ModelPurpose::Reason => "output_limits.reason",
            monoagens_core::ModelPurpose::Classify => "output_limits.classify",
            monoagens_core::ModelPurpose::Consolidate => "output_limits.consolidate",
        }
    }
}

/// Configuration for [`OpenAiModelProvider`].
#[derive(Clone)]
pub struct OpenAiModelConfig {
    pub endpoint_url: String,
    pub api_key: Option<SecretString>,
    pub default_timeout_ms: u64,
    pub reasoning_effort: Option<ReasoningEffort>,
    pub extra_headers: Vec<(String, ExtraHeaderValue)>,
    pub usage_paths: UsageFieldPaths,
    pub output_limit_param: String,
    pub output_limits: OutputLimitProfile,
    pub profile_name: Option<String>,
    pub user_agent: Option<String>,
    pub max_retries: u32,
}

impl fmt::Debug for OpenAiModelConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let header_names: Vec<&str> = self.extra_headers.iter().map(|(k, _)| k.as_str()).collect();
        f.debug_struct("OpenAiModelConfig")
            .field("endpoint_url", &self.endpoint_url)
            .field("api_key", &self.api_key.as_ref().map(|_| "[REDACTED]"))
            .field("default_timeout_ms", &self.default_timeout_ms)
            .field("reasoning_effort", &self.reasoning_effort)
            .field("extra_header_names", &header_names)
            .field("usage_paths", &self.usage_paths)
            .field("output_limit_param", &self.output_limit_param)
            .field("output_limits", &self.output_limits)
            .field("profile_name", &self.profile_name)
            .field("user_agent", &self.user_agent)
            .field("max_retries", &self.max_retries)
            .finish()
    }
}

impl OpenAiModelConfig {
    /// Creates an OpenAI-compatible provider config targeting `endpoint_url`.
    #[must_use]
    pub fn new(endpoint_url: impl Into<String>, api_key: Option<SecretString>) -> Self {
        Self {
            endpoint_url: endpoint_url.into(),
            api_key,
            default_timeout_ms: 30_000,
            reasoning_effort: None,
            extra_headers: Vec::new(),
            usage_paths: UsageFieldPaths::default(),
            output_limit_param: DEFAULT_OUTPUT_LIMIT_PARAM.to_owned(),
            output_limits: OutputLimitProfile::default(),
            profile_name: None,
            user_agent: None,
            max_retries: 0,
        }
    }
}

#[derive(Deserialize)]
struct ChatCompletionsResponse {
    model: Option<String>,
    choices: Vec<ChatChoice>,
    usage: Option<Value>,
}

#[derive(Deserialize)]
struct ChatChoice {
    message: ChatMessageResp,
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct ChatMessageResp {
    content: Option<Value>,
    refusal: Option<String>,
}

/// Cumulative HTTP retry and provider-reported usage telemetry from [`OpenAiModelProvider`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OpenAiTelemetrySnapshot {
    pub retries_total: u64,
    pub rate_limit_hits: u64,
    pub cumulative_cost_usd: f64,
    pub quota_headers_seen: Vec<String>,
    /// Per-call usage captured for the report's mean/p95 token and latency figures.
    pub call_count: u64,
    pub cumulative_tokens_in: u64,
    pub cumulative_tokens_out: u64,
    pub cumulative_reasoning_tokens: u64,
    /// Sum of provider-reported cached input tokens.
    #[serde(default)]
    pub cumulative_cached_tokens: u64,
    /// Calls whose provider response reported no cached-token count at all (`unknown`, never `0`).
    #[serde(default)]
    pub cached_tokens_unknown_calls: u64,
    pub call_tokens_in: Vec<u64>,
    pub call_tokens_out: Vec<u64>,
    pub call_reasoning_tokens: Vec<u64>,
    /// Per-call cached input tokens; `None` means the provider reported `unknown`.
    #[serde(default)]
    pub call_cached_tokens: Vec<Option<u64>>,
    pub call_latencies_ms: Vec<u64>,
    #[serde(default)]
    pub runaways_reason: u64,
    #[serde(default)]
    pub runaways_classify: u64,
    #[serde(default)]
    pub runaways_consolidate: u64,
}

impl OpenAiTelemetrySnapshot {
    /// Returns the number of calls that hit the output limit for `purpose`.
    #[must_use]
    pub const fn runaways_for(&self, purpose: monoagens_core::ModelPurpose) -> u64 {
        match purpose {
            monoagens_core::ModelPurpose::Reason => self.runaways_reason,
            monoagens_core::ModelPurpose::Classify => self.runaways_classify,
            monoagens_core::ModelPurpose::Consolidate => self.runaways_consolidate,
        }
    }
}

/// Stateless OpenAI-compatible HTTP [`ModelProvider`] sending `store: false` and strict schemas.
#[derive(Debug)]
pub struct OpenAiModelProvider {
    config: OpenAiModelConfig,
    client: Client,
    parsed_headers: HeaderMap,
    conversation_headers: Vec<HeaderName>,
    header_secrets: Vec<SecretString>,
    retries_total: AtomicU64,
    rate_limit_hits: AtomicU64,
    telemetry: Mutex<OpenAiTelemetrySnapshot>,
}

impl OpenAiModelProvider {
    /// Builds a blocking HTTP provider with bounded request timeouts.
    pub fn new(config: OpenAiModelConfig) -> Result<Self, String> {
        if config.endpoint_url.trim().is_empty() {
            return Err("OpenAI-compatible endpoint_url must not be empty".to_owned());
        }
        let mut parsed_headers = HeaderMap::new();
        let mut conversation_headers = Vec::new();
        let mut header_secrets = Vec::new();
        for (name, val) in &config.extra_headers {
            let trimmed = validate_extra_header_name(name)?;
            let h_name = HeaderName::from_bytes(trimmed.as_bytes())
                .map_err(|e| format!("invalid extra header name '{trimmed}': {e}"))?;
            match val {
                ExtraHeaderValue::Literal(secret) => {
                    let h_val = HeaderValue::from_str(secret.expose())
                        .map_err(|e| format!("invalid extra header value for '{trimmed}': {e}"))?;
                    header_secrets.push(secret.clone());
                    parsed_headers.insert(h_name, h_val);
                }
                ExtraHeaderValue::Env(var) => {
                    let env_val = std::env::var(var).map_err(|_| {
                        format!("extra header '{trimmed}' env var '{var}' is not set")
                    })?;
                    let secret = SecretString::new(env_val);
                    let h_val = HeaderValue::from_str(secret.expose())
                        .map_err(|e| format!("invalid extra header value for '{trimmed}': {e}"))?;
                    header_secrets.push(secret);
                    parsed_headers.insert(h_name, h_val);
                }
                ExtraHeaderValue::ConversationId => {
                    conversation_headers.push(h_name);
                }
            }
        }
        let mut builder =
            Client::builder().timeout(Duration::from_millis(config.default_timeout_ms.max(1)));
        if let Some(ua) = config
            .user_agent
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            builder = builder.user_agent(ua);
        }
        let client = builder
            .build()
            .map_err(|err| format!("failed to build HTTP client: {err}"))?;
        Ok(Self {
            config,
            client,
            parsed_headers,
            conversation_headers,
            header_secrets,
            retries_total: AtomicU64::new(0),
            rate_limit_hits: AtomicU64::new(0),
            telemetry: Mutex::new(OpenAiTelemetrySnapshot::default()),
        })
    }

    /// Returns cumulative retries, rate-limit hits, and usage telemetry.
    #[must_use]
    pub fn telemetry_snapshot(&self) -> OpenAiTelemetrySnapshot {
        let mut snap = self.telemetry.lock().map(|g| g.clone()).unwrap_or_default();
        snap.retries_total = self.retries_total.load(Ordering::Relaxed);
        snap.rate_limit_hits = self.rate_limit_hits.load(Ordering::Relaxed);
        snap
    }

    fn scrub(&self, text: &str) -> String {
        let mut secrets: Vec<&SecretString> = Vec::new();
        if let Some(key) = &self.config.api_key {
            secrets.push(key);
        }
        for val in &self.header_secrets {
            secrets.push(val);
        }
        redact_secrets(text, &secrets)
    }

    fn make_failure(
        &self,
        request: &ModelRequest,
        class: ProviderFailureClass,
        status_code: Option<u16>,
        latency_ms: u64,
        raw_detail: &str,
    ) -> ModelCallFailure {
        ModelCallFailure {
            class,
            record: Box::new(ModelCallRecord {
                request_id: request.request_id,
                purpose: request.purpose,
                provider: self.provider_name().to_owned(),
                model: request.model.clone(),
                usage: None,
                latency_ms,
                outcome: CallOutcome::Failed(class),
                strict_schema: request.response_schema != crate::model::ResponseSchemaMode::None,
            }),
            status_code,
            failed_wake: None,
            detail: self.scrub(raw_detail),
        }
    }
}

impl ModelProvider for OpenAiModelProvider {
    fn provider_name(&self) -> &str {
        "openai_compatible"
    }

    fn invoke(&self, request: &ModelRequest) -> Result<ModelCallSuccess, ModelCallFailure> {
        self.invoke_inner(request, None)
    }

    fn invoke_streaming(
        &self,
        request: &ModelRequest,
        on_text: &mut dyn FnMut(&str),
    ) -> Result<ModelCallSuccess, ModelCallFailure> {
        self.invoke_inner(request, Some(on_text))
    }
}

impl OpenAiModelProvider {
    /// Shared body of `invoke` and `invoke_streaming`: with a sink the request asks for SSE
    /// (`stream: true`, usage in the final chunk) and the chunks are folded back into the
    /// blocking envelope, so everything after the read is the same code path.
    #[allow(clippy::too_many_lines)]
    fn invoke_inner(
        &self,
        request: &ModelRequest,
        mut stream: Option<&mut dyn FnMut(&str)>,
    ) -> Result<ModelCallSuccess, ModelCallFailure> {
        let start = Instant::now();
        let timeout_ms = if request.timeout_ms > 0 {
            request.timeout_ms
        } else {
            self.config.default_timeout_ms
        };
        if timeout_ms == 0 {
            return Err(self.make_failure(
                request,
                ProviderFailureClass::Timeout,
                None,
                0,
                "request timeout_ms must be > 0",
            ));
        }

        let messages_json: Vec<Value> = request
            .messages
            .iter()
            .map(|msg| {
                let has_attachment = msg
                    .parts
                    .iter()
                    .any(|p| matches!(p, MessagePart::Attachment(_)));
                let content = if !has_attachment && msg.parts.len() == 1 {
                    match &msg.parts[0] {
                        MessagePart::Text(t) => json!(t),
                        MessagePart::Attachment(_) => unreachable!(),
                    }
                } else {
                    let parts: Vec<Value> = msg
                        .parts
                        .iter()
                        .map(|part| match part {
                            MessagePart::Text(t) => json!({
                                "type": "text",
                                "text": t,
                            }),
                            MessagePart::Attachment(att) => match att.modality_slot() {
                                ModalitySlot::Image => json!({
                                    "type": "image_url",
                                    "image_url": { "url": att.to_data_uri() },
                                }),
                                ModalitySlot::AudioVoice => json!({
                                    "type": "input_audio",
                                    "input_audio": {
                                        "data": att.encode_base64(),
                                        "format": "wav",
                                    },
                                }),
                                ModalitySlot::Video | ModalitySlot::SensorBlob => json!({
                                    "type": "text",
                                    "text": format!(
                                        "[attachment:{} slot={:?} bytes={}]",
                                        att.handle_id(),
                                        att.modality_slot(),
                                        att.byte_len()
                                    ),
                                }),
                            },
                        })
                        .collect();
                    Value::Array(parts)
                };
                json!({
                    "role": msg.role.as_str(),
                    "content": content,
                })
            })
            .collect();

        let mut payload = json!({
            "model": request.model,
            "messages": messages_json,
            "store": false,
        });

        let effective_limit = request
            .max_output_tokens
            .unwrap_or_else(|| self.config.output_limits.for_purpose(request.purpose));
        let limit_param = if self.config.output_limit_param.trim().is_empty() {
            DEFAULT_OUTPUT_LIMIT_PARAM
        } else {
            self.config.output_limit_param.trim()
        };

        if let Some(obj) = payload.as_object_mut() {
            match request.response_schema {
                ResponseSchemaMode::None => {}
                ResponseSchemaMode::ThoughtBatchStrict => {
                    obj.insert(
                        "response_format".to_owned(),
                        json!({
                            "type": "json_schema",
                            "json_schema": {
                                "name": "monoagens_thought_batch",
                                "strict": true,
                                "schema": strict_thought_batch_schema(),
                            }
                        }),
                    );
                }
                ResponseSchemaMode::ClassificationVerdictStrict => {
                    obj.insert(
                        "response_format".to_owned(),
                        json!({
                            "type": "json_schema",
                            "json_schema": {
                                "name": "monoagens_classification_verdict",
                                "strict": true,
                                "schema": strict_classification_verdict_schema(),
                            }
                        }),
                    );
                }
            }
            if stream.is_some() {
                obj.insert("stream".to_owned(), json!(true));
                obj.insert("stream_options".to_owned(), json!({ "include_usage": true }));
            }
            obj.insert(limit_param.to_owned(), json!(effective_limit));
            if let Some(temp) = request.temperature {
                obj.insert("temperature".to_owned(), json!(temp));
            }
            if let Some(effort) = request.reasoning_effort.or(self.config.reasoning_effort) {
                obj.insert("reasoning_effort".to_owned(), json!(effort.as_str()));
            }
        }

        let mut attempt: u32 = 0;
        let conv_header_val = request
            .conversation_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .and_then(|cid| HeaderValue::from_str(cid).ok());
        let (status, raw_body) = loop {
            let mut headers = self.parsed_headers.clone();
            if let Some(val) = &conv_header_val {
                for h_name in &self.conversation_headers {
                    headers.insert(h_name.clone(), val.clone());
                }
            }
            for (name, value) in &request.extra_headers {
                let Ok(trimmed) = validate_extra_header_name(name) else {
                    continue;
                };
                let Ok(h_name) = HeaderName::from_bytes(trimmed.as_bytes()) else {
                    continue;
                };
                let Ok(h_val) = HeaderValue::from_str(value) else {
                    continue;
                };
                headers.insert(h_name, h_val);
            }
            let mut req = self
                .client
                .post(&self.config.endpoint_url)
                .timeout(Duration::from_millis(timeout_ms))
                .headers(headers)
                .json(&payload);
            if let Some(key) = &self.config.api_key {
                req = req.bearer_auth(key.expose());
            }

            match req.send() {
                Ok(resp) => {
                    let status = resp.status();
                    let retry_after_secs = resp
                        .headers()
                        .get(reqwest::header::RETRY_AFTER)
                        .and_then(|v| v.to_str().ok())
                        .and_then(|s| s.trim().parse::<u64>().ok());
                    if let Ok(mut t) = self.telemetry.lock() {
                        for (hk, hv) in resp.headers() {
                            let hks = hk.as_str().to_ascii_lowercase();
                            if (hks.contains("ratelimit")
                                || hks.contains("quota")
                                || hks.contains("allowance"))
                                && let Ok(vs) = hv.to_str()
                            {
                                let entry = format!("{hks}={vs}");
                                if !t.quota_headers_seen.contains(&entry)
                                    && t.quota_headers_seen.len() < 16
                                {
                                    t.quota_headers_seen.push(entry);
                                }
                            }
                        }
                    }
                    if status.as_u16() == 429 {
                        self.rate_limit_hits.fetch_add(1, Ordering::Relaxed);
                    }
                    let mut emitted_text = false;
                    // A server that ignores `stream: true` answers a plain JSON body: read it as such.
                    let is_event_stream = resp
                        .headers()
                        .get(reqwest::header::CONTENT_TYPE)
                        .and_then(|v| v.to_str().ok())
                        .is_some_and(|v| v.to_ascii_lowercase().contains("text/event-stream"));
                    let body_result: Result<String, (bool, String)> = match stream.as_deref_mut() {
                        Some(sink) if status.is_success() && is_event_stream => {
                            let mut folder = crate::model::sse::ChatStreamFolder::new(sink);
                            let read = crate::model::sse::read_sse(resp, |d| folder.feed(d));
                            let folded = folder.finish();
                            emitted_text = folded.emitted_text;
                            match read {
                                Ok(()) => Ok(folded.envelope.to_string()),
                                Err(e) => Err((
                                    e.kind() == std::io::ErrorKind::TimedOut,
                                    e.to_string(),
                                )),
                            }
                        }
                        _ => resp.text().map_err(|e| (e.is_timeout(), e.to_string())),
                    };
                    match body_result {
                        Ok(body) => {
                            let code = status.as_u16();
                            let transient =
                                code == 429 || code == 408 || (500..=599).contains(&code);
                            if !status.is_success()
                                && transient
                                && attempt < self.config.max_retries
                            {
                                attempt += 1;
                                self.retries_total.fetch_add(1, Ordering::Relaxed);
                                let delay_ms = retry_after_secs
                                    .map(|s| s.saturating_mul(1_000))
                                    .unwrap_or_else(|| (1_000_u64 << (attempt - 1)).min(30_000));
                                std::thread::sleep(Duration::from_millis(delay_ms));
                                continue;
                            }
                            break (status, body);
                        }
                        Err((timed_out, detail)) => {
                            if attempt < self.config.max_retries && !emitted_text {
                                attempt += 1;
                                self.retries_total.fetch_add(1, Ordering::Relaxed);
                                let delay_ms = (1_000_u64 << (attempt - 1)).min(30_000);
                                std::thread::sleep(Duration::from_millis(delay_ms));
                                continue;
                            }
                            let latency_ms = start.elapsed().as_millis() as u64;
                            let class = if timed_out {
                                ProviderFailureClass::Timeout
                            } else {
                                ProviderFailureClass::Server
                            };
                            return Err(self.make_failure(
                                request,
                                class,
                                Some(status.as_u16()),
                                latency_ms,
                                &detail,
                            ));
                        }
                    }
                }
                Err(err) => {
                    let transient = err.is_timeout() || err.is_connect() || err.is_request();
                    if transient && attempt < self.config.max_retries {
                        attempt += 1;
                        self.retries_total.fetch_add(1, Ordering::Relaxed);
                        let delay_ms = (1_000_u64 << (attempt - 1)).min(30_000);
                        std::thread::sleep(Duration::from_millis(delay_ms));
                        continue;
                    }
                    let latency_ms = start.elapsed().as_millis() as u64;
                    let class = if err.is_timeout() {
                        ProviderFailureClass::Timeout
                    } else {
                        ProviderFailureClass::Server
                    };
                    return Err(self.make_failure(
                        request,
                        class,
                        err.status().map(|s| s.as_u16()),
                        latency_ms,
                        &err.to_string(),
                    ));
                }
            }
        };
        let latency_ms = start.elapsed().as_millis() as u64;

        if !status.is_success() {
            let code = status.as_u16();
            let class = match code {
                401 | 403 => ProviderFailureClass::Authentication,
                429 => ProviderFailureClass::RateLimit,
                408 | 504 => ProviderFailureClass::Timeout,
                500..=599 => ProviderFailureClass::Server,
                _ => ProviderFailureClass::InvalidOutput,
            };
            return Err(self.make_failure(
                request,
                class,
                Some(code),
                latency_ms,
                &format!("HTTP {code}: {raw_body}"),
            ));
        }

        let parsed: ChatCompletionsResponse = serde_json::from_str(&raw_body).map_err(|err| {
            self.make_failure(
                request,
                ProviderFailureClass::InvalidOutput,
                Some(status.as_u16()),
                latency_ms,
                &format!("malformed OpenAI JSON envelope: {err}"),
            )
        })?;

        let resolved_model = parsed
            .model
            .filter(|m| !m.trim().is_empty())
            .unwrap_or_else(|| request.model.clone());

        let usage = parsed.usage.map(|u| {
            let paths = &self.config.usage_paths;
            let input_tokens = lookup_u64_path(&u, &paths.input_tokens).unwrap_or(0);
            let output_tokens = lookup_u64_path(&u, &paths.output_tokens).unwrap_or(0);
            let cached_tokens = lookup_u64_path(&u, &paths.cached_tokens);
            let reasoning_tokens = lookup_u64_path(&u, &paths.reasoning_tokens);
            let cost = paths.cost.as_deref().and_then(|p| lookup_f64_path(&u, p));
            if let Ok(mut t) = self.telemetry.lock() {
                t.call_count = t.call_count.saturating_add(1);
                t.cumulative_tokens_in = t.cumulative_tokens_in.saturating_add(input_tokens);
                t.cumulative_tokens_out = t.cumulative_tokens_out.saturating_add(output_tokens);
                t.cumulative_reasoning_tokens = t
                    .cumulative_reasoning_tokens
                    .saturating_add(reasoning_tokens.unwrap_or(0));
                match cached_tokens {
                    Some(c) => {
                        t.cumulative_cached_tokens = t.cumulative_cached_tokens.saturating_add(c);
                    }
                    None => {
                        t.cached_tokens_unknown_calls =
                            t.cached_tokens_unknown_calls.saturating_add(1);
                    }
                }
                t.call_tokens_in.push(input_tokens);
                t.call_tokens_out.push(output_tokens);
                t.call_reasoning_tokens.push(reasoning_tokens.unwrap_or(0));
                t.call_cached_tokens.push(cached_tokens);
                t.call_latencies_ms.push(latency_ms);
                if let Some(c) = cost {
                    t.cumulative_cost_usd += c;
                }
            }
            let total_tokens =
                lookup_u64_path(&u, &paths.total_tokens).unwrap_or(input_tokens + output_tokens);
            TokenUsage {
                input_tokens,
                output_tokens,
                cached_tokens,
                reasoning_tokens,
                total_tokens,
            }
        });

        let choice = parsed.choices.into_iter().next().ok_or_else(|| {
            self.make_failure(
                request,
                ProviderFailureClass::InvalidOutput,
                Some(status.as_u16()),
                latency_ms,
                "OpenAI response contained zero choices",
            )
        })?;

        if let Some(refusal) = choice.message.refusal
            && !refusal.trim().is_empty()
        {
            return Err(self.make_failure(
                request,
                ProviderFailureClass::InvalidOutput,
                Some(status.as_u16()),
                latency_ms,
                &format!("model refusal: {refusal}"),
            ));
        }

        let at_limit = choice.finish_reason.as_deref() == Some("length")
            || usage
                .as_ref()
                .is_some_and(|u| u.output_tokens >= u64::from(effective_limit));
        if at_limit && let Ok(mut t) = self.telemetry.lock() {
            match request.purpose {
                monoagens_core::ModelPurpose::Reason => {
                    t.runaways_reason = t.runaways_reason.saturating_add(1);
                }
                monoagens_core::ModelPurpose::Classify => {
                    t.runaways_classify = t.runaways_classify.saturating_add(1);
                }
                monoagens_core::ModelPurpose::Consolidate => {
                    t.runaways_consolidate = t.runaways_consolidate.saturating_add(1);
                }
            }
        }

        let content_text = extract_message_text(choice.message.content.as_ref());
        let visible_empty = content_text.as_deref().is_none_or(|s| s.trim().is_empty());
        if at_limit && visible_empty {
            let setting = OutputLimitProfile::setting_name(request.purpose);
            let profile = self
                .config
                .profile_name
                .as_deref()
                .unwrap_or(&resolved_model);
            return Err(self.make_failure(
                request,
                ProviderFailureClass::InvalidOutput,
                Some(status.as_u16()),
                latency_ms,
                &format!(
                    "reasoning used the whole limit ({effective_limit} tokens); raise '{setting}' for model profile '{profile}'"
                ),
            ));
        }

        let content_text = content_text.ok_or_else(|| {
            self.make_failure(
                request,
                ProviderFailureClass::InvalidOutput,
                Some(status.as_u16()),
                latency_ms,
                "OpenAI choice message missing text content",
            )
        })?;

        decode_model_response(
            self.provider_name(),
            request,
            resolved_model,
            content_text,
            usage,
            latency_ms,
        )
        .map_err(|mut failure| {
            failure.detail = self.scrub(&failure.detail);
            failure
        })
    }
}

fn extract_message_text(content: Option<&Value>) -> Option<String> {
    match content? {
        Value::String(s) => Some(s.clone()),
        Value::Array(parts) => {
            let mut buf = String::new();
            for part in parts {
                if let Some(txt) = part.get("text").and_then(Value::as_str) {
                    buf.push_str(txt);
                }
            }
            if buf.is_empty() { None } else { Some(buf) }
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};
    use std::thread;

    use monoagens_core::ModelPurpose;
    use monoagens_protocol::EventId;

    use super::*;

    fn spawn_single_capture_server(captured: Arc<Mutex<String>>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let addr = listener.local_addr().expect("local addr");
        thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = Vec::new();
                let mut tmp = [0_u8; 4096];
                let mut expected_body_len: Option<usize> = None;
                let mut header_end: Option<usize> = None;
                loop {
                    let n = stream.read(&mut tmp).unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    buf.extend_from_slice(&tmp[..n]);
                    if header_end.is_none()
                        && let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n")
                    {
                        header_end = Some(pos + 4);
                        let hdr = String::from_utf8_lossy(&buf[..pos]);
                        for line in hdr.lines() {
                            if let Some((k, v)) = line.split_once(':')
                                && k.trim().eq_ignore_ascii_case("content-length")
                            {
                                expected_body_len = v.trim().parse::<usize>().ok();
                            }
                        }
                    }
                    if let (Some(he), Some(bl)) = (header_end, expected_body_len)
                        && buf.len() >= he + bl
                    {
                        break;
                    }
                }
                let body = header_end.map_or_else(String::new, |he| {
                    String::from_utf8_lossy(&buf[he..]).into_owned()
                });
                *captured.lock().expect("lock") = body;
                let resp_body = r#"{"model":"test-model","choices":[{"message":{"content":"{\"acts\":[]}"},"finish_reason":"stop"}]}"#;
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    resp_body.len(),
                    resp_body
                );
                let _ = stream.write_all(resp.as_bytes());
            }
        });
        format!("http://{addr}/v1/chat/completions")
    }

    #[test]
    fn per_request_reasoning_effort_overrides_provider_config() {
        let cap1 = Arc::new(Mutex::new(String::new()));
        let url1 = spawn_single_capture_server(Arc::clone(&cap1));
        let mut cfg = OpenAiModelConfig::new(url1, None);
        cfg.reasoning_effort = Some(ReasoningEffort::High);
        let provider = OpenAiModelProvider::new(cfg).expect("provider");

        // Request override = Low overrides config High.
        let mut req = ModelRequest::text(
            EventId::new(1),
            ModelPurpose::Reason,
            "test-model",
            "sys",
            "usr",
        );
        req.reasoning_effort = Some(ReasoningEffort::Low);
        let _ = provider.invoke(&req).expect("invoke override");
        let parsed1: Value = serde_json::from_str(&cap1.lock().unwrap()).expect("json1");
        assert_eq!(parsed1["reasoning_effort"], "low");

        // Request override = None falls back to config High.
        let cap2 = Arc::new(Mutex::new(String::new()));
        let url2 = spawn_single_capture_server(Arc::clone(&cap2));
        let mut cfg2 = OpenAiModelConfig::new(url2, None);
        cfg2.reasoning_effort = Some(ReasoningEffort::High);
        let provider2 = OpenAiModelProvider::new(cfg2).expect("provider2");
        let req2 = ModelRequest::text(
            EventId::new(2),
            ModelPurpose::Reason,
            "test-model",
            "sys",
            "usr",
        );
        assert_eq!(req2.reasoning_effort, None);
        let _ = provider2.invoke(&req2).expect("invoke fallback");
        let parsed2: Value = serde_json::from_str(&cap2.lock().unwrap()).expect("json2");
        assert_eq!(parsed2["reasoning_effort"], "high");
    }
}
