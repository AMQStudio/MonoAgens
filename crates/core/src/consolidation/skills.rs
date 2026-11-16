//! Contrastive lesson extraction (`A-10`), cross-place procedure extraction (`A-11`, `R-38`),
//! and CUSUM / schema-drift procedure lifecycle monitoring (`R-40`).

use std::collections::{BTreeMap, BTreeSet};

use monoagens_protocol::{
    ActorId, Audience, BodyId, CapabilityId, CapabilitySchemaHash, DomainId, EventId, PlaceId,
    ReceiptOutcome, TaggedEmbedding, Taint, TerminationReason, Time,
};
use serde::{Deserialize, Serialize};

use crate::beliefs::BetaCredence;
use crate::context::{CapabilityCatalogEntry, ProcedureEntry};
use crate::prior::Prior;
use crate::registry::Switches;

/// Lifecycle state of an extracted skill procedure (`R-40`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcedureLifecycleState {
    Active,
    Warned,
    Retired,
}

impl ProcedureLifecycleState {
    pub(crate) const fn code(self) -> u8 {
        match self {
            Self::Active => 1,
            Self::Warned => 2,
            Self::Retired => 3,
        }
    }

    pub(crate) const fn from_code(code: u8) -> Option<Self> {
        match code {
            1 => Some(Self::Active),
            2 => Some(Self::Warned),
            3 => Some(Self::Retired),
            _ => None,
        }
    }
}

/// Recorded failure episode used for contrastive lesson clustering (`A-10`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FailureEpisodeRecord {
    pub episode_id: EventId,
    pub signature_key: String,
    pub domain: Option<DomainId>,
    pub capability_id: Option<CapabilityId>,
    pub place: PlaceId,
    pub actors: Vec<ActorId>,
    pub taint: Taint,
    pub timestamp_ms: Time,
}

/// Recorded successful episode eligible as a contrast example (`A-10`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SuccessEpisodeRecord {
    pub episode_id: EventId,
    pub signature_key: String,
    pub domain: Option<DomainId>,
    pub capability_id: Option<CapabilityId>,
    pub place: PlaceId,
    pub actors: Vec<ActorId>,
    pub taint: Taint,
    pub timestamp_ms: Time,
}

/// Extracted contrastive lesson backed by Beta credence (`A-10`, `tasks.md 14.1`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LessonEntry {
    pub lesson_id: EventId,
    pub pattern: String,
    pub failure_signature: String,
    pub domain: Option<DomainId>,
    pub failure_ids: Vec<EventId>,
    pub contrastive_success_ids: Vec<EventId>,
    pub actors: Vec<ActorId>,
    pub credence: BetaCredence,
    pub taint: Taint,
    pub created_at_ms: Time,
    pub updated_at_ms: Time,
    pub active: bool,
}

/// Monitored skill procedure with discounted Beta reliability and downward Bernoulli CUSUM (`A-11`, `R-38`, `R-40`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrackedProcedure {
    pub procedure_id: EventId,
    pub name: String,
    pub steps: Vec<String>,
    pub required_cap_ids: Vec<CapabilityId>,
    pub capability_schema_hash: CapabilitySchemaHash,
    pub domain: Option<DomainId>,
    pub places: Vec<PlaceId>,
    pub actors: Vec<ActorId>,
    pub audience: Audience,
    pub owner_signed: bool,
    pub embedding: Option<TaggedEmbedding>,
    pub source_goal_id: Option<EventId>,
    pub source_event_ids: Vec<EventId>,
    pub taint: Taint,
    pub state: ProcedureLifecycleState,
    pub retired_reason: Option<String>,
    pub retired_at_ms: Option<Time>,
    pub created_at_ms: Time,
    pub updated_at_ms: Time,
    /// Discounted Beta `(alpha, beta)` reliability counts (base prior `(1, 1)`).
    pub alpha: f64,
    pub beta: f64,
    /// Total non-canceled execution observations recorded.
    pub trial_count: u64,
    pub success_count: u64,
    /// Baseline success probability locked once `trial_count >= proc_cusum_n_min_lock`.
    pub cusum_baseline_p0: Option<f64>,
    /// One-sided downward CUSUM accumulator.
    pub cusum_stat: f64,
    pub warn_count: u32,
}

