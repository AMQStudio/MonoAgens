//! Extension seams for Gateway capabilities (`4.3`, `4.4`, `4.5`, and `6.x` Valve/HITL).
//!
//! Where a seam is `None` (not wired), the corresponding capability is absent from the
//! Gateway registry with zero fake or stubbed behavior.

use core::fmt;
use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use monoagens_core::{
    ContextPlan, DoorDecision, DoorKind, Doors, EventDetail, EventKind, EventRecord, LogSeq,
    NoopDoors, NumericFeatures,
};
use monoagens_protocol::{
    Action, ActionId, ActorId, ApprovalResolveParams, ApprovalResolveResult, Audience,
    BlobGetParams, BlobGetResult, BlobPutParams, BlobPutResult, BodyId, CapabilityDescriptor,
    EventId, OpaquePayload, OriginActor, Percept, PlaceId, Receipt, ReceiptOutcome, Taint,
    TerminationReason, Time, Waist2Error,
};
use monoagens_valve::{ApprovalLogRecord, Valve, ValveReasonTag};

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
/// decoupled out-of-band `approval/resolve` handling.
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

    /// Synchronizes Core's `state_epoch` (`HardForget` counter) into the door controller.
    fn sync_state_epoch(&mut self, _state_epoch: u64) {}

    /// Reconstructs persisted approval and digest state from append-only events on startup.
    fn replay_from_store_events(&mut self, _events: &[EventRecord]) {}

    /// Synchronizes newly appended event audiences into the door snapshot for `Line2Scope`.
    fn sync_item_audiences(&mut self, _events: &[EventRecord]) {}

    /// Handles Waist-2 `approval/resolve` when Valve 6.x is wired.
    ///
    /// Returning `None` indicates the approval capability is absent.
    fn resolve_approval(
        &mut self,
        _params: &ApprovalResolveParams,
    ) -> Option<Result<ApprovalResolveResult, Waist2Error>> {
        None
    }

    /// Records a fail-closed audit trace when an untrusted/in-band chat body attempts `approval/resolve`.
    fn record_in_band_rejection_audit(&mut self, _params: &ApprovalResolveParams, _detail: &str) {}

    /// Drains append-only governance events (held actions, resolutions, grant consumption, audits)
    /// to be persisted into Core's store BEFORE action re-dispatch.
    fn take_pending_approval_events(&mut self) -> Vec<EventRecord> {
        Vec::new()
    }

    /// Hot-reloads door settings (Valve mode, chain-wake cap, budget, RBAC) from a freshly
    /// validated `config`; takes effect for the next door evaluation. Returns `true` when a
    /// Valve was reloaded.
    fn reload_valve(&mut self, _config: &crate::config::GatewayConfig) -> bool {
        false
    }

    /// Takes an approved [`Action`] ready for re-evaluation and dispatch after event persistence.
    fn take_approved_action(&mut self, _action_id: &ActionId) -> Option<Action> {
        None
    }

    /// Takes a terminal `Refused` receipt generated by an owner denial.
    fn take_pending_denial_receipt(&mut self, _action_id: &ActionId) -> Option<Receipt> {
        None
    }
}

impl DoorController for NoopDoors {}

/// Wraps any [`Doors`] implementation (`NoopDoors`, test guards) as a [`DoorController`].
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

/// Concrete [`DoorController`] wrapping [`Valve`] with event-sourced `ApprovalManager` /
/// `DigestQueue` replay and decoupled out-of-band `I-HITL` `approval/resolve` execution.
pub struct ValveDoorController {
    valve: Valve,
    pending_events: Vec<EventRecord>,
    approved_actions: BTreeMap<ActionId, Action>,
    denial_receipts: BTreeMap<ActionId, Receipt>,
}

impl fmt::Debug for ValveDoorController {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ValveDoorController")
            .field("pending_approvals", &self.valve.approval_manager().pending().len())
            .field("state_epoch", &self.valve.snapshot().state_epoch)
            .finish()
    }
}

impl ValveDoorController {
    /// Creates a [`ValveDoorController`] around `valve`.
    #[must_use]
    pub fn new(valve: Valve) -> Self {
        Self {
            valve,
            pending_events: Vec::new(),
            approved_actions: BTreeMap::new(),
            denial_receipts: BTreeMap::new(),
        }
    }

    /// Returns a shared reference to the underlying [`Valve`].
    #[must_use]
    pub fn valve(&self) -> &Valve {
        &self.valve
    }

