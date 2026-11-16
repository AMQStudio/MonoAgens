use body_mcp_bridge::{
    LoopbackMcpTransport, McpBridgeBody, McpBridgeConfig, McpProtocolVersion, StdioMcpTransport,
};
use monoagens_body_kit::PersistentBodyRuntime;
use monoagens_body_sdk::{
    ActionExecuteParams, ActionExecuteResult, ActionId, ActorId, ApprovalResolveParams,
    ApprovalResolveResult, Audience, BodyDiscoverParams, BodyHandler, BodyStatus, CapabilityId,
    HAct, OpaquePayload, PlaceId, ReceiptOutcome, RpcId, StateQueryParams, StateQueryResult,
    StateQueryStatus, Time, Waist2ErrorCode, Waist2Headers, Waist2Method, Waist2Request,
    Waist2Response,
};
use serde_json::json;
use tempfile::TempDir;

fn mock_server_bin() -> &'static str {
    env!("CARGO_BIN_EXE_mock-mcp-server")
}

#[test]
fn mcp_bridge_input_required_held_then_approval_resolve_single_receipt() {
    let tmp = TempDir::new().expect("tempdir");
    let mut rt = PersistentBodyRuntime::new(Some(tmp.path().to_path_buf()));
    let mut bridge = McpBridgeBody::connect(
        McpBridgeConfig::new("mock"),
        StdioMcpTransport::spawn(mock_server_bin(), &[]),
    );
    assert_eq!(
        bridge.mcp_protocol_version(),
        McpProtocolVersion::V2026_07_28
    );
    assert_eq!(bridge.health_status(), BodyStatus::Ready);

    let disc = bridge
        .discover(BodyDiscoverParams {
            headers: Waist2Headers::new("d-1"),
            body_id: None,
        })
        .expect("discover");
    assert_eq!(disc.places, vec![PlaceId::new("mcp:mock")]);
    assert!(
        disc.capabilities
            .iter()
            .any(|c| c.id.as_str() == "mcp.mock.echo")
    );

    // input_required -> held
    let exec_req = serde_json::to_string(&Waist2Request::new(
        Some(RpcId::Int(1)),
        Waist2Method::ActionExecute,
        ActionExecuteParams {
            headers: Waist2Headers::new("k-mcp-hold"),
            action_id: ActionId::new("act-mcp-hold"),
            capability_id: CapabilityId::new("mcp.mock.echo"),
            places: vec![PlaceId::new("mcp:mock")],
            audience: Audience::new([ActorId::new("per_owner")]),
            payload: OpaquePayload::from_bytes(b"hold".as_slice()),
            timeout_ms: Some(5_000),
            h_act: Some(HAct::new("blake3:mcp-hold")),
        },
    ))
    .expect("encode");
    let exec_resp: Waist2Response<ActionExecuteResult> =
        serde_json::from_str(&rt.dispatch_line(&exec_req, &mut bridge).expect("resp"))
            .expect("decode");
    let held_rcpt = exec_resp.result.expect("res").receipt.expect("receipt");
    assert_eq!(held_rcpt.outcome, ReceiptOutcome::Held);

    // approval/resolve -> single terminal Done receipt
    let approve_req = serde_json::to_string(&Waist2Request::new(
        Some(RpcId::Int(2)),
        Waist2Method::ApprovalResolve,
        ApprovalResolveParams {
            headers: Waist2Headers::new("k-mcp-approve"),
            action_id: ActionId::new("act-mcp-hold"),
            h_act: HAct::new("blake3:mcp-hold"),
            state_epoch: 1,
            approved: true,
            resolver: ActorId::new("per_owner"),
            resolved_at_ms: Time::from_millis(2_000),
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
            headers: Waist2Headers::new("k-mcp-sq"),
            action_id: ActionId::new("act-mcp-hold"),
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

    // Duplicate approval/resolve with new idempotency key fails cleanly.
    let dup_approve = serde_json::to_string(&Waist2Request::new(
        Some(RpcId::Int(4)),
        Waist2Method::ApprovalResolve,
        ApprovalResolveParams {
            headers: Waist2Headers::new("k-mcp-approve-2"),
            action_id: ActionId::new("act-mcp-hold"),
            h_act: HAct::new("blake3:mcp-hold"),
            state_epoch: 1,
            approved: true,
            resolver: ActorId::new("per_owner"),
            resolved_at_ms: Time::from_millis(2_500),
        },
    ))
    .expect("encode");
    let dup_resp: Waist2Response<ApprovalResolveResult> =
        serde_json::from_str(&rt.dispatch_line(&dup_approve, &mut bridge).expect("resp"))
            .expect("decode");
    assert_eq!(
        dup_resp.error.expect("err").error_code(),
        Some(Waist2ErrorCode::InvalidReceiptTransition)
    );
}

#[test]
fn mcp_bridge_task_polling_yields_single_receipt_and_supports_2025_fallback() {
    for (args, expected_ver) in [
        (vec![], McpProtocolVersion::V2026_07_28),
        (
            vec!["--legacy-2025".to_owned()],
            McpProtocolVersion::V2025_11_25,
        ),
    ] {
        let mut bridge = McpBridgeBody::connect(
            McpBridgeConfig::new("mock"),
            StdioMcpTransport::spawn(mock_server_bin(), &args),
        );
        assert_eq!(bridge.mcp_protocol_version(), expected_ver);

        let res = bridge
            .execute(ActionExecuteParams {
                headers: Waist2Headers::new("k-poll"),
                action_id: ActionId::new("act-poll-1"),
                capability_id: CapabilityId::new("mcp.mock.long_job"),
                places: vec![PlaceId::new("mcp:mock")],
                audience: Audience::new([ActorId::new("per_owner")]),
                payload: OpaquePayload::from_bytes(json!({ "polls": 3 }).to_string().into_bytes()),
                timeout_ms: Some(5_000),
                h_act: None,
            })
            .expect("execute");
        assert!(res.accepted);
        let receipt = res.receipt.expect("single receipt");
        assert_eq!(receipt.outcome, ReceiptOutcome::Done);
        assert_eq!(receipt.progress, Some(1.0));
        assert_eq!(receipt.delivered_units, Some(1));
    }
}

#[test]
fn mcp_bridge_schema_change_updates_capability_schema_hash_and_unknown_cap_refused() {
    let mut bridge = McpBridgeBody::connect(
        McpBridgeConfig::new("mock"),
        StdioMcpTransport::spawn(mock_server_bin(), &[]),
    );
    let before = bridge
        .discover(BodyDiscoverParams {
            headers: Waist2Headers::new("d1"),
            body_id: None,
        })
        .expect("discover before");
    let before_hash = before
        .capabilities
        .iter()
        .find(|c| c.id.as_str() == "mcp.mock.echo")
        .expect("echo cap")
        .capability_schema_hash
        .clone();

    // Mutate schema via mock server and observe notification -> hash update on body/discover.
    let _ = bridge
        .execute(ActionExecuteParams {
            headers: Waist2Headers::new("k-mutate"),
            action_id: ActionId::new("act-mutate"),
            capability_id: CapabilityId::new("mcp.mock.echo"),
            places: vec![PlaceId::new("mcp:mock")],
            audience: Audience::new([ActorId::new("per_owner")]),
            payload: OpaquePayload::from_bytes(
                json!({ "mutate_schema": "extra_field" })
                    .to_string()
                    .into_bytes(),
            ),
            timeout_ms: Some(5_000),
            h_act: None,
        })
        .expect("mutate call");

    let after = bridge
        .discover(BodyDiscoverParams {
            headers: Waist2Headers::new("d2"),
            body_id: None,
        })
        .expect("discover after");
    let after_hash = after
        .capabilities
        .iter()
        .find(|c| c.id.as_str() == "mcp.mock.echo")
        .expect("echo cap")
        .capability_schema_hash
        .clone();
    assert_ne!(before_hash, after_hash);

    // Unknown capability returns -32001 ERR_UNKNOWN_CAPABILITY.
    let err = bridge
        .execute(ActionExecuteParams {
            headers: Waist2Headers::new("k-unknown"),
            action_id: ActionId::new("act-unknown"),
            capability_id: CapabilityId::new("mcp.mock.does_not_exist"),
            places: vec![PlaceId::new("mcp:mock")],
            audience: Audience::new([ActorId::new("per_owner")]),
            payload: OpaquePayload::from_bytes(b"{}".as_slice()),
            timeout_ms: Some(1_000),
            h_act: None,
        })
        .expect_err("must reject unknown capability");
    assert_eq!(err.error_code(), Some(Waist2ErrorCode::UnknownCapability));
}

#[test]
fn mcp_bridge_absent_server_degrades_gracefully_without_crashing() {
    let mut bridge = McpBridgeBody::connect(
        McpBridgeConfig::new("broken"),
        StdioMcpTransport::spawn("/nonexistent/mcp-server-binary", &[]),
    );
    assert_eq!(bridge.health_status(), BodyStatus::Degraded);
    assert!(bridge.last_error().is_some());

    let hb = bridge.build_heartbeat(RpcId::Int(99), 1_000, "hb-broken");
    assert_eq!(hb.params.status, BodyStatus::Degraded);

    // Bridge stays alive and still answers body/discover cleanly.
    let disc = bridge
        .discover(BodyDiscoverParams {
            headers: Waist2Headers::new("d-broken"),
            body_id: None,
        })
        .expect("discover stays alive");
    assert_eq!(disc.places, vec![PlaceId::new("mcp:broken")]);

    // Also verify loopback polling budget overflow yields Unknown/TimedOut without panic.
    let mut cfg = McpBridgeConfig::new("loop");
    cfg.max_poll_attempts = 1;
    let mut loop_bridge = McpBridgeBody::connect(cfg, LoopbackMcpTransport::new(false));
    let timeout_res = loop_bridge
        .execute(ActionExecuteParams {
            headers: Waist2Headers::new("k-timeout"),
            action_id: ActionId::new("act-timeout"),
            capability_id: CapabilityId::new("mcp.loop.long_job"),
            places: vec![PlaceId::new("mcp:loop")],
            audience: Audience::new([ActorId::new("per_owner")]),
            payload: OpaquePayload::from_bytes(json!({ "polls": 10 }).to_string().into_bytes()),
            timeout_ms: Some(1_000),
            h_act: None,
        })
        .expect("bounded timeout receipt");
    assert_eq!(
        timeout_res.receipt.expect("receipt").outcome,
        ReceiptOutcome::Unknown
    );
}
