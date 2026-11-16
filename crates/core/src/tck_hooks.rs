//! Deterministic TCK (Test Compatibility Kit) verification hooks gated behind `tck-hooks` (`20.1`, `18.2`).
//!
//! Performs zero filesystem, clock, network, or paid-model IO, running exclusively over
//! [`Core`], [`MemStore`], [`Switches`], and deterministic seeded inputs.

use monoagens_protocol::{
    ActorId, Audience, BodyId, BodyState, BodyStatus, CapabilityDescriptor, CapabilityFlags,
    CapabilityId, CapabilitySchemaHash, EventId, OpaquePayload, Percept, PlaceId, Receipt,
    ReceiptOutcome, TaggedEmbedding, Time,
};
use serde::{Deserialize, Serialize};

use crate::context::{CoreSelfCard, SourceLedgerCard};
use crate::event::{DecisionOutcome, EventDetail};
use crate::mem_store::MemStore;
use crate::ports::{ContextPlan, DoorDecision, Doors, Store};
use crate::prior::Prior;
use crate::registry::{MechanismId, Switches};
use crate::state::AdaptiveWeightsAnchor;
use crate::{Core, CoreError, Effect, Input, ReplayMode};

/// Deterministic `Switches` builder for single-mechanism neutral ablation (`18.2`).
#[must_use]
pub fn switches_with_only_neutral(disabled: MechanismId) -> Switches {
    let mut s = Switches::all_enabled();
    set_mechanism_switch(&mut s, disabled, false);
    s
}

/// Returns `Switches` with ALL 22 brain-map mechanisms neutralized (single-loop baseline).
#[must_use]
pub fn switches_all_neutral() -> Switches {
    let mut s = Switches::all_enabled();
    for id in MechanismId::ALL {
        set_mechanism_switch(&mut s, id, false);
    }
    s
}

/// Sets a specific [`MechanismId`] field inside [`Switches`] exhaustively across all 22 rows.
pub const fn set_mechanism_switch(switches: &mut Switches, id: MechanismId, enabled: bool) {
    match id {
        MechanismId::Salience => switches.salience = enabled,
        MechanismId::FloorCost => switches.floor_cost = enabled,
        MechanismId::Habituation => switches.habituation = enabled,
        MechanismId::DriveThreshold => switches.drive_threshold = enabled,
        MechanismId::FamiliarityReserve => switches.familiarity_reserve = enabled,
        MechanismId::StochasticGoNoGo => switches.stochastic_go_no_go = enabled,
        MechanismId::CommitmentBreaker => switches.commitment_breaker = enabled,
        MechanismId::Patience => switches.patience = enabled,
        MechanismId::ReportBarOnThought => switches.report_bar_on_thought = enabled,
        MechanismId::ActivationRecall => switches.activation_recall = enabled,
        MechanismId::VectorFade => switches.vector_fade = enabled,
        MechanismId::PredictionAndGrounding => switches.prediction_and_grounding = enabled,
        MechanismId::ProspectiveMemory => switches.prospective_memory = enabled,
        MechanismId::Credence => switches.credence = enabled,
        MechanismId::BoundedLearning => switches.bounded_learning = enabled,
        MechanismId::MemoryTruth => switches.memory_truth = enabled,
        MechanismId::ReportControl => switches.report_control = enabled,
        MechanismId::ConsolidationAndMerge => switches.consolidation_and_merge = enabled,
        MechanismId::PerceptionLedger => switches.perception_ledger = enabled,
        MechanismId::Agency => switches.agency = enabled,
        MechanismId::EventBoundaries => switches.event_boundaries = enabled,
        MechanismId::BacklogChunking => switches.backlog_chunking = enabled,
    }
}

/// Summary of one neutral-switch single-loop verification run (`18.2`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TckNeutralRunReport {
    pub mechanism_id: Option<MechanismId>,
    pub stable_id: String,
    pub total_events: usize,
    pub decision_count: usize,
    pub replay_ok: bool,
    pub weights_unchanged_under_neutral_learning: bool,
}

/// Captures [`Doors::context`] context plan cards while permitting all traffic for TCK assertions.
#[derive(Debug, Default)]
pub struct TckRecordingDoors {
    pub context_plans: Vec<ContextPlan>,
}

impl Doors for TckRecordingDoors {
    fn inbound(&mut self, _percept: &mut Percept) -> DoorDecision {
        DoorDecision::Allow
    }

    fn context(&mut self, plan: &mut ContextPlan) -> DoorDecision {
        self.context_plans.push(plan.clone());
        DoorDecision::Allow
    }

    fn action(&mut self, _action: &mut monoagens_protocol::Action) -> DoorDecision {
        DoorDecision::Allow
    }
}

