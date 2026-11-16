//! Pure `(log_seq, t_now)` snapshot state evaluated deterministically by `Valve`.

use std::collections::{BTreeMap, BTreeSet};

use monoagens_core::LogSeq;
use monoagens_protocol::{
    ActionId, ActorId, Audience, CapabilityDescriptor, CapabilityId, DeclassToken, EventId,
    OriginActor, OwnerSigned, PendingApproval, PlaceId, Taint, Time,
};
use serde::{Deserialize, Serialize};

use crate::approvals::ActionRiskMetadata;

/// Verified place-level room presence attested by a body (`room_presence_ttl_s = 300 s`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttestedPlacePresence {
    pub actor: ActorId,
    pub place: PlaceId,
    pub attested_at: Time,
}

/// Verified physical co-location presence attested by a body (`physical_presence_ttl_s = 15 s`,
/// `physical_room_radius_m = 4.0 m`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AttestedPhysicalPresence {
    pub actor: ActorId,
    pub place: PlaceId,
    pub distance_m: f64,
    pub attested_at: Time,
}

/// Per-wake / per-action scheduling metadata used by budget and monitor guards (`R-46`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct WakeDispatchHint {
    pub is_single_shot_promise: bool,
    pub is_recurrence_monitor: bool,
    pub is_proactive_notification: bool,
    pub chain_wakes: u32,
}

/// Request payload for the fourth door (`memory-write`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryWriteRequest {
    pub writer: ActorId,
    pub origin_actor: OriginActor,
    pub taint: Taint,
    pub is_procedure: bool,
    pub owner_signed: bool,
    pub places: Vec<PlaceId>,
    pub audience: Audience,
    pub target_event_id: Option<EventId>,
}

/// Pure, replayable `(log_seq, t_now)` snapshot of facts consumed by `Valve`.
///
/// No ambient clock, network, or LLM state is ever consulted outside this snapshot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ValveSnapshot {
    pub log_seq: LogSeq,
    pub t_now: Time,
    pub state_epoch: u64,
    pub owner_actor_id: Option<ActorId>,
    pub known_actors: BTreeSet<ActorId>,
    pub capabilities: BTreeMap<CapabilityId, CapabilityDescriptor>,
    pub place_presences: Vec<AttestedPlacePresence>,
    pub physical_presences: Vec<AttestedPhysicalPresence>,
    pub item_audiences: BTreeMap<EventId, Audience>,
    pub logged_declass_tokens: Vec<OwnerSigned<DeclassToken>>,
    pub daily_tokens_used: u64,
    pub proactive_sent_today: u32,
    pub current_wake_hint: WakeDispatchHint,
    pub action_hints: BTreeMap<ActionId, WakeDispatchHint>,
    pub chain_wake_counts: BTreeMap<EventId, u32>,
    pub pending_approvals: BTreeMap<ActionId, PendingApproval>,
    pub turn_taint: Taint,
    pub quiet_hours_active: bool,

    // V2 risk, cross-audience, embodied, and quiet-hours timezone facts
    pub active_context_audience: Option<Audience>,
    pub capability_risk_metadata: BTreeMap<CapabilityId, ActionRiskMetadata>,
    pub action_risk_metadata: BTreeMap<ActionId, ActionRiskMetadata>,
    pub embodied_places: BTreeSet<PlaceId>,
    pub current_urgency: Option<f64>,
    pub action_urgencies: BTreeMap<ActionId, f64>,
    pub active_recipient: Option<ActorId>,
    pub recipient_timezones: BTreeMap<ActorId, String>,
}

impl Default for ValveSnapshot {
    fn default() -> Self {
        Self::new(LogSeq::new(0), Time::from_millis(0))
    }
}

impl ValveSnapshot {
    /// Creates an empty deterministic snapshot at `(log_seq, t_now)`.
    #[must_use]
    pub fn new(log_seq: LogSeq, t_now: Time) -> Self {
        Self {
            log_seq,
            t_now,
            state_epoch: 0,
            owner_actor_id: None,
            known_actors: BTreeSet::new(),
            capabilities: BTreeMap::new(),
            place_presences: Vec::new(),
            physical_presences: Vec::new(),
            item_audiences: BTreeMap::new(),
            logged_declass_tokens: Vec::new(),
            daily_tokens_used: 0,
            proactive_sent_today: 0,
            current_wake_hint: WakeDispatchHint::default(),
            action_hints: BTreeMap::new(),
            chain_wake_counts: BTreeMap::new(),
            pending_approvals: BTreeMap::new(),
            turn_taint: Taint::Trusted,
            quiet_hours_active: false,
            active_context_audience: None,
            capability_risk_metadata: BTreeMap::new(),
            action_risk_metadata: BTreeMap::new(),
            embodied_places: BTreeSet::new(),
            current_urgency: None,
            action_urgencies: BTreeMap::new(),
            active_recipient: None,
            recipient_timezones: BTreeMap::new(),
        }
    }

    /// Returns `true` if `actor` is the configured owner.
    #[must_use]
    pub fn is_owner(&self, actor: &ActorId) -> bool {
        self.owner_actor_id
            .as_ref()
            .is_some_and(|owner| owner == actor)
    }

    /// Registers a verified [`CapabilityDescriptor`] in the snapshot.
    pub fn register_capability(&mut self, desc: CapabilityDescriptor) {
        self.capabilities.insert(desc.id.clone(), desc);
    }

    /// Registers an item's [`Audience`] for Absolute Line 2 enforcement.
    pub fn register_item_audience(&mut self, event_id: EventId, audience: Audience) {
        self.item_audiences.insert(event_id, audience);
    }
}
