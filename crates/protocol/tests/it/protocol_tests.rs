use std::collections::BTreeMap;

use monoagens_protocol::{
    Action, ActionCancelParams, ActionCancelResult, ActionCanonicalInput, ActionExecuteParams,
    ActionExecuteResult, ActionId, ActionReceiptParams, ActionReceiptResult, ActorFacts, ActorId,
    ApprovalResolveParams, ApprovalResolveResult, AttachmentHandle, Audience, BlobGetParams,
    BlobGetResult, BlobPutParams, BlobPutResult, BodyDiscoverParams, BodyDiscoverResult,
    BodyHeartbeatParams, BodyHeartbeatResult, BodyId, BodyShutdownParams, BodyShutdownResult,
    BodyState, BodyStatus, CapabilityDescriptor, CapabilityFlags, CapabilityId,
    CapabilitySchemaHash, CatchupPolicy, ChannelId, ChannelPrediction, ClassificationVerdict,
    ConsolidateOutput, CorrelationId, DeclassToken, DomainFactKind, DomainId, EmbeddingTag,
    EventId, ExpectKind, FModal, HAct, MetaValue, ModalitySlot, OpaquePayload, OrientScore,
    OriginActor, OwnerSigned, PendingApproval, Percept, PerceptEmitParams, PerceptEmitResult,
    PlaceId, PrincipalLink, PrincipalLinkStatus, PromiseMeta, ProtocolError, ProtocolVersion,
    QuestionAnswerParams, QuestionAnswerResult, Receipt, ReceiptOutcome, RecurrenceSchedule,
    RecurrenceSpec, ResolveOutcome, RpcId, SensedEffect, StateQueryParams, StateQueryResult,
    StateQueryStatus, TaggedEmbedding, Taint, TerminationReason, ThoughtAct, ThoughtBatch, Time,
    VisibleScope, Waist2Call, Waist2Error, Waist2ErrorCode, Waist2Headers, Waist2Method,
    Waist2Request, Waist2Response, sort_capabilities,
};
use serde::{Deserialize, Serialize};
use serde_json::json;

fn round_trip<T>(value: &T) -> T
where
    T: Serialize + for<'de> Deserialize<'de> + PartialEq + std::fmt::Debug,
{
    let serialized = serde_json::to_string(value).expect("serialize");
    let deserialized: T = serde_json::from_str(&serialized).expect("deserialize");
    assert_eq!(value, &deserialized);
    deserialized
}

