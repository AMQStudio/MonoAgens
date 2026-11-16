//! Canonical `body-cli` Waist-2 body (`monoagens.body.v1`): terminal place, local owner
//! account, `chat.speak` with `d_art = 0 ms`, honest `1 s` `working_cue` and `10 s`
//! `progress_note` cues (`R-47`), and `R-09` echo correlation.

#![forbid(unsafe_code)]

use std::collections::HashSet;
use std::io::{self, Write};

use monoagens_body_kit::{
    BargeInToken, BubbleSender, EchoRegistry, EmittedCue, HonestCueKind, HonestCueThresholds,
    HonestCueTracker, capability_schema_hash, deliver_split_bubbles, make_refused_receipt,
    make_terminal_receipt,
};
use monoagens_body_sdk::{
    ActionCancelParams, ActionCancelResult, ActionExecuteParams, ActionExecuteResult, ActionId,
    ActorId, ApprovalResolveParams, ApprovalResolveResult, Audience, BodyDiscoverParams,
    BodyDiscoverResult, BodyHandler, BodyId, CURRENT_PROTOCOL_VERSION, CapabilityDescriptor,
    CapabilityFlags, CapabilityId, ChannelId, CorrelationId, MetaValue, OpaquePayload, OriginActor,
    PerceptEmitParams, PerceptEmitResult, PlaceId, Receipt, ReceiptOutcome, SensedEffect, Taint,
    TerminationReason, Time, Waist2Error, Waist2Headers, sort_capabilities,
};
use serde_json::json;

/// Default Waist-2 body identifier for `body-cli`.
pub const DEFAULT_BODY_ID: &str = "body-cli";
/// Default terminal place owned by `body-cli`.
pub const DEFAULT_TERMINAL_PLACE: &str = "cli:terminal";
/// Default local owner actor identifier.
pub const DEFAULT_OWNER_ACTOR: &str = "per_owner";
/// Default platform chunk cap `L_cap` for terminal bubbles.
pub const DEFAULT_BUBBLE_CAP: usize = 4_096;

/// Output sink receiving immediately rendered text (`d_art = 0 ms`) and honest lifecycle cues.
pub trait TerminalOutputSink {
    fn render_bubble(&mut self, pid: &str, text: &str) -> Result<(), String>;
    fn render_cue(&mut self, cue: &EmittedCue) -> Result<(), String>;
}

/// Terminal sink writing rendered chat text and cues to an arbitrary `Write` stream (e.g. `stderr`).
pub struct WriterTerminalSink<W: Write> {
    writer: W,
}

impl<W: Write> WriterTerminalSink<W> {
    /// Wraps `writer` as a [`TerminalOutputSink`].
    #[must_use]
    pub fn new(writer: W) -> Self {
        Self { writer }
    }

    /// Borrows the underlying writer.
    #[must_use]
    pub fn writer(&self) -> &W {
        &self.writer
    }
}

impl<W: Write> TerminalOutputSink for WriterTerminalSink<W> {
    fn render_bubble(&mut self, _pid: &str, text: &str) -> Result<(), String> {
        writeln!(self.writer, "{text}").map_err(|e| e.to_string())?;
        self.writer.flush().map_err(|e| e.to_string())
    }

    fn render_cue(&mut self, cue: &EmittedCue) -> Result<(), String> {
        let label = match cue.kind {
            HonestCueKind::WorkingCue => "working...",
            HonestCueKind::ProgressNote => "still working (waiting on completion)...",
        };
        writeln!(self.writer, "[{}] {label}", cue.wait_id).map_err(|e| e.to_string())?;
        self.writer.flush().map_err(|e| e.to_string())
    }
}

/// Default stderr sink so NDJSON `stdout` stays strictly reserved for Waist-2 RPC frames.
pub struct StderrTerminalSink;

impl TerminalOutputSink for StderrTerminalSink {
    fn render_bubble(&mut self, _pid: &str, text: &str) -> Result<(), String> {
        let mut err = io::stderr().lock();
        writeln!(err, "{text}").map_err(|e| e.to_string())?;
        err.flush().map_err(|e| e.to_string())
    }

    fn render_cue(&mut self, cue: &EmittedCue) -> Result<(), String> {
        let label = match cue.kind {
            HonestCueKind::WorkingCue => "working...",
            HonestCueKind::ProgressNote => "still working (waiting on completion)...",
        };
        let mut err = io::stderr().lock();
        writeln!(err, "[{}] {label}", cue.wait_id).map_err(|e| e.to_string())?;
        err.flush().map_err(|e| e.to_string())
    }
}