/// Inspectable state snapshot dumped by [`TckHarness`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TckDecisionDump {
    pub total_events: usize,
    pub last_decision: Option<DecisionOutcome>,
    pub adaptive_weights: AdaptiveWeightsAnchor,
    pub self_card: CoreSelfCard,
    pub lesson_count: usize,
    pub procedure_count: usize,
    pub retired_procedure_count: usize,
    pub belief_count: usize,
    pub learning_updates: u64,
    pub rollbacks: u64,
}

/// Result of verifying hard-forget erasure across all C1-C7 state projections.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HardForgetAudit {
    pub surviving_memory_entries: usize,
    pub surviving_beliefs: usize,
    pub surviving_ledger_cards: usize,
    pub surviving_skill_lessons: usize,
    pub surviving_procedures: usize,
    pub epoch_frozen: bool,
}

impl HardForgetAudit {
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.surviving_memory_entries == 0
            && self.surviving_beliefs == 0
            && self.surviving_ledger_cards == 0
            && self.surviving_skill_lessons == 0
            && self.surviving_procedures == 0
            && self.epoch_frozen
    }
}

/// Deterministic in-memory harness for running TCK probes against `Core<MemStore>`.
pub struct TckHarness {
    pub core: Core<MemStore>,
    pub doors: TckRecordingDoors,
}

impl TckHarness {
    /// Opens a deterministic harness over `MemStore`.
    pub fn open(prior: Prior, switches: Switches, seed: u64) -> Result<Self, CoreError> {
        let core = Core::open(MemStore::new(), prior, switches, seed)?;
        Ok(Self {
            core,
            doors: TckRecordingDoors::default(),
        })
    }

    /// Registers a basic active body capability so actions & schemas are trackable.
    pub fn register_default_body(
        &mut self,
        cap_name: &str,
        schema_hash: &str,
        now_ms: u64,
    ) -> Result<Vec<Effect>, CoreError> {
        let body = BodyState {
            protocol_version: Default::default(),
            body_id: BodyId::new("body_tck"),
            status: BodyStatus::Ready,
            capabilities: vec![CapabilityDescriptor {
                id: CapabilityId::new(cap_name),
                description: "TCK test capability".to_owned(),
                places: vec![PlaceId::new("room:tck")],
                flags: CapabilityFlags {
                    communicative: true,
                    identity_changing: false,
                    irreversible: false,
                },
                typical_latency_ms: 200,
                capability_schema_hash: CapabilitySchemaHash::new(schema_hash),
                sensed_effect: None,
            }],
            timestamp_ms: Time::from_millis(now_ms),
        };
        self.core.step(
            Input::BodyState(body),
            Time::from_millis(now_ms),
            &mut self.doors,
        )
    }

    /// Injects one standard inbound percept from `actor` at `place`.
    pub fn inject_percept(
        &mut self,
        actor: &str,
        place: &str,
        audience: &[&str],
        ts_ms: u64,
        emb: Option<TaggedEmbedding>,
    ) -> Result<Vec<Effect>, CoreError> {
        let mut p = Percept::minimal(
            ActorId::new(actor),
            PlaceId::new(place),
            Audience::new(audience.iter().map(|s| ActorId::new(*s))),
            Time::from_millis(ts_ms),
            OpaquePayload::from_bytes(b"tck_percept"),
        );
        if let Some(e) = emb {
            p.embeddings.push(e);
        }
        self.core.step(
            Input::Percept(Box::new(p)),
            Time::from_millis(ts_ms),
            &mut self.doors,
        )
    }

    /// Steps a clock tick.
    pub fn inject_tick(&mut self, ts_ms: u64) -> Result<Vec<Effect>, CoreError> {
        self.core
            .step(Input::Tick, Time::from_millis(ts_ms), &mut self.doors)
    }

    /// Injects a terminal receipt for an action.
    pub fn inject_receipt(
        &mut self,
        action_id: &str,
        capability_id: &str,
        outcome: ReceiptOutcome,
        termination_reason: Option<monoagens_protocol::TerminationReason>,
        ts_ms: u64,
    ) -> Result<Vec<Effect>, CoreError> {
        let receipt = Receipt {
            action_id: monoagens_protocol::ActionId::new(action_id),
            capability_id: CapabilityId::new(capability_id),
            outcome,
            reason_code: None,
            detail: None,
            termination_reason,
            progress: None,
            delivered_units: None,
            h_act: None,
            timestamp_ms: Time::from_millis(ts_ms),
        };
        self.core.step(
            Input::Receipt(receipt),
            Time::from_millis(ts_ms),
            &mut self.doors,
        )
    }

