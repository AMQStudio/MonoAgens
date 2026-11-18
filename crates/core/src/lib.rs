//! Sans-IO cognitive core for MonoAgens.
//!
//! `monoagens-core` depends strictly on `monoagens-protocol` and `serde`, performing
//! no filesystem, network, clock, or process IO directly.

#![forbid(unsafe_code)]

use core::fmt;

use monoagens_protocol::{
    Action, ActionId, ActorFacts, ActorId, Audience, BodyState, ClassificationVerdict,
    DeclassToken, DomainId, EventId, OpaquePayload, OriginActor, OwnerSigned, Percept, PlaceId,
    PromiseMeta, ProtocolError, Receipt, TaggedEmbedding, Taint, TerminationReason, ThoughtAct,
    Time,
};
use serde::{Deserialize, Serialize};

pub mod activation;
pub mod beliefs;
pub mod blake3;
pub mod consolidation;
pub mod context;
pub mod event;
pub mod expectations;
pub mod gate;
pub mod learn;
pub mod ledger;
pub mod r#loop;
pub mod map;
pub mod mem_store;
pub mod ports;
pub mod prior;
pub mod privacy;
pub mod recall;
pub use recall as memory;
pub mod registry;
pub mod replay;
pub mod rng;
pub mod state;
#[cfg(any(test, feature = "tck-hooks"))]
pub mod tck_hooks;
pub mod turn;

pub use activation::{
    ActivationTrace, MIN_AGE_HOURS, closeness_from_trace, context_base_level, decay_exponent,
    petrov_from_timestamps, petrov_summary, place_familiarity, sigmoid,
};
pub use beliefs::{
    BeliefBook, BeliefLogEntry, BeliefRecord, BetaCredence, CredenceRebaseRecord,
    DEFAULT_DOMAIN_ID, DomainCentroid, DomainClusterBook, DomainClusterUpdate,
    DomainTransitionKind, DomainTransitionRecord, hungarian_min_cost, parse_basis_str,
    verify_claimed_basis,
};
pub use consolidation::{
    BoundaryEvaluation, BoundaryInputs, EpisodeAdmitParams, EpisodeBook, EpisodeEventOutcome,
    EpisodeRecord, EpisodeState, ExtractProcedureParams, FadeCandidate, FailureEpisodeRecord,
    LessonEntry, PendingSleepRun, PlaceBoundaryState, ProcedureLifecycleState,
    ProcedureMonitorOutcome, RecordLessonParams, RegisterProcedureParams, SkillBook,
    SleepTriggerStatus, SuccessEpisodeRecord, TrackedProcedure, compute_procedure_schema_hash,
    empirical_bayes_moments, evaluate_sleep_trigger, join_episode_taint, participant_jaccard,
    remaining_unconsolidated_episodes, schema_matches_catalog, select_fade_candidates,
};
pub use context::{
    AutoRecallTriggerState, CapabilityCatalogEntry, CommitmentSlotItem, ContextSections,
    ContextSegment, ContextSlotKind, CoreSelfCard, CoreSelfSnapshot, DroppedContextItem,
    IdentityAnchor, MechanismSwitchState, ModelTask, PrincipleAuditVerdict, PrincipleDescriptor,
    PrincipleSeverity, ProcedureEntry, SelectedToolSet, SelfGuideTemplateEntry, SourceLedgerCard,
    ToolSelectionBudget, TrustSegmentKind, ValveSelfSnapshot, VisibleScopeSummary,
    calibrate_confidence_0_20, classify_trust_segment, compute_report_threshold,
    compute_static_prefix_digest, enforce_context_budget, estimate_record_bytes,
    segment_from_event, segment_from_recalled, select_tools_for_context, self_guide_template_for,
    self_guide_template_table,
};
pub use event::{
    DecisionOutcome, DoorKind, EventDetail, EventFormat, EventFormatError, EventKind, EventRecord,
    EventRecordV1, ExpectationLifecycleState, LogSeq, NumericFeatures,
};
pub use expectations::{
    ConditionVerdictRecord, ContentMissReport, DeferredDependentAct, EffectPredictionSpec,
    ExpectationBook, ExpectationRecord, MonitorState, PendingClassifyKind, PlaceDistanceStats,
    PromiseConflict, ResponseLatencyTracker, ResponsePredictionSpec, WakeGateCalibrator,
    advance_recurrence_schedule, is_fulfilled_state, is_positive_verdict_label, is_terminal_state,
    map_resolve_outcome,
};
pub use gate::{
    BacklogEventInput, BacklogScheduleResult, CommitmentInputs, DecisionReason, DecisionTrace,
    DegEvaluation, EffortControl, FrozenDialogueActHead, GateOutcome, GateSignals, HabituationBook,
    HabituationEntry, HabituationKey, LatencySketch, MAX_CHUNKS_PER_WAKE, OpenWait,
    PatienceEstimator, PendingBeliefSummary, PlaceBacklogChunk, PromiseSummary, RecomputedTrace,
    SalienceEvaluation, SalienceWeights, SwitchingBudget, ThresholdTerms,
};
pub use learn::{
    BoundedLearner, LearningEpochCheckpoint, LearningOutcomeKind, LearningSample,
    LearningStepResult, calibrate_wake_gate_lo, clip_ips_weight,
};
pub use ledger::{
    ActorCard, CapabilityHealthStats, FalseBeliefFlag, OpenRequestItem, PerceptionLedger,
    PrincipleRecordSummary, RecalibrationTrigger, SelfCalibrationStats, ToldLedgerItem,
    WitnessedFactSummary, collect_source_card_actors,
};
pub use r#loop::{
    CLASS_A_CAPACITY, CLASS_B_CAPACITY, CLASS_C_CAPACITY, QueueClass, QueueOverflow,
    ThreeClassQueue,
};
pub use map::{CoOccurrenceEdge, CognitiveMap, MapVertex};
pub use mem_store::MemStore;
pub use ports::{
    ContextPlan, DoorDecision, Doors, ModelPurpose, NoopDoors, SnapshotCheckpoint, Store,
    StoreError,
};
pub use prior::{CURRENT_PRIOR_FORMAT_VERSION, Prior, PriorError};
pub use recall::{
    ContextMatchTag, ImportanceInputs, MemoryBasis, MemoryEntry, RecallQuery, RecallResult,
    RecalledItem, RecollectionConstraint, SourceEventFact, classify_context_match,
    compute_importance, evaluate_familiarity, run_recall, verify_recollection,
};
pub use registry::{
    MechanismEntry, MechanismId, NeutralValue, RegistryError, Switches, brain_map_registry,
};
pub use replay::{
    RecomputedDecisionValue, ReplayDivergence, ReplayMode, ReplayReport, replay_from_checkpoint,
    replay_log,
};
pub use rng::{GateDraw, SeededRng};
pub use state::{AdaptiveWeightsAnchor, CoreState, EpochAnchor};

