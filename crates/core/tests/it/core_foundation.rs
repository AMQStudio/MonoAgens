use std::collections::BTreeSet;

use monoagens_core::{
    CLASS_A_CAPACITY, CLASS_B_CAPACITY, CLASS_C_CAPACITY, ContextPlan, Core, CoreConfig, CoreError,
    DoorDecision, Doors, Effect, EventFormatError, EventKind, EventRecord, EventRecordV1, Input,
    LogSeq, MemStore, NoopDoors, NumericFeatures, QueueClass, QueueOverflow, Store, StoreError,
};
use monoagens_protocol::{
    Action, ActorFacts, ActorId, Audience, BodyId, BodyState, BodyStatus, CapabilityId, EventId,
    OpaquePayload, OriginActor, Percept, PlaceId, ProtocolVersion, Receipt, ReceiptOutcome, Taint,
    TerminationReason, ThoughtAct, Time,
};

#[derive(Default)]
struct CountingDoors {
    inbound_calls: usize,
    context_calls: usize,
    action_calls: usize,
    action_decision: Option<DoorDecision>,
}

impl Doors for CountingDoors {
    fn inbound(&mut self, _percept: &mut Percept) -> DoorDecision {
        self.inbound_calls += 1;
        DoorDecision::Allow
    }

    fn context(&mut self, _plan: &mut ContextPlan) -> DoorDecision {
        self.context_calls += 1;
        DoorDecision::Allow
    }

    fn action(&mut self, _action: &mut Action) -> DoorDecision {
        self.action_calls += 1;
        self.action_decision.clone().unwrap_or(DoorDecision::Allow)
    }
}

fn make_percept(actor: &str, place: &str, ts_ms: u64, payload: &[u8]) -> Percept {
    let actor_id = ActorId::new(actor);
    Percept::minimal(
        actor_id.clone(),
        PlaceId::new(place),
        Audience::new([actor_id]),
        Time::from_millis(ts_ms),
        OpaquePayload::from_bytes(payload),
    )
}

/// Compile-time exhaustive match over all 30 canonical event kinds.
fn assert_exhaustive_kind(kind: EventKind) -> &'static str {
    match kind {
        EventKind::Percept => "percept",
        EventKind::Decision => "decision",
        EventKind::WakeStarted => "wake_started",
        EventKind::ModelCall => "model_call",
        EventKind::Thought => "thought",
        EventKind::Withheld => "withheld",
        EventKind::Action => "action",
        EventKind::ActionFeedback => "action_feedback",
        EventKind::Receipt => "receipt",
        EventKind::ExpectationOpened => "expectation_opened",
        EventKind::ExpectationChanged => "expectation_changed",
        EventKind::Verdict => "verdict",
        EventKind::MemoryUsed => "memory_used",
        EventKind::BeliefChanged => "belief_changed",
        EventKind::Evidence => "evidence",
        EventKind::EpisodeClosed => "episode_closed",
        EventKind::SleepRun => "sleep_run",
        EventKind::DomainsUpdated => "domains_updated",
        EventKind::VectorReleased => "vector_released",
        EventKind::TimingUpdated => "timing_updated",
        EventKind::LearningChange => "learning_change",
        EventKind::Rollback => "rollback",
        EventKind::DoorVerdict => "door_verdict",
        EventKind::IdentityChanged => "identity_changed",
        EventKind::BodyState => "body_state",
        EventKind::ActorFacts => "actor_facts",
        EventKind::Tick => "tick",
        EventKind::Failure => "failure",
        EventKind::HardForget => "hard_forget",
        EventKind::ConfigLoaded => "config_loaded",
    }
}

#[test]
fn thirty_canonical_kinds_exhaustive() {
    assert_eq!(EventKind::ALL.len(), 30);
    let mut seen = BTreeSet::new();
    for kind in EventKind::ALL {
        let label = assert_exhaustive_kind(kind);
        assert_eq!(label, kind.as_str());
        assert!(seen.insert(label), "duplicate kind {label}");
    }
    assert_eq!(seen.len(), 30);
}

