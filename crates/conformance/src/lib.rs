//! Standalone black-box Waist-1 / Waist-2 conformance TCK runner (`monoagens-conformance`).
//!
//! Executes Body TCK checks (`B-MUST-01..07`, `B-SHOULD-01..03`, `B-MAY-01`) over real
//! NDJSON `stdio` JSON-RPC 2.0 against a spawned child body without in-memory shortcuts,
//! emitting deterministic `compatibility.json` reports.

#![forbid(unsafe_code)]

use core::fmt;
use std::io::{BufRead, BufReader, BufWriter, Read, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

use monoagens_protocol::{
    ActionExecuteParams, ActionExecuteResult, ActionId, ActionReceiptParams, ActionReceiptResult,
    ActorId, ApprovalResolveParams, ApprovalResolveResult, Audience, BlobGetParams, BlobGetResult,
    BlobPutParams, BlobPutResult, BodyDiscoverParams, BodyDiscoverResult, BodyHeartbeatParams,
    BodyHeartbeatResult, BodyId, BodyShutdownParams, BodyShutdownResult, BodyStatus,
    CURRENT_PROTOCOL_VERSION, CapabilityId, HAct, JSONRPC_VERSION, MetaValue, OpaquePayload,
    PerceptEmitResult, PlaceId, Receipt, ReceiptOutcome, RpcId, StateQueryParams, StateQueryResult,
    StateQueryStatus, Time, Waist2ErrorCode, Waist2Headers, Waist2Method, Waist2Request,
    Waist2Response, sort_capabilities,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tempfile::TempDir;

const DEFAULT_RPC_TIMEOUT: Duration = Duration::from_millis(2_500);
const CRASH_RECOVERY_BOUND_MS: u64 = 5_000;
const METHOD_NOT_FOUND_CODE: i32 = -32601;

/// Canonical conformance level grading flag (`--level must|should|may`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConformanceLevel {
    Must,
    Should,
    May,
}

impl ConformanceLevel {
    /// Parses a CLI level string (`must`, `should`, `may`).
    pub fn parse(raw: &str) -> Result<Self, String> {
        match raw {
            "must" => Ok(Self::Must),
            "should" => Ok(Self::Should),
            "may" => Ok(Self::May),
            other => Err(format!(
                "invalid conformance level '{other}' (expected must, should, or may)"
            )),
        }
    }

    /// Returns the canonical lowercase string form.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Must => "must",
            Self::Should => "should",
            Self::May => "may",
        }
    }
}

/// Outcome of a single TCK check in `compatibility.json`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    Pass,
    Fail,
    Skip,
}

/// Single deterministic check record in `compatibility.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConformanceCheckResult {
    pub id: String,
    pub title: String,
    pub status: CheckStatus,
    pub detail: String,
}

/// Deterministic black-box conformance report (`compatibility.json`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompatibilityReport {
    pub schema_version: String,
    pub target: String,
    pub level: ConformanceLevel,
    pub checks: Vec<ConformanceCheckResult>,
}

impl CompatibilityReport {
    /// Returns `true` only when every required check for the requested level passes.
    ///
    /// At `--level must`, all `B-MUST-*` checks must be `Pass`.
    /// At `--level should`, all `B-MUST-*` and `B-SHOULD-*` checks must be `Pass`.
    /// At `--level may`, all `B-MUST-*` and `B-SHOULD-*` checks must be `Pass` and no check may `Fail`.
    #[must_use]
    pub fn passed(&self) -> bool {
        self.checks.iter().all(|check| match self.level {
            ConformanceLevel::Must | ConformanceLevel::Should => check.status == CheckStatus::Pass,
            ConformanceLevel::May => {
                if check.id.starts_with("B-MAY-") || check.id.starts_with("C-MAY-") {
                    check.status != CheckStatus::Fail
                } else {
                    check.status == CheckStatus::Pass
                }
            }
        })
    }
}

/// Errors returned by the conformance runner when spawning or framing fails fatally.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConformanceError {
    SpawnFailed { target: String, detail: String },
    Io(String),
    NotImplemented(&'static str),
}

impl fmt::Display for ConformanceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SpawnFailed { target, detail } => {
                write!(f, "failed to spawn target '{target}': {detail}")
            }
            Self::Io(detail) => write!(f, "conformance I/O error: {detail}"),
            Self::NotImplemented(msg) => write!(f, "not implemented: {msg}"),
        }
    }
}

impl std::error::Error for ConformanceError {}

#[derive(Debug, Clone)]
struct OutboundRpcRecord {
    method: String,
    params: Value,
}

struct ChildRpcSession {
    child: Child,
    stdin: Option<BufWriter<ChildStdin>>,
    stdout_rx: Receiver<Result<Option<String>, String>>,
    next_id: i64,
    outbound_rpcs: Vec<OutboundRpcRecord>,
    waited: bool,
}

impl Drop for ChildRpcSession {
    fn drop(&mut self) {
        if !self.waited {
            drop(self.stdin.take());
            let _ = self.child.kill();
            let _ = self.child.wait();
            self.waited = true;
        }
    }
}

impl ChildRpcSession {
    fn spawn(
        target: &str,
        extra_args: &[String],
        state_dir: &Path,
    ) -> Result<Self, ConformanceError> {
        let mut cmd = if Path::new(target).exists() || !target.contains(char::is_whitespace) {
            let mut c = Command::new(target);
            c.args(extra_args);
            c
        } else {
            let mut parts = target.split_whitespace();
            let exe = parts.next().ok_or_else(|| ConformanceError::SpawnFailed {
                target: target.to_owned(),
                detail: "empty target command".to_owned(),
            })?;
            let mut c = Command::new(exe);
            c.args(parts);
            c.args(extra_args);
            c
        };

        cmd.env("MONOAGENS_BODY_STATE_DIR", state_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = cmd.spawn().map_err(|err| ConformanceError::SpawnFailed {
            target: target.to_owned(),
            detail: err.to_string(),
        })?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| ConformanceError::Io("missing child stdin".to_owned()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| ConformanceError::Io("missing child stdout".to_owned()))?;
        let stderr = child.stderr.take();

        if let Some(mut err_stream) = stderr {
            thread::spawn(move || {
                let mut sink = [0u8; 1024];
                while let Ok(n) = err_stream.read(&mut sink) {
                    if n == 0 {
                        break;
                    }
                }
            });
        }

        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let mut line = String::new();
                match reader.read_line(&mut line) {
                    Ok(0) => {
                        let _ = tx.send(Ok(None));
                        break;
                    }
                    Ok(_) => {
                        if tx.send(Ok(Some(line))).is_err() {
                            break;
                        }
                    }
                    Err(err) => {
                        let _ = tx.send(Err(err.to_string()));
                        break;
                    }
                }
            }
        });

        Ok(Self {
            child,
            stdin: Some(BufWriter::new(stdin)),
            stdout_rx: rx,
            next_id: 1,
            outbound_rpcs: Vec::new(),
            waited: false,
        })
    }

    fn call<P, R>(&mut self, method: Waist2Method, params: P) -> Result<Waist2Response<R>, String>
    where
        P: Serialize,
        R: serde::de::DeserializeOwned,
    {
        self.call_with_timeout(method, params, DEFAULT_RPC_TIMEOUT)
    }

    fn call_with_timeout<P, R>(
        &mut self,
        method: Waist2Method,
        params: P,
        timeout: Duration,
    ) -> Result<Waist2Response<R>, String>
    where
        P: Serialize,
        R: serde::de::DeserializeOwned,
    {
        let params_val =
            serde_json::to_value(params).map_err(|e| format!("encode params failed: {e}"))?;
        self.call_raw_with_timeout(method, params_val, timeout)
    }

    fn call_raw_with_timeout<R>(
        &mut self,
        method: Waist2Method,
        params_val: Value,
        timeout: Duration,
    ) -> Result<Waist2Response<R>, String>
    where
        R: serde::de::DeserializeOwned,
    {
        let id = RpcId::Int(self.next_id);
        self.next_id = self.next_id.saturating_add(1);

        let req = Waist2Request::new(Some(id.clone()), method, params_val);
        let wire =
            serde_json::to_string(&req).map_err(|e| format!("encode request failed: {e}"))?;

        self.send_line(&wire)?;

        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(format!(
                    "timed out waiting for JSON-RPC response to {method}"
                ));
            }

            let line = match self.stdout_rx.recv_timeout(remaining) {
                Ok(Ok(Some(line))) => line,
                Ok(Ok(None)) => {
                    return Err(format!(
                        "target closed stdout before sending JSON-RPC response for {method}"
                    ));
                }
                Ok(Err(err)) => {
                    return Err(format!("read from target stdout failed: {err}"));
                }
                Err(RecvTimeoutError::Timeout) => {
                    return Err(format!(
                        "timed out waiting for JSON-RPC response to {method}"
                    ));
                }
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(format!(
                        "target stdout reader disconnected before response for {method}"
                    ));
                }
            };

            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            let raw: Value = serde_json::from_str(trimmed)
                .map_err(|e| format!("invalid JSON frame from target for {method}: {e}"))?;

            if let Some(outbound_method) = raw.get("method").and_then(Value::as_str) {
                let outbound_params = raw.get("params").cloned().unwrap_or(Value::Null);
                self.outbound_rpcs.push(OutboundRpcRecord {
                    method: outbound_method.to_owned(),
                    params: outbound_params,
                });
                if let Some(req_id_val) = raw.get("id").filter(|v| !v.is_null())
                    && let Ok(req_id) = serde_json::from_value::<RpcId>(req_id_val.clone())
                {
                    let _ = self.ack_outbound_request(req_id, outbound_method);
                }
                continue;
            }

            let resp: Waist2Response<R> = serde_json::from_value(raw)
                .map_err(|e| format!("invalid JSON-RPC response for {method}: {e}"))?;
            if resp.jsonrpc != JSONRPC_VERSION {
                return Err(format!(
                    "invalid jsonrpc field '{}', expected '{JSONRPC_VERSION}'",
                    resp.jsonrpc
                ));
            }
            if resp.id != Some(id) {
                return Err(format!("mismatched JSON-RPC response id {:?}", resp.id));
            }
            return Ok(resp);
        }
    }

    fn send_line(&mut self, wire: &str) -> Result<(), String> {
        let stdin = self
            .stdin
            .as_mut()
            .ok_or_else(|| "child stdin already closed".to_owned())?;
        writeln!(stdin, "{wire}").map_err(|e| format!("write to target stdin failed: {e}"))?;
        stdin
            .flush()
            .map_err(|e| format!("flush target stdin failed: {e}"))?;
        Ok(())
    }

    fn ack_outbound_request(&mut self, req_id: RpcId, method: &str) -> Result<(), String> {
        let ack = match method {
            "action/receipt" => serde_json::to_string(&Waist2Response::ok(
                Some(req_id),
                ActionReceiptResult { recorded: true },
            )),
            "body/heartbeat" => serde_json::to_string(&Waist2Response::ok(
                Some(req_id),
                BodyHeartbeatResult {
                    acknowledged_at_ms: Time::from_millis(0),
                },
            )),
            "percept/emit" => serde_json::to_string(&Waist2Response::ok(
                Some(req_id),
                PerceptEmitResult { accepted: true },
            )),
            _ => serde_json::to_string(&Waist2Response::ok(Some(req_id), json!({ "ok": true }))),
        }
        .map_err(|e| format!("encode outbound ack failed: {e}"))?;
        self.send_line(&ack)
    }

    fn kill_sigkill(&mut self) {
        if !self.waited {
            drop(self.stdin.take());
            let _ = self.child.kill();
            let _ = self.child.wait();
            self.waited = true;
        }
    }

    fn shutdown_and_wait(&mut self, body_id: Option<BodyId>) {
        if !self.waited {
            if let Some(bid) = body_id {
                let _: Result<Waist2Response<BodyShutdownResult>, String> = self.call_with_timeout(
                    Waist2Method::BodyShutdown,
                    BodyShutdownParams {
                        headers: Waist2Headers::new("tck-shutdown-1"),
                        body_id: bid,
                        reason: Some("conformance_complete".to_owned()),
                    },
                    Duration::from_millis(500),
                );
            }
            drop(self.stdin.take());
            let _ = self.child.wait();
            self.waited = true;
        }
    }
}

