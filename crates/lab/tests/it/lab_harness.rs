use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::Instant;

use monoagens_core::{CoreConfig, DecisionOutcome, DecisionReason, EventKind, MemStore};
use monoagens_lab::{
    CheckKind, ExpectationCheck, RunRecord, Scenario, ScenarioSet, ScriptedActionRule,
    ScriptedEvent, ScriptedEventKind, ScriptedFollowUpPercept, ScriptedModelResponse, SoulDir,
    TestBody, TestBodySpec, run_scenario, sha256_hex,
};
use monoagens_protocol::{
    ActionCancelParams, ActionCancelResult, ActionExecuteParams, ActionExecuteResult, ActionId,
    ActionReceiptParams, ActionReceiptResult, ActorId, ApprovalResolveParams,
    ApprovalResolveResult, Audience, BlobGetParams, BlobGetResult, BlobPutParams, BlobPutResult,
    BodyDiscoverParams, BodyDiscoverResult, BodyHeartbeatParams, BodyHeartbeatResult, BodyId,
    BodyShutdownParams, BodyShutdownResult, BodyStatus, CapabilityDescriptor, CapabilityFlags,
    CapabilityId, CapabilitySchemaHash, ChannelId, HAct, MetaValue, OpaquePayload, Percept,
    PerceptEmitParams, PerceptEmitResult, PlaceId, ProtocolVersion, QuestionAnswerParams,
    QuestionAnswerResult, Receipt, ReceiptOutcome, RpcId, StateQueryParams, StateQueryResult,
    StateQueryStatus, TerminationReason, ThoughtAct, Time, Waist2Call, Waist2ErrorCode,
    Waist2Headers, Waist2Method, Waist2Request, Waist2Response,
};
use serde::Serialize;
use serde_json::Value;
use tempfile::TempDir;

fn protocol_jsonschema_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates dir")
        .join("protocol")
        .join("jsonschema")
}

fn assert_matches_schema<T: Serialize>(schema_name: &str, value: &T) {
    let path = protocol_jsonschema_dir().join(format!("{schema_name}.schema.json"));
    let raw = fs::read_to_string(&path).expect("read schema file");
    let schema: Value = serde_json::from_str(&raw).expect("parse schema json");
    let validator = jsonschema::validator_for(&schema).expect("compile validator");
    let json_val = serde_json::to_value(value).expect("to_value");
    validator
        .validate(&json_val)
        .unwrap_or_else(|err| panic!("{schema_name} validation failed: {err}"));
}

fn test_capability(id: &str, places: Vec<PlaceId>) -> CapabilityDescriptor {
    CapabilityDescriptor {
        id: CapabilityId::new(id),
        description: format!("Capability {id}"),
        places,
        flags: CapabilityFlags {
            communicative: true,
            identity_changing: false,
            irreversible: false,
        },
        typical_latency_ms: 5,
        capability_schema_hash: CapabilitySchemaHash::new(format!("sha256:cap-{id}")),
        sensed_effect: None,
    }
}

fn make_test_body() -> TestBody {
    let room = PlaceId::new("lab:room-1");
    let sensor_room = PlaceId::new("lab:device-1");
    TestBody::from_spec(TestBodySpec {
        body_id: BodyId::new("body-test-1"),
        places: vec![sensor_room.clone(), room.clone()],
        capabilities: vec![
            test_capability("z.push", vec![sensor_room]),
            test_capability("a.speak", vec![room]),
        ],
        action_scripts: vec![ScriptedActionRule {
            capability_id: CapabilityId::new("z.push"),
            place: None,
            payload_contains: Some("hold".to_owned()),
            outcome: ReceiptOutcome::Held,
            receipt_delay_ms: 0,
            termination_reason: Some(TerminationReason::Normal),
            progress: Some(0.0),
            delivered_units: Some(0),
            reason_code: Some("approval_required".to_owned()),
            detail: Some("held for approval".to_owned()),
            follow_up_percepts: Vec::new(),
        }],
    })
}