impl TrackedProcedure {
    /// Posterior reliability estimate `alpha / (alpha + beta)`.
    #[must_use]
    pub fn posterior_mean(&self) -> f64 {
        let denom = self.alpha + self.beta;
        if denom > 0.0 && denom.is_finite() {
            (self.alpha / denom).clamp(0.0, 1.0)
        } else {
            0.5
        }
    }

    /// Converts into a [`ProcedureEntry`] for progressive `Tool RAG` disclosure (`R-38`).
    #[must_use]
    pub fn to_context_entry(&self) -> ProcedureEntry {
        let retired = self.state == ProcedureLifecycleState::Retired;
        ProcedureEntry {
            procedure_id: self.procedure_id,
            name: self.name.clone(),
            steps: self.steps.clone(),
            required_cap_ids: self.required_cap_ids.clone(),
            capability_schema_hash: self.capability_schema_hash.clone(),
            audience: self.audience.clone(),
            domain: self.domain.clone(),
            active: !retired,
            retired,
            owner_signed: self.owner_signed,
            embedding: self.embedding.clone(),
        }
    }
}

/// Outcome of recording a procedure execution receipt.
#[derive(Debug, Clone, PartialEq)]
pub enum ProcedureMonitorOutcome {
    IgnoredCanceledOrPreempted,
    IgnoredNotFound,
    IgnoredAlreadyRetired,
    Updated {
        procedure_id: EventId,
        previous_state: ProcedureLifecycleState,
        new_state: ProcedureLifecycleState,
        posterior_mean: f64,
        cusum_stat: f64,
        reason: Option<String>,
    },
}

/// State book for contrastive lessons and monitored procedures.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct SkillBook {
    pub failure_episodes: Vec<FailureEpisodeRecord>,
    pub success_episodes: Vec<SuccessEpisodeRecord>,
    pub lessons: BTreeMap<EventId, LessonEntry>,
    pub procedures: BTreeMap<EventId, TrackedProcedure>,
    pub next_synthetic_id: u64,
}

/// Parameters for [`SkillBook::record_consolidated_lesson`].
#[derive(Debug, Clone)]
pub struct RecordLessonParams {
    pub lesson_id: Option<EventId>,
    pub pattern: String,
    pub failure_signature: String,
    pub failure_ids: Vec<EventId>,
    pub contrastive_success_ids: Vec<EventId>,
    pub domain: Option<DomainId>,
    pub taint: Taint,
    pub now_ms: Time,
}

/// Parameters for [`SkillBook::extract_procedure_from_goal_chain`].
#[derive(Debug, Clone)]
pub struct ExtractProcedureParams {
    pub procedure_id: Option<EventId>,
    pub source_goal_id: Option<EventId>,
    pub name: String,
    pub steps: Vec<String>,
    pub required_cap_ids: Vec<CapabilityId>,
    pub capability_schema_hash: CapabilitySchemaHash,
    pub places: Vec<PlaceId>,
    pub actors: Vec<ActorId>,
    pub audience: Audience,
    pub domain: Option<DomainId>,
    pub source_event_ids: Vec<EventId>,
    pub taint: Taint,
    pub now_ms: Time,
}

/// Parameters for [`SkillBook::register_consolidated_procedure`].
#[derive(Debug, Clone)]
pub struct RegisterProcedureParams {
    pub procedure_id: Option<EventId>,
    pub name: String,
    pub steps: Vec<String>,
    pub required_cap_ids: Vec<CapabilityId>,
    pub capability_schema_hash: CapabilitySchemaHash,
    pub audience: Audience,
    pub domain: Option<DomainId>,
    pub taint: Taint,
    pub now_ms: Time,
}

impl SkillBook {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns whether chat-habit bypass is permitted under `prior` (`false` by default).
    #[must_use]
    pub const fn chat_habit_bypass_allowed(prior: &Prior) -> bool {
        prior.allow_chat_habit_bypass
    }

