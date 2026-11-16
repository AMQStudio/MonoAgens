//! Six domain facts, governance tokens, capability descriptors, and Option A receipts.
//!
//! Waist 1 describes every body exclusively through six domain facts:
//! actor, place, audience, event, capability, and receipt.

use core::fmt;
use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::ids::{
    ActionId, ActorId, BodyId, CapabilityId, CapabilitySchemaHash, ChannelId, EventId, PlaceId,
    Time,
};
use crate::version::VersionError;

/// The six canonical domain facts across Waist 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum DomainFactKind {
    Actor,
    Place,
    Audience,
    Event,
    Capability,
    Receipt,
}

impl fmt::Display for DomainFactKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Actor => "actor",
            Self::Place => "place",
            Self::Audience => "audience",
            Self::Event => "event",
            Self::Capability => "capability",
            Self::Receipt => "receipt",
        };
        f.write_str(name)
    }
}

/// Validation errors across Waist 1 domain types.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProtocolError {
    /// Protocol version incompatibility (`M_msg != M_gw || m_msg > m_gw`).
    UnsupportedVersion(VersionError),
    /// A required domain fact (such as `audience`) was missing or empty.
    MissingDomainFact {
        fact: DomainFactKind,
        detail: String,
    },
    /// A field violated a protocol invariant (e.g. out-of-range progress).
    InvalidField { field: &'static str, detail: String },
}

impl fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedVersion(err) => write!(f, "{err}"),
            Self::MissingDomainFact { fact, detail } => {
                write!(f, "missing required domain fact '{fact}': {detail}")
            }
            Self::InvalidField { field, detail } => {
                write!(f, "invalid protocol field '{field}': {detail}")
            }
        }
    }
}

impl std::error::Error for ProtocolError {}

impl From<VersionError> for ProtocolError {
    fn from(err: VersionError) -> Self {
        Self::UnsupportedVersion(err)
    }
}

/// Actor that originated an event chain; inherited monotonically along `caused_by`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(transparent)]
pub struct OriginActor(pub ActorId);

impl OriginActor {
    /// Creates an `OriginActor` from an [`ActorId`].
    #[must_use]
    pub fn new(actor: ActorId) -> Self {
        Self(actor)
    }

    /// Returns the underlying [`ActorId`].
    #[must_use]
    pub fn actor_id(&self) -> &ActorId {
        &self.0
    }
}

impl From<ActorId> for OriginActor {
    fn from(actor: ActorId) -> Self {
        Self(actor)
    }
}

/// Monotonic minimum-trust provenance taint label (`Untrusted < Provisional < Trusted`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum Taint {
    /// Lowest trust: external/unverified source or derived from one.
    Untrusted,
    /// Intermediate trust: corroborated or provisional.
    Provisional,
    /// Highest trust: verified trusted origin.
    Trusted,
}

impl Taint {
    /// Monotonic minimum-trust join: `min(self, other)`.
    #[must_use]
    pub fn combine(self, other: Self) -> Self {
        self.min(other)
    }

    /// Joins an iterator of taints using minimum-trust semantics (defaults to `Trusted` if empty).
    #[must_use]
    pub fn join_min(items: impl IntoIterator<Item = Self>) -> Self {
        items.into_iter().fold(Self::Trusted, Self::combine)
    }
}

/// Set of actors that could perceive an event.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(transparent)]
pub struct Audience {
    pub members: BTreeSet<ActorId>,
}

impl Audience {
    /// Creates an empty `Audience` (invalid for emitted percepts).
    #[must_use]
    pub fn empty() -> Self {
        Self {
            members: BTreeSet::new(),
        }
    }

    /// Creates an `Audience` from any iterator of [`ActorId`]s.
    #[must_use]
    pub fn new(members: impl IntoIterator<Item = ActorId>) -> Self {
        Self {
            members: members.into_iter().collect(),
        }
    }

    /// Returns `true` if no actors are named.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.members.is_empty()
    }

    /// Returns the number of distinct actors in this audience.
    #[must_use]
    pub fn len(&self) -> usize {
        self.members.len()
    }

    /// Returns `true` if `actor` is in the audience.
    #[must_use]
    pub fn contains(&self, actor: &ActorId) -> bool {
        self.members.contains(actor)
    }

    /// Returns `true` if every member of `self` is also in `other` (`self ⊆ other`).
    #[must_use]
    pub fn is_subset_of(&self, other: &Self) -> bool {
        self.members.is_subset(&other.members)
    }
}

