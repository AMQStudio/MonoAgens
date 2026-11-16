//! Scripted `TestBody` wired strictly through the canonical `monoagens.body.v1` JSON-RPC codec.
//!
//! Zero parallel in-memory structs and zero direct calls into `Core`. Every RPC crosses
//! `Waist2Request` / `Waist2Response` JSON serialization using `monoagens_protocol` types.

use std::collections::BTreeMap;
use std::fs;
use std::io::{self, BufRead, Write};
use std::path::Path;

use monoagens_protocol::{
    ActionCancelParams, ActionCancelResult, ActionExecuteParams, ActionExecuteResult, ActionId,
    ActionReceiptParams, ActionReceiptResult, ApprovalResolveParams, ApprovalResolveResult,
    BlobGetParams, BlobGetResult, BlobPutParams, BlobPutResult, BodyDiscoverParams,
    BodyDiscoverResult, BodyHeartbeatParams, BodyHeartbeatResult, BodyId, BodyShutdownParams,
    BodyShutdownResult, BodyStatus, CURRENT_PROTOCOL_VERSION, CapabilityDescriptor, CapabilityId,
    ChannelPrediction, JSONRPC_VERSION, OpaquePayload, Percept, PerceptEmitParams,
    PerceptEmitResult, PlaceId, QuestionAnswerParams, QuestionAnswerResult, Receipt,
    ReceiptOutcome, RpcId, StateQueryParams, StateQueryResult, StateQueryStatus, TerminationReason,
    Time, Waist2Error, Waist2Headers, Waist2Method, Waist2Request, Waist2Response,
    sort_capabilities,
};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::scenario::{ScriptedActionRule, ScriptedFollowUpPercept, TestBodySpec};

/// Outbound JSON-RPC 2.0 wire frame scheduled by a [`TestBody`] relative to action dispatch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduledWireRpc {
    pub delay_ms: u64,
    pub json_rpc: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredBlob {
    bytes: Vec<u8>,
    sha256: String,
    expires_at_ms: Time,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CachedCall {
    fingerprint: String,
    result_raw_json: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct PersistedTestBodyState {
    idempotency_cache: BTreeMap<String, CachedCall>,
    journal: BTreeMap<ActionId, StateQueryStatus>,
    executions_per_action: BTreeMap<ActionId, usize>,
    total_executions: usize,
    blobs: BTreeMap<String, StoredBlob>,
}

/// Computes lowercase hex SHA-256 digest of `bytes`.
#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(&mut out, "{byte:02x}");
    }
    out
}

/// Deterministic scripted body communicating strictly over `monoagens.body.v1` JSON-RPC 2.0.
#[derive(Debug, Clone)]
pub struct TestBody {
    body_id: BodyId,
    places: Vec<PlaceId>,
    capabilities: Vec<CapabilityDescriptor>,
    status: BodyStatus,
    current_time: Time,
    next_rpc_id: i64,
    idempotency_cache: BTreeMap<String, CachedCall>,
    journal: BTreeMap<ActionId, StateQueryStatus>,
    executions_per_action: BTreeMap<ActionId, usize>,
    total_executions: usize,
    blobs: BTreeMap<String, StoredBlob>,
    emitted_percepts: Vec<Percept>,
    answered_questions: Vec<QuestionAnswerParams>,
    outbound_queue: Vec<ScheduledWireRpc>,
    action_scripts: Vec<ScriptedActionRule>,
    shutdown_requested: bool,
    shutdown_reason: Option<String>,
}

impl TestBody {
    /// Creates a `TestBody` from a declarative [`TestBodySpec`], sorting capabilities and places.
    #[must_use]
    pub fn from_spec(spec: TestBodySpec) -> Self {
        let mut places = spec.places;
        places.sort();
        places.dedup();

        let mut capabilities = spec.capabilities;
        sort_capabilities(&mut capabilities);

        Self {
            body_id: spec.body_id,
            places,
            capabilities,
            status: BodyStatus::Ready,
            current_time: Time::from_millis(0),
            next_rpc_id: 1,
            idempotency_cache: BTreeMap::new(),
            journal: BTreeMap::new(),
            executions_per_action: BTreeMap::new(),
            total_executions: 0,
            blobs: BTreeMap::new(),
            emitted_percepts: Vec::new(),
            answered_questions: Vec::new(),
            outbound_queue: Vec::new(),
            action_scripts: spec.action_scripts,
            shutdown_requested: false,
            shutdown_reason: None,
        }
    }

