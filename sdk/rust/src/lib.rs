//! Thin Waist-2 (`monoagens.body.v1`) Rust body SDK shim wrapping `monoagens-protocol`.

#![forbid(unsafe_code)]

use std::collections::{BTreeMap, HashMap};
use std::io::{self, BufRead, Write};

pub use monoagens_protocol::*;
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;
use sha2::{Digest, Sha256};

/// Pointer to the canonical OpenRPC 1.4.1 contract in `monoagens-protocol`.
pub const OPENRPC_SCHEMA_PATH: &str = "crates/protocol/openrpc/monoagens.body.v1.openrpc.json";

/// Computes lowercase hex SHA-256 digest of `bytes`.
#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// Verifies `bytes` against `expected` SHA-256 hex digest, returning `-32005` on mismatch.
pub fn verify_sha256(expected: &str, bytes: &[u8]) -> Result<String, Waist2Error> {
    let actual = sha256_hex(bytes);
    if actual.eq_ignore_ascii_case(expected) { Ok(actual) } else { Err(Waist2Error::hash_mismatch(expected, &actual)) }
}

/// Verifies `blob/put` payload bytes and byte length against declared SHA-256.
pub fn verify_blob_put(params: &BlobPutParams) -> Result<String, Waist2Error> {
    if let Some(inline) = &params.inline_bytes {
        if inline.len() as u64 != params.byte_len {
            return Err(Waist2Error::hash_mismatch(&params.sha256, "byte_len_mismatch"));
        }
        return verify_sha256(&params.sha256, inline.as_bytes());
    }
    Ok(params.sha256.clone())
}

/// Verifies `blob/get` returned bytes against expected SHA-256.
pub fn verify_blob_get(result: &BlobGetResult, expected: Option<&str>) -> Result<(), Waist2Error> {
    let target = expected.unwrap_or(&result.sha256);
    if let Some(bytes) = &result.bytes { verify_sha256(target, bytes.as_bytes())?; }
    if let Some(exp) = expected && !result.sha256.eq_ignore_ascii_case(exp) {
        return Err(Waist2Error::hash_mismatch(exp, &result.sha256));
    }
    Ok(())
}

/// Classification of an inbound NDJSON frame on a bidirectional Waist-2 stream.
#[derive(Debug, Clone, PartialEq)]
pub enum InboundFrame {
    Request(Waist2Request<Value>),
    Response(Waist2Response<Value>),
}

/// Parses one NDJSON line as either an inbound JSON-RPC request or a response
/// to an outbound request, distinguishing them by the presence of `"method"`.
pub fn parse_inbound_line(line: &str) -> Result<InboundFrame, serde_json::Error> {
    let val: Value = serde_json::from_str(line)?;
    if val.get("method").is_some() {
        Ok(InboundFrame::Request(serde_json::from_value(val)?))
    } else {
        Ok(InboundFrame::Response(serde_json::from_value(val)?))
    }
}

/// Ergonomic builder for outbound Waist-2 requests with monotonic `RpcId` and `idempotency_key`.
#[derive(Debug, Clone)]
pub struct OutboundBuilder {
    prefix: String,
    next_id: i64,
    next_key: u64,
}

impl Default for OutboundBuilder {
    fn default() -> Self {
        Self::new("body")
    }
}

impl OutboundBuilder {
    /// Creates a new [`OutboundBuilder`] with the given `idempotency_key` prefix.
    #[must_use]
    pub fn new(prefix: impl Into<String>) -> Self {
        Self {
            prefix: prefix.into(),
            next_id: 1,
            next_key: 1,
        }
    }

    /// Allocates the next monotonic numeric [`RpcId`].
    pub fn next_rpc_id(&mut self) -> RpcId {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        RpcId::Int(id)
    }

    /// Allocates the next monotonic `idempotency_key` tagged with `tag`.
    pub fn next_idempotency_key(&mut self, tag: &str) -> String {
        let seq = self.next_key;
        self.next_key = self.next_key.saturating_add(1);
        format!("{}-{tag}-{seq}", self.prefix)
    }

    /// Builds [`Waist2Headers`] for `CURRENT_PROTOCOL_VERSION` with a fresh `idempotency_key`.
    pub fn next_headers(&mut self, tag: &str) -> Waist2Headers {
        Waist2Headers::new(self.next_idempotency_key(tag))
    }