#[test]
fn all_waist1_messages_round_trip() {
    let alice = ActorId::new("per_alice");
    let bob = ActorId::new("per_bob");
    let place = PlaceId::new("chat:room-1");
    let audience = Audience::new([alice.clone(), bob.clone()]);

    let mut percept = Percept::minimal(
        alice.clone(),
        place.clone(),
        audience.clone(),
        Time::from_millis(1_700_000_000_000),
        OpaquePayload::from_bytes(b"hello world".as_slice()),
    );
    percept.addressees = vec![bob.clone()];
    percept.named = vec![bob.clone()];
    percept.orient = vec![OrientScore {
        actor: bob.clone(),
        score: 0.85,
    }];
    percept.reply_to = Some(EventId::new(42));
    percept.correlation_id = Some(CorrelationId::new("corr-99"));
    percept.channel = Some(ChannelId::new("audio"));
    percept.delta_obs = Some(0.5);
    percept.channel_prediction = Some(ChannelPrediction {
        channel: ChannelId::new("door-sensor"),
        delta_obs: Some(1.0),
        delta_pred: Some(1.0),
    });
    percept.length = Some(11);
    percept.negative_flag = Some(false);
    percept.repair = Some(false);
    percept.continued_attention = Some(true);
    percept.f_modal = Some(FModal([0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0]));
    percept.embeddings = vec![TaggedEmbedding {
        tag: EmbeddingTag {
            embed_model_ver: "embeddinggemma-2".to_owned(),
            dim_d: 256,
            prompt_regime_id: "t_q".to_owned(),
        },
        values: vec![0.25, -0.75],
    }];
    percept.attachments = vec![AttachmentHandle {
        handle_id: "att_01".to_owned(),
        modality_slot: ModalitySlot::Image,
        byte_len: 4096,
        expires_at_ms: Time::from_millis(1_700_000_060_000),
        embed_model_ver: "embeddinggemma-2".to_owned(),
    }];
    percept.dialogue_act_gap = Some(0.91);
    round_trip(&percept);

    let action = Action {
        protocol_version: ProtocolVersion::new(1, 0),
        action_id: ActionId::new("act-1"),
        capability_id: CapabilityId::new("chat.speak"),
        places: vec![place.clone()],
        audience: audience.clone(),
        origin_actor: OriginActor::new(alice.clone()),
        taint: Taint::Trusted,
        payload: OpaquePayload::from_bytes(b"reply payload".as_slice()),
        correlation_id: Some(CorrelationId::new("corr-1")),
        expected_effect: Some(SensedEffect {
            channel: ChannelId::new("door"),
            window_ms: 2000,
            noise: 0.05,
            delta_pred: 1.0,
        }),
        h_act: Some(HAct::new("blake3:deadbeef")),
        timestamp_ms: Time::from_millis(1_700_000_000_100),
    };
    round_trip(&action);

    let body_state = BodyState {
        protocol_version: ProtocolVersion::new(1, 0),
        body_id: BodyId::new("body-cli"),
        status: BodyStatus::Ready,
        capabilities: vec![CapabilityDescriptor {
            id: CapabilityId::new("chat.speak"),
            description: "Send a chat message".to_owned(),
            places: vec![place.clone()],
            flags: CapabilityFlags {
                communicative: true,
                identity_changing: false,
                irreversible: false,
            },
            typical_latency_ms: 120,
            capability_schema_hash: CapabilitySchemaHash::new("sha256:abcd"),
            sensed_effect: None,
        }],
        timestamp_ms: Time::from_millis(1_700_000_000_200),
    };
    round_trip(&body_state);

    let actor_facts = ActorFacts {
        protocol_version: ProtocolVersion::new(1, 0),
        actor_id: alice.clone(),
        alpha_0: 2.0,
        beta_0: 1.0,
        teacher_weight: 1.0,
        trusted_origin: true,
    };
    round_trip(&actor_facts);

    let owner_signed = OwnerSigned {
        payload: "soul-anchor-v1".to_owned(),
        signer: alice.clone(),
        signed_at_ms: Time::from_millis(1_000),
        signature: "ed25519:sig".to_owned(),
    };
    round_trip(&owner_signed);

    let principal_link = PrincipalLink {
        body_id: BodyId::new("matrix"),
        platform_actor: ActorId::new("@alice:matrix.org"),
        canonical_actor: alice.clone(),
        status: PrincipalLinkStatus::Linked,
        updated_at_ms: Time::from_millis(5_000),
        challenge_hash: Some("blake3:challenge".to_owned()),
    };
    round_trip(&principal_link);

    let pending_approval = PendingApproval {
        action_id: ActionId::new("act-2"),
        capability_id: CapabilityId::new("fs.write"),
        h_act: HAct::new("blake3:cafebabe"),
        state_epoch: 7,
        requested_at_ms: Time::from_millis(10_000),
        expires_at_ms: Time::from_millis(910_000),
        places: vec![place],
        reason: Some("PendingApproval".to_owned()),
    };
    round_trip(&pending_approval);
}

