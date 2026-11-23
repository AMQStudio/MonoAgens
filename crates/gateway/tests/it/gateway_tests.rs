use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use monoagens_core::{
    ContextPlan, CoreConfig, DoorDecision, Doors, EventDetail, EventKind, Input, MemStore,
    ModelPurpose, NoopDoors, QueueClass,
};
use monoagens_gateway::{
    AsyncGatewayHook, Clock, ConfigError, CueRecorder, DiagnosticSink, DoorsAdapter, GatewayConfig,
    GatewayError, GatewayOptions, GatewayRuntime, GatewaySeams, IngressQueue, LifecycleCueKind,
    ModelProviderKind, SecretsLoadError, VirtualClock, extract_cli_config_arg, load_config_file,
    load_or_init_gateway_env, parse_config_str, resolve_paths_with, update_config_with_backup,
};
use monoagens_protocol::{
    Action, ActionExecuteParams, ActionExecuteResult, ActorId, Audience, BodyDiscoverParams,
    BodyDiscoverResult, BodyId, CURRENT_PROTOCOL_VERSION, CapabilityDescriptor, CapabilityFlags,
    CapabilityId, CapabilitySchemaHash, OpaquePayload, Percept, PlaceId, Receipt, ReceiptOutcome,
    TerminationReason, Time, Waist2Method, Waist2Request, Waist2Response,
};
use monoagens_reason::{
    CallOutcome, ModelCallFailure, ModelCallRecord, ModelCallSuccess, ModelProvider, ModelRequest,
    ProviderFailureClass, ProviderSecrets, ScriptedModelProvider, ScriptedReply,
};
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader as TokioBufReader};
use tokio::sync::Notify;

fn make_percept(actor: &str, place: &str, audience: &[&str], ts_ms: u64, body: &[u8]) -> Percept {
    let mut p = Percept::minimal(
        ActorId::new(actor),
        PlaceId::new(place),
        Audience::new(audience.iter().map(|s| ActorId::new(*s))),
        Time::from_millis(ts_ms),
        OpaquePayload::from_bytes(body),
    );
    p.addressees = vec![ActorId::new("soul")];
    p
}

fn default_test_config(provider: ModelProviderKind) -> GatewayConfig {
    let mut cfg = GatewayConfig::default();
    cfg.model.provider = provider;
    cfg.model.model = "mock-gpt".to_owned();
    cfg.runtime.action_timeout_ms = 500;
    cfg.runtime.hook_timeout_ms = 200;
    cfg.runtime.model_timeout_ms = 5_000;
    cfg
}

/// Custom `Doors` wrapper verifying via its own atomic guard counter that `Core::step`
/// is never invoked concurrently from two tasks.
#[derive(Clone, Default)]
struct ConcurrencyGuardDoors {
    active: Arc<AtomicUsize>,
    max_seen: Arc<AtomicUsize>,
    calls: Arc<AtomicUsize>,
}

impl ConcurrencyGuardDoors {
    fn enter(&self) {
        let prev = self.active.fetch_add(1, Ordering::SeqCst);
        self.max_seen.fetch_max(prev + 1, Ordering::SeqCst);
        assert_eq!(prev, 0, "Doors/Core::step entered concurrently!");
    }

    fn exit(&self) {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.active.fetch_sub(1, Ordering::SeqCst);
    }
}

impl Doors for ConcurrencyGuardDoors {
    fn inbound(&mut self, _percept: &mut Percept) -> DoorDecision {
        self.enter();
        self.exit();
        DoorDecision::Allow
    }

    fn context(&mut self, _plan: &mut ContextPlan) -> DoorDecision {
        self.enter();
        self.exit();
        DoorDecision::Allow
    }

    fn action(&mut self, _action: &mut Action) -> DoorDecision {
        self.enter();
        self.exit();
        DoorDecision::Allow
    }
}

#[derive(Clone)]
struct MockHttpReply {
    status_line: &'static str,
    body: String,
}

fn spawn_mock_http_server(
    replies: Vec<MockHttpReply>,
) -> (String, Arc<Mutex<Vec<String>>>, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock server");
    let addr = listener.local_addr().expect("local addr");
    let captured = Arc::new(Mutex::new(Vec::new()));
    let captured_clone = Arc::clone(&captured);

    let handle = thread::spawn(move || {
        for reply in replies {
            let (mut stream, _) = listener.accept().expect("accept");
            let req_body = read_http_body(&mut stream);
            captured_clone.lock().expect("lock bodies").push(req_body);
            let resp = format!(
                "HTTP/1.1 {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                reply.status_line,
                reply.body.len(),
                reply.body
            );
            let _ = stream.write_all(resp.as_bytes());
            let _ = stream.flush();
        }
    });

    (
        format!("http://{addr}/v1/chat/completions"),
        captured,
        handle,
    )
}

fn read_http_body(stream: &mut TcpStream) -> String {
    let mut reader = BufReader::new(stream.try_clone().expect("clone stream"));
    let mut content_len = 0usize;
    loop {
        let mut line = String::new();
        let n = reader.read_line(&mut line).expect("read header");
        if n == 0 || line == "\r\n" || line == "\n" {
            break;
        }
        let lower = line.to_ascii_lowercase();
        if let Some(rest) = lower.strip_prefix("content-length:") {
            content_len = rest.trim().parse().unwrap_or(0);
        }
    }
    let mut buf = vec![0u8; content_len];
    if content_len > 0 {
        reader.read_exact(&mut buf).expect("read body");
    }
    String::from_utf8_lossy(&buf).into_owned()
}

#[derive(Clone, Copy)]
pub(crate) enum MockBodyBehavior {
    ImmediateDone,
    CrashMidAction,
    HangPastTimeout,
}

/// Spawns an in-memory Waist-2 NDJSON body over `tokio::io::duplex` implementing real JSON-RPC 2.0
/// framing (`body/discover`, `action/execute`, optional delayed `action/receipt`).
pub(crate) fn spawn_ndjson_mock_body(
    handle: &monoagens_gateway::GatewayHandle,
    body_id: &str,
    place_id: &str,
    cap_id: &str,
    behavior: MockBodyBehavior,
) -> (
    tokio::task::JoinHandle<Vec<ActionExecuteParams>>,
    mpsc_tx_handle::BodyControl,
) {
    let (gw_stream, body_stream) = tokio::io::duplex(64 * 1024);
    let (gw_read, gw_write) = tokio::io::split(gw_stream);
    let (body_read, mut body_write) = tokio::io::split(body_stream);

    let (cmd_tx, mut cmd_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let bid = BodyId::new(body_id);
    let pid = PlaceId::new(place_id);
    let cid = CapabilityId::new(cap_id);

    let body_task = tokio::spawn(async move {
        let mut reader = TokioBufReader::new(body_read);
        let mut line = String::new();
        let mut executed = Vec::new();

        loop {
            line.clear();
            tokio::select! {
                biased;
                maybe_outbound = cmd_rx.recv() => {
                    if let Some(json_line) = maybe_outbound {
                        let _ = body_write.write_all(json_line.as_bytes()).await;
                        let _ = body_write.write_all(b"\n").await;
                        let _ = body_write.flush().await;
                    }
                }
                read_res = reader.read_line(&mut line) => {
                    let Ok(n) = read_res else { break; };
                    if n == 0 {
                        break;
                    }
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        continue;
                    }
                    let raw_val: Value = serde_json::from_str(trimmed).expect("valid json line");
                    if raw_val.get("method").is_none() {
                        // Response from Gateway to an earlier outbound percept/receipt RPC.
                        continue;
                    }
                    let req: Waist2Request<Value> = serde_json::from_value(raw_val).expect("valid waist2 req");
                    match req.method {
                        Waist2Method::BodyDiscover => {
                            let _params: BodyDiscoverParams = serde_json::from_value(req.params).expect("discover params");
                            let res = BodyDiscoverResult {
                                body_id: bid.clone(),
                                protocol_version: CURRENT_PROTOCOL_VERSION,
                                capabilities: vec![CapabilityDescriptor {
                                    id: cid.clone(),
                                    description: format!("Capability on {bid}"),
                                    places: vec![pid.clone()],
                                    flags: CapabilityFlags {
                                        communicative: true,
                                        identity_changing: false,
                                        irreversible: false,
                                    },
                                    typical_latency_ms: 10,
                                    capability_schema_hash: CapabilitySchemaHash::new("h1"),
                                    sensed_effect: None,
                                }],
                                places: vec![pid.clone()],
                            };
                            let resp = serde_json::to_string(&Waist2Response::ok(req.id, res)).unwrap();
                            let _ = body_write.write_all(resp.as_bytes()).await;
                            let _ = body_write.write_all(b"\n").await;
                            let _ = body_write.flush().await;
                        }
                        Waist2Method::ActionExecute => {
                            let params: ActionExecuteParams = serde_json::from_value(req.params).expect("exec params");
                            executed.push(params.clone());
                            match behavior {
                                MockBodyBehavior::ImmediateDone => {
                                    let receipt = Receipt {
                                        action_id: params.action_id.clone(),
                                        capability_id: params.capability_id.clone(),
                                        outcome: ReceiptOutcome::Done,
                                        reason_code: None,
                                        detail: Some("completed".to_owned()),
                                        termination_reason: Some(TerminationReason::Normal),
                                        progress: Some(1.0),
                                        delivered_units: Some(1),
                                        h_act: params.h_act.clone(),
                                        timestamp_ms: Time::from_millis(1_000_050),
                                    };
                                    let res = ActionExecuteResult {
                                        accepted: true,
                                        receipt: Some(receipt),
                                    };
                                    let resp = serde_json::to_string(&Waist2Response::ok(req.id, res)).unwrap();
                                    let _ = body_write.write_all(resp.as_bytes()).await;
                                    let _ = body_write.write_all(b"\n").await;
                                    let _ = body_write.flush().await;
                                }
                                MockBodyBehavior::CrashMidAction => {
                                    // Accept the action asynchronously, then drop the stream (crash!).
                                    let res = ActionExecuteResult {
                                        accepted: true,
                                        receipt: None,
                                    };
                                    let resp = serde_json::to_string(&Waist2Response::ok(req.id, res)).unwrap();
                                    let _ = body_write.write_all(resp.as_bytes()).await;
                                    let _ = body_write.write_all(b"\n").await;
                                    let _ = body_write.flush().await;
                                    break;
                                }
                                MockBodyBehavior::HangPastTimeout => {
                                    // Never reply to action/execute; stay connected so late receipt can be sent.
                                }
                            }
                        }
                        Waist2Method::BodyShutdown => {
                            let resp = serde_json::to_string(&Waist2Response::ok(
                                req.id,
                                monoagens_protocol::BodyShutdownResult { accepted: true },
                            ))
                            .unwrap();
                            let _ = body_write.write_all(resp.as_bytes()).await;
                            let _ = body_write.write_all(b"\n").await;
                            let _ = body_write.flush().await;
                            break;
                        }
                        _ => {}
                    }
                }
            }
        }
        executed
    });

    // Connect on gateway side.
    let h = handle.clone();
    let expected_bid = BodyId::new(body_id);
    let ctrl = mpsc_tx_handle::BodyControl {
        cmd_tx,
        connect_fut: tokio::spawn(async move {
            h.connect_ndjson_body(gw_read, gw_write, Some(expected_bid))
                .await
                .expect("connect ndjson body")
        }),
    };

    (body_task, ctrl)
}

