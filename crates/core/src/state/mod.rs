//! Slim Core V2 projection state rebuilt deterministically from the canonical event log.
//!
//! Retains all deterministic books (`BeliefBook`, `ExpectationBook`, `EpisodeBook`, `SkillBook`,
//! `PerceptionLedger`, `MemoryEntry` index) plus per-place neural recurrent state (`place_memory`
//! `64d` `h_t`) and bounded per-place online adaptation equalizers (`place_adapt`).

pub mod snapshot;

use std::collections::{BTreeMap, BTreeSet};

use monoagens_protocol::{
    ActionId, ActorId, Audience, BodyId, BodyStatus, CapabilityDescriptor, CapabilityId,
    ChannelId, DeclassToken, DomainId, EventId, ExpectKind, HAct, OpaquePayload, OwnerSigned,
    PlaceId, Percept, Receipt, ReceiptOutcome, Taint, TerminationReason, Time, VisibleScope,
};
use serde::{Deserialize, Serialize};

use crate::activation::{ActivationTrace, MS_PER_HOUR};
use crate::beliefs::{BeliefBook, BeliefLogEntry};
use crate::config::CoreConfig;
use crate::consolidation::{
    EpisodeAdmitParams, EpisodeBook, EpisodeState, PendingSleepRun, ProcedureLifecycleState,
    SkillBook, TrackedProcedure,
};
use crate::context::{
    AutoRecallTriggerState, CapabilityCatalogEntry, CoreSelfCard, IdentityAnchor,
    PrincipleDescriptor, ProcedureEntry,
};
use crate::event::{
    DecisionOutcome, EventDetail, EventKind, EventRecord, ExpectationLifecycleState, LogSeq,
};
use crate::expectations::{
    EffectPredictionSpec, ExpectationBook, ExpectationRecord, MonitorState, ResponsePredictionSpec,
    is_terminal_state, max_matching_cosine,
};
use crate::gate::{
    CandidateMemory, CommitmentInputs, DecisionReason, DecisionTrace, ExpectationCue,
    FrozenDialogueActHead, GateOutcome, OpenWait, PlaceHistoryEntry, PromiseSummary, decide_gate,
};
use crate::ledger::PerceptionLedger;
use crate::neural::{
    AdaptConfig, AdaptFeedback, DEFAULT_H_DIM, NeuralModel, PlaceAdaptState,
    extract_embedding_for_model, extract_embedding_for_model_config, extract_features_from_state,
    forward_step,
};
use crate::privacy::build_visible_scope;
use crate::recall::{ImportanceInputs, MemoryBasis, MemoryEntry};

const MAX_PLACE_HISTORY_ENTRIES: usize = 64;
const MAX_TALK_TURNS_PER_PLACE: usize = 512;
pub(crate) const DEFAULT_DEG_ALLOC_THRESHOLD: f64 = 0.50;
const DEFAULT_EXCHANGE_LISTENER_MAX: f64 = 3.0;
const DEFAULT_LATENCY_DECAY_RHO: f64 = 0.995;

/// Tracked in-flight action awaiting a terminal receipt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InFlightAction {
    pub action_id: ActionId,
    pub capability_id: CapabilityId,
    pub places: Vec<PlaceId>,
    pub dispatched_at_ms: Time,
    pub event_id: EventId,
}

/// Non-retrying approval state bound to `H_act` when an action returns `held` (`R-42`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingApprovalEntry {
    pub action_id: ActionId,
    pub capability_id: CapabilityId,
    pub h_act: Option<HAct>,
    pub held_at_ms: Time,
}

/// Per-actor reliability prior and teacher weight supplied by Gateway.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActorFactsEntry {
    pub alpha_0: f64,
    pub beta_0: f64,
    pub teacher_weight: f64,
    pub trusted_origin: bool,
    #[serde(default)]
    pub automated_peer: bool,
}

/// Per-wake projection snapshot recorded at wake start for deterministic crash/tail consistency.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WakeSnapshotRecord {
    pub visible_ids: BTreeSet<EventId>,
    pub visible_scope: VisibleScope,
    pub disclosed_capabilities: BTreeSet<CapabilityId>,
    pub turn_allocated: bool,
    pub wake_place: PlaceId,
    pub anchor_check_due: bool,
}

/// Complete deterministic projection state rebuilt from snapshot + event tail.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CoreState {
    pub last_seq: LogSeq,
    pub epoch: u64,
    pub next_action_seq: u64,
    pub last_timestamp_ms: Option<Time>,
    pub identity: Option<OwnerSigned<OpaquePayload>>,
    pub body_states: BTreeMap<BodyId, BodyStatus>,
    pub actor_facts: BTreeMap<ActorId, ActorFactsEntry>,
    pub in_flight_actions: BTreeMap<ActionId, InFlightAction>,
    pub pending_approvals: BTreeMap<ActionId, PendingApprovalEntry>,
    pub terminal_receipts: BTreeMap<ActionId, Receipt>,
    pub unconsumed_inputs: BTreeMap<EventId, EventRecord>,

    pub soul_actor_id: ActorId,
    pub place_presence: BTreeMap<PlaceId, ActivationTrace>,
    pub actor_exchanges: BTreeMap<ActorId, ActivationTrace>,
    pub place_history: BTreeMap<PlaceId, Vec<PlaceHistoryEntry>>,
    pub place_participants: BTreeMap<PlaceId, BTreeSet<ActorId>>,
    pub talk_turns: BTreeMap<PlaceId, Vec<(Time, bool)>>,

    /// Per-place `64d` recurrent neural working memory `h_t`.
    pub place_memory: BTreeMap<PlaceId, Vec<f32>>,
    /// Per-place bounded Warm-Start UCB online adaptation equalizer state.
    pub place_adapt: BTreeMap<PlaceId, PlaceAdaptState>,

    pub memory_candidates: Vec<CandidateMemory>,
    pub open_promises: BTreeMap<EventId, PromiseSummary>,
    pub expectation_cues: Vec<ExpectationCue>,
    pub event_authors: BTreeMap<EventId, ActorId>,
    pub active_chains: BTreeSet<EventId>,
    pub dialogue_act_head: Option<FrozenDialogueActHead>,
    pub daily_budget_used: f64,
    pub daily_budget_limit: f64,
    pub unconsolidated_count: usize,
    pub last_sleep_ms: Option<Time>,

    pub open_waits: BTreeMap<EventId, OpenWait>,
    pub memory_entries: BTreeMap<EventId, MemoryEntry>,
    pub logged_declass_tokens: Vec<OwnerSigned<DeclassToken>>,

    pub expectation_book: ExpectationBook,
    pub capability_descriptors: BTreeMap<CapabilityId, CapabilityDescriptor>,
    pub startup_recovery_pending: bool,

    pub identity_anchor: Option<OwnerSigned<IdentityAnchor>>,
    pub principles: Vec<OwnerSigned<PrincipleDescriptor>>,
    pub capability_catalog: BTreeMap<CapabilityId, CapabilityCatalogEntry>,
    pub procedures: BTreeMap<EventId, ProcedureEntry>,
    pub knowledge_cutoff: Option<String>,
    pub domain_calibration_offsets: BTreeMap<DomainId, f64>,
    pub active_wake_snapshots: BTreeMap<EventId, WakeSnapshotRecord>,
    pub auto_recall_states: BTreeMap<EventId, AutoRecallTriggerState>,
    pub recent_receipt_cues: Vec<String>,
    pub wake_count_since_anchor_check: u64,
    pub anchor_check_pending_post_sleep: bool,
    pub last_sleep_seq: LogSeq,

    pub reliability_alpha: f64,
    pub reliability_beta: f64,
    pub latency_samples_ms: Vec<u64>,
    pub turn_allocated_by_trigger: BTreeMap<EventId, bool>,
    pub calibration_samples: u64,
    pub calibration_brier_sum: f64,
    pub principle_alpha: f64,
    pub principle_beta: f64,
    pub contradiction_flags: u64,

    pub belief_book: BeliefBook,
    pub episode_book: EpisodeBook,
    pub pending_sleep_run: Option<PendingSleepRun>,
    pub last_global_activity_ms: Option<Time>,

    pub skill_book: SkillBook,
    pub ledger: PerceptionLedger,
    pub procedure_links: BTreeMap<ActionId, EventId>,
    pub recalibration_pending: bool,

    pub recorded_config: Option<CoreConfig>,
    pub config_loaded_count: u64,
    #[serde(default)]
    pub decision_neural_hints: BTreeMap<EventId, (Option<f32>, Option<f32>)>,
    /// Per-place count of consecutive automated-actor/soul turns (resets to 0 on non-automated turn).
    #[serde(default)]
    pub consecutive_automated_turns_by_place: BTreeMap<PlaceId, u32>,
}

impl Default for CoreState {
    fn default() -> Self {
        Self::new()
    }
}