#[derive(Debug, Clone)]
struct ProbeTarget {
    body_id: BodyId,
    capability_id: CapabilityId,
    place_id: PlaceId,
    all_capabilities: Vec<CapabilityId>,
    all_places: Vec<PlaceId>,
}

impl ProbeTarget {
    fn from_discover(discover: Option<&BodyDiscoverResult>) -> Self {
        match discover {
            Some(d) => {
                let first_cap = d
                    .capabilities
                    .first()
                    .map(|c| c.id.clone())
                    .unwrap_or_else(|| CapabilityId::new("a.echo"));
                let first_place = d
                    .capabilities
                    .first()
                    .and_then(|c| c.places.first().cloned())
                    .or_else(|| d.places.first().cloned())
                    .unwrap_or_else(|| PlaceId::new("tck:place-0"));
                Self {
                    body_id: d.body_id.clone(),
                    capability_id: first_cap,
                    place_id: first_place,
                    all_capabilities: d.capabilities.iter().map(|c| c.id.clone()).collect(),
                    all_places: d.places.clone(),
                }
            }
            None => Self {
                body_id: BodyId::new("tck-fallback-body"),
                capability_id: CapabilityId::new("a.echo"),
                place_id: PlaceId::new("tck:place-0"),
                all_capabilities: vec![CapabilityId::new("a.echo")],
                all_places: vec![PlaceId::new("tck:place-0")],
            },
        }
    }
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

fn check_result(
    id: &str,
    title: &str,
    status: CheckStatus,
    detail: impl Into<String>,
) -> ConformanceCheckResult {
    ConformanceCheckResult {
        id: id.to_owned(),
        title: title.to_owned(),
        status,
        detail: detail.into(),
    }
}

fn is_method_not_found(err: &monoagens_protocol::Waist2Error) -> bool {
    err.code == METHOD_NOT_FOUND_CODE
}

fn extract_outbound_receipts(records: &[OutboundRpcRecord], action_id: &ActionId) -> Vec<Receipt> {
    records
        .iter()
        .filter(|rec| rec.method == "action/receipt")
        .filter_map(|rec| serde_json::from_value::<ActionReceiptParams>(rec.params.clone()).ok())
        .map(|p| p.receipt)
        .filter(|r| &r.action_id == action_id)
        .collect()
}

fn state_status_label(status: &StateQueryStatus) -> &'static str {
    match status {
        StateQueryStatus::InFlight { .. } => "in_flight",
        StateQueryStatus::Terminal { .. } => "terminal",
        StateQueryStatus::Unknown => "unknown",
    }
}

/// Runs the Waist-2 Body conformance checks against `target` over NDJSON `stdio`
/// filtered by `level`, returning a deterministic [`CompatibilityReport`].
pub fn run_body_conformance(
    target: &str,
    extra_args: &[String],
    level: ConformanceLevel,
) -> Result<CompatibilityReport, ConformanceError> {
    let state_dir = TempDir::new().map_err(|e| ConformanceError::Io(e.to_string()))?;
    let mut session = ChildRpcSession::spawn(target, extra_args, state_dir.path())?;
    let mut checks = Vec::new();

    // B-MUST-01: body/discover canonical envelope, version compatibility, sorted capabilities + schema hash.
    let (b_must_01, discover_opt) = run_b_must_01(&mut session);
    checks.push(b_must_01);

    let probe = ProbeTarget::from_discover(discover_opt.as_ref());

    // B-MUST-02: 3x idempotency_key replay -> == 1 execution and single matching receipt.
    checks.push(run_b_must_02(&mut session, &probe));

    // B-MUST-03: SIGKILL mid-execution -> restart handshake -> state/query <= 5000 ms, zero duplicate side effect.
    checks.push(run_b_must_03(
        &mut session,
        target,
        extra_args,
        state_dir.path(),
        &probe,
    ));

    // B-MUST-04: audience / place isolation.
    checks.push(run_b_must_04(&mut session, &probe));

    // B-MUST-05: held receipt round-trip and exactly-once approval/resolve.
    checks.push(run_b_must_05(&mut session, &probe));

    // B-MUST-06: SHA-256 blob integrity -> -32005 ERR_HASH_MISMATCH before any percept reference.
    checks.push(run_b_must_06(&mut session));

    // B-MUST-07: unknown capability yields typed -32001 ERR_UNKNOWN_CAPABILITY refusal.
    checks.push(run_b_must_07(&mut session, &probe));

    if level >= ConformanceLevel::Should {
        // B-SHOULD-01: heartbeat liveness.
        checks.push(run_b_should_01(&mut session, &probe));

        // B-SHOULD-02: honest working_cue / progress_note timing (d_art=0ms, 1s/10s cues).
        checks.push(run_b_should_02(&mut session, &probe));

        // B-SHOULD-03: reject undeclared audience with -32006 ERR_UNDECLARED_AUDIENCE.
        checks.push(run_b_should_03(&mut session, &probe));
    }

    if level >= ConformanceLevel::May {
        checks.push(check_result(
            "B-MAY-01",
            "Optional extended body hints",
            CheckStatus::Skip,
            "deferred to future milestone: optional extended body hints",
        ));
    }

    session.shutdown_and_wait(discover_opt.map(|d| d.body_id));

    Ok(CompatibilityReport {
        schema_version: "1.0".to_owned(),
        target: target.to_owned(),
        level,
        checks,
    })
}