    /// Returns this body's identifier.
    #[must_use]
    pub fn body_id(&self) -> &BodyId {
        &self.body_id
    }

    /// Returns this body's sorted declared places.
    #[must_use]
    pub fn places(&self) -> &[PlaceId] {
        &self.places
    }

    /// Returns this body's sorted capabilities.
    #[must_use]
    pub fn capabilities(&self) -> &[CapabilityDescriptor] {
        &self.capabilities
    }

    /// Updates the body's deterministic virtual clock view (never reads wall-clock time).
    pub fn set_virtual_time(&mut self, now: Time) {
        self.current_time = now;
    }

    /// Returns how many times `action_id` actually executed (excluding idempotent replays).
    #[must_use]
    pub fn execution_count(&self, action_id: &ActionId) -> usize {
        self.executions_per_action
            .get(action_id)
            .copied()
            .unwrap_or(0)
    }

    /// Returns total non-replayed action executions performed by this body.
    #[must_use]
    pub const fn total_executions(&self) -> usize {
        self.total_executions
    }

    /// Returns all `percept/emit` payloads accepted over the wire.
    #[must_use]
    pub fn emitted_percepts(&self) -> &[Percept] {
        &self.emitted_percepts
    }

    /// Returns all `question/answer` payloads accepted over the wire.
    #[must_use]
    pub fn answered_questions(&self) -> &[QuestionAnswerParams] {
        &self.answered_questions
    }

    /// Returns `true` if `body/shutdown` was requested.
    #[must_use]
    pub const fn is_shutdown(&self) -> bool {
        self.shutdown_requested
    }

    /// Returns the optional shutdown reason.
    #[must_use]
    pub fn shutdown_reason(&self) -> Option<&str> {
        self.shutdown_reason.as_deref()
    }

    /// Drains all outbound JSON-RPC 2.0 wire frames (`action/receipt`, `percept/emit`).
    pub fn drain_outbound_json(&mut self) -> Vec<ScheduledWireRpc> {
        std::mem::take(&mut self.outbound_queue)
    }

    /// Serializes a canonical `percept/emit` JSON-RPC 2.0 request using `PerceptEmitParams`.
    pub fn encode_percept_emit_rpc(
        &mut self,
        idempotency_key: impl Into<String>,
        percept: Percept,
    ) -> Result<String, serde_json::Error> {
        let id = self.alloc_rpc_id();
        let params = PerceptEmitParams::from_percept(Waist2Headers::new(idempotency_key), percept);
        let req = Waist2Request::new(Some(id), Waist2Method::PerceptEmit, params);
        serde_json::to_string(&req)
    }

    /// Round-trips a typed Waist-2 request through JSON serialization, `handle_json_rpc`, and
    /// JSON response deserialization (`0` in-memory bypass structs).
    pub fn call<P, R>(
        &mut self,
        id: RpcId,
        method: Waist2Method,
        params: P,
    ) -> Result<Result<R, Waist2Error>, serde_json::Error>
    where
        P: Serialize,
        R: DeserializeOwned,
    {
        let req = Waist2Request::new(Some(id), method, params);
        let req_line = serde_json::to_string(&req)?;
        let resp_line = self.handle_json_rpc(&req_line);
        let resp: Waist2Response<R> = serde_json::from_str(&resp_line)?;
        if let Some(err) = resp.error {
            Ok(Err(err))
        } else if let Some(res) = resp.result {
            Ok(Ok(res))
        } else {
            Ok(Err(Waist2Error::invalid_receipt_transition(
                "JSON-RPC response contained neither result nor error",
            )))
        }
    }

