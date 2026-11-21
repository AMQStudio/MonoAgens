//! Beta-Bernoulli `(alpha, beta)` belief & credibility store, basis verification,
//! `told` source-prior initialization, `k_corrob` corroboration, `R-32` convention discounting,
//! and log-rebuildable `hard_forget` projection recomputation (`R-34`).

use std::collections::{BTreeMap, BTreeSet};

use monoagens_protocol::{ActorId, Audience, DomainId, EventId, PlaceId, Taint, Time};
use serde::{Deserialize, Serialize};

use crate::event::LogSeq;
use crate::recall::{MemoryBasis, MemoryEntry};

/// Default domain identifier when no explicit domain is supplied.
pub const DEFAULT_DOMAIN_ID: &str = "general";

/// Fast-timescale daily relaxation factor for short-horizon preference drift (`0.80/day`).
pub const CREDENCE_LAMBDA_FAST: f64 = 0.80;

const CREDENCE_LAMBDA_DOMAIN: f64 = 0.98;
const CREDENCE_PERIOD_MS: u64 = 86_400_000;
const DEFAULT_K_CORROB: usize = 2;
const CONV_VICARIOUS_DISCOUNT: f64 = 0.25;
const CONV_PEER_FLOOR: f64 = 0.2;
const CONV_ALPHA_0: f64 = 1.0;
const CONV_BETA_0: f64 = 1.0;
const CONV_LAMBDA: f64 = 0.95;
const CONV_TAU: f64 = 4.0;
const CONV_KAPPA: f64 = 1.0;
const CONV_BIAS_MAX: f64 = 0.5;

/// Anchored Beta-Bernoulli credence `(alpha, beta)` with continuous-time relaxation toward `(alpha_0, beta_0)`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BetaCredence {
    pub alpha_0: f64,
    pub beta_0: f64,
    pub alpha: f64,
    pub beta: f64,
    pub last_updated_ms: Time,
}

impl BetaCredence {
    /// Creates a new Beta credence anchored at `(alpha_0, beta_0)`.
    #[must_use]
    pub fn from_prior(alpha_0: f64, beta_0: f64, now_ms: Time) -> Self {
        let a0 = if alpha_0.is_finite() && alpha_0 > 0.0 {
            alpha_0
        } else {
            1.0
        };
        let b0 = if beta_0.is_finite() && beta_0 > 0.0 {
            beta_0
        } else {
            1.0
        };
        Self {
            alpha_0: a0,
            beta_0: b0,
            alpha: a0,
            beta: b0,
            last_updated_ms: now_ms,
        }
    }

    /// Creates a Beta credence initialized at explicit `(alpha, beta)` while anchored at `(alpha_0, beta_0)`.
    #[must_use]
    pub fn with_counts(alpha_0: f64, beta_0: f64, alpha: f64, beta: f64, now_ms: Time) -> Self {
        let mut s = Self::from_prior(alpha_0, beta_0, now_ms);
        if alpha.is_finite() && alpha > 0.0 {
            s.alpha = alpha;
        }
        if beta.is_finite() && beta > 0.0 {
            s.beta = beta;
        }
        s
    }

    /// Posterior expected probability `E = alpha / (alpha + beta)`.
    #[must_use]
    pub fn expected(&self) -> f64 {
        let denom = self.alpha + self.beta;
        if denom <= 0.0 || !denom.is_finite() {
            0.5
        } else {
            (self.alpha / denom).clamp(0.0, 1.0)
        }
    }

    /// Posterior Beta variance `alpha * beta / (n^2 * (n + 1))`.
    #[must_use]
    pub fn variance(&self) -> f64 {
        let n = self.alpha + self.beta;
        let denom = n * n * (n + 1.0);
        if denom <= 0.0 || !denom.is_finite() {
            0.25
        } else {
            (self.alpha * self.beta / denom).max(0.0)
        }
    }

    /// Effective sample size above prior: `(alpha - alpha_0) + (beta - beta_0)`.
    #[must_use]
    pub fn effective_evidence(&self) -> f64 {
        ((self.alpha - self.alpha_0).max(0.0)) + ((self.beta - self.beta_0).max(0.0))
    }

    /// Relaxes `(alpha, beta)` toward `(alpha_0, beta_0)` by `lambda_domain ^ (dt / period)`.
    pub fn decay_toward_prior(&mut self, now_ms: Time, lambda_domain: f64, period_ms: u64) {
        if now_ms <= self.last_updated_ms {
            return;
        }
        let lambda = lambda_domain.clamp(0.0, 1.0);
        if (lambda - 1.0).abs() <= 1e-12 {
            self.last_updated_ms = now_ms;
            return;
        }
        let dt_periods =
            (now_ms.saturating_sub_ms(self.last_updated_ms) as f64) / (period_ms.max(1) as f64);
        let factor = lambda.powf(dt_periods);
        self.alpha = self.alpha_0 + (self.alpha - self.alpha_0) * factor;
        self.beta = self.beta_0 + (self.beta - self.beta_0) * factor;
        self.last_updated_ms = now_ms;
    }