fn run_b_must_01(
    session: &mut ChildRpcSession,
) -> (ConformanceCheckResult, Option<BodyDiscoverResult>) {
    const TITLE: &str =
        "body/discover returns deterministically sorted capabilities with capability_schema_hash";

    let resp = match session.call::<BodyDiscoverParams, BodyDiscoverResult>(
        Waist2Method::BodyDiscover,
        BodyDiscoverParams {
            headers: Waist2Headers::new("tck-b-must-01-discover-1"),
            body_id: None,
        },
    ) {
        Ok(resp) => resp,
        Err(err) => {
            return (
                check_result("B-MUST-01", TITLE, CheckStatus::Fail, err),
                None,
            );
        }
    };

    if let Some(err) = resp.error {
        return (
            check_result(
                "B-MUST-01",
                TITLE,
                CheckStatus::Fail,
                format!(
                    "body/discover returned error [{}]: {}",
                    err.code, err.message
                ),
            ),
            None,
        );
    }

    let Some(result) = resp.result else {
        return (
            check_result(
                "B-MUST-01",
                TITLE,
                CheckStatus::Fail,
                "body/discover response missing result",
            ),
            None,
        );
    };

    if result.body_id.as_str().trim().is_empty() {
        return (
            check_result(
                "B-MUST-01",
                TITLE,
                CheckStatus::Fail,
                "body/discover returned empty body_id",
            ),
            Some(result),
        );
    }

    if !result
        .protocol_version
        .is_compatible_with(CURRENT_PROTOCOL_VERSION)
    {
        return (
            check_result(
                "B-MUST-01",
                TITLE,
                CheckStatus::Fail,
                format!(
                    "incompatible protocol_version {} (expected <= {CURRENT_PROTOCOL_VERSION})",
                    result.protocol_version
                ),
            ),
            Some(result),
        );
    }

    if result.capabilities.is_empty() {
        return (
            check_result(
                "B-MUST-01",
                TITLE,
                CheckStatus::Fail,
                "body/discover returned zero capabilities",
            ),
            Some(result),
        );
    }

    let mut expected_sorted = result.capabilities.clone();
    sort_capabilities(&mut expected_sorted);
    if result.capabilities != expected_sorted {
        return (
            check_result(
                "B-MUST-01",
                TITLE,
                CheckStatus::Fail,
                "body/discover capabilities are not deterministically sorted",
            ),
            Some(result),
        );
    }

    if let Some(missing_hash) = result
        .capabilities
        .iter()
        .find(|c| c.capability_schema_hash.as_str().trim().is_empty())
    {
        return (
            check_result(
                "B-MUST-01",
                TITLE,
                CheckStatus::Fail,
                format!(
                    "capability '{}' missing capability_schema_hash",
                    missing_hash.id
                ),
            ),
            Some(result),
        );
    }

    let mut expected_places = result.places.clone();
    expected_places.sort();
    expected_places.dedup();
    if result.places != expected_places {
        return (
            check_result(
                "B-MUST-01",
                TITLE,
                CheckStatus::Fail,
                "body/discover places are not sorted and deduplicated",
            ),
            Some(result),
        );
    }

    // Second discover call must be deterministic.
    match session.call::<BodyDiscoverParams, BodyDiscoverResult>(
        Waist2Method::BodyDiscover,
        BodyDiscoverParams {
            headers: Waist2Headers::new("tck-b-must-01-discover-2"),
            body_id: None,
        },
    ) {
        Ok(resp2) => {
            if let Some(err) = resp2.error {
                return (
                    check_result(
                        "B-MUST-01",
                        TITLE,
                        CheckStatus::Fail,
                        format!(
                            "second body/discover returned error [{}]: {}",
                            err.code, err.message
                        ),
                    ),
                    Some(result),
                );
            }
            if resp2.result.as_ref() != Some(&result) {
                return (
                    check_result(
                        "B-MUST-01",
                        TITLE,
                        CheckStatus::Fail,
                        "repeated body/discover calls produced non-deterministic descriptors",
                    ),
                    Some(result),
                );
            }
        }
        Err(err) => {
            return (
                check_result("B-MUST-01", TITLE, CheckStatus::Fail, err),
                Some(result),
            );
        }
    }

    let detail = format!(
        "body_id='{}', protocol_version={}, capabilities={}",
        result.body_id,
        result.protocol_version,
        result.capabilities.len()
    );
    (
        check_result("B-MUST-01", TITLE, CheckStatus::Pass, detail),
        Some(result),
    )
}

fn run_b_must_02(session: &mut ChildRpcSession, probe: &ProbeTarget) -> ConformanceCheckResult {
    const TITLE: &str =
        "3x identical idempotency_key action/execute yields single execution and matching receipt";

    let action_id = ActionId::new("tck-act-idem-01");
    let params = ActionExecuteParams {
        headers: Waist2Headers::new("tck-b-must-02-idem-exec"),
        action_id: action_id.clone(),
        capability_id: probe.capability_id.clone(),
        places: vec![probe.place_id.clone()],
        audience: Audience::new([ActorId::new("per_tck_actor")]),
        payload: OpaquePayload::from_bytes(b"idempotency-probe".as_slice()),
        timeout_ms: Some(5_000),
        h_act: None,
    };

    let outbound_start = session.outbound_rpcs.len();
    let mut results = Vec::with_capacity(3);
    for attempt in 1..=3 {
        let resp = match session.call::<ActionExecuteParams, ActionExecuteResult>(
            Waist2Method::ActionExecute,
            params.clone(),
        ) {
            Ok(resp) => resp,
            Err(err) => return check_result("B-MUST-02", TITLE, CheckStatus::Fail, err),
        };
        if let Some(err) = resp.error {
            if is_method_not_found(&err) {
                return check_result(
                    "B-MUST-02",
                    TITLE,
                    CheckStatus::Skip,
                    format!("target does not implement action/execute: {}", err.message),
                );
            }
            return check_result(
                "B-MUST-02",
                TITLE,
                CheckStatus::Fail,
                format!(
                    "action/execute attempt {attempt} returned error [{}]: {}",
                    err.code, err.message
                ),
            );
        }
        let Some(res) = resp.result else {
            return check_result(
                "B-MUST-02",
                TITLE,
                CheckStatus::Fail,
                format!("action/execute attempt {attempt} missing result"),
            );
        };
        if !res.accepted {
            return check_result(
                "B-MUST-02",
                TITLE,
                CheckStatus::Fail,
                format!("action/execute attempt {attempt} was not accepted"),
            );
        }
        results.push(res);
    }

    if results[1] != results[0] || results[2] != results[0] {
        return check_result(
            "B-MUST-02",
            TITLE,
            CheckStatus::Fail,
            "3x idempotency_key replay returned distinct ActionExecuteResult / receipt payloads",
        );
    }

    let outbound_receipts =
        extract_outbound_receipts(&session.outbound_rpcs[outbound_start..], &action_id);
    if outbound_receipts.len() > 1 {
        return check_result(
            "B-MUST-02",
            TITLE,
            CheckStatus::Fail,
            format!(
                "expected at most 1 outbound action/receipt for idempotency_key replay, got {}",
                outbound_receipts.len()
            ),
        );
    }
    if let (Some(inline), Some(outbound)) = (&results[0].receipt, outbound_receipts.first())
        && inline != outbound
    {
        return check_result(
            "B-MUST-02",
            TITLE,
            CheckStatus::Fail,
            "outbound action/receipt differed from inline action/execute receipt",
        );
    }

    if results[0].receipt.is_none() && outbound_receipts.is_empty() {
        // Verify via state/query that the action exists in the body's journal.
        match session.call::<StateQueryParams, StateQueryResult>(
            Waist2Method::StateQuery,
            StateQueryParams {
                headers: Waist2Headers::new("tck-b-must-02-sq"),
                action_id,
            },
        ) {
            Ok(sq_resp) => {
                if let Some(err) = sq_resp.error {
                    return check_result(
                        "B-MUST-02",
                        TITLE,
                        CheckStatus::Fail,
                        format!("state/query returned error [{}]: {}", err.code, err.message),
                    );
                }
                if let Some(sq) = sq_resp.result
                    && matches!(sq.status, StateQueryStatus::Unknown)
                {
                    return check_result(
                        "B-MUST-02",
                        TITLE,
                        CheckStatus::Fail,
                        "accepted action returned no receipt and state/query reported unknown",
                    );
                }
            }
            Err(err) => return check_result("B-MUST-02", TITLE, CheckStatus::Fail, err),
        }
    }

    check_result(
        "B-MUST-02",
        TITLE,
        CheckStatus::Pass,
        "3x idempotency_key replay produced 1 execution and identical receipt",
    )
}

