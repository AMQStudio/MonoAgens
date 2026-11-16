//! Beta-Bernoulli `(alpha, beta)` belief & credibility store, basis verification,
//! `told` source-prior initialization, `k_corrob` corroboration, `R-32` convention discounting,
//! and log-rebuildable `hard_forget` projection recomputation (`R-34`).

use std::collections::{BTreeMap, BTreeSet};

use monoagens_protocol::{ActorId, DomainId, EventId, PlaceId, Taint, Time};
use serde::{Deserialize, Serialize};

use crate::beliefs::domains::{CredenceRebaseRecord, DomainTransitionKind, DomainTransitionRecord};
use crate::event::LogSeq;
use crate::prior::Prior;
use crate::recall::{MemoryBasis, MemoryEntry};
use crate::registry::Switches;

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

/// Stored belief entry with basis provenance, absolute validity bounds, and credence (`§7`, `R-17`, `R-43`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BeliefRecord {
    pub belief_id: EventId,
    pub claim_key: String,
    pub requested_basis: MemoryBasis,
    pub basis: MemoryBasis,
    pub basis_downgraded: bool,
    pub source_actor: Option<ActorId>,
    pub corroborating_actors: BTreeSet<ActorId>,
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
    DomainsRebased {
        event_id: EventId,
        timestamp_ms: Time,
        seq: LogSeq,
        transitions: Vec<DomainTransitionRecord>,
        credence_rebased: Vec<CredenceRebaseRecord>,
    },
}

impl BeliefLogEntry {
    #[must_use]
    pub const fn timestamp_ms(&self) -> Time {
        match self {
            Self::BeliefRecorded { timestamp_ms, .. }
            | Self::EvidenceApplied { timestamp_ms, .. }
            | Self::DomainsRebased { timestamp_ms, .. } => *timestamp_ms,
        }
    }

    #[must_use]
    pub const fn seq(&self) -> LogSeq {
        match self {
            Self::BeliefRecorded { seq, .. }
            | Self::EvidenceApplied { seq, .. }
            | Self::DomainsRebased { seq, .. } => *seq,
        }
    }

    #[must_use]
    pub const fn event_id(&self) -> EventId {
        match self {
            Self::BeliefRecorded { event_id, .. }
            | Self::EvidenceApplied { event_id, .. }
            | Self::DomainsRebased { event_id, .. } => *event_id,
        }
    }
}

/// Checks reasoning's claimed [`MemoryBasis`] against actual memory provenance (`beliefs/spec.md`, `R-17`).
///
/// - Mind inferences (`is_mind_inference`) are always stored as `Inferred`.
/// - `Observed` claims whose links reach no witnessed `Observed`/`SelfCaused` percept are
///   downgraded to `Inferred` with `downgraded = true`.
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

    // Bounded BFS over `used_ids` -> `source_ids` checking for a witnessed percept.
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
/// and `R-32` discounted place conventions, strictly rebuildable from the event log.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct BeliefBook {
    pub beliefs: BTreeMap<EventId, BeliefRecord>,
    pub active_claims: BTreeMap<String, EventId>,
    pub source_reliability: BTreeMap<(ActorId, DomainId), BetaCredence>,
    pub item_credences: BTreeMap<EventId, BetaCredence>,
    pub conventions: BTreeMap<(PlaceId, String), BetaCredence>,
    pub actor_priors: BTreeMap<ActorId, (f64, f64)>,
    pub log_entries: Vec<BeliefLogEntry>,
}

