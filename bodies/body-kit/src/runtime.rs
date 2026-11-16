//! Persistent Waist-2 (`monoagens.body.v1`) body runner built atop `monoagens_body_sdk`.
//!
//! Adds atomic file-backed `MONOAGENS_BODY_STATE_DIR` persistence for `idempotency_key`
//! deduplication, `state/query` crash recovery (`B-MUST-03`), `blob/put`/`blob/get`,
//! and strict audience/place validation (`-32006 ERR_UNDECLARED_AUDIENCE`).

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};

use monoagens_body_sdk::{
    ActionCancelParams, ActionExecuteParams, ActionExecuteResult, ActionId, ActionReceiptParams,
    ActionReceiptResult, ApprovalResolveParams, Audience, BlobGetParams, BlobGetResult,
    BlobPutParams, BlobPutResult, BodyDiscoverParams, BodyHandler, BodyHeartbeatParams,
    BodyHeartbeatResult, BodyShutdownParams, BodyStatus, CURRENT_PROTOCOL_VERSION, CapabilityId,
    CapabilitySchemaHash, JSONRPC_VERSION, OpaquePayload, PerceptEmitParams, QuestionAnswerParams,
    Receipt, ReceiptOutcome, StateQueryParams, StateQueryResult, StateQueryStatus,
    TerminationReason, Time, Waist2Error, Waist2Headers, Waist2Method, Waist2Request,
    Waist2Response, sha256_hex, verify_blob_get, verify_blob_put,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

const STATE_FILENAME: &str = "waist2_body_state.json";

/// Reads `MONOAGENS_BODY_STATE_DIR` if set and non-empty.
#[must_use]
pub fn load_state_dir_from_env() -> Option<PathBuf> {
    env::var_os("MONOAGENS_BODY_STATE_DIR")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

/// Computes deterministic `"sha256:<hex>"` hash for a capability's JSON schema/metadata.
#[must_use]
pub fn capability_schema_hash(schema: &Value) -> CapabilitySchemaHash {
    let canonical = canonical_json(schema);
    let encoded = serde_json::to_vec(&canonical).unwrap_or_default();
    CapabilitySchemaHash::new(format!("sha256:{}", sha256_hex(&encoded)))
}

fn canonical_json(val: &Value) -> Value {
    match val {
        Value::Object(map) => {
            let mut entries: Vec<(&String, &Value)> = map.iter().collect();
            entries.sort_by_key(|(a, _)| *a);
            let mut sorted = serde_json::Map::new();
            for (k, v) in entries {
                sorted.insert(k.clone(), canonical_json(v));
            }
            Value::Object(sorted)
        }
        Value::Array(items) => Value::Array(items.iter().map(canonical_json).collect()),
        other => other.clone(),
    }
}

/// Constructs a terminal [`Receipt`] with Option A fields.
#[must_use]
#[allow(clippy::too_many_arguments)]
pub fn make_terminal_receipt(
    action_id: ActionId,
    capability_id: CapabilityId,
    outcome: ReceiptOutcome,
    reason_code: Option<String>,
    detail: Option<String>,
    termination_reason: Option<TerminationReason>,
    progress: Option<f32>,
    delivered_units: Option<u64>,
    h_act: Option<monoagens_body_sdk::HAct>,
    timestamp_ms: Time,
) -> Receipt {
    Receipt {
        action_id,
        capability_id,
        outcome,
        reason_code,
        detail,
        termination_reason,
        progress,
        delivered_units,
        h_act,
        timestamp_ms,
    }
}

/// Constructs a refused [`Receipt`] with `progress = 0.0` and `delivered_units = 0`.
#[must_use]
pub fn make_refused_receipt(
    action_id: ActionId,
    capability_id: CapabilityId,
    reason_code: impl Into<String>,
    detail: impl Into<String>,
    h_act: Option<monoagens_body_sdk::HAct>,
    timestamp_ms: Time,
) -> Receipt {
    make_terminal_receipt(
        action_id,
        capability_id,
        ReceiptOutcome::Refused,
        Some(reason_code.into()),
        Some(detail.into()),
        Some(TerminationReason::Normal),
        Some(0.0),
        Some(0),
        h_act,
        timestamp_ms,
    )
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct PersistedState {
    idempotency_cache: BTreeMap<String, Value>,
    journal: BTreeMap<String, StateQueryStatus>,
    blobs: BTreeMap<String, Vec<u8>>,
}

/// Persistent Waist-2 runtime wrapping [`BodyHandler`] with atomic crash-journal state.
pub struct PersistentBodyRuntime {
    state_file: Option<PathBuf>,
    state: PersistedState,
    status: BodyStatus,
}

impl PersistentBodyRuntime {
    /// Creates a runtime optionally backed by `state_dir/waist2_body_state.json`.
    #[must_use]
    pub fn new(state_dir: Option<PathBuf>) -> Self {
        let state_file = state_dir.map(|dir| dir.join(STATE_FILENAME));
        let state = state_file
            .as_deref()
            .and_then(load_state)
            .unwrap_or_default();
        Self {
            state_file,
            state,
            status: BodyStatus::Ready,
        }
    }

    /// Updates reported operational status (`Ready`, `Busy`, `Degraded`, `Stopped`).
    pub fn set_status(&mut self, status: BodyStatus) {
        self.status = status;
    }

    /// Returns current operational status.
    #[must_use]
    pub const fn status(&self) -> BodyStatus {
        self.status
    }

    /// Records an in-flight action status and flushes state to disk if configured.
    pub fn record_in_flight(&mut self, action_id: &ActionId, progress: Option<f32>) {
        self.state.journal.insert(
            action_id.as_str().to_owned(),
            StateQueryStatus::InFlight { progress },
        );
        self.save();
    }

    /// Records a terminal/held receipt in the crash journal and flushes state to disk.
    pub fn record_terminal(&mut self, receipt: &Receipt) {
        self.state.journal.insert(
            receipt.action_id.as_str().to_owned(),
            StateQueryStatus::Terminal {
                receipt: receipt.clone(),
            },
        );
        self.save();
    }

    /// Queries the crash journal for `action_id`.
    #[must_use]
    pub fn state_query(&self, action_id: &ActionId) -> StateQueryResult {
        StateQueryResult {
            action_id: action_id.clone(),
            status: self
                .state
                .journal
                .get(action_id.as_str())
                .cloned()
                .unwrap_or(StateQueryStatus::Unknown),
        }
    }

    /// Returns the stored terminal receipt for `action_id` if present.
    #[must_use]
    pub fn journal_receipt(&self, action_id: &ActionId) -> Option<&Receipt> {
        match self.state.journal.get(action_id.as_str()) {
            Some(StateQueryStatus::Terminal { receipt }) => Some(receipt),
            _ => None,
        }
    }

    /// Dispatches one NDJSON JSON-RPC 2.0 request line, returning `None` for responses/empty lines.
    pub fn dispatch_line(&mut self, line: &str, handler: &mut dyn BodyHandler) -> Option<String> {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return None;
        }
        if let Ok(val) = serde_json::from_str::<Value>(trimmed)
            && val.get("method").is_none()
            && (val.get("result").is_some() || val.get("error").is_some())
            && val.get("jsonrpc").and_then(Value::as_str) == Some(JSONRPC_VERSION)
        {
            return None;
        }

        let req: Waist2Request<Value> = match serde_json::from_str(trimmed) {
            Ok(req) => req,
            Err(err) => {
                let resp: Waist2Response<Value> = Waist2Response::err(
                    None,
                    Waist2Error::invalid_receipt_transition(format!("invalid JSON-RPC: {err}")),
                );
                return serde_json::to_string(&resp).ok();
            }
        };
        let id = req.id.clone();
        let resp = match self.dispatch_inner(req, handler) {
            Ok(val) => Waist2Response::ok(id, val),
            Err(err) => Waist2Response::err(id, err),
        };
        serde_json::to_string(&resp).ok()
    }

    /// Runs synchronous NDJSON `stdio` framing until EOF.
    pub fn run_stdio(
        &mut self,
        reader: impl BufRead,
        mut writer: impl Write,
        handler: &mut dyn BodyHandler,
    ) -> io::Result<()> {
        for line in reader.lines() {
            let line = line?;
            if let Some(out) = self.dispatch_line(&line, handler) {
                writeln!(writer, "{out}")?;
                writer.flush()?;
            }
        }
        Ok(())
    }

    fn dispatch_inner(
        &mut self,
        req: Waist2Request<Value>,
        handler: &mut dyn BodyHandler,
    ) -> Result<Value, Waist2Error> {
        let headers: Waist2Headers =
            serde_json::from_value(req.params.get("headers").cloned().unwrap_or(Value::Null))
                .map_err(|e| {
                    Waist2Error::invalid_receipt_transition(format!("invalid headers: {e}"))
                })?;
        headers.validate_version(CURRENT_PROTOCOL_VERSION)?;

        let cache_key = format!("{}:{}", req.method, headers.idempotency_key);
        if let Some(cached) = self.state.idempotency_cache.get(&cache_key) {
            return Ok(cached.clone());
        }

        let result = match req.method {
            Waist2Method::BodyDiscover => {
                call::<BodyDiscoverParams, _>(&req.params, |p| handler.discover(p))
            }
            Waist2Method::BodyHeartbeat => call::<BodyHeartbeatParams, _>(&req.params, |p| {
                Ok(BodyHeartbeatResult {
                    acknowledged_at_ms: p.timestamp_ms,
                })
            }),
            Waist2Method::BodyShutdown => {
                call::<BodyShutdownParams, _>(&req.params, |p| handler.on_shutdown(p))
            }
            Waist2Method::PerceptEmit => call::<PerceptEmitParams, _>(&req.params, |p| {
                if p.audience.as_ref().is_none_or(Audience::is_empty) {
                    return Err(Waist2Error::undeclared_audience());
                }
                handler.on_percept(p)
            }),
            Waist2Method::ActionExecute => call::<ActionExecuteParams, _>(&req.params, |p| {
                if p.audience.is_empty() {
                    return Err(Waist2Error::undeclared_audience());
                }
                if let Some(StateQueryStatus::Terminal { receipt }) =
                    self.state.journal.get(p.action_id.as_str()).cloned()
                {
                    return Ok(ActionExecuteResult {
                        accepted: true,
                        receipt: Some(receipt),
                    });
                }
                self.record_in_flight(&p.action_id, None);
                let res = handler.execute(p)?;
                if let Some(ref receipt) = res.receipt {
                    self.record_terminal(receipt);
                }
                Ok(res)
            }),
            Waist2Method::ActionCancel => call::<ActionCancelParams, _>(&req.params, |p| {
                let res = handler.cancel(p)?;
                if let Some(ref receipt) = res.receipt {
                    self.record_terminal(receipt);
                }
                Ok(res)
            }),
            Waist2Method::ActionReceipt => call::<ActionReceiptParams, _>(&req.params, |p| {
                self.record_terminal(&p.receipt);
                Ok(ActionReceiptResult { recorded: true })
            }),
            Waist2Method::ApprovalResolve => call::<ApprovalResolveParams, _>(&req.params, |p| {
                if let Some(existing) = self.journal_receipt(&p.action_id)
                    && existing.outcome != ReceiptOutcome::Held
                {
                    return Err(Waist2Error::invalid_receipt_transition(format!(
                        "action '{}' is {:?}, not held",
                        p.action_id, existing.outcome
                    )));
                }
                let action_id = p.action_id.clone();
                let approved = p.approved;
                let h_act = p.h_act.clone();
                let resolved_at_ms = p.resolved_at_ms;
                let existing_cap = self
                    .journal_receipt(&action_id)
                    .map(|r| r.capability_id.clone());
                let res = handler.resolve_approval(p)?;
                if res.applied
                    && let Some(cap_id) = existing_cap
                {
                    let still_held = self
                        .journal_receipt(&action_id)
                        .is_some_and(|r| r.outcome == ReceiptOutcome::Held);
                    if still_held {
                        let updated = make_terminal_receipt(
                            action_id,
                            cap_id,
                            if approved {
                                ReceiptOutcome::Done
                            } else {
                                ReceiptOutcome::Refused
                            },
                            Some(if approved { "approved" } else { "rejected" }.to_owned()),
                            None,
                            Some(TerminationReason::Normal),
                            Some(if approved { 1.0 } else { 0.0 }),
                            Some(u64::from(approved)),
                            Some(h_act),
                            resolved_at_ms,
                        );
                        self.record_terminal(&updated);
                    }
                }
                Ok(res)
            }),
            Waist2Method::QuestionAnswer => call::<QuestionAnswerParams, _>(&req.params, |p| {
                if p.audience.is_empty() {
                    return Err(Waist2Error::undeclared_audience());
                }
                handler.on_question(p)
            }),
            Waist2Method::StateQuery => {
                call::<StateQueryParams, _>(&req.params, |p| Ok(self.state_query(&p.action_id)))
            }
            Waist2Method::BlobPut => call::<BlobPutParams, _>(&req.params, |p| {
                if p.inline_bytes.is_none() && p.byte_len > 0 {
                    return Err(Waist2Error::hash_mismatch(
                        &p.sha256,
                        "missing_inline_bytes",
                    ));
                }
                let digest = verify_blob_put(&p)?;
                let blob_ref = format!("blob://sha256/{digest}");
                self.state.blobs.insert(
                    blob_ref.clone(),
                    p.inline_bytes.map(|b| b.0).unwrap_or_default(),
                );
                self.save();
                Ok(BlobPutResult {
                    blob_ref,
                    sha256: digest,
                    expires_at_ms: Time::from_millis(60_000),
                })
            }),
            Waist2Method::BlobGet => call::<BlobGetParams, _>(&req.params, |p| {
                let Some(bytes) = self.state.blobs.get(&p.blob_ref).cloned() else {
                    return Err(Waist2Error::invalid_receipt_transition(format!(
                        "blob '{}' not found",
                        p.blob_ref
                    )));
                };
                let res = BlobGetResult {
                    blob_ref: p.blob_ref,
                    sha256: sha256_hex(&bytes),
                    byte_len: bytes.len() as u64,
                    bytes: Some(OpaquePayload(bytes)),
                };
                verify_blob_get(&res, p.expected_sha256.as_deref())?;
                Ok(res)
            }),
        }?;

        self.state
            .idempotency_cache
            .insert(cache_key, result.clone());
        self.save();
        Ok(result)
    }

    fn save(&self) {
        let Some(path) = &self.state_file else {
            return;
        };
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let tmp = path.with_extension("tmp");
        if let Ok(raw) = serde_json::to_vec(&self.state)
            && fs::write(&tmp, raw).is_ok()
        {
            let _ = fs::rename(&tmp, path);
        }
    }
}

fn load_state(path: &Path) -> Option<PersistedState> {
    let raw = fs::read(path).ok()?;
    serde_json::from_slice(&raw).ok()
}

fn call<P: serde::de::DeserializeOwned, R: Serialize>(
    params: &Value,
    f: impl FnOnce(P) -> Result<R, Waist2Error>,
) -> Result<Value, Waist2Error> {
    let parsed: P = serde_json::from_value(params.clone())
        .map_err(|e| Waist2Error::invalid_receipt_transition(format!("invalid params: {e}")))?;
    serde_json::to_value(f(parsed)?)
        .map_err(|e| Waist2Error::invalid_receipt_transition(e.to_string()))
}
