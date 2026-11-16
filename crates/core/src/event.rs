//! Canonical event log types, causal provenance, and forward/backward format upcasting.

use core::fmt;

use monoagens_protocol::{
    ActionId, ActorId, AttachmentHandle, Audience, BodyId, BodyStatus, CapabilityDescriptor,
    CapabilityFlags, CapabilityId, CapabilitySchemaHash, CatchupPolicy, ChannelId, CorrelationId,
    DeclassToken, DomainId, EmbeddingTag, EventId, ExpectKind, FModal, HAct, ModalitySlot,
    OpaquePayload, OrientScore, OriginActor, OwnerSigned, PlaceId, PromiseMeta, ReceiptOutcome,
    RecurrenceSchedule, RecurrenceSpec, SensedEffect, TaggedEmbedding, Taint, TerminationReason,
    Time,
};
use serde::{Deserialize, Serialize};

use crate::beliefs::{CredenceRebaseRecord, DomainTransitionKind, DomainTransitionRecord};
use crate::gate::{
    DecisionReason, DecisionTrace, GateSignals, RecomputedTrace, SalienceWeights, ThresholdTerms,
};
use crate::recall::MemoryBasis;

/// Monotonic total-order log position assigned by [`crate::ports::Store::append`].
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct LogSeq(pub u64);

impl LogSeq {
    /// Sentinel unassigned sequence number (`0`). Stores assign `>= 1`.
    pub const ZERO: Self = Self(0);

    /// Creates a `LogSeq` from a numeric position.
    #[must_use]
    pub const fn new(seq: u64) -> Self {
        Self(seq)
    }

    /// Returns the underlying `u64` sequence number.
    #[must_use]
    pub const fn as_u64(self) -> u64 {
        self.0
    }
}

impl fmt::Display for LogSeq {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// All 29 canonical event kinds in `design.md` §4 (`R-12`, `R-34`, `R-40`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    Percept,
    Decision,
    WakeStarted,
    ModelCall,
    Thought,
    Withheld,
    Action,
    ActionFeedback,
    Receipt,
    ExpectationOpened,
    ExpectationChanged,
    Verdict,
    MemoryUsed,
    BeliefChanged,
    Evidence,
    EpisodeClosed,
    SleepRun,
    DomainsUpdated,
    VectorReleased,
    TimingUpdated,
    LearningChange,
    Rollback,
    DoorVerdict,
    IdentityChanged,
    BodyState,
    ActorFacts,
    Tick,
    Failure,
    HardForget,
}

impl EventKind {
    /// Exhaustive compile-time list of all 29 canonical event kinds.
    pub const ALL: [Self; 29] = [
        Self::Percept,
        Self::Decision,
        Self::WakeStarted,
        Self::ModelCall,
        Self::Thought,
        Self::Withheld,
        Self::Action,
        Self::ActionFeedback,
        Self::Receipt,
        Self::ExpectationOpened,
        Self::ExpectationChanged,
        Self::Verdict,
        Self::MemoryUsed,
        Self::BeliefChanged,
        Self::Evidence,
        Self::EpisodeClosed,
        Self::SleepRun,
        Self::DomainsUpdated,
        Self::VectorReleased,
        Self::TimingUpdated,
        Self::LearningChange,
        Self::Rollback,
        Self::DoorVerdict,
        Self::IdentityChanged,
        Self::BodyState,
        Self::ActorFacts,
        Self::Tick,
        Self::Failure,
        Self::HardForget,
    ];

    /// Canonical snake_case format name stored in data.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Percept => "percept",
            Self::Decision => "decision",
            Self::WakeStarted => "wake_started",
            Self::ModelCall => "model_call",
            Self::Thought => "thought",
            Self::Withheld => "withheld",
            Self::Action => "action",
            Self::ActionFeedback => "action_feedback",
            Self::Receipt => "receipt",
            Self::ExpectationOpened => "expectation_opened",
            Self::ExpectationChanged => "expectation_changed",
            Self::Verdict => "verdict",
            Self::MemoryUsed => "memory_used",
            Self::BeliefChanged => "belief_changed",
            Self::Evidence => "evidence",
            Self::EpisodeClosed => "episode_closed",
            Self::SleepRun => "sleep_run",
            Self::DomainsUpdated => "domains_updated",
            Self::VectorReleased => "vector_released",
            Self::TimingUpdated => "timing_updated",
            Self::LearningChange => "learning_change",
            Self::Rollback => "rollback",
            Self::DoorVerdict => "door_verdict",
            Self::IdentityChanged => "identity_changed",
            Self::BodyState => "body_state",
            Self::ActorFacts => "actor_facts",
            Self::Tick => "tick",
            Self::Failure => "failure",
            Self::HardForget => "hard_forget",
        }
    }

    /// Stable numeric wire code for compact binary framing.
    #[must_use]
    pub const fn code(self) -> u16 {
        match self {
            Self::Percept => 1,
            Self::Decision => 2,
            Self::WakeStarted => 3,
            Self::ModelCall => 4,
            Self::Thought => 5,
            Self::Withheld => 6,
            Self::Action => 7,
            Self::ActionFeedback => 8,
            Self::Receipt => 9,
            Self::ExpectationOpened => 10,
            Self::ExpectationChanged => 11,
            Self::Verdict => 12,
            Self::MemoryUsed => 13,
            Self::BeliefChanged => 14,
            Self::Evidence => 15,
            Self::EpisodeClosed => 16,
            Self::SleepRun => 17,
            Self::DomainsUpdated => 18,
            Self::VectorReleased => 19,
            Self::TimingUpdated => 20,
            Self::LearningChange => 21,
            Self::Rollback => 22,
            Self::DoorVerdict => 23,
            Self::IdentityChanged => 24,
            Self::BodyState => 25,
            Self::ActorFacts => 26,
            Self::Tick => 27,
            Self::Failure => 28,
            Self::HardForget => 29,
        }
    }

    /// Decodes an [`EventKind`] from its numeric wire code.
    #[must_use]
    pub const fn from_code(code: u16) -> Option<Self> {
        match code {
            1 => Some(Self::Percept),
            2 => Some(Self::Decision),
            3 => Some(Self::WakeStarted),
            4 => Some(Self::ModelCall),
            5 => Some(Self::Thought),
            6 => Some(Self::Withheld),
            7 => Some(Self::Action),
            8 => Some(Self::ActionFeedback),
            9 => Some(Self::Receipt),
            10 => Some(Self::ExpectationOpened),
            11 => Some(Self::ExpectationChanged),
            12 => Some(Self::Verdict),
            13 => Some(Self::MemoryUsed),
            14 => Some(Self::BeliefChanged),
            15 => Some(Self::Evidence),
            16 => Some(Self::EpisodeClosed),
            17 => Some(Self::SleepRun),
            18 => Some(Self::DomainsUpdated),
            19 => Some(Self::VectorReleased),
            20 => Some(Self::TimingUpdated),
            21 => Some(Self::LearningChange),
            22 => Some(Self::Rollback),
            23 => Some(Self::DoorVerdict),
            24 => Some(Self::IdentityChanged),
            25 => Some(Self::BodyState),
            26 => Some(Self::ActorFacts),
            27 => Some(Self::Tick),
            28 => Some(Self::Failure),
            29 => Some(Self::HardForget),
            _ => None,
        }
    }

    /// Minimum supported stored format version for this kind.
    #[must_use]
    pub const fn min_supported_version(self) -> u16 {
        let _ = self;
        1
    }

    /// Current format version for this kind (`v2` supports `v1 -> v2` upcasting).
    #[must_use]
    pub const fn current_version(self) -> u16 {
        let _ = self;
        2
    }

    /// Returns the current [`EventFormat`] descriptor for this kind.
    #[must_use]
    pub const fn current_format(self) -> EventFormat {
        EventFormat {
            kind: self,
            version: self.current_version(),
        }
    }
}

impl fmt::Display for EventKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Format identifier and schema version carried by every stored event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EventFormat {
    pub kind: EventKind,
    pub version: u16,
}

impl EventFormat {
    /// Creates an [`EventFormat`] for `kind` at `version`.
    #[must_use]
    pub const fn new(kind: EventKind, version: u16) -> Self {
        Self { kind, version }
    }
}

impl fmt::Display for EventFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.v{}", self.kind.as_str(), self.version)
    }
}

/// Errors produced when decoding or upcasting a stored event format.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventFormatError {
    UnsupportedFutureVersion {
        format: EventFormat,
        supported_version: u16,
    },
    UnsupportedLegacyVersion {
        format: EventFormat,
        min_supported: u16,
    },
    MalformedRecord(String),
}

impl fmt::Display for EventFormatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedFutureVersion {
                format,
                supported_version,
            } => write!(
                f,
                "unsupported event format '{}' version {} (supported up to {})",
                format.kind.as_str(),
                format.version,
                supported_version
            ),
            Self::UnsupportedLegacyVersion {
                format,
                min_supported,
            } => write!(
                f,
                "unsupported legacy event format '{}' version {} (minimum supported {})",
                format.kind.as_str(),
                format.version,
                min_supported
            ),
            Self::MalformedRecord(detail) => write!(f, "malformed event record: {detail}"),
        }
    }
}

impl std::error::Error for EventFormatError {}

/// Numeric feature slots carried on an event without reading raw payload bytes.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct NumericFeatures {
    pub f_modal: Option<FModal>,
    pub orient: Vec<OrientScore>,
    pub deg_soul: Option<f32>,
    pub dialogue_act_gap: Option<f32>,
    pub delta_obs: Option<f64>,
    pub length: Option<u64>,
    pub negative_flag: Option<bool>,
    pub repair: Option<bool>,
    pub continued_attention: Option<bool>,
    pub slots: Vec<f64>,
}

/// Which of the three Gateway doors produced a [`EventKind::DoorVerdict`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DoorKind {
    Inbound,
    Context,
    Action,
}

impl DoorKind {
    const fn code(self) -> u8 {
        match self {
            Self::Inbound => 1,
            Self::Context => 2,
            Self::Action => 3,
        }
    }

    const fn from_code(code: u8) -> Option<Self> {
        match code {
            1 => Some(Self::Inbound),
            2 => Some(Self::Context),
            3 => Some(Self::Action),
            _ => None,
        }
    }
}

/// Gate decision outcome recorded on [`EventKind::Decision`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionOutcome {
    Go,
    Note,
    Silent,
    DoorBlocked,
    Processed,
}

impl DecisionOutcome {
    const fn code(self) -> u8 {
        match self {
            Self::Go => 1,
            Self::Note => 2,
            Self::Silent => 3,
            Self::DoorBlocked => 4,
            Self::Processed => 5,
        }
    }

    const fn from_code(code: u8) -> Option<Self> {
        match code {
            1 => Some(Self::Go),
            2 => Some(Self::Note),
            3 => Some(Self::Silent),
            4 => Some(Self::DoorBlocked),
            5 => Some(Self::Processed),
            _ => None,
        }
    }
}

/// Lifecycle state for expectations and `PendingApproval` transitions (`R-42`, `R-46`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExpectationLifecycleState {
    Open,
    PendingApproval,
    Approved,
    Rejected,
    Kept,
    KeptLate,
    Renegotiated,
    Released,
    Failed,
    Cancelled,
    Answered,
    Reached,
    Expired,
}

impl ExpectationLifecycleState {
    pub(crate) const fn code(self) -> u8 {
        match self {
            Self::Open => 1,
            Self::PendingApproval => 2,
            Self::Approved => 3,
            Self::Rejected => 4,
            Self::Kept => 5,
            Self::KeptLate => 6,
            Self::Renegotiated => 7,
            Self::Released => 8,
            Self::Failed => 9,
            Self::Cancelled => 10,
            Self::Answered => 11,
            Self::Reached => 12,
            Self::Expired => 13,
        }
    }

    pub(crate) const fn from_code(code: u8) -> Option<Self> {
        match code {
            1 => Some(Self::Open),
            2 => Some(Self::PendingApproval),
            3 => Some(Self::Approved),
            4 => Some(Self::Rejected),
            5 => Some(Self::Kept),
            6 => Some(Self::KeptLate),
            7 => Some(Self::Renegotiated),
            8 => Some(Self::Released),
            9 => Some(Self::Failed),
            10 => Some(Self::Cancelled),
            11 => Some(Self::Answered),
            12 => Some(Self::Reached),
            13 => Some(Self::Expired),
            _ => None,
        }
    }
}

