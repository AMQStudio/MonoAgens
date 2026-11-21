//! `body-voice`: embodied voice body implementing AEC -> Silero/WebRTC VAD -> causal
//! z-scored `f_modal` -> `< 50 ms` TTS barge-in mute and single Option A `Receipt`
//! (`monoagens.body.v1`).

#![forbid(unsafe_code)]

pub mod audio;

use std::collections::HashSet;
use std::path::Path;

use monoagens_body_kit::{capability_schema_hash, make_refused_receipt, make_terminal_receipt};
use monoagens_body_sdk::{
    ActionCancelParams, ActionCancelResult, ActionExecuteParams, ActionExecuteResult, ActionId,
    ActorId, ApprovalResolveParams, ApprovalResolveResult, Audience, BodyDiscoverParams,
    BodyDiscoverResult, BodyHandler, BodyId, CURRENT_PROTOCOL_VERSION, CapabilityDescriptor,
    CapabilityFlags, CapabilityId, ChannelId, OpaquePayload, OriginActor, PerceptEmitParams,
    PerceptEmitResult, PlaceId, Receipt, ReceiptOutcome, SensedEffect, Taint, TerminationReason,
    Time, Waist2Error, Waist2Headers, sort_capabilities,
};
use serde::{Deserialize, Serialize};
use serde_json::json;

pub use audio::{
    AcousticEchoCanceller, ActiveVadKind, BARGE_SUSTAIN_MS, BARGE_VAD_PROB_THRESHOLD,
    BargeInDecision, BargeInMonitor, CausalFModalExtractor, CausalZScore, FRAME_DURATION_MS,
    FRAME_SAMPLES_20MS, LOCAL_MUTE_BUDGET_MS, SimulatedVoiceClock, TOTAL_CUTOFF_BUDGET_MS,
    VOICE_SAMPLE_RATE_HZ, VadBackend, VoiceClock, WebRtcVadBackend, select_vad_backend,
};

/// Default Waist-2 body ID for `body-voice`.
pub const DEFAULT_BODY_ID: &str = "body-voice";
/// Default voice place owned by `body-voice`.
pub const DEFAULT_VOICE_PLACE: &str = "voice:local";
/// Default local speaker actor ID.
pub const DEFAULT_VOICE_ACTOR: &str = "per_owner";

/// Local TTS playback sink muted synchronously on barge-in.
pub trait TtsPlaybackSink {
    fn start_utterance(&mut self, action_id: &ActionId, text: &str, total_units: u64);
    fn mute(&mut self, now_ms: u64);
    fn is_muted(&self) -> bool;
}

/// Deterministic in-memory TTS playback sink for unit tests and headless execution.
#[derive(Debug, Default, Clone)]
pub struct MemoryTtsSink {
    pub active_action: Option<ActionId>,
    pub text: String,
    pub total_units: u64,
    pub delivered_units: u64,
    pub progress: f32,
    pub muted: bool,
    pub muted_at_ms: Option<u64>,
}

impl TtsPlaybackSink for MemoryTtsSink {
    fn start_utterance(&mut self, action_id: &ActionId, text: &str, total_units: u64) {
        self.active_action = Some(action_id.clone());
        self.text = text.to_owned();
        self.total_units = total_units.max(1);
        self.delivered_units = 0;
        self.progress = 0.0;
        self.muted = false;
        self.muted_at_ms = None;
    }

    fn mute(&mut self, now_ms: u64) {
        self.muted = true;
        self.muted_at_ms = Some(now_ms);
    }

    fn is_muted(&self) -> bool {
        self.muted
    }
}

/// Latency report from an interruption event, explicitly distinguishing simulated vs hardware clocks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MuteLatencyMeasurement {
    pub clock_kind: &'static str,
    pub local_mute_latency_ms: u64,
    pub total_cutoff_latency_ms: u64,
}

/// Active `speech.speak` session state driving incremental playback + barge-in monitoring.
#[derive(Debug, Clone)]
pub struct ActiveSpeechSession {
    pub action_id: ActionId,
    pub capability_id: CapabilityId,
    pub h_act: Option<monoagens_body_sdk::HAct>,
    pub total_units: u64,
    pub delivered_units: u64,
    pub progress: f32,
    pub settled: bool,
}

