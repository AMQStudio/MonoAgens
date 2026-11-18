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

/// Configuration for [`OpenAiModelProvider`].
#[derive(Clone)]
pub struct OpenAiModelConfig {
    pub endpoint_url: String,
    pub api_key: Option<SecretString>,
    pub default_timeout_ms: u64,
    pub reasoning_effort: Option<ReasoningEffort>,
    pub extra_headers: Vec<(String, SecretString)>,
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
            user_agent: None,
            max_retries: 0,
        }
    }
}

#[derive(Deserialize)]
struct ChatCompletionsResponse {
    model: Option<String>,
    choices: Vec<ChatChoice>,
    usage: Option<ChatUsage>,
}

#[derive(Deserialize)]
struct ChatChoice {
    message: ChatMessageResp,
}

#[derive(Deserialize)]
struct ChatMessageResp {
    content: Option<Value>,
    refusal: Option<String>,
}

#[derive(Deserialize)]
struct ChatUsage {
    prompt_tokens: Option<u64>,
    completion_tokens: Option<u64>,
    total_tokens: Option<u64>,
    prompt_tokens_details: Option<PromptTokensDetails>,
    completion_tokens_details: Option<CompletionTokensDetails>,
    cost: Option<f64>,
    opencode: Option<OpenCodeUsageMeta>,
}

#[derive(Deserialize)]
struct PromptTokensDetails {
    cached_tokens: Option<u64>,
}

#[derive(Deserialize)]
struct CompletionTokensDetails {
    reasoning_tokens: Option<u64>,
}

#[derive(Deserialize, Clone)]
struct OpenCodeUsageMeta {
    billing_source: Option<String>,
    list_cost: Option<f64>,
    allowance: Option<f64>,
    tokens: Option<OpenCodeTokenDetails>,
}

#[derive(Deserialize, Clone)]
#[allow(dead_code)]
struct OpenCodeTokenDetails {
    input: Option<u64>,
    output: Option<u64>,
    reasoning: Option<u64>,
    cache_read: Option<u64>,
    cache_write_5m: Option<u64>,
    cache_write_1h: Option<u64>,
}

/// Cumulative HTTP retry and provider-reported quota telemetry from [`OpenAiModelProvider`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OpenAiTelemetrySnapshot {
    pub retries_total: u64,
    pub rate_limit_hits: u64,
    pub last_billing_source: Option<String>,
    pub cumulative_cost_usd: f64,
    pub cumulative_list_cost_usd: f64,
    pub cumulative_allowance_usd: f64,
    pub quota_headers_seen: Vec<String>,
    /// Per-call usage captured for the report's mean/p95 token and latency figures.
    pub call_count: u64,
    pub cumulative_tokens_in: u64,
    pub cumulative_tokens_out: u64,
    pub cumulative_reasoning_tokens: u64,
    /// Sum of provider-reported cached input tokens (`cache_read` or `cached_tokens`).
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
}

