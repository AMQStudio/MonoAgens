use monoagens_core::{
    Core, CoreState, DecisionOutcome, EventDetail, EventKind, EventRecord, Input, LogSeq, MemStore,
    NoopDoors, NumericFeatures, Prior, ReplayMode, SnapshotCheckpoint, Store, Switches, blake3,
    replay_from_checkpoint, replay_log,
};
use monoagens_protocol::{
    ActorId, Audience, EventId, OpaquePayload, OriginActor, Percept, PlaceId, Taint, Time,
};

fn make_percept(actor: &str, place: &str, ts_ms: u64, payload: &[u8]) -> Percept {
    let mut p = Percept::minimal(
        ActorId::new(actor),
        PlaceId::new(place),
        Audience::new([ActorId::new(actor), ActorId::new("soul")]),
        Time::from_millis(ts_ms),
        OpaquePayload::from_bytes(payload),
    );
    p.named = vec![ActorId::new("soul")];
    p
}

#[test]
fn restart_with_same_prior_appends_none_and_edited_prior_appends_one() {
    let prior_a = Prior::default();
    let mut switches = Switches::all_enabled();
    switches.stochastic_go_no_go = false;

    let mut core =
        Core::open(MemStore::new(), prior_a.clone(), switches.clone(), 17).expect("initial open");
    let initial_events = core.store().read_in_order(None, usize::MAX).unwrap();
    assert_eq!(initial_events.len(), 1);
    assert_eq!(initial_events[0].kind, EventKind::PriorLoaded);

    let p1 = make_percept("per_alice", "room:1", 1_000, b"hello under A");
    core.step(
        Input::Percept(Box::new(p1)),
        Time::from_millis(1_000),
        &mut NoopDoors,
    )
    .expect("step 1");
    let count_after_step1 = core.store().read_in_order(None, usize::MAX).unwrap().len();

    // Reopen with the exact same prior -> no new prior_loaded event appended.
    let store_same = core.store().clone();
    let core_same = Core::open(store_same, prior_a.clone(), switches.clone(), 17)
        .expect("reopen with same prior");
    let events_same = core_same
        .store()
        .read_in_order(None, usize::MAX)
        .expect("read");
    assert_eq!(events_same.len(), count_after_step1);
    assert_eq!(
        events_same
            .iter()
            .filter(|e| e.kind == EventKind::PriorLoaded)
            .count(),
        1
    );

    // Reopen with an edited prior (even keeping the same content_version) -> appends one prior_loaded.
    let prior_b = Prior {
        talkativeness: 0.92,
        ..prior_a
    };
    let core_b = Core::open(core_same.store().clone(), prior_b.clone(), switches, 17)
        .expect("reopen with edited prior");
    let events_b = core_b
        .store()
        .read_in_order(None, usize::MAX)
        .expect("read");
    assert_eq!(events_b.len(), count_after_step1 + 1);
    let last = events_b.last().expect("last event");
    assert_eq!(last.kind, EventKind::PriorLoaded);
    match &last.detail {
        Some(EventDetail::PriorLoaded { prior }) => assert_eq!(prior.as_ref(), &prior_b),
        other => panic!("expected PriorLoaded detail, got {other:?}"),
    }
}