#[test]
fn test_body_round_trips_all_12_waist2_envelopes_and_matches_schema_shapes() {
    let mut body = make_test_body();
    let actor = ActorId::new("per_alice");
    let place = PlaceId::new("lab:room-1");
    let audience = Audience::new([actor.clone()]);
    let mut headers = Waist2Headers::new("idem-01");
    headers
        .meta
        .insert("org.monoagens.lab".to_owned(), MetaValue::Bool(true));

    // 1. body/discover (unsorted input caps come back deterministically sorted)
    let discover_params = BodyDiscoverParams {
        headers: Waist2Headers::new("idem-discover"),
        body_id: None,
    };
    assert_matches_schema("BodyDiscoverParams", &discover_params);
    let discover_req = Waist2Request::new(
        Some(RpcId::Int(1)),
        Waist2Method::BodyDiscover,
        discover_params.clone(),
    );
    assert_matches_schema("Waist2Request", &discover_req);
    let discover_res: BodyDiscoverResult = body
        .call(RpcId::Int(1), Waist2Method::BodyDiscover, discover_params)
        .expect("codec")
        .expect("ok");
    assert_matches_schema("BodyDiscoverResult", &discover_res);
    assert_eq!(discover_res.capabilities[0].id.as_str(), "a.speak");
    assert_eq!(discover_res.capabilities[1].id.as_str(), "z.push");
    assert_eq!(discover_res.places[0].as_str(), "lab:device-1");

    // Byte-compare canonical Waist2Response serialization through codec (fresh key so cache doesn't re-encode Value)
    let expected_resp_bytes = serde_json::to_vec(&Waist2Response::ok(
        Some(RpcId::Int(1)),
        discover_res.clone(),
    ))
    .expect("serialize expected");
    let raw_resp = body.handle_json_rpc(
        &serde_json::to_string(&Waist2Request::new(
            Some(RpcId::Int(1)),
            Waist2Method::BodyDiscover,
            BodyDiscoverParams {
                headers: Waist2Headers::new("idem-discover-raw"),
                body_id: None,
            },
        ))
        .expect("serialize req"),
    );
    assert_eq!(raw_resp.as_bytes(), expected_resp_bytes.as_slice());

    // 2. body/heartbeat
    let hb_params = BodyHeartbeatParams {
        headers: Waist2Headers::new("idem-hb"),
        body_id: BodyId::new("body-test-1"),
        status: BodyStatus::Ready,
        timestamp_ms: Time::from_millis(500),
    };
    assert_matches_schema("BodyHeartbeatParams", &hb_params);
    let hb_res: BodyHeartbeatResult = body
        .call(RpcId::Int(2), Waist2Method::BodyHeartbeat, hb_params)
        .expect("codec")
        .expect("ok");
    assert_matches_schema("BodyHeartbeatResult", &hb_res);
    assert_eq!(hb_res.acknowledged_at_ms, Time::from_millis(500));

    // 3. percept/emit (also verify Waist2Call byte equivalence)
    let percept = Percept::minimal(
        actor.clone(),
        place.clone(),
        audience.clone(),
        Time::from_millis(600),
        OpaquePayload::from_bytes(b"hello".as_slice()),
    );
    let pe_params = PerceptEmitParams::from_percept(Waist2Headers::new("idem-percept"), percept);
    assert_matches_schema("PerceptEmitParams", &pe_params);
    let call_bytes = serde_json::to_vec(&Waist2Call::PerceptEmit(Box::new(pe_params.clone())))
        .expect("serialize Waist2Call");
    let req_val: Value = serde_json::to_value(Waist2Request::new(
        Some(RpcId::Int(3)),
        Waist2Method::PerceptEmit,
        pe_params.clone(),
    ))
    .expect("to_value");
    let call_val: Value = serde_json::from_slice(&call_bytes).expect("parse Waist2Call");
    assert_eq!(req_val["method"], call_val["method"]);
    assert_eq!(req_val["params"], call_val["params"]);
    let pe_res: PerceptEmitResult = body
        .call(RpcId::Int(3), Waist2Method::PerceptEmit, pe_params)
        .expect("codec")
        .expect("ok");
    assert_matches_schema("PerceptEmitResult", &pe_res);
    assert!(pe_res.accepted);

    // 4. action/execute (immediate Done) + duplicate idempotency_key check
    let exec_params = ActionExecuteParams {
        headers: Waist2Headers::new("idem-exec-1"),
        action_id: ActionId::new("act-1"),
        capability_id: CapabilityId::new("a.speak"),
        places: vec![place.clone()],
        audience: audience.clone(),
        payload: OpaquePayload::from_bytes(b"hi".as_slice()),
        timeout_ms: Some(1_000),
        h_act: Some(HAct::new("blake3:1111")),
    };
    assert_matches_schema("ActionExecuteParams", &exec_params);
    let exec_res: ActionExecuteResult = body
        .call(
            RpcId::Int(4),
            Waist2Method::ActionExecute,
            exec_params.clone(),
        )
        .expect("codec")
        .expect("ok");
    assert_matches_schema("ActionExecuteResult", &exec_res);
    assert!(exec_res.accepted);
    assert_eq!(
        exec_res.receipt.as_ref().map(|r| r.outcome),
        Some(ReceiptOutcome::Done)
    );
    assert_eq!(body.execution_count(&ActionId::new("act-1")), 1);

    // Replay same idempotency_key twice -> single execution!
    let replay_res: ActionExecuteResult = body
        .call(RpcId::Int(5), Waist2Method::ActionExecute, exec_params)
        .expect("codec")
        .expect("ok");
    assert_eq!(replay_res, exec_res);
    assert_eq!(body.execution_count(&ActionId::new("act-1")), 1);

    // Reuse same idempotency_key for a different action -> -32002 ERR_DUPLICATE_IDEMPOTENCY
    let conflict_params = ActionExecuteParams {
        headers: Waist2Headers::new("idem-exec-1"),
        action_id: ActionId::new("act-conflict"),
        capability_id: CapabilityId::new("a.speak"),
        places: vec![place.clone()],
        audience: audience.clone(),
        payload: OpaquePayload::from_bytes(b"different".as_slice()),
        timeout_ms: None,
        h_act: None,
    };
    let conflict_err = body
        .call::<_, ActionExecuteResult>(RpcId::Int(6), Waist2Method::ActionExecute, conflict_params)
        .expect("codec")
        .expect_err("must reject conflicting idempotency_key");
    assert_eq!(
        conflict_err.error_code(),
        Some(Waist2ErrorCode::DuplicateIdempotency)
    );

    // 5. state/query on settled action
    let sq_params = StateQueryParams {
        headers: Waist2Headers::new("idem-sq-1"),
        action_id: ActionId::new("act-1"),
    };
    assert_matches_schema("StateQueryParams", &sq_params);
    let sq_res: StateQueryResult = body
        .call(RpcId::Int(7), Waist2Method::StateQuery, sq_params)
        .expect("codec")
        .expect("ok");
    assert_matches_schema("StateQueryResult", &sq_res);
    assert!(matches!(sq_res.status, StateQueryStatus::Terminal { .. }));

    // 6. action/execute -> Held -> approval/resolve -> Done
    let held_exec = ActionExecuteParams {
        headers: Waist2Headers::new("idem-exec-held"),
        action_id: ActionId::new("act-held"),
        capability_id: CapabilityId::new("z.push"),
        places: vec![PlaceId::new("lab:device-1")],
        audience: audience.clone(),
        payload: OpaquePayload::from_bytes(b"hold please".as_slice()),
        timeout_ms: None,
        h_act: Some(HAct::new("blake3:hold")),
    };
    let held_res: ActionExecuteResult = body
        .call(RpcId::Int(8), Waist2Method::ActionExecute, held_exec)
        .expect("codec")
        .expect("ok");
    assert_eq!(
        held_res.receipt.as_ref().map(|r| r.outcome),
        Some(ReceiptOutcome::Held)
    );

    let approve_params = ApprovalResolveParams {
        headers: Waist2Headers::new("idem-approve"),
        action_id: ActionId::new("act-held"),
        h_act: HAct::new("blake3:hold"),
        state_epoch: 1,
        approved: true,
        resolver: actor.clone(),
        resolved_at_ms: Time::from_millis(700),
    };
    assert_matches_schema("ApprovalResolveParams", &approve_params);
    let approve_res: ApprovalResolveResult = body
        .call(RpcId::Int(9), Waist2Method::ApprovalResolve, approve_params)
        .expect("codec")
        .expect("ok");
    assert_matches_schema("ApprovalResolveResult", &approve_res);
    assert!(approve_res.applied);

    // 7. action/receipt & action/cancel
    let direct_receipt = Receipt {
        action_id: ActionId::new("act-direct"),
        capability_id: CapabilityId::new("a.speak"),
        outcome: ReceiptOutcome::Done,
        reason_code: None,
        detail: None,
        termination_reason: Some(TerminationReason::Normal),
        progress: Some(1.0),
        delivered_units: Some(1),
        h_act: None,
        timestamp_ms: Time::from_millis(750),
    };
    let rcpt_params = ActionReceiptParams {
        headers: Waist2Headers::new("idem-rcpt"),
        receipt: direct_receipt,
    };
    assert_matches_schema("ActionReceiptParams", &rcpt_params);
    let rcpt_res: ActionReceiptResult = body
        .call(RpcId::Int(10), Waist2Method::ActionReceipt, rcpt_params)
        .expect("codec")
        .expect("ok");
    assert_matches_schema("ActionReceiptResult", &rcpt_res);
    assert!(rcpt_res.recorded);

    let cancel_params = ActionCancelParams {
        headers: Waist2Headers::new("idem-cancel"),
        action_id: ActionId::new("act-1"),
        reason: Some("already_done".to_owned()),
    };
    assert_matches_schema("ActionCancelParams", &cancel_params);
    let cancel_res: ActionCancelResult = body
        .call(RpcId::Int(11), Waist2Method::ActionCancel, cancel_params)
        .expect("codec")
        .expect("ok");
    assert_matches_schema("ActionCancelResult", &cancel_res);
    assert!(!cancel_res.cancelled);

    // 8. question/answer
    let qa_params = QuestionAnswerParams {
        headers: Waist2Headers::new("idem-qa"),
        question_id: "q-1".to_owned(),
        actor,
        place,
        audience,
        answer: OpaquePayload::from_bytes(b"yes".as_slice()),
        timestamp_ms: Time::from_millis(800),
    };
    assert_matches_schema("QuestionAnswerParams", &qa_params);
    let qa_res: QuestionAnswerResult = body
        .call(RpcId::Int(12), Waist2Method::QuestionAnswer, qa_params)
        .expect("codec")
        .expect("ok");
    assert_matches_schema("QuestionAnswerResult", &qa_res);
    assert!(qa_res.accepted);

    // 9. blob/put & blob/get (valid + corrupt hash mismatch)
    let blob_bytes = b"attachment-payload-bytes".to_vec();
    let digest = sha256_hex(&blob_bytes);
    let put_params = BlobPutParams {
        headers: Waist2Headers::new("idem-blob-put"),
        sha256: digest.clone(),
        byte_len: blob_bytes.len() as u64,
        inline_bytes: Some(OpaquePayload::from_bytes(blob_bytes.clone())),
    };
    assert_matches_schema("BlobPutParams", &put_params);
    let put_res: BlobPutResult = body
        .call(RpcId::Int(13), Waist2Method::BlobPut, put_params)
        .expect("codec")
        .expect("ok");
    assert_matches_schema("BlobPutResult", &put_res);
    assert_eq!(put_res.sha256, digest);

    let get_params = BlobGetParams {
        headers: Waist2Headers::new("idem-blob-get"),
        blob_ref: put_res.blob_ref.clone(),
        expected_sha256: Some(digest.clone()),
    };
    assert_matches_schema("BlobGetParams", &get_params);
    let get_res: BlobGetResult = body
        .call(RpcId::Int(14), Waist2Method::BlobGet, get_params)
        .expect("codec")
        .expect("ok");
    assert_matches_schema("BlobGetResult", &get_res);
    assert_eq!(get_res.bytes.unwrap().as_bytes(), blob_bytes.as_slice());

    // Corrupt blob bytes -> -32005 ERR_HASH_MISMATCH
    let corrupt_put = BlobPutParams {
        headers: Waist2Headers::new("idem-blob-corrupt"),
        sha256: digest,
        byte_len: blob_bytes.len() as u64,
        inline_bytes: Some(OpaquePayload::from_bytes(
            b"corrupted-payload-bytes!".as_slice(),
        )),
    };
    let corrupt_err = body
        .call::<_, BlobPutResult>(RpcId::Int(15), Waist2Method::BlobPut, corrupt_put)
        .expect("codec")
        .expect_err("corrupt blob must fail");
    assert_eq!(
        corrupt_err.error_code(),
        Some(Waist2ErrorCode::HashMismatch)
    );

    // Unknown capability -> -32001, undeclared audience -> -32006, unsupported version -> -32004
    let bad_cap = ActionExecuteParams {
        headers: Waist2Headers::new("idem-bad-cap"),
        action_id: ActionId::new("act-bad-cap"),
        capability_id: CapabilityId::new("nonexistent.cap"),
        places: vec![PlaceId::new("lab:room-1")],
        audience: Audience::new([ActorId::new("per_alice")]),
        payload: OpaquePayload::default(),
        timeout_ms: None,
        h_act: None,
    };
    let cap_err = body
        .call::<_, ActionExecuteResult>(RpcId::Int(16), Waist2Method::ActionExecute, bad_cap)
        .expect("codec")
        .expect_err("unknown capability must fail");
    assert_eq!(
        cap_err.error_code(),
        Some(Waist2ErrorCode::UnknownCapability)
    );

    let mut future_headers = Waist2Headers::new("idem-bad-ver");
    future_headers.protocol_version = ProtocolVersion::new(2, 0);
    let ver_err = body
        .call::<_, BodyDiscoverResult>(
            RpcId::Int(17),
            Waist2Method::BodyDiscover,
            BodyDiscoverParams {
                headers: future_headers,
                body_id: None,
            },
        )
        .expect("codec")
        .expect_err("unsupported version must fail");
    assert_eq!(
        ver_err.error_code(),
        Some(Waist2ErrorCode::UnsupportedVersion)
    );

    let no_aud = PerceptEmitParams {
        headers: Waist2Headers::new("idem-no-aud"),
        actor: ActorId::new("per_alice"),
        origin_actor: None,
        place: PlaceId::new("lab:room-1"),
        audience: None,
        timestamp_ms: Time::from_millis(850),
        payload: OpaquePayload::from_bytes(b"secret".as_slice()),
        taint: monoagens_protocol::Taint::Untrusted,
        addressees: Vec::new(),
        named: Vec::new(),
        orient: Vec::new(),
        reply_to: None,
        correlation_id: None,
        channel: None,
        delta_obs: None,
        channel_prediction: None,
        length: None,
        negative_flag: None,
        repair: None,
        continued_attention: None,
        f_modal: None,
        embeddings: Vec::new(),
        attachments: Vec::new(),
        dialogue_act_gap: None,
    };
    let aud_err = body
        .call::<_, PerceptEmitResult>(RpcId::Int(18), Waist2Method::PerceptEmit, no_aud)
        .expect("codec")
        .expect_err("missing audience must fail");
    assert_eq!(
        aud_err.error_code(),
        Some(Waist2ErrorCode::UndeclaredAudience)
    );

    // 10. body/shutdown
    let sd_params = BodyShutdownParams {
        headers: Waist2Headers::new("idem-shutdown"),
        body_id: BodyId::new("body-test-1"),
        reason: Some("test_complete".to_owned()),
    };
    assert_matches_schema("BodyShutdownParams", &sd_params);
    let sd_res: BodyShutdownResult = body
        .call(RpcId::Int(19), Waist2Method::BodyShutdown, sd_params)
        .expect("codec")
        .expect("ok");
    assert_matches_schema("BodyShutdownResult", &sd_res);
    assert!(sd_res.accepted);
    assert!(body.is_shutdown());
}