/// Waist-2 voice body uniting AEC, Silero/WebRTC VAD, causal `f_modal`, and `< 50 ms` barge-in.
pub struct VoiceBody<T: TtsPlaybackSink = MemoryTtsSink> {
    body_id: BodyId,
    place: PlaceId,
    actor: ActorId,
    vad: Box<dyn VadBackend>,
    vad_fallback_note: Option<String>,
    aec: AcousticEchoCanceller,
    f_modal: CausalFModalExtractor,
    barge_monitor: BargeInMonitor,
    tts_sink: T,
    active_session: Option<ActiveSpeechSession>,
    held_actions: HashSet<ActionId>,
    last_mute_measurement: Option<MuteLatencyMeasurement>,
    clock_now_ms: u64,
}

impl VoiceBody<MemoryTtsSink> {
    /// Creates a `VoiceBody` checking `silero_model_path` (falling back to WebRTC VAD if absent).
    #[must_use]
    pub fn with_memory_sink(silero_model_path: Option<&Path>) -> Self {
        Self::new(
            BodyId::new(DEFAULT_BODY_ID),
            PlaceId::new(DEFAULT_VOICE_PLACE),
            ActorId::new(DEFAULT_VOICE_ACTOR),
            silero_model_path,
            MemoryTtsSink::default(),
        )
    }
}

impl<T: TtsPlaybackSink> VoiceBody<T> {
    /// Creates a configurable [`VoiceBody`].
    #[must_use]
    pub fn new(
        body_id: BodyId,
        place: PlaceId,
        actor: ActorId,
        silero_model_path: Option<&Path>,
        tts_sink: T,
    ) -> Self {
        let (vad, vad_fallback_note) = select_vad_backend(silero_model_path);
        Self {
            body_id,
            place,
            actor,
            vad,
            vad_fallback_note,
            aec: AcousticEchoCanceller::default(),
            f_modal: CausalFModalExtractor::default(),
            barge_monitor: BargeInMonitor::default(),
            tts_sink,
            active_session: None,
            held_actions: HashSet::new(),
            last_mute_measurement: None,
            clock_now_ms: 0,
        }
    }

    /// Returns the active VAD backend (`SileroOnnx` or `WebRtcFallback`).
    #[must_use]
    pub fn active_vad_kind(&self) -> ActiveVadKind {
        self.vad.kind()
    }

    /// Returns any fallback note emitted when Silero model weights were absent.
    #[must_use]
    pub fn vad_fallback_note(&self) -> Option<&str> {
        self.vad_fallback_note.as_deref()
    }

    /// Borrows the TTS playback sink.
    #[must_use]
    pub fn tts_sink(&self) -> &T {
        &self.tts_sink
    }

    /// Returns the most recent mute latency measurement (with `clock_kind` explicitly labelled).
    #[must_use]
    pub fn last_mute_measurement(&self) -> Option<&MuteLatencyMeasurement> {
        self.last_mute_measurement.as_ref()
    }

    /// Starts a simulated streaming `speech.speak` playback session for frame-by-frame barge-in tests.
    pub fn start_streaming_speech(
        &mut self,
        action_id: ActionId,
        text: &str,
        total_units: u64,
        h_act: Option<monoagens_body_sdk::HAct>,
    ) {
        self.barge_monitor.reset();
        self.last_mute_measurement = None;
        self.tts_sink.start_utterance(&action_id, text, total_units);
        self.active_session = Some(ActiveSpeechSession {
            action_id,
            capability_id: CapabilityId::new("speech.speak"),
            h_act,
            total_units: total_units.max(1),
            delivered_units: 0,
            progress: 0.0,
            settled: false,
        });
    }

    /// Updates playback progress (`progress` in `[0.0, 1.0]`, `delivered_units`).
    pub fn set_playback_progress(&mut self, progress: f32, delivered_units: u64) {
        if let Some(session) = &mut self.active_session {
            session.progress = progress.clamp(0.0, 1.0);
            session.delivered_units = delivered_units;
        }
    }