#[test]
fn upcast_older_version_never_rewrites_bytes_and_newer_version_fails_loudly() {
    let mut store = MemStore::new();
    let v1 = EventRecordV1 {
        id: EventId::new(10),
        kind: EventKind::Percept,
        format_version: 1,
        timestamp_ms: Time::from_millis(1_000),
        actor: ActorId::new("per_alice"),
        place: PlaceId::new("cli:0"),
        audience: Audience::new([ActorId::new("per_alice")]),
        origin_actor: OriginActor::new(ActorId::new("per_alice")),
        taint: Taint::Trusted,
        caused_by: None,
        payload: OpaquePayload::from_bytes(b"v1-bytes".as_slice()),
    };
    let raw_v1 = EventRecord::encode_v1_bytes(&v1);
    let seq = store.insert_raw_record_bytes(EventId::new(10), raw_v1.clone());

    // Reading upcasts in memory to current version.
    let loaded = store
        .get(EventId::new(10))
        .expect("get succeeds")
        .expect("present");
    assert_eq!(loaded.format_version, EventKind::Percept.current_version());
    assert_eq!(loaded.seq, seq);
    assert_eq!(loaded.payload.as_bytes(), b"v1-bytes");
    // Stored bytes MUST remain unchanged.
    assert_eq!(store.raw_bytes_at(seq), Some(raw_v1.as_slice()));

    // Reading a newer version fails naming the format and both versions.
    let mut future_store = MemStore::new();
    let raw_future =
        EventRecord::encode_with_version_for_test(EventKind::Decision, 77, EventId::new(11));
    future_store.insert_raw_record_bytes(EventId::new(11), raw_future);

    let err = Core::open(future_store, CoreConfig::default(), None)
        .expect_err("future format version must fail Core::open");
    assert!(matches!(
        err,
        CoreError::Store(StoreError::EventFormat(
            EventFormatError::UnsupportedFutureVersion { .. }
        ))
    ));
    let msg = err.to_string();
    assert!(msg.contains("decision"), "missing format name: {msg}");
    assert!(msg.contains("77"), "missing stored version: {msg}");
    assert!(
        msg.contains(&EventKind::Decision.current_version().to_string()),
        "missing supported version: {msg}"
    );
}

#[test]
fn log_seq_total_order_across_interleaved_bodies() {
    let mut core = Core::open(MemStore::new(), CoreConfig::default(), None).expect("open");
    let mut doors = NoopDoors;

    // Interleave inputs from two different bodies / places.
    core.step(
        Input::BodyState(BodyState {
            protocol_version: ProtocolVersion::new(1, 0),
            body_id: BodyId::new("body-chat"),
            status: BodyStatus::Ready,
            capabilities: Vec::new(),
            timestamp_ms: Time::from_millis(100),
        }),
        Time::from_millis(100),
        &mut doors,
    )
    .expect("body 1");
    core.step(
        Input::Percept(Box::new(make_percept(
            "per_alice",
            "body-chat:room",
            110,
            b"from body 1",
        ))),
        Time::from_millis(110),
        &mut doors,
    )
    .expect("percept 1");
    core.step(
        Input::BodyState(BodyState {
            protocol_version: ProtocolVersion::new(1, 0),
            body_id: BodyId::new("body-robot"),
            status: BodyStatus::Ready,
            capabilities: Vec::new(),
            timestamp_ms: Time::from_millis(115),
        }),
        Time::from_millis(115),
        &mut doors,
    )
    .expect("body 2");
    core.step(
        Input::Percept(Box::new(make_percept(
            "per_bob",
            "body-robot:hall",
            120,
            b"from body 2",
        ))),
        Time::from_millis(120),
        &mut doors,
    )
    .expect("percept 2");

    let all = core.store().read_in_order(None, 100).expect("read all");
    assert!(all.len() >= 8);
    for pair in all.windows(2) {
        assert!(
            pair[0].seq < pair[1].seq,
            "LogSeq must strictly increase: {:?} vs {:?}",
            pair[0].seq,
            pair[1].seq
        );
    }
    let tail = core
        .store()
        .read_after_seq(Some(LogSeq::new(4)), 100)
        .expect("read tail");
    assert_eq!(tail.len(), all.len() - 4);
    assert_eq!(tail[0].seq, LogSeq::new(5));
}