fn run_b_must_03(
    session: &mut ChildRpcSession,
    target: &str,
    extra_args: &[String],
    state_dir: &Path,
    probe: &ProbeTarget,
) -> ConformanceCheckResult {
    const TITLE: &str =
        "SIGKILL crash recovery answers state/query within 5000 ms without duplicate execution";

    let action_id = ActionId::new("tck-act-crash-01");
    let exec_params = ActionExecuteParams {
        headers: Waist2Headers::new("tck-b-must-03-crash-exec"),
        action_id: action_id.clone(),
        capability_id: probe.capability_id.clone(),
        places: vec![probe.place_id.clone()],
        audience: Audience::new([ActorId::new("per_tck_actor")]),
        payload: OpaquePayload::from_bytes(b"crash-recovery-probe".as_slice()),
        timeout_ms: Some(5_000),
        h_act: None,
    };

    let pre_kill_receipt = match session.call::<ActionExecuteParams, ActionExecuteResult>(
        Waist2Method::ActionExecute,
        exec_params.clone(),
    ) {
        Ok(resp) => {
            if let Some(err) = resp.error {
                if is_method_not_found(&err) {
                    return check_result(
                        "B-MUST-03",
                        TITLE,
                        CheckStatus::Skip,
                        format!("target does not implement action/execute: {}", err.message),
                    );
                }
                return check_result(
                    "B-MUST-03",
                    TITLE,
                    CheckStatus::Fail,
                    format!(
                        "pre-SIGKILL action/execute failed [{}]: {}",
                        err.code, err.message
                    ),
                );
            }
            match resp.result {
                Some(r) if r.accepted => r.receipt,
                Some(_) => {
                    return check_result(
                        "B-MUST-03",
                        TITLE,
                        CheckStatus::Fail,
                        "pre-SIGKILL action/execute was not accepted",
                    );
                }
                None => {
                    return check_result(
                        "B-MUST-03",
                        TITLE,
                        CheckStatus::Fail,
                        "pre-SIGKILL action/execute missing result",
                    );
                }
            }
        }
        Err(err) => return check_result("B-MUST-03", TITLE, CheckStatus::Fail, err),
    };

    // Inject SIGKILL mid-flight / immediately after acceptance, then respawn.
    session.kill_sigkill();

    let restarted = match ChildRpcSession::spawn(target, extra_args, state_dir) {
        Ok(s) => s,
        Err(err) => {
            return check_result(
                "B-MUST-03",
                TITLE,
                CheckStatus::Fail,
                format!("respawn after SIGKILL failed: {err}"),
            );
        }
    };
    *session = restarted;

    // Explicit restart handshake: body/discover greeting.
    match session.call::<BodyDiscoverParams, BodyDiscoverResult>(
        Waist2Method::BodyDiscover,
        BodyDiscoverParams {
            headers: Waist2Headers::new("tck-b-must-03-restart-discover"),
            body_id: None,
        },
    ) {
        Ok(resp) if resp.error.is_some() => {
            let err = resp.error.unwrap();
            return check_result(
                "B-MUST-03",
                TITLE,
                CheckStatus::Fail,
                format!(
                    "post-SIGKILL body/discover greeting failed [{}]: {}",
                    err.code, err.message
                ),
            );
        }
        Ok(resp) if resp.result.is_none() => {
            return check_result(
                "B-MUST-03",
                TITLE,
                CheckStatus::Fail,
                "post-SIGKILL body/discover greeting missing result",
            );
        }
        Err(err) => {
            return check_result(
                "B-MUST-03",
                TITLE,
                CheckStatus::Fail,
                format!("post-SIGKILL body/discover handshake failed: {err}"),
            );
        }
        Ok(_) => {}
    }

    // Measure 5000 ms bound from child's body/discover greeting after restart.
    let started = Instant::now();
    let bound = Duration::from_millis(CRASH_RECOVERY_BOUND_MS);
    let sq_resp = match session.call_with_timeout::<StateQueryParams, StateQueryResult>(
        Waist2Method::StateQuery,
        StateQueryParams {
            headers: Waist2Headers::new("tck-b-must-03-state-query"),
            action_id: action_id.clone(),
        },
        bound,
    ) {
        Ok(resp) => resp,
        Err(err) => {
            return check_result(
                "B-MUST-03",
                TITLE,
                CheckStatus::Fail,
                format!("post-SIGKILL state/query failed: {err}"),
            );
        }
    };
    let elapsed = started.elapsed();
    if elapsed > bound {
        return check_result(
            "B-MUST-03",
            TITLE,
            CheckStatus::Fail,
            "post-SIGKILL state/query exceeded 5000 ms bound",
        );
    }

    if let Some(err) = sq_resp.error {
        return check_result(
            "B-MUST-03",
            TITLE,
            CheckStatus::Fail,
            format!(
                "post-SIGKILL state/query returned error [{}]: {}",
                err.code, err.message
            ),
        );
    }
    let Some(sq) = sq_resp.result else {
        return check_result(
            "B-MUST-03",
            TITLE,
            CheckStatus::Fail,
            "post-SIGKILL state/query missing result",
        );
    };
    if sq.action_id != action_id {
        return check_result(
            "B-MUST-03",
            TITLE,
            CheckStatus::Fail,
            format!(
                "post-SIGKILL state/query returned mismatched action_id '{}'",
                sq.action_id
            ),
        );
    }

    // Replay same action/execute after restart to ensure zero duplicate side effect.
    let replay_start = session.outbound_rpcs.len();
    match session
        .call::<ActionExecuteParams, ActionExecuteResult>(Waist2Method::ActionExecute, exec_params)
    {
        Ok(resp) => {
            if let Some(err) = resp.error {
                return check_result(
                    "B-MUST-03",
                    TITLE,
                    CheckStatus::Fail,
                    format!(
                        "post-SIGKILL action/execute replay returned error [{}]: {}",
                        err.code, err.message
                    ),
                );
            }
            let Some(replay) = resp.result else {
                return check_result(
                    "B-MUST-03",
                    TITLE,
                    CheckStatus::Fail,
                    "post-SIGKILL action/execute replay missing result",
                );
            };
            if !replay.accepted {
                return check_result(
                    "B-MUST-03",
                    TITLE,
                    CheckStatus::Fail,
                    "post-SIGKILL action/execute replay was not accepted",
                );
            }
            if let StateQueryStatus::Terminal {
                receipt: ref q_rcpt,
            } = sq.status
            {
                if let Some(ref replay_rcpt) = replay.receipt
                    && replay_rcpt != q_rcpt
                {
                    return check_result(
                        "B-MUST-03",
                        TITLE,
                        CheckStatus::Fail,
                        "post-SIGKILL replay produced a distinct receipt from state/query",
                    );
                }
                if let Some(ref before_rcpt) = pre_kill_receipt
                    && before_rcpt != q_rcpt
                {
                    return check_result(
                        "B-MUST-03",
                        TITLE,
                        CheckStatus::Fail,
                        "post-SIGKILL terminal receipt differed from pre-SIGKILL receipt",
                    );
                }
                let post_outbound =
                    extract_outbound_receipts(&session.outbound_rpcs[replay_start..], &action_id);
                if post_outbound.iter().any(|r| r != q_rcpt) {
                    return check_result(
                        "B-MUST-03",
                        TITLE,
                        CheckStatus::Fail,
                        "post-SIGKILL replay emitted a conflicting outbound receipt",
                    );
                }
            }
        }
        Err(err) => {
            return check_result(
                "B-MUST-03",
                TITLE,
                CheckStatus::Fail,
                format!("post-SIGKILL action/execute replay failed: {err}"),
            );
        }
    }

    check_result(
        "B-MUST-03",
        TITLE,
        CheckStatus::Pass,
        format!(
            "SIGKILL restart handshake + state/query answered within 5000ms bound without duplicate execution (status={})",
            state_status_label(&sq.status)
        ),
    )
}

fn run_b_must_04(session: &mut ChildRpcSession, probe: &ProbeTarget) -> ConformanceCheckResult {
    const TITLE: &str =
        "action/execute and percept/emit enforce declared place and audience isolation";

    let mut foreign_place = PlaceId::new("tck:foreign-unowned-place-999");
    if probe.all_places.contains(&foreign_place) {
        foreign_place = PlaceId::new("tck:foreign-unowned-place-alt");
    }

    // 1. Percept for a place the body does not own must be refused.
    let foreign_percept = json!({
        "headers": Waist2Headers::new("tck-b-must-04-foreign-percept"),
        "actor": "per_tck_actor",
        "place": foreign_place,
        "audience": ["per_tck_actor"],
        "timestamp_ms": 1_000,
        "payload": b"foreign-place-percept".to_vec()
    });
    match session.call_raw_with_timeout::<PerceptEmitResult>(
        Waist2Method::PerceptEmit,
        foreign_percept,
        DEFAULT_RPC_TIMEOUT,
    ) {
        Ok(resp) => {
            if let Some(err) = resp.error {
                if is_method_not_found(&err) {
                    return check_result(
                        "B-MUST-04",
                        TITLE,
                        CheckStatus::Skip,
                        format!("target does not implement percept/emit: {}", err.message),
                    );
                }
            } else if let Some(res) = resp.result
                && res.accepted
            {
                return check_result(
                    "B-MUST-04",
                    TITLE,
                    CheckStatus::Fail,
                    "percept/emit accepted a percept for an unowned place",
                );
            }
        }
        Err(err) => return check_result("B-MUST-04", TITLE, CheckStatus::Fail, err),
    }

    // 2. Action targeting an unowned place must be refused (error or ReceiptOutcome::Refused).
    let foreign_action = ActionExecuteParams {
        headers: Waist2Headers::new("tck-b-must-04-foreign-action"),
        action_id: ActionId::new("tck-act-foreign-place"),
        capability_id: probe.capability_id.clone(),
        places: vec![foreign_place.clone()],
        audience: Audience::new([ActorId::new("per_tck_actor")]),
        payload: OpaquePayload::from_bytes(b"foreign-place-action".as_slice()),
        timeout_ms: Some(5_000),
        h_act: None,
    };
    match session.call::<ActionExecuteParams, ActionExecuteResult>(
        Waist2Method::ActionExecute,
        foreign_action,
    ) {
        Ok(resp) => {
            if let Some(err) = resp.error {
                if is_method_not_found(&err) {
                    return check_result(
                        "B-MUST-04",
                        TITLE,
                        CheckStatus::Skip,
                        format!("target does not implement action/execute: {}", err.message),
                    );
                }
            } else if let Some(res) = resp.result {
                let refused = !res.accepted
                    || res
                        .receipt
                        .as_ref()
                        .is_some_and(|r| r.outcome == ReceiptOutcome::Refused);
                if !refused {
                    return check_result(
                        "B-MUST-04",
                        TITLE,
                        CheckStatus::Fail,
                        "action/execute accepted or executed an action for an unowned place",
                    );
                }
            } else {
                return check_result(
                    "B-MUST-04",
                    TITLE,
                    CheckStatus::Fail,
                    "action/execute missing result on foreign place check",
                );
            }
        }
        Err(err) => return check_result("B-MUST-04", TITLE, CheckStatus::Fail, err),
    }

    // 3. Action addressed to a declared place and specific audience must stay isolated.
    let target_aud = Audience::new([ActorId::new("per_tck_alice")]);
    let outbound_start = session.outbound_rpcs.len();
    let owned_action = ActionExecuteParams {
        headers: Waist2Headers::new("tck-b-must-04-owned-action"),
        action_id: ActionId::new("tck-act-owned-place"),
        capability_id: probe.capability_id.clone(),
        places: vec![probe.place_id.clone()],
        audience: target_aud.clone(),
        payload: OpaquePayload::from_bytes(b"owned-place-action".as_slice()),
        timeout_ms: Some(5_000),
        h_act: None,
    };
    match session
        .call::<ActionExecuteParams, ActionExecuteResult>(Waist2Method::ActionExecute, owned_action)
    {
        Ok(resp) => {
            if let Some(err) = resp.error {
                return check_result(
                    "B-MUST-04",
                    TITLE,
                    CheckStatus::Fail,
                    format!(
                        "action/execute for owned place returned error [{}]: {}",
                        err.code, err.message
                    ),
                );
            }
            let Some(res) = resp.result else {
                return check_result(
                    "B-MUST-04",
                    TITLE,
                    CheckStatus::Fail,
                    "action/execute for owned place missing result",
                );
            };
            if !res.accepted {
                return check_result(
                    "B-MUST-04",
                    TITLE,
                    CheckStatus::Fail,
                    "action/execute for owned place was not accepted",
                );
            }
            if let Some(ref rcpt) = res.receipt
                && rcpt.outcome == ReceiptOutcome::Refused
            {
                return check_result(
                    "B-MUST-04",
                    TITLE,
                    CheckStatus::Fail,
                    "action/execute for owned place was unexpectedly refused",
                );
            }
        }
        Err(err) => return check_result("B-MUST-04", TITLE, CheckStatus::Fail, err),
    }

    // Check any outbound percepts emitted during owned execution didn't cross place/audience.
    for rec in &session.outbound_rpcs[outbound_start..] {
        if rec.method == "percept/emit" {
            if let Some(place) = rec.params.get("place").and_then(Value::as_str)
                && place != probe.place_id.as_str()
            {
                return check_result(
                    "B-MUST-04",
                    TITLE,
                    CheckStatus::Fail,
                    format!("outbound percept crossed into unaddressed place '{place}'"),
                );
            }
            if let Some(aud_val) = rec.params.get("audience")
                && let Ok(aud) = serde_json::from_value::<Audience>(aud_val.clone())
                && !aud.is_subset_of(&target_aud)
            {
                return check_result(
                    "B-MUST-04",
                    TITLE,
                    CheckStatus::Fail,
                    "outbound percept leaked outside addressed audience",
                );
            }
        }
    }

    check_result(
        "B-MUST-04",
        TITLE,
        CheckStatus::Pass,
        "unowned place percept/action refused and owned place/audience stayed isolated",
    )
}