/// Stateless OpenAI-compatible HTTP [`ModelProvider`] sending `store: false` and strict schemas.
#[derive(Debug)]
pub struct OpenAiModelProvider {
    config: OpenAiModelConfig,
    client: Client,
    parsed_headers: HeaderMap,
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
        for (name, val) in &config.extra_headers {
            let trimmed = name.trim();
            if trimmed.is_empty() {
                return Err("extra header name must not be empty".to_owned());
            }
            let h_name = HeaderName::from_bytes(trimmed.as_bytes())
                .map_err(|e| format!("invalid extra header name '{trimmed}': {e}"))?;
            let h_val = HeaderValue::from_str(val.expose())
                .map_err(|e| format!("invalid extra header value for '{trimmed}': {e}"))?;
            parsed_headers.insert(h_name, h_val);
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
            retries_total: AtomicU64::new(0),
            rate_limit_hits: AtomicU64::new(0),
            telemetry: Mutex::new(OpenAiTelemetrySnapshot::default()),
        })
    }

    /// Returns cumulative retries, rate-limit hits, and OpenCode usage/quota telemetry.
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
        for (_, val) in &self.config.extra_headers {
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
            if let Some(max_tok) = request.max_output_tokens {
                obj.insert("max_completion_tokens".to_owned(), json!(max_tok));
            }
            if let Some(temp) = request.temperature {
                obj.insert("temperature".to_owned(), json!(temp));
            }
            if let Some(effort) = self.config.reasoning_effort {
                obj.insert("reasoning_effort".to_owned(), json!(effort.as_str()));
            }
        }

        let mut attempt: u32 = 0;
        let session_header = request
            .session_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .and_then(|sid| HeaderValue::from_str(sid).ok());
        let (status, raw_body) = loop {
            let mut headers = self.parsed_headers.clone();
            if let Some(val) = &session_header {
                headers.insert(HeaderName::from_static("x-opencode-session"), val.clone());
            }
            for (name, value) in &request.extra_headers {
                let Ok(h_name) = HeaderName::from_bytes(name.trim().as_bytes()) else {
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
                    match resp.text() {
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
                        Err(err) => {
                            if attempt < self.config.max_retries {
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
                                Some(status.as_u16()),
                                latency_ms,
                                &err.to_string(),
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
            let input_tokens = u.prompt_tokens.unwrap_or(0);
            let output_tokens = u.completion_tokens.unwrap_or(0);
            // OpenCode reports cached reads in `usage.opencode.tokens.cache_read` (its `input`
            // excludes cache hits); OpenAI-compatible APIs use `prompt_tokens_details.cached_tokens`.
            let opencode_reasoning = u
                .opencode
                .as_ref()
                .and_then(|oc| oc.tokens.as_ref().and_then(|t| t.reasoning));
            let cached_tokens = u
                .opencode
                .as_ref()
                .and_then(|oc| oc.tokens.as_ref().and_then(|t| t.cache_read))
                .or_else(|| {
                    u.prompt_tokens_details
                        .as_ref()
                        .and_then(|d| d.cached_tokens)
                });
            let reasoning_tokens = opencode_reasoning
                .or_else(|| u.completion_tokens_details.and_then(|d| d.reasoning_tokens));
            if let Ok(mut t) = self.telemetry.lock() {
                t.call_count = t.call_count.saturating_add(1);
                t.cumulative_tokens_in = t.cumulative_tokens_in.saturating_add(input_tokens);
                t.cumulative_tokens_out = t.cumulative_tokens_out.saturating_add(output_tokens);
                t.cumulative_reasoning_tokens = t
                    .cumulative_reasoning_tokens
                    .saturating_add(reasoning_tokens.unwrap_or(0));
                match cached_tokens {
                    Some(c) => {
                        t.cumulative_cached_tokens = t.cumulative_cached_tokens.saturating_add(c)
                    }
                    None => {
                        t.cached_tokens_unknown_calls =
                            t.cached_tokens_unknown_calls.saturating_add(1)
                    }
                }
                t.call_tokens_in.push(input_tokens);
                t.call_tokens_out.push(output_tokens);
                t.call_reasoning_tokens.push(reasoning_tokens.unwrap_or(0));
                t.call_cached_tokens.push(cached_tokens);
                t.call_latencies_ms.push(latency_ms);
                if let Some(c) = u.cost {
                    t.cumulative_cost_usd += c;
                }
                if let Some(oc) = u.opencode {
                    if let Some(bs) = oc.billing_source {
                        t.last_billing_source = Some(bs);
                    }
                    if let Some(lc) = oc.list_cost {
                        t.cumulative_list_cost_usd += lc;
                    }
                    if let Some(al) = oc.allowance {
                        t.cumulative_allowance_usd += al;
                    }
                }
            }
            let total_tokens = u.total_tokens.unwrap_or(input_tokens + output_tokens);
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

        let content_text =
            extract_message_text(choice.message.content.as_ref()).ok_or_else(|| {
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