pub(crate) mod mpsc_tx_handle {
    use monoagens_protocol::{
        ActionId, ActionReceiptParams, BodyId, CapabilityId, Percept, PerceptEmitParams, Receipt,
        RpcId, Waist2Headers, Waist2Method, Waist2Request,
    };

    pub struct BodyControl {
        pub cmd_tx: tokio::sync::mpsc::UnboundedSender<String>,
        pub connect_fut: tokio::task::JoinHandle<BodyId>,
    }

    impl BodyControl {
        pub async fn wait_connected(self) -> tokio::sync::mpsc::UnboundedSender<String> {
            self.connect_fut.await.expect("body connect join");
            self.cmd_tx
        }

        pub fn send_receipt(
            tx: &tokio::sync::mpsc::UnboundedSender<String>,
            rpc_id: i64,
            receipt: Receipt,
        ) {
            let req = Waist2Request::new(
                Some(RpcId::Int(rpc_id)),
                Waist2Method::ActionReceipt,
                ActionReceiptParams {
                    headers: Waist2Headers::new(format!("late-rcpt-{}", receipt.action_id)),
                    receipt,
                },
            );
            let line = serde_json::to_string(&req).unwrap();
            let _ = tx.send(line);
        }

        pub fn send_percept(
            tx: &tokio::sync::mpsc::UnboundedSender<String>,
            rpc_id: i64,
            percept: Percept,
        ) {
            let req = Waist2Request::new(
                Some(RpcId::Int(rpc_id)),
                Waist2Method::PerceptEmit,
                PerceptEmitParams::from_percept(
                    Waist2Headers::new(format!("percept-{rpc_id}")),
                    percept,
                ),
            );
            let line = serde_json::to_string(&req).unwrap();
            let _ = tx.send(line);
        }

        #[allow(dead_code)]
        pub fn _keep(_: ActionId, _: CapabilityId) {}
    }
}

async fn poll_until<F, Fut>(mut check: F)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    for _ in 0..200 {
        if check().await {
            return;
        }
        tokio::task::yield_now().await;
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    panic!("condition not satisfied before test polling bound");
}

#[tokio::test]
async fn same_thread_core_ownership_serializes_concurrent_submissions() {
    let clock = Arc::new(VirtualClock::new(Time::from_millis(1_000_000)));
    let secrets = ProviderSecrets::default();
    let cfg = default_test_config(ModelProviderKind::Scripted);
    let doors = ConcurrencyGuardDoors::default();
    let max_seen = Arc::clone(&doors.max_seen);
    let door_calls = Arc::clone(&doors.calls);

    let rt = GatewayRuntime::start(
        cfg,
        &secrets,
        || Ok(MemStore::new()),
        CoreConfig::default(),
        DoorsAdapter(doors),
        GatewayOptions {
            clock: clock.clone(),
            provider: Some(Arc::new(ScriptedModelProvider::new())),
            diagnostics: DiagnosticSink::new(&secrets),
            cues: CueRecorder::new(),
            seams: GatewaySeams::default(),
        },
    )
    .await
    .expect("start gateway");

    let handle = rt.handle();
    let mut joins = Vec::new();
    for idx in 0..32 {
        let h = handle.clone();
        joins.push(tokio::spawn(async move {
            let mut p = make_percept(
                &format!("per_{idx}"),
                "room:concurrent",
                &["per_owner", "soul"],
                1_000_000 + idx as u64,
                format!("msg-{idx}").as_bytes(),
            );
            p.addressees.clear();
            h.submit_and_wait(Input::Percept(Box::new(p)))
                .await
                .expect("submit succeeds");
        }));
    }
    for j in joins {
        j.await.expect("task join");
    }

    assert_eq!(handle.ownership_stats().max_concurrent_calls(), 1);
    assert_eq!(handle.ownership_stats().total_steps(), 32);
    assert_eq!(max_seen.load(Ordering::SeqCst), 1);
    assert_eq!(door_calls.load(Ordering::SeqCst), 32);

    let events = handle.read_events().await.expect("read events");
    let percept_count = events
        .iter()
        .filter(|e| e.kind == EventKind::Percept)
        .count();
    assert_eq!(percept_count, 32);

    rt.shutdown().await;
}

#[tokio::test]
async fn next_due_timer_fires_tick_and_noop_tick_records_nothing() {
    let t0 = 1_000_000_u64;
    let clock = Arc::new(VirtualClock::new(Time::from_millis(t0)));
    let secrets = ProviderSecrets::default();
    let cfg = default_test_config(ModelProviderKind::Scripted);

    let expect_reply = json!({
        "protocol_version": { "major": 1, "minor": 0 },
        "acts": [
            {
                "act": "expect",
                "expect_kind": "monitor",
                "about_ids": [1],
                "places": ["room:timer"],
                "people": ["per_alice"],
                "condition": "follow up on report",
                "due_ms": t0 + 500,
                "expiry_ms": t0 + 5_000,
                "recurrence": {
                    "schedule": { "interval_ms": 5000 },
                    "catchup": "coalesce_one"
                },
                "promise_meta": null,
                "domain": null
            }
        ]
    })
    .to_string();
    let followup_reply = json!({
        "protocol_version": { "major": 1, "minor": 0 },
        "acts": [
            {
                "act": "think",
                "thought": "Monitor fired on tick.",
                "motivation": 3,
                "confidence_0_20": 15,
                "used_ids": [],
                "importance": 0.5,
                "basis": "observed",
                "value": null,
                "cost": null,
                "domain": null
            }
        ]
    })
    .to_string();

    let scripted = ScriptedModelProvider::from_replies([
        ScriptedReply::text(expect_reply),
        ScriptedReply::text(followup_reply),
    ]);

    let rt = GatewayRuntime::start(
        cfg,
        &secrets,
        || Ok(MemStore::new()),
        CoreConfig::default(),
        NoopDoors,
        GatewayOptions {
            clock: clock.clone(),
            provider: Some(Arc::new(scripted)),
            diagnostics: DiagnosticSink::new(&secrets),
            cues: CueRecorder::new(),
            seams: GatewaySeams::default(),
        },
    )
    .await
    .expect("start gateway");

    let handle = rt.handle();

    // No-op tick when nothing is due appends zero events (only initial config_loaded exists).
    handle
        .submit_and_wait(Input::Tick)
        .await
        .expect("noop tick");
    let before = handle.read_events().await.expect("read events");
    assert!(
        before.iter().all(|e| e.kind == EventKind::ConfigLoaded),
        "no-op tick must append nothing: {before:?}"
    );

    // Percept wakes model -> opens expectation with due_ms = t0 + 500.
    let p = make_percept(
        "per_alice",
        "room:timer",
        &["per_alice", "soul"],
        t0,
        b"remind me",
    );
    handle
        .submit_and_wait(Input::Percept(Box::new(p)))
        .await
        .expect("percept turn");

    poll_until(|| {
        let h = handle.clone();
        async move { h.next_due().await.ok().flatten() == Some(Time::from_millis(t0 + 500)) }
    })
    .await;

    // Early tick before t0 + 500 must also append no Tick event.
    clock.advance_to(Time::from_millis(t0 + 200));
    let count_before_early_tick = handle.read_events().await.unwrap().len();
    handle
        .submit_and_wait(Input::Tick)
        .await
        .expect("early tick");
    let count_after_early_tick = handle.read_events().await.unwrap().len();
    assert_eq!(
        count_before_early_tick, count_after_early_tick,
        "early tick before next_due must record nothing"
    );

    // Advance virtual clock to t0 + 500: owner's next_due() timer injects Tick automatically!
    clock.advance_to(Time::from_millis(t0 + 500));

    poll_until(|| {
        let h = handle.clone();
        async move {
            let events = h.read_events().await.unwrap();
            let has_tick = events.iter().any(|e| e.kind == EventKind::Tick);
            let has_transition = events.iter().any(|e| {
                e.kind == EventKind::ExpectationChanged
                    && matches!(&e.detail, Some(EventDetail::MonitorFired { .. }))
            });
            has_tick && has_transition
        }
    })
    .await;

    rt.shutdown().await;
}

