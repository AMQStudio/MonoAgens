//! Waist 2 (`monoagens.body.v1`) stateless JSON-RPC 2.0 wire envelopes and typed errors.
//!
//! Transport headers (`idempotency_key`, `protocol_version`, reverse-DNS `_meta`,
//! `traceparent`) are cleanly separated from the six Waist 1 domain facts.
//! Unknown keys in envelopes and `_meta` are ignored for additive forward compatibility.

use core::fmt;
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::facts::{
    Audience, CapabilityDescriptor, DomainFactKind, HAct, OriginActor, ProtocolError, Receipt,
    Taint,
};
use crate::ids::{
    ActionId, ActorId, BodyId, CapabilityId, ChannelId, CorrelationId, EventId, PlaceId, Time,
};
use crate::message::{
    AttachmentHandle, BodyStatus, ChannelPrediction, FModal, OpaquePayload, OrientScore, Percept,
    TaggedEmbedding,
};
use crate::version::{CURRENT_PROTOCOL_VERSION, ProtocolVersion, check_version};

/// JSON-RPC version string required by all Waist 2 messages.
pub const JSONRPC_VERSION: &str = "2.0";

/// Maximum inline media payload size (`256 KiB`) on `percept/emit` (`D4`).
pub const MAX_INLINE_ATTACHMENT_BYTES: usize = 256 * 1024;

/// JSON value subset for reverse-DNS `_meta` extensions without tying `protocol` to `serde_json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(untagged)]
pub enum MetaValue {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(String),
    Array(Vec<MetaValue>),
    Object(BTreeMap<String, MetaValue>),
}

/// JSON-RPC 2.0 request/response correlation identifier.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(untagged)]
pub enum RpcId {
    Int(i64),
    Str(String),
}

/// Transport-level Waist 2 headers separated from domain facts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Waist2Headers {
    pub idempotency_key: String,
    pub protocol_version: ProtocolVersion,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub traceparent: Option<String>,
    #[serde(rename = "_meta", default, skip_serializing_if = "BTreeMap::is_empty")]
    pub meta: BTreeMap<String, MetaValue>,
}

impl Waist2Headers {
    /// Creates headers for the current protocol version with no trace or `_meta` entries.
    #[must_use]
    pub fn new(idempotency_key: impl Into<String>) -> Self {
        Self {
            idempotency_key: idempotency_key.into(),
            protocol_version: CURRENT_PROTOCOL_VERSION,
            traceparent: None,
            meta: BTreeMap::new(),
        }
    }

    /// Validates `protocol_version` against `supported`, returning `-32004` on mismatch.
    pub fn validate_version(&self, supported: ProtocolVersion) -> Result<(), Waist2Error> {
        check_version(self.protocol_version, supported)
            .map_err(|err| Waist2Error::unsupported_version(err.received, err.supported))
    }
}

/// The twelve stateless JSON-RPC 2.0 methods defining Waist 2 (`monoagens.body.v1`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum Waist2Method {
    #[serde(rename = "body/discover")]
    BodyDiscover,
    #[serde(rename = "body/heartbeat")]
    BodyHeartbeat,
    #[serde(rename = "body/shutdown")]
    BodyShutdown,
    #[serde(rename = "percept/emit")]
    PerceptEmit,
    #[serde(rename = "action/execute")]
    ActionExecute,
    #[serde(rename = "action/cancel")]
    ActionCancel,
    #[serde(rename = "action/receipt")]
    ActionReceipt,
    #[serde(rename = "approval/resolve")]
    ApprovalResolve,
    #[serde(rename = "question/answer")]
    QuestionAnswer,
    #[serde(rename = "state/query")]
    StateQuery,
    #[serde(rename = "blob/put")]
    BlobPut,
    #[serde(rename = "blob/get")]
    BlobGet,
}