impl BeliefBook {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
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
        prior: &Prior,
    ) -> BetaCredence {
        let (a0, b0) = self.actor_prior(actor);
        let mut cred = self
            .source_reliability
            .get(&(actor.clone(), domain.clone()))
            .copied()
            .unwrap_or_else(|| BetaCredence::from_prior(a0, b0, now_ms));
        cred.decay_toward_prior(
            now_ms,
            prior.credence_lambda_domain,
            prior.credence_period_ms,
        );
        cred
    }

    /// Computes an item's Beta credence decayed to `now_ms` (without mutating state).
    #[must_use]
    pub fn item_credence_at(
        &self,
        item_id: EventId,
        now_ms: Time,
        prior: &Prior,
    ) -> Option<BetaCredence> {
        let mut cred = self
            .beliefs
            .get(&item_id)
            .and_then(|b| b.credence)
            .or_else(|| self.item_credences.get(&item_id).copied())?;
        cred.decay_toward_prior(
            now_ms,
            prior.credence_lambda_domain,
            prior.credence_period_ms,
        );
        Some(cred)
    }

    /// Computes `R-32` discounted convention preference bias `Delta b_{p, pi} in [-B_conv, +B_conv]`.
    #[must_use]
    pub fn convention_bias_at(
        &self,
        place: &PlaceId,
        pattern_id: &str,
        now_ms: Time,
        prior: &Prior,
    ) -> f64 {
        if prior.conv_vicarious_discount <= 0.0 || prior.conv_bias_max <= 0.0 {
            return 0.0;
        }
        let Some(mut cred) = self
            .conventions
            .get(&(place.clone(), pattern_id.to_owned()))
            .copied()
        else {
            return 0.0;
        };
        cred.decay_toward_prior(now_ms, prior.conv_lambda, prior.credence_period_ms);
        let mu = cred.expected().clamp(1e-6, 1.0 - 1e-6);
        let n_eff = cred.effective_evidence();
        let nu = n_eff / (n_eff + prior.conv_tau.max(1e-6));
        let raw = prior.conv_kappa * nu * (mu / (1.0 - mu)).ln();
        if raw.is_finite() {
            raw.clamp(-prior.conv_bias_max, prior.conv_bias_max)
        } else {
            0.0
        }
    }

    /// Applies a single `BeliefLogEntry` deterministically onto the projection maps.
    pub fn apply_log_entry(
        &mut self,
        entry: BeliefLogEntry,
        prior: &Prior,
        switches: &Switches,
        record_in_log: bool,
    ) {
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
                // Check whether an existing active belief exists for `claim_key`.
                if let Some(&prev_id) = self.active_claims.get(&claim_key)
                    && prev_id != event_id
                    && let Some(prev_belief) = self.beliefs.get_mut(&prev_id)
                {
                    let prev_is_direct = prev_belief.basis == MemoryBasis::Observed
                        || prev_belief.basis == MemoryBasis::SelfCaused;
                    let new_is_uncorroborated_told = basis == MemoryBasis::Told
                        && (taint != Taint::Trusted || prior.k_corrob > 1);

                    // Skeptical corroboration (`R-43`): if a single untrusted `told` claim
                    // repeats/conflicts with an existing direct observation, it NEVER overrides it!
                    if prev_is_direct && new_is_uncorroborated_told {
                        let cred = if matches!(basis, MemoryBasis::Told | MemoryBasis::Inferred) {
                            Some(self.init_belief_credence(
                                source_actor.as_ref(),
                                &domain,
                                alpha_0,
                                beta_0,
                                timestamp_ms,
                                prior,
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

                    // If same `claim_key` is corroborated by another distinct source on an existing Told belief:
                    if prev_belief.basis == MemoryBasis::Told
                        && basis == MemoryBasis::Told
                        && replaced_by.is_none()
                        && valid_to_ms.is_none()
                        && let Some(ref sa) = source_actor
                        && !prev_belief.corroborating_actors.contains(sa)
                    {
                        prev_belief.corroborating_actors.insert(sa.clone());
                        // Keep `min(trust)` taint across corroborating sources (`R-43`).
                        prev_belief.taint = prev_belief.taint.combine(taint);
                    }

                    // Explicit replacement or observation superseding prior belief without deleting old record.
                    if !prev_is_direct
                        || basis == MemoryBasis::Observed
                        || basis == MemoryBasis::SelfCaused
                    {
                        prev_belief.valid_to_ms = Some(timestamp_ms);
                        prev_belief.replaced_by = Some(event_id);
                    }
                }

                // Cleanly separate Observed/SelfCaused direct evidence from Told/Inferred credence (`10.1`).
                let credence = if matches!(basis, MemoryBasis::Told | MemoryBasis::Inferred) {
                    let c = self.init_belief_credence(
                        source_actor.as_ref(),
                        &domain,
                        alpha_0,
                        beta_0,
                        timestamp_ms,
                        prior,
                    );
                    self.item_credences.insert(event_id, c);
                    Some(c)
                } else {
                    None
                };

                let mut corroborating_actors = BTreeSet::new();
                if let Some(ref sa) = source_actor {
                    corroborating_actors.insert(sa.clone());
                }

                // Collect prior corroborating distinct actors and `min(trust)` taint for same `claim_key`.
                let mut combined_taint = taint;
                for existing in self.beliefs.values() {
                    if !existing.erased
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
                    taint: combined_taint,
                    domain,
                    source_event_ids,
                    valid_from_ms,
                    valid_to_ms,
                    replaced_by,
                    credence,
                    is_mind_inference,
                    erased: false,
                };
                if record.is_current() {
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
                if !switches.credence {
                    return;
                }
                if is_convention {
                    if let (Some(pl), Some(pat)) = (place, pattern_id) {
                        // `R-32`: vicarious peer observation strictly discounted (`lambda_vic < 1.0`).
                        let peer_rel = source_actor
                            .as_ref()
                            .map(|a| {
                                self.source_reliability_at(a, &domain, timestamp_ms, prior)
                                    .expected()
                            })
                            .unwrap_or(1.0);
                        let w_obs = if source_actor.is_some() {
                            prior.conv_vicarious_discount
                                * peer_rel.clamp(prior.conv_peer_floor, 1.0)
                        } else {
                            1.0
                        };
                        let conv = self.conventions.entry((pl, pat)).or_insert_with(|| {
                            BetaCredence::from_prior(
                                prior.conv_alpha_0,
                                prior.conv_beta_0,
                                timestamp_ms,
                            )
                        });
                        conv.apply_evidence(
                            w_obs * positive,
                            w_obs * negative,
                            timestamp_ms,
                            prior.conv_lambda,
                            prior.credence_period_ms,
                        );
                    }
                    return;
                }

                // Update cited item's credence (`10.1`: ONLY cited items move!).
                if target_id.as_u64() != 0 {
                    if let Some(belief) = self.beliefs.get_mut(&target_id)
                        && let Some(cred) = &mut belief.credence
                    {
                        cred.apply_evidence(
                            positive,
                            negative,
                            timestamp_ms,
                            prior.credence_lambda_domain,
                            prior.credence_period_ms,
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
                            prior.credence_lambda_domain,
                            prior.credence_period_ms,
                        );
                    }
                }

                // Update `(source, domain)` reliability when attributed to a source actor.
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
                        prior.credence_lambda_domain,
                        prior.credence_period_ms,
                    );
                }
            }
            BeliefLogEntry::DomainsRebased {
                timestamp_ms,
                transitions,
                credence_rebased,
                ..
            } => {
                if !switches.credence {
                    return;
                }
                if !credence_rebased.is_empty() {
                    for rb in credence_rebased {
                        if rb.from_domain != rb.to_domain {
                            self.source_reliability
                                .remove(&(rb.actor.clone(), rb.from_domain.clone()));
                        }
                        self.source_reliability.insert(
                            (rb.actor, rb.to_domain),
                            BetaCredence::with_counts(
                                rb.alpha_0,
                                rb.beta_0,
                                rb.new_alpha,
                                rb.new_beta,
                                timestamp_ms,
                            ),
                        );
                    }
                } else {
                    self.apply_domain_transitions(&transitions, timestamp_ms, prior);
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
        prior: &Prior,
    ) -> BetaCredence {
        if let Some(actor) = source_actor {
            // If the source has an existing `(actor, domain)` reliability entry, initialize `told`
            // from that domain reliability; otherwise use `(explicit_alpha_0, explicit_beta_0)`
            // or the source's `(alpha_0, beta_0)` prior (`A-01`, `beliefs/spec.md` L60-63 & L86-88).
            if let Some(rel) = self
                .source_reliability
                .get(&(actor.clone(), domain.clone()))
            {
                let mut c = *rel;
                c.decay_toward_prior(
                    now_ms,
                    prior.credence_lambda_domain,
                    prior.credence_period_ms,
                );
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

    /// Computes Jøsang drift/merge/split credence transitions across `source_reliability` (`A-12`).
    pub fn compute_rebase_records(
        &self,
        transitions: &[DomainTransitionRecord],
        now_ms: Time,
        prior: &Prior,
    ) -> Vec<CredenceRebaseRecord> {
        let mut out = Vec::new();
        let gamma = prior.domain_gamma_drift.max(0.0);

        for tr in transitions {
            let sim = tr.similarity.clamp(0.0, 1.0);
            let retain = if gamma == 0.0 { 1.0 } else { sim.powf(gamma) };
            match tr.kind {
                DomainTransitionKind::Continued => {
                    if let Some(did) = &tr.old_domain {
                        for ((actor, dom), cred) in &self.source_reliability {
                            if dom == did {
                                let mut c = *cred;
                                c.decay_toward_prior(
                                    now_ms,
                                    prior.credence_lambda_domain,
                                    prior.credence_period_ms,
                                );
                                let da = (c.alpha - c.alpha_0).max(0.0) * retain;
                                let db = (c.beta - c.beta_0).max(0.0) * retain;
                                out.push(CredenceRebaseRecord {
                                    actor: actor.clone(),
                                    from_domain: did.clone(),
                                    to_domain: did.clone(),
                                    new_alpha: c.alpha_0 + da,
                                    new_beta: c.beta_0 + db,
                                    alpha_0: c.alpha_0,
                                    beta_0: c.beta_0,
                                });
                            }
                        }
                    }
                }
                DomainTransitionKind::Merged => {
                    if let (Some(dead), Some(surv)) = (&tr.old_domain, &tr.new_domain) {
                        for ((actor, dom), dead_cred) in &self.source_reliability {
                            if dom == dead {
                                let mut dc = *dead_cred;
                                dc.decay_toward_prior(
                                    now_ms,
                                    prior.credence_lambda_domain,
                                    prior.credence_period_ms,
                                );
                                let da_dead = (dc.alpha - dc.alpha_0).max(0.0) * retain;
                                let db_dead = (dc.beta - dc.beta_0).max(0.0) * retain;

                                let (a0, b0) = self.actor_prior(actor);
                                let mut sc = self
                                    .source_reliability
                                    .get(&(actor.clone(), surv.clone()))
                                    .copied()
                                    .unwrap_or_else(|| BetaCredence::from_prior(a0, b0, now_ms));
                                sc.decay_toward_prior(
                                    now_ms,
                                    prior.credence_lambda_domain,
                                    prior.credence_period_ms,
                                );
                                out.push(CredenceRebaseRecord {
                                    actor: actor.clone(),
                                    from_domain: dead.clone(),
                                    to_domain: surv.clone(),
                                    new_alpha: sc.alpha + da_dead,
                                    new_beta: sc.beta + db_dead,
                                    alpha_0: sc.alpha_0,
                                    beta_0: sc.beta_0,
                                });
                            }
                        }
                    }
                }
                DomainTransitionKind::Split => {
                    if let (Some(old_id), Some(new_id)) = (&tr.old_domain, &tr.new_domain)
                        && prior.domain_rho_split > 0.0
                    {
                        for ((actor, dom), old_cred) in &self.source_reliability {
                            if dom == old_id {
                                let mut oc = *old_cred;
                                oc.decay_toward_prior(
                                    now_ms,
                                    prior.credence_lambda_domain,
                                    prior.credence_period_ms,
                                );
                                let frac = (prior.domain_rho_split * retain).clamp(0.0, 0.5);
                                let da_move = (oc.alpha - oc.alpha_0).max(0.0) * frac;
                                let db_move = (oc.beta - oc.beta_0).max(0.0) * frac;
                                out.push(CredenceRebaseRecord {
                                    actor: actor.clone(),
                                    from_domain: old_id.clone(),
                                    to_domain: new_id.clone(),
                                    new_alpha: oc.alpha_0 + da_move,
                                    new_beta: oc.beta_0 + db_move,
                                    alpha_0: oc.alpha_0,
                                    beta_0: oc.beta_0,
                                });
                                out.push(CredenceRebaseRecord {
                                    actor: actor.clone(),
                                    from_domain: old_id.clone(),
                                    to_domain: old_id.clone(),
                                    new_alpha: oc.alpha - da_move,
                                    new_beta: oc.beta - db_move,
                                    alpha_0: oc.alpha_0,
                                    beta_0: oc.beta_0,
                                });
                            }
                        }
                    }
                }
                DomainTransitionKind::Birth | DomainTransitionKind::Retired => {}
            }
        }
        out
    }

    fn apply_domain_transitions(
        &mut self,
        transitions: &[DomainTransitionRecord],
        now_ms: Time,
        prior: &Prior,
    ) {
        let rebased = self.compute_rebase_records(transitions, now_ms, prior);
        for rb in rebased {
            if rb.from_domain != rb.to_domain {
                self.source_reliability
                    .remove(&(rb.actor.clone(), rb.from_domain.clone()));
            }
            self.source_reliability.insert(
                (rb.actor, rb.to_domain),
                BetaCredence::with_counts(rb.alpha_0, rb.beta_0, rb.new_alpha, rb.new_beta, now_ms),
            );
        }
    }

    /// Recomputes all `(alpha, beta)` credence and `(source, domain)` reliability projections cleanly
    /// from surviving log entries after an authorized `hard_forget` erasure (`R-34`, `10.4`).
    pub fn recompute_after_forget(
        &mut self,
        target_event_ids: &[EventId],
        target_actor_ids: &[ActorId],
        prior: &Prior,
        switches: &Switches,
    ) {
        let erased_events: BTreeSet<EventId> = target_event_ids.iter().copied().collect();
        let erased_actors: BTreeSet<ActorId> = target_actor_ids.iter().cloned().collect();

        for actor in &erased_actors {
            self.actor_priors.remove(actor);
        }

        // Compute exclusive dependency closure across logged beliefs/evidence.
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
                BeliefLogEntry::DomainsRebased {
                    event_id,
                    timestamp_ms,
                    seq,
                    transitions,
                    mut credence_rebased,
                } => {
                    if expanded_erased.contains(&event_id) {
                        continue;
                    }
                    credence_rebased.retain(|rb| !erased_actors.contains(&rb.actor));
                    surviving_entries.push(BeliefLogEntry::DomainsRebased {
                        event_id,
                        timestamp_ms,
                        seq,
                        transitions,
                        credence_rebased,
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

        // Clear all derived projections and refold cleanly from surviving log entries.
        self.beliefs.clear();
        self.active_claims.clear();
        self.source_reliability.clear();
        self.item_credences.clear();
        self.conventions.clear();

        for entry in surviving_entries {
            self.apply_log_entry(entry, prior, switches, true);
        }
    }

    /// Rebuilds an identical `BeliefBook` strictly by replaying its `log_entries` and `actor_priors`.
    #[must_use]
    pub fn rebuild_from_log(&self, prior: &Prior, switches: &Switches) -> Self {
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
            rebuilt.apply_log_entry(entry, prior, switches, true);
        }
        rebuilt
    }
}
