//! Gate decision assembly: commitment breaker, `tau -> 0` deterministic step limit,
//! seeded Bernoulli draw `(seed, event_id)`, and self-contained recomputable trace.

use monoagens_protocol::EventId;
use serde::{Deserialize, Serialize};

use crate::activation::sigmoid;
use crate::event::DecisionOutcome;
use crate::gate::salience::{SalienceEvaluation, SalienceWeights};
use crate::gate::signals::GateSignals;
use crate::gate::threshold::ThresholdTerms;
use crate::prior::Prior;
use crate::registry::Switches;
use crate::rng::SeededRng;

/// Canonical reason recorded on a gate decision (`specs/appraisal-gate/spec.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionReason {
    /// Bypassed the draw because the turn is allocated, answers/fires an expectation, or continues an active chain.
    Commitment,
    /// Stochastic draw `draw < P` yielded `Go`.
    StochasticDraw,
    /// Deterministic `tau -> 0` step limit (`S_eff >= theta`) yielded `Go`.
    ThresholdStep,
    /// No-go with `P >= note_band` yielded `Note`.
    NoteBand,
    /// No-go with `P < note_band` (or `tau -> 0` with `S_eff < theta`) yielded `Silent`.
    BelowNoteBand,
    /// Inbound door held or refused the event.
    DoorBlocked,
    /// Crash recovery resumed an interrupted input without calling a model.
    Recovered,
    /// Unallocated `Go` waited on patience and yielded when a third party answered first.
    Yielded,
    /// Percept matched an open `Effect` prediction and was marked self-caused without appraisal.
    SelfCaused,
}

impl DecisionReason {
    #[must_use]
    pub const fn code(self) -> u8 {
        match self {
            Self::Commitment => 1,
            Self::StochasticDraw => 2,
            Self::ThresholdStep => 3,
            Self::NoteBand => 4,
            Self::BelowNoteBand => 5,
            Self::DoorBlocked => 6,
            Self::Recovered => 7,
            Self::Yielded => 8,
            Self::SelfCaused => 9,
        }
    }

    #[must_use]
    pub const fn from_code(code: u8) -> Option<Self> {
        match code {
            1 => Some(Self::Commitment),
            2 => Some(Self::StochasticDraw),
            3 => Some(Self::ThresholdStep),
            4 => Some(Self::NoteBand),
            5 => Some(Self::BelowNoteBand),
            6 => Some(Self::DoorBlocked),
            7 => Some(Self::Recovered),
            8 => Some(Self::Yielded),
            9 => Some(Self::SelfCaused),
            _ => None,
        }
    }
}

/// Commitment-breaker triggers (`specs/appraisal-gate/spec.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct CommitmentInputs {
    /// Turn was allocated to the soul (`deg(e, soul) >= theta_alloc` or direct allocation).
    pub turn_allocated: bool,
    /// Event fires or answers an open expectation / prediction.
    pub fires_or_answers_expectation: bool,
    /// Event continues an active chain of the soul's own work.
    pub continues_active_chain: bool,
}

impl CommitmentInputs {
    /// Returns `true` if any commitment trigger is active.
    #[must_use]
    pub const fn any(&self) -> bool {
        self.turn_allocated || self.fires_or_answers_expectation || self.continues_active_chain
    }
}

/// Self-contained decision trace stored on [`crate::event::EventDetail::Decision`] so every
/// decision's salience, threshold, probability, and outcome can be recomputed from the log event alone.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DecisionTrace {
    pub signals: GateSignals,
    pub weights: SalienceWeights,
    pub raw_salience: f64,
    pub h_pre: f64,
    pub kappa: f64,
    pub habituation_factor: f64,
    pub effective_salience: f64,
    pub threshold_terms: ThresholdTerms,
    pub tau: f64,
    pub note_band: f64,
    pub stochastic_enabled: bool,
    pub commitment_bypass: bool,
    pub probability: f64,
    pub draw: f64,
    pub outcome: DecisionOutcome,
    pub reason: DecisionReason,
}

/// Values recomputed solely from a stored [`DecisionTrace`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RecomputedTrace {
    pub raw_salience: f64,
    pub habituation_factor: f64,
    pub effective_salience: f64,
    pub threshold: f64,
    pub probability: f64,
    pub outcome: DecisionOutcome,
    pub reason: DecisionReason,
}

impl DecisionTrace {
    /// Recomputes `S`, `habituation_factor`, `S_eff`, `theta`, `P`, `outcome`, and `reason`
    /// strictly from the numbers persisted in `self`.
    #[must_use]
    pub fn recompute(&self) -> RecomputedTrace {
        let raw_salience = self.weights.evaluate(&self.signals);
        let h_pre = if self.h_pre.is_finite() {
            self.h_pre.max(0.0)
        } else {
            0.0
        };
        let kappa = if self.kappa.is_finite() {
            self.kappa.max(0.0)
        } else {
            0.0
        };
        let habituation_factor = 1.0 / (1.0 + kappa * h_pre);
        let effective_salience = (raw_salience * habituation_factor).clamp(0.0, 1.0);
        let threshold = self.threshold_terms.recompute_theta();

        if self.reason == DecisionReason::Yielded {
            return RecomputedTrace {
                raw_salience,
                habituation_factor,
                effective_salience,
                threshold,
                probability: self.probability,
                outcome: DecisionOutcome::Silent,
                reason: DecisionReason::Yielded,
            };
        }

        if self.commitment_bypass {
            return RecomputedTrace {
                raw_salience,
                habituation_factor,
                effective_salience,
                threshold,
                probability: 1.0,
                outcome: DecisionOutcome::Go,
                reason: DecisionReason::Commitment,
            };
        }

        if !self.stochastic_enabled || self.tau <= 1e-12 {
            let passed = effective_salience >= threshold;
            let probability = if passed { 1.0 } else { 0.0 };
            let (outcome, reason) = if passed {
                (DecisionOutcome::Go, DecisionReason::ThresholdStep)
            } else {
                (DecisionOutcome::Silent, DecisionReason::BelowNoteBand)
            };
            return RecomputedTrace {
                raw_salience,
                habituation_factor,
                effective_salience,
                threshold,
                probability,
                outcome,
                reason,
            };
        }

        let probability = sigmoid((effective_salience - threshold) / self.tau).clamp(0.0, 1.0);
        let (outcome, reason) = if self.draw < probability {
            (DecisionOutcome::Go, DecisionReason::StochasticDraw)
        } else if probability >= self.note_band {
            (DecisionOutcome::Note, DecisionReason::NoteBand)
        } else {
            (DecisionOutcome::Silent, DecisionReason::BelowNoteBand)
        };

        RecomputedTrace {
            raw_salience,
            habituation_factor,
            effective_salience,
            threshold,
            probability,
            outcome,
            reason,
        }
    }