#[test]
fn record_first_failed_append_emits_zero_effects_and_zero_dispatches() {
    let mut core = Core::open(MemStore::new(), CoreConfig::default(), None).expect("open");
    let seq_before = core.state().last_seq;
    let len_before = core.store().len();
    core.store_mut().fail_next_append("simulated disk error");
    let mut doors = CountingDoors::default();

    let err = core
        .step(
            Input::Percept(Box::new(make_percept("per_alice", "cli:0", 10, b"hi"))),
            Time::from_millis(10),
            &mut doors,
        )
        .expect_err("failed append must abort step");
    assert!(matches!(err, CoreError::Store(StoreError::AppendFailed(_))));
    assert_eq!(doors.inbound_calls, 0);
    assert_eq!(doors.context_calls, 0);
    assert_eq!(doors.action_calls, 0);
    assert_eq!(core.store().len(), len_before);
    assert_eq!(core.state().last_seq, seq_before);

    // Also verify record-first for outbound Action append failure: zero Dispatches emitted!
    core.step(
        Input::Percept(Box::new(make_percept("per_alice", "cli:0", 20, b"cause"))),
        Time::from_millis(20),
        &mut doors,
    )
    .expect("cause percept");
    let percept_id = core
        .store()
        .read_in_order(None, 100)
        .unwrap()
        .into_iter()
        .find(|e| e.kind == EventKind::Percept)
        .expect("percept event")
        .id;
    let log_len_before = core.store().len();
    core.store_mut()
        .fail_next_append_for_kind(EventKind::Action, "action log full");

    let action_err = core
        .step(
            Input::ModelResult {
                request_id: percept_id,
                acts: vec![ThoughtAct::Act {
                    capability_id: CapabilityId::new("chat.speak"),
                    places: vec![PlaceId::new("cli:0")],
                    people: vec![ActorId::new("per_alice")],
                    payload: OpaquePayload::from_bytes(b"reply".as_slice()),
                    motivation: 4,
                    confidence_0_20: 15,
                    used_ids: vec![percept_id],
                    expected: None,
                    domain: None,
                }],
            },
            Time::from_millis(30),
            &mut doors,
        )
        .expect_err("failed action append must abort without dispatching");
    assert!(matches!(
        action_err,
        CoreError::Store(StoreError::AppendFailed(_))
    ));
    assert!(core.state().in_flight_actions.is_empty());
    // Only Thought was appended before Action failed; no Action record was appended.
    assert_eq!(core.store().len(), log_len_before + 1);
}