    /// Applies positive/negative evidence after decaying to `now_ms`.
    pub fn apply_evidence(
        &mut self,
        positive: f64,
        negative: f64,
        now_ms: Time,
        lambda_domain: f64,
        period_ms: u64,
    ) {
        self.decay_toward_prior(now_ms, lambda_domain, period_ms);
        if positive.is_finite() && positive > 0.0 {
            self.alpha += positive;
        }
        if negative.is_finite() && negative > 0.0 {
            self.beta += negative;
        }
    }
}

/// Stored belief entry with basis provenance, visibility audience, validity bounds, and credence.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BeliefRecord {
    pub belief_id: EventId,
    pub claim_key: String,
    pub requested_basis: MemoryBasis,
    pub basis: MemoryBasis,
    pub basis_downgraded: bool,
    pub source_actor: Option<ActorId>,
    pub corroborating_actors: BTreeSet<ActorId>,
    pub audience: Audience,
    pub taint: Taint,
    pub domain: DomainId,
    pub source_event_ids: Vec<EventId>,
    pub valid_from_ms: Time,
    pub valid_to_ms: Option<Time>,
    pub replaced_by: Option<EventId>,
    pub credence: Option<BetaCredence>,
    pub is_mind_inference: bool,
    pub erased: bool,
}

impl BeliefRecord {
    /// Returns `true` if this belief is currently valid (not replaced and not erased).
    #[must_use]
    pub const fn is_current(&self) -> bool {
        self.replaced_by.is_none() && self.valid_to_ms.is_none() && !self.erased
    }

    /// Returns `true` if this claim carries untrusted taint or lacks `k_corrob` corroboration.
    #[must_use]
    pub fn is_uncorroborated_stranger_claim(&self, k_corrob: usize) -> bool {
        self.basis == MemoryBasis::Told
            && (self.taint != Taint::Trusted || self.corroborating_actors.len() < k_corrob.max(1))
    }
}

/// Event-sourced log entry stored in `BeliefBook` for deterministic log rebuild and `hard_forget` recomputation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum BeliefLogEntry {
    BeliefRecorded {
        event_id: EventId,
        timestamp_ms: Time,
        seq: LogSeq,
        claim_key: String,
        requested_basis: MemoryBasis,
        basis: MemoryBasis,
        basis_downgraded: bool,
        source_actor: Option<ActorId>,
        #[serde(default)]
        audience: Audience,
        domain: DomainId,
        source_event_ids: Vec<EventId>,
        taint: Taint,
        valid_from_ms: Time,
        valid_to_ms: Option<Time>,
        replaced_by: Option<EventId>,
        alpha_0: f64,
        beta_0: f64,
        is_mind_inference: bool,
    },
    EvidenceApplied {
        event_id: EventId,
        timestamp_ms: Time,
        seq: LogSeq,
        target_id: EventId,
        source_actor: Option<ActorId>,
        domain: DomainId,
        positive: f64,
        negative: f64,
        is_convention: bool,
        place: Option<PlaceId>,
        pattern_id: Option<String>,
    },
}

impl BeliefLogEntry {
    #[must_use]
    pub const fn timestamp_ms(&self) -> Time {
        match self {
            Self::BeliefRecorded { timestamp_ms, .. } | Self::EvidenceApplied { timestamp_ms, .. } => {
                *timestamp_ms
            }
        }
    }

    #[must_use]
    pub const fn seq(&self) -> LogSeq {
        match self {
            Self::BeliefRecorded { seq, .. } | Self::EvidenceApplied { seq, .. } => *seq,
        }
    }

    #[must_use]
    pub const fn event_id(&self) -> EventId {
        match self {
            Self::BeliefRecorded { event_id, .. } | Self::EvidenceApplied { event_id, .. } => {
                *event_id
            }
        }
    }
}