/// Owner-signed payload wrapper used for identity anchors and governance bundles (`R-48`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct OwnerSigned<T> {
    pub payload: T,
    pub signer: ActorId,
    pub signed_at_ms: Time,
    pub signature: String,
}

/// Explicit declassification token authorizing sharing specific event ids with a target (`R-33`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct DeclassToken {
    pub token_id: String,
    pub issuer: ActorId,
    pub scope: Vec<EventId>,
    pub expiry: Time,
    pub target_place: Option<PlaceId>,
    pub target_audience: Audience,
    pub signature: String,
}

impl DeclassToken {
    /// Returns `true` if the token covers `event_id`, has not expired at `now`, and matches the target.
    #[must_use]
    pub fn authorizes(
        &self,
        event_id: EventId,
        context_audience: &Audience,
        context_place: Option<&PlaceId>,
        now: Time,
    ) -> bool {
        if now > self.expiry {
            return false;
        }
        if !self.scope.contains(&event_id) {
            return false;
        }
        if let Some(required_place) = &self.target_place
            && context_place != Some(required_place)
        {
            return false;
        }
        !context_audience.is_empty() && context_audience.is_subset_of(&self.target_audience)
    }
}

/// Pre-Step-5 privacy scope governing candidate recall and context assembly (`R-33`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct VisibleScope {
    pub audience: Audience,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_place: Option<PlaceId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub declass_tokens: Vec<DeclassToken>,
}

impl VisibleScope {
    /// Creates a `VisibleScope` for an active context audience.
    #[must_use]
    pub fn new(audience: Audience, target_place: Option<PlaceId>) -> Self {
        Self {
            audience,
            target_place,
            declass_tokens: Vec::new(),
        }
    }

    /// Returns `true` when `audience(context) ⊆ audience(item)` (with non-empty audiences)
    /// or an active [`DeclassToken`] explicitly authorizes `item_event`.
    #[must_use]
    pub fn allows(&self, item_audience: &Audience, item_event: Option<EventId>, now: Time) -> bool {
        if !self.audience.is_empty()
            && !item_audience.is_empty()
            && self.audience.is_subset_of(item_audience)
        {
            return true;
        }
        if let Some(ev) = item_event {
            return self
                .declass_tokens
                .iter()
                .any(|tok| tok.authorizes(ev, &self.audience, self.target_place.as_ref(), now));
        }
        false
    }
}

/// Status of a cross-body principal identity link (`R-44`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum PrincipalLinkStatus {
    Linked,
    Unlinked,
}

/// Cross-body identity linkage record mapping a body-local actor to a canonical person actor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct PrincipalLink {
    pub body_id: BodyId,
    pub platform_actor: ActorId,
    pub canonical_actor: ActorId,
    pub status: PrincipalLinkStatus,
    pub updated_at_ms: Time,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub challenge_hash: Option<String>,
}

/// Canonical action digest `H_act` (`R-42`) binding approvals to exact action parameters.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(transparent)]
pub struct HAct(String);

/// Alias for [`HAct`].
pub type ActionHash = HAct;

impl HAct {
    /// Wraps a computed canonical action hash string.
    #[must_use]
    pub fn new(digest: impl Into<String>) -> Self {
        Self(digest.into())
    }