impl CoreState {
    /// Creates an empty initial V2 projection state.
    #[must_use]
    pub fn new() -> Self {
        Self {
            last_seq: LogSeq::ZERO,
            epoch: 0,
            next_action_seq: 1,
            last_timestamp_ms: None,
            identity: None,
            body_states: BTreeMap::new(),
            actor_facts: BTreeMap::new(),
            in_flight_actions: BTreeMap::new(),
            pending_approvals: BTreeMap::new(),
            terminal_receipts: BTreeMap::new(),
            unconsumed_inputs: BTreeMap::new(),
            soul_actor_id: ActorId::new("soul"),
            place_presence: BTreeMap::new(),
            actor_exchanges: BTreeMap::new(),
            place_history: BTreeMap::new(),
            place_participants: BTreeMap::new(),
            talk_turns: BTreeMap::new(),
            place_memory: BTreeMap::new(),
            place_adapt: BTreeMap::new(),
            memory_candidates: Vec::new(),
            open_promises: BTreeMap::new(),
            expectation_cues: Vec::new(),
            event_authors: BTreeMap::new(),
            active_chains: BTreeSet::new(),
            dialogue_act_head: None,
            daily_budget_used: 0.0,
            daily_budget_limit: 1.0,
            unconsolidated_count: 0,
            last_sleep_ms: None,
            open_waits: BTreeMap::new(),
            memory_entries: BTreeMap::new(),
            logged_declass_tokens: Vec::new(),
            expectation_book: ExpectationBook::new(),
            capability_descriptors: BTreeMap::new(),
            startup_recovery_pending: false,
            identity_anchor: None,
            principles: Vec::new(),
            capability_catalog: BTreeMap::new(),
            procedures: BTreeMap::new(),
            knowledge_cutoff: None,
            domain_calibration_offsets: BTreeMap::new(),
            active_wake_snapshots: BTreeMap::new(),
            auto_recall_states: BTreeMap::new(),
            recent_receipt_cues: Vec::new(),
            wake_count_since_anchor_check: 0,
            anchor_check_pending_post_sleep: false,
            last_sleep_seq: LogSeq::ZERO,
            reliability_alpha: 1.0,
            reliability_beta: 1.0,
            latency_samples_ms: Vec::new(),
            turn_allocated_by_trigger: BTreeMap::new(),
            calibration_samples: 0,
            calibration_brier_sum: 0.0,
            principle_alpha: 1.0,
            principle_beta: 1.0,
            contradiction_flags: 0,
            belief_book: BeliefBook::new(),
            episode_book: EpisodeBook::new(),
            pending_sleep_run: None,
            last_global_activity_ms: None,
            skill_book: SkillBook::new(),
            ledger: PerceptionLedger::new(),
            procedure_links: BTreeMap::new(),
            recalibration_pending: false,
            recorded_config: None,
            config_loaded_count: 0,
            decision_neural_hints: BTreeMap::new(),
            consecutive_automated_turns_by_place: BTreeMap::new(),
        }
    }

    /// Returns the effective [`AdaptConfig`] projected from the latest `ConfigLoaded` record
    /// (or [`AdaptConfig::default`] before any config event has been applied).
    #[must_use]
    pub fn effective_adapt_config(&self) -> AdaptConfig {
        self.recorded_config
            .as_ref()
            .map_or_else(AdaptConfig::default, CoreConfig::effective_adapt_config)
    }

    /// Computes a deterministic bounded 2-scalar numeric `context_hint` in `[-1.0, 1.0]` for
    /// FiLM online adaptation from observable place participants and talk turns (`C-3`).
    #[must_use]
    pub fn compute_place_context_hint(&self, place: &PlaceId) -> [f32; 2] {
        let non_soul_count = self
            .place_participants
            .get(place)
            .map_or(1usize, |set| {
                set.iter().filter(|a| *a != &self.soul_actor_id).count()
            });
        let dyadic_vs_group = if non_soul_count <= 1 {
            0.8_f32
        } else {
            -((1.0_f32 - 1.0_f32 / (non_soul_count as f32)).clamp(0.2, 1.0))
        };

        let soul_share_centered = match self.talk_turns.get(place) {
            Some(turns) if !turns.is_empty() => {
                let soul_turns = turns.iter().filter(|(_, is_soul)| *is_soul).count() as f32;
                let total = turns.len() as f32;
                let share = (soul_turns / total).clamp(0.0, 1.0);
                // Shift by 0.4 instead of 0.5 so balanced 50/50 conversations also emit non-zero signal.
                ((share - 0.4) * 2.0).clamp(-1.0, 1.0)
            }
            _ => -0.5_f32,
        };

        [dyadic_vs_group, soul_share_centered]
    }

    /// Maps a log [`EventId`] to the key used for `(seed, event_id)` seeded draws, subtracting
    /// bookkeeping `config_loaded_count` so config records never shift RNG sequences.
    #[must_use]
    pub const fn draw_key(&self, rec_id: EventId) -> EventId {
        EventId::new(rec_id.as_u64().saturating_sub(self.config_loaded_count))
    }

    /// Synchronizes structural limits from [`CoreConfig`].
    pub fn sync_config(&mut self, config: &CoreConfig) {
        self.daily_budget_limit = config.daily_budget_limit;
    }

    /// Registers or replaces an owner-signed behavioral [`PrincipleDescriptor`].
    pub fn register_principle(&mut self, principle: OwnerSigned<PrincipleDescriptor>) {
        if let Some(existing) = self
            .principles
            .iter_mut()
            .find(|p| p.payload.id == principle.payload.id)
        {
            *existing = principle;
        } else {
            self.principles.push(principle);
        }
    }

    /// Registers or updates a capability catalog entry (`Tool RAG` disclosure store).
    pub fn register_capability_item(&mut self, entry: CapabilityCatalogEntry) {
        self.capability_descriptors
            .insert(entry.descriptor.id.clone(), entry.descriptor.clone());
        self.capability_catalog
            .insert(entry.descriptor.id.clone(), entry);
    }

    /// Registers or updates a procedural skill entry for progressive `Tool RAG`.
    pub fn register_procedure(&mut self, proc_entry: ProcedureEntry) {
        self.procedures.insert(proc_entry.procedure_id, proc_entry);
    }

    pub(crate) fn push_receipt_cue(&mut self, cue: String) {
        self.recent_receipt_cues.push(cue);
        if self.recent_receipt_cues.len() > 16 {
            let drain = self.recent_receipt_cues.len() - 16;
            self.recent_receipt_cues.drain(0..drain);
        }
    }

    /// Returns `true` if `actor` is the soul itself or an edge-attested `automated_peer`.
    #[must_use]
    pub fn is_automated_actor(&self, actor: &ActorId) -> bool {
        actor == &self.soul_actor_id
            || self
                .actor_facts
                .get(actor)
                .is_some_and(|entry| entry.automated_peer)
    }

    /// Returns `true` when `actor` is an `automated_peer` and the place has already seen
    /// `consecutive_automated_turns >= 2`, suppressing unconditional `turn_allocated` bypass.
    #[must_use]
    pub fn is_peer_turn_allocation_suppressed(&self, actor: &ActorId, place: &PlaceId) -> bool {
        let is_peer = self
            .actor_facts
            .get(actor)
            .is_some_and(|entry| entry.automated_peer);
        if !is_peer {
            return false;
        }
        self.consecutive_automated_turns_by_place
            .get(place)
            .copied()
            .unwrap_or(0)
            >= 2
    }

    /// Looks up whether `cause` (or any of its `caused_by` ancestors) was marked `turn_allocated`.
    #[must_use]
    pub fn lookup_turn_allocated(&self, cause: &EventRecord) -> bool {
        if let Some(&alloc) = self.turn_allocated_by_trigger.get(&cause.id) {
            return alloc;
        }
        for &cid in &cause.caused_by {
            if let Some(&alloc) = self.turn_allocated_by_trigger.get(&cid) {
                return alloc;
            }
        }
        true
    }

    /// Returns all registered capability catalog entries, synthesizing defaults for body-only descriptors.
    #[must_use]
    pub fn effective_capability_catalog(&self) -> Vec<CapabilityCatalogEntry> {
        let mut out = Vec::new();
        for (id, desc) in &self.capability_descriptors {
            if let Some(entry) = self.capability_catalog.get(id) {
                out.push(entry.clone());
            } else {
                out.push(CapabilityCatalogEntry {
                    descriptor: desc.clone(),
                    is_static_core: false,
                    owner_signed: false,
                    active: true,
                    audience: Audience::default(),
                    embedding: None,
                });
            }
        }
        for (id, entry) in &self.capability_catalog {
            if !self.capability_descriptors.contains_key(id) {
                out.push(entry.clone());
            }
        }
        out
    }

    /// Builds Core's derived 4-pillar `self_card` snapshot (`7.1`).
    #[must_use]
    pub fn build_self_card(&self) -> CoreSelfCard {
        let mut card = self.ledger.build_self_card();
        if self.ledger.capabilities.is_empty()
            && (self.reliability_alpha > 1.0 || self.reliability_beta > 1.0)
        {
            card.reliability_alpha = self.reliability_alpha;
            card.reliability_beta = self.reliability_beta;
        }
        if card.mean_delivery_latency_ms == 0 && !self.latency_samples_ms.is_empty() {
            card.mean_delivery_latency_ms = self.latency_samples_ms.iter().sum::<u64>()
                / (self.latency_samples_ms.len() as u64);
        }
        if self.ledger.calibration.samples == 0 && self.calibration_samples > 0 {
            card.calibration_samples = self.calibration_samples;
            card.calibration_brier = self.calibration_brier_sum / (self.calibration_samples as f64);
        }
        if self.ledger.daily_budget_limit == 0.0 && self.daily_budget_limit > 0.0 {
            card.budget_headroom_ratio =
                (1.0 - (self.daily_budget_used / self.daily_budget_limit)).clamp(0.0, 1.0);
        }
        if self.ledger.principle_alpha <= 1.0
            && self.ledger.principle_beta <= 1.0
            && (self.principle_alpha > 1.0 || self.principle_beta > 1.0)
        {
            card.principle_alpha = self.principle_alpha;
            card.principle_beta = self.principle_beta;
        }
        card.contradiction_flags = card.contradiction_flags.max(self.contradiction_flags);
        card
    }