#[test]
fn virtual_clock_runs_30_days_in_milliseconds_without_sleeping() {
    let thirty_days_ms: u64 = 30 * 24 * 3_600 * 1_000;
    let alice = ActorId::new("per_alice");
    let soul = ActorId::new("soul");
    let place = PlaceId::new("chat:room");
    let audience = Audience::new([alice.clone(), soul.clone()]);

    let mut p1 = Percept::minimal(
        alice.clone(),
        place.clone(),
        audience.clone(),
        Time::from_millis(1_000),
        OpaquePayload::from_bytes(b"day 0".as_slice()),
    );
    p1.named = vec![soul.clone()];

    let mut p2 = Percept::minimal(
        alice,
        place.clone(),
        audience,
        Time::from_millis(1_000 + thirty_days_ms),
        OpaquePayload::from_bytes(b"day 30".as_slice()),
    );
    p2.named = vec![soul];

    let scenario = Scenario {
        id: "thirty-day-gap".to_owned(),
        name: "Thirty days apart".to_owned(),
        set: ScenarioSet::Development,
        start_time_ms: 0,
        model_latency_ms: 100,
        model_id: "scripted".to_owned(),
        embedder_id: "none".to_owned(),
        bodies: vec![TestBodySpec {
            body_id: BodyId::new("chat-body"),
            places: vec![place.clone()],
            capabilities: vec![test_capability("chat.speak", vec![place.clone()])],
            action_scripts: Vec::new(),
        }],
        events: vec![
            ScriptedEvent {
                at_ms: 1_000,
                body_id: Some(BodyId::new("chat-body")),
                event: ScriptedEventKind::Percept(Box::new(p1)),
            },
            ScriptedEvent {
                at_ms: 1_000 + thirty_days_ms,
                body_id: Some(BodyId::new("chat-body")),
                event: ScriptedEventKind::Percept(Box::new(p2)),
            },
        ],
        scripted_model: Vec::new(),
        checks: vec![ExpectationCheck::PayloadContains {
            name: "day30_recorded".to_owned(),
            kind: CheckKind::Structural,
            place: Some(place),
            event_kind: Some(EventKind::Percept),
            text: "day 30".to_owned(),
            expected_present: true,
        }],
        core_config: None,
        model_path: None,
        online_adaptation: None,
        cost_guard: None,
        annotations: Default::default(),
    };

    let started = Instant::now();
    let out = run_scenario(MemStore::new(), CoreConfig::default(), &scenario, 42).expect("run ok");
    let elapsed = started.elapsed();

    assert!(
        elapsed.as_secs_f64() < 1.0,
        "30-day simulation took too long: {elapsed:?}"
    );
    assert!(out.run_record.passed);

    let percepts: Vec<_> = out
        .events
        .iter()
        .filter(|e| e.kind == EventKind::Percept)
        .collect();
    assert_eq!(percepts.len(), 2);
    assert_eq!(
        percepts[1]
            .timestamp_ms
            .saturating_sub_ms(percepts[0].timestamp_ms),
        thirty_days_ms
    );
}

