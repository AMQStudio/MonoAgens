//! Extension seams for unimplemented sibling tasks (`4.3`, `4.4`, `4.5`, and `6.x`).
//!
//! Where a seam is `None` (not wired), the corresponding capability is absent from the
//! Gateway registry with zero fake or stubbed behavior.

use core::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use monoagens_core::{ContextPlan, DoorDecision, Doors, LogSeq, NoopDoors};
use monoagens_protocol::{
    Action, ActorId, ApprovalResolveParams, ApprovalResolveResult, BlobGetParams, BlobGetResult,
    BlobPutParams, BlobPutResult, BodyId, CapabilityDescriptor, EventId, Percept, Time,
    Waist2Error,
};

/// Typed error surfaced by an attached Gateway seam.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeamError {
    pub seam: &'static str,
    pub detail: String,
}

impl SeamError {
    /// Creates a new [`SeamError`].
    #[must_use]
    pub fn new(seam: &'static str, detail: impl Into<String>) -> Self {
        Self {
            seam,
            detail: detail.into(),
        }
    }
}

impl fmt::Display for SeamError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "seam '{}' error: {}", self.seam, self.detail)
    }
}

impl std::error::Error for SeamError {}

/// Seam for task `4.3`: canonical `κ(b, a)` identity resolution and bilateral account linking.
pub trait IdentitySeam: Send + Sync {
    /// Resolves `(body_id, account_id)` to a canonical `ActorId` (e.g., `per_...`).
    fn resolve_actor(&self, body_id: &BodyId, account_id: &ActorId) -> ActorId;
}

/// Seam for task `4.4`: scope-keyed `BLAKE3` attachment custody, `blob/put`/`blob/get`,
/// and `hard_forget` erasure cascade.
pub trait AttachmentCustodySeam: Send + Sync {
    /// Strips/stores inline payloads (`<= 256 KiB`) or validates references before Core queueing.
    fn ingest_percept_attachments(
        &mut self,
        body_id: &BodyId,
        percept: &mut Percept,
    ) -> Result<(), SeamError>;

    /// Handles Waist-2 `blob/put` when attachment custody is wired.
    fn handle_blob_put(&mut self, params: BlobPutParams) -> Result<BlobPutResult, Waist2Error>;

    /// Handles Waist-2 `blob/get` when attachment custody is wired.
    fn handle_blob_get(&self, params: BlobGetParams) -> Result<BlobGetResult, Waist2Error>;

    /// Executes synchronous blob/cache wipe for `hard_forget`.
    fn cascade_hard_forget(
        &mut self,
        event_ids: &[EventId],
        actor_ids: &[ActorId],
    ) -> Result<(), SeamError>;
}

/// Seam for task `4.5`: pre-queue `EmbeddingGemma 2` (`256d`) and hybrid `Tool RAG` indexing.
pub trait PreQueueIndexSeam: Send + Sync {
    /// Computes pre-queue embeddings and `dialogue_act_gap` scores; errors are logged and
    /// leave percepts operational without vectors.
    fn enrich_percept(&self, percept: &mut Percept) -> Result<(), SeamError>;

    /// Indexes capabilities reported via `body/discover` or `BodyState`.
    fn index_capabilities(&mut self, body_id: &BodyId, capabilities: &[CapabilityDescriptor]);

    /// Removes capabilities owned by a crashed or stopped body.
    fn remove_body_capabilities(&mut self, body_id: &BodyId);
}

/// Seam for task `6.x` (`crates/valve`): [`Doors`] plus snapshot synchronization and
/// `approval/resolve` handling.
pub trait DoorController: Doors + Send {
    /// Synchronizes `(log_seq, now, daily_tokens_used, capabilities)` before or after turns.
    fn sync_runtime_state(
        &mut self,
        _log_seq: LogSeq,
        _now: Time,
        _daily_tokens_used: u64,
        _capabilities: &[CapabilityDescriptor],
    ) {
    }

    /// Handles Waist-2 `approval/resolve` when Valve 6.x is wired.
    ///
    /// Returning `None` indicates the approval capability is absent.
    fn resolve_approval(
        &mut self,
        _params: &ApprovalResolveParams,
    ) -> Option<Result<ApprovalResolveResult, Waist2Error>> {
        None
    }
}

impl DoorController for NoopDoors {}

/// Wraps any [`Doors`] implementation (`NoopDoors`, `monoagens_valve::Valve`, or test guards)
/// as a [`DoorController`].
#[derive(Debug)]
pub struct DoorsAdapter<D>(pub D);

impl<D: Doors + Send> Doors for DoorsAdapter<D> {
    fn inbound(&mut self, percept: &mut Percept) -> DoorDecision {
        self.0.inbound(percept)
    }

    fn context(&mut self, plan: &mut ContextPlan) -> DoorDecision {
        self.0.context(plan)
    }

    fn action(&mut self, action: &mut Action) -> DoorDecision {
        self.0.action(action)
    }
}

impl<D: Doors + Send> DoorController for DoorsAdapter<D> {}

/// Bounded async plugin hook run under `hook_timeout_ms` (`4.1`).
pub trait AsyncGatewayHook: Send + Sync {
    /// Stable hook name for diagnostic error reporting.
    fn name(&self) -> &str;

    /// Runs hook inspection on an admitted percept.
    fn on_percept<'a>(
        &'a self,
        percept: &'a Percept,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>>;
}

/// Container of optional Gateway extension seams (`None` when not wired).
#[derive(Default, Clone)]
pub struct GatewaySeams {
    pub identity: Option<Arc<dyn IdentitySeam>>,
    pub attachments: Option<Arc<Mutex<dyn AttachmentCustodySeam>>>,
    pub pre_queue_index: Option<Arc<Mutex<dyn PreQueueIndexSeam>>>,
    pub pre_queue_service: Option<Arc<crate::prequeue::PreQueueService>>,
    pub hooks: Vec<Arc<dyn AsyncGatewayHook>>,
}

impl fmt::Debug for GatewaySeams {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GatewaySeams")
            .field("identity_wired", &self.identity.is_some())
            .field("attachments_wired", &self.attachments.is_some())
            .field(
                "pre_queue_index_wired",
                &(self.pre_queue_index.is_some() || self.pre_queue_service.is_some()),
            )
            .field("hook_count", &self.hooks.len())
            .finish()
    }
}
