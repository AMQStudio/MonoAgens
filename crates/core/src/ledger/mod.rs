//! Epistemic perception ledger (`R-25`), four-pillar `self_card` (`R-35`),
//! 2x5 metacognitive calibration, and deterministic external-door recalibration.

use std::collections::{BTreeMap, BTreeSet};

use monoagens_protocol::{
    ActorId, CapabilityId, EventId, ExpectKind, PlaceId, ReceiptOutcome, TerminationReason, Time,
};
use serde::{Deserialize, Serialize};

use crate::context::{CoreSelfCard, SourceLedgerCard};
use crate::expectations::{ExpectationBook, is_terminal_state};
use crate::prior::Prior;
use crate::registry::Switches;

/// Summary of an item witnessed with a specific actor (`R-25`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WitnessedFactSummary {
    pub event_id: EventId,
    pub place: PlaceId,
    pub timestamp_ms: Time,
}

/// Open request or commitment tracked on a participant's ledger card (`R-25`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenRequestItem {
    pub expectation_id: EventId,
    pub kind: ExpectKind,
    pub due_ms: Time,
}

/// Epistemic `told` item awaiting verification or corroboration (`R-25`, `R-41`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToldLedgerItem {
    pub belief_id: EventId,
    pub predicate: String,
    pub timestamp_ms: Time,
    pub corroborated: bool,
    pub refuted: bool,
}

/// Refuted claim / false-belief flag attributed to an actor (`R-25`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FalseBeliefFlag {
    pub belief_id: EventId,
    pub predicate: String,
    pub flagged_at_ms: Time,
}

/// Epistemic ledger card for a single participant (`R-25`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActorCard {
    pub actor_id: ActorId,
    pub reliability_alpha: f64,
    pub reliability_beta: f64,
    pub trusted_origin: bool,
    pub witnessed: Vec<WitnessedFactSummary>,
    pub open_requests: Vec<OpenRequestItem>,
    pub told_items: Vec<ToldLedgerItem>,
    pub false_beliefs: Vec<FalseBeliefFlag>,
}

impl ActorCard {
    #[must_use]
    pub fn new(actor_id: ActorId, alpha_0: f64, beta_0: f64, trusted_origin: bool) -> Self {
        Self {
            actor_id,
            reliability_alpha: alpha_0.max(0.01),
            reliability_beta: beta_0.max(0.01),
            trusted_origin,
            witnessed: Vec::new(),
            open_requests: Vec::new(),
            told_items: Vec::new(),
            false_beliefs: Vec::new(),
        }
    }

    #[must_use]
    pub fn reliability_mean(&self) -> f64 {
        let denom = self.reliability_alpha + self.reliability_beta;
        if denom > 0.0 && denom.is_finite() {
            (self.reliability_alpha / denom).clamp(0.0, 1.0)
        } else {
            0.5
        }
    }

    fn trim(&mut self, max_items: usize) {
        let cap = max_items.max(1);
        if self.witnessed.len() > cap {
            let drain = self.witnessed.len() - cap;
            self.witnessed.drain(0..drain);
        }
        if self.open_requests.len() > cap {
            let drain = self.open_requests.len() - cap;
            self.open_requests.drain(0..drain);
        }
        if self.told_items.len() > cap {
            let drain = self.told_items.len() - cap;
            self.told_items.drain(0..drain);
        }
        if self.false_beliefs.len() > cap {
            let drain = self.false_beliefs.len() - cap;
            self.false_beliefs.drain(0..drain);
        }
    }
}

/// Per-capability delivery & failure health statistics (`R-35` Pillar 1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CapabilityHealthStats {
    pub capability_id: CapabilityId,
    pub trials: u64,
    pub successes: u64,
    pub failures: u64,
    pub consec_fail: u32,
    pub alpha: f64,
    pub beta: f64,
    pub degraded: bool,
    pub latency_sum_ms: u128,
    pub latency_samples: u64,
    pub last_updated_ms: Time,
}

impl CapabilityHealthStats {
    #[must_use]
    pub fn new(capability_id: CapabilityId) -> Self {
        Self {
            capability_id,
            trials: 0,
            successes: 0,
            failures: 0,
            consec_fail: 0,
            alpha: 1.0,
            beta: 1.0,
            degraded: false,
            latency_sum_ms: 0,
            latency_samples: 0,
            last_updated_ms: Time::from_millis(0),
        }
    }

