//! MonoAgens Waist 1 and Waist 2 protocol contracts.
//!
//! `monoagens-protocol` defines the six Waist 1 domain facts (actor, place, audience,
//! event, capability, receipt), version compatibility rules, reasoning thought acts,
//! and Waist 2 JSON-RPC 2.0 envelopes/errors without IO or transport dependencies.

#![forbid(unsafe_code)]

pub mod facts;
pub mod ids;
pub mod message;
#[cfg(feature = "schema")]
pub mod schema;
pub mod thought;
pub mod version;
pub mod waist2;

pub use facts::{
    ActionCanonicalInput, ActionHash, Audience, CapabilityDescriptor, CapabilityFlags,
    CatchupPolicy, DeclassToken, DomainFactKind, HAct, OriginActor, OwnerSigned, PendingApproval,
    PrincipalLink, PrincipalLinkStatus, PromiseMeta, ProtocolError, Receipt, ReceiptOutcome,
    RecurrenceSchedule, RecurrenceSpec, SensedEffect, Taint, TerminationReason, VisibleScope,
    sort_capabilities,
};
pub use ids::{
    ActionId, ActorId, BodyId, CapabilityId, CapabilitySchemaHash, ChannelId, CorrelationId,
    DomainId, EventId, PlaceId, Time,
};
pub use message::{
    Action, ActorFacts, AttachmentHandle, BodyState, BodyStatus, ChannelPrediction, EmbeddingTag,
    FModal, ModalitySlot, OpaquePayload, OrientScore, Percept, TaggedEmbedding,
};
pub use thought::{
    ClassificationVerdict, ConsolidateOutput, ExpectKind, ResolveOutcome, ThoughtAct, ThoughtBatch,
};
pub use version::{CURRENT_PROTOCOL_VERSION, ProtocolVersion, VersionError, check_version};
pub use waist2::{
    ActionCancelParams, ActionCancelResult, ActionExecuteParams, ActionExecuteResult,
    ActionReceiptParams, ActionReceiptResult, ApprovalResolveParams, ApprovalResolveResult,
    BlobGetParams, BlobGetResult, BlobPutParams, BlobPutResult, BodyDiscoverParams,
    BodyDiscoverResult, BodyHeartbeatParams, BodyHeartbeatResult, BodyShutdownParams,
    BodyShutdownResult, JSONRPC_VERSION, MAX_INLINE_ATTACHMENT_BYTES, MetaValue, PerceptEmitParams,
    PerceptEmitResult, QuestionAnswerParams, QuestionAnswerResult, RpcId, StateQueryParams,
    StateQueryResult, StateQueryStatus, Waist2Call, Waist2Error, Waist2ErrorCode, Waist2ErrorData,
    Waist2Headers, Waist2Method, Waist2Request, Waist2Response,
};
