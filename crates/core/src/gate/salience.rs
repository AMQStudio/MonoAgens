//! Salience assembly `S = clamp(sum w_k * s_k, 0, 1)` with exact reachable neutrals.
//!
//! Every disposition map reaches exact `0.0` when its neutral value is selected or when
//! its mechanism switch (`Switches::salience`, `Switches::floor_cost`) is neutralized,
//! executing on one single code path (`ARCHITECTURE.md`).

use std::collections::BTreeMap;

use monoagens_protocol::{ActorId, ChannelId, PlaceId, Time};
use serde::{Deserialize, Serialize};

use crate::gate::signals::GateSignals;
use crate::prior::Prior;
use crate::registry::Switches;

/// Milliseconds per minute used for `gamma^(dt_minutes)` habituation recovery.
const MS_PER_MINUTE: f64 = 60_000.0;

/// Partition key `(place, source_channel)` for post-evaluation habituation (`A-03`, `R-36`).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct HabituationKey {
    pub place: PlaceId,
    pub source_channel: ChannelId,
}

/// Per-`(place, source_channel)` habituation state `h_{(p, k_src)}` (`A-03`, `R-36`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub struct HabituationEntry {
    pub h: f64,
    pub last_updated_ms: Time,
    pub count: u64,
}

impl HabituationEntry {
    /// Decays stored habituation by `gamma^(dt_minutes)` prior to evaluating `S_eff`.
    ///
    /// Returns `0.0` when no prior stimuli have been recorded (`count == 0`), guaranteeing
    /// stimulus 1 retains full unhabituated salience `S_eff == S`.
    #[must_use]
    pub fn decayed_prior(&self, now_ms: Time, gamma: f64) -> f64 {
        if self.count == 0 || self.h <= 0.0 || !self.h.is_finite() {
            return 0.0;
        }
        let dt_minutes = (now_ms.saturating_sub_ms(self.last_updated_ms) as f64) / MS_PER_MINUTE;
        let gamma_c = if gamma.is_finite() {
            gamma.clamp(0.0, 1.0)
        } else {
            0.9
        };
        (self.h * gamma_c.powf(dt_minutes)).max(0.0)
    }

    /// Post-evaluation increment `h <- h_pre + 1` applied strictly AFTER `S_eff` evaluation (`A-03`).
    pub fn increment_post_eval(&mut self, now_ms: Time, gamma: f64) {
        let h_pre = self.decayed_prior(now_ms, gamma);
        self.h = h_pre + 1.0;
        self.last_updated_ms = now_ms;
        self.count = self.count.saturating_add(1);
    }
}

/// Partitioned habituation book keyed by `(place, source_channel)` (`A-03`, `R-36`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct HabituationBook {
    entries: BTreeMap<HabituationKey, HabituationEntry>,
}

impl HabituationBook {
    /// Derives the canonical source channel id: explicit `ChannelId` when supplied by the body,
    /// falling back to `"actor:<actor_id>"` so distinct speakers remain partitioned.
    #[must_use]
    pub fn source_channel_for(channel: Option<&ChannelId>, actor: &ActorId) -> ChannelId {
        channel
            .cloned()
            .unwrap_or_else(|| ChannelId::new(format!("actor:{}", actor.as_str())))
    }

    /// Reads the decayed pre-evaluation habituation `h_pre` without mutating state.
    #[must_use]
    pub fn h_pre(
        &self,
        place: &PlaceId,
        source_channel: &ChannelId,
        now_ms: Time,
        gamma: f64,
    ) -> f64 {
        let key = HabituationKey {
            place: place.clone(),
            source_channel: source_channel.clone(),
        };
        self.entries
            .get(&key)
            .map_or(0.0, |entry| entry.decayed_prior(now_ms, gamma))
    }

    /// Computes `(habituation_factor, S_eff)` from `S` and `h_pre` (`kappa = 0` is exact identity).
    #[must_use]
    pub fn effective_salience(
        raw_salience: f64,
        h_pre: f64,
        kappa: f64,
        habituation_enabled: bool,
    ) -> (f64, f64) {
        let k = if habituation_enabled && kappa.is_finite() {
            kappa.max(0.0)
        } else {
            0.0
        };
        if k == 0.0 {
            return (1.0, raw_salience);
        }
        let h = if h_pre.is_finite() {
            h_pre.max(0.0)
        } else {
            0.0
        };
        let factor = 1.0 / (1.0 + k * h);
        let s_eff = if raw_salience.is_finite() {
            (raw_salience * factor).clamp(0.0, 1.0)
        } else {
            0.0
        };
        (factor, s_eff)
    }