/// Checks reasoning's claimed [`MemoryBasis`] against actual memory provenance.
#[must_use]
pub fn verify_claimed_basis(
    requested: MemoryBasis,
    used_ids: &[EventId],
    memories: &BTreeMap<EventId, MemoryEntry>,
    soul_id: &ActorId,
    is_mind_inference: bool,
) -> (MemoryBasis, bool) {
    if is_mind_inference {
        return (MemoryBasis::Inferred, requested != MemoryBasis::Inferred);
    }
    if requested != MemoryBasis::Observed {
        return (requested, false);
    }

    let mut visited = BTreeSet::new();
    let mut frontier: Vec<EventId> = used_ids.to_vec();
    let mut steps = 0usize;
    while let Some(id) = frontier.pop() {
        if steps >= 128 {
            break;
        }
        steps += 1;
        if !visited.insert(id) {
            continue;
        }
        let Some(entry) = memories.get(&id) else {
            continue;
        };
        if entry.erased {
            continue;
        }
        let witnessed_percept = entry.basis == MemoryBasis::Observed
            || (entry.basis == MemoryBasis::SelfCaused && &entry.actor == soul_id);
        if witnessed_percept {
            return (MemoryBasis::Observed, false);
        }
        for &src in &entry.source_ids {
            if !visited.contains(&src) {
                frontier.push(src);
            }
        }
    }

    (MemoryBasis::Inferred, true)
}

/// Parses a reasoning `basis` string into [`MemoryBasis`].
#[must_use]
pub fn parse_basis_str(raw: &str) -> Option<MemoryBasis> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "observed" => Some(MemoryBasis::Observed),
        "told" => Some(MemoryBasis::Told),
        "inferred" => Some(MemoryBasis::Inferred),
        "self" | "self_caused" => Some(MemoryBasis::SelfCaused),
        _ => None,
    }
}

/// Complete projection of beliefs, `(alpha, beta)` credences, `(source, domain)` reliabilities,
/// fast-timescale preference shadows, domain-isolated active claims (`MEMDRIFT` guard),
/// and discounted place conventions, strictly rebuildable from the event log.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BeliefBook {
    pub beliefs: BTreeMap<EventId, BeliefRecord>,
    pub active_claims: BTreeMap<String, EventId>,
    /// Domain-isolated active claims `(DomainId, claim_key) -> EventId` (`MEMDRIFT` guard).
    #[serde(default)]
    pub active_claims_by_domain: BTreeMap<(DomainId, String), EventId>,
    pub source_reliability: BTreeMap<(ActorId, DomainId), BetaCredence>,
    pub item_credences: BTreeMap<EventId, BetaCredence>,
    /// Fast-timescale shadow `BetaCredence` (`CREDENCE_LAMBDA_FAST = 0.80/day`) per belief/item.
    #[serde(default)]
    pub fast_credences: BTreeMap<EventId, BetaCredence>,
    pub conventions: BTreeMap<(PlaceId, String), BetaCredence>,
    pub actor_priors: BTreeMap<ActorId, (f64, f64)>,
    pub log_entries: Vec<BeliefLogEntry>,
}

impl PartialEq for BeliefBook {
    fn eq(&self, other: &Self) -> bool {
        self.beliefs == other.beliefs
            && self.active_claims == other.active_claims
            && self.source_reliability == other.source_reliability
            && self.item_credences == other.item_credences
            && self.conventions == other.conventions
            && self.actor_priors == other.actor_priors
            && self.log_entries == other.log_entries
    }
}

impl BeliefBook {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the currently active belief ID strictly within `(domain, claim_key)` (`MEMDRIFT` guard).
    #[must_use]
    pub fn active_claim_for_domain(&self, domain: &DomainId, claim_key: &str) -> Option<EventId> {
        if let Some(&id) = self
            .active_claims_by_domain
            .get(&(domain.clone(), claim_key.to_owned()))
        {
            return Some(id);
        }
        self.active_claims
            .get(claim_key)
            .copied()
            .filter(|id| self.beliefs.get(id).is_some_and(|b| &b.domain == domain))
    }

    /// Computes an item's fast-timescale (`lambda_fast = 0.80/day`) Beta credence decayed to `now_ms`.
    #[must_use]
    pub fn fast_credence_at(&self, belief_id: EventId, now_ms: Time) -> Option<BetaCredence> {
        let mut cred = self
            .fast_credences
            .get(&belief_id)
            .copied()
            .or_else(|| self.beliefs.get(&belief_id).and_then(|b| b.credence))
            .or_else(|| self.item_credences.get(&belief_id).copied())?;
        cred.decay_toward_prior(now_ms, CREDENCE_LAMBDA_FAST, CREDENCE_PERIOD_MS);
        Some(cred)
    }

    /// Returns `(fast_expected, slow_expected)` preference credence expectations decayed to `now_ms`.
    #[must_use]
    pub fn effective_preference_at(&self, belief_id: EventId, now_ms: Time) -> Option<(f64, f64)> {
        let slow = self.item_credence_at(belief_id, now_ms)?;
        let fast = self.fast_credence_at(belief_id, now_ms).unwrap_or(slow);
        Some((fast.expected(), slow.expected()))
    }