#[tokio::test]
async fn crash_and_timeout_settle_once_as_unknown_while_sibling_runs_and_late_receipt_rejected() {
    let t0 = 1_000_000_u64;
    let clock = Arc::new(VirtualClock::new(Time::from_millis(t0)));
    let secrets = ProviderSecrets::default();
    let mut cfg = default_test_config(ModelProviderKind::Scripted);
    cfg.runtime.action_timeout_ms = 500;

    // Reply 1: act on place:crash (body crashes mid-action)
    // Reply 2: act on place:sibling (body answers Done)
    // Reply 3: act on place:hang (body hangs past 500ms timeout, then sends late receipt)
    let act_crash = json!({
        "acts": [{
            "act": "act",
            "capability_id": "crash.op",
            "places": ["place:crash"],
            "people": ["per_alice"],
            "payload": [103, 111],
            "motivation": 4,
            "confidence_0_20": 16,
            "used_ids": [],
            "expected": null,
            "domain": null
        }]
    })
    .to_string();
    let act_sibling = json!({
        "acts": [{
            "act": "act",
            "capability_id": "sib.op",
            "places": ["place:sibling"],
            "people": ["per_alice"],
            "payload": [111, 107],
            "motivation": 4,
            "confidence_0_20": 16,
            "used_ids": [],
            "expected": null,
            "domain": null
        }]
    })
    .to_string();
    let act_hang = json!({
        "acts": [{
            "act": "act",
            "capability_id": "hang.op",
            "places": ["place:hang"],
            "people": ["per_alice"],
            "payload": [119, 97, 105, 116],
            "motivation": 4,
            "confidence_0_20": 16,
            "used_ids": [],
            "expected": null,
            "domain": null
        }]
    })
    .to_string();

    let scripted = ScriptedModelProvider::from_replies([
        ScriptedReply::text(act_crash),
        ScriptedReply::text(act_sibling),
        ScriptedReply::text(act_hang),
    ]);

    let diagnostics = DiagnosticSink::new(&secrets);
    let rt = GatewayRuntime::start(
        cfg,
        &secrets,
        || Ok(MemStore::new()),
        CoreConfig::default(),
        NoopDoors,
        GatewayOptions {
            clock: clock.clone(),
            provider: Some(Arc::new(scripted)),
            diagnostics: diagnostics.clone(),
            cues: CueRecorder::new(),
            seams: GatewaySeams::default(),
        },
    )
    .await
    .expect("start gateway");

    let handle = rt.handle();

    let (_crash_task, crash_ctrl) = spawn_ndjson_mock_body(
        &handle,
        "body-crash",
        "place:crash",
        "crash.op",
        MockBodyBehavior::CrashMidAction,
    );
    let _crash_tx = crash_ctrl.wait_connected().await;

    let (_sib_task, sib_ctrl) = spawn_ndjson_mock_body(
        &handle,
        "body-sibling",
        "place:sibling",
        "sib.op",
        MockBodyBehavior::ImmediateDone,
    );
    let _sib_tx = sib_ctrl.wait_connected().await;

    let (hang_task, hang_ctrl) = spawn_ndjson_mock_body(
        &handle,
        "body-hang",
        "place:hang",
        "hang.op",
        MockBodyBehavior::HangPastTimeout,
    );
    let hang_tx = hang_ctrl.wait_connected().await;

    // 1. Trigger crashing body -> settles ONCE as Unknown, body removed from routing, failure logged.
    let p1 = make_percept(
        "per_alice",
        "place:crash",
        &["per_alice", "soul"],
        t0,
        b"crash",
    );
    handle
        .submit_and_wait(Input::Percept(Box::new(p1)))
        .await
        .expect("p1");

    poll_until(|| {
        let h = handle.clone();
        async move {
            let events = h.read_events().await.unwrap();
            events.iter().any(|e| {
                matches!(
                    &e.detail,
                    Some(EventDetail::Receipt {
                        capability_id,
                        outcome: ReceiptOutcome::Unknown,
                        ..
                    }) if capability_id.as_str() == "crash.op"
                )
            })
        }
    })
    .await;

    assert!(
        !handle
            .router()
            .registered_bodies()
            .contains(&BodyId::new("body-crash")),
        "crashed body must be removed from routing"
    );
    assert!(
        diagnostics
            .records()
            .iter()
            .any(|r| r.message.contains("body-crash")),
        "crash failure must be logged for owner"
    );

    // 2. Sibling body continues operating normally and answers Done.
    let p2 = make_percept(
        "per_alice",
        "place:sibling",
        &["per_alice", "soul"],
        t0 + 10,
        b"sibling",
    );
    handle
        .submit_and_wait(Input::Percept(Box::new(p2)))
        .await
        .expect("p2");

    poll_until(|| {
        let h = handle.clone();
        async move {
            let events = h.read_events().await.unwrap();
            events.iter().any(|e| {
                matches!(
                    &e.detail,
                    Some(EventDetail::Receipt {
                        capability_id,
                        outcome: ReceiptOutcome::Done,
                        ..
                    }) if capability_id.as_str() == "sib.op"
                )
            })
        }
    })
    .await;

    // 3. Hanging body exceeds action_timeout_ms (500ms) -> settles ONCE as Unknown + TimedOut.
    let p3 = make_percept(
        "per_alice",
        "place:hang",
        &["per_alice", "soul"],
        t0 + 20,
        b"hang",
    );
    handle
        .submit_and_wait(Input::Percept(Box::new(p3)))
        .await
        .expect("p3");

    poll_until(|| {
        let h = handle.clone();
        async move {
            let events = h.read_events().await.unwrap();
            events.iter().any(|e| {
                matches!(
                    &e.detail,
                    Some(EventDetail::Action { capability_id, .. }) if capability_id.as_str() == "hang.op"
                )
            })
        }
    })
    .await;

    clock.advance_by(600);

    let hang_action_id = loop {
        let events = handle.read_events().await.unwrap();
        if let Some(aid) = events.iter().find_map(|e| match &e.detail {
            Some(EventDetail::Receipt {
                action_id,
                capability_id,
                outcome: ReceiptOutcome::Unknown,
                termination_reason: Some(TerminationReason::TimedOut),
                ..
            }) if capability_id.as_str() == "hang.op" => Some(action_id.clone()),
            _ => None,
        }) {
            break aid;
        }
        tokio::task::yield_now().await;
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    };

    // 4. Send a duplicate late receipt for the timed-out action -> MUST be rejected/ignored!
    mpsc_tx_handle::BodyControl::send_receipt(
        &hang_tx,
        9001,
        Receipt {
            action_id: hang_action_id.clone(),
            capability_id: CapabilityId::new("hang.op"),
            outcome: ReceiptOutcome::Done,
            reason_code: None,
            detail: Some("too late".to_owned()),
            termination_reason: Some(TerminationReason::Normal),
            progress: Some(1.0),
            delivered_units: Some(1),
            h_act: None,
            timestamp_ms: clock.now(),
        },
    );

    poll_until(|| {
        let d = diagnostics.clone();
        async move {
            d.records().iter().any(|r| {
                r.message
                    .contains("ignored duplicate or post-timeout receipt")
            })
        }
    })
    .await;

    let final_events = handle.read_events().await.unwrap();
    let hang_receipt_count = final_events
        .iter()
        .filter(|e| {
            matches!(
                &e.detail,
                Some(EventDetail::Receipt { action_id, .. }) if action_id == &hang_action_id
            )
        })
        .count();
    assert_eq!(
        hang_receipt_count, 1,
        "timed-out action must settle with exactly ONE receipt"
    );

    rt.shutdown().await;
    let _ = hang_task.await;
}

/// Blocking model provider used to prove slow wakes never stall ingress.
struct BlockingModelProvider {
    entered: Arc<Notify>,
    release: Arc<Notify>,
}

impl ModelProvider for BlockingModelProvider {
    fn provider_name(&self) -> &str {
        "blocking_test"
    }