#[test]
fn thought_acts_and_strict_nullable_schemas_round_trip() {
    let place = PlaceId::new("cli:main");
    let alice = ActorId::new("per_alice");

    let acts = vec![
        ThoughtAct::Think {
            thought: "check context".to_owned(),
            motivation: 3,
            confidence_0_20: 14,
            used_ids: vec![EventId::new(1)],
            importance: None,
            basis: Some("observed".to_owned()),
            value: None,
            cost: None,
            domain: None,
        },
        ThoughtAct::Act {
            capability_id: CapabilityId::new("chat.speak"),
            places: vec![place.clone()],
            people: vec![alice.clone()],
            payload: OpaquePayload::from_bytes(b"hi".as_slice()),
            motivation: 4,
            confidence_0_20: 16,
            used_ids: vec![EventId::new(1)],
            expected: None,
            domain: Some(DomainId::new("general")),
        },
        ThoughtAct::Expect {
            expect_kind: ExpectKind::Monitor,
            about_ids: vec![EventId::new(2)],
            places: vec![place.clone()],
            people: vec![alice.clone()],
            condition: "check mailbox".to_owned(),
            due_ms: Some(Time::from_millis(60_000)),
            expiry_ms: Time::from_millis(120_000),
            recurrence: Some(RecurrenceSpec {
                schedule: RecurrenceSchedule::IntervalMs(300_000),
                catchup: CatchupPolicy::CoalesceOne,
            }),
            promise_meta: Some(PromiseMeta {
                promisee_ids: vec![alice.clone()],
                hard_deadline_ms: Some(Time::from_millis(120_000)),
                conflict_key: Some("mail:check".to_owned()),
                depends_on: Some(EventId::new(1)),
            }),
            domain: None,
        },
        ThoughtAct::Resolve {
            expectation_id: EventId::new(3),
            outcome: ResolveOutcome::KeptLate,
            note: Some("answered after retry".to_owned()),
            places: vec![place.clone()],
            people: vec![alice.clone()],
            domain: None,
        },
        ThoughtAct::Revise {
            target_id: EventId::new(3),
            new_version: 2,
            condition: Some("updated condition".to_owned()),
            due_ms: None,
            expiry_ms: Some(Time::from_millis(200_000)),
            places: vec![place.clone()],
            people: vec![alice.clone()],
            domain: None,
        },
        ThoughtAct::Consolidate {
            output: ConsolidateOutput::Gist {
                summary: "short summary".to_owned(),
                about_ids: vec![EventId::new(1)],
                places: vec![place.clone()],
                people: vec![alice.clone()],
                domain: None,
            },
        },
        ThoughtAct::Consolidate {
            output: ConsolidateOutput::Lesson {
                pattern: "ask before deleting".to_owned(),
                failure_signature: "refused:irreversible".to_owned(),
                contrastive_success_ids: vec![EventId::new(10)],
                failure_ids: vec![EventId::new(9)],
                domain: Some(DomainId::new("safety")),
            },
        },
        ThoughtAct::Consolidate {
            output: ConsolidateOutput::Procedure {
                name: "safe_deploy".to_owned(),
                steps: vec!["test".to_owned(), "deploy".to_owned()],
                required_cap_ids: vec![CapabilityId::new("shell.run")],
                capability_schema_hash: CapabilitySchemaHash::new("sha256:proc"),
                domain: None,
            },
        },
    ];

    let batch = ThoughtBatch {
        protocol_version: ProtocolVersion::new(1, 0),
        acts,
    };
    batch
        .validate(ProtocolVersion::new(1, 0))
        .expect("valid batch");
    let json_val = serde_json::to_value(&batch).expect("to value");
    // Strict nullable schema emits explicit null for None fields.
    assert!(
        json_val["acts"][0]
            .get("domain")
            .is_some_and(serde_json::Value::is_null)
    );
    round_trip(&batch);

    // Check every resolve outcome variant round-trips.
    for outcome in [
        ResolveOutcome::Kept,
        ResolveOutcome::KeptLate,
        ResolveOutcome::Renegotiated,
        ResolveOutcome::Released,
        ResolveOutcome::Failed,
        ResolveOutcome::Cancelled,
        ResolveOutcome::Answered,
        ResolveOutcome::Reached,
    ] {
        round_trip(&outcome);
    }

    let verdict = ClassificationVerdict {
        protocol_version: ProtocolVersion::new(1, 0),
        label: "missed_silence".to_owned(),
        score: Some(0.88),
        rationale: None,
    };
    round_trip(&verdict);
}