/// Recording sink used in unit tests to assert `d_art = 0 ms`, delivered chunks, and cues.
#[derive(Debug, Default, Clone)]
pub struct MemoryTerminalSink {
    pub rendered: Vec<(String, String)>,
    pub cues: Vec<EmittedCue>,
}

impl TerminalOutputSink for MemoryTerminalSink {
    fn render_bubble(&mut self, pid: &str, text: &str) -> Result<(), String> {
        self.rendered.push((pid.to_owned(), text.to_owned()));
        Ok(())
    }

    fn render_cue(&mut self, cue: &EmittedCue) -> Result<(), String> {
        self.cues.push(cue.clone());
        Ok(())
    }
}

/// Canonical `body-cli` Waist-2 body implementation.
pub struct CliBody<S: TerminalOutputSink = StderrTerminalSink> {
    body_id: BodyId,
    place: PlaceId,
    owner_actor: ActorId,
    bubble_cap: usize,
    thresholds: HonestCueThresholds,
    sink: S,
    echo_registry: EchoRegistry,
    barge_in: BargeInToken,
    held_actions: HashSet<ActionId>,
    active_wait: Option<HonestCueTracker>,
    next_msg_seq: u64,
    clock_now_ms: u64,
}

impl CliBody<StderrTerminalSink> {
    /// Creates a `body-cli` instance rendering to `stderr` with canonical defaults.
    #[must_use]
    pub fn with_stderr() -> Self {
        Self::new(
            BodyId::new(DEFAULT_BODY_ID),
            PlaceId::new(DEFAULT_TERMINAL_PLACE),
            ActorId::new(DEFAULT_OWNER_ACTOR),
            DEFAULT_BUBBLE_CAP,
            StderrTerminalSink,
        )
    }
}

impl<S: TerminalOutputSink> CliBody<S> {
    /// Creates a configurable `CliBody` with an injected [`TerminalOutputSink`].
    #[must_use]
    pub fn new(
        body_id: BodyId,
        place: PlaceId,
        owner_actor: ActorId,
        bubble_cap: usize,
        sink: S,
    ) -> Self {
        Self {
            body_id,
            place,
            owner_actor,
            bubble_cap: bubble_cap.max(1),
            thresholds: HonestCueThresholds::default(),
            sink,
            echo_registry: EchoRegistry::default(),
            barge_in: BargeInToken::new(),
            held_actions: HashSet::new(),
            active_wait: None,
            next_msg_seq: 0,
            clock_now_ms: 0,
        }
    }

    /// Returns the owned terminal [`PlaceId`].
    #[must_use]
    pub fn place(&self) -> &PlaceId {
        &self.place
    }

    /// Returns the local owner [`ActorId`].
    #[must_use]
    pub fn owner_actor(&self) -> &ActorId {
        &self.owner_actor
    }

    /// Returns a reference to the [`EchoRegistry`] for `R-09` self-echo/reply correlation.
    #[must_use]
    pub fn echo_registry(&self) -> &EchoRegistry {
        &self.echo_registry
    }

    /// Returns a reference to the output sink.
    #[must_use]
    pub fn sink(&self) -> &S {
        &self.sink
    }

    /// Returns a mutable reference to the output sink.
    pub fn sink_mut(&mut self) -> &mut S {
        &mut self.sink
    }

    /// Advances the local virtual/injected clock timestamp used for receipts and cues.
    pub fn set_now_ms(&mut self, now_ms: u64) {
        self.clock_now_ms = now_ms;
    }

    /// Starts tracking a genuine in-flight wait (`working_cue` at 1s, `progress_note` at 10s).
    pub fn begin_wait(&mut self, wait_id: impl Into<String>, now_ms: u64) {
        self.clock_now_ms = now_ms;
        self.active_wait = Some(HonestCueTracker::start(wait_id, now_ms, self.thresholds));
    }

    /// Completes the active wait, cancelling any un-emitted cues.
    pub fn finish_wait(&mut self) {
        if let Some(wait) = &mut self.active_wait {
            wait.finish();
        }
        self.active_wait = None;
    }

    /// Polls active wait cues at `now_ms`, rendering any newly crossed thresholds to the sink.
    pub fn poll_wait_cues(&mut self, now_ms: u64) -> Vec<EmittedCue> {
        self.clock_now_ms = now_ms;
        let Some(wait) = &mut self.active_wait else {
            return Vec::new();
        };
        let cues = wait.poll(now_ms);
        for cue in &cues {
            let _ = self.sink.render_cue(cue);
        }
        cues
    }

    /// Triggers mid-stream barge-in (e.g. local owner typed new input while speaking).
    pub fn trigger_barge_in(&self) {
        self.barge_in.trigger_external();
    }

