//! `[0, 1]`-normalized `EM-LLM` event boundary segmentation with Empirical-Bayes cold-start shrinkage (`A-06`, `R-23`).

use std::collections::BTreeSet;

use monoagens_protocol::{ActorId, TaggedEmbedding, Time};
use serde::{Deserialize, Serialize};

use crate::gate::cosine_similarity;
use crate::prior::Prior;
use crate::registry::Switches;

/// Causal per-place boundary detection window (`A-06`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct PlaceBoundaryState {
    /// Causal history of prior normalized surprise scores `s_t` (max length `w`).
    pub surprise_history: Vec<f64>,
    /// Causal history of prior log-time gaps `u_t = ln(dt_s + 1.0)` (max length `w`).
    pub log_gap_history: Vec<f64>,
    /// Timestamp of the last event seen in this place.
    pub last_event_ms: Option<Time>,
    /// Participants of the last event seen in this place (for vectorless Jaccard fallback).
    pub last_participants: BTreeSet<ActorId>,
}

/// Breakdown of one event's `[0, 1]` normalized surprise evaluation (`A-06`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BoundaryEvaluation {
    pub pe_time: f64,
    pub pe_content: f64,
    pub novelty: f64,
    pub s_t: f64,
    pub threshold_t: f64,
    pub i_surprise: f64,
    pub is_boundary: bool,
}

/// Inputs describing the incoming event and the current episode at that place.
pub struct BoundaryInputs<'a> {
    pub timestamp_ms: Time,
    pub event_embeddings: &'a [TaggedEmbedding],
    pub episode_centroid: Option<&'a TaggedEmbedding>,
    pub episode_len: usize,
    pub event_participants: &'a BTreeSet<ActorId>,
    pub episode_participants: &'a BTreeSet<ActorId>,
    pub resolved_response_embeddings: Option<(&'a [TaggedEmbedding], &'a [TaggedEmbedding])>,
    pub missed_expectation: bool,
}

impl PlaceBoundaryState {
    /// Returns an estimate of the 95th percentile inter-event gap in ms from `log_gap_history`.
    #[must_use]
    pub fn p95_gap_ms(&self) -> Option<u64> {
        if self.log_gap_history.len() < 4 {
            return None;
        }
        let mut sorted = self.log_gap_history.clone();
        sorted.sort_by(f64::total_cmp);
        let idx = ((sorted.len() as f64) * 0.95).floor() as usize;
        let idx = idx.min(sorted.len() - 1);
        let dt_s = (sorted[idx].exp() - 1.0).max(0.0);
        Some((dt_s * 1000.0).round() as u64)
    }

    /// Evaluates `B(e | E)` without mutating `self` (used both before appending and on `provisional` reopen).
    #[must_use]
    pub fn evaluate_only(
        &self,
        inputs: &BoundaryInputs<'_>,
        prior: &Prior,
        switches: &Switches,
    ) -> BoundaryEvaluation {
        // Neutral ablation (`switches.event_boundaries == false`): fixed-count splits only.
        if !switches.event_boundaries {
            let is_boundary = inputs.episode_len >= prior.em_fixed_interval.max(1);
            return BoundaryEvaluation {
                pe_time: 0.0,
                pe_content: 0.0,
                novelty: 0.0,
                s_t: 0.0,
                threshold_t: 1.0,
                i_surprise: 0.0,
                is_boundary,
            };
        }

        // 1. `novelty_t = clip01(1 - cos(v_t, c_ep))`
        let has_vec = !inputs.event_embeddings.is_empty();
        let novelty = if let (true, Some(ep_c)) = (has_vec, inputs.episode_centroid) {
            let best_cos = inputs
                .event_embeddings
                .iter()
                .filter(|e| e.tag == ep_c.tag)
                .filter_map(|e| cosine_similarity(&e.values, &ep_c.values))
                .fold(None, |acc: Option<f64>, c| {
                    Some(acc.map_or(c, |p| p.max(c)))
                });
            best_cos.map_or(0.0, |c| (1.0 - c).clamp(0.0, 1.0))
        } else {
            0.0
        };

        // 2. `PE_content,t = clip01(1 - max(cos(v_t, act), cos(v_t, expected)))` or `1.0` on miss.
        let pe_content = if inputs.missed_expectation {
            1.0
        } else if let Some((act_embs, exp_embs)) = inputs.resolved_response_embeddings {
            let mut best_cos: Option<f64> = None;
            for v in inputs.event_embeddings {
                for a in act_embs {
                    if v.tag == a.tag
                        && let Some(c) = cosine_similarity(&v.values, &a.values)
                    {
                        best_cos = Some(best_cos.map_or(c, |p| p.max(c)));
                    }
                }
                for e in exp_embs {
                    if v.tag == e.tag
                        && let Some(c) = cosine_similarity(&v.values, &e.values)
                    {
                        best_cos = Some(best_cos.map_or(c, |p| p.max(c)));
                    }
                }
            }
            best_cos.map_or(0.0, |c| (1.0 - c).clamp(0.0, 1.0))
        } else {
            0.0
        };

        // 3. `PE_time,t = clip01(z_time^+ / z_max)` over causal log-gaps `u_t = ln(dt_s + 1)`.
        let mut gap_exceeds_p95 = false;
        let pe_time = if let Some(prev_ms) = self.last_event_ms
            && self.log_gap_history.len() >= 2
        {
            let dt_s = (inputs.timestamp_ms.saturating_sub_ms(prev_ms) as f64) / 1000.0;
            let u_t = (dt_s + 1.0).ln();
            let (mu_u, sigma_u) = mean_and_std(&self.log_gap_history);
            let z_pos = ((u_t - mu_u) / sigma_u.max(prior.em_boundary_sigma_u_floor)).max(0.0);
            if let Some(p95) = self.p95_gap_ms()
                && inputs.timestamp_ms.saturating_sub_ms(prev_ms) > p95
            {
                gap_exceeds_p95 = true;
            }
            (z_pos / prior.em_boundary_z_max.max(1e-6)).clamp(0.0, 1.0)
        } else {
            0.0
        };

        let s_t = pe_time.max(pe_content).max(novelty).clamp(0.0, 1.0);

        // 4. Causal Empirical-Bayes shrinkage moments over prior `H_t`.
        let (mu_eff, sigma_eff) = empirical_bayes_moments(
            &self.surprise_history,
            prior.em_boundary_mu0,
            prior.em_boundary_sigma0,
            prior.em_boundary_m0,
        );
        let threshold_t = mu_eff + prior.em_boundary_gamma * sigma_eff;

        let z_enc = ((s_t - mu_eff) / sigma_eff.max(1e-6)).max(0.0);
        let i_surprise = (z_enc / prior.em_boundary_z_max.max(1e-6)).clamp(0.0, 1.0);

        let n = self.surprise_history.len();
        let ep_len = inputs.episode_len;
        let hard_cap = ep_len >= prior.n_ep_max.max(1);

        let is_boundary = if hard_cap {
            true
        } else if has_vec && inputs.episode_centroid.is_some() {
            n >= prior.em_boundary_warmup && ep_len >= prior.n_ep_min && s_t > threshold_t
        } else {
            // Vectorless fallback (`R-23`): Jaccard < 0.5, gap > p95, or prediction miss.
            let jac = participant_jaccard(inputs.event_participants, inputs.episode_participants);
            ep_len >= prior.n_ep_min && (jac < 0.5 || gap_exceeds_p95 || inputs.missed_expectation)
        };

        BoundaryEvaluation {
            pe_time,
            pe_content,
            novelty,
            s_t,
            threshold_t,
            i_surprise,
            is_boundary,
        }
    }