    /// Returns a mutable reference to the underlying [`Valve`].
    pub fn valve_mut(&mut self) -> &mut Valve {
        &mut self.valve
    }

    #[allow(clippy::too_many_arguments)]
    fn build_door_verdict_event(
        &self,
        now: Time,
        actor: ActorId,
        place: PlaceId,
        audience: Audience,
        taint: Taint,
        rule: impl Into<String>,
        verdict: impl Into<String>,
        payload: OpaquePayload,
    ) -> EventRecord {
        EventRecord::new_root(
            EventKind::DoorVerdict,
            now,
            actor.clone(),
            place,
            audience,
            OriginActor::new(actor),
            taint,
            NumericFeatures::default(),
            payload,
            Vec::new(),
            Some(EventDetail::DoorVerdict {
                door: DoorKind::Action,
                target_event_id: None,
                rule: rule.into(),
                verdict: verdict.into(),
            }),
        )
    }
}

impl Doors for ValveDoorController {
    fn inbound(&mut self, percept: &mut Percept) -> DoorDecision {
        self.valve.inbound(percept)
    }

    fn context(&mut self, plan: &mut ContextPlan) -> DoorDecision {
        self.valve.context(plan)
    }

    fn action(&mut self, action: &mut Action) -> DoorDecision {
        let grants_before = self.valve.approval_manager().grants().len();
        let decision = self.valve.action(action);
        let grants_after = self.valve.approval_manager().grants().len();
        let now = self.valve.snapshot().t_now;
        let place = action
            .places
            .first()
            .cloned()
            .unwrap_or_else(|| PlaceId::new("valve:action"));

        match &decision {
            DoorDecision::Hold { rule, reason }
                if reason == ValveReasonTag::PendingApproval.as_str()
                    || reason == ValveReasonTag::TaintBlocked.as_str() =>
            {
                if let Some(pending) = self
                    .valve
                    .approval_manager()
                    .pending()
                    .get(&action.action_id)
                    .cloned()
                {
                    let is_embodied = self
                        .valve
                        .approval_manager()
                        .pending_is_embodied(&action.action_id);
                    let log_rec = ApprovalLogRecord::Held {
                        pending,
                        taint: action.taint,
                        is_embodied,
                        action: Some(action.clone()),
                    };
                    if let Ok(bytes) = serde_json::to_vec(&log_rec) {
                        let ev = self.build_door_verdict_event(
                            now,
                            ActorId::new("valve"),
                            place,
                            action.audience.clone(),
                            action.taint,
                            rule.clone(),
                            format!("hold: {reason}"),
                            OpaquePayload::from_bytes(bytes),
                        );
                        self.pending_events.push(ev);
                    }
                }
            }
            DoorDecision::Allow | DoorDecision::Strip { .. } if grants_after < grants_before => {
                if let Some(h_act) = &action.h_act {
                    let log_rec = ApprovalLogRecord::GrantConsumed {
                        h_act: h_act.clone(),
                    };
                    if let Ok(bytes) = serde_json::to_vec(&log_rec) {
                        let ev = self.build_door_verdict_event(
                            now,
                            ActorId::new("valve"),
                            place,
                            action.audience.clone(),
                            Taint::Trusted,
                            "valve.approval.grant_consumed",
                            "consumed",
                            OpaquePayload::from_bytes(bytes),
                        );
                        self.pending_events.push(ev);
                    }
                }
            }
            _ => {}
        }

        decision
    }
}

impl DoorController for ValveDoorController {
    fn reload_valve(&mut self, config: &crate::config::GatewayConfig) -> bool {
        use monoagens_valve::ValveMode;
        let mut next = self.valve.config().clone();
        next.mode = match config.valve.mode.as_str() {
            "partial" => ValveMode::Partial,
            "off" => ValveMode::Off,
            _ => ValveMode::Enforcing,
        };
        next.max_chain_wakes = config.valve.max_chain_wakes;
        next.daily_token_budget = config.model.daily_token_budget;
        next.rbac = config.rbac.clone();
        self.valve.set_config(next);
        true
    }

    fn sync_runtime_state(
        &mut self,
        log_seq: LogSeq,
        now: Time,
        daily_tokens_used: u64,
        capabilities: &[CapabilityDescriptor],
    ) {
        let mut snap = self.valve.snapshot().clone();
        snap.log_seq = log_seq;
        snap.t_now = now;
        snap.daily_tokens_used = daily_tokens_used;
        for cap in capabilities {
            snap.register_capability(cap.clone());
        }
        self.valve.update_snapshot(snap);
    }

