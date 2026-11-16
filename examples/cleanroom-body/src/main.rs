#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::io::{self, BufRead, Write};
use std::path::PathBuf;

use monoagens_body_sdk::{
    ActionCancelParams, ActionCancelResult, ActionExecuteParams, ActionExecuteResult, ActionId,
    ActionReceiptParams, ActionReceiptResult, ApprovalResolveParams, ApprovalResolveResult,
    Audience, BlobGetParams, BlobGetResult, BlobPutParams, BlobPutResult, BodyDiscoverParams,
    BodyDiscoverResult, BodyHandler, BodyHeartbeatParams, BodyHeartbeatResult, BodyId,
    BodyShutdownParams, CURRENT_PROTOCOL_VERSION, CapabilityDescriptor, CapabilityFlags,
    CapabilityId, CapabilitySchemaHash, HAct, OpaquePayload, PerceptEmitParams, PerceptEmitResult,
    PlaceId, QuestionAnswerParams, Receipt, ReceiptOutcome, StateQueryParams, StateQueryResult,
    StateQueryStatus, TerminationReason, Waist2Error, Waist2Headers, Waist2Method, Waist2Request,
    Waist2Response, sha256_hex, sort_capabilities, verify_blob_get, verify_blob_put,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;

#[derive(Default, Serialize, Deserialize)]
struct PersistedState {
    cache: BTreeMap<String, Value>,
    journal: BTreeMap<ActionId, StateQueryStatus>,
    blobs: BTreeMap<String, Vec<u8>>,
    resolved: BTreeSet<ActionId>,
}

struct CleanroomBody {
    place: PlaceId,
    cap: CapabilityId,
    state_path: Option<PathBuf>,
    state: PersistedState,
}

impl CleanroomBody {
    fn load(state_path: Option<PathBuf>) -> Self {
        let state = state_path
            .as_ref()
            .and_then(|p| fs::read(p).ok())
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        Self {
            place: PlaceId::new("cleanroom:main"),
            cap: CapabilityId::new("cleanroom.echo"),
            state_path,
            state,
        }
    }

    fn save(&self) {
        if let Some(path) = &self.state_path
            && let Ok(bytes) = serde_json::to_vec(&self.state)
        {
            if let Some(dir) = path.parent() {
                let _ = fs::create_dir_all(dir);
            }
            let tmp = path.with_extension("tmp");
            if fs::write(&tmp, bytes).is_ok() {
                let _ = fs::rename(tmp, path);
            }
        }
    }

    fn receipt(
        &self,
        id: ActionId,
        outcome: ReceiptOutcome,
        term: TerminationReason,
        h_act: Option<HAct>,
    ) -> Receipt {
        Receipt {
            action_id: id,
            capability_id: self.cap.clone(),
            outcome,
            reason_code: None,
            detail: None,
            termination_reason: Some(term),
            progress: Some(1.0),
            delivered_units: Some(1),
            h_act,
            timestamp_ms: 1_000.into(),
        }
    }
}

impl BodyHandler for CleanroomBody {
    fn discover(&mut self, _: BodyDiscoverParams) -> Result<BodyDiscoverResult, Waist2Error> {
        let mut capabilities = vec![CapabilityDescriptor {
            id: self.cap.clone(),
            description: "Clean-room SDK echo capability".to_owned(),
            places: vec![self.place.clone()],
            flags: CapabilityFlags {
                communicative: true,
                identity_changing: false,
                irreversible: false,
            },
            typical_latency_ms: 0,
            capability_schema_hash: CapabilitySchemaHash::new(sha256_hex(b"cleanroom.echo:v1")),
            sensed_effect: None,
        }];
        sort_capabilities(&mut capabilities);
        Ok(BodyDiscoverResult {
            body_id: BodyId::new("body:cleanroom"),
            protocol_version: CURRENT_PROTOCOL_VERSION,
            places: vec![self.place.clone()],
            capabilities,
        })
    }

    fn execute(&mut self, p: ActionExecuteParams) -> Result<ActionExecuteResult, Waist2Error> {
        if p.capability_id != self.cap {
            return Err(Waist2Error::unknown_capability(&p.capability_id));
        }
        if p.audience.is_empty() {
            return Err(Waist2Error::undeclared_audience());
        }
        if p.places.is_empty() || p.places.iter().any(|pl| pl != &self.place) {
            let r = self.receipt(
                p.action_id.clone(),
                ReceiptOutcome::Refused,
                TerminationReason::Normal,
                p.h_act,
            );
            self.state.journal.insert(
                p.action_id,
                StateQueryStatus::Terminal { receipt: r.clone() },
            );
            return Ok(ActionExecuteResult {
                accepted: false,
                receipt: Some(r),
            });
        }
        let outcome = if p.payload.0 == b"hold" {
            ReceiptOutcome::Held
        } else {
            ReceiptOutcome::Done
        };
        let r = self.receipt(
            p.action_id.clone(),
            outcome,
            TerminationReason::Normal,
            p.h_act,
        );
        self.state.journal.insert(
            p.action_id,
            StateQueryStatus::Terminal { receipt: r.clone() },
        );
        Ok(ActionExecuteResult {
            accepted: true,
            receipt: Some(r),
        })
    }

    fn cancel(&mut self, p: ActionCancelParams) -> Result<ActionCancelResult, Waist2Error> {
        let r = self.receipt(
            p.action_id,
            ReceiptOutcome::Failed,
            TerminationReason::CanceledByCore,
            None,
        );
        Ok(ActionCancelResult {
            cancelled: true,
            receipt: Some(r),
        })
    }

    fn resolve_approval(
        &mut self,
        p: ApprovalResolveParams,
    ) -> Result<ApprovalResolveResult, Waist2Error> {
        let held = matches!(self.state.journal.get(&p.action_id), Some(StateQueryStatus::Terminal { receipt }) if receipt.outcome == ReceiptOutcome::Held);
        if self.state.resolved.contains(&p.action_id) || !held {
            return Ok(ApprovalResolveResult { applied: false });
        }
        self.state.resolved.insert(p.action_id.clone());
        let outcome = if p.approved {
            ReceiptOutcome::Done
        } else {
            ReceiptOutcome::Refused
        };
        let r = self.receipt(
            p.action_id.clone(),
            outcome,
            TerminationReason::Normal,
            Some(p.h_act),
        );
        self.state
            .journal
            .insert(p.action_id, StateQueryStatus::Terminal { receipt: r });
        Ok(ApprovalResolveResult { applied: true })
    }

    fn on_percept(&mut self, p: PerceptEmitParams) -> Result<PerceptEmitResult, Waist2Error> {
        Ok(PerceptEmitResult {
            accepted: p.place == self.place,
        })
    }
}

fn call<P: DeserializeOwned, R: Serialize>(
    v: &Value,
    f: impl FnOnce(P) -> Result<R, Waist2Error>,
) -> Result<Value, Waist2Error> {
    let p: P = serde_json::from_value(v.clone())
        .map_err(|e| Waist2Error::invalid_receipt_transition(e.to_string()))?;
    serde_json::to_value(f(p)?).map_err(|e| Waist2Error::invalid_receipt_transition(e.to_string()))
}

fn dispatch(body: &mut CleanroomBody, req: Waist2Request<Value>) -> Result<Value, Waist2Error> {
    let h: Waist2Headers =
        serde_json::from_value(req.params.get("headers").cloned().unwrap_or(Value::Null))
            .map_err(|e| Waist2Error::invalid_receipt_transition(e.to_string()))?;
    h.validate_version(CURRENT_PROTOCOL_VERSION)?;
    let key = format!("{}:{}", req.method, h.idempotency_key);
    if let Some(cached) = body.state.cache.get(&key) {
        return Ok(cached.clone());
    }
    let val = match req.method {
        Waist2Method::BodyDiscover => {
            call::<BodyDiscoverParams, _>(&req.params, |p| body.discover(p))
        }
        Waist2Method::BodyHeartbeat => call::<BodyHeartbeatParams, _>(&req.params, |p| {
            Ok(BodyHeartbeatResult {
                acknowledged_at_ms: p.timestamp_ms,
            })
        }),
        Waist2Method::BodyShutdown => {
            call::<BodyShutdownParams, _>(&req.params, |p| body.on_shutdown(p))
        }
        Waist2Method::PerceptEmit => call::<PerceptEmitParams, _>(&req.params, |p| {
            if p.audience.as_ref().is_none_or(Audience::is_empty) {
                return Err(Waist2Error::undeclared_audience());
            }
            body.on_percept(p)
        }),
        Waist2Method::ActionExecute => {
            call::<ActionExecuteParams, _>(&req.params, |p| body.execute(p))
        }
        Waist2Method::ActionCancel => {
            call::<ActionCancelParams, _>(&req.params, |p| body.cancel(p))
        }
        Waist2Method::ActionReceipt => call::<ActionReceiptParams, _>(&req.params, |p| {
            body.state.journal.insert(
                p.receipt.action_id.clone(),
                StateQueryStatus::Terminal { receipt: p.receipt },
            );
            Ok(ActionReceiptResult { recorded: true })
        }),
        Waist2Method::ApprovalResolve => {
            call::<ApprovalResolveParams, _>(&req.params, |p| body.resolve_approval(p))
        }
        Waist2Method::QuestionAnswer => {
            call::<QuestionAnswerParams, _>(&req.params, |p| body.on_question(p))
        }
        Waist2Method::StateQuery => call::<StateQueryParams, _>(&req.params, |p| {
            Ok(StateQueryResult {
                action_id: p.action_id.clone(),
                status: body
                    .state
                    .journal
                    .get(&p.action_id)
                    .cloned()
                    .unwrap_or(StateQueryStatus::Unknown),
            })
        }),
        Waist2Method::BlobPut => call::<BlobPutParams, _>(&req.params, |p| {
            let sha = verify_blob_put(&p)?;
            let blob_ref = format!("blob://sha256/{sha}");
            body.state.blobs.insert(
                blob_ref.clone(),
                p.inline_bytes.map(|b| b.0).unwrap_or_default(),
            );
            Ok(BlobPutResult {
                blob_ref,
                sha256: sha,
                expires_at_ms: 60_000.into(),
            })
        }),
        Waist2Method::BlobGet => call::<BlobGetParams, _>(&req.params, |p| {
            let bytes = body
                .state
                .blobs
                .get(&p.blob_ref)
                .cloned()
                .unwrap_or_default();
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
    body.state.cache.insert(key, val.clone());
    body.save();
    Ok(val)
}

fn main() -> io::Result<()> {
    let mut body = CleanroomBody::load(
        env::var_os("MONOAGENS_BODY_STATE_DIR").map(|d| PathBuf::from(d).join("state.json")),
    );
    let mut out = io::stdout().lock();
    for line in io::stdin().lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let Ok(req) = serde_json::from_str::<Waist2Request<Value>>(&line) else {
            continue;
        };
        let resp = match dispatch(&mut body, req.clone()) {
            Ok(val) => Waist2Response::ok(req.id, val),
            Err(err) => Waist2Response::err(req.id, err),
        };
        writeln!(out, "{}", serde_json::to_string(&resp)?)?;
        out.flush()?;
    }
    Ok(())
}