    /// Registers or updates the gateway-supplied reliability prior `(alpha_0, beta_0)` for `actor`.
    pub fn set_actor_prior(&mut self, actor: ActorId, alpha_0: f64, beta_0: f64) {
        let a0 = if alpha_0.is_finite() && alpha_0 > 0.0 {
            alpha_0
        } else {
            1.0
        };
        let b0 = if beta_0.is_finite() && beta_0 > 0.0 {
            beta_0
        } else {
            1.0
        };
        self.actor_priors.insert(actor, (a0, b0));
    }

    /// Returns the supplied prior `(alpha_0, beta_0)` for `actor`, defaulting to uniform `Beta(1, 1)`.
    #[must_use]
    pub fn actor_prior(&self, actor: &ActorId) -> (f64, f64) {
        self.actor_priors.get(actor).copied().unwrap_or((1.0, 1.0))
    }

    /// Computes `(source, domain)` reliability decayed to `now_ms` (without mutating state).
    #[must_use]
    pub fn source_reliability_at(
        &self,
        actor: &ActorId,
        domain: &DomainId,
        now_ms: Time,
    ) -> BetaCredence {
        let (a0, b0) = self.actor_prior(actor);
        let mut cred = self
            .source_reliability
            .get(&(actor.clone(), domain.clone()))
            .copied()
            .unwrap_or_else(|| BetaCredence::from_prior(a0, b0, now_ms));
        cred.decay_toward_prior(now_ms, CREDENCE_LAMBDA_DOMAIN, CREDENCE_PERIOD_MS);
        cred
    }

    /// Computes an item's Beta credence decayed to `now_ms` (without mutating state).
    #[must_use]
    pub fn item_credence_at(&self, item_id: EventId, now_ms: Time) -> Option<BetaCredence> {
        let mut cred = self
            .beliefs
            .get(&item_id)
            .and_then(|b| b.credence)
            .or_else(|| self.item_credences.get(&item_id).copied())?;
        cred.decay_toward_prior(now_ms, CREDENCE_LAMBDA_DOMAIN, CREDENCE_PERIOD_MS);
        Some(cred)
    }

    /// Computes discounted convention preference bias in `[-CONV_BIAS_MAX, +CONV_BIAS_MAX]`.
    #[must_use]
    pub fn convention_bias_at(&self, place: &PlaceId, pattern_id: &str, now_ms: Time) -> f64 {
        let Some(mut cred) = self
            .conventions
            .get(&(place.clone(), pattern_id.to_owned()))
            .copied()
        else {
            return 0.0;
        };
        cred.decay_toward_prior(now_ms, CONV_LAMBDA, CREDENCE_PERIOD_MS);
        let mu = cred.expected().clamp(1e-6, 1.0 - 1e-6);
        let n_eff = cred.effective_evidence();
        let nu = n_eff / (n_eff + CONV_TAU);
        let raw = CONV_KAPPA * nu * (mu / (1.0 - mu)).ln();
        if raw.is_finite() {
            raw.clamp(-CONV_BIAS_MAX, CONV_BIAS_MAX)
        } else {
            0.0
        }
    }

