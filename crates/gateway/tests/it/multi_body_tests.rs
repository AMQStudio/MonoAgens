use std::sync::Arc;

use monoagens_core::{CoreConfig, EventDetail, EventKind, MemStore, NoopDoors};
use monoagens_gateway::{
    BodyPluginConfig, BodyTransportConfig, CueRecorder, DiagnosticSink, DoorsAdapter,
    GatewayConfig, GatewayHandle, GatewayOptions, GatewayRuntime, GatewaySeams, LogLevel,
    ModelProviderKind, VirtualClock,
};
use monoagens_protocol::{
    Action, ActionExecuteParams, ActionExecuteResult, ActionId, ActorId, Audience,
    BodyDiscoverParams, BodyDiscoverResult, BodyId, CURRENT_PROTOCOL_VERSION,
    CapabilityDescriptor, CapabilityFlags, CapabilityId, CapabilitySchemaHash, OpaquePayload,
    OriginActor, Percept, PerceptEmitParams, PlaceId, Receipt, ReceiptOutcome, RpcId, Taint,
    TerminationReason, Time, Waist2Headers, Waist2Method, Waist2Request, Waist2Response,
};
use monoagens_reason::{ProviderSecrets, ScriptedModelProvider};
use serde_json::Value;
use tempfile::TempDir;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader as TokioBufReader};
use tokio::net::UnixListener;
use tokio::sync::mpsc;

fn test_gateway_config() -> GatewayConfig {
    let mut cfg = GatewayConfig::default();
    cfg.model.provider = ModelProviderKind::Scripted;
    cfg.model.model = "mock-gpt".to_owned();
    cfg.runtime.action_timeout_ms = 500;
    cfg.runtime.hook_timeout_ms = 500;
    cfg.runtime.model_timeout_ms = 5_000;
    cfg
}

async fn start_test_gateway() -> (GatewayRuntime, Arc<VirtualClock>, DiagnosticSink) {
    let clock = Arc::new(VirtualClock::new(Time::from_millis(1_000_000)));
    let secrets = ProviderSecrets::default();
    let diagnostics = DiagnosticSink::new(&secrets);
    let rt = GatewayRuntime::start(
        test_gateway_config(),
        &secrets,
        || Ok(MemStore::new()),
        CoreConfig::default(),
        DoorsAdapter(NoopDoors),
        GatewayOptions {
            clock: clock.clone(),
            provider: Some(Arc::new(ScriptedModelProvider::new())),
            diagnostics: diagnostics.clone(),
            cues: CueRecorder::new(),
            seams: GatewaySeams::default(),
        },
    )
    .await
    .expect("start gateway");
    (rt, clock, diagnostics)
}

fn make_percept(actor: &str, place: &str, ts_ms: u64, payload: &[u8]) -> Percept {
    let mut p = Percept::minimal(
        ActorId::new(actor),
        PlaceId::new(place),
        Audience::new([ActorId::new(actor), ActorId::new("soul")]),
        Time::from_millis(ts_ms),
        OpaquePayload::from_bytes(payload),
    );
    p.addressees = vec![ActorId::new("soul")];
    p
}

fn make_action(action_id: &str, cap_id: &str, place: &str, ts_ms: u64) -> Action {
    Action {
        protocol_version: CURRENT_PROTOCOL_VERSION,
        action_id: ActionId::new(action_id),
        capability_id: CapabilityId::new(cap_id),
        places: vec![PlaceId::new(place)],
        audience: Audience::new([ActorId::new("per_alice"), ActorId::new("soul")]),
        origin_actor: OriginActor::new(ActorId::new("soul")),
        taint: Taint::Trusted,
        payload: OpaquePayload::from_bytes(b"{\"text\":\"hello\"}"),
        correlation_id: None,
        expected_effect: None,
        h_act: None,
        timestamp_ms: Time::from_millis(ts_ms),
    }
}

struct MockBodyEndpoint {
    outbound_tx: mpsc::UnboundedSender<String>,
    responses_rx: mpsc::UnboundedReceiver<Waist2Response<Value>>,
    executed_rx: mpsc::UnboundedReceiver<ActionExecuteParams>,
    shutdown_tx: mpsc::UnboundedSender<()>,
    task: tokio::task::JoinHandle<()>,
}

