//! Neural Cognitive Controller gate decision assembly:
//! deterministic commitment breaker -> Pure-Rust neural forward step -> calibrated categorical/deterministic
//! `[Silent, Note, Wait, Go]` + `CIC` abstention + neutral fallback + Trace-Pinned [`DecisionTrace`].

use monoagens_protocol::EventId;
use serde::{Deserialize, Serialize};

use crate::config::CoreConfig;
use crate::event::DecisionOutcome;
use crate::neural::{
    DEFAULT_H_DIM, FEATURE_SCHEMA_VERSION, GateClass, NeuralModel, NeuralOutputs, PlaceAdaptState,
    forward_step, neutral_outputs,
};
use crate::rng::SeededRng;

/// Canonical reason recorded on a Slim Core V2 gate decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionReason {
    /// Hard commitment breaker bypassed neural gating (`Go`).
    Commitment,
    /// Stochastic categorical draw selected `Go`.
    StochasticDraw,
    /// Deterministic neural argmax selected `Go`.
    NeuralGo,
    /// Selected `Note` band.
    NoteBand,
    /// Selected `Silent` (below `Go`/`Note`/`Wait`).
    BelowNoteBand,
    /// Inbound door held or refused the event.
    DoorBlocked,
    /// Crash recovery resumed an interrupted input without calling a model.
    Recovered,
    /// Open `wait` expectation yielded when a third party answered first.
    Yielded,
    /// Percept matched an open `Effect` prediction and was marked self-caused without appraisal.
    SelfCaused,
    /// Neutral fallback (`model = None`) yielded deterministic `Silent`.
    NeutralFallback,
    /// Neural `CIC` selective abstention forced `Silent`.
    CicAbstention,
    /// Neural `Wait` selected deferred patience (`Silent` this turn + `OpenWait` armed).
    WaitDeferral,
}

impl DecisionReason {
    #[must_use]
    pub const fn code(self) -> u8 {
        match self {
            Self::Commitment => 1,
            Self::StochasticDraw => 2,
            Self::NeuralGo => 3,
            Self::NoteBand => 4,
            Self::BelowNoteBand => 5,
            Self::DoorBlocked => 6,
            Self::Recovered => 7,
            Self::Yielded => 8,
            Self::SelfCaused => 9,
            Self::NeutralFallback => 10,
            Self::CicAbstention => 11,
            Self::WaitDeferral => 12,
        }
    }

    #[must_use]
    pub const fn from_code(code: u8) -> Option<Self> {
        match code {
            1 => Some(Self::Commitment),
            2 => Some(Self::StochasticDraw),
            3 => Some(Self::NeuralGo),
            4 => Some(Self::NoteBand),
            5 => Some(Self::BelowNoteBand),
            6 => Some(Self::DoorBlocked),
            7 => Some(Self::Recovered),
            8 => Some(Self::Yielded),
            9 => Some(Self::SelfCaused),
            10 => Some(Self::NeutralFallback),
            11 => Some(Self::CicAbstention),
            12 => Some(Self::WaitDeferral),
            _ => None,
        }
    }
}

/// Deterministic commitment-breaker triggers (`CommitmentBreaker`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct CommitmentInputs {
    /// Turn was explicitly allocated or directly addressed to the soul.
    pub turn_allocated: bool,
    /// Event fires or answers an open expectation / prediction.
    pub fires_or_answers_expectation: bool,
    /// Event continues an active chain of the soul's own work.
    pub continues_active_chain: bool,
}

impl CommitmentInputs {
    /// Returns `true` if any deterministic commitment trigger is active.
    #[must_use]
    pub const fn any(&self) -> bool {
        self.turn_allocated || self.fires_or_answers_expectation || self.continues_active_chain
    }
}

/// Self-contained Trace-Pinned decision record persisted on [`crate::event::EventDetail::Decision`].
///
/// Replay verifies recorded outcomes, pinned model `BLAKE3` digest, schema version, adaptation flag,
/// and neural outputs directly without requiring bit-identical floating-point re-execution across CPUs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DecisionTrace {
    pub neural: NeuralOutputs,
    pub model_blake3: Option<[u8; 32]>,
    pub feature_schema_version: u16,
    pub online_adaptation_applied: bool,
    pub commitment_bypass: bool,
    pub self_caused: bool,
    pub draw: f64,
    pub probability: f64,
    pub outcome: DecisionOutcome,
    pub reason: DecisionReason,
}

/// Outcome of evaluating the V2 gate for one percept or backlog chunk.
#[derive(Debug, Clone, PartialEq)]
pub struct GateOutcome {
    pub outcome: DecisionOutcome,
    pub reason: DecisionReason,
    pub gate_class: GateClass,
    pub wait_delta_ms: u64,
    pub probability: f64,
    pub draw: f64,
    pub neural: NeuralOutputs,
    pub trace: DecisionTrace,
}

impl DecisionTrace {
    /// Returns an audit tag when speech was withheld for an inspectable reason.
    #[must_use]
    pub fn silence_audit_reason(&self) -> Option<&'static str> {
        match self.outcome {
            DecisionOutcome::Note => Some("note_band"),
            DecisionOutcome::DoorBlocked => Some("door_blocked"),
            DecisionOutcome::Silent => match self.reason {
                DecisionReason::Yielded => Some("yielded"),
                DecisionReason::CicAbstention => Some("cic_abstention"),
                DecisionReason::NeutralFallback => Some("neutral_fallback"),
                DecisionReason::WaitDeferral => Some("wait_deferral"),
                _ => None,
            },
            DecisionOutcome::Go | DecisionOutcome::Processed => None,
        }
    }
}

