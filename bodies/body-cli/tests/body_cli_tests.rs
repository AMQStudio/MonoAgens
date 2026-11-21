use std::collections::BTreeSet;

use body_cli::{CliBody, MemoryTerminalSink, extract_receipt_pids};
use monoagens_body_kit::{HonestCueKind, PersistentBodyRuntime};
use monoagens_body_sdk::{
    ActionExecuteParams, ActionExecuteResult, ActionId, ActorId, ApprovalResolveParams,
    ApprovalResolveResult, Audience, BodyDiscoverParams, BodyHandler, CapabilityId, HAct,
    OpaquePayload, PerceptEmitResult, PlaceId, ReceiptOutcome, RpcId, StateQueryParams,
    StateQueryResult, StateQueryStatus, Time, Waist2ErrorCode, Waist2Headers, Waist2Method,
    Waist2Request, Waist2Response, sort_capabilities,
};
use tempfile::TempDir;

#[test]
fn body_cli_discovers_sorted_capabilities_with_schema_hashes() {
    let mut body = CliBody::new(
        monoagens_body_sdk::BodyId::new("body-cli"),
        PlaceId::new("cli:terminal"),
        ActorId::new("per_owner"),
        4096,
        MemoryTerminalSink::default(),
    );

    let discover = body
        .discover(BodyDiscoverParams {
            headers: Waist2Headers::new("d1"),
            body_id: None,
        })
        .expect("discover");

    assert_eq!(discover.places, vec![PlaceId::new("cli:terminal")]);
    assert_eq!(discover.capabilities.len(), 1);
    assert_eq!(discover.capabilities[0].id, CapabilityId::new("chat.speak"));
    assert!(
        discover.capabilities[0]
            .capability_schema_hash
            .as_str()
            .starts_with("sha256:")
    );
    let mut expected_sorted = discover.capabilities.clone();
    sort_capabilities(&mut expected_sorted);
    assert_eq!(discover.capabilities, expected_sorted);
}

#[test]
fn body_cli_percept_round_trips_and_action_produces_single_receipt_with_echo_id() {
    let mut body = CliBody::new(
        monoagens_body_sdk::BodyId::new("body-cli"),
        PlaceId::new("cli:terminal"),
        ActorId::new("per_owner"),
        4096,
        MemoryTerminalSink::default(),
    );
    body.set_now_ms(1_500);

    // 1. Percept round-trip.
    let (percept, is_echo) =
        body.build_user_percept("hello mind", None, None, Time::from_millis(1_000), "p1");
    assert!(!is_echo);
    let p_res = body.on_percept(percept).expect("percept accepted");
    assert!(p_res.accepted);

    // 2. Action execution renders immediately (d_art = 0 ms), returns 1 receipt, registers echo pid.
    let exec_res = body
        .execute(ActionExecuteParams {
            headers: Waist2Headers::new("exec-1"),
            action_id: ActionId::new("act-speak-1"),
            capability_id: CapabilityId::new("chat.speak"),
            places: vec![PlaceId::new("cli:terminal")],
            audience: Audience::new([ActorId::new("per_owner")]),
            payload: OpaquePayload::from_bytes(b"hello owner".as_slice()),
            timeout_ms: Some(5_000),
            h_act: None,
        })
        .expect("execute");
    assert!(exec_res.accepted);
    let receipt = exec_res.receipt.expect("single receipt");
    assert_eq!(receipt.outcome, ReceiptOutcome::Done);
    assert_eq!(receipt.delivered_units, Some(1));
    assert_eq!(receipt.progress, Some(1.0));
    assert_eq!(body.sink().rendered.len(), 1);
    assert_eq!(body.sink().rendered[0].1, "hello owner");

    let pids = extract_receipt_pids(&receipt);
    assert_eq!(pids.len(), 1);
    assert!(body.echo_registry().is_self_echo(&pids[0]));

    // 3. Subsequent percept carrying the same platform message id is flagged as self-echo.
    let (_echo_percept, detected_echo) = body.build_user_percept(
        "hello owner",
        Some(&pids[0]),
        None,
        Time::from_millis(1_600),
        "p2",
    );
    assert!(detected_echo);
}

#[test]
fn body_cli_cues_appear_only_past_virtual_clock_thresholds() {
    let mut body = CliBody::new(
        monoagens_body_sdk::BodyId::new("body-cli"),
        PlaceId::new("cli:terminal"),
        ActorId::new("per_owner"),
        4096,
        MemoryTerminalSink::default(),
    );

    body.begin_wait("wait-act-1", 10_000);
    assert!(body.poll_wait_cues(10_500).is_empty());
    assert!(body.poll_wait_cues(10_999).is_empty());
    assert!(body.sink().cues.is_empty());

    let cues_1s = body.poll_wait_cues(11_000);
    assert_eq!(cues_1s.len(), 1);
    assert_eq!(cues_1s[0].kind, HonestCueKind::WorkingCue);

    assert!(body.poll_wait_cues(19_999).is_empty());

    let cues_10s = body.poll_wait_cues(20_000);
    assert_eq!(cues_10s.len(), 1);
    assert_eq!(cues_10s[0].kind, HonestCueKind::ProgressNote);
    assert_eq!(body.sink().cues.len(), 2);

    body.finish_wait();
    assert!(body.poll_wait_cues(50_000).is_empty());
}