    /// Dumps structured decision/projection metrics for TCK assertions.
    pub fn dump(&self) -> Result<TckDecisionDump, CoreError> {
        let events = self.core.store().read_after_seq(None, usize::MAX)?;
        let last_decision = events.iter().rev().find_map(|ev| match &ev.detail {
            Some(EventDetail::Decision { outcome, .. }) => Some(*outcome),
            _ => None,
        });
        let retired_procedure_count = self
            .core
            .state()
            .skill_book
            .procedures
            .values()
            .filter(|p| p.state == crate::consolidation::ProcedureLifecycleState::Retired)
            .count();
        Ok(TckDecisionDump {
            total_events: events.len(),
            last_decision,
            adaptive_weights: self.core.state().adaptive_weights,
            self_card: self.core.state().build_self_card(),
            lesson_count: self.core.state().skill_book.lessons.len(),
            procedure_count: self.core.state().skill_book.procedures.len(),
            retired_procedure_count,
            belief_count: self.core.state().belief_book.beliefs.len(),
            learning_updates: self.core.state().learner.update_count,
            rollbacks: self.core.state().learner.rollback_count,
        })
    }

    /// Returns the most recently captured `source_cards` from `Doors::context`.
    #[must_use]
    pub fn last_source_cards(&self) -> &[SourceLedgerCard] {
        self.doors
            .context_plans
            .last()
            .map_or(&[], |plan| plan.sections.source_cards.as_slice())
    }
}

/// Runs a fixed deterministic single-loop sequence with `switches`, returning a verification report.
pub fn run_neutral_parity_probe(
    mechanism_id: Option<MechanismId>,
    switches: Switches,
    prior: &Prior,
    seed: u64,
) -> Result<TckNeutralRunReport, CoreError> {
    let mut h = TckHarness::open(prior.clone(), switches.clone(), seed)?;
    h.register_default_body("chat.speak", "blake3:tck0", 500)?;
    let _ = h.inject_percept("alice", "room:tck", &["alice", "soul"], 1_000, None)?;
    let _ = h.inject_percept("alice", "room:tck", &["alice", "soul"], 2_000, None)?;
    let _ = h.inject_tick(5_000)?;
    let _ = h.inject_percept("bob", "room:tck", &["bob", "soul"], 6_000, None)?;

    let w_before = h.core.state().adaptive_weights;
    let events = h.core.store().read_after_seq(None, usize::MAX)?;
    let decision_count = events
        .iter()
        .filter(|e| matches!(&e.detail, Some(EventDetail::Decision { .. })))
        .count();
    let replay_report = h.core.replay_verify(ReplayMode::FullReplay)?;
    let weights_unchanged =
        switches.bounded_learning || h.core.state().adaptive_weights == w_before;

    let stable_id = mechanism_id
        .map(|id| id.stable_id().to_owned())
        .unwrap_or_else(|| "all_neutral".to_owned());

    Ok(TckNeutralRunReport {
        mechanism_id,
        stable_id,
        total_events: events.len(),
        decision_count,
        replay_ok: replay_report.first_divergence.is_none(),
        weights_unchanged_under_neutral_learning: weights_unchanged,
    })
}

/// Runs all 22 single-switch neutral ablation probes PLUS `all_neutral` (`18.2`).
pub fn verify_all_22_neutral_parities(
    prior: &Prior,
    seed: u64,
) -> Result<Vec<TckNeutralRunReport>, CoreError> {
    let mut reports = Vec::with_capacity(23);
    for id in MechanismId::ALL {
        let sw = switches_with_only_neutral(id);
        reports.push(run_neutral_parity_probe(Some(id), sw, prior, seed)?);
    }
    reports.push(run_neutral_parity_probe(
        None,
        switches_all_neutral(),
        prior,
        seed,
    )?);
    Ok(reports)
}

/// Executes `hard_forget` for `target_actor` and verifies zero leftover actor state across all C1-C7 books (`20.1`).
pub fn verify_hard_forget_erasure(
    core: &mut Core<MemStore>,
    target_actor: &ActorId,
    now_ms: u64,
) -> Result<HardForgetAudit, CoreError> {
    let events = core.store().read_after_seq(None, usize::MAX)?;
    let target_ids: Vec<EventId> = events
        .iter()
        .filter(|e| &e.actor == target_actor)
        .map(|e| e.id)
        .collect();
    let _ = core.hard_forget(
        target_ids,
        vec![target_actor.clone()],
        "tck_hard_forget_audit",
        Time::from_millis(now_ms),
    )?;
    let st = core.state();
    Ok(HardForgetAudit {
        surviving_memory_entries: st
            .memory_entries
            .values()
            .filter(|m| &m.actor == target_actor || m.audience.contains(target_actor))
            .count(),
        surviving_beliefs: st
            .belief_book
            .beliefs
            .values()
            .filter(|b| b.source_actor.as_ref() == Some(target_actor))
            .count(),
        surviving_ledger_cards: usize::from(st.ledger.actors.contains_key(target_actor)),
        surviving_skill_lessons: st
            .skill_book
            .lessons
            .values()
            .filter(|l| l.actors.contains(target_actor))
            .count(),
        surviving_procedures: st
            .skill_book
            .procedures
            .values()
            .filter(|p| p.actors.contains(target_actor) || p.audience.contains(target_actor))
            .count(),
        epoch_frozen: st.epoch_anchor.frozen,
    })
}
