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

#[test]
fn self_caused_percept_advances_h_t_without_waking_and_replays_identically() {
    let (model, digest) = load_golden_model();
    let cfg = CoreConfig {
        seed: 77,
        ..CoreConfig::default()
    };
    let mut core = Core::open(MemStore::new(), cfg, Some(Arc::clone(&model))).unwrap();
    let place = PlaceId::new("room:self_h");

    // 1. Incoming percept wakes reasoning.
    let mut p1 = make_percept("per_alice", "room:self_h", 1_000, b"hi soul");
    p1.addressees = vec![ActorId::new("soul")];
    let eff1 = core
        .step(
            Input::Percept(Box::new(p1)),
            Time::from_millis(1_000),
            &mut NoopDoors,
        )
        .unwrap();
    let trigger_id = match eff1.as_slice() {
        [monoagens_core::Effect::Model { plan, .. }] => plan.trigger_event_id.unwrap(),
        other => panic!("expected Reason wake, got {other:?}"),
    };
    let h_before_echo = core.state().place_memory.get(&place).cloned().expect("h1");

    // Dispatch a chat reply (opens Effect expectation with correlation_id).
    let act_eff = core
        .step(
            Input::ModelResult {
                request_id: trigger_id,
                acts: vec![monoagens_protocol::ThoughtAct::Act {
                    capability_id: monoagens_protocol::CapabilityId::new("chat.speak"),
                    places: vec![place.clone()],
                    people: vec![ActorId::new("per_alice")],
                    payload: OpaquePayload::from_bytes(b"hello back"),
                    motivation: 4,
                    confidence_0_20: 16,
                    used_ids: vec![trigger_id],
                    expected: None,
                    domain: None,
                }],
            },
            Time::from_millis(1_100),
            &mut NoopDoors,
        )
        .unwrap();
    let corr_id = match act_eff.as_slice() {
        [monoagens_core::Effect::Dispatch(action)] => action.correlation_id.clone(),
        other => panic!("expected Dispatch effect, got {other:?}"),
    };

    // 2. Echo percept matching the open Effect expectation -> SelfCaused, zero wakes, advances h_t!
    let mut echo = make_percept("soul", "room:self_h", 1_200, b"hello back");
    echo.named.clear();
    echo.correlation_id = corr_id;
    let echo_eff = core
        .step(
            Input::Percept(Box::new(echo)),
            Time::from_millis(1_200),
            &mut NoopDoors,
        )
        .unwrap();
    assert!(
        echo_eff.is_empty(),
        "SelfCaused echo must never trigger a wake"
    );
    let h_after_echo = core.state().place_memory.get(&place).cloned().expect("h2");
    assert_ne!(
        h_before_echo, h_after_echo,
        "C-9: SelfCaused percept must advance place_memory h_t"
    );

    // 3. Replay from event log reproduces identical h_t.
    let rep = replay_log(core.store(), ReplayMode::Pinned, Some(digest)).expect("replay");
    assert!(rep.is_exact());
    assert_eq!(rep.state.place_memory.get(&place), Some(&h_after_echo));
}