#[test]
fn slow_scripted_model_lets_earlier_events_be_appraised_first_and_runs_waist2_device_loop() {
    let alice = ActorId::new("per_alice");
    let soul = ActorId::new("soul");
    let room = PlaceId::new("lab:device");
    let audience = Audience::new([alice.clone(), soul.clone()]);

    // Event 1 at t=1_000 triggers a wake; model latency is 5_000ms -> ModelResult lands at t=6_000.
    let mut p1 = Percept::minimal(
        alice.clone(),
        room.clone(),
        audience.clone(),
        Time::from_millis(1_000),
        OpaquePayload::from_bytes(b"please push the button".as_slice()),
    );
    p1.addressees = vec![soul.clone()];
    p1.named = vec![soul.clone()];

    // Event 2 at t=2_000 arrives BEFORE the slow model result at t=6_000 and must be appraised first.
    let p2 = Percept::minimal(
        ActorId::new("per_bob"),
        room.clone(),
        Audience::new([ActorId::new("per_bob"), soul.clone()]),
        Time::from_millis(2_000),
        OpaquePayload::from_bytes(b"background chatter".as_slice()),
    );

    let scenario = Scenario {
        id: "slow-model-ordering".to_owned(),
        name: "Slow model lets earlier event appraise first".to_owned(),
        set: ScenarioSet::Development,
        start_time_ms: 0,
        model_latency_ms: 5_000,
        model_id: "scripted-slow".to_owned(),
        embedder_id: "none".to_owned(),
        bodies: vec![TestBodySpec {
            body_id: BodyId::new("device-body"),
            places: vec![room.clone()],
            capabilities: vec![test_capability("device.push", vec![room.clone()])],
            action_scripts: vec![ScriptedActionRule {
                capability_id: CapabilityId::new("device.push"),
                place: Some(room.clone()),
                payload_contains: None,
                outcome: ReceiptOutcome::Done,
                receipt_delay_ms: 100,
                termination_reason: Some(TerminationReason::Normal),
                progress: Some(1.0),
                delivered_units: Some(1),
                reason_code: None,
                detail: Some("button pressed".to_owned()),
                follow_up_percepts: vec![ScriptedFollowUpPercept {
                    delay_ms: 250,
                    actor: ActorId::new("sensor-1"),
                    place: room.clone(),
                    audience: audience.clone(),
                    payload: b"sensor:pressed".to_vec(),
                    channel: Some(ChannelId::new("tactile")),
                    delta_obs: Some(1.0),
                    addressees: Vec::new(),
                    named: Vec::new(),
                    dialogue_act_gap: None,
                }],
            }],
        }],
        events: vec![
            ScriptedEvent {
                at_ms: 1_000,
                body_id: Some(BodyId::new("device-body")),
                event: ScriptedEventKind::Percept(Box::new(p1)),
            },
            ScriptedEvent {
                at_ms: 2_000,
                body_id: Some(BodyId::new("device-body")),
                event: ScriptedEventKind::Percept(Box::new(p2)),
            },
        ],
        scripted_model: vec![ScriptedModelResponse {
            wake_index: Some(0),
            acts: vec![ThoughtAct::Act {
                capability_id: CapabilityId::new("device.push"),
                places: vec![room.clone()],
                people: vec![alice],
                payload: OpaquePayload::from_bytes(b"press".as_slice()),
                motivation: 4,
                confidence_0_20: 18,
                used_ids: Vec::new(),
                expected: None,
                domain: None,
            }],
            tokens_in: 120,
            tokens_out: 30,
            latency_ms: None,
            fail_reason: None,
        }],
        checks: vec![
            ExpectationCheck::DecisionIs {
                name: "first_percept_woke".to_owned(),
                kind: CheckKind::Structural,
                target_event_id: None,
                decision_index: Some(0),
                expected_outcome: DecisionOutcome::Go,
                expected_reason: Some(DecisionReason::Commitment),
            },
            ExpectationCheck::ActionInPlace {
                name: "push_action_dispatched".to_owned(),
                kind: CheckKind::Structural,
                place: room.clone(),
                capability_id: Some(CapabilityId::new("device.push")),
                after_ms: 6_000,
                before_ms: 6_000,
                expected_present: true,
            },
            ExpectationCheck::ContextContains {
                name: "context_captured_room".to_owned(),
                kind: CheckKind::Structural,
                item_event_id: None,
                label: Some("lab:device".to_owned()),
                expected_present: true,
            },
            ExpectationCheck::ReceiptOutcomeIs {
                name: "push_receipt_done".to_owned(),
                kind: CheckKind::Structural,
                action_id: None,
                capability_id: Some(CapabilityId::new("device.push")),
                expected: ReceiptOutcome::Done,
            },
            ExpectationCheck::PayloadContains {
                name: "sensor_followup_recorded".to_owned(),
                kind: CheckKind::Structural,
                place: Some(room),
                event_kind: Some(EventKind::Percept),
                text: "sensor:pressed".to_owned(),
                expected_present: true,
            },
        ],
        core_config: None,
        model_path: None,
        online_adaptation: None,
        cost_guard: None,
        annotations: Default::default(),
    };

    let out = run_scenario(MemStore::new(), CoreConfig::default(), &scenario, 99).expect("run ok");
    assert!(out.run_record.passed, "checks: {:?}", out.run_record.checks);

    // Verify p2 decision at t=2_000 appears BEFORE p1's Thought/Action at t=6_000.
    let decisions: Vec<_> = out
        .events
        .iter()
        .filter(|e| e.kind == EventKind::Decision)
        .collect();
    let thoughts: Vec<_> = out
        .events
        .iter()
        .filter(|e| e.kind == EventKind::Thought)
        .collect();
    assert!(decisions.len() >= 2);
    assert_eq!(decisions[1].timestamp_ms, Time::from_millis(2_000));
    assert_eq!(thoughts[0].timestamp_ms, Time::from_millis(6_000));
    assert!(decisions[1].seq < thoughts[0].seq);
}