#[test]
fn waist2_all_twelve_methods_round_trip() {
    let mut meta = BTreeMap::new();
    meta.insert(
        "org.monoagens.vendor".to_owned(),
        MetaValue::String("test-body".to_owned()),
    );
    let mut headers = Waist2Headers::new("idem-001");
    headers.traceparent =
        Some("00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01".to_owned());
    headers.meta = meta;

    let actor = ActorId::new("per_alice");
    let place = PlaceId::new("cli:0");
    let audience = Audience::new([actor.clone()]);
    let cap_id = CapabilityId::new("chat.speak");
    let action_id = ActionId::new("act-42");
    let body_id = BodyId::new("body-cli");

    let receipt = Receipt {
        action_id: action_id.clone(),
        capability_id: cap_id.clone(),
        outcome: ReceiptOutcome::Done,
        reason_code: None,
        detail: None,
        termination_reason: Some(TerminationReason::Normal),
        progress: Some(1.0),
        delivered_units: Some(3),
        h_act: None,
        timestamp_ms: Time::from_millis(500),
    };

    let calls = vec![
        Waist2Call::BodyDiscover(BodyDiscoverParams {
            headers: headers.clone(),
            body_id: Some(body_id.clone()),
        }),
        Waist2Call::BodyHeartbeat(BodyHeartbeatParams {
            headers: headers.clone(),
            body_id: body_id.clone(),
            status: BodyStatus::Ready,
            timestamp_ms: Time::from_millis(100),
        }),
        Waist2Call::BodyShutdown(BodyShutdownParams {
            headers: headers.clone(),
            body_id: body_id.clone(),
            reason: Some("graceful".to_owned()),
        }),
        Waist2Call::PerceptEmit(Box::new(PerceptEmitParams::from_percept(
            headers.clone(),
            Percept::minimal(
                actor.clone(),
                place.clone(),
                audience.clone(),
                Time::from_millis(200),
                OpaquePayload::from_bytes(b"hello".as_slice()),
            ),
        ))),
        Waist2Call::ActionExecute(ActionExecuteParams {
            headers: headers.clone(),
            action_id: action_id.clone(),
            capability_id: cap_id,
            places: vec![place.clone()],
            audience: audience.clone(),
            payload: OpaquePayload::from_bytes(b"say hi".as_slice()),
            timeout_ms: Some(5_000),
            h_act: Some(HAct::new("blake3:1234")),
        }),
        Waist2Call::ActionCancel(ActionCancelParams {
            headers: headers.clone(),
            action_id: action_id.clone(),
            reason: Some("barge_in".to_owned()),
        }),
        Waist2Call::ActionReceipt(ActionReceiptParams {
            headers: headers.clone(),
            receipt: receipt.clone(),
        }),
        Waist2Call::ApprovalResolve(ApprovalResolveParams {
            headers: headers.clone(),
            action_id: action_id.clone(),
            h_act: HAct::new("blake3:1234"),
            state_epoch: 1,
            approved: true,
            resolver: actor.clone(),
            resolved_at_ms: Time::from_millis(300),
        }),
        Waist2Call::QuestionAnswer(QuestionAnswerParams {
            headers: headers.clone(),
            question_id: "q-1".to_owned(),
            actor,
            place: place.clone(),
            audience,
            answer: OpaquePayload::from_bytes(b"yes".as_slice()),
            timestamp_ms: Time::from_millis(400),
        }),
        Waist2Call::StateQuery(StateQueryParams {
            headers: headers.clone(),
            action_id: action_id.clone(),
        }),
        Waist2Call::BlobPut(BlobPutParams {
            headers: headers.clone(),
            sha256: "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855".to_owned(),
            byte_len: 0,
            inline_bytes: Some(OpaquePayload::default()),
        }),
        Waist2Call::BlobGet(BlobGetParams {
            headers,
            blob_ref: "blob://1".to_owned(),
            expected_sha256: None,
        }),
    ];

    assert_eq!(calls.len(), 12);
    for call in &calls {
        round_trip(call);
    }

    // Typed request/response wrappers round-trip cleanly too.
    let req = Waist2Request::new(
        Some(RpcId::Int(1)),
        Waist2Method::BodyDiscover,
        BodyDiscoverParams {
            headers: Waist2Headers::new("idem-discover"),
            body_id: Some(body_id.clone()),
        },
    );
    round_trip(&req);

    round_trip(&Waist2Response::ok(
        Some(RpcId::Int(1)),
        BodyDiscoverResult {
            body_id,
            protocol_version: ProtocolVersion::new(1, 0),
            capabilities: Vec::new(),
            places: vec![place],
        },
    ));
    round_trip(&BodyHeartbeatResult {
        acknowledged_at_ms: Time::from_millis(100),
    });
    round_trip(&BodyShutdownResult { accepted: true });
    round_trip(&PerceptEmitResult { accepted: true });
    round_trip(&ActionExecuteResult {
        accepted: true,
        receipt: Some(receipt.clone()),
    });
    round_trip(&ActionCancelResult {
        cancelled: true,
        receipt: Some(receipt.clone()),
    });
    round_trip(&ActionReceiptResult { recorded: true });
    round_trip(&ApprovalResolveResult { applied: true });
    round_trip(&QuestionAnswerResult { accepted: true });
    round_trip(&StateQueryResult {
        action_id,
        status: StateQueryStatus::Terminal { receipt },
    });
    round_trip(&BlobPutResult {
        blob_ref: "blob://1".to_owned(),
        sha256: "abc".to_owned(),
        expires_at_ms: Time::from_millis(60_000),
    });
    round_trip(&BlobGetResult {
        blob_ref: "blob://1".to_owned(),
        sha256: "abc".to_owned(),
        byte_len: 3,
        bytes: Some(OpaquePayload::from_bytes(b"xyz".as_slice())),
    });
}