#[test]
fn consolidated_gist_attaches_episode_centroid_and_is_recalled_by_vector_partition() {
    use monoagens_core::RecallQuery;
    use monoagens_protocol::{EmbeddingTag, TaggedEmbedding, VisibleScope};

    let cfg = CoreConfig {
        sleep_min_episodes: 1,
        sleep_idle_ms: 1_000,
        ..CoreConfig::default()
    };
    let mut core = Core::open(MemStore::new(), cfg, None).unwrap();
    let tag = EmbeddingTag {
        embed_model_ver: "egemma2-256".to_owned(),
        dim_d: 4,
        prompt_regime_id: "t_d".to_owned(),
    };
    let emb = TaggedEmbedding {
        tag: tag.clone(),
        values: vec![0.8, 0.6, 0.0, 0.0],
    };

    for i in 0..3u64 {
        let mut p = make_percept("per_alice", "room:gist", 1_000 + i * 100, b"topic turn");
        p.named.clear();
        p.embeddings = vec![emb.clone()];
        core.step(
            Input::Percept(Box::new(p)),
            Time::from_millis(1_000 + i * 100),
            &mut NoopDoors,
        )
        .unwrap();
    }

    // Trigger sleep consolidation -> emits Consolidate effect for the episode.
    let sleep_eff = core
        .run_sleep_if_due(Time::from_millis(50_000), &mut NoopDoors)
        .unwrap();
    let (req_id, ep_items) = sleep_eff
        .iter()
        .find_map(|e| match e {
            monoagens_core::Effect::Model {
                purpose: monoagens_core::ModelPurpose::Consolidate,
                plan,
            } => Some((plan.trigger_event_id.unwrap(), plan.items.clone())),
            _ => None,
        })
        .expect("consolidate effect");

    core.step(
        Input::ModelResult {
            request_id: req_id,
            acts: vec![monoagens_protocol::ThoughtAct::Consolidate {
                output: monoagens_protocol::ConsolidateOutput::Gist {
                    summary: "summary of topic".to_owned(),
                    about_ids: ep_items,
                    places: vec![PlaceId::new("room:gist")],
                    people: vec![ActorId::new("per_alice")],
                    domain: None,
                },
            }],
        },
        Time::from_millis(50_100),
        &mut NoopDoors,
    )
    .unwrap();

    // Vector-partitioned recall must find the consolidated gist immediately (C-3).
    let scope = VisibleScope::new(
        Audience::new([ActorId::new("per_alice"), ActorId::new("soul")]),
        Some(PlaceId::new("room:gist")),
    );
    let mut query = RecallQuery::new(scope);
    query.partition_tag = Some(tag);
    query.query_embeddings = vec![emb];
    query.limit = 10;
    let res = core.recall(&query, Time::from_millis(50_200)).unwrap();
    assert!(
        res.items.iter().any(|item| item.source_ids.len() >= 3),
        "consolidated gist with episode centroid must appear in vector recall: {:?}",
        res.items
    );
}

