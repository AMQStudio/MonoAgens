//! Bounded per-place Warm-Start UCB contextual-bandit online adaptation equalizer.
//!
//! Keeps the loaded neural backbone and recurrent cell completely frozen/immutable
//! at runtime, adapting only a tiny per-place [`PlaceAdaptState`] (`4` gate logit
//! offsets + `4` FiLM scale/shift scalars + per-arm UCB counts/reward stats) strictly
//! clamped within `[-max_adapt_delta, +max_adapt_delta]`.
//!
//! Setting `online_adaptation = false` completely isolates base-model behavior:
//! adaptation state is neither applied during inference nor mutated on feedback.

use monoagens_protocol::{Taint, Time};
use serde::{Deserialize, Serialize};

use crate::neural::{GateClass, NeuralError};

/// Default maximum magnitude for any per-place adaptation offset or FiLM scalar.
pub const DEFAULT_MAX_ADAPT_DELTA: f32 = 0.5;

/// Default daily exponential decay factor for per-place adaptation (`0.90`; `1.0` disables decay).
pub const DEFAULT_ADAPT_DAILY_LAMBDA: f32 = 0.90;

const MS_PER_DAY_F64: f64 = 86_400_000.0;

const fn default_adapt_daily_lambda() -> f32 {
    DEFAULT_ADAPT_DAILY_LAMBDA
}

/// Arm alias for the 4 gate actions `[Silent, Note, Wait, Go]`.
pub type GateArm = GateClass;

/// Configuration governing bounded per-place online adaptation.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct AdaptConfig {
    /// Debug / ablation toggle. When `false`, adaptation state is neither read nor mutated.
    pub online_adaptation: bool,
    /// Hard symmetric clamp bound `[-max_adapt_delta, +max_adapt_delta]` (default `0.5`).
    pub max_adapt_delta: f32,
    /// UCB exploration coefficient `c >= 0.0` (default `0.25`).
    pub ucb_c: f32,
    /// Step size for bounded bandit updates (default `0.1`).
    pub learning_rate: f32,
    /// Warm-start pseudo-count `n_0 >= 1.0` anchoring UCB before local pulls accumulate.
    pub warm_start_prior_weight: f32,
    /// Discount weight in `(0, 1]` applied to delayed one-sided feedback (e.g. ignored message).
    pub delayed_negative_decay: f32,
    /// Daily exponential relaxation factor in `(0, 1]` applied over event timestamps (`C-10`).
    #[serde(default = "default_adapt_daily_lambda")]
    pub adapt_daily_lambda: f32,
}

impl Default for AdaptConfig {
    fn default() -> Self {
        Self {
            online_adaptation: true,
            max_adapt_delta: DEFAULT_MAX_ADAPT_DELTA,
            ucb_c: 0.25,
            learning_rate: 0.1,
            warm_start_prior_weight: 4.0,
            delayed_negative_decay: 0.5,
            adapt_daily_lambda: DEFAULT_ADAPT_DAILY_LAMBDA,
        }
    }
}

impl AdaptConfig {
    /// Validates that all adaptation hyperparameters are finite and within safe bounds.
    pub fn validate(&self) -> Result<(), NeuralError> {
        if !self.max_adapt_delta.is_finite() || self.max_adapt_delta <= 0.0 {
            return Err(NeuralError::InvalidAdaptConfig(
                "max_adapt_delta must be finite and > 0",
            ));
        }
        if !self.ucb_c.is_finite() || self.ucb_c < 0.0 {
            return Err(NeuralError::InvalidAdaptConfig(
                "ucb_c must be finite and >= 0",
            ));
        }
        if !self.learning_rate.is_finite() || self.learning_rate <= 0.0 {
            return Err(NeuralError::InvalidAdaptConfig(
                "learning_rate must be finite and > 0",
            ));
        }
        if !self.warm_start_prior_weight.is_finite() || self.warm_start_prior_weight < 1.0 {
            return Err(NeuralError::InvalidAdaptConfig(
                "warm_start_prior_weight must be finite and >= 1.0",
            ));
        }
        if !self.delayed_negative_decay.is_finite()
            || self.delayed_negative_decay <= 0.0
            || self.delayed_negative_decay > 1.0
        {
            return Err(NeuralError::InvalidAdaptConfig(
                "delayed_negative_decay must be finite and in (0, 1]",
            ));
        }
        if !self.adapt_daily_lambda.is_finite()
            || self.adapt_daily_lambda <= 0.0
            || self.adapt_daily_lambda > 1.0
        {
            return Err(NeuralError::InvalidAdaptConfig(
                "adapt_daily_lambda must be finite and in (0, 1]",
            ));
        }
        Ok(())
    }
}

