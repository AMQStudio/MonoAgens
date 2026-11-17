use monoagens_core::gate::GateSignals;
use monoagens_core::{
    CapabilityCatalogEntry, Core, CoreSelfCard, Doors, EventDetail, EventKind, EventRecord,
    ExtractProcedureParams, FailureEpisodeRecord, Input, LearningOutcomeKind, LearningSample,
    MechanismId, MemStore, Prior, ProcedureLifecycleState, ProcedureMonitorOutcome, ReplayMode,
    SkillBook, Store, SuccessEpisodeRecord, Switches, ToolSelectionBudget, calibrate_wake_gate_lo,
    clip_ips_weight, compute_procedure_schema_hash, select_tools_for_context,
};
use monoagens_protocol::{
    ActionId, ActorId, Audience, BodyId, BodyState, BodyStatus, CapabilityDescriptor,
    CapabilityFlags, CapabilityId, CapabilitySchemaHash, ChannelId, DomainId, EventId, ExpectKind,
    OpaquePayload, OriginActor, OwnerSigned, Percept, PlaceId, PromiseMeta, Receipt,
    ReceiptOutcome, Taint, TerminationReason, Time, VisibleScope,
};

#[derive(Default)]
struct RecordingContextDoors {
    plans: Vec<monoagens_core::ContextPlan>,
}

impl Doors for RecordingContextDoors {
    fn inbound(&mut self, _percept: &mut Percept) -> monoagens_core::DoorDecision {
        monoagens_core::DoorDecision::Allow
    }

    fn context(&mut self, plan: &mut monoagens_core::ContextPlan) -> monoagens_core::DoorDecision {
        self.plans.push(plan.clone());
        monoagens_core::DoorDecision::Allow
    }

    fn action(&mut self, _action: &mut monoagens_protocol::Action) -> monoagens_core::DoorDecision {
        monoagens_core::DoorDecision::Allow
    }
}

fn make_cap(id: &str, schema_hash: &str) -> CapabilityDescriptor {
    CapabilityDescriptor {
        id: CapabilityId::new(id),
        description: format!("cap {id}"),
        places: vec![PlaceId::new("room:a"), PlaceId::new("room:b")],
        flags: CapabilityFlags {
            communicative: true,
            identity_changing: false,
            irreversible: false,
        },
        typical_latency_ms: 200,
        capability_schema_hash: CapabilitySchemaHash::new(schema_hash),
        sensed_effect: None,
    }
}

fn body_event(cap_name: &str, schema_hash: &str, ts_ms: u64) -> BodyState {
    BodyState {
        protocol_version: Default::default(),
        body_id: BodyId::new("b1"),
        status: BodyStatus::Ready,
        capabilities: vec![make_cap(cap_name, schema_hash)],
        timestamp_ms: Time::from_millis(ts_ms),
    }
}