fn run_b_must_05(session: &mut ChildRpcSession, probe: &ProbeTarget) -> ConformanceCheckResult {
    const TITLE: &str = "held action receipt round-trips through approval/resolve exactly once";

    let h_act = HAct::new("blake3:tck-held-01");
    let mut held_action_id: Option<ActionId> = None;

    for (idx, cap_id) in probe.all_capabilities.iter().enumerate() {
        let candidate_id = ActionId::new(format!("tck-act-held-{idx:02}"));
        let exec_params = ActionExecuteParams {
            headers: Waist2Headers::new(format!("tck-b-must-05-held-exec-{idx}")),
            action_id: candidate_id.clone(),
            capability_id: cap_id.clone(),
            places: vec![probe.place_id.clone()],
            audience: Audience::new([ActorId::new("per_tck_actor")]),
            payload: OpaquePayload::from_bytes(b"hold".as_slice()),
            timeout_ms: Some(5_000),
            h_act: Some(h_act.clone()),
        };

        let resp = match session.call::<ActionExecuteParams, ActionExecuteResult>(
            Waist2Method::ActionExecute,
            exec_params,
        ) {
            Ok(resp) => resp,
            Err(err) => return check_result("B-MUST-05", TITLE, CheckStatus::Fail, err),
        };

        if let Some(err) = resp.error {
            if is_method_not_found(&err) {
                return check_result(
                    "B-MUST-05",
                    TITLE,
                    CheckStatus::Skip,
                    format!("target does not implement action/execute: {}", err.message),
                );
            }
            continue;
        }
        let Some(res) = resp.result else {
            continue;
        };
        if !res.accepted {
            continue;
        }

        let is_held = match res.receipt {
            Some(ref r) => r.outcome == ReceiptOutcome::Held,
            None => matches!(
                session.call::<StateQueryParams, StateQueryResult>(
                    Waist2Method::StateQuery,
                    StateQueryParams {
                        headers: Waist2Headers::new(format!("tck-b-must-05-sq-pre-{idx}")),
                        action_id: candidate_id.clone(),
                    },
                ),
                Ok(Waist2Response {
                    result: Some(StateQueryResult {
                        status: StateQueryStatus::Terminal { ref receipt },
                        ..
                    }),
                    error: None,
                    ..
                }) if receipt.outcome == ReceiptOutcome::Held
            ),
        };

        if is_held {
            held_action_id = Some(candidate_id);
            break;
        }
    }

    let Some(action_id) = held_action_id else {
        return check_result(
            "B-MUST-05",
            TITLE,
            CheckStatus::Skip,
            "target did not emit a held receipt on approval probe",
        );
    };

    let outbound_before_resolve = session.outbound_rpcs.len();

    // First approval/resolve -> applied: true, terminal receipt.
    let resolve_1 = ApprovalResolveParams {
        headers: Waist2Headers::new("tck-b-must-05-approve-1"),
        action_id: action_id.clone(),
        h_act: h_act.clone(),
        state_epoch: 1,
        approved: true,
        resolver: ActorId::new("per_tck_owner"),
        resolved_at_ms: Time::from_millis(1_000),
        signature: None,
        out_of_band_channel: false,
    };
    match session.call::<ApprovalResolveParams, ApprovalResolveResult>(
        Waist2Method::ApprovalResolve,
        resolve_1,
    ) {
        Ok(resp) => {
            if let Some(err) = resp.error {
                if is_method_not_found(&err) {
                    return check_result(
                        "B-MUST-05",
                        TITLE,
                        CheckStatus::Skip,
                        format!(
                            "target does not implement approval/resolve: {}",
                            err.message
                        ),
                    );
                }
                return check_result(
                    "B-MUST-05",
                    TITLE,
                    CheckStatus::Fail,
                    format!(
                        "first approval/resolve returned error [{}]: {}",
                        err.code, err.message
                    ),
                );
            }
            match resp.result {
                Some(ApprovalResolveResult { applied: true }) => {}
                Some(ApprovalResolveResult { applied: false }) => {
                    return check_result(
                        "B-MUST-05",
                        TITLE,
                        CheckStatus::Fail,
                        "first approval/resolve returned applied=false for held action",
                    );
                }
                None => {
                    return check_result(
                        "B-MUST-05",
                        TITLE,
                        CheckStatus::Fail,
                        "first approval/resolve missing result",
                    );
                }
            }
        }
        Err(err) => return check_result("B-MUST-05", TITLE, CheckStatus::Fail, err),
    }

    // Verify state/query shows terminal receipt (Done).
    match session.call::<StateQueryParams, StateQueryResult>(
        Waist2Method::StateQuery,
        StateQueryParams {
            headers: Waist2Headers::new("tck-b-must-05-sq-post"),
            action_id: action_id.clone(),
        },
    ) {
        Ok(resp) => {
            if let Some(err) = resp.error {
                return check_result(
                    "B-MUST-05",
                    TITLE,
                    CheckStatus::Fail,
                    format!(
                        "post-approval state/query returned error [{}]: {}",
                        err.code, err.message
                    ),
                );
            }
            match resp.result {
                Some(StateQueryResult {
                    status: StateQueryStatus::Terminal { receipt },
                    ..
                }) if receipt.outcome == ReceiptOutcome::Done => {}
                Some(other) => {
                    return check_result(
                        "B-MUST-05",
                        TITLE,
                        CheckStatus::Fail,
                        format!(
                            "expected Terminal(Done) after approval/resolve, got {:?}",
                            other.status
                        ),
                    );
                }
                None => {
                    return check_result(
                        "B-MUST-05",
                        TITLE,
                        CheckStatus::Fail,
                        "post-approval state/query missing result",
                    );
                }
            }
        }
        Err(err) => return check_result("B-MUST-05", TITLE, CheckStatus::Fail, err),
    }

    let outbound_after_first = extract_outbound_receipts(
        &session.outbound_rpcs[outbound_before_resolve..],
        &action_id,
    );
    if outbound_after_first.len() > 1 {
        return check_result(
            "B-MUST-05",
            TITLE,
            CheckStatus::Fail,
            format!(
                "approval/resolve emitted {} terminal receipts (expected at most 1)",
                outbound_after_first.len()
            ),
        );
    }

    // Second approval/resolve with a fresh idempotency_key must be refused / not applied.
    let outbound_before_second = session.outbound_rpcs.len();
    let resolve_2 = ApprovalResolveParams {
        headers: Waist2Headers::new("tck-b-must-05-approve-2"),
        action_id: action_id.clone(),
        h_act,
        state_epoch: 1,
        approved: true,
        resolver: ActorId::new("per_tck_owner"),
        resolved_at_ms: Time::from_millis(1_100),
        signature: None,
        out_of_band_channel: false,
    };
    match session.call::<ApprovalResolveParams, ApprovalResolveResult>(
        Waist2Method::ApprovalResolve,
        resolve_2,
    ) {
        Ok(resp) => {
            if resp.error.is_none()
                && matches!(resp.result, Some(ApprovalResolveResult { applied: true }))
            {
                return check_result(
                    "B-MUST-05",
                    TITLE,
                    CheckStatus::Fail,
                    "second approval/resolve returned applied=true for already-settled action",
                );
            }
        }
        Err(err) => return check_result("B-MUST-05", TITLE, CheckStatus::Fail, err),
    }

    let outbound_after_second =
        extract_outbound_receipts(&session.outbound_rpcs[outbound_before_second..], &action_id);
    if !outbound_after_second.is_empty() {
        return check_result(
            "B-MUST-05",
            TITLE,
            CheckStatus::Fail,
            "second approval/resolve emitted an extra terminal action/receipt",
        );
    }

    check_result(
        "B-MUST-05",
        TITLE,
        CheckStatus::Pass,
        "held receipt settled once via approval/resolve and rejected duplicate resolution",
    )
}

