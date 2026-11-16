//! Right-censored, hierarchically shrunk quantile patience estimator (`A-08`, `R-08`).
//!
//! Uses a bounded log-spaced DDSketch histogram (`[1 ms, 365 d]`, `alpha = 0.05`, 242 buckets)
//! with discrete Kaplan-Meier right-censored survival estimation and hierarchical shrinkage:
//! `(place, medium) -> medium -> log-normal prior F_0`, returning `q_0.10` clamped to
//! `[t_min, t_max]` (or `0` when `Switches::patience == false`).

use std::collections::BTreeMap;

use monoagens_protocol::{ActorId, Audience, ChannelId, EventId, PlaceId, Time};
use serde::{Deserialize, Serialize};

use crate::event::{EventDetail, EventRecord};
use crate::prior::Prior;
use crate::registry::Switches;

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

/// Hierarchically shrunk latency quantile estimator keyed by `(place, medium)` and `medium` (`A-08`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct PatienceEstimator {
    pub by_place_medium: BTreeMap<(PlaceId, ChannelId), LatencySketch>,
    pub by_medium: BTreeMap<ChannelId, LatencySketch>,
}

impl PatienceEstimator {
    /// Records an observed (uncensored) third-party reply latency for `(place, medium)`.
    pub fn observe_reply(
        &mut self,
        place: &PlaceId,
        medium: &ChannelId,
        latency_ms: u64,
        rho: f64,
    ) {
        self.by_place_medium
            .entry((place.clone(), medium.clone()))
            .or_default()
            .observe(latency_ms, true, rho);
        self.by_medium
            .entry(medium.clone())
            .or_default()
            .observe(latency_ms, true, rho);
    }

    /// Records a right-censored wait window expiration (`T > wait_ms`) for `(place, medium)`.
    pub fn observe_censored(
        &mut self,
        place: &PlaceId,
        medium: &ChannelId,
        wait_ms: u64,
        rho: f64,
    ) {
        self.by_place_medium
            .entry((place.clone(), medium.clone()))
            .or_default()
            .observe(wait_ms, false, rho);
        self.by_medium
            .entry(medium.clone())
            .or_default()
            .observe(wait_ms, false, rho);
    }

    /// Estimates the shrunk `q_0.10` patience duration in milliseconds, clamped to `[t_min, t_max]`.
    ///
    /// Returns exact `0` when `!switches.patience` (Row 8 neutral value).
    #[must_use]
    pub fn estimate_patience_ms(
        &self,
        place: &PlaceId,
        medium: &ChannelId,
        prior: &Prior,
        switches: &Switches,
    ) -> u64 {
        if !switches.patience {
            return 0;
        }

        let ln_l_typ = (prior.patience_default_ms.max(1) as f64).ln();
        let sigma_ln = prior.patience_sigma_ln.max(1e-3);
        let n0_global = prior.patience_prior_n0.max(0.0);
        let n0_place = prior.patience_place_n0.max(0.0);
        let q = prior.patience_quantile.clamp(0.01, 0.99);

        let (w_med, cdf_med) = self
            .by_medium
            .get(medium)
            .map_or((0.0, Vec::new()), LatencySketch::kaplan_meier_cdf);
        let (w_pm, cdf_pm) = self
            .by_place_medium
            .get(&(place.clone(), medium.clone()))
            .map_or((0.0, Vec::new()), LatencySketch::kaplan_meier_cdf);

        let mut chosen_bucket = PATIENCE_NUM_BUCKETS - 1;
        for k in 0..PATIENCE_NUM_BUCKETS {
            let u_k = bucket_upper_ms(k as u16);
            let f0 = normal_cdf((u_k.max(1.0).ln() - ln_l_typ) / sigma_ln);

            let f_med = if w_med > 0.0 && (n0_global + w_med) > 0.0 {
                (n0_global * f0 + w_med * cdf_med[k]) / (n0_global + w_med)
            } else {
                f0
            };

            let f_pm = if w_pm > 0.0 && (n0_place + w_pm) > 0.0 {
                (n0_place * f_med + w_pm * cdf_pm[k]) / (n0_place + w_pm)
            } else {
                f_med
            };

            if f_pm >= q {
                chosen_bucket = k;
                break;
            }
        }

        let raw_ms = bucket_midpoint_ms(chosen_bucket as u16).round().max(0.0) as u64;
        raw_ms.clamp(prior.patience_t_min_ms, prior.patience_t_max_ms)
    }
}

/// Active `wait` expectation opened on an unallocated `Go` decision (`A-08`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenWait {
    pub expectation_id: EventId,
    pub target_event_id: EventId,
    pub place: PlaceId,
    pub sender: ActorId,
    pub medium: ChannelId,
    pub audience: Audience,
    pub opened_at_ms: Time,
    pub due_ms: Time,
}

impl OpenWait {
    /// Returns `true` if `percept` is a third-party answer to this open `wait` within the window.
    #[must_use]
    pub fn matches_third_party_reply(&self, percept: &EventRecord, soul_id: &ActorId) -> bool {
        if percept.place != self.place {
            return false;
        }
        if &percept.actor == soul_id || percept.actor == self.sender {
            return false;
        }
        if percept.timestamp_ms > self.due_ms {
            return false;
        }
        match &percept.detail {
            Some(EventDetail::PerceptMeta {
                addressees,
                reply_to,
                ..
            }) => {
                *reply_to == Some(self.target_event_id)
                    || addressees.contains(&self.sender)
                    || (reply_to.is_none() && addressees.is_empty())
            }
            _ => percept.caused_by.contains(&self.target_event_id) || percept.caused_by.is_empty(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{LatencySketch, PatienceEstimator, normal_cdf};
    use crate::prior::Prior;
    use crate::registry::Switches;
    use monoagens_protocol::{ChannelId, PlaceId};

    #[test]
    fn normal_cdf_symmetry_and_bounds() {
        assert!((normal_cdf(0.0) - 0.5).abs() < 1e-6);
        assert!((normal_cdf(-1.28155) - 0.10).abs() < 1e-4);
        assert!((normal_cdf(1.28155) - 0.90).abs() < 1e-4);
    }

    #[test]
    fn disabled_patience_is_exact_zero_and_censoring_raises_quantile() {
        let prior = Prior::default();
        let mut sw = Switches::all_enabled();
        sw.patience = false;
        let est = PatienceEstimator::default();
        let place = PlaceId::new("room:1");
        let med = ChannelId::new("chat");
        assert_eq!(est.estimate_patience_ms(&place, &med, &prior, &sw), 0);

        // Uncensored fast replies pull q_0.10 down; censored long waits pull it up.
        let mut fast = PatienceEstimator::default();
        let mut censored = PatienceEstimator::default();
        for _ in 0..15 {
            fast.observe_reply(&place, &med, 800, 1.0);
            censored.observe_censored(&place, &med, 30_000, 1.0);
        }
        let sw_on = Switches::all_enabled();
        let t_fast = fast.estimate_patience_ms(&place, &med, &prior, &sw_on);
        let t_cens = censored.estimate_patience_ms(&place, &med, &prior, &sw_on);
        assert!(t_fast >= prior.patience_t_min_ms && t_fast <= prior.patience_t_max_ms);
        assert!(
            t_cens >= t_fast,
            "censored {t_cens} should be >= fast {t_fast}"
        );

        let _ = LatencySketch::default();
    }
}