#[test]
fn unknown_fields_are_ignored() {
    let percept_json = json!({
        "protocol_version": { "major": 1, "minor": 0, "future_patch": 9 },
        "actor": "per_alice",
        "place": "cli:0",
        "audience": ["per_alice"],
        "timestamp_ms": 1234,
        "payload": [104, 105],
        "unknown_extension_field": { "nested": [1, 2, 3] }
    });
    let percept: Percept = serde_json::from_value(percept_json).expect("ignores unknown fields");
    percept
        .validate(ProtocolVersion::new(1, 1))
        .expect("valid percept");

    let rpc_json = json!({
        "jsonrpc": "2.0",
        "id": "req-1",
        "method": "body/heartbeat",
        "params": {
            "headers": {
                "idempotency_key": "k-1",
                "protocol_version": { "major": 1, "minor": 0 },
                "_meta": {
                    "com.example.unknown": { "x": 1 }
                },
                "future_header_field": true
            },
            "body_id": "body-1",
            "status": "ready",
            "timestamp_ms": 999,
            "future_param_field": "ignored"
        }
    });
    let call: Waist2Call = serde_json::from_value(rpc_json).expect("ignores unknown RPC fields");
    assert_eq!(call.method(), Waist2Method::BodyHeartbeat);
}

#[test]
fn newer_version_rejected_and_older_minor_accepted() {
    let supported = ProtocolVersion::new(1, 1);

    // Older minor 1.0 accepted by 1.1 reader.
    let older_headers = Waist2Headers {
        idempotency_key: "k-old".to_owned(),
        protocol_version: ProtocolVersion::new(1, 0),
        traceparent: None,
        meta: BTreeMap::new(),
    };
    assert!(older_headers.validate_version(supported).is_ok());

    // Same minor 1.1 accepted by 1.1 reader.
    let equal_headers = Waist2Headers {
        idempotency_key: "k-eq".to_owned(),
        protocol_version: ProtocolVersion::new(1, 1),
        traceparent: None,
        meta: BTreeMap::new(),
    };
    assert!(equal_headers.validate_version(supported).is_ok());

    // Newer minor 1.2 rejected by 1.1 reader, naming both versions and returning -32004.
    let newer_minor = Waist2Headers {
        idempotency_key: "k-new-minor".to_owned(),
        protocol_version: ProtocolVersion::new(1, 2),
        traceparent: None,
        meta: BTreeMap::new(),
    };
    let err = newer_minor
        .validate_version(supported)
        .expect_err("newer minor must be rejected");
    assert_eq!(err.error_code(), Some(Waist2ErrorCode::UnsupportedVersion));
    assert_eq!(err.code, -32004);
    assert!(
        err.message.contains("1.2"),
        "missing received version: {}",
        err.message
    );
    assert!(
        err.message.contains("1.1"),
        "missing supported version: {}",
        err.message
    );

    // Different major 2.0 rejected by 1.1 reader.
    let percept_v2 = Percept {
        protocol_version: ProtocolVersion::new(2, 0),
        ..Percept::minimal(
            ActorId::new("a"),
            PlaceId::new("p"),
            Audience::new([ActorId::new("a")]),
            Time::from_millis(1),
            OpaquePayload::default(),
        )
    };
    let proto_err = percept_v2
        .validate(supported)
        .expect_err("major mismatch must be rejected");
    let msg = proto_err.to_string();
    assert!(
        msg.contains("2.0") && msg.contains("1.1"),
        "unexpected error: {msg}"
    );
}