    /// Evaluates conservative lower reliability bound `mean - z * std`.
    #[must_use]
    pub fn conservative_lower_bound(&self, z: f64) -> f64 {
        let n = self.alpha + self.beta;
        if n <= 0.0 || !n.is_finite() {
            return 0.5;
        }
        let mean = self.alpha / n;
        let var = (self.alpha * self.beta) / (n * n * (n + 1.0));
        let sd = var.max(0.0).sqrt();
        (mean - z.max(0.0) * sd).clamp(0.0, 1.0)
    }
}

/// 2x5 confidence calibration histogram (`[0..=20]` quantized into 5 bins of width 4-5) (`R-35` Pillar 2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct SelfCalibrationStats {
    /// Counts per bin `[0..=4, 5..=8, 9..=12, 13..=16, 17..=20]` for incorrect outcomes (`y=0`).
    pub bin_incorrect: [u64; 5],
    /// Counts per bin for correct outcomes (`y=1`).
    pub bin_correct: [u64; 5],
    /// Raw sum of `(confidence_prob - y)^2`.
    pub brier_sum: f64,
    /// Sum of `(confidence_prob - y)` for signed calibration bias estimation.
    pub signed_error_sum: f64,
    pub samples: u64,
}

impl SelfCalibrationStats {
    #[must_use]
    pub const fn bin_for(conf_0_20: u8) -> usize {
        let c = if conf_0_20 > 20 { 20 } else { conf_0_20 };
        match c {
            0..=4 => 0,
            5..=8 => 1,
            9..=12 => 2,
            13..=16 => 3,
            _ => 4,
        }
    }

    pub fn record(&mut self, conf_0_20: u8, correct: bool) {
        let clamped = conf_0_20.min(20);
        let bin = Self::bin_for(clamped);
        let prob = (clamped as f64) / 20.0;
        let y = if correct { 1.0 } else { 0.0 };
        if correct {
            self.bin_correct[bin] = self.bin_correct[bin].saturating_add(1);
        } else {
            self.bin_incorrect[bin] = self.bin_incorrect[bin].saturating_add(1);
        }
        let err = prob - y;
        self.brier_sum += err * err;
        self.signed_error_sum += err;
        self.samples = self.samples.saturating_add(1);
    }

    #[must_use]
    pub fn brier(&self) -> f64 {
        if self.samples == 0 {
            0.0
        } else {
            self.brier_sum / (self.samples as f64)
        }
    }

    /// Expected Calibration Error across the 5 confidence bins.
    #[must_use]
    pub fn ece(&self) -> f64 {
        if self.samples == 0 {
            return 0.0;
        }
        const BIN_MIDPOINTS: [f64; 5] = [0.1, 0.325, 0.525, 0.725, 0.925];
        let total = self.samples as f64;
        let mut ece = 0.0;
        for (i, &mid) in BIN_MIDPOINTS.iter().enumerate() {
            let c = self.bin_correct[i] as f64;
            let w = self.bin_incorrect[i] as f64;
            let n = c + w;
            if n > 0.0 {
                let acc = c / n;
                ece += (n / total) * (acc - mid).abs();
            }
        }
        ece.clamp(0.0, 1.0)
    }

    /// Empirical Type-2 AUROC computed across the 5 binned confidence levels.
    #[must_use]
    pub fn auroc_2(&self) -> f64 {
        let total_pos: u64 = self.bin_correct.iter().sum();
        let total_neg: u64 = self.bin_incorrect.iter().sum();
        if total_pos == 0 || total_neg == 0 {
            return 0.5;
        }
        let mut concordant = 0.0;
        let mut ties = 0.0;
        for (b_pos, &pos_count) in self.bin_correct.iter().enumerate() {
            let pos = pos_count as f64;
            if pos == 0.0 {
                continue;
            }
            for (b_neg, &neg_count) in self.bin_incorrect.iter().enumerate() {
                let neg = neg_count as f64;
                if neg == 0.0 {
                    continue;
                }
                if b_pos > b_neg {
                    concordant += pos * neg;
                } else if b_pos == b_neg {
                    ties += pos * neg;
                }
            }
        }
        let denom = (total_pos as f64) * (total_neg as f64);
        ((concordant + 0.5 * ties) / denom).clamp(0.0, 1.0)
    }