    /// Evaluates `B(e | E)` and updates causal `H_t` and `log_gap_history` windows (`A-06`).
    pub fn evaluate_and_record(
        &mut self,
        inputs: &BoundaryInputs<'_>,
        prior: &Prior,
        switches: &Switches,
    ) -> BoundaryEvaluation {
        let eval = self.evaluate_only(inputs, prior, switches);
        let w = prior.em_boundary_window.max(1);

        if let Some(prev_ms) = self.last_event_ms {
            let dt_s = (inputs.timestamp_ms.saturating_sub_ms(prev_ms) as f64) / 1000.0;
            let u_t = (dt_s + 1.0).ln();
            self.log_gap_history.push(u_t);
            if self.log_gap_history.len() > w {
                let drain = self.log_gap_history.len() - w;
                self.log_gap_history.drain(0..drain);
            }
        }
        self.last_event_ms = Some(
            self.last_event_ms
                .map_or(inputs.timestamp_ms, |p| p.max(inputs.timestamp_ms)),
        );
        self.last_participants = inputs.event_participants.clone();

        if switches.event_boundaries {
            self.surprise_history.push(eval.s_t);
            if self.surprise_history.len() > w {
                let drain = self.surprise_history.len() - w;
                self.surprise_history.drain(0..drain);
            }
        }

        eval
    }
}

/// Empirical-Bayes shrunk `(mu_eff, sigma_eff)` over `history` (`A-06`).
#[must_use]
pub fn empirical_bayes_moments(history: &[f64], mu_0: f64, sigma_0: f64, m_0: f64) -> (f64, f64) {
    let n = history.len() as f64;
    if history.is_empty() {
        return (mu_0, sigma_0.max(1e-6));
    }
    let mu_n = history.iter().sum::<f64>() / n;
    let v_n = if history.len() >= 2 {
        history
            .iter()
            .map(|&x| (x - mu_n) * (x - mu_n))
            .sum::<f64>()
            / (n - 1.0)
    } else {
        0.0
    };
    let denom = (n + m_0).max(1e-9);
    let mu_eff = (n * mu_n + m_0 * mu_0) / denom;
    let var_num = (n - 1.0).max(0.0) * v_n
        + n * (mu_n - mu_eff).powi(2)
        + m_0 * (sigma_0.powi(2) + (mu_0 - mu_eff).powi(2));
    let sigma_eff = (var_num / denom).max(0.0).sqrt().max(1e-6);
    (mu_eff, sigma_eff)
}

/// Computes Jaccard overlap `|A ∩ B| / |A ∪ B|` (`1.0` if both empty).
#[must_use]
pub fn participant_jaccard(a: &BTreeSet<ActorId>, b: &BTreeSet<ActorId>) -> f64 {
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    let inter = a.intersection(b).count() as f64;
    let union = a.union(b).count() as f64;
    if union <= 0.0 { 1.0 } else { inter / union }
}

fn mean_and_std(xs: &[f64]) -> (f64, f64) {
    if xs.is_empty() {
        return (0.0, 0.0);
    }
    let n = xs.len() as f64;
    let mean = xs.iter().sum::<f64>() / n;
    if xs.len() < 2 {
        return (mean, 0.0);
    }
    let var = xs.iter().map(|&x| (x - mean).powi(2)).sum::<f64>() / (n - 1.0);
    (mean, var.max(0.0).sqrt())
}