    /// Builds [`Waist2Headers`] with reverse-DNS `_meta` entries attached.
    pub fn next_headers_with_meta(
        &mut self,
        tag: &str,
        meta: BTreeMap<String, MetaValue>,
    ) -> Waist2Headers {
        let mut headers = self.next_headers(tag);
        headers.meta = meta;
        headers
    }

    /// Builds a `percept/emit` request, validating a non-empty `audience` (`-32006`).
    pub fn percept_emit(
        &mut self,
        actor: ActorId,
        place: PlaceId,
        audience: Audience,
        timestamp_ms: Time,
        payload: OpaquePayload,
    ) -> Result<Waist2Request<PerceptEmitParams>, Waist2Error> {
        if audience.is_empty() {
            return Err(Waist2Error::undeclared_audience());
        }
        let headers = self.next_headers("percept");
        let percept = Percept::minimal(actor, place, audience, timestamp_ms, payload);
        Ok(Waist2Request::new(
            Some(self.next_rpc_id()),
            Waist2Method::PerceptEmit,
            PerceptEmitParams::from_percept(headers, percept),
        ))
    }

    /// Wraps a full [`Percept`] as a `percept/emit` request, validating a non-empty `audience` (`-32006`).
    pub fn percept_emit_from_percept(
        &mut self,
        percept: Percept,
    ) -> Result<Waist2Request<PerceptEmitParams>, Waist2Error> {
        if percept.audience.is_empty() {
            return Err(Waist2Error::undeclared_audience());
        }
        let headers = self.next_headers("percept");
        Ok(Waist2Request::new(
            Some(self.next_rpc_id()),
            Waist2Method::PerceptEmit,
            PerceptEmitParams::from_percept(headers, percept),
        ))
    }

    /// Builds an `action/receipt` request for asynchronous/event-driven receipt delivery.
    pub fn action_receipt(&mut self, receipt: Receipt) -> Waist2Request<ActionReceiptParams> {
        let headers = self.next_headers("receipt");
        Waist2Request::new(
            Some(self.next_rpc_id()),
            Waist2Method::ActionReceipt,
            ActionReceiptParams { headers, receipt },
        )
    }

    /// Builds a `body/heartbeat` request.
    pub fn heartbeat(
        &mut self,
        body_id: BodyId,
        status: BodyStatus,
        timestamp_ms: Time,
    ) -> Waist2Request<BodyHeartbeatParams> {
        let headers = self.next_headers("heartbeat");
        Waist2Request::new(
            Some(self.next_rpc_id()),
            Waist2Method::BodyHeartbeat,
            BodyHeartbeatParams {
                headers,
                body_id,
                status,
                timestamp_ms,
            },
        )
    }

    /// Builds a `blob/put` request, automatically computing SHA-256 and `byte_len`.
    pub fn blob_put(
        &mut self,
        bytes: &[u8],
        content_type: Option<&str>,
        scope_id: Option<&str>,
    ) -> Waist2Request<BlobPutParams> {
        let mut meta = BTreeMap::new();
        if let Some(ct) = content_type {
            meta.insert("content_type".to_owned(), MetaValue::String(ct.to_owned()));
        }
        if let Some(scope) = scope_id {
            meta.insert("scope_id".to_owned(), MetaValue::String(scope.to_owned()));
        }
        let headers = self.next_headers_with_meta("blob-put", meta);
        Waist2Request::new(
            Some(self.next_rpc_id()),
            Waist2Method::BlobPut,
            BlobPutParams {
                headers,
                sha256: sha256_hex(bytes),
                byte_len: bytes.len() as u64,
                inline_bytes: Some(OpaquePayload::from_bytes(bytes)),
            },
        )
    }

    /// Builds a `blob/get` request with optional `expected_sha256` verification digest.
    pub fn blob_get(
        &mut self,
        blob_ref: impl Into<String>,
        expected_sha256: Option<String>,
    ) -> Waist2Request<BlobGetParams> {
        let headers = self.next_headers("blob-get");
        Waist2Request::new(
            Some(self.next_rpc_id()),
            Waist2Method::BlobGet,
            BlobGetParams {
                headers,
                blob_ref: blob_ref.into(),
                expected_sha256,
            },
        )
    }
}