#[test]
fn two_prior_log_replay_reproduces_decisions_from_start_and_checkpoint() {
    let mut switches = Switches::all_enabled();
    switches.stochastic_go_no_go = false;
    switches.patience = false;

    // Prior A: very high baseline threshold (talkativeness = 0.0 -> theta_0 = 0.85), stays silent.
    let prior_a = Prior {
        talkativeness: 0.0,
        own_name_weight: 0.5,
        ..Prior::default()
    };
    let mut core_a =
        Core::open(MemStore::new(), prior_a.clone(), switches.clone(), 42).expect("open A");
    let mut p1 = make_percept("per_alice", "room:two", 1_000, b"first under A");
    p1.addressees = Vec::new();
    core_a
        .step(
            Input::Percept(Box::new(p1)),
            Time::from_millis(1_000),
            &mut NoopDoors,
        )
        .expect("step under A");

    let ckpt_a = core_a.take_snapshot("under_a").expect("snapshot A");
    let store_after_a = core_a.store().clone();

    // Prior B: very low baseline threshold (talkativeness = 1.0 -> theta_0 = 0.35), wakes.
    let prior_b = Prior {
        talkativeness: 1.0,
        own_name_weight: 1.0,
        reserve_when_unfamiliar: 0.0,
        ..Prior::default()
    };
    let mut core_b =
        Core::open(store_after_a, prior_b.clone(), switches.clone(), 42).expect("open B");
    let mut p2 = make_percept("per_alice", "room:two", 2_000, b"second under B");
    p2.addressees = Vec::new();
    core_b
        .step(
            Input::Percept(Box::new(p2)),
            Time::from_millis(2_000),
            &mut NoopDoors,
        )
        .expect("step under B");

    let events = core_b.store().read_in_order(None, usize::MAX).unwrap();
    let decisions: Vec<_> = events
        .iter()
        .filter(|e| e.kind == EventKind::Decision)
        .collect();
    assert_eq!(decisions.len(), 2);
    assert!(matches!(
        decisions[0].detail,
        Some(EventDetail::Decision {
            outcome: DecisionOutcome::Silent | DecisionOutcome::Note,
            ..
        })
    ));
    assert!(matches!(
        decisions[1].detail,
        Some(EventDetail::Decision {
            outcome: DecisionOutcome::Go,
            ..
        })
    ));

    // Full replay from start (no checkpoint, no caller prior) must match both A and B decisions.
    let full_rep =
        replay_from_checkpoint(core_b.store(), None, &switches, 42, ReplayMode::FullReplay)
            .expect("full replay");
    assert!(
        full_rep.is_bit_identical(),
        "full replay diverged: {:?}",
        full_rep.first_divergence
    );
    assert_eq!(full_rep.matched_decisions, 2);

    // Checkpoint replay from snapshot taken under A must switch to B at the second prior_loaded.
    let ckpt_rep = replay_from_checkpoint(
        core_b.store(),
        Some(&ckpt_a),
        &switches,
        42,
        ReplayMode::TwoEpochFrozen,
    )
    .expect("checkpoint replay");
    assert!(
        ckpt_rep.is_bit_identical(),
        "ckpt replay diverged: {:?}",
        ckpt_rep.first_divergence
    );
    assert_eq!(ckpt_rep.matched_decisions, 1);

    // Core::replay_verify and replay_log also reproduce cleanly.
    let verify_rep = core_b
        .replay_verify(ReplayMode::FullReplay)
        .expect("replay_verify");
    assert!(verify_rep.is_bit_identical());
    let log_rep = replay_log(core_b.store(), &switches, 42).expect("replay_log");
    assert!(log_rep.is_bit_identical());
}

#[test]
fn crash_recovery_runs_under_logged_prior_before_appending_new_prior() {
    let prior_a = Prior::default();
    let prior_b = Prior {
        talkativeness: 0.88,
        ..Prior::default()
    };
    let switches = Switches::all_enabled();

    let core_a = Core::open(MemStore::new(), prior_a, switches.clone(), 5).expect("open A");
    let mut crashed_store = core_a.store().clone();
    // Simulate a crash after recording an input percept before its decision marker.
    let unconsumed = EventRecord::new_root(
        EventKind::Percept,
        Time::from_millis(5_000),
        ActorId::new("per_bob"),
        PlaceId::new("room:crash"),
        Audience::new([ActorId::new("per_bob"), ActorId::new("soul")]),
        OriginActor::new(ActorId::new("per_bob")),
        Taint::Trusted,
        NumericFeatures::default(),
        OpaquePayload::from_bytes(b"interrupted".as_slice()),
        Vec::new(),
        None,
    );
    let unconsumed_id = crashed_store.append(unconsumed).expect("append unconsumed");

    // Reopen with prior_b: recovery marker for unconsumed_id must be appended BEFORE prior_loaded(B).
    let reopened = Core::open(crashed_store, prior_b.clone(), switches, 5).expect("reopen B");
    assert_eq!(
        reopened.recovery_report().resumed_inputs,
        vec![unconsumed_id]
    );
    let events = reopened.store().read_in_order(None, usize::MAX).unwrap();
    let recovered_seq = events
        .iter()
        .find(|e| e.kind == EventKind::Decision)
        .expect("recovered decision")
        .seq;
    let prior_b_seq = events
        .iter()
        .filter(|e| e.kind == EventKind::PriorLoaded)
        .nth(1)
        .expect("second prior_loaded")
        .seq;
    assert!(
        recovered_seq < prior_b_seq,
        "crash recovery ({recovered_seq}) must finish before new prior_loaded ({prior_b_seq})"
    );
    assert_eq!(reopened.state().recorded_prior.as_ref(), Some(&prior_b));
}