/// All synchronous inputs admitted into [`Core::step`] and [`Core::enqueue`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Input {
    Percept(Box<Percept>),
    Receipt(Receipt),
    CancelAction {
        action_id: ActionId,
        termination_reason: TerminationReason,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason_code: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        detail: Option<String>,
    },
    ModelResult {
        request_id: EventId,
        acts: Vec<ThoughtAct>,
    },
    ClassifyResult {
        request_id: EventId,
        verdict: ClassificationVerdict,
    },
    ModelFailure {
        request_id: EventId,
        reason: String,
    },
    Tick,
    Identity(OwnerSigned<OpaquePayload>),
    BodyState(BodyState),
    ActorFacts(ActorFacts),
}

/// Synchronous effects returned by [`Core::step`] for the Gateway to execute.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Effect {
    Model {
        purpose: ModelPurpose,
        plan: ContextPlan,
    },
    Dispatch(Action),
}

/// Summary of crash recovery performed during [`Core::open`].
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct RecoveryReport {
    pub snapshot_cursor: Option<LogSeq>,
    pub replayed_events: usize,
    pub resumed_inputs: Vec<EventId>,
}

/// Errors produced by [`Core::open`], [`Core::enqueue`], or [`Core::step`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoreError {
    Store(StoreError),
    Protocol(ProtocolError),
    EventFormat(EventFormatError),
    Prior(PriorError),
    Registry(RegistryError),
    QueueOverflow(QueueOverflow),
    DuplicateReceipt(ActionId),
    SnapshotChecksumMismatch {
        snapshot_id: String,
        expected: String,
        actual: String,
    },
    NotImplemented(&'static str),
}

impl fmt::Display for CoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Store(err) => write!(f, "{err}"),
            Self::Protocol(err) => write!(f, "{err}"),
            Self::EventFormat(err) => write!(f, "{err}"),
            Self::Prior(err) => write!(f, "{err}"),
            Self::Registry(err) => write!(f, "{err}"),
            Self::QueueOverflow(err) => write!(f, "{err}"),
            Self::DuplicateReceipt(action_id) => {
                write!(f, "duplicate terminal receipt for action '{action_id}'")
            }
            Self::SnapshotChecksumMismatch {
                snapshot_id,
                expected,
                actual,
            } => write!(
                f,
                "snapshot '{snapshot_id}' blake3 checksum mismatch: expected {expected}, got {actual}"
            ),
            Self::NotImplemented(msg) => write!(f, "not implemented yet: {msg}"),
        }
    }
}

impl std::error::Error for CoreError {}

