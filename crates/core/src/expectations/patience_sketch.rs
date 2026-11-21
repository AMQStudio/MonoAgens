//! Right-censored log-spaced DDSketch histogram (`[1 ms, 365 d]`, `alpha = 0.05`, 242 buckets).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Minimum bucket latency in milliseconds (`1 ms`).
pub const PATIENCE_L_MIN_MS: f64 = 1.0;

/// Maximum bucket latency in milliseconds (`365 days`).
pub const PATIENCE_L_MAX_MS: f64 = 365.0 * 24.0 * 3_600_000.0;

/// DDSketch relative error tolerance (`5%`).
pub const PATIENCE_ALPHA_LAT: f64 = 0.05;

/// Geometric bucket ratio `(1 + alpha) / (1 - alpha)`.
pub const PATIENCE_GAMMA: f64 = (1.0 + PATIENCE_ALPHA_LAT) / (1.0 - PATIENCE_ALPHA_LAT);

/// Fixed bucket count covering `[1 ms, 365 d]` at `alpha = 0.05`.
pub const PATIENCE_NUM_BUCKETS: usize = 242;

/// Maps a millisecond latency into DDSketch bucket index `0..PATIENCE_NUM_BUCKETS`.
#[must_use]
pub fn bucket_for_ms(latency_ms: u64) -> u16 {
    let x = (latency_ms.max(1) as f64).clamp(PATIENCE_L_MIN_MS, PATIENCE_L_MAX_MS);
    let idx = ((x / PATIENCE_L_MIN_MS).ln() / PATIENCE_GAMMA.ln()).ceil();
    idx.clamp(0.0, (PATIENCE_NUM_BUCKETS - 1) as f64) as u16
}

/// Upper boundary `u_k = L_min * gamma^k` in milliseconds.
#[must_use]
pub fn bucket_upper_ms(bucket: u16) -> f64 {
    PATIENCE_L_MIN_MS * PATIENCE_GAMMA.powi(i32::from(bucket))
}

/// DDSketch midpoint `2 * u_k / (gamma + 1)` in milliseconds.
#[must_use]
pub fn bucket_midpoint_ms(bucket: u16) -> f64 {
    2.0 * bucket_upper_ms(bucket) / (PATIENCE_GAMMA + 1.0)
}

/// Standard normal CDF `Phi(z)` using Abramowitz & Stegun 7.1.26 (`|err| <= 1.5e-7`).
#[must_use]
pub fn normal_cdf(z: f64) -> f64 {
    if !z.is_finite() {
        return if z.is_nan() {
            0.5
        } else if z.is_sign_positive() {
            1.0
        } else {
            0.0
        };
    }
    let sign = if z < 0.0 { -1.0 } else { 1.0 };
    let x = z.abs() / core::f64::consts::SQRT_2;
    let t = 1.0 / (1.0 + 0.327_591_1 * x);
    let poly = t
        * (0.254_829_592
            + t * (-0.284_496_736
                + t * (1.421_413_741 + t * (-1.453_152_027 + t * 1.061_405_429))));
    let erf = sign * (1.0 - poly * (-x * x).exp());
    (0.5 * (1.0 + erf)).clamp(0.0, 1.0)
}

/// Bounded log-spaced Kaplan-Meier latency sketch tracking observed replies and right-censored waits.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct LatencySketch {
    pub events: BTreeMap<u16, f64>,
    pub censored: BTreeMap<u16, f64>,
}

impl LatencySketch {
    /// Records one latency observation (`observed == true` for a third-party reply,
    /// `observed == false` for a right-censored wait expiry) with exponential forgetting `rho`.
    pub fn observe(&mut self, latency_ms: u64, observed: bool, rho: f64) {
        let rho_c = if rho.is_finite() {
            rho.clamp(0.0, 1.0)
        } else {
            1.0
        };
        if rho_c < 1.0 {
            for val in self.events.values_mut() {
                *val *= rho_c;
            }
            for val in self.censored.values_mut() {
                *val *= rho_c;
            }
            self.events.retain(|_, v| *v > 1e-12);
            self.censored.retain(|_, v| *v > 1e-12);
        }
        let bucket = bucket_for_ms(latency_ms);
        let map = if observed {
            &mut self.events
        } else {
            &mut self.censored
        };
        *map.entry(bucket).or_insert(0.0) += 1.0;
    }

    /// Total effective observation weight (observed + right-censored).
    #[must_use]
    pub fn total_weight(&self) -> f64 {
        let ev: f64 = self.events.values().sum();
        let cen: f64 = self.censored.values().sum();
        (ev + cen).max(0.0)
    }

    /// Computes `(total_weight, F_KM[0..PATIENCE_NUM_BUCKETS])` via discrete Kaplan-Meier survival.
    #[must_use]
    pub fn kaplan_meier_cdf(&self) -> (f64, Vec<f64>) {
        let total = self.total_weight();
        if total <= 0.0 {
            return (0.0, vec![0.0; PATIENCE_NUM_BUCKETS]);
        }

        let mut bucket_ev = vec![0.0_f64; PATIENCE_NUM_BUCKETS];
        let mut bucket_tot = vec![0.0_f64; PATIENCE_NUM_BUCKETS];
        for (&b, &w) in &self.events {
            let idx = (b as usize).min(PATIENCE_NUM_BUCKETS - 1);
            bucket_ev[idx] += w.max(0.0);
            bucket_tot[idx] += w.max(0.0);
        }
        for (&b, &w) in &self.censored {
            let idx = (b as usize).min(PATIENCE_NUM_BUCKETS - 1);
            bucket_tot[idx] += w.max(0.0);
        }

        let mut at_risk = vec![0.0_f64; PATIENCE_NUM_BUCKETS];
        let mut running = 0.0_f64;
        for idx in (0..PATIENCE_NUM_BUCKETS).rev() {
            running += bucket_tot[idx];
            at_risk[idx] = running;
        }

        let mut cdf = vec![0.0_f64; PATIENCE_NUM_BUCKETS];
        let mut survival = 1.0_f64;
        for idx in 0..PATIENCE_NUM_BUCKETS {
            let y = at_risk[idx];
            let d = bucket_ev[idx];
            if y > 0.0 && d > 0.0 {
                let hazard = (d / y).clamp(0.0, 1.0);
                survival *= 1.0 - hazard;
            }
            cdf[idx] = (1.0 - survival).clamp(0.0, 1.0);
        }

        (total, cdf)
    }
}