    /// Processes a raw 20 ms `16 kHz` mic frame through AEC -> VAD -> causal `f_modal` -> barge-in,
    /// returning `(percept, Option<Receipt>)` where a barge-in emits AT MOST ONE `Receipt`.
    pub fn process_audio_frame(
        &mut self,
        mic_16k: &[f32],
        tts_ref_16k: Option<&[f32]>,
        now_ms: u64,
        clock_kind: &'static str,
        idempotency_key: impl Into<String>,
    ) -> Result<(PerceptEmitParams, Option<Receipt>), String> {
        self.clock_now_ms = now_ms;
        let post_aec = self.aec.cancel(mic_16k, tts_ref_16k);
        let vad_prob = self.vad.frame_vad_prob(&post_aec)?;
        self.process_vad_observation(
            &post_aec,
            vad_prob,
            FRAME_DURATION_MS,
            now_ms,
            clock_kind,
            idempotency_key,
        )
    }

    /// Processes a post-AEC frame with an explicit `vad_prob` (for deterministic threshold boundary tests
    /// such as `0.69` vs `0.70` and `119 ms` vs `120 ms`).
    pub fn process_vad_observation(
        &mut self,
        post_aec: &[f32],
        vad_prob: f32,
        frame_ms: u64,
        now_ms: u64,
        clock_kind: &'static str,
        idempotency_key: impl Into<String>,
    ) -> Result<(PerceptEmitParams, Option<Receipt>), String> {
        self.clock_now_ms = now_ms;
        let f_modal = self.f_modal.extract(post_aec, vad_prob);
        let percept = PerceptEmitParams {
            headers: Waist2Headers::new(idempotency_key),
            actor: self.actor.clone(),
            origin_actor: Some(OriginActor::new(self.actor.clone())),
            place: self.place.clone(),
            audience: Some(Audience::new([self.actor.clone()])),
            timestamp_ms: Time::from_millis(now_ms),
            payload: OpaquePayload::from_bytes(
                json!({
                    "vad_prob": vad_prob,
                    "speech_active": f_modal.0[1] > 0.5,
                    "vad_backend": self.vad.kind().as_str(),
                })
                .to_string()
                .into_bytes(),
            ),
            taint: Taint::Untrusted,
            addressees: Vec::new(),
            named: Vec::new(),
            orient: Vec::new(),
            reply_to: None,
            correlation_id: None,
            channel: Some(ChannelId::new("voice:mic")),
            delta_obs: None,
            channel_prediction: None,
            length: Some(post_aec.len() as u64),
            negative_flag: None,
            repair: None,
            continued_attention: None,
            f_modal: Some(f_modal),
            embeddings: Vec::new(),
            attachments: Vec::new(),
            dialogue_act_gap: None,
            peer_expertise_margin: None,
        };

        let decision = self.barge_monitor.observe(vad_prob, frame_ms, now_ms);
        let mut emitted_receipt = None;

        if let BargeInDecision::TriggerMute {
            threshold_crossed_at_ms,
            muted_at_ms,
            local_mute_latency_ms,
        } = decision
        {
            self.tts_sink.mute(muted_at_ms);
            self.last_mute_measurement = Some(MuteLatencyMeasurement {
                clock_kind,
                local_mute_latency_ms,
                total_cutoff_latency_ms: muted_at_ms.saturating_sub(threshold_crossed_at_ms),
            });
            if let Some(session) = &mut self.active_session
                && !session.settled
            {
                session.settled = true;
                emitted_receipt = Some(make_terminal_receipt(
                    session.action_id.clone(),
                    session.capability_id.clone(),
                    ReceiptOutcome::Failed,
                    Some("preempted_external".to_owned()),
                    Some(format!(
                        "voice barge-in muted TTS in {local_mute_latency_ms}ms ({clock_kind})"
                    )),
                    Some(TerminationReason::PreemptedExternal),
                    Some(session.progress),
                    Some(session.delivered_units),
                    session.h_act.clone(),
                    Time::from_millis(now_ms),
                ));
            }
        }

        Ok((percept, emitted_receipt))
    }