impl Waist2Method {
    /// Returns the wire method name string.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::BodyDiscover => "body/discover",
            Self::BodyHeartbeat => "body/heartbeat",
            Self::BodyShutdown => "body/shutdown",
            Self::PerceptEmit => "percept/emit",
            Self::ActionExecute => "action/execute",
            Self::ActionCancel => "action/cancel",
            Self::ActionReceipt => "action/receipt",
            Self::ApprovalResolve => "approval/resolve",
            Self::QuestionAnswer => "question/answer",
            Self::StateQuery => "state/query",
            Self::BlobPut => "blob/put",
            Self::BlobGet => "blob/get",
        }
    }
}

impl fmt::Display for Waist2Method {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Typed Waist 2 JSON-RPC error codes `-32001`..`-32006`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum Waist2ErrorCode {
    UnknownCapability = -32001,
    DuplicateIdempotency = -32002,
    InvalidReceiptTransition = -32003,
    UnsupportedVersion = -32004,
    HashMismatch = -32005,
    UndeclaredAudience = -32006,
}

impl Waist2ErrorCode {
    /// Returns the numeric JSON-RPC error code.
    #[must_use]
    pub const fn code(self) -> i32 {
        self as i32
    }

    /// Returns the canonical symbolic error identifier.
    #[must_use]
    pub const fn symbol(self) -> &'static str {
        match self {
            Self::UnknownCapability => "ERR_UNKNOWN_CAPABILITY",
            Self::DuplicateIdempotency => "ERR_DUPLICATE_IDEMPOTENCY",
            Self::InvalidReceiptTransition => "ERR_INVALID_RECEIPT_TRANSITION",
            Self::UnsupportedVersion => "ERR_UNSUPPORTED_VERSION",
            Self::HashMismatch => "ERR_HASH_MISMATCH",
            Self::UndeclaredAudience => "ERR_UNDECLARED_AUDIENCE",
        }
    }

    /// Parses a numeric JSON-RPC error code into a known Waist 2 error code.
    #[must_use]
    pub const fn from_i32(code: i32) -> Option<Self> {
        match code {
            -32001 => Some(Self::UnknownCapability),
            -32002 => Some(Self::DuplicateIdempotency),
            -32003 => Some(Self::InvalidReceiptTransition),
            -32004 => Some(Self::UnsupportedVersion),
            -32005 => Some(Self::HashMismatch),
            -32006 => Some(Self::UndeclaredAudience),
            _ => None,
        }
    }
}

/// Optional structured detail payload on a Waist 2 error object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Waist2ErrorData {
    pub symbol: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub received_version: Option<ProtocolVersion>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supported_version: Option<ProtocolVersion>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub missing_fact: Option<DomainFactKind>,
}

/// JSON-RPC 2.0 error object carrying typed Waist 2 failure codes `-32001`..`-32006`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Waist2Error {
    pub code: i32,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Waist2ErrorData>,
}

impl Waist2Error {
    /// Constructs `-32001 ERR_UNKNOWN_CAPABILITY`.
    #[must_use]
    pub fn unknown_capability(capability_id: &CapabilityId) -> Self {
        Self::from_code(
            Waist2ErrorCode::UnknownCapability,
            format!(
                "{}: unknown capability '{}'",
                Waist2ErrorCode::UnknownCapability.symbol(),
                capability_id
            ),
            None,
        )
    }

    /// Constructs `-32002 ERR_DUPLICATE_IDEMPOTENCY`.
    #[must_use]
    pub fn duplicate_idempotency(key: &str) -> Self {
        Self::from_code(
            Waist2ErrorCode::DuplicateIdempotency,
            format!(
                "{}: duplicate idempotency_key '{}'",
                Waist2ErrorCode::DuplicateIdempotency.symbol(),
                key
            ),
            None,
        )
    }

    /// Constructs `-32003 ERR_INVALID_RECEIPT_TRANSITION`.
    #[must_use]
    pub fn invalid_receipt_transition(detail: impl Into<String>) -> Self {
        Self::from_code(
            Waist2ErrorCode::InvalidReceiptTransition,
            format!(
                "{}: {}",
                Waist2ErrorCode::InvalidReceiptTransition.symbol(),
                detail.into()
            ),
            None,
        )
    }

