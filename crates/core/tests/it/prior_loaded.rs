use std::sync::Arc;

use monoagens_core::neural::NeuralModel;
use monoagens_core::{
    Core, CoreConfig, CoreState, DecisionOutcome, EventDetail, EventKind, EventRecord, Input,
    LogSeq, MemStore, NoopDoors, NumericFeatures, ReplayMode, SnapshotCheckpoint, Store, blake3,
    replay_from_checkpoint, replay_log,
};
use monoagens_protocol::{
    ActorId, Audience, OpaquePayload, OriginActor, Percept, PlaceId, Taint, Time,
};

fn load_golden_model() -> (Arc<NeuralModel>, [u8; 32]) {
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let json_path = manifest_dir.join("tests/fixtures/neural_phase1_golden.json");
    let json_text = std::fs::read_to_string(&json_path).expect("read golden json");
    let val: serde_json::Value = serde_json::from_str(&json_text).expect("parse golden json");
    let st_name = val["cases"][0]["safetensors_file"]
        .as_str()
        .expect("safetensors_file");
    let bytes = std::fs::read(manifest_dir.join("tests/fixtures").join(st_name))
        .expect("read golden safetensors");
    let digest = blake3::hash(&bytes);
    let model = Arc::new(
        NeuralModel::from_safetensors_bytes(&bytes, Some(digest)).expect("load golden model"),
    );
    (model, digest)
}

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
fn restart_with_same_config_appends_none_and_edited_config_appends_one() {
    let cfg_a = CoreConfig::default();

    let mut core = Core::open(MemStore::new(), cfg_a.clone(), None).expect("initial open");
    let initial_events = core.store().read_in_order(None, usize::MAX).unwrap();
    assert_eq!(initial_events.len(), 1);
    assert_eq!(initial_events[0].kind, EventKind::ConfigLoaded);

    let p1 = make_percept("per_alice", "room:1", 1_000, b"hello under A");
    core.step(
        Input::Percept(Box::new(p1)),
        Time::from_millis(1_000),
        &mut NoopDoors,
    )
    .expect("step 1");
    let count_after_step1 = core.store().read_in_order(None, usize::MAX).unwrap().len();

    // Reopen with the exact same CoreConfig -> no new config_loaded event appended.
    let store_same = core.store().clone();
    let core_same = Core::open(store_same, cfg_a.clone(), None).expect("reopen with same config");
    let events_same = core_same
        .store()
        .read_in_order(None, usize::MAX)
        .expect("read");
    assert_eq!(events_same.len(), count_after_step1);
    assert_eq!(
        events_same
            .iter()
            .filter(|e| e.kind == EventKind::ConfigLoaded)
            .count(),
        1
    );

    // Reopen with an edited CoreConfig -> appends one config_loaded.
    let cfg_b = CoreConfig {
        seed: 99,
        ..cfg_a
    };
    let core_b = Core::open(core_same.store().clone(), cfg_b.clone(), None)
        .expect("reopen with edited config");
    let events_b = core_b
        .store()
        .read_in_order(None, usize::MAX)
        .expect("read");
    assert_eq!(events_b.len(), count_after_step1 + 1);
    let last = events_b.last().expect("last event");
    assert_eq!(last.kind, EventKind::ConfigLoaded);
    match &last.detail {
        Some(EventDetail::ConfigLoaded { config, .. }) => assert_eq!(config.as_ref(), &cfg_b),
        other => panic!("expected ConfigLoaded detail, got {other:?}"),
    }
}

#[test]
fn trace_pinned_replay_verifies_full_log_and_checkpoint_across_config_and_model_runs() {
    let (model, digest) = load_golden_model();
    let cfg_a = CoreConfig {
        seed: 42,
        stochastic_gate_sampling: false,
        online_adaptation: false,
        ..CoreConfig::default()
    };

    let mut core_a = Core::open(MemStore::new(), cfg_a.clone(), Some(Arc::clone(&model)))
        .expect("open A with neural model");
    let mut p1 = make_percept("per_alice", "room:two", 1_000, b"first under A");
    p1.addressees = Vec::new();
    p1.named.clear();
    core_a
        .step(
            Input::Percept(Box::new(p1)),
            Time::from_millis(1_000),
            &mut NoopDoors,
        )
        .expect("step under A");

    let ckpt_a = core_a.take_snapshot("under_a").expect("snapshot A");
    assert!(
        !core_a.state().place_memory.is_empty(),
        "neural step must populate per-place 64d recurrent h_t"
    );
    let store_after_a = core_a.store().clone();

    let cfg_b = CoreConfig {
        online_adaptation: true,
        ..cfg_a
    };
    let mut core_b = Core::open(store_after_a, cfg_b, Some(Arc::clone(&model))).expect("open B");
    let mut p2 = make_percept("per_alice", "room:two", 2_000, b"second under B");
    p2.addressees = vec![ActorId::new("soul")];
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
        decisions[1].detail,
        Some(EventDetail::Decision {
            outcome: DecisionOutcome::Go,
            ..
        })
    ));

    // Pinned full replay verifies both decisions & digest pin.
    let full_rep =
        replay_from_checkpoint(core_b.store(), None, ReplayMode::Pinned, Some(digest))
            .expect("full replay");
    assert!(
        full_rep.is_exact(),
        "full replay diverged: {:?}",
        full_rep.mismatches
    );
    assert_eq!(full_rep.verified_decisions, 2);

    // Pinned replay from checkpoint under A verifies remaining 1 decision.
    let ckpt_rep = replay_from_checkpoint(
        core_b.store(),
        Some(&ckpt_a),
        ReplayMode::Pinned,
        Some(digest),
    )
    .expect("checkpoint replay");
    assert!(
        ckpt_rep.is_exact(),
        "ckpt replay diverged: {:?}",
        ckpt_rep.mismatches
    );
    assert_eq!(ckpt_rep.verified_decisions, 1);

    // Pinned replay with a wrong model digest detects mismatch cleanly.
    let wrong_digest = [0xAA_u8; 32];
    let bad_pin =
        replay_log(core_b.store(), ReplayMode::Pinned, Some(wrong_digest)).expect("bad pin");
    assert!(!bad_pin.is_exact());
    assert_eq!(bad_pin.mismatches[0].field, "model_blake3");

    // Unpinned replay ignores digest changes across upgrades.
    let unpinned = replay_log(core_b.store(), ReplayMode::Unpinned, Some(wrong_digest))
        .expect("unpinned");
    assert!(unpinned.is_exact());

    // Core::replay_verify also passes cleanly.
    let verify_rep = core_b
        .replay_verify(ReplayMode::Pinned)
        .expect("replay_verify");
    assert!(verify_rep.is_exact());
}

