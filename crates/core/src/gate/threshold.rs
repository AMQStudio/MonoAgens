//! Gate threshold `theta`, cold-start shrunk `talk_rate` (`A-07`), `promise_debt`,
//! permanent `Delta theta_sleep = 0` (`A-04`), and breakpoint-aware `Delta theta_init` (`R-30`).

use monoagens_protocol::Time;
use serde::{Deserialize, Serialize};

use crate::activation::MS_PER_HOUR;
use crate::prior::Prior;
use crate::registry::Switches;

/// Summary of an open promise or recurring monitor used for `promise_debt` calculation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PromiseSummary {
    pub due_ms: Time,
    pub urgency_weight: f64,
    /// `true` for recurring `monitor` expectations (`RecurrenceSpec`), which are excluded from `promise_debt`.
    pub is_recurring_monitor: bool,
    pub active: bool,
}

/// Initiative habituation state stored per place under synthetic `"init:<place_id>"` (`R-30`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub struct InitHabituationEntry {
    pub h_init: f64,
    pub last_updated_ms: Time,
}

impl InitHabituationEntry {
    /// Decays `h_init` by `gamma_init^(dt_hours)` up to `now_ms`.
    #[must_use]
    pub fn decayed_prior(&self, now_ms: Time, gamma_init: f64) -> f64 {
        if self.h_init <= 0.0 || !self.h_init.is_finite() {
            return 0.0;
        }
        let dt_hours = (now_ms.saturating_sub_ms(self.last_updated_ms) as f64) / MS_PER_HOUR;
        (self.h_init * gamma_init.clamp(0.0, 1.0).powf(dt_hours)).max(0.0)
    }

    /// Updates `h_init` after an initiative emit (`+delta_emit`) or ignore/held (`+delta_ign`).
    pub fn record_delta(&mut self, now_ms: Time, gamma_init: f64, delta: f64) {
        let h_pre = self.decayed_prior(now_ms, gamma_init);
        self.h_init = (h_pre + delta.max(0.0)).max(0.0);
        self.last_updated_ms = now_ms;
    }
}

/// Inputs required to assemble the gate threshold `theta`.
#[derive(Debug, Clone)]
pub struct ThresholdInputs<'a> {
    pub now_ms: Time,
    pub soul_acts_in_window: usize,
    pub total_acts_in_window: usize,
    pub active_speakers: usize,
    pub budget_used_ratio: f64,
    pub promises: &'a [PromiseSummary],
    pub familiarity: f64,
    pub init_habituation: Option<InitHabituationEntry>,
    pub is_idle_initiative: bool,
}

/// Complete term-by-term threshold breakdown stored inside every recorded gate decision.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub struct ThresholdTerms {
    pub theta_0: f64,
    pub a: f64,
    pub talk_rate: f64,
    pub b: f64,
    pub budget_used: f64,
    pub c: f64,
    pub promise_debt: f64,
    pub r: f64,
    pub familiarity: f64,
    pub delta_theta_init: f64,
    /// Permanently `0.0` (`A-04` invariant: background sleep never raises gate threshold).
    pub delta_theta_sleep: f64,
    pub theta: f64,
}

impl ThresholdTerms {
    /// Recomputes `theta` strictly from the stored terms (for log-only decision verification).
    #[must_use]
    pub fn recompute_theta(&self) -> f64 {
        let raw = self.theta_0 + self.a * self.talk_rate + self.b * self.budget_used
            - self.c * self.promise_debt
            + self.r * (1.0 - self.familiarity)
            + self.delta_theta_init
            + self.delta_theta_sleep;
        if raw.is_finite() { raw } else { self.theta_0 }
    }
}

/// Computes cold-start shrunk `talk_rate` (`A-07`, `RESEARCH.md` §3.3).
///
/// Shrinks empirical share `(soul_acts / total_acts)` toward fair share `1 / max(1, active_speakers)`
/// with pseudo-count `prior_count`, returning excess above fair share normalized to `[0, 1]`
/// (or shrunk rate directly when `active_speakers <= 1`). Zero-history returns `0.0`.
#[must_use]
pub fn compute_talk_rate(
    soul_acts_in_window: usize,
    total_acts_in_window: usize,
    active_speakers: usize,
    prior_count: f64,
) -> f64 {
    if soul_acts_in_window == 0 {
        return 0.0;
    }
    let n0 = if prior_count.is_finite() {
        prior_count.max(0.0)
    } else {
        0.0
    };
    let s = soul_acts_in_window as f64;
    let t = (total_acts_in_window.max(soul_acts_in_window)) as f64;

    if active_speakers <= 1 {
        let denom = t + n0;
        if denom <= 0.0 {
            return 0.0;
        }
        return (s / denom).clamp(0.0, 1.0);
    }

    let fair_share = 1.0 / (active_speakers as f64);
    let denom = t + n0;
    if denom <= 0.0 {
        return 0.0;
    }
    let shrunk_share = (s + n0 * fair_share) / denom;
    let excess = (shrunk_share - fair_share).max(0.0);
    (excess / (1.0 - fair_share)).clamp(0.0, 1.0)
}