    /// Constructs `-32004 ERR_UNSUPPORTED_VERSION`, naming both received and supported versions.
    #[must_use]
    pub fn unsupported_version(received: ProtocolVersion, supported: ProtocolVersion) -> Self {
        Self::from_code(
            Waist2ErrorCode::UnsupportedVersion,
            format!(
                "{}: unsupported protocol version {received} (supported {supported})",
                Waist2ErrorCode::UnsupportedVersion.symbol()
            ),
            Some(Waist2ErrorData {
                symbol: Waist2ErrorCode::UnsupportedVersion.symbol().to_owned(),
                received_version: Some(received),
                supported_version: Some(supported),
                missing_fact: None,
            }),
        )
    }

    /// Constructs `-32005 ERR_HASH_MISMATCH`.
    #[must_use]
    pub fn hash_mismatch(expected: &str, actual: &str) -> Self {
        Self::from_code(
            Waist2ErrorCode::HashMismatch,
            format!(
                "{}: SHA-256 digest mismatch (expected {expected}, got {actual})",
                Waist2ErrorCode::HashMismatch.symbol()
            ),
            None,
        )
    }

    /// Constructs `-32006 ERR_UNDECLARED_AUDIENCE`.
    #[must_use]
    pub fn undeclared_audience() -> Self {
        Self::from_code(
            Waist2ErrorCode::UndeclaredAudience,
            format!(
                "{}: percept must declare a non-empty audience",
                Waist2ErrorCode::UndeclaredAudience.symbol()
            ),
            Some(Waist2ErrorData {
                symbol: Waist2ErrorCode::UndeclaredAudience.symbol().to_owned(),
                received_version: None,
                supported_version: None,
                missing_fact: Some(DomainFactKind::Audience),
            }),
        )
    }

    /// Returns the typed [`Waist2ErrorCode`] if `self.code` is in `-32001`..=`-32006`.
    #[must_use]
    pub const fn error_code(&self) -> Option<Waist2ErrorCode> {
        Waist2ErrorCode::from_i32(self.code)
    }

    fn from_code(code: Waist2ErrorCode, message: String, data: Option<Waist2ErrorData>) -> Self {
        let data = data.or_else(|| {
            Some(Waist2ErrorData {
                symbol: code.symbol().to_owned(),
                received_version: None,
                supported_version: None,
                missing_fact: None,
            })
        });
        Self {
            code: code.code(),
            message,
            data,
        }
    }
}

impl fmt::Display for Waist2Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}", self.code, self.message)
    }
}

impl std::error::Error for Waist2Error {}

impl From<ProtocolError> for Waist2Error {
    fn from(err: ProtocolError) -> Self {
        match err {
            ProtocolError::UnsupportedVersion(v) => {
                Self::unsupported_version(v.received, v.supported)
            }
            ProtocolError::MissingDomainFact {
                fact: DomainFactKind::Audience,
                ..
            } => Self::undeclared_audience(),
            ProtocolError::MissingDomainFact { fact, detail } => {
                Self::invalid_receipt_transition(format!("missing {fact}: {detail}"))
            }
            ProtocolError::InvalidField { field, detail } => {
                Self::invalid_receipt_transition(format!("invalid {field}: {detail}"))
            }
        }
    }
}

/// Generic JSON-RPC 2.0 request envelope for Waist 2 calls.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Waist2Request<P> {
    #[cfg_attr(
        feature = "schema",
        schemars(transform = crate::schema::transform_jsonrpc_const)
    )]
    pub jsonrpc: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<RpcId>,
    pub method: Waist2Method,
    pub params: P,
}

impl<P> Waist2Request<P> {
    /// Creates a JSON-RPC `2.0` request envelope.
    #[must_use]
    pub fn new(id: Option<RpcId>, method: Waist2Method, params: P) -> Self {
        Self {
            jsonrpc: JSONRPC_VERSION.to_owned(),
            id,
            method,
            params,
        }
    }
}

fn option_none<T>() -> Option<T> {
    None
}