    /// Increments `h_{(p, k_src)} <- h_pre + 1` strictly after gate evaluation (`A-03`).
    pub fn record_post_eval(
        &mut self,
        place: PlaceId,
        source_channel: ChannelId,
        now_ms: Time,
        gamma: f64,
    ) {
        let key = HabituationKey {
            place,
            source_channel,
        };
        self.entries
            .entry(key)
            .or_default()
            .increment_post_eval(now_ms, gamma);
    }

    /// Removes actor-scoped fallback channels when an actor is erased by `hard_forget`.
    pub fn remove_actor(&mut self, actor: &ActorId) {
        let synthetic = ChannelId::new(format!("actor:{}", actor.as_str()));
        self.entries
            .retain(|key, _| key.source_channel != synthetic);
    }

    /// Returns all stored `(place, source_channel)` habituation entries.
    #[must_use]
    pub fn entries(&self) -> &BTreeMap<HabituationKey, HabituationEntry> {
        &self.entries
    }

    /// Inserts a decoded habituation entry (for snapshot restoration).
    pub fn insert_entry(&mut self, key: HabituationKey, entry: HabituationEntry) {
        self.entries.insert(key, entry);
    }
}

/// Signal weights computed from [`Prior`] and [`Switches`].
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub struct SalienceWeights {
    pub w_own_name: f64,
    pub w_info: f64,
    pub w_fok: f64,
    pub w_rel: f64,
    pub w_commitment: f64,
    pub w_sc: f64,
}

impl SalienceWeights {
    /// Derives exact weights from `prior` and `switches`.
    ///
    /// - `w_info = info_scale * helpfulness` (`0.0` when `helpfulness == 0` or `info_scale == 0`)
    /// - `w_fok = fok_scale * helpfulness` (`0.0` when `helpfulness == 0` or `fok_scale == 0`)
    /// - `w_sc = -restraint_scale * restraint` (`0.0` when `restraint == 0` or `!switches.floor_cost`)
    /// - All weights equal `0.0` when `!switches.salience` (Row 1 neutral).
    #[must_use]
    pub fn from_prior(prior: &Prior, switches: &Switches) -> Self {
        let salience_scale = if switches.salience { 1.0 } else { 0.0 };
        let floor_scale = if switches.salience && switches.floor_cost {
            1.0
        } else {
            0.0
        };

        let raw_sc = -floor_scale * prior.restraint_scale * prior.restraint;
        let w_sc = if raw_sc == 0.0 { 0.0 } else { raw_sc };

        Self {
            w_own_name: salience_scale * prior.own_name_weight,
            w_info: salience_scale * prior.info_scale * prior.helpfulness,
            w_fok: salience_scale * prior.fok_scale * prior.helpfulness,
            w_rel: salience_scale * prior.relevance_weight,
            w_commitment: salience_scale * prior.commitment_weight,
            w_sc,
        }
    }

    /// Evaluates clamped salience `S = clamp(sum w_k * s_k, 0, 1)`.
    #[must_use]
    pub fn evaluate(&self, signals: &GateSignals) -> f64 {
        let raw = self.w_own_name * signals.own_name
            + self.w_info * signals.info_gap
            + self.w_fok * signals.feeling_of_knowing
            + self.w_rel * signals.relevance
            + self.w_commitment * signals.commitment
            + self.w_sc * signals.social_cost;
        if raw.is_finite() {
            raw.clamp(0.0, 1.0)
        } else {
            0.0
        }
    }
}

/// Complete salience evaluation result.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub struct SalienceEvaluation {
    pub weights: SalienceWeights,
    pub raw_salience: f64,
}

/// Computes effective weights and clamped salience `S in [0, 1]`.
#[must_use]
pub fn compute_salience(
    signals: &GateSignals,
    prior: &Prior,
    switches: &Switches,
) -> SalienceEvaluation {
    let weights = SalienceWeights::from_prior(prior, switches);
    let raw_salience = weights.evaluate(signals);
    SalienceEvaluation {
        weights,
        raw_salience,
    }
}
