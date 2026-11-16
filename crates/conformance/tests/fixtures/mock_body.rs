//! Configurable Waist-2 mock body binary for `monoagens-conformance` positive & negative tests.

#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};

use monoagens_protocol::{
    ActionCancelParams, ActionCancelResult, ActionExecuteParams, ActionExecuteResult,
    ActionReceiptParams, ApprovalResolveParams, ApprovalResolveResult, Audience, BlobGetParams,
    BlobGetResult, BlobPutParams, BlobPutResult, BodyDiscoverParams, BodyDiscoverResult,
    BodyHeartbeatParams, BodyHeartbeatResult, BodyId, BodyShutdownParams, BodyShutdownResult,
    CURRENT_PROTOCOL_VERSION, CapabilityDescriptor, CapabilityFlags, CapabilityId,
    CapabilitySchemaHash, JSONRPC_VERSION, OpaquePayload, PerceptEmitParams, PerceptEmitResult,
    PlaceId, QuestionAnswerParams, QuestionAnswerResult, Receipt, ReceiptOutcome, RpcId,
    StateQueryParams, StateQueryResult, StateQueryStatus, TerminationReason, Time, Waist2Error,
    Waist2Headers, Waist2Method, Waist2Request, Waist2Response, sort_capabilities,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FixtureMode {
    Good,
    UnsortedDiscover,
    DuplicateExecution,
    SilentCrash,
    CrossPlace,
    DoubleReceipt,
    BadDigest,
    UnknownCapability,
    BadHeartbeat,
    BadDeliveryDelay,
    AcceptUndeclaredAudience,
    DiscoverOnly,
}

impl FixtureMode {
    fn parse(raw: &str) -> Result<Self, String> {
        match raw {
            "good" => Ok(Self::Good),
            "unsorted_discover" => Ok(Self::UnsortedDiscover),
            "duplicate_execution" => Ok(Self::DuplicateExecution),
            "silent_crash" => Ok(Self::SilentCrash),
            "cross_place" => Ok(Self::CrossPlace),
            "double_receipt" => Ok(Self::DoubleReceipt),
            "bad_digest" => Ok(Self::BadDigest),
            "unknown_capability" => Ok(Self::UnknownCapability),
            "bad_heartbeat" => Ok(Self::BadHeartbeat),
            "bad_delivery_delay" => Ok(Self::BadDeliveryDelay),
            "accept_undeclared_audience" => Ok(Self::AcceptUndeclaredAudience),
            "discover_only" => Ok(Self::DiscoverOnly),
            other => Err(format!("unknown fixture mode '{other}'")),
        }
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct PersistedState {
    idempotency_cache: BTreeMap<String, Value>,
    journal: BTreeMap<String, StateQueryStatus>,
    blobs: BTreeMap<String, Vec<u8>>,
    exec_counter: u64,
    crash_primed: bool,
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(&mut out, "{byte:02x}");
    }
    out
}

struct MockBodyServer {
    mode: FixtureMode,
    state_file: Option<PathBuf>,
    state: PersistedState,
    owned_places: Vec<PlaceId>,
    capabilities: Vec<CapabilityDescriptor>,
    next_outbound_id: i64,
}

impl MockBodyServer {
    fn new(mode: FixtureMode, state_dir: Option<PathBuf>) -> Self {
        let p0 = PlaceId::new("fixture:0");
        let p1 = PlaceId::new("fixture:1");
        let mut capabilities = vec![
            CapabilityDescriptor {
                id: CapabilityId::new("z.tool"),
                description: "Secondary fixture tool".to_owned(),
                places: vec![p1.clone(), p0.clone()],
                flags: CapabilityFlags {
                    communicative: false,
                    identity_changing: false,
                    irreversible: true,
                },
                typical_latency_ms: 5,
                capability_schema_hash: CapabilitySchemaHash::new("sha256:fixture-z-tool"),
                sensed_effect: None,
            },
            CapabilityDescriptor {
                id: CapabilityId::new("a.echo"),
                description: "Primary echo tool".to_owned(),
                places: vec![p0.clone()],
                flags: CapabilityFlags {
                    communicative: true,
                    identity_changing: false,
                    irreversible: false,
                },
                typical_latency_ms: 1,
                capability_schema_hash: CapabilitySchemaHash::new("sha256:fixture-a-echo"),
                sensed_effect: None,
            },
        ];
        if mode != FixtureMode::UnsortedDiscover {
            sort_capabilities(&mut capabilities);
        }

        let mut owned_places = vec![p0, p1];
        owned_places.sort();

        let state_file = state_dir.map(|d| d.join("mock_body_state.json"));
        let state = state_file
            .as_deref()
            .and_then(load_state)
            .unwrap_or_default();

        Self {
            mode,
            state_file,
            state,
            owned_places,
            capabilities,
            next_outbound_id: 1000,
        }
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

    fn handle_line(&mut self, line: &str, out: &mut impl Write) -> bool {
        let req: Waist2Request<Value> = match serde_json::from_str(line) {
            Ok(req) => req,
            Err(err) => {
                let resp: Waist2Response<Value> = Waist2Response::err(
                    None,
                    Waist2Error::invalid_receipt_transition(format!("invalid JSON-RPC: {err}")),
                );
                let _ = write_json_line(out, &resp);
                return true;
            }
        };

        let id = req.id.clone();

        if self.mode == FixtureMode::DiscoverOnly
            && !matches!(
                req.method,
                Waist2Method::BodyDiscover | Waist2Method::StateQuery | Waist2Method::BodyShutdown
            )
        {
            let resp: Waist2Response<Value> = Waist2Response::err(
                id,
                Waist2Error {
                    code: -32601,
                    message: format!("Method not found: {}", req.method),
                    data: None,
                },
            );
            let _ = write_json_line(out, &resp);
            return true;
        }

        if self.mode == FixtureMode::SilentCrash
            && req.method == Waist2Method::StateQuery
            && self.state.crash_primed
        {
            // Intentionally exit silently without responding to simulate silent crash failure.
            return false;
        }

        let headers_val = req.params.get("headers").cloned().unwrap_or(Value::Null);
        let headers: Waist2Headers = match serde_json::from_value(headers_val) {
            Ok(h) => h,
            Err(err) => {
                let resp: Waist2Response<Value> = Waist2Response::err(
                    id,
                    Waist2Error::invalid_receipt_transition(format!("invalid headers: {err}")),
                );
                let _ = write_json_line(out, &resp);
                return true;
            }
        };

        if let Err(err) = headers.validate_version(CURRENT_PROTOCOL_VERSION) {
            let resp: Waist2Response<Value> = Waist2Response::err(id, err);
            let _ = write_json_line(out, &resp);
            return true;
        }

        let cache_key = format!("{}:{}", req.method, headers.idempotency_key);
        if self.mode != FixtureMode::DuplicateExecution
            && let Some(cached) = self.state.idempotency_cache.get(&cache_key)
        {
            let resp = Waist2Response::ok(id, cached.clone());
            let _ = write_json_line(out, &resp);
            return true;
        }

        let result = match req.method {
            Waist2Method::BodyDiscover => {
                let _params: BodyDiscoverParams = match parse_params(&req.params) {
                    Ok(p) => p,
                    Err(e) => return emit_err(out, id, e),
                };
                serde_json::to_value(BodyDiscoverResult {
                    body_id: BodyId::new("tck-mock-body"),
                    protocol_version: CURRENT_PROTOCOL_VERSION,
                    capabilities: self.capabilities.clone(),
                    places: self.owned_places.clone(),
                })
                .map_err(|e| Waist2Error::invalid_receipt_transition(e.to_string()))
            }
            Waist2Method::BodyHeartbeat => {
                let params: BodyHeartbeatParams = match parse_params(&req.params) {
                    Ok(p) => p,
                    Err(e) => return emit_err(out, id, e),
                };
                let ack_ts = if self.mode == FixtureMode::BadHeartbeat {
                    Time::from_millis(0)
                } else {
                    params.timestamp_ms
                };
                serde_json::to_value(BodyHeartbeatResult {
                    acknowledged_at_ms: ack_ts,
                })
                .map_err(|e| Waist2Error::invalid_receipt_transition(e.to_string()))
            }
            Waist2Method::BodyShutdown => {
                let _params: BodyShutdownParams = match parse_params(&req.params) {
                    Ok(p) => p,
                    Err(e) => return emit_err(out, id, e),
                };
                serde_json::to_value(BodyShutdownResult { accepted: true })
                    .map_err(|e| Waist2Error::invalid_receipt_transition(e.to_string()))
            }
            Waist2Method::PerceptEmit => {
                let params: PerceptEmitParams = match parse_params(&req.params) {
                    Ok(p) => p,
                    Err(e) => return emit_err(out, id, e),
                };
                self.on_percept_emit(params).and_then(|res| {
                    serde_json::to_value(res)
                        .map_err(|e| Waist2Error::invalid_receipt_transition(e.to_string()))
                })
            }
            Waist2Method::ActionExecute => {
                let params: ActionExecuteParams = match parse_params(&req.params) {
                    Ok(p) => p,
                    Err(e) => return emit_err(out, id, e),
                };
                self.on_action_execute(params, out).and_then(|res| {
                    serde_json::to_value(res)
                        .map_err(|e| Waist2Error::invalid_receipt_transition(e.to_string()))
                })
            }
            Waist2Method::ActionCancel => {
                let params: ActionCancelParams = match parse_params(&req.params) {
                    Ok(p) => p,
                    Err(e) => return emit_err(out, id, e),
                };
                serde_json::to_value(ActionCancelResult {
                    cancelled: false,
                    receipt: Some(Receipt {
                        action_id: params.action_id,
                        capability_id: CapabilityId::new("a.echo"),
                        outcome: ReceiptOutcome::Failed,
                        reason_code: Some("canceled_by_core".to_owned()),
                        detail: params.reason,
                        termination_reason: Some(TerminationReason::CanceledByCore),
                        progress: Some(0.5),
                        delivered_units: Some(0),
                        h_act: None,
                        timestamp_ms: Time::from_millis(100),
                    }),
                })
                .map_err(|e| Waist2Error::invalid_receipt_transition(e.to_string()))
            }
            Waist2Method::ActionReceipt => {
                let params: ActionReceiptParams = match parse_params(&req.params) {
                    Ok(p) => p,
                    Err(e) => return emit_err(out, id, e),
                };
                self.state.journal.insert(
                    params.receipt.action_id.as_str().to_owned(),
                    StateQueryStatus::Terminal {
                        receipt: params.receipt,
                    },
                );
                serde_json::to_value(monoagens_protocol::ActionReceiptResult { recorded: true })
                    .map_err(|e| Waist2Error::invalid_receipt_transition(e.to_string()))
            }
            Waist2Method::ApprovalResolve => {
                let params: ApprovalResolveParams = match parse_params(&req.params) {
                    Ok(p) => p,
                    Err(e) => return emit_err(out, id, e),
                };
                self.on_approval_resolve(params, out).and_then(|res| {
                    serde_json::to_value(res)
                        .map_err(|e| Waist2Error::invalid_receipt_transition(e.to_string()))
                })
            }
            Waist2Method::QuestionAnswer => {
                let params: QuestionAnswerParams = match parse_params(&req.params) {
                    Ok(p) => p,
                    Err(e) => return emit_err(out, id, e),
                };
                if params.audience.is_empty() && self.mode != FixtureMode::AcceptUndeclaredAudience
                {
                    Err(Waist2Error::undeclared_audience())
                } else {
                    serde_json::to_value(QuestionAnswerResult { accepted: true })
                        .map_err(|e| Waist2Error::invalid_receipt_transition(e.to_string()))
                }
            }
            Waist2Method::StateQuery => {
                let params: StateQueryParams = match parse_params(&req.params) {
                    Ok(p) => p,
                    Err(e) => return emit_err(out, id, e),
                };
                let status = self
                    .state
                    .journal
                    .get(params.action_id.as_str())
                    .cloned()
                    .unwrap_or(StateQueryStatus::Unknown);
                serde_json::to_value(StateQueryResult {
                    action_id: params.action_id,
                    status,
                })
                .map_err(|e| Waist2Error::invalid_receipt_transition(e.to_string()))
            }
            Waist2Method::BlobPut => {
                let params: BlobPutParams = match parse_params(&req.params) {
                    Ok(p) => p,
                    Err(e) => return emit_err(out, id, e),
                };
                self.on_blob_put(params).and_then(|res| {
                    serde_json::to_value(res)
                        .map_err(|e| Waist2Error::invalid_receipt_transition(e.to_string()))
                })
            }
            Waist2Method::BlobGet => {
                let params: BlobGetParams = match parse_params(&req.params) {
                    Ok(p) => p,
                    Err(e) => return emit_err(out, id, e),
                };
                self.on_blob_get(params).and_then(|res| {
                    serde_json::to_value(res)
                        .map_err(|e| Waist2Error::invalid_receipt_transition(e.to_string()))
                })
            }
        };

        match result {
            Ok(val) => {
                if self.mode != FixtureMode::DuplicateExecution {
                    self.state.idempotency_cache.insert(cache_key, val.clone());
                }
                self.save();
                let resp = Waist2Response::ok(id, val);
                let _ = write_json_line(out, &resp);
            }
            Err(err) => {
                self.save();
                let resp: Waist2Response<Value> = Waist2Response::err(id, err);
                let _ = write_json_line(out, &resp);
            }
        }
        true
    }

    fn on_percept_emit(
        &mut self,
        params: PerceptEmitParams,
    ) -> Result<PerceptEmitResult, Waist2Error> {
        if self.mode != FixtureMode::AcceptUndeclaredAudience
            && params.audience.as_ref().is_none_or(Audience::is_empty)
        {
            return Err(Waist2Error::undeclared_audience());
        }
        if self.mode != FixtureMode::CrossPlace && !self.owned_places.contains(&params.place) {
            return Err(Waist2Error::invalid_receipt_transition(format!(
                "percept place '{}' is not owned by mock body",
                params.place
            )));
        }
        Ok(PerceptEmitResult { accepted: true })
    }

    fn on_action_execute(
        &mut self,
        params: ActionExecuteParams,
        out: &mut impl Write,
    ) -> Result<ActionExecuteResult, Waist2Error> {
        if self.mode != FixtureMode::AcceptUndeclaredAudience && params.audience.is_empty() {
            return Err(Waist2Error::undeclared_audience());
        }

        let cap_opt = self
            .capabilities
            .iter()
            .find(|c| c.id == params.capability_id)
            .cloned();

        if cap_opt.is_none() && self.mode != FixtureMode::UnknownCapability {
            return Err(Waist2Error::unknown_capability(&params.capability_id));
        }

        if self.mode != FixtureMode::CrossPlace {
            let cap_places = cap_opt.as_ref().map(|c| c.places.as_slice()).unwrap_or(&[]);
            let allowed = !params.places.is_empty()
                && params.places.iter().all(|p| {
                    self.owned_places.contains(p)
                        && (cap_places.is_empty() || cap_places.contains(p))
                });
            if !allowed {
                let refused_receipt = Receipt {
                    action_id: params.action_id,
                    capability_id: params.capability_id,
                    outcome: ReceiptOutcome::Refused,
                    reason_code: Some("ERR_UNKNOWN_PLACE".to_owned()),
                    detail: Some("action targets an unowned place".to_owned()),
                    termination_reason: Some(TerminationReason::Normal),
                    progress: Some(0.0),
                    delivered_units: Some(0),
                    h_act: params.h_act,
                    timestamp_ms: Time::from_millis(100),
                };
                return Ok(ActionExecuteResult {
                    accepted: false,
                    receipt: Some(refused_receipt),
                });
            }
        }

        if self.mode != FixtureMode::DuplicateExecution
            && let Some(existing) = self.state.journal.get(params.action_id.as_str())
        {
            return match existing {
                StateQueryStatus::Terminal { receipt } => Ok(ActionExecuteResult {
                    accepted: true,
                    receipt: Some(receipt.clone()),
                }),
                StateQueryStatus::InFlight { .. } | StateQueryStatus::Unknown => {
                    Ok(ActionExecuteResult {
                        accepted: true,
                        receipt: None,
                    })
                }
            };
        }

        self.state.exec_counter = self.state.exec_counter.saturating_add(1);
        let payload_text = String::from_utf8_lossy(params.payload.as_bytes());
        if payload_text.contains("crash-recovery-probe") {
            self.state.crash_primed = true;
        }

        let is_held = payload_text.contains("hold");
        let is_timing_probe = payload_text.contains("delivery-timing-probe");

        let detail = if self.mode == FixtureMode::DuplicateExecution {
            Some(format!("exec={}", self.state.exec_counter))
        } else if is_timing_probe {
            if self.mode == FixtureMode::BadDeliveryDelay {
                Some(
                    json!({
                        "d_art_ms": 250,
                        "working_cue_ms": 5_000,
                        "progress_note_ms": 10_000
                    })
                    .to_string(),
                )
            } else {
                Some(
                    json!({
                        "d_art_ms": 0,
                        "working_cue_ms": 1_000,
                        "progress_note_ms": 10_000
                    })
                    .to_string(),
                )
            }
        } else {
            None
        };

        let receipt = Receipt {
            action_id: params.action_id.clone(),
            capability_id: params.capability_id,
            outcome: if is_held {
                ReceiptOutcome::Held
            } else {
                ReceiptOutcome::Done
            },
            reason_code: if is_held {
                Some("PendingApproval".to_owned())
            } else {
                None
            },
            detail,
            termination_reason: Some(TerminationReason::Normal),
            progress: if is_held { Some(0.0) } else { Some(1.0) },
            delivered_units: if is_held { Some(0) } else { Some(1) },
            h_act: params.h_act,
            timestamp_ms: Time::from_millis(100 + self.state.exec_counter),
        };

        self.state.journal.insert(
            params.action_id.as_str().to_owned(),
            StateQueryStatus::Terminal {
                receipt: receipt.clone(),
            },
        );

        if self.mode == FixtureMode::DuplicateExecution {
            self.emit_outbound_receipt(out, receipt.clone());
        }

        Ok(ActionExecuteResult {
            accepted: true,
            receipt: Some(receipt),
        })
    }

    fn on_approval_resolve(
        &mut self,
        params: ApprovalResolveParams,
        out: &mut impl Write,
    ) -> Result<ApprovalResolveResult, Waist2Error> {
        let Some(StateQueryStatus::Terminal { receipt }) =
            self.state.journal.get(params.action_id.as_str()).cloned()
        else {
            return Ok(ApprovalResolveResult { applied: false });
        };

        if self.mode != FixtureMode::DoubleReceipt && receipt.outcome != ReceiptOutcome::Held {
            return Err(Waist2Error::invalid_receipt_transition(format!(
                "action '{}' is {:?}, not held",
                params.action_id, receipt.outcome
            )));
        }

        let updated = Receipt {
            action_id: params.action_id.clone(),
            capability_id: receipt.capability_id,
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
            progress: Some(if params.approved { 1.0 } else { 0.0 }),
            delivered_units: Some(u64::from(params.approved)),
            h_act: Some(params.h_act),
            timestamp_ms: params.resolved_at_ms,
        };

        self.state.journal.insert(
            params.action_id.as_str().to_owned(),
            StateQueryStatus::Terminal {
                receipt: updated.clone(),
            },
        );

        if self.mode == FixtureMode::DoubleReceipt {
            // Emit duplicate outbound receipts for the same action.
            self.emit_outbound_receipt(out, updated.clone());
            self.emit_outbound_receipt(out, updated);
        }

        Ok(ApprovalResolveResult { applied: true })
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

        if self.mode != FixtureMode::BadDigest {
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
        }

        let sha = sha256_hex(&bytes);
        let blob_ref = format!("blob://sha256/{sha}");
        self.state.blobs.insert(blob_ref.clone(), bytes);
        Ok(BlobPutResult {
            blob_ref,
            sha256: params.sha256,
            expires_at_ms: Time::from_millis(60_000),
        })
    }

    fn on_blob_get(&self, params: BlobGetParams) -> Result<BlobGetResult, Waist2Error> {
        let Some(bytes) = self.state.blobs.get(&params.blob_ref) else {
            return Err(Waist2Error::invalid_receipt_transition(format!(
                "blob '{}' not found",
                params.blob_ref
            )));
        };
        let actual_sha = sha256_hex(bytes);
        if self.mode != FixtureMode::BadDigest
            && let Some(expected) = &params.expected_sha256
            && !actual_sha.eq_ignore_ascii_case(expected)
        {
            return Err(Waist2Error::hash_mismatch(expected, &actual_sha));
        }
        Ok(BlobGetResult {
            blob_ref: params.blob_ref,
            sha256: actual_sha,
            byte_len: bytes.len() as u64,
            bytes: Some(OpaquePayload::from_bytes(bytes.clone())),
        })
    }

    fn emit_outbound_receipt(&mut self, out: &mut impl Write, receipt: Receipt) {
        let id = RpcId::Int(self.next_outbound_id);
        self.next_outbound_id = self.next_outbound_id.saturating_add(1);
        let req = Waist2Request::new(
            Some(id),
            Waist2Method::ActionReceipt,
            ActionReceiptParams {
                headers: Waist2Headers::new(format!(
                    "outbound-rcpt-{}-{}",
                    receipt.action_id, self.next_outbound_id
                )),
                receipt,
            },
        );
        if let Ok(line) = serde_json::to_string(&req) {
            let _ = writeln!(out, "{line}");
            let _ = out.flush();
        }
    }
}

fn emit_err(out: &mut impl Write, id: Option<RpcId>, err: Waist2Error) -> bool {
    let resp: Waist2Response<Value> = Waist2Response::err(id, err);
    let _ = write_json_line(out, &resp);
    true
}

fn write_json_line<T: Serialize>(out: &mut impl Write, val: &T) -> io::Result<()> {
    let line = serde_json::to_string(val)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
    writeln!(out, "{line}")?;
    out.flush()
}

fn parse_params<P: serde::de::DeserializeOwned>(params: &Value) -> Result<P, Waist2Error> {
    serde_json::from_value(params.clone())
        .map_err(|e| Waist2Error::invalid_receipt_transition(format!("invalid params: {e}")))
}

fn load_state(path: &Path) -> Option<PersistedState> {
    let raw = fs::read(path).ok()?;
    serde_json::from_slice(&raw).ok()
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let mut mode = FixtureMode::Good;
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--mode" {
            i += 1;
            let raw = args.get(i).expect("--mode requires a value");
            mode = FixtureMode::parse(raw).expect("valid fixture mode");
        }
        i += 1;
    }

    let state_dir = env::var_os("MONOAGENS_BODY_STATE_DIR").map(PathBuf::from);
    let mut server = MockBodyServer::new(mode, state_dir);

    let stdin = io::stdin();
    let mut stdout = io::BufWriter::new(io::stdout().lock());
    for line_res in stdin.lock().lines() {
        let Ok(line) = line_res else {
            break;
        };
        if line.trim().is_empty() {
            continue;
        }
        // Ignore JSON-RPC responses (e.g. acks for outbound action/receipt).
        if let Ok(val) = serde_json::from_str::<Value>(&line)
            && val.get("method").is_none()
            && (val.get("result").is_some() || val.get("error").is_some())
            && val.get("jsonrpc").and_then(Value::as_str) == Some(JSONRPC_VERSION)
        {
            continue;
        }
        if !server.handle_line(&line, &mut stdout) {
            break;
        }
    }
}
