//! Unified language-model provider trait, request/response envelopes, usage records,
//! and secret-free failure classification.

use core::fmt;

use monoagens_core::{Input, ModelPurpose};
use monoagens_protocol::{ClassificationVerdict, EventId};
use serde::{Deserialize, Serialize};

use crate::attachments::ResolvedAttachment;
use crate::parse::{
    FailedWake, ParseOptions, ParsedModelActs, parse_classification_verdict,
    parse_thought_output_with_options,
};

pub mod openai;
pub mod scripted;

pub use openai::{OpenAiModelConfig, OpenAiModelProvider, ReasoningEffort};
pub use scripted::{ScriptedModelProvider, ScriptedReply};

/// Message role inside a [`ModelRequest`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    System,
    Developer,
    User,
    Assistant,
}

impl MessageRole {
    /// Wire role string for OpenAI-compatible chat payloads.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Developer => "developer",
            Self::User => "user",
            Self::Assistant => "assistant",
        }
    }
}

/// Content part inside a [`ModelMessage`].
///
/// Multimodal media bytes can ONLY be supplied via [`ResolvedAttachment`], which is
/// constructible solely after `VisibleScope` and context-door checks pass.
#[derive(Debug, Clone)]
pub enum MessagePart {
    Text(String),
    Attachment(ResolvedAttachment),
}

/// One ordered message in a stateless [`ModelRequest`].
#[derive(Debug, Clone)]
pub struct ModelMessage {
    pub role: MessageRole,
    pub parts: Vec<MessagePart>,
}

impl ModelMessage {
    /// Builds a text-only message for `role`.
    #[must_use]
    pub fn text(role: MessageRole, content: impl Into<String>) -> Self {
        Self {
            role,
            parts: vec![MessagePart::Text(content.into())],
        }
    }

    /// Builds a user message containing text and already-authorized attachments.
    #[must_use]
    pub fn with_attachments(
        role: MessageRole,
        text: impl Into<String>,
        attachments: Vec<ResolvedAttachment>,
    ) -> Self {
        let mut parts = Vec::with_capacity(1 + attachments.len());
        parts.push(MessagePart::Text(text.into()));
        for att in attachments {
            parts.push(MessagePart::Attachment(att));
        }
        Self { role, parts }
    }
}

/// Strict structured-output schema selection for [`ModelRequest`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResponseSchemaMode {
    None,
    ThoughtBatchStrict,
    ClassificationVerdictStrict,
}

/// Stateless model invocation request (`store: false` on remote endpoints).
#[derive(Debug, Clone)]
pub struct ModelRequest {
    pub request_id: EventId,
    pub purpose: ModelPurpose,
    pub model: String,
    pub messages: Vec<ModelMessage>,
    pub response_schema: ResponseSchemaMode,
    pub parse_options: ParseOptions,
    pub timeout_ms: u64,
    pub max_output_tokens: Option<u32>,
    pub temperature: Option<f32>,
}

impl ModelRequest {
    /// Constructs a stateless text request with default `30_000 ms` timeout and strict schema matching `purpose`.
    #[must_use]
    pub fn text(
        request_id: EventId,
        purpose: ModelPurpose,
        model: impl Into<String>,
        system_prompt: impl Into<String>,
        user_prompt: impl Into<String>,
    ) -> Self {
        let response_schema = match purpose {
            ModelPurpose::Reason | ModelPurpose::Consolidate => {
                ResponseSchemaMode::ThoughtBatchStrict
            }
            ModelPurpose::Classify => ResponseSchemaMode::ClassificationVerdictStrict,
        };
        Self {
            request_id,
            purpose,
            model: model.into(),
            messages: vec![
                ModelMessage::text(MessageRole::System, system_prompt),
                ModelMessage::text(MessageRole::User, user_prompt),
            ],
            response_schema,
            parse_options: ParseOptions::default(),
            timeout_ms: 30_000,
            max_output_tokens: None,
            temperature: None,
        }
    }
}

/// Token accounting reported for every provider call (including cached/reasoning tokens when stated).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cached_tokens: Option<u64>,
    #[serde(default)]
    pub reasoning_tokens: Option<u64>,
    pub total_tokens: u64,
}

/// Five canonical failure classes (`model-providers/spec.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderFailureClass {
    Authentication,
    RateLimit,
    Timeout,
    Server,
    InvalidOutput,
}

impl ProviderFailureClass {
    /// Canonical snake_case label.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Authentication => "authentication",
            Self::RateLimit => "rate_limit",
            Self::Timeout => "timeout",
            Self::Server => "server",
            Self::InvalidOutput => "invalid_output",
        }
    }
}

