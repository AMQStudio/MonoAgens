//! `messages_api` dialect: stateless HTTP provider.
//!
//! `POST {base}/messages` with the key in a configurable header and catalog-supplied extra
//! headers (such as a protocol version); system prompts
//! travel in the top-level `system` field and `max_tokens` is always sent (required by the API).
//! Replies are mapped into the same [`ModelCallSuccess`] as the OpenAI path through the shared
//! [`decode_model_response`].

use std::time::{Duration, Instant};

use monoagens_protocol::ModalitySlot;
use reqwest::blocking::Client;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::model::{
    CallOutcome, MessagePart, MessageRole, ModelCallFailure, ModelCallRecord, ModelCallSuccess,
    ModelProvider, ModelRequest, OutputLimitProfile, ProviderFailureClass, TokenUsage,
    classify_http_status, decode_model_response,
};
use crate::secrets::{SecretString, redact_secrets};

/// Configuration for [`MessagesModelProvider`].
#[derive(Clone)]
pub struct MessagesModelConfig {
    /// Full messages endpoint, e.g. `https://api.messages.com/v1/messages`.
    pub endpoint_url: String,
    pub api_key: Option<SecretString>,
    /// Header name carrying the key.
    pub key_header: String,
    /// Extra fixed headers sent on every call.
    pub extra_headers: Vec<(String, String)>,
    pub default_timeout_ms: u64,
    pub output_limits: OutputLimitProfile,
    pub profile_name: Option<String>,
}

impl core::fmt::Debug for MessagesModelConfig {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("MessagesModelConfig")
            .field("endpoint_url", &self.endpoint_url)
            .field("api_key", &self.api_key.as_ref().map(|_| "[REDACTED]"))
            .field("key_header", &self.key_header)
            .field("default_timeout_ms", &self.default_timeout_ms)
            .field("output_limits", &self.output_limits)
            .field("profile_name", &self.profile_name)
            .finish()
    }
}

impl MessagesModelConfig {
    /// Creates a config targeting `endpoint_url` (the full `/messages` URL).
    #[must_use]
    pub fn new(endpoint_url: impl Into<String>, api_key: Option<SecretString>) -> Self {
        Self {
            endpoint_url: endpoint_url.into(),
            api_key,
            key_header: "x-api-key".to_owned(),
            extra_headers: Vec::new(),
            default_timeout_ms: 30_000,
            output_limits: OutputLimitProfile::default(),
            profile_name: None,
        }
    }
}

#[derive(Deserialize)]
struct MessagesResponse {
    model: Option<String>,
    #[serde(default)]
    content: Vec<Value>,
    stop_reason: Option<String>,
    usage: Option<UsageResp>,
}

#[derive(Deserialize)]
struct UsageResp {
    #[serde(default)]
    input_tokens: u64,
    #[serde(default)]
    output_tokens: u64,
    cache_read_input_tokens: Option<u64>,
}

/// Stateless Messages-API Messages [`ModelProvider`].
#[derive(Debug)]
pub struct MessagesModelProvider {
    config: MessagesModelConfig,
    client: Client,
}

impl MessagesModelProvider {
    /// Builds a blocking HTTP provider.
    pub fn new(config: MessagesModelConfig) -> Result<Self, String> {
        if config.endpoint_url.trim().is_empty() {
            return Err("Messages-API endpoint_url must not be empty".to_owned());
        }
        let client = Client::builder()
            .timeout(Duration::from_millis(config.default_timeout_ms.max(1)))
            .build()
            .map_err(|err| format!("failed to build HTTP client: {err}"))?;
        Ok(Self { config, client })
    }