#[test]
fn two_matching_errors_plus_contrast_produce_lesson_with_credence_and_single_error_does_not() {
    let prior = Prior::default();
    let switches = Switches::all_enabled();
    let mut core = Core::open(MemStore::new(), prior.clone(), switches, 7).expect("open core");

    // 1. Single failure episode must NOT form a lesson (< lesson_min_episodes = 2).
    core.record_failure_episode(FailureEpisodeRecord {
        episode_id: EventId::new(101),
        signature_key: "http_timeout:cap.fetch".to_owned(),
        domain: Some(DomainId::new("ops")),
        capability_id: Some(CapabilityId::new("cap.fetch")),
        place: PlaceId::new("room:a"),
        actors: vec![ActorId::new("alice")],
        taint: Taint::Trusted,
        timestamp_ms: Time::from_millis(1_000),
    });
    let first = core
        .cluster_contrastive_lessons(Time::from_millis(2_000))
        .expect("cluster 1");
    assert!(
        first.is_empty(),
        "single error episode must not emit a lesson"
    );

    // 2. Add 2nd matching failure and 3 contrastive successes -> lesson caps contrasts at `lesson_max_contrasts` (2).
    core.record_failure_episode(FailureEpisodeRecord {
        episode_id: EventId::new(102),
        signature_key: "http_timeout:cap.fetch".to_owned(),
        domain: Some(DomainId::new("ops")),
        capability_id: Some(CapabilityId::new("cap.fetch")),
        place: PlaceId::new("room:a"),
        actors: vec![ActorId::new("alice")],
        taint: Taint::Trusted,
        timestamp_ms: Time::from_millis(3_000),
    });
    for i in 0..3u64 {
        core.record_success_episode(SuccessEpisodeRecord {
            episode_id: EventId::new(200 + i),
            signature_key: "http_timeout:cap.fetch".to_owned(),
            domain: Some(DomainId::new("ops")),
            capability_id: Some(CapabilityId::new("cap.fetch")),
            place: PlaceId::new("room:a"),
            actors: vec![ActorId::new("alice")],
            taint: Taint::Trusted,
            timestamp_ms: Time::from_millis(3_500 + i * 100),
        });
    }

    let lessons = core
        .cluster_contrastive_lessons(Time::from_millis(4_000))
        .expect("cluster 2");
    assert_eq!(lessons.len(), 1);
    let lesson = &lessons[0];
    assert_eq!(lesson.failure_ids.len(), 2);
    assert_eq!(
        lesson.contrastive_success_ids.len(),
        prior.lesson_max_contrasts
    );
    assert!(lesson.credence.expected() > 0.5);

    let replay = core.replay_verify(ReplayMode::FullReplay).expect("replay");
    assert!(replay.first_divergence.is_none());
}

#[test]
fn cross_place_goal_extracts_procedure_and_schema_drift_retires_without_deleting() {
    let prior = Prior::default();
    let switches = Switches::all_enabled();
    let mut core = Core::open(MemStore::new(), prior.clone(), switches, 11).expect("open");
    let mut doors = RecordingContextDoors::default();
    core.step(
        Input::BodyState(body_event("cap.deploy", "schema_v1", 500)),
        Time::from_millis(500),
        &mut doors,
    )
    .expect("body v1");

    // Chat-habit bypass is disabled by default (`allow_chat_habit_bypass == false`).
    assert!(!SkillBook::chat_habit_bypass_allowed(&prior));

    // Single-place sequence does NOT qualify as a cross-place procedure.
    let single_place_hash = compute_procedure_schema_hash([(
        &CapabilityId::new("cap.deploy"),
        &CapabilitySchemaHash::new("schema_v1"),
    )]);
    let single = core.extract_procedure_from_goal_chain(ExtractProcedureParams {
        procedure_id: None,
        source_goal_id: Some(EventId::new(1)),
        name: "single_place_attempt".to_owned(),
        steps: vec!["deploy".to_owned()],
        required_cap_ids: vec![CapabilityId::new("cap.deploy")],
        capability_schema_hash: single_place_hash.clone(),
        places: vec![PlaceId::new("room:a")],
        actors: vec![],
        audience: Audience::new([ActorId::new("soul")]),
        domain: Some(DomainId::new("ops")),
        source_event_ids: vec![],
        taint: Taint::Trusted,
        now_ms: Time::from_millis(600),
    });
    assert!(
        single.is_none(),
        "single-place goal must not extract procedure"
    );

    // Cross-place goal (`room:a` + `room:b`) DOES extract an active procedure.
    core.extract_procedure_from_goal_chain(ExtractProcedureParams {
        procedure_id: Some(EventId::new(777)),
        source_goal_id: Some(EventId::new(2)),
        name: "cross_place_deploy".to_owned(),
        steps: vec!["fetch".to_owned(), "deploy".to_owned()],
        required_cap_ids: vec![CapabilityId::new("cap.deploy")],
        capability_schema_hash: single_place_hash,
        places: vec![PlaceId::new("room:a"), PlaceId::new("room:b")],
        actors: vec![],
        audience: Audience::new([ActorId::new("soul")]),
        domain: Some(DomainId::new("ops")),
        source_event_ids: vec![],
        taint: Taint::Trusted,
        now_ms: Time::from_millis(700),
    })
    .expect("cross-place procedure extracted");

    let caps: Vec<CapabilityCatalogEntry> =
        core.state().capability_catalog.values().cloned().collect();
    let procs: Vec<_> = core.state().procedures.values().cloned().collect();
    let scope = VisibleScope::new(Audience::new([ActorId::new("soul")]), None);
    let selected_before = select_tools_for_context(
        &[],
        &[],
        &caps,
        &procs,
        &scope,
        Time::from_millis(800),
        ToolSelectionBudget::from_prior(&prior, caps.len()),
    );
    assert_eq!(selected_before.procedures.len(), 1);
    assert_eq!(selected_before.excluded_stale_procedures, 0);

    // Schema drift arrives via BodyState -> procedure is RETIRED (not deleted!).
    core.step(
        Input::BodyState(body_event("cap.deploy", "schema_v2_breaking", 1_000)),
        Time::from_millis(1_000),
        &mut doors,
    )
    .expect("body v2");
    let stored_proc = core
        .state()
        .skill_book
        .procedures
        .get(&EventId::new(777))
        .expect("retired procedure MUST remain in history, never deleted");
    assert_eq!(stored_proc.state, ProcedureLifecycleState::Retired);
    assert_eq!(
        stored_proc.retired_reason.as_deref(),
        Some("capability_schema_drift")
    );

    let caps_after: Vec<CapabilityCatalogEntry> =
        core.state().capability_catalog.values().cloned().collect();
    let procs_after: Vec<_> = core.state().procedures.values().cloned().collect();
    let selected_after = select_tools_for_context(
        &[],
        &[],
        &caps_after,
        &procs_after,
        &scope,
        Time::from_millis(1_100),
        ToolSelectionBudget::from_prior(&prior, caps_after.len()),
    );
    assert!(selected_after.procedures.is_empty());
    assert_eq!(procs_after.len(), 1);
    assert!(procs_after[0].retired && !procs_after[0].active);
}