/// Generic JSON-RPC 2.0 response envelope for Waist 2 calls.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Waist2Response<R> {
    #[cfg_attr(
        feature = "schema",
        schemars(transform = crate::schema::transform_jsonrpc_const)
    )]
    pub jsonrpc: String,
    pub id: Option<RpcId>,
    #[serde(default = "option_none", skip_serializing_if = "Option::is_none")]
    #[cfg_attr(
        feature = "schema",
        schemars(with = "Option<R>", default = "option_none::<()>")
    )]
    pub result: Option<R>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<Waist2Error>,
}

impl<R> Waist2Response<R> {
    /// Constructs a success response.
    #[must_use]
    pub fn ok(id: Option<RpcId>, result: R) -> Self {
        Self {
            jsonrpc: JSONRPC_VERSION.to_owned(),
            id,
            result: Some(result),
            error: None,
        }
    }

    /// Constructs an error response.
    #[must_use]
    pub fn err(id: Option<RpcId>, error: Waist2Error) -> Self {
        Self {
            jsonrpc: JSONRPC_VERSION.to_owned(),
            id,
            result: None,
            error: Some(error),
        }
    }
}

/// Parameters for `body/discover`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct BodyDiscoverParams {
    pub headers: Waist2Headers,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body_id: Option<BodyId>,
}

/// Result for `body/discover`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct BodyDiscoverResult {
    pub body_id: BodyId,
    pub protocol_version: ProtocolVersion,
    pub capabilities: Vec<CapabilityDescriptor>,
    pub places: Vec<PlaceId>,
}

/// Parameters for `body/heartbeat`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct BodyHeartbeatParams {
    pub headers: Waist2Headers,
    pub body_id: BodyId,
    pub status: BodyStatus,
    pub timestamp_ms: Time,
}

/// Result for `body/heartbeat`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct BodyHeartbeatResult {
    pub acknowledged_at_ms: Time,
}

/// Parameters for `body/shutdown`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct BodyShutdownParams {
    pub headers: Waist2Headers,
    pub body_id: BodyId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Result for `body/shutdown`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct BodyShutdownResult {
    pub accepted: bool,
}

/// Parameters for `percept/emit`, separating transport headers from domain facts.
///
/// `audience` is modeled as `Option<Audience>` on the wire so missing or empty
/// audiences yield `-32006 ERR_UNDECLARED_AUDIENCE` rather than a generic schema error.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(
    feature = "schema",
    schemars(transform = crate::schema::transform_percept_emit_params)
)]
pub struct PerceptEmitParams {
    pub headers: Waist2Headers,
    pub actor: ActorId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin_actor: Option<OriginActor>,
    pub place: PlaceId,
    #[serde(default)]
    pub audience: Option<Audience>,
    pub timestamp_ms: Time,
    pub payload: OpaquePayload,
    #[serde(default = "default_untrusted")]
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

const fn default_untrusted() -> Taint {
    Taint::Untrusted
}

impl PerceptEmitParams {
    /// Wraps an existing Waist 1 [`Percept`] with Waist 2 transport headers.
    #[must_use]
    pub fn from_percept(headers: Waist2Headers, percept: Percept) -> Self {
        Self {
            headers,
            actor: percept.actor,
            origin_actor: percept.origin_actor,
            place: percept.place,
            audience: Some(percept.audience),
            timestamp_ms: percept.timestamp_ms,
            payload: percept.payload,
            taint: percept.taint,
            addressees: percept.addressees,
            named: percept.named,
            orient: percept.orient,
            reply_to: percept.reply_to,
            correlation_id: percept.correlation_id,
            channel: percept.channel,
            delta_obs: percept.delta_obs,
            channel_prediction: percept.channel_prediction,
            length: percept.length,
            negative_flag: percept.negative_flag,
            repair: percept.repair,
            continued_attention: percept.continued_attention,
            f_modal: percept.f_modal,
            embeddings: percept.embeddings,
            attachments: percept.attachments,
            dialogue_act_gap: percept.dialogue_act_gap,
            peer_expertise_margin: percept.peer_expertise_margin,
        }
    }