impl From<StoreError> for CoreError {
    fn from(err: StoreError) -> Self {
        Self::Store(err)
    }
}

impl From<ProtocolError> for CoreError {
    fn from(err: ProtocolError) -> Self {
        Self::Protocol(err)
    }
}

impl From<EventFormatError> for CoreError {
    fn from(err: EventFormatError) -> Self {
        Self::EventFormat(err)
    }
}

impl From<PriorError> for CoreError {
    fn from(err: PriorError) -> Self {
        Self::Prior(err)
    }
}

impl From<RegistryError> for CoreError {
    fn from(err: RegistryError) -> Self {
        Self::Registry(err)
    }
}

impl From<QueueOverflow> for CoreError {
    fn from(err: QueueOverflow) -> Self {
        Self::QueueOverflow(err)
    }
}

/// Sans-IO deterministic state machine driving the MonoAgens cognitive loop.
#[derive(Debug)]
pub struct Core<S: Store> {
    store: S,
    prior: Prior,
    switches: Switches,
    seed: u64,
    state: CoreState,
    queue: ThreeClassQueue<Input>,
    recovery_report: RecoveryReport,
}

impl<S: Store> Core<S> {
    /// Opens a `Core` over `store`, verifying any `blake3` snapshot, replaying the tail in
    /// [`LogSeq`] order, and resuming interrupted inputs deterministically before admitting new ingress.
    pub fn open(store: S, prior: Prior, switches: Switches, seed: u64) -> Result<Self, CoreError> {
        turn::open_core(store, prior, switches, seed)
    }

    pub(crate) fn from_parts(
        store: S,
        prior: Prior,
        switches: Switches,
        seed: u64,
        state: CoreState,
        recovery_report: RecoveryReport,
    ) -> Self {
        Self {
            store,
            prior,
            switches,
            seed,
            state,
            queue: ThreeClassQueue::new(),
            recovery_report,
        }
    }

    /// Enqueues an [`Input`] into its default bounded priority class (`A > B > C`).
    pub fn enqueue(&mut self, input: Input) -> Result<QueueClass, CoreError> {
        let class = input.queue_class();
        self.queue.push(class, input)?;
        Ok(class)
    }

    /// Enqueues an [`Input`] into an explicit bounded priority [`QueueClass`].
    pub fn enqueue_in_class(&mut self, class: QueueClass, input: Input) -> Result<(), CoreError> {
        self.queue.push(class, input)?;
        Ok(())
    }

    /// Pops and processes the highest-priority queued [`Input`] (`A > B > C`), if any.
    pub fn step_queued(
        &mut self,
        now: Time,
        doors: &mut dyn Doors,
    ) -> Result<Option<Vec<Effect>>, CoreError> {
        let Some((_class, input)) = self.queue.pop() else {
            return Ok(None);
        };
        let effects = turn::execute_turn(self, input, now, doors)?;
        Ok(Some(effects))
    }

    /// Enqueues `input` and executes one turn of the loop (draining the highest-priority queued item).
    pub fn step(
        &mut self,
        input: Input,
        now: Time,
        doors: &mut dyn Doors,
    ) -> Result<Vec<Effect>, CoreError> {
        let class = input.queue_class();
        self.queue.push(class, input)?;
        let Some((_drained_class, next_input)) = self.queue.pop() else {
            return Ok(Vec::new());
        };
        turn::execute_turn(self, next_input, now, doors)
    }

    /// Persists a `blake3`-checksummed state snapshot at the current [`LogSeq`] cursor.
    pub fn take_snapshot(
        &mut self,
        snapshot_id: impl Into<String>,
    ) -> Result<SnapshotCheckpoint, CoreError> {
        turn::take_snapshot(self, snapshot_id)
    }

    /// Returns the earliest scheduled due timestamp across open waits, expectations, and sleep, if any.
    #[must_use]
    pub fn next_due(&self) -> Option<Time> {
        let wait_due = self.state.open_waits.values().map(|w| w.due_ms).min();
        let exp_due = self.state.expectation_book.next_due();
        let sleep_due = if self.switches.consolidation_and_merge
            && self.state.pending_sleep_run.is_none()
            && self.state.unconsolidated_count >= self.prior.n_min_sleep
        {
            self.state.last_global_activity_ms.map(|last_act| {
                let wake_elapsed = self
                    .state
                    .last_sleep_ms
                    .map_or(0, |s| last_act.saturating_sub_ms(s));
                let d_sleep = gate::sleep_pressure(
                    self.state.unconsolidated_count,
                    wake_elapsed,
                    &self.prior,
                );
                let eff_idle = gate::effective_idle_trigger_ms(self.prior.t_idle_sleep_ms, d_sleep);
                last_act.saturating_add_ms(eff_idle)
            })
        } else {
            None
        };
        [wait_due, exp_due, sleep_due].into_iter().flatten().min()
    }