    /// Applies a single `BeliefLogEntry` deterministically onto the projection maps.
    pub fn apply_log_entry(&mut self, entry: BeliefLogEntry, record_in_log: bool) {
        if record_in_log {
            self.log_entries.push(entry.clone());
        }
        match entry {
            BeliefLogEntry::BeliefRecorded {
                event_id,
                timestamp_ms,
                claim_key,
                requested_basis,
                basis,
                basis_downgraded,
                source_actor,
                audience,
                domain,
                source_event_ids,
                taint,
                valid_from_ms,
                valid_to_ms,
                replaced_by,
                alpha_0,
                beta_0,
                is_mind_inference,
                ..
            } => {
                let prev_id_opt = self.active_claim_for_domain(&domain, &claim_key);
                if let Some(prev_id) = prev_id_opt
                    && prev_id != event_id
                    && let Some(prev_belief) = self.beliefs.get_mut(&prev_id)
                {
                    let prev_is_direct = prev_belief.basis == MemoryBasis::Observed
                        || prev_belief.basis == MemoryBasis::SelfCaused;
                    let new_is_uncorroborated_told = basis == MemoryBasis::Told
                        && (taint != Taint::Trusted || DEFAULT_K_CORROB > 1);

                    if prev_is_direct && new_is_uncorroborated_told {
                        let cred = if matches!(basis, MemoryBasis::Told | MemoryBasis::Inferred) {
                            Some(self.init_belief_credence(
                                source_actor.as_ref(),
                                &domain,
                                alpha_0,
                                beta_0,
                                timestamp_ms,
                            ))
                        } else {
                            None
                        };
                        let mut corr = BTreeSet::new();
                        if let Some(ref sa) = source_actor {
                            corr.insert(sa.clone());
                        }
                        if let Some(c) = cred {
                            self.item_credences.insert(event_id, c);
                            self.fast_credences.insert(event_id, c);
                        }
                        self.beliefs.insert(
                            event_id,
                            BeliefRecord {
                                belief_id: event_id,
                                claim_key,
                                requested_basis,
                                basis,
                                basis_downgraded,
                                source_actor,
                                corroborating_actors: corr,
                                audience,
                                taint,
                                domain,
                                source_event_ids,
                                valid_from_ms,
                                valid_to_ms,
                                replaced_by,
                                credence: cred,
                                is_mind_inference,
                                erased: false,
                            },
                        );
                        return;
                    }

                    if prev_belief.basis == MemoryBasis::Told
                        && basis == MemoryBasis::Told
                        && replaced_by.is_none()
                        && valid_to_ms.is_none()
                        && let Some(ref sa) = source_actor
                        && !prev_belief.corroborating_actors.contains(sa)
                    {
                        prev_belief.corroborating_actors.insert(sa.clone());
                        prev_belief.taint = prev_belief.taint.combine(taint);
                    }

                    if !prev_is_direct
                        || basis == MemoryBasis::Observed
                        || basis == MemoryBasis::SelfCaused
                    {
                        prev_belief.valid_to_ms = Some(timestamp_ms);
                        prev_belief.replaced_by = Some(event_id);
                    }
                }

                let credence = if matches!(basis, MemoryBasis::Told | MemoryBasis::Inferred) {
                    let c = self.init_belief_credence(
                        source_actor.as_ref(),
                        &domain,
                        alpha_0,
                        beta_0,
                        timestamp_ms,
                    );
                    self.item_credences.insert(event_id, c);
                    self.fast_credences.insert(event_id, c);
                    Some(c)
                } else {
                    None
                };

                let mut corroborating_actors = BTreeSet::new();
                if let Some(ref sa) = source_actor {
                    corroborating_actors.insert(sa.clone());
                }

                let mut combined_taint = taint;
                for existing in self.beliefs.values() {
                    if !existing.erased
                        && existing.domain == domain
                        && existing.claim_key == claim_key
                        && existing.basis == MemoryBasis::Told
                    {
                        combined_taint = combined_taint.combine(existing.taint);
                        if let Some(ref sa) = existing.source_actor {
                            corroborating_actors.insert(sa.clone());
                        }
                    }
                }

                let record = BeliefRecord {
                    belief_id: event_id,
                    claim_key: claim_key.clone(),
                    requested_basis,
                    basis,
                    basis_downgraded,
                    source_actor,
                    corroborating_actors,
                    audience,
                    taint: combined_taint,
                    domain: domain.clone(),
                    source_event_ids,
                    valid_from_ms,
                    valid_to_ms,
                    replaced_by,
                    credence,
                    is_mind_inference,
                    erased: false,
                };
                if record.is_current() {
                    self.active_claims_by_domain
                        .insert((domain, claim_key.clone()), event_id);
                    self.active_claims.insert(claim_key, event_id);
                }
                self.beliefs.insert(event_id, record);
            }
            BeliefLogEntry::EvidenceApplied {
                timestamp_ms,
                target_id,
                source_actor,
                domain,
                positive,
                negative,
                is_convention,
                place,
                pattern_id,
                ..
            } => {
                if is_convention {
                    if let (Some(pl), Some(pat)) = (place, pattern_id) {
                        let peer_rel = source_actor
                            .as_ref()
                            .map(|a| self.source_reliability_at(a, &domain, timestamp_ms).expected())
                            .unwrap_or(1.0);
                        let w_obs = if source_actor.is_some() {
                            CONV_VICARIOUS_DISCOUNT * peer_rel.clamp(CONV_PEER_FLOOR, 1.0)
                        } else {
                            1.0
                        };
                        let conv = self.conventions.entry((pl, pat)).or_insert_with(|| {
                            BetaCredence::from_prior(CONV_ALPHA_0, CONV_BETA_0, timestamp_ms)
                        });
                        conv.apply_evidence(
                            w_obs * positive,
                            w_obs * negative,
                            timestamp_ms,
                            CONV_LAMBDA,
                            CREDENCE_PERIOD_MS,
                        );
                    }
                    return;
                }

                if target_id.as_u64() != 0 {
                    let prior_seed = self
                        .beliefs
                        .get(&target_id)
                        .and_then(|b| b.credence)
                        .or_else(|| self.item_credences.get(&target_id).copied())
                        .unwrap_or_else(|| BetaCredence::from_prior(1.0, 1.0, timestamp_ms));
                    if let Some(belief) = self.beliefs.get_mut(&target_id)
                        && let Some(cred) = &mut belief.credence
                    {
                        cred.apply_evidence(
                            positive,
                            negative,
                            timestamp_ms,
                            CREDENCE_LAMBDA_DOMAIN,
                            CREDENCE_PERIOD_MS,
                        );
                        self.item_credences.insert(target_id, *cred);
                    } else {
                        let cred = self
                            .item_credences
                            .entry(target_id)
                            .or_insert_with(|| BetaCredence::from_prior(1.0, 1.0, timestamp_ms));
                        cred.apply_evidence(
                            positive,
                            negative,
                            timestamp_ms,
                            CREDENCE_LAMBDA_DOMAIN,
                            CREDENCE_PERIOD_MS,
                        );
                    }
                    let fast = self.fast_credences.entry(target_id).or_insert(prior_seed);
                    fast.apply_evidence(
                        positive,
                        negative,
                        timestamp_ms,
                        CREDENCE_LAMBDA_FAST,
                        CREDENCE_PERIOD_MS,
                    );
                }

                if let Some(actor) = source_actor {
                    let (a0, b0) = self.actor_prior(&actor);
                    let rel = self
                        .source_reliability
                        .entry((actor, domain))
                        .or_insert_with(|| BetaCredence::from_prior(a0, b0, timestamp_ms));
                    rel.apply_evidence(
                        positive,
                        negative,
                        timestamp_ms,
                        CREDENCE_LAMBDA_DOMAIN,
                        CREDENCE_PERIOD_MS,
                    );
                }
            }
        }
    }

