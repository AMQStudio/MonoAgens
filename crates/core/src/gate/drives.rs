//! Opportunity-cost drives (`R-31`, `A-04`, `A-14`):
//! - `D_sleep`: homeostatic consolidation urgency feeding `T_idle_eff` and idle initiatives (`Δθ_sleep = 0` on external events)
//! - `D_switch`: switching/exploration drive compressing dead-end `B_steps`
//! - `D_curiosity`: epistemic curiosity combining `FoK` inverted-U, surprise EMA, and 1-step Beta EVSI
//! - `D_effort`: Shenhav EVC marginal deliberation control over `(k, n_recall)`

use serde::{Deserialize, Serialize};

use crate::prior::Prior;

/// Summary of a pending Beta belief `(alpha_b, beta_b)` linked to an active goal/decision (`R-31`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PendingBeliefSummary {
    pub alpha: f64,
    pub beta: f64,
    pub stakes: f64,
    pub depends_on_active_goal: bool,
}

/// Output of `effort_control` (`D_effort`, ambient opportunity cost, marginal EVC, and stop verdict).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub struct EffortControl {
    pub rho_opp: f64,
    pub cost_ctrl: f64,
    pub delta_benefit: f64,
    pub marginal_cost: f64,
    pub evc_marg: f64,
    pub d_effort: f64,
    pub should_stop: bool,
}

/// Result of `switching_drive` evaluation (`D_switch` and adjusted step budget `b_eff`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub struct SwitchingBudget {
    pub d_switch: f64,
    pub effective_b_steps: usize,
    pub should_switch: bool,
}

/// Computes Drive 1: Sleep Pressure `D_sleep in [0, D_sleep^max]` (`R-31`, `A-04`).
///
/// Feeds ONLY the consolidation idle trigger (`T_idle_eff = T_idle / max(1, D_sleep)`)
/// and idle-initiative cost (`C_int`), NEVER external incoming `theta` (`Delta theta_sleep = 0`).
#[must_use]
pub fn sleep_pressure(unconsolidated_count: usize, wake_elapsed_ms: u64, prior: &Prior) -> f64 {
    let n_min = (prior.n_min_sleep.max(1)) as f64;
    let backlog_ratio = (unconsolidated_count as f64) / n_min;
    let dt_wake_h = (wake_elapsed_ms as f64) / 3_600_000.0;
    let wake_term = 1.0 - (-dt_wake_h / prior.tau_wake_hours.max(1e-6)).exp();
    let raw = prior.alpha_u * backlog_ratio + (1.0 - prior.alpha_u) * wake_term;
    if raw.is_finite() {
        raw.clamp(0.0, prior.d_sleep_max)
    } else {
        0.0
    }
}

/// Computes effective idle trigger duration `T_idle_eff = T_idle_ms / max(1.0, D_sleep)`.
#[must_use]
pub fn effective_idle_trigger_ms(base_t_idle_ms: u64, d_sleep: f64) -> u64 {
    let denom = if d_sleep.is_finite() {
        d_sleep.max(1.0)
    } else {
        1.0
    };
    ((base_t_idle_ms as f64) / denom).round().max(0.0) as u64
}

