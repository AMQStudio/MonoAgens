//! Bounded reinforcement learning with clipped IPS on judged silences (`A-13`, `R-19`, `R-26`),
//! non-penalty on external/core cancellation, shrunk `q_0.10` wake-gate calibration (`16.3`),
//! and exact epoch rollback (`16.4`, `R-34`).

use std::collections::BTreeMap;

use monoagens_protocol::{EventId, TerminationReason, Time};
use serde::{Deserialize, Serialize};

use crate::gate::{GateSignals, SalienceWeights};
use crate::prior::Prior;
use crate::registry::Switches;
use crate::state::AdaptiveWeightsAnchor;

const MS_PER_DAY: u64 = 86_400_000;

/// Four outcome polarities driving bounded wake-policy learning (`16.1`, `16.2`, `R-19`, `R-26`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LearningOutcomeKind {
    /// Agent acted/spoke and received positive/grounded confirmation -> wake UP (`w` up, `theta_0` down).
    GoodAct,
    /// Agent acted/spoke and outcome was negative/ungrounded/rejected -> wake DOWN (`w` down, `theta_0` up).
    BadAct,
    /// Agent chose silence and silence missed an obligation/need -> wake UP (`w` up, `theta_0` down).
    MissedSilence,
    /// Agent chose silence and silence was validated (e.g. another answered) -> wake DOWN (`w` down, `theta_0` up).
    AnsweredSilence,
}

impl LearningOutcomeKind {
    pub(crate) const fn code(self) -> u8 {
        match self {
            Self::GoodAct => 1,
            Self::BadAct => 2,
            Self::MissedSilence => 3,
            Self::AnsweredSilence => 4,
        }
    }

    /// Target wake label `y in {0, 1}` ("should wake/speak") used by sign-consistent `(y - P)` updates.
    #[must_use]
    pub const fn wake_target(self) -> f64 {
        match self {
            Self::GoodAct | Self::MissedSilence => 1.0,
            Self::BadAct | Self::AnsweredSilence => 0.0,
        }
    }

    /// Returns `true` when this outcome comes from a sampled chosen silence (eligible for IPS weighting).
    #[must_use]
    pub const fn is_silence_sample(self) -> bool {
        matches!(self, Self::MissedSilence | Self::AnsweredSilence)
    }
}

/// Observation tuple passed into [`BoundedLearner::apply_outcome`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LearningSample {
    pub source_event_id: EventId,
    pub outcome_kind: LearningOutcomeKind,
    /// Predicted wake probability `P_go in [0, 1]` at the originating gate decision.
    pub prob_go: f64,
    /// Gate feature vector `[0, 1]` active at the originating gate decision.
    pub signals: GateSignals,
    /// Sampling propensity `rho in (0, 1]` for judged silences (`None` / `1.0` for acted outcomes).
    pub sample_propensity: Option<f64>,
    /// Teacher / feedback source reliability weight `[0, 1]` (`1.0` for verified environmental truth).
    pub teacher_reliability: f64,
    pub termination_reason: Option<TerminationReason>,
    pub timestamp_ms: Time,
}

/// Snapshot of adaptive weights saved prior to each applied update (`16.4`, `R-34`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LearningEpochCheckpoint {
    pub epoch_id: u64,
    pub source_event_id: EventId,
    pub timestamp_ms: Time,
    pub weights_before: AdaptiveWeightsAnchor,
    pub daily_abs_delta_before: [f64; 7],
    pub day_index_before: u64,
}

/// Detailed audit record returned after evaluating a learning step.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LearningStepResult {
    pub applied: bool,
    pub skip_reason: Option<String>,
    pub epoch_id: Option<u64>,
    pub outcome_kind: LearningOutcomeKind,
    pub signed_error: f64,
    pub ips_weight: f64,
    pub delta_weights: SalienceWeights,
    pub delta_theta_0: f64,
    pub weights_after: AdaptiveWeightsAnchor,
}

/// Clipped IPS weight `min(M_ips, 1 / rho)` (`R-19`).
#[must_use]
pub fn clip_ips_weight(propensity: Option<f64>, ips_clip_m: f64) -> f64 {
    let m = if ips_clip_m.is_finite() {
        ips_clip_m.max(1.0)
    } else {
        10.0
    };
    let Some(rho) = propensity else {
        return 1.0;
    };
    if !rho.is_finite() || rho <= 0.0 {
        return m;
    }
    (1.0 / rho.min(1.0)).min(m)
}