#[test]
fn missing_or_empty_audience_rejected_with_typed_error() {
    let percept_empty_aud = Percept::minimal(
        ActorId::new("a"),
        PlaceId::new("p"),
        Audience::empty(),
        Time::from_millis(10),
        OpaquePayload::from_bytes(b"secret".as_slice()),
    );
    let err = percept_empty_aud
        .validate(ProtocolVersion::new(1, 0))
        .expect_err("empty audience must be rejected");
    assert!(matches!(
        err,
        ProtocolError::MissingDomainFact {
            fact: DomainFactKind::Audience,
            ..
        }
    ));
    assert!(err.to_string().contains("audience"));

    // Waist 2 percept/emit with omitted "audience" key produces -32006 ERR_UNDECLARED_AUDIENCE.
    let raw = json!({
        "headers": {
            "idempotency_key": "k-no-aud",
            "protocol_version": { "major": 1, "minor": 0 }
        },
        "actor": "per_alice",
        "place": "cli:0",
        "timestamp_ms": 100,
        "payload": [1, 2, 3]
    });
    let params: PerceptEmitParams = serde_json::from_value(raw).expect("parse params");
    let w2_err = params
        .into_percept(ProtocolVersion::new(1, 0))
        .expect_err("missing audience must return -32006");
    assert_eq!(w2_err.code, -32006);
    assert_eq!(
        w2_err.error_code(),
        Some(Waist2ErrorCode::UndeclaredAudience)
    );
    assert!(w2_err.message.contains("ERR_UNDECLARED_AUDIENCE"));
}