#[test]
fn cusum_accuracy_decay_retires_procedure_and_cancellations_do_not_penalize() {
    let prior = Prior::default();
    let switches = Switches::all_enabled();
    let mut core = Core::open(MemStore::new(), prior, switches, 13).expect("open");

    let proc_id = EventId::new(901);
    let schema = compute_procedure_schema_hash([(
        &CapabilityId::new("cap.exec"),
        &CapabilitySchemaHash::new("h1"),
    )]);
    core.extract_procedure_from_goal_chain(ExtractProcedureParams {
        procedure_id: Some(proc_id),
        source_goal_id: Some(EventId::new(50)),
        name: "exec_flow".to_owned(),
        steps: vec!["prepare".to_owned(), "exec".to_owned()],
        required_cap_ids: vec![CapabilityId::new("cap.exec")],
        capability_schema_hash: schema,
        places: vec![PlaceId::new("room:1"), PlaceId::new("room:2")],
        actors: vec![],
        audience: Audience::new([ActorId::new("soul")]),
        domain: None,
        source_event_ids: vec![],
        taint: Taint::Trusted,
        now_ms: Time::from_millis(100),
    })
    .expect("extract");

    // 5 initial successes lock baseline p0 (n_min_lock = 5).
    for i in 0..5u64 {
        let out = core.monitor_procedure_outcome(
            proc_id,
            ReceiptOutcome::Done,
            Some(TerminationReason::Normal),
            Time::from_millis(1_000 + i * 100),
        );
        assert!(matches!(out, ProcedureMonitorOutcome::Updated { .. }));
    }

    // CanceledByCore and PreemptedExternal MUST NOT penalize or advance CUSUM.
    for reason in [
        TerminationReason::CanceledByCore,
        TerminationReason::PreemptedExternal,
    ] {
        let ignored = core.monitor_procedure_outcome(
            proc_id,
            ReceiptOutcome::Failed,
            Some(reason),
            Time::from_millis(2_000),
        );
        assert_eq!(ignored, ProcedureMonitorOutcome::IgnoredCanceledOrPreempted);
    }
    let tp_pre = &core.state().skill_book.procedures[&proc_id];
    assert_eq!(tp_pre.trial_count, 5);
    assert_eq!(tp_pre.cusum_stat, 0.0);
    assert_eq!(tp_pre.state, ProcedureLifecycleState::Active);

    // Genuine accuracy decay drives Warned then Retired (and retains record).
    let mut saw_warned = false;
    let mut saw_retired = false;
    for i in 0..8u64 {
        let out = core.monitor_procedure_outcome(
            proc_id,
            ReceiptOutcome::Failed,
            Some(TerminationReason::Normal),
            Time::from_millis(3_000 + i * 100),
        );
        if let ProcedureMonitorOutcome::Updated { new_state, .. } = out {
            if new_state == ProcedureLifecycleState::Warned {
                saw_warned = true;
            }
            if new_state == ProcedureLifecycleState::Retired {
                saw_retired = true;
                break;
            }
        }
    }
    assert!(saw_warned, "expected CUSUM warning before retirement");
    assert!(
        saw_retired,
        "expected CUSUM retirement on sustained failures"
    );
    let tp_post = &core.state().skill_book.procedures[&proc_id];
    assert_eq!(tp_post.state, ProcedureLifecycleState::Retired);
}