    fn invoke(&self, request: &ModelRequest) -> Result<ModelCallSuccess, ModelCallFailure> {
        self.entered.notify_one();
        let rt = tokio::runtime::Handle::current();
        rt.block_on(self.release.notified());
        Ok(ModelCallSuccess {
            record: ModelCallRecord {
                request_id: request.request_id,
                purpose: request.purpose,
                provider: "blocking_test".to_owned(),
                model: request.model.clone(),
                usage: None,
                latency_ms: 50,
                outcome: CallOutcome::Success,
                strict_schema: true,
            },
            output: monoagens_reason::ModelOutput::ThoughtActs(monoagens_reason::ParsedModelActs {
                protocol_version: CURRENT_PROTOCOL_VERSION,
                acts: vec![monoagens_protocol::ThoughtAct::Think {
                    thought: "Finally finished.".to_owned(),
                    motivation: 3,
                    confidence_0_20: 14,
                    used_ids: vec![],
                    importance: Some(0.4),
                    basis: None,
                    value: None,
                    cost: None,
                    domain: None,
                }],
                rubrics: vec![None],
                fallback_plain_text: false,
            }),
            raw_text: "Finally finished.".to_owned(),
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn slow_model_call_does_not_block_percept_ingress() {
    let clock = Arc::new(VirtualClock::new(Time::from_millis(1_000_000)));
    let secrets = ProviderSecrets::default();
    let cfg = default_test_config(ModelProviderKind::Scripted);

    let entered = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let provider = Arc::new(BlockingModelProvider {
        entered: Arc::clone(&entered),
        release: Arc::clone(&release),
    });

    let rt = GatewayRuntime::start(
        cfg,
        &secrets,
        || Ok(MemStore::new()),
        CoreConfig::default(),
        NoopDoors,
        GatewayOptions {
            clock: clock.clone(),
            provider: Some(provider),
            diagnostics: DiagnosticSink::new(&secrets),
            cues: CueRecorder::new(),
            seams: GatewaySeams::default(),
        },
    )
    .await
    .expect("start gateway");

    let handle = rt.handle();

    // Percept 1 triggers a slow blocking model wake.
    let p1 = make_percept(
        "per_alice",
        "room:main",
        &["per_alice", "soul"],
        1_000_000,
        b"first",
    );
    handle
        .submit_and_wait(Input::Percept(Box::new(p1)))
        .await
        .expect("p1");
    entered.notified().await;

    // While model call 1 is blocked, submit 3 more percepts and verify they are recorded BEFORE release!
    for i in 2..=4 {
        let mut p = make_percept(
            "per_bob",
            "room:quiet",
            &["per_bob", "soul"],
            1_000_000 + i,
            format!("urgent-{i}").as_bytes(),
        );
        // Empty addressees so only recorded without spawning another blocked wake
        p.addressees.clear();
        handle
            .submit_and_wait(Input::Percept(Box::new(p)))
            .await
            .expect("percept during model call");
    }

    let events_while_blocked = handle.read_events().await.unwrap();
    let percepts_recorded = events_while_blocked
        .iter()
        .filter(|e| e.kind == EventKind::Percept)
        .count();
    assert_eq!(
        percepts_recorded, 4,
        "all 3 subsequent percepts must be recorded while model call is in flight"
    );
    assert!(
        !events_while_blocked
            .iter()
            .any(|e| e.kind == EventKind::Thought),
        "model call has not returned yet"
    );

    // Release slow model call -> ModelResult enters via ingress queue and records Thought.
    release.notify_waiters();
    poll_until(|| {
        let h = handle.clone();
        async move {
            h.read_events()
                .await
                .unwrap()
                .iter()
                .any(|e| e.kind == EventKind::Thought)
        }
    })
    .await;

    rt.shutdown().await;
}

#[test]
fn config_first_run_precedence_version_range_and_inline_secret_checks() {
    let tmp = TempDir::new().expect("tempdir");

    // 1. Precedence: --config > MONOAGENS_HOME > XDG_CONFIG_HOME > HOME/.config
    let cli_arg = extract_cli_config_arg(["monoagens", "--config", "/tmp/custom.toml"])
        .expect("cli arg")
        .expect("path");
    let mut env = BTreeMap::new();
    env.insert("MONOAGENS_HOME".to_owned(), "/tmp/mh".to_owned());
    env.insert("XDG_CONFIG_HOME".to_owned(), "/tmp/xdg".to_owned());
    env.insert("HOME".to_owned(), "/tmp/home".to_owned());

    let p_cli = resolve_paths_with(Some(&cli_arg), |k| env.get(k).cloned()).unwrap();
    assert_eq!(p_cli.config_path, Path::new("/tmp/custom.toml"));

    let p_mh = resolve_paths_with(None, |k| env.get(k).cloned()).unwrap();
    assert_eq!(p_mh.config_path, Path::new("/tmp/mh/monoagens.toml"));

    env.remove("MONOAGENS_HOME");
    let p_xdg = resolve_paths_with(None, |k| env.get(k).cloned()).unwrap();
    assert_eq!(
        p_xdg.config_path,
        Path::new("/tmp/xdg/monoagens/monoagens.toml")
    );

    env.remove("XDG_CONFIG_HOME");
    let p_home = resolve_paths_with(None, |k| env.get(k).cloned()).unwrap();
    assert_eq!(
        p_home.config_path,
        Path::new("/tmp/home/.config/monoagens/monoagens.toml")
    );

    // 2. First run creates default config without secrets and ensures startup fails non-zero.
    let first_run_home = tmp.path().join("first_run");
    let mut first_env = BTreeMap::new();
    first_env.insert(
        "MONOAGENS_HOME".to_owned(),
        first_run_home.display().to_string(),
    );
    let loaded =
        load_or_init_gateway_env(None, |k| first_env.get(k).cloned()).expect("first run bootstrap");
    assert!(loaded.created_default);
    assert!(loaded.paths.config_path.exists());
    let created_raw = std::fs::read_to_string(&loaded.paths.config_path).unwrap();
    assert!(created_raw.contains("version = 1"));
    assert!(!created_raw.contains("api_key"));
    assert!(!created_raw.contains("sk-"));
    let startup_err = loaded
        .ensure_startup_ready()
        .expect_err("first run must abort startup");
    assert!(matches!(startup_err, ConfigError::FirstRunCreated { .. }));

    // 3. Unknown keys tolerated + toml_edit update creates .bak backup preserving comments/unknown keys.
    let cfg_path = tmp.path().join("monoagens.toml");
    let sec_path = tmp.path().join("secrets.toml");
    std::fs::write(
        &cfg_path,
        "# user comment\nversion = 1\nfuture_unknown_flag = true\n\n[model]\nprovider = \"scripted\"\nmodel = \"mock-1\"\n",
    )
    .unwrap();
    let parsed = load_config_file(&cfg_path, &sec_path).expect("unknown keys tolerated");
    assert_eq!(parsed.model.provider, ModelProviderKind::Scripted);

    update_config_with_backup(&cfg_path, &sec_path, |doc| {
        doc["soul"]["name"] = toml_edit::value("UpdatedSoul");
        Ok(())
    })
    .expect("toml_edit update with backup");
    assert!(tmp.path().join("monoagens.toml.bak").exists());
    let updated_raw = std::fs::read_to_string(&cfg_path).unwrap();
    assert!(updated_raw.contains("# user comment"));
    assert!(updated_raw.contains("future_unknown_flag = true"));
    assert!(updated_raw.contains("UpdatedSoul"));

    // 4. Version 2 rejected naming both versions (2 and 1).
    let ver_err =
        parse_config_str("version = 2\n", &cfg_path, &sec_path).expect_err("version 2 must fail");
    let ver_msg = ver_err.to_string();
    assert!(
        ver_msg.contains('2') && ver_msg.contains('1'),
        "msg: {ver_msg}"
    );

    // 5. Out-of-range value rejected naming key and range.
    let range_err = parse_config_str(
        "version = 1\n[runtime]\naction_timeout_ms = 0\n",
        &cfg_path,
        &sec_path,
    )
    .expect_err("out of range must fail");
    let range_msg = range_err.to_string();
    assert!(
        range_msg.contains("runtime.action_timeout_ms") && range_msg.contains("1..=600000"),
        "msg: {range_msg}"
    );

    // 6. Inline key in monoagens.toml aborts startup and names where secrets belong.
    let inline_err = parse_config_str(
        "version = 1\n[model]\nprovider = \"openai_compatible\"\napi_key = \"sk-canary-inline-00001\"\n",
        &cfg_path,
        &sec_path,
    )
    .expect_err("inline key must abort");
    assert!(matches!(
        inline_err,
        ConfigError::Secrets(SecretsLoadError::InlineSecretInConfig { .. })
    ));
    let inline_msg = inline_err.to_string();
    assert!(
        inline_msg.contains("secrets.toml") && inline_msg.contains("environment variables"),
        "msg: {inline_msg}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mock_http_provider_sends_store_false_tracks_usage_and_redacts_canary_on_429_and_500() {
    let canary = "canary-secret-token-xyz-998877";
    let tmp = TempDir::new().expect("tempdir");
    let cfg_path = tmp.path().join("monoagens.toml");
    let sec_path = tmp.path().join("secrets.toml");

    std::fs::write(
        &cfg_path,
        "version = 1\n[model]\nprovider = \"openai_compatible\"\nmodel = \"mock-gpt\"\ndaily_token_budget = 1000\n",
    )
    .unwrap();

    // Mock server: 1) 200 OK with cached tokens, 2) 429 echoing canary, 3) 500 echoing canary.
    let ok_body = json!({
        "id": "cmpl-1",
        "model": "mock-gpt",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": json!({
                    "protocol_version": { "major": 1, "minor": 0 },
                    "acts": [{
                        "act": "think",
                        "thought": "HTTP OK",
                        "motivation": 3,
                        "confidence_0_20": 15,
                        "used_ids": [],
                        "importance": 0.5,
                        "basis": "observed",
                        "value": null,
                        "cost": null,
                        "domain": null
                    }]
                }).to_string(),
                "refusal": null
            }
        }],
        "usage": {
            "prompt_tokens": 80,
            "completion_tokens": 20,
            "total_tokens": 100,
            "prompt_tokens_details": { "cached_tokens": 32 }
        }
    })
    .to_string();

    let (endpoint, captured_bodies, server_join) = spawn_mock_http_server(vec![
        MockHttpReply {
            status_line: "200 OK",
            body: ok_body,
        },
        MockHttpReply {
            status_line: "429 Too Many Requests",
            body: format!("rate limited for token {canary}"),
        },
        MockHttpReply {
            status_line: "500 Internal Server Error",
            body: format!("upstream crash near Bearer {canary}"),
        },
    ]);

    std::fs::write(
        &sec_path,
        format!("[model]\napi_key = \"{canary}\"\nendpoint = \"{endpoint}\"\n"),
    )
    .unwrap();

    let env = load_or_init_gateway_env(Some(&cfg_path), |_| None).expect("load env");
    env.ensure_startup_ready().expect("ready");

    let clock = Arc::new(VirtualClock::new(Time::from_millis(1_000_000)));
    let diagnostics = DiagnosticSink::new(&env.secrets);
    let rt = GatewayRuntime::start(
        env.config,
        &env.secrets,
        || Ok(MemStore::new()),
        CoreConfig::default(),
        NoopDoors,
        GatewayOptions {
            clock: clock.clone(),
            provider: None,
            diagnostics: diagnostics.clone(),
            cues: CueRecorder::new(),
            seams: GatewaySeams::default(),
        },
    )
    .await
    .expect("start gateway");

    let handle = rt.handle();
    for i in 0..3_u64 {
        let p = make_percept(
            "per_alice",
            "room:http",
            &["per_alice", "soul"],
            1_000_000 + i,
            format!("turn-{i}").as_bytes(),
        );
        handle
            .submit_and_wait(Input::Percept(Box::new(p)))
            .await
            .expect("submit percept");
        // `submit_and_wait` returns before the model call finishes: wait for this call's
        // record so the mock server sees the three calls in submit order.
        let want = usize::try_from(i).expect("small index") + 1;
        poll_until(|| {
            let d = diagnostics.clone();
            async move { d.model_calls().len() == want }
        })
        .await;
    }