fn spawn_body_loop<R, W>(
    body_id: &str,
    initial_places: Vec<PlaceId>,
    cap_id: &str,
    reader: R,
    mut writer: W,
) -> MockBodyEndpoint
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let (outbound_tx, mut outbound_rx) = mpsc::unbounded_channel::<String>();
    let (responses_tx, responses_rx) = mpsc::unbounded_channel::<Waist2Response<Value>>();
    let (executed_tx, executed_rx) = mpsc::unbounded_channel::<ActionExecuteParams>();
    let (shutdown_tx, mut shutdown_rx) = mpsc::unbounded_channel::<()>();

    let bid = BodyId::new(body_id);
    let cid = CapabilityId::new(cap_id);
    let task = tokio::spawn(async move {
        let mut buf_reader = TokioBufReader::new(reader);
        let mut line = String::new();
        loop {
            line.clear();
            tokio::select! {
                biased;
                _ = shutdown_rx.recv() => {
                    break;
                }
                maybe_out = outbound_rx.recv() => {
                    match maybe_out {
                        Some(msg) => {
                            if writer.write_all(msg.as_bytes()).await.is_err()
                                || writer.write_all(b"\n").await.is_err()
                                || writer.flush().await.is_err()
                            {
                                break;
                            }
                        }
                        None => break,
                    }
                }
                read_res = buf_reader.read_line(&mut line) => {
                    let Ok(n) = read_res else { break; };
                    if n == 0 {
                        break;
                    }
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        continue;
                    }
                    let raw: Value = serde_json::from_str(trimmed).expect("valid json");
                    if raw.get("method").is_none() {
                        if let Ok(resp) = serde_json::from_value::<Waist2Response<Value>>(raw) {
                            let _ = responses_tx.send(resp);
                        }
                        continue;
                    }
                    let req: Waist2Request<Value> = serde_json::from_value(raw).expect("valid waist2 req");
                    match req.method {
                        Waist2Method::BodyDiscover => {
                            let _p: BodyDiscoverParams = serde_json::from_value(req.params).expect("discover");
                            let res = BodyDiscoverResult {
                                body_id: bid.clone(),
                                protocol_version: CURRENT_PROTOCOL_VERSION,
                                capabilities: vec![CapabilityDescriptor {
                                    id: cid.clone(),
                                    description: format!("Capability on {bid}"),
                                    places: initial_places.clone(),
                                    flags: CapabilityFlags {
                                        communicative: true,
                                        identity_changing: false,
                                        irreversible: false,
                                    },
                                    typical_latency_ms: 10,
                                    capability_schema_hash: CapabilitySchemaHash::new("h1"),
                                    sensed_effect: None,
                                }],
                                places: initial_places.clone(),
                            };
                            let resp = serde_json::to_string(&Waist2Response::ok(req.id, res)).unwrap();
                            let _ = writer.write_all(resp.as_bytes()).await;
                            let _ = writer.write_all(b"\n").await;
                            let _ = writer.flush().await;
                        }
                        Waist2Method::ActionExecute => {
                            let params: ActionExecuteParams = serde_json::from_value(req.params).expect("exec");
                            let _ = executed_tx.send(params.clone());
                            let receipt = Receipt {
                                action_id: params.action_id,
                                capability_id: params.capability_id,
                                outcome: ReceiptOutcome::Done,
                                reason_code: None,
                                detail: Some("ok".to_owned()),
                                termination_reason: Some(TerminationReason::Normal),
                                progress: Some(1.0),
                                delivered_units: Some(1),
                                h_act: params.h_act,
                                timestamp_ms: Time::from_millis(1_000_050),
                            };
                            let resp = serde_json::to_string(&Waist2Response::ok(
                                req.id,
                                ActionExecuteResult {
                                    accepted: true,
                                    receipt: Some(receipt),
                                },
                            ))
                            .unwrap();
                            let _ = writer.write_all(resp.as_bytes()).await;
                            let _ = writer.write_all(b"\n").await;
                            let _ = writer.flush().await;
                        }
                        Waist2Method::BodyShutdown => {
                            let resp = serde_json::to_string(&Waist2Response::ok(
                                req.id,
                                monoagens_protocol::BodyShutdownResult { accepted: true },
                            ))
                            .unwrap();
                            let _ = writer.write_all(resp.as_bytes()).await;
                            let _ = writer.write_all(b"\n").await;
                            let _ = writer.flush().await;
                            break;
                        }
                        _ => {}
                    }
                }
            }
        }
    });

    MockBodyEndpoint {
        outbound_tx,
        responses_rx,
        executed_rx,
        shutdown_tx,
        task,
    }
}

async fn connect_duplex_body(
    handle: &GatewayHandle,
    body_id: &str,
    initial_places: Vec<PlaceId>,
    cap_id: &str,
) -> (BodyId, MockBodyEndpoint) {
    let (gw_stream, body_stream) = tokio::io::duplex(64 * 1024);
    let (gw_read, gw_write) = tokio::io::split(gw_stream);
    let (body_read, body_write) = tokio::io::split(body_stream);

    let endpoint = spawn_body_loop(body_id, initial_places, cap_id, body_read, body_write);
    let connected_id = handle
        .connect_ndjson_body(gw_read, gw_write, Some(BodyId::new(body_id)))
        .await
        .expect("connect ndjson body");
    (connected_id, endpoint)
}