    /// Translates local terminal user input into a Waist-2 `percept/emit` payload,
    /// correlating echo/reply-to IDs against [`EchoRegistry`].
    #[must_use]
    pub fn build_user_percept(
        &self,
        text: &str,
        platform_msg_id: Option<&str>,
        reply_to_pid: Option<&str>,
        timestamp_ms: Time,
        idempotency_key: impl Into<String>,
    ) -> (PerceptEmitParams, bool) {
        let is_self_echo = platform_msg_id.is_some_and(|pid| self.echo_registry.is_self_echo(pid));
        let correlation_id = platform_msg_id.map(CorrelationId::new).or_else(|| {
            reply_to_pid
                .and_then(|pid| self.echo_registry.resolve_reply_to(pid))
                .map(|rec| CorrelationId::new(rec.action_id.as_str()))
        });

        let params = PerceptEmitParams {
            headers: Waist2Headers::new(idempotency_key),
            actor: self.owner_actor.clone(),
            origin_actor: Some(OriginActor::new(self.owner_actor.clone())),
            place: self.place.clone(),
            audience: Some(Audience::new([self.owner_actor.clone()])),
            timestamp_ms,
            payload: OpaquePayload::from_bytes(text.as_bytes()),
            taint: Taint::Trusted,
            addressees: Vec::new(),
            named: Vec::new(),
            orient: Vec::new(),
            reply_to: None,
            correlation_id,
            channel: Some(ChannelId::new(self.place.as_str())),
            delta_obs: None,
            channel_prediction: None,
            length: Some(text.len() as u64),
            negative_flag: None,
            repair: None,
            continued_attention: None,
            f_modal: None,
            embeddings: Vec::new(),
            attachments: Vec::new(),
            dialogue_act_gap: None,
        };
        (params, is_self_echo)
    }

    fn chat_speak_descriptor(&self) -> CapabilityDescriptor {
        let schema = json!({
            "capability": "chat.speak",
            "input": {
                "type": "object",
                "properties": {
                    "text": { "type": "string" }
                }
            },
            "place": self.place.as_str()
        });
        CapabilityDescriptor {
            id: CapabilityId::new("chat.speak"),
            description:
                "Render text immediately to the local terminal place with R-47 feedback cues"
                    .to_owned(),
            places: vec![self.place.clone()],
            flags: CapabilityFlags {
                communicative: true,
                identity_changing: false,
                irreversible: false,
            },
            typical_latency_ms: 0,
            capability_schema_hash: capability_schema_hash(&schema),
            sensed_effect: Some(SensedEffect {
                channel: ChannelId::new(self.place.as_str()),
                window_ms: 500,
                noise: 0.0,
                delta_pred: 1.0,
            }),
        }
    }
}

struct CliBubbleSink<'a, S: TerminalOutputSink> {
    sink: &'a mut S,
    next_msg_seq: &'a mut u64,
}

impl<S: TerminalOutputSink> BubbleSender for CliBubbleSink<'_, S> {
    fn send_bubble(&mut self, _chunk_index: usize, chunk_text: &str) -> Result<String, String> {
        *self.next_msg_seq = self.next_msg_seq.saturating_add(1);
        let pid = format!("cli-msg-{}", *self.next_msg_seq);
        self.sink.render_bubble(&pid, chunk_text)?;
        Ok(pid)
    }
}

impl<S: TerminalOutputSink> BodyHandler for CliBody<S> {
    fn discover(&mut self, params: BodyDiscoverParams) -> Result<BodyDiscoverResult, Waist2Error> {
        let mut capabilities = vec![self.chat_speak_descriptor()];
        sort_capabilities(&mut capabilities);
        let places = vec![self.place.clone()];
        Ok(BodyDiscoverResult {
            body_id: params.body_id.unwrap_or_else(|| self.body_id.clone()),
            protocol_version: CURRENT_PROTOCOL_VERSION,
            capabilities,
            places,
        })
    }