    // Wait for both ModelFailure events to hit Core's log as well.
    poll_until(|| {
        let h = handle.clone();
        async move {
            h.read_events()
                .await
                .unwrap()
                .iter()
                .filter(|e| e.kind == EventKind::Failure)
                .count()
                == 2
        }
    })
    .await;

    // Verify store: false in captured HTTP request bodies.
    let bodies = captured_bodies.lock().unwrap().clone();
    assert_eq!(bodies.len(), 3);
    for raw in &bodies {
        let v: Value = serde_json::from_str(raw).unwrap();
        assert_eq!(v["store"], Value::Bool(false));
    }

    // Verify call audit records (model, purpose, cached tokens, outcomes) & daily budget.
    let calls = diagnostics.model_calls();
    assert_eq!(calls[0].outcome, CallOutcome::Success);
    assert_eq!(calls[0].purpose, ModelPurpose::Reason);
    assert_eq!(calls[0].model, "mock-gpt");
    assert_eq!(calls[0].usage.unwrap().cached_tokens, Some(32));
    assert_eq!(
        calls[1].outcome,
        CallOutcome::Failed(ProviderFailureClass::RateLimit)
    );
    assert_eq!(
        calls[2].outcome,
        CallOutcome::Failed(ProviderFailureClass::Server)
    );
    assert_eq!(handle.budget().snapshot(clock.now()).0, 100);

    // Canary secret MUST NOT appear in ANY diagnostic record, model call record, or Core event.
    for rec in diagnostics.records() {
        assert!(
            !rec.message.contains(canary),
            "canary leaked into diagnostic log: {}",
            rec.message
        );
    }
    let events = handle.read_events().await.unwrap();
    for ev in &events {
        let payload_str = String::from_utf8_lossy(ev.payload.as_bytes());
        assert!(
            !payload_str.contains(canary),
            "canary leaked into event payload: {payload_str}"
        );
        if let Some(EventDetail::Failure { detail, .. }) = &ev.detail {
            assert!(
                !detail.contains(canary),
                "canary leaked into Failure detail: {detail}"
            );
        }
    }

    rt.shutdown().await;
    server_join.join().expect("mock server join");
}

#[tokio::test]
async fn scripted_provider_drives_full_loop_offline_via_body() {
    let t0 = 2_000_000_u64;
    let clock = Arc::new(VirtualClock::new(Time::from_millis(t0)));
    let secrets = ProviderSecrets::default();
    let cfg = default_test_config(ModelProviderKind::Scripted);

    let act_reply = json!({
        "protocol_version": { "major": 1, "minor": 0 },
        "acts": [
            {
                "act": "think",
                "thought": "Reply to Alice in room:chat",
                "motivation": 4,
                "confidence_0_20": 18,
                "used_ids": [],
                "importance": 0.6,
                "basis": "observed",
                "value": null,
                "cost": null,
                "domain": null
            },
            {
                "act": "act",
                "capability_id": "chat.speak",
                "places": ["room:chat"],
                "people": ["per_alice"],
                "payload": [104, 105],
                "motivation": 4,
                "confidence_0_20": 18,
                "used_ids": [],
                "expected": null,
                "domain": null
            }
        ]
    })
    .to_string();

    let scripted = ScriptedModelProvider::from_replies([ScriptedReply::text(act_reply)]);

    let rt = GatewayRuntime::start(
        cfg,
        &secrets,
        || Ok(MemStore::new()),
        CoreConfig::default(),
        NoopDoors,
        GatewayOptions {
            clock: clock.clone(),
            provider: Some(Arc::new(scripted)),
            diagnostics: DiagnosticSink::new(&secrets),
            cues: CueRecorder::new(),
            seams: GatewaySeams::default(),
        },
    )
    .await
    .expect("start gateway");

    let handle = rt.handle();
    let (body_task, body_ctrl) = spawn_ndjson_mock_body(
        &handle,
        "body-chat",
        "room:chat",
        "chat.speak",
        MockBodyBehavior::ImmediateDone,
    );
    let body_tx = body_ctrl.wait_connected().await;

    // Emit percept from the Waist-2 body over NDJSON.
    let p = make_percept(
        "per_alice",
        "room:chat",
        &["per_alice", "soul"],
        t0,
        b"hello",
    );
    mpsc_tx_handle::BodyControl::send_percept(&body_tx, 1, p);

    poll_until(|| {
        let h = handle.clone();
        async move {
            h.read_events().await.unwrap().iter().any(|e| {
                matches!(
                    &e.detail,
                    Some(EventDetail::Receipt {
                        capability_id,
                        outcome: ReceiptOutcome::Done,
                        ..
                    }) if capability_id.as_str() == "chat.speak"
                )
            })
        }
    })
    .await;

    rt.shutdown().await;
    let executed = body_task.await.expect("body task");
    assert_eq!(executed.len(), 1);
    assert_eq!(executed[0].capability_id, CapabilityId::new("chat.speak"));
}

#[tokio::test]
async fn lifecycle_cues_respect_1s_and_10s_boundaries_with_virtual_clock() {
    let clock: Arc<VirtualClock> = Arc::new(VirtualClock::new(Time::from_millis(10_000)));
    let recorder = CueRecorder::new();

    // 1. Fast path (< 1000 ms): zero cues emitted.
    let fast_guard = monoagens_gateway::spawn_wait_cue_monitor(
        clock.clone(),
        recorder.clone(),
        "fast-wait",
        None,
        1_000,
        10_000,
    );
    clock.advance_by(500);
    tokio::task::yield_now().await;
    fast_guard.finish();
    tokio::task::yield_now().await;
    assert!(
        recorder.events().is_empty(),
        "fast wait < 1s must emit zero cues"
    );

    // 2. Medium wait (> 1000 ms, < 10000 ms): working_cue at 1s, no progress_note.
    let med_guard = monoagens_gateway::spawn_wait_cue_monitor(
        clock.clone(),
        recorder.clone(),
        "med-wait",
        Some(PlaceId::new("room:cue")),
        1_000,
        10_000,
    );
    tokio::task::yield_now().await;
    clock.advance_by(999);
    tokio::task::yield_now().await;
    assert!(
        recorder.events().is_empty(),
        "999ms must not emit working_cue yet"
    );
    clock.advance_by(1);
    poll_until(|| {
        let r = recorder.clone();
        async move { r.events().len() == 1 }
    })
    .await;
    assert_eq!(recorder.events()[0].kind, LifecycleCueKind::WorkingCue);
    assert_eq!(recorder.events()[0].elapsed_ms, 1_000);
    med_guard.finish();
    clock.advance_by(10_000);
    tokio::task::yield_now().await;
    assert_eq!(
        recorder.events().len(),
        1,
        "finished wait must not emit progress_note"
    );

    // 3. Long wait (> 10000 ms): both working_cue (1s) and progress_note (10s).
    let long_guard = monoagens_gateway::spawn_wait_cue_monitor(
        clock.clone(),
        recorder.clone(),
        "long-wait",
        Some(PlaceId::new("room:cue")),
        1_000,
        10_000,
    );
    tokio::task::yield_now().await;
    clock.advance_by(1_000);
    poll_until(|| {
        let r = recorder.clone();
        async move { r.events().len() == 2 }
    })
    .await;
    assert_eq!(recorder.events()[1].kind, LifecycleCueKind::WorkingCue);
    assert_eq!(recorder.events()[1].elapsed_ms, 1_000);

    clock.advance_by(8_999);
    tokio::task::yield_now().await;
    assert_eq!(
        recorder.events().len(),
        2,
        "9999ms must not emit progress_note yet"
    );

    clock.advance_by(1);
    poll_until(|| {
        let r = recorder.clone();
        async move { r.events().len() == 3 }
    })
    .await;
    assert_eq!(recorder.events()[2].kind, LifecycleCueKind::ProgressNote);
    assert_eq!(recorder.events()[2].elapsed_ms, 10_000);
    long_guard.finish();
}

#[test]
fn ingress_backpressure_returns_typed_queue_overflow_without_silent_drop() {
    let q = IngressQueue::new();
    for idx in 0..monoagens_core::CLASS_A_CAPACITY {
        q.try_submit(Input::CancelAction {
            action_id: monoagens_protocol::ActionId::new(format!("act-{idx}")),
            termination_reason: TerminationReason::CanceledByCore,
            reason_code: None,
            detail: None,
        })
        .expect("within Class A capacity");
    }

    let err = q
        .try_submit(Input::CancelAction {
            action_id: monoagens_protocol::ActionId::new("overflow-act"),
            termination_reason: TerminationReason::CanceledByCore,
            reason_code: None,
            detail: None,
        })
        .expect_err("Class A overflow must return typed error");

    match err {
        GatewayError::QueueOverflow(ov) => {
            assert_eq!(ov.class, QueueClass::A);
            assert_eq!(ov.capacity, monoagens_core::CLASS_A_CAPACITY);
            assert_eq!(ov.current_len, monoagens_core::CLASS_A_CAPACITY);
        }
        other => panic!("expected QueueOverflow, got {other:?}"),
    }
    assert_eq!(q.class_len(QueueClass::A), monoagens_core::CLASS_A_CAPACITY);
}

struct HangingHook;

impl AsyncGatewayHook for HangingHook {
    fn name(&self) -> &str {
        "hanging_test_hook"
    }

    fn on_percept<'a>(
        &'a self,
        _percept: &'a Percept,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), String>> + Send + 'a>> {
        Box::pin(std::future::pending::<Result<(), String>>())
    }
}