/// Typed structured metadata attached to specific canonical [`EventKind`] records.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "detail_kind", rename_all = "snake_case")]
pub enum EventDetail {
    Decision {
        target_event_id: EventId,
        outcome: DecisionOutcome,
        salience: f64,
        threshold: f64,
        probability: f64,
        draw: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<DecisionReason>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        trace: Option<Box<DecisionTrace>>,
    },
    PerceptMeta {
        addressees: Vec<ActorId>,
        named: Vec<ActorId>,
        reply_to: Option<EventId>,
        correlation_id: Option<CorrelationId>,
        embeddings: Vec<TaggedEmbedding>,
        is_dyadic_chat: bool,
        answered: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        channel: Option<ChannelId>,
    },
    Withheld {
        target_event_id: EventId,
        raw_confidence_0_20: u8,
        calibrated_confidence: f64,
        report_threshold: f64,
        reason: String,
    },
    Action {
        action_id: ActionId,
        capability_id: CapabilityId,
        h_act: Option<HAct>,
        correlation_id: Option<CorrelationId>,
    },
    ActionFeedback {
        action_id: ActionId,
        correlation_id: Option<CorrelationId>,
        progress: Option<f32>,
        delivered_units: Option<u64>,
        note: Option<String>,
    },
    Receipt {
        action_id: ActionId,
        capability_id: CapabilityId,
        outcome: ReceiptOutcome,
        termination_reason: Option<TerminationReason>,
        progress: Option<f32>,
        delivered_units: Option<u64>,
        h_act: Option<HAct>,
    },
    ExpectationTransition {
        expectation_id: EventId,
        action_id: Option<ActionId>,
        h_act: Option<HAct>,
        from_state: Option<ExpectationLifecycleState>,
        to_state: ExpectationLifecycleState,
    },
    Verdict {
        target_event_id: EventId,
        label: String,
    },
    DoorVerdict {
        door: DoorKind,
        target_event_id: Option<EventId>,
        rule: String,
        verdict: String,
    },
    Failure {
        target_event_id: Option<EventId>,
        component: String,
        detail: String,
    },
    HardForget {
        target_event_ids: Vec<EventId>,
        target_actor_ids: Vec<ActorId>,
        epoch: u64,
        reason: String,
    },
    ExpectationOpened {
        expectation_id: EventId,
        target_event_id: EventId,
        kind: ExpectKind,
        place: PlaceId,
        sender: ActorId,
        medium: ChannelId,
        due_ms: Time,
        expiry_ms: Time,
    },
    MemoryUsed {
        used_ids: Vec<EventId>,
    },
    DeclassGranted {
        signed_token: OwnerSigned<DeclassToken>,
    },
    ExpectationRecordOpened {
        expectation_id: EventId,
        origin_event_id: EventId,
        target_event_id: EventId,
        kind: ExpectKind,
        places: Vec<PlaceId>,
        people: Vec<ActorId>,
        domain: Option<DomainId>,
        condition: String,
        condition_embeddings: Vec<TaggedEmbedding>,
        due_ms: Time,
        expiry_ms: Time,
        recurrence: Option<RecurrenceSpec>,
        promise_meta: Option<PromiseMeta>,
        urgency_weight: f64,
        action_id: Option<ActionId>,
        capability_id: Option<CapabilityId>,
        channel: Option<ChannelId>,
        lat_min_ms: u64,
        lat_max_ms: u64,
        delta_pred: Option<f64>,
        sigma_channel: Option<f64>,
        correlation_id: Option<CorrelationId>,
        declared_sensor: bool,
        act_embeddings: Vec<TaggedEmbedding>,
        expected_reply_embeddings: Vec<TaggedEmbedding>,
        is_judged_silence: bool,
        silence_propensity: Option<f64>,
        silence_sender: Option<ActorId>,
        stakes: f64,
        medium: Option<ChannelId>,
        about_ids: Vec<EventId>,
        accept_done_receipt: bool,
        expected_latency_ms: u64,
    },
    ExpectationRevised {
        expectation_id: EventId,
        wake_event_id: EventId,
        new_version: u32,
        condition: Option<String>,
        due_ms: Option<Time>,
        expiry_ms: Option<Time>,
        places: Vec<PlaceId>,
        people: Vec<ActorId>,
        domain: Option<DomainId>,
    },
    BodyCapabilities {
        body_id: BodyId,
        status: BodyStatus,
        capabilities: Vec<CapabilityDescriptor>,
    },
    ConditionVerdict {
        expectation_id: EventId,
        target_event_id: EventId,
        similarity: f64,
        met: bool,
        label: String,
    },
    ContentMiss {
        expectation_id: EventId,
        reply_event_id: EventId,
        who: ActorId,
        timing_delta_ms: i64,
        distance: f64,
        usual_distance: f64,
        z_score: f64,
    },
    SilenceJudged {
        expectation_id: EventId,
        target_event_id: EventId,
        missed: bool,
        reason: String,
        propensity: f64,
    },
    ConflictFlagged {
        proposed_expectation_id: Option<EventId>,
        conflicting_expectation_id: EventId,
        conflict_key: Option<String>,
        overlapping_places: Vec<PlaceId>,
    },
    ExpectationRetryScheduled {
        expectation_id: EventId,
        action_id: ActionId,
        retry_count: u32,
        next_retry_at_ms: Time,
    },
    MonitorFired {
        expectation_id: EventId,
        fired_at_ms: Time,
        next_due_ms: Time,
        coalesced: bool,
        skipped_slots: u64,
    },
    ExpectationLinked {
        expectation_id: EventId,
        linked_event_id: EventId,
        action_id: Option<ActionId>,
    },
    BeliefChanged {
        belief_id: EventId,
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
    Evidence {
        target_id: EventId,
        source_actor: Option<ActorId>,
        domain: DomainId,
        positive: f64,
        negative: f64,
        is_convention: bool,
        place: Option<PlaceId>,
        pattern_id: Option<String>,
        cited_ids: Vec<EventId>,
    },
    EpisodeClosed {
        episode_id: EventId,
        place: PlaceId,
        state_label: String,
        event_ids: Vec<EventId>,
        gist_id: Option<EventId>,
        superseded_gist_id: Option<EventId>,
        merged_from: Vec<EventId>,
    },
    SleepRun {
        snapshot_ms: Time,
        snapshot_seq: LogSeq,
        covered_event_ids: Vec<EventId>,
        produced_ids: Vec<EventId>,
        tokens_used: u64,
        completed: bool,
    },
    DomainsUpdated {
        centroids: Vec<(DomainId, TaggedEmbedding)>,
        transitions: Vec<DomainTransitionRecord>,
        credence_rebased: Vec<CredenceRebaseRecord>,
    },
    VectorReleased {
        event_id: EventId,
        base_level_b: f64,
        fade_threshold: f64,
    },
    LessonRecorded {
        lesson_id: EventId,
        pattern: String,
        failure_signature: String,
        failure_ids: Vec<EventId>,
        contrastive_success_ids: Vec<EventId>,
        domain: Option<DomainId>,
    },
    ProcedureUpdated {
        procedure_id: EventId,
        name: String,
        steps: Vec<String>,
        required_cap_ids: Vec<CapabilityId>,
        capability_schema_hash: CapabilitySchemaHash,
        domain: Option<DomainId>,
        state_code: u8,
        reason: Option<String>,
    },
    LearningUpdate {
        epoch_id: u64,
        source_event_id: EventId,
        outcome_code: u8,
        delta_w: [f64; 6],
        delta_theta_0: f64,
        ips_weight: f64,
        signed_error: f64,
    },
    WeightRollback {
        restored_epoch_id: u64,
        reason: String,
    },
    SelfCardRecalibrated {
        degraded_capabilities: Vec<CapabilityId>,
        calibration_samples: u64,
        budget_headroom_ratio: f64,
        contradiction_flags: u64,
        reason: String,
    },
    CalibrationRecorded {
        conf_0_20: u8,
        correct: bool,
    },
    BudgetUpdated {
        used: f64,
        limit: f64,
    },
}

/// Legacy Phase-0 (`v1`) event record layout supported for backward-compatible upcasting.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventRecordV1 {
    pub id: EventId,
    pub kind: EventKind,
    pub format_version: u16,
    pub timestamp_ms: Time,
    pub actor: ActorId,
    pub place: PlaceId,
    pub audience: Audience,
    pub origin_actor: OriginActor,
    pub taint: Taint,
    pub caused_by: Option<EventId>,
    pub payload: OpaquePayload,
}

/// Canonical recorded log entry stored and retrieved through [`crate::ports::Store`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventRecord {
    pub id: EventId,
    pub seq: LogSeq,
    pub kind: EventKind,
    pub format_version: u16,
    pub timestamp_ms: Time,
    pub actor: ActorId,
    pub place: PlaceId,
    pub audience: Audience,
    pub origin_actor: OriginActor,
    pub taint: Taint,
    pub caused_by: Vec<EventId>,
    pub numeric_features: NumericFeatures,
    pub payload: OpaquePayload,
    pub attachments: Vec<AttachmentHandle>,
    pub detail: Option<EventDetail>,
}

impl EventRecord {
    /// Constructs a new root event at the current format version (`seq` and `id` may be `0` until `Store::append`).
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn new_root(
        kind: EventKind,
        timestamp_ms: Time,
        actor: ActorId,
        place: PlaceId,
        audience: Audience,
        origin_actor: OriginActor,
        taint: Taint,
        numeric_features: NumericFeatures,
        payload: OpaquePayload,
        attachments: Vec<AttachmentHandle>,
        detail: Option<EventDetail>,
    ) -> Self {
        Self {
            id: EventId::new(0),
            seq: LogSeq::ZERO,
            kind,
            format_version: kind.current_version(),
            timestamp_ms,
            actor,
            place,
            audience,
            origin_actor,
            taint,
            caused_by: Vec::new(),
            numeric_features,
            payload,
            attachments,
            detail,
        }
    }

    /// Constructs a derived event whose `origin_actor` and minimum-trust `taint` are inherited
    /// exclusively from `causing` (never settable from model output).
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn derived_from(
        causing: &Self,
        kind: EventKind,
        timestamp_ms: Time,
        actor: ActorId,
        place: PlaceId,
        audience: Audience,
        extra_taint: Option<Taint>,
        numeric_features: NumericFeatures,
        payload: OpaquePayload,
        attachments: Vec<AttachmentHandle>,
        detail: Option<EventDetail>,
    ) -> Self {
        let mut chain = Vec::with_capacity(1 + causing.caused_by.len());
        if causing.id.as_u64() != 0 {
            chain.push(causing.id);
        }
        for &ancestor in &causing.caused_by {
            if !chain.contains(&ancestor) {
                chain.push(ancestor);
            }
        }
        let taint = match extra_taint {
            Some(extra) => causing.taint.combine(extra),
            None => causing.taint,
        };
        Self {
            id: EventId::new(0),
            seq: LogSeq::ZERO,
            kind,
            format_version: kind.current_version(),
            timestamp_ms,
            actor,
            place,
            audience,
            origin_actor: causing.origin_actor.clone(),
            taint,
            caused_by: chain,
            numeric_features,
            payload,
            attachments,
            detail,
        }
    }

    /// Constructs a derived event from multiple causing events, preserving the primary cause's
    /// `origin_actor` and taking `min(taint)` across all causes.
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn derived_from_chain(
        causes: &[&Self],
        fallback_origin: OriginActor,
        kind: EventKind,
        timestamp_ms: Time,
        actor: ActorId,
        place: PlaceId,
        audience: Audience,
        extra_taint: Option<Taint>,
        numeric_features: NumericFeatures,
        payload: OpaquePayload,
        attachments: Vec<AttachmentHandle>,
        detail: Option<EventDetail>,
    ) -> Self {
        let origin_actor = causes
            .first()
            .map_or(fallback_origin, |c| c.origin_actor.clone());
        let mut taint = if causes.is_empty() {
            Taint::Untrusted
        } else {
            Taint::join_min(causes.iter().map(|c| c.taint))
        };
        if let Some(extra) = extra_taint {
            taint = taint.combine(extra);
        }
        let mut chain = Vec::new();
        for cause in causes {
            if cause.id.as_u64() != 0 && !chain.contains(&cause.id) {
                chain.push(cause.id);
            }
            for &ancestor in &cause.caused_by {
                if !chain.contains(&ancestor) {
                    chain.push(ancestor);
                }
            }
        }
        Self {
            id: EventId::new(0),
            seq: LogSeq::ZERO,
            kind,
            format_version: kind.current_version(),
            timestamp_ms,
            actor,
            place,
            audience,
            origin_actor,
            taint,
            caused_by: chain,
            numeric_features,
            payload,
            attachments,
            detail,
        }
    }

    /// Returns this event's [`EventFormat`].
    #[must_use]
    pub const fn format(&self) -> EventFormat {
        EventFormat::new(self.kind, self.format_version)
    }

    /// Recomputes a `Decision` outcome strictly from the numbers stored inside this event record.
    #[must_use]
    pub fn recompute_decision_outcome(&self) -> Option<RecomputedTrace> {
        let Some(EventDetail::Decision {
            trace: Some(trace), ..
        }) = &self.detail
        else {
            return None;
        };
        Some(trace.recompute())
    }

    /// Validates `format_version` and upcasts an older supported [`EventRecordV1`] in memory.
    #[must_use]
    pub fn from_v1(v1: EventRecordV1) -> Self {
        Self {
            id: v1.id,
            seq: LogSeq::ZERO,
            kind: v1.kind,
            format_version: v1.kind.current_version(),
            timestamp_ms: v1.timestamp_ms,
            actor: v1.actor,
            place: v1.place,
            audience: v1.audience,
            origin_actor: v1.origin_actor,
            taint: v1.taint,
            caused_by: v1.caused_by.into_iter().collect(),
            numeric_features: NumericFeatures::default(),
            payload: v1.payload,
            attachments: Vec::new(),
            detail: None,
        }
    }

    /// Validates this record's format version and upcasts it in memory if older than current.
    pub fn upcast(mut self) -> Result<Self, EventFormatError> {
        let supported = self.kind.current_version();
        let min_supported = self.kind.min_supported_version();
        if self.format_version < min_supported {
            return Err(EventFormatError::UnsupportedLegacyVersion {
                format: self.format(),
                min_supported,
            });
        }
        if self.format_version > supported {
            return Err(EventFormatError::UnsupportedFutureVersion {
                format: self.format(),
                supported_version: supported,
            });
        }
        self.format_version = supported;
        Ok(self)
    }

