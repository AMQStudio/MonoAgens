use monoagens_body_sdk::{
    ActionCancelParams, ActionCancelResult, ActionExecuteParams, ActionExecuteResult, ActionId,
    ApprovalResolveParams, ApprovalResolveResult, BodyDiscoverParams, BodyDiscoverResult,
    BodyHandler, BodyId, BodyRuntime, BodyStatus, HeartbeatScheduler, OpaquePayload, RpcId,
    StateQueryResult, StateQueryStatus, Waist2Error, Waist2ErrorCode, Waist2Headers, Waist2Method,
    Waist2Request, Waist2Response, sha256_hex,
};
use monoagens_protocol::{
    ActorId, Audience, CapabilityDescriptor, CapabilityFlags, CapabilityId, CapabilitySchemaHash,
    PlaceId, ProtocolVersion, Receipt, ReceiptOutcome, TerminationReason, Time,
};
use serde_json::Value;

struct MockBody {
    exec_count: usize,
}

impl BodyHandler for MockBody {
    fn discover(&mut self, params: BodyDiscoverParams) -> Result<BodyDiscoverResult, Waist2Error> {
        Ok(BodyDiscoverResult {
            body_id: params.body_id.unwrap_or_else(|| BodyId::new("body-mock")),
            protocol_version: ProtocolVersion::new(1, 0),
            capabilities: vec![CapabilityDescriptor {
                id: CapabilityId::new("chat.speak"),
                description: "Speak".to_owned(),
                places: vec![PlaceId::new("cli:0")],
                flags: CapabilityFlags::default(),
                typical_latency_ms: 10,
                capability_schema_hash: CapabilitySchemaHash::new("sha256:00"),
                sensed_effect: None,
            }],
            places: vec![PlaceId::new("cli:0")],
        })
    }

    fn execute(&mut self, params: ActionExecuteParams) -> Result<ActionExecuteResult, Waist2Error> {
        self.exec_count += 1;
        Ok(ActionExecuteResult {
            accepted: true,
            receipt: Some(Receipt {
                action_id: params.action_id,
                capability_id: params.capability_id,
                outcome: ReceiptOutcome::Done,
                reason_code: None,
                detail: None,
                termination_reason: Some(TerminationReason::Normal),
                progress: Some(1.0),
                delivered_units: Some(1),
                h_act: params.h_act,
                timestamp_ms: Time::from_millis(100),
            }),
        })
    }

    fn cancel(&mut self, params: ActionCancelParams) -> Result<ActionCancelResult, Waist2Error> {
        Ok(ActionCancelResult {
            cancelled: true,
            receipt: Some(Receipt {
                action_id: params.action_id,
                capability_id: CapabilityId::new("chat.speak"),
                outcome: ReceiptOutcome::Failed,
                reason_code: Some("canceled_by_core".to_owned()),
                detail: params.reason,
                termination_reason: Some(TerminationReason::CanceledByCore),
                progress: Some(0.5),
                delivered_units: Some(1),
                h_act: None,
                timestamp_ms: Time::from_millis(150),
            }),
        })
    }

    fn resolve_approval(
        &mut self,
        _params: ApprovalResolveParams,
    ) -> Result<ApprovalResolveResult, Waist2Error> {
        Ok(ApprovalResolveResult { applied: true })
    }
}

#[test]
fn smoke_round_trip_dedup_journal_blob_and_heartbeat() {
    let req = Waist2Request::new(
        Some(RpcId::Int(1)),
        Waist2Method::ActionExecute,
        ActionExecuteParams {
            headers: Waist2Headers::new("idem-exec-1"),
            action_id: ActionId::new("act-1"),
            capability_id: CapabilityId::new("chat.speak"),
            places: vec![PlaceId::new("cli:0")],
            audience: Audience::new([ActorId::new("per_alice")]),
            payload: OpaquePayload::from_bytes(b"hello".as_slice()),
            timeout_ms: Some(5_000),
            h_act: None,
        },
    );
    let wire = serde_json::to_string(&req).expect("serialize");
    let parsed: Waist2Request<ActionExecuteParams> =
        serde_json::from_str(&wire).expect("parse back");
    assert_eq!(req, parsed);

    let mut rt = BodyRuntime::default();
    let mut body = MockBody { exec_count: 0 };

    // 3x identical idempotency_key -> executes once.
    for _ in 0..3 {
        let resp_line = rt.dispatch_line(&wire, &mut body).expect("response");
        let resp: Waist2Response<ActionExecuteResult> =
            serde_json::from_str(&resp_line).expect("resp");
        assert!(resp.result.expect("ok").accepted);
    }
    assert_eq!(body.exec_count, 1);

    // Crash journal state/query returns terminal receipt for act-1 and unknown for act-999.
    let q_req = serde_json::to_string(&Waist2Request::new(
        Some(RpcId::Int(2)),
        Waist2Method::StateQuery,
        monoagens_body_sdk::StateQueryParams {
            headers: Waist2Headers::new("idem-q-1"),
            action_id: ActionId::new("act-1"),
        },
    ))
    .expect("serialize");
    let q_resp: Waist2Response<StateQueryResult> =
        serde_json::from_str(&rt.dispatch_line(&q_req, &mut body).expect("resp")).expect("parse");
    assert!(matches!(
        q_resp.result.expect("ok").status,
        StateQueryStatus::Terminal { .. }
    ));
    assert!(matches!(
        rt.state_query(&ActionId::new("act-999")).status,
        StateQueryStatus::Unknown
    ));

    // Blob SHA-256 mismatch returns -32005 ERR_HASH_MISMATCH.
    let bad_blob = serde_json::to_string(&Waist2Request::new(
        Some(RpcId::Int(3)),
        Waist2Method::BlobPut,
        monoagens_body_sdk::BlobPutParams {
            headers: Waist2Headers::new("idem-blob-bad"),
            sha256: "0".repeat(64),
            byte_len: 3,
            inline_bytes: Some(OpaquePayload::from_bytes(b"abc".as_slice())),
        },
    ))
    .expect("serialize");
    let blob_resp: Waist2Response<Value> =
        serde_json::from_str(&rt.dispatch_line(&bad_blob, &mut body).expect("resp")).expect("parse");
    assert_eq!(
        blob_resp.error.expect("err").error_code(),
        Some(Waist2ErrorCode::HashMismatch)
    );
    assert_eq!(
        sha256_hex(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );

    // Heartbeat scheduler.
    let mut hb = HeartbeatScheduler::new(1_000);
    assert!(hb.is_due(100));
    let _hb_req = hb.next_request(
        RpcId::Int(4),
        BodyId::new("body-mock"),
        BodyStatus::Ready,
        100,
        "idem-hb-1",
    );
    assert!(!hb.is_due(500));
    assert!(hb.is_due(1_100));
}