fn run_b_must_06(session: &mut ChildRpcSession) -> ConformanceCheckResult {
    const TITLE: &str = "corrupted out-of-band blob bytes fail with -32005 ERR_HASH_MISMATCH before percept reference";

    let valid_bytes = b"tck-valid-blob-payload".to_vec();
    let valid_sha = sha256_hex(&valid_bytes);
    let put_valid = BlobPutParams {
        headers: Waist2Headers::new("tck-b-must-06-blob-put-valid"),
        sha256: valid_sha.clone(),
        byte_len: valid_bytes.len() as u64,
        inline_bytes: Some(OpaquePayload::from_bytes(valid_bytes.clone())),
    };

    let valid_ref =
        match session.call::<BlobPutParams, BlobPutResult>(Waist2Method::BlobPut, put_valid) {
            Ok(resp) => {
                if let Some(err) = resp.error {
                    if is_method_not_found(&err) {
                        return check_result(
                            "B-MUST-06",
                            TITLE,
                            CheckStatus::Skip,
                            format!("target does not implement blob/put: {}", err.message),
                        );
                    }
                    return check_result(
                        "B-MUST-06",
                        TITLE,
                        CheckStatus::Fail,
                        format!("valid blob/put failed [{}]: {}", err.code, err.message),
                    );
                }
                let Some(res) = resp.result else {
                    return check_result(
                        "B-MUST-06",
                        TITLE,
                        CheckStatus::Fail,
                        "valid blob/put missing result",
                    );
                };
                if !res.sha256.eq_ignore_ascii_case(&valid_sha) {
                    return check_result(
                        "B-MUST-06",
                        TITLE,
                        CheckStatus::Fail,
                        "valid blob/put returned mismatched sha256",
                    );
                }
                res.blob_ref
            }
            Err(err) => return check_result("B-MUST-06", TITLE, CheckStatus::Fail, err),
        };

    let bad_bytes = b"corrupted-tck-blob-bytes".to_vec();
    let expected_sha = sha256_hex(b"expected-uncorrupted-bytes");
    let outbound_start = session.outbound_rpcs.len();
    let put_corrupt = BlobPutParams {
        headers: Waist2Headers::new("tck-b-must-06-blob-put-corrupt"),
        sha256: expected_sha.clone(),
        byte_len: bad_bytes.len() as u64,
        inline_bytes: Some(OpaquePayload::from_bytes(bad_bytes)),
    };

    match session.call::<BlobPutParams, BlobPutResult>(Waist2Method::BlobPut, put_corrupt) {
        Ok(resp) => {
            if let Some(err) = resp.error {
                if err.code != Waist2ErrorCode::HashMismatch.code() {
                    return check_result(
                        "B-MUST-06",
                        TITLE,
                        CheckStatus::Fail,
                        format!(
                            "corrupt blob/put returned code {}, expected -32005 ERR_HASH_MISMATCH",
                            err.code
                        ),
                    );
                }
            } else {
                return check_result(
                    "B-MUST-06",
                    TITLE,
                    CheckStatus::Fail,
                    "corrupt blob/put was accepted instead of failing with -32005 ERR_HASH_MISMATCH",
                );
            }
        }
        Err(err) => return check_result("B-MUST-06", TITLE, CheckStatus::Fail, err),
    }

    for rec in &session.outbound_rpcs[outbound_start..] {
        if rec.method == "percept/emit" && rec.params.to_string().contains(&expected_sha) {
            return check_result(
                "B-MUST-06",
                TITLE,
                CheckStatus::Fail,
                "corrupted blob digest was referenced by an outbound percept/emit",
            );
        }
    }

    // Also verify blob/get rejects a mismatched expected_sha256 with -32005.
    let get_mismatch = BlobGetParams {
        headers: Waist2Headers::new("tck-b-must-06-blob-get-mismatch"),
        blob_ref: valid_ref,
        expected_sha256: Some(expected_sha),
    };
    match session.call::<BlobGetParams, BlobGetResult>(Waist2Method::BlobGet, get_mismatch) {
        Ok(resp) => {
            if let Some(err) = resp.error {
                if !is_method_not_found(&err) && err.code != Waist2ErrorCode::HashMismatch.code() {
                    return check_result(
                        "B-MUST-06",
                        TITLE,
                        CheckStatus::Fail,
                        format!(
                            "mismatched blob/get returned code {}, expected -32005 ERR_HASH_MISMATCH",
                            err.code
                        ),
                    );
                }
            } else {
                return check_result(
                    "B-MUST-06",
                    TITLE,
                    CheckStatus::Fail,
                    "mismatched blob/get succeeded instead of returning -32005 ERR_HASH_MISMATCH",
                );
            }
        }
        Err(err) => return check_result("B-MUST-06", TITLE, CheckStatus::Fail, err),
    }

    check_result(
        "B-MUST-06",
        TITLE,
        CheckStatus::Pass,
        "corrupted blob bytes rejected with -32005 ERR_HASH_MISMATCH without percept reference",
    )
}

fn run_b_must_07(session: &mut ChildRpcSession, probe: &ProbeTarget) -> ConformanceCheckResult {
    const TITLE: &str = "unknown capability yields typed -32001 ERR_UNKNOWN_CAPABILITY refusal";

    let params = ActionExecuteParams {
        headers: Waist2Headers::new("tck-b-must-07-unknown-cap"),
        action_id: ActionId::new("tck-act-unknown-cap-01"),
        capability_id: CapabilityId::new("tck.nonexistent.capability.xyz"),
        places: vec![probe.place_id.clone()],
        audience: Audience::new([ActorId::new("per_tck_actor")]),
        payload: OpaquePayload::from_bytes(b"unknown-cap-probe".as_slice()),
        timeout_ms: Some(5_000),
        h_act: None,
    };

    match session
        .call::<ActionExecuteParams, ActionExecuteResult>(Waist2Method::ActionExecute, params)
    {
        Ok(resp) => {
            if let Some(err) = resp.error {
                if is_method_not_found(&err) {
                    return check_result(
                        "B-MUST-07",
                        TITLE,
                        CheckStatus::Skip,
                        format!("target does not implement action/execute: {}", err.message),
                    );
                }
                if err.code == Waist2ErrorCode::UnknownCapability.code() {
                    check_result(
                        "B-MUST-07",
                        TITLE,
                        CheckStatus::Pass,
                        "unknown capability rejected with -32001 ERR_UNKNOWN_CAPABILITY",
                    )
                } else {
                    check_result(
                        "B-MUST-07",
                        TITLE,
                        CheckStatus::Fail,
                        format!(
                            "unknown capability returned error code {}, expected -32001 ERR_UNKNOWN_CAPABILITY",
                            err.code
                        ),
                    )
                }
            } else {
                check_result(
                    "B-MUST-07",
                    TITLE,
                    CheckStatus::Fail,
                    "unknown capability was accepted/executed instead of returning -32001 ERR_UNKNOWN_CAPABILITY",
                )
            }
        }
        Err(err) => check_result("B-MUST-07", TITLE, CheckStatus::Fail, err),
    }
}

fn run_b_should_01(session: &mut ChildRpcSession, probe: &ProbeTarget) -> ConformanceCheckResult {
    const TITLE: &str = "body/heartbeat reports liveness acknowledgement";

    let ts = Time::from_millis(12_345);
    let params = BodyHeartbeatParams {
        headers: Waist2Headers::new("tck-b-should-01-hb"),
        body_id: probe.body_id.clone(),
        status: BodyStatus::Ready,
        timestamp_ms: ts,
    };

    match session
        .call::<BodyHeartbeatParams, BodyHeartbeatResult>(Waist2Method::BodyHeartbeat, params)
    {
        Ok(resp) => {
            if let Some(err) = resp.error {
                if is_method_not_found(&err) {
                    return check_result(
                        "B-SHOULD-01",
                        TITLE,
                        CheckStatus::Skip,
                        format!("target does not implement body/heartbeat: {}", err.message),
                    );
                }
                return check_result(
                    "B-SHOULD-01",
                    TITLE,
                    CheckStatus::Fail,
                    format!(
                        "body/heartbeat returned error [{}]: {}",
                        err.code, err.message
                    ),
                );
            }
            let Some(res) = resp.result else {
                return check_result(
                    "B-SHOULD-01",
                    TITLE,
                    CheckStatus::Fail,
                    "body/heartbeat missing result",
                );
            };
            if res.acknowledged_at_ms < ts {
                return check_result(
                    "B-SHOULD-01",
                    TITLE,
                    CheckStatus::Fail,
                    format!(
                        "body/heartbeat acknowledged_at_ms ({}) < sent timestamp ({})",
                        res.acknowledged_at_ms, ts
                    ),
                );
            }
            check_result(
                "B-SHOULD-01",
                TITLE,
                CheckStatus::Pass,
                "body/heartbeat acknowledged liveness with monotonic timestamp",
            )
        }
        Err(err) => check_result("B-SHOULD-01", TITLE, CheckStatus::Fail, err),
    }
}

fn run_b_should_02(session: &mut ChildRpcSession, probe: &ProbeTarget) -> ConformanceCheckResult {
    const TITLE: &str =
        "honest delivery timing enforces d_art=0ms and 1s working_cue / 10s progress_note contract";

    let mut headers = Waist2Headers::new("tck-b-should-02-timing");
    headers.meta.insert(
        "org.monoagens.delivery_probe".to_owned(),
        MetaValue::Bool(true),
    );
    let params = ActionExecuteParams {
        headers,
        action_id: ActionId::new("tck-act-timing-01"),
        capability_id: probe.capability_id.clone(),
        places: vec![probe.place_id.clone()],
        audience: Audience::new([ActorId::new("per_tck_actor")]),
        payload: OpaquePayload::from_bytes(b"delivery-timing-probe".as_slice()),
        timeout_ms: Some(5_000),
        h_act: None,
    };

    let started = Instant::now();
    let resp = match session
        .call::<ActionExecuteParams, ActionExecuteResult>(Waist2Method::ActionExecute, params)
    {
        Ok(resp) => resp,
        Err(err) => return check_result("B-SHOULD-02", TITLE, CheckStatus::Fail, err),
    };
    let elapsed = started.elapsed();

    if let Some(err) = resp.error {
        if is_method_not_found(&err) {
            return check_result(
                "B-SHOULD-02",
                TITLE,
                CheckStatus::Skip,
                format!("target does not implement action/execute: {}", err.message),
            );
        }
        return check_result(
            "B-SHOULD-02",
            TITLE,
            CheckStatus::Fail,
            format!(
                "timing probe returned error [{}]: {}",
                err.code, err.message
            ),
        );
    }

    let Some(res) = resp.result else {
        return check_result(
            "B-SHOULD-02",
            TITLE,
            CheckStatus::Fail,
            "timing probe missing result",
        );
    };
    if !res.accepted {
        return check_result(
            "B-SHOULD-02",
            TITLE,
            CheckStatus::Fail,
            "timing probe action/execute was not accepted",
        );
    }

    if elapsed >= Duration::from_millis(500) {
        return check_result(
            "B-SHOULD-02",
            TITLE,
            CheckStatus::Fail,
            "immediate action exceeded 500ms turnaround (violates d_art=0ms)",
        );
    }

    // Literal reading of underspecified SHOULD cue schema: if receipt.detail carries JSON timing
    // fields (d_art_ms / working_cue_ms / progress_note_ms), verify exact R-47 constants.
    if let Some(detail_str) = res.receipt.as_ref().and_then(|r| r.detail.as_deref())
        && let Ok(meta_json) = serde_json::from_str::<Value>(detail_str)
        && meta_json.is_object()
    {
        if let Some(d_art) = meta_json.get("d_art_ms").and_then(Value::as_u64)
            && d_art != 0
        {
            return check_result(
                "B-SHOULD-02",
                TITLE,
                CheckStatus::Fail,
                format!("body reported artificial delay d_art_ms={d_art}, expected 0"),
            );
        }
        if let Some(wc) = meta_json.get("working_cue_ms").and_then(Value::as_u64)
            && wc != 1_000
        {
            return check_result(
                "B-SHOULD-02",
                TITLE,
                CheckStatus::Fail,
                format!("body reported working_cue_ms={wc}, expected 1000"),
            );
        }
        if let Some(pn) = meta_json.get("progress_note_ms").and_then(Value::as_u64)
            && pn != 10_000
        {
            return check_result(
                "B-SHOULD-02",
                TITLE,
                CheckStatus::Fail,
                format!("body reported progress_note_ms={pn}, expected 10000"),
            );
        }
    }

    check_result(
        "B-SHOULD-02",
        TITLE,
        CheckStatus::Pass,
        "immediate action completed with d_art=0ms under 1000ms working_cue / 10000ms progress_note threshold",
    )
}