    /// Runs coalesced downtime startup recovery explicitly (also triggered automatically on first `Tick` after `Core::open`).
    pub fn recover_startup(
        &mut self,
        now: Time,
        doors: &mut dyn Doors,
    ) -> Result<Vec<Effect>, CoreError> {
        turn::run_startup_recovery(self, now, doors)
    }

    /// Attaches embedding vectors to an open expectation's free-text condition and syncs its cue.
    pub fn set_expectation_embeddings(
        &mut self,
        expectation_id: EventId,
        embeddings: Vec<TaggedEmbedding>,
    ) {
        if let Some(exp) = self
            .state
            .expectation_book
            .expectations
            .get_mut(&expectation_id)
        {
            exp.condition_embeddings = embeddings;
            self.state.sync_expectation_entry(expectation_id);
        }
    }

    /// Attaches act and/or expected-reply embeddings to an open `Response` prediction for `action_id`.
    pub fn set_response_embeddings(
        &mut self,
        action_id: &ActionId,
        act_embeddings: Vec<TaggedEmbedding>,
        expected_reply_embeddings: Vec<TaggedEmbedding>,
    ) {
        for exp in self.state.expectation_book.expectations.values_mut() {
            if exp.kind == monoagens_protocol::ExpectKind::Response
                && exp.is_active()
                && let Some(spec) = &mut exp.response_spec
                && spec.action_id.as_ref() == Some(action_id)
            {
                if !act_embeddings.is_empty() {
                    spec.act_embeddings = act_embeddings.clone();
                }
                if !expected_reply_embeddings.is_empty() {
                    spec.expected_reply_embeddings = expected_reply_embeddings.clone();
                }
            }
        }
    }

    /// Attempts to adopt `candidate_hi` as the upper similarity band (`wake_gate_hi`) by the rule of three.
    pub fn try_adopt_wake_gate_upper_band(&mut self, candidate_hi: f64) -> bool {
        self.state
            .expectation_book
            .wake_gate
            .try_adopt_upper_band(candidate_hi, &self.prior)
    }

    /// Checks whether a candidate `PromiseMeta` conflicts with active promises in the expectation book.
    #[must_use]
    pub fn check_promise_conflict(
        &self,
        places: &[PlaceId],
        meta: Option<&PromiseMeta>,
    ) -> Option<PromiseConflict> {
        self.state
            .expectation_book
            .check_conflict(places, meta, None)
    }

    /// Executes hybrid `VisibleScope`-filtered memory recall against the underlying [`Store`]
    /// and projected [`CoreState`], optionally recording `MemoryUsed` ONLY for visible returned items.
    pub fn recall(&mut self, query: &RecallQuery, now: Time) -> Result<RecallResult, CoreError> {
        let result = run_recall(
            &self.store,
            &self.state,
            query,
            now,
            &self.prior,
            &self.switches,
        )?;
        if query.record_uses && !result.used_event_ids.is_empty() {
            let place = query
                .wake_place
                .clone()
                .unwrap_or_else(|| PlaceId::new("core:recall"));
            let mut rec = EventRecord::new_root(
                EventKind::MemoryUsed,
                now,
                ActorId::new("core"),
                place,
                query.visible_scope.audience.clone(),
                monoagens_protocol::OriginActor::new(self.state.soul_actor_id.clone()),
                Taint::Trusted,
                NumericFeatures::default(),
                OpaquePayload::default(),
                Vec::new(),
                Some(EventDetail::MemoryUsed {
                    used_ids: result.used_event_ids.clone(),
                }),
            );
            rec.caused_by = result.used_event_ids.clone();
            let id = self.store.append(rec.clone())?;
            if let Some(persisted) = self.store.get(id)? {
                self.state.apply_event(&persisted);
            } else {
                rec.id = id;
                rec.seq = LogSeq::new(self.state.last_seq.as_u64().saturating_add(1));
                self.state.apply_event(&rec);
            }
        }
        Ok(result)
    }

    /// Logs an owner-signed [`DeclassToken`] in the event log and updates Core's active scope state.
    pub fn log_declass_token(
        &mut self,
        signed_token: OwnerSigned<DeclassToken>,
        now: Time,
    ) -> Result<EventId, CoreError> {
        let place = signed_token
            .payload
            .target_place
            .clone()
            .unwrap_or_else(|| PlaceId::new("core:declass"));
        let mut aud = signed_token.payload.target_audience.clone();
        if aud.is_empty() {
            aud = Audience::new([signed_token.signer.clone()]);
        }
        let mut rec = EventRecord::new_root(
            EventKind::DoorVerdict,
            now,
            signed_token.signer.clone(),
            place,
            aud,
            monoagens_protocol::OriginActor::new(signed_token.signer.clone()),
            Taint::Trusted,
            NumericFeatures::default(),
            OpaquePayload::default(),
            Vec::new(),
            Some(EventDetail::DeclassGranted { signed_token }),
        );
        let id = self.store.append(rec.clone())?;
        if let Some(persisted) = self.store.get(id)? {
            self.state.apply_event(&persisted);
        } else {
            rec.id = id;
            rec.seq = LogSeq::new(self.state.last_seq.as_u64().saturating_add(1));
            self.state.apply_event(&rec);
        }
        Ok(id)
    }