/// Evaluates the V2 Neural Cognitive Controller gate for one visible event.
///
/// Even when `commitments.any()` triggers a hard `Go` bypass, if `model.is_some()` the forward pass
/// still runs cleanly so `h_next` (the per-place 64d recurrent memory) advances without missing the turn.
#[must_use]
#[allow(clippy::too_many_arguments)]
pub fn decide_gate(
    event_id: EventId,
    config: &CoreConfig,
    model: Option<&NeuralModel>,
    e_256: &[f32],
    x_num: &[f32],
    h_prev: &[f32],
    place_adapt: Option<&PlaceAdaptState>,
    commitments: CommitmentInputs,
) -> GateOutcome {
    let adapt_cfg = config.effective_adapt_config();
    let adapt_arg = if adapt_cfg.online_adaptation {
        place_adapt.map(|st| (st, &adapt_cfg))
    } else {
        None
    };

    let neural = forward_step(model, e_256, x_num, h_prev, adapt_arg).unwrap_or_else(|_| {
        let zeros = vec![0.0_f32; h_prev.len().max(DEFAULT_H_DIM)];
        neutral_outputs(&zeros)
    });

    let model_blake3 = model.map(|m| m.blake3);
    let schema_ver = model.map_or(FEATURE_SCHEMA_VERSION, |m| {
        m.config.feature_schema_version
    });
    let online_adaptation_applied = model.is_some() && adapt_cfg.online_adaptation && place_adapt.is_some();
    let commitment_bypass = commitments.any();

    let (gate_class, probability, draw, outcome, reason, wait_delta_ms) = if commitment_bypass {
        (
            GateClass::Go,
            1.0_f64,
            0.0_f64,
            DecisionOutcome::Go,
            DecisionReason::Commitment,
            0_u64,
        )
    } else if model.is_none() {
        (
            GateClass::Silent,
            f64::from(neural.gate_probs[3]),
            0.0_f64,
            DecisionOutcome::Silent,
            DecisionReason::NeutralFallback,
            0_u64,
        )
    } else if neural.cic_abstain {
        (
            GateClass::Silent,
            f64::from(neural.gate_probs[3]),
            0.0_f64,
            DecisionOutcome::Silent,
            DecisionReason::CicAbstention,
            0_u64,
        )
    } else if config.stochastic_gate_sampling {
        let d = SeededRng::draw_unit(config.seed, event_id);
        let p_go = f64::from(neural.gate_probs[3]).clamp(0.0, 1.0);
        let p_wait = f64::from(neural.gate_probs[2]).clamp(0.0, 1.0);
        let p_note = f64::from(neural.gate_probs[1]).clamp(0.0, 1.0);
        if d < p_go {
            (
                GateClass::Go,
                p_go,
                d,
                DecisionOutcome::Go,
                DecisionReason::StochasticDraw,
                0_u64,
            )
        } else if d < p_go + p_wait {
            (
                GateClass::Wait,
                p_go,
                d,
                DecisionOutcome::Silent,
                DecisionReason::WaitDeferral,
                neural.wait_delta_ms.round().max(1.0) as u64,
            )
        } else if d < p_go + p_wait + p_note {
            (
                GateClass::Note,
                p_go,
                d,
                DecisionOutcome::Note,
                DecisionReason::NoteBand,
                0_u64,
            )
        } else {
            (
                GateClass::Silent,
                p_go,
                d,
                DecisionOutcome::Silent,
                DecisionReason::BelowNoteBand,
                0_u64,
            )
        }
    } else {
        let p_go = f64::from(neural.gate_probs[3]).clamp(0.0, 1.0);
        match neural.predicted_class {
            GateClass::Go => (
                GateClass::Go,
                p_go,
                0.0_f64,
                DecisionOutcome::Go,
                DecisionReason::NeuralGo,
                0_u64,
            ),
            GateClass::Wait => (
                GateClass::Wait,
                p_go,
                0.0_f64,
                DecisionOutcome::Silent,
                DecisionReason::WaitDeferral,
                neural.wait_delta_ms.round().max(1.0) as u64,
            ),
            GateClass::Note => (
                GateClass::Note,
                p_go,
                0.0_f64,
                DecisionOutcome::Note,
                DecisionReason::NoteBand,
                0_u64,
            ),
            GateClass::Silent => (
                GateClass::Silent,
                p_go,
                0.0_f64,
                DecisionOutcome::Silent,
                DecisionReason::BelowNoteBand,
                0_u64,
            ),
        }
    };

    let trace = DecisionTrace {
        neural: neural.clone(),
        model_blake3,
        feature_schema_version: schema_ver,
        online_adaptation_applied,
        commitment_bypass,
        self_caused: false,
        draw,
        probability,
        outcome,
        reason,
    };

    GateOutcome {
        outcome,
        reason,
        gate_class,
        wait_delta_ms,
        probability,
        draw,
        neural,
        trace,
    }
}