    fn execute(&mut self, params: ActionExecuteParams) -> Result<ActionExecuteResult, Waist2Error> {
        if params.audience.is_empty() {
            return Err(Waist2Error::undeclared_audience());
        }
        if params.capability_id.as_str() != "chat.speak" {
            return Err(Waist2Error::unknown_capability(&params.capability_id));
        }

        let place_ok = !params.places.is_empty() && params.places.iter().all(|p| p == &self.place);
        if !place_ok {
            let receipt = make_refused_receipt(
                params.action_id,
                params.capability_id,
                "ERR_UNKNOWN_PLACE",
                "action targets a place not owned by body-cli",
                params.h_act,
                Time::from_millis(self.clock_now_ms),
            );
            return Ok(ActionExecuteResult {
                accepted: false,
                receipt: Some(receipt),
            });
        }

        if params.h_act.is_some() && params.payload.as_bytes() == b"hold" {
            self.held_actions.insert(params.action_id.clone());
            let receipt = make_terminal_receipt(
                params.action_id,
                params.capability_id,
                ReceiptOutcome::Held,
                Some("PendingApproval".to_owned()),
                Some("awaiting owner approval before terminal output".to_owned()),
                Some(TerminationReason::Normal),
                Some(0.0),
                Some(0),
                params.h_act,
                Time::from_millis(self.clock_now_ms),
            );
            return Ok(ActionExecuteResult {
                accepted: true,
                receipt: Some(receipt),
            });
        }

        let is_delivery_probe = params.payload.as_bytes() == b"delivery-timing-probe"
            || params.headers.meta.get("org.monoagens.delivery_probe")
                == Some(&MetaValue::Bool(true));

        let text = String::from_utf8_lossy(params.payload.as_bytes()).into_owned();
        let mut bubble_sink = CliBubbleSink {
            sink: &mut self.sink,
            next_msg_seq: &mut self.next_msg_seq,
        };
        let mut outcome = deliver_split_bubbles(
            params.action_id,
            params.capability_id,
            &text,
            self.bubble_cap,
            &mut bubble_sink,
            &self.barge_in,
            &mut self.echo_registry,
            params.h_act,
            Time::from_millis(self.clock_now_ms),
        )
        .map_err(|e| Waist2Error::invalid_receipt_transition(e.to_string()))?;

        // Reset barge-in flag after handling cancellation if triggered.
        if self.barge_in.is_triggered() {
            self.barge_in = BargeInToken::new();
        }

        if is_delivery_probe {
            outcome.receipt.detail = Some(
                json!({
                    "d_art_ms": self.thresholds.d_art_ms,
                    "working_cue_ms": self.thresholds.working_cue_ms,
                    "progress_note_ms": self.thresholds.progress_note_ms,
                    "pids": outcome.delivered_pids,
                })
                .to_string(),
            );
        } else if let Some(first_pid) = outcome.delivered_pids.first() {
            outcome.receipt.detail = Some(
                json!({
                    "pids": outcome.delivered_pids,
                    "primary_pid": first_pid,
                })
                .to_string(),
            );
        }

        Ok(ActionExecuteResult {
            accepted: true,
            receipt: Some(outcome.receipt),
        })
    }

    fn cancel(&mut self, params: ActionCancelParams) -> Result<ActionCancelResult, Waist2Error> {
        self.barge_in.trigger_core_cancel();
        let receipt = make_terminal_receipt(
            params.action_id,
            CapabilityId::new("chat.speak"),
            ReceiptOutcome::Failed,
            Some("canceled_by_core".to_owned()),
            params.reason,
            Some(TerminationReason::CanceledByCore),
            Some(0.0),
            Some(0),
            None,
            Time::from_millis(self.clock_now_ms),
        );
        Ok(ActionCancelResult {
            cancelled: true,
            receipt: Some(receipt),
        })
    }

    fn resolve_approval(
        &mut self,
        params: ApprovalResolveParams,
    ) -> Result<ApprovalResolveResult, Waist2Error> {
        if self.held_actions.remove(&params.action_id) {
            if params.approved {
                let mut bubble_sink = CliBubbleSink {
                    sink: &mut self.sink,
                    next_msg_seq: &mut self.next_msg_seq,
                };
                if let Ok(pid) = bubble_sink.send_bubble(0, "[approved]") {
                    self.echo_registry.register_echo(
                        pid,
                        params.action_id,
                        0,
                        params.resolved_at_ms,
                    );
                }
            }
            return Ok(ApprovalResolveResult { applied: true });
        }
        Ok(ApprovalResolveResult { applied: false })
    }

    fn on_percept(&mut self, params: PerceptEmitParams) -> Result<PerceptEmitResult, Waist2Error> {
        if params.audience.as_ref().is_none_or(Audience::is_empty) {
            return Err(Waist2Error::undeclared_audience());
        }
        if params.place != self.place {
            return Err(Waist2Error::invalid_receipt_transition(format!(
                "percept place '{}' is not owned by body-cli",
                params.place
            )));
        }
        Ok(PerceptEmitResult { accepted: true })
    }
}

/// Converts a terminal [`Receipt`] produced by `CliBody` into the registered platform IDs if any.
#[must_use]
pub fn extract_receipt_pids(receipt: &Receipt) -> Vec<String> {
    let Some(detail) = receipt.detail.as_deref() else {
        return Vec::new();
    };
    let Ok(val) = serde_json::from_str::<serde_json::Value>(detail) else {
        return Vec::new();
    };
    val.get("pids")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|x| x.as_str().map(ToOwned::to_owned))
                .collect()
        })
        .unwrap_or_default()
}