fn send_percept_req(tx: &mpsc::UnboundedSender<String>, rpc_id: i64, percept: Percept) {
    let req = Waist2Request::new(
        Some(RpcId::Int(rpc_id)),
        Waist2Method::PerceptEmit,
        PerceptEmitParams::from_percept(
            Waist2Headers::new(format!("percept-{rpc_id}")),
            percept,
        ),
    );
    let line = serde_json::to_string(&req).expect("serialize percept req");
    tx.send(line).expect("send percept line");
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
async fn dynamic_place_routing_registers_on_percept_emit_and_dispatches_action() {
    let (rt, _clock, _diag) = start_test_gateway().await;
    let handle = rt.handle();

    let (bid, mut endpoint) = connect_duplex_body(&handle, "wa-body", Vec::new(), "wa.send").await;
    assert!(handle.router().connected_bodies().contains(&bid));
    assert!(handle.router().owned_places(&bid).is_empty());

    let dynamic_place = "wa:group:120363@g.us";
    send_percept_req(
        &endpoint.outbound_tx,
        10,
        make_percept("per_alice", dynamic_place, 1_000_010, b"hi group"),
    );

    let resp = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        endpoint.responses_rx.recv(),
    )
    .await
    .expect("response timeout")
    .expect("response received");
    assert!(resp.error.is_none(), "expected ok response, got {:?}", resp.error);
    assert_eq!(
        resp.result.as_ref().and_then(|v| v.get("accepted")).and_then(Value::as_bool),
        Some(true)
    );

    assert_eq!(
        handle.router().owned_places(&bid),
        vec![PlaceId::new(dynamic_place)]
    );

    let action = make_action("act-dyn-1", "wa.send", dynamic_place, 1_000_020);
    handle.router().dispatch_action(action.clone()).await;

    let executed = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        endpoint.executed_rx.recv(),
    )
    .await
    .expect("action timeout")
    .expect("action executed");
    assert_eq!(executed.action_id, action.action_id);
    assert_eq!(executed.places, vec![PlaceId::new(dynamic_place)]);

    poll_until(|| async {
        handle
            .read_events()
            .await
            .expect("read events")
            .iter()
            .any(|ev| {
                matches!(
                    &ev.detail,
                    Some(EventDetail::Receipt {
                        action_id,
                        outcome: ReceiptOutcome::Done,
                        ..
                    }) if action_id == &action.action_id
                )
            })
    })
    .await;

    rt.shutdown().await;
    let _ = endpoint.task.await;
}

#[tokio::test]
async fn dynamic_place_anti_hijack_guard_refuses_cross_body_claim() {
    let (rt, _clock, diag) = start_test_gateway().await;
    let handle = rt.handle();

    let (body_a, mut ep_a) = connect_duplex_body(&handle, "body-a", Vec::new(), "wa.send").await;
    let (body_b, mut ep_b) = connect_duplex_body(&handle, "body-b", Vec::new(), "tg.send").await;

    let shared_place = "wa:group:120363@g.us";
    send_percept_req(
        &ep_a.outbound_tx,
        1,
        make_percept("per_alice", shared_place, 1_000_010, b"legit owner"),
    );
    let resp_a = ep_a.responses_rx.recv().await.expect("resp a");
    assert!(resp_a.error.is_none());

    // Wait for Body A's percept to enter Core so we can assert Body B's spoof does not.
    poll_until(|| async {
        handle
            .read_events()
            .await
            .expect("events")
            .iter()
            .any(|e| e.kind == EventKind::Percept)
    })
    .await;
    let events_before = handle.read_events().await.expect("events before");

    // Body B tries to hijack the place owned by Body A.
    send_percept_req(
        &ep_b.outbound_tx,
        2,
        make_percept("per_mallory", shared_place, 1_000_020, b"spoofed"),
    );
    let resp_b = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        ep_b.responses_rx.recv(),
    )
    .await
    .expect("resp b timeout")
    .expect("resp b");

    let err = resp_b.error.expect("expected JSON-RPC error for hijack");
    assert!(
        err.message.contains("ERR_PLACE_OWNED_BY_OTHER_BODY"),
        "unexpected error message: {}",
        err.message
    );

    assert_eq!(
        handle.router().owned_places(&body_a),
        vec![PlaceId::new(shared_place)]
    );
    assert!(handle.router().owned_places(&body_b).is_empty());

    let events_after = handle.read_events().await.expect("events after");
    assert_eq!(
        events_before.len(),
        events_after.len(),
        "hijacked percept must not be submitted to Core"
    );

    assert!(diag.records().iter().any(|r| {
        r.level == LogLevel::Warn
            && r.component == "gateway.waist2"
            && r.message.contains("ERR_PLACE_OWNED_BY_OTHER_BODY")
    }));

    rt.shutdown().await;
    let _ = ep_a.task.await;
    let _ = ep_b.task.await;
}

