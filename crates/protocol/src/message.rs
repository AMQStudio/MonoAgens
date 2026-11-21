//! Waist 1 message types: percepts, actions, multimodal handles, body state, and actor facts.

use serde::{Deserialize, Serialize};

use crate::facts::{
    Audience, CapabilityDescriptor, DomainFactKind, HAct, OriginActor, ProtocolError, SensedEffect,
    Taint,
};
use crate::ids::{
    ActionId, ActorId, BodyId, CapabilityId, ChannelId, CorrelationId, EventId, PlaceId, Time,
};
use crate::version::{CURRENT_PROTOCOL_VERSION, ProtocolVersion, check_version};

/// Opaque payload bytes carried unchanged through Core without interpretation.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(transparent)]
pub struct OpaquePayload(pub Vec<u8>);

impl OpaquePayload {
    /// Wraps raw bytes without inspecting or modifying them.
    #[must_use]
    pub fn from_bytes(bytes: impl Into<Vec<u8>>) -> Self {
        Self(bytes.into())
    }

    /// Returns the payload slice.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// Returns payload byte length.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Returns `true` if empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl From<Vec<u8>> for OpaquePayload {
    fn from(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }
}

impl From<&[u8]> for OpaquePayload {
    fn from(bytes: &[u8]) -> Self {
        Self(bytes.to_vec())
    }
}

/// Media modality classification for opaque attachment handles (`R-37`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum ModalitySlot {
    Image,
    AudioVoice,
    Video,
    SensorBlob,
}

/// Opaque attachment reference crossing Waist 1 with zero raw media bytes (`R-37`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct AttachmentHandle {
    pub handle_id: String,
    pub modality_slot: ModalitySlot,
    pub byte_len: u64,
    pub expires_at_ms: Time,
    pub embed_model_ver: String,
}

/// 10-element structural acoustic/prosody slot `f_modal` (`R-37`, zero emotion labels).
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(transparent)]
pub struct FModal(pub [f32; 10]);

/// Orientation score `orient(e, l)` toward a participant actor (`R-29`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct OrientScore {
    pub actor: ActorId,
    pub score: f32,
}

/// Channel prediction / observation tuple `(channel, Δ_obs, Δ_pred)` (`R-09`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ChannelPrediction {
    pub channel: ChannelId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delta_obs: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delta_pred: Option<f64>,
}

/// Versioned embedding space identifier `(embed_model_ver, dim_d, prompt_regime_id)`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct EmbeddingTag {
    pub embed_model_ver: String,
    pub dim_d: u32,
    pub prompt_regime_id: String,
}

/// Embedding vector tagged with its model version, dimension, and prompt regime.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct TaggedEmbedding {
    pub tag: EmbeddingTag,
    pub values: Vec<f32>,
}

/// A body-reported percept carrying the required domain facts and optional numeric/id features.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Percept {
    #[serde(default)]
    pub protocol_version: ProtocolVersion,
    pub actor: ActorId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin_actor: Option<OriginActor>,
    pub place: PlaceId,
    #[cfg_attr(
        feature = "schema",
        schemars(transform = crate::schema::transform_non_empty_array)
    )]
    pub audience: Audience,
    pub timestamp_ms: Time,
    pub payload: OpaquePayload,
    #[serde(default = "default_taint")]
    pub taint: Taint,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub addressees: Vec<ActorId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub named: Vec<ActorId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub orient: Vec<OrientScore>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<EventId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correlation_id: Option<CorrelationId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<ChannelId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delta_obs: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel_prediction: Option<ChannelPrediction>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub length: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub negative_flag: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repair: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub continued_attention: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub f_modal: Option<FModal>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub embeddings: Vec<TaggedEmbedding>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<AttachmentHandle>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dialogue_act_gap: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peer_expertise_margin: Option<f32>,
}

const fn default_taint() -> Taint {
    Taint::Untrusted
}

impl Percept {
    /// Constructs a minimal valid `Percept` with required domain facts and neutral optional fields.
    #[must_use]
    pub fn minimal(
        actor: ActorId,
        place: PlaceId,
        audience: Audience,
        timestamp_ms: Time,
        payload: OpaquePayload,
    ) -> Self {
        let origin = OriginActor::new(actor.clone());
        Self {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            actor,
            origin_actor: Some(origin),
            place,
            audience,
            timestamp_ms,
            payload,
            taint: Taint::Untrusted,
            addressees: Vec::new(),
            named: Vec::new(),
            orient: Vec::new(),
            reply_to: None,
            correlation_id: None,
            channel: None,
            delta_obs: None,
            channel_prediction: None,
            length: None,
            negative_flag: None,
            repair: None,
            continued_attention: None,
            f_modal: None,
            embeddings: Vec::new(),
            attachments: Vec::new(),
            dialogue_act_gap: None,
            peer_expertise_margin: None,
        }
    }

    /// Validates version compatibility and enforces that the percept names a non-empty `audience`.
    pub fn validate(&self, supported_version: ProtocolVersion) -> Result<(), ProtocolError> {
        check_version(self.protocol_version, supported_version)?;
        if self.audience.is_empty() {
            return Err(ProtocolError::MissingDomainFact {
                fact: DomainFactKind::Audience,
                detail: "percept must declare at least one actor in audience".to_owned(),
            });
        }
        Ok(())
    }
}

/// Outbound action emitted by Core toward a body capability.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Action {
    #[serde(default)]
    pub protocol_version: ProtocolVersion,
    pub action_id: ActionId,
    pub capability_id: CapabilityId,
    pub places: Vec<PlaceId>,
    #[cfg_attr(
        feature = "schema",
        schemars(transform = crate::schema::transform_non_empty_array)
    )]
    pub audience: Audience,
    pub origin_actor: OriginActor,
    pub taint: Taint,
    pub payload: OpaquePayload,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correlation_id: Option<CorrelationId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_effect: Option<SensedEffect>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub h_act: Option<HAct>,
    pub timestamp_ms: Time,
}

impl Action {
    /// Validates protocol version compatibility and required action facts.
    pub fn validate(&self, supported_version: ProtocolVersion) -> Result<(), ProtocolError> {
        check_version(self.protocol_version, supported_version)?;
        if self.audience.is_empty() {
            return Err(ProtocolError::MissingDomainFact {
                fact: DomainFactKind::Audience,
                detail: "action must declare a non-empty audience".to_owned(),
            });
        }
        if self.places.is_empty() {
            return Err(ProtocolError::MissingDomainFact {
                fact: DomainFactKind::Place,
                detail: "action must target at least one place".to_owned(),
            });
        }
        Ok(())
    }
}

/// Operational health state reported for a body.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum BodyStatus {
    Ready,
    Busy,
    Degraded,
    Stopped,
}

/// Body availability and capability advertisement carried into Core.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct BodyState {
    #[serde(default)]
    pub protocol_version: ProtocolVersion,
    pub body_id: BodyId,
    pub status: BodyStatus,
    pub capabilities: Vec<CapabilityDescriptor>,
    pub timestamp_ms: Time,
}

/// Reliability priors and teacher weight supplied per actor by Gateway (never "owner" inside Core).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ActorFacts {
    #[serde(default)]
    pub protocol_version: ProtocolVersion,
    pub actor_id: ActorId,
    pub alpha_0: f64,
    pub beta_0: f64,
    pub teacher_weight: f64,
    pub trusted_origin: bool,
    #[serde(default)]
    pub automated_peer: bool,
}