#[tokio::test]
async fn hanging_hook_times_out_under_hook_timeout_ms_without_stalling_loop() {
    let t0 = 5_000_000_u64;
    let clock = Arc::new(VirtualClock::new(Time::from_millis(t0)));
    let secrets = ProviderSecrets::default();
    let mut cfg = default_test_config(ModelProviderKind::Scripted);
    cfg.runtime.hook_timeout_ms = 250;

    let diagnostics = DiagnosticSink::new(&secrets);
    let mut seams = GatewaySeams::default();
    seams.hooks.push(Arc::new(HangingHook));

    let rt = GatewayRuntime::start(
        cfg,
        &secrets,
        || Ok(MemStore::new()),
        CoreConfig::default(),
        NoopDoors,
        GatewayOptions {
            clock: clock.clone(),
            provider: Some(Arc::new(ScriptedModelProvider::new())),
            diagnostics: diagnostics.clone(),
            cues: CueRecorder::new(),
            seams,
        },
    )
    .await
    .expect("start gateway");

    let handle = rt.handle();
    let (_body_task, body_ctrl) = spawn_ndjson_mock_body(
        &handle,
        "body-hook",
        "room:hook",
        "chat.speak",
        MockBodyBehavior::ImmediateDone,
    );
    let body_tx = body_ctrl.wait_connected().await;

    let mut p = make_percept("per_alice", "room:hook", &["per_alice", "soul"], t0, b"hi");
    p.addressees.clear();
    mpsc_tx_handle::BodyControl::send_percept(&body_tx, 1, p);

    // Yield so the hook starts waiting, then advance clock past hook_timeout_ms (250ms).
    tokio::task::yield_now().await;
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    clock.advance_by(300);

    poll_until(|| {
        let d = diagnostics.clone();
        let h = handle.clone();
        async move {
            let has_timeout_log = d.records().iter().any(|r| {
                r.message.contains("hanging_test_hook") && r.message.contains("timed out")
            });
            let has_percept = h
                .read_events()
                .await
                .unwrap()
                .iter()
                .any(|e| e.kind == EventKind::Percept);
            has_timeout_log && has_percept
        }
    })
    .await;

    rt.shutdown().await;
}

#[tokio::test]
async fn soul_core_config_and_neural_model_load_with_blake3_and_reject_bad_inputs() {
    let dir = TempDir::new().expect("tempdir");
    let valid_cfg_path = dir.path().join("custom_core.json");
    std::fs::write(
        &valid_cfg_path,
        r#"{
          "format_version": 1,
          "certainty_bar": 0.72,
          "online_adaptation": false
        }"#,
    )
    .unwrap();

    // Also point [soul] model_path at the golden phase-1 safetensors fixture with verified BLAKE3.
    let fixture_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../core/tests/fixtures/neural_phase1_golden.safetensors");
    let model_bytes = std::fs::read(&fixture_path).expect("read golden fixture");
    let model_blake3_hex =
        monoagens_core::blake3::to_hex(&monoagens_core::blake3::hash(&model_bytes));

    let cfg_path = dir.path().join("monoagens.toml");
    let secrets_path = dir.path().join("secrets.toml");
    let toml = format!(
        "version = 1\n\n[soul]\nname = \"MonoTest\"\nseed = 1\ncore_config = \"{}\"\nmodel_path = \"{}\"\nexpected_blake3 = \"{}\"\nonline_adaptation = false\n\n[model]\nprovider = \"scripted\"\nmodel = \"scripted-test\"\ndaily_token_budget = 10000\n",
        valid_cfg_path.display(),
        fixture_path.display(),
        model_blake3_hex,
    );
    let parsed_cfg = monoagens_gateway::parse_config_str(&toml, &cfg_path, &secrets_path)
        .expect("parse [soul] core_config");
    assert_eq!(parsed_cfg.soul.core_config, Some(valid_cfg_path));

    let rt = GatewayRuntime::start_with_configured_core(
        parsed_cfg,
        &ProviderSecrets::default(),
        || Ok(MemStore::new()),
        NoopDoors,
        GatewayOptions {
            clock: Arc::new(VirtualClock::new(Time::from_millis(1_000))),
            provider: Some(Arc::new(ScriptedModelProvider::new())),
            diagnostics: DiagnosticSink::new(&ProviderSecrets::default()),
            cues: CueRecorder::new(),
            seams: GatewaySeams::default(),
        },
    )
    .await
    .expect("start gateway with configured core_config and safetensors model");

    let events = rt.handle().read_events().await.expect("read events");
    let loaded_ev = events
        .iter()
        .find(|e| e.kind == EventKind::ConfigLoaded)
        .expect("config_loaded event recorded");
    match &loaded_ev.detail {
        Some(monoagens_core::EventDetail::ConfigLoaded { config, .. }) => {
            assert!((config.certainty_bar - 0.72).abs() < 1e-12);
            assert!(!config.online_adaptation);
        }
        other => panic!("unexpected detail {other:?}"),
    }
    rt.shutdown().await;

    // BLAKE3 mismatch fails startup loudly.
    let mut bad_hash_cfg = default_test_config(ModelProviderKind::Scripted);
    bad_hash_cfg.soul.model_path = Some(fixture_path);
    bad_hash_cfg.soul.expected_blake3 = Some("0".repeat(64));
    let err_hash = match GatewayRuntime::start_with_configured_core(
        bad_hash_cfg,
        &ProviderSecrets::default(),
        || Ok(MemStore::new()),
        NoopDoors,
        GatewayOptions {
            clock: Arc::new(VirtualClock::new(Time::from_millis(1_000))),
            provider: Some(Arc::new(ScriptedModelProvider::new())),
            diagnostics: DiagnosticSink::new(&ProviderSecrets::default()),
            cues: CueRecorder::new(),
            seams: GatewaySeams::default(),
        },
    )
    .await
    {
        Ok(_) => panic!("mismatched BLAKE3 must fail"),
        Err(e) => e,
    };
    assert!(
        err_hash.to_string().contains("BLAKE3") || err_hash.to_string().contains("mismatch"),
        "err={err_hash}"
    );

    // Missing core_config path fails start-up naming the path.
    let mut bad_cfg = default_test_config(ModelProviderKind::Scripted);
    let missing_path = dir.path().join("missing_core_cfg.json");
    bad_cfg.soul.core_config = Some(missing_path);
    let err_missing = match GatewayRuntime::start_with_configured_core(
        bad_cfg,
        &ProviderSecrets::default(),
        || Ok(MemStore::new()),
        NoopDoors,
        GatewayOptions {
            clock: Arc::new(VirtualClock::new(Time::from_millis(1_000))),
            provider: Some(Arc::new(ScriptedModelProvider::new())),
            diagnostics: DiagnosticSink::new(&ProviderSecrets::default()),
            cues: CueRecorder::new(),
            seams: GatewaySeams::default(),
        },
    )
    .await
    {
        Ok(_) => panic!("missing core_config path must fail"),
        Err(e) => e,
    };
    assert!(
        err_missing.to_string().contains("missing_core_cfg.json"),
        "err={err_missing}"
    );

    // Unknown key fails start-up naming the unknown key.
    let unknown_path = dir.path().join("unknown_core_cfg.json");
    std::fs::write(
        &unknown_path,
        r#"{ "format_version": 1, "unknown_gw_key": 0.4 }"#,
    )
    .unwrap();
    let mut unknown_cfg = default_test_config(ModelProviderKind::Scripted);
    unknown_cfg.soul.core_config = Some(unknown_path);
    let err_unknown = match GatewayRuntime::start_with_configured_core(
        unknown_cfg,
        &ProviderSecrets::default(),
        || Ok(MemStore::new()),
        NoopDoors,
        GatewayOptions {
            clock: Arc::new(VirtualClock::new(Time::from_millis(1_000))),
            provider: Some(Arc::new(ScriptedModelProvider::new())),
            diagnostics: DiagnosticSink::new(&ProviderSecrets::default()),
            cues: CueRecorder::new(),
            seams: GatewaySeams::default(),
        },
    )
    .await
    {
        Ok(_) => panic!("unknown key must fail"),
        Err(e) => e,
    };
    assert!(
        err_unknown.to_string().contains("unknown_gw_key"),
        "err={err_unknown}"
    );

    // Out-of-range value fails start-up naming the invalid field.
    let range_path = dir.path().join("range_core_cfg.json");
    std::fs::write(
        &range_path,
        r#"{ "format_version": 1, "max_wait_ms": 0.0 }"#,
    )
    .unwrap();
    let mut range_cfg = default_test_config(ModelProviderKind::Scripted);
    range_cfg.soul.core_config = Some(range_path);
    let err_range = match GatewayRuntime::start_with_configured_core(
        range_cfg,
        &ProviderSecrets::default(),
        || Ok(MemStore::new()),
        NoopDoors,
        GatewayOptions {
            clock: Arc::new(VirtualClock::new(Time::from_millis(1_000))),
            provider: Some(Arc::new(ScriptedModelProvider::new())),
            diagnostics: DiagnosticSink::new(&ProviderSecrets::default()),
            cues: CueRecorder::new(),
            seams: GatewaySeams::default(),
        },
    )
    .await
    {
        Ok(_) => panic!("out of range must fail"),
        Err(e) => e,
    };
    assert!(
        err_range.to_string().contains("max_wait_ms"),
        "err={err_range}"
    );
}

#[test]
fn gateway_config_validates_protocol_headers_and_default_output_limits() {
    let base_cfg = monoagens_gateway::parse_config_str(
        r#"
version = 1
[model]
provider = "openai_compatible"
model = "neutral-model"
"#,
        std::path::Path::new("monoagens.toml"),
        std::path::Path::new("."),
    )
    .expect("valid default config");
    assert_eq!(base_cfg.model.output_limits.reason, 16_384);
    assert_eq!(base_cfg.model.output_limits.classify, 16_384);
    assert_eq!(base_cfg.model.output_limits.consolidate, 16_384);
    assert_eq!(base_cfg.model.output_limit_param, "max_completion_tokens");

    let bad = monoagens_gateway::parse_config_str(
        r#"
version = 1
[model.extra_headers]
Authorization = "Bearer bad"
"#,
        std::path::Path::new("monoagens.toml"),
        std::path::Path::new("."),
    )
    .expect_err("must reject protocol header");
    assert!(
        bad.to_string()
            .to_ascii_lowercase()
            .contains("authorization")
    );
}