    /// Shrinkage-regularized `meta-d'` sensitivity proxy derived from `auroc_2`.
    #[must_use]
    pub fn meta_d_prime(&self, cal_n0: f64) -> f64 {
        let weight = (self.samples as f64) / ((self.samples as f64) + cal_n0.max(0.5));
        let auc = self.auroc_2().clamp(0.02, 0.98);
        // Smooth monotone transform of shrunk AUROC centered at 0.5 (`0.0` at chance).
        let centered = ((auc / (1.0 - auc)).ln()) * std::f64::consts::SQRT_2;
        centered * weight
    }

    /// Shrunk calibration bias applied to recalibrate reported confidence (`R-35`).
    #[must_use]
    pub fn shrunk_bias(&self, prior: &Prior) -> f64 {
        let denom = (self.samples as f64) + prior.self_card_cal_n0.max(0.5);
        let raw_bias = if denom > 0.0 {
            self.signed_error_sum / denom
        } else {
            0.0
        };
        let cap = prior.self_card_kappa_bias.max(0.0);
        raw_bias.clamp(-cap, cap)
    }

    /// Applies shrunk calibration adjustment to raw `[0, 20]` confidence.
    #[must_use]
    pub fn recalibrated_confidence(&self, raw_conf_0_20: u8, prior: &Prior) -> f64 {
        let raw_prob = (raw_conf_0_20.min(20) as f64) / 20.0;
        let bias = self.shrunk_bias(prior);
        (raw_prob - bias).clamp(0.0, 1.0)
    }
}

/// Audit verdict or principle consistency record (`R-35` Pillar 4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrincipleRecordSummary {
    pub event_id: EventId,
    pub consistent: bool,
    pub timestamp_ms: Time,
}

/// Reasons a deterministic external-door recalibration check is triggered (`15.3`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecalibrationTrigger {
    pub capability_status_changed: Vec<CapabilityId>,
    pub budget_critical: bool,
    pub calibration_shifted: bool,
    pub contradiction_delta: u64,
}

impl RecalibrationTrigger {
    #[must_use]
    pub fn should_recalibrate(&self) -> bool {
        !self.capability_status_changed.is_empty()
            || self.budget_critical
            || self.calibration_shifted
            || self.contradiction_delta > 0
    }
}

/// Complete Perception Ledger & Four-Pillar Self-Card projection rebuilt from events (`15.1`..`15.3`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct PerceptionLedger {
    pub actors: BTreeMap<ActorId, ActorCard>,
    pub capabilities: BTreeMap<CapabilityId, CapabilityHealthStats>,
    pub calibration: SelfCalibrationStats,
    pub principle_alpha: f64,
    pub principle_beta: f64,
    pub contradiction_flags: u64,
    pub daily_budget_used: f64,
    pub daily_budget_limit: f64,
    pub recalibration_count: u64,
    pub last_recalibration_ms: Option<Time>,
    pub last_capability_schema_epoch: u64,
}

impl PerceptionLedger {
    #[must_use]
    pub fn new() -> Self {
        Self {
            actors: BTreeMap::new(),
            capabilities: BTreeMap::new(),
            calibration: SelfCalibrationStats::default(),
            principle_alpha: 1.0,
            principle_beta: 1.0,
            contradiction_flags: 0,
            daily_budget_used: 0.0,
            daily_budget_limit: 0.0,
            recalibration_count: 0,
            last_recalibration_ms: None,
            last_capability_schema_epoch: 0,
        }
    }

    /// Initializes or updates an actor's base reliability prior from `ActorFacts`.
    pub fn set_actor_facts(
        &mut self,
        actor_id: ActorId,
        alpha_0: f64,
        beta_0: f64,
        trusted_origin: bool,
    ) {
        let entry = self
            .actors
            .entry(actor_id.clone())
            .or_insert_with(|| ActorCard::new(actor_id, alpha_0, beta_0, trusted_origin));
        if entry.told_items.is_empty() && entry.false_beliefs.is_empty() {
            entry.reliability_alpha = alpha_0.max(0.01);
            entry.reliability_beta = beta_0.max(0.01);
        }
        entry.trusted_origin = trusted_origin;
    }