    /// Finishes an active non-interrupted `speech.speak` session, returning its `Done` receipt
    /// if it was not already preempted by barge-in.
    pub fn finish_streaming_speech(&mut self, now_ms: u64) -> Option<Receipt> {
        let mut session = self.active_session.take()?;
        if session.settled {
            return None;
        }
        session.settled = true;
        Some(make_terminal_receipt(
            session.action_id,
            session.capability_id,
            ReceiptOutcome::Done,
            None,
            Some("speech playback finished".to_owned()),
            Some(TerminationReason::Normal),
            Some(1.0),
            Some(session.total_units),
            session.h_act,
            Time::from_millis(now_ms),
        ))
    }

    fn descriptors(&self) -> Vec<CapabilityDescriptor> {
        let mut caps = vec![CapabilityDescriptor {
            id: CapabilityId::new("speech.speak"),
            description:
                "Speak utterance with AEC, VAD barge-in (<50ms mute), and Option A receipts"
                    .to_owned(),
            places: vec![self.place.clone()],
            flags: CapabilityFlags {
                communicative: true,
                identity_changing: false,
                irreversible: false,
            },
            typical_latency_ms: 40,
            capability_schema_hash: capability_schema_hash(&json!({
                "capability": "speech.speak",
                "type": "object",
                "properties": { "text": { "type": "string" } }
            })),
            sensed_effect: Some(SensedEffect {
                channel: ChannelId::new("voice:mic"),
                window_ms: 50,
                noise: 0.05,
                delta_pred: 1.0,
            }),
        }];
        sort_capabilities(&mut caps);
        caps
    }
}

impl<T: TtsPlaybackSink> BodyHandler for VoiceBody<T> {
    fn discover(&mut self, params: BodyDiscoverParams) -> Result<BodyDiscoverResult, Waist2Error> {
        Ok(BodyDiscoverResult {
            body_id: params.body_id.unwrap_or_else(|| self.body_id.clone()),
            protocol_version: CURRENT_PROTOCOL_VERSION,
            capabilities: self.descriptors(),
            places: vec![self.place.clone()],
        })
    }

    fn execute(&mut self, params: ActionExecuteParams) -> Result<ActionExecuteResult, Waist2Error> {
        if params.audience.is_empty() {
            return Err(Waist2Error::undeclared_audience());
        }
        if params.capability_id.as_str() != "speech.speak" {
            return Err(Waist2Error::unknown_capability(&params.capability_id));
        }
        let place_ok = !params.places.is_empty() && params.places.iter().all(|p| p == &self.place);
        if !place_ok {
            let receipt = make_refused_receipt(
                params.action_id,
                params.capability_id,
                "ERR_UNKNOWN_PLACE",
                "action targets a place not owned by body-voice",
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
                Some("awaiting approval before spoken output".to_owned()),
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

        let text = String::from_utf8_lossy(params.payload.as_bytes()).into_owned();
        self.start_streaming_speech(params.action_id, &text, 1, params.h_act);
        self.set_playback_progress(1.0, 1);
        let receipt = self.finish_streaming_speech(self.clock_now_ms);
        Ok(ActionExecuteResult {
            accepted: true,
            receipt,
        })
    }

    fn cancel(&mut self, params: ActionCancelParams) -> Result<ActionCancelResult, Waist2Error> {
        self.tts_sink.mute(self.clock_now_ms);
        let mut progress = 0.0;
        let mut delivered = 0;
        if let Some(mut s) = self.active_session.take() {
            s.settled = true;
            progress = s.progress;
            delivered = s.delivered_units;
        }
        let receipt = make_terminal_receipt(
            params.action_id,
            CapabilityId::new("speech.speak"),
            ReceiptOutcome::Failed,
            Some("canceled_by_core".to_owned()),
            params.reason,
            Some(TerminationReason::CanceledByCore),
            Some(progress),
            Some(delivered),
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
        Ok(ApprovalResolveResult {
            applied: self.held_actions.remove(&params.action_id),
        })
    }

    fn on_percept(&mut self, params: PerceptEmitParams) -> Result<PerceptEmitResult, Waist2Error> {
        if params.audience.as_ref().is_none_or(Audience::is_empty) {
            return Err(Waist2Error::undeclared_audience());
        }
        if params.place != self.place {
            return Err(Waist2Error::invalid_receipt_transition(format!(
                "percept place '{}' is not owned by body-voice",
                params.place
            )));
        }
        Ok(PerceptEmitResult { accepted: true })
    }
}