    fn sync_state_epoch(&mut self, state_epoch: u64) {
        self.valve.snapshot_mut().state_epoch = state_epoch;
    }

    fn replay_from_store_events(&mut self, events: &[EventRecord]) {
        self.valve.replay_from_events(events);
    }

    fn sync_item_audiences(&mut self, events: &[EventRecord]) {
        self.valve.sync_event_audiences(events);
    }

    fn resolve_approval(
        &mut self,
        params: &ApprovalResolveParams,
    ) -> Option<Result<ApprovalResolveResult, Waist2Error>> {
        let now = self.valve.snapshot().t_now;
        let owner_aud = self
            .valve
            .snapshot()
            .owner_actor_id
            .clone()
            .map(|o| Audience::new([o]))
            .unwrap_or_else(|| Audience::new([params.resolver.clone()]));
        match self.valve.resolve_approval(params) {
            Ok(outcome) => {
                let place = outcome
                    .pending
                    .places
                    .first()
                    .cloned()
                    .unwrap_or_else(|| PlaceId::new("valve:approval"));
                let payload = serde_json::to_vec(&outcome.log_record)
                    .map(OpaquePayload::from_bytes)
                    .unwrap_or_default();
                let verdict = if params.approved {
                    "approval_granted"
                } else {
                    "approval_denied"
                };
                let ev = self.build_door_verdict_event(
                    params.resolved_at_ms,
                    params.resolver.clone(),
                    place,
                    owner_aud,
                    Taint::Trusted,
                    "valve.approval.resolve",
                    verdict,
                    payload,
                );
                self.pending_events.push(ev);

                if params.approved {
                    if let Some(act) = outcome.action {
                        self.approved_actions
                            .insert(params.action_id.clone(), act);
                    }
                } else {
                    self.denial_receipts.insert(
                        params.action_id.clone(),
                        Receipt {
                            action_id: params.action_id.clone(),
                            capability_id: outcome.pending.capability_id.clone(),
                            outcome: ReceiptOutcome::Refused,
                            reason_code: Some("approval_denied".to_owned()),
                            detail: Some("denied by owner via out-of-band approval/resolve".to_owned()),
                            termination_reason: Some(TerminationReason::Normal),
                            progress: Some(0.0),
                            delivered_units: Some(0),
                            h_act: Some(params.h_act.clone()),
                            timestamp_ms: params.resolved_at_ms,
                        },
                    );
                }

                Some(Ok(ApprovalResolveResult { applied: true }))
            }
            Err(err) => {
                let ev = self.build_door_verdict_event(
                    now,
                    params.resolver.clone(),
                    PlaceId::new("valve:approval"),
                    owner_aud,
                    Taint::Untrusted,
                    "valve.approval.resolve_rejected",
                    err.reason_str(),
                    OpaquePayload::default(),
                );
                self.pending_events.push(ev);
                Some(Err(Waist2Error::invalid_receipt_transition(err.reason_str())))
            }
        }
    }

    fn record_in_band_rejection_audit(&mut self, params: &ApprovalResolveParams, detail: &str) {
        self.valve.approval_manager_mut().record_spoof_rejection();
        let now = self.valve.snapshot().t_now;
        let aud = self
            .valve
            .snapshot()
            .owner_actor_id
            .clone()
            .map(|o| Audience::new([o]))
            .unwrap_or_else(|| Audience::new([params.resolver.clone()]));
        let ev = self.build_door_verdict_event(
            now,
            params.resolver.clone(),
            PlaceId::new("valve:approval"),
            aud,
            Taint::Untrusted,
            "valve.approval.resolve_rejected",
            detail,
            OpaquePayload::default(),
        );
        self.pending_events.push(ev);
    }

    fn take_pending_approval_events(&mut self) -> Vec<EventRecord> {
        std::mem::take(&mut self.pending_events)
    }

    fn take_approved_action(&mut self, action_id: &ActionId) -> Option<Action> {
        self.approved_actions.remove(action_id)
    }

    fn take_pending_denial_receipt(&mut self, action_id: &ActionId) -> Option<Receipt> {
        self.denial_receipts.remove(action_id)
    }
}

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