#[test]
fn receipt_option_a_serde_round_trip_and_bounds() {
    let outcomes = [
        ReceiptOutcome::Done,
        ReceiptOutcome::Refused,
        ReceiptOutcome::Held,
        ReceiptOutcome::Failed,
        ReceiptOutcome::Unknown,
    ];
    let reasons = [
        None,
        Some(TerminationReason::Normal),
        Some(TerminationReason::CanceledByCore),
        Some(TerminationReason::PreemptedExternal),
        Some(TerminationReason::TimedOut),
    ];

    for (idx, outcome) in outcomes.into_iter().enumerate() {
        let receipt = Receipt {
            action_id: ActionId::new(format!("act-{idx}")),
            capability_id: CapabilityId::new("tts.speak"),
            outcome,
            reason_code: Some("preempted".to_owned()),
            detail: Some("user barge-in".to_owned()),
            termination_reason: reasons[idx % reasons.len()],
            progress: Some(0.42),
            delivered_units: Some(17),
            h_act: Some(HAct::new("blake3:9999")),
            timestamp_ms: Time::from_millis(50_000),
        };
        receipt.validate().expect("valid receipt");
        round_trip(&receipt);
    }

    let bad_receipt = Receipt {
        action_id: ActionId::new("bad"),
        capability_id: CapabilityId::new("tts.speak"),
        outcome: ReceiptOutcome::Failed,
        reason_code: None,
        detail: None,
        termination_reason: Some(TerminationReason::PreemptedExternal),
        progress: Some(1.5),
        delivered_units: None,
        h_act: None,
        timestamp_ms: Time::from_millis(1),
    };
    assert!(bad_receipt.validate().is_err());
}

#[test]
fn opaque_payload_round_trips_byte_for_byte() {
    let arbitrary_bytes: Vec<u8> = (0u8..=255u8)
        .chain([0x00, 0xFF, 0xFE, 0x80, 0xC3, 0x28])
        .collect();
    let percept = Percept::minimal(
        ActorId::new("a"),
        PlaceId::new("p"),
        Audience::new([ActorId::new("a")]),
        Time::from_millis(1),
        OpaquePayload::from_bytes(arbitrary_bytes.clone()),
    );
    let restored = round_trip(&percept);
    assert_eq!(restored.payload.as_bytes(), arbitrary_bytes.as_slice());
}

#[test]
fn capability_descriptors_sort_deterministically() {
    let mut caps = vec![
        CapabilityDescriptor {
            id: CapabilityId::new("z.tool"),
            description: "Z".to_owned(),
            places: vec![PlaceId::new("p2"), PlaceId::new("p1"), PlaceId::new("p1")],
            flags: CapabilityFlags::default(),
            typical_latency_ms: 10,
            capability_schema_hash: CapabilitySchemaHash::new("h2"),
            sensed_effect: None,
        },
        CapabilityDescriptor {
            id: CapabilityId::new("a.tool"),
            description: "A".to_owned(),
            places: vec![PlaceId::new("p3"), PlaceId::new("p1")],
            flags: CapabilityFlags {
                communicative: true,
                identity_changing: false,
                irreversible: false,
            },
            typical_latency_ms: 5,
            capability_schema_hash: CapabilitySchemaHash::new("h1"),
            sensed_effect: Some(SensedEffect {
                channel: ChannelId::new("door"),
                window_ms: 2000,
                noise: 0.1,
                delta_pred: 1.0,
            }),
        },
    ];
    sort_capabilities(&mut caps);
    assert_eq!(caps[0].id.as_str(), "a.tool");
    assert_eq!(caps[0].places, vec![PlaceId::new("p1"), PlaceId::new("p3")]);
    assert_eq!(caps[1].id.as_str(), "z.tool");
    assert_eq!(caps[1].places, vec![PlaceId::new("p1"), PlaceId::new("p2")]);
}