#[test]
fn promisee_aware_source_cards_cover_tick_and_wake_participants() {
    let prior = Prior {
        talkativeness: 1.0,
        ..Prior::default()
    };
    let switches = Switches::all_enabled();
    let mut core = Core::open(MemStore::new(), prior, switches, 17).expect("open");
    let mut doors = RecordingContextDoors::default();

    // Open an explicit promise expectation via log + checkpoint reopen so tick wakes directly on due promise.
    let exp_rec = EventRecord::new_root(
        EventKind::ExpectationOpened,
        Time::from_millis(1_000),
        ActorId::new("soul"),
        PlaceId::new("room:ops"),
        Audience::new([ActorId::new("soul")]),
        OriginActor::new(ActorId::new("soul")),
        Taint::Trusted,
        Default::default(),
        OpaquePayload::default(),
        Vec::new(),
        Some(EventDetail::ExpectationOpened {
            expectation_id: EventId::new(10_000),
            target_event_id: EventId::new(1),
            kind: ExpectKind::Wait,
            place: PlaceId::new("room:ops"),
            sender: ActorId::new("dana"),
            medium: ChannelId::new("chat"),
            due_ms: Time::from_millis(5_000),
            expiry_ms: Time::from_millis(10_000),
        }),
    );
    let _ = core.store_mut().append(exp_rec).expect("append wait");

    let prom_rec = EventRecord::new_root(
        EventKind::ExpectationOpened,
        Time::from_millis(1_100),
        ActorId::new("soul"),
        PlaceId::new("room:ops"),
        Audience::new([ActorId::new("soul")]),
        OriginActor::new(ActorId::new("soul")),
        Taint::Trusted,
        Default::default(),
        OpaquePayload::default(),
        Vec::new(),
        Some(EventDetail::ExpectationRecordOpened {
            expectation_id: EventId::new(10_001),
            origin_event_id: EventId::new(1),
            target_event_id: EventId::new(1),
            kind: ExpectKind::Promise,
            places: vec![PlaceId::new("room:ops")],
            people: vec![ActorId::new("dana")],
            domain: None,
            condition: "deliver report".to_owned(),
            condition_embeddings: Vec::new(),
            due_ms: Time::from_millis(2_000),
            expiry_ms: Time::from_millis(20_000),
            recurrence: None,
            promise_meta: Some(PromiseMeta {
                promisee_ids: vec![ActorId::new("charlie")],
                hard_deadline_ms: None,
                conflict_key: None,
                depends_on: None,
            }),
            urgency_weight: 0.8,
            action_id: None,
            capability_id: None,
            channel: None,
            lat_min_ms: 0,
            lat_max_ms: 0,
            delta_pred: None,
            sigma_channel: None,
            correlation_id: None,
            declared_sensor: false,
            act_embeddings: Vec::new(),
            expected_reply_embeddings: Vec::new(),
            is_judged_silence: false,
            silence_propensity: None,
            silence_sender: None,
            stakes: 0.5,
            medium: None,
            about_ids: Vec::new(),
            accept_done_receipt: false,
            expected_latency_ms: 0,
        }),
    );
    let _ = core.store_mut().append(prom_rec).expect("append prom");

    // Reopen from store so projection reflects both appended expectations cleanly without private state_mut.
    let store = core.store().clone();
    let mut core = Core::open(
        store,
        Prior {
            talkativeness: 1.0,
            ..Prior::default()
        },
        Switches::all_enabled(),
        17,
    )
    .expect("reopen");

    // Record a refuted told claim for charlie to verify ledger false_belief & reliability tracking.
    core.record_told_claim(
        &ActorId::new("charlie"),
        EventId::new(500),
        "server_healthy",
        Time::from_millis(1_200),
    );
    core.resolve_told_claim(
        &ActorId::new("charlie"),
        EventId::new(500),
        "server_healthy",
        false,
        Time::from_millis(1_300),
    );

    // Fire an Input::Tick when promise is due -> wake emits ContextPlan carrying both promisee `"charlie"` and `"dana"`.
    let _effects = core
        .step(Input::Tick, Time::from_millis(2_500), &mut doors)
        .expect("tick");
    let cards = &doors
        .plans
        .last()
        .expect("tick wake context plan")
        .sections
        .source_cards;
    let charlie_card = cards
        .iter()
        .find(|c| c.actor_id.as_str() == "charlie")
        .expect("promisee charlie MUST appear in tick source_cards");
    assert!(cards.iter().any(|c| c.actor_id.as_str() == "dana"));
    assert!(!cards.iter().any(|c| c.actor_id.as_str() == "soul"));
    assert!(charlie_card.open_requests_count >= 1);
    assert_eq!(charlie_card.false_belief_count, 1);
    assert!(charlie_card.reliability_beta > charlie_card.reliability_alpha);

    // Neutral `!switches.perception_ledger` omits source_cards (`absent`).
    let mut sw_off = Switches::all_enabled();
    sw_off.perception_ledger = false;
    let neutral_cards = core.build_source_cards_with_switches(&[ActorId::new("charlie")], &sw_off);
    assert!(neutral_cards.is_empty());
}