    fn alloc_id(&mut self, hint: Option<EventId>) -> EventId {
        if let Some(id) = hint
            && id.as_u64() != 0
        {
            self.next_synthetic_id = self.next_synthetic_id.max(id.as_u64());
            return id;
        }
        self.next_synthetic_id = self.next_synthetic_id.saturating_add(1).max(9_000_000);
        EventId::new(self.next_synthetic_id)
    }

    /// Records a failure episode observation for future contrastive lesson extraction (`A-10`).
    pub fn record_failure_episode(&mut self, rec: FailureEpisodeRecord) {
        if self
            .failure_episodes
            .iter()
            .any(|existing| existing.episode_id == rec.episode_id)
        {
            return;
        }
        self.failure_episodes.push(rec);
    }

    /// Records a success episode observation eligible for contrastive pairing (`A-10`).
    pub fn record_success_episode(&mut self, rec: SuccessEpisodeRecord) {
        if self
            .success_episodes
            .iter()
            .any(|existing| existing.episode_id == rec.episode_id)
        {
            return;
        }
        self.success_episodes.push(rec);
    }

    /// Extracts contrastive lessons from clustered failure episodes + bounded contrasts (`A-10`, `14.1`).
    ///
    /// Requires at least `prior.lesson_min_episodes` matching failures with the same
    /// normalized error signature (`single error` never produces a lesson), and attaches
    /// at most `prior.lesson_max_contrasts` matching success contrasts.
    pub fn cluster_contrastive_lessons(
        &mut self,
        now_ms: Time,
        prior: &Prior,
        switches: &Switches,
    ) -> Vec<LessonEntry> {
        if !switches.consolidation_and_merge {
            return Vec::new();
        }
        let mut by_sig: BTreeMap<String, Vec<FailureEpisodeRecord>> = BTreeMap::new();
        for f in &self.failure_episodes {
            if f.signature_key.trim().is_empty() {
                continue;
            }
            by_sig
                .entry(f.signature_key.clone())
                .or_default()
                .push(f.clone());
        }

        let mut created = Vec::new();
        let success_snapshot = self.success_episodes.clone();
        for (sig, mut failures) in by_sig {
            failures.sort_by_key(|f| (f.timestamp_ms.as_millis(), f.episode_id.as_u64()));
            if failures.len() < prior.lesson_min_episodes {
                continue;
            }
            let failure_ids: Vec<EventId> = failures.iter().map(|f| f.episode_id).collect();
            // Avoid duplicating an existing active lesson for the exact same failure signature.
            if self
                .lessons
                .values()
                .any(|l| l.active && l.failure_signature == sig && l.failure_ids == failure_ids)
            {
                continue;
            }

            // Pick up to `lesson_max_contrasts` matching successful traces.
            let mut contrasts: Vec<&SuccessEpisodeRecord> = success_snapshot
                .iter()
                .filter(|s| {
                    s.signature_key == sig
                        || failures.first().is_some_and(|f| {
                            (f.capability_id.is_some() && f.capability_id == s.capability_id)
                                || (f.domain.is_some() && f.domain == s.domain)
                        })
                })
                .collect();
            contrasts.sort_by_key(|s| (s.timestamp_ms.as_millis(), s.episode_id.as_u64()));
            contrasts.truncate(prior.lesson_max_contrasts);

            let mut taint = Taint::Trusted;
            let mut actor_set: BTreeSet<ActorId> = BTreeSet::new();
            for f in &failures {
                taint = taint.combine(f.taint);
                for a in &f.actors {
                    actor_set.insert(a.clone());
                }
            }
            let mut contrastive_success_ids = Vec::with_capacity(contrasts.len());
            for c in &contrasts {
                taint = taint.combine(c.taint);
                contrastive_success_ids.push(c.episode_id);
                for a in &c.actors {
                    actor_set.insert(a.clone());
                }
            }

            let mut credence = BetaCredence::from_prior(1.0, 1.0, now_ms);
            if switches.credence {
                let supporting = failures.len() + contrastive_success_ids.len();
                credence.alpha = 1.0 + (supporting as f64);
                credence.last_updated_ms = now_ms;
            }

            let domain = failures.iter().find_map(|f| f.domain.clone());
            let lesson_id = self.alloc_id(failures.first().map(|f| {
                // Keep synthetic lesson id distinct from episode id if possible.
                EventId::new(f.episode_id.as_u64().saturating_add(100_000_000))
            }));
            let entry = LessonEntry {
                lesson_id,
                pattern: format!("avoid_or_guard:{sig}"),
                failure_signature: sig,
                domain,
                failure_ids,
                contrastive_success_ids,
                actors: actor_set.into_iter().collect(),
                credence,
                taint,
                created_at_ms: now_ms,
                updated_at_ms: now_ms,
                active: true,
            };
            self.lessons.insert(lesson_id, entry.clone());
            created.push(entry);
        }
        created
    }