#[test]
fn deterministic_double_run_produces_identical_serialized_logs_and_postcard_round_trip() {
    let place = PlaceId::new("chat:det");
    let alice = ActorId::new("per_alice");
    let soul = ActorId::new("soul");
    let mut p = Percept::minimal(
        alice.clone(),
        place.clone(),
        Audience::new([alice, soul.clone()]),
        Time::from_millis(1_000),
        OpaquePayload::from_bytes(b"determinism test".as_slice()),
    );
    p.named = vec![soul];

    let scenario = Scenario {
        id: "det-double-run".to_owned(),
        name: "Deterministic double run".to_owned(),
        set: ScenarioSet::Development,
        start_time_ms: 0,
        model_latency_ms: 50,
        model_id: "scripted".to_owned(),
        embedder_id: "none".to_owned(),
        bodies: vec![TestBodySpec {
            body_id: BodyId::new("chat-body"),
            places: vec![place.clone()],
            capabilities: vec![test_capability("chat.speak", vec![place.clone()])],
            action_scripts: Vec::new(),
        }],
        events: vec![ScriptedEvent {
            at_ms: 1_000,
            body_id: Some(BodyId::new("chat-body")),
            event: ScriptedEventKind::Percept(Box::new(p)),
        }],
        scripted_model: vec![ScriptedModelResponse {
            wake_index: Some(0),
            acts: vec![ThoughtAct::Act {
                capability_id: CapabilityId::new("chat.speak"),
                places: vec![place],
                people: Vec::new(),
                payload: OpaquePayload::from_bytes(b"deterministic reply".as_slice()),
                motivation: 3,
                confidence_0_20: 15,
                used_ids: Vec::new(),
                expected: None,
                domain: None,
            }],
            tokens_in: 50,
            tokens_out: 10,
            latency_ms: None,
            fail_reason: None,
        }],
        checks: Vec::new(),
        core_config: None,
        model_path: None,
        online_adaptation: None,
        cost_guard: None,
        annotations: Default::default(),
    };

    let run1 = run_scenario(MemStore::new(), CoreConfig::default(), &scenario, 777).expect("run1");
    let run2 = run_scenario(MemStore::new(), CoreConfig::default(), &scenario, 777).expect("run2");

    let log1: Vec<Vec<u8>> = run1.events.iter().map(|e| e.encode_bytes()).collect();
    let log2: Vec<Vec<u8>> = run2.events.iter().map(|e| e.encode_bytes()).collect();
    assert_eq!(log1, log2, "serialized event logs must match byte-for-byte");

    // Verify SoulDir + SegmentedStore + postcard RunRecord persistence.
    let tmp = TempDir::new().expect("tempdir");
    let soul_dir = SoulDir::open_or_init(tmp.path(), "soul-det", 777).expect("soul_dir");
    let seg_store = soul_dir.open_store().expect("open segmented store");
    let seg_run = run_scenario(seg_store, CoreConfig::default(), &scenario, 777).expect("seg run");
    let log3: Vec<Vec<u8>> = seg_run.events.iter().map(|e| e.encode_bytes()).collect();
    assert_eq!(log1, log3);

    let saved_path = soul_dir
        .save_run_record(&seg_run.run_record)
        .expect("save postcard record");
    let loaded: RunRecord = SoulDir::load_run_record(&saved_path).expect("load postcard record");
    assert_eq!(loaded, seg_run.run_record);
}