#[test]
fn four_pillar_self_card_and_recalibration_preserve_owner_signed_identity() {
    let prior = Prior {
        self_card_cap_min_trials: 3,
        self_card_cap_consec_fail: 3,
        ..Prior::default()
    };
    let switches = Switches::all_enabled();
    let mut core = Core::open(MemStore::new(), prior, switches, 19).expect("open");
    let mut doors = RecordingContextDoors::default();

    // Register immutable OwnerSigned identity anchor.
    let signed_id = OwnerSigned {
        payload: OpaquePayload::from_bytes(b"identity_blob_v1"),
        signer: ActorId::new("owner"),
        signed_at_ms: Time::from_millis(10),
        signature: "sig_owner".to_owned(),
    };
    core.step(
        Input::Identity(signed_id.clone()),
        Time::from_millis(10),
        &mut doors,
    )
    .expect("set identity");
    let anchor_before = core.state().identity_anchor.clone();

    // Pillar 1: Capability failure streak marks capability degraded & triggers recalibration.
    core.step(
        Input::BodyState(body_event("cap.net", "s1", 50)),
        Time::from_millis(50),
        &mut doors,
    )
    .expect("body");
    for i in 0..3u64 {
        core.step(
            Input::Receipt(Receipt {
                action_id: ActionId::new(format!("act-{i}")),
                capability_id: CapabilityId::new("cap.net"),
                outcome: ReceiptOutcome::Failed,
                reason_code: None,
                detail: None,
                termination_reason: Some(TerminationReason::Normal),
                progress: None,
                delivered_units: None,
                h_act: None,
                timestamp_ms: Time::from_millis(100 + i * 10),
            }),
            Time::from_millis(100 + i * 10),
            &mut doors,
        )
        .expect("failed receipt");
    }

    // Pillar 2: 2x5 calibration observations + shrunk confidence recalibration.
    for _ in 0..4 {
        core.record_calibration_outcome(18, false, Time::from_millis(200))
            .expect("cal");
    }
    core.record_calibration_outcome(18, true, Time::from_millis(205))
        .expect("cal");
    core.record_calibration_outcome(4, false, Time::from_millis(210))
        .expect("cal");
    assert!(core.recalibrated_confidence(18) < 0.90);

    // Pillar 3: Operational budget event.
    core.record_budget_event(25.0, 100.0, Time::from_millis(220))
        .expect("budget");

    let card: CoreSelfCard = core.state().build_self_card();
    assert_eq!(
        card.degraded_capabilities,
        vec![CapabilityId::new("cap.net")]
    );
    assert_eq!(card.calibration_samples, 6);
    assert!(card.calibration_ece >= 0.0);
    assert!((card.budget_headroom_ratio - 0.75).abs() < 1e-9);
    assert!(core.state().ledger.recalibration_count >= 1);

    // OwnerSigned identity MUST remain completely immutable across recalibration & capability changes!
    assert_eq!(core.state().identity.as_ref(), Some(&signed_id));
    assert_eq!(core.state().identity_anchor, anchor_before);

    let replay = core.replay_verify(ReplayMode::FullReplay).expect("replay");
    assert!(replay.first_divergence.is_none());
}