#[test]
fn three_class_queue_priority_overflow_and_single_preemption_receipt() {
    assert_eq!(CLASS_A_CAPACITY, 256);
    assert_eq!(CLASS_B_CAPACITY, 512);
    assert_eq!(CLASS_C_CAPACITY, 256);

    let mut core = Core::open(MemStore::new(), CoreConfig::default(), None).expect("open");
    let mut doors = NoopDoors;

    // Enqueue a percept and dispatch an action so it becomes in-flight.
    core.step(
        Input::Percept(Box::new(make_percept("per_alice", "cli:0", 10, b"go"))),
        Time::from_millis(10),
        &mut doors,
    )
    .expect("percept");
    let effects = core
        .step(
            Input::ModelResult {
                request_id: EventId::new(1),
                acts: vec![ThoughtAct::Act {
                    capability_id: CapabilityId::new("tts.speak"),
                    places: vec![PlaceId::new("cli:0")],
                    people: vec![ActorId::new("per_alice")],
                    payload: OpaquePayload::from_bytes(b"speaking".as_slice()),
                    motivation: 4,
                    confidence_0_20: 18,
                    used_ids: vec![EventId::new(1)],
                    expected: None,
                    domain: None,
                }],
            },
            Time::from_millis(20),
            &mut doors,
        )
        .expect("dispatch action");
    assert_eq!(effects.len(), 1);
    let action_id = match &effects[0] {
        Effect::Dispatch(a) => a.action_id.clone(),
        other => panic!("expected Dispatch, got {other:?}"),
    };
    assert!(core.state().in_flight_actions.contains_key(&action_id));

    // Enqueue C, then B, then A (barge-in cancel); verify strict A > B > C drain.
    assert_eq!(core.enqueue(Input::Tick).expect("C"), QueueClass::C);
    assert_eq!(
        core.enqueue(Input::ActorFacts(ActorFacts {
            protocol_version: ProtocolVersion::new(1, 0),
            actor_id: ActorId::new("per_alice"),
            alpha_0: 2.0,
            beta_0: 1.0,
            teacher_weight: 1.0,
            trusted_origin: true,
        }))
        .expect("B"),
        QueueClass::B
    );
    assert_eq!(
        core.enqueue(Input::CancelAction {
            action_id: action_id.clone(),
            termination_reason: TerminationReason::PreemptedExternal,
            reason_code: Some("barge_in".to_owned()),
            detail: Some("user interrupted".to_owned()),
        })
        .expect("A"),
        QueueClass::A
    );

    // Turn 1 drains Class A first: issues the single terminal preempted_external receipt.
    core.step_queued(Time::from_millis(30), &mut doors)
        .expect("drain A")
        .expect("had item");
    assert!(!core.state().in_flight_actions.contains_key(&action_id));
    let term_receipt = core
        .state()
        .terminal_receipts
        .get(&action_id)
        .expect("terminal receipt recorded");
    assert_eq!(
        term_receipt.termination_reason,
        Some(TerminationReason::PreemptedExternal)
    );

    // Turn 2 drains Class B (ActorFacts).
    core.step_queued(Time::from_millis(31), &mut doors)
        .expect("drain B")
        .expect("had item");
    assert!(
        core.state()
            .actor_facts
            .contains_key(&ActorId::new("per_alice"))
    );

    // Turn 3 drains Class C (Tick).
    core.step_queued(Time::from_millis(32), &mut doors)
        .expect("drain C")
        .expect("had item");

    // Duplicate or late receipt for the same action MUST be rejected with DuplicateReceipt.
    let dup_err = core
        .step(
            Input::Receipt(Receipt {
                action_id: action_id.clone(),
                capability_id: CapabilityId::new("tts.speak"),
                outcome: ReceiptOutcome::Done,
                reason_code: None,
                detail: None,
                termination_reason: Some(TerminationReason::Normal),
                progress: Some(1.0),
                delivered_units: Some(10),
                h_act: None,
                timestamp_ms: Time::from_millis(40),
            }),
            Time::from_millis(40),
            &mut doors,
        )
        .expect_err("duplicate receipt must be rejected");
    assert_eq!(dup_err, CoreError::DuplicateReceipt(action_id.clone()));

    // Second Class A cancel yields zero additional receipts.
    let receipts_before = core
        .store()
        .read_in_order(None, 500)
        .unwrap()
        .into_iter()
        .filter(|e| e.kind == EventKind::Receipt)
        .count();
    core.step(
        Input::CancelAction {
            action_id: action_id.clone(),
            termination_reason: TerminationReason::CanceledByCore,
            reason_code: None,
            detail: None,
        },
        Time::from_millis(45),
        &mut doors,
    )
    .expect("idempotent second cancel");
    let receipts_after = core
        .store()
        .read_in_order(None, 500)
        .unwrap()
        .into_iter()
        .filter(|e| e.kind == EventKind::Receipt)
        .count();
    assert_eq!(receipts_before, receipts_after);
    assert_eq!(receipts_after, 1);

    // Queue overflow returns typed QueueOverflow without dropping items.
    for _ in 0..CLASS_C_CAPACITY {
        core.enqueue(Input::Tick).expect("fill C");
    }
    let overflow_err = core.enqueue(Input::Tick).expect_err("C must overflow");
    assert_eq!(
        overflow_err,
        CoreError::QueueOverflow(QueueOverflow {
            class: QueueClass::C,
            capacity: CLASS_C_CAPACITY,
            current_len: CLASS_C_CAPACITY,
        })
    );
}

