//! Projection state ("six books" skeleton) rebuilt deterministically from the event log.

use std::collections::{BTreeMap, BTreeSet};

use monoagens_protocol::{
    ActionId, ActorFacts, ActorId, AttachmentHandle, Audience, BodyId, BodyStatus,
    CapabilityDescriptor, CapabilityFlags, CapabilityId, CapabilitySchemaHash, ChannelId,
    DeclassToken, DomainId, EmbeddingTag, EventId, ExpectKind, HAct, ModalitySlot, OpaquePayload,
    OriginActor, OwnerSigned, PlaceId, Receipt, ReceiptOutcome, TaggedEmbedding, Taint,
    TerminationReason, Time, VisibleScope,
};
use serde::{Deserialize, Serialize};

use crate::activation::{ActivationTrace, MS_PER_HOUR, place_familiarity};
use crate::beliefs::{
    BeliefBook, BeliefLogEntry, CredenceRebaseRecord, DomainCentroid, DomainClusterBook,
    DomainTransitionKind, DomainTransitionRecord,
};
use crate::consolidation::{
    EpisodeBook, EpisodeRecord, EpisodeState, PendingSleepRun, PlaceBoundaryState,
    ProcedureLifecycleState, SkillBook, TrackedProcedure,
};
use crate::context::{
    AutoRecallTriggerState, CapabilityCatalogEntry, CoreSelfCard, IdentityAnchor,
    PrincipleDescriptor, PrincipleSeverity, ProcedureEntry,
};
use crate::event::{
    BinReader, BinWriter, DecisionOutcome, EventDetail, EventFormatError, EventKind, EventRecord,
    ExpectationLifecycleState, LogSeq, decode_opt_sensed_effect, decode_tagged_embedding,
    decode_taint, encode_opt_sensed_effect, encode_tagged_embedding, encode_taint,
};
use crate::expectations::{
    EffectPredictionSpec, ExpectationBook, ExpectationRecord, MonitorState, ResponsePredictionSpec,
    is_terminal_state, max_matching_cosine,
};
use crate::gate::{
    CandidateMemory, CommitmentInputs, ExpectationCue, FrozenDialogueActHead, GateOutcome,
    HabituationBook, HabituationEntry, HabituationKey, InitHabituationEntry, LatencySketch,
    OpenWait, PatienceEstimator, PendingBeliefSummary, PlaceHistoryEntry, PromiseSummary,
    SalienceWeights, SignalInputs, ThresholdInputs, compute_salience, compute_threshold,
    decide_gate, evaluate_signals,
};
use crate::learn::BoundedLearner;
use crate::ledger::PerceptionLedger;
use crate::map::{CoOccurrenceEdge, CognitiveMap, MapVertex};
use crate::prior::Prior;
use crate::privacy::build_visible_scope;
use crate::recall::{ImportanceInputs, MemoryBasis, MemoryEntry};
use crate::registry::Switches;

const SNAPSHOT_MAGIC: &[u8; 4] = b"MGSN";
const SNAPSHOT_VERSION: u16 = 7;

const MAX_PLACE_HISTORY_ENTRIES: usize = 64;
const MAX_TALK_TURNS_PER_PLACE: usize = 512;

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
}

/// Adaptive gate weight multipliers captured in an epoch snapshot anchor (`R-34`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct AdaptiveWeightsAnchor {
    pub salience_weights: SalienceWeights,
    pub theta_0: f64,
    pub a: f64,
    pub b: f64,
    pub c: f64,
    pub r: f64,
}

impl AdaptiveWeightsAnchor {
    #[must_use]
    pub fn from_prior(prior: &Prior, switches: &Switches) -> Self {
        Self {
            salience_weights: SalienceWeights::from_prior(prior, switches),
            theta_0: prior.theta_0(),
            a: if switches.drive_threshold {
                prior.a_scale * prior.restraint
            } else {
                0.0
            },
            b: if switches.drive_threshold {
                prior.budget_weight
            } else {
                0.0
            },
            c: if switches.drive_threshold {
                prior.promise_debt_weight
            } else {
                0.0
            },
            r: if switches.familiarity_reserve {
                prior.reserve_when_unfamiliar
            } else {
                0.0
            },
        }
    }
}

impl Default for AdaptiveWeightsAnchor {
    fn default() -> Self {
        Self::from_prior(&Prior::default(), &Switches::all_enabled())
    }
}

/// Two-Epoch Replay anchor preserved across authorized `hard_forget` erasures (`R-34`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct EpochAnchor {
    pub epoch: u64,
    pub anchor_seq: LogSeq,
    pub frozen: bool,
    pub frozen_weights: Option<AdaptiveWeightsAnchor>,
}

/// Deterministic projection state rebuilt from [`EventRecord`]s and snapshots.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CoreState {
    /// Highest [`LogSeq`] applied to this projection.
    pub last_seq: LogSeq,
    /// Two-Epoch Replay epoch counter (`0` prior to any `hard_forget`).
    pub epoch: u64,
    /// Monotonic counter for deterministic [`ActionId`] generation.
    pub next_action_seq: u64,
    /// Latest timestamp observed on any applied event.
    pub last_timestamp_ms: Option<Time>,
    /// Latest owner-signed identity payload.
    pub identity: Option<OwnerSigned<OpaquePayload>>,
    /// Latest operational status reported per body.
    pub body_states: BTreeMap<BodyId, BodyStatus>,
    /// Per-actor reliability priors supplied via `ActorFacts`.
    pub actor_facts: BTreeMap<ActorId, ActorFactsEntry>,
    /// Actions currently in flight awaiting a terminal receipt.
    pub in_flight_actions: BTreeMap<ActionId, InFlightAction>,
    /// Actions held in `PendingApproval`.
    pub pending_approvals: BTreeMap<ActionId, PendingApprovalEntry>,
    /// Terminal receipts already processed (`Done | Refused | Failed | Unknown`), enforcing `R-36`.
    pub terminal_receipts: BTreeMap<ActionId, Receipt>,
    /// Trailing recorded inputs that lack a terminal `decision`/`verdict` marker.
    pub unconsumed_inputs: BTreeMap<EventId, EventRecord>,

    // --- C2 activation, gate signals, threshold, and Two-Epoch Replay projections ---
    /// Opaque identifier of the soul actor (defaults to `"soul"`).
    pub soul_actor_id: ActorId,
    /// Two-Epoch Replay anchor surviving authorized `hard_forget` erasures (`R-34`).
    pub epoch_anchor: EpochAnchor,
    /// Current adaptive weight multipliers (updated or frozen at `epoch_anchor`).
    pub adaptive_weights: AdaptiveWeightsAnchor,
    /// Place presence usage history (`B_P`) per place.
    pub place_presence: BTreeMap<PlaceId, ActivationTrace>,
    /// Bilateral exchange usage history (`B_p`) per actor.
    pub actor_exchanges: BTreeMap<ActorId, ActivationTrace>,
    /// Bounded place momentum ring buffer (`H_p`, `W_c` entries) for `cont(e, l)`.
    pub place_history: BTreeMap<PlaceId, Vec<PlaceHistoryEntry>>,
    /// Known active participants per place (for `B_ctx` and `deg`).
    pub place_participants: BTreeMap<PlaceId, BTreeSet<ActorId>>,
    /// Bounded communicative turns `(timestamp_ms, is_soul)` per place for `talk_rate`.
    pub talk_turns: BTreeMap<PlaceId, Vec<(Time, bool)>>,
    /// Synthetic `"init:<place_id>"` initiative habituation entries (`R-30`).
    pub init_habituation: BTreeMap<PlaceId, InitHabituationEntry>,
    /// Indexed memory candidates projected through `VisibleScope` before FOK/relevance.
    pub memory_candidates: Vec<CandidateMemory>,
    /// Open promises and monitors indexed by expectation event id (`promise_debt`).
    pub open_promises: BTreeMap<EventId, PromiseSummary>,
    /// Active expectation cues for `link` and `relevance`.
    pub expectation_cues: Vec<ExpectationCue>,
    /// Pending Beta beliefs linked to active goals/decisions (`own_info_gap`).
    pub pending_beliefs: Vec<PendingBeliefSummary>,
    /// Author lookup for recorded events (used to resolve `reply_to_actor`).
    pub event_authors: BTreeMap<EventId, ActorId>,
    /// Active event chain IDs belonging to the soul's ongoing work.
    pub active_chains: BTreeSet<EventId>,
    /// Optional frozen `A-14` logistic `dialogue_act_gap` head `(w_gap, b_gap)`.
    pub dialogue_act_head: Option<FrozenDialogueActHead>,
    /// Daily budget consumed (same units as `daily_budget_limit`).
    pub daily_budget_used: f64,
    /// Daily budget limit (`> 0`).
    pub daily_budget_limit: f64,
    /// Unconsolidated event count for `D_sleep`.
    pub unconsolidated_count: usize,
    /// Last sleep consolidation timestamp for `D_sleep`.
    pub last_sleep_ms: Option<Time>,
    /// Minimum degree `theta_ex` to log a bilateral exchange (synced from `Prior`).
    pub theta_ex: f64,
    /// Max sum-of-degrees `m_ex` to log a bilateral exchange (synced from `Prior`).
    pub m_ex: f64,

    // --- C3 habituation, patience, cognitive map, memory provenance, and DeclassTokens ---
    /// Partitioned post-evaluation habituation `h_{(p, k_src)}` (`A-03`, `R-36`).
    pub habituation: HabituationBook,
    /// Habituation per-minute decay `gamma` (synced from `Prior`).
    pub habituation_gamma: f64,
    /// Right-censored, hierarchically shrunk patience quantile estimator (`A-08`).
    pub patience: PatienceEstimator,
    /// Patience observation decay `rho_lat` (synced from `Prior`).
    pub patience_decay_rho: f64,
    /// Open `wait` expectations keyed by `expectation_id`.
    pub open_waits: BTreeMap<EventId, OpenWait>,
    /// Topological SR / co-occurrence cognitive map over `{actor, place, concept}` (`R-32`).
    pub cognitive_map: CognitiveMap,
    /// Provenance-bearing memory entries keyed by `event_id`.
    pub memory_entries: BTreeMap<EventId, MemoryEntry>,
    /// Logged owner-signed `DeclassToken`s (`R-33`).
    pub logged_declass_tokens: Vec<OwnerSigned<DeclassToken>>,
    /// Map update parameters synced from `Prior` for deterministic projection updates.
    pub map_prior: Prior,

    // --- C4 expectations, recurrences, promises, grounding, agency, and judged silence ---
    /// Typed expectation store covering promises, goals, monitors, approvals, and predictions.
    pub expectation_book: ExpectationBook,
    /// Body-advertised capability descriptors keyed by `CapabilityId`.
    pub capability_descriptors: BTreeMap<CapabilityId, CapabilityDescriptor>,
    /// True after `Core::open` until the first startup recovery tick/call runs.
    pub startup_recovery_pending: bool,

    // --- C5 ContextPlan, Slot-0 IdentityAnchor, Tool RAG, P_rc, SelfGuide ---
    /// Opaque owner-signed Slot-0 identity anchor derived on `EventKind::IdentityChanged`.
    pub identity_anchor: Option<OwnerSigned<IdentityAnchor>>,
    /// Owner-signed behavioral principles checked on `[ANCHOR_CHECK]` & `PrincipleAudit`.
    pub principles: Vec<OwnerSigned<PrincipleDescriptor>>,
    /// Catalog of body capabilities enriched with static-core and `OwnerSigned` metadata.
    pub capability_catalog: BTreeMap<CapabilityId, CapabilityCatalogEntry>,
    /// Distilled procedural skills indexed by event id for `Tool RAG`.
    pub procedures: BTreeMap<EventId, ProcedureEntry>,
    /// Optional model knowledge cutoff string surfaced in `ContextPlan`.
    pub knowledge_cutoff: Option<String>,
    /// Per-domain logistic calibration offsets applied to `[0, 20]` raw confidence scores.
    pub domain_calibration_offsets: BTreeMap<DomainId, f64>,
    /// Disclosed wake snapshots keyed by `request_id` (`trigger_event_id`) for pre-act guards.
    pub active_wake_snapshots: BTreeMap<EventId, WakeSnapshotRecord>,
    /// Bounded auto-recall state keyed by `trigger_event_id`.
    pub auto_recall_states: BTreeMap<EventId, AutoRecallTriggerState>,
    /// Recent held/refused receipt summary strings surfaced in `ContextSections::working_set`.
    pub recent_receipt_cues: Vec<String>,
    /// Reasoning wakes elapsed since the last `[ANCHOR_CHECK]`.
    pub wake_count_since_anchor_check: u64,
    /// Set by `EventKind::SleepRun` so the next reasoning wake triggers `[ANCHOR_CHECK]`.
    pub anchor_check_pending_post_sleep: bool,
    /// Sequence number of the last `SleepRun` event.
    pub last_sleep_seq: LogSeq,
    /// Beta prior alpha for outbound delivery reliability (1.0 initial).
    pub reliability_alpha: f64,
    /// Beta prior beta for outbound delivery reliability (excludes external preemption/cancellation).
    pub reliability_beta: f64,
    /// Recent outbound action delivery latencies (ms) for median estimation.
    pub latency_samples_ms: Vec<u64>,
    /// Recorded per-trigger allocation state (`true` = allocated turn, `false` = unallocated Go turn).
    pub turn_allocated_by_trigger: BTreeMap<EventId, bool>,
    /// Calibration sample count and cumulative Brier error for `self_card`.
    pub calibration_samples: u64,
    pub calibration_brier_sum: f64,
    /// Principle consistency Beta parameters & contradiction count for `self_card`.
    pub principle_alpha: f64,
    pub principle_beta: f64,
    pub contradiction_flags: u64,

    // --- C6 Beliefs, Credence, Domain Clusters, Episodes & Sleep ---
    /// Belief records, Beta `(alpha, beta)` credences, `(source, domain)` reliabilities, and conventions.
    pub belief_book: BeliefBook,
    /// Warm-started spherical k-means + Hungarian-aligned domain centroids (`A-12`).
    pub domain_clusters: DomainClusterBook,
    /// Per-place `{open, provisional, sealed}` episodes and EM-LLM boundary state (`A-05`, `A-06`).
    pub episode_book: EpisodeBook,
    /// Active or interrupted snapshot-isolated sleep consolidation run (`12.1`).
    pub pending_sleep_run: Option<PendingSleepRun>,
    /// Timestamp of the last global external/core activity for `T_idle` sleep trigger (`A-04`).
    pub last_global_activity_ms: Option<Time>,
    /// Active mechanism switches synced from `Core` for deterministic `apply_event` projections.
    pub map_switches: Switches,

    // --- C7 Contrastive Lessons, Monitored Procedures, Ledger, Bounded Learning ---
    /// Contrastive lesson clustering, cross-place procedures, and CUSUM drift tracking (`14.1`..`14.3`).
    pub skill_book: SkillBook,
    /// Epistemic participant cards and derived 4-pillar `self_card` (`15.1`..`15.3`).
    pub ledger: PerceptionLedger,
    /// Clipped IPS bounded reinforcement learner & rollback history (`16.1`..`16.4`).
    pub learner: BoundedLearner,
    /// Maps dispatched `ActionId` to active `procedure_id` for CUSUM execution monitoring (`14.3`).
    pub procedure_links: BTreeMap<ActionId, EventId>,
    /// Flag raised when capability/budget/contradiction changes warrant deterministic recalibration (`15.3`).
    pub recalibration_pending: bool,
}

/// Snapshot of what was visible and disclosed on a reasoning wake (`7.3`, `7.4`, `7.5`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WakeSnapshotRecord {
    pub visible_ids: BTreeSet<EventId>,
    pub visible_scope: VisibleScope,
    pub disclosed_capabilities: BTreeSet<CapabilityId>,
    pub turn_allocated: bool,
    pub wake_place: PlaceId,
    pub anchor_check_due: bool,
}

impl Default for CoreState {
    fn default() -> Self {
        Self::new()
    }
}