    /// Returns the hash string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<String> for HAct {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for HAct {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

impl fmt::Display for HAct {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Canonical input material used to derive `H_act = BLAKE3("monoagens.act.v1" || canonical_preimage)`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ActionCanonicalInput {
    pub capability_id: CapabilityId,
    pub actor: ActorId,
    pub places: Vec<PlaceId>,
    pub state_epoch: u64,
    pub payload: Vec<u8>,
}

impl ActionCanonicalInput {
    /// Produces a deterministic length-prefixed canonical byte preimage with places sorted and deduplicated.
    #[must_use]
    pub fn canonical_preimage(&self) -> Vec<u8> {
        fn push_len_prefixed(buf: &mut Vec<u8>, bytes: &[u8]) {
            let len = u32::try_from(bytes.len()).unwrap_or(u32::MAX);
            buf.extend_from_slice(&len.to_le_bytes());
            buf.extend_from_slice(bytes);
        }

        let mut sorted_places = self.places.clone();
        sorted_places.sort();
        sorted_places.dedup();

        let mut out = Vec::with_capacity(64 + self.payload.len());
        out.extend_from_slice(b"monoagens.act.v1");
        push_len_prefixed(&mut out, self.capability_id.as_str().as_bytes());
        push_len_prefixed(&mut out, self.actor.as_str().as_bytes());
        let place_count = u32::try_from(sorted_places.len()).unwrap_or(u32::MAX);
        out.extend_from_slice(&place_count.to_le_bytes());
        for place in &sorted_places {
            push_len_prefixed(&mut out, place.as_str().as_bytes());
        }
        out.extend_from_slice(&self.state_epoch.to_le_bytes());
        push_len_prefixed(&mut out, &self.payload);
        out
    }
}

/// Capability behavioral flags declared by a body.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct CapabilityFlags {
    pub communicative: bool,
    pub identity_changing: bool,
    pub irreversible: bool,
}

/// Optional expected physical/sensor effect declared for a capability (`R-09`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct SensedEffect {
    pub channel: ChannelId,
    pub window_ms: u64,
    pub noise: f64,
    pub delta_pred: f64,
}

/// Descriptor for one capability advertised by a body.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct CapabilityDescriptor {
    pub id: CapabilityId,
    pub description: String,
    pub places: Vec<PlaceId>,
    pub flags: CapabilityFlags,
    pub typical_latency_ms: u64,
    pub capability_schema_hash: CapabilitySchemaHash,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sensed_effect: Option<SensedEffect>,
}

/// Sorts capability descriptors deterministically by capability id and schema hash,
/// and canonicalizes each descriptor's `places` list.
pub fn sort_capabilities(descriptors: &mut [CapabilityDescriptor]) {
    for desc in descriptors.iter_mut() {
        desc.places.sort();
        desc.places.dedup();
    }
    descriptors.sort_by(|a, b| {
        a.id.cmp(&b.id)
            .then_with(|| a.capability_schema_hash.cmp(&b.capability_schema_hash))
            .then_with(|| a.description.cmp(&b.description))
    });
}

/// Schedule definition for a recurring monitor expectation (`R-46`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum RecurrenceSchedule {
    IntervalMs(u64),
    Cron(String),
}

/// Catch-up policy when recurring triggers fell due during downtime (`R-46`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum CatchupPolicy {
    CoalesceOne,
    SkipToNext,
}

/// Recurrence specification attached to a `monitor` expectation (`R-46`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct RecurrenceSpec {
    pub schedule: RecurrenceSchedule,
    pub catchup: CatchupPolicy,
}

/// Commitment metadata attached to a `promise` expectation (`R-49`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct PromiseMeta {
    pub promisee_ids: Vec<ActorId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hard_deadline_ms: Option<Time>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conflict_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depends_on: Option<EventId>,
}

/// Non-retrying approval state bound to `H_act` when an action returns `held` (`R-42`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct PendingApproval {
    pub action_id: ActionId,
    pub capability_id: CapabilityId,
    pub h_act: HAct,
    pub state_epoch: u64,
    pub requested_at_ms: Time,
    pub expires_at_ms: Time,
    pub places: Vec<PlaceId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Five canonical outcomes of an action (`ARCHITECTURE.md` L198).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum ReceiptOutcome {
    Done,
    Refused,
    Held,
    Failed,
    Unknown,
}

/// Option A additive termination reason (`R-36`, `R-37`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum TerminationReason {
    Normal,
    CanceledByCore,
    PreemptedExternal,
    TimedOut,
}

/// Terminal or held receipt produced for an action.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Receipt {
    pub action_id: ActionId,
    pub capability_id: CapabilityId,
    pub outcome: ReceiptOutcome,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub termination_reason: Option<TerminationReason>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "schema", schemars(range(min = 0.0, max = 1.0)))]
    pub progress: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivered_units: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub h_act: Option<HAct>,
    pub timestamp_ms: Time,
}

impl Receipt {
    /// Validates Option A additive metadata bounds (`progress` in `[0.0, 1.0]`).
    pub fn validate(&self) -> Result<(), ProtocolError> {
        if let Some(p) = self.progress
            && (!p.is_finite() || !(0.0..=1.0).contains(&p))
        {
            return Err(ProtocolError::InvalidField {
                field: "progress",
                detail: format!("progress {p} must be finite and within [0.0, 1.0]"),
            });
        }
        Ok(())
    }
}