    /// Records that `actor_id` witnessed an event at `place` (`15.1`).
    pub fn record_witnessed(
        &mut self,
        actor_id: &ActorId,
        event_id: EventId,
        place: PlaceId,
        timestamp_ms: Time,
        prior: &Prior,
    ) {
        let card = self
            .actors
            .entry(actor_id.clone())
            .or_insert_with(|| ActorCard::new(actor_id.clone(), 1.0, 1.0, false));
        if !card.witnessed.iter().any(|w| w.event_id == event_id) {
            card.witnessed.push(WitnessedFactSummary {
                event_id,
                place,
                timestamp_ms,
            });
            card.trim(prior.ledger_max_items_per_card);
        }
    }

    /// Records a `told` claim attributed to `source_actor`.
    pub fn record_told_claim(
        &mut self,
        source_actor: &ActorId,
        belief_id: EventId,
        predicate: String,
        timestamp_ms: Time,
        prior: &Prior,
    ) {
        let card = self
            .actors
            .entry(source_actor.clone())
            .or_insert_with(|| ActorCard::new(source_actor.clone(), 1.0, 1.0, false));
        if !card.told_items.iter().any(|t| t.belief_id == belief_id) {
            card.told_items.push(ToldLedgerItem {
                belief_id,
                predicate,
                timestamp_ms,
                corroborated: false,
                refuted: false,
            });
            card.trim(prior.ledger_max_items_per_card);
        }
    }

    /// Records corroborated verification or refutation of a `told` claim, updating source reliability
    /// and flagging false beliefs when refuted.
    pub fn resolve_told_claim(
        &mut self,
        source_actor: &ActorId,
        belief_id: EventId,
        predicate: &str,
        supported: bool,
        timestamp_ms: Time,
        prior: &Prior,
    ) {
        let card = self
            .actors
            .entry(source_actor.clone())
            .or_insert_with(|| ActorCard::new(source_actor.clone(), 1.0, 1.0, false));
        if supported {
            card.reliability_alpha += 1.0;
        } else {
            card.reliability_beta += 1.0;
            if !card.false_beliefs.iter().any(|f| f.belief_id == belief_id) {
                card.false_beliefs.push(FalseBeliefFlag {
                    belief_id,
                    predicate: predicate.to_owned(),
                    flagged_at_ms: timestamp_ms,
                });
            }
        }
        for item in &mut card.told_items {
            if item.belief_id == belief_id {
                if supported {
                    item.corroborated = true;
                } else {
                    item.refuted = true;
                }
            }
        }
        card.trim(prior.ledger_max_items_per_card);
    }

    /// Synchronizes open request/promise counts per participant from the active [`ExpectationBook`] (`15.1`).
    pub fn sync_open_requests(&mut self, book: &ExpectationBook, prior: &Prior) {
        for card in self.actors.values_mut() {
            card.open_requests.clear();
        }
        for exp in book.expectations.values() {
            if is_terminal_state(exp.state) {
                continue;
            }
            let item = OpenRequestItem {
                expectation_id: exp.expectation_id,
                kind: exp.kind,
                due_ms: exp.due_ms,
            };
            let mut linked: BTreeSet<ActorId> = BTreeSet::new();
            for p in &exp.people {
                linked.insert(p.clone());
            }
            if let Some(meta) = &exp.promise_meta {
                for p in &meta.promisee_ids {
                    linked.insert(p.clone());
                }
            }
            for actor in linked {
                let card = self
                    .actors
                    .entry(actor.clone())
                    .or_insert_with(|| ActorCard::new(actor, 1.0, 1.0, false));
                card.open_requests.push(item.clone());
                card.trim(prior.ledger_max_items_per_card);
            }
        }
    }