#[test]
fn crash_recovery_runs_under_logged_config_before_appending_new_config() {
    let cfg_a = CoreConfig::default();
    let cfg_b = CoreConfig {
        seed: 77,
        ..CoreConfig::default()
    };

    let core_a = Core::open(MemStore::new(), cfg_a, None).expect("open A");
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

    // Reopen with cfg_b: recovery marker for unconsumed_id must be appended BEFORE config_loaded(B).
    let reopened = Core::open(crashed_store, cfg_b.clone(), None).expect("reopen B");
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
    let cfg_b_seq = events
        .iter()
        .filter(|e| e.kind == EventKind::ConfigLoaded)
        .nth(1)
        .expect("second config_loaded")
        .seq;
    assert!(
        recovered_seq < cfg_b_seq,
        "crash recovery ({recovered_seq}) must finish before new config_loaded ({cfg_b_seq})"
    );
    assert_eq!(reopened.state().recorded_config.as_ref(), Some(&cfg_b));
}

#[test]
fn legacy_log_without_config_loaded_opens_and_replays_cleanly() {
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

    let core = Core::open(store, CoreConfig::default(), None).expect("open legacy log");
    let events = core.store().read_in_order(None, usize::MAX).unwrap();
    assert!(events.iter().any(|e| e.kind == EventKind::ConfigLoaded));
    let rep = replay_log(core.store(), ReplayMode::Unpinned, None).expect("replay legacy");
    assert!(rep.is_exact());
}

#[test]
fn snapshot_persists_place_memory_h_t_and_place_adapt_across_restart() {
    let (model, _) = load_golden_model();
    let cfg = CoreConfig {
        seed: 1337,
        online_adaptation: true,
        ..CoreConfig::default()
    };

    let mut core = Core::open(MemStore::new(), cfg.clone(), Some(Arc::clone(&model))).unwrap();
    let place = PlaceId::new("room:persist");
    let mut p = make_percept("per_alice", "room:persist", 1_000, b"first");
    p.named.clear();
    core.step(
        Input::Percept(Box::new(p)),
        Time::from_millis(1_000),
        &mut NoopDoors,
    )
    .unwrap();
    core.record_place_feedback(
        &place,
        monoagens_core::neural::AdaptFeedback {
            arm: monoagens_core::neural::GateArm::Go,
            reward: 1.0,
            is_delayed_one_sided: false,
            context_hint: [1.0, 0.5],
        },
    );

    let h_before = core.state().place_memory.get(&place).cloned().expect("h_t");
    let adapt_before = core
        .state()
        .place_adapt
        .get(&place)
        .cloned()
        .expect("place_adapt");
    let ckpt = core.take_snapshot("neural-state").unwrap();

    // Pre-seeded store check too: config_loaded_count == 1 vs decoded snapshot.
    let decoded = CoreState::decode_snapshot(&ckpt.bytes).unwrap();
    assert_eq!(decoded.config_loaded_count, 1);
    assert_eq!(decoded.place_memory.get(&place), Some(&h_before));
    assert_eq!(decoded.place_adapt.get(&place), Some(&adapt_before));

    // Reopen from snapshot store with same config + model -> preserves h_t and PlaceAdaptState.
    let reopened = Core::open(core.store().clone(), cfg.clone(), Some(model)).unwrap();
    assert_eq!(reopened.state().place_memory.get(&place), Some(&h_before));
    assert_eq!(reopened.state().place_adapt.get(&place), Some(&adapt_before));
    assert_eq!(reopened.state().config_loaded_count, 1);

    // Pre-seeded empty store with recorded_config == cfg and config_loaded_count == 0
    // verifies Core::open skips redundant ConfigLoaded event.
    let mut seed_state = CoreState::new();
    seed_state.recorded_config = Some(cfg.clone());
    seed_state.config_loaded_count = 0;
    let snap_bytes = seed_state.encode_snapshot();
    let snap_hash = blake3::hash(&snap_bytes);
    let mut store_zero = MemStore::new();
    store_zero
        .snapshot(SnapshotCheckpoint {
            snapshot_id: "zero-cfg-events".to_owned(),
            cursor: LogSeq::ZERO,
            epoch: 0,
            bytes: snap_bytes,
            blake3: snap_hash,
        })
        .unwrap();
    let core_zero = Core::open(store_zero, cfg, None).unwrap();
    assert_eq!(core_zero.state().config_loaded_count, 0);
}