    /// Records a lesson explicitly proposed by `ThoughtAct::Consolidate { output: Lesson { .. } }`,
    /// enforcing the minimum-failure-episodes and bounded-contrasts invariants (`14.1`).
    pub fn record_consolidated_lesson(
        &mut self,
        params: RecordLessonParams,
        prior: &Prior,
        switches: &Switches,
    ) -> Option<LessonEntry> {
        let RecordLessonParams {
            lesson_id,
            pattern,
            failure_signature,
            failure_ids,
            mut contrastive_success_ids,
            domain,
            taint,
            now_ms,
        } = params;
        if !switches.consolidation_and_merge || failure_ids.len() < prior.lesson_min_episodes {
            return None;
        }
        contrastive_success_ids.truncate(prior.lesson_max_contrasts);
        let id = self.alloc_id(lesson_id);
        let mut credence = BetaCredence::from_prior(1.0, 1.0, now_ms);
        if switches.credence {
            credence.alpha = 1.0 + ((failure_ids.len() + contrastive_success_ids.len()) as f64);
            credence.last_updated_ms = now_ms;
        }
        let entry = LessonEntry {
            lesson_id: id,
            pattern,
            failure_signature,
            domain,
            failure_ids,
            contrastive_success_ids,
            actors: Vec::new(),
            credence,
            taint,
            created_at_ms: now_ms,
            updated_at_ms: now_ms,
            active: true,
        };
        self.lessons.insert(id, entry.clone());
        Some(entry)
    }

    /// Updates a lesson's Beta credence from later outcome evidence.
    pub fn update_lesson_credence(
        &mut self,
        lesson_id: EventId,
        supported: bool,
        weight: f64,
        now_ms: Time,
        prior: &Prior,
        switches: &Switches,
    ) -> Option<f64> {
        let lesson = self.lessons.get_mut(&lesson_id)?;
        if !switches.credence || !weight.is_finite() || weight <= 0.0 {
            return Some(lesson.credence.expected());
        }
        let (pos, neg) = if supported {
            (weight, 0.0)
        } else {
            (0.0, weight)
        };
        lesson.credence.apply_evidence(
            pos,
            neg,
            now_ms,
            prior.credence_lambda_domain,
            prior.credence_period_ms,
        );
        lesson.updated_at_ms = now_ms;
        Some(lesson.credence.expected())
    }