    /// Returns an audit reason tag if this decision withheld speech for an auditable cause
    /// (e.g., `note_band`, `yielded`, `habituated`, `floor_cost`, or `door_blocked`).
    #[must_use]
    pub fn silence_audit_reason(&self) -> Option<&'static str> {
        match self.outcome {
            DecisionOutcome::Note => Some("note_band"),
            DecisionOutcome::DoorBlocked => Some("door_blocked"),
            DecisionOutcome::Silent => {
                if self.reason == DecisionReason::Yielded {
                    Some("yielded")
                } else if self.h_pre > 0.0
                    && self.kappa > 0.0
                    && self.raw_salience > self.effective_salience
                    && self.raw_salience >= self.threshold_terms.theta
                {
                    Some("habituated")
                } else if self.signals.social_cost > 0.0 && self.weights.w_sc < 0.0 {
                    Some("floor_cost")
                } else {
                    None
                }
            }
            DecisionOutcome::Go | DecisionOutcome::Processed => None,
        }
    }
}

/// Result of evaluating the gate for one event or chunk.
#[derive(Debug, Clone, PartialEq)]
pub struct GateOutcome {
    pub outcome: DecisionOutcome,
    pub reason: DecisionReason,
    pub effective_salience: f64,
    pub threshold: f64,
    pub probability: f64,
    pub draw: f64,
    pub trace: DecisionTrace,
}

/// Evaluates the full gate decision from `signals`, `salience`, `threshold_terms`, `h_pre`,
/// `commitments`, `(seed, event_id)`, `prior`, and `switches`.
#[must_use]
#[allow(clippy::too_many_arguments)]
pub fn decide_gate(
    event_id: EventId,
    seed: u64,
    signals: GateSignals,
    salience: SalienceEvaluation,
    threshold_terms: ThresholdTerms,
    h_pre: f64,
    commitments: CommitmentInputs,
    prior: &Prior,
    switches: &Switches,
) -> GateOutcome {
    let kappa = if switches.habituation {
        prior.kappa
    } else {
        0.0
    };
    let h_pre_clean = if h_pre.is_finite() {
        h_pre.max(0.0)
    } else {
        0.0
    };
    let habituation_factor = 1.0 / (1.0 + kappa * h_pre_clean);
    let effective_salience = (salience.raw_salience * habituation_factor).clamp(0.0, 1.0);
    let tau = prior.tau();
    let stochastic_enabled = switches.stochastic_go_no_go && tau > 1e-12;
    let commitment_bypass = switches.commitment_breaker && commitments.any();

    let (probability, draw, outcome, reason) = if commitment_bypass {
        (1.0, 0.0, DecisionOutcome::Go, DecisionReason::Commitment)
    } else if !stochastic_enabled {
        let passed = effective_salience >= threshold_terms.theta;
        let p = if passed { 1.0 } else { 0.0 };
        let (out, rsn) = if passed {
            (DecisionOutcome::Go, DecisionReason::ThresholdStep)
        } else {
            (DecisionOutcome::Silent, DecisionReason::BelowNoteBand)
        };
        (p, 0.0, out, rsn)
    } else {
        let p = sigmoid((effective_salience - threshold_terms.theta) / tau).clamp(0.0, 1.0);
        let d = SeededRng::draw_unit(seed, event_id);
        let (out, rsn) = if d < p {
            (DecisionOutcome::Go, DecisionReason::StochasticDraw)
        } else if p >= prior.note_band {
            (DecisionOutcome::Note, DecisionReason::NoteBand)
        } else {
            (DecisionOutcome::Silent, DecisionReason::BelowNoteBand)
        };
        (p, d, out, rsn)
    };

    let trace = DecisionTrace {
        signals,
        weights: salience.weights,
        raw_salience: salience.raw_salience,
        h_pre: h_pre_clean,
        kappa,
        habituation_factor,
        effective_salience,
        threshold_terms,
        tau,
        note_band: prior.note_band,
        stochastic_enabled,
        commitment_bypass,
        probability,
        draw,
        outcome,
        reason,
    };

    GateOutcome {
        outcome,
        reason,
        effective_salience,
        threshold: threshold_terms.theta,
        probability,
        draw,
        trace,
    }
}

/// Convenience helper for recomputing a [`DecisionTrace`] directly.
#[must_use]
pub fn recompute_decision_from_trace(trace: &DecisionTrace) -> RecomputedTrace {
    trace.recompute()
}