    pub(crate) fn sync_expectation_entry(&mut self, exp_id: EventId) {
        if let Some(exp) = self.expectation_book.expectations.get(&exp_id) {
            if exp.is_active() {
                if let Some(summary) = exp.to_promise_summary() {
                    self.open_promises.insert(exp_id, summary);
                } else {
                    self.open_promises.remove(&exp_id);
                }
            } else {
                self.open_promises.remove(&exp_id);
            }
            self.expectation_cues
                .retain(|c| c.expectation_id != Some(exp_id));
            if exp.is_active() && (!exp.condition_embeddings.is_empty() || !exp.people.is_empty()) {
                self.expectation_cues.push(ExpectationCue {
                    expectation_id: Some(exp_id),
                    audience: exp.audience.clone(),
                    counterparty: exp.people.first().cloned(),
                    answers_event: false,
                    embeddings: exp.condition_embeddings.clone(),
                });
            }
        } else {
            self.open_promises.remove(&exp_id);
            self.expectation_cues
                .retain(|c| c.expectation_id != Some(exp_id));
        }
        self.ledger.sync_open_requests(&self.expectation_book);
    }

    /// Builds a [`VisibleScope`] populated with valid, active, owner-signed logged [`DeclassToken`]s.
    #[must_use]
    pub fn visible_scope_for(
        &self,
        audience: Audience,
        target_place: Option<PlaceId>,
        now_ms: Time,
    ) -> VisibleScope {
        let expected_owner = self.identity.as_ref().map(|id| &id.signer);
        build_visible_scope(
            audience,
            target_place,
            &self.logged_declass_tokens,
            expected_owner,
            now_ms,
        )
    }

    /// Inserts or updates a provenance-bearing [`MemoryEntry`] in Core's memory projection.
    pub fn upsert_memory_entry(&mut self, entry: MemoryEntry) {
        if !entry.embeddings.is_empty() && !entry.erased {
            if let Some(existing) = self
                .memory_candidates
                .iter_mut()
                .find(|c| c.event_id == entry.event_id)
            {
                existing.audience = entry.audience.clone();
                existing.activation = entry.activation;
                existing.importance = entry.importance_score();
                existing.embeddings = entry.embeddings.clone();
            } else {
                self.memory_candidates.push(CandidateMemory {
                    event_id: entry.event_id,
                    audience: entry.audience.clone(),
                    activation: entry.activation,
                    importance: entry.importance_score(),
                    embeddings: entry.embeddings.clone(),
                });
            }
        }
        self.memory_entries.insert(entry.event_id, entry);
    }

    /// Marks `old_event_id` as superseded by `new_event_id`.
    pub fn supersede_memory(&mut self, old_event_id: EventId, new_event_id: EventId) {
        if let Some(entry) = self.memory_entries.get_mut(&old_event_id) {
            entry.superseded_by = Some(new_event_id);
        }
    }

    /// Evaluates the V2 Neural Cognitive Controller gate for a recorded `Percept` event WITHOUT
    /// mutating `self` (strictly reading pre-event `VisibleScope`-filtered state).
    #[must_use]
    pub fn appraise_percept_record(
        &self,
        percept: &EventRecord,
        config: &CoreConfig,
        model: Option<&NeuralModel>,
        backlog_queue_depth: usize,
    ) -> GateOutcome {
        let (addressees, named, reply_to, embeddings) = match &percept.detail {
            Some(EventDetail::PerceptMeta {
                addressees,
                named,
                reply_to,
                embeddings,
                ..
            }) => (
                addressees.as_slice(),
                named.as_slice(),
                *reply_to,
                embeddings.as_slice(),
            ),
            _ => (
                &[][..],
                &[][..],
                percept.caused_by.first().copied(),
                &[][..],
            ),
        };

        let visible_scope = self.visible_scope_for(
            percept.audience.clone(),
            Some(percept.place.clone()),
            percept.timestamp_ms,
        );

        let deg_soul = percept
            .numeric_features
            .deg_soul
            .or_else(|| {
                percept
                    .numeric_features
                    .orient
                    .iter()
                    .find(|o| o.actor == self.soul_actor_id)
                    .map(|o| o.score)
            })
            .unwrap_or(0.0);

        let turn_allocated = (f64::from(deg_soul) >= DEFAULT_DEG_ALLOC_THRESHOLD
            || addressees.contains(&self.soul_actor_id))
            && !self.is_peer_turn_allocation_suppressed(&percept.actor, &percept.place);
        let answers_cue = self.expectation_cues.iter().any(|c| {
            c.answers_event
                && visible_scope.allows(&c.audience, c.expectation_id, percept.timestamp_ms)
        });
        let answers_response = self
            .expectation_book
            .matches_any_response_prediction(percept, &self.soul_actor_id, reply_to)
            .is_some();
        let matches_goal = self.expectation_book.matches_goal_chain(percept).is_some();
        let matches_upper_condition =
            self.expectation_book
                .wake_gate
                .wake_gate_hi
                .is_some_and(|hi| {
                    self.expectation_book.expectations.values().any(|exp| {
                        exp.is_active()
                            && !exp.condition_embeddings.is_empty()
                            && percept.timestamp_ms <= exp.expiry_ms
                            && max_matching_cosine(embeddings, &exp.condition_embeddings)
                                .is_some_and(|s| s >= hi)
                    })
                });
        let fires_or_answers_expectation =
            answers_cue || answers_response || matches_goal || matches_upper_condition;
        let continues_active_chain = reply_to.is_some_and(|id| self.active_chains.contains(&id))
            || percept
                .caused_by
                .iter()
                .any(|id| self.active_chains.contains(id));

        let commitments = CommitmentInputs {
            turn_allocated,
            fires_or_answers_expectation,
            continues_active_chain,
        };

        let mut synthetic_percept = Percept::minimal(
            percept.actor.clone(),
            percept.place.clone(),
            percept.audience.clone(),
            percept.timestamp_ms,
            percept.payload.clone(),
        );
        synthetic_percept.taint = percept.taint;
        synthetic_percept.origin_actor = Some(percept.origin_actor.clone());
        synthetic_percept.addressees = addressees.to_vec();
        synthetic_percept.named = named.to_vec();
        synthetic_percept.reply_to = reply_to;
        synthetic_percept.embeddings = embeddings.to_vec();
        synthetic_percept.f_modal = percept.numeric_features.f_modal;

        let x_num = extract_features_from_state(
            self,
            &synthetic_percept,
            percept.timestamp_ms,
            backlog_queue_depth,
        );
        let d_emb = model.map_or(256, |m| m.config.d_in_emb);
        let e_256 = match model {
            Some(m) => extract_embedding_for_model_config(&synthetic_percept, &m.config),
            None => extract_embedding_for_model(&synthetic_percept, d_emb),
        };
        let h_dim = model.map_or(DEFAULT_H_DIM, |m| m.config.h_dim);
        let h_prev = self
            .place_memory
            .get(&percept.place)
            .filter(|v| v.len() == h_dim)
            .cloned()
            .unwrap_or_else(|| vec![0.0_f32; h_dim]);
        let adapt_cfg = config.effective_adapt_config();
        let decayed_adapt = self
            .place_adapt
            .get(&percept.place)
            .map(|st| st.decayed_at(percept.timestamp_ms, &adapt_cfg));

        decide_gate(
            self.draw_key(percept.id),
            config,
            model,
            &e_256,
            &x_num,
            &h_prev,
            decayed_adapt.as_ref(),
            commitments,
        )
    }