/// User-implemented body callbacks for Waist-2 RPCs.
pub trait BodyHandler {
    fn discover(&mut self, params: BodyDiscoverParams) -> Result<BodyDiscoverResult, Waist2Error>;
    fn execute(&mut self, params: ActionExecuteParams) -> Result<ActionExecuteResult, Waist2Error>;
    fn cancel(&mut self, params: ActionCancelParams) -> Result<ActionCancelResult, Waist2Error>;
    fn resolve_approval(&mut self, params: ApprovalResolveParams) -> Result<ApprovalResolveResult, Waist2Error>;
    fn on_percept(&mut self, _p: PerceptEmitParams) -> Result<PerceptEmitResult, Waist2Error> { Ok(PerceptEmitResult { accepted: true }) }
    fn on_question(&mut self, _p: QuestionAnswerParams) -> Result<QuestionAnswerResult, Waist2Error> { Ok(QuestionAnswerResult { accepted: true }) }
    fn on_shutdown(&mut self, _p: BodyShutdownParams) -> Result<BodyShutdownResult, Waist2Error> { Ok(BodyShutdownResult { accepted: true }) }
}

/// Body-side runtime providing NDJSON framing, `idempotency_key` dedup, crash journal, and blob store.
#[derive(Debug, Default)]
pub struct BodyRuntime {
    idempotency_cache: HashMap<String, Value>,
    journal: HashMap<ActionId, StateQueryStatus>,
    blobs: HashMap<String, Vec<u8>>,
}

impl BodyRuntime {
    pub fn record_in_flight(&mut self, action_id: ActionId, progress: Option<f32>) {
        self.journal.insert(action_id, StateQueryStatus::InFlight { progress });
    }

    pub fn record_terminal(&mut self, receipt: Receipt) {
        self.journal.insert(receipt.action_id.clone(), StateQueryStatus::Terminal { receipt });
    }

    #[must_use]
    pub fn state_query(&self, action_id: &ActionId) -> StateQueryResult {
        StateQueryResult { action_id: action_id.clone(), status: self.journal.get(action_id).cloned().unwrap_or(StateQueryStatus::Unknown) }
    }

    /// Dispatches one NDJSON JSON-RPC 2.0 request line across all twelve Waist-2 methods.
    pub fn dispatch_line(&mut self, line: &str, handler: &mut dyn BodyHandler) -> Option<String> {
        let req: Waist2Request<Value> = serde_json::from_str(line).ok()?;
        let id = req.id.clone();
        let resp = match self.dispatch_inner(req, handler) {
            Ok(val) => Waist2Response::ok(id, val),
            Err(err) => Waist2Response::err(id, err),
        };
        serde_json::to_string(&resp).ok()
    }

    /// Runs synchronous NDJSON `stdio` framing until EOF.
    pub fn run_stdio(&mut self, reader: impl BufRead, mut writer: impl Write, handler: &mut dyn BodyHandler) -> io::Result<()> {
        for line in reader.lines() {
            let line = line?;
            if !line.trim().is_empty() && let Some(out) = self.dispatch_line(&line, handler) {
                writeln!(writer, "{out}")?;
                writer.flush()?;
            }
        }
        Ok(())
    }