    /// Validates version compatibility (`-32004`) and audience presence (`-32006`),
    /// translating the envelope into a Waist 1 [`Percept`].
    pub fn into_percept(self, supported_version: ProtocolVersion) -> Result<Percept, Waist2Error> {
        self.headers.validate_version(supported_version)?;
        let audience = match self.audience {
            Some(aud) if !aud.is_empty() => aud,
            _ => return Err(Waist2Error::undeclared_audience()),
        };
        let percept = Percept {
            protocol_version: self.headers.protocol_version,
            actor: self.actor,
            origin_actor: self.origin_actor,
            place: self.place,
            audience,
            timestamp_ms: self.timestamp_ms,
            payload: self.payload,
            taint: self.taint,
            addressees: self.addressees,
            named: self.named,
            orient: self.orient,
            reply_to: self.reply_to,
            correlation_id: self.correlation_id,
            channel: self.channel,
            delta_obs: self.delta_obs,
            channel_prediction: self.channel_prediction,
            length: self.length,
            negative_flag: self.negative_flag,
            repair: self.repair,
            continued_attention: self.continued_attention,
            f_modal: self.f_modal,
            embeddings: self.embeddings,
            attachments: self.attachments,
            dialogue_act_gap: self.dialogue_act_gap,
            peer_expertise_margin: self.peer_expertise_margin,
        };
        percept.validate(supported_version)?;
        Ok(percept)
    }
}

/// Result for `percept/emit`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct PerceptEmitResult {
    pub accepted: bool,
}

/// Parameters for `action/execute`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ActionExecuteParams {
    pub headers: Waist2Headers,
    pub action_id: ActionId,
    pub capability_id: CapabilityId,
    pub places: Vec<PlaceId>,
    #[cfg_attr(
        feature = "schema",
        schemars(transform = crate::schema::transform_non_empty_array)
    )]
    pub audience: Audience,
    pub payload: OpaquePayload,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub h_act: Option<HAct>,
}

/// Result for `action/execute`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ActionExecuteResult {
    pub accepted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub receipt: Option<Receipt>,
}

/// Parameters for `action/cancel`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ActionCancelParams {
    pub headers: Waist2Headers,
    pub action_id: ActionId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Result for `action/cancel`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ActionCancelResult {
    pub cancelled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub receipt: Option<Receipt>,
}

/// Parameters for `action/receipt`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ActionReceiptParams {
    pub headers: Waist2Headers,
    pub receipt: Receipt,
}

/// Result for `action/receipt`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ActionReceiptResult {
    pub recorded: bool,
}

/// Parameters for `approval/resolve`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ApprovalResolveParams {
    pub headers: Waist2Headers,
    pub action_id: ActionId,
    pub h_act: HAct,
    pub state_epoch: u64,
    pub approved: bool,
    pub resolver: ActorId,
    pub resolved_at_ms: Time,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
    #[serde(default)]
    pub out_of_band_channel: bool,
}

/// Result for `approval/resolve`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ApprovalResolveResult {
    pub applied: bool,
}

/// Parameters for `question/answer`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct QuestionAnswerParams {
    pub headers: Waist2Headers,
    pub question_id: String,
    pub actor: ActorId,
    pub place: PlaceId,
    #[cfg_attr(
        feature = "schema",
        schemars(transform = crate::schema::transform_non_empty_array)
    )]
    pub audience: Audience,
    pub answer: OpaquePayload,
    pub timestamp_ms: Time,
}

/// Result for `question/answer`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct QuestionAnswerResult {
    pub accepted: bool,
}

/// Parameters for `state/query` used after body crashes or restarts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct StateQueryParams {
    pub headers: Waist2Headers,
    pub action_id: ActionId,
}

/// Crash-recovery state reported by `state/query`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum StateQueryStatus {
    InFlight {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        progress: Option<f32>,
    },
    Terminal {
        receipt: Receipt,
    },
    Unknown,
}

/// Result for `state/query`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct StateQueryResult {
    pub action_id: ActionId,
    pub status: StateQueryStatus,
}