/// Outcome feedback delivered to a place's Warm-Start UCB bandit equalizer.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct AdaptFeedback {
    /// Selected gate arm receiving credit/blame.
    pub arm: GateArm,
    /// Scalar reward in `[-1.0, 1.0]` (clamped on update).
    pub reward: f32,
    /// True for delayed one-sided signals (e.g. no reply in group chat).
    pub is_delayed_one_sided: bool,
    /// Compact two-half context activity hint in `[-1.0, 1.0]` for FiLM adaptation.
    pub context_hint: [f32; 2],
}

/// Per-place bounded online adaptation state ("Equalizer").
///
/// Holds `4` gate logit offsets + `4` FiLM scale/shift scalars (`2` scales + `2` shifts),
/// all clamped to `[-max_adapt_delta, +max_adapt_delta]`, plus per-arm Warm-Start UCB stats.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlaceAdaptState {
    /// Learned additive logit offsets for `[Silent, Note, Wait, Go]`.
    pub logit_offsets: [f32; 4],
    /// Learned FiLM scale deltas (`gamma = 1.0 + film_scale_deltas[half]`).
    pub film_scale_deltas: [f32; 2],
    /// Learned FiLM shift deltas (`beta = film_shift_deltas[half]`).
    pub film_shift_deltas: [f32; 2],
    /// Number of observed outcomes per arm.
    pub arm_pulls: [u64; 4],
    /// Cumulative discounted reward per arm.
    pub arm_reward_sums: [f64; 4],
    /// Total observed outcomes across all arms for this place.
    pub total_pulls: u64,
    /// Deterministic event timestamp (ms) of the last adaptation update or decay (`C-10`).
    #[serde(default)]
    pub last_updated_ms: Option<Time>,
}

impl Default for PlaceAdaptState {
    fn default() -> Self {
        Self::new()
    }
}