    /// Registers or updates a provenance-bearing [`MemoryEntry`] in Core's state projection.
    pub fn register_memory_entry(&mut self, entry: MemoryEntry) {
        self.state.upsert_memory_entry(entry);
    }

    /// Marks `old_event_id` as superseded by `new_event_id`.
    pub fn supersede_memory(&mut self, old_event_id: EventId, new_event_id: EventId) {
        self.state.supersede_memory(old_event_id, new_event_id);
    }

    /// Returns a shared reference to the underlying store.
    #[must_use]
    pub fn store(&self) -> &S {
        &self.store
    }

    /// Returns a mutable reference to the underlying store.
    pub fn store_mut(&mut self) -> &mut S {
        &mut self.store
    }

    /// Returns the active [`Prior`].
    #[must_use]
    pub fn prior(&self) -> &Prior {
        &self.prior
    }

    /// Returns the active [`Switches`].
    #[must_use]
    pub fn switches(&self) -> &Switches {
        &self.switches
    }

    /// Returns the deterministic PRNG root seed.
    #[must_use]
    pub const fn seed(&self) -> u64 {
        self.seed
    }

    /// Returns the projected [`CoreState`].
    #[must_use]
    pub fn state(&self) -> &CoreState {
        &self.state
    }

    pub(crate) fn state_mut(&mut self) -> &mut CoreState {
        &mut self.state
    }

    /// Returns the bounded 3-class ingress queue.
    #[must_use]
    pub fn queue(&self) -> &ThreeClassQueue<Input> {
        &self.queue
    }

    /// Returns the recovery report from [`Core::open`].
    #[must_use]
    pub fn recovery_report(&self) -> &RecoveryReport {
        &self.recovery_report
    }

    /// Configures the opaque [`ActorId`] representing the soul itself (default `"soul"`).
    pub fn set_soul_actor_id(&mut self, actor_id: ActorId) {
        self.state.soul_actor_id = actor_id;
    }

    /// Configures the frozen `A-14` logistic `dialogue_act_gap` head `(w_gap, b_gap)`.
    pub fn set_dialogue_act_head(&mut self, head: Option<FrozenDialogueActHead>) {
        self.state.dialogue_act_head = head;
    }

    /// Sets the daily budget usage and limit (`budget_used_ratio = (used / limit).clamp(0, 1)`).
    pub fn set_daily_budget(&mut self, used: f64, limit: f64) {
        self.state.daily_budget_used = used;
        self.state.daily_budget_limit = limit;
        if self.state.ledger.update_budget(used, limit) {
            self.state.recalibration_pending = true;
        }
    }

    /// Registers an owner-signed [`PrincipleDescriptor`] for `[ANCHOR_CHECK]` and `PrincipleAudit`.
    pub fn register_principle(&mut self, principle: OwnerSigned<PrincipleDescriptor>) {
        self.state.register_principle(principle);
    }

    /// Registers or updates a tool capability item with its static-core flag and optional `OwnerSigned` descriptor.
    pub fn register_capability_item(&mut self, entry: CapabilityCatalogEntry) {
        self.state.register_capability_item(entry);
    }

    /// Registers or updates a distilled procedural skill entry (`Tool RAG` procedure store).
    pub fn register_procedure(&mut self, proc_entry: ProcedureEntry) {
        self.state.register_procedure(proc_entry);
    }

    /// Configures the model knowledge cutoff metadata surfaced in `ContextPlan` (`7.1`).
    pub fn set_knowledge_cutoff(&mut self, cutoff: Option<String>) {
        self.state.knowledge_cutoff = cutoff;
    }

    /// Configures a per-domain logistic calibration offset used by `calibrate_confidence_0_20`.
    pub fn set_domain_calibration(&mut self, domain: DomainId, offset: f64) {
        self.state.domain_calibration_offsets.insert(domain, offset);
    }

    /// Returns auto-recall state for a trigger event, if any (`7.4`).
    #[must_use]
    pub fn auto_recall_state(&self, trigger_id: EventId) -> Option<&AutoRecallTriggerState> {
        self.state.auto_recall_states.get(&trigger_id)
    }

