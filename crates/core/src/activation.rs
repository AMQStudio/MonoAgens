//! Guarded Petrov hybrid base-level activation (`A-01`, `R-04`), closeness, and familiarity.
//!
//! Shared across memory activation, actor closeness (`sigma(B_p)`), and place familiarity
//! (`sigma(B_ctx)`). Uses Petrov's (2006) hybrid (`k = 1` exact + power-law tail) guarded
//! by `n > k && t_n > t_k` with a 1-minute floor (`1/60` hours), neutral `0` on `n = 0`,
//! and cold-start pseudo-count shrinkage (`A-07`).

use monoagens_protocol::Time;

/// Minimum age floor in hours (1 minute = `1/60` hours), preventing `0/0` and singularities.
pub const MIN_AGE_HOURS: f64 = 1.0 / 60.0;

/// Milliseconds per hour (`3_600_000`).
pub const MS_PER_HOUR: f64 = 3_600_000.0;

/// Default exact recent-uses count `k` in Petrov's hybrid (`R-04`).
pub const DEFAULT_PETROV_K: usize = 1;

/// Maps `memory_persistence` in `[0.0, 1.0]` to ACT-R/Petrov decay exponent `d` in `[0.3, 0.7]`.
///
/// `0.0 -> 0.7` (faster fade), `0.5 -> 0.5` (ACT-R default), `1.0 -> 0.3` (slower fade).
#[must_use]
pub fn decay_exponent(memory_persistence: f64) -> f64 {
    let mp = if memory_persistence.is_finite() {
        memory_persistence.clamp(0.0, 1.0)
    } else {
        0.5
    };
    0.7 - 0.4 * mp
}

/// Logistic function `1 / (1 + exp(-x))`, safe across extreme inputs without NaN.
#[must_use]
pub fn sigmoid(x: f64) -> f64 {
    if !x.is_finite() {
        if x.is_nan() {
            return 0.5;
        }
        return if x.is_sign_positive() { 1.0 } else { 0.0 };
    }
    if x >= 0.0 {
        let z = (-x).exp();
        1.0 / (1.0 + z)
    } else {
        let z = x.exp();
        z / (1.0 + z)
    }
}

/// Converts `(now_ms - used_ms)` into hours, floored at [`MIN_AGE_HOURS`] (1 minute).
#[must_use]
pub fn age_hours_floored(used_ms: Time, now_ms: Time) -> f64 {
    let elapsed_ms = now_ms.saturating_sub_ms(used_ms) as f64;
    (elapsed_ms / MS_PER_HOUR).max(MIN_AGE_HOURS)
}

/// Evaluates guarded Petrov (2006) hybrid base-level activation from compact summary stats
/// (`k = 1`, `most_recent_ms`, `first_used_ms`, total count `n`).
///
/// - Returns `0.0` (neutral) when `n == 0`.
/// - Uses the power-law tail integral ONLY when `n > 1 && t_n > t_1` (with epsilon guard
///   against catastrophic cancellation / `0/0`).
/// - Otherwise computes the exact finite sum `ln(n * t_1^(-d))`, avoiding NaN/Inf even when
///   all uses fall within the 1-minute floor.
#[must_use]
pub fn petrov_summary(
    most_recent_ms: Time,
    first_used_ms: Time,
    n: u64,
    now_ms: Time,
    d: f64,
) -> f64 {
    if n == 0 {
        return 0.0;
    }
    let d_clamped = if d.is_finite() {
        d.clamp(0.0, 2.0)
    } else {
        0.5
    };
    let t1 = age_hours_floored(most_recent_ms, now_ms);
    let tn = age_hours_floored(first_used_ms, now_ms).max(t1);

    if n > DEFAULT_PETROV_K as u64 && tn > t1 + 1e-12 {
        let exact_head = t1.powf(-d_clamped);
        let tail = petrov_tail(n - DEFAULT_PETROV_K as u64, t1, tn, d_clamped);
        let total = (exact_head + tail).max(f64::MIN_POSITIVE);
        total.ln()
    } else {
        let total = ((n as f64) * t1.powf(-d_clamped)).max(f64::MIN_POSITIVE);
        total.ln()
    }
}

