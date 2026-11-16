//! OpenAI-compatible stateless HTTP (`store: false`) model provider.

use core::fmt;
use std::time::{Duration, Instant};

use monoagens_protocol::ModalitySlot;
use reqwest::blocking::Client;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::model::{
    CallOutcome, MessagePart, ModelCallFailure, ModelCallRecord, ModelCallSuccess, ModelProvider,
    ModelRequest, ProviderFailureClass, ResponseSchemaMode, TokenUsage, decode_model_response,
};
use crate::parse::{strict_classification_verdict_schema, strict_thought_batch_schema};
use crate::secrets::{SecretString, redact_secrets};

/// Configuration for [`OpenAiModelProvider`].
#[derive(Clone)]
pub struct OpenAiModelConfig {
    pub endpoint_url: String,
    pub api_key: Option<SecretString>,
    pub default_timeout_ms: u64,
}

impl fmt::Debug for OpenAiModelConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OpenAiModelConfig")
            .field("endpoint_url", &self.endpoint_url)
            .field("api_key", &self.api_key.as_ref().map(|_| "[REDACTED]"))
            .field("default_timeout_ms", &self.default_timeout_ms)
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
}

#[derive(Deserialize)]
struct PromptTokensDetails {
    cached_tokens: Option<u64>,
}

/// Stateless OpenAI-compatible HTTP [`ModelProvider`] sending `store: false` and strict schemas.
#[derive(Debug)]
pub struct OpenAiModelProvider {
    config: OpenAiModelConfig,
    client: Client,
}

impl OpenAiModelProvider {
    /// Builds a blocking HTTP provider with bounded request timeouts.
    pub fn new(config: OpenAiModelConfig) -> Result<Self, String> {
        if config.endpoint_url.trim().is_empty() {
            return Err("OpenAI-compatible endpoint_url must not be empty".to_owned());
        }
        let client = Client::builder()
            .timeout(Duration::from_millis(config.default_timeout_ms.max(1)))
            .build()
            .map_err(|err| format!("failed to build HTTP client: {err}"))?;
        Ok(Self { config, client })
    }

    fn scrub(&self, text: &str) -> String {
        match &self.config.api_key {
            Some(key) => redact_secrets(text, &[key]),
            None => redact_secrets(text, &[]),
        }
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
        }

        let mut req = self
            .client
            .post(&self.config.endpoint_url)
            .timeout(Duration::from_millis(timeout_ms))
            .json(&payload);
        if let Some(key) = &self.config.api_key {
            req = req.bearer_auth(key.expose());
        }

        let resp = req.send().map_err(|err| {
            let latency_ms = start.elapsed().as_millis() as u64;
            let class = if err.is_timeout() {
                ProviderFailureClass::Timeout
            } else {
                ProviderFailureClass::Server
            };
            self.make_failure(
                request,
                class,
                err.status().map(|s| s.as_u16()),
                latency_ms,
                &err.to_string(),
            )
        })?;

        let status = resp.status();
        let raw_body = resp.text().map_err(|err| {
            let latency_ms = start.elapsed().as_millis() as u64;
            let class = if err.is_timeout() {
                ProviderFailureClass::Timeout
            } else {
                ProviderFailureClass::Server
            };
            self.make_failure(
                request,
                class,
                Some(status.as_u16()),
                latency_ms,
                &err.to_string(),
            )
        })?;
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
            let cached_tokens = u.prompt_tokens_details.and_then(|d| d.cached_tokens);
            let total_tokens = u.total_tokens.unwrap_or(input_tokens + output_tokens);
            TokenUsage {
                input_tokens,
                output_tokens,
                cached_tokens,
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