#[test]
fn phase2_p_verify_and_p_uncertainty_populate_context_plan_and_receipt_done_does_not_reward_go() {
    use monoagens_core::{ContextPlan, DoorDecision, Doors};

    #[derive(Default)]
    struct CaptureDoors {
        plans: Vec<ContextPlan>,
    }
    impl Doors for CaptureDoors {
        fn inbound(&mut self, _percept: &mut Percept) -> DoorDecision {
            DoorDecision::Allow
        }
        fn context(&mut self, plan: &mut ContextPlan) -> DoorDecision {
            self.plans.push(plan.clone());
            DoorDecision::Allow
        }
        fn action(&mut self, _action: &mut monoagens_protocol::Action) -> DoorDecision {
            DoorDecision::Allow
        }
    }

    // 1. Neutral fallback leaves neural_p_verify / neural_p_uncertainty as None.
    let mut core_neutral = Core::open(MemStore::new(), CoreConfig::default(), None).unwrap();
    let mut p_comm = make_percept("per_alice", "room:p2", 1_000, b"direct question");
    p_comm.addressees = vec![ActorId::new("soul")];
    let mut doors_neutral = CaptureDoors::default();
    let _ = core_neutral
        .step(
            Input::Percept(Box::new(p_comm)),
            Time::from_millis(1_000),
            &mut doors_neutral,
        )
        .unwrap();
    let neutral_plan = doors_neutral.plans.last().expect("neutral plan");
    assert!(neutral_plan.neural_p_verify.is_none());
    assert!(neutral_plan.neural_p_uncertainty.is_none());

    // 2. Phase 2 model populates bounded neural_p_verify & neural_p_uncertainty on ContextPlan (C-4).
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let p2_bytes =
        std::fs::read(manifest_dir.join("tests/fixtures/neural_phase2_golden.safetensors"))
            .expect("read phase2 golden");
    let p2_model = Arc::new(NeuralModel::from_safetensors_bytes(&p2_bytes, None).expect("p2 model"));
    let mut core_p2 = Core::open(MemStore::new(), CoreConfig::default(), Some(p2_model)).unwrap();
    let mut p_p2 = make_percept("per_alice", "room:p2_live", 2_000, b"wake with phase2");
    p_p2.addressees = vec![ActorId::new("soul")];
    let mut doors_p2 = CaptureDoors::default();
    let eff = core_p2
        .step(
            Input::Percept(Box::new(p_p2)),
            Time::from_millis(2_000),
            &mut doors_p2,
        )
        .unwrap();
    let req_id = match eff.as_slice() {
        [monoagens_core::Effect::Model { plan, .. }] => {
            assert!(plan.neural_p_verify.is_some());
            assert!(plan.neural_p_uncertainty.is_some());
            let pv = plan.neural_p_verify.unwrap();
            let pu = plan.neural_p_uncertainty.unwrap();
            assert!((0.0..=1.0).contains(&pv) && pv.is_finite());
            assert!((0.0..=1.0).contains(&pu) && pu.is_finite());
            plan.trigger_event_id.unwrap()
        }
        other => panic!("expected Model effect, got {other:?}"),
    };

    // 3. Dispatching an action and receiving ReceiptOutcome::Done must NOT prematurely reward Go +0.5 (C-10).
    let place = PlaceId::new("room:p2_live");
    let eff_act1 = core_p2
        .step(
            Input::ModelResult {
                request_id: req_id,
                acts: vec![monoagens_protocol::ThoughtAct::Act {
                    capability_id: monoagens_protocol::CapabilityId::new("chat.speak"),
                    places: vec![place.clone()],
                    people: vec![ActorId::new("per_alice")],
                    payload: OpaquePayload::from_bytes(b"reply"),
                    motivation: 4,
                    confidence_0_20: 16,
                    used_ids: vec![req_id],
                    expected: None,
                    domain: None,
                }],
            },
            Time::from_millis(2_100),
            &mut NoopDoors,
        )
        .unwrap();
    let action_id_1 = match eff_act1.as_slice() {
        [monoagens_core::Effect::Dispatch(a)] => a.action_id.clone(),
        other => panic!("expected Dispatch 1, got {other:?}"),
    };
    core_p2
        .step(
            Input::Receipt(monoagens_protocol::Receipt {
                action_id: action_id_1,
                capability_id: monoagens_protocol::CapabilityId::new("chat.speak"),
                outcome: monoagens_protocol::ReceiptOutcome::Done,
                reason_code: None,
                detail: None,
                termination_reason: Some(monoagens_protocol::TerminationReason::Normal),
                progress: Some(1.0),
                delivered_units: Some(1),
                h_act: None,
                timestamp_ms: Time::from_millis(2_200),
            }),
            Time::from_millis(2_200),
            &mut NoopDoors,
        )
        .unwrap();
    assert!(
        core_p2
            .state()
            .place_adapt
            .get(&place)
            .is_none_or(|ad| ad.arm_pulls[monoagens_core::neural::GateArm::Go as usize] == 0),
        "ReceiptOutcome::Done must not prematurely reward GateArm::Go"
    );

    // 4. Non-fault Failed receipt records event-sourced negative Go feedback & replays bit-identically (C-3, C-10).
    let eff_act2 = core_p2
        .step(
            Input::ModelResult {
                request_id: req_id,
                acts: vec![monoagens_protocol::ThoughtAct::Act {
                    capability_id: monoagens_protocol::CapabilityId::new("chat.speak"),
                    places: vec![place.clone()],
                    people: vec![ActorId::new("per_alice")],
                    payload: OpaquePayload::from_bytes(b"reply2"),
                    motivation: 4,
                    confidence_0_20: 16,
                    used_ids: vec![req_id],
                    expected: None,
                    domain: None,
                }],
            },
            Time::from_millis(2_300),
            &mut NoopDoors,
        )
        .unwrap();
    let action_id_2 = match eff_act2.as_slice() {
        [monoagens_core::Effect::Dispatch(a)] => a.action_id.clone(),
        other => panic!("expected Dispatch 2, got {other:?}"),
    };
    core_p2
        .step(
            Input::Receipt(monoagens_protocol::Receipt {
                action_id: action_id_2,
                capability_id: monoagens_protocol::CapabilityId::new("chat.speak"),
                outcome: monoagens_protocol::ReceiptOutcome::Failed,
                reason_code: Some("timeout".to_owned()),
                detail: None,
                termination_reason: Some(monoagens_protocol::TerminationReason::TimedOut),
                progress: None,
                delivered_units: None,
                h_act: None,
                timestamp_ms: Time::from_millis(2_400),
            }),
            Time::from_millis(2_400),
            &mut NoopDoors,
        )
        .unwrap();

    let live_adapt = core_p2
        .state()
        .place_adapt
        .get(&place)
        .cloned()
        .expect("place_adapt populated on failure");
    assert_eq!(
        live_adapt.arm_pulls[monoagens_core::neural::GateArm::Go as usize],
        1
    );
    assert!(
        live_adapt.film_scale_deltas[0].abs() > 0.0,
        "non-zero context_hint must move FiLM scale deltas"
    );

    let rep = replay_log(core_p2.store(), ReplayMode::Pinned, None).expect("replay");
    assert!(rep.is_exact());
    assert_eq!(
        rep.state.place_adapt.get(&place),
        Some(&live_adapt),
        "event-sourced PlaceAdaptState must replay bit-identically"
    );
}