    /// Runs `Two-Epoch Replay` verification over the underlying store from the latest snapshot.
    pub fn replay_verify(&self, mode: ReplayMode) -> Result<ReplayReport, CoreError> {
        let ckpt = self.store.latest_checkpoint()?;
        replay::replay_from_checkpoint(&self.store, ckpt.as_ref(), &self.switches, self.seed, mode)
    }

    /// Records a belief with basis verification (`Observed` -> `Inferred` downgrade if ungrounded),
    /// absolute validity timestamps, and `told`/source-prior credence initialization (`10.1`, `10.3`).
    #[allow(clippy::too_many_arguments)]
    pub fn record_belief(
        &mut self,
        claim_key: impl Into<String>,
        requested_basis: MemoryBasis,
        source_actor: Option<ActorId>,
        domain: Option<DomainId>,
        source_event_ids: Vec<EventId>,
        taint: Taint,
        is_mind_inference: bool,
        now: Time,
    ) -> Result<EventId, CoreError> {
        turn::record_belief_event(
            self,
            claim_key.into(),
            requested_basis,
            source_actor,
            domain,
            source_event_ids,
            taint,
            is_mind_inference,
            now,
        )
    }

    /// Records cited outcome evidence (`10.1`), updating ONLY `cited_ids` (never uncited context items).
    #[allow(clippy::too_many_arguments)]
    pub fn record_cited_outcome(
        &mut self,
        cited_ids: &[EventId],
        _uncited_context_ids: &[EventId],
        positive: bool,
        weight: f64,
        source_actor: Option<ActorId>,
        domain: Option<DomainId>,
        now: Time,
    ) -> Result<Vec<EventId>, CoreError> {
        turn::record_cited_outcome_events(
            self,
            cited_ids,
            positive,
            weight,
            source_actor,
            domain,
            now,
        )
    }

    /// Records a discounted `R-32` vicarious peer convention observation (`10.3`).
    pub fn record_convention_observation(
        &mut self,
        place: PlaceId,
        pattern_id: impl Into<String>,
        peer_actor: ActorId,
        domain: Option<DomainId>,
        observed_score: f64,
        now: Time,
    ) -> Result<EventId, CoreError> {
        turn::record_convention_event(
            self,
            place,
            pattern_id.into(),
            peer_actor,
            domain,
            observed_score,
            now,
        )
    }

    /// Applies a post-sleep `consolidate` verbatim-vs-gist verification verdict (`10.2`).
    pub fn verify_gist_conflict(
        &mut self,
        gist_id: EventId,
        conflict_detected: bool,
        now: Time,
    ) -> Result<EventId, CoreError> {
        turn::apply_gist_verification_verdict(self, gist_id, conflict_detected, now)
    }

    /// Executes an authorized `hard_forget` erasure inside `Core`, releasing vectors,
    /// cleanly recomputing surviving `(alpha, beta)` projections, and saving a clean snapshot (`10.4`).
    pub fn hard_forget(
        &mut self,
        target_event_ids: Vec<EventId>,
        target_actor_ids: Vec<ActorId>,
        reason: impl Into<String>,
        now: Time,
    ) -> Result<EventId, CoreError> {
        turn::execute_hard_forget_in_core(
            self,
            target_event_ids,
            target_actor_ids,
            reason.into(),
            now,
        )
    }

    /// Runs snapshot-isolated sleep consolidation if `unconsolidated >= N_min` and `idle >= T_idle_eff`
    /// (or resumes an interrupted sleep run).
    pub fn run_sleep_if_due(
        &mut self,
        now: Time,
        doors: &mut dyn Doors,
    ) -> Result<Vec<Effect>, CoreError> {
        turn::run_sleep_if_due(self, now, doors)
    }

    /// Clusters failure episodes matching `>= lesson_min_episodes` against `<= lesson_max_contrasts`
    /// successful traces (`14.1`), logging a `LessonRecorded` event for each extracted lesson.
    pub fn cluster_contrastive_lessons(
        &mut self,
        now: Time,
    ) -> Result<Vec<LessonEntry>, CoreError> {
        let extracted =
            self.state
                .skill_book
                .cluster_contrastive_lessons(now, &self.prior, &self.switches);
        for lesson in &extracted {
            let mut rec = EventRecord::new_root(
                EventKind::LearningChange,
                now,
                ActorId::new("core"),
                PlaceId::new("core:skills"),
                Audience::new([self.state.soul_actor_id.clone()]),
                OriginActor::new(self.state.soul_actor_id.clone()),
                lesson.taint,
                NumericFeatures::default(),
                OpaquePayload::default(),
                Vec::new(),
                Some(EventDetail::LessonRecorded {
                    lesson_id: lesson.lesson_id,
                    pattern: lesson.pattern.clone(),
                    failure_signature: lesson.failure_signature.clone(),
                    failure_ids: lesson.failure_ids.clone(),
                    contrastive_success_ids: lesson.contrastive_success_ids.clone(),
                    domain: lesson.domain.clone(),
                }),
            );
            let id = self.store.append(rec.clone())?;
            rec.id = id;
            self.state.apply_event(&rec);
        }
        Ok(extracted)
    }