fn run_b_should_03(session: &mut ChildRpcSession, probe: &ProbeTarget) -> ConformanceCheckResult {
    const TITLE: &str = "percept/emit and action/execute reject undeclared audience with -32006 ERR_UNDECLARED_AUDIENCE";

    // 1. Omitted audience on percept/emit -> -32006.
    let missing_aud_percept = json!({
        "headers": Waist2Headers::new("tck-b-should-03-missing-aud"),
        "actor": "per_tck_actor",
        "place": probe.place_id,
        "timestamp_ms": 2_000,
        "payload": b"missing-audience".to_vec()
    });
    match session.call_raw_with_timeout::<PerceptEmitResult>(
        Waist2Method::PerceptEmit,
        missing_aud_percept,
        DEFAULT_RPC_TIMEOUT,
    ) {
        Ok(resp) => {
            if let Some(err) = resp.error {
                if is_method_not_found(&err) {
                    return check_result(
                        "B-SHOULD-03",
                        TITLE,
                        CheckStatus::Skip,
                        format!("target does not implement percept/emit: {}", err.message),
                    );
                }
                if err.code != Waist2ErrorCode::UndeclaredAudience.code() {
                    return check_result(
                        "B-SHOULD-03",
                        TITLE,
                        CheckStatus::Fail,
                        format!(
                            "missing audience percept/emit returned code {}, expected -32006 ERR_UNDECLARED_AUDIENCE",
                            err.code
                        ),
                    );
                }
            } else {
                return check_result(
                    "B-SHOULD-03",
                    TITLE,
                    CheckStatus::Fail,
                    "percept/emit accepted missing audience instead of returning -32006",
                );
            }
        }
        Err(err) => return check_result("B-SHOULD-03", TITLE, CheckStatus::Fail, err),
    }

    // 2. Empty audience array on percept/emit -> -32006.
    let empty_aud_percept = json!({
        "headers": Waist2Headers::new("tck-b-should-03-empty-aud"),
        "actor": "per_tck_actor",
        "place": probe.place_id,
        "audience": [],
        "timestamp_ms": 2_001,
        "payload": b"empty-audience".to_vec()
    });
    match session.call_raw_with_timeout::<PerceptEmitResult>(
        Waist2Method::PerceptEmit,
        empty_aud_percept,
        DEFAULT_RPC_TIMEOUT,
    ) {
        Ok(resp) => {
            if let Some(err) = resp.error {
                if err.code != Waist2ErrorCode::UndeclaredAudience.code() {
                    return check_result(
                        "B-SHOULD-03",
                        TITLE,
                        CheckStatus::Fail,
                        format!(
                            "empty audience percept/emit returned code {}, expected -32006 ERR_UNDECLARED_AUDIENCE",
                            err.code
                        ),
                    );
                }
            } else {
                return check_result(
                    "B-SHOULD-03",
                    TITLE,
                    CheckStatus::Fail,
                    "percept/emit accepted empty audience instead of returning -32006",
                );
            }
        }
        Err(err) => return check_result("B-SHOULD-03", TITLE, CheckStatus::Fail, err),
    }

    // 3. Empty audience on action/execute -> -32006.
    let empty_aud_action = ActionExecuteParams {
        headers: Waist2Headers::new("tck-b-should-03-empty-aud-action"),
        action_id: ActionId::new("tck-act-empty-aud-01"),
        capability_id: probe.capability_id.clone(),
        places: vec![probe.place_id.clone()],
        audience: Audience::empty(),
        payload: OpaquePayload::from_bytes(b"empty-audience-action".as_slice()),
        timeout_ms: Some(5_000),
        h_act: None,
    };
    match session.call::<ActionExecuteParams, ActionExecuteResult>(
        Waist2Method::ActionExecute,
        empty_aud_action,
    ) {
        Ok(resp) => {
            if let Some(err) = resp.error {
                if err.code != Waist2ErrorCode::UndeclaredAudience.code() {
                    return check_result(
                        "B-SHOULD-03",
                        TITLE,
                        CheckStatus::Fail,
                        format!(
                            "empty audience action/execute returned code {}, expected -32006 ERR_UNDECLARED_AUDIENCE",
                            err.code
                        ),
                    );
                }
            } else {
                return check_result(
                    "B-SHOULD-03",
                    TITLE,
                    CheckStatus::Fail,
                    "action/execute accepted empty audience instead of returning -32006",
                );
            }
        }
        Err(err) => return check_result("B-SHOULD-03", TITLE, CheckStatus::Fail, err),
    }

    check_result(
        "B-SHOULD-03",
        TITLE,
        CheckStatus::Pass,
        "missing and empty audiences rejected with -32006 ERR_UNDECLARED_AUDIENCE",
    )
}

/// Executes the standalone black-box Core TCK (`C-MUST-01..04`, plus `C-SHOULD-01..04`
/// when `--level should` or `may`) against a spawned core fixture/binary speaking NDJSON `stdio`.
pub fn run_core_conformance(
    target: &str,
    extra_args: &[String],
    level: ConformanceLevel,
) -> Result<CompatibilityReport, ConformanceError> {
    let state_dir = TempDir::new().map_err(|e| ConformanceError::Io(e.to_string()))?;
    let mut session = CoreProbeSession::spawn(target, extra_args, state_dir.path())?;
    let mut checks = vec![
        run_c_must_01(&mut session),
        run_c_must_02(&mut session),
        run_c_must_03(&mut session),
        run_c_must_04(&mut session),
    ];

    if level >= ConformanceLevel::Should {
        checks.push(run_c_should_01(&mut session));
        checks.push(run_c_should_02(&mut session));
        checks.push(run_c_should_03(&mut session));
        checks.push(run_c_should_04(&mut session));
    }

    if level >= ConformanceLevel::May {
        checks.push(check_result(
            "C-MAY-01",
            "Optional extended core hints",
            CheckStatus::Skip,
            "deferred to future milestone: optional extended core telemetry",
        ));
    }

    session.shutdown_and_wait();

    Ok(CompatibilityReport {
        schema_version: "1.0".to_owned(),
        target: target.to_owned(),
        level,
        checks,
    })
}

/// Backward-compatible seam delegating to [`run_core_conformance`] when given a target binary.
pub fn run_core_conformance_seam() -> Result<CompatibilityReport, ConformanceError> {
    Err(ConformanceError::NotImplemented(
        "pass --target <core-tck-harness> to run Core TCK (C-MUST-01..04)",
    ))
}

struct CoreProbeSession {
    child: Child,
    stdin: Option<BufWriter<ChildStdin>>,
    stdout_rx: Receiver<Result<Option<String>, String>>,
    waited: bool,
}

impl Drop for CoreProbeSession {
    fn drop(&mut self) {
        if !self.waited {
            drop(self.stdin.take());
            let _ = self.child.kill();
            let _ = self.child.wait();
            self.waited = true;
        }
    }
}