/// Computes Drive 2: Switching / Exploration Drive `D_switch in [0, 1]` and compressed step budget `B_eff` (`R-31`).
#[must_use]
pub fn switching_drive(
    n_stag: usize,
    n_fail: usize,
    stakes: f64,
    prior: &Prior,
) -> SwitchingBudget {
    let b_steps = prior.b_steps.max(1);
    if n_stag == 0 && n_fail == 0 {
        return SwitchingBudget {
            d_switch: 0.0,
            effective_b_steps: b_steps,
            should_switch: false,
        };
    }
    let rho = if stakes.is_finite() {
        stakes.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let denom = (prior.tau_sw * (b_steps as f64)).max(1e-6);
    let exponent = -((n_stag as f64) + prior.w_fail * (n_fail as f64)) / denom;
    let d_switch = ((1.0 - prior.lambda_stakes * rho) * (1.0 - exponent.exp())).clamp(0.0, 1.0);
    let effective_b_steps = ((b_steps as f64) * (1.0 - prior.w_sw * d_switch))
        .ceil()
        .max(1.0) as usize;
    let should_switch = d_switch >= prior.theta_sw;
    SwitchingBudget {
        d_switch,
        effective_b_steps,
        should_switch,
    }
}

/// Computes normalized 1-step Beta Expected Variance Reduction (EVSI) weighted by decision impact (`R-31`).
///
/// `36 * DeltaVar_1(b) = 36 * (alpha * beta) / (n^2 * (n + 1)^2)` in `(0, 1]`.
#[must_use]
pub fn max_belief_voi(pending_beliefs: &[PendingBeliefSummary]) -> f64 {
    let mut best = 0.0_f64;
    for b in pending_beliefs {
        if !b.depends_on_active_goal {
            continue;
        }
        if !b.alpha.is_finite() || !b.beta.is_finite() || b.alpha <= 0.0 || b.beta <= 0.0 {
            continue;
        }
        let n = b.alpha + b.beta;
        let denom = n * n * (n + 1.0) * (n + 1.0);
        if denom <= 0.0 || !denom.is_finite() {
            continue;
        }
        let evsi_norm = (36.0 * (b.alpha * b.beta) / denom).clamp(0.0, 1.0);
        let p = b.alpha / n;
        let stakes = if b.stakes.is_finite() {
            b.stakes.clamp(0.0, 1.0)
        } else {
            0.0
        };
        let impact = stakes * (1.0 - (2.0 * p - 1.0).abs());
        best = best.max(evsi_norm * impact);
    }
    best.clamp(0.0, 1.0)
}

/// Computes Drive 3: Curiosity Drive `D_curiosity in [0, 1]` (`R-31`, `A-14`).
#[must_use]
pub fn curiosity_drive(
    goal_relevance: f64,
    fok: f64,
    surprise_ema: f64,
    pending_beliefs: &[PendingBeliefSummary],
    prior: &Prior,
) -> f64 {
    let rel = if goal_relevance.is_finite() {
        goal_relevance.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let fok_c = if fok.is_finite() {
        fok.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let surp = if surprise_ema.is_finite() {
        surprise_ema.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let voi = max_belief_voi(pending_beliefs);
    let raw = prior.lambda_fok * rel * 4.0 * fok_c * (1.0 - fok_c)
        + prior.lambda_surp * surp
        + prior.lambda_voi * voi;
    if raw.is_finite() {
        raw.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

/// Computes Drive 4: Effort Cost & EVC Marginal Deliberation Control (`R-31`).
#[must_use]
#[allow(clippy::too_many_arguments)]
pub fn effort_control(
    queue_pending: usize,
    d_sleep: f64,
    talk_rate: f64,
    step_k: usize,
    n_recall: usize,
    stakes: f64,
    confidence: f64,
    prior: &Prior,
) -> EffortControl {
    let sleep_excess = if d_sleep.is_finite() {
        (d_sleep - 1.0).max(0.0)
    } else {
        0.0
    };
    let tr = if talk_rate.is_finite() {
        talk_rate.max(0.0)
    } else {
        0.0
    };
    let rho_opp = ((queue_pending as f64) / prior.q_ref.max(1.0)
        + prior.eta_sl * sleep_excess
        + prior.eta_tr * tr)
        .clamp(0.0, 2.0);

    let b_steps = prior.b_steps.max(1) as f64;
    let n_rec_max = prior.n_rec_max.max(1) as f64;
    let u_curr = (step_k as f64) / b_steps + prior.w_rec * ((n_recall as f64) / n_rec_max);
    let u_next = ((step_k + 1) as f64) / b_steps + prior.w_rec * ((n_recall as f64) / n_rec_max);

    let scale = prior.c0 * (1.0 + prior.gamma_opp * rho_opp);
    let cost_ctrl = scale * u_curr.max(0.0).powf(prior.alpha_pow);
    let cost_next = scale * u_next.max(0.0).powf(prior.alpha_pow);
    let marginal_cost = (cost_next - cost_ctrl).max(0.0);

    let rho_stakes = if stakes.is_finite() {
        stakes.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let conf = if confidence.is_finite() {
        confidence.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let delta_benefit =
        rho_stakes * (1.0 - conf) * (-(step_k as f64) / prior.tau_k.max(1e-6)).exp();
    let evc_marg = delta_benefit - marginal_cost;
    let d_effort = (cost_ctrl / (rho_stakes + prior.c0).max(1e-9)).clamp(0.0, 1.0);
    let should_stop = prior.c0 > 0.0 && evc_marg <= 0.0;

    EffortControl {
        rho_opp,
        cost_ctrl,
        delta_benefit,
        marginal_cost,
        evc_marg,
        d_effort,
        should_stop,
    }
}