    /// Links an outbound action to an extracted procedure so receipts drive CUSUM tracking (`14.3`).
    pub fn link_action_to_procedure(&mut self, action_id: ActionId, procedure_id: EventId) {
        self.state.procedure_links.insert(action_id, procedure_id);
    }

    /// Records a failure episode observation for future contrastive lesson clustering (`14.1`).
    pub fn record_failure_episode(&mut self, rec: FailureEpisodeRecord) {
        self.state.skill_book.record_failure_episode(rec);
    }

    /// Records a success episode observation for future contrastive lesson pairing (`14.1`).
    pub fn record_success_episode(&mut self, rec: SuccessEpisodeRecord) {
        self.state.skill_book.record_success_episode(rec);
    }

    /// Extracts a cross-place procedure from a goal chain (`14.2`).
    pub fn extract_procedure_from_goal_chain(
        &mut self,
        params: ExtractProcedureParams,
    ) -> Option<TrackedProcedure> {
        let tp = self.state.skill_book.extract_procedure_from_goal_chain(
            params,
            &self.state.capability_catalog,
            &self.prior,
            &self.switches,
        )?;
        self.state.register_procedure(tp.to_context_entry());
        Some(tp)
    }

    /// Updates CUSUM / Beta reliability monitoring for `procedure_id` from a terminal outcome (`14.3`).
    pub fn monitor_procedure_outcome(
        &mut self,
        procedure_id: EventId,
        outcome: monoagens_protocol::ReceiptOutcome,
        termination_reason: Option<TerminationReason>,
        now: Time,
    ) -> ProcedureMonitorOutcome {
        let res = self.state.skill_book.monitor_procedure_outcome(
            procedure_id,
            outcome,
            termination_reason,
            now,
            &self.prior,
        );
        if let Some(tp) = self.state.skill_book.procedures.get(&procedure_id) {
            self.state
                .procedures
                .insert(procedure_id, tp.to_context_entry());
        }
        res
    }

    /// Records an actor-attributed `told` claim in `PerceptionLedger` (`15.1`).
    pub fn record_told_claim(
        &mut self,
        source_actor: &ActorId,
        belief_id: EventId,
        predicate: impl Into<String>,
        timestamp_ms: Time,
    ) {
        self.state.ledger.record_told_claim(
            source_actor,
            belief_id,
            predicate.into(),
            timestamp_ms,
            &self.prior,
        );
    }

    /// Resolves a `told` claim in `PerceptionLedger` and flags refuted claims (`15.1`).
    pub fn resolve_told_claim(
        &mut self,
        source_actor: &ActorId,
        belief_id: EventId,
        predicate: &str,
        supported: bool,
        timestamp_ms: Time,
    ) {
        self.state.ledger.resolve_told_claim(
            source_actor,
            belief_id,
            predicate,
            supported,
            timestamp_ms,
            &self.prior,
        );
    }

    /// Builds `source_cards` using an explicit `Switches` override (`15.1`, `18.2`).
    #[must_use]
    pub fn build_source_cards_with_switches(
        &self,
        participants: &[ActorId],
        switches: &Switches,
    ) -> Vec<SourceLedgerCard> {
        self.state.ledger.build_source_cards(
            participants,
            &self.state.expectation_book,
            &self.state.soul_actor_id,
            switches,
            |_| 0.0,
        )
    }

    /// Records a 2x5 metacognitive calibration observation (`[0, 20]` confidence vs boolean correctness)
    /// strictly as an audited [`EventKind::Verdict`] event (`15.2`).
    pub fn record_calibration_outcome(
        &mut self,
        conf_0_20: u8,
        correct: bool,
        now: Time,
    ) -> Result<EventId, CoreError> {
        let mut rec = EventRecord::new_root(
            EventKind::Verdict,
            now,
            ActorId::new("core"),
            PlaceId::new("core:self_card"),
            Audience::new([self.state.soul_actor_id.clone()]),
            OriginActor::new(self.state.soul_actor_id.clone()),
            Taint::Trusted,
            NumericFeatures::default(),
            OpaquePayload::default(),
            Vec::new(),
            Some(EventDetail::CalibrationRecorded { conf_0_20, correct }),
        );
        let id = self.store.append(rec.clone())?;
        rec.id = id;
        self.state.apply_event(&rec);
        Ok(id)
    }

