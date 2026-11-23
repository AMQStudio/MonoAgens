//! Deterministic offline `scripted` model provider for tests and Lab runs.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

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
    /// Bytes per text delta when asked to stream (rounded up to a char boundary).
    stream_chunk_bytes: usize,
    /// Pause after each streamed delta, to make a turn observably slow in tests and smoke runs.
    stream_delay_ms: u64,
    streamed_calls: AtomicU64,
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
        Self::with_queue(VecDeque::new())
    }

    /// Creates a [`ScriptedModelProvider`] pre-loaded with sequential replies.
    #[must_use]
    pub fn from_replies(replies: impl IntoIterator<Item = ScriptedReply>) -> Self {
        Self::with_queue(replies.into_iter().collect())
    }

    fn with_queue(queue: VecDeque<ScriptedReply>) -> Self {
        Self {
            queue: Mutex::new(queue),
            fallback: None,
            stream_chunk_bytes: 16,
            stream_delay_ms: 0,
            streamed_calls: AtomicU64::new(0),
        }
    }

    /// Sets how a streamed call paces its text deltas: `chunk_bytes` per delta and
    /// `delay_ms` of sleep after each.
    #[must_use]
    pub fn with_stream_pacing(mut self, chunk_bytes: usize, delay_ms: u64) -> Self {
        self.stream_chunk_bytes = chunk_bytes.max(1);
        self.stream_delay_ms = delay_ms;
        self
    }

    /// Number of calls made through `invoke_streaming` (a request that asked for streaming).
    #[must_use]
    pub fn streamed_calls(&self) -> u64 {
        self.streamed_calls.load(Ordering::SeqCst)
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

    fn invoke_streaming(
        &self,
        request: &ModelRequest,
        on_text: &mut dyn FnMut(&str),
    ) -> Result<ModelCallSuccess, ModelCallFailure> {
        self.streamed_calls.fetch_add(1, Ordering::SeqCst);
        let upcoming = self
            .queue
            .lock()
            .expect("scripted provider queue mutex")
            .front()
            .cloned()
            .or_else(|| self.fallback.clone());
        if let Some(ScriptedReply::RawText { text, .. }) = upcoming {
            let mut rest = text.as_str();
            while !rest.is_empty() {
                let mut cut = self.stream_chunk_bytes.min(rest.len());
                while !rest.is_char_boundary(cut) {
                    cut += 1;
                }
                let (head, tail) = rest.split_at(cut);
                on_text(head);
                rest = tail;
                if self.stream_delay_ms > 0 {
                    std::thread::sleep(Duration::from_millis(self.stream_delay_ms));
                }
            }
        }
        self.invoke(request)
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
                    strict_schema: request.response_schema
                        != crate::model::ResponseSchemaMode::None,
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
                    strict_schema: request.response_schema
                        != crate::model::ResponseSchemaMode::None,
                }),
                status_code,
                failed_wake: None,
                detail: redact_secrets(&detail, &[]),
            }),
        }
    }
}