#[tokio::test]
async fn dynamic_place_cleaned_up_on_disconnect_and_reclaimable() {
    let (rt, _clock, _diag) = start_test_gateway().await;
    let handle = rt.handle();

    let (body_a, mut ep_a) = connect_duplex_body(&handle, "body-a", Vec::new(), "wa.send").await;
    let place = "wa:group:120363@g.us";

    send_percept_req(
        &ep_a.outbound_tx,
        1,
        make_percept("per_alice", place, 1_000_010, b"hello"),
    );
    let resp_a = ep_a.responses_rx.recv().await.expect("resp a");
    assert!(resp_a.error.is_none());
    assert_eq!(handle.router().owned_places(&body_a), vec![PlaceId::new(place)]);

    // Simulate Body A disconnecting unexpectedly.
    let _ = ep_a.shutdown_tx.send(());
    let _ = ep_a.task.await;

    poll_until(|| async {
        !handle.router().connected_bodies().contains(&body_a)
            && handle.router().owned_places(&body_a).is_empty()
    })
    .await;

    // A newly connected body can now legitimately claim the place.
    let (body_b, mut ep_b) = connect_duplex_body(&handle, "body-b", Vec::new(), "wa.send").await;
    send_percept_req(
        &ep_b.outbound_tx,
        2,
        make_percept("per_alice", place, 1_000_020, b"reclaimed"),
    );
    let resp_b = ep_b.responses_rx.recv().await.expect("resp b");
    assert!(resp_b.error.is_none(), "expected reclaim to succeed: {:?}", resp_b.error);
    assert_eq!(handle.router().owned_places(&body_b), vec![PlaceId::new(place)]);

    let action = make_action("act-reclaim-1", "wa.send", place, 1_000_030);
    handle.router().dispatch_action(action.clone()).await;
    let executed = ep_b.executed_rx.recv().await.expect("executed on body_b");
    assert_eq!(executed.action_id, action.action_id);

    rt.shutdown().await;
    let _ = ep_b.task.await;
}

#[tokio::test]
async fn connect_configured_bodies_isolates_failing_plugin_from_valid_sibling() {
    let (rt, _clock, diag) = start_test_gateway().await;
    let handle = rt.handle();

    let tmp = TempDir::new().expect("tempdir");
    let sock_path = tmp.path().join("valid-body.sock");
    let listener = UnixListener::bind(&sock_path).expect("bind unix listener");

    let (ep_tx, ep_rx) = tokio::sync::oneshot::channel::<MockBodyEndpoint>();
    let accept_task = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("accept unix body");
        let (read_half, write_half) = stream.into_split();
        let ep = spawn_body_loop(
            "tg-body",
            vec![PlaceId::new("tg:chat:42")],
            "tg.send",
            read_half,
            write_half,
        );
        let _ = ep_tx.send(ep);
    });

    let bodies = vec![
        BodyPluginConfig {
            id: "broken-wa".to_owned(),
            transport: BodyTransportConfig::Stdio {
                command: "/nonexistent/monoagens-body-bin".to_owned(),
                args: Vec::new(),
            },
        },
        BodyPluginConfig {
            id: "valid-tg".to_owned(),
            transport: BodyTransportConfig::UnixSocket {
                path: sock_path.clone(),
            },
        },
    ];

    let outcomes = handle.connect_configured_bodies(&bodies).await;
    assert_eq!(outcomes.len(), 2);
    assert_eq!(outcomes[0].config_id, "broken-wa");
    assert!(outcomes[0].result.is_err(), "invalid binary should fail");
    assert_eq!(outcomes[1].config_id, "valid-tg");
    assert_eq!(outcomes[1].result, Ok(BodyId::new("tg-body")));

    accept_task.await.expect("accept task");
    let mut ep = ep_rx.await.expect("mock endpoint");

    assert_eq!(handle.router().connected_bodies(), vec![BodyId::new("tg-body")]);
    assert!(diag.records().iter().any(|r| {
        r.level == LogLevel::Warn
            && r.component == "gateway.body"
            && r.message.contains("broken-wa")
    }));

    // Verify the valid sibling is fully operational for both percepts and actions.
    send_percept_req(
        &ep.outbound_tx,
        7,
        make_percept("per_bob", "tg:chat:42", 1_000_010, b"ping"),
    );
    let resp = ep.responses_rx.recv().await.expect("percept resp");
    assert!(resp.error.is_none());

    let action = make_action("act-tg-1", "tg.send", "tg:chat:42", 1_000_020);
    handle.router().dispatch_action(action.clone()).await;
    let executed = ep.executed_rx.recv().await.expect("action on tg-body");
    assert_eq!(executed.action_id, action.action_id);

    rt.shutdown().await;
    let _ = ep.task.await;
}