    /// Records an audited budget update verdict driving Pillar 3 of `self_card` (`15.2`).
    pub fn record_budget_event(
        &mut self,
        used: f64,
        limit: f64,
        now: Time,
    ) -> Result<EventId, CoreError> {
        let mut rec = EventRecord::new_root(
            EventKind::Verdict,
            now,
            ActorId::new("core"),
            PlaceId::new("core:self_card"),
            Audience::new([self.state.soul_actor_id.clone()]),
            OriginActor::new(self.state.soul_actor_id.clone()),
            Taint::Trusted,
            NumericFeatures::default(),
            OpaquePayload::default(),
            Vec::new(),
            Some(EventDetail::BudgetUpdated { used, limit }),
        );
        let id = self.store.append(rec.clone())?;
        rec.id = id;
        self.state.apply_event(&rec);
        Ok(id)
    }

    /// Evaluates and records a bounded learning step (`16.1`, `16.2`), returning full step diagnostics.
    pub fn record_learning_outcome(
        &mut self,
        sample: &LearningSample,
    ) -> Result<LearningStepResult, CoreError> {
        let mut weights = self.state.adaptive_weights;
        let res =
            self.state
                .learner
                .apply_outcome(&mut weights, sample, &self.prior, &self.switches);
        if res.applied {
            self.state.adaptive_weights = weights;
            if let Some(epoch_id) = res.epoch_id {
                let dw = res.delta_weights;
                let mut rec = EventRecord::new_root(
                    EventKind::LearningChange,
                    sample.timestamp_ms,
                    ActorId::new("core"),
                    PlaceId::new("core:learn"),
                    Audience::new([self.state.soul_actor_id.clone()]),
                    OriginActor::new(self.state.soul_actor_id.clone()),
                    Taint::Trusted,
                    NumericFeatures::default(),
                    OpaquePayload::default(),
                    Vec::new(),
                    Some(EventDetail::LearningUpdate {
                        epoch_id,
                        source_event_id: sample.source_event_id,
                        outcome_code: sample.outcome_kind.code(),
                        delta_w: [
                            dw.w_own_name,
                            dw.w_info,
                            dw.w_fok,
                            dw.w_rel,
                            dw.w_commitment,
                            dw.w_sc,
                        ],
                        delta_theta_0: res.delta_theta_0,
                        ips_weight: res.ips_weight,
                        signed_error: res.signed_error,
                    }),
                );
                let id = self.store.append(rec.clone())?;
                rec.id = id;
                self.state.apply_event(&rec);
            }
        }
        Ok(res)
    }

    /// Rolls back adaptive weights to their exact pre-update checkpoint prior to the latest update
    /// (or a specific `epoch_id`) and appends an [`EventKind::Rollback`] audit record (`16.4`).
    pub fn rollback_learning(
        &mut self,
        epoch_id: Option<u64>,
        reason: impl Into<String>,
        now: Time,
    ) -> Result<Option<LearningEpochCheckpoint>, CoreError> {
        let reason_str = reason.into();
        let mut weights = self.state.adaptive_weights;
        let restored = match epoch_id {
            Some(eid) => self.state.learner.rollback_to_epoch(&mut weights, eid),
            None => self.state.learner.rollback_to_previous(&mut weights),
        };
        if let Some(cp) = restored {
            self.state.adaptive_weights = weights;
            let mut rec = EventRecord::new_root(
                EventKind::Rollback,
                now,
                ActorId::new("core"),
                PlaceId::new("core:learn"),
                Audience::new([self.state.soul_actor_id.clone()]),
                OriginActor::new(self.state.soul_actor_id.clone()),
                Taint::Trusted,
                NumericFeatures::default(),
                OpaquePayload::default(),
                Vec::new(),
                Some(EventDetail::WeightRollback {
                    restored_epoch_id: cp.epoch_id,
                    reason: reason_str,
                }),
            );
            let id = self.store.append(rec.clone())?;
            rec.id = id;
            self.state.apply_event(&rec);
            Ok(Some(cp))
        } else {
            Ok(None)
        }
    }

    /// Recomputes and applies shrunk empirical `q_0.10` `wake_gate_lo` calibration (`16.3`).
    pub fn calibrate_wake_gate_lo(&mut self, positive_similarities: &[f64]) -> f64 {
        let updated = calibrate_wake_gate_lo(positive_similarities, &self.prior);
        self.state.expectation_book.wake_gate.wake_gate_lo = updated;
        updated
    }

    /// Returns recalibrated `[0, 1]` confidence adjusted by shrunk empirical calibration bias (`15.2`).
    #[must_use]
    pub fn recalibrated_confidence(&self, raw_conf_0_20: u8) -> f64 {
        self.state
            .ledger
            .calibration
            .recalibrated_confidence(raw_conf_0_20, &self.prior)
    }
}