    /// Computes a non-waking recurrent memory step (`h_next`) for a `SelfCaused` percept (`C-9`)
    /// when a neural model is loaded, returning `None` under neutral fallback (`model = None`).
    #[must_use]
    pub fn step_self_caused_neural(
        &self,
        percept: &EventRecord,
        model: Option<&NeuralModel>,
    ) -> Option<DecisionTrace> {
        let m = model?;
        let embeddings = match &percept.detail {
            Some(EventDetail::PerceptMeta { embeddings, .. }) => embeddings.as_slice(),
            _ => &[][..],
        };
        let mut synthetic_percept = Percept::minimal(
            percept.actor.clone(),
            percept.place.clone(),
            percept.audience.clone(),
            percept.timestamp_ms,
            percept.payload.clone(),
        );
        synthetic_percept.taint = percept.taint;
        synthetic_percept.origin_actor = Some(percept.origin_actor.clone());
        synthetic_percept.embeddings = embeddings.to_vec();
        synthetic_percept.f_modal = percept.numeric_features.f_modal;

        let x_num = extract_features_from_state(self, &synthetic_percept, percept.timestamp_ms, 0);
        let e_256 = extract_embedding_for_model_config(&synthetic_percept, &m.config);
        let h_dim = m.config.h_dim;
        let h_prev = self
            .place_memory
            .get(&percept.place)
            .filter(|v| v.len() == h_dim)
            .cloned()
            .unwrap_or_else(|| vec![0.0_f32; h_dim]);

        let neural = forward_step(Some(m), &e_256, &x_num, &h_prev, None).ok()?;
        Some(DecisionTrace {
            neural,
            model_blake3: Some(m.blake3),
            feature_schema_version: m.config.feature_schema_version,
            online_adaptation_applied: false,
            commitment_bypass: false,
            self_caused: true,
            draw: 0.0,
            probability: 0.0,
            outcome: DecisionOutcome::Processed,
            reason: DecisionReason::SelfCaused,
        })
    }