/// Computes shrunk empirical lower quantile `q` for `wake_gate_lo` (`16.3`, `A-08`).
#[must_use]
pub fn calibrate_wake_gate_lo(positive_similarities: &[f64], prior: &Prior) -> f64 {
    let mut vals: Vec<f64> = positive_similarities
        .iter()
        .copied()
        .filter(|v| v.is_finite())
        .collect();
    if vals.is_empty() {
        return prior.wake_gate_lo.clamp(0.0, 1.0);
    }
    vals.sort_by(f64::total_cmp);
    let q = prior.learn_wake_gate_quantile.clamp(0.01, 0.50);
    let idx = ((vals.len() as f64 - 1.0) * q).floor().max(0.0) as usize;
    let q_est = (vals[idx] - prior.wake_gate_margin).clamp(0.0, 1.0);
    let n = vals.len() as f64;
    let n0 = prior.learn_wake_gate_n0.max(0.5);
    ((n0 * prior.wake_gate_lo + n * q_est) / (n0 + n)).clamp(0.0, 1.0)
}

/// Stateful bounded learner maintaining daily caps and exact rollback checkpoints (`16.1`..`16.4`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BoundedLearner {
    pub day_index: u64,
    /// Cumulative magnitude moved on `day_index` for `[own_name, info, fok, rel, commitment, sc, theta_0]`.
    pub daily_abs_delta: [f64; 7],
    pub next_epoch_id: u64,
    pub checkpoints: Vec<LearningEpochCheckpoint>,
    pub update_count: u64,
    pub rollback_count: u64,
    pub processed_event_ids: Vec<EventId>,
}

impl Default for BoundedLearner {
    fn default() -> Self {
        Self::new()
    }
}

impl BoundedLearner {
    #[must_use]
    pub fn new() -> Self {
        Self {
            day_index: 0,
            daily_abs_delta: [0.0; 7],
            next_epoch_id: 1,
            checkpoints: Vec::new(),
            update_count: 0,
            rollback_count: 0,
            processed_event_ids: Vec::new(),
        }
    }

    fn sync_day(&mut self, timestamp_ms: Time) {
        let day = timestamp_ms.as_millis() / MS_PER_DAY;
        if day != self.day_index {
            self.day_index = day;
            self.daily_abs_delta = [0.0; 7];
        }
    }

    fn apply_daily_capped(
        &mut self,
        idx: usize,
        raw_delta: f64,
        daily_cap: f64,
        current_val: f64,
        min_val: f64,
        max_val: f64,
    ) -> (f64, f64) {
        let remaining = (daily_cap.max(0.0) - self.daily_abs_delta[idx]).max(0.0);
        if remaining == 0.0 || !raw_delta.is_finite() || raw_delta == 0.0 {
            return (current_val, 0.0);
        }
        let clipped_step = raw_delta.clamp(-remaining, remaining);
        let next_val = (current_val + clipped_step).clamp(min_val, max_val);
        let actual_delta = next_val - current_val;
        self.daily_abs_delta[idx] += actual_delta.abs();
        (next_val, actual_delta)
    }