#[test]
fn bounded_learning_polarities_ips_clip_non_penalty_wake_gate_and_rollback() {
    let prior = Prior {
        learn_ips_clip_m: 10.0,
        learn_daily_max_delta: 0.05,
        learn_wake_gate_n0: 5.0,
        learn_wake_gate_quantile: 0.10,
        ..Prior::default()
    };
    let switches = Switches::all_enabled();
    let mut core = Core::open(MemStore::new(), prior.clone(), switches, 23).expect("open");

    let signals = GateSignals {
        own_name: 0.5,
        info_gap: 0.8,
        feeling_of_knowing: 0.4,
        relevance: 0.9,
        commitment: 0.6,
        social_cost: 0.5,
        ..GateSignals::default()
    };

    // 1. IPS clipping bounds at M_ips = 10.0 (`rho = 0.01 -> 10.0`, `rho = 0.25 -> 4.0`).
    assert!((clip_ips_weight(Some(0.01), prior.learn_ips_clip_m) - 10.0).abs() < 1e-9);
    assert!((clip_ips_weight(Some(0.25), prior.learn_ips_clip_m) - 4.0).abs() < 1e-9);

    // 2. External/core cancellation causes ZERO weight movement.
    let w_start = core.state().adaptive_weights;
    for reason in [
        TerminationReason::CanceledByCore,
        TerminationReason::PreemptedExternal,
    ] {
        let skipped = core
            .record_learning_outcome(&LearningSample {
                source_event_id: EventId::new(0),
                outcome_kind: LearningOutcomeKind::BadAct,
                prob_go: 0.5,
                signals: signals.clone(),
                sample_propensity: None,
                teacher_reliability: 1.0,
                termination_reason: Some(reason),
                timestamp_ms: Time::from_millis(1_000),
            })
            .expect("skip");
        assert!(!skipped.applied);
    }
    assert_eq!(core.state().adaptive_weights, w_start);

    // 3. Polarity 1: GoodAct -> weights UP, w_sc less negative, theta_0 DOWN.
    let good = core
        .record_learning_outcome(&LearningSample {
            source_event_id: EventId::new(10),
            outcome_kind: LearningOutcomeKind::GoodAct,
            prob_go: 0.5,
            signals: signals.clone(),
            sample_propensity: None,
            teacher_reliability: 1.0,
            termination_reason: Some(TerminationReason::Normal),
            timestamp_ms: Time::from_millis(2_000),
        })
        .expect("good");
    assert!(good.applied);
    assert!(good.delta_weights.w_info > 0.0);
    assert!(good.delta_weights.w_sc > 0.0);
    assert!(good.delta_theta_0 < 0.0);
    let w_after_good = core.state().adaptive_weights;

    // 4. Polarity 2: BadAct -> weights DOWN, theta_0 UP.
    let bad = core
        .record_learning_outcome(&LearningSample {
            source_event_id: EventId::new(11),
            outcome_kind: LearningOutcomeKind::BadAct,
            prob_go: 0.5,
            signals: signals.clone(),
            sample_propensity: None,
            teacher_reliability: 1.0,
            termination_reason: Some(TerminationReason::Normal),
            timestamp_ms: Time::from_millis(2_100),
        })
        .expect("bad");
    assert!(bad.applied);
    assert!(bad.delta_weights.w_info < 0.0);
    assert!(bad.delta_weights.w_sc < 0.0);
    assert!(bad.delta_theta_0 > 0.0);

    // 5. Exact epoch rollback restores bit-for-bit `w_after_good` (and then `w_start`).
    core.rollback_learning(bad.epoch_id, "revert_bad", Time::from_millis(2_200))
        .expect("rollback bad");
    assert_eq!(core.state().adaptive_weights, w_after_good);
    core.rollback_learning(None, "revert_good", Time::from_millis(2_300))
        .expect("rollback good");
    assert_eq!(core.state().adaptive_weights, w_start);

    // 6. Polarity 3: MissedSilence (clipped IPS) -> UP.
    let missed = core
        .record_learning_outcome(&LearningSample {
            source_event_id: EventId::new(12),
            outcome_kind: LearningOutcomeKind::MissedSilence,
            prob_go: 0.4,
            signals: signals.clone(),
            sample_propensity: Some(0.05),
            teacher_reliability: 1.0,
            termination_reason: None,
            timestamp_ms: Time::from_millis(3_000),
        })
        .expect("missed");
    assert!(missed.applied);
    assert!((missed.ips_weight - 10.0).abs() < 1e-9);
    assert!(missed.delta_weights.w_info > 0.0);
    assert!(missed.delta_theta_0 < 0.0);

    // Verify daily cap `<= learn_daily_max_delta` (0.05) per parameter across saturated steps!
    for idx in 0..10u64 {
        let _ = core
            .record_learning_outcome(&LearningSample {
                source_event_id: EventId::new(30 + idx),
                outcome_kind: LearningOutcomeKind::MissedSilence,
                prob_go: 0.1,
                signals: signals.clone(),
                sample_propensity: Some(0.01),
                teacher_reliability: 1.0,
                termination_reason: None,
                timestamp_ms: Time::from_millis(3_100 + idx),
            })
            .expect("saturated step");
    }
    for (&k, &used) in &core.state().learner.daily_usage_map() {
        assert!(
            used <= prior.learn_daily_max_delta + 1e-12,
            "daily cap violated for {k}: {used}"
        );
    }

    // 7. Polarity 4: AnsweredSilence (next day) -> DOWN.
    let answered = core
        .record_learning_outcome(&LearningSample {
            source_event_id: EventId::new(13),
            outcome_kind: LearningOutcomeKind::AnsweredSilence,
            prob_go: 0.6,
            signals,
            sample_propensity: Some(0.5),
            teacher_reliability: 1.0,
            termination_reason: None,
            timestamp_ms: Time::from_millis(86_400_000 + 1_000),
        })
        .expect("answered");
    assert!(answered.applied);
    assert!(answered.delta_weights.w_info < 0.0);
    assert!(answered.delta_theta_0 > 0.0);

    // 8. Shrunk q_0.10 wake_gate_lo calibration (`16.3`).
    let calibrated = calibrate_wake_gate_lo(
        &[0.70, 0.72, 0.78, 0.80, 0.85, 0.88, 0.90, 0.92, 0.95, 0.97],
        &prior,
    );
    let updated =
        core.calibrate_wake_gate_lo(&[0.70, 0.72, 0.78, 0.80, 0.85, 0.88, 0.90, 0.92, 0.95, 0.97]);
    assert!((calibrated - updated).abs() < 1e-12);
    assert!(updated > prior.wake_gate_lo && updated < 0.70);
}