    fn dispatch_inner(&mut self, req: Waist2Request<Value>, handler: &mut dyn BodyHandler) -> Result<Value, Waist2Error> {
        let headers: Waist2Headers = serde_json::from_value(req.params.get("headers").cloned().unwrap_or(Value::Null))
            .map_err(|e| Waist2Error::invalid_receipt_transition(e.to_string()))?;
        headers.validate_version(CURRENT_PROTOCOL_VERSION)?;
        let cache_key = format!("{}:{}", req.method, headers.idempotency_key);
        if let Some(cached) = self.idempotency_cache.get(&cache_key) { return Ok(cached.clone()); }
        let result = match req.method {
            Waist2Method::BodyDiscover => call::<BodyDiscoverParams, _>(&req.params, |p| handler.discover(p)),
            Waist2Method::BodyHeartbeat => call::<BodyHeartbeatParams, _>(&req.params, |p| Ok(BodyHeartbeatResult { acknowledged_at_ms: p.timestamp_ms })),
            Waist2Method::BodyShutdown => call::<BodyShutdownParams, _>(&req.params, |p| handler.on_shutdown(p)),
            Waist2Method::PerceptEmit => call::<PerceptEmitParams, _>(&req.params, |p| {
                if p.audience.as_ref().is_none_or(Audience::is_empty) { return Err(Waist2Error::undeclared_audience()); }
                handler.on_percept(p)
            }),
            Waist2Method::ActionExecute => call::<ActionExecuteParams, _>(&req.params, |p| {
                self.record_in_flight(p.action_id.clone(), None);
                let res = handler.execute(p)?;
                if let Some(r) = &res.receipt { self.record_terminal(r.clone()); }
                Ok(res)
            }),
            Waist2Method::ActionCancel => call::<ActionCancelParams, _>(&req.params, |p| {
                let res = handler.cancel(p)?;
                if let Some(r) = &res.receipt { self.record_terminal(r.clone()); }
                Ok(res)
            }),
            Waist2Method::ActionReceipt => call::<ActionReceiptParams, _>(&req.params, |p| {
                self.record_terminal(p.receipt);
                Ok(ActionReceiptResult { recorded: true })
            }),
            Waist2Method::ApprovalResolve => call::<ApprovalResolveParams, _>(&req.params, |p| handler.resolve_approval(p)),
            Waist2Method::QuestionAnswer => call::<QuestionAnswerParams, _>(&req.params, |p| handler.on_question(p)),
            Waist2Method::StateQuery => call::<StateQueryParams, _>(&req.params, |p| Ok(self.state_query(&p.action_id))),
            Waist2Method::BlobPut => call::<BlobPutParams, _>(&req.params, |p| {
                let digest = verify_blob_put(&p)?;
                let blob_ref = format!("blob://sha256/{digest}");
                self.blobs.insert(blob_ref.clone(), p.inline_bytes.map(|b| b.0).unwrap_or_default());
                Ok(BlobPutResult { blob_ref, sha256: digest, expires_at_ms: 60_000.into() })
            }),
            Waist2Method::BlobGet => call::<BlobGetParams, _>(&req.params, |p| {
                let bytes = self.blobs.get(&p.blob_ref).cloned().unwrap_or_default();
                let res = BlobGetResult { blob_ref: p.blob_ref, sha256: sha256_hex(&bytes), byte_len: bytes.len() as u64, bytes: Some(OpaquePayload(bytes)) };
                verify_blob_get(&res, p.expected_sha256.as_deref())?;
                Ok(res)
            }),
        }?;
        self.idempotency_cache.insert(cache_key, result.clone());
        Ok(result)
    }
}

fn call<P: DeserializeOwned, R: Serialize>(params: &Value, f: impl FnOnce(P) -> Result<R, Waist2Error>) -> Result<Value, Waist2Error> {
    let parsed: P = serde_json::from_value(params.clone()).map_err(|e| Waist2Error::invalid_receipt_transition(e.to_string()))?;
    serde_json::to_value(f(parsed)?).map_err(|e| Waist2Error::invalid_receipt_transition(e.to_string()))
}

/// Deterministic heartbeat scheduling helper.
#[derive(Debug, Clone)]
pub struct HeartbeatScheduler {
    pub interval_ms: u64,
    last_sent_ms: Option<u64>,
}

impl HeartbeatScheduler {
    #[must_use]
    pub const fn new(interval_ms: u64) -> Self { Self { interval_ms, last_sent_ms: None } }

    #[must_use]
    pub fn is_due(&self, now_ms: u64) -> bool { self.last_sent_ms.is_none_or(|prev| now_ms.saturating_sub(prev) >= self.interval_ms) }

    pub fn next_request(&mut self, id: RpcId, body_id: BodyId, status: BodyStatus, now_ms: u64, key: impl Into<String>) -> Waist2Request<BodyHeartbeatParams> {
        self.last_sent_ms = Some(now_ms);
        Waist2Request::new(Some(id), Waist2Method::BodyHeartbeat, BodyHeartbeatParams { headers: Waist2Headers::new(key), body_id, status, timestamp_ms: now_ms.into() })
    }
}
