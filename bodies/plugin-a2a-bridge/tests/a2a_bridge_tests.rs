use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::thread;

use monoagens_body_kit::PersistentBodyRuntime;
use monoagens_body_sdk::{
    ActionExecuteParams, ActionExecuteResult, ActionId, ActorId, ApprovalResolveParams,
    ApprovalResolveResult, Audience, BodyDiscoverParams, BodyHandler, CapabilityId, HAct,
    OpaquePayload, PlaceId, ReceiptOutcome, RpcId, StateQueryParams, StateQueryResult,
    StateQueryStatus, Time, Waist2ErrorCode, Waist2Headers, Waist2Method, Waist2Request,
    Waist2Response,
};
use plugin_a2a_bridge::{
    A2aBridgeBody, A2aBridgeConfig, HttpLoopbackA2aTransport, LocalMockA2aPeer,
};
use serde_json::Value;
use tempfile::TempDir;

fn spawn_local_a2a_http_peer(agent_name: &str, max_requests: usize) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let addr = listener.local_addr().expect("addr");
    let rpc_url = format!("http://{addr}/rpc");
    let peer = Arc::new(Mutex::new(LocalMockA2aPeer::new(agent_name, &rpc_url)));

    thread::spawn(move || {
        for stream_res in listener.incoming().take(max_requests) {
            let Ok(mut stream) = stream_res else {
                break;
            };
            let mut reader = BufReader::new(stream.try_clone().expect("clone stream"));
            let mut req_line = String::new();
            if reader.read_line(&mut req_line).unwrap_or(0) == 0 {
                continue;
            }
            let mut parts = req_line.split_whitespace();
            let method = parts.next().unwrap_or("GET");
            let path = parts.next().unwrap_or("/");
            let mut content_len = 0usize;
            loop {
                let mut hline = String::new();
                if reader.read_line(&mut hline).unwrap_or(0) == 0
                    || hline == "\r\n"
                    || hline == "\n"
                {
                    break;
                }
                if let Some((k, v)) = hline.split_once(':')
                    && k.trim().eq_ignore_ascii_case("Content-Length")
                {
                    content_len = v.trim().parse::<usize>().unwrap_or(0);
                }
            }
            let mut body_buf = vec![0u8; content_len];
            if content_len > 0 {
                let _ = reader.read_exact(&mut body_buf);
            }

            let resp_json = if method == "GET" && path == "/.well-known/agent-card.json" {
                let guard = peer.lock().expect("peer");
                serde_json::to_vec(guard.agent_card()).expect("card json")
            } else {
                let rpc_req: Value = serde_json::from_slice(&body_buf).expect("rpc req");
                let mut guard = peer.lock().expect("peer");
                let rpc_resp = guard.handle_jsonrpc(&rpc_req);
                serde_json::to_vec(&rpc_resp).expect("rpc resp")
            };

            let resp_head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                resp_json.len()
            );
            let _ = stream.write_all(resp_head.as_bytes());
            let _ = stream.write_all(&resp_json);
            let _ = stream.flush();
        }
    });

    addr.to_string()
}