#[test]
fn declass_token_preserves_fields_and_visible_scope_enforces_rules() {
    let alice = ActorId::new("per_alice");
    let bob = ActorId::new("per_bob");
    let target_place = PlaceId::new("group:public");
    let tok = DeclassToken {
        token_id: "declass-1".to_owned(),
        issuer: alice.clone(),
        scope: vec![EventId::new(10), EventId::new(11)],
        expiry: Time::from_millis(1_000),
        target_place: Some(target_place.clone()),
        target_audience: Audience::new([alice.clone(), bob.clone()]),
        signature: "ed25519:toksig".to_owned(),
    };
    let restored = round_trip(&tok);
    assert_eq!(restored.issuer, alice);
    assert_eq!(restored.scope, vec![EventId::new(10), EventId::new(11)]);
    assert_eq!(restored.expiry, Time::from_millis(1_000));
    assert_eq!(restored.target_place, Some(target_place.clone()));
    assert_eq!(
        restored.target_audience,
        Audience::new([alice.clone(), bob.clone()])
    );

    let mut scope = VisibleScope::new(Audience::new([alice.clone(), bob]), Some(target_place));
    let private_aud = Audience::new([alice]);
    // Without token: {alice, bob} is NOT a subset of {alice}.
    assert!(!scope.allows(&private_aud, Some(EventId::new(10)), Time::from_millis(500)));

    // With token before expiry: authorized for event 10, not event 99.
    scope.declass_tokens.push(restored);
    assert!(scope.allows(&private_aud, Some(EventId::new(10)), Time::from_millis(500)));
    assert!(!scope.allows(&private_aud, Some(EventId::new(99)), Time::from_millis(500)));
    // Expired token no longer authorizes.
    assert!(!scope.allows(
        &private_aud,
        Some(EventId::new(10)),
        Time::from_millis(1_001)
    ));
}

#[test]
fn h_act_canonical_form_is_stable() {
    let input_a = ActionCanonicalInput {
        capability_id: CapabilityId::new("fs.write"),
        actor: ActorId::new("per_owner"),
        places: vec![PlaceId::new("b"), PlaceId::new("a"), PlaceId::new("a")],
        state_epoch: 42,
        payload: b"{\"path\":\"/tmp/x\"}".to_vec(),
    };
    let input_b = ActionCanonicalInput {
        capability_id: CapabilityId::new("fs.write"),
        actor: ActorId::new("per_owner"),
        places: vec![PlaceId::new("a"), PlaceId::new("b")],
        state_epoch: 42,
        payload: b"{\"path\":\"/tmp/x\"}".to_vec(),
    };
    let input_c = ActionCanonicalInput {
        state_epoch: 43,
        ..input_b.clone()
    };

    let bytes_a = input_a.canonical_preimage();
    let bytes_b = input_b.canonical_preimage();
    let bytes_c = input_c.canonical_preimage();

    assert!(bytes_a.starts_with(b"monoagens.act.v1"));
    assert_eq!(bytes_a, bytes_b);
    assert_ne!(bytes_a, bytes_c);
    round_trip(&input_a);
    round_trip(&HAct::new("blake3:0123456789abcdef"));
}

#[test]
fn waist2_typed_error_codes_and_taint_monotonicity() {
    let errs = [
        (
            Waist2Error::unknown_capability(&CapabilityId::new("nope")),
            -32001,
            Waist2ErrorCode::UnknownCapability,
        ),
        (
            Waist2Error::duplicate_idempotency("dup-1"),
            -32002,
            Waist2ErrorCode::DuplicateIdempotency,
        ),
        (
            Waist2Error::invalid_receipt_transition("done -> held"),
            -32003,
            Waist2ErrorCode::InvalidReceiptTransition,
        ),
        (
            Waist2Error::unsupported_version(
                ProtocolVersion::new(1, 9),
                ProtocolVersion::new(1, 0),
            ),
            -32004,
            Waist2ErrorCode::UnsupportedVersion,
        ),
        (
            Waist2Error::hash_mismatch("aaa", "bbb"),
            -32005,
            Waist2ErrorCode::HashMismatch,
        ),
        (
            Waist2Error::undeclared_audience(),
            -32006,
            Waist2ErrorCode::UndeclaredAudience,
        ),
    ];

    for (err, expected_code, expected_variant) in errs {
        assert_eq!(err.code, expected_code);
        assert_eq!(err.error_code(), Some(expected_variant));
        round_trip(&err);
    }

    assert!(Taint::Untrusted < Taint::Provisional);
    assert!(Taint::Provisional < Taint::Trusted);
    assert_eq!(Taint::Trusted.combine(Taint::Untrusted), Taint::Untrusted);
    assert_eq!(
        Taint::join_min([Taint::Trusted, Taint::Provisional, Taint::Trusted]),
        Taint::Provisional
    );
}