    fn failure(
        &self,
        request: &ModelRequest,
        class: ProviderFailureClass,
        status_code: Option<u16>,
        latency_ms: u64,
        raw_detail: &str,
    ) -> ModelCallFailure {
        let secrets: Vec<&SecretString> = self.config.api_key.iter().collect();
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
            detail: redact_secrets(raw_detail, &secrets),
        }
    }

    /// Builds the JSON request body (exposed for tests).
    pub(crate) fn build_payload(&self, request: &ModelRequest) -> Value {
        let mut system_parts: Vec<String> = Vec::new();
        let mut messages: Vec<Value> = Vec::new();
        for msg in &request.messages {
            if matches!(msg.role, MessageRole::System | MessageRole::Developer) {
                for part in &msg.parts {
                    if let MessagePart::Text(t) = part {
                        system_parts.push(t.clone());
                    }
                }
                continue;
            }
            let content: Vec<Value> = msg
                .parts
                .iter()
                .map(|part| match part {
                    MessagePart::Text(t) => json!({"type": "text", "text": t}),
                    MessagePart::Attachment(att) => match att.modality_slot() {
                        ModalitySlot::Image => json!({
                            "type": "image",
                            "source": {
                                "type": "base64",
                                "media_type": att.mime_type(),
                                "data": att.encode_base64(),
                            },
                        }),
                        _ => json!({
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
            let role = if msg.role == MessageRole::Assistant {
                "assistant"
            } else {
                "user"
            };
            messages.push(json!({"role": role, "content": content}));
        }
        let max_tokens = request
            .max_output_tokens
            .unwrap_or_else(|| self.config.output_limits.for_purpose(request.purpose));
        let mut payload = json!({
            "model": request.model,
            "max_tokens": max_tokens,
            "messages": messages,
        });
        if let Some(obj) = payload.as_object_mut() {
            if !system_parts.is_empty() {
                obj.insert("system".to_owned(), json!(system_parts.join("\n\n")));
            }
            if let Some(temp) = request.temperature {
                obj.insert("temperature".to_owned(), json!(temp));
            }
        }
        payload
    }
}

impl ModelProvider for MessagesModelProvider {
    fn provider_name(&self) -> &str {
        "messages_api"
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

impl MessagesModelProvider {
    /// Shared body of `invoke` and `invoke_streaming` (see the openai dialect): with a sink the
    /// request asks for streaming events, which are folded back into the blocking envelope.
    fn invoke_inner(
        &self,
        request: &ModelRequest,
        stream: Option<&mut dyn FnMut(&str)>,
    ) -> Result<ModelCallSuccess, ModelCallFailure> {
        let start = Instant::now();
        let timeout_ms = if request.timeout_ms > 0 {
            request.timeout_ms
        } else {
            self.config.default_timeout_ms
        };
        let mut payload = self.build_payload(request);
        if stream.is_some()
            && let Some(obj) = payload.as_object_mut()
        {
            obj.insert("stream".to_owned(), json!(true));
        }
        let mut req = self
            .client
            .post(&self.config.endpoint_url)
            .timeout(Duration::from_millis(timeout_ms))
            .json(&payload);
        for (k, v) in &self.config.extra_headers {
            req = req.header(k.as_str(), v.as_str());
        }
        if let Some(key) = &self.config.api_key {
            req = req.header(self.config.key_header.as_str(), key.expose());
        }
        let resp = req.send().map_err(|err| {
            let class = if err.is_timeout() {
                ProviderFailureClass::Timeout
            } else {
                ProviderFailureClass::Server
            };
            self.failure(
                request,
                class,
                err.status().map(|s| s.as_u16()),
                start.elapsed().as_millis() as u64,
                &err.to_string(),
            )
        })?;
        let status = resp.status();
        let is_event_stream = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.to_ascii_lowercase().contains("text/event-stream"));
        let body = match stream {
            Some(sink) if status.is_success() && is_event_stream => {
                let mut folder = crate::model::sse::MessagesStreamFolder::new(sink);
                let read = crate::model::sse::read_sse(resp, |d| folder.feed(d));
                let folded = folder.finish();
                read.map_err(|err| {
                    let class = if err.kind() == std::io::ErrorKind::TimedOut {
                        ProviderFailureClass::Timeout
                    } else {
                        ProviderFailureClass::Server
                    };
                    self.failure(
                        request,
                        class,
                        Some(status.as_u16()),
                        start.elapsed().as_millis() as u64,
                        &err.to_string(),
                    )
                })?;
                folded.envelope.to_string()
            }
            _ => resp.text().map_err(|err| {
                let class = if err.is_timeout() {
                    ProviderFailureClass::Timeout
                } else {
                    ProviderFailureClass::Server
                };
                self.failure(
                    request,
                    class,
                    Some(status.as_u16()),
                    start.elapsed().as_millis() as u64,
                    &err.to_string(),
                )
            })?,
        };
        let latency_ms = start.elapsed().as_millis() as u64;
        if !status.is_success() {
            let code = status.as_u16();
            return Err(self.failure(
                request,
                classify_http_status(code),
                Some(code),
                latency_ms,
                &format!("HTTP {code}: {body}"),
            ));
        }
        let parsed: MessagesResponse = serde_json::from_str(&body).map_err(|err| {
            self.failure(
                request,
                ProviderFailureClass::InvalidOutput,
                Some(status.as_u16()),
                latency_ms,
                &format!("malformed Messages-API JSON envelope: {err}"),
            )
        })?;
        let usage = parsed.usage.map(|u| TokenUsage {
            input_tokens: u.input_tokens,
            output_tokens: u.output_tokens,
            cached_tokens: u.cache_read_input_tokens,
            reasoning_tokens: None,
            total_tokens: u.input_tokens + u.output_tokens,
        });
        let text: String = parsed
            .content
            .iter()
            .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
            .filter_map(|b| b.get("text").and_then(Value::as_str))
            .collect();
        if text.trim().is_empty() {
            let detail = if parsed.stop_reason.as_deref() == Some("max_tokens") {
                let setting = OutputLimitProfile::setting_name(request.purpose);
                let profile = self
                    .config
                    .profile_name
                    .as_deref()
                    .unwrap_or(&request.model);
                format!(
                    "response hit max_tokens with no text; raise '{setting}' for model profile '{profile}'"
                )
            } else {
                "Messages-API response contained no text content".to_owned()
            };
            return Err(self.failure(
                request,
                ProviderFailureClass::InvalidOutput,
                Some(status.as_u16()),
                latency_ms,
                &detail,
            ));
        }
        let resolved_model = parsed
            .model
            .filter(|m| !m.trim().is_empty())
            .unwrap_or_else(|| request.model.clone());
        decode_model_response(
            self.provider_name(),
            request,
            resolved_model,
            text,
            usage,
            latency_ms,
        )
        .map_err(|mut failure| {
            let secrets: Vec<&SecretString> = self.config.api_key.iter().collect();
            failure.detail = redact_secrets(&failure.detail, &secrets);
            failure
        })
    }
}