#[test]
fn a2a_bridge_http_loopback_peer_task_lifecycle_and_held_approval() {
    let host_port = spawn_local_a2a_http_peer("planner", 8);
    let tmp = TempDir::new().expect("tempdir");
    let mut rt = PersistentBodyRuntime::new(Some(tmp.path().to_path_buf()));
    let mut bridge = A2aBridgeBody::connect(
        A2aBridgeConfig::new("planner"),
        HttpLoopbackA2aTransport::new(host_port),
    );

    let disc = bridge
        .discover(BodyDiscoverParams {
            headers: Waist2Headers::new("d1"),
            body_id: None,
        })
        .expect("discover");
    assert_eq!(disc.places, vec![PlaceId::new("a2a:planner")]);
    assert!(
        disc.capabilities
            .iter()
            .any(|c| c.id.as_str() == "a2a.planner.async_review")
    );

    // 1. Async task polling via GetTask -> single Done receipt.
    let async_res = bridge
        .execute(ActionExecuteParams {
            headers: Waist2Headers::new("k-async"),
            action_id: ActionId::new("act-a2a-async"),
            capability_id: CapabilityId::new("a2a.planner.async_review"),
            places: vec![PlaceId::new("a2a:planner")],
            audience: Audience::new([ActorId::new("per_owner")]),
            payload: OpaquePayload::from_bytes(b"review doc".as_slice()),
            timeout_ms: Some(5_000),
            h_act: None,
        })
        .expect("async exec");
    let async_rcpt = async_res.receipt.expect("receipt");
    assert_eq!(async_rcpt.outcome, ReceiptOutcome::Done);
    assert_eq!(async_rcpt.progress, Some(1.0));

    // 2. TASK_STATE_INPUT_REQUIRED -> Held -> approval/resolve -> Done.
    let hold_req = serde_json::to_string(&Waist2Request::new(
        Some(RpcId::Int(1)),
        Waist2Method::ActionExecute,
        ActionExecuteParams {
            headers: Waist2Headers::new("k-hold"),
            action_id: ActionId::new("act-a2a-hold"),
            capability_id: CapabilityId::new("a2a.planner.delegate"),
            places: vec![PlaceId::new("a2a:planner")],
            audience: Audience::new([ActorId::new("per_owner")]),
            payload: OpaquePayload::from_bytes(b"hold".as_slice()),
            timeout_ms: Some(5_000),
            h_act: Some(HAct::new("blake3:a2a-hold")),
        },
    ))
    .expect("encode");
    let hold_resp: Waist2Response<ActionExecuteResult> =
        serde_json::from_str(&rt.dispatch_line(&hold_req, &mut bridge).expect("resp"))
            .expect("decode");
    assert_eq!(
        hold_resp.result.expect("res").receipt.expect("r").outcome,
        ReceiptOutcome::Held
    );

    let approve_req = serde_json::to_string(&Waist2Request::new(
        Some(RpcId::Int(2)),
        Waist2Method::ApprovalResolve,
        ApprovalResolveParams {
            headers: Waist2Headers::new("k-approve"),
            action_id: ActionId::new("act-a2a-hold"),
            h_act: HAct::new("blake3:a2a-hold"),
            state_epoch: 1,
            approved: true,
            resolver: ActorId::new("per_owner"),
            resolved_at_ms: Time::from_millis(3_000),
        },
    ))
    .expect("encode");
    let approve_resp: Waist2Response<ApprovalResolveResult> =
        serde_json::from_str(&rt.dispatch_line(&approve_req, &mut bridge).expect("resp"))
            .expect("decode");
    assert!(approve_resp.result.expect("res").applied);

    let sq_req = serde_json::to_string(&Waist2Request::new(
        Some(RpcId::Int(3)),
        Waist2Method::StateQuery,
        StateQueryParams {
            headers: Waist2Headers::new("k-sq"),
            action_id: ActionId::new("act-a2a-hold"),
        },
    ))
    .expect("encode");
    let sq_resp: Waist2Response<StateQueryResult> =
        serde_json::from_str(&rt.dispatch_line(&sq_req, &mut bridge).expect("resp"))
            .expect("decode");
    assert!(matches!(
        sq_resp.result.expect("sq").status,
        StateQueryStatus::Terminal { receipt } if receipt.outcome == ReceiptOutcome::Done
    ));

    // 3. Unknown capability -> -32001.
    let err = bridge
        .execute(ActionExecuteParams {
            headers: Waist2Headers::new("k-bad"),
            action_id: ActionId::new("act-bad"),
            capability_id: CapabilityId::new("a2a.planner.missing"),
            places: vec![PlaceId::new("a2a:planner")],
            audience: Audience::new([ActorId::new("per_owner")]),
            payload: OpaquePayload::from_bytes(b"hi".as_slice()),
            timeout_ms: Some(1_000),
            h_act: None,
        })
        .expect_err("unknown capability");
    assert_eq!(err.error_code(), Some(Waist2ErrorCode::UnknownCapability));
}