#[test]
fn legacy_log_without_prior_loaded_opens_and_replays_cleanly() {
    let mut store = MemStore::new();
    let p_rec = EventRecord::new_root(
        EventKind::Percept,
        Time::from_millis(100),
        ActorId::new("per_alice"),
        PlaceId::new("room:legacy"),
        Audience::new([ActorId::new("per_alice"), ActorId::new("soul")]),
        OriginActor::new(ActorId::new("per_alice")),
        Taint::Trusted,
        NumericFeatures::default(),
        OpaquePayload::from_bytes(b"legacy"),
        Vec::new(),
        None,
    );
    let pid = store.append(p_rec).unwrap();
    assert_eq!(pid.as_u64(), 1);

    let core =
        Core::open(store, Prior::default(), Switches::default(), 1).expect("open legacy log");
    let events = core.store().read_in_order(None, usize::MAX).unwrap();
    assert!(events.iter().any(|e| e.kind == EventKind::PriorLoaded));
    let rep = replay_log(core.store(), &Switches::default(), 1).expect("replay legacy");
    assert!(rep.is_bit_identical());
}

#[test]
fn identical_draw_outcomes_with_and_without_open_prior_loaded() {
    let prior = Prior::default();
    let switches = Switches::all_enabled();
    let seed = 1337;

    // Run A: normal Core::open appends `prior_loaded` -> prior_loaded_count == 1.
    let mut core_with =
        Core::open(MemStore::new(), prior.clone(), switches.clone(), seed).expect("open with");
    assert_eq!(core_with.state().prior_loaded_count, 1);

    // Run B: pre-seeded empty store with recorded_prior == prior and prior_loaded_count == 0
    // so Core::open appends no `prior_loaded` event.
    let mut seed_state = CoreState::new();
    seed_state.recorded_prior = Some(prior.clone());
    seed_state.prior_loaded_count = 0;
    let snap_bytes = seed_state.encode_snapshot();
    let snap_hash = blake3::hash(&snap_bytes);
    let mut store_without = MemStore::new();
    store_without
        .snapshot(SnapshotCheckpoint {
            snapshot_id: "zero-prior-events".to_owned(),
            cursor: LogSeq::ZERO,
            epoch: 0,
            bytes: snap_bytes,
            blake3: snap_hash,
        })
        .unwrap();
    let mut core_without = Core::open(store_without, prior, switches, seed).expect("open without");
    assert_eq!(core_without.state().prior_loaded_count, 0);

    for i in 0..12u64 {
        let ts = (i + 1) * 1_000;
        let mut p = make_percept("per_alice", "room:draw", ts, format!("msg {i}").as_bytes());
        p.named.clear();
        core_with
            .step(
                Input::Percept(Box::new(p.clone())),
                Time::from_millis(ts),
                &mut NoopDoors,
            )
            .unwrap();
        core_without
            .step(
                Input::Percept(Box::new(p)),
                Time::from_millis(ts),
                &mut NoopDoors,
            )
            .unwrap();
    }

    let extract_draws = |events: &[EventRecord]| -> Vec<(DecisionOutcome, u64, u64, u64, u64)> {
        events
            .iter()
            .filter_map(|e| match &e.detail {
                Some(EventDetail::Decision {
                    outcome,
                    salience,
                    threshold,
                    probability,
                    draw,
                    ..
                }) => Some((
                    *outcome,
                    salience.to_bits(),
                    threshold.to_bits(),
                    probability.to_bits(),
                    draw.to_bits(),
                )),
                _ => None,
            })
            .collect()
    };

    let events_with = core_with.store().read_in_order(None, usize::MAX).unwrap();
    let events_without = core_without
        .store()
        .read_in_order(None, usize::MAX)
        .unwrap();
    assert_eq!(events_with.len(), events_without.len() + 1);
    assert_eq!(extract_draws(&events_with), extract_draws(&events_without));
}