    fn init_belief_credence(
        &self,
        source_actor: Option<&ActorId>,
        domain: &DomainId,
        explicit_alpha_0: f64,
        explicit_beta_0: f64,
        now_ms: Time,
    ) -> BetaCredence {
        if let Some(actor) = source_actor {
            if let Some(rel) = self.source_reliability.get(&(actor.clone(), domain.clone())) {
                let mut c = *rel;
                c.decay_toward_prior(now_ms, CREDENCE_LAMBDA_DOMAIN, CREDENCE_PERIOD_MS);
                return c;
            }
            let (pa, pb) = self.actor_prior(actor);
            let a0 = if explicit_alpha_0 > 0.0
                && (pa - 1.0).abs() <= 1e-12
                && !self.actor_priors.contains_key(actor)
            {
                explicit_alpha_0
            } else {
                pa
            };
            let b0 = if explicit_beta_0 > 0.0
                && (pb - 1.0).abs() <= 1e-12
                && !self.actor_priors.contains_key(actor)
            {
                explicit_beta_0
            } else {
                pb
            };
            return BetaCredence::from_prior(a0, b0, now_ms);
        }
        BetaCredence::from_prior(explicit_alpha_0, explicit_beta_0, now_ms)
    }

    /// Recomputes all `(alpha, beta)` credence and `(source, domain)` reliability projections cleanly
    /// from surviving log entries after an authorized `hard_forget` erasure.
    pub fn recompute_after_forget(
        &mut self,
        target_event_ids: &[EventId],
        target_actor_ids: &[ActorId],
    ) {
        let erased_events: BTreeSet<EventId> = target_event_ids.iter().copied().collect();
        let erased_actors: BTreeSet<ActorId> = target_actor_ids.iter().cloned().collect();

        for actor in &erased_actors {
            self.actor_priors.remove(actor);
        }

        let mut expanded_erased = erased_events;
        let mut changed = true;
        while changed {
            changed = false;
            for entry in &self.log_entries {
                if let BeliefLogEntry::BeliefRecorded {
                    event_id,
                    source_actor,
                    source_event_ids,
                    ..
                } = entry
                {
                    if expanded_erased.contains(event_id) {
                        continue;
                    }
                    let actor_erased = source_actor
                        .as_ref()
                        .is_some_and(|a| erased_actors.contains(a));
                    let all_sources_erased = !source_event_ids.is_empty()
                        && source_event_ids
                            .iter()
                            .all(|sid| expanded_erased.contains(sid));
                    if actor_erased || all_sources_erased {
                        expanded_erased.insert(*event_id);
                        changed = true;
                    }
                }
            }
        }

        let mut surviving_entries = Vec::with_capacity(self.log_entries.len());
        for entry in self.log_entries.drain(..) {
            match entry {
                BeliefLogEntry::BeliefRecorded {
                    event_id,
                    timestamp_ms,
                    seq,
                    claim_key,
                    requested_basis,
                    basis,
                    basis_downgraded,
                    source_actor,
                    audience,
                    domain,
                    mut source_event_ids,
                    taint,
                    valid_from_ms,
                    valid_to_ms,
                    mut replaced_by,
                    alpha_0,
                    beta_0,
                    is_mind_inference,
                } => {
                    if expanded_erased.contains(&event_id)
                        || source_actor
                            .as_ref()
                            .is_some_and(|a| erased_actors.contains(a))
                    {
                        continue;
                    }
                    source_event_ids.retain(|sid| !expanded_erased.contains(sid));
                    if let Some(rep) = replaced_by
                        && expanded_erased.contains(&rep)
                    {
                        replaced_by = None;
                    }
                    surviving_entries.push(BeliefLogEntry::BeliefRecorded {
                        event_id,
                        timestamp_ms,
                        seq,
                        claim_key,
                        requested_basis,
                        basis,
                        basis_downgraded,
                        source_actor,
                        audience,
                        domain,
                        source_event_ids,
                        taint,
                        valid_from_ms,
                        valid_to_ms: if replaced_by.is_none() {
                            None
                        } else {
                            valid_to_ms
                        },
                        replaced_by,
                        alpha_0,
                        beta_0,
                        is_mind_inference,
                    });
                }
                BeliefLogEntry::EvidenceApplied {
                    event_id,
                    timestamp_ms,
                    seq,
                    target_id,
                    source_actor,
                    domain,
                    positive,
                    negative,
                    is_convention,
                    place,
                    pattern_id,
                } => {
                    if expanded_erased.contains(&event_id)
                        || expanded_erased.contains(&target_id)
                        || source_actor
                            .as_ref()
                            .is_some_and(|a| erased_actors.contains(a))
                    {
                        continue;
                    }
                    surviving_entries.push(BeliefLogEntry::EvidenceApplied {
                        event_id,
                        timestamp_ms,
                        seq,
                        target_id,
                        source_actor,
                        domain,
                        positive,
                        negative,
                        is_convention,
                        place,
                        pattern_id,
                    });
                }
            }
        }

        surviving_entries.sort_by(|a, b| {
            a.timestamp_ms()
                .cmp(&b.timestamp_ms())
                .then_with(|| a.seq().cmp(&b.seq()))
                .then_with(|| a.event_id().cmp(&b.event_id()))
        });

        self.beliefs.clear();
        self.active_claims.clear();
        self.active_claims_by_domain.clear();
        self.source_reliability.clear();
        self.item_credences.clear();
        self.fast_credences.clear();
        self.conventions.clear();

        for entry in surviving_entries {
            self.apply_log_entry(entry, true);
        }
    }

