//! Deterministic offline `scripted` model provider for tests and Lab runs.

use std::collections::VecDeque;
use std::sync::Mutex;

use crate::model::{
    CallOutcome, ModelCallFailure, ModelCallRecord, ModelCallSuccess, ModelProvider, ModelRequest,
    ProviderFailureClass, TokenUsage, decode_model_response,
};
use crate::secrets::redact_secrets;

/// Prepared deterministic reply queued on [`ScriptedModelProvider`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScriptedReply {
    RawText {
        text: String,
        usage: TokenUsage,
        latency_ms: u64,
    },
    Failure {
        class: ProviderFailureClass,
        status_code: Option<u16>,
        detail: String,
        latency_ms: u64,
    },
}

impl ScriptedReply {
    /// Builds a scripted raw text reply with deterministic token accounting.
    #[must_use]
    pub fn text(text: impl Into<String>) -> Self {
        let text = text.into();
        let out_tokens = (text.len().div_ceil(4)).max(1) as u64;
        Self::RawText {
            text,
            usage: TokenUsage {
                input_tokens: 32,
                output_tokens: out_tokens,
                cached_tokens: Some(16),
                reasoning_tokens: None,
                total_tokens: 32 + out_tokens,
            },
            latency_ms: 1,
        }
    }

    /// Builds a scripted failure reply.
    #[must_use]
    pub fn failure(class: ProviderFailureClass, detail: impl Into<String>) -> Self {
        Self::Failure {
            class,
            status_code: None,
            detail: detail.into(),
            latency_ms: 1,
        }
    }
}

/// Deterministic offline [`ModelProvider`] requiring zero network access.
#[derive(Debug)]
pub struct ScriptedModelProvider {
    queue: Mutex<VecDeque<ScriptedReply>>,
    fallback: Option<ScriptedReply>,
}

impl Default for ScriptedModelProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl ScriptedModelProvider {
    /// Creates an empty [`ScriptedModelProvider`].
    #[must_use]
    pub fn new() -> Self {
        Self {
            queue: Mutex::new(VecDeque::new()),
            fallback: None,
        }
    }

    /// Creates a [`ScriptedModelProvider`] pre-loaded with sequential replies.
    #[must_use]
    pub fn from_replies(replies: impl IntoIterator<Item = ScriptedReply>) -> Self {
        Self {
            queue: Mutex::new(replies.into_iter().collect()),
            fallback: None,
        }
    }

    /// Appends a prepared reply to the queue.
    pub fn push(&self, reply: ScriptedReply) {
        self.queue
            .lock()
            .expect("scripted provider queue mutex")
            .push_back(reply);
    }

    /// Configures a deterministic fallback reply when the queue is exhausted.
    pub fn set_fallback(&mut self, reply: ScriptedReply) {
        self.fallback = Some(reply);
    }
}

impl ModelProvider for ScriptedModelProvider {
    fn provider_name(&self) -> &str {
        "scripted"
    }

    fn invoke(&self, request: &ModelRequest) -> Result<ModelCallSuccess, ModelCallFailure> {
        let reply = self
            .queue
            .lock()
            .expect("scripted provider queue mutex")
            .pop_front()
            .or_else(|| self.fallback.clone())
            .ok_or_else(|| ModelCallFailure {
                class: ProviderFailureClass::InvalidOutput,
                record: Box::new(ModelCallRecord {
                    request_id: request.request_id,
                    purpose: request.purpose,
                    provider: self.provider_name().to_owned(),
                    model: request.model.clone(),
                    usage: None,
                    latency_ms: 0,
                    outcome: CallOutcome::Failed(ProviderFailureClass::InvalidOutput),
                }),
                status_code: None,
                failed_wake: None,
                detail: "scripted model provider queue exhausted with no fallback".to_owned(),
            })?;

        match reply {
            ScriptedReply::RawText {
                text,
                usage,
                latency_ms,
            } => decode_model_response(
                self.provider_name(),
                request,
                request.model.clone(),
                text,
                Some(usage),
                latency_ms,
            ),
            ScriptedReply::Failure {
                class,
                status_code,
                detail,
                latency_ms,
            } => Err(ModelCallFailure {
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
                detail: redact_secrets(&detail, &[]),
            }),
        }
    }
}