impl PlaceAdaptState {
    /// Creates a zero-initialized (identity) per-place adaptation state.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            logit_offsets: [0.0; 4],
            film_scale_deltas: [0.0; 2],
            film_shift_deltas: [0.0; 2],
            arm_pulls: [0; 4],
            arm_reward_sums: [0.0; 4],
            total_pulls: 0,
            last_updated_ms: None,
        }
    }

    /// Relaxes offsets, FiLM deltas, reward sums, and pull counts using deterministic
    /// exponential decay over `now_ms - last_updated_ms` (`C-10`).
    pub fn apply_time_decay(&mut self, now_ms: Time, config: &AdaptConfig) {
        if !config.online_adaptation {
            return;
        }
        let lambda = if config.adapt_daily_lambda.is_finite()
            && config.adapt_daily_lambda > 0.0
            && config.adapt_daily_lambda <= 1.0
        {
            config.adapt_daily_lambda
        } else {
            DEFAULT_ADAPT_DAILY_LAMBDA
        };
        if let Some(prev) = self.last_updated_ms
            && now_ms > prev
            && lambda < 1.0
        {
            let dt_days = (now_ms.saturating_sub_ms(prev) as f64) / MS_PER_DAY_F64;
            let gamma = f64::from(lambda).powf(dt_days);
            let gamma_f32 = gamma as f32;
            for v in &mut self.logit_offsets {
                *v *= gamma_f32;
            }
            for v in &mut self.film_scale_deltas {
                *v *= gamma_f32;
            }
            for v in &mut self.film_shift_deltas {
                *v *= gamma_f32;
            }
            for i in 0..4 {
                self.arm_reward_sums[i] *= gamma;
                self.arm_pulls[i] = ((self.arm_pulls[i] as f64) * gamma).floor() as u64;
            }
            self.total_pulls = self.arm_pulls.iter().sum();
        }
        self.last_updated_ms = Some(self.last_updated_ms.map_or(now_ms, |p| p.max(now_ms)));
    }

    /// Returns a cloned copy relaxed to `now_ms` without mutating `self`.
    #[must_use]
    pub fn decayed_at(&self, now_ms: Time, config: &AdaptConfig) -> Self {
        let mut copy = self.clone();
        copy.apply_time_decay(now_ms, config);
        copy
    }

    /// Returns the effective clamped gate logit offsets `[Silent, Note, Wait, Go]`
    /// combining the learned per-arm offset with the Warm-Start UCB exploration term.
    ///
    /// Returns `[0.0; 4]` when `config.online_adaptation` is `false`.
    #[must_use]
    pub fn effective_logit_offsets(&self, config: &AdaptConfig) -> [f32; 4] {
        if !config.online_adaptation {
            return [0.0; 4];
        }
        let max_d = if config.max_adapt_delta.is_finite() && config.max_adapt_delta > 0.0 {
            config.max_adapt_delta
        } else {
            DEFAULT_MAX_ADAPT_DELTA
        };
        let n0 = config.warm_start_prior_weight.max(1.0);
        let log_total = (self.total_pulls as f32 + n0).ln().max(0.0);

        let mut out = [0.0_f32; 4];
        for i in 0..4 {
            let n_eff = self.arm_pulls[i] as f32 + n0;
            let ucb_bonus = if config.ucb_c > 0.0 && self.total_pulls > 0 {
                config.ucb_c * (log_total / n_eff).sqrt()
            } else {
                0.0
            };
            let base = if self.logit_offsets[i].is_finite() {
                self.logit_offsets[i]
            } else {
                0.0
            };
            out[i] = (base + ucb_bonus).clamp(-max_d, max_d);
        }
        out
    }

    /// Applies bounded FiLM scale and shift modulation to the trunk representation slice.
    ///
    /// Does nothing when `config.online_adaptation` is `false`.
    pub fn apply_film(&self, hidden: &mut [f32], config: &AdaptConfig) {
        if !config.online_adaptation || hidden.is_empty() {
            return;
        }
        let max_d = if config.max_adapt_delta.is_finite() && config.max_adapt_delta > 0.0 {
            config.max_adapt_delta
        } else {
            DEFAULT_MAX_ADAPT_DELTA
        };
        let s0 = sanitize_clamped(self.film_scale_deltas[0], max_d);
        let s1 = sanitize_clamped(self.film_scale_deltas[1], max_d);
        let b0 = sanitize_clamped(self.film_shift_deltas[0], max_d);
        let b1 = sanitize_clamped(self.film_shift_deltas[1], max_d);

        let mid = hidden.len() / 2;
        for v in &mut hidden[..mid] {
            *v = (1.0 + s0) * *v + b0;
        }
        for v in &mut hidden[mid..] {
            *v = (1.0 + s1) * *v + b1;
        }
    }

    /// Updates this place's adaptation state from an observed or delayed one-sided outcome.
    ///
    /// When `config.online_adaptation` is `false`, returns without mutating any field.
    pub fn record_feedback(&mut self, feedback: AdaptFeedback, config: &AdaptConfig) {
        self.record_feedback_at(feedback, config, None, Taint::Trusted);
    }

    /// Updates this place's adaptation state with explicit event timestamp (`C-10`) and
    /// taint guard (`C-4`: untrusted negative/delayed social feedback cannot poison logits).
    pub fn record_feedback_at(
        &mut self,
        feedback: AdaptFeedback,
        config: &AdaptConfig,
        now_ms: Option<Time>,
        source_taint: Taint,
    ) {
        if !config.online_adaptation {
            return;
        }
        if !feedback.reward.is_finite() {
            return;
        }
        if source_taint == Taint::Untrusted
            && (feedback.reward < 0.0 || feedback.is_delayed_one_sided)
        {
            return;
        }
        if let Some(ts) = now_ms {
            self.apply_time_decay(ts, config);
        }
        let max_d = if config.max_adapt_delta.is_finite() && config.max_adapt_delta > 0.0 {
            config.max_adapt_delta
        } else {
            DEFAULT_MAX_ADAPT_DELTA
        };
        let arm_idx = feedback.arm as usize;
        let decay = if feedback.is_delayed_one_sided {
            config.delayed_negative_decay.clamp(0.0, 1.0)
        } else {
            1.0
        };
        let reward = feedback.reward.clamp(-1.0, 1.0);
        let step = config.learning_rate.max(0.0) * decay * reward;

        self.arm_pulls[arm_idx] = self.arm_pulls[arm_idx].saturating_add(1);
        self.total_pulls = self.total_pulls.saturating_add(1);
        self.arm_reward_sums[arm_idx] += f64::from(decay * reward);

        // Warm-Start UCB value-guided offset update, strictly clamped to [-max_d, +max_d].
        let base_offset = if self.logit_offsets[arm_idx].is_finite() {
            self.logit_offsets[arm_idx]
        } else {
            0.0
        };
        self.logit_offsets[arm_idx] = (base_offset + step).clamp(-max_d, max_d);

        // Coupled bounded FiLM update modulated by context_hint.
        let film_lr = 0.25 * step;
        for h in 0..2 {
            let hint = if feedback.context_hint[h].is_finite() {
                feedback.context_hint[h].clamp(-1.0, 1.0)
            } else {
                0.0
            };
            let s = sanitize_clamped(self.film_scale_deltas[h], max_d);
            let b = sanitize_clamped(self.film_shift_deltas[h], max_d);
            self.film_scale_deltas[h] = (s + film_lr * hint).clamp(-max_d, max_d);
            self.film_shift_deltas[h] = (b + film_lr).clamp(-max_d, max_d);
        }
    }

    /// Returns the empirical warm-start mean reward for `arm`.
    #[must_use]
    pub fn warm_start_mean_reward(&self, arm: GateArm, config: &AdaptConfig) -> f64 {
        let idx = arm as usize;
        let denom = self.arm_pulls[idx] as f64 + f64::from(config.warm_start_prior_weight.max(1.0));
        self.arm_reward_sums[idx] / denom
    }
}