impl CoreState {
    /// Creates an empty initial projection state.
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
            epoch_anchor: EpochAnchor::default(),
            adaptive_weights: AdaptiveWeightsAnchor::default(),
            place_presence: BTreeMap::new(),
            actor_exchanges: BTreeMap::new(),
            place_history: BTreeMap::new(),
            place_participants: BTreeMap::new(),
            talk_turns: BTreeMap::new(),
            init_habituation: BTreeMap::new(),
            memory_candidates: Vec::new(),
            open_promises: BTreeMap::new(),
            expectation_cues: Vec::new(),
            pending_beliefs: Vec::new(),
            event_authors: BTreeMap::new(),
            active_chains: BTreeSet::new(),
            dialogue_act_head: None,
            daily_budget_used: 0.0,
            daily_budget_limit: 1.0,
            unconsolidated_count: 0,
            last_sleep_ms: None,
            theta_ex: 0.50,
            m_ex: 3.0,
            habituation: HabituationBook::default(),
            habituation_gamma: Prior::default().habituation_gamma(),
            patience: PatienceEstimator::default(),
            patience_decay_rho: Prior::default().patience_decay_rho,
            open_waits: BTreeMap::new(),
            cognitive_map: CognitiveMap::new(),
            memory_entries: BTreeMap::new(),
            logged_declass_tokens: Vec::new(),
            map_prior: Prior::default(),
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
            domain_clusters: DomainClusterBook::new(),
            episode_book: EpisodeBook::new(),
            pending_sleep_run: None,
            last_global_activity_ms: None,
            map_switches: Switches::all_enabled(),
            skill_book: SkillBook::new(),
            ledger: PerceptionLedger::new(),
            learner: BoundedLearner::new(),
            procedure_links: BTreeMap::new(),
            recalibration_pending: false,
        }
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

    /// Looks up whether `cause` (or any of its `caused_by` ancestors) was marked `turn_allocated`.
    /// Core/expectation wakes default to `true`; explicit unallocated percept Go turns return `false`.
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
        if !self.map_switches.perception_ledger {
            return CoreSelfCard::default();
        }
        let mut card = self
            .ledger
            .build_self_card(&self.map_prior, &self.map_switches);
        // Merge any legacy log counters when capability-scoped ledger stats have not yet populated.
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

    /// Synchronizes threshold/exchange/habituation/patience/map calibration parameters from `prior` and `switches`.
    pub fn sync_prior(&mut self, prior: &Prior, switches: &Switches) {
        self.theta_ex = prior.theta_ex;
        self.m_ex = prior.m_ex;
        self.habituation_gamma = prior.habituation_gamma();
        self.patience_decay_rho = prior.patience_decay_rho;
        self.map_prior = prior.clone();
        self.map_switches = switches.clone();
        if self.expectation_book.wake_gate.verdicts.is_empty() {
            self.expectation_book.wake_gate.wake_gate_lo = prior.wake_gate_lo;
        }
        if !self.epoch_anchor.frozen && self.learner.update_count == 0 {
            self.adaptive_weights = AdaptiveWeightsAnchor::from_prior(prior, switches);
        }
    }

    /// Synchronizes `pending_beliefs` summary vector from `belief_book` for `own_info_gap` (`R-31`).
    pub fn sync_pending_beliefs(&mut self) {
        let mut out = Vec::new();
        for b in self.belief_book.beliefs.values() {
            if !b.erased
                && b.is_current()
                && let Some(cred) = b.credence
            {
                out.push(PendingBeliefSummary {
                    alpha: cred.alpha,
                    beta: cred.beta,
                    stakes: 1.0,
                    depends_on_active_goal: true,
                });
            }
        }
        self.pending_beliefs = out;
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
        self.ledger
            .sync_open_requests(&self.expectation_book, &self.map_prior);
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

    /// Inserts or updates a provenance-bearing [`MemoryEntry`] in Core's memory projection
    /// and binds its opaque `{actor, place, concept}` vertices in the cognitive map.
    pub fn upsert_memory_entry(&mut self, entry: MemoryEntry) {
        let mut verts = vec![
            MapVertex::Actor(entry.actor.clone()),
            MapVertex::Place(entry.place.clone()),
        ];
        for member in &entry.audience.members {
            if member != &self.soul_actor_id {
                let av = MapVertex::Actor(member.clone());
                if !verts.contains(&av) {
                    verts.push(av);
                }
            }
        }
        for concept in &entry.concepts {
            verts.push(MapVertex::Concept(concept.clone()));
        }
        self.cognitive_map.record_event_vertices(
            entry.event_id,
            &verts,
            &entry.place,
            entry.timestamp_ms,
            &self.map_prior,
        );

        // Keep FOK/relevance candidate list synchronized for visible embeddings.
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

    /// Marks `old_event_id` as superseded by `new_event_id` (`5.2`).
    pub fn supersede_memory(&mut self, old_event_id: EventId, new_event_id: EventId) {
        if let Some(entry) = self.memory_entries.get_mut(&old_event_id) {
            entry.superseded_by = Some(new_event_id);
        }
    }

    /// Evaluates the full C2/C3 appraisal gate for a recorded `Percept` event against `self`
    /// WITHOUT mutating `self` (strictly reading history prior to this percept).
    #[must_use]
    pub fn appraise_percept_record(
        &self,
        percept: &EventRecord,
        prior: &Prior,
        switches: &Switches,
        seed: u64,
        use_frozen_anchor: bool,
    ) -> GateOutcome {
        let (
            addressees,
            named,
            reply_to,
            correlation_id,
            embeddings,
            is_dyadic_chat,
            answered,
            channel,
        ) = match &percept.detail {
            Some(EventDetail::PerceptMeta {
                addressees,
                named,
                reply_to,
                correlation_id,
                embeddings,
                is_dyadic_chat,
                answered,
                channel,
            }) => (
                addressees.as_slice(),
                named.as_slice(),
                *reply_to,
                correlation_id.as_ref(),
                embeddings.as_slice(),
                *is_dyadic_chat,
                *answered,
                channel.as_ref(),
            ),
            _ => (
                &[][..],
                &[][..],
                percept.caused_by.first().copied(),
                None,
                &[][..],
                false,
                0.0,
                None,
            ),
        };

        let _ = correlation_id;
        let reply_to_actor = reply_to.and_then(|eid| self.event_authors.get(&eid));
        let sender_exchange_trace = self
            .actor_exchanges
            .get(&percept.actor)
            .copied()
            .unwrap_or_default();
        let place_hist = self
            .place_history
            .get(&percept.place)
            .map_or(&[][..], Vec::as_slice);
        let active_participants: Vec<ActorId> = self
            .place_participants
            .get(&percept.place)
            .map(|set| set.iter().cloned().collect())
            .unwrap_or_default();
        let visible_scope = self.visible_scope_for(
            percept.audience.clone(),
            Some(percept.place.clone()),
            percept.timestamp_ms,
        );

        let sig_inputs = SignalInputs {
            soul_id: &self.soul_actor_id,
            sender: &percept.actor,
            timestamp_ms: percept.timestamp_ms,
            audience: &percept.audience,
            addressees,
            named,
            orient: &percept.numeric_features.orient,
            reply_to_actor,
            f_modal: percept.numeric_features.f_modal,
            embeddings,
            precomputed_dialogue_act_gap: percept.numeric_features.dialogue_act_gap,
            is_dyadic_chat,
            answered,
            sender_exchange_trace,
            place_history: place_hist,
            active_participants: &active_participants,
            visible_scope: &visible_scope,
            candidate_memories: &self.memory_candidates,
            expectation_cues: &self.expectation_cues,
            pending_beliefs: &self.pending_beliefs,
            dialogue_act_head: self.dialogue_act_head.as_ref(),
            is_idle_probe: false,
            curiosity_override: None,
        };

        let (signals, _deg_eval) = evaluate_signals(&sig_inputs, prior);

        let mut salience = compute_salience(&signals, prior, switches);
        if use_frozen_anchor
            && self.epoch_anchor.frozen
            && let Some(frozen) = self.epoch_anchor.frozen_weights
        {
            salience.weights = frozen.salience_weights;
            salience.raw_salience = frozen.salience_weights.evaluate(&signals);
        } else if self.learner.update_count > 0 {
            let mut w = self.adaptive_weights.salience_weights;
            if !switches.salience {
                w = SalienceWeights::default();
            } else if !switches.floor_cost {
                w.w_sc = 0.0;
            }
            salience.weights = w;
            salience.raw_salience = w.evaluate(&signals);
        }

        // Compute place familiarity B_ctx across place presence and present actors.
        let place_trace = self
            .place_presence
            .get(&percept.place)
            .copied()
            .unwrap_or_default();
        let mut present_traces: Vec<&ActivationTrace> = Vec::new();
        for member in &percept.audience.members {
            if member != &self.soul_actor_id
                && let Some(tr) = self.actor_exchanges.get(member)
            {
                present_traces.push(tr);
            }
        }
        let d = prior.petrov_d();
        let familiarity = place_familiarity(
            &place_trace,
            &present_traces,
            percept.timestamp_ms,
            d,
            prior.closeness_prior_count,
        );

        // Talk rate over rolling window.
        let window_ms = (prior.talk_rate_window_minutes * 60_000.0).round().max(0.0) as u64;
        let window_start =
            Time::from_millis(percept.timestamp_ms.as_millis().saturating_sub(window_ms));
        let (soul_acts_in_window, total_acts_in_window) = self
            .talk_turns
            .get(&percept.place)
            .map(|turns| {
                let mut total = 0usize;
                let mut soul = 0usize;
                for &(ts, is_soul) in turns {
                    if ts >= window_start && ts <= percept.timestamp_ms {
                        total += 1;
                        if is_soul {
                            soul += 1;
                        }
                    }
                }
                (soul, total)
            })
            .unwrap_or((0, 0));

        let mut speakers = BTreeSet::new();
        for a in &active_participants {
            speakers.insert(a.clone());
        }
        for a in &percept.audience.members {
            speakers.insert(a.clone());
        }
        speakers.insert(percept.actor.clone());
        speakers.insert(self.soul_actor_id.clone());

        let budget_used_ratio = if self.daily_budget_limit > 0.0 {
            (self.daily_budget_used / self.daily_budget_limit).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let promises: Vec<PromiseSummary> = self.open_promises.values().cloned().collect();
        let thr_inputs = ThresholdInputs {
            now_ms: percept.timestamp_ms,
            soul_acts_in_window,
            total_acts_in_window,
            active_speakers: speakers.len().max(1),
            budget_used_ratio,
            promises: &promises,
            familiarity,
            init_habituation: self.init_habituation.get(&percept.place).copied(),
            is_idle_initiative: false,
        };
        let mut threshold_terms = compute_threshold(&thr_inputs, prior, switches);
        if use_frozen_anchor
            && self.epoch_anchor.frozen
            && let Some(frozen) = self.epoch_anchor.frozen_weights
        {
            threshold_terms.theta_0 = frozen.theta_0;
            threshold_terms.a = frozen.a;
            threshold_terms.b = frozen.b;
            threshold_terms.c = frozen.c;
            threshold_terms.r = frozen.r;
            threshold_terms.theta = threshold_terms.recompute_theta();
        } else if self.learner.update_count > 0 {
            threshold_terms.theta_0 = self.adaptive_weights.theta_0;
            threshold_terms.theta = threshold_terms.recompute_theta();
        }

        let turn_allocated =
            signals.deg_soul >= prior.theta_alloc || addressees.contains(&self.soul_actor_id);
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

        let source_channel = HabituationBook::source_channel_for(channel, &percept.actor);
        let h_pre = self.habituation.h_pre(
            &percept.place,
            &source_channel,
            percept.timestamp_ms,
            prior.habituation_gamma(),
        );

        decide_gate(
            percept.id,
            seed,
            signals,
            salience,
            threshold_terms,
            h_pre,
            commitments,
            prior,
            switches,
        )
    }

    /// Deterministically projects `record` onto `self` without performing IO or calling doors.
    pub fn apply_event(&mut self, record: &EventRecord) {
        if record.seq > self.last_seq {
            self.last_seq = record.seq;
        }
        self.last_timestamp_ms = Some(
            self.last_timestamp_ms
                .map_or(record.timestamp_ms, |prev| prev.max(record.timestamp_ms)),
        );

        // Track unconsumed inputs vs completion markers for crash recovery.
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
                // Before removing from unconsumed_inputs, apply post-decision percept updates
                // if this Decision finishes an inbound Percept without blocking it.
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
                    if *reason == Some(crate::gate::DecisionReason::SelfCaused) {
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

        // Apply kind-specific state projections.
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
                if (recipients.len() as f64) <= self.m_ex {
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
                                // Only attribute core/dispatch failures when not externally preempted/canceled.
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
                                &self.map_prior,
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
                                    &self.map_prior,
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
                                            self.patience_decay_rho,
                                        );
                                    }
                                    ExpectationLifecycleState::Expired
                                    | ExpectationLifecycleState::Failed => {
                                        self.expectation_book.response_latency.observe_censored(
                                            &place,
                                            &rs.medium,
                                            elapsed,
                                            self.patience_decay_rho,
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
                        if let Some(wait) = self.open_waits.remove(expectation_id) {
                            let elapsed_ms =
                                record.timestamp_ms.saturating_sub_ms(wait.opened_at_ms);
                            match to_state {
                                ExpectationLifecycleState::Cancelled => {
                                    self.patience.observe_reply(
                                        &wait.place,
                                        &wait.medium,
                                        elapsed_ms,
                                        self.patience_decay_rho,
                                    );
                                }
                                ExpectationLifecycleState::Reached
                                | ExpectationLifecycleState::Expired => {
                                    self.patience.observe_censored(
                                        &wait.place,
                                        &wait.medium,
                                        elapsed_ms,
                                        self.patience_decay_rho,
                                    );
                                }
                                _ => {}
                            }
                        }
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
                        &self.map_prior,
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
                } else if let Ok(status) = parse_body_status(record.payload.as_bytes()) {
                    let body_id = BodyId::new(record.actor.as_str());
                    self.body_states.insert(body_id, status);
                    self.recalibration_pending = true;
                }
            }
            EventKind::ActorFacts => {
                if let Ok(entry) = decode_actor_facts(record.payload.as_bytes()) {
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
                        source_event_ids: source_event_ids.clone(),
                        taint: *taint,
                        valid_from_ms: *valid_from_ms,
                        valid_to_ms: *valid_to_ms,
                        replaced_by: *replaced_by,
                        alpha_0: *alpha_0,
                        beta_0: *beta_0,
                        is_mind_inference: *is_mind_inference,
                    };
                    self.belief_book.apply_log_entry(
                        log_entry,
                        &self.map_prior,
                        &self.map_switches,
                        true,
                    );
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
                            &self.map_prior,
                        );
                    }
                    self.sync_pending_beliefs();
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
                        self.belief_book.apply_log_entry(
                            log_entry,
                            &self.map_prior,
                            &self.map_switches,
                            true,
                        );
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
                                    &self.map_prior,
                                );
                            } else if *negative > *positive {
                                self.ledger.resolve_told_claim(
                                    orig_src,
                                    tid,
                                    &b.claim_key,
                                    false,
                                    record.timestamp_ms,
                                    &self.map_prior,
                                );
                            }
                        }
                    }
                    self.sync_pending_beliefs();
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
            EventKind::DomainsUpdated => {
                if let Some(EventDetail::DomainsUpdated {
                    centroids,
                    transitions,
                    credence_rebased,
                }) = &record.detail
                {
                    self.domain_clusters
                        .apply_domains_updated(centroids, transitions);
                    self.belief_book.apply_log_entry(
                        BeliefLogEntry::DomainsRebased {
                            event_id: record.id,
                            timestamp_ms: record.timestamp_ms,
                            seq: record.seq,
                            transitions: transitions.clone(),
                            credence_rebased: credence_rebased.clone(),
                        },
                        &self.map_prior,
                        &self.map_switches,
                        true,
                    );
                    self.sync_pending_beliefs();
                }
            }
            EventKind::IdentityChanged => {
                if let Ok(signed) = decode_identity_payload(record.payload.as_bytes()) {
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

                    // Preserve and freeze Two-Epoch Replay anchor across authorized erasures (R-34).
                    self.epoch_anchor = EpochAnchor {
                        epoch: next_epoch,
                        anchor_seq: record.seq,
                        frozen: true,
                        frozen_weights: Some(
                            self.epoch_anchor
                                .frozen_weights
                                .unwrap_or(self.adaptive_weights),
                        ),
                    };

                    for ev in target_event_ids {
                        self.unconsumed_inputs.remove(ev);
                        self.event_authors.remove(ev);
                        self.active_chains.remove(ev);
                        self.open_promises.remove(ev);
                        self.open_waits
                            .retain(|exp_id, w| exp_id != ev && &w.target_event_id != ev);
                        self.memory_entries.remove(ev);
                        self.cognitive_map.forget_event(*ev);
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
                        self.habituation.remove_actor(actor);
                        self.cognitive_map.forget_actor(actor);
                        self.open_waits.retain(|_, w| &w.sender != actor);
                        let erased_mem_ids: Vec<EventId> = self
                            .memory_entries
                            .iter()
                            .filter(|(_, m)| &m.actor == actor || m.audience.contains(actor))
                            .map(|(&id, _)| id)
                            .collect();
                        for mid in erased_mem_ids {
                            self.memory_entries.remove(&mid);
                            self.cognitive_map.forget_event(mid);
                        }
                        for set in self.place_participants.values_mut() {
                            set.remove(actor);
                        }
                        for hist in self.place_history.values_mut() {
                            hist.retain(|h| &h.actor != actor);
                        }
                        self.memory_candidates
                            .retain(|m| !m.audience.contains(actor));
                        self.expectation_cues.retain(|c| {
                            c.counterparty.as_ref() != Some(actor) && !c.audience.contains(actor)
                        });
                    }
                    self.expectation_book
                        .forget(target_event_ids, target_actor_ids);
                    self.open_promises = self.expectation_book.promise_summaries();
                    for ev in target_event_ids {
                        self.active_wake_snapshots.remove(ev);
                        self.auto_recall_states.remove(ev);
                        self.procedures.remove(ev);
                    }
                    self.procedures
                        .retain(|_, p| !target_actor_ids.iter().any(|a| p.audience.contains(a)));
                    self.episode_book.forget(target_event_ids, target_actor_ids);
                    self.belief_book.recompute_after_forget(
                        target_event_ids,
                        target_actor_ids,
                        &self.map_prior,
                        &self.map_switches,
                    );
                    for actor in target_actor_ids {
                        self.skill_book.remove_actor(actor);
                        self.ledger.remove_actor(actor);
                    }
                    self.learner
                        .processed_event_ids
                        .retain(|id| !target_event_ids.contains(id));
                    self.sync_pending_beliefs();
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
                                taint: record.taint,
                                now_ms: record.timestamp_ms,
                            },
                            &self.map_prior,
                            &self.map_switches,
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
                Some(EventDetail::LearningUpdate {
                    epoch_id,
                    source_event_id,
                    delta_w,
                    delta_theta_0,
                    ..
                }) if !self
                    .learner
                    .checkpoints
                    .iter()
                    .any(|cp| cp.epoch_id == *epoch_id) =>
                {
                    let cp = crate::learn::LearningEpochCheckpoint {
                        epoch_id: *epoch_id,
                        source_event_id: *source_event_id,
                        timestamp_ms: record.timestamp_ms,
                        weights_before: self.adaptive_weights,
                        daily_abs_delta_before: self.learner.daily_abs_delta,
                        day_index_before: self.learner.day_index,
                    };
                    self.learner.checkpoints.push(cp);
                    self.learner.next_epoch_id =
                        self.learner.next_epoch_id.max(epoch_id.saturating_add(1));
                    self.learner.update_count = self.learner.update_count.saturating_add(1);
                    if source_event_id.as_u64() != 0
                        && !self.learner.processed_event_ids.contains(source_event_id)
                    {
                        self.learner.processed_event_ids.push(*source_event_id);
                    }
                    self.adaptive_weights.salience_weights.w_own_name =
                        (self.adaptive_weights.salience_weights.w_own_name + delta_w[0])
                            .clamp(0.0, 2.0);
                    self.adaptive_weights.salience_weights.w_info =
                        (self.adaptive_weights.salience_weights.w_info + delta_w[1])
                            .clamp(0.0, 2.0);
                    self.adaptive_weights.salience_weights.w_fok =
                        (self.adaptive_weights.salience_weights.w_fok + delta_w[2]).clamp(0.0, 2.0);
                    self.adaptive_weights.salience_weights.w_rel =
                        (self.adaptive_weights.salience_weights.w_rel + delta_w[3]).clamp(0.0, 2.0);
                    self.adaptive_weights.salience_weights.w_commitment =
                        (self.adaptive_weights.salience_weights.w_commitment + delta_w[4])
                            .clamp(0.0, 2.0);
                    self.adaptive_weights.salience_weights.w_sc =
                        (self.adaptive_weights.salience_weights.w_sc + delta_w[5]).clamp(-2.0, 0.0);
                    self.adaptive_weights.theta_0 =
                        (self.adaptive_weights.theta_0 + delta_theta_0).clamp(0.0, 1.0);
                }
                _ => {}
            },
            EventKind::Rollback => {
                if let Some(EventDetail::WeightRollback {
                    restored_epoch_id, ..
                }) = &record.detail
                {
                    let _ = self
                        .learner
                        .rollback_to_epoch(&mut self.adaptive_weights, *restored_epoch_id);
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
        trace: Option<&crate::gate::DecisionTrace>,
    ) {
        // Presence in place P (R-04): audience contains soul or actor is soul.
        if percept.audience.contains(&self.soul_actor_id) || percept.actor == self.soul_actor_id {
            self.place_presence
                .entry(percept.place.clone())
                .or_default()
                .record(percept.timestamp_ms);
        }

        // Track active place participants & ledger witnessed events.
        if percept.actor != self.soul_actor_id && percept.id.as_u64() != 0 {
            self.ledger.record_witnessed(
                &percept.actor,
                percept.id,
                percept.place.clone(),
                percept.timestamp_ms,
                &self.map_prior,
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

        // Track communicative turns for talk_rate.
        self.record_talk_turn(
            &percept.place,
            percept.timestamp_ms,
            percept.actor == self.soul_actor_id,
        );

        // Record place momentum entry and check bilateral closeness exchange (R-29).
        let mut deg_by_actor = BTreeMap::new();
        if let Some(tr) = trace {
            deg_by_actor.insert(self.soul_actor_id.clone(), tr.signals.deg_soul);
            if percept.actor != self.soul_actor_id
                && tr.signals.deg_soul >= self.theta_ex
                && tr.signals.sum_deg_listeners <= self.m_ex
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
            timestamp_ms: percept.timestamp_ms,
            deg_by_actor,
        });
        if hist.len() > MAX_PLACE_HISTORY_ENTRIES {
            let drain = hist.len() - MAX_PLACE_HISTORY_ENTRIES;
            hist.drain(0..drain);
        }

        // Post-evaluation partitioned habituation increment h <- h_pre + 1 (A-03, R-36).
        let (embeddings, channel) = match &percept.detail {
            Some(EventDetail::PerceptMeta {
                embeddings,
                channel,
                ..
            }) => (embeddings.clone(), channel.as_ref()),
            _ => (Vec::new(), None),
        };
        let source_channel = HabituationBook::source_channel_for(channel, &percept.actor);
        self.habituation.record_post_eval(
            percept.place.clone(),
            source_channel,
            percept.timestamp_ms,
            self.habituation_gamma,
        );

        // Index provenance-bearing memory entry + cognitive map vertices (even when embeddings are empty).
        if percept.id.as_u64() != 0 {
            let mut ep_participants = BTreeSet::new();
            ep_participants.insert(percept.actor.clone());
            for m in &percept.audience.members {
                if m != &self.soul_actor_id {
                    ep_participants.insert(m.clone());
                }
            }
            let ep_outcome = self.episode_book.admit_event(
                crate::consolidation::EpisodeAdmitParams {
                    event_id: percept.id,
                    place: &percept.place,
                    timestamp_ms: percept.timestamp_ms,
                    participants: ep_participants,
                    taint: percept.taint,
                    embeddings: &embeddings,
                    missed_expectation: false,
                },
                &self.map_prior,
                &self.map_switches,
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
                    i_surprise: ep_outcome.boundary_eval.i_surprise,
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
        // Keep bounded window (last 24h or 512 turns).
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
        let mut w = BinWriter::new();
        w.raw(SNAPSHOT_MAGIC);
        w.u16(SNAPSHOT_VERSION);
        w.u64(self.last_seq.as_u64());
        w.u64(self.epoch);
        w.u64(self.next_action_seq);
        w.opt_u64(self.last_timestamp_ms.map(Time::as_millis));

        match &self.identity {
            Some(id) => {
                w.u8(1);
                w.bytes(&encode_identity_payload(id));
            }
            None => w.u8(0),
        }

        w.u32(self.body_states.len() as u32);
        for (body_id, status) in &self.body_states {
            w.str(body_id.as_str());
            w.u8(encode_body_status(*status));
        }

        w.u32(self.actor_facts.len() as u32);
        for (actor_id, entry) in &self.actor_facts {
            w.str(actor_id.as_str());
            w.bytes(&encode_actor_facts(entry));
        }

        w.u32(self.in_flight_actions.len() as u32);
        for (action_id, act) in &self.in_flight_actions {
            w.str(action_id.as_str());
            w.str(act.capability_id.as_str());
            w.u32(act.places.len() as u32);
            for place in &act.places {
                w.str(place.as_str());
            }
            w.u64(act.dispatched_at_ms.as_millis());
            w.u64(act.event_id.as_u64());
        }

        w.u32(self.pending_approvals.len() as u32);
        for (action_id, pa) in &self.pending_approvals {
            w.str(action_id.as_str());
            w.str(pa.capability_id.as_str());
            w.opt_str(pa.h_act.as_ref().map(HAct::as_str));
            w.u64(pa.held_at_ms.as_millis());
        }

        w.u32(self.terminal_receipts.len() as u32);
        for (action_id, receipt) in &self.terminal_receipts {
            w.str(action_id.as_str());
            w.str(receipt.capability_id.as_str());
            w.u8(encode_receipt_outcome(receipt.outcome));
            w.opt_str(receipt.reason_code.as_deref());
            w.opt_str(receipt.detail.as_deref());
            match receipt.termination_reason {
                Some(tr) => {
                    w.u8(1);
                    w.u8(encode_term_reason(tr));
                }
                None => w.u8(0),
            }
            w.opt_f32(receipt.progress);
            w.opt_u64(receipt.delivered_units);
            w.opt_str(receipt.h_act.as_ref().map(HAct::as_str));
            w.u64(receipt.timestamp_ms.as_millis());
        }

        w.u32(self.unconsumed_inputs.len() as u32);
        for (&event_id, rec) in &self.unconsumed_inputs {
            w.u64(event_id.as_u64());
            w.bytes(&rec.encode_bytes());
        }

        // C2 state fields
        w.str(self.soul_actor_id.as_str());
        encode_epoch_anchor(&mut w, &self.epoch_anchor);
        encode_adaptive_weights(&mut w, &self.adaptive_weights);

        w.u32(self.place_presence.len() as u32);
        for (place, tr) in &self.place_presence {
            w.str(place.as_str());
            encode_activation_trace(&mut w, tr);
        }

        w.u32(self.actor_exchanges.len() as u32);
        for (actor, tr) in &self.actor_exchanges {
            w.str(actor.as_str());
            encode_activation_trace(&mut w, tr);
        }

        w.u32(self.place_history.len() as u32);
        for (place, entries) in &self.place_history {
            w.str(place.as_str());
            w.u32(entries.len() as u32);
            for entry in entries {
                w.u64(entry.event_id.as_u64());
                w.str(entry.actor.as_str());
                w.u64(entry.timestamp_ms.as_millis());
                w.u32(entry.deg_by_actor.len() as u32);
                for (a, &d) in &entry.deg_by_actor {
                    w.str(a.as_str());
                    w.f64(d);
                }
            }
        }

        w.u32(self.place_participants.len() as u32);
        for (place, set) in &self.place_participants {
            w.str(place.as_str());
            w.u32(set.len() as u32);
            for actor in set {
                w.str(actor.as_str());
            }
        }

        w.u32(self.talk_turns.len() as u32);
        for (place, turns) in &self.talk_turns {
            w.str(place.as_str());
            w.u32(turns.len() as u32);
            for &(ts, is_soul) in turns {
                w.u64(ts.as_millis());
                w.u8(u8::from(is_soul));
            }
        }

        w.u32(self.init_habituation.len() as u32);
        for (place, ih) in &self.init_habituation {
            w.str(place.as_str());
            w.f64(ih.h_init);
            w.u64(ih.last_updated_ms.as_millis());
        }

        w.u32(self.memory_candidates.len() as u32);
        for mem in &self.memory_candidates {
            w.u64(mem.event_id.as_u64());
            encode_audience(&mut w, &mem.audience);
            encode_activation_trace(&mut w, &mem.activation);
            w.f64(mem.importance);
            encode_tagged_embeddings(&mut w, &mem.embeddings);
        }

        w.u32(self.open_promises.len() as u32);
        for (&eid, p) in &self.open_promises {
            w.u64(eid.as_u64());
            w.u64(p.due_ms.as_millis());
            w.f64(p.urgency_weight);
            w.u8(u8::from(p.is_recurring_monitor));
            w.u8(u8::from(p.active));
        }

        w.u32(self.expectation_cues.len() as u32);
        for cue in &self.expectation_cues {
            w.opt_u64(cue.expectation_id.map(EventId::as_u64));
            encode_audience(&mut w, &cue.audience);
            w.opt_str(cue.counterparty.as_ref().map(ActorId::as_str));
            w.u8(u8::from(cue.answers_event));
            encode_tagged_embeddings(&mut w, &cue.embeddings);
        }

        w.u32(self.pending_beliefs.len() as u32);
        for pb in &self.pending_beliefs {
            w.f64(pb.alpha);
            w.f64(pb.beta);
            w.f64(pb.stakes);
            w.u8(u8::from(pb.depends_on_active_goal));
        }

        w.u32(self.event_authors.len() as u32);
        for (&eid, actor) in &self.event_authors {
            w.u64(eid.as_u64());
            w.str(actor.as_str());
        }

        w.u32(self.active_chains.len() as u32);
        for &eid in &self.active_chains {
            w.u64(eid.as_u64());
        }

        match &self.dialogue_act_head {
            Some(head) => {
                w.u8(1);
                w.u32(head.w_gap.len() as u32);
                for &val in &head.w_gap {
                    w.f32(val);
                }
                w.f32(head.b_gap);
                match &head.expected_tag {
                    Some(tag) => {
                        w.u8(1);
                        w.str(&tag.embed_model_ver);
                        w.u32(tag.dim_d);
                        w.str(&tag.prompt_regime_id);
                    }
                    None => w.u8(0),
                }
            }
            None => w.u8(0),
        }

        w.f64(self.daily_budget_used);
        w.f64(self.daily_budget_limit);
        w.u64(self.unconsolidated_count as u64);
        w.opt_u64(self.last_sleep_ms.map(Time::as_millis));
        w.f64(self.theta_ex);
        w.f64(self.m_ex);

        // C3 snapshot fields (v3)
        encode_habituation_book(&mut w, &self.habituation);
        w.f64(self.habituation_gamma);
        encode_patience_estimator(&mut w, &self.patience);
        w.f64(self.patience_decay_rho);
        encode_open_waits(&mut w, &self.open_waits);
        encode_cognitive_map(&mut w, &self.cognitive_map);
        encode_memory_entries(&mut w, &self.memory_entries);
        encode_logged_declass_tokens(&mut w, &self.logged_declass_tokens);

        // C4 snapshot fields (v4)
        self.expectation_book.encode(&mut w);
        encode_capability_descriptors(&mut w, &self.capability_descriptors);

        // C5 snapshot fields (v5)
        encode_c5_state(&mut w, self);

        // C6 snapshot fields (v6)
        encode_c6_state(&mut w, self);

        // C7 snapshot fields (v7)
        encode_c7_state(&mut w, self);

        w.finish()
    }

    /// Decodes a snapshot payload produced by [`Self::encode_snapshot`].
    pub fn decode_snapshot(bytes: &[u8]) -> Result<Self, EventFormatError> {
        let mut r = BinReader::new(bytes);
        let magic = r.fixed::<4>()?;
        if &magic != SNAPSHOT_MAGIC {
            return Err(EventFormatError::MalformedRecord(
                "invalid snapshot magic header".to_owned(),
            ));
        }
        let version = r.u16()?;
        if !(2..=SNAPSHOT_VERSION).contains(&version) {
            return Err(EventFormatError::MalformedRecord(format!(
                "unsupported snapshot version {version} (supported {SNAPSHOT_VERSION})"
            )));
        }

        let last_seq = LogSeq::new(r.u64()?);
        let epoch = r.u64()?;
        let next_action_seq = r.u64()?;
        let last_timestamp_ms = r.opt_u64()?.map(Time::from_millis);

        let identity = match r.u8()? {
            0 => None,
            1 => Some(decode_identity_payload(&r.bytes()?)?),
            tag => {
                return Err(EventFormatError::MalformedRecord(format!(
                    "invalid snapshot identity tag {tag}"
                )));
            }
        };

        let body_len = r.u32()? as usize;
        let mut body_states = BTreeMap::new();
        for _ in 0..body_len {
            let body_id = BodyId::new(r.string()?);
            let status = decode_body_status(r.u8()?)?;
            body_states.insert(body_id, status);
        }

        let af_len = r.u32()? as usize;
        let mut actor_facts = BTreeMap::new();
        for _ in 0..af_len {
            let actor_id = ActorId::new(r.string()?);
            let entry = decode_actor_facts(&r.bytes()?)?;
            actor_facts.insert(actor_id, entry);
        }

        let inf_len = r.u32()? as usize;
        let mut in_flight_actions = BTreeMap::new();
        for _ in 0..inf_len {
            let action_id = ActionId::new(r.string()?);
            let capability_id = CapabilityId::new(r.string()?);
            let place_len = r.u32()? as usize;
            let mut places = Vec::with_capacity(place_len.min(64));
            for _ in 0..place_len {
                places.push(PlaceId::new(r.string()?));
            }
            let dispatched_at_ms = Time::from_millis(r.u64()?);
            let event_id = EventId::new(r.u64()?);
            in_flight_actions.insert(
                action_id.clone(),
                InFlightAction {
                    action_id,
                    capability_id,
                    places,
                    dispatched_at_ms,
                    event_id,
                },
            );
        }

        let pa_len = r.u32()? as usize;
        let mut pending_approvals = BTreeMap::new();
        for _ in 0..pa_len {
            let action_id = ActionId::new(r.string()?);
            let capability_id = CapabilityId::new(r.string()?);
            let h_act = r.opt_string()?.map(HAct::new);
            let held_at_ms = Time::from_millis(r.u64()?);
            pending_approvals.insert(
                action_id.clone(),
                PendingApprovalEntry {
                    action_id,
                    capability_id,
                    h_act,
                    held_at_ms,
                },
            );
        }

        let tr_len = r.u32()? as usize;
        let mut terminal_receipts = BTreeMap::new();
        for _ in 0..tr_len {
            let action_id = ActionId::new(r.string()?);
            let capability_id = CapabilityId::new(r.string()?);
            let outcome = decode_receipt_outcome(r.u8()?)?;
            let reason_code = r.opt_string()?;
            let detail = r.opt_string()?;
            let termination_reason = match r.u8()? {
                0 => None,
                1 => Some(decode_term_reason(r.u8()?)?),
                tag => {
                    return Err(EventFormatError::MalformedRecord(format!(
                        "invalid snapshot termination reason tag {tag}"
                    )));
                }
            };
            let progress = r.opt_f32()?;
            let delivered_units = r.opt_u64()?;
            let h_act = r.opt_string()?.map(HAct::new);
            let timestamp_ms = Time::from_millis(r.u64()?);
            terminal_receipts.insert(
                action_id.clone(),
                Receipt {
                    action_id,
                    capability_id,
                    outcome,
                    reason_code,
                    detail,
                    termination_reason,
                    progress,
                    delivered_units,
                    h_act,
                    timestamp_ms,
                },
            );
        }

        let unc_len = r.u32()? as usize;
        let mut unconsumed_inputs = BTreeMap::new();
        for _ in 0..unc_len {
            let event_id = EventId::new(r.u64()?);
            let rec_bytes = r.bytes()?;
            let rec = EventRecord::decode_and_upcast(&rec_bytes)?;
            unconsumed_inputs.insert(event_id, rec);
        }

        let soul_actor_id = ActorId::new(r.string()?);
        let epoch_anchor = decode_epoch_anchor(&mut r)?;
        let adaptive_weights = decode_adaptive_weights(&mut r)?;

        let pp_len = r.u32()? as usize;
        let mut place_presence = BTreeMap::new();
        for _ in 0..pp_len {
            let place = PlaceId::new(r.string()?);
            let tr = decode_activation_trace(&mut r)?;
            place_presence.insert(place, tr);
        }

        let ae_len = r.u32()? as usize;
        let mut actor_exchanges = BTreeMap::new();
        for _ in 0..ae_len {
            let actor = ActorId::new(r.string()?);
            let tr = decode_activation_trace(&mut r)?;
            actor_exchanges.insert(actor, tr);
        }

        let ph_len = r.u32()? as usize;
        let mut place_history = BTreeMap::new();
        for _ in 0..ph_len {
            let place = PlaceId::new(r.string()?);
            let n = r.u32()? as usize;
            let mut entries = Vec::with_capacity(n.min(256));
            for _ in 0..n {
                let event_id = EventId::new(r.u64()?);
                let actor = ActorId::new(r.string()?);
                let timestamp_ms = Time::from_millis(r.u64()?);
                let deg_n = r.u32()? as usize;
                let mut deg_by_actor = BTreeMap::new();
                for _ in 0..deg_n {
                    let a = ActorId::new(r.string()?);
                    let d = r.f64()?;
                    deg_by_actor.insert(a, d);
                }
                entries.push(PlaceHistoryEntry {
                    event_id,
                    actor,
                    timestamp_ms,
                    deg_by_actor,
                });
            }
            place_history.insert(place, entries);
        }

        let part_len = r.u32()? as usize;
        let mut place_participants = BTreeMap::new();
        for _ in 0..part_len {
            let place = PlaceId::new(r.string()?);
            let n = r.u32()? as usize;
            let mut set = BTreeSet::new();
            for _ in 0..n {
                set.insert(ActorId::new(r.string()?));
            }
            place_participants.insert(place, set);
        }

        let tt_len = r.u32()? as usize;
        let mut talk_turns = BTreeMap::new();
        for _ in 0..tt_len {
            let place = PlaceId::new(r.string()?);
            let n = r.u32()? as usize;
            let mut turns = Vec::with_capacity(n.min(1024));
            for _ in 0..n {
                let ts = Time::from_millis(r.u64()?);
                let is_soul = r.u8()? != 0;
                turns.push((ts, is_soul));
            }
            talk_turns.insert(place, turns);
        }

        let ih_len = r.u32()? as usize;
        let mut init_habituation = BTreeMap::new();
        for _ in 0..ih_len {
            let place = PlaceId::new(r.string()?);
            let h_init = r.f64()?;
            let last_updated_ms = Time::from_millis(r.u64()?);
            init_habituation.insert(
                place,
                InitHabituationEntry {
                    h_init,
                    last_updated_ms,
                },
            );
        }

        let mc_len = r.u32()? as usize;
        let mut memory_candidates = Vec::with_capacity(mc_len.min(4096));
        for _ in 0..mc_len {
            let event_id = EventId::new(r.u64()?);
            let audience = decode_audience(&mut r)?;
            let activation = decode_activation_trace(&mut r)?;
            let importance = r.f64()?;
            let embeddings = decode_tagged_embeddings(&mut r)?;
            memory_candidates.push(CandidateMemory {
                event_id,
                audience,
                activation,
                importance,
                embeddings,
            });
        }

        let op_len = r.u32()? as usize;
        let mut open_promises = BTreeMap::new();
        for _ in 0..op_len {
            let eid = EventId::new(r.u64()?);
            let due_ms = Time::from_millis(r.u64()?);
            let urgency_weight = r.f64()?;
            let is_recurring_monitor = r.u8()? != 0;
            let active = r.u8()? != 0;
            open_promises.insert(
                eid,
                PromiseSummary {
                    due_ms,
                    urgency_weight,
                    is_recurring_monitor,
                    active,
                },
            );
        }

        let ec_len = r.u32()? as usize;
        let mut expectation_cues = Vec::with_capacity(ec_len.min(1024));
        for _ in 0..ec_len {
            let expectation_id = r.opt_u64()?.map(EventId::new);
            let audience = decode_audience(&mut r)?;
            let counterparty = r.opt_string()?.map(ActorId::new);
            let answers_event = r.u8()? != 0;
            let embeddings = decode_tagged_embeddings(&mut r)?;
            expectation_cues.push(ExpectationCue {
                expectation_id,
                audience,
                counterparty,
                answers_event,
                embeddings,
            });
        }

        let pb_len = r.u32()? as usize;
        let mut pending_beliefs = Vec::with_capacity(pb_len.min(1024));
        for _ in 0..pb_len {
            let alpha = r.f64()?;
            let beta = r.f64()?;
            let stakes = r.f64()?;
            let depends_on_active_goal = r.u8()? != 0;
            pending_beliefs.push(PendingBeliefSummary {
                alpha,
                beta,
                stakes,
                depends_on_active_goal,
            });
        }

        let ea_len = r.u32()? as usize;
        let mut event_authors = BTreeMap::new();
        for _ in 0..ea_len {
            let eid = EventId::new(r.u64()?);
            let actor = ActorId::new(r.string()?);
            event_authors.insert(eid, actor);
        }

        let ac_len = r.u32()? as usize;
        let mut active_chains = BTreeSet::new();
        for _ in 0..ac_len {
            active_chains.insert(EventId::new(r.u64()?));
        }

        let dialogue_act_head = match r.u8()? {
            0 => None,
            1 => {
                let w_len = r.u32()? as usize;
                let mut w_gap = Vec::with_capacity(w_len.min(4096));
                for _ in 0..w_len {
                    w_gap.push(r.f32()?);
                }
                let b_gap = r.f32()?;
                let expected_tag = match r.u8()? {
                    0 => None,
                    1 => {
                        let embed_model_ver = r.string()?;
                        let dim_d = r.u32()?;
                        let prompt_regime_id = r.string()?;
                        Some(EmbeddingTag {
                            embed_model_ver,
                            dim_d,
                            prompt_regime_id,
                        })
                    }
                    tag => {
                        return Err(EventFormatError::MalformedRecord(format!(
                            "invalid expected_tag tag {tag}"
                        )));
                    }
                };
                Some(FrozenDialogueActHead {
                    w_gap,
                    b_gap,
                    expected_tag,
                })
            }
            tag => {
                return Err(EventFormatError::MalformedRecord(format!(
                    "invalid dialogue_act_head tag {tag}"
                )));
            }
        };

        let daily_budget_used = r.f64()?;
        let daily_budget_limit = r.f64()?;
        let unconsolidated_count = r.u64()? as usize;
        let last_sleep_ms = r.opt_u64()?.map(Time::from_millis);
        let theta_ex = r.f64()?;
        let m_ex = r.f64()?;

        let (
            habituation,
            habituation_gamma,
            patience,
            patience_decay_rho,
            open_waits,
            cognitive_map,
            memory_entries,
            logged_declass_tokens,
        ) = if version >= 3 {
            (
                decode_habituation_book(&mut r)?,
                r.f64()?,
                decode_patience_estimator(&mut r)?,
                r.f64()?,
                decode_open_waits(&mut r)?,
                decode_cognitive_map(&mut r)?,
                decode_memory_entries(&mut r)?,
                decode_logged_declass_tokens(&mut r)?,
            )
        } else {
            (
                HabituationBook::default(),
                Prior::default().habituation_gamma(),
                PatienceEstimator::default(),
                Prior::default().patience_decay_rho,
                BTreeMap::new(),
                CognitiveMap::new(),
                BTreeMap::new(),
                Vec::new(),
            )
        };

        let (expectation_book, capability_descriptors) = if version >= 4 {
            (
                ExpectationBook::decode(&mut r)?,
                decode_capability_descriptors(&mut r)?,
            )
        } else {
            (ExpectationBook::new(), BTreeMap::new())
        };

        let c5 = if version >= 5 {
            decode_c5_state(&mut r)?
        } else {
            DecodedC5State::from_legacy(identity.as_ref(), &capability_descriptors)
        };
        let c6 = if version >= 6 {
            decode_c6_state(&mut r)?
        } else {
            DecodedC6State::default()
        };
        let c7 = if version >= 7 {
            decode_c7_state(&mut r)?
        } else {
            DecodedC7State::default()
        };

        Ok(Self {
            last_seq,
            epoch,
            next_action_seq,
            last_timestamp_ms,
            identity,
            body_states,
            actor_facts,
            in_flight_actions,
            pending_approvals,
            terminal_receipts,
            unconsumed_inputs,
            soul_actor_id,
            epoch_anchor,
            adaptive_weights,
            place_presence,
            actor_exchanges,
            place_history,
            place_participants,
            talk_turns,
            init_habituation,
            memory_candidates,
            open_promises,
            expectation_cues,
            pending_beliefs,
            event_authors,
            active_chains,
            dialogue_act_head,
            daily_budget_used,
            daily_budget_limit,
            unconsolidated_count,
            last_sleep_ms,
            theta_ex,
            m_ex,
            habituation,
            habituation_gamma,
            patience,
            patience_decay_rho,
            open_waits,
            cognitive_map,
            memory_entries,
            logged_declass_tokens,
            map_prior: Prior::default(),
            expectation_book,
            capability_descriptors,
            startup_recovery_pending: false,
            identity_anchor: c5.identity_anchor,
            principles: c5.principles,
            capability_catalog: c5.capability_catalog,
            procedures: c5.procedures,
            knowledge_cutoff: c5.knowledge_cutoff,
            domain_calibration_offsets: c5.domain_calibration_offsets,
            active_wake_snapshots: BTreeMap::new(),
            auto_recall_states: c5.auto_recall_states,
            recent_receipt_cues: c5.recent_receipt_cues,
            wake_count_since_anchor_check: c5.wake_count_since_anchor_check,
            anchor_check_pending_post_sleep: c5.anchor_check_pending_post_sleep,
            last_sleep_seq: c5.last_sleep_seq,
            reliability_alpha: c5.reliability_alpha,
            reliability_beta: c5.reliability_beta,
            latency_samples_ms: c5.latency_samples_ms,
            turn_allocated_by_trigger: c5.turn_allocated_by_trigger,
            calibration_samples: c5.calibration_samples,
            calibration_brier_sum: c5.calibration_brier_sum,
            principle_alpha: c5.principle_alpha,
            principle_beta: c5.principle_beta,
            contradiction_flags: c5.contradiction_flags,
            belief_book: c6.belief_book,
            domain_clusters: c6.domain_clusters,
            episode_book: c6.episode_book,
            pending_sleep_run: c6.pending_sleep_run,
            last_global_activity_ms: c6.last_global_activity_ms,
            map_switches: Switches::all_enabled(),
            skill_book: c7.skill_book,
            ledger: c7.ledger,
            learner: c7.learner,
            procedure_links: c7.procedure_links,
            recalibration_pending: c7.recalibration_pending,
        })
    }
}

fn encode_capability_descriptors(
    w: &mut BinWriter,
    caps: &BTreeMap<CapabilityId, CapabilityDescriptor>,
) {
    w.u32(caps.len() as u32);
    for (id, cap) in caps {
        w.str(id.as_str());
        w.str(&cap.description);
        w.u32(cap.places.len() as u32);
        for p in &cap.places {
            w.str(p.as_str());
        }
        w.u8(u8::from(cap.flags.communicative));
        w.u8(u8::from(cap.flags.identity_changing));
        w.u8(u8::from(cap.flags.irreversible));
        w.u64(cap.typical_latency_ms);
        w.str(cap.capability_schema_hash.as_str());
        encode_opt_sensed_effect(w, cap.sensed_effect.as_ref());
    }
}

fn decode_capability_descriptors(
    r: &mut BinReader<'_>,
) -> Result<BTreeMap<CapabilityId, CapabilityDescriptor>, EventFormatError> {
    let len = r.u32()? as usize;
    let mut out = BTreeMap::new();
    for _ in 0..len {
        let id = CapabilityId::new(r.string()?);
        let description = r.string()?;
        let plen = r.u32()? as usize;
        let mut places = Vec::with_capacity(plen.min(128));
        for _ in 0..plen {
            places.push(PlaceId::new(r.string()?));
        }
        let communicative = r.u8()? != 0;
        let identity_changing = r.u8()? != 0;
        let irreversible = r.u8()? != 0;
        let typical_latency_ms = r.u64()?;
        let capability_schema_hash = CapabilitySchemaHash::new(r.string()?);
        let sensed_effect = decode_opt_sensed_effect(r)?;
        out.insert(
            id.clone(),
            CapabilityDescriptor {
                id,
                description,
                places,
                flags: CapabilityFlags {
                    communicative,
                    identity_changing,
                    irreversible,
                },
                typical_latency_ms,
                capability_schema_hash,
                sensed_effect,
            },
        );
    }
    Ok(out)
}

fn encode_activation_trace(w: &mut BinWriter, tr: &ActivationTrace) {
    w.u64(tr.count);
    w.u64(tr.first_ms.as_millis());
    w.u64(tr.last_ms.as_millis());
}

fn decode_activation_trace(r: &mut BinReader<'_>) -> Result<ActivationTrace, EventFormatError> {
    Ok(ActivationTrace {
        count: r.u64()?,
        first_ms: Time::from_millis(r.u64()?),
        last_ms: Time::from_millis(r.u64()?),
    })
}

fn encode_adaptive_weights(w: &mut BinWriter, aw: &AdaptiveWeightsAnchor) {
    w.f64(aw.salience_weights.w_own_name);
    w.f64(aw.salience_weights.w_info);
    w.f64(aw.salience_weights.w_fok);
    w.f64(aw.salience_weights.w_rel);
    w.f64(aw.salience_weights.w_commitment);
    w.f64(aw.salience_weights.w_sc);
    w.f64(aw.theta_0);
    w.f64(aw.a);
    w.f64(aw.b);
    w.f64(aw.c);
    w.f64(aw.r);
}

fn decode_adaptive_weights(
    r: &mut BinReader<'_>,
) -> Result<AdaptiveWeightsAnchor, EventFormatError> {
    Ok(AdaptiveWeightsAnchor {
        salience_weights: SalienceWeights {
            w_own_name: r.f64()?,
            w_info: r.f64()?,
            w_fok: r.f64()?,
            w_rel: r.f64()?,
            w_commitment: r.f64()?,
            w_sc: r.f64()?,
        },
        theta_0: r.f64()?,
        a: r.f64()?,
        b: r.f64()?,
        c: r.f64()?,
        r: r.f64()?,
    })
}

fn encode_epoch_anchor(w: &mut BinWriter, anchor: &EpochAnchor) {
    w.u64(anchor.epoch);
    w.u64(anchor.anchor_seq.as_u64());
    w.u8(u8::from(anchor.frozen));
    match &anchor.frozen_weights {
        Some(fw) => {
            w.u8(1);
            encode_adaptive_weights(w, fw);
        }
        None => w.u8(0),
    }
}

fn decode_epoch_anchor(r: &mut BinReader<'_>) -> Result<EpochAnchor, EventFormatError> {
    let epoch = r.u64()?;
    let anchor_seq = LogSeq::new(r.u64()?);
    let frozen = r.u8()? != 0;
    let frozen_weights = match r.u8()? {
        0 => None,
        1 => Some(decode_adaptive_weights(r)?),
        tag => {
            return Err(EventFormatError::MalformedRecord(format!(
                "invalid epoch anchor frozen_weights tag {tag}"
            )));
        }
    };
    Ok(EpochAnchor {
        epoch,
        anchor_seq,
        frozen,
        frozen_weights,
    })
}

fn encode_audience(w: &mut BinWriter, aud: &Audience) {
    w.u32(aud.members.len() as u32);
    for m in &aud.members {
        w.str(m.as_str());
    }
}

fn decode_audience(r: &mut BinReader<'_>) -> Result<Audience, EventFormatError> {
    let len = r.u32()? as usize;
    let mut members = Vec::with_capacity(len.min(1024));
    for _ in 0..len {
        members.push(ActorId::new(r.string()?));
    }
    Ok(Audience::new(members))
}

fn encode_tagged_embeddings(w: &mut BinWriter, embs: &[TaggedEmbedding]) {
    w.u32(embs.len() as u32);
    for emb in embs {
        w.str(&emb.tag.embed_model_ver);
        w.u32(emb.tag.dim_d);
        w.str(&emb.tag.prompt_regime_id);
        w.u32(emb.values.len() as u32);
        for &v in &emb.values {
            w.f32(v);
        }
    }
}

fn decode_tagged_embeddings(
    r: &mut BinReader<'_>,
) -> Result<Vec<TaggedEmbedding>, EventFormatError> {
    let len = r.u32()? as usize;
    let mut out = Vec::with_capacity(len.min(64));
    for _ in 0..len {
        let embed_model_ver = r.string()?;
        let dim_d = r.u32()?;
        let prompt_regime_id = r.string()?;
        let vlen = r.u32()? as usize;
        let mut values = Vec::with_capacity(vlen.min(4096));
        for _ in 0..vlen {
            values.push(r.f32()?);
        }
        out.push(TaggedEmbedding {
            tag: EmbeddingTag {
                embed_model_ver,
                dim_d,
                prompt_regime_id,
            },
            values,
        });
    }
    Ok(out)
}

pub(crate) fn encode_body_status(status: BodyStatus) -> u8 {
    match status {
        BodyStatus::Ready => 1,
        BodyStatus::Busy => 2,
        BodyStatus::Degraded => 3,
        BodyStatus::Stopped => 4,
    }
}

pub(crate) fn decode_body_status(code: u8) -> Result<BodyStatus, EventFormatError> {
    match code {
        1 => Ok(BodyStatus::Ready),
        2 => Ok(BodyStatus::Busy),
        3 => Ok(BodyStatus::Degraded),
        4 => Ok(BodyStatus::Stopped),
        other => Err(EventFormatError::MalformedRecord(format!(
            "invalid body status code {other}"
        ))),
    }
}

fn parse_body_status(bytes: &[u8]) -> Result<BodyStatus, EventFormatError> {
    if bytes.is_empty() {
        return Err(EventFormatError::MalformedRecord(
            "empty body status payload".to_owned(),
        ));
    }
    decode_body_status(bytes[0])
}

pub(crate) fn encode_actor_facts_from_msg(facts: &ActorFacts) -> Vec<u8> {
    encode_actor_facts(&ActorFactsEntry {
        alpha_0: facts.alpha_0,
        beta_0: facts.beta_0,
        teacher_weight: facts.teacher_weight,
        trusted_origin: facts.trusted_origin,
    })
}

fn encode_actor_facts(entry: &ActorFactsEntry) -> Vec<u8> {
    let mut w = BinWriter::new();
    w.f64(entry.alpha_0);
    w.f64(entry.beta_0);
    w.f64(entry.teacher_weight);
    w.u8(u8::from(entry.trusted_origin));
    w.finish()
}

fn decode_actor_facts(bytes: &[u8]) -> Result<ActorFactsEntry, EventFormatError> {
    let mut r = BinReader::new(bytes);
    Ok(ActorFactsEntry {
        alpha_0: r.f64()?,
        beta_0: r.f64()?,
        teacher_weight: r.f64()?,
        trusted_origin: r.u8()? != 0,
    })
}

pub(crate) fn encode_identity_payload(signed: &OwnerSigned<OpaquePayload>) -> Vec<u8> {
    let mut w = BinWriter::new();
    w.str(signed.signer.as_str());
    w.u64(signed.signed_at_ms.as_millis());
    w.str(&signed.signature);
    w.bytes(signed.payload.as_bytes());
    w.finish()
}

fn decode_identity_payload(bytes: &[u8]) -> Result<OwnerSigned<OpaquePayload>, EventFormatError> {
    let mut r = BinReader::new(bytes);
    let signer = ActorId::new(r.string()?);
    let signed_at_ms = Time::from_millis(r.u64()?);
    let signature = r.string()?;
    let payload = OpaquePayload::from_bytes(r.bytes()?);
    Ok(OwnerSigned {
        payload,
        signer,
        signed_at_ms,
        signature,
    })
}

fn encode_receipt_outcome(o: ReceiptOutcome) -> u8 {
    match o {
        ReceiptOutcome::Done => 1,
        ReceiptOutcome::Refused => 2,
        ReceiptOutcome::Held => 3,
        ReceiptOutcome::Failed => 4,
        ReceiptOutcome::Unknown => 5,
    }
}

fn decode_receipt_outcome(code: u8) -> Result<ReceiptOutcome, EventFormatError> {
    match code {
        1 => Ok(ReceiptOutcome::Done),
        2 => Ok(ReceiptOutcome::Refused),
        3 => Ok(ReceiptOutcome::Held),
        4 => Ok(ReceiptOutcome::Failed),
        5 => Ok(ReceiptOutcome::Unknown),
        other => Err(EventFormatError::MalformedRecord(format!(
            "invalid receipt outcome code {other}"
        ))),
    }
}

fn encode_term_reason(r: monoagens_protocol::TerminationReason) -> u8 {
    match r {
        monoagens_protocol::TerminationReason::Normal => 1,
        monoagens_protocol::TerminationReason::CanceledByCore => 2,
        monoagens_protocol::TerminationReason::PreemptedExternal => 3,
        monoagens_protocol::TerminationReason::TimedOut => 4,
    }
}

fn decode_term_reason(code: u8) -> Result<monoagens_protocol::TerminationReason, EventFormatError> {
    match code {
        1 => Ok(monoagens_protocol::TerminationReason::Normal),
        2 => Ok(monoagens_protocol::TerminationReason::CanceledByCore),
        3 => Ok(monoagens_protocol::TerminationReason::PreemptedExternal),
        4 => Ok(monoagens_protocol::TerminationReason::TimedOut),
        other => Err(EventFormatError::MalformedRecord(format!(
            "invalid termination reason code {other}"
        ))),
    }
}

fn encode_habituation_book(w: &mut BinWriter, book: &HabituationBook) {
    let entries = book.entries();
    w.u32(entries.len() as u32);
    for (key, entry) in entries {
        w.str(key.place.as_str());
        w.str(key.source_channel.as_str());
        w.f64(entry.h);
        w.u64(entry.last_updated_ms.as_millis());
        w.u64(entry.count);
    }
}

fn decode_habituation_book(r: &mut BinReader<'_>) -> Result<HabituationBook, EventFormatError> {
    let len = r.u32()? as usize;
    let mut book = HabituationBook::default();
    for _ in 0..len {
        let place = PlaceId::new(r.string()?);
        let source_channel = ChannelId::new(r.string()?);
        let h = r.f64()?;
        let last_updated_ms = Time::from_millis(r.u64()?);
        let count = r.u64()?;
        book.insert_entry(
            HabituationKey {
                place,
                source_channel,
            },
            HabituationEntry {
                h,
                last_updated_ms,
                count,
            },
        );
    }
    Ok(book)
}

fn encode_latency_sketch(w: &mut BinWriter, sk: &LatencySketch) {
    w.u32(sk.events.len() as u32);
    for (&b, &v) in &sk.events {
        w.u16(b);
        w.f64(v);
    }
    w.u32(sk.censored.len() as u32);
    for (&b, &v) in &sk.censored {
        w.u16(b);
        w.f64(v);
    }
}

fn decode_latency_sketch(r: &mut BinReader<'_>) -> Result<LatencySketch, EventFormatError> {
    let ev_len = r.u32()? as usize;
    let mut events = BTreeMap::new();
    for _ in 0..ev_len {
        let b = r.u16()?;
        let v = r.f64()?;
        events.insert(b, v);
    }
    let cen_len = r.u32()? as usize;
    let mut censored = BTreeMap::new();
    for _ in 0..cen_len {
        let b = r.u16()?;
        let v = r.f64()?;
        censored.insert(b, v);
    }
    Ok(LatencySketch { events, censored })
}

fn encode_patience_estimator(w: &mut BinWriter, est: &PatienceEstimator) {
    w.u32(est.by_place_medium.len() as u32);
    for ((place, med), sk) in &est.by_place_medium {
        w.str(place.as_str());
        w.str(med.as_str());
        encode_latency_sketch(w, sk);
    }
    w.u32(est.by_medium.len() as u32);
    for (med, sk) in &est.by_medium {
        w.str(med.as_str());
        encode_latency_sketch(w, sk);
    }
}

fn decode_patience_estimator(r: &mut BinReader<'_>) -> Result<PatienceEstimator, EventFormatError> {
    let pm_len = r.u32()? as usize;
    let mut by_place_medium = BTreeMap::new();
    for _ in 0..pm_len {
        let place = PlaceId::new(r.string()?);
        let med = ChannelId::new(r.string()?);
        let sk = decode_latency_sketch(r)?;
        by_place_medium.insert((place, med), sk);
    }
    let m_len = r.u32()? as usize;
    let mut by_medium = BTreeMap::new();
    for _ in 0..m_len {
        let med = ChannelId::new(r.string()?);
        let sk = decode_latency_sketch(r)?;
        by_medium.insert(med, sk);
    }
    Ok(PatienceEstimator {
        by_place_medium,
        by_medium,
    })
}

fn encode_open_waits(w: &mut BinWriter, waits: &BTreeMap<EventId, OpenWait>) {
    w.u32(waits.len() as u32);
    for (&eid, wait) in waits {
        w.u64(eid.as_u64());
        w.u64(wait.target_event_id.as_u64());
        w.str(wait.place.as_str());
        w.str(wait.sender.as_str());
        w.str(wait.medium.as_str());
        encode_audience(w, &wait.audience);
        w.u64(wait.opened_at_ms.as_millis());
        w.u64(wait.due_ms.as_millis());
    }
}

fn decode_open_waits(
    r: &mut BinReader<'_>,
) -> Result<BTreeMap<EventId, OpenWait>, EventFormatError> {
    let len = r.u32()? as usize;
    let mut out = BTreeMap::new();
    for _ in 0..len {
        let expectation_id = EventId::new(r.u64()?);
        let target_event_id = EventId::new(r.u64()?);
        let place = PlaceId::new(r.string()?);
        let sender = ActorId::new(r.string()?);
        let medium = ChannelId::new(r.string()?);
        let audience = decode_audience(r)?;
        let opened_at_ms = Time::from_millis(r.u64()?);
        let due_ms = Time::from_millis(r.u64()?);
        out.insert(
            expectation_id,
            OpenWait {
                expectation_id,
                target_event_id,
                place,
                sender,
                medium,
                audience,
                opened_at_ms,
                due_ms,
            },
        );
    }
    Ok(out)
}

fn encode_map_vertex(w: &mut BinWriter, v: &MapVertex) {
    match v {
        MapVertex::Actor(id) => {
            w.u8(1);
            w.str(id.as_str());
        }
        MapVertex::Place(id) => {
            w.u8(2);
            w.str(id.as_str());
        }
        MapVertex::Concept(id) => {
            w.u8(3);
            w.str(id);
        }
    }
}

fn decode_map_vertex(r: &mut BinReader<'_>) -> Result<MapVertex, EventFormatError> {
    match r.u8()? {
        1 => Ok(MapVertex::Actor(ActorId::new(r.string()?))),
        2 => Ok(MapVertex::Place(PlaceId::new(r.string()?))),
        3 => Ok(MapVertex::Concept(r.string()?)),
        tag => Err(EventFormatError::MalformedRecord(format!(
            "invalid map vertex tag {tag}"
        ))),
    }
}

fn encode_cognitive_map(w: &mut BinWriter, map: &CognitiveMap) {
    w.u32(map.co_occurrence.len() as u32);
    for (u, row) in &map.co_occurrence {
        encode_map_vertex(w, u);
        w.u32(row.len() as u32);
        for (v, edge) in row {
            encode_map_vertex(w, v);
            w.f64(edge.weight);
            w.u64(edge.last_updated_ms.as_millis());
        }
    }
    w.u32(map.sr_matrix.len() as u32);
    for (u, row) in &map.sr_matrix {
        encode_map_vertex(w, u);
        w.u32(row.len() as u32);
        for (v, &val) in row {
            encode_map_vertex(w, v);
            w.f64(val);
        }
    }
    w.u32(map.last_place_anchor.len() as u32);
    for (place, (v, ts)) in &map.last_place_anchor {
        w.str(place.as_str());
        encode_map_vertex(w, v);
        w.u64(ts.as_millis());
    }
    w.u32(map.vertex_events.len() as u32);
    for (v, eids) in &map.vertex_events {
        encode_map_vertex(w, v);
        w.u32(eids.len() as u32);
        for &eid in eids {
            w.u64(eid.as_u64());
        }
    }
    w.u32(map.event_vertices.len() as u32);
    for (&eid, verts) in &map.event_vertices {
        w.u64(eid.as_u64());
        w.u32(verts.len() as u32);
        for v in verts {
            encode_map_vertex(w, v);
        }
    }
}

fn decode_cognitive_map(r: &mut BinReader<'_>) -> Result<CognitiveMap, EventFormatError> {
    let co_len = r.u32()? as usize;
    let mut co_occurrence = BTreeMap::new();
    for _ in 0..co_len {
        let u = decode_map_vertex(r)?;
        let row_len = r.u32()? as usize;
        let mut row = BTreeMap::new();
        for _ in 0..row_len {
            let v = decode_map_vertex(r)?;
            let weight = r.f64()?;
            let last_updated_ms = Time::from_millis(r.u64()?);
            row.insert(
                v,
                CoOccurrenceEdge {
                    weight,
                    last_updated_ms,
                },
            );
        }
        co_occurrence.insert(u, row);
    }

    let sr_len = r.u32()? as usize;
    let mut sr_matrix = BTreeMap::new();
    for _ in 0..sr_len {
        let u = decode_map_vertex(r)?;
        let row_len = r.u32()? as usize;
        let mut row = BTreeMap::new();
        for _ in 0..row_len {
            let v = decode_map_vertex(r)?;
            let val = r.f64()?;
            row.insert(v, val);
        }
        sr_matrix.insert(u, row);
    }

    let anchor_len = r.u32()? as usize;
    let mut last_place_anchor = BTreeMap::new();
    for _ in 0..anchor_len {
        let place = PlaceId::new(r.string()?);
        let v = decode_map_vertex(r)?;
        let ts = Time::from_millis(r.u64()?);
        last_place_anchor.insert(place, (v, ts));
    }

    let ve_len = r.u32()? as usize;
    let mut vertex_events = BTreeMap::new();
    for _ in 0..ve_len {
        let v = decode_map_vertex(r)?;
        let n = r.u32()? as usize;
        let mut set = BTreeSet::new();
        for _ in 0..n {
            set.insert(EventId::new(r.u64()?));
        }
        vertex_events.insert(v, set);
    }

    let ev_len = r.u32()? as usize;
    let mut event_vertices = BTreeMap::new();
    for _ in 0..ev_len {
        let eid = EventId::new(r.u64()?);
        let n = r.u32()? as usize;
        let mut verts = Vec::with_capacity(n.min(256));
        for _ in 0..n {
            verts.push(decode_map_vertex(r)?);
        }
        event_vertices.insert(eid, verts);
    }

    Ok(CognitiveMap {
        co_occurrence,
        sr_matrix,
        last_place_anchor,
        vertex_events,
        event_vertices,
    })
}

fn encode_memory_entries(w: &mut BinWriter, entries: &BTreeMap<EventId, MemoryEntry>) {
    w.u32(entries.len() as u32);
    for (&eid, entry) in entries {
        w.u64(eid.as_u64());
        w.u32(entry.source_ids.len() as u32);
        for &sid in &entry.source_ids {
            w.u64(sid.as_u64());
        }
        w.u8(u8::from(entry.is_gist));
        w.u64(entry.timestamp_ms.as_millis());
        w.str(entry.place.as_str());
        w.str(entry.actor.as_str());
        w.str(entry.origin_actor.actor_id().as_str());
        w.u8(entry.basis.code());
        w.u8(match entry.taint {
            Taint::Untrusted => 0,
            Taint::Provisional => 1,
            Taint::Trusted => 2,
        });
        encode_audience(w, &entry.audience);
        w.u32(entry.attachments.len() as u32);
        for att in &entry.attachments {
            w.str(&att.handle_id);
            w.u8(match att.modality_slot {
                ModalitySlot::Image => 1,
                ModalitySlot::AudioVoice => 2,
                ModalitySlot::Video => 3,
                ModalitySlot::SensorBlob => 4,
            });
            w.u64(att.byte_len);
            w.u64(att.expires_at_ms.as_millis());
            w.str(&att.embed_model_ver);
        }
        encode_activation_trace(w, &entry.activation);
        w.f64(entry.importance.i_reason);
        w.u8(u8::from(entry.importance.linked_to_commitment));
        w.f64(entry.importance.i_surprise);
        encode_tagged_embeddings(w, &entry.embeddings);
        w.u32(entry.concepts.len() as u32);
        for c in &entry.concepts {
            w.str(c);
        }
        w.opt_u64(entry.superseded_by.map(EventId::as_u64));
        w.u8(u8::from(entry.erased));
    }
}

fn decode_memory_entries(
    r: &mut BinReader<'_>,
) -> Result<BTreeMap<EventId, MemoryEntry>, EventFormatError> {
    let len = r.u32()? as usize;
    let mut out = BTreeMap::new();
    for _ in 0..len {
        let event_id = EventId::new(r.u64()?);
        let src_len = r.u32()? as usize;
        let mut source_ids = Vec::with_capacity(src_len.min(1024));
        for _ in 0..src_len {
            source_ids.push(EventId::new(r.u64()?));
        }
        let is_gist = r.u8()? != 0;
        let timestamp_ms = Time::from_millis(r.u64()?);
        let place = PlaceId::new(r.string()?);
        let actor = ActorId::new(r.string()?);
        let origin_actor = OriginActor::new(ActorId::new(r.string()?));
        let basis_code = r.u8()?;
        let basis = MemoryBasis::from_code(basis_code).ok_or_else(|| {
            EventFormatError::MalformedRecord(format!("invalid memory basis code {basis_code}"))
        })?;
        let taint = match r.u8()? {
            0 => Taint::Untrusted,
            1 => Taint::Provisional,
            2 => Taint::Trusted,
            tag => {
                return Err(EventFormatError::MalformedRecord(format!(
                    "invalid memory taint tag {tag}"
                )));
            }
        };
        let audience = decode_audience(r)?;
        let att_len = r.u32()? as usize;
        let mut attachments = Vec::with_capacity(att_len.min(256));
        for _ in 0..att_len {
            let handle_id = r.string()?;
            let modality_slot = match r.u8()? {
                1 => ModalitySlot::Image,
                2 => ModalitySlot::AudioVoice,
                3 => ModalitySlot::Video,
                4 => ModalitySlot::SensorBlob,
                tag => {
                    return Err(EventFormatError::MalformedRecord(format!(
                        "invalid attachment modality tag {tag}"
                    )));
                }
            };
            let byte_len = r.u64()?;
            let expires_at_ms = Time::from_millis(r.u64()?);
            let embed_model_ver = r.string()?;
            attachments.push(AttachmentHandle {
                handle_id,
                modality_slot,
                byte_len,
                expires_at_ms,
                embed_model_ver,
            });
        }
        let activation = decode_activation_trace(r)?;
        let i_reason = r.f64()?;
        let linked_to_commitment = r.u8()? != 0;
        let i_surprise = r.f64()?;
        let embeddings = decode_tagged_embeddings(r)?;
        let con_len = r.u32()? as usize;
        let mut concepts = Vec::with_capacity(con_len.min(256));
        for _ in 0..con_len {
            concepts.push(r.string()?);
        }
        let superseded_by = r.opt_u64()?.map(EventId::new);
        let erased = r.u8()? != 0;
        out.insert(
            event_id,
            MemoryEntry {
                event_id,
                source_ids,
                is_gist,
                timestamp_ms,
                place,
                actor,
                origin_actor,
                basis,
                taint,
                audience,
                attachments,
                activation,
                importance: ImportanceInputs {
                    i_reason,
                    linked_to_commitment,
                    i_surprise,
                },
                embeddings,
                concepts,
                superseded_by,
                erased,
            },
        );
    }
    Ok(out)
}

fn encode_logged_declass_tokens(w: &mut BinWriter, tokens: &[OwnerSigned<DeclassToken>]) {
    w.u32(tokens.len() as u32);
    for signed in tokens {
        w.str(signed.signer.as_str());
        w.u64(signed.signed_at_ms.as_millis());
        w.str(&signed.signature);
        w.str(&signed.payload.token_id);
        w.str(signed.payload.issuer.as_str());
        w.u32(signed.payload.scope.len() as u32);
        for id in &signed.payload.scope {
            w.u64(id.as_u64());
        }
        w.u64(signed.payload.expiry.as_millis());
        w.opt_str(signed.payload.target_place.as_ref().map(PlaceId::as_str));
        encode_audience(w, &signed.payload.target_audience);
        w.str(&signed.payload.signature);
    }
}

fn decode_logged_declass_tokens(
    r: &mut BinReader<'_>,
) -> Result<Vec<OwnerSigned<DeclassToken>>, EventFormatError> {
    let len = r.u32()? as usize;
    let mut out = Vec::with_capacity(len.min(1024));
    for _ in 0..len {
        let signer = ActorId::new(r.string()?);
        let signed_at_ms = Time::from_millis(r.u64()?);
        let outer_sig = r.string()?;
        let token_id = r.string()?;
        let issuer = ActorId::new(r.string()?);
        let scope_len = r.u32()? as usize;
        let mut scope = Vec::with_capacity(scope_len.min(1024));
        for _ in 0..scope_len {
            scope.push(EventId::new(r.u64()?));
        }
        let expiry = Time::from_millis(r.u64()?);
        let target_place = r.opt_string()?.map(PlaceId::new);
        let target_audience = decode_audience(r)?;
        let inner_sig = r.string()?;
        out.push(OwnerSigned {
            payload: DeclassToken {
                token_id,
                issuer,
                scope,
                expiry,
                target_place,
                target_audience,
                signature: inner_sig,
            },
            signer,
            signed_at_ms,
            signature: outer_sig,
        });
    }
    Ok(out)
}

struct DecodedC5State {
    identity_anchor: Option<OwnerSigned<IdentityAnchor>>,
    principles: Vec<OwnerSigned<PrincipleDescriptor>>,
    capability_catalog: BTreeMap<CapabilityId, CapabilityCatalogEntry>,
    procedures: BTreeMap<EventId, ProcedureEntry>,
    knowledge_cutoff: Option<String>,
    domain_calibration_offsets: BTreeMap<DomainId, f64>,
    auto_recall_states: BTreeMap<EventId, AutoRecallTriggerState>,
    recent_receipt_cues: Vec<String>,
    wake_count_since_anchor_check: u64,
    anchor_check_pending_post_sleep: bool,
    last_sleep_seq: LogSeq,
    reliability_alpha: f64,
    reliability_beta: f64,
    latency_samples_ms: Vec<u64>,
    turn_allocated_by_trigger: BTreeMap<EventId, bool>,
    calibration_samples: u64,
    calibration_brier_sum: f64,
    principle_alpha: f64,
    principle_beta: f64,
    contradiction_flags: u64,
}

impl DecodedC5State {
    fn from_legacy(
        identity: Option<&OwnerSigned<OpaquePayload>>,
        capability_descriptors: &BTreeMap<CapabilityId, CapabilityDescriptor>,
    ) -> Self {
        let mut capability_catalog = BTreeMap::new();
        for (id, desc) in capability_descriptors {
            capability_catalog.insert(
                id.clone(),
                CapabilityCatalogEntry {
                    descriptor: desc.clone(),
                    is_static_core: false,
                    owner_signed: false,
                    active: true,
                    audience: Audience::new([ActorId::new("soul")]),
                    embedding: None,
                },
            );
        }
        Self {
            identity_anchor: identity.map(IdentityAnchor::from_owner_signed),
            principles: Vec::new(),
            capability_catalog,
            procedures: BTreeMap::new(),
            knowledge_cutoff: None,
            domain_calibration_offsets: BTreeMap::new(),
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
        }
    }
}

fn encode_c5_state(w: &mut BinWriter, state: &CoreState) {
    match &state.identity_anchor {
        None => w.u8(0),
        Some(signed) => {
            w.u8(1);
            w.str(signed.signer.as_str());
            w.u64(signed.signed_at_ms.as_millis());
            w.str(&signed.signature);
            w.str(&signed.payload.anchor_id);
            w.bytes(&signed.payload.identity_blob_hash);
            w.bytes(&signed.payload.owner_sig_hash);
            w.bytes(signed.payload.payload.as_bytes());
        }
    }

    w.u32(state.principles.len() as u32);
    for signed in &state.principles {
        w.str(signed.signer.as_str());
        w.u64(signed.signed_at_ms.as_millis());
        w.str(&signed.signature);
        w.str(&signed.payload.id);
        let sev_tag = match signed.payload.severity {
            PrincipleSeverity::HardLine => 0,
            PrincipleSeverity::StrongNorm => 1,
            PrincipleSeverity::Advisory => 2,
        };
        w.u8(sev_tag);
        w.u32(signed.payload.blocked_capabilities.len() as u32);
        for cap in &signed.payload.blocked_capabilities {
            w.str(cap.as_str());
        }
        w.u8(u8::from(signed.payload.forbid_identity_change));
        w.u8(u8::from(signed.payload.forbid_untrusted_taint));
        w.u64(signed.payload.created_seq.as_u64());
        w.bytes(&signed.payload.owner_sig_hash);
    }

    w.u32(state.capability_catalog.len() as u32);
    for (cap_id, entry) in &state.capability_catalog {
        w.str(cap_id.as_str());
        w.u8(u8::from(entry.is_static_core));
        w.u8(u8::from(entry.owner_signed));
        w.u8(u8::from(entry.active));
        encode_audience(w, &entry.audience);
        let single_map: BTreeMap<CapabilityId, CapabilityDescriptor> =
            [(cap_id.clone(), entry.descriptor.clone())]
                .into_iter()
                .collect();
        encode_capability_descriptors(w, &single_map);
        let embs: Vec<TaggedEmbedding> = entry.embedding.iter().cloned().collect();
        encode_tagged_embeddings(w, &embs);
    }

    w.u32(state.procedures.len() as u32);
    for (proc_id, proc_entry) in &state.procedures {
        w.u64(proc_id.as_u64());
        w.str(&proc_entry.name);
        w.u32(proc_entry.steps.len() as u32);
        for step in &proc_entry.steps {
            w.str(step);
        }
        w.u32(proc_entry.required_cap_ids.len() as u32);
        for cid in &proc_entry.required_cap_ids {
            w.str(cid.as_str());
        }
        w.str(proc_entry.capability_schema_hash.as_str());
        encode_audience(w, &proc_entry.audience);
        w.opt_str(proc_entry.domain.as_ref().map(DomainId::as_str));
        w.u8(u8::from(proc_entry.active));
        w.u8(u8::from(proc_entry.retired));
        w.u8(u8::from(proc_entry.owner_signed));
        let embs: Vec<TaggedEmbedding> = proc_entry.embedding.iter().cloned().collect();
        encode_tagged_embeddings(w, &embs);
    }

    w.opt_str(state.knowledge_cutoff.as_deref());
    w.u32(state.domain_calibration_offsets.len() as u32);
    for (domain, offset) in &state.domain_calibration_offsets {
        w.str(domain.as_str());
        w.f64(*offset);
    }

    w.u32(state.auto_recall_states.len() as u32);
    for (ev_id, ar) in &state.auto_recall_states {
        w.u64(ev_id.as_u64());
        w.u8(u8::from(ar.active));
        w.u8(ar.attempt);
        w.u8(ar.max_attempts);
        w.u8(u8::from(ar.coarser_grain_allowed));
        w.opt_str(ar.last_reason.as_deref());
    }

    w.u32(state.recent_receipt_cues.len() as u32);
    for cue in &state.recent_receipt_cues {
        w.str(cue);
    }
    w.u64(state.wake_count_since_anchor_check);
    w.u8(u8::from(state.anchor_check_pending_post_sleep));
    w.u64(state.last_sleep_seq.as_u64());
    w.f64(state.reliability_alpha);
    w.f64(state.reliability_beta);
    w.u32(state.latency_samples_ms.len() as u32);
    for &ms in &state.latency_samples_ms {
        w.u64(ms);
    }
    w.u32(state.turn_allocated_by_trigger.len() as u32);
    for (ev_id, alloc) in &state.turn_allocated_by_trigger {
        w.u64(ev_id.as_u64());
        w.u8(u8::from(*alloc));
    }
    w.u64(state.calibration_samples);
    w.f64(state.calibration_brier_sum);
    w.f64(state.principle_alpha);
    w.f64(state.principle_beta);
    w.u64(state.contradiction_flags);
}

fn decode_fixed_32(r: &mut BinReader<'_>) -> Result<[u8; 32], EventFormatError> {
    let bytes = r.bytes()?;
    if bytes.len() != 32 {
        return Err(EventFormatError::MalformedRecord(
            "expected 32-byte digest".to_owned(),
        ));
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(&bytes);
    Ok(out)
}

fn decode_c5_state(r: &mut BinReader<'_>) -> Result<DecodedC5State, EventFormatError> {
    let identity_anchor = match r.u8()? {
        0 => None,
        1 => {
            let signer = ActorId::new(r.string()?);
            let signed_at_ms = Time::from_millis(r.u64()?);
            let signature = r.string()?;
            let anchor_id = r.string()?;
            let identity_blob_hash = decode_fixed_32(r)?;
            let owner_sig_hash = decode_fixed_32(r)?;
            let payload = OpaquePayload::from_bytes(r.bytes()?);
            Some(OwnerSigned {
                payload: IdentityAnchor {
                    anchor_id,
                    identity_blob_hash,
                    owner_sig_hash,
                    payload,
                },
                signer,
                signed_at_ms,
                signature,
            })
        }
        tag => {
            return Err(EventFormatError::MalformedRecord(format!(
                "invalid c5 identity_anchor tag {tag}"
            )));
        }
    };

    let p_len = r.u32()? as usize;
    let mut principles = Vec::with_capacity(p_len.min(256));
    for _ in 0..p_len {
        let signer = ActorId::new(r.string()?);
        let signed_at_ms = Time::from_millis(r.u64()?);
        let signature = r.string()?;
        let id = r.string()?;
        let severity = match r.u8()? {
            0 => PrincipleSeverity::HardLine,
            1 => PrincipleSeverity::StrongNorm,
            2 => PrincipleSeverity::Advisory,
            tag => {
                return Err(EventFormatError::MalformedRecord(format!(
                    "invalid principle severity tag {tag}"
                )));
            }
        };
        let b_len = r.u32()? as usize;
        let mut blocked_capabilities = Vec::with_capacity(b_len.min(256));
        for _ in 0..b_len {
            blocked_capabilities.push(CapabilityId::new(r.string()?));
        }
        let forbid_identity_change = r.u8()? != 0;
        let forbid_untrusted_taint = r.u8()? != 0;
        let created_seq = LogSeq::new(r.u64()?);
        let owner_sig_hash = decode_fixed_32(r)?;
        principles.push(OwnerSigned {
            payload: PrincipleDescriptor {
                id,
                severity,
                blocked_capabilities,
                forbid_identity_change,
                forbid_untrusted_taint,
                created_seq,
                owner_sig_hash,
            },
            signer,
            signed_at_ms,
            signature,
        });
    }

    let cc_len = r.u32()? as usize;
    let mut capability_catalog = BTreeMap::new();
    for _ in 0..cc_len {
        let cap_id = CapabilityId::new(r.string()?);
        let is_static_core = r.u8()? != 0;
        let owner_signed = r.u8()? != 0;
        let active = r.u8()? != 0;
        let audience = decode_audience(r)?;
        let single_map = decode_capability_descriptors(r)?;
        let descriptor = single_map.into_values().next().ok_or_else(|| {
            EventFormatError::MalformedRecord("missing catalog descriptor".to_owned())
        })?;
        let embedding = decode_tagged_embeddings(r)?.into_iter().next();
        capability_catalog.insert(
            cap_id,
            CapabilityCatalogEntry {
                descriptor,
                is_static_core,
                owner_signed,
                active,
                audience,
                embedding,
            },
        );
    }

    let proc_len = r.u32()? as usize;
    let mut procedures = BTreeMap::new();
    for _ in 0..proc_len {
        let procedure_id = EventId::new(r.u64()?);
        let name = r.string()?;
        let st_len = r.u32()? as usize;
        let mut steps = Vec::with_capacity(st_len.min(256));
        for _ in 0..st_len {
            steps.push(r.string()?);
        }
        let rc_len = r.u32()? as usize;
        let mut required_cap_ids = Vec::with_capacity(rc_len.min(256));
        for _ in 0..rc_len {
            required_cap_ids.push(CapabilityId::new(r.string()?));
        }
        let capability_schema_hash = CapabilitySchemaHash::new(r.string()?);
        let audience = decode_audience(r)?;
        let domain = r.opt_string()?.map(DomainId::new);
        let active = r.u8()? != 0;
        let retired = r.u8()? != 0;
        let owner_signed = r.u8()? != 0;
        let embedding = decode_tagged_embeddings(r)?.into_iter().next();
        procedures.insert(
            procedure_id,
            ProcedureEntry {
                procedure_id,
                name,
                steps,
                required_cap_ids,
                capability_schema_hash,
                audience,
                domain,
                active,
                retired,
                owner_signed,
                embedding,
            },
        );
    }

    let knowledge_cutoff = r.opt_string()?;
    let d_len = r.u32()? as usize;
    let mut domain_calibration_offsets = BTreeMap::new();
    for _ in 0..d_len {
        let domain = DomainId::new(r.string()?);
        let offset = r.f64()?;
        domain_calibration_offsets.insert(domain, offset);
    }

    let ar_len = r.u32()? as usize;
    let mut auto_recall_states = BTreeMap::new();
    for _ in 0..ar_len {
        let ev_id = EventId::new(r.u64()?);
        let active = r.u8()? != 0;
        let attempt = r.u8()?;
        let max_attempts = r.u8()?;
        let coarser_grain_allowed = r.u8()? != 0;
        let last_reason = r.opt_string()?;
        auto_recall_states.insert(
            ev_id,
            AutoRecallTriggerState {
                active,
                attempt,
                max_attempts,
                coarser_grain_allowed,
                last_reason,
            },
        );
    }

    let cue_len = r.u32()? as usize;
    let mut recent_receipt_cues = Vec::with_capacity(cue_len.min(128));
    for _ in 0..cue_len {
        recent_receipt_cues.push(r.string()?);
    }
    let wake_count_since_anchor_check = r.u64()?;
    let anchor_check_pending_post_sleep = r.u8()? != 0;
    let last_sleep_seq = LogSeq::new(r.u64()?);
    let reliability_alpha = r.f64()?;
    let reliability_beta = r.f64()?;
    let lat_len = r.u32()? as usize;
    let mut latency_samples_ms = Vec::with_capacity(lat_len.min(256));
    for _ in 0..lat_len {
        latency_samples_ms.push(r.u64()?);
    }
    let ta_len = r.u32()? as usize;
    let mut turn_allocated_by_trigger = BTreeMap::new();
    for _ in 0..ta_len {
        let ev_id = EventId::new(r.u64()?);
        let alloc = r.u8()? != 0;
        turn_allocated_by_trigger.insert(ev_id, alloc);
    }
    let calibration_samples = r.u64()?;
    let calibration_brier_sum = r.f64()?;
    let principle_alpha = r.f64()?;
    let principle_beta = r.f64()?;
    let contradiction_flags = r.u64()?;

    Ok(DecodedC5State {
        identity_anchor,
        principles,
        capability_catalog,
        procedures,
        knowledge_cutoff,
        domain_calibration_offsets,
        auto_recall_states,
        recent_receipt_cues,
        wake_count_since_anchor_check,
        anchor_check_pending_post_sleep,
        last_sleep_seq,
        reliability_alpha,
        reliability_beta,
        latency_samples_ms,
        turn_allocated_by_trigger,
        calibration_samples,
        calibration_brier_sum,
        principle_alpha,
        principle_beta,
        contradiction_flags,
    })
}

#[derive(Default)]
struct DecodedC6State {
    belief_book: BeliefBook,
    domain_clusters: DomainClusterBook,
    episode_book: EpisodeBook,
    pending_sleep_run: Option<PendingSleepRun>,
    last_global_activity_ms: Option<Time>,
}

fn encode_beta_credence(w: &mut BinWriter, c: &crate::beliefs::BetaCredence) {
    w.f64(c.alpha_0);
    w.f64(c.beta_0);
    w.f64(c.alpha);
    w.f64(c.beta);
    w.u64(c.last_updated_ms.as_millis());
}

fn decode_beta_credence(
    r: &mut BinReader<'_>,
) -> Result<crate::beliefs::BetaCredence, EventFormatError> {
    Ok(crate::beliefs::BetaCredence {
        alpha_0: r.f64()?,
        beta_0: r.f64()?,
        alpha: r.f64()?,
        beta: r.f64()?,
        last_updated_ms: Time::from_millis(r.u64()?),
    })
}

fn encode_c6_state(w: &mut BinWriter, state: &CoreState) {
    w.opt_u64(state.last_global_activity_ms.map(Time::as_millis));

    // BeliefBook
    w.u32(state.belief_book.actor_priors.len() as u32);
    for (actor, &(a0, b0)) in &state.belief_book.actor_priors {
        w.str(actor.as_str());
        w.f64(a0);
        w.f64(b0);
    }
    w.u32(state.belief_book.beliefs.len() as u32);
    for (&bid, b) in &state.belief_book.beliefs {
        w.u64(bid.as_u64());
        w.str(&b.claim_key);
        w.u8(b.requested_basis.code());
        w.u8(b.basis.code());
        w.u8(u8::from(b.basis_downgraded));
        w.opt_str(b.source_actor.as_ref().map(ActorId::as_str));
        w.u32(b.corroborating_actors.len() as u32);
        for ca in &b.corroborating_actors {
            w.str(ca.as_str());
        }
        w.u8(encode_taint(b.taint));
        w.str(b.domain.as_str());
        w.u32(b.source_event_ids.len() as u32);
        for &sid in &b.source_event_ids {
            w.u64(sid.as_u64());
        }
        w.u64(b.valid_from_ms.as_millis());
        w.opt_u64(b.valid_to_ms.map(Time::as_millis));
        w.opt_u64(b.replaced_by.map(EventId::as_u64));
        match &b.credence {
            Some(c) => {
                w.u8(1);
                encode_beta_credence(w, c);
            }
            None => w.u8(0),
        }
        w.u8(u8::from(b.is_mind_inference));
        w.u8(u8::from(b.erased));
    }
    w.u32(state.belief_book.active_claims.len() as u32);
    for (k, &eid) in &state.belief_book.active_claims {
        w.str(k);
        w.u64(eid.as_u64());
    }
    w.u32(state.belief_book.source_reliability.len() as u32);
    for ((actor, dom), c) in &state.belief_book.source_reliability {
        w.str(actor.as_str());
        w.str(dom.as_str());
        encode_beta_credence(w, c);
    }
    w.u32(state.belief_book.item_credences.len() as u32);
    for (&eid, c) in &state.belief_book.item_credences {
        w.u64(eid.as_u64());
        encode_beta_credence(w, c);
    }
    w.u32(state.belief_book.conventions.len() as u32);
    for ((pl, pat), c) in &state.belief_book.conventions {
        w.str(pl.as_str());
        w.str(pat);
        encode_beta_credence(w, c);
    }
    w.u32(state.belief_book.log_entries.len() as u32);
    for le in &state.belief_book.log_entries {
        match le {
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
                source_event_ids,
                taint,
                valid_from_ms,
                valid_to_ms,
                replaced_by,
                alpha_0,
                beta_0,
                is_mind_inference,
            } => {
                w.u8(1);
                w.u64(event_id.as_u64());
                w.u64(timestamp_ms.as_millis());
                w.u64(seq.as_u64());
                w.str(claim_key);
                w.u8(requested_basis.code());
                w.u8(basis.code());
                w.u8(u8::from(*basis_downgraded));
                w.opt_str(source_actor.as_ref().map(ActorId::as_str));
                w.str(domain.as_str());
                w.u32(source_event_ids.len() as u32);
                for &sid in source_event_ids {
                    w.u64(sid.as_u64());
                }
                w.u8(encode_taint(*taint));
                w.u64(valid_from_ms.as_millis());
                w.opt_u64(valid_to_ms.map(Time::as_millis));
                w.opt_u64(replaced_by.map(EventId::as_u64));
                w.f64(*alpha_0);
                w.f64(*beta_0);
                w.u8(u8::from(*is_mind_inference));
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
                w.u8(2);
                w.u64(event_id.as_u64());
                w.u64(timestamp_ms.as_millis());
                w.u64(seq.as_u64());
                w.u64(target_id.as_u64());
                w.opt_str(source_actor.as_ref().map(ActorId::as_str));
                w.str(domain.as_str());
                w.f64(*positive);
                w.f64(*negative);
                w.u8(u8::from(*is_convention));
                w.opt_str(place.as_ref().map(PlaceId::as_str));
                w.opt_str(pattern_id.as_deref());
            }
            BeliefLogEntry::DomainsRebased {
                event_id,
                timestamp_ms,
                seq,
                transitions,
                credence_rebased,
            } => {
                w.u8(3);
                w.u64(event_id.as_u64());
                w.u64(timestamp_ms.as_millis());
                w.u64(seq.as_u64());
                w.u32(transitions.len() as u32);
                for tr in transitions {
                    w.u8(tr.kind.code());
                    w.opt_str(tr.old_domain.as_ref().map(DomainId::as_str));
                    w.opt_str(tr.new_domain.as_ref().map(DomainId::as_str));
                    w.f64(tr.similarity);
                }
                w.u32(credence_rebased.len() as u32);
                for rb in credence_rebased {
                    w.str(rb.actor.as_str());
                    w.str(rb.from_domain.as_str());
                    w.str(rb.to_domain.as_str());
                    w.f64(rb.new_alpha);
                    w.f64(rb.new_beta);
                    w.f64(rb.alpha_0);
                    w.f64(rb.beta_0);
                }
            }
        }
    }

    // DomainClusterBook
    w.u64(state.domain_clusters.next_domain_seq);
    w.u32(state.domain_clusters.clusters.len() as u32);
    for (did, cl) in &state.domain_clusters.clusters {
        w.str(did.as_str());
        encode_tagged_embeddings(w, std::slice::from_ref(&cl.centroid));
        w.u8(u8::from(cl.active));
        w.u64(cl.member_count as u64);
    }

    // EpisodeBook
    w.u32(state.episode_book.episodes.len() as u32);
    for (&ep_id, ep) in &state.episode_book.episodes {
        w.u64(ep_id.as_u64());
        w.str(ep.place.as_str());
        w.str(ep.state.as_str());
        w.u32(ep.event_ids.len() as u32);
        for &eid in &ep.event_ids {
            w.u64(eid.as_u64());
        }
        w.u32(ep.participants.len() as u32);
        for p in &ep.participants {
            w.str(p.as_str());
        }
        w.u64(ep.start_ms.as_millis());
        w.u64(ep.end_ms.as_millis());
        let cents: Vec<TaggedEmbedding> = ep.centroid.iter().cloned().collect();
        encode_tagged_embeddings(w, &cents);
        w.u64(ep.vector_count as u64);
        w.u8(encode_taint(ep.taint));
        w.opt_u64(ep.gist_id.map(EventId::as_u64));
        w.u8(u8::from(ep.dirty));
        w.u8(u8::from(ep.consolidated));
    }
    w.u32(state.episode_book.active_by_place.len() as u32);
    for (pl, &eid) in &state.episode_book.active_by_place {
        w.str(pl.as_str());
        w.u64(eid.as_u64());
    }
    w.u32(state.episode_book.boundary_states.len() as u32);
    for (pl, bs) in &state.episode_book.boundary_states {
        w.str(pl.as_str());
        w.u32(bs.surprise_history.len() as u32);
        for &s in &bs.surprise_history {
            w.f64(s);
        }
        w.u32(bs.log_gap_history.len() as u32);
        for &g in &bs.log_gap_history {
            w.f64(g);
        }
        w.opt_u64(bs.last_event_ms.map(Time::as_millis));
        w.u32(bs.last_participants.len() as u32);
        for a in &bs.last_participants {
            w.str(a.as_str());
        }
    }
    w.u32(state.episode_book.unconsolidated_events.len() as u32);
    for &eid in &state.episode_book.unconsolidated_events {
        w.u64(eid.as_u64());
    }
    w.u32(state.episode_book.consolidated_events.len() as u32);
    for &eid in &state.episode_book.consolidated_events {
        w.u64(eid.as_u64());
    }

    // PendingSleepRun
    match &state.pending_sleep_run {
        None => w.u8(0),
        Some(run) => {
            w.u8(1);
            w.u64(run.run_id.as_u64());
            w.u64(run.snapshot_ms.as_millis());
            w.u64(run.snapshot_seq.as_u64());
            w.u32(run.covered_event_ids.len() as u32);
            for &id in &run.covered_event_ids {
                w.u64(id.as_u64());
            }
            w.u32(run.pending_episode_ids.len() as u32);
            for &id in &run.pending_episode_ids {
                w.u64(id.as_u64());
            }
            w.u32(run.completed_episode_ids.len() as u32);
            for &id in &run.completed_episode_ids {
                w.u64(id.as_u64());
            }
            w.u32(run.produced_gist_ids.len() as u32);
            for &id in &run.produced_gist_ids {
                w.u64(id.as_u64());
            }
            w.u32(run.episode_taints.len() as u32);
            for (&id, &t) in &run.episode_taints {
                w.u64(id.as_u64());
                w.u8(encode_taint(t));
            }
            w.u64(run.tokens_used);
        }
    }
}

fn decode_c6_state(r: &mut BinReader<'_>) -> Result<DecodedC6State, EventFormatError> {
    let last_global_activity_ms = r.opt_u64()?.map(Time::from_millis);

    let ap_len = r.u32()? as usize;
    let mut actor_priors = BTreeMap::new();
    for _ in 0..ap_len {
        let actor = ActorId::new(r.string()?);
        let a0 = r.f64()?;
        let b0 = r.f64()?;
        actor_priors.insert(actor, (a0, b0));
    }

    let b_len = r.u32()? as usize;
    let mut beliefs = BTreeMap::new();
    for _ in 0..b_len {
        let belief_id = EventId::new(r.u64()?);
        let claim_key = r.string()?;
        let req_c = r.u8()?;
        let requested_basis = MemoryBasis::from_code(req_c).ok_or_else(|| {
            EventFormatError::MalformedRecord(format!("invalid c6 req basis {req_c}"))
        })?;
        let bas_c = r.u8()?;
        let basis = MemoryBasis::from_code(bas_c).ok_or_else(|| {
            EventFormatError::MalformedRecord(format!("invalid c6 basis {bas_c}"))
        })?;
        let basis_downgraded = r.u8()? != 0;
        let source_actor = r.opt_string()?.map(ActorId::new);
        let ca_len = r.u32()? as usize;
        let mut corroborating_actors = BTreeSet::new();
        for _ in 0..ca_len {
            corroborating_actors.insert(ActorId::new(r.string()?));
        }
        let taint = decode_taint(r.u8()?)?;
        let domain = DomainId::new(r.string()?);
        let se_len = r.u32()? as usize;
        let mut source_event_ids = Vec::with_capacity(se_len.min(1024));
        for _ in 0..se_len {
            source_event_ids.push(EventId::new(r.u64()?));
        }
        let valid_from_ms = Time::from_millis(r.u64()?);
        let valid_to_ms = r.opt_u64()?.map(Time::from_millis);
        let replaced_by = r.opt_u64()?.map(EventId::new);
        let credence = match r.u8()? {
            0 => None,
            1 => Some(decode_beta_credence(r)?),
            tag => {
                return Err(EventFormatError::MalformedRecord(format!(
                    "invalid c6 credence tag {tag}"
                )));
            }
        };
        let is_mind_inference = r.u8()? != 0;
        let erased = r.u8()? != 0;
        beliefs.insert(
            belief_id,
            crate::beliefs::BeliefRecord {
                belief_id,
                claim_key,
                requested_basis,
                basis,
                basis_downgraded,
                source_actor,
                corroborating_actors,
                taint,
                domain,
                source_event_ids,
                valid_from_ms,
                valid_to_ms,
                replaced_by,
                credence,
                is_mind_inference,
                erased,
            },
        );
    }

    let ac_len = r.u32()? as usize;
    let mut active_claims = BTreeMap::new();
    for _ in 0..ac_len {
        let k = r.string()?;
        let eid = EventId::new(r.u64()?);
        active_claims.insert(k, eid);
    }

    let sr_len = r.u32()? as usize;
    let mut source_reliability = BTreeMap::new();
    for _ in 0..sr_len {
        let actor = ActorId::new(r.string()?);
        let dom = DomainId::new(r.string()?);
        let c = decode_beta_credence(r)?;
        source_reliability.insert((actor, dom), c);
    }

    let ic_len = r.u32()? as usize;
    let mut item_credences = BTreeMap::new();
    for _ in 0..ic_len {
        let eid = EventId::new(r.u64()?);
        let c = decode_beta_credence(r)?;
        item_credences.insert(eid, c);
    }

    let cv_len = r.u32()? as usize;
    let mut conventions = BTreeMap::new();
    for _ in 0..cv_len {
        let pl = PlaceId::new(r.string()?);
        let pat = r.string()?;
        let c = decode_beta_credence(r)?;
        conventions.insert((pl, pat), c);
    }

    let le_len = r.u32()? as usize;
    let mut log_entries = Vec::with_capacity(le_len.min(4096));
    for _ in 0..le_len {
        match r.u8()? {
            1 => {
                let event_id = EventId::new(r.u64()?);
                let timestamp_ms = Time::from_millis(r.u64()?);
                let seq = LogSeq::new(r.u64()?);
                let claim_key = r.string()?;
                let req_c = r.u8()?;
                let requested_basis = MemoryBasis::from_code(req_c).ok_or_else(|| {
                    EventFormatError::MalformedRecord(format!("invalid log req basis {req_c}"))
                })?;
                let bas_c = r.u8()?;
                let basis = MemoryBasis::from_code(bas_c).ok_or_else(|| {
                    EventFormatError::MalformedRecord(format!("invalid log basis {bas_c}"))
                })?;
                let basis_downgraded = r.u8()? != 0;
                let source_actor = r.opt_string()?.map(ActorId::new);
                let domain = DomainId::new(r.string()?);
                let s_len = r.u32()? as usize;
                let mut source_event_ids = Vec::with_capacity(s_len.min(1024));
                for _ in 0..s_len {
                    source_event_ids.push(EventId::new(r.u64()?));
                }
                let taint = decode_taint(r.u8()?)?;
                let valid_from_ms = Time::from_millis(r.u64()?);
                let valid_to_ms = r.opt_u64()?.map(Time::from_millis);
                let replaced_by = r.opt_u64()?.map(EventId::new);
                let alpha_0 = r.f64()?;
                let beta_0 = r.f64()?;
                let is_mind_inference = r.u8()? != 0;
                log_entries.push(BeliefLogEntry::BeliefRecorded {
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
                    valid_to_ms,
                    replaced_by,
                    alpha_0,
                    beta_0,
                    is_mind_inference,
                });
            }
            2 => {
                let event_id = EventId::new(r.u64()?);
                let timestamp_ms = Time::from_millis(r.u64()?);
                let seq = LogSeq::new(r.u64()?);
                let target_id = EventId::new(r.u64()?);
                let source_actor = r.opt_string()?.map(ActorId::new);
                let domain = DomainId::new(r.string()?);
                let positive = r.f64()?;
                let negative = r.f64()?;
                let is_convention = r.u8()? != 0;
                let place = r.opt_string()?.map(PlaceId::new);
                let pattern_id = r.opt_string()?;
                log_entries.push(BeliefLogEntry::EvidenceApplied {
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
            3 => {
                let event_id = EventId::new(r.u64()?);
                let timestamp_ms = Time::from_millis(r.u64()?);
                let seq = LogSeq::new(r.u64()?);
                let tr_len = r.u32()? as usize;
                let mut transitions = Vec::with_capacity(tr_len.min(256));
                for _ in 0..tr_len {
                    let k = r.u8()?;
                    let kind = DomainTransitionKind::from_code(k).ok_or_else(|| {
                        EventFormatError::MalformedRecord(format!("invalid transition {k}"))
                    })?;
                    let old_domain = r.opt_string()?.map(DomainId::new);
                    let new_domain = r.opt_string()?.map(DomainId::new);
                    let similarity = r.f64()?;
                    transitions.push(DomainTransitionRecord {
                        kind,
                        old_domain,
                        new_domain,
                        similarity,
                    });
                }
                let rb_len = r.u32()? as usize;
                let mut credence_rebased = Vec::with_capacity(rb_len.min(1024));
                for _ in 0..rb_len {
                    credence_rebased.push(CredenceRebaseRecord {
                        actor: ActorId::new(r.string()?),
                        from_domain: DomainId::new(r.string()?),
                        to_domain: DomainId::new(r.string()?),
                        new_alpha: r.f64()?,
                        new_beta: r.f64()?,
                        alpha_0: r.f64()?,
                        beta_0: r.f64()?,
                    });
                }
                log_entries.push(BeliefLogEntry::DomainsRebased {
                    event_id,
                    timestamp_ms,
                    seq,
                    transitions,
                    credence_rebased,
                });
            }
            tag => {
                return Err(EventFormatError::MalformedRecord(format!(
                    "invalid belief log entry tag {tag}"
                )));
            }
        }
    }

    let next_domain_seq = r.u64()?;
    let dc_len = r.u32()? as usize;
    let mut clusters = BTreeMap::new();
    for _ in 0..dc_len {
        let did = DomainId::new(r.string()?);
        let centroid = decode_tagged_embeddings(r)?
            .into_iter()
            .next()
            .ok_or_else(|| EventFormatError::MalformedRecord("missing centroid".to_owned()))?;
        let active = r.u8()? != 0;
        let member_count = r.u64()? as usize;
        clusters.insert(
            did.clone(),
            DomainCentroid {
                domain_id: did,
                centroid,
                active,
                member_count,
            },
        );
    }

    let ep_len = r.u32()? as usize;
    let mut episodes = BTreeMap::new();
    for _ in 0..ep_len {
        let episode_id = EventId::new(r.u64()?);
        let place = PlaceId::new(r.string()?);
        let state_str = r.string()?;
        let state = EpisodeState::parse(&state_str).ok_or_else(|| {
            EventFormatError::MalformedRecord(format!("invalid episode state {state_str}"))
        })?;
        let ev_len = r.u32()? as usize;
        let mut event_ids = Vec::with_capacity(ev_len.min(2048));
        for _ in 0..ev_len {
            event_ids.push(EventId::new(r.u64()?));
        }
        let p_len = r.u32()? as usize;
        let mut participants = BTreeSet::new();
        for _ in 0..p_len {
            participants.insert(ActorId::new(r.string()?));
        }
        let start_ms = Time::from_millis(r.u64()?);
        let end_ms = Time::from_millis(r.u64()?);
        let centroid = decode_tagged_embeddings(r)?.into_iter().next();
        let vector_count = r.u64()? as usize;
        let taint = decode_taint(r.u8()?)?;
        let gist_id = r.opt_u64()?.map(EventId::new);
        let dirty = r.u8()? != 0;
        let consolidated = r.u8()? != 0;
        episodes.insert(
            episode_id,
            EpisodeRecord {
                episode_id,
                place,
                state,
                event_ids,
                participants,
                start_ms,
                end_ms,
                centroid,
                vector_count,
                taint,
                gist_id,
                dirty,
                consolidated,
            },
        );
    }

    let abp_len = r.u32()? as usize;
    let mut active_by_place = BTreeMap::new();
    for _ in 0..abp_len {
        let pl = PlaceId::new(r.string()?);
        let eid = EventId::new(r.u64()?);
        active_by_place.insert(pl, eid);
    }

    let bs_len = r.u32()? as usize;
    let mut boundary_states = BTreeMap::new();
    for _ in 0..bs_len {
        let pl = PlaceId::new(r.string()?);
        let sh_len = r.u32()? as usize;
        let mut surprise_history = Vec::with_capacity(sh_len.min(512));
        for _ in 0..sh_len {
            surprise_history.push(r.f64()?);
        }
        let lg_len = r.u32()? as usize;
        let mut log_gap_history = Vec::with_capacity(lg_len.min(512));
        for _ in 0..lg_len {
            log_gap_history.push(r.f64()?);
        }
        let last_event_ms = r.opt_u64()?.map(Time::from_millis);
        let lp_len = r.u32()? as usize;
        let mut last_participants = BTreeSet::new();
        for _ in 0..lp_len {
            last_participants.insert(ActorId::new(r.string()?));
        }
        boundary_states.insert(
            pl,
            PlaceBoundaryState {
                surprise_history,
                log_gap_history,
                last_event_ms,
                last_participants,
            },
        );
    }

    let unc_len = r.u32()? as usize;
    let mut unconsolidated_events = BTreeSet::new();
    for _ in 0..unc_len {
        unconsolidated_events.insert(EventId::new(r.u64()?));
    }
    let con_len = r.u32()? as usize;
    let mut consolidated_events = BTreeSet::new();
    for _ in 0..con_len {
        consolidated_events.insert(EventId::new(r.u64()?));
    }

    let pending_sleep_run = match r.u8()? {
        0 => None,
        1 => {
            let run_id = EventId::new(r.u64()?);
            let snapshot_ms = Time::from_millis(r.u64()?);
            let snapshot_seq = LogSeq::new(r.u64()?);
            let c_len = r.u32()? as usize;
            let mut covered_event_ids = Vec::with_capacity(c_len.min(4096));
            for _ in 0..c_len {
                covered_event_ids.push(EventId::new(r.u64()?));
            }
            let p_len = r.u32()? as usize;
            let mut pending_episode_ids = Vec::with_capacity(p_len.min(1024));
            for _ in 0..p_len {
                pending_episode_ids.push(EventId::new(r.u64()?));
            }
            let comp_len = r.u32()? as usize;
            let mut completed_episode_ids = Vec::with_capacity(comp_len.min(1024));
            for _ in 0..comp_len {
                completed_episode_ids.push(EventId::new(r.u64()?));
            }
            let g_len = r.u32()? as usize;
            let mut produced_gist_ids = Vec::with_capacity(g_len.min(1024));
            for _ in 0..g_len {
                produced_gist_ids.push(EventId::new(r.u64()?));
            }
            let et_len = r.u32()? as usize;
            let mut episode_taints = BTreeMap::new();
            for _ in 0..et_len {
                let id = EventId::new(r.u64()?);
                let t = decode_taint(r.u8()?)?;
                episode_taints.insert(id, t);
            }
            let tokens_used = r.u64()?;
            Some(PendingSleepRun {
                run_id,
                snapshot_ms,
                snapshot_seq,
                covered_event_ids,
                pending_episode_ids,
                completed_episode_ids,
                produced_gist_ids,
                episode_taints,
                tokens_used,
            })
        }
        tag => {
            return Err(EventFormatError::MalformedRecord(format!(
                "invalid pending_sleep_run tag {tag}"
            )));
        }
    };

    Ok(DecodedC6State {
        belief_book: BeliefBook {
            beliefs,
            active_claims,
            source_reliability,
            item_credences,
            conventions,
            actor_priors,
            log_entries,
        },
        domain_clusters: DomainClusterBook {
            clusters,
            next_domain_seq,
        },
        episode_book: EpisodeBook {
            episodes,
            active_by_place,
            boundary_states,
            unconsolidated_events,
            consolidated_events,
        },
        pending_sleep_run,
        last_global_activity_ms,
    })
}

#[derive(Default)]
struct DecodedC7State {
    skill_book: SkillBook,
    ledger: PerceptionLedger,
    learner: BoundedLearner,
    procedure_links: BTreeMap<ActionId, EventId>,
    recalibration_pending: bool,
}

fn encode_c7_state(w: &mut BinWriter, state: &CoreState) {
    w.u8(u8::from(state.recalibration_pending));
    w.u32(state.procedure_links.len() as u32);
    for (aid, &pid) in &state.procedure_links {
        w.str(aid.as_str());
        w.u64(pid.as_u64());
    }

    // SkillBook
    w.u64(state.skill_book.next_synthetic_id);
    w.u32(state.skill_book.failure_episodes.len() as u32);
    for f in &state.skill_book.failure_episodes {
        w.u64(f.episode_id.as_u64());
        w.str(&f.signature_key);
        w.opt_str(f.domain.as_ref().map(DomainId::as_str));
        w.opt_str(f.capability_id.as_ref().map(CapabilityId::as_str));
        w.str(f.place.as_str());
        w.u32(f.actors.len() as u32);
        for a in &f.actors {
            w.str(a.as_str());
        }
        w.u8(encode_taint(f.taint));
        w.u64(f.timestamp_ms.as_millis());
    }
    w.u32(state.skill_book.success_episodes.len() as u32);
    for s in &state.skill_book.success_episodes {
        w.u64(s.episode_id.as_u64());
        w.str(&s.signature_key);
        w.opt_str(s.domain.as_ref().map(DomainId::as_str));
        w.opt_str(s.capability_id.as_ref().map(CapabilityId::as_str));
        w.str(s.place.as_str());
        w.u32(s.actors.len() as u32);
        for a in &s.actors {
            w.str(a.as_str());
        }
        w.u8(encode_taint(s.taint));
        w.u64(s.timestamp_ms.as_millis());
    }
    w.u32(state.skill_book.lessons.len() as u32);
    for (&lid, l) in &state.skill_book.lessons {
        w.u64(lid.as_u64());
        w.str(&l.pattern);
        w.str(&l.failure_signature);
        w.opt_str(l.domain.as_ref().map(DomainId::as_str));
        w.u32(l.failure_ids.len() as u32);
        for fid in &l.failure_ids {
            w.u64(fid.as_u64());
        }
        w.u32(l.contrastive_success_ids.len() as u32);
        for cid in &l.contrastive_success_ids {
            w.u64(cid.as_u64());
        }
        w.u32(l.actors.len() as u32);
        for a in &l.actors {
            w.str(a.as_str());
        }
        w.f64(l.credence.alpha);
        w.f64(l.credence.beta);
        w.u64(l.credence.last_updated_ms.as_millis());
        w.u8(encode_taint(l.taint));
        w.u64(l.created_at_ms.as_millis());
        w.u64(l.updated_at_ms.as_millis());
        w.u8(u8::from(l.active));
    }
    w.u32(state.skill_book.procedures.len() as u32);
    for (&pid, p) in &state.skill_book.procedures {
        w.u64(pid.as_u64());
        w.str(&p.name);
        w.u32(p.steps.len() as u32);
        for st in &p.steps {
            w.str(st);
        }
        w.u32(p.required_cap_ids.len() as u32);
        for c in &p.required_cap_ids {
            w.str(c.as_str());
        }
        w.str(p.capability_schema_hash.as_str());
        w.opt_str(p.domain.as_ref().map(DomainId::as_str));
        w.u32(p.places.len() as u32);
        for pl in &p.places {
            w.str(pl.as_str());
        }
        w.u32(p.actors.len() as u32);
        for a in &p.actors {
            w.str(a.as_str());
        }
        encode_audience(w, &p.audience);
        w.u8(u8::from(p.owner_signed));
        match &p.embedding {
            Some(emb) => {
                w.u8(1);
                encode_tagged_embedding(w, emb);
            }
            None => w.u8(0),
        }
        w.opt_u64(p.source_goal_id.map(EventId::as_u64));
        w.u32(p.source_event_ids.len() as u32);
        for sid in &p.source_event_ids {
            w.u64(sid.as_u64());
        }
        w.u8(encode_taint(p.taint));
        w.u8(p.state.code());
        w.opt_str(p.retired_reason.as_deref());
        w.opt_u64(p.retired_at_ms.map(Time::as_millis));
        w.u64(p.created_at_ms.as_millis());
        w.u64(p.updated_at_ms.as_millis());
        w.f64(p.alpha);
        w.f64(p.beta);
        w.u64(p.trial_count);
        w.u64(p.success_count);
        match p.cusum_baseline_p0 {
            Some(b) => {
                w.u8(1);
                w.f64(b);
            }
            None => w.u8(0),
        }
        w.f64(p.cusum_stat);
        w.u32(p.warn_count);
    }

    // PerceptionLedger
    w.u32(state.ledger.actors.len() as u32);
    for (aid, card) in &state.ledger.actors {
        w.str(aid.as_str());
        w.f64(card.reliability_alpha);
        w.f64(card.reliability_beta);
        w.u8(u8::from(card.trusted_origin));
        w.u32(card.witnessed.len() as u32);
        for ws in &card.witnessed {
            w.u64(ws.event_id.as_u64());
            w.str(ws.place.as_str());
            w.u64(ws.timestamp_ms.as_millis());
        }
        w.u32(card.open_requests.len() as u32);
        for or in &card.open_requests {
            w.u64(or.expectation_id.as_u64());
            w.u8(crate::event::encode_expect_kind(or.kind));
            w.u64(or.due_ms.as_millis());
        }
        w.u32(card.told_items.len() as u32);
        for ti in &card.told_items {
            w.u64(ti.belief_id.as_u64());
            w.str(&ti.predicate);
            w.u64(ti.timestamp_ms.as_millis());
            w.u8(u8::from(ti.corroborated));
            w.u8(u8::from(ti.refuted));
        }
        w.u32(card.false_beliefs.len() as u32);
        for fb in &card.false_beliefs {
            w.u64(fb.belief_id.as_u64());
            w.str(&fb.predicate);
            w.u64(fb.flagged_at_ms.as_millis());
        }
    }
    w.u32(state.ledger.capabilities.len() as u32);
    for (cid, cap) in &state.ledger.capabilities {
        w.str(cid.as_str());
        w.u64(cap.trials);
        w.u64(cap.successes);
        w.u64(cap.failures);
        w.u32(cap.consec_fail);
        w.f64(cap.alpha);
        w.f64(cap.beta);
        w.u8(u8::from(cap.degraded));
        w.u64(cap.latency_sum_ms.min(u128::from(u64::MAX)) as u64);
        w.u64(cap.latency_samples);
        w.u64(cap.last_updated_ms.as_millis());
    }
    for v in state.ledger.calibration.bin_incorrect {
        w.u64(v);
    }
    for v in state.ledger.calibration.bin_correct {
        w.u64(v);
    }
    w.f64(state.ledger.calibration.brier_sum);
    w.f64(state.ledger.calibration.signed_error_sum);
    w.u64(state.ledger.calibration.samples);
    w.f64(state.ledger.principle_alpha);
    w.f64(state.ledger.principle_beta);
    w.u64(state.ledger.contradiction_flags);
    w.f64(state.ledger.daily_budget_used);
    w.f64(state.ledger.daily_budget_limit);
    w.u64(state.ledger.recalibration_count);
    w.opt_u64(state.ledger.last_recalibration_ms.map(Time::as_millis));
    w.u64(state.ledger.last_capability_schema_epoch);

    // BoundedLearner
    w.u64(state.learner.day_index);
    for v in state.learner.daily_abs_delta {
        w.f64(v);
    }
    w.u64(state.learner.next_epoch_id);
    w.u64(state.learner.update_count);
    w.u64(state.learner.rollback_count);
    w.u32(state.learner.processed_event_ids.len() as u32);
    for pid in &state.learner.processed_event_ids {
        w.u64(pid.as_u64());
    }
    w.u32(state.learner.checkpoints.len() as u32);
    for cp in &state.learner.checkpoints {
        w.u64(cp.epoch_id);
        w.u64(cp.source_event_id.as_u64());
        w.u64(cp.timestamp_ms.as_millis());
        encode_adaptive_weights(w, &cp.weights_before);
        for v in cp.daily_abs_delta_before {
            w.f64(v);
        }
        w.u64(cp.day_index_before);
    }
}

fn decode_c7_state(r: &mut BinReader<'_>) -> Result<DecodedC7State, EventFormatError> {
    use crate::beliefs::BetaCredence;
    use crate::consolidation::{FailureEpisodeRecord, LessonEntry, SuccessEpisodeRecord};
    use crate::learn::LearningEpochCheckpoint;
    use crate::ledger::{
        ActorCard, CapabilityHealthStats, FalseBeliefFlag, OpenRequestItem, SelfCalibrationStats,
        ToldLedgerItem, WitnessedFactSummary,
    };

    let recalibration_pending = r.u8()? != 0;
    let pl_len = r.u32()? as usize;
    let mut procedure_links = BTreeMap::new();
    for _ in 0..pl_len {
        let aid = ActionId::new(r.string()?);
        let pid = EventId::new(r.u64()?);
        procedure_links.insert(aid, pid);
    }

    let next_synthetic_id = r.u64()?;
    let fe_len = r.u32()? as usize;
    let mut failure_episodes = Vec::with_capacity(fe_len.min(1024));
    for _ in 0..fe_len {
        let episode_id = EventId::new(r.u64()?);
        let signature_key = r.string()?;
        let domain = r.opt_string()?.map(DomainId::new);
        let capability_id = r.opt_string()?.map(CapabilityId::new);
        let place = PlaceId::new(r.string()?);
        let a_len = r.u32()? as usize;
        let mut actors = Vec::with_capacity(a_len.min(64));
        for _ in 0..a_len {
            actors.push(ActorId::new(r.string()?));
        }
        let taint = decode_taint(r.u8()?)?;
        let timestamp_ms = Time::from_millis(r.u64()?);
        failure_episodes.push(FailureEpisodeRecord {
            episode_id,
            signature_key,
            domain,
            capability_id,
            place,
            actors,
            taint,
            timestamp_ms,
        });
    }

    let se_len = r.u32()? as usize;
    let mut success_episodes = Vec::with_capacity(se_len.min(1024));
    for _ in 0..se_len {
        let episode_id = EventId::new(r.u64()?);
        let signature_key = r.string()?;
        let domain = r.opt_string()?.map(DomainId::new);
        let capability_id = r.opt_string()?.map(CapabilityId::new);
        let place = PlaceId::new(r.string()?);
        let a_len = r.u32()? as usize;
        let mut actors = Vec::with_capacity(a_len.min(64));
        for _ in 0..a_len {
            actors.push(ActorId::new(r.string()?));
        }
        let taint = decode_taint(r.u8()?)?;
        let timestamp_ms = Time::from_millis(r.u64()?);
        success_episodes.push(SuccessEpisodeRecord {
            episode_id,
            signature_key,
            domain,
            capability_id,
            place,
            actors,
            taint,
            timestamp_ms,
        });
    }

    let l_len = r.u32()? as usize;
    let mut lessons = BTreeMap::new();
    for _ in 0..l_len {
        let lesson_id = EventId::new(r.u64()?);
        let pattern = r.string()?;
        let failure_signature = r.string()?;
        let domain = r.opt_string()?.map(DomainId::new);
        let failure_ids = {
            let len = r.u32()? as usize;
            let mut v = Vec::with_capacity(len.min(1024));
            for _ in 0..len {
                v.push(EventId::new(r.u64()?));
            }
            v
        };
        let contrastive_success_ids = {
            let len = r.u32()? as usize;
            let mut v = Vec::with_capacity(len.min(1024));
            for _ in 0..len {
                v.push(EventId::new(r.u64()?));
            }
            v
        };
        let a_len = r.u32()? as usize;
        let mut actors = Vec::with_capacity(a_len.min(64));
        for _ in 0..a_len {
            actors.push(ActorId::new(r.string()?));
        }
        let alpha = r.f64()?;
        let beta = r.f64()?;
        let cred_ts = Time::from_millis(r.u64()?);
        let taint = decode_taint(r.u8()?)?;
        let created_at_ms = Time::from_millis(r.u64()?);
        let updated_at_ms = Time::from_millis(r.u64()?);
        let active = r.u8()? != 0;
        lessons.insert(
            lesson_id,
            LessonEntry {
                lesson_id,
                pattern,
                failure_signature,
                domain,
                failure_ids,
                contrastive_success_ids,
                actors,
                credence: BetaCredence {
                    alpha_0: 1.0,
                    beta_0: 1.0,
                    alpha,
                    beta,
                    last_updated_ms: cred_ts,
                },
                taint,
                created_at_ms,
                updated_at_ms,
                active,
            },
        );
    }

    let p_len = r.u32()? as usize;
    let mut procedures = BTreeMap::new();
    for _ in 0..p_len {
        let procedure_id = EventId::new(r.u64()?);
        let name = r.string()?;
        let st_len = r.u32()? as usize;
        let mut steps = Vec::with_capacity(st_len.min(128));
        for _ in 0..st_len {
            steps.push(r.string()?);
        }
        let cap_len = r.u32()? as usize;
        let mut required_cap_ids = Vec::with_capacity(cap_len.min(64));
        for _ in 0..cap_len {
            required_cap_ids.push(CapabilityId::new(r.string()?));
        }
        let capability_schema_hash = CapabilitySchemaHash::new(r.string()?);
        let domain = r.opt_string()?.map(DomainId::new);
        let pl_len = r.u32()? as usize;
        let mut places = Vec::with_capacity(pl_len.min(64));
        for _ in 0..pl_len {
            places.push(PlaceId::new(r.string()?));
        }
        let a_len = r.u32()? as usize;
        let mut actors = Vec::with_capacity(a_len.min(64));
        for _ in 0..a_len {
            actors.push(ActorId::new(r.string()?));
        }
        let audience = decode_audience(r)?;
        let owner_signed = r.u8()? != 0;
        let embedding = match r.u8()? {
            0 => None,
            1 => Some(decode_tagged_embedding(r)?),
            tag => {
                return Err(EventFormatError::MalformedRecord(format!(
                    "invalid proc embedding tag {tag}"
                )));
            }
        };
        let source_goal_id = r.opt_u64()?.map(EventId::new);
        let source_event_ids = {
            let len = r.u32()? as usize;
            let mut v = Vec::with_capacity(len.min(1024));
            for _ in 0..len {
                v.push(EventId::new(r.u64()?));
            }
            v
        };
        let taint = decode_taint(r.u8()?)?;
        let state_code = r.u8()?;
        let state = ProcedureLifecycleState::from_code(state_code)
            .unwrap_or(ProcedureLifecycleState::Active);
        let retired_reason = r.opt_string()?;
        let retired_at_ms = r.opt_u64()?.map(Time::from_millis);
        let created_at_ms = Time::from_millis(r.u64()?);
        let updated_at_ms = Time::from_millis(r.u64()?);
        let alpha = r.f64()?;
        let beta = r.f64()?;
        let trial_count = r.u64()?;
        let success_count = r.u64()?;
        let cusum_baseline_p0 = match r.u8()? {
            0 => None,
            1 => Some(r.f64()?),
            tag => {
                return Err(EventFormatError::MalformedRecord(format!(
                    "invalid cusum baseline tag {tag}"
                )));
            }
        };
        let cusum_stat = r.f64()?;
        let warn_count = r.u32()?;
        procedures.insert(
            procedure_id,
            TrackedProcedure {
                procedure_id,
                name,
                steps,
                required_cap_ids,
                capability_schema_hash,
                domain,
                places,
                actors,
                audience,
                owner_signed,
                embedding,
                source_goal_id,
                source_event_ids,
                taint,
                state,
                retired_reason,
                retired_at_ms,
                created_at_ms,
                updated_at_ms,
                alpha,
                beta,
                trial_count,
                success_count,
                cusum_baseline_p0,
                cusum_stat,
                warn_count,
            },
        );
    }

    let act_len = r.u32()? as usize;
    let mut actors = BTreeMap::new();
    for _ in 0..act_len {
        let actor_id = ActorId::new(r.string()?);
        let reliability_alpha = r.f64()?;
        let reliability_beta = r.f64()?;
        let trusted_origin = r.u8()? != 0;
        let w_len = r.u32()? as usize;
        let mut witnessed = Vec::with_capacity(w_len.min(256));
        for _ in 0..w_len {
            witnessed.push(WitnessedFactSummary {
                event_id: EventId::new(r.u64()?),
                place: PlaceId::new(r.string()?),
                timestamp_ms: Time::from_millis(r.u64()?),
            });
        }
        let or_len = r.u32()? as usize;
        let mut open_requests = Vec::with_capacity(or_len.min(256));
        for _ in 0..or_len {
            open_requests.push(OpenRequestItem {
                expectation_id: EventId::new(r.u64()?),
                kind: crate::event::decode_expect_kind(r.u8()?)?,
                due_ms: Time::from_millis(r.u64()?),
            });
        }
        let ti_len = r.u32()? as usize;
        let mut told_items = Vec::with_capacity(ti_len.min(256));
        for _ in 0..ti_len {
            told_items.push(ToldLedgerItem {
                belief_id: EventId::new(r.u64()?),
                predicate: r.string()?,
                timestamp_ms: Time::from_millis(r.u64()?),
                corroborated: r.u8()? != 0,
                refuted: r.u8()? != 0,
            });
        }
        let fb_len = r.u32()? as usize;
        let mut false_beliefs = Vec::with_capacity(fb_len.min(256));
        for _ in 0..fb_len {
            false_beliefs.push(FalseBeliefFlag {
                belief_id: EventId::new(r.u64()?),
                predicate: r.string()?,
                flagged_at_ms: Time::from_millis(r.u64()?),
            });
        }
        actors.insert(
            actor_id.clone(),
            ActorCard {
                actor_id,
                reliability_alpha,
                reliability_beta,
                trusted_origin,
                witnessed,
                open_requests,
                told_items,
                false_beliefs,
            },
        );
    }

    let cap_len = r.u32()? as usize;
    let mut capabilities = BTreeMap::new();
    for _ in 0..cap_len {
        let capability_id = CapabilityId::new(r.string()?);
        let trials = r.u64()?;
        let successes = r.u64()?;
        let failures = r.u64()?;
        let consec_fail = r.u32()?;
        let alpha = r.f64()?;
        let beta = r.f64()?;
        let degraded = r.u8()? != 0;
        let latency_sum_ms = u128::from(r.u64()?);
        let latency_samples = r.u64()?;
        let last_updated_ms = Time::from_millis(r.u64()?);
        capabilities.insert(
            capability_id.clone(),
            CapabilityHealthStats {
                capability_id,
                trials,
                successes,
                failures,
                consec_fail,
                alpha,
                beta,
                degraded,
                latency_sum_ms,
                latency_samples,
                last_updated_ms,
            },
        );
    }

    let mut bin_incorrect = [0u64; 5];
    for slot in &mut bin_incorrect {
        *slot = r.u64()?;
    }
    let mut bin_correct = [0u64; 5];
    for slot in &mut bin_correct {
        *slot = r.u64()?;
    }
    let brier_sum = r.f64()?;
    let signed_error_sum = r.f64()?;
    let samples = r.u64()?;
    let principle_alpha = r.f64()?;
    let principle_beta = r.f64()?;
    let contradiction_flags = r.u64()?;
    let daily_budget_used = r.f64()?;
    let daily_budget_limit = r.f64()?;
    let recalibration_count = r.u64()?;
    let last_recalibration_ms = r.opt_u64()?.map(Time::from_millis);
    let last_capability_schema_epoch = r.u64()?;

    let day_index = r.u64()?;
    let mut daily_abs_delta = [0.0f64; 7];
    for slot in &mut daily_abs_delta {
        *slot = r.f64()?;
    }
    let next_epoch_id = r.u64()?;
    let update_count = r.u64()?;
    let rollback_count = r.u64()?;
    let processed_event_ids = {
        let len = r.u32()? as usize;
        let mut v = Vec::with_capacity(len.min(4096));
        for _ in 0..len {
            v.push(EventId::new(r.u64()?));
        }
        v
    };
    let cp_len = r.u32()? as usize;
    let mut checkpoints = Vec::with_capacity(cp_len.min(1024));
    for _ in 0..cp_len {
        let epoch_id = r.u64()?;
        let source_event_id = EventId::new(r.u64()?);
        let timestamp_ms = Time::from_millis(r.u64()?);
        let weights_before = decode_adaptive_weights(r)?;
        let mut daily_abs_delta_before = [0.0f64; 7];
        for slot in &mut daily_abs_delta_before {
            *slot = r.f64()?;
        }
        let day_index_before = r.u64()?;
        checkpoints.push(LearningEpochCheckpoint {
            epoch_id,
            source_event_id,
            timestamp_ms,
            weights_before,
            daily_abs_delta_before,
            day_index_before,
        });
    }

    Ok(DecodedC7State {
        skill_book: SkillBook {
            failure_episodes,
            success_episodes,
            lessons,
            procedures,
            next_synthetic_id,
        },
        ledger: PerceptionLedger {
            actors,
            capabilities,
            calibration: SelfCalibrationStats {
                bin_incorrect,
                bin_correct,
                brier_sum,
                signed_error_sum,
                samples,
            },
            principle_alpha,
            principle_beta,
            contradiction_flags,
            daily_budget_used,
            daily_budget_limit,
            recalibration_count,
            last_recalibration_ms,
            last_capability_schema_epoch,
        },
        learner: BoundedLearner {
            day_index,
            daily_abs_delta,
            next_epoch_id,
            checkpoints,
            update_count,
            rollback_count,
            processed_event_ids,
        },
        procedure_links,
        recalibration_pending,
    })
}