    /// Records a capability receipt outcome into Pillar 1 health stats and checks for degradation change (`15.2`, `15.3`).
    ///
    /// `CanceledByCore` and `PreemptedExternal` NEVER penalize capability health.
    pub fn record_capability_receipt(
        &mut self,
        capability_id: CapabilityId,
        outcome: ReceiptOutcome,
        termination_reason: Option<TerminationReason>,
        latency_ms: Option<u64>,
        now_ms: Time,
        prior: &Prior,
    ) -> bool {
        if matches!(
            termination_reason,
            Some(TerminationReason::CanceledByCore | TerminationReason::PreemptedExternal)
        ) {
            return false;
        }
        if matches!(outcome, ReceiptOutcome::Held | ReceiptOutcome::Unknown) {
            return false;
        }
        let stat = self
            .capabilities
            .entry(capability_id.clone())
            .or_insert_with(|| CapabilityHealthStats::new(capability_id));
        let prev_degraded = stat.degraded;
        stat.trials = stat.trials.saturating_add(1);
        let succeeded = matches!(outcome, ReceiptOutcome::Done);
        if succeeded {
            stat.successes = stat.successes.saturating_add(1);
            stat.consec_fail = 0;
            stat.alpha += 1.0;
        } else {
            stat.failures = stat.failures.saturating_add(1);
            stat.consec_fail = stat.consec_fail.saturating_add(1);
            stat.beta += 1.0;
        }
        if let Some(lat) = latency_ms {
            stat.latency_sum_ms = stat.latency_sum_ms.saturating_add(u128::from(lat));
            stat.latency_samples = stat.latency_samples.saturating_add(1);
        }
        stat.last_updated_ms = now_ms;

        if stat.trials >= (prior.self_card_cap_min_trials as u64) {
            let lcb = stat.conservative_lower_bound(prior.self_card_cap_z);
            stat.degraded = lcb < prior.self_card_cap_deg_threshold
                || stat.consec_fail >= prior.self_card_cap_consec_fail;
        } else {
            stat.degraded = stat.consec_fail >= prior.self_card_cap_consec_fail;
        }

        prev_degraded != stat.degraded
    }

    /// Records a calibration `(conf_0_20, correct)` outcome into Pillar 2.
    pub fn record_calibration_outcome(&mut self, conf_0_20: u8, correct: bool) {
        self.calibration.record(conf_0_20, correct);
    }

    /// Records budget usage into Pillar 3 (`DailyBudget`).
    pub fn update_budget(&mut self, used: f64, limit: f64) -> bool {
        let prev_critical = self.daily_budget_limit > 0.0
            && (self.daily_budget_used / self.daily_budget_limit) >= 0.90;
        self.daily_budget_used = used.max(0.0);
        self.daily_budget_limit = limit.max(0.0);
        let now_critical = self.daily_budget_limit > 0.0
            && (self.daily_budget_used / self.daily_budget_limit) >= 0.90;
        !prev_critical && now_critical
    }

    /// Records a principle consistency verdict into Pillar 4.
    pub fn record_principle_verdict(&mut self, consistent: bool, contradiction: bool) -> bool {
        if consistent {
            self.principle_alpha += 1.0;
        } else {
            self.principle_beta += 1.0;
        }
        if contradiction {
            self.contradiction_flags = self.contradiction_flags.saturating_add(1);
            return true;
        }
        false
    }

    /// Marks deterministic external-door recalibration completion without mutating identity (`15.3`).
    pub fn mark_recalibrated(&mut self, now_ms: Time) {
        self.recalibration_count = self.recalibration_count.saturating_add(1);
        self.last_recalibration_ms = Some(now_ms);
    }

    /// Builds [`CoreSelfCard`] from audited log projections (`15.2`).
    ///
    /// Returns an empty zeroed neutral card when `!switches.perception_ledger`.
    #[must_use]
    pub fn build_self_card(&self, prior: &Prior, switches: &Switches) -> CoreSelfCard {
        if !switches.perception_ledger {
            return CoreSelfCard::default();
        }
        let mut alpha_sum = 1.0;
        let mut beta_sum = 1.0;
        let mut lat_sum: u128 = 0;
        let mut lat_count: u64 = 0;
        let mut degraded_capabilities = Vec::new();
        for (cap_id, stats) in &self.capabilities {
            alpha_sum += (stats.alpha - 1.0).max(0.0);
            beta_sum += (stats.beta - 1.0).max(0.0);
            lat_sum = lat_sum.saturating_add(stats.latency_sum_ms);
            lat_count = lat_count.saturating_add(stats.latency_samples);
            if stats.degraded {
                degraded_capabilities.push(cap_id.clone());
            }
        }
        let mean_delivery_latency_ms = if lat_count == 0 {
            0
        } else {
            (lat_sum / u128::from(lat_count)).min(u128::from(u64::MAX)) as u64
        };
        let budget_headroom_ratio = if self.daily_budget_limit <= 0.0 {
            1.0
        } else {
            (1.0 - (self.daily_budget_used / self.daily_budget_limit)).clamp(0.0, 1.0)
        };
        CoreSelfCard {
            reliability_alpha: alpha_sum,
            reliability_beta: beta_sum,
            mean_delivery_latency_ms,
            calibration_samples: self.calibration.samples,
            calibration_brier: self.calibration.brier(),
            budget_headroom_ratio,
            principle_alpha: self.principle_alpha.max(1.0),
            principle_beta: self.principle_beta.max(1.0),
            contradiction_flags: self.contradiction_flags,
            calibration_ece: self.calibration.ece(),
            calibration_auroc_2: self.calibration.auroc_2(),
            calibration_meta_d_prime: self.calibration.meta_d_prime(prior.self_card_cal_n0),
            calibration_bias: self.calibration.shrunk_bias(prior),
            degraded_capabilities,
        }
    }