/// Parameters for `blob/put` out-of-band attachment staging (`60 s` TTL, SHA-256 verified).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct BlobPutParams {
    pub headers: Waist2Headers,
    pub sha256: String,
    pub byte_len: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inline_bytes: Option<OpaquePayload>,
}

/// Result for `blob/put`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct BlobPutResult {
    pub blob_ref: String,
    pub sha256: String,
    pub expires_at_ms: Time,
}

/// Parameters for `blob/get`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct BlobGetParams {
    pub headers: Waist2Headers,
    pub blob_ref: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_sha256: Option<String>,
}

/// Result for `blob/get`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct BlobGetResult {
    pub blob_ref: String,
    pub sha256: String,
    pub byte_len: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes: Option<OpaquePayload>,
}

/// Adjacently-tagged dispatch representation covering all twelve Waist 2 methods.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(tag = "method", content = "params")]
pub enum Waist2Call {
    #[serde(rename = "body/discover")]
    BodyDiscover(BodyDiscoverParams),
    #[serde(rename = "body/heartbeat")]
    BodyHeartbeat(BodyHeartbeatParams),
    #[serde(rename = "body/shutdown")]
    BodyShutdown(BodyShutdownParams),
    #[serde(rename = "percept/emit")]
    PerceptEmit(Box<PerceptEmitParams>),
    #[serde(rename = "action/execute")]
    ActionExecute(ActionExecuteParams),
    #[serde(rename = "action/cancel")]
    ActionCancel(ActionCancelParams),
    #[serde(rename = "action/receipt")]
    ActionReceipt(ActionReceiptParams),
    #[serde(rename = "approval/resolve")]
    ApprovalResolve(ApprovalResolveParams),
    #[serde(rename = "question/answer")]
    QuestionAnswer(QuestionAnswerParams),
    #[serde(rename = "state/query")]
    StateQuery(StateQueryParams),
    #[serde(rename = "blob/put")]
    BlobPut(BlobPutParams),
    #[serde(rename = "blob/get")]
    BlobGet(BlobGetParams),
}

impl Waist2Call {
    /// Returns the transport headers carried by this call.
    #[must_use]
    pub fn headers(&self) -> &Waist2Headers {
        match self {
            Self::BodyDiscover(p) => &p.headers,
            Self::BodyHeartbeat(p) => &p.headers,
            Self::BodyShutdown(p) => &p.headers,
            Self::PerceptEmit(p) => &p.headers,
            Self::ActionExecute(p) => &p.headers,
            Self::ActionCancel(p) => &p.headers,
            Self::ActionReceipt(p) => &p.headers,
            Self::ApprovalResolve(p) => &p.headers,
            Self::QuestionAnswer(p) => &p.headers,
            Self::StateQuery(p) => &p.headers,
            Self::BlobPut(p) => &p.headers,
            Self::BlobGet(p) => &p.headers,
        }
    }

    /// Returns the matching [`Waist2Method`] variant.
    #[must_use]
    pub const fn method(&self) -> Waist2Method {
        match self {
            Self::BodyDiscover(_) => Waist2Method::BodyDiscover,
            Self::BodyHeartbeat(_) => Waist2Method::BodyHeartbeat,
            Self::BodyShutdown(_) => Waist2Method::BodyShutdown,
            Self::PerceptEmit(_) => Waist2Method::PerceptEmit,
            Self::ActionExecute(_) => Waist2Method::ActionExecute,
            Self::ActionCancel(_) => Waist2Method::ActionCancel,
            Self::ActionReceipt(_) => Waist2Method::ActionReceipt,
            Self::ApprovalResolve(_) => Waist2Method::ApprovalResolve,
            Self::QuestionAnswer(_) => Waist2Method::QuestionAnswer,
            Self::StateQuery(_) => Waist2Method::StateQuery,
            Self::BlobPut(_) => Waist2Method::BlobPut,
            Self::BlobGet(_) => Waist2Method::BlobGet,
        }
    }
}