#[test]
fn body_cli_persistent_runtime_handles_unknown_cap_held_approval_and_crash_journal() {
    let tmp = TempDir::new().expect("tempdir");
    let state_dir = tmp.path().to_path_buf();

    let mut body = CliBody::new(
        monoagens_body_sdk::BodyId::new("body-cli"),
        PlaceId::new("cli:terminal"),
        ActorId::new("per_owner"),
        4096,
        MemoryTerminalSink::default(),
    );
    let mut rt = PersistentBodyRuntime::new(Some(state_dir.clone()));

    // Unknown capability -> -32001.
    let bad_cap_req = serde_json::to_string(&Waist2Request::new(
        Some(RpcId::Int(1)),
        Waist2Method::ActionExecute,
        ActionExecuteParams {
            headers: Waist2Headers::new("k-bad-cap"),
            action_id: ActionId::new("act-bad"),
            capability_id: CapabilityId::new("nonexistent.cap"),
            places: vec![PlaceId::new("cli:terminal")],
            audience: Audience::new([ActorId::new("per_owner")]),
            payload: OpaquePayload::from_bytes(b"hi".as_slice()),
            timeout_ms: Some(1_000),
            h_act: None,
        },
    ))
    .expect("encode");
    let bad_resp: Waist2Response<ActionExecuteResult> =
        serde_json::from_str(&rt.dispatch_line(&bad_cap_req, &mut body).expect("line"))
            .expect("decode");
    assert_eq!(
        bad_resp.error.expect("err").error_code(),
        Some(Waist2ErrorCode::UnknownCapability)
    );

    // Held -> approval/resolve -> terminal Done, then restart checks state/query.
    let hold_req = serde_json::to_string(&Waist2Request::new(
        Some(RpcId::Int(2)),
        Waist2Method::ActionExecute,
        ActionExecuteParams {
            headers: Waist2Headers::new("k-hold"),
            action_id: ActionId::new("act-hold"),
            capability_id: CapabilityId::new("chat.speak"),
            places: vec![PlaceId::new("cli:terminal")],
            audience: Audience::new([ActorId::new("per_owner")]),
            payload: OpaquePayload::from_bytes(b"hold".as_slice()),
            timeout_ms: Some(1_000),
            h_act: Some(HAct::new("blake3:hold")),
        },
    ))
    .expect("encode");
    let hold_resp: Waist2Response<ActionExecuteResult> =
        serde_json::from_str(&rt.dispatch_line(&hold_req, &mut body).expect("line"))
            .expect("decode");
    assert_eq!(
        hold_resp.result.expect("res").receipt.expect("r").outcome,
        ReceiptOutcome::Held
    );

    let approve_req = serde_json::to_string(&Waist2Request::new(
        Some(RpcId::Int(3)),
        Waist2Method::ApprovalResolve,
        ApprovalResolveParams {
            headers: Waist2Headers::new("k-approve"),
            action_id: ActionId::new("act-hold"),
            h_act: HAct::new("blake3:hold"),
            state_epoch: 1,
            approved: true,
            resolver: ActorId::new("per_owner"),
            resolved_at_ms: Time::from_millis(2_000),
            signature: None,
            out_of_band_channel: false,
        },
    ))
    .expect("encode");
    let approve_resp: Waist2Response<ApprovalResolveResult> =
        serde_json::from_str(&rt.dispatch_line(&approve_req, &mut body).expect("line"))
            .expect("decode");
    assert!(approve_resp.result.expect("res").applied);

    // Simulate process restart with same state_dir.
    drop(rt);
    let mut rt2 = PersistentBodyRuntime::new(Some(state_dir));
    let sq_req = serde_json::to_string(&Waist2Request::new(
        Some(RpcId::Int(4)),
        Waist2Method::StateQuery,
        StateQueryParams {
            headers: Waist2Headers::new("k-sq"),
            action_id: ActionId::new("act-hold"),
        },
    ))
    .expect("encode");
    let sq_resp: Waist2Response<StateQueryResult> =
        serde_json::from_str(&rt2.dispatch_line(&sq_req, &mut body).expect("line"))
            .expect("decode");
    assert!(matches!(
        sq_resp.result.expect("sq").status,
        StateQueryStatus::Terminal { receipt } if receipt.outcome == ReceiptOutcome::Done
    ));

    // Undeclared audience rejected with -32006.
    let bad_percept = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 5,
        "method": "percept/emit",
        "params": {
            "headers": Waist2Headers::new("k-no-aud"),
            "actor": "per_owner",
            "place": "cli:terminal",
            "audience": [],
            "timestamp_ms": 3000,
            "payload": b"x".to_vec()
        }
    })
    .to_string();
    let p_resp: Waist2Response<PerceptEmitResult> =
        serde_json::from_str(&rt2.dispatch_line(&bad_percept, &mut body).expect("line"))
            .expect("decode");
    assert_eq!(
        p_resp.error.expect("err").error_code(),
        Some(Waist2ErrorCode::UndeclaredAudience)
    );
    let _ = BTreeSet::<String>::new();
}