    /// Deterministic projection of a single canonical log record onto `self`.
    pub fn apply_event(&mut self, record: &EventRecord) {
        if record.seq > self.last_seq {
            self.last_seq = record.seq;
        }
        self.last_timestamp_ms = Some(
            self.last_timestamp_ms
                .map_or(record.timestamp_ms, |prev| prev.max(record.timestamp_ms)),
        );

        match record.kind {
            EventKind::Percept
            | EventKind::Receipt
            | EventKind::Thought
            | EventKind::BodyState
            | EventKind::ActorFacts
            | EventKind::IdentityChanged
            | EventKind::Tick => {
                if record.id.as_u64() != 0 {
                    self.unconsumed_inputs.insert(record.id, record.clone());
                }
            }
            EventKind::Decision
            | EventKind::Verdict
            | EventKind::DoorVerdict
            | EventKind::Failure => {
                if let Some(EventDetail::Decision {
                    target_event_id,
                    outcome,
                    reason,
                    trace,
                    ..
                }) = &record.detail
                    && *outcome != DecisionOutcome::DoorBlocked
                    && let Some(percept) = self.unconsumed_inputs.get(target_event_id).cloned()
                    && percept.kind == EventKind::Percept
                {
                    if *reason == Some(DecisionReason::SelfCaused) {
                        self.record_self_caused_percept(&percept);
                    } else if *outcome != DecisionOutcome::Processed {
                        self.apply_completed_percept(&percept, trace.as_deref());
                    }
                }

                for cause_id in &record.caused_by {
                    self.unconsumed_inputs.remove(cause_id);
                }
                if let Some(
                    EventDetail::Decision {
                        target_event_id, ..
                    }
                    | EventDetail::Verdict {
                        target_event_id, ..
                    }
                    | EventDetail::DoorVerdict {
                        target_event_id: Some(target_event_id),
                        ..
                    }
                    | EventDetail::Failure {
                        target_event_id: Some(target_event_id),
                        ..
                    },
                ) = &record.detail
                {
                    self.unconsumed_inputs.remove(target_event_id);
                }
            }
            _ => {}
        }

        match record.kind {
            EventKind::Percept => {
                if record.id.as_u64() != 0 {
                    self.event_authors.insert(record.id, record.actor.clone());
                }
                self.last_global_activity_ms = Some(
                    self.last_global_activity_ms
                        .map_or(record.timestamp_ms, |p| p.max(record.timestamp_ms)),
                );
                self.unconsolidated_count = self.unconsolidated_count.saturating_add(1);
            }
            EventKind::Decision => {
                if let Some(EventDetail::Decision {
                    target_event_id,
                    outcome,
                    reason,
                    trace: Some(tr),
                    ..
                }) = &record.detail
                {
                    if (!matches!(outcome, DecisionOutcome::DoorBlocked | DecisionOutcome::Processed)
                        || *reason == Some(DecisionReason::SelfCaused))
                        && !tr.neural.h_next.is_empty()
                        && tr.neural.h_next.iter().all(|v| v.is_finite())
                    {
                        self.place_memory
                            .insert(record.place.clone(), tr.neural.h_next.clone());
                    }
                    if !tr.neural.is_neutral_fallback
                        && (tr.neural.p_verify.is_some() || tr.neural.p_uncertainty.is_some())
                    {
                        self.decision_neural_hints.insert(
                            *target_event_id,
                            (tr.neural.p_verify, tr.neural.p_uncertainty),
                        );
                    }
                }
            }
            EventKind::Action => {
                if let Some(EventDetail::Action {
                    action_id,
                    capability_id,
                    ..
                }) = &record.detail
                {
                    self.in_flight_actions.insert(
                        action_id.clone(),
                        InFlightAction {
                            action_id: action_id.clone(),
                            capability_id: capability_id.clone(),
                            places: vec![record.place.clone()],
                            dispatched_at_ms: record.timestamp_ms,
                            event_id: record.id,
                        },
                    );
                    self.next_action_seq = self.next_action_seq.saturating_add(1).max(1);
                    if capability_id.as_str() != "memory.recall" {
                        let count = self
                            .consecutive_automated_turns_by_place
                            .entry(record.place.clone())
                            .or_insert(0);
                        *count = count.saturating_add(1);
                    }
                }
                if record.id.as_u64() != 0 {
                    self.event_authors
                        .insert(record.id, self.soul_actor_id.clone());
                    self.active_chains.insert(record.id);
                }
                for &cid in &record.caused_by {
                    self.active_chains.insert(cid);
                }
                self.place_presence
                    .entry(record.place.clone())
                    .or_default()
                    .record(record.timestamp_ms);
                self.record_talk_turn(&record.place, record.timestamp_ms, true);
                let recipients: Vec<&ActorId> = record
                    .audience
                    .members
                    .iter()
                    .filter(|a| *a != &self.soul_actor_id)
                    .collect();
                if (recipients.len() as f64) <= DEFAULT_EXCHANGE_LISTENER_MAX {
                    for recipient in recipients {
                        self.actor_exchanges
                            .entry(recipient.clone())
                            .or_default()
                            .record(record.timestamp_ms);
                    }
                }
            }
            EventKind::Receipt => {
                if let Some(EventDetail::Receipt {
                    action_id,
                    capability_id,
                    outcome,
                    termination_reason,
                    progress,
                    delivered_units,
                    h_act,
                }) = &record.detail
                {
                    let receipt = Receipt {
                        action_id: action_id.clone(),
                        capability_id: capability_id.clone(),
                        outcome: *outcome,
                        reason_code: None,
                        detail: None,
                        termination_reason: *termination_reason,
                        progress: *progress,
                        delivered_units: *delivered_units,
                        h_act: h_act.clone(),
                        timestamp_ms: record.timestamp_ms,
                    };
                    if *outcome == ReceiptOutcome::Held {
                        self.in_flight_actions.remove(action_id);
                        self.pending_approvals.insert(
                            action_id.clone(),
                            PendingApprovalEntry {
                                action_id: action_id.clone(),
                                capability_id: capability_id.clone(),
                                h_act: h_act.clone(),
                                held_at_ms: record.timestamp_ms,
                            },
                        );
                        self.push_receipt_cue(format!(
                            "held:{}:{}",
                            capability_id.as_str(),
                            action_id.as_str()
                        ));
                    } else {
                        let latency_ms = self.in_flight_actions.get(action_id).map(|inflight| {
                            record
                                .timestamp_ms
                                .as_millis()
                                .saturating_sub(inflight.dispatched_at_ms.as_millis())
                        });
                        if let Some(elapsed) = latency_ms {
                            if *outcome == ReceiptOutcome::Done {
                                self.reliability_alpha += 1.0;
                                self.latency_samples_ms.push(elapsed);
                                if self.latency_samples_ms.len() > 64 {
                                    let drain = self.latency_samples_ms.len() - 64;
                                    self.latency_samples_ms.drain(0..drain);
                                }
                            } else if *outcome == ReceiptOutcome::Refused {
                                self.reliability_beta += 1.0;
                                self.push_receipt_cue(format!(
                                    "refused:{}:{}",
                                    capability_id.as_str(),
                                    action_id.as_str()
                                ));
                            } else if *outcome == ReceiptOutcome::Failed {
                                let non_fault = matches!(
                                    termination_reason,
                                    Some(
                                        TerminationReason::PreemptedExternal
                                            | TerminationReason::CanceledByCore
                                    )
                                );
                                if !non_fault {
                                    self.reliability_beta += 1.0;
                                }
                            }
                        }
                        if !self.terminal_receipts.contains_key(action_id) {
                            let status_changed = self.ledger.record_capability_receipt(
                                capability_id.clone(),
                                *outcome,
                                *termination_reason,
                                latency_ms,
                                record.timestamp_ms,
                            );
                            if status_changed {
                                self.recalibration_pending = true;
                            }
                            if let Some(&proc_id) = self.procedure_links.get(action_id) {
                                self.skill_book.monitor_procedure_outcome(
                                    proc_id,
                                    *outcome,
                                    *termination_reason,
                                    record.timestamp_ms,
                                );
                                if let Some(tp) = self.skill_book.procedures.get(&proc_id) {
                                    self.procedures.insert(proc_id, tp.to_context_entry());
                                }
                            }
                            self.in_flight_actions.remove(action_id);
                            self.pending_approvals.remove(action_id);
                            self.terminal_receipts.insert(action_id.clone(), receipt);
                        }
                    }
                }
            }
            EventKind::ExpectationOpened => match &record.detail {
                Some(EventDetail::ExpectationOpened {
                    expectation_id,
                    target_event_id,
                    kind: ExpectKind::Wait,
                    place,
                    sender,
                    medium,
                    due_ms,
                    ..
                }) => {
                    self.open_waits.insert(
                        *expectation_id,
                        OpenWait {
                            expectation_id: *expectation_id,
                            target_event_id: *target_event_id,
                            place: place.clone(),
                            sender: sender.clone(),
                            medium: medium.clone(),
                            audience: record.audience.clone(),
                            opened_at_ms: record.timestamp_ms,
                            due_ms: *due_ms,
                        },
                    );
                }
                Some(EventDetail::ExpectationRecordOpened {
                    expectation_id,
                    origin_event_id,
                    target_event_id,
                    kind,
                    places,
                    people,
                    domain,
                    condition,
                    condition_embeddings,
                    due_ms,
                    expiry_ms,
                    recurrence,
                    promise_meta,
                    urgency_weight,
                    action_id,
                    capability_id,
                    channel,
                    lat_min_ms,
                    lat_max_ms,
                    delta_pred,
                    sigma_channel,
                    correlation_id,
                    declared_sensor,
                    act_embeddings,
                    expected_reply_embeddings,
                    is_judged_silence,
                    silence_propensity,
                    silence_sender,
                    stakes,
                    medium,
                    about_ids,
                    accept_done_receipt,
                    expected_latency_ms,
                }) => {
                    let effect_spec = if *kind == ExpectKind::Effect {
                        Some(EffectPredictionSpec {
                            action_id: action_id.clone().unwrap_or_else(|| ActionId::new("")),
                            capability_id: capability_id
                                .clone()
                                .unwrap_or_else(|| CapabilityId::new("")),
                            channel: channel.clone().unwrap_or_else(|| ChannelId::new("default")),
                            lat_min_ms: *lat_min_ms,
                            lat_max_ms: *lat_max_ms,
                            delta_pred: delta_pred.unwrap_or(0.0),
                            sigma_channel: sigma_channel.unwrap_or(0.0),
                            correlation_id: correlation_id.clone(),
                            declared_sensor: *declared_sensor,
                        })
                    } else {
                        None
                    };
                    let response_spec = if *kind == ExpectKind::Response {
                        Some(ResponsePredictionSpec {
                            action_id: action_id.clone(),
                            medium: medium.clone().unwrap_or_else(|| ChannelId::new("chat")),
                            addressees: people.clone(),
                            accept_done_receipt: *accept_done_receipt,
                            act_embeddings: act_embeddings.clone(),
                            expected_reply_embeddings: expected_reply_embeddings.clone(),
                            is_judged_silence: *is_judged_silence,
                            silence_sample_propensity: *silence_propensity,
                            silence_sender: silence_sender.clone(),
                            awaiting_reask_classify: false,
                            pending_reask_event_id: None,
                            stakes: *stakes,
                            expected_latency_ms: *expected_latency_ms,
                        })
                    } else {
                        None
                    };
                    let monitor_spec = if *kind == ExpectKind::Monitor {
                        Some(MonitorState::default())
                    } else {
                        None
                    };
                    let rec = ExpectationRecord {
                        expectation_id: *expectation_id,
                        origin_event_id: *origin_event_id,
                        target_event_id: *target_event_id,
                        kind: *kind,
                        state: ExpectationLifecycleState::Open,
                        version: 1,
                        places: places.clone(),
                        people: people.clone(),
                        audience: record.audience.clone(),
                        domain: domain.clone(),
                        condition: condition.clone(),
                        condition_embeddings: condition_embeddings.clone(),
                        about_ids: about_ids.clone(),
                        created_at_ms: record.timestamp_ms,
                        updated_at_ms: record.timestamp_ms,
                        due_ms: *due_ms,
                        expiry_ms: *expiry_ms,
                        recurrence: recurrence.clone(),
                        promise_meta: promise_meta.clone(),
                        urgency_weight: *urgency_weight,
                        is_overdue: *due_ms <= record.timestamp_ms && *kind == ExpectKind::Promise,
                        due_woken: false,
                        retry_count: 0,
                        next_retry_at_ms: None,
                        retry_action: None,
                        last_action_id: action_id.clone(),
                        last_capability_id: capability_id.clone(),
                        last_h_act: None,
                        delivered_units: None,
                        termination_reason: None,
                        approval_prompt_emitted: false,
                        effect_spec,
                        response_spec,
                        monitor_spec,
                        goal_chain_ids: if *kind == ExpectKind::Goal {
                            let mut ids = vec![*expectation_id, *target_event_id];
                            for &aid in about_ids {
                                if !ids.contains(&aid) {
                                    ids.push(aid);
                                }
                            }
                            ids
                        } else {
                            Vec::new()
                        },
                        linked_action_ids: action_id.iter().cloned().collect(),
                        linked_expectation_id: None,
                    };
                    let id = *expectation_id;
                    self.expectation_book.expectations.insert(id, rec);
                    self.sync_expectation_entry(id);
                }
                _ => {}
            },
            EventKind::ExpectationChanged => match &record.detail {
                Some(EventDetail::ExpectationTransition {
                    expectation_id,
                    action_id,
                    h_act,
                    to_state,
                    ..
                }) => {
                    if let Some(aid) = action_id
                        && matches!(
                            to_state,
                            ExpectationLifecycleState::Approved
                                | ExpectationLifecycleState::Rejected
                                | ExpectationLifecycleState::Cancelled
                                | ExpectationLifecycleState::Expired
                        )
                    {
                        self.pending_approvals.remove(aid);
                    }
                    if let Some(exp) = self.expectation_book.expectations.get_mut(expectation_id) {
                        exp.state = *to_state;
                        exp.updated_at_ms = record.timestamp_ms;
                        if let Some(aid) = action_id {
                            exp.last_action_id = Some(aid.clone());
                        }
                        if let Some(h) = h_act {
                            exp.last_h_act = Some(h.clone());
                        }
                        if *to_state == ExpectationLifecycleState::PendingApproval {
                            exp.kind = ExpectKind::PendingApproval;
                            exp.approval_prompt_emitted = true;
                            exp.next_retry_at_ms = None;
                        }
                        if is_terminal_state(*to_state) {
                            exp.next_retry_at_ms = None;
                            exp.retry_action = None;
                            if exp.kind == ExpectKind::Response
                                && let Some(rs) = &exp.response_spec
                            {
                                let elapsed =
                                    record.timestamp_ms.saturating_sub_ms(exp.created_at_ms);
                                let place = exp
                                    .places
                                    .first()
                                    .cloned()
                                    .unwrap_or_else(|| record.place.clone());
                                match to_state {
                                    ExpectationLifecycleState::Answered => {
                                        self.expectation_book.response_latency.observe_response(
                                            &place,
                                            &rs.medium,
                                            elapsed,
                                            DEFAULT_LATENCY_DECAY_RHO,
                                        );
                                    }
                                    ExpectationLifecycleState::Expired
                                    | ExpectationLifecycleState::Failed => {
                                        self.expectation_book.response_latency.observe_censored(
                                            &place,
                                            &rs.medium,
                                            elapsed,
                                            DEFAULT_LATENCY_DECAY_RHO,
                                        );
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                    self.sync_expectation_entry(*expectation_id);

                    if is_terminal_state(*to_state) {
                        self.open_promises.remove(expectation_id);
                        self.open_waits.remove(expectation_id);
                    }
                }
                Some(EventDetail::ExpectationRevised {
                    expectation_id,
                    new_version,
                    condition,
                    due_ms,
                    expiry_ms,
                    places,
                    people,
                    domain,
                    ..
                }) => {
                    if let Some(exp) = self.expectation_book.expectations.get_mut(expectation_id) {
                        exp.version = *new_version;
                        exp.updated_at_ms = record.timestamp_ms;
                        if let Some(cond) = condition {
                            exp.condition = cond.clone();
                        }
                        if let Some(due) = due_ms {
                            exp.due_ms = *due;
                            exp.due_woken = false;
                            exp.is_overdue = *due <= record.timestamp_ms;
                        }
                        if let Some(expiry) = expiry_ms {
                            exp.expiry_ms = *expiry;
                        }
                        if !places.is_empty() {
                            exp.places = places.clone();
                        }
                        if !people.is_empty() {
                            exp.people = people.clone();
                        }
                        if domain.is_some() {
                            exp.domain = domain.clone();
                        }
                    }
                    self.sync_expectation_entry(*expectation_id);
                }
                Some(EventDetail::ExpectationRetryScheduled {
                    expectation_id,
                    action_id,
                    retry_count,
                    next_retry_at_ms,
                }) => {
                    if let Some(exp) = self.expectation_book.expectations.get_mut(expectation_id) {
                        exp.last_action_id = Some(action_id.clone());
                        exp.retry_count = *retry_count;
                        exp.next_retry_at_ms = Some(*next_retry_at_ms);
                        exp.updated_at_ms = record.timestamp_ms;
                    }
                }
                Some(EventDetail::MonitorFired {
                    expectation_id,
                    fired_at_ms,
                    next_due_ms,
                    coalesced,
                    skipped_slots,
                }) => {
                    if let Some(exp) = self.expectation_book.expectations.get_mut(expectation_id) {
                        exp.due_ms = *next_due_ms;
                        exp.updated_at_ms = record.timestamp_ms;
                        let ms = exp.monitor_spec.get_or_insert_with(MonitorState::default);
                        if *coalesced || *skipped_slots == 0 {
                            ms.last_fired_ms = Some(*fired_at_ms);
                            if *coalesced {
                                ms.catchup_fired_count = ms.catchup_fired_count.saturating_add(1);
                            }
                        }
                        ms.skipped_slots = ms.skipped_slots.saturating_add(*skipped_slots);
                    }
                    self.sync_expectation_entry(*expectation_id);
                }
                Some(EventDetail::ExpectationLinked {
                    expectation_id,
                    linked_event_id,
                    action_id,
                }) => {
                    if let Some(exp) = self.expectation_book.expectations.get_mut(expectation_id) {
                        if !exp.goal_chain_ids.contains(linked_event_id) {
                            exp.goal_chain_ids.push(*linked_event_id);
                        }
                        if let Some(aid) = action_id
                            && !exp.linked_action_ids.contains(aid)
                        {
                            exp.linked_action_ids.push(aid.clone());
                        }
                    }
                }
                _ => {}
            },
            EventKind::MemoryUsed => {
                let mut used = BTreeSet::new();
                for &cid in &record.caused_by {
                    used.insert(cid);
                }
                if let Some(EventDetail::MemoryUsed { used_ids }) = &record.detail {
                    for &uid in used_ids {
                        used.insert(uid);
                    }
                }
                for uid in used {
                    if let Some(entry) = self.memory_entries.get_mut(&uid) {
                        entry.activation.record(record.timestamp_ms);
                    }
                    for cand in &mut self.memory_candidates {
                        if cand.event_id == uid {
                            cand.activation.record(record.timestamp_ms);
                        }
                    }
                }
            }
            EventKind::DoorVerdict => {
                if let Some(EventDetail::DeclassGranted { signed_token }) = &record.detail
                    && !self
                        .logged_declass_tokens
                        .iter()
                        .any(|t| t.payload.token_id == signed_token.payload.token_id)
                {
                    self.logged_declass_tokens.push(signed_token.clone());
                } else if matches!(
                    &record.detail,
                    Some(EventDetail::SelfCardRecalibrated { .. })
                ) {
                    self.ledger.mark_recalibrated(record.timestamp_ms);
                    self.recalibration_pending = false;
                }
            }
            EventKind::VectorReleased => {
                for &cid in &record.caused_by {
                    if let Some(entry) = self.memory_entries.get_mut(&cid) {
                        entry.embeddings.clear();
                    }
                    self.memory_candidates.retain(|c| c.event_id != cid);
                }
                if let Some(EventDetail::Verdict {
                    target_event_id, ..
                }) = &record.detail
                {
                    if let Some(entry) = self.memory_entries.get_mut(target_event_id) {
                        entry.embeddings.clear();
                    }
                    self.memory_candidates
                        .retain(|c| &c.event_id != target_event_id);
                }
                if let Some(EventDetail::VectorReleased { event_id, .. }) = &record.detail {
                    if let Some(entry) = self.memory_entries.get_mut(event_id) {
                        entry.embeddings.clear();
                    }
                    self.memory_candidates.retain(|c| &c.event_id != event_id);
                }
            }
            EventKind::Verdict => match &record.detail {
                Some(EventDetail::ConditionVerdict {
                    expectation_id,
                    target_event_id,
                    similarity,
                    met,
                    ..
                }) => {
                    self.expectation_book.wake_gate.record_verdict(
                        *target_event_id,
                        *expectation_id,
                        *similarity,
                        *met,
                    );
                    self.expectation_book
                        .pending_classify
                        .remove(target_event_id);
                }
                Some(EventDetail::CalibrationRecorded { conf_0_20, correct }) => {
                    self.ledger.record_calibration_outcome(*conf_0_20, *correct);
                    self.calibration_samples = self.ledger.calibration.samples;
                    self.calibration_brier_sum = self.ledger.calibration.brier_sum;
                }
                Some(EventDetail::BudgetUpdated { used, limit }) => {
                    self.daily_budget_used = *used;
                    self.daily_budget_limit = *limit;
                    if self.ledger.update_budget(*used, *limit) {
                        self.recalibration_pending = true;
                    }
                }
                Some(EventDetail::AdaptFeedbackApplied {
                    place,
                    arm,
                    reward,
                    is_delayed_one_sided,
                    context_hint,
                    source_taint,
                }) => {
                    let adapt_cfg = self.effective_adapt_config();
                    if adapt_cfg.online_adaptation
                        && !(*source_taint == Taint::Untrusted
                            && (*reward < 0.0 || *is_delayed_one_sided))
                    {
                        let st = self.place_adapt.entry(place.clone()).or_default();
                        st.record_feedback_at(
                            AdaptFeedback {
                                arm: *arm,
                                reward: *reward,
                                is_delayed_one_sided: *is_delayed_one_sided,
                                context_hint: *context_hint,
                            },
                            &adapt_cfg,
                            Some(record.timestamp_ms),
                            *source_taint,
                        );
                    }
                }
                _ => {}
            },
            EventKind::BodyState => {
                if let Some(EventDetail::BodyCapabilities {
                    body_id,
                    status,
                    capabilities,
                }) = &record.detail
                {
                    self.body_states.insert(body_id.clone(), *status);
                    for cap in capabilities {
                        self.capability_descriptors
                            .insert(cap.id.clone(), cap.clone());
                        self.capability_catalog
                            .entry(cap.id.clone())
                            .and_modify(|e| {
                                e.descriptor = cap.clone();
                                e.active = true;
                            })
                            .or_insert_with(|| CapabilityCatalogEntry {
                                descriptor: cap.clone(),
                                is_static_core: false,
                                owner_signed: false,
                                active: true,
                                audience: Audience::default(),
                                embedding: None,
                            });
                    }
                    let retired_ids = self.skill_book.invalidate_procedures_on_schema_change(
                        Some(body_id),
                        &self.capability_catalog,
                        record.timestamp_ms,
                    );
                    for pid in retired_ids {
                        if let Some(tp) = self.skill_book.procedures.get(&pid) {
                            self.procedures.insert(pid, tp.to_context_entry());
                        }
                    }
                    self.recalibration_pending = true;
                } else if let Ok(status) = snapshot::parse_body_status(record.payload.as_bytes()) {
                    let body_id = BodyId::new(record.actor.as_str());
                    self.body_states.insert(body_id, status);
                    self.recalibration_pending = true;
                }
            }
            EventKind::ActorFacts => {
                if let Ok(entry) = snapshot::decode_actor_facts(record.payload.as_bytes()) {
                    self.belief_book.set_actor_prior(
                        record.actor.clone(),
                        entry.alpha_0,
                        entry.beta_0,
                    );
                    self.ledger.set_actor_facts(
                        record.actor.clone(),
                        entry.alpha_0,
                        entry.beta_0,
                        entry.trusted_origin,
                    );
                    self.actor_facts.insert(record.actor.clone(), entry);
                }
            }
            EventKind::BeliefChanged => {
                if let Some(EventDetail::BeliefChanged {
                    belief_id,
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
                }) = &record.detail
                {
                    let log_entry = BeliefLogEntry::BeliefRecorded {
                        event_id: if belief_id.as_u64() != 0 {
                            *belief_id
                        } else {
                            record.id
                        },
                        timestamp_ms: record.timestamp_ms,
                        seq: record.seq,
                        claim_key: claim_key.clone(),
                        requested_basis: *requested_basis,
                        basis: *basis,
                        basis_downgraded: *basis_downgraded,
                        source_actor: source_actor.clone(),
                        domain: domain.clone(),
                        audience: record.audience.clone(),
                        source_event_ids: source_event_ids.clone(),
                        taint: *taint,
                        valid_from_ms: *valid_from_ms,
                        valid_to_ms: *valid_to_ms,
                        replaced_by: *replaced_by,
                        alpha_0: *alpha_0,
                        beta_0: *beta_0,
                        is_mind_inference: *is_mind_inference,
                    };
                    self.belief_book.apply_log_entry(log_entry, true);
                    if let Some(src) = source_actor
                        && *basis == MemoryBasis::Told
                        && src != &self.soul_actor_id
                    {
                        let eid = if belief_id.as_u64() != 0 {
                            *belief_id
                        } else {
                            record.id
                        };
                        self.ledger.record_told_claim(
                            src,
                            eid,
                            claim_key.clone(),
                            record.timestamp_ms,
                        );
                    }
                }
            }
            EventKind::Evidence => {
                if let Some(EventDetail::Evidence {
                    target_id,
                    source_actor,
                    domain,
                    positive,
                    negative,
                    is_convention,
                    place,
                    pattern_id,
                    cited_ids,
                }) = &record.detail
                {
                    let targets: Vec<EventId> = if cited_ids.is_empty() {
                        vec![*target_id]
                    } else {
                        cited_ids.clone()
                    };
                    for tid in targets {
                        let log_entry = BeliefLogEntry::EvidenceApplied {
                            event_id: record.id,
                            timestamp_ms: record.timestamp_ms,
                            seq: record.seq,
                            target_id: tid,
                            source_actor: source_actor.clone(),
                            domain: domain.clone(),
                            positive: *positive,
                            negative: *negative,
                            is_convention: *is_convention,
                            place: place.clone(),
                            pattern_id: pattern_id.clone(),
                        };
                        self.belief_book.apply_log_entry(log_entry, true);
                        if !*is_convention
                            && let Some(b) = self.belief_book.beliefs.get(&tid)
                            && let Some(orig_src) = &b.source_actor
                            && orig_src != &self.soul_actor_id
                        {
                            if *positive > *negative {
                                self.ledger.resolve_told_claim(
                                    orig_src,
                                    tid,
                                    &b.claim_key,
                                    true,
                                    record.timestamp_ms,
                                );
                            } else if *negative > *positive {
                                self.ledger.resolve_told_claim(
                                    orig_src,
                                    tid,
                                    &b.claim_key,
                                    false,
                                    record.timestamp_ms,
                                );
                            }
                        }
                    }
                }
            }
            EventKind::EpisodeClosed => {
                if let Some(EventDetail::EpisodeClosed {
                    episode_id,
                    state_label,
                    gist_id,
                    superseded_gist_id,
                    ..
                }) = &record.detail
                {
                    if let Some(old_gist) = superseded_gist_id
                        && let Some(new_gist) = gist_id
                    {
                        self.supersede_memory(*old_gist, *new_gist);
                    }
                    if let Some(parsed) = EpisodeState::parse(state_label) {
                        if let Some(ep) = self.episode_book.episodes.get_mut(episode_id) {
                            ep.state = parsed;
                        }
                        if matches!(parsed, EpisodeState::Provisional | EpisodeState::Sealed)
                            && let Some(gid) = gist_id
                        {
                            self.episode_book
                                .mark_episode_consolidated(*episode_id, *gid);
                        }
                    }
                    self.unconsolidated_count = self.episode_book.unconsolidated_events.len();
                }
            }
            EventKind::IdentityChanged => {
                if let Ok(signed) = snapshot::decode_identity_payload(record.payload.as_bytes()) {
                    self.identity_anchor = Some(IdentityAnchor::from_owner_signed(&signed));
                    self.identity = Some(signed);
                }
            }
            EventKind::SleepRun => match &record.detail {
                Some(EventDetail::SleepRun {
                    completed: false, ..
                }) => {
                    self.last_sleep_seq = record.seq;
                }
                _ => {
                    self.pending_sleep_run = None;
                    self.last_sleep_ms = Some(record.timestamp_ms);
                    self.last_sleep_seq = record.seq;
                    self.anchor_check_pending_post_sleep = true;
                    self.unconsolidated_count = self.episode_book.unconsolidated_events.len();
                }
            },
            EventKind::HardForget => {
                if let Some(EventDetail::HardForget {
                    target_event_ids,
                    target_actor_ids,
                    epoch,
                    ..
                }) = &record.detail
                {
                    let next_epoch = (*epoch).max(self.epoch.saturating_add(1));
                    self.epoch = next_epoch;

                    let mut tainted_places = BTreeSet::new();
                    for ev in target_event_ids {
                        if let Some(mem) = self.memory_entries.get(ev) {
                            tainted_places.insert(mem.place.clone());
                        }
                        self.unconsumed_inputs.remove(ev);
                        self.event_authors.remove(ev);
                        self.active_chains.remove(ev);
                        self.open_promises.remove(ev);
                        self.open_waits
                            .retain(|exp_id, w| exp_id != ev && &w.target_event_id != ev);
                        self.memory_entries.remove(ev);
                        self.active_wake_snapshots.remove(ev);
                        self.auto_recall_states.remove(ev);
                        self.decision_neural_hints.remove(ev);
                        self.procedures.remove(ev);
                    }
                    for (place, hist) in &self.place_history {
                        if hist.iter().any(|h| {
                            target_event_ids.contains(&h.event_id)
                                || target_actor_ids.contains(&h.actor)
                                || target_actor_ids.iter().any(|a| h.audience.contains(a))
                        }) {
                            tainted_places.insert(place.clone());
                        }
                    }
                    for place in &tainted_places {
                        self.place_memory.remove(place);
                        self.place_adapt.remove(place);
                        self.consecutive_automated_turns_by_place.remove(place);
                    }

                    self.memory_candidates
                        .retain(|m| !target_event_ids.contains(&m.event_id));
                    self.expectation_cues.retain(|c| {
                        c.expectation_id
                            .is_none_or(|eid| !target_event_ids.contains(&eid))
                    });
                    for tok in &mut self.logged_declass_tokens {
                        tok.payload
                            .scope
                            .retain(|id| !target_event_ids.contains(id));
                    }
                    self.logged_declass_tokens
                        .retain(|tok| !tok.payload.scope.is_empty());

                    for actor in target_actor_ids {
                        self.actor_facts.remove(actor);
                        self.actor_exchanges.remove(actor);
                        self.open_waits.retain(|_, w| &w.sender != actor);
                        self.memory_entries
                            .retain(|_, m| &m.actor != actor && !m.audience.contains(actor));
                        for set in self.place_participants.values_mut() {
                            set.remove(actor);
                        }
                        for hist in self.place_history.values_mut() {
                            hist.retain(|h| &h.actor != actor && !h.audience.contains(actor));
                        }
                        self.memory_candidates
                            .retain(|m| !m.audience.contains(actor));
                        self.expectation_cues.retain(|c| {
                            c.counterparty.as_ref() != Some(actor) && !c.audience.contains(actor)
                        });
                        self.skill_book.remove_actor(actor);
                        self.ledger.remove_actor(actor);
                    }
                    self.expectation_book
                        .forget(target_event_ids, target_actor_ids);
                    self.open_promises = self.expectation_book.promise_summaries();
                    self.procedures
                        .retain(|_, p| !target_actor_ids.iter().any(|a| p.audience.contains(a)));
                    self.skill_book.forget(target_event_ids, target_actor_ids);
                    self.episode_book.forget(target_event_ids, target_actor_ids);
                    self.episode_book
                        .recompute_centroids_from_entries(&self.memory_entries);
                    self.belief_book
                        .recompute_after_forget(target_event_ids, target_actor_ids);
                    self.unconsolidated_count = self.episode_book.unconsolidated_events.len();
                    if let Some(run) = &mut self.pending_sleep_run {
                        run.covered_event_ids
                            .retain(|id| !target_event_ids.contains(id));
                        run.pending_episode_ids
                            .retain(|id| !target_event_ids.contains(id));
                        run.completed_episode_ids
                            .retain(|id| !target_event_ids.contains(id));
                        run.produced_gist_ids
                            .retain(|id| !target_event_ids.contains(id));
                    }
                }
            }
            EventKind::LearningChange => match &record.detail {
                Some(EventDetail::LessonRecorded {
                    lesson_id,
                    pattern,
                    failure_signature,
                    failure_ids,
                    contrastive_success_ids,
                    domain,
                }) => {
                    if !self.skill_book.lessons.contains_key(lesson_id) {
                        let _ = self.skill_book.record_consolidated_lesson(
                            crate::consolidation::RecordLessonParams {
                                lesson_id: Some(*lesson_id),
                                pattern: pattern.clone(),
                                failure_signature: failure_signature.clone(),
                                failure_ids: failure_ids.clone(),
                                contrastive_success_ids: contrastive_success_ids.clone(),
                                domain: domain.clone(),
                                audience: record.audience.clone(),
                                taint: record.taint,
                                now_ms: record.timestamp_ms,
                            },
                        );
                    }
                }
                Some(EventDetail::ProcedureUpdated {
                    procedure_id,
                    name,
                    steps,
                    required_cap_ids,
                    capability_schema_hash,
                    domain,
                    state_code,
                    reason,
                }) => {
                    let parsed_state = ProcedureLifecycleState::from_code(*state_code)
                        .unwrap_or(ProcedureLifecycleState::Active);
                    if let Some(existing) = self.skill_book.procedures.get_mut(procedure_id) {
                        existing.state = parsed_state;
                        if reason.is_some() {
                            existing.retired_reason = reason.clone();
                        }
                        if parsed_state == ProcedureLifecycleState::Retired
                            && existing.retired_at_ms.is_none()
                        {
                            existing.retired_at_ms = Some(record.timestamp_ms);
                        }
                        existing.updated_at_ms = record.timestamp_ms;
                        self.procedures
                            .insert(*procedure_id, existing.to_context_entry());
                    } else {
                        let tp = TrackedProcedure {
                            procedure_id: *procedure_id,
                            name: name.clone(),
                            steps: steps.clone(),
                            required_cap_ids: required_cap_ids.clone(),
                            capability_schema_hash: capability_schema_hash.clone(),
                            domain: domain.clone(),
                            places: Vec::new(),
                            actors: Vec::new(),
                            audience: record.audience.clone(),
                            owner_signed: false,
                            embedding: None,
                            source_goal_id: None,
                            source_event_ids: record.caused_by.clone(),
                            taint: record.taint,
                            state: parsed_state,
                            retired_reason: reason.clone(),
                            retired_at_ms: (parsed_state == ProcedureLifecycleState::Retired)
                                .then_some(record.timestamp_ms),
                            created_at_ms: record.timestamp_ms,
                            updated_at_ms: record.timestamp_ms,
                            alpha: 1.0,
                            beta: 1.0,
                            trial_count: 0,
                            success_count: 0,
                            cusum_baseline_p0: None,
                            cusum_stat: 0.0,
                            warn_count: 0,
                        };
                        self.procedures.insert(*procedure_id, tp.to_context_entry());
                        self.skill_book.procedures.insert(*procedure_id, tp);
                    }
                }
                _ => {}
            },
            EventKind::ConfigLoaded => {
                self.config_loaded_count = self.config_loaded_count.saturating_add(1);
                if let Some(EventDetail::ConfigLoaded { config, .. }) = &record.detail {
                    self.recorded_config = Some((**config).clone());
                    self.sync_config(config);
                }
            }
            _ => {}
        }
    }

    fn record_self_caused_percept(&mut self, percept: &EventRecord) {
        if percept.id.as_u64() != 0 {
            let embeddings = match &percept.detail {
                Some(EventDetail::PerceptMeta { embeddings, .. }) => embeddings.clone(),
                _ => Vec::new(),
            };
            let mut act_trace = ActivationTrace::default();
            act_trace.record(percept.timestamp_ms);
            let entry = MemoryEntry {
                event_id: percept.id,
                source_ids: vec![percept.id],
                is_gist: false,
                timestamp_ms: percept.timestamp_ms,
                place: percept.place.clone(),
                actor: percept.actor.clone(),
                origin_actor: percept.origin_actor.clone(),
                basis: MemoryBasis::SelfCaused,
                taint: percept.taint,
                audience: percept.audience.clone(),
                attachments: percept.attachments.clone(),
                activation: act_trace,
                importance: ImportanceInputs::default(),
                embeddings,
                concepts: Vec::new(),
                superseded_by: None,
                erased: false,
            };
            self.upsert_memory_entry(entry);
        }
    }

    fn apply_completed_percept(
        &mut self,
        percept: &EventRecord,
        trace: Option<&DecisionTrace>,
    ) {
        if let Some(tr) = trace
            && let Some(enc_imp) = tr.neural.p_encode_importance
            && enc_imp.is_finite()
            && let Some(entry) = self.memory_entries.get_mut(&percept.id)
        {
            entry.importance.i_surprise = entry
                .importance
                .i_surprise
                .max(f64::from(enc_imp.clamp(0.0, 1.0)));
        }
        if percept.audience.contains(&self.soul_actor_id) || percept.actor == self.soul_actor_id {
            self.place_presence
                .entry(percept.place.clone())
                .or_default()
                .record(percept.timestamp_ms);
        }

        if percept.actor != self.soul_actor_id && percept.id.as_u64() != 0 {
            self.ledger.record_witnessed(
                &percept.actor,
                percept.id,
                percept.place.clone(),
                percept.timestamp_ms,
            );
        }
        let participants = self
            .place_participants
            .entry(percept.place.clone())
            .or_default();
        participants.insert(percept.actor.clone());
        for member in &percept.audience.members {
            participants.insert(member.clone());
        }

        self.record_talk_turn(
            &percept.place,
            percept.timestamp_ms,
            percept.actor == self.soul_actor_id,
        );
        if self.is_automated_actor(&percept.actor) {
            let count = self
                .consecutive_automated_turns_by_place
                .entry(percept.place.clone())
                .or_insert(0);
            *count = count.saturating_add(1);
        } else {
            self.consecutive_automated_turns_by_place
                .insert(percept.place.clone(), 0);
        }

        let mut deg_by_actor = BTreeMap::new();
        if let Some(ds) = percept.numeric_features.deg_soul {
            deg_by_actor.insert(self.soul_actor_id.clone(), f64::from(ds));
            if percept.actor != self.soul_actor_id
                && f64::from(ds) >= DEFAULT_DEG_ALLOC_THRESHOLD
                && percept.audience.len() <= 2
            {
                self.actor_exchanges
                    .entry(percept.actor.clone())
                    .or_default()
                    .record(percept.timestamp_ms);
            }
        }
        let hist = self.place_history.entry(percept.place.clone()).or_default();
        hist.push(PlaceHistoryEntry {
            event_id: percept.id,
            actor: percept.actor.clone(),
            audience: percept.audience.clone(),
            timestamp_ms: percept.timestamp_ms,
            deg_by_actor,
        });
        if hist.len() > MAX_PLACE_HISTORY_ENTRIES {
            let drain = hist.len() - MAX_PLACE_HISTORY_ENTRIES;
            hist.drain(0..drain);
        }

        let embeddings = match &percept.detail {
            Some(EventDetail::PerceptMeta { embeddings, .. }) => embeddings.clone(),
            _ => Vec::new(),
        };

        if percept.id.as_u64() != 0 {
            let mut ep_participants = BTreeSet::new();
            ep_participants.insert(percept.actor.clone());
            for m in &percept.audience.members {
                if m != &self.soul_actor_id {
                    ep_participants.insert(m.clone());
                }
            }
            let ep_config = self.recorded_config.clone().unwrap_or_default();
            let ep_outcome = self.episode_book.admit_event(
                EpisodeAdmitParams {
                    event_id: percept.id,
                    place: &percept.place,
                    timestamp_ms: percept.timestamp_ms,
                    participants: ep_participants,
                    taint: percept.taint,
                    embeddings: &embeddings,
                    missed_expectation: false,
                },
                &ep_config,
            );
            if let Some((stale_gist, reopen_ev)) = ep_outcome.reopened_stale_gist {
                self.supersede_memory(stale_gist, reopen_ev);
            }
            self.unconsolidated_count = self.episode_book.unconsolidated_events.len();

            let mut act_trace = ActivationTrace::default();
            act_trace.record(percept.timestamp_ms);
            let basis = if percept.actor == self.soul_actor_id {
                MemoryBasis::SelfCaused
            } else if percept.taint == Taint::Trusted {
                MemoryBasis::Observed
            } else {
                MemoryBasis::Told
            };
            let entry = MemoryEntry {
                event_id: percept.id,
                source_ids: vec![percept.id],
                is_gist: false,
                timestamp_ms: percept.timestamp_ms,
                place: percept.place.clone(),
                actor: percept.actor.clone(),
                origin_actor: percept.origin_actor.clone(),
                basis,
                taint: percept.taint,
                audience: percept.audience.clone(),
                attachments: percept.attachments.clone(),
                activation: act_trace,
                importance: ImportanceInputs {
                    i_surprise: if ep_outcome.is_boundary { 1.0 } else { 0.0 },
                    ..ImportanceInputs::default()
                },
                embeddings,
                concepts: Vec::new(),
                superseded_by: None,
                erased: false,
            };
            self.upsert_memory_entry(entry);
        }
    }

    fn record_talk_turn(&mut self, place: &PlaceId, timestamp_ms: Time, is_soul: bool) {
        let turns = self.talk_turns.entry(place.clone()).or_default();
        turns.push((timestamp_ms, is_soul));
        let cutoff = Time::from_millis(
            timestamp_ms
                .as_millis()
                .saturating_sub(24 * MS_PER_HOUR as u64),
        );
        turns.retain(|&(ts, _)| ts >= cutoff);
        if turns.len() > MAX_TALK_TURNS_PER_PLACE {
            let drain = turns.len() - MAX_TALK_TURNS_PER_PLACE;
            turns.drain(0..drain);
        }
    }

    /// Encodes `self` deterministically into snapshot bytes for checksummed persistence.
    #[must_use]
    pub fn encode_snapshot(&self) -> Vec<u8> {
        snapshot::encode_snapshot(self)
    }

    /// Decodes a snapshot payload produced by [`Self::encode_snapshot`].
    pub fn decode_snapshot(bytes: &[u8]) -> Result<Self, crate::event::EventFormatError> {
        snapshot::decode_snapshot(bytes)
    }
}

pub(crate) fn encode_actor_facts_from_msg(facts: &monoagens_protocol::ActorFacts) -> Vec<u8> {
    snapshot::encode_actor_facts_from_msg(facts)
}

/// Decodes a canonical `EventKind::ActorFacts` payload into [`ActorFactsEntry`],
/// defaulting `automated_peer` to `false` for legacy pre-Wave-B payloads.
pub fn decode_actor_facts_entry(
    bytes: &[u8],
) -> Result<ActorFactsEntry, crate::event::EventFormatError> {
    snapshot::decode_actor_facts(bytes)
}

pub(crate) fn encode_identity_payload(signed: &OwnerSigned<OpaquePayload>) -> Vec<u8> {
    snapshot::encode_identity_payload(signed)
}

pub(crate) fn encode_body_status(status: BodyStatus) -> u8 {
    snapshot::encode_body_status(status)
}