    /// Extracts a new procedure from a reached cross-place goal chain (`A-11`, `14.2`).
    ///
    /// Returns `None` if consolidation is disabled, steps are empty, schema mismatches current
    /// capability descriptors, or fewer than 2 distinct places were traversed (`cross-place` guard).
    pub fn extract_procedure_from_goal_chain(
        &mut self,
        params: ExtractProcedureParams,
        capability_catalog: &BTreeMap<CapabilityId, CapabilityCatalogEntry>,
        prior: &Prior,
        switches: &Switches,
    ) -> Option<TrackedProcedure> {
        let ExtractProcedureParams {
            procedure_id,
            source_goal_id,
            name,
            steps,
            required_cap_ids,
            capability_schema_hash,
            places,
            actors,
            audience,
            domain,
            source_event_ids,
            taint,
            now_ms,
        } = params;
        if !switches.consolidation_and_merge || steps.is_empty() {
            return None;
        }
        // Anti-bypass guard: chat-only 1-place sequences cannot form a skill shortcut unless
        // `allow_chat_habit_bypass` is explicitly enabled and capabilities are non-empty.
        let distinct_places: BTreeSet<&str> = places.iter().map(|p| p.as_str()).collect();
        if distinct_places.len() < 2 {
            return None;
        }
        if required_cap_ids.is_empty() && !Self::chat_habit_bypass_allowed(prior) {
            return None;
        }

        // Verify capability schemas exist and match when catalog entries are known.
        if !schema_matches_catalog(
            &required_cap_ids,
            &capability_schema_hash,
            capability_catalog,
        ) {
            return None;
        }

        let id = self.alloc_id(procedure_id);
        let proc_entry = TrackedProcedure {
            procedure_id: id,
            name,
            steps,
            required_cap_ids,
            capability_schema_hash,
            domain,
            places,
            actors,
            audience,
            owner_signed: false,
            embedding: None,
            source_goal_id,
            source_event_ids,
            taint,
            state: ProcedureLifecycleState::Active,
            retired_reason: None,
            retired_at_ms: None,
            created_at_ms: now_ms,
            updated_at_ms: now_ms,
            alpha: 1.0,
            beta: 1.0,
            trial_count: 0,
            success_count: 0,
            cusum_baseline_p0: None,
            cusum_stat: 0.0,
            warn_count: 0,
        };
        self.procedures.insert(id, proc_entry.clone());
        Some(proc_entry)
    }

    /// Registers a `ThoughtAct::Consolidate { output: Procedure { .. } }` procedure (`14.2`).
    pub fn register_consolidated_procedure(
        &mut self,
        params: RegisterProcedureParams,
        capability_catalog: &BTreeMap<CapabilityId, CapabilityCatalogEntry>,
        prior: &Prior,
        switches: &Switches,
    ) -> Option<TrackedProcedure> {
        let RegisterProcedureParams {
            procedure_id,
            name,
            steps,
            required_cap_ids,
            capability_schema_hash,
            audience,
            domain,
            taint,
            now_ms,
        } = params;
        if !switches.consolidation_and_merge || steps.is_empty() {
            return None;
        }
        if required_cap_ids.is_empty() && !Self::chat_habit_bypass_allowed(prior) {
            return None;
        }
        let matches = schema_matches_catalog(
            &required_cap_ids,
            &capability_schema_hash,
            capability_catalog,
        );
        let id = self.alloc_id(procedure_id);
        let (state, retired_reason, retired_at_ms) = if matches {
            (ProcedureLifecycleState::Active, None, None)
        } else {
            (
                ProcedureLifecycleState::Retired,
                Some("capability_schema_drift".to_owned()),
                Some(now_ms),
            )
        };
        let proc_entry = TrackedProcedure {
            procedure_id: id,
            name,
            steps,
            required_cap_ids,
            capability_schema_hash,
            domain,
            places: Vec::new(),
            actors: Vec::new(),
            audience,
            owner_signed: false,
            embedding: None,
            source_goal_id: None,
            source_event_ids: Vec::new(),
            taint,
            state,
            retired_reason,
            retired_at_ms,
            created_at_ms: now_ms,
            updated_at_ms: now_ms,
            alpha: 1.0,
            beta: 1.0,
            trial_count: 0,
            success_count: 0,
            cusum_baseline_p0: None,
            cusum_stat: 0.0,
            warn_count: 0,
        };
        self.procedures.insert(id, proc_entry.clone());
        Some(proc_entry)
    }