#[test]
fn neutral_parity_loop_runs_single_loop_over_every_mechanism_switch() {
    assert_eq!(MechanismId::ALL.len(), 22);
    let prior = Prior::default();
    let mut doors = RecordingContextDoors::default();

    for id in MechanismId::ALL {
        let mut sw = Switches::all_enabled();
        match id {
            MechanismId::Salience => sw.salience = false,
            MechanismId::FloorCost => sw.floor_cost = false,
            MechanismId::Habituation => sw.habituation = false,
            MechanismId::DriveThreshold => sw.drive_threshold = false,
            MechanismId::FamiliarityReserve => sw.familiarity_reserve = false,
            MechanismId::StochasticGoNoGo => sw.stochastic_go_no_go = false,
            MechanismId::CommitmentBreaker => sw.commitment_breaker = false,
            MechanismId::Patience => sw.patience = false,
            MechanismId::ReportBarOnThought => sw.report_bar_on_thought = false,
            MechanismId::ActivationRecall => sw.activation_recall = false,
            MechanismId::VectorFade => sw.vector_fade = false,
            MechanismId::PredictionAndGrounding => sw.prediction_and_grounding = false,
            MechanismId::ProspectiveMemory => sw.prospective_memory = false,
            MechanismId::Credence => sw.credence = false,
            MechanismId::BoundedLearning => sw.bounded_learning = false,
            MechanismId::MemoryTruth => sw.memory_truth = false,
            MechanismId::ReportControl => sw.report_control = false,
            MechanismId::ConsolidationAndMerge => sw.consolidation_and_merge = false,
            MechanismId::PerceptionLedger => sw.perception_ledger = false,
            MechanismId::Agency => sw.agency = false,
            MechanismId::EventBoundaries => sw.event_boundaries = false,
            MechanismId::BacklogChunking => sw.backlog_chunking = false,
        }
        let mut core = Core::open(MemStore::new(), prior.clone(), sw, 99).expect("open");
        let p = Percept::minimal(
            ActorId::new("alice"),
            PlaceId::new("room:tck"),
            Audience::new([ActorId::new("alice"), ActorId::new("soul")]),
            Time::from_millis(1_000),
            OpaquePayload::from_bytes(b"neutral_probe"),
        );
        let _ = core
            .step(
                Input::Percept(Box::new(p)),
                Time::from_millis(1_000),
                &mut doors,
            )
            .expect("step");
        let replay = core.replay_verify(ReplayMode::FullReplay).expect("replay");
        assert!(
            replay.first_divergence.is_none(),
            "neutral switch replay diverged for {}",
            id.stable_id()
        );
    }
}