    /// Rebuilds an identical `BeliefBook` strictly by replaying its `log_entries` and `actor_priors`.
    #[must_use]
    pub fn rebuild_from_log(&self) -> Self {
        let mut rebuilt = Self {
            actor_priors: self.actor_priors.clone(),
            ..Self::default()
        };
        let mut ordered = self.log_entries.clone();
        ordered.sort_by(|a, b| {
            a.timestamp_ms()
                .cmp(&b.timestamp_ms())
                .then_with(|| a.seq().cmp(&b.seq()))
                .then_with(|| a.event_id().cmp(&b.event_id()))
        });
        for entry in ordered {
            rebuilt.apply_log_entry(entry, true);
        }
        rebuilt
    }
}

#[cfg(test)]
mod tests {
    use monoagens_protocol::{OpaquePayload, OwnerSigned};

    use super::*;
    use crate::context::{IdentityAnchor, PrincipleDescriptor, PrincipleSeverity};

    fn belief_entry(
        id: u64,
        seq: u64,
        ts_ms: u64,
        claim_key: &str,
        domain: &str,
        actor: &str,
    ) -> BeliefLogEntry {
        BeliefLogEntry::BeliefRecorded {
            event_id: EventId::new(id),
            timestamp_ms: Time::from_millis(ts_ms),
            seq: LogSeq::new(seq),
            claim_key: claim_key.to_owned(),
            requested_basis: MemoryBasis::Told,
            basis: MemoryBasis::Told,
            basis_downgraded: false,
            source_actor: Some(ActorId::new(actor)),
            audience: Audience::new([ActorId::new(actor), ActorId::new("soul")]),
            domain: DomainId::new(domain),
            source_event_ids: vec![EventId::new(id)],
            taint: Taint::Trusted,
            valid_from_ms: Time::from_millis(ts_ms),
            valid_to_ms: None,
            replaced_by: None,
            alpha_0: 1.0,
            beta_0: 1.0,
            is_mind_inference: false,
        }
    }