    /// Processes a single NDJSON JSON-RPC 2.0 request line and returns the serialized JSON-RPC 2.0 response line.
    pub fn handle_json_rpc(&mut self, line: &str) -> String {
        let req: Waist2Request<Value> = match serde_json::from_str(line) {
            Ok(req) => req,
            Err(err) => {
                let resp: Waist2Response<Value> = Waist2Response::err(
                    None,
                    Waist2Error::invalid_receipt_transition(format!(
                        "invalid JSON-RPC request: {err}"
                    )),
                );
                return serde_json::to_string(&resp).unwrap_or_else(|_| {
                    "{\"jsonrpc\":\"2.0\",\"id\":null,\"error\":{\"code\":-32003,\"message\":\"ERR_INVALID_RECEIPT_TRANSITION: serialize failure\"}}".to_owned()
                });
            }
        };

        let id = req.id.clone();
        if req.jsonrpc != JSONRPC_VERSION {
            let resp: Waist2Response<Value> = Waist2Response::err(
                id,
                Waist2Error::invalid_receipt_transition(format!(
                    "expected jsonrpc '{JSONRPC_VERSION}', got '{}'",
                    req.jsonrpc
                )),
            );
            return serde_json::to_string(&resp).unwrap_or_default();
        }

        match self.dispatch_request(req) {
            Ok(raw_result_json) => {
                if let Some(rpc_id) = id {
                    let id_json =
                        serde_json::to_string(&rpc_id).unwrap_or_else(|_| "null".to_owned());
                    format!("{{\"jsonrpc\":\"2.0\",\"id\":{id_json},\"result\":{raw_result_json}}}")
                } else {
                    format!("{{\"jsonrpc\":\"2.0\",\"id\":null,\"result\":{raw_result_json}}}")
                }
            }
            Err(err) => {
                let resp: Waist2Response<Value> = Waist2Response::err(id, err);
                serde_json::to_string(&resp).unwrap_or_else(|e| {
                    let fallback: Waist2Response<Value> = Waist2Response::err(
                        None,
                        Waist2Error::invalid_receipt_transition(format!(
                            "response encode error: {e}"
                        )),
                    );
                    serde_json::to_string(&fallback).unwrap_or_default()
                })
            }
        }
    }

    /// Runs an NDJSON JSON-RPC 2.0 loop over `reader` and `writer` until EOF.
    pub fn run_stdio(&mut self, reader: impl BufRead, writer: impl Write) -> io::Result<()> {
        self.run_stdio_with_state_dir(None, reader, writer)
    }