    /// Encodes this record in the current (`v2`) binary format.
    #[must_use]
    pub fn encode_bytes(&self) -> Vec<u8> {
        let mut w = BinWriter::new();
        w.raw(MAGIC);
        w.u16(self.kind.code());
        w.u16(self.kind.current_version());

        w.u64(self.id.as_u64());
        w.u64(self.seq.as_u64());
        w.u64(self.timestamp_ms.as_millis());
        w.str(self.actor.as_str());
        w.str(self.place.as_str());
        w.u32(self.audience.members.len() as u32);
        for member in &self.audience.members {
            w.str(member.as_str());
        }
        w.str(self.origin_actor.actor_id().as_str());
        w.u8(encode_taint(self.taint));
        w.u32(self.caused_by.len() as u32);
        for cause in &self.caused_by {
            w.u64(cause.as_u64());
        }
        encode_numeric_features(&mut w, &self.numeric_features);
        w.bytes(self.payload.as_bytes());
        w.u32(self.attachments.len() as u32);
        for att in &self.attachments {
            w.str(&att.handle_id);
            w.u8(encode_modality(att.modality_slot));
            w.u64(att.byte_len);
            w.u64(att.expires_at_ms.as_millis());
            w.str(&att.embed_model_ver);
        }
        encode_opt_detail(&mut w, self.detail.as_ref());
        w.finish()
    }

    /// Encodes a legacy `v1` record into binary bytes (for backward-compatibility tests).
    #[must_use]
    pub fn encode_v1_bytes(v1: &EventRecordV1) -> Vec<u8> {
        let mut w = BinWriter::new();
        w.raw(MAGIC);
        w.u16(v1.kind.code());
        w.u16(1);
        w.u64(v1.id.as_u64());
        w.u64(v1.timestamp_ms.as_millis());
        w.str(v1.actor.as_str());
        w.str(v1.place.as_str());
        w.u32(v1.audience.members.len() as u32);
        for member in &v1.audience.members {
            w.str(member.as_str());
        }
        w.str(v1.origin_actor.actor_id().as_str());
        w.u8(encode_taint(v1.taint));
        match v1.caused_by {
            Some(c) => {
                w.u8(1);
                w.u64(c.as_u64());
            }
            None => w.u8(0),
        }
        w.bytes(v1.payload.as_bytes());
        w.finish()
    }

    /// Encodes an arbitrary format header version (for future-version failure tests).
    #[must_use]
    pub fn encode_with_version_for_test(kind: EventKind, version: u16, id: EventId) -> Vec<u8> {
        let mut w = BinWriter::new();
        w.raw(MAGIC);
        w.u16(kind.code());
        w.u16(version);
        w.u64(id.as_u64());
        w.finish()
    }