/// Sums undiluted urgency weights `sum min(1.0, w_p)` over imminent non-recurring promises
/// due within `horizon_ms` (or already overdue), explicitly excluding recurring `monitor`s (`R-46`).
#[must_use]
pub fn compute_promise_debt(promises: &[PromiseSummary], now_ms: Time, horizon_ms: u64) -> f64 {
    let deadline_cutoff = now_ms.saturating_add_ms(horizon_ms);
    promises
        .iter()
        .filter(|p| p.active && !p.is_recurring_monitor && p.due_ms <= deadline_cutoff)
        .map(|p| {
            if p.urgency_weight.is_finite() {
                p.urgency_weight.clamp(0.0, 1.0)
            } else {
                0.0
            }
        })
        .sum()
}

/// Computes deterministic breakpoint receptivity `b_bp(t, p) in {0.0, 0.5, 1.0}` (`R-30`).
#[must_use]
#[allow(clippy::too_many_arguments)]
pub fn breakpoint_score(
    now_ms: Time,
    last_boundary_ms: Option<Time>,
    expectation_just_resolved_done: bool,
    b_phys: bool,
    open_waits: usize,
    quiet_elapsed_ms: u64,
    med_lat_ms: u64,
    prior: &Prior,
) -> f64 {
    let within_boundary_window = last_boundary_ms.is_some_and(|b_ms| {
        let dt_s = now_ms.saturating_sub_ms(b_ms) as f64 / 1000.0;
        dt_s <= prior.tau_win_s
    });
    if within_boundary_window || expectation_just_resolved_done || b_phys {
        return 1.0;
    }
    let threshold_quiet_ms = ((prior.kappa_bp * (med_lat_ms as f64)).round().max(0.0)) as u64;
    if open_waits == 0 && med_lat_ms > 0 && quiet_elapsed_ms >= threshold_quiet_ms {
        0.5
    } else {
        0.0
    }
}

/// Computes idle initiative cooldown `Delta theta_init = step_init * h_init_prior(p)` (`R-30`).
#[must_use]
pub fn compute_delta_theta_init(
    entry: Option<InitHabituationEntry>,
    now_ms: Time,
    prior: &Prior,
) -> f64 {
    let Some(entry) = entry else {
        return 0.0;
    };
    let h_pre = entry.decayed_prior(now_ms, prior.gamma_init);
    (prior.step_init * h_pre).max(0.0)
}

/// Computes Horvitz (1999) interruption cost `C_int(t, p)` for breakpoint-aware idle initiatives (`R-30`).
#[must_use]
pub fn idle_interruption_cost(b_bp: f64, burst_load: f64, d_sleep: f64, prior: &Prior) -> f64 {
    let bp = if b_bp.is_finite() {
        b_bp.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let load = if burst_load.is_finite() {
        burst_load.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let sleep_excess = if d_sleep.is_finite() {
        (d_sleep - 1.0).max(0.0)
    } else {
        0.0
    };
    (prior.c_base * (1.0 - prior.beta_bp * bp)
        + prior.beta_load * load
        + prior.lambda_sleep_init * sleep_excess)
        .max(0.0)
}

/// Assembles `theta = theta_0 + a*talk_rate + b*budget_used - c*promise_debt + r*(1 - familiarity) + Delta theta_init`.
///
/// Enforces `Delta theta_sleep = 0.0` permanently (`A-04`) and reachable `a = b = c = 0.0` / `r = 0.0`
/// via both [`Prior`] parameters and [`Switches`].
#[must_use]
pub fn compute_threshold(
    inputs: &ThresholdInputs<'_>,
    prior: &Prior,
    switches: &Switches,
) -> ThresholdTerms {
    let theta_0 = prior.theta_0();
    let drive_on = switches.drive_threshold;
    let a = if drive_on {
        prior.a_scale * prior.restraint
    } else {
        0.0
    };
    let b = if drive_on { prior.budget_weight } else { 0.0 };
    let c = if drive_on {
        prior.promise_debt_weight
    } else {
        0.0
    };
    let r = if switches.familiarity_reserve {
        prior.reserve_when_unfamiliar
    } else {
        0.0
    };

    let talk_rate = compute_talk_rate(
        inputs.soul_acts_in_window,
        inputs.total_acts_in_window,
        inputs.active_speakers,
        prior.talk_rate_prior_count,
    );
    let budget_used = if inputs.budget_used_ratio.is_finite() {
        inputs.budget_used_ratio.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let promise_debt = compute_promise_debt(
        inputs.promises,
        inputs.now_ms,
        prior.promise_debt_horizon_ms,
    );
    let familiarity = if inputs.familiarity.is_finite() {
        inputs.familiarity.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let delta_theta_init = if inputs.is_idle_initiative {
        compute_delta_theta_init(inputs.init_habituation, inputs.now_ms, prior)
    } else {
        0.0
    };
    let delta_theta_sleep = 0.0;

    let mut terms = ThresholdTerms {
        theta_0,
        a,
        talk_rate,
        b,
        budget_used,
        c,
        promise_debt,
        r,
        familiarity,
        delta_theta_init,
        delta_theta_sleep,
        theta: 0.0,
    };
    terms.theta = terms.recompute_theta();
    terms
}