    /// Retires (NEVER deletes) active procedures whose required capabilities or schema hash drifted (`R-40`).
    pub fn invalidate_procedures_on_schema_change(
        &mut self,
        body_id: Option<&BodyId>,
        capability_catalog: &BTreeMap<CapabilityId, CapabilityCatalogEntry>,
        now_ms: Time,
    ) -> Vec<EventId> {
        let _ = body_id;
        let mut newly_retired = Vec::new();
        for proc_entry in self.procedures.values_mut() {
            if proc_entry.state == ProcedureLifecycleState::Retired {
                continue;
            }
            if !schema_matches_catalog(
                &proc_entry.required_cap_ids,
                &proc_entry.capability_schema_hash,
                capability_catalog,
            ) {
                proc_entry.state = ProcedureLifecycleState::Retired;
                proc_entry.retired_reason = Some("capability_schema_drift".to_owned());
                proc_entry.retired_at_ms = Some(now_ms);
                proc_entry.updated_at_ms = now_ms;
                newly_retired.push(proc_entry.procedure_id);
            }
        }
        newly_retired
    }

    /// Updates discounted Beta reliability and downward Bernoulli CUSUM from a procedure receipt (`14.3`).
    ///
    /// Canceled (`CanceledByCore`) and externally preempted (`PreemptedExternal`) outcomes are
    /// strictly ignored (`0` reliability penalty, `0` CUSUM drift).
    /// Retired procedures stay retained (`state == Retired`) and are never deleted.
    pub fn monitor_procedure_outcome(
        &mut self,
        procedure_id: EventId,
        outcome: ReceiptOutcome,
        termination_reason: Option<TerminationReason>,
        now_ms: Time,
        prior: &Prior,
    ) -> ProcedureMonitorOutcome {
        if matches!(
            termination_reason,
            Some(TerminationReason::CanceledByCore | TerminationReason::PreemptedExternal)
        ) {
            return ProcedureMonitorOutcome::IgnoredCanceledOrPreempted;
        }
        let Some(proc_entry) = self.procedures.get_mut(&procedure_id) else {
            return ProcedureMonitorOutcome::IgnoredNotFound;
        };
        if proc_entry.state == ProcedureLifecycleState::Retired {
            return ProcedureMonitorOutcome::IgnoredAlreadyRetired;
        }

        if matches!(outcome, ReceiptOutcome::Held | ReceiptOutcome::Unknown) {
            return ProcedureMonitorOutcome::IgnoredCanceledOrPreempted;
        }
        let succeeded = matches!(outcome, ReceiptOutcome::Done);
        let gamma = prior.proc_gamma_beta.clamp(0.0, 1.0);
        proc_entry.alpha =
            1.0 + gamma * (proc_entry.alpha - 1.0) + if succeeded { 1.0 } else { 0.0 };
        proc_entry.beta = 1.0 + gamma * (proc_entry.beta - 1.0) + if succeeded { 0.0 } else { 1.0 };
        proc_entry.trial_count = proc_entry.trial_count.saturating_add(1);
        if succeeded {
            proc_entry.success_count = proc_entry.success_count.saturating_add(1);
        }

        let previous_state = proc_entry.state;
        let posterior_mean = proc_entry.posterior_mean();

        // Phase 1: lock baseline p0 once at least `proc_cusum_n_min_lock` trials are observed.
        if proc_entry.cusum_baseline_p0.is_none()
            && proc_entry.trial_count >= (prior.proc_cusum_n_min_lock as u64)
        {
            let emp = (proc_entry.success_count as f64) / (proc_entry.trial_count as f64);
            // Clamp baseline away from trivial 0 so downward drift remains sensitive.
            proc_entry.cusum_baseline_p0 = Some(emp.clamp(0.5, 0.99));
            proc_entry.cusum_stat = 0.0;
        } else if let Some(p0) = proc_entry.cusum_baseline_p0 {
            let y_t = if succeeded { 1.0 } else { 0.0 };
            let increment = p0 - prior.proc_cusum_k_drift - y_t;
            proc_entry.cusum_stat = (proc_entry.cusum_stat + increment).max(0.0);

            if proc_entry.cusum_stat >= prior.proc_cusum_h_retire
                || posterior_mean < prior.proc_theta_retire
            {
                proc_entry.state = ProcedureLifecycleState::Retired;
                let reason = if proc_entry.cusum_stat >= prior.proc_cusum_h_retire {
                    "cusum_accuracy_decay"
                } else {
                    "posterior_reliability_below_retire"
                };
                proc_entry.retired_reason = Some(reason.to_owned());
                proc_entry.retired_at_ms = Some(now_ms);
            } else if proc_entry.cusum_stat >= prior.proc_cusum_h_warn {
                proc_entry.state = ProcedureLifecycleState::Warned;
                proc_entry.warn_count = proc_entry.warn_count.saturating_add(1);
            } else {
                proc_entry.state = ProcedureLifecycleState::Active;
            }
        }
        proc_entry.updated_at_ms = now_ms;

        ProcedureMonitorOutcome::Updated {
            procedure_id,
            previous_state,
            new_state: proc_entry.state,
            posterior_mean,
            cusum_stat: proc_entry.cusum_stat,
            reason: proc_entry.retired_reason.clone(),
        }
    }