#[test]
fn core_open_verifies_blake3_replays_tail_and_resumes_interrupted_input_once() {
    let mut core = Core::open(MemStore::new(), CoreConfig::default(), None).expect("open");
    let mut doors = NoopDoors;

    // Phase 1: run initial events and take a blake3-verified snapshot.
    core.step(
        Input::Percept(Box::new(make_percept("per_alice", "chat:1", 100, b"one"))),
        Time::from_millis(100),
        &mut doors,
    )
    .unwrap();
    core.step(
        Input::BodyState(BodyState {
            protocol_version: ProtocolVersion::new(1, 0),
            body_id: BodyId::new("body-1"),
            status: BodyStatus::Ready,
            capabilities: Vec::new(),
            timestamp_ms: Time::from_millis(110),
        }),
        Time::from_millis(110),
        &mut doors,
    )
    .unwrap();
    let ckpt = core.take_snapshot("snap-v1").expect("snapshot");
    assert!(ckpt.cursor.as_u64() > 0);

    // Phase 2: append more completed events after the snapshot.
    core.step(
        Input::Percept(Box::new(make_percept("per_bob", "chat:1", 200, b"two"))),
        Time::from_millis(200),
        &mut doors,
    )
    .unwrap();

    // Clone store with snapshot+tail vs fresh store with all events (no snapshot).
    let store_with_snapshot = core.store().clone();
    let mut store_full_replay = MemStore::new();
    for ev in store_with_snapshot.read_in_order(None, 500).unwrap() {
        store_full_replay.append(ev).unwrap();
    }

    let reopened_tail = Core::open(store_with_snapshot.clone(), CoreConfig::default(), None)
        .expect("reopen with snapshot + tail");
    let reopened_full =
        Core::open(store_full_replay, CoreConfig::default(), None).expect("reopen full replay");
    assert_eq!(reopened_tail.state(), reopened_full.state());

    // Corrupted snapshot must fail loudly naming the snapshot.
    let mut corrupted_store = store_with_snapshot.clone();
    assert!(corrupted_store.corrupt_latest_snapshot_byte(0));
    let corrupt_err = Core::open(corrupted_store, CoreConfig::default(), None)
        .expect_err("corrupted snapshot must fail");
    assert!(matches!(
        corrupt_err,
        CoreError::SnapshotChecksumMismatch { .. }
    ));
    assert!(
        corrupt_err.to_string().contains("snap-v1"),
        "error must name the snapshot: {corrupt_err}"
    );

    // Simulate crash after recording trailing Percepts without Decision markers
    // (out-of-order timestamps to test deterministic timestamp-then-LogSeq resumption).
    let mut crash_store = store_with_snapshot;
    let p_later_ts = EventRecord::new_root(
        EventKind::Percept,
        Time::from_millis(400),
        ActorId::new("per_bob"),
        PlaceId::new("chat:1"),
        Audience::new([ActorId::new("per_bob")]),
        OriginActor::new(ActorId::new("per_bob")),
        Taint::Trusted,
        NumericFeatures::default(),
        OpaquePayload::from_bytes(b"interrupted-later".as_slice()),
        Vec::new(),
        None,
    );
    let p_earlier_ts = EventRecord::new_root(
        EventKind::Percept,
        Time::from_millis(300),
        ActorId::new("per_alice"),
        PlaceId::new("chat:1"),
        Audience::new([ActorId::new("per_alice")]),
        OriginActor::new(ActorId::new("per_alice")),
        Taint::Trusted,
        NumericFeatures::default(),
        OpaquePayload::from_bytes(b"interrupted-earlier".as_slice()),
        Vec::new(),
        None,
    );
    let id_later = crash_store.append(p_later_ts).unwrap();
    let id_earlier = crash_store.append(p_earlier_ts).unwrap();

    let recovered = Core::open(crash_store, CoreConfig::default(), None)
        .expect("recover interrupted inputs");
    // Resumed in timestamp-then-LogSeq order: id_earlier (300ms) before id_later (400ms).
    assert_eq!(
        recovered.recovery_report().resumed_inputs,
        vec![id_earlier, id_later]
    );
    assert!(recovered.state().unconsumed_inputs.is_empty());
    assert!(recovered.state().in_flight_actions.is_empty());

    // Reopening again must NOT re-resume already-resumed inputs.
    let reopened_again = Core::open(recovered.store().clone(), CoreConfig::default(), None)
        .expect("second reopen");
    assert!(reopened_again.recovery_report().resumed_inputs.is_empty());
}