    #[test]
    fn memdrift_domain_isolation_three_timescale_decay_and_immutable_owner_signed_anchors() {
        let mut book = BeliefBook::new();
        let chat_domain = DomainId::new("chat");
        let tools_domain = DomainId::new("tools");
        let t0 = 1_000_000u64;

        // 1. Record same claim_key under `tools` and `chat` domains.
        book.apply_log_entry(
            belief_entry(10, 1, t0, "pref:verbosity", "tools", "alice"),
            true,
        );
        book.apply_log_entry(
            belief_entry(20, 2, t0 + 1_000, "pref:verbosity", "chat", "alice"),
            true,
        );

        // Tool-domain belief MUST remain current and untouched by chat-domain claim (MEMDRIFT guard).
        assert!(book.beliefs[&EventId::new(10)].is_current());
        assert!(book.beliefs[&EventId::new(20)].is_current());
        assert_eq!(
            book.active_claim_for_domain(&tools_domain, "pref:verbosity"),
            Some(EventId::new(10))
        );
        assert_eq!(
            book.active_claim_for_domain(&chat_domain, "pref:verbosity"),
            Some(EventId::new(20))
        );

        // Updating `chat` claim again supersedes ONLY `chat` (20 -> 21), never `tools` (10).
        book.apply_log_entry(
            belief_entry(21, 3, t0 + 2_000, "pref:verbosity", "chat", "alice"),
            true,
        );
        assert!(book.beliefs[&EventId::new(10)].is_current());
        assert_eq!(book.beliefs[&EventId::new(20)].replaced_by, Some(EventId::new(21)));
        assert_eq!(
            book.active_claim_for_domain(&tools_domain, "pref:verbosity"),
            Some(EventId::new(10))
        );
        assert_eq!(
            book.active_claim_for_domain(&chat_domain, "pref:verbosity"),
            Some(EventId::new(21))
        );

        // 2. Fast (0.80/day) vs slow (0.98/day) relaxation over 7 days.
        book.apply_log_entry(
            BeliefLogEntry::EvidenceApplied {
                event_id: EventId::new(30),
                timestamp_ms: Time::from_millis(t0 + 3_000),
                seq: LogSeq::new(4),
                target_id: EventId::new(21),
                source_actor: Some(ActorId::new("alice")),
                domain: chat_domain.clone(),
                positive: 10.0,
                negative: 0.0,
                is_convention: false,
                place: None,
                pattern_id: None,
            },
            true,
        );
        let (fast_now, slow_now) = book
            .effective_preference_at(EventId::new(21), Time::from_millis(t0 + 3_000))
            .expect("credence");
        assert!((fast_now - slow_now).abs() < 1e-9);
        assert!(fast_now > 0.90);

        let t_7d = Time::from_millis(t0 + 3_000 + 7 * CREDENCE_PERIOD_MS);
        let (fast_7d, slow_7d) = book
            .effective_preference_at(EventId::new(21), t_7d)
            .expect("7d credence");
        // Fast (0.80^7 ≈ 0.21) decays significantly further back toward prior 0.5 than slow (0.98^7 ≈ 0.87).
        assert!(
            fast_7d < slow_7d - 0.10,
            "fast={fast_7d} must decay faster than slow={slow_7d} over 7 days"
        );
        assert!(fast_7d > 0.5 && slow_7d > 0.85);

        // 3. OwnerSigned IdentityAnchor & principles never decay (lambda = 1.0).
        let mut invariant_cred = BetaCredence::with_counts(1.0, 1.0, 11.0, 1.0, Time::from_millis(t0));
        let expected_before = invariant_cred.expected();
        invariant_cred.decay_toward_prior(t_7d, 1.0, CREDENCE_PERIOD_MS);
        assert_eq!(invariant_cred.expected().to_bits(), expected_before.to_bits());

        let signed_payload = OwnerSigned {
            payload: OpaquePayload::from_bytes(b"immutable-owner-anchor".as_slice()),
            signer: ActorId::new("owner"),
            signed_at_ms: Time::from_millis(t0),
            signature: "sig-owner-1".to_owned(),
        };
        let anchor = IdentityAnchor::from_owner_signed(&signed_payload);
        let principle = OwnerSigned {
            payload: PrincipleDescriptor {
                id: "p-hardline".to_owned(),
                severity: PrincipleSeverity::HardLine,
                blocked_capabilities: Vec::new(),
                forbid_identity_change: true,
                forbid_untrusted_taint: true,
                created_seq: LogSeq::new(1),
                owner_sig_hash: anchor.payload.owner_sig_hash,
            },
            signer: ActorId::new("owner"),
            signed_at_ms: Time::from_millis(t0),
            signature: "sig-owner-2".to_owned(),
        };
        assert!(principle.payload.forbid_identity_change);
        assert_eq!(anchor.signer, ActorId::new("owner"));

        // 4. Log rebuild preserves domain isolation and fast credences identically.
        let rebuilt = book.rebuild_from_log();
        assert_eq!(rebuilt.active_claims_by_domain, book.active_claims_by_domain);
        assert_eq!(
            rebuilt.effective_preference_at(EventId::new(21), t_7d),
            Some((fast_7d, slow_7d))
        );
    }
}