impl CoreProbeSession {
    fn spawn(
        target: &str,
        extra_args: &[String],
        state_dir: &Path,
    ) -> Result<Self, ConformanceError> {
        let mut cmd = if Path::new(target).exists() || !target.contains(char::is_whitespace) {
            let mut c = Command::new(target);
            c.args(extra_args);
            c
        } else {
            let mut parts = target.split_whitespace();
            let exe = parts.next().ok_or_else(|| ConformanceError::SpawnFailed {
                target: target.to_owned(),
                detail: "empty target command".to_owned(),
            })?;
            let mut c = Command::new(exe);
            c.args(parts);
            c.args(extra_args);
            c
        };

        cmd.env("MONOAGENS_CORE_STATE_DIR", state_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = cmd.spawn().map_err(|e| ConformanceError::SpawnFailed {
            target: target.to_owned(),
            detail: e.to_string(),
        })?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| ConformanceError::Io("missing child stdin".to_owned()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| ConformanceError::Io("missing child stdout".to_owned()))?;
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let mut buf = String::new();
                match reader.read_line(&mut buf) {
                    Ok(0) => {
                        let _ = tx.send(Ok(None));
                        break;
                    }
                    Ok(_) => {
                        if tx.send(Ok(Some(buf))).is_err() {
                            break;
                        }
                    }
                    Err(e) => {
                        let _ = tx.send(Err(e.to_string()));
                        break;
                    }
                }
            }
        });

        Ok(Self {
            child,
            stdin: Some(BufWriter::new(stdin)),
            stdout_rx: rx,
            waited: false,
        })
    }

    fn probe(&mut self, op: &str, timeout: Duration) -> Result<Value, String> {
        let Some(stdin) = self.stdin.as_mut() else {
            return Err("core target stdin closed".to_owned());
        };
        let encoded = serde_json::to_string(&json!({ "op": op }))
            .map_err(|e| format!("encode core probe: {e}"))?;
        writeln!(stdin, "{encoded}").map_err(|e| format!("write core probe '{op}': {e}"))?;
        stdin
            .flush()
            .map_err(|e| format!("flush core probe '{op}': {e}"))?;
        match self.stdout_rx.recv_timeout(timeout) {
            Ok(Ok(Some(line))) => {
                let val: Value = serde_json::from_str(line.trim())
                    .map_err(|e| format!("invalid JSON response for '{op}': {e}"))?;
                if val.get("ok").and_then(Value::as_bool) != Some(true) {
                    let detail = val
                        .get("error")
                        .and_then(Value::as_str)
                        .unwrap_or("probe returned ok=false");
                    return Err(format!("core probe '{op}' failed: {detail}"));
                }
                Ok(val)
            }
            Ok(Ok(None)) => Err(format!("core target exited before answering '{op}'")),
            Ok(Err(e)) => Err(format!("core stdout read error on '{op}': {e}")),
            Err(RecvTimeoutError::Timeout) => Err(format!("timed out waiting for '{op}'")),
            Err(RecvTimeoutError::Disconnected) => {
                Err(format!("core target disconnected on '{op}'"))
            }
        }
    }

    fn shutdown_and_wait(&mut self) {
        let _ = self.probe("shutdown", DEFAULT_RPC_TIMEOUT);
        drop(self.stdin.take());
        let _ = self.child.wait();
        self.waited = true;
    }
}

fn run_c_must_01(session: &mut CoreProbeSession) -> ConformanceCheckResult {
    const TITLE: &str =
        "VisibleScope non-interference Delta_NI == 0 across private vs public histories";
    match session.probe("probe_scope", DEFAULT_RPC_TIMEOUT) {
        Ok(v) if v.get("delta_ni_zero").and_then(Value::as_bool) == Some(true) => check_result(
            "C-MUST-01",
            TITLE,
            CheckStatus::Pass,
            format!(
                "Delta_NI == 0 verified (sig_match={}, priv_use_count={}, source_leaked={})",
                v["sig_match"], v["priv_use_count"], v["source_leaked"]
            ),
        ),
        Ok(v) => check_result(
            "C-MUST-01",
            TITLE,
            CheckStatus::Fail,
            format!(
                "VisibleScope leakage detected: delta_ni_zero={}, sig_match={}, priv_use_count={}, source_leaked={}",
                v["delta_ni_zero"], v["sig_match"], v["priv_use_count"], v["source_leaked"]
            ),
        ),
        Err(err) => check_result("C-MUST-01", TITLE, CheckStatus::Fail, err),
    }
}

fn run_c_must_02(session: &mut CoreProbeSession) -> ConformanceCheckResult {
    const TITLE: &str = "hard_forget erasure leaves zero residue in store/FTS5/snapshot/replay";
    match session.probe("probe_forget", Duration::from_secs(10)) {
        Ok(v) if v.get("erasure_clean").and_then(Value::as_bool) == Some(true) => check_result(
            "C-MUST-02",
            TITLE,
            CheckStatus::Pass,
            format!(
                "hard_forget erased all projections (mem={}, raw_hits={}, post_forget_replay_ok={})",
                v["surviving_memory_entries"], v["raw_byte_scan_hits"], v["post_forget_replay_ok"]
            ),
        ),
        Ok(v) => check_result(
            "C-MUST-02",
            TITLE,
            CheckStatus::Fail,
            format!(
                "erasure/resurrection failure: erasure_clean={}, mem={}, raw_hits={}, post_forget_replay_ok={}",
                v["erasure_clean"],
                v["surviving_memory_entries"],
                v["raw_byte_scan_hits"],
                v["post_forget_replay_ok"]
            ),
        ),
        Err(err) => check_result("C-MUST-02", TITLE, CheckStatus::Fail, err),
    }
}

fn run_c_must_03(session: &mut CoreProbeSession) -> ConformanceCheckResult {
    const TITLE: &str = "Deterministic seeded log replay is bit-identical across runs";
    match session.probe("probe_replay", DEFAULT_RPC_TIMEOUT) {
        Ok(v) if v.get("bit_identical").and_then(Value::as_bool) == Some(true) => check_result(
            "C-MUST-03",
            TITLE,
            CheckStatus::Pass,
            format!(
                "seeded replay bit-identical (events={}, replay_verify_ok={})",
                v["event_count"], v["replay_verify_ok"]
            ),
        ),
        Ok(v) => check_result(
            "C-MUST-03",
            TITLE,
            CheckStatus::Fail,
            format!(
                "non-deterministic seeded replay: bit_identical={}, replay_verify_ok={}",
                v["bit_identical"], v["replay_verify_ok"]
            ),
        ),
        Err(err) => check_result("C-MUST-03", TITLE, CheckStatus::Fail, err),
    }
}

fn run_c_must_04(session: &mut CoreProbeSession) -> ConformanceCheckResult {
    const TITLE: &str =
        "Neutral-profile 10k-tick soak maintains finite bounded state without crashes";
    match session.probe("probe_soak", Duration::from_secs(15)) {
        Ok(v) if v.get("weights_finite").and_then(Value::as_bool) == Some(true) => check_result(
            "C-MUST-04",
            TITLE,
            CheckStatus::Pass,
            format!(
                "10k-tick neutral soak finite & stable (ticks={}, events={})",
                v["ticks_completed"], v["total_events"]
            ),
        ),
        Ok(v) => check_result(
            "C-MUST-04",
            TITLE,
            CheckStatus::Fail,
            format!(
                "neutral soak instability/NaN detected: ticks={}, weights_finite={}",
                v["ticks_completed"], v["weights_finite"]
            ),
        ),
        Err(err) => check_result("C-MUST-04", TITLE, CheckStatus::Fail, err),
    }
}

fn run_c_should_01(session: &mut CoreProbeSession) -> ConformanceCheckResult {
    const TITLE: &str =
        "Core loop input queue ordering preserves priority without dropping percepts";
    match session.probe("probe_queue_order", DEFAULT_RPC_TIMEOUT) {
        Ok(v) if v.get("priority_preserved").and_then(Value::as_bool) == Some(true) => {
            check_result(
                "C-SHOULD-01",
                TITLE,
                CheckStatus::Pass,
                format!("queue priority preserved (events={})", v["total_events"]),
            )
        }
        Ok(v) => check_result(
            "C-SHOULD-01",
            TITLE,
            CheckStatus::Fail,
            format!("queue ordering violation: {}", v),
        ),
        Err(err) => check_result("C-SHOULD-01", TITLE, CheckStatus::Fail, err),
    }
}

fn run_c_should_02(session: &mut CoreProbeSession) -> ConformanceCheckResult {
    const TITLE: &str =
        "Single-receipt preemption records cancellation without spurious learning penalty";
    match session.probe("probe_preempt", DEFAULT_RPC_TIMEOUT) {
        Ok(v) if v.get("preemption_non_penalty").and_then(Value::as_bool) == Some(true) => {
            check_result(
                "C-SHOULD-02",
                TITLE,
                CheckStatus::Pass,
                format!(
                    "preemption did not trigger rollback penalty (rollbacks={}->{})",
                    v["rollbacks_before"], v["rollbacks_after"]
                ),
            )
        }
        Ok(v) => check_result(
            "C-SHOULD-02",
            TITLE,
            CheckStatus::Fail,
            format!("preemption penalty violation: {}", v),
        ),
        Err(err) => check_result("C-SHOULD-02", TITLE, CheckStatus::Fail, err),
    }
}

fn run_c_should_03(session: &mut CoreProbeSession) -> ConformanceCheckResult {
    const TITLE: &str = "Record-first WAL crash recovery restores exact event state on Core::open";
    match session.probe("probe_crash_recovery", DEFAULT_RPC_TIMEOUT) {
        Ok(v) if v.get("recovered_cleanly").and_then(Value::as_bool) == Some(true) => check_result(
            "C-SHOULD-03",
            TITLE,
            CheckStatus::Pass,
            format!(
                "crash recovery restored {}/{} events",
                v["recovered_count"], v["first_count"]
            ),
        ),
        Ok(v) => check_result(
            "C-SHOULD-03",
            TITLE,
            CheckStatus::Fail,
            format!("crash recovery mismatch: {}", v),
        ),
        Err(err) => check_result("C-SHOULD-03", TITLE, CheckStatus::Fail, err),
    }
}

fn run_c_should_04(session: &mut CoreProbeSession) -> ConformanceCheckResult {
    const TITLE: &str =
        "Pre-Step-5 VisibleScope and Taint isolation prevents tainted private bleed";
    match session.probe("probe_pre_step5_scope", DEFAULT_RPC_TIMEOUT) {
        Ok(v) if v.get("isolated").and_then(Value::as_bool) == Some(true) => check_result(
            "C-SHOULD-04",
            TITLE,
            CheckStatus::Pass,
            "tainted private percept excluded from public ContextPlan source_cards",
        ),
        Ok(v) => check_result(
            "C-SHOULD-04",
            TITLE,
            CheckStatus::Fail,
            format!("pre-Step-5 VisibleScope/Taint bleed: {}", v),
        ),
        Err(err) => check_result("C-SHOULD-04", TITLE, CheckStatus::Fail, err),
    }
}