#[cfg(feature = "tck-hooks")]
#[test]
fn tck_hooks_feature_builds_and_verifies_neutral_parities_and_erasure() {
    use monoagens_core::tck_hooks::{
        TckHarness, run_neutral_parity_probe, switches_with_only_neutral,
        verify_all_22_neutral_parities, verify_hard_forget_erasure,
    };
    use monoagens_protocol::{EmbeddingTag, TaggedEmbedding};

    let prior = Prior::default();
    let reports = verify_all_22_neutral_parities(&prior, 99).expect("neutral 22 parity");
    assert_eq!(reports.len(), 23);
    for r in &reports {
        assert!(r.replay_ok, "replay failed for {}", r.stable_id);
        assert!(
            r.weights_unchanged_under_neutral_learning,
            "neutral learning moved weights for {}",
            r.stable_id
        );
        assert!(
            r.decision_count >= 3,
            "missing decisions for {}",
            r.stable_id
        );
    }

    let r_bl = run_neutral_parity_probe(
        Some(MechanismId::BoundedLearning),
        switches_with_only_neutral(MechanismId::BoundedLearning),
        &prior,
        5,
    )
    .expect("bl probe");
    assert!(r_bl.replay_ok && r_bl.weights_unchanged_under_neutral_learning);

    let mut h = TckHarness::open(prior, Switches::all_enabled(), 42).expect("open");
    h.register_default_body("chat.speak", "blake3:s0", 100)
        .expect("body");
    let _ = h
        .inject_percept(
            "eve",
            "room:tck",
            &["eve", "soul"],
            1_000,
            Some(TaggedEmbedding {
                tag: EmbeddingTag {
                    embed_model_ver: "m1".to_owned(),
                    dim_d: 3,
                    prompt_regime_id: "t_clf".to_owned(),
                },
                values: vec![1.0, 0.0, 0.0],
            }),
        )
        .expect("percept");
    let dump = h.dump().expect("dump");
    assert!(dump.total_events >= 2);
    assert!(dump.last_decision.is_some());

    let audit =
        verify_hard_forget_erasure(&mut h.core, &ActorId::new("eve"), 5_000).expect("hard forget");
    assert!(
        audit.is_clean(),
        "expected zero surviving eve data: {audit:?}"
    );
}