    /// Runs an NDJSON JSON-RPC 2.0 loop over `reader` and `writer`, persisting crash-recovery
    /// journal and idempotency state under `state_dir` when provided.
    pub fn run_stdio_with_state_dir(
        &mut self,
        state_dir: Option<&Path>,
        reader: impl BufRead,
        mut writer: impl Write,
    ) -> io::Result<()> {
        let state_file = state_dir.map(|dir| dir.join("test_body_state.json"));
        if let Some(ref path) = state_file
            && let Ok(raw) = fs::read(path)
            && let Ok(loaded) = serde_json::from_slice::<PersistedTestBodyState>(&raw)
        {
            self.idempotency_cache = loaded.idempotency_cache;
            self.journal = loaded.journal;
            self.executions_per_action = loaded.executions_per_action;
            self.total_executions = loaded.total_executions;
            self.blobs = loaded.blobs;
        }

        for line in reader.lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            let out = self.handle_json_rpc(&line);
            if let Some(ref path) = state_file {
                self.persist_state(path)?;
            }
            writeln!(writer, "{out}")?;
            writer.flush()?;
        }
        Ok(())
    }

    fn persist_state(&self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let snapshot = PersistedTestBodyState {
            idempotency_cache: self.idempotency_cache.clone(),
            journal: self.journal.clone(),
            executions_per_action: self.executions_per_action.clone(),
            total_executions: self.total_executions,
            blobs: self.blobs.clone(),
        };
        let bytes = serde_json::to_vec(&snapshot)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
        let tmp = path.with_extension("tmp");
        fs::write(&tmp, bytes)?;
        fs::rename(&tmp, path)?;
        Ok(())
    }

    fn alloc_rpc_id(&mut self) -> RpcId {
        let id = RpcId::Int(self.next_rpc_id);
        self.next_rpc_id = self.next_rpc_id.saturating_add(1);
        id
    }

    fn dispatch_request(&mut self, req: Waist2Request<Value>) -> Result<String, Waist2Error> {
        let headers_val = req
            .params
            .get("headers")
            .cloned()
            .ok_or_else(|| Waist2Error::invalid_receipt_transition("missing params.headers"))?;
        let headers: Waist2Headers = serde_json::from_value(headers_val).map_err(|e| {
            Waist2Error::invalid_receipt_transition(format!("invalid headers: {e}"))
        })?;
        headers.validate_version(CURRENT_PROTOCOL_VERSION)?;

        let fingerprint = compute_request_fingerprint(req.method, &req.params);
        if let Some(cached) = self.idempotency_cache.get(&headers.idempotency_key) {
            if cached.fingerprint == fingerprint {
                return Ok(cached.result_raw_json.clone());
            }
            return Err(Waist2Error::duplicate_idempotency(&headers.idempotency_key));
        }

        let result_raw_json = match req.method {
            Waist2Method::BodyDiscover => {
                let params: BodyDiscoverParams = parse_params(&req.params)?;
                let res = self.on_discover(params);
                encode_result_json(&res)?
            }
            Waist2Method::BodyHeartbeat => {
                let params: BodyHeartbeatParams = parse_params(&req.params)?;
                let res = self.on_heartbeat(params);
                encode_result_json(&res)?
            }
            Waist2Method::BodyShutdown => {
                let params: BodyShutdownParams = parse_params(&req.params)?;
                let res = self.on_shutdown(params);
                encode_result_json(&res)?
            }
            Waist2Method::PerceptEmit => {
                let params: PerceptEmitParams = parse_params(&req.params)?;
                let res = self.on_percept_emit(params)?;
                encode_result_json(&res)?
            }
            Waist2Method::ActionExecute => {
                let params: ActionExecuteParams = parse_params(&req.params)?;
                let res = self.on_action_execute(params)?;
                encode_result_json(&res)?
            }
            Waist2Method::ActionCancel => {
                let params: ActionCancelParams = parse_params(&req.params)?;
                let res = self.on_action_cancel(params);
                encode_result_json(&res)?
            }
            Waist2Method::ActionReceipt => {
                let params: ActionReceiptParams = parse_params(&req.params)?;
                let res = self.on_action_receipt(params)?;
                encode_result_json(&res)?
            }
            Waist2Method::ApprovalResolve => {
                let params: ApprovalResolveParams = parse_params(&req.params)?;
                let res = self.on_approval_resolve(params)?;
                encode_result_json(&res)?
            }
            Waist2Method::QuestionAnswer => {
                let params: QuestionAnswerParams = parse_params(&req.params)?;
                let res = self.on_question_answer(params)?;
                encode_result_json(&res)?
            }
            Waist2Method::StateQuery => {
                let params: StateQueryParams = parse_params(&req.params)?;
                let res = self.on_state_query(params);
                encode_result_json(&res)?
            }
            Waist2Method::BlobPut => {
                let params: BlobPutParams = parse_params(&req.params)?;
                let res = self.on_blob_put(params)?;
                encode_result_json(&res)?
            }
            Waist2Method::BlobGet => {
                let params: BlobGetParams = parse_params(&req.params)?;
                let res = self.on_blob_get(params)?;
                encode_result_json(&res)?
            }
        };

        self.idempotency_cache.insert(
            headers.idempotency_key,
            CachedCall {
                fingerprint,
                result_raw_json: result_raw_json.clone(),
            },
        );
        Ok(result_raw_json)
    }

    fn on_discover(&self, _params: BodyDiscoverParams) -> BodyDiscoverResult {
        let mut caps = self.capabilities.clone();
        sort_capabilities(&mut caps);
        let mut places = self.places.clone();
        places.sort();
        places.dedup();
        BodyDiscoverResult {
            body_id: self.body_id.clone(),
            protocol_version: CURRENT_PROTOCOL_VERSION,
            capabilities: caps,
            places,
        }
    }

    fn on_heartbeat(&mut self, params: BodyHeartbeatParams) -> BodyHeartbeatResult {
        self.status = params.status;
        if params.timestamp_ms > self.current_time {
            self.current_time = params.timestamp_ms;
        }
        BodyHeartbeatResult {
            acknowledged_at_ms: params.timestamp_ms,
        }
    }

    fn on_shutdown(&mut self, params: BodyShutdownParams) -> BodyShutdownResult {
        self.shutdown_requested = true;
        self.shutdown_reason = params.reason;
        self.status = BodyStatus::Stopped;
        BodyShutdownResult { accepted: true }
    }

    fn on_percept_emit(
        &mut self,
        params: PerceptEmitParams,
    ) -> Result<PerceptEmitResult, Waist2Error> {
        let percept = params.into_percept(CURRENT_PROTOCOL_VERSION)?;
        if !self.places.is_empty() && !self.places.contains(&percept.place) {
            return Err(Waist2Error::invalid_receipt_transition(format!(
                "percept place '{}' is not owned by body '{}'",
                percept.place, self.body_id
            )));
        }
        if percept.timestamp_ms > self.current_time {
            self.current_time = percept.timestamp_ms;
        }
        self.emitted_percepts.push(percept);
        Ok(PerceptEmitResult { accepted: true })
    }

    fn on_action_execute(
        &mut self,
        params: ActionExecuteParams,
    ) -> Result<ActionExecuteResult, Waist2Error> {
        if params.audience.is_empty() {
            return Err(Waist2Error::undeclared_audience());
        }
        let Some(cap) = self
            .capabilities
            .iter()
            .find(|cap| cap.id == params.capability_id)
        else {
            return Err(Waist2Error::unknown_capability(&params.capability_id));
        };

        let place_allowed = !params.places.is_empty()
            && params.places.iter().all(|p| {
                self.places.contains(p) && (cap.places.is_empty() || cap.places.contains(p))
            });
        if !place_allowed {
            let receipt = Receipt {
                action_id: params.action_id,
                capability_id: params.capability_id,
                outcome: ReceiptOutcome::Refused,
                reason_code: Some("ERR_UNKNOWN_PLACE".to_owned()),
                detail: Some("action targets a place not owned by this body/capability".to_owned()),
                termination_reason: Some(TerminationReason::Normal),
                progress: Some(0.0),
                delivered_units: Some(0),
                h_act: params.h_act,
                timestamp_ms: self.current_time,
            };
            return Ok(ActionExecuteResult {
                accepted: false,
                receipt: Some(receipt),
            });
        }

        // Single-receipt / single-execution invariant even if retried with a different idempotency_key.
        if let Some(existing) = self.journal.get(&params.action_id) {
            return match existing {
                StateQueryStatus::Terminal { receipt } => Ok(ActionExecuteResult {
                    accepted: true,
                    receipt: Some(receipt.clone()),
                }),
                StateQueryStatus::InFlight { .. } => Ok(ActionExecuteResult {
                    accepted: true,
                    receipt: None,
                }),
                StateQueryStatus::Unknown => Ok(ActionExecuteResult {
                    accepted: true,
                    receipt: None,
                }),
            };
        }

        *self
            .executions_per_action
            .entry(params.action_id.clone())
            .or_insert(0) += 1;
        self.total_executions += 1;

        let matched_rule = self.find_matching_rule(&params).cloned();
        let (
            outcome,
            receipt_delay_ms,
            termination_reason,
            progress,
            delivered_units,
            reason_code,
            detail,
            follow_ups,
        ) = match matched_rule {
            Some(rule) => (
                rule.outcome,
                rule.receipt_delay_ms,
                rule.termination_reason,
                rule.progress,
                rule.delivered_units,
                rule.reason_code,
                rule.detail,
                rule.follow_up_percepts,
            ),
            None => (
                ReceiptOutcome::Done,
                0,
                Some(TerminationReason::Normal),
                Some(1.0),
                Some(1),
                None,
                None,
                Vec::new(),
            ),
        };

        let receipt = Receipt {
            action_id: params.action_id.clone(),
            capability_id: params.capability_id.clone(),
            outcome,
            reason_code,
            detail,
            termination_reason,
            progress,
            delivered_units,
            h_act: params.h_act.clone(),
            timestamp_ms: self.current_time.saturating_add_ms(receipt_delay_ms),
        };
        receipt.validate()?;

        self.queue_follow_up_percepts(&params.action_id, &follow_ups)?;

        if receipt_delay_ms == 0 {
            self.journal.insert(
                params.action_id.clone(),
                StateQueryStatus::Terminal {
                    receipt: receipt.clone(),
                },
            );
            self.queue_action_receipt_rpc(0, &params.headers.idempotency_key, receipt.clone())?;
            Ok(ActionExecuteResult {
                accepted: true,
                receipt: Some(receipt),
            })
        } else {
            self.journal
                .insert(params.action_id, StateQueryStatus::InFlight { progress });
            self.queue_action_receipt_rpc(
                receipt_delay_ms,
                &params.headers.idempotency_key,
                receipt,
            )?;
            Ok(ActionExecuteResult {
                accepted: true,
                receipt: None,
            })
        }
    }

    fn on_action_cancel(&mut self, params: ActionCancelParams) -> ActionCancelResult {
        match self.journal.get(&params.action_id) {
            Some(StateQueryStatus::InFlight { progress }) => {
                let receipt = Receipt {
                    action_id: params.action_id.clone(),
                    capability_id: CapabilityId::new("unknown"),
                    outcome: ReceiptOutcome::Failed,
                    reason_code: Some("canceled_by_core".to_owned()),
                    detail: params.reason,
                    termination_reason: Some(TerminationReason::CanceledByCore),
                    progress: *progress,
                    delivered_units: Some(0),
                    h_act: None,
                    timestamp_ms: self.current_time,
                };
                self.journal.insert(
                    params.action_id,
                    StateQueryStatus::Terminal {
                        receipt: receipt.clone(),
                    },
                );
                let _ = self.queue_action_receipt_rpc(
                    0,
                    &format!("cancel-{}", params.headers.idempotency_key),
                    receipt.clone(),
                );
                ActionCancelResult {
                    cancelled: true,
                    receipt: Some(receipt),
                }
            }
            Some(StateQueryStatus::Terminal { .. } | StateQueryStatus::Unknown) | None => {
                ActionCancelResult {
                    cancelled: false,
                    receipt: None,
                }
            }
        }
    }

    fn on_action_receipt(
        &mut self,
        params: ActionReceiptParams,
    ) -> Result<ActionReceiptResult, Waist2Error> {
        params.receipt.validate()?;
        if let Some(StateQueryStatus::Terminal { receipt: existing }) =
            self.journal.get(&params.receipt.action_id)
            && existing.outcome != ReceiptOutcome::Held
        {
            return Err(Waist2Error::invalid_receipt_transition(format!(
                "action '{}' already settled with {:?}",
                params.receipt.action_id, existing.outcome
            )));
        }
        self.journal.insert(
            params.receipt.action_id.clone(),
            StateQueryStatus::Terminal {
                receipt: params.receipt,
            },
        );
        Ok(ActionReceiptResult { recorded: true })
    }

    fn on_approval_resolve(
        &mut self,
        params: ApprovalResolveParams,
    ) -> Result<ApprovalResolveResult, Waist2Error> {
        let Some(StateQueryStatus::Terminal { receipt }) = self.journal.get(&params.action_id)
        else {
            return Ok(ApprovalResolveResult { applied: false });
        };
        if receipt.outcome != ReceiptOutcome::Held {
            return Err(Waist2Error::invalid_receipt_transition(format!(
                "action '{}' is {:?}, not held",
                params.action_id, receipt.outcome
            )));
        }
        if let Some(expected_h) = &receipt.h_act
            && expected_h != &params.h_act
        {
            return Err(Waist2Error::invalid_receipt_transition(
                "approval h_act mismatch",
            ));
        }

        let updated = Receipt {
            action_id: params.action_id.clone(),
            capability_id: receipt.capability_id.clone(),
            outcome: if params.approved {
                ReceiptOutcome::Done
            } else {
                ReceiptOutcome::Refused
            },
            reason_code: Some(if params.approved {
                "approved".to_owned()
            } else {
                "rejected".to_owned()
            }),
            detail: None,
            termination_reason: Some(TerminationReason::Normal),
            progress: if params.approved {
                Some(1.0)
            } else {
                Some(0.0)
            },
            delivered_units: if params.approved { Some(1) } else { Some(0) },
            h_act: Some(params.h_act),
            timestamp_ms: params.resolved_at_ms,
        };
        self.journal.insert(
            params.action_id,
            StateQueryStatus::Terminal {
                receipt: updated.clone(),
            },
        );
        self.queue_action_receipt_rpc(
            0,
            &format!("approval-{}", params.headers.idempotency_key),
            updated,
        )?;
        Ok(ApprovalResolveResult { applied: true })
    }

    fn on_question_answer(
        &mut self,
        params: QuestionAnswerParams,
    ) -> Result<QuestionAnswerResult, Waist2Error> {
        if params.audience.is_empty() {
            return Err(Waist2Error::undeclared_audience());
        }
        if params.timestamp_ms > self.current_time {
            self.current_time = params.timestamp_ms;
        }
        self.answered_questions.push(params);
        Ok(QuestionAnswerResult { accepted: true })
    }

    fn on_state_query(&self, params: StateQueryParams) -> StateQueryResult {
        let status = self
            .journal
            .get(&params.action_id)
            .cloned()
            .unwrap_or(StateQueryStatus::Unknown);
        StateQueryResult {
            action_id: params.action_id,
            status,
        }
    }

    fn on_blob_put(&mut self, params: BlobPutParams) -> Result<BlobPutResult, Waist2Error> {
        let bytes = match params.inline_bytes {
            Some(payload) => payload.0,
            None if params.byte_len == 0 => Vec::new(),
            None => {
                return Err(Waist2Error::hash_mismatch(
                    &params.sha256,
                    "missing_inline_bytes",
                ));
            }
        };

        if bytes.len() as u64 != params.byte_len {
            return Err(Waist2Error::hash_mismatch(
                &params.sha256,
                &format!("byte_len_mismatch:{}", bytes.len()),
            ));
        }

        let actual_sha = sha256_hex(&bytes);
        if !actual_sha.eq_ignore_ascii_case(&params.sha256) {
            return Err(Waist2Error::hash_mismatch(&params.sha256, &actual_sha));
        }

        let blob_ref = format!("blob://sha256/{actual_sha}");
        let expires_at_ms = self.current_time.saturating_add_ms(60_000);
        self.blobs.insert(
            blob_ref.clone(),
            StoredBlob {
                bytes,
                sha256: actual_sha.clone(),
                expires_at_ms,
            },
        );
        Ok(BlobPutResult {
            blob_ref,
            sha256: actual_sha,
            expires_at_ms,
        })
    }

    fn on_blob_get(&self, params: BlobGetParams) -> Result<BlobGetResult, Waist2Error> {
        let Some(stored) = self.blobs.get(&params.blob_ref) else {
            return Err(Waist2Error::invalid_receipt_transition(format!(
                "blob '{}' not found",
                params.blob_ref
            )));
        };
        if self.current_time > stored.expires_at_ms {
            return Err(Waist2Error::invalid_receipt_transition(format!(
                "blob '{}' expired at {}",
                params.blob_ref, stored.expires_at_ms
            )));
        }
        let actual_sha = sha256_hex(&stored.bytes);
        if actual_sha != stored.sha256 {
            return Err(Waist2Error::hash_mismatch(&stored.sha256, &actual_sha));
        }
        if let Some(expected) = &params.expected_sha256
            && !actual_sha.eq_ignore_ascii_case(expected)
        {
            return Err(Waist2Error::hash_mismatch(expected, &actual_sha));
        }

        Ok(BlobGetResult {
            blob_ref: params.blob_ref,
            sha256: actual_sha,
            byte_len: stored.bytes.len() as u64,
            bytes: Some(OpaquePayload::from_bytes(stored.bytes.clone())),
        })
    }

    fn find_matching_rule(&self, params: &ActionExecuteParams) -> Option<&ScriptedActionRule> {
        let payload_text = String::from_utf8_lossy(params.payload.as_bytes());
        self.action_scripts.iter().find(|rule| {
            rule.capability_id == params.capability_id
                && rule
                    .place
                    .as_ref()
                    .is_none_or(|p| params.places.contains(p))
                && rule
                    .payload_contains
                    .as_ref()
                    .is_none_or(|needle| payload_text.contains(needle.as_str()))
        })
    }

    fn queue_action_receipt_rpc(
        &mut self,
        delay_ms: u64,
        key_prefix: &str,
        receipt: Receipt,
    ) -> Result<(), Waist2Error> {
        let id = self.alloc_rpc_id();
        let req = Waist2Request::new(
            Some(id),
            Waist2Method::ActionReceipt,
            ActionReceiptParams {
                headers: Waist2Headers::new(format!("rcpt-{key_prefix}-{}", receipt.action_id)),
                receipt,
            },
        );
        let json_rpc = serde_json::to_string(&req).map_err(|e| {
            Waist2Error::invalid_receipt_transition(format!("encode action/receipt failed: {e}"))
        })?;
        self.outbound_queue
            .push(ScheduledWireRpc { delay_ms, json_rpc });
        Ok(())
    }

    fn queue_follow_up_percepts(
        &mut self,
        action_id: &ActionId,
        follow_ups: &[ScriptedFollowUpPercept],
    ) -> Result<(), Waist2Error> {
        for (idx, item) in follow_ups.iter().enumerate() {
            let ts = self.current_time.saturating_add_ms(item.delay_ms);
            let mut percept = Percept::minimal(
                item.actor.clone(),
                item.place.clone(),
                item.audience.clone(),
                ts,
                OpaquePayload::from_bytes(item.payload.clone()),
            );
            percept.channel = item.channel.clone();
            percept.delta_obs = item.delta_obs;
            if let Some(channel) = &item.channel {
                percept.channel_prediction = Some(ChannelPrediction {
                    channel: channel.clone(),
                    delta_obs: item.delta_obs,
                    delta_pred: None,
                });
            }
            percept.addressees = item.addressees.clone();
            percept.named = item.named.clone();
            percept.dialogue_act_gap = item.dialogue_act_gap;

            let json_rpc = self
                .encode_percept_emit_rpc(format!("followup-{action_id}-{idx}"), percept)
                .map_err(|e| {
                    Waist2Error::invalid_receipt_transition(format!(
                        "encode follow-up percept/emit failed: {e}"
                    ))
                })?;
            self.outbound_queue.push(ScheduledWireRpc {
                delay_ms: item.delay_ms,
                json_rpc,
            });
        }
        Ok(())
    }
}

fn parse_params<P: DeserializeOwned>(params: &Value) -> Result<P, Waist2Error> {
    serde_json::from_value(params.clone())
        .map_err(|e| Waist2Error::invalid_receipt_transition(format!("invalid params: {e}")))
}

fn encode_result_json<R: Serialize>(result: &R) -> Result<String, Waist2Error> {
    serde_json::to_string(result)
        .map_err(|e| Waist2Error::invalid_receipt_transition(format!("encode result failed: {e}")))
}

fn compute_request_fingerprint(method: Waist2Method, params: &Value) -> String {
    let mut sanitized = params.clone();
    if let Some(obj) = sanitized.as_object_mut()
        && let Some(headers) = obj.get_mut("headers").and_then(Value::as_object_mut)
    {
        headers.remove("traceparent");
    }
    format!(
        "{}:{}",
        method.as_str(),
        serde_json::to_string(&sanitized).unwrap_or_default()
    )
}