/// Evaluates guarded Petrov (2006) hybrid base-level activation over explicit use timestamps.
///
/// `timestamps_ms` may be in any order.
/// - Returns `0.0` (neutral) when empty (`n == 0`).
/// - When `n > k && t_n > t_k` (after flooring each age at 1 minute), uses exact sum of the
///   `k` most recent uses plus Petrov's tail integral over the remaining `n - k` uses.
/// - Otherwise computes exact `ln(sum_{j=1..n} t_j^(-d))`.
#[must_use]
pub fn petrov_from_timestamps(timestamps_ms: &[Time], now_ms: Time, d: f64, k: usize) -> f64 {
    let n = timestamps_ms.len();
    if n == 0 {
        return 0.0;
    }
    let d_clamped = if d.is_finite() {
        d.clamp(0.0, 2.0)
    } else {
        0.5
    };
    let k_eff = k.max(1);

    let mut ages: Vec<f64> = timestamps_ms
        .iter()
        .map(|&ts| age_hours_floored(ts, now_ms))
        .collect();
    // Sort ascending so ages[0] = t_1 (most recent) and ages[n-1] = t_n (oldest).
    ages.sort_by(f64::total_cmp);

    if n > k_eff {
        let tk = ages[k_eff - 1];
        let tn = ages[n - 1];
        if tn > tk + 1e-12 {
            let exact_head: f64 = ages[..k_eff].iter().map(|&t| t.powf(-d_clamped)).sum();
            let tail = petrov_tail((n - k_eff) as u64, tk, tn, d_clamped);
            let total = (exact_head + tail).max(f64::MIN_POSITIVE);
            return total.ln();
        }
    }

    let exact_sum: f64 = ages.iter().map(|&t| t.powf(-d_clamped)).sum();
    exact_sum.max(f64::MIN_POSITIVE).ln()
}

fn petrov_tail(rem_count: u64, tk: f64, tn: f64, d: f64) -> f64 {
    if rem_count == 0 || tn <= tk {
        return 0.0;
    }
    let span = tn - tk;
    let one_minus_d = 1.0 - d;
    let integral_factor = if one_minus_d.abs() < 1e-9 {
        (tn / tk).ln() / span
    } else {
        (tn.powf(one_minus_d) - tk.powf(one_minus_d)) / (one_minus_d * span)
    };
    ((rem_count as f64) * integral_factor).max(0.0)
}

/// Compact O(1) usage history summary used for memory items, place presence, and actor exchanges.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct ActivationTrace {
    pub count: u64,
    pub first_ms: Time,
    pub last_ms: Time,
}

impl ActivationTrace {
    /// Records one occurrence at `timestamp_ms`.
    pub fn record(&mut self, timestamp_ms: Time) {
        if self.count == 0 {
            self.count = 1;
            self.first_ms = timestamp_ms;
            self.last_ms = timestamp_ms;
        } else {
            self.count = self.count.saturating_add(1);
            if timestamp_ms < self.first_ms {
                self.first_ms = timestamp_ms;
            }
            if timestamp_ms > self.last_ms {
                self.last_ms = timestamp_ms;
            }
        }
    }

    /// Evaluates guarded Petrov base-level activation `B` at `now_ms` (`0.0` if `count == 0`).
    #[must_use]
    pub fn base_level(&self, now_ms: Time, d: f64) -> f64 {
        petrov_summary(self.last_ms, self.first_ms, self.count, now_ms, d)
    }
}

/// Computes actor closeness in `[0.0, 1.0]` as `sigma(B_p)` with `A-07` cold-start shrinkage.
///
/// - When `n == 0`, returns exact neutral `0.0` (even when `pseudocount == 0.0`).
/// - Shrinks toward `0.0` via `(n / (n + pseudocount)) * sigma(B_p)` so first-minute
///   exchanges do not immediately saturate closeness and forgotten contacts decay cleanly.
#[must_use]
pub fn closeness_from_trace(
    trace: &ActivationTrace,
    now_ms: Time,
    d: f64,
    pseudocount: f64,
) -> f64 {
    if trace.count == 0 {
        return 0.0;
    }
    let b_p = trace.base_level(now_ms, d);
    let raw = sigmoid(b_p);
    let n = trace.count as f64;
    let n0 = if pseudocount.is_finite() {
        pseudocount.max(0.0)
    } else {
        0.0
    };
    let weight = n / (n + n0);
    (weight * raw).clamp(0.0, 1.0)
}

/// Computes combined context base-level activation `B_ctx = (B_P + sum_{p in Present} B_p) / (1 + |Present|)`.
///
/// Returns `None` when neither the place nor any present actor has any recorded history (`n = 0` everywhere),
/// allowing [`place_familiarity`] to return exact neutral `0.0`.
#[must_use]
pub fn context_base_level(
    place_trace: &ActivationTrace,
    present_actor_traces: &[&ActivationTrace],
    now_ms: Time,
    d: f64,
) -> Option<f64> {
    let any_history = place_trace.count > 0 || present_actor_traces.iter().any(|t| t.count > 0);
    if !any_history {
        return None;
    }
    let b_place = place_trace.base_level(now_ms, d);
    let sum_actors: f64 = present_actor_traces
        .iter()
        .map(|t| t.base_level(now_ms, d))
        .sum();
    let denom = (1 + present_actor_traces.len()) as f64;
    Some((b_place + sum_actors) / denom)
}