#[test]
fn config_validation_and_neutral_neural_fallback_work_cleanly() {
    let default_cfg = CoreConfig::default();
    assert!(default_cfg.validate().is_ok());

    let bad_cfg = CoreConfig {
        max_wait_ms: 0.0,
        ..default_cfg.clone()
    };
    assert!(matches!(bad_cfg.validate(), Err(monoagens_core::ConfigError::OutOfRange { .. })));

    let open_err = Core::open(MemStore::new(), bad_cfg, None).expect_err("invalid config must fail");
    assert!(matches!(open_err, CoreError::Config(_)));
}

#[test]
fn model_result_inherits_origin_actor_and_min_taint_monotonically() {
    let mut core = Core::open(MemStore::new(), CoreConfig::default(), None).expect("open");
    let mut doors = NoopDoors;

    let mut untrusted_percept = make_percept("per_eve", "chat:group", 50, b"prompt injection");
    untrusted_percept.origin_actor = Some(OriginActor::new(ActorId::new("per_eve")));
    untrusted_percept.taint = Taint::Untrusted;

    core.step(
        Input::Percept(Box::new(untrusted_percept)),
        Time::from_millis(50),
        &mut doors,
    )
    .unwrap();
    let percept_id = core
        .store()
        .read_in_order(None, 100)
        .unwrap()
        .into_iter()
        .find(|e| e.kind == EventKind::Percept)
        .expect("percept event")
        .id;

    // Model attempts to act while claiming a different actor/people list in ThoughtAct::Act.
    let effects = core
        .step(
            Input::ModelResult {
                request_id: percept_id,
                acts: vec![ThoughtAct::Act {
                    capability_id: CapabilityId::new("chat.speak"),
                    places: vec![PlaceId::new("chat:group")],
                    people: vec![ActorId::new("per_owner")],
                    payload: OpaquePayload::from_bytes(b"hi".as_slice()),
                    motivation: 4,
                    confidence_0_20: 15,
                    used_ids: vec![percept_id],
                    expected: None,
                    domain: None,
                }],
            },
            Time::from_millis(60),
            &mut doors,
        )
        .unwrap();

    assert_eq!(effects.len(), 1);
    let Effect::Dispatch(action) = &effects[0] else {
        panic!("expected Dispatch");
    };
    assert_eq!(
        action.origin_actor,
        OriginActor::new(ActorId::new("per_eve"))
    );
    assert_eq!(action.taint, Taint::Untrusted);
}