impl fmt::Display for ProviderFailureClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Outcome recorded on every [`ModelCallRecord`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CallOutcome {
    Success,
    Failed(ProviderFailureClass),
}

/// Per-call audit record emitted regardless of success or failure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelCallRecord {
    pub request_id: EventId,
    pub purpose: ModelPurpose,
    pub provider: String,
    pub model: String,
    pub usage: Option<TokenUsage>,
    pub latency_ms: u64,
    pub outcome: CallOutcome,
}

/// Typed decoded output returned on a successful model call.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ModelOutput {
    ThoughtActs(ParsedModelActs),
    Verdict(ClassificationVerdict),
}

/// Successful model invocation result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelCallSuccess {
    pub record: ModelCallRecord,
    pub output: ModelOutput,
    pub raw_text: String,
}

impl ModelCallSuccess {
    /// Converts a reasoning/consolidation result into Core's [`Input::ModelResult`].
    #[must_use]
    pub fn into_core_input(self) -> Option<Input> {
        match self.output {
            ModelOutput::ThoughtActs(parsed) => Some(Input::ModelResult {
                request_id: self.record.request_id,
                acts: parsed.acts,
            }),
            ModelOutput::Verdict(_) => None,
        }
    }
}

/// Secret-free recoverable failure returned by a [`ModelProvider`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelCallFailure {
    pub class: ProviderFailureClass,
    pub record: Box<ModelCallRecord>,
    pub status_code: Option<u16>,
    pub failed_wake: Option<Box<FailedWake>>,
    pub detail: String,
}

impl ModelCallFailure {
    /// Converts this failure into Core's [`Input::ModelFailure`] so the loop continues safely.
    #[must_use]
    pub fn to_core_input(&self) -> Input {
        Input::ModelFailure {
            request_id: self.record.request_id,
            reason: self.to_string(),
        }
    }
}

impl fmt::Display for ModelCallFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "model call failed ({}, model '{}', latency {}ms): {}",
            self.class, self.record.model, self.record.latency_ms, self.detail
        )
    }
}

impl std::error::Error for ModelCallFailure {}

/// Single provider interface implemented by `scripted` and `openai` adapters.
pub trait ModelProvider: Send + Sync {
    /// Provider adapter name (e.g. `"scripted"`, `"openai_compatible"`).
    fn provider_name(&self) -> &str;

    /// Executes a stateless model call under `request.timeout_ms`.
    fn invoke(&self, request: &ModelRequest) -> Result<ModelCallSuccess, ModelCallFailure>;
}

/// Shared parser entrypoint used identically by `scripted` and HTTP providers (`9.6`).
pub(crate) fn decode_model_response(
    provider_name: &str,
    request: &ModelRequest,
    resolved_model: String,
    raw_text: String,
    usage: Option<TokenUsage>,
    latency_ms: u64,
) -> Result<ModelCallSuccess, ModelCallFailure> {
    let build_record = |outcome: CallOutcome| ModelCallRecord {
        request_id: request.request_id,
        purpose: request.purpose,
        provider: provider_name.to_owned(),
        model: resolved_model.clone(),
        usage,
        latency_ms,
        outcome,
    };

    let output = match request.purpose {
        ModelPurpose::Reason | ModelPurpose::Consolidate => {
            match parse_thought_output_with_options(&raw_text, request.parse_options) {
                Ok(parsed) => ModelOutput::ThoughtActs(parsed),
                Err(fw) => {
                    let detail = fw.to_string();
                    return Err(ModelCallFailure {
                        class: ProviderFailureClass::InvalidOutput,
                        record: Box::new(build_record(CallOutcome::Failed(
                            ProviderFailureClass::InvalidOutput,
                        ))),
                        status_code: Some(200),
                        failed_wake: Some(Box::new(fw)),
                        detail,
                    });
                }
            }
        }
        ModelPurpose::Classify => match parse_classification_verdict(&raw_text) {
            Ok(verdict) => ModelOutput::Verdict(verdict),
            Err(fw) => {
                let detail = fw.to_string();
                return Err(ModelCallFailure {
                    class: ProviderFailureClass::InvalidOutput,
                    record: Box::new(build_record(CallOutcome::Failed(
                        ProviderFailureClass::InvalidOutput,
                    ))),
                    status_code: Some(200),
                    failed_wake: Some(Box::new(fw)),
                    detail,
                });
            }
        },
    };

    Ok(ModelCallSuccess {
        record: build_record(CallOutcome::Success),
        output,
        raw_text,
    })
}