    /// Applies one bounded reinforcement update to `weights` (`16.1`, `16.2`).
    ///
    /// Guarantees:
    /// - Zero movement when `!switches.bounded_learning` (`learn_rate = 0`).
    /// - Zero movement when `termination_reason` is `CanceledByCore` or `PreemptedExternal`.
    /// - Clipped IPS weighting on judged silences (`min(learn_ips_clip_m, 1 / rho)`).
    /// - Strict per-day cap `prior.learn_daily_max_delta` per weight & threshold.
    /// - Saves an exact pre-update [`LearningEpochCheckpoint`] for instant rollback (`16.4`).
    pub fn apply_outcome(
        &mut self,
        weights: &mut AdaptiveWeightsAnchor,
        sample: &LearningSample,
        prior: &Prior,
        switches: &Switches,
    ) -> LearningStepResult {
        if !switches.bounded_learning || prior.learn_rate_w <= 0.0 {
            return LearningStepResult {
                applied: false,
                skip_reason: Some("bounded_learning_neutral".to_owned()),
                epoch_id: None,
                outcome_kind: sample.outcome_kind,
                signed_error: 0.0,
                ips_weight: 1.0,
                delta_weights: SalienceWeights::default(),
                delta_theta_0: 0.0,
                weights_after: *weights,
            };
        }

        if matches!(
            sample.termination_reason,
            Some(TerminationReason::CanceledByCore | TerminationReason::PreemptedExternal)
        ) {
            return LearningStepResult {
                applied: false,
                skip_reason: Some("canceled_or_preempted_no_penalty".to_owned()),
                epoch_id: None,
                outcome_kind: sample.outcome_kind,
                signed_error: 0.0,
                ips_weight: 1.0,
                delta_weights: SalienceWeights::default(),
                delta_theta_0: 0.0,
                weights_after: *weights,
            };
        }

        if sample.source_event_id.as_u64() != 0
            && self.processed_event_ids.contains(&sample.source_event_id)
        {
            return LearningStepResult {
                applied: false,
                skip_reason: Some("duplicate_outcome_event".to_owned()),
                epoch_id: None,
                outcome_kind: sample.outcome_kind,
                signed_error: 0.0,
                ips_weight: 1.0,
                delta_weights: SalienceWeights::default(),
                delta_theta_0: 0.0,
                weights_after: *weights,
            };
        }

        self.sync_day(sample.timestamp_ms);

        let checkpoint = LearningEpochCheckpoint {
            epoch_id: self.next_epoch_id,
            source_event_id: sample.source_event_id,
            timestamp_ms: sample.timestamp_ms,
            weights_before: *weights,
            daily_abs_delta_before: self.daily_abs_delta,
            day_index_before: self.day_index,
        };

        let p = if sample.prob_go.is_finite() {
            sample.prob_go.clamp(1e-6, 1.0 - 1e-6)
        } else {
            0.5
        };
        let target = sample.outcome_kind.wake_target();
        // Sign-consistent score error `(y_target - P)`:
        // - GoodAct (1 - P) > 0 => UP
        // - MissedSilence (1 - P) > 0 => UP
        // - BadAct (-P) < 0 => DOWN
        // - AnsweredSilence (-P) < 0 => DOWN
        let signed_error = target - p;
        let ips_weight = if sample.outcome_kind.is_silence_sample() {
            clip_ips_weight(sample.sample_propensity, prior.learn_ips_clip_m)
        } else {
            1.0
        };
        let reliability = if sample.teacher_reliability.is_finite() {
            sample.teacher_reliability.clamp(0.0, 1.0)
        } else {
            1.0
        };
        let step_scale = prior.learn_rate_w * reliability * signed_error * ips_weight;
        let cap = prior.learn_daily_max_delta;

        let s = &sample.signals;
        let (next_own, d_own) = self.apply_daily_capped(
            0,
            step_scale * s.own_name.clamp(0.0, 1.0),
            cap,
            weights.salience_weights.w_own_name,
            0.0,
            2.0,
        );
        let (next_info, d_info) = self.apply_daily_capped(
            1,
            step_scale * s.info_gap.clamp(0.0, 1.0),
            cap,
            weights.salience_weights.w_info,
            0.0,
            2.0,
        );
        let (next_fok, d_fok) = self.apply_daily_capped(
            2,
            step_scale * s.feeling_of_knowing.clamp(0.0, 1.0),
            cap,
            weights.salience_weights.w_fok,
            0.0,
            2.0,
        );
        let (next_rel, d_rel) = self.apply_daily_capped(
            3,
            step_scale * s.relevance.clamp(0.0, 1.0),
            cap,
            weights.salience_weights.w_rel,
            0.0,
            2.0,
        );
        let (next_commit, d_commit) = self.apply_daily_capped(
            4,
            step_scale * s.commitment.clamp(0.0, 1.0),
            cap,
            weights.salience_weights.w_commitment,
            0.0,
            2.0,
        );
        // `w_sc` contributes as `w_sc * social_cost` (`w_sc <= 0`), so increasing salience moves `w_sc`
        // toward `0.0` (+step_scale * s_sc), while decreasing salience makes `w_sc` more negative.
        let (next_sc, d_sc) = self.apply_daily_capped(
            5,
            step_scale * s.social_cost.clamp(0.0, 1.0),
            cap,
            weights.salience_weights.w_sc,
            -2.0,
            0.0,
        );
        // Threshold moves in exact opposite polarity to salience (`theta_0` down => wake UP).
        let (next_theta_0, d_theta_0) = self.apply_daily_capped(
            6,
            -step_scale * prior.learn_rate_theta_ratio,
            cap,
            weights.theta_0,
            0.0,
            1.0,
        );

        let moved = d_own != 0.0
            || d_info != 0.0
            || d_fok != 0.0
            || d_rel != 0.0
            || d_commit != 0.0
            || d_sc != 0.0
            || d_theta_0 != 0.0;

        if !moved {
            return LearningStepResult {
                applied: false,
                skip_reason: Some("zero_gradient_or_daily_cap_reached".to_owned()),
                epoch_id: None,
                outcome_kind: sample.outcome_kind,
                signed_error,
                ips_weight,
                delta_weights: SalienceWeights::default(),
                delta_theta_0: 0.0,
                weights_after: *weights,
            };
        }

        weights.salience_weights.w_own_name = next_own;
        weights.salience_weights.w_info = next_info;
        weights.salience_weights.w_fok = next_fok;
        weights.salience_weights.w_rel = next_rel;
        weights.salience_weights.w_commitment = next_commit;
        weights.salience_weights.w_sc = next_sc;
        weights.theta_0 = next_theta_0;

        let epoch_id = self.next_epoch_id;
        self.next_epoch_id = self.next_epoch_id.saturating_add(1);
        self.update_count = self.update_count.saturating_add(1);
        self.checkpoints.push(checkpoint);
        if sample.source_event_id.as_u64() != 0 {
            self.processed_event_ids.push(sample.source_event_id);
        }

        LearningStepResult {
            applied: true,
            skip_reason: None,
            epoch_id: Some(epoch_id),
            outcome_kind: sample.outcome_kind,
            signed_error,
            ips_weight,
            delta_weights: SalienceWeights {
                w_own_name: d_own,
                w_info: d_info,
                w_fok: d_fok,
                w_rel: d_rel,
                w_commitment: d_commit,
                w_sc: d_sc,
            },
            delta_theta_0: d_theta_0,
            weights_after: *weights,
        }
    }