    /// Builds compact `source_cards` covering triggering participants, visible scope members,
    /// AND all `people`/`promisee_ids` from active expectations (including promises & goals on ticks) (`15.1`).
    ///
    /// Returns an empty vector when `!switches.perception_ledger`.
    #[must_use]
    pub fn build_source_cards<F>(
        &self,
        participants: &[ActorId],
        expectations: &ExpectationBook,
        soul_id: &ActorId,
        switches: &Switches,
        closeness_fn: F,
    ) -> Vec<SourceLedgerCard>
    where
        F: Fn(&ActorId) -> f64,
    {
        if !switches.perception_ledger {
            return Vec::new();
        }
        let actor_set = collect_source_card_actors(participants, expectations, soul_id);
        let mut out = Vec::with_capacity(actor_set.len());
        for actor in actor_set {
            let (
                alpha,
                beta,
                trusted,
                witnessed_count,
                open_requests_count,
                ungrounded_told_count,
                false_belief_count,
            ) = match self.actors.get(&actor) {
                Some(card) => {
                    let ungrounded = card
                        .told_items
                        .iter()
                        .filter(|t| !t.corroborated && !t.refuted)
                        .count();
                    let open_exp = count_actor_open_expectations(expectations, &actor)
                        .max(card.open_requests.len());
                    (
                        card.reliability_alpha,
                        card.reliability_beta,
                        card.trusted_origin,
                        card.witnessed.len(),
                        open_exp,
                        ungrounded,
                        card.false_beliefs.len(),
                    )
                }
                None => (
                    1.0,
                    1.0,
                    false,
                    0,
                    count_actor_open_expectations(expectations, &actor),
                    0,
                    0,
                ),
            };
            out.push(SourceLedgerCard {
                actor_id: actor.clone(),
                reliability_alpha: alpha,
                reliability_beta: beta,
                closeness: closeness_fn(&actor),
                trusted_origin: trusted,
                witnessed_count,
                open_requests_count,
                ungrounded_told_count,
                false_belief_count,
            });
        }
        out
    }

    /// Erases actor-scoped card state for `hard_forget`.
    pub fn remove_actor(&mut self, actor: &ActorId) {
        self.actors.remove(actor);
    }
}

/// Collects the deterministic set of non-soul participants required in `source_cards` (`15.1`):
/// triggering/scope participants PLUS active expectation `people` and `promise_meta.promisee_ids`.
#[must_use]
pub fn collect_source_card_actors(
    participants: &[ActorId],
    expectations: &ExpectationBook,
    soul_id: &ActorId,
) -> BTreeSet<ActorId> {
    let mut actors = BTreeSet::new();
    for actor in participants {
        if actor != soul_id {
            actors.insert(actor.clone());
        }
    }
    for exp in expectations.expectations.values() {
        if is_terminal_state(exp.state) {
            continue;
        }
        for person in &exp.people {
            if person != soul_id {
                actors.insert(person.clone());
            }
        }
        if let Some(meta) = &exp.promise_meta {
            for promisee in &meta.promisee_ids {
                if promisee != soul_id {
                    actors.insert(promisee.clone());
                }
            }
        }
    }
    actors
}

fn count_actor_open_expectations(expectations: &ExpectationBook, actor: &ActorId) -> usize {
    expectations
        .expectations
        .values()
        .filter(|exp| {
            !is_terminal_state(exp.state)
                && (exp.people.iter().any(|p| p == actor)
                    || exp
                        .promise_meta
                        .as_ref()
                        .is_some_and(|m| m.promisee_ids.iter().any(|p| p == actor)))
        })
        .count()
}