#[test]
fn lab_cli_json_stdout_is_pure_json_and_errors_exit_nonzero_on_stderr() {
    let tmp = TempDir::new().expect("tempdir");
    let scenario_path = tmp.path().join("scenario.json");
    let place = PlaceId::new("chat:cli");
    let p = Percept::minimal(
        ActorId::new("per_alice"),
        place.clone(),
        Audience::new([ActorId::new("per_alice"), ActorId::new("soul")]),
        Time::from_millis(100),
        OpaquePayload::from_bytes(b"hello cli".as_slice()),
    );
    let scenario = Scenario {
        id: "cli-json-test".to_owned(),
        name: "CLI JSON test".to_owned(),
        set: ScenarioSet::Development,
        start_time_ms: 0,
        model_latency_ms: 10,
        model_id: "scripted".to_owned(),
        embedder_id: "none".to_owned(),
        bodies: Vec::new(),
        events: vec![ScriptedEvent {
            at_ms: 100,
            body_id: None,
            event: ScriptedEventKind::Percept(Box::new(p)),
        }],
        scripted_model: Vec::new(),
        checks: vec![ExpectationCheck::ActionInPlace {
            name: "silence_in_place".to_owned(),
            kind: CheckKind::Structural,
            place,
            capability_id: None,
            after_ms: 0,
            before_ms: 1_000,
            expected_present: false,
        }],
        core_config: None,
        model_path: None,
        online_adaptation: None,
        cost_guard: None,
        annotations: Default::default(),
    };
    fs::write(
        &scenario_path,
        serde_json::to_string_pretty(&scenario).expect("json"),
    )
    .expect("write scenario");

    let exe = env!("CARGO_BIN_EXE_monoagens-lab");
    let out = Command::new(exe)
        .args([
            "run",
            "--scenario",
            scenario_path.to_str().unwrap(),
            "--seed",
            "5",
            "--json",
        ])
        .output()
        .expect("run monoagens-lab");

    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).expect("utf8 stdout");
    let parsed: Value = serde_json::from_str(&stdout).expect("stdout must be pure JSON");
    assert_eq!(parsed["scenario_id"], "cli-json-test");
    assert_eq!(parsed["passed"], true);
    assert!(out.stderr.is_empty());

    // Missing scenario file -> non-zero exit with error on stderr, empty stdout.
    let err_out = Command::new(exe)
        .args(["run", "--scenario", "/nonexistent/missing.json", "--json"])
        .output()
        .expect("run monoagens-lab missing");
    assert!(!err_out.status.success());
    assert!(err_out.stdout.is_empty());
    assert!(!err_out.stderr.is_empty());
}

