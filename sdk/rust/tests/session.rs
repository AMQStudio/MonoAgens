use monoagens_body_sdk::{
    ActionId, ActorId, Audience, BodyId, BodyStatus, CapabilityId, InboundFrame, MetaValue,
    OpaquePayload, OutboundBuilder, Percept, PerceptEmitParams, PlaceId, Receipt, ReceiptOutcome,
    RpcId, TerminationReason, Time, Waist2ErrorCode, Waist2Headers, Waist2Method, Waist2Request,
    Waist2Response, parse_inbound_line, sha256_hex,
};
use serde_json::json;

#[test]
fn parse_inbound_line_classifies_request_and_response() {
    let req_line = serde_json::to_string(&Waist2Request::new(
        Some(RpcId::Int(7)),
        Waist2Method::BodyDiscover,
        json!({
            "headers": Waist2Headers::new("disc-7"),
            "body_id": null
        }),
    ))
    .expect("serialize request");

    match parse_inbound_line(&req_line).expect("parse request") {
        InboundFrame::Request(req) => {
            assert_eq!(req.id, Some(RpcId::Int(7)));
            assert_eq!(req.method, Waist2Method::BodyDiscover);
        }
        InboundFrame::Response(_) => panic!("expected request frame"),
    }

    let resp_line = serde_json::to_string(&Waist2Response::ok(
        Some(RpcId::Int(1)),
        json!({ "accepted": true }),
    ))
    .expect("serialize response");

    match parse_inbound_line(&resp_line).expect("parse response") {
        InboundFrame::Response(resp) => {
            assert_eq!(resp.id, Some(RpcId::Int(1)));
            assert_eq!(resp.result, Some(json!({ "accepted": true })));
            assert!(resp.error.is_none());
        }
        InboundFrame::Request(_) => panic!("expected response frame"),
    }

    assert!(parse_inbound_line("not json").is_err());
}

#[test]
fn outbound_builder_round_trips_and_validates_audience() {
    let mut builder = OutboundBuilder::new("teloxide");

    // Empty audience fails locally with -32006 ERR_UNDECLARED_AUDIENCE.
    let empty_err = builder
        .percept_emit(
            ActorId::new("tg:1"),
            PlaceId::new("tg:chat-1"),
            Audience::empty(),
            Time::from_millis(100),
            OpaquePayload::from_bytes(b"hi".as_slice()),
        )
        .expect_err("empty audience must be rejected");
    assert_eq!(empty_err.error_code(), Some(Waist2ErrorCode::UndeclaredAudience));

    let empty_percept = Percept::minimal(
        ActorId::new("tg:1"),
        PlaceId::new("tg:chat-1"),
        Audience::empty(),
        Time::from_millis(100),
        OpaquePayload::from_bytes(b"hi".as_slice()),
    );
    let empty_percept_err = builder
        .percept_emit_from_percept(empty_percept)
        .expect_err("empty audience percept must be rejected");
    assert_eq!(
        empty_percept_err.error_code(),
        Some(Waist2ErrorCode::UndeclaredAudience)
    );

    // Valid percept/emit round-trip.
    let percept_req = builder
        .percept_emit(
            ActorId::new("tg:1"),
            PlaceId::new("tg:chat-1"),
            Audience::new([ActorId::new("tg:1"), ActorId::new("owner")]),
            Time::from_millis(1000),
            OpaquePayload::from_bytes(b"hello".as_slice()),
        )
        .expect("valid percept");
    assert_eq!(percept_req.id, Some(RpcId::Int(1)));
    assert_eq!(percept_req.method, Waist2Method::PerceptEmit);
    let wire_p = serde_json::to_string(&percept_req).expect("serialize percept");
    let round_p: Waist2Request<PerceptEmitParams> =
        serde_json::from_str(&wire_p).expect("deserialize percept");
    assert_eq!(percept_req, round_p);

    // action/receipt round-trip.
    let receipt_req = builder.action_receipt(Receipt {
        action_id: ActionId::new("act-1"),
        capability_id: CapabilityId::new("chat.send"),
        outcome: ReceiptOutcome::Done,
        reason_code: None,
        detail: None,
        termination_reason: Some(TerminationReason::Normal),
        progress: Some(1.0),
        delivered_units: Some(1),
        h_act: None,
        timestamp_ms: Time::from_millis(1001),
    });
    assert_eq!(receipt_req.id, Some(RpcId::Int(2)));
    assert_eq!(receipt_req.method, Waist2Method::ActionReceipt);
    let wire_r = serde_json::to_string(&receipt_req).expect("serialize receipt");
    assert_eq!(
        serde_json::from_str::<Waist2Request<_>>(&wire_r).expect("round-trip receipt"),
        receipt_req
    );

    // body/heartbeat round-trip.
    let hb_req = builder.heartbeat(
        BodyId::new("body-rs"),
        BodyStatus::Ready,
        Time::from_millis(1002),
    );
    assert_eq!(hb_req.id, Some(RpcId::Int(3)));
    assert_eq!(hb_req.method, Waist2Method::BodyHeartbeat);
    let wire_hb = serde_json::to_string(&hb_req).expect("serialize heartbeat");
    assert_eq!(
        serde_json::from_str::<Waist2Request<_>>(&wire_hb).expect("round-trip hb"),
        hb_req
    );

    // blob/put & blob/get round-trip.
    let data = b"voice-note-bytes";
    let put_req = builder.blob_put(data, Some("audio/ogg"), Some("scope-rs"));
    assert_eq!(put_req.id, Some(RpcId::Int(4)));
    assert_eq!(put_req.params.sha256, sha256_hex(data));
    assert_eq!(put_req.params.byte_len, data.len() as u64);
    assert_eq!(
        put_req.params.headers.meta.get("content_type"),
        Some(&MetaValue::String("audio/ogg".to_owned()))
    );
    assert_eq!(
        put_req.params.headers.meta.get("scope_id"),
        Some(&MetaValue::String("scope-rs".to_owned()))
    );
    let wire_put = serde_json::to_string(&put_req).expect("serialize blob/put");
    assert_eq!(
        serde_json::from_str::<Waist2Request<_>>(&wire_put).expect("round-trip blob/put"),
        put_req
    );

    let get_req = builder.blob_get("blob://staged/1", Some(sha256_hex(data)));
    assert_eq!(get_req.id, Some(RpcId::Int(5)));
    assert_eq!(get_req.method, Waist2Method::BlobGet);
    let wire_get = serde_json::to_string(&get_req).expect("serialize blob/get");
    assert_eq!(
        serde_json::from_str::<Waist2Request<_>>(&wire_get).expect("round-trip blob/get"),
        get_req
    );
}