#[tokio::test]
async fn gateway_sends_configured_headers_and_per_purpose_limits_via_mock_provider() {
    use monoagens_core::ContextPlan;
    use monoagens_gateway::{
        DailyBudgetTracker, GatewayModelExecutor, build_provider_from_config_async,
    };
    use monoagens_protocol::EventId;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    let thought = serde_json::json!({
        "acts": [{
            "act": "think",
            "thought": "gateway mock ok",
            "motivation": 3,
            "confidence_0_20": 15,
            "used_ids": []
        }]
    })
    .to_string();
    let classify_ok = serde_json::json!({
        "label": "matched",
        "score": 0.9,
        "rationale": null
    })
    .to_string();

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock");
    let addr = listener.local_addr().expect("addr");
    let captured: Arc<Mutex<Vec<(String, serde_json::Value)>>> = Arc::new(Mutex::new(Vec::new()));
    let cap_clone = Arc::clone(&captured);
    let handle = std::thread::spawn(move || {
        for idx in 0..4 {
            let (mut stream, _) = listener.accept().expect("accept");
            let mut reader = BufReader::new(stream.try_clone().expect("clone"));
            let mut hdrs = String::new();
            let mut content_len = 0_usize;
            loop {
                let mut line = String::new();
                let n = reader.read_line(&mut line).expect("read");
                if n == 0 || line == "\r\n" || line == "\n" {
                    break;
                }
                hdrs.push_str(&line);
                if let Some(rest) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    content_len = rest.trim().parse().unwrap_or(0);
                }
            }
            let mut body = vec![0_u8; content_len];
            if content_len > 0 {
                reader.read_exact(&mut body).expect("body");
            }
            let json_body: serde_json::Value = serde_json::from_slice(&body).expect("json");
            cap_clone.lock().expect("lock").push((hdrs, json_body));
            let content = if idx == 2 { &classify_ok } else { &thought };
            let resp_body = serde_json::json!({
                "model": "neutral-model",
                "choices": [{
                    "index": 0,
                    "finish_reason": "stop",
                    "message": { "role": "assistant", "content": content }
                }],
                "usage": { "prompt_tokens": 50, "completion_tokens": 10, "total_tokens": 60 }
            })
            .to_string();
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                resp_body.len(),
                resp_body
            );
            let _ = stream.write_all(resp.as_bytes());
            let _ = stream.flush();
        }
    });

    let cfg_text = format!(
        r#"
version = 1
[providers.custom.mock-gw]
base_url = "http://{addr}/v1"
[model]
provider = "mock-gw"
model = "neutral-model"
output_limit_param = "max_output_tokens"

[model.output_limits]
reason = 4096
classify = 1024
consolidate = 8192

[model.extra_headers]
x-conv-id = "{{conversation_id}}"
x-static-hdr = "secret-gw-hdr-val"
"#
    );
    let cfg = monoagens_gateway::parse_config_str(
        &cfg_text,
        std::path::Path::new("monoagens.toml"),
        std::path::Path::new("."),
    )
    .expect("valid config");

    let secrets = ProviderSecrets::default();
    let provider = build_provider_from_config_async(&cfg, &secrets)
        .await
        .expect("build provider");
    let diagnostics = DiagnosticSink::new(&secrets);
    let exec = GatewayModelExecutor::with_provider(
        provider,
        &cfg,
        DailyBudgetTracker::new(Time::from_millis(1_000), 100_000),
        diagnostics.clone(),
        CueRecorder::new(),
        Arc::new(VirtualClock::new(Time::from_millis(1_000))),
    );
    let store = MemStore::new();

    let plan_alpha = ContextPlan {
        trigger_event_id: Some(EventId::new(1)),
        place: Some(PlaceId::new("room:alpha")),
        ..ContextPlan::default()
    };
    let plan_beta = ContextPlan {
        trigger_event_id: Some(EventId::new(2)),
        place: Some(PlaceId::new("room:beta")),
        ..ContextPlan::default()
    };

    // 2 calls on room:alpha (Reason, Consolidate), 1 Classify on room:alpha, 1 Reason on room:beta
    let req1 = exec.prepare_request(ModelPurpose::Reason, &plan_alpha, &store);
    let req2 = exec.prepare_request(ModelPurpose::Consolidate, &plan_alpha, &store);
    let req3 = exec.prepare_request(ModelPurpose::Classify, &plan_alpha, &store);
    let req4 = exec.prepare_request(ModelPurpose::Reason, &plan_beta, &store);

    let _ = exec.execute_prepared(req1).await;
    let _ = exec.execute_prepared(req2).await;
    let _ = exec.execute_prepared(req3).await;
    let _ = exec.execute_prepared(req4).await;
    handle.join().expect("server finishes");

    let reqs = captured.lock().expect("captured");
    assert_eq!(reqs.len(), 4);
    let extract_hdr = |hdrs: &str, name: &str| -> String {
        hdrs.lines()
            .find_map(|l| {
                let lower = l.to_ascii_lowercase();
                lower
                    .strip_prefix(&format!("{name}:"))
                    .map(|v| v.trim().to_owned())
            })
            .expect("header present")
    };
    let alpha_1 = extract_hdr(&reqs[0].0, "x-conv-id");
    let alpha_2 = extract_hdr(&reqs[1].0, "x-conv-id");
    let beta_1 = extract_hdr(&reqs[3].0, "x-conv-id");
    assert_eq!(alpha_1, alpha_2, "same chat must share header value");
    assert_ne!(alpha_1, beta_1, "different chats must differ");
    assert_eq!(extract_hdr(&reqs[0].0, "x-static-hdr"), "secret-gw-hdr-val");

    assert_eq!(reqs[0].1["max_output_tokens"], serde_json::json!(4096));
    assert_eq!(reqs[1].1["max_output_tokens"], serde_json::json!(8192));
    assert_eq!(reqs[2].1["max_output_tokens"], serde_json::json!(1024));
    assert_eq!(reqs[3].1["max_output_tokens"], serde_json::json!(4096));

    // No header value logged in diagnostics
    let logs = diagnostics.records();
    for entry in logs {
        assert!(!entry.message.contains(&alpha_1));
        assert!(!entry.message.contains("secret-gw-hdr-val"));
    }
}