#[inline]
fn sanitize_clamped(v: f32, max_d: f32) -> f32 {
    if v.is_finite() {
        v.clamp(-max_d, max_d)
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::neural::forward::forward_step;
    use crate::neural::weights::tests_support::build_synthetic_safetensors;
    use crate::neural::{CellType, NeuralModel};

    #[test]
    fn clamping_bounds_hold_under_extreme_rewards() {
        let cfg = AdaptConfig::default();
        let mut state = PlaceAdaptState::new();

        for _ in 0..500 {
            state.record_feedback(
                AdaptFeedback {
                    arm: GateArm::Go,
                    reward: 1_000_000.0,
                    is_delayed_one_sided: false,
                    context_hint: [10.0, -10.0],
                },
                &cfg,
            );
        }
        for &v in &state.logit_offsets {
            assert!(v >= -cfg.max_adapt_delta && v <= cfg.max_adapt_delta);
        }
        for &v in &state.film_scale_deltas {
            assert!(v >= -cfg.max_adapt_delta && v <= cfg.max_adapt_delta);
        }
        for &v in &state.film_shift_deltas {
            assert!(v >= -cfg.max_adapt_delta && v <= cfg.max_adapt_delta);
        }
        for v in state.effective_logit_offsets(&cfg) {
            assert!(v >= -cfg.max_adapt_delta && v <= cfg.max_adapt_delta);
        }

        for _ in 0..1000 {
            state.record_feedback(
                AdaptFeedback {
                    arm: GateArm::Go,
                    reward: -1_000_000.0,
                    is_delayed_one_sided: false,
                    context_hint: [-10.0, 10.0],
                },
                &cfg,
            );
        }
        for &v in &state.logit_offsets {
            assert!(v >= -cfg.max_adapt_delta && v <= cfg.max_adapt_delta);
        }
        for &v in &state.film_scale_deltas {
            assert!(v >= -cfg.max_adapt_delta && v <= cfg.max_adapt_delta);
        }
        for &v in &state.film_shift_deltas {
            assert!(v >= -cfg.max_adapt_delta && v <= cfg.max_adapt_delta);
        }
        for v in state.effective_logit_offsets(&cfg) {
            assert!(v >= -cfg.max_adapt_delta && v <= cfg.max_adapt_delta);
        }
    }

    #[test]
    fn delayed_one_sided_feedback_converges_away_from_ignored_arm() {
        let cfg = AdaptConfig::default();
        let mut state = PlaceAdaptState::new();

        // Repeatedly penalize Go via delayed one-sided feedback (e.g., ignored messages)
        // and mildly reward Silent.
        for _ in 0..20 {
            state.record_feedback(
                AdaptFeedback {
                    arm: GateArm::Go,
                    reward: -1.0,
                    is_delayed_one_sided: true,
                    context_hint: [0.5, 0.5],
                },
                &cfg,
            );
            state.record_feedback(
                AdaptFeedback {
                    arm: GateArm::Silent,
                    reward: 0.5,
                    is_delayed_one_sided: false,
                    context_hint: [0.5, 0.5],
                },
                &cfg,
            );
        }

        let offsets = state.effective_logit_offsets(&cfg);
        assert!(offsets[GateArm::Go as usize] < 0.0);
        assert!(offsets[GateArm::Silent as usize] > 0.0);
        assert!(offsets[GateArm::Silent as usize] > offsets[GateArm::Go as usize]);
        assert!(
            state.warm_start_mean_reward(GateArm::Silent, &cfg)
                > state.warm_start_mean_reward(GateArm::Go, &cfg)
        );
    }

    #[test]
    fn backbone_remains_immutable_and_disabled_toggle_isolates_completely() {
        let bytes = build_synthetic_safetensors(
            256,
            18,
            64,
            32,
            128,
            192,
            2,
            8,
            64,
            8,
            CellType::MinGru,
            false,
            false,
        );
        let model = NeuralModel::from_safetensors_bytes(&bytes, None).expect("valid model");
        let digest_before = model.blake3;

        let e_256 = vec![0.1_f32; 256];
        let x_num = vec![0.5_f32; 18];
        let h_prev = vec![0.0_f32; 64];

        let base_out = forward_step(Some(&model), &e_256, &x_num, &h_prev, None).expect("base");

        // Train adaptation state with online_adaptation = true.
        let active_cfg = AdaptConfig::default();
        let mut state = PlaceAdaptState::new();
        for _ in 0..10 {
            state.record_feedback(
                AdaptFeedback {
                    arm: GateArm::Go,
                    reward: 1.0,
                    is_delayed_one_sided: false,
                    context_hint: [1.0, 1.0],
                },
                &active_cfg,
            );
        }
        let adapted_out = forward_step(
            Some(&model),
            &e_256,
            &x_num,
            &h_prev,
            Some((&state, &active_cfg)),
        )
        .expect("adapted");
        assert_ne!(base_out.gate_logits, adapted_out.gate_logits);

        // Now turn online_adaptation = false: state must not be applied OR mutated.
        let disabled_cfg = AdaptConfig {
            online_adaptation: false,
            ..AdaptConfig::default()
        };
        let state_snapshot = state.clone();
        state.record_feedback(
            AdaptFeedback {
                arm: GateArm::Silent,
                reward: -1.0,
                is_delayed_one_sided: false,
                context_hint: [-1.0, -1.0],
            },
            &disabled_cfg,
        );
        assert_eq!(state, state_snapshot);

        let isolated_out = forward_step(
            Some(&model),
            &e_256,
            &x_num,
            &h_prev,
            Some((&state, &disabled_cfg)),
        )
        .expect("isolated");
        assert_eq!(base_out.gate_logits, isolated_out.gate_logits);
        assert_eq!(base_out.gate_probs, isolated_out.gate_probs);
        assert_eq!(model.blake3, digest_before);
    }

    #[test]
    fn taint_guard_blocks_untrusted_negative_poisoning_and_time_decay_relaxes_offsets() {
        let cfg = AdaptConfig::default();
        let mut st = PlaceAdaptState::new();

        // Untrusted negative / delayed feedback must be ignored (C-4).
        st.record_feedback_at(
            AdaptFeedback {
                arm: GateArm::Go,
                reward: -1.0,
                is_delayed_one_sided: false,
                context_hint: [0.8, -0.5],
            },
            &cfg,
            Some(Time::from_millis(1_000)),
            Taint::Untrusted,
        );
        assert_eq!(st.total_pulls, 0);
        assert_eq!(st.logit_offsets, [0.0; 4]);

        // Trusted feedback with non-zero context_hint moves FiLM scale deltas away from 0.0 (C-3).
        st.record_feedback_at(
            AdaptFeedback {
                arm: GateArm::Go,
                reward: 1.0,
                is_delayed_one_sided: false,
                context_hint: [0.8, -0.5],
            },
            &cfg,
            Some(Time::from_millis(1_000)),
            Taint::Trusted,
        );
        assert_eq!(st.total_pulls, 1);
        assert!(st.logit_offsets[GateArm::Go as usize] > 0.0);
        assert!(st.film_scale_deltas[0].abs() > 0.0);
        assert!(st.film_scale_deltas[1].abs() > 0.0);

        // 10 days later with adapt_daily_lambda = 0.90 -> offset decays by 0.90^10 (~0.3487) (C-10).
        let ten_days_ms = 1_000 + 10 * 86_400_000;
        let decayed = st.decayed_at(Time::from_millis(ten_days_ms), &cfg);
        let ratio =
            decayed.logit_offsets[GateArm::Go as usize] / st.logit_offsets[GateArm::Go as usize];
        assert!(
            (ratio - 0.90_f32.powi(10)).abs() < 1e-4,
            "expected decay ratio ~0.3487, got {ratio}"
        );
    }
}