    /// Exports active and retired procedures into `ContextPlan` tool-selection entries (`R-38`, `R-40`).
    #[must_use]
    pub fn context_entries(&self) -> Vec<ProcedureEntry> {
        self.procedures
            .values()
            .map(TrackedProcedure::to_context_entry)
            .collect()
    }

    /// Removes actor-scoped episodes, lessons, and procedures for `hard_forget`.
    pub fn remove_actor(&mut self, actor: &ActorId) {
        self.failure_episodes
            .retain(|rec| !rec.actors.iter().any(|a| a == actor));
        self.success_episodes
            .retain(|rec| !rec.actors.iter().any(|a| a == actor));
        self.lessons
            .retain(|_, l| !l.actors.iter().any(|a| a == actor));
        self.procedures
            .retain(|_, p| !p.actors.iter().any(|a| a == actor));
    }
}

/// Checks whether all required capabilities exist in `catalog` (when non-empty) and whether
/// either a direct schema hash matches any required capability or the combined canonical digest matches.
#[must_use]
pub fn schema_matches_catalog(
    required_cap_ids: &[CapabilityId],
    procedure_hash: &CapabilitySchemaHash,
    catalog: &BTreeMap<CapabilityId, CapabilityCatalogEntry>,
) -> bool {
    if required_cap_ids.is_empty() || catalog.is_empty() {
        return true;
    }
    let mut pairs: Vec<(&str, &str)> = Vec::with_capacity(required_cap_ids.len());
    for cap_id in required_cap_ids {
        let Some(entry) = catalog.get(cap_id) else {
            return false;
        };
        if !entry.active {
            return false;
        }
        pairs.push((
            cap_id.as_str(),
            entry.descriptor.capability_schema_hash.as_str(),
        ));
    }
    if pairs.len() == 1 {
        return procedure_hash.as_str() == pairs[0].1;
    }
    pairs.sort_unstable();
    let mut combined = String::new();
    for (id, h) in pairs {
        combined.push_str(id);
        combined.push(':');
        combined.push_str(h);
        combined.push(';');
    }
    let digest = crate::blake3::to_hex(&crate::blake3::hash(combined.as_bytes()));
    procedure_hash.as_str() == digest
}

/// Computes canonical [`CapabilitySchemaHash`] over a sequence of `(CapabilityId, schema_hash)` entries.
#[must_use]
pub fn compute_procedure_schema_hash<'a>(
    caps: impl IntoIterator<Item = (&'a CapabilityId, &'a CapabilitySchemaHash)>,
) -> CapabilitySchemaHash {
    let mut items: Vec<(&str, &str)> = caps
        .into_iter()
        .map(|(id, h)| (id.as_str(), h.as_str()))
        .collect();
    if items.len() == 1 {
        return CapabilitySchemaHash::new(items[0].1.to_owned());
    }
    items.sort_unstable();
    let mut combined = String::new();
    for (id, h) in items {
        combined.push_str(id);
        combined.push(':');
        combined.push_str(h);
        combined.push(';');
    }
    CapabilitySchemaHash::new(crate::blake3::to_hex(&crate::blake3::hash(
        combined.as_bytes(),
    )))
}