#[test]
fn mid_log_prior_loaded_with_stochastic_draws_replays_bit_identically_and_upcasts_v8_snapshot() {
    let switches = Switches::all_enabled();
    let seed = 99;
    let prior_a = Prior::default();
    let prior_b = Prior {
        talkativeness: 0.78,
        ..Prior::default()
    };

    let mut core_a = Core::open(MemStore::new(), prior_a, switches.clone(), seed).unwrap();
    for i in 0..5u64 {
        let ts = (i + 1) * 1_000;
        let mut p = make_percept(
            "per_alice",
            "room:stoch",
            ts,
            format!("phase1-{i}").as_bytes(),
        );
        p.named.clear();
        core_a
            .step(
                Input::Percept(Box::new(p)),
                Time::from_millis(ts),
                &mut NoopDoors,
            )
            .unwrap();
    }
    let ckpt_mid = core_a.take_snapshot("mid-stoch").unwrap();
    let store_mid = core_a.store().clone();

    let mut core_b = Core::open(store_mid, prior_b, switches.clone(), seed).unwrap();
    assert_eq!(core_b.state().prior_loaded_count, 2);
    for i in 5..10u64 {
        let ts = (i + 1) * 1_000;
        let mut p = make_percept(
            "per_alice",
            "room:stoch",
            ts,
            format!("phase2-{i}").as_bytes(),
        );
        p.named.clear();
        core_b
            .step(
                Input::Percept(Box::new(p)),
                Time::from_millis(ts),
                &mut NoopDoors,
            )
            .unwrap();
    }

    let full_rep = replay_from_checkpoint(
        core_b.store(),
        None,
        &switches,
        seed,
        ReplayMode::FullReplay,
    )
    .unwrap();
    assert!(
        full_rep.is_bit_identical(),
        "full replay diverged: {:?}",
        full_rep.first_divergence
    );
    assert_eq!(full_rep.matched_decisions, 10);

    let ckpt_rep = replay_from_checkpoint(
        core_b.store(),
        Some(&ckpt_mid),
        &switches,
        seed,
        ReplayMode::TwoEpochFrozen,
    )
    .unwrap();
    assert!(
        ckpt_rep.is_bit_identical(),
        "checkpoint replay diverged: {:?}",
        ckpt_rep.first_divergence
    );
    assert_eq!(ckpt_rep.matched_decisions, 5);
    assert!(
        core_b
            .replay_verify(ReplayMode::FullReplay)
            .unwrap()
            .is_bit_identical()
    );
    assert!(
        replay_log(core_b.store(), &switches, seed)
            .unwrap()
            .is_bit_identical()
    );

    // Snapshot v9 round-trip preserves prior_loaded_count == 2; v8 snapshot upcasts with 0.
    let decoded_v9 = CoreState::decode_snapshot(&ckpt_mid.bytes).unwrap();
    assert_eq!(decoded_v9.prior_loaded_count, 1);
    let snap_b = core_b.take_snapshot("after-b").unwrap();
    assert_eq!(
        CoreState::decode_snapshot(&snap_b.bytes)
            .unwrap()
            .prior_loaded_count,
        2
    );

    // Strip the trailing u64 (prior_loaded_count) and patch version 9 -> 8 to simulate a v8 snapshot.
    let mut v8_bytes = snap_b.bytes[..snap_b.bytes.len() - 8].to_vec();
    assert_eq!(&v8_bytes[4..6], &9u16.to_le_bytes());
    v8_bytes[4..6].copy_from_slice(&8u16.to_le_bytes());
    let decoded_v8 = CoreState::decode_snapshot(&v8_bytes).unwrap();
    assert_eq!(decoded_v8.prior_loaded_count, 0);
    assert_eq!(decoded_v8.draw_key(EventId::new(10)), EventId::new(10));
}