#[test]
fn end_to_end_approval_resolve_rejects_in_band_and_dispatches_attested_oob_once() {
    use monoagens_gateway::ValveDoorController;
    use monoagens_protocol::{ApprovalResolveParams, ApprovalResolveResult, Waist2Headers};
    use monoagens_valve::{
        Valve, ValveConfig, ValveSnapshot, derive_approval_mac_key, sign_approval_resolution,
    };

    let owner = ActorId::new("per_owner");
    let _place = PlaceId::new("room:ops");
    let mac_key = derive_approval_mac_key(b"gw-oob-test-secret-01");
    let clock = Arc::new(VirtualClock::new(Time::from_millis(5_000_000)));

    let mut snap = ValveSnapshot::new(monoagens_core::LogSeq::ZERO, clock.now());
    snap.owner_actor_id = Some(owner.clone());
    snap.known_actors.insert(owner.clone());
    snap.known_actors.insert(ActorId::new("soul"));
    snap.active_context_audience = Some(Audience::new([owner.clone(), ActorId::new("soul")]));
    snap.capability_risk_metadata.insert(
        CapabilityId::new("wallet.pay"),
        monoagens_valve::ActionRiskMetadata {
            is_financial_or_credential: true,
            ..Default::default()
        },
    );
    let mut valve = Valve::with_default_policies(ValveConfig::default(), snap).expect("valve");
    valve.set_approval_attestation_key(mac_key);

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    rt.block_on(async move {
        let scripted = Arc::new(ScriptedModelProvider::from_replies([ScriptedReply::text(
            json!({
                "acts": [{
                    "act": "act",
                    "capability_id": "wallet.pay",
                    "places": ["room:ops"],
                    "people": ["per_owner"],
                    "payload": [112, 97, 121],
                    "motivation": 4,
                    "confidence_0_20": 18,
                    "used_ids": [],
                    "expected": null,
                    "domain": null
                }]
            })
            .to_string(),
        )]));

        let cfg = default_test_config(ModelProviderKind::Scripted);
        let core_cfg = CoreConfig {
            seed: 21,
            ..CoreConfig::default()
        };

        let runtime = GatewayRuntime::start(
            cfg,
            &ProviderSecrets::default(),
            || Ok(MemStore::new()),
            core_cfg,
            ValveDoorController::new(valve),
            GatewayOptions {
                clock: clock.clone(),
                provider: Some(scripted),
                diagnostics: DiagnosticSink::default(),
                cues: CueRecorder::default(),
                seams: GatewaySeams::default(),
            },
        )
        .await
        .expect("start gateway");

        let handle = runtime.handle();
        let (executed_join, body_ctrl) = spawn_ndjson_mock_body(
            &handle,
            "body-chat",
            "room:ops",
            "wallet.pay",
            MockBodyBehavior::ImmediateDone,
        );
        let body_tx = body_ctrl.wait_connected().await;
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;

        // Trigger model wake -> wallet.pay (financial R2) -> Held as PendingApproval
        let mut p = make_percept(
            "per_owner",
            "room:ops",
            &["per_owner", "soul"],
            5_000_000,
            b"pay 50",
        );
        p.taint = monoagens_protocol::Taint::Trusted;
        handle
            .submit_and_wait(Input::Percept(Box::new(p)))
            .await
            .expect("submit percept");

        let mut held_info = None;
        for _ in 0..40 {
            let events = handle.read_events().await.expect("read events");
            for ev in &events {
                if let Some(EventDetail::Receipt {
                    action_id,
                    outcome: ReceiptOutcome::Held,
                    h_act: Some(h_act),
                    ..
                }) = &ev.detail
                {
                    held_info = Some((action_id.clone(), h_act.clone()));
                }
            }
            if held_info.is_some() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(15)).await;
        }
        let (action_id, h_act) = match held_info {
            Some(info) => info,
            None => {
                let evs = handle.read_events().await.unwrap_or_default();
                panic!("action held with H_act missing; events={evs:#?}");
            }
        };

        // 1. In-band chat body (`body-chat`, NOT registered as OOB control) attempting `approval/resolve`
        //    over Waist-2 NDJSON MUST fail closed (NOT -32601, and NOT applied!).
        let valid_sig = sign_approval_resolution(
            &mac_key,
            &action_id,
            &h_act,
            0,
            true,
            Time::from_millis(5_001_000),
        );
        let inband_req = Waist2Request::new(
            Some(monoagens_protocol::RpcId::Int(999)),
            Waist2Method::ApprovalResolve,
            ApprovalResolveParams {
                headers: Waist2Headers::new("inband-spoof"),
                action_id: action_id.clone(),
                h_act: h_act.clone(),
                state_epoch: 0,
                approved: true,
                resolver: owner.clone(),
                resolved_at_ms: Time::from_millis(5_001_000),
                signature: Some(valid_sig.clone()),
                out_of_band_channel: true,
            },
        );
        let _ = body_tx.send(serde_json::to_string(&inband_req).unwrap());
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;

        let events_after_inband = handle.read_events().await.expect("events");
        assert!(events_after_inband.iter().any(|ev| matches!(
            &ev.detail,
            Some(EventDetail::DoorVerdict { rule, .. }) if rule == "valve.approval.resolve_rejected"
        )));
        assert!(!events_after_inband.iter().any(|ev| matches!(
            &ev.detail,
            Some(EventDetail::Receipt { outcome: ReceiptOutcome::Done, .. })
        )));

        // 2. Invalid signature on out-of-band handle fails closed
        let bad_oob = ApprovalResolveParams {
            headers: Waist2Headers::new("oob-bad-sig"),
            action_id: action_id.clone(),
            h_act: h_act.clone(),
            state_epoch: 0,
            approved: true,
            resolver: owner.clone(),
            resolved_at_ms: Time::from_millis(5_001_000),
            signature: Some(
                "blake3-mac:0000000000000000000000000000000000000000000000000000000000000000"
                    .to_owned(),
            ),
            out_of_band_channel: true,
        };
        let bad_res = handle.resolve_approval(bad_oob).await;
        assert!(bad_res.is_err());

        // 3. Valid attested out-of-band resolution persists grant, dispatches once, and settles Done
        clock.advance_by(1_000);
        let good_oob = ApprovalResolveParams {
            headers: Waist2Headers::new("oob-valid"),
            action_id: action_id.clone(),
            h_act: h_act.clone(),
            state_epoch: 0,
            approved: true,
            resolver: owner,
            resolved_at_ms: Time::from_millis(5_001_000),
            signature: Some(valid_sig),
            out_of_band_channel: true,
        };
        let ok_res: ApprovalResolveResult = handle
            .resolve_approval(good_oob.clone())
            .await
            .expect("attested OOB approval succeeds");
        assert!(ok_res.applied);

        let mut done_seen = false;
        for _ in 0..40 {
            let events = handle.read_events().await.expect("events");
            if events.iter().any(|ev| matches!(
                &ev.detail,
                Some(EventDetail::Receipt { action_id: aid, outcome: ReceiptOutcome::Done, .. }) if aid == &action_id
            )) {
                done_seen = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(15)).await;
        }
        assert!(done_seen, "expected Done receipt after OOB approval");

        // Replaying the same OOB resolution fails closed (single-use grant)
        assert!(handle.resolve_approval(good_oob).await.is_err());

        runtime.shutdown().await;
        let executed = executed_join.await.expect("body join");
        assert_eq!(executed.len(), 1, "action must dispatch at most once");
        assert_eq!(executed[0].action_id, action_id);
    });
}

#[test]
fn waist2_attested_oob_body_approval_resolve_deny_and_restart_replay() {
    use monoagens_gateway::ValveDoorController;
    use monoagens_protocol::{ApprovalResolveParams, Waist2Headers};
    use monoagens_valve::{
        Valve, ValveConfig, ValveSnapshot, derive_approval_mac_key, sign_approval_resolution,
    };

    let owner = ActorId::new("per_owner");
    let mac_key = derive_approval_mac_key(b"gw-oob-deny-secret");
    let clock = Arc::new(VirtualClock::new(Time::from_millis(6_000_000)));

    let mut snap = ValveSnapshot::new(monoagens_core::LogSeq::ZERO, clock.now());
    snap.owner_actor_id = Some(owner.clone());
    snap.known_actors.insert(owner.clone());
    snap.known_actors.insert(ActorId::new("soul"));
    snap.active_context_audience = Some(Audience::new([owner.clone(), ActorId::new("soul")]));
    snap.capability_risk_metadata.insert(
        CapabilityId::new("wallet.pay"),
        monoagens_valve::ActionRiskMetadata {
            is_financial_or_credential: true,
            ..Default::default()
        },
    );
    let mut valve = Valve::with_default_policies(ValveConfig::default(), snap).expect("valve");
    valve.set_approval_attestation_key(mac_key);

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    let owner_for_async = owner.clone();
    let (persisted, executed) = rt.block_on(async move {
        let owner = owner_for_async;
        let scripted = Arc::new(ScriptedModelProvider::from_replies([ScriptedReply::text(
            json!({
                "acts": [{
                    "act": "act",
                    "capability_id": "wallet.pay",
                    "places": ["room:ops"],
                    "people": ["per_owner"],
                    "payload": [100, 101, 110, 121],
                    "motivation": 4,
                    "confidence_0_20": 18,
                    "used_ids": [],
                    "expected": null,
                    "domain": null
                }]
            })
            .to_string(),
        )]));

        let cfg = default_test_config(ModelProviderKind::Scripted);
        let core_cfg = CoreConfig {
            seed: 29,
            ..CoreConfig::default()
        };

        let runtime = GatewayRuntime::start(
            cfg,
            &ProviderSecrets::default(),
            || Ok(MemStore::new()),
            core_cfg,
            ValveDoorController::new(valve),
            GatewayOptions {
                clock: clock.clone(),
                provider: Some(scripted),
                diagnostics: DiagnosticSink::default(),
                cues: CueRecorder::default(),
                seams: GatewaySeams::default(),
            },
        )
        .await
        .expect("start gateway");

        let handle = runtime.handle();
        let (executed_join, body_ctrl) = spawn_ndjson_mock_body(
            &handle,
            "oob-console",
            "room:ops",
            "wallet.pay",
            MockBodyBehavior::ImmediateDone,
        );
        let body_tx = body_ctrl.wait_connected().await;
        handle.register_out_of_band_control_body(BodyId::new("oob-console"));
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;

        let mut p = make_percept(
            "per_owner",
            "room:ops",
            &["per_owner", "soul"],
            6_000_000,
            b"pay 99",
        );
        p.taint = monoagens_protocol::Taint::Trusted;
        handle
            .submit_and_wait(Input::Percept(Box::new(p)))
            .await
            .expect("submit percept");

        let mut held_info = None;
        for _ in 0..40 {
            let events = handle.read_events().await.expect("events");
            for ev in &events {
                if let Some(EventDetail::Receipt {
                    action_id,
                    outcome: ReceiptOutcome::Held,
                    h_act: Some(h_act),
                    ..
                }) = &ev.detail
                {
                    held_info = Some((action_id.clone(), h_act.clone()));
                }
            }
            if held_info.is_some() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(15)).await;
        }
        let (action_id, h_act) = held_info.expect("held action");

        // Attested OOB control body sends Waist-2 NDJSON `approval/resolve` with `approved = false`
        let deny_ts = Time::from_millis(6_002_000);
        clock.advance_by(2_000);
        let deny_sig =
            sign_approval_resolution(&mac_key, &action_id, &h_act, 0, false, deny_ts);
        let deny_req = Waist2Request::new(
            Some(monoagens_protocol::RpcId::Int(55)),
            Waist2Method::ApprovalResolve,
            ApprovalResolveParams {
                headers: Waist2Headers::new("oob-deny-1"),
                action_id: action_id.clone(),
                h_act,
                state_epoch: 0,
                approved: false,
                resolver: owner.clone(),
                resolved_at_ms: deny_ts,
                signature: Some(deny_sig),
                out_of_band_channel: true,
            },
        );
        let _ = body_tx.send(serde_json::to_string(&deny_req).unwrap());

        let mut refused_seen = false;
        for _ in 0..40 {
            let events = handle.read_events().await.expect("events");
            if events.iter().any(|ev| matches!(
                &ev.detail,
                Some(EventDetail::Receipt { action_id: aid, outcome: ReceiptOutcome::Refused, .. }) if aid == &action_id
            )) {
                refused_seen = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(15)).await;
        }
        assert!(refused_seen, "expected Refused receipt after OOB denial");

        let persisted = handle.read_events().await.expect("events");
        runtime.shutdown().await;
        let executed = executed_join.await.expect("body join");
        (persisted, executed)
    });

    assert!(executed.is_empty(), "denied action must never dispatch");

    // Verify persisted events replay cleanly in a fresh Valve after restart
    let mut replayed_snap = ValveSnapshot::new(monoagens_core::LogSeq::ZERO, Time::from_millis(6_002_000));
    replayed_snap.owner_actor_id = Some(owner);
    let mut replayed_valve =
        Valve::with_default_policies(ValveConfig::default(), replayed_snap).expect("v");
    replayed_valve.set_approval_attestation_key(mac_key);
    replayed_valve.replay_from_events(&persisted);
    assert!(replayed_valve.approval_manager().pending().is_empty());
}