#[test]
fn sealed_set_redacts_checks_and_cost_guard_refuses_over_budget_wakes() {
    let place = PlaceId::new("lab:sealed");
    let alice = ActorId::new("per_alice");
    let soul = ActorId::new("soul");
    let mut p = Percept::minimal(
        alice.clone(),
        place.clone(),
        Audience::new([alice, soul.clone()]),
        Time::from_millis(100),
        OpaquePayload::from_bytes(b"secret sealed text".as_slice()),
    );
    p.addressees = vec![soul.clone()];
    p.named = vec![soul];

    let scenario = Scenario {
        id: "sealed-guard-test".to_owned(),
        name: "Sealed + CostGuard".to_owned(),
        set: ScenarioSet::Sealed,
        start_time_ms: 0,
        model_latency_ms: 10,
        model_id: "scripted".to_owned(),
        embedder_id: "none".to_owned(),
        bodies: Vec::new(),
        events: vec![ScriptedEvent {
            at_ms: 100,
            body_id: None,
            event: ScriptedEventKind::Percept(Box::new(p)),
        }],
        scripted_model: Vec::new(),
        checks: vec![ExpectationCheck::PayloadContains {
            name: "sealed_secret_check".to_owned(),
            kind: CheckKind::Structural,
            place: Some(place),
            event_kind: Some(EventKind::Percept),
            text: "secret sealed text".to_owned(),
            expected_present: true,
        }],
        core_config: None,
        model_path: None,
        online_adaptation: None,
        cost_guard: Some(monoagens_lab::CostGuardConfig {
            max_tokens: Some(0),
            max_chain_wakes: None,
        }),
        annotations: Default::default(),
    };

    let out = run_scenario(MemStore::new(), CoreConfig::default(), &scenario, 11).expect("run");
    assert_eq!(out.guard_refusals.len(), 1);
    assert!(out.run_record.sealed);

    let json = out.run_record.to_machine_json();
    let rendered = serde_json::to_string(&json).expect("serialize");
    assert!(!rendered.contains("sealed_secret_check"));
    assert!(!rendered.contains("secret sealed text"));
    assert_eq!(json["aggregate"]["total_checks"], 1);
}