    /// Decodes raw stored bytes, rejecting unsupported newer versions and upcasting older ones in memory.
    pub fn decode_and_upcast(bytes: &[u8]) -> Result<Self, EventFormatError> {
        let mut r = BinReader::new(bytes);
        let magic = r.fixed::<4>()?;
        if &magic != MAGIC {
            return Err(EventFormatError::MalformedRecord(
                "invalid event record magic header".to_owned(),
            ));
        }
        let kind_code = r.u16()?;
        let kind = EventKind::from_code(kind_code).ok_or_else(|| {
            EventFormatError::MalformedRecord(format!("unknown event kind code {kind_code}"))
        })?;
        let version = r.u16()?;
        let format = EventFormat::new(kind, version);
        let supported = kind.current_version();
        let min_supported = kind.min_supported_version();

        if version < min_supported {
            return Err(EventFormatError::UnsupportedLegacyVersion {
                format,
                min_supported,
            });
        }
        if version > supported {
            return Err(EventFormatError::UnsupportedFutureVersion {
                format,
                supported_version: supported,
            });
        }

        if version == 1 {
            let id = EventId::new(r.u64()?);
            let timestamp_ms = Time::from_millis(r.u64()?);
            let actor = ActorId::new(r.string()?);
            let place = PlaceId::new(r.string()?);
            let aud_len = r.u32()? as usize;
            let mut members = Vec::with_capacity(aud_len.min(1024));
            for _ in 0..aud_len {
                members.push(ActorId::new(r.string()?));
            }
            let audience = Audience::new(members);
            let origin_actor = OriginActor::new(ActorId::new(r.string()?));
            let taint = decode_taint(r.u8()?)?;
            let caused_by = match r.u8()? {
                0 => None,
                1 => Some(EventId::new(r.u64()?)),
                tag => {
                    return Err(EventFormatError::MalformedRecord(format!(
                        "invalid v1 caused_by tag {tag}"
                    )));
                }
            };
            let payload = OpaquePayload::from_bytes(r.bytes()?);
            return Ok(Self::from_v1(EventRecordV1 {
                id,
                kind,
                format_version: 1,
                timestamp_ms,
                actor,
                place,
                audience,
                origin_actor,
                taint,
                caused_by,
                payload,
            }));
        }

        // Version 2
        let id = EventId::new(r.u64()?);
        let seq = LogSeq::new(r.u64()?);
        let timestamp_ms = Time::from_millis(r.u64()?);
        let actor = ActorId::new(r.string()?);
        let place = PlaceId::new(r.string()?);
        let aud_len = r.u32()? as usize;
        let mut members = Vec::with_capacity(aud_len.min(1024));
        for _ in 0..aud_len {
            members.push(ActorId::new(r.string()?));
        }
        let audience = Audience::new(members);
        let origin_actor = OriginActor::new(ActorId::new(r.string()?));
        let taint = decode_taint(r.u8()?)?;
        let cause_len = r.u32()? as usize;
        let mut caused_by = Vec::with_capacity(cause_len.min(1024));
        for _ in 0..cause_len {
            caused_by.push(EventId::new(r.u64()?));
        }
        let numeric_features = decode_numeric_features(&mut r)?;
        let payload = OpaquePayload::from_bytes(r.bytes()?);
        let att_len = r.u32()? as usize;
        let mut attachments = Vec::with_capacity(att_len.min(256));
        for _ in 0..att_len {
            let handle_id = r.string()?;
            let modality_slot = decode_modality(r.u8()?)?;
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
        let detail = decode_opt_detail(&mut r)?;

        Ok(Self {
            id,
            seq,
            kind,
            format_version: supported,
            timestamp_ms,
            actor,
            place,
            audience,
            origin_actor,
            taint,
            caused_by,
            numeric_features,
            payload,
            attachments,
            detail,
        })
    }
}

const MAGIC: &[u8; 4] = b"MGEV";

pub(crate) fn encode_taint(t: Taint) -> u8 {
    match t {
        Taint::Untrusted => 0,
        Taint::Provisional => 1,
        Taint::Trusted => 2,
    }
}

pub(crate) fn decode_taint(code: u8) -> Result<Taint, EventFormatError> {
    match code {
        0 => Ok(Taint::Untrusted),
        1 => Ok(Taint::Provisional),
        2 => Ok(Taint::Trusted),
        other => Err(EventFormatError::MalformedRecord(format!(
            "invalid taint tag {other}"
        ))),
    }
}

fn encode_modality(m: ModalitySlot) -> u8 {
    match m {
        ModalitySlot::Image => 1,
        ModalitySlot::AudioVoice => 2,
        ModalitySlot::Video => 3,
        ModalitySlot::SensorBlob => 4,
    }
}

fn decode_modality(code: u8) -> Result<ModalitySlot, EventFormatError> {
    match code {
        1 => Ok(ModalitySlot::Image),
        2 => Ok(ModalitySlot::AudioVoice),
        3 => Ok(ModalitySlot::Video),
        4 => Ok(ModalitySlot::SensorBlob),
        other => Err(EventFormatError::MalformedRecord(format!(
            "invalid modality slot tag {other}"
        ))),
    }
}

pub(crate) fn encode_outcome(o: ReceiptOutcome) -> u8 {
    match o {
        ReceiptOutcome::Done => 1,
        ReceiptOutcome::Refused => 2,
        ReceiptOutcome::Held => 3,
        ReceiptOutcome::Failed => 4,
        ReceiptOutcome::Unknown => 5,
    }
}

pub(crate) fn decode_outcome(code: u8) -> Result<ReceiptOutcome, EventFormatError> {
    match code {
        1 => Ok(ReceiptOutcome::Done),
        2 => Ok(ReceiptOutcome::Refused),
        3 => Ok(ReceiptOutcome::Held),
        4 => Ok(ReceiptOutcome::Failed),
        5 => Ok(ReceiptOutcome::Unknown),
        other => Err(EventFormatError::MalformedRecord(format!(
            "invalid receipt outcome tag {other}"
        ))),
    }
}

pub(crate) fn encode_term_reason(r: TerminationReason) -> u8 {
    match r {
        TerminationReason::Normal => 1,
        TerminationReason::CanceledByCore => 2,
        TerminationReason::PreemptedExternal => 3,
        TerminationReason::TimedOut => 4,
    }
}

pub(crate) fn decode_term_reason(code: u8) -> Result<TerminationReason, EventFormatError> {
    match code {
        1 => Ok(TerminationReason::Normal),
        2 => Ok(TerminationReason::CanceledByCore),
        3 => Ok(TerminationReason::PreemptedExternal),
        4 => Ok(TerminationReason::TimedOut),
        other => Err(EventFormatError::MalformedRecord(format!(
            "invalid termination reason tag {other}"
        ))),
    }
}

fn encode_numeric_features(w: &mut BinWriter, nf: &NumericFeatures) {
    match nf.f_modal {
        Some(FModal(vals)) => {
            w.u8(1);
            for v in vals {
                w.f32(v);
            }
        }
        None => w.u8(0),
    }
    w.u32(nf.orient.len() as u32);
    for o in &nf.orient {
        w.str(o.actor.as_str());
        w.f32(o.score);
    }
    w.opt_f32(nf.deg_soul);
    w.opt_f32(nf.dialogue_act_gap);
    w.opt_f64(nf.delta_obs);
    w.opt_u64(nf.length);
    w.opt_bool(nf.negative_flag);
    w.opt_bool(nf.repair);
    w.opt_bool(nf.continued_attention);
    w.u32(nf.slots.len() as u32);
    for &s in &nf.slots {
        w.f64(s);
    }
}

fn decode_numeric_features(r: &mut BinReader<'_>) -> Result<NumericFeatures, EventFormatError> {
    let f_modal = match r.u8()? {
        0 => None,
        1 => {
            let mut vals = [0.0f32; 10];
            for item in &mut vals {
                *item = r.f32()?;
            }
            Some(FModal(vals))
        }
        tag => {
            return Err(EventFormatError::MalformedRecord(format!(
                "invalid f_modal tag {tag}"
            )));
        }
    };
    let orient_len = r.u32()? as usize;
    let mut orient = Vec::with_capacity(orient_len.min(256));
    for _ in 0..orient_len {
        let actor = ActorId::new(r.string()?);
        let score = r.f32()?;
        orient.push(OrientScore { actor, score });
    }
    let deg_soul = r.opt_f32()?;
    let dialogue_act_gap = r.opt_f32()?;
    let delta_obs = r.opt_f64()?;
    let length = r.opt_u64()?;
    let negative_flag = r.opt_bool()?;
    let repair = r.opt_bool()?;
    let continued_attention = r.opt_bool()?;
    let slot_len = r.u32()? as usize;
    let mut slots = Vec::with_capacity(slot_len.min(256));
    for _ in 0..slot_len {
        slots.push(r.f64()?);
    }
    Ok(NumericFeatures {
        f_modal,
        orient,
        deg_soul,
        dialogue_act_gap,
        delta_obs,
        length,
        negative_flag,
        repair,
        continued_attention,
        slots,
    })
}

fn encode_opt_detail(w: &mut BinWriter, detail: Option<&EventDetail>) {
    let Some(detail) = detail else {
        w.u8(0);
        return;
    };
    match detail {
        EventDetail::Decision {
            target_event_id,
            outcome,
            salience,
            threshold,
            probability,
            draw,
            reason,
            trace,
        } => {
            if reason.is_none() && trace.is_none() {
                w.u8(1);
                w.u64(target_event_id.as_u64());
                w.u8(outcome.code());
                w.f64(*salience);
                w.f64(*threshold);
                w.f64(*probability);
                w.f64(*draw);
            } else {
                w.u8(11);
                w.u64(target_event_id.as_u64());
                w.u8(outcome.code());
                w.f64(*salience);
                w.f64(*threshold);
                w.f64(*probability);
                w.f64(*draw);
                match reason {
                    Some(rsn) => {
                        w.u8(1);
                        w.u8(rsn.code());
                    }
                    None => w.u8(0),
                }
                match trace {
                    Some(tr) => {
                        w.u8(1);
                        encode_decision_trace(w, tr);
                    }
                    None => w.u8(0),
                }
            }
        }
        EventDetail::PerceptMeta {
            addressees,
            named,
            reply_to,
            correlation_id,
            embeddings,
            is_dyadic_chat,
            answered,
            channel,
        } => {
            w.u8(13);
            w.u32(addressees.len() as u32);
            for a in addressees {
                w.str(a.as_str());
            }
            w.u32(named.len() as u32);
            for n in named {
                w.str(n.as_str());
            }
            w.opt_u64(reply_to.map(EventId::as_u64));
            w.opt_str(correlation_id.as_ref().map(CorrelationId::as_str));
            w.u32(embeddings.len() as u32);
            for emb in embeddings {
                w.str(&emb.tag.embed_model_ver);
                w.u32(emb.tag.dim_d);
                w.str(&emb.tag.prompt_regime_id);
                w.u32(emb.values.len() as u32);
                for &v in &emb.values {
                    w.f32(v);
                }
            }
            w.u8(u8::from(*is_dyadic_chat));
            w.f64(*answered);
            w.opt_str(channel.as_ref().map(ChannelId::as_str));
        }
        EventDetail::Withheld {
            target_event_id,
            raw_confidence_0_20,
            calibrated_confidence,
            report_threshold,
            reason,
        } => {
            w.u8(2);
            w.u64(target_event_id.as_u64());
            w.u8(*raw_confidence_0_20);
            w.f64(*calibrated_confidence);
            w.f64(*report_threshold);
            w.str(reason);
        }
        EventDetail::Action {
            action_id,
            capability_id,
            h_act,
            correlation_id,
        } => {
            w.u8(3);
            w.str(action_id.as_str());
            w.str(capability_id.as_str());
            w.opt_str(h_act.as_ref().map(HAct::as_str));
            w.opt_str(correlation_id.as_ref().map(CorrelationId::as_str));
        }
        EventDetail::ActionFeedback {
            action_id,
            correlation_id,
            progress,
            delivered_units,
            note,
        } => {
            w.u8(4);
            w.str(action_id.as_str());
            w.opt_str(correlation_id.as_ref().map(CorrelationId::as_str));
            w.opt_f32(*progress);
            w.opt_u64(*delivered_units);
            w.opt_str(note.as_deref());
        }
        EventDetail::Receipt {
            action_id,
            capability_id,
            outcome,
            termination_reason,
            progress,
            delivered_units,
            h_act,
        } => {
            w.u8(5);
            w.str(action_id.as_str());
            w.str(capability_id.as_str());
            w.u8(encode_outcome(*outcome));
            match termination_reason {
                Some(tr) => {
                    w.u8(1);
                    w.u8(encode_term_reason(*tr));
                }
                None => w.u8(0),
            }
            w.opt_f32(*progress);
            w.opt_u64(*delivered_units);
            w.opt_str(h_act.as_ref().map(HAct::as_str));
        }
        EventDetail::ExpectationTransition {
            expectation_id,
            action_id,
            h_act,
            from_state,
            to_state,
        } => {
            w.u8(6);
            w.u64(expectation_id.as_u64());
            w.opt_str(action_id.as_ref().map(ActionId::as_str));
            w.opt_str(h_act.as_ref().map(HAct::as_str));
            match from_state {
                Some(s) => {
                    w.u8(1);
                    w.u8(s.code());
                }
                None => w.u8(0),
            }
            w.u8(to_state.code());
        }
        EventDetail::Verdict {
            target_event_id,
            label,
        } => {
            w.u8(7);
            w.u64(target_event_id.as_u64());
            w.str(label);
        }
        EventDetail::DoorVerdict {
            door,
            target_event_id,
            rule,
            verdict,
        } => {
            w.u8(8);
            w.u8(door.code());
            w.opt_u64(target_event_id.map(EventId::as_u64));
            w.str(rule);
            w.str(verdict);
        }
        EventDetail::Failure {
            target_event_id,
            component,
            detail,
        } => {
            w.u8(9);
            w.opt_u64(target_event_id.map(EventId::as_u64));
            w.str(component);
            w.str(detail);
        }
        EventDetail::HardForget {
            target_event_ids,
            target_actor_ids,
            epoch,
            reason,
        } => {
            w.u8(10);
            w.u32(target_event_ids.len() as u32);
            for id in target_event_ids {
                w.u64(id.as_u64());
            }
            w.u32(target_actor_ids.len() as u32);
            for actor in target_actor_ids {
                w.str(actor.as_str());
            }
            w.u64(*epoch);
            w.str(reason);
        }
        EventDetail::ExpectationOpened {
            expectation_id,
            target_event_id,
            kind,
            place,
            sender,
            medium,
            due_ms,
            expiry_ms,
        } => {
            w.u8(14);
            w.u64(expectation_id.as_u64());
            w.u64(target_event_id.as_u64());
            w.u8(encode_expect_kind(*kind));
            w.str(place.as_str());
            w.str(sender.as_str());
            w.str(medium.as_str());
            w.u64(due_ms.as_millis());
            w.u64(expiry_ms.as_millis());
        }
        EventDetail::MemoryUsed { used_ids } => {
            w.u8(15);
            w.u32(used_ids.len() as u32);
            for id in used_ids {
                w.u64(id.as_u64());
            }
        }
        EventDetail::DeclassGranted { signed_token } => {
            w.u8(16);
            w.str(signed_token.signer.as_str());
            w.u64(signed_token.signed_at_ms.as_millis());
            w.str(&signed_token.signature);
            w.str(&signed_token.payload.token_id);
            w.str(signed_token.payload.issuer.as_str());
            w.u32(signed_token.payload.scope.len() as u32);
            for id in &signed_token.payload.scope {
                w.u64(id.as_u64());
            }
            w.u64(signed_token.payload.expiry.as_millis());
            w.opt_str(
                signed_token
                    .payload
                    .target_place
                    .as_ref()
                    .map(PlaceId::as_str),
            );
            w.u32(signed_token.payload.target_audience.members.len() as u32);
            for m in &signed_token.payload.target_audience.members {
                w.str(m.as_str());
            }
            w.str(&signed_token.payload.signature);
        }
        EventDetail::ExpectationRecordOpened {
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
        } => {
            w.u8(17);
            w.u64(expectation_id.as_u64());
            w.u64(origin_event_id.as_u64());
            w.u64(target_event_id.as_u64());
            w.u8(encode_expect_kind(*kind));
            w.u32(places.len() as u32);
            for p in places {
                w.str(p.as_str());
            }
            w.u32(people.len() as u32);
            for a in people {
                w.str(a.as_str());
            }
            w.opt_str(domain.as_ref().map(DomainId::as_str));
            w.str(condition);
            encode_tagged_embeddings_bin(w, condition_embeddings);
            w.u64(due_ms.as_millis());
            w.u64(expiry_ms.as_millis());
            encode_opt_recurrence_spec(w, recurrence.as_ref());
            encode_opt_promise_meta(w, promise_meta.as_ref());
            w.f64(*urgency_weight);
            w.opt_str(action_id.as_ref().map(ActionId::as_str));
            w.opt_str(capability_id.as_ref().map(CapabilityId::as_str));
            w.opt_str(channel.as_ref().map(ChannelId::as_str));
            w.u64(*lat_min_ms);
            w.u64(*lat_max_ms);
            w.opt_f64(*delta_pred);
            w.opt_f64(*sigma_channel);
            w.opt_str(correlation_id.as_ref().map(CorrelationId::as_str));
            w.u8(u8::from(*declared_sensor));
            encode_tagged_embeddings_bin(w, act_embeddings);
            encode_tagged_embeddings_bin(w, expected_reply_embeddings);
            w.u8(u8::from(*is_judged_silence));
            w.opt_f64(*silence_propensity);
            w.opt_str(silence_sender.as_ref().map(ActorId::as_str));
            w.f64(*stakes);
            w.opt_str(medium.as_ref().map(ChannelId::as_str));
            w.u32(about_ids.len() as u32);
            for id in about_ids {
                w.u64(id.as_u64());
            }
            w.u8(u8::from(*accept_done_receipt));
            w.u64(*expected_latency_ms);
        }
        EventDetail::ExpectationRevised {
            expectation_id,
            wake_event_id,
            new_version,
            condition,
            due_ms,
            expiry_ms,
            places,
            people,
            domain,
        } => {
            w.u8(18);
            w.u64(expectation_id.as_u64());
            w.u64(wake_event_id.as_u64());
            w.u32(*new_version);
            w.opt_str(condition.as_deref());
            w.opt_u64(due_ms.map(Time::as_millis));
            w.opt_u64(expiry_ms.map(Time::as_millis));
            w.u32(places.len() as u32);
            for p in places {
                w.str(p.as_str());
            }
            w.u32(people.len() as u32);
            for a in people {
                w.str(a.as_str());
            }
            w.opt_str(domain.as_ref().map(DomainId::as_str));
        }
        EventDetail::BodyCapabilities {
            body_id,
            status,
            capabilities,
        } => {
            w.u8(19);
            w.str(body_id.as_str());
            w.u8(encode_body_status_ev(*status));
            w.u32(capabilities.len() as u32);
            for cap in capabilities {
                w.str(cap.id.as_str());
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
        EventDetail::ConditionVerdict {
            expectation_id,
            target_event_id,
            similarity,
            met,
            label,
        } => {
            w.u8(20);
            w.u64(expectation_id.as_u64());
            w.u64(target_event_id.as_u64());
            w.f64(*similarity);
            w.u8(u8::from(*met));
            w.str(label);
        }
        EventDetail::ContentMiss {
            expectation_id,
            reply_event_id,
            who,
            timing_delta_ms,
            distance,
            usual_distance,
            z_score,
        } => {
            w.u8(21);
            w.u64(expectation_id.as_u64());
            w.u64(reply_event_id.as_u64());
            w.str(who.as_str());
            w.i64(*timing_delta_ms);
            w.f64(*distance);
            w.f64(*usual_distance);
            w.f64(*z_score);
        }
        EventDetail::SilenceJudged {
            expectation_id,
            target_event_id,
            missed,
            reason,
            propensity,
        } => {
            w.u8(22);
            w.u64(expectation_id.as_u64());
            w.u64(target_event_id.as_u64());
            w.u8(u8::from(*missed));
            w.str(reason);
            w.f64(*propensity);
        }
        EventDetail::ConflictFlagged {
            proposed_expectation_id,
            conflicting_expectation_id,
            conflict_key,
            overlapping_places,
        } => {
            w.u8(23);
            w.opt_u64(proposed_expectation_id.map(EventId::as_u64));
            w.u64(conflicting_expectation_id.as_u64());
            w.opt_str(conflict_key.as_deref());
            w.u32(overlapping_places.len() as u32);
            for p in overlapping_places {
                w.str(p.as_str());
            }
        }
        EventDetail::ExpectationRetryScheduled {
            expectation_id,
            action_id,
            retry_count,
            next_retry_at_ms,
        } => {
            w.u8(24);
            w.u64(expectation_id.as_u64());
            w.str(action_id.as_str());
            w.u32(*retry_count);
            w.u64(next_retry_at_ms.as_millis());
        }
        EventDetail::MonitorFired {
            expectation_id,
            fired_at_ms,
            next_due_ms,
            coalesced,
            skipped_slots,
        } => {
            w.u8(25);
            w.u64(expectation_id.as_u64());
            w.u64(fired_at_ms.as_millis());
            w.u64(next_due_ms.as_millis());
            w.u8(u8::from(*coalesced));
            w.u64(*skipped_slots);
        }
        EventDetail::ExpectationLinked {
            expectation_id,
            linked_event_id,
            action_id,
        } => {
            w.u8(26);
            w.u64(expectation_id.as_u64());
            w.u64(linked_event_id.as_u64());
            w.opt_str(action_id.as_ref().map(ActionId::as_str));
        }
        EventDetail::BeliefChanged {
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
        } => {
            w.u8(27);
            w.u64(belief_id.as_u64());
            w.str(claim_key);
            w.u8(requested_basis.code());
            w.u8(basis.code());
            w.u8(u8::from(*basis_downgraded));
            w.opt_str(source_actor.as_ref().map(ActorId::as_str));
            w.str(domain.as_str());
            w.u32(source_event_ids.len() as u32);
            for id in source_event_ids {
                w.u64(id.as_u64());
            }
            w.u8(encode_taint(*taint));
            w.u64(valid_from_ms.as_millis());
            w.opt_u64(valid_to_ms.map(Time::as_millis));
            w.opt_u64(replaced_by.map(EventId::as_u64));
            w.f64(*alpha_0);
            w.f64(*beta_0);
            w.u8(u8::from(*is_mind_inference));
        }
        EventDetail::Evidence {
            target_id,
            source_actor,
            domain,
            positive,
            negative,
            is_convention,
            place,
            pattern_id,
            cited_ids,
        } => {
            w.u8(28);
            w.u64(target_id.as_u64());
            w.opt_str(source_actor.as_ref().map(ActorId::as_str));
            w.str(domain.as_str());
            w.f64(*positive);
            w.f64(*negative);
            w.u8(u8::from(*is_convention));
            w.opt_str(place.as_ref().map(PlaceId::as_str));
            w.opt_str(pattern_id.as_deref());
            w.u32(cited_ids.len() as u32);
            for id in cited_ids {
                w.u64(id.as_u64());
            }
        }
        EventDetail::EpisodeClosed {
            episode_id,
            place,
            state_label,
            event_ids,
            gist_id,
            superseded_gist_id,
            merged_from,
        } => {
            w.u8(29);
            w.u64(episode_id.as_u64());
            w.str(place.as_str());
            w.str(state_label);
            w.u32(event_ids.len() as u32);
            for id in event_ids {
                w.u64(id.as_u64());
            }
            w.opt_u64(gist_id.map(EventId::as_u64));
            w.opt_u64(superseded_gist_id.map(EventId::as_u64));
            w.u32(merged_from.len() as u32);
            for id in merged_from {
                w.u64(id.as_u64());
            }
        }
        EventDetail::SleepRun {
            snapshot_ms,
            snapshot_seq,
            covered_event_ids,
            produced_ids,
            tokens_used,
            completed,
        } => {
            w.u8(30);
            w.u64(snapshot_ms.as_millis());
            w.u64(snapshot_seq.as_u64());
            w.u32(covered_event_ids.len() as u32);
            for id in covered_event_ids {
                w.u64(id.as_u64());
            }
            w.u32(produced_ids.len() as u32);
            for id in produced_ids {
                w.u64(id.as_u64());
            }
            w.u64(*tokens_used);
            w.u8(u8::from(*completed));
        }
        EventDetail::DomainsUpdated {
            centroids,
            transitions,
            credence_rebased,
        } => {
            w.u8(31);
            w.u32(centroids.len() as u32);
            for (did, emb) in centroids {
                w.str(did.as_str());
                encode_tagged_embedding(w, emb);
            }
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
        EventDetail::VectorReleased {
            event_id,
            base_level_b,
            fade_threshold,
        } => {
            w.u8(32);
            w.u64(event_id.as_u64());
            w.f64(*base_level_b);
            w.f64(*fade_threshold);
        }
        EventDetail::LessonRecorded {
            lesson_id,
            pattern,
            failure_signature,
            failure_ids,
            contrastive_success_ids,
            domain,
        } => {
            w.u8(33);
            w.u64(lesson_id.as_u64());
            w.str(pattern);
            w.str(failure_signature);
            w.u32(failure_ids.len() as u32);
            for fid in failure_ids {
                w.u64(fid.as_u64());
            }
            w.u32(contrastive_success_ids.len() as u32);
            for cid in contrastive_success_ids {
                w.u64(cid.as_u64());
            }
            w.opt_str(domain.as_ref().map(DomainId::as_str));
        }
        EventDetail::ProcedureUpdated {
            procedure_id,
            name,
            steps,
            required_cap_ids,
            capability_schema_hash,
            domain,
            state_code,
            reason,
        } => {
            w.u8(34);
            w.u64(procedure_id.as_u64());
            w.str(name);
            w.u32(steps.len() as u32);
            for s in steps {
                w.str(s);
            }
            w.u32(required_cap_ids.len() as u32);
            for c in required_cap_ids {
                w.str(c.as_str());
            }
            w.str(capability_schema_hash.as_str());
            w.opt_str(domain.as_ref().map(DomainId::as_str));
            w.u8(*state_code);
            w.opt_str(reason.as_deref());
        }
        EventDetail::LearningUpdate {
            epoch_id,
            source_event_id,
            outcome_code,
            delta_w,
            delta_theta_0,
            ips_weight,
            signed_error,
        } => {
            w.u8(35);
            w.u64(*epoch_id);
            w.u64(source_event_id.as_u64());
            w.u8(*outcome_code);
            for d in delta_w {
                w.f64(*d);
            }
            w.f64(*delta_theta_0);
            w.f64(*ips_weight);
            w.f64(*signed_error);
        }
        EventDetail::WeightRollback {
            restored_epoch_id,
            reason,
        } => {
            w.u8(36);
            w.u64(*restored_epoch_id);
            w.str(reason);
        }
        EventDetail::SelfCardRecalibrated {
            degraded_capabilities,
            calibration_samples,
            budget_headroom_ratio,
            contradiction_flags,
            reason,
        } => {
            w.u8(37);
            w.u32(degraded_capabilities.len() as u32);
            for c in degraded_capabilities {
                w.str(c.as_str());
            }
            w.u64(*calibration_samples);
            w.f64(*budget_headroom_ratio);
            w.u64(*contradiction_flags);
            w.str(reason);
        }
        EventDetail::CalibrationRecorded { conf_0_20, correct } => {
            w.u8(38);
            w.u8(*conf_0_20);
            w.u8(u8::from(*correct));
        }
        EventDetail::BudgetUpdated { used, limit } => {
            w.u8(39);
            w.f64(*used);
            w.f64(*limit);
        }
    }
}

fn decode_opt_detail(r: &mut BinReader<'_>) -> Result<Option<EventDetail>, EventFormatError> {
    let tag = r.u8()?;
    match tag {
        0 => Ok(None),
        1 => {
            let target_event_id = EventId::new(r.u64()?);
            let outcome_code = r.u8()?;
            let outcome = DecisionOutcome::from_code(outcome_code).ok_or_else(|| {
                EventFormatError::MalformedRecord(format!(
                    "invalid decision outcome code {outcome_code}"
                ))
            })?;
            Ok(Some(EventDetail::Decision {
                target_event_id,
                outcome,
                salience: r.f64()?,
                threshold: r.f64()?,
                probability: r.f64()?,
                draw: r.f64()?,
                reason: None,
                trace: None,
            }))
        }
        2 => Ok(Some(EventDetail::Withheld {
            target_event_id: EventId::new(r.u64()?),
            raw_confidence_0_20: r.u8()?,
            calibrated_confidence: r.f64()?,
            report_threshold: r.f64()?,
            reason: r.string()?,
        })),
        3 => Ok(Some(EventDetail::Action {
            action_id: ActionId::new(r.string()?),
            capability_id: CapabilityId::new(r.string()?),
            h_act: r.opt_string()?.map(HAct::new),
            correlation_id: r.opt_string()?.map(CorrelationId::new),
        })),
        4 => Ok(Some(EventDetail::ActionFeedback {
            action_id: ActionId::new(r.string()?),
            correlation_id: r.opt_string()?.map(CorrelationId::new),
            progress: r.opt_f32()?,
            delivered_units: r.opt_u64()?,
            note: r.opt_string()?,
        })),
        5 => {
            let action_id = ActionId::new(r.string()?);
            let capability_id = CapabilityId::new(r.string()?);
            let outcome = decode_outcome(r.u8()?)?;
            let termination_reason = match r.u8()? {
                0 => None,
                1 => Some(decode_term_reason(r.u8()?)?),
                other => {
                    return Err(EventFormatError::MalformedRecord(format!(
                        "invalid opt termination_reason tag {other}"
                    )));
                }
            };
            let progress = r.opt_f32()?;
            let delivered_units = r.opt_u64()?;
            let h_act = r.opt_string()?.map(HAct::new);
            Ok(Some(EventDetail::Receipt {
                action_id,
                capability_id,
                outcome,
                termination_reason,
                progress,
                delivered_units,
                h_act,
            }))
        }
        6 => {
            let expectation_id = EventId::new(r.u64()?);
            let action_id = r.opt_string()?.map(ActionId::new);
            let h_act = r.opt_string()?.map(HAct::new);
            let from_state = match r.u8()? {
                0 => None,
                1 => {
                    let c = r.u8()?;
                    Some(ExpectationLifecycleState::from_code(c).ok_or_else(|| {
                        EventFormatError::MalformedRecord(format!(
                            "invalid expectation state code {c}"
                        ))
                    })?)
                }
                other => {
                    return Err(EventFormatError::MalformedRecord(format!(
                        "invalid opt from_state tag {other}"
                    )));
                }
            };
            let to_code = r.u8()?;
            let to_state = ExpectationLifecycleState::from_code(to_code).ok_or_else(|| {
                EventFormatError::MalformedRecord(format!(
                    "invalid expectation state code {to_code}"
                ))
            })?;
            Ok(Some(EventDetail::ExpectationTransition {
                expectation_id,
                action_id,
                h_act,
                from_state,
                to_state,
            }))
        }
        7 => Ok(Some(EventDetail::Verdict {
            target_event_id: EventId::new(r.u64()?),
            label: r.string()?,
        })),
        8 => {
            let door_code = r.u8()?;
            let door = DoorKind::from_code(door_code).ok_or_else(|| {
                EventFormatError::MalformedRecord(format!("invalid door kind code {door_code}"))
            })?;
            let target_event_id = r.opt_u64()?.map(EventId::new);
            let rule = r.string()?;
            let verdict = r.string()?;
            Ok(Some(EventDetail::DoorVerdict {
                door,
                target_event_id,
                rule,
                verdict,
            }))
        }
        9 => Ok(Some(EventDetail::Failure {
            target_event_id: r.opt_u64()?.map(EventId::new),
            component: r.string()?,
            detail: r.string()?,
        })),
        10 => {
            let id_len = r.u32()? as usize;
            let mut target_event_ids = Vec::with_capacity(id_len.min(1024));
            for _ in 0..id_len {
                target_event_ids.push(EventId::new(r.u64()?));
            }
            let actor_len = r.u32()? as usize;
            let mut target_actor_ids = Vec::with_capacity(actor_len.min(1024));
            for _ in 0..actor_len {
                target_actor_ids.push(ActorId::new(r.string()?));
            }
            let epoch = r.u64()?;
            let reason = r.string()?;
            Ok(Some(EventDetail::HardForget {
                target_event_ids,
                target_actor_ids,
                epoch,
                reason,
            }))
        }
        11 => {
            let target_event_id = EventId::new(r.u64()?);
            let outcome_code = r.u8()?;
            let outcome = DecisionOutcome::from_code(outcome_code).ok_or_else(|| {
                EventFormatError::MalformedRecord(format!(
                    "invalid decision outcome code {outcome_code}"
                ))
            })?;
            let salience = r.f64()?;
            let threshold = r.f64()?;
            let probability = r.f64()?;
            let draw = r.f64()?;
            let reason = match r.u8()? {
                0 => None,
                1 => {
                    let c = r.u8()?;
                    Some(DecisionReason::from_code(c).ok_or_else(|| {
                        EventFormatError::MalformedRecord(format!(
                            "invalid decision reason code {c}"
                        ))
                    })?)
                }
                tag => {
                    return Err(EventFormatError::MalformedRecord(format!(
                        "invalid opt decision reason tag {tag}"
                    )));
                }
            };
            let trace = match r.u8()? {
                0 => None,
                1 => Some(Box::new(decode_decision_trace(r)?)),
                tag => {
                    return Err(EventFormatError::MalformedRecord(format!(
                        "invalid opt decision trace tag {tag}"
                    )));
                }
            };
            Ok(Some(EventDetail::Decision {
                target_event_id,
                outcome,
                salience,
                threshold,
                probability,
                draw,
                reason,
                trace,
            }))
        }
        12 | 13 => {
            let addr_len = r.u32()? as usize;
            let mut addressees = Vec::with_capacity(addr_len.min(256));
            for _ in 0..addr_len {
                addressees.push(ActorId::new(r.string()?));
            }
            let named_len = r.u32()? as usize;
            let mut named = Vec::with_capacity(named_len.min(256));
            for _ in 0..named_len {
                named.push(ActorId::new(r.string()?));
            }
            let reply_to = r.opt_u64()?.map(EventId::new);
            let correlation_id = r.opt_string()?.map(CorrelationId::new);
            let emb_len = r.u32()? as usize;
            let mut embeddings = Vec::with_capacity(emb_len.min(64));
            for _ in 0..emb_len {
                let embed_model_ver = r.string()?;
                let dim_d = r.u32()?;
                let prompt_regime_id = r.string()?;
                let val_len = r.u32()? as usize;
                let mut values = Vec::with_capacity(val_len.min(4096));
                for _ in 0..val_len {
                    values.push(r.f32()?);
                }
                embeddings.push(TaggedEmbedding {
                    tag: EmbeddingTag {
                        embed_model_ver,
                        dim_d,
                        prompt_regime_id,
                    },
                    values,
                });
            }
            let is_dyadic_chat = r.u8()? != 0;
            let answered = r.f64()?;
            let channel = if tag == 13 {
                r.opt_string()?.map(ChannelId::new)
            } else {
                None
            };
            Ok(Some(EventDetail::PerceptMeta {
                addressees,
                named,
                reply_to,
                correlation_id,
                embeddings,
                is_dyadic_chat,
                answered,
                channel,
            }))
        }
        14 => {
            let expectation_id = EventId::new(r.u64()?);
            let target_event_id = EventId::new(r.u64()?);
            let kind = decode_expect_kind(r.u8()?)?;
            let place = PlaceId::new(r.string()?);
            let sender = ActorId::new(r.string()?);
            let medium = ChannelId::new(r.string()?);
            let due_ms = Time::from_millis(r.u64()?);
            let expiry_ms = Time::from_millis(r.u64()?);
            Ok(Some(EventDetail::ExpectationOpened {
                expectation_id,
                target_event_id,
                kind,
                place,
                sender,
                medium,
                due_ms,
                expiry_ms,
            }))
        }
        15 => {
            let len = r.u32()? as usize;
            let mut used_ids = Vec::with_capacity(len.min(1024));
            for _ in 0..len {
                used_ids.push(EventId::new(r.u64()?));
            }
            Ok(Some(EventDetail::MemoryUsed { used_ids }))
        }
        16 => {
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
            let aud_len = r.u32()? as usize;
            let mut members = Vec::with_capacity(aud_len.min(1024));
            for _ in 0..aud_len {
                members.push(ActorId::new(r.string()?));
            }
            let inner_sig = r.string()?;
            Ok(Some(EventDetail::DeclassGranted {
                signed_token: OwnerSigned {
                    payload: DeclassToken {
                        token_id,
                        issuer,
                        scope,
                        expiry,
                        target_place,
                        target_audience: Audience::new(members),
                        signature: inner_sig,
                    },
                    signer,
                    signed_at_ms,
                    signature: outer_sig,
                },
            }))
        }
        17 => {
            let expectation_id = EventId::new(r.u64()?);
            let origin_event_id = EventId::new(r.u64()?);
            let target_event_id = EventId::new(r.u64()?);
            let kind = decode_expect_kind(r.u8()?)?;
            let places_len = r.u32()? as usize;
            let mut places = Vec::with_capacity(places_len.min(128));
            for _ in 0..places_len {
                places.push(PlaceId::new(r.string()?));
            }
            let people_len = r.u32()? as usize;
            let mut people = Vec::with_capacity(people_len.min(128));
            for _ in 0..people_len {
                people.push(ActorId::new(r.string()?));
            }
            let domain = r.opt_string()?.map(DomainId::new);
            let condition = r.string()?;
            let condition_embeddings = decode_tagged_embeddings_bin(r)?;
            let due_ms = Time::from_millis(r.u64()?);
            let expiry_ms = Time::from_millis(r.u64()?);
            let recurrence = decode_opt_recurrence_spec(r)?;
            let promise_meta = decode_opt_promise_meta(r)?;
            let urgency_weight = r.f64()?;
            let action_id = r.opt_string()?.map(ActionId::new);
            let capability_id = r.opt_string()?.map(CapabilityId::new);
            let channel = r.opt_string()?.map(ChannelId::new);
            let lat_min_ms = r.u64()?;
            let lat_max_ms = r.u64()?;
            let delta_pred = r.opt_f64()?;
            let sigma_channel = r.opt_f64()?;
            let correlation_id = r.opt_string()?.map(CorrelationId::new);
            let declared_sensor = r.u8()? != 0;
            let act_embeddings = decode_tagged_embeddings_bin(r)?;
            let expected_reply_embeddings = decode_tagged_embeddings_bin(r)?;
            let is_judged_silence = r.u8()? != 0;
            let silence_propensity = r.opt_f64()?;
            let silence_sender = r.opt_string()?.map(ActorId::new);
            let stakes = r.f64()?;
            let medium = r.opt_string()?.map(ChannelId::new);
            let about_len = r.u32()? as usize;
            let mut about_ids = Vec::with_capacity(about_len.min(256));
            for _ in 0..about_len {
                about_ids.push(EventId::new(r.u64()?));
            }
            let accept_done_receipt = r.u8()? != 0;
            let expected_latency_ms = r.u64()?;
            Ok(Some(EventDetail::ExpectationRecordOpened {
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
            }))
        }
        18 => {
            let expectation_id = EventId::new(r.u64()?);
            let wake_event_id = EventId::new(r.u64()?);
            let new_version = r.u32()?;
            let condition = r.opt_string()?;
            let due_ms = r.opt_u64()?.map(Time::from_millis);
            let expiry_ms = r.opt_u64()?.map(Time::from_millis);
            let places_len = r.u32()? as usize;
            let mut places = Vec::with_capacity(places_len.min(128));
            for _ in 0..places_len {
                places.push(PlaceId::new(r.string()?));
            }
            let people_len = r.u32()? as usize;
            let mut people = Vec::with_capacity(people_len.min(128));
            for _ in 0..people_len {
                people.push(ActorId::new(r.string()?));
            }
            let domain = r.opt_string()?.map(DomainId::new);
            Ok(Some(EventDetail::ExpectationRevised {
                expectation_id,
                wake_event_id,
                new_version,
                condition,
                due_ms,
                expiry_ms,
                places,
                people,
                domain,
            }))
        }
        19 => {
            let body_id = BodyId::new(r.string()?);
            let status = decode_body_status_ev(r.u8()?)?;
            let cap_len = r.u32()? as usize;
            let mut capabilities = Vec::with_capacity(cap_len.min(128));
            for _ in 0..cap_len {
                let id = CapabilityId::new(r.string()?);
                let description = r.string()?;
                let p_len = r.u32()? as usize;
                let mut places = Vec::with_capacity(p_len.min(128));
                for _ in 0..p_len {
                    places.push(PlaceId::new(r.string()?));
                }
                let communicative = r.u8()? != 0;
                let identity_changing = r.u8()? != 0;
                let irreversible = r.u8()? != 0;
                let typical_latency_ms = r.u64()?;
                let capability_schema_hash = CapabilitySchemaHash::new(r.string()?);
                let sensed_effect = decode_opt_sensed_effect(r)?;
                capabilities.push(CapabilityDescriptor {
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
                });
            }
            Ok(Some(EventDetail::BodyCapabilities {
                body_id,
                status,
                capabilities,
            }))
        }
        20 => Ok(Some(EventDetail::ConditionVerdict {
            expectation_id: EventId::new(r.u64()?),
            target_event_id: EventId::new(r.u64()?),
            similarity: r.f64()?,
            met: r.u8()? != 0,
            label: r.string()?,
        })),
        21 => Ok(Some(EventDetail::ContentMiss {
            expectation_id: EventId::new(r.u64()?),
            reply_event_id: EventId::new(r.u64()?),
            who: ActorId::new(r.string()?),
            timing_delta_ms: r.i64()?,
            distance: r.f64()?,
            usual_distance: r.f64()?,
            z_score: r.f64()?,
        })),
        22 => Ok(Some(EventDetail::SilenceJudged {
            expectation_id: EventId::new(r.u64()?),
            target_event_id: EventId::new(r.u64()?),
            missed: r.u8()? != 0,
            reason: r.string()?,
            propensity: r.f64()?,
        })),
        23 => {
            let proposed_expectation_id = r.opt_u64()?.map(EventId::new);
            let conflicting_expectation_id = EventId::new(r.u64()?);
            let conflict_key = r.opt_string()?;
            let len = r.u32()? as usize;
            let mut overlapping_places = Vec::with_capacity(len.min(128));
            for _ in 0..len {
                overlapping_places.push(PlaceId::new(r.string()?));
            }
            Ok(Some(EventDetail::ConflictFlagged {
                proposed_expectation_id,
                conflicting_expectation_id,
                conflict_key,
                overlapping_places,
            }))
        }
        24 => Ok(Some(EventDetail::ExpectationRetryScheduled {
            expectation_id: EventId::new(r.u64()?),
            action_id: ActionId::new(r.string()?),
            retry_count: r.u32()?,
            next_retry_at_ms: Time::from_millis(r.u64()?),
        })),
        25 => Ok(Some(EventDetail::MonitorFired {
            expectation_id: EventId::new(r.u64()?),
            fired_at_ms: Time::from_millis(r.u64()?),
            next_due_ms: Time::from_millis(r.u64()?),
            coalesced: r.u8()? != 0,
            skipped_slots: r.u64()?,
        })),
        26 => Ok(Some(EventDetail::ExpectationLinked {
            expectation_id: EventId::new(r.u64()?),
            linked_event_id: EventId::new(r.u64()?),
            action_id: r.opt_string()?.map(ActionId::new),
        })),
        27 => {
            let belief_id = EventId::new(r.u64()?);
            let claim_key = r.string()?;
            let req_code = r.u8()?;
            let requested_basis = MemoryBasis::from_code(req_code).ok_or_else(|| {
                EventFormatError::MalformedRecord(format!("invalid requested_basis {req_code}"))
            })?;
            let bas_code = r.u8()?;
            let basis = MemoryBasis::from_code(bas_code).ok_or_else(|| {
                EventFormatError::MalformedRecord(format!("invalid basis {bas_code}"))
            })?;
            let basis_downgraded = r.u8()? != 0;
            let source_actor = r.opt_string()?.map(ActorId::new);
            let domain = DomainId::new(r.string()?);
            let src_len = r.u32()? as usize;
            let mut source_event_ids = Vec::with_capacity(src_len.min(1024));
            for _ in 0..src_len {
                source_event_ids.push(EventId::new(r.u64()?));
            }
            let taint = decode_taint(r.u8()?)?;
            let valid_from_ms = Time::from_millis(r.u64()?);
            let valid_to_ms = r.opt_u64()?.map(Time::from_millis);
            let replaced_by = r.opt_u64()?.map(EventId::new);
            let alpha_0 = r.f64()?;
            let beta_0 = r.f64()?;
            let is_mind_inference = r.u8()? != 0;
            Ok(Some(EventDetail::BeliefChanged {
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
            }))
        }
        28 => {
            let target_id = EventId::new(r.u64()?);
            let source_actor = r.opt_string()?.map(ActorId::new);
            let domain = DomainId::new(r.string()?);
            let positive = r.f64()?;
            let negative = r.f64()?;
            let is_convention = r.u8()? != 0;
            let place = r.opt_string()?.map(PlaceId::new);
            let pattern_id = r.opt_string()?;
            let c_len = r.u32()? as usize;
            let mut cited_ids = Vec::with_capacity(c_len.min(1024));
            for _ in 0..c_len {
                cited_ids.push(EventId::new(r.u64()?));
            }
            Ok(Some(EventDetail::Evidence {
                target_id,
                source_actor,
                domain,
                positive,
                negative,
                is_convention,
                place,
                pattern_id,
                cited_ids,
            }))
        }
        29 => {
            let episode_id = EventId::new(r.u64()?);
            let place = PlaceId::new(r.string()?);
            let state_label = r.string()?;
            let ev_len = r.u32()? as usize;
            let mut event_ids = Vec::with_capacity(ev_len.min(2048));
            for _ in 0..ev_len {
                event_ids.push(EventId::new(r.u64()?));
            }
            let gist_id = r.opt_u64()?.map(EventId::new);
            let superseded_gist_id = r.opt_u64()?.map(EventId::new);
            let m_len = r.u32()? as usize;
            let mut merged_from = Vec::with_capacity(m_len.min(256));
            for _ in 0..m_len {
                merged_from.push(EventId::new(r.u64()?));
            }
            Ok(Some(EventDetail::EpisodeClosed {
                episode_id,
                place,
                state_label,
                event_ids,
                gist_id,
                superseded_gist_id,
                merged_from,
            }))
        }
        30 => {
            let snapshot_ms = Time::from_millis(r.u64()?);
            let snapshot_seq = LogSeq::new(r.u64()?);
            let c_len = r.u32()? as usize;
            let mut covered_event_ids = Vec::with_capacity(c_len.min(4096));
            for _ in 0..c_len {
                covered_event_ids.push(EventId::new(r.u64()?));
            }
            let p_len = r.u32()? as usize;
            let mut produced_ids = Vec::with_capacity(p_len.min(1024));
            for _ in 0..p_len {
                produced_ids.push(EventId::new(r.u64()?));
            }
            let tokens_used = r.u64()?;
            let completed = r.u8()? != 0;
            Ok(Some(EventDetail::SleepRun {
                snapshot_ms,
                snapshot_seq,
                covered_event_ids,
                produced_ids,
                tokens_used,
                completed,
            }))
        }
        31 => {
            let cent_len = r.u32()? as usize;
            let mut centroids = Vec::with_capacity(cent_len.min(128));
            for _ in 0..cent_len {
                let did = DomainId::new(r.string()?);
                let emb = decode_tagged_embedding(r)?;
                centroids.push((did, emb));
            }
            let tr_len = r.u32()? as usize;
            let mut transitions = Vec::with_capacity(tr_len.min(256));
            for _ in 0..tr_len {
                let kcode = r.u8()?;
                let kind = DomainTransitionKind::from_code(kcode).ok_or_else(|| {
                    EventFormatError::MalformedRecord(format!("invalid domain transition {kcode}"))
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
                let actor = ActorId::new(r.string()?);
                let from_domain = DomainId::new(r.string()?);
                let to_domain = DomainId::new(r.string()?);
                let new_alpha = r.f64()?;
                let new_beta = r.f64()?;
                let alpha_0 = r.f64()?;
                let beta_0 = r.f64()?;
                credence_rebased.push(CredenceRebaseRecord {
                    actor,
                    from_domain,
                    to_domain,
                    new_alpha,
                    new_beta,
                    alpha_0,
                    beta_0,
                });
            }
            Ok(Some(EventDetail::DomainsUpdated {
                centroids,
                transitions,
                credence_rebased,
            }))
        }
        32 => Ok(Some(EventDetail::VectorReleased {
            event_id: EventId::new(r.u64()?),
            base_level_b: r.f64()?,
            fade_threshold: r.f64()?,
        })),
        33 => {
            let lesson_id = EventId::new(r.u64()?);
            let pattern = r.string()?;
            let failure_signature = r.string()?;
            let f_len = r.u32()? as usize;
            let mut failure_ids = Vec::with_capacity(f_len.min(1024));
            for _ in 0..f_len {
                failure_ids.push(EventId::new(r.u64()?));
            }
            let c_len = r.u32()? as usize;
            let mut contrastive_success_ids = Vec::with_capacity(c_len.min(1024));
            for _ in 0..c_len {
                contrastive_success_ids.push(EventId::new(r.u64()?));
            }
            let domain = r.opt_string()?.map(DomainId::new);
            Ok(Some(EventDetail::LessonRecorded {
                lesson_id,
                pattern,
                failure_signature,
                failure_ids,
                contrastive_success_ids,
                domain,
            }))
        }
        34 => {
            let procedure_id = EventId::new(r.u64()?);
            let name = r.string()?;
            let steps_len = r.u32()? as usize;
            let mut steps = Vec::with_capacity(steps_len);
            for _ in 0..steps_len {
                steps.push(r.string()?);
            }
            let cap_len = r.u32()? as usize;
            let mut required_cap_ids = Vec::with_capacity(cap_len);
            for _ in 0..cap_len {
                required_cap_ids.push(CapabilityId::new(r.string()?));
            }
            let capability_schema_hash = CapabilitySchemaHash::new(r.string()?);
            let domain = r.opt_string()?.map(DomainId::new);
            let state_code = r.u8()?;
            let reason = r.opt_string()?;
            Ok(Some(EventDetail::ProcedureUpdated {
                procedure_id,
                name,
                steps,
                required_cap_ids,
                capability_schema_hash,
                domain,
                state_code,
                reason,
            }))
        }
        35 => {
            let epoch_id = r.u64()?;
            let source_event_id = EventId::new(r.u64()?);
            let outcome_code = r.u8()?;
            let mut delta_w = [0.0; 6];
            for slot in &mut delta_w {
                *slot = r.f64()?;
            }
            let delta_theta_0 = r.f64()?;
            let ips_weight = r.f64()?;
            let signed_error = r.f64()?;
            Ok(Some(EventDetail::LearningUpdate {
                epoch_id,
                source_event_id,
                outcome_code,
                delta_w,
                delta_theta_0,
                ips_weight,
                signed_error,
            }))
        }
        36 => Ok(Some(EventDetail::WeightRollback {
            restored_epoch_id: r.u64()?,
            reason: r.string()?,
        })),
        37 => {
            let cap_len = r.u32()? as usize;
            let mut degraded_capabilities = Vec::with_capacity(cap_len);
            for _ in 0..cap_len {
                degraded_capabilities.push(CapabilityId::new(r.string()?));
            }
            let calibration_samples = r.u64()?;
            let budget_headroom_ratio = r.f64()?;
            let contradiction_flags = r.u64()?;
            let reason = r.string()?;
            Ok(Some(EventDetail::SelfCardRecalibrated {
                degraded_capabilities,
                calibration_samples,
                budget_headroom_ratio,
                contradiction_flags,
                reason,
            }))
        }
        38 => Ok(Some(EventDetail::CalibrationRecorded {
            conf_0_20: r.u8()?,
            correct: r.u8()? != 0,
        })),
        39 => Ok(Some(EventDetail::BudgetUpdated {
            used: r.f64()?,
            limit: r.f64()?,
        })),
        other => Err(EventFormatError::MalformedRecord(format!(
            "invalid detail tag {other}"
        ))),
    }
}

pub(crate) fn encode_expect_kind(kind: ExpectKind) -> u8 {
    match kind {
        ExpectKind::Effect => 1,
        ExpectKind::Response => 2,
        ExpectKind::Wait => 3,
        ExpectKind::Promise => 4,
        ExpectKind::Goal => 5,
        ExpectKind::Monitor => 6,
        ExpectKind::PendingApproval => 7,
    }
}

pub(crate) fn decode_expect_kind(code: u8) -> Result<ExpectKind, EventFormatError> {
    match code {
        1 => Ok(ExpectKind::Effect),
        2 => Ok(ExpectKind::Response),
        3 => Ok(ExpectKind::Wait),
        4 => Ok(ExpectKind::Promise),
        5 => Ok(ExpectKind::Goal),
        6 => Ok(ExpectKind::Monitor),
        7 => Ok(ExpectKind::PendingApproval),
        other => Err(EventFormatError::MalformedRecord(format!(
            "invalid expect kind code {other}"
        ))),
    }
}

pub(crate) fn encode_tagged_embedding(w: &mut BinWriter, emb: &TaggedEmbedding) {
    w.str(&emb.tag.embed_model_ver);
    w.u32(emb.tag.dim_d);
    w.str(&emb.tag.prompt_regime_id);
    w.u32(emb.values.len() as u32);
    for &v in &emb.values {
        w.f32(v);
    }
}

pub(crate) fn decode_tagged_embedding(
    r: &mut BinReader<'_>,
) -> Result<TaggedEmbedding, EventFormatError> {
    let embed_model_ver = r.string()?;
    let dim_d = r.u32()?;
    let prompt_regime_id = r.string()?;
    let vlen = r.u32()? as usize;
    let mut values = Vec::with_capacity(vlen.min(4096));
    for _ in 0..vlen {
        values.push(r.f32()?);
    }
    Ok(TaggedEmbedding {
        tag: EmbeddingTag {
            embed_model_ver,
            dim_d,
            prompt_regime_id,
        },
        values,
    })
}

pub(crate) fn encode_tagged_embeddings_bin(w: &mut BinWriter, embs: &[TaggedEmbedding]) {
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

pub(crate) fn decode_tagged_embeddings_bin(
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

pub(crate) fn encode_opt_recurrence_spec(w: &mut BinWriter, spec: Option<&RecurrenceSpec>) {
    match spec {
        None => w.u8(0),
        Some(s) => {
            w.u8(1);
            match &s.schedule {
                RecurrenceSchedule::IntervalMs(ms) => {
                    w.u8(1);
                    w.u64(*ms);
                }
                RecurrenceSchedule::Cron(expr) => {
                    w.u8(2);
                    w.str(expr);
                }
            }
            match s.catchup {
                CatchupPolicy::CoalesceOne => w.u8(1),
                CatchupPolicy::SkipToNext => w.u8(2),
            }
        }
    }
}

pub(crate) fn decode_opt_recurrence_spec(
    r: &mut BinReader<'_>,
) -> Result<Option<RecurrenceSpec>, EventFormatError> {
    match r.u8()? {
        0 => Ok(None),
        1 => {
            let schedule = match r.u8()? {
                1 => RecurrenceSchedule::IntervalMs(r.u64()?),
                2 => RecurrenceSchedule::Cron(r.string()?),
                tag => {
                    return Err(EventFormatError::MalformedRecord(format!(
                        "invalid recurrence schedule tag {tag}"
                    )));
                }
            };
            let catchup = match r.u8()? {
                1 => CatchupPolicy::CoalesceOne,
                2 => CatchupPolicy::SkipToNext,
                tag => {
                    return Err(EventFormatError::MalformedRecord(format!(
                        "invalid catchup policy tag {tag}"
                    )));
                }
            };
            Ok(Some(RecurrenceSpec { schedule, catchup }))
        }
        tag => Err(EventFormatError::MalformedRecord(format!(
            "invalid opt recurrence tag {tag}"
        ))),
    }
}

pub(crate) fn encode_opt_promise_meta(w: &mut BinWriter, meta: Option<&PromiseMeta>) {
    match meta {
        None => w.u8(0),
        Some(m) => {
            w.u8(1);
            w.u32(m.promisee_ids.len() as u32);
            for id in &m.promisee_ids {
                w.str(id.as_str());
            }
            w.opt_u64(m.hard_deadline_ms.map(Time::as_millis));
            w.opt_str(m.conflict_key.as_deref());
            w.opt_u64(m.depends_on.map(EventId::as_u64));
        }
    }
}

pub(crate) fn decode_opt_promise_meta(
    r: &mut BinReader<'_>,
) -> Result<Option<PromiseMeta>, EventFormatError> {
    match r.u8()? {
        0 => Ok(None),
        1 => {
            let len = r.u32()? as usize;
            let mut promisee_ids = Vec::with_capacity(len.min(128));
            for _ in 0..len {
                promisee_ids.push(ActorId::new(r.string()?));
            }
            let hard_deadline_ms = r.opt_u64()?.map(Time::from_millis);
            let conflict_key = r.opt_string()?;
            let depends_on = r.opt_u64()?.map(EventId::new);
            Ok(Some(PromiseMeta {
                promisee_ids,
                hard_deadline_ms,
                conflict_key,
                depends_on,
            }))
        }
        tag => Err(EventFormatError::MalformedRecord(format!(
            "invalid opt promise_meta tag {tag}"
        ))),
    }
}

pub(crate) fn encode_opt_sensed_effect(w: &mut BinWriter, eff: Option<&SensedEffect>) {
    match eff {
        None => w.u8(0),
        Some(e) => {
            w.u8(1);
            w.str(e.channel.as_str());
            w.u64(e.window_ms);
            w.f64(e.noise);
            w.f64(e.delta_pred);
        }
    }
}

pub(crate) fn decode_opt_sensed_effect(
    r: &mut BinReader<'_>,
) -> Result<Option<SensedEffect>, EventFormatError> {
    match r.u8()? {
        0 => Ok(None),
        1 => Ok(Some(SensedEffect {
            channel: ChannelId::new(r.string()?),
            window_ms: r.u64()?,
            noise: r.f64()?,
            delta_pred: r.f64()?,
        })),
        tag => Err(EventFormatError::MalformedRecord(format!(
            "invalid opt sensed_effect tag {tag}"
        ))),
    }
}

fn encode_body_status_ev(status: BodyStatus) -> u8 {
    match status {
        BodyStatus::Ready => 1,
        BodyStatus::Busy => 2,
        BodyStatus::Degraded => 3,
        BodyStatus::Stopped => 4,
    }
}

fn decode_body_status_ev(code: u8) -> Result<BodyStatus, EventFormatError> {
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

fn encode_decision_trace(w: &mut BinWriter, tr: &DecisionTrace) {
    w.f64(tr.signals.own_name);
    w.f64(tr.signals.dialogue_act_gap);
    w.f64(tr.signals.asr_clarification_boost);
    w.f64(tr.signals.info_gap);
    w.f64(tr.signals.own_info_gap);
    w.f64(tr.signals.feeling_of_knowing);
    w.f64(tr.signals.relevance);
    w.f64(tr.signals.commitment);
    w.f64(tr.signals.deg_soul);
    w.f64(tr.signals.max_deg_other);
    w.f64(tr.signals.sum_deg_listeners);
    w.f64(tr.signals.answered);
    w.f64(tr.signals.closeness_sender);
    w.f64(tr.signals.social_cost);

    w.f64(tr.weights.w_own_name);
    w.f64(tr.weights.w_info);
    w.f64(tr.weights.w_fok);
    w.f64(tr.weights.w_rel);
    w.f64(tr.weights.w_commitment);
    w.f64(tr.weights.w_sc);

    w.f64(tr.raw_salience);
    w.f64(tr.h_pre);
    w.f64(tr.kappa);
    w.f64(tr.habituation_factor);
    w.f64(tr.effective_salience);

    w.f64(tr.threshold_terms.theta_0);
    w.f64(tr.threshold_terms.a);
    w.f64(tr.threshold_terms.talk_rate);
    w.f64(tr.threshold_terms.b);
    w.f64(tr.threshold_terms.budget_used);
    w.f64(tr.threshold_terms.c);
    w.f64(tr.threshold_terms.promise_debt);
    w.f64(tr.threshold_terms.r);
    w.f64(tr.threshold_terms.familiarity);
    w.f64(tr.threshold_terms.delta_theta_init);
    w.f64(tr.threshold_terms.delta_theta_sleep);
    w.f64(tr.threshold_terms.theta);

    w.f64(tr.tau);
    w.f64(tr.note_band);
    w.u8(u8::from(tr.stochastic_enabled));
    w.u8(u8::from(tr.commitment_bypass));
    w.f64(tr.probability);
    w.f64(tr.draw);
    w.u8(tr.outcome.code());
    w.u8(tr.reason.code());
}

fn decode_decision_trace(r: &mut BinReader<'_>) -> Result<DecisionTrace, EventFormatError> {
    let signals = GateSignals {
        own_name: r.f64()?,
        dialogue_act_gap: r.f64()?,
        asr_clarification_boost: r.f64()?,
        info_gap: r.f64()?,
        own_info_gap: r.f64()?,
        feeling_of_knowing: r.f64()?,
        relevance: r.f64()?,
        commitment: r.f64()?,
        deg_soul: r.f64()?,
        max_deg_other: r.f64()?,
        sum_deg_listeners: r.f64()?,
        answered: r.f64()?,
        closeness_sender: r.f64()?,
        social_cost: r.f64()?,
    };
    let weights = SalienceWeights {
        w_own_name: r.f64()?,
        w_info: r.f64()?,
        w_fok: r.f64()?,
        w_rel: r.f64()?,
        w_commitment: r.f64()?,
        w_sc: r.f64()?,
    };
    let raw_salience = r.f64()?;
    let h_pre = r.f64()?;
    let kappa = r.f64()?;
    let habituation_factor = r.f64()?;
    let effective_salience = r.f64()?;
    let threshold_terms = ThresholdTerms {
        theta_0: r.f64()?,
        a: r.f64()?,
        talk_rate: r.f64()?,
        b: r.f64()?,
        budget_used: r.f64()?,
        c: r.f64()?,
        promise_debt: r.f64()?,
        r: r.f64()?,
        familiarity: r.f64()?,
        delta_theta_init: r.f64()?,
        delta_theta_sleep: r.f64()?,
        theta: r.f64()?,
    };
    let tau = r.f64()?;
    let note_band = r.f64()?;
    let stochastic_enabled = r.u8()? != 0;
    let commitment_bypass = r.u8()? != 0;
    let probability = r.f64()?;
    let draw = r.f64()?;
    let out_code = r.u8()?;
    let outcome = DecisionOutcome::from_code(out_code).ok_or_else(|| {
        EventFormatError::MalformedRecord(format!("invalid trace outcome code {out_code}"))
    })?;
    let rsn_code = r.u8()?;
    let reason = DecisionReason::from_code(rsn_code).ok_or_else(|| {
        EventFormatError::MalformedRecord(format!("invalid trace reason code {rsn_code}"))
    })?;
    Ok(DecisionTrace {
        signals,
        weights,
        raw_salience,
        h_pre,
        kappa,
        habituation_factor,
        effective_salience,
        threshold_terms,
        tau,
        note_band,
        stochastic_enabled,
        commitment_bypass,
        probability,
        draw,
        outcome,
        reason,
    })
}

pub(crate) struct BinWriter {
    buf: Vec<u8>,
}

impl BinWriter {
    pub(crate) fn new() -> Self {
        Self {
            buf: Vec::with_capacity(128),
        }
    }

    pub(crate) fn raw(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }

    pub(crate) fn u8(&mut self, v: u8) {
        self.buf.push(v);
    }

    pub(crate) fn u16(&mut self, v: u16) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    pub(crate) fn u32(&mut self, v: u32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    pub(crate) fn u64(&mut self, v: u64) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    pub(crate) fn i64(&mut self, v: i64) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    pub(crate) fn f32(&mut self, v: f32) {
        self.buf.extend_from_slice(&v.to_bits().to_le_bytes());
    }

    pub(crate) fn f64(&mut self, v: f64) {
        self.buf.extend_from_slice(&v.to_bits().to_le_bytes());
    }

    pub(crate) fn bytes(&mut self, b: &[u8]) {
        self.u32(u32::try_from(b.len()).unwrap_or(u32::MAX));
        self.buf.extend_from_slice(b);
    }

    pub(crate) fn str(&mut self, s: &str) {
        self.bytes(s.as_bytes());
    }

    pub(crate) fn opt_str(&mut self, s: Option<&str>) {
        match s {
            Some(val) => {
                self.u8(1);
                self.str(val);
            }
            None => self.u8(0),
        }
    }

    pub(crate) fn opt_u64(&mut self, v: Option<u64>) {
        match v {
            Some(val) => {
                self.u8(1);
                self.u64(val);
            }
            None => self.u8(0),
        }
    }

    pub(crate) fn opt_f32(&mut self, v: Option<f32>) {
        match v {
            Some(val) => {
                self.u8(1);
                self.f32(val);
            }
            None => self.u8(0),
        }
    }

    pub(crate) fn opt_f64(&mut self, v: Option<f64>) {
        match v {
            Some(val) => {
                self.u8(1);
                self.f64(val);
            }
            None => self.u8(0),
        }
    }

    pub(crate) fn opt_bool(&mut self, v: Option<bool>) {
        match v {
            Some(true) => self.u8(2),
            Some(false) => self.u8(1),
            None => self.u8(0),
        }
    }

    pub(crate) fn finish(self) -> Vec<u8> {
        self.buf
    }
}

pub(crate) struct BinReader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> BinReader<'a> {
    pub(crate) fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    pub(crate) fn fixed<const N: usize>(&mut self) -> Result<[u8; N], EventFormatError> {
        if self.pos + N > self.buf.len() {
            return Err(EventFormatError::MalformedRecord(
                "unexpected EOF".to_owned(),
            ));
        }
        let mut out = [0u8; N];
        out.copy_from_slice(&self.buf[self.pos..self.pos + N]);
        self.pos += N;
        Ok(out)
    }

    pub(crate) fn u8(&mut self) -> Result<u8, EventFormatError> {
        Ok(self.fixed::<1>()?[0])
    }

    pub(crate) fn u16(&mut self) -> Result<u16, EventFormatError> {
        Ok(u16::from_le_bytes(self.fixed::<2>()?))
    }

    pub(crate) fn u32(&mut self) -> Result<u32, EventFormatError> {
        Ok(u32::from_le_bytes(self.fixed::<4>()?))
    }

    pub(crate) fn u64(&mut self) -> Result<u64, EventFormatError> {
        Ok(u64::from_le_bytes(self.fixed::<8>()?))
    }

    pub(crate) fn i64(&mut self) -> Result<i64, EventFormatError> {
        Ok(i64::from_le_bytes(self.fixed::<8>()?))
    }

    pub(crate) fn f32(&mut self) -> Result<f32, EventFormatError> {
        Ok(f32::from_bits(self.u32()?))
    }

    pub(crate) fn f64(&mut self) -> Result<f64, EventFormatError> {
        Ok(f64::from_bits(self.u64()?))
    }

    pub(crate) fn bytes(&mut self) -> Result<Vec<u8>, EventFormatError> {
        let len = self.u32()? as usize;
        if self.pos + len > self.buf.len() {
            return Err(EventFormatError::MalformedRecord(
                "length exceeds buffer".to_owned(),
            ));
        }
        let slice = self.buf[self.pos..self.pos + len].to_vec();
        self.pos += len;
        Ok(slice)
    }

    pub(crate) fn string(&mut self) -> Result<String, EventFormatError> {
        let raw = self.bytes()?;
        String::from_utf8(raw)
            .map_err(|e| EventFormatError::MalformedRecord(format!("invalid UTF-8: {e}")))
    }

    pub(crate) fn opt_string(&mut self) -> Result<Option<String>, EventFormatError> {
        match self.u8()? {
            0 => Ok(None),
            1 => Ok(Some(self.string()?)),
            tag => Err(EventFormatError::MalformedRecord(format!(
                "invalid opt string tag {tag}"
            ))),
        }
    }

    pub(crate) fn opt_u64(&mut self) -> Result<Option<u64>, EventFormatError> {
        match self.u8()? {
            0 => Ok(None),
            1 => Ok(Some(self.u64()?)),
            tag => Err(EventFormatError::MalformedRecord(format!(
                "invalid opt u64 tag {tag}"
            ))),
        }
    }

    pub(crate) fn opt_f32(&mut self) -> Result<Option<f32>, EventFormatError> {
        match self.u8()? {
            0 => Ok(None),
            1 => Ok(Some(self.f32()?)),
            tag => Err(EventFormatError::MalformedRecord(format!(
                "invalid opt f32 tag {tag}"
            ))),
        }
    }

    pub(crate) fn opt_f64(&mut self) -> Result<Option<f64>, EventFormatError> {
        match self.u8()? {
            0 => Ok(None),
            1 => Ok(Some(self.f64()?)),
            tag => Err(EventFormatError::MalformedRecord(format!(
                "invalid opt f64 tag {tag}"
            ))),
        }
    }

    pub(crate) fn opt_bool(&mut self) -> Result<Option<bool>, EventFormatError> {
        match self.u8()? {
            0 => Ok(None),
            1 => Ok(Some(false)),
            2 => Ok(Some(true)),
            tag => Err(EventFormatError::MalformedRecord(format!(
                "invalid opt bool tag {tag}"
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DecisionOutcome, EventDetail, EventFormatError, EventKind, EventRecord, EventRecordV1,
        ExpectationLifecycleState, LogSeq, NumericFeatures,
    };
    use monoagens_protocol::{
        ActionId, ActorId, AttachmentHandle, Audience, CapabilityId, EmbeddingTag, EventId, FModal,
        HAct, ModalitySlot, OpaquePayload, OrientScore, OriginActor, PlaceId, TaggedEmbedding,
        Taint, Time,
    };

    /// Exhaustive match over all 29 canonical event kinds (fails compilation if any variant is unhandled).
    fn kind_index(kind: EventKind) -> usize {
        match kind {
            EventKind::Percept => 0,
            EventKind::Decision => 1,
            EventKind::WakeStarted => 2,
            EventKind::ModelCall => 3,
            EventKind::Thought => 4,
            EventKind::Withheld => 5,
            EventKind::Action => 6,
            EventKind::ActionFeedback => 7,
            EventKind::Receipt => 8,
            EventKind::ExpectationOpened => 9,
            EventKind::ExpectationChanged => 10,
            EventKind::Verdict => 11,
            EventKind::MemoryUsed => 12,
            EventKind::BeliefChanged => 13,
            EventKind::Evidence => 14,
            EventKind::EpisodeClosed => 15,
            EventKind::SleepRun => 16,
            EventKind::DomainsUpdated => 17,
            EventKind::VectorReleased => 18,
            EventKind::TimingUpdated => 19,
            EventKind::LearningChange => 20,
            EventKind::Rollback => 21,
            EventKind::DoorVerdict => 22,
            EventKind::IdentityChanged => 23,
            EventKind::BodyState => 24,
            EventKind::ActorFacts => 25,
            EventKind::Tick => 26,
            EventKind::Failure => 27,
            EventKind::HardForget => 28,
        }
    }

    #[test]
    fn all_29_canonical_event_kinds_round_trip() {
        assert_eq!(EventKind::ALL.len(), 29);
        for (i, &kind) in EventKind::ALL.iter().enumerate() {
            assert_eq!(kind_index(kind), i);
            let mut rec = EventRecord::new_root(
                kind,
                Time::from_millis(100 + i as u64),
                ActorId::new("per_alice"),
                PlaceId::new("room:1"),
                Audience::new([ActorId::new("per_alice")]),
                OriginActor::new(ActorId::new("per_alice")),
                Taint::Trusted,
                NumericFeatures {
                    f_modal: Some(FModal([0.1; 10])),
                    orient: vec![OrientScore {
                        actor: ActorId::new("soul"),
                        score: 0.9,
                    }],
                    deg_soul: Some(0.8),
                    dialogue_act_gap: Some(0.4),
                    delta_obs: Some(0.25),
                    length: Some(5),
                    negative_flag: Some(false),
                    repair: Some(false),
                    continued_attention: Some(true),
                    slots: vec![1.0, 2.5],
                },
                OpaquePayload::from_bytes(b"opaque".as_slice()),
                vec![AttachmentHandle {
                    handle_id: "att_1".to_owned(),
                    modality_slot: ModalitySlot::Image,
                    byte_len: 128,
                    expires_at_ms: Time::from_millis(9_999),
                    embed_model_ver: "embeddinggemma-2".to_owned(),
                }],
                None,
            );
            rec.id = EventId::new(i as u64 + 1);
            rec.seq = LogSeq::new(i as u64 + 1);
            let bytes = rec.encode_bytes();
            let decoded = EventRecord::decode_and_upcast(&bytes).expect("decode v2");
            assert_eq!(decoded, rec);
        }
    }

    #[test]
    fn v1_upcasts_in_memory_and_future_version_names_format_and_versions() {
        let v1 = EventRecordV1 {
            id: EventId::new(7),
            kind: EventKind::Percept,
            format_version: 1,
            timestamp_ms: Time::from_millis(42_000),
            actor: ActorId::new("per_bob"),
            place: PlaceId::new("cli:0"),
            audience: Audience::new([ActorId::new("per_bob")]),
            origin_actor: OriginActor::new(ActorId::new("per_bob")),
            taint: Taint::Provisional,
            caused_by: Some(EventId::new(3)),
            payload: OpaquePayload::from_bytes(b"legacy".as_slice()),
        };
        let raw_v1 = EventRecord::encode_v1_bytes(&v1);
        let raw_v1_copy = raw_v1.clone();
        let upcasted = EventRecord::decode_and_upcast(&raw_v1).expect("upcast v1");
        assert_eq!(raw_v1, raw_v1_copy);
        assert_eq!(
            upcasted.format_version,
            EventKind::Percept.current_version()
        );
        assert_eq!(upcasted.caused_by, vec![EventId::new(3)]);
        assert_eq!(upcasted.origin_actor, v1.origin_actor);

        let future =
            EventRecord::encode_with_version_for_test(EventKind::Receipt, 99, EventId::new(8));
        let err = EventRecord::decode_and_upcast(&future).expect_err("must reject future version");
        assert!(matches!(
            err,
            EventFormatError::UnsupportedFutureVersion { .. }
        ));
        let msg = err.to_string();
        assert!(msg.contains("receipt"), "missing format name: {msg}");
        assert!(msg.contains("99"), "missing stored version: {msg}");
        assert!(
            msg.contains(&EventKind::Receipt.current_version().to_string()),
            "missing supported version: {msg}"
        );
    }

    #[test]
    fn origin_and_min_trust_taint_propagate_monotonically_from_causing_event() {
        let mut root = EventRecord::new_root(
            EventKind::Percept,
            Time::from_millis(10),
            ActorId::new("per_alice"),
            PlaceId::new("chat:1"),
            Audience::new([ActorId::new("per_alice")]),
            OriginActor::new(ActorId::new("per_alice")),
            Taint::Trusted,
            NumericFeatures::default(),
            OpaquePayload::default(),
            Vec::new(),
            None,
        );
        root.id = EventId::new(1);

        let mut thought = EventRecord::derived_from(
            &root,
            EventKind::Thought,
            Time::from_millis(20),
            ActorId::new("soul"),
            PlaceId::new("chat:1"),
            Audience::new([ActorId::new("per_alice")]),
            Some(Taint::Provisional),
            NumericFeatures::default(),
            OpaquePayload::default(),
            Vec::new(),
            None,
        );
        thought.id = EventId::new(2);
        assert_eq!(thought.origin_actor, root.origin_actor);
        assert_eq!(thought.taint, Taint::Provisional);
        assert_eq!(thought.caused_by, vec![EventId::new(1)]);

        // Attempting to pass Trusted extra_taint cannot raise Provisional back to Trusted.
        let action = EventRecord::derived_from(
            &thought,
            EventKind::Action,
            Time::from_millis(30),
            ActorId::new("soul"),
            PlaceId::new("chat:1"),
            Audience::new([ActorId::new("per_alice")]),
            Some(Taint::Trusted),
            NumericFeatures::default(),
            OpaquePayload::default(),
            Vec::new(),
            Some(EventDetail::Action {
                action_id: ActionId::new("act-1"),
                capability_id: CapabilityId::new("chat.speak"),
                h_act: Some(HAct::new("blake3:aa")),
                correlation_id: None,
            }),
        );
        assert_eq!(action.origin_actor, root.origin_actor);
        assert_eq!(action.taint, Taint::Provisional);
        assert_eq!(action.caused_by, vec![EventId::new(2), EventId::new(1)]);

        // Check special event details round-trip cleanly.
        let details = [
            EventDetail::Decision {
                target_event_id: EventId::new(1),
                outcome: DecisionOutcome::Silent,
                salience: 0.2,
                threshold: 0.7,
                probability: 0.1,
                draw: 0.5,
                reason: None,
                trace: None,
            },
            EventDetail::PerceptMeta {
                addressees: vec![ActorId::new("soul")],
                named: vec![ActorId::new("soul")],
                reply_to: Some(EventId::new(1)),
                correlation_id: None,
                embeddings: vec![TaggedEmbedding {
                    tag: EmbeddingTag {
                        embed_model_ver: "gemma2-256".to_owned(),
                        dim_d: 2,
                        prompt_regime_id: "t_clf".to_owned(),
                    },
                    values: vec![0.6, 0.8],
                }],
                is_dyadic_chat: true,
                answered: 0.0,
                channel: None,
            },
            EventDetail::Withheld {
                target_event_id: EventId::new(2),
                raw_confidence_0_20: 8,
                calibrated_confidence: 0.4,
                report_threshold: 0.6,
                reason: "below certainty bar".to_owned(),
            },
            EventDetail::ActionFeedback {
                action_id: ActionId::new("act-1"),
                correlation_id: None,
                progress: Some(0.5),
                delivered_units: Some(2),
                note: Some("streaming".to_owned()),
            },
            EventDetail::ExpectationTransition {
                expectation_id: EventId::new(5),
                action_id: Some(ActionId::new("act-1")),
                h_act: Some(HAct::new("blake3:aa")),
                from_state: Some(ExpectationLifecycleState::PendingApproval),
                to_state: ExpectationLifecycleState::Approved,
            },
            EventDetail::HardForget {
                target_event_ids: vec![EventId::new(1)],
                target_actor_ids: vec![ActorId::new("per_alice")],
                epoch: 1,
                reason: "erasure".to_owned(),
            },
        ];
        for detail in details {
            let rec = EventRecord::derived_from(
                &thought,
                EventKind::ExpectationChanged,
                Time::from_millis(40),
                ActorId::new("soul"),
                PlaceId::new("chat:1"),
                Audience::new([ActorId::new("per_alice")]),
                None,
                NumericFeatures::default(),
                OpaquePayload::default(),
                Vec::new(),
                Some(detail),
            );
            let encoded = rec.encode_bytes();
            let decoded = EventRecord::decode_and_upcast(&encoded).expect("detail round-trip");
            assert_eq!(decoded, rec);
        }
    }
}