/// Computes place familiarity in `[0.0, 1.0]` as `sigma(B_ctx)` with `A-07` cold-start shrinkage.
///
/// - When there is no presence or exchange history (`n = 0` everywhere), returns exact neutral `0.0`.
/// - Shrinks toward `0.0` via `(n_eff / (n_eff + pseudocount)) * sigma(B_ctx)` where `n_eff` is
///   total presence + present-actor exchange counts.
#[must_use]
pub fn place_familiarity(
    place_trace: &ActivationTrace,
    present_actor_traces: &[&ActivationTrace],
    now_ms: Time,
    d: f64,
    pseudocount: f64,
) -> f64 {
    let Some(b_ctx) = context_base_level(place_trace, present_actor_traces, now_ms, d) else {
        return 0.0;
    };
    let raw = sigmoid(b_ctx);
    let total_n: u64 = place_trace
        .count
        .saturating_add(present_actor_traces.iter().map(|t| t.count).sum());
    if total_n == 0 {
        return 0.0;
    }
    let n = total_n as f64;
    let n0 = if pseudocount.is_finite() {
        pseudocount.max(0.0)
    } else {
        0.0
    };
    let weight = n / (n + n0);
    (weight * raw).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::{
        ActivationTrace, MIN_AGE_HOURS, closeness_from_trace, context_base_level, decay_exponent,
        petrov_from_timestamps, petrov_summary, place_familiarity, sigmoid,
    };
    use monoagens_protocol::Time;

    #[test]
    fn petrov_zero_history_is_neutral_zero_and_first_minute_avoids_nan() {
        let now = Time::from_millis(120_000);
        assert_eq!(petrov_summary(now, now, 0, now, 0.5), 0.0);
        assert_eq!(petrov_from_timestamps(&[], now, 0.5, 1), 0.0);

        // Multiple identical timestamps within the 1-minute floor (t_1 = t_n = 1/60 h).
        let ts = [
            Time::from_millis(100_000),
            Time::from_millis(100_000),
            Time::from_millis(110_000),
            Time::from_millis(120_000),
        ];
        let b_ts = petrov_from_timestamps(&ts, now, 0.5, 1);
        let b_sum = petrov_summary(
            Time::from_millis(120_000),
            Time::from_millis(100_000),
            4,
            now,
            0.5,
        );
        assert!(b_ts.is_finite());
        assert!(b_sum.is_finite());
        let expected = (4.0 * MIN_AGE_HOURS.powf(-0.5)).ln();
        assert!((b_ts - expected).abs() < 1e-12);
        assert!((b_sum - expected).abs() < 1e-12);
    }

    #[test]
    fn petrov_long_gap_matches_30_day_fade_threshold_and_handles_d_near_one() {
        let t0 = Time::from_millis(0);
        let now_30d = Time::from_millis(720 * 3_600_000);
        let b_30d = petrov_summary(t0, t0, 1, now_30d, 0.5);
        assert!((b_30d - (-3.2896)).abs() < 1e-3, "got {b_30d}");

        // Hybrid tail when n > 1 and t_n > t_1, including d = 1.0 limit.
        let recent = Time::from_millis(719 * 3_600_000);
        let b_hybrid = petrov_summary(recent, t0, 10, now_30d, 0.5);
        let b_d1 = petrov_summary(recent, t0, 10, now_30d, 1.0);
        assert!(b_hybrid.is_finite());
        assert!(b_d1.is_finite());
        assert!(b_hybrid > b_30d);
    }

    #[test]
    fn closeness_and_familiarity_decay_and_stay_zero_with_no_exchanges() {
        let mut trace = ActivationTrace::default();
        let now = Time::from_millis(100_000);
        assert_eq!(closeness_from_trace(&trace, now, 0.5, 5.0), 0.0);
        assert_eq!(place_familiarity(&trace, &[], now, 0.5, 5.0), 0.0);
        assert_eq!(context_base_level(&trace, &[], now, 0.5), None);

        // Large channel where 500 actors have 0 exchanges and soul has 0 presence.
        let empty = ActivationTrace::default();
        let crowd: Vec<&ActivationTrace> = vec![&empty; 50];
        assert_eq!(place_familiarity(&trace, &crowd, now, 0.5, 5.0), 0.0);

        // Build contact then pause 30 days -> closeness and familiarity decay.
        trace.record(Time::from_millis(0));
        trace.record(Time::from_millis(60_000));
        trace.record(Time::from_millis(120_000));
        let soon = Time::from_millis(180_000);
        let later = Time::from_millis(180_000 + 30 * 24 * 3_600_000);

        let d = decay_exponent(0.5);
        let close_soon = closeness_from_trace(&trace, soon, d, 5.0);
        let close_later = closeness_from_trace(&trace, later, d, 5.0);
        assert!(close_soon > close_later);
        assert!(close_later >= 0.0);

        let fam_soon = place_familiarity(&trace, &[&trace], soon, d, 5.0);
        let fam_later = place_familiarity(&trace, &[&trace], later, d, 5.0);
        assert!(fam_soon > fam_later);
        assert!(sigmoid(0.0) == 0.5);
    }
}