    /// Restores exact weights from the most recent checkpoint (`16.4`, `R-34`).
    pub fn rollback_to_previous(
        &mut self,
        weights: &mut AdaptiveWeightsAnchor,
    ) -> Option<LearningEpochCheckpoint> {
        let cp = self.checkpoints.pop()?;
        *weights = cp.weights_before;
        self.day_index = cp.day_index_before;
        self.daily_abs_delta = cp.daily_abs_delta_before;
        self.rollback_count = self.rollback_count.saturating_add(1);
        Some(cp)
    }

    /// Restores exact weights from the specified `epoch_id` checkpoint (`16.4`, `R-34`).
    pub fn rollback_to_epoch(
        &mut self,
        weights: &mut AdaptiveWeightsAnchor,
        epoch_id: u64,
    ) -> Option<LearningEpochCheckpoint> {
        let idx = self
            .checkpoints
            .iter()
            .position(|cp| cp.epoch_id == epoch_id)?;
        let cp = self.checkpoints[idx].clone();
        self.checkpoints.truncate(idx);
        *weights = cp.weights_before;
        self.day_index = cp.day_index_before;
        self.daily_abs_delta = cp.daily_abs_delta_before;
        self.rollback_count = self.rollback_count.saturating_add(1);
        Some(cp)
    }

    /// Reconciles pending updates map indexed by event id for checkpoint persistence.
    #[must_use]
    pub fn latest_epoch_id(&self) -> Option<u64> {
        self.checkpoints.last().map(|cp| cp.epoch_id)
    }

    /// Exposes cumulative magnitude used per parameter slot `[0..7]` on `day_index`.
    #[must_use]
    pub fn daily_usage_map(&self) -> BTreeMap<&'static str, f64> {
        let mut m = BTreeMap::new();
        m.insert("w_own_name", self.daily_abs_delta[0]);
        m.insert("w_info", self.daily_abs_delta[1]);
        m.insert("w_fok", self.daily_abs_delta[2]);
        m.insert("w_rel", self.daily_abs_delta[3]);
        m.insert("w_commitment", self.daily_abs_delta[4]);
        m.insert("w_sc", self.daily_abs_delta[5]);
        m.insert("theta_0", self.daily_abs_delta[6]);
        m
    }
}
