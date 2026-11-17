//! Integration tests for C4: Expectations, recurrences, promises, predictions,
//! agency, content miss, judged silence, free-text condition bands, and goals.
//!
//! Runs strictly sans-IO (`MemStore`, zero sleeps, zero network, zero filesystem).

use monoagens_core::{
    Core, DecisionReason, DoorDecision, Doors, Effect, EventDetail, ExpectationLifecycleState,
    Input, MemStore, ModelPurpose, NoopDoors, Prior, Store, Switches, replay_log,
};
use monoagens_protocol::{
    Action, ActorId, Audience, BodyId, BodyState, BodyStatus, CapabilityDescriptor,
    CapabilityFlags, CapabilityId, CapabilitySchemaHash, CatchupPolicy, ChannelId,
    ClassificationVerdict, DomainId, EmbeddingTag, EventId, ExpectKind, HAct, OpaquePayload,
    Percept, PlaceId, PromiseMeta, Receipt, ReceiptOutcome, RecurrenceSchedule, RecurrenceSpec,
    ResolveOutcome, SensedEffect, TaggedEmbedding, TerminationReason, ThoughtAct, Time,
};

fn tag() -> EmbeddingTag {
    EmbeddingTag {
        embed_model_ver: "gemma2-256".to_owned(),
        dim_d: 3,
        prompt_regime_id: "default".to_owned(),
    }
}

fn emb(vals: [f32; 3]) -> TaggedEmbedding {
    TaggedEmbedding {
        tag: tag(),
        values: vals.to_vec(),
    }
}

fn make_percept(actor: &str, place: &str, audience: &[&str], ts_ms: u64) -> Percept {
    Percept::minimal(
        ActorId::new(actor),
        PlaceId::new(place),
        Audience::new(audience.iter().map(|s| ActorId::new(*s))),
        Time::from_millis(ts_ms),
        OpaquePayload::from_bytes(b"opaque"),
    )
}

struct PromiseCall<'a> {
    trigger_actor: &'a str,
    places: &'a [&'a str],
    people: &'a [&'a str],
    due_ms: u64,
    expiry_ms: u64,
    promise_meta: Option<PromiseMeta>,
    domain: Option<&'a str>,
    now_ms: u64,
}

fn open_promise_via_model<S: monoagens_core::Store>(
    core: &mut Core<S>,
    args: PromiseCall<'_>,
    doors: &mut dyn Doors,
) -> (EventId, Vec<Effect>) {
    let PromiseCall {
        trigger_actor,
        places,
        people,
        due_ms,
        expiry_ms,
        promise_meta,
        domain,
        now_ms,
    } = args;
    let mut p = make_percept(trigger_actor, places[0], &["per_alice", "soul"], now_ms);
    p.addressees = vec![ActorId::new("soul")];
    let eff = core
        .step(
            Input::Percept(Box::new(p)),
            Time::from_millis(now_ms),
            doors,
        )
        .unwrap();
    let req_id = match eff.as_slice() {
        [Effect::Model { plan, .. }] => plan.trigger_event_id.unwrap(),
        other => panic!("expected Model wake to open promise, got {other:?}"),
    };
    let acts = vec![ThoughtAct::Expect {
        expect_kind: ExpectKind::Promise,
        about_ids: vec![req_id],
        places: places.iter().map(|s| PlaceId::new(*s)).collect(),
        people: people.iter().map(|s| ActorId::new(*s)).collect(),
        condition: "deliver report".to_owned(),
        due_ms: Some(Time::from_millis(due_ms)),
        expiry_ms: Time::from_millis(expiry_ms),
        recurrence: None,
        promise_meta,
        domain: domain.map(DomainId::new),
    }];
    let out = core
        .step(
            Input::ModelResult {
                request_id: req_id,
                acts,
            },
            Time::from_millis(now_ms),
            doors,
        )
        .unwrap();
    let exp_id = *core
        .state()
        .expectation_book
        .expectations
        .keys()
        .max()
        .expect("expectation stored");
    (exp_id, out)
}

#[test]
fn multi_place_multi_person_promise_rides_across_places_and_revises_without_early_wake() {
    let prior = Prior::default();
    let mut sw = Switches::all_enabled();
    sw.stochastic_go_no_go = false;
    let mut core = Core::open(MemStore::new(), prior, sw, 1).unwrap();
    let mut doors = NoopDoors;

    let (exp_id, _) = open_promise_via_model(
        &mut core,
        PromiseCall {
            trigger_actor: "per_alice",
            places: &["room:ask", "room:report"],
            people: &["per_alice", "per_bob"],
            due_ms: 10_000,
            expiry_ms: 20_000,
            promise_meta: Some(PromiseMeta {
                promisee_ids: vec![ActorId::new("per_alice"), ActorId::new("per_bob")],
                hard_deadline_ms: Some(Time::from_millis(20_000)),
                conflict_key: None,
                depends_on: None,
            }),
            domain: Some("work"),
            now_ms: 1_000,
        },
        &mut doors,
    );

    let rec = core
        .state()
        .expectation_book
        .expectations
        .get(&exp_id)
        .expect("stored");
    assert_eq!(rec.places.len(), 2);
    assert_eq!(rec.people.len(), 2);
    assert_eq!(rec.domain, Some(DomainId::new("work")));

    // Fires no earlier than due.
    let early = core
        .step(Input::Tick, Time::from_millis(9_999), &mut doors)
        .unwrap();
    assert!(early.is_empty(), "reminder must never fire early");

    // Revise due time to 15_000 before 10_000 arrives -> old due time 10_000 must not fire.
    let mut p_rev = make_percept("per_alice", "room:report", &["per_alice", "soul"], 5_000);
    p_rev.addressees = vec![ActorId::new("soul")];
    let eff_rev = core
        .step(
            Input::Percept(Box::new(p_rev)),
            Time::from_millis(5_000),
            &mut doors,
        )
        .unwrap();
    let req_rev = match eff_rev.as_slice() {
        [Effect::Model { plan, .. }] => plan.trigger_event_id.unwrap(),
        _ => panic!("expected wake"),
    };
    core.step(
        Input::ModelResult {
            request_id: req_rev,
            acts: vec![ThoughtAct::Revise {
                target_id: exp_id,
                new_version: 2,
                condition: Some("deliver report later".to_owned()),
                due_ms: Some(Time::from_millis(15_000)),
                expiry_ms: Some(Time::from_millis(25_000)),
                places: vec![PlaceId::new("room:ask"), PlaceId::new("room:report")],
                people: vec![ActorId::new("per_alice"), ActorId::new("per_bob")],
                domain: Some(DomainId::new("work")),
            }],
        },
        Time::from_millis(5_000),
        &mut doors,
    )
    .unwrap();

    let at_old_due = core
        .step(Input::Tick, Time::from_millis(10_000), &mut doors)
        .unwrap();
    assert!(
        at_old_due.is_empty(),
        "old due time must not fire after revise"
    );

    let at_new_due = core
        .step(Input::Tick, Time::from_millis(15_000), &mut doors)
        .unwrap();
    assert!(
        matches!(
            at_new_due.as_slice(),
            [Effect::Model {
                purpose: ModelPurpose::Reason,
                ..
            }]
        ),
        "new due time must wake reasoning: {at_new_due:?}"
    );
}

#[test]
fn downtime_recovery_wakes_once_per_place_and_coalesces_recurring_monitors_without_storm() {
    let prior = Prior::default();
    let mut sw = Switches::all_enabled();
    sw.stochastic_go_no_go = false;
    let mut store = MemStore::new();
    let mut doors = NoopDoors;

    {
        let mut core = Core::open(store, prior.clone(), sw.clone(), 7).unwrap();
        let mut p1 = make_percept("per_alice", "room:a", &["per_alice", "soul"], 1_000);
        p1.addressees = vec![ActorId::new("soul")];
        let eff = core
            .step(
                Input::Percept(Box::new(p1)),
                Time::from_millis(1_000),
                &mut doors,
            )
            .unwrap();
        let req1 = match eff.as_slice() {
            [Effect::Model { plan, .. }] => plan.trigger_event_id.unwrap(),
            _ => panic!("expected wake"),
        };

        // Open two overdue promises in room:a, one in room:b, one coalesce_one monitor, one skip_to_next monitor.
        core.step(
            Input::ModelResult {
                request_id: req1,
                acts: vec![
                    ThoughtAct::Expect {
                        expect_kind: ExpectKind::Promise,
                        about_ids: vec![req1],
                        places: vec![PlaceId::new("room:a")],
                        people: vec![ActorId::new("per_alice")],
                        condition: "p_a1".to_owned(),
                        due_ms: Some(Time::from_millis(2_000)),
                        expiry_ms: Time::from_millis(50_000),
                        recurrence: None,
                        promise_meta: None,
                        domain: None,
                    },
                    ThoughtAct::Expect {
                        expect_kind: ExpectKind::Promise,
                        about_ids: vec![req1],
                        places: vec![PlaceId::new("room:a")],
                        people: vec![ActorId::new("per_alice")],
                        condition: "p_a2".to_owned(),
                        due_ms: Some(Time::from_millis(3_000)),
                        expiry_ms: Time::from_millis(50_000),
                        recurrence: None,
                        promise_meta: None,
                        domain: None,
                    },
                    ThoughtAct::Expect {
                        expect_kind: ExpectKind::Promise,
                        about_ids: vec![req1],
                        places: vec![PlaceId::new("room:b")],
                        people: vec![ActorId::new("per_bob")],
                        condition: "p_b1".to_owned(),
                        due_ms: Some(Time::from_millis(4_000)),
                        expiry_ms: Time::from_millis(50_000),
                        recurrence: None,
                        promise_meta: None,
                        domain: None,
                    },
                    ThoughtAct::Expect {
                        expect_kind: ExpectKind::Monitor,
                        about_ids: vec![req1],
                        places: vec![PlaceId::new("room:a")],
                        people: vec![],
                        condition: "poll_a".to_owned(),
                        due_ms: Some(Time::from_millis(5_000)),
                        expiry_ms: Time::from_millis(500_000),
                        recurrence: Some(RecurrenceSpec {
                            schedule: RecurrenceSchedule::IntervalMs(1_000),
                            catchup: CatchupPolicy::CoalesceOne,
                        }),
                        promise_meta: None,
                        domain: None,
                    },
                    ThoughtAct::Expect {
                        expect_kind: ExpectKind::Monitor,
                        about_ids: vec![req1],
                        places: vec![PlaceId::new("room:c")],
                        people: vec![],
                        condition: "poll_c".to_owned(),
                        due_ms: Some(Time::from_millis(5_000)),
                        expiry_ms: Time::from_millis(500_000),
                        recurrence: Some(RecurrenceSpec {
                            schedule: RecurrenceSchedule::IntervalMs(1_000),
                            catchup: CatchupPolicy::SkipToNext,
                        }),
                        promise_meta: None,
                        domain: None,
                    },
                ],
            },
            Time::from_millis(1_000),
            &mut doors,
        )
        .unwrap();
        core.take_snapshot("pre_stop").unwrap();
        store = core.store().clone();
    }

    // Reopen after long downtime (now = 25_500 -> 21 missed monitor slots!).
    let mut core = Core::open(store, prior, sw, 7).unwrap();
    let effects = core
        .step(Input::Tick, Time::from_millis(25_500), &mut doors)
        .unwrap();
    let reason_wakes: Vec<&PlaceId> = effects
        .iter()
        .filter_map(|e| match e {
            Effect::Model {
                purpose: ModelPurpose::Reason,
                plan,
            } => plan.place.as_ref(),
            _ => None,
        })
        .collect();
    // 2 promise places (room:a once despite 2 overdue promises, room:b once) + 1 coalesced monitor (room:a) = 3 wakes total, 0 for skip_to_next (room:c)!
    assert_eq!(
        reason_wakes.len(),
        3,
        "expected 3 wakes, got {reason_wakes:?}"
    );
    assert!(
        !reason_wakes.contains(&&PlaceId::new("room:c")),
        "skip_to_next monitor must not catch-up fire"
    );

    // Subsequent tick at 25_600 must NOT storm.
    let next_tick = core
        .step(Input::Tick, Time::from_millis(25_600), &mut doors)
        .unwrap();
    assert!(
        next_tick.is_empty(),
        "must not emit alarm storm after recovery"
    );
}

#[test]
fn backoff_retry_then_kept_late_never_conflated_with_kept() {
    let prior = Prior {
        promise_retry_base_ms: 1_000,
        promise_retry_max: 3,
        ..Prior::default()
    };
    let mut sw = Switches::all_enabled();
    sw.stochastic_go_no_go = false;
    let mut core = Core::open(MemStore::new(), prior, sw, 2).unwrap();
    let mut doors = NoopDoors;

    let (exp_id, _) = open_promise_via_model(
        &mut core,
        PromiseCall {
            trigger_actor: "per_alice",
            places: &["room:1"],
            people: &["per_alice"],
            due_ms: 5_000,
            expiry_ms: 50_000,
            promise_meta: None,
            domain: None,
            now_ms: 1_000,
        },
        &mut doors,
    );

    // Wake at due=5_000 and dispatch action act-1.
    let wake = core
        .step(Input::Tick, Time::from_millis(5_000), &mut doors)
        .unwrap();
    let req = match wake.as_slice() {
        [Effect::Model { plan, .. }] => plan.trigger_event_id.unwrap(),
        _ => panic!("expected wake"),
    };
    let dispatch = core
        .step(
            Input::ModelResult {
                request_id: req,
                acts: vec![ThoughtAct::Act {
                    capability_id: CapabilityId::new("chat.send"),
                    places: vec![PlaceId::new("room:1")],
                    people: vec![ActorId::new("per_alice")],
                    payload: OpaquePayload::from_bytes(b"reminder"),
                    motivation: 4,
                    confidence_0_20: 16,
                    used_ids: vec![exp_id],
                    expected: None,
                    domain: None,
                }],
            },
            Time::from_millis(5_000),
            &mut doors,
        )
        .unwrap();
    let act_id_1 = match dispatch.as_slice() {
        [Effect::Dispatch(a)] => a.action_id.clone(),
        other => panic!("expected Dispatch, got {other:?}"),
    };

    // Transient failure at 5_100 -> schedules retry 1 at 5_100 + 1_000 = 6_100.
    core.step(
        Input::Receipt(Receipt {
            action_id: act_id_1,
            capability_id: CapabilityId::new("chat.send"),
            outcome: ReceiptOutcome::Failed,
            reason_code: Some("transient".to_owned()),
            detail: None,
            termination_reason: Some(TerminationReason::Normal),
            progress: None,
            delivered_units: None,
            h_act: None,
            timestamp_ms: Time::from_millis(5_100),
        }),
        Time::from_millis(5_100),
        &mut doors,
    )
    .unwrap();

    // Tick before backoff (6_099) -> no retry yet.
    assert!(
        core.step(Input::Tick, Time::from_millis(6_099), &mut doors)
            .unwrap()
            .is_empty()
    );

    // Tick at 6_100 -> dispatches retry act-2!
    let retry_eff = core
        .step(Input::Tick, Time::from_millis(6_100), &mut doors)
        .unwrap();
    let act_id_2 = match retry_eff.as_slice() {
        [Effect::Dispatch(a)] => a.action_id.clone(),
        other => panic!("expected retry Dispatch, got {other:?}"),
    };

    // Succeeds at 6_200 > due_ms (5_000) -> recorded strictly as KeptLate.
    core.step(
        Input::Receipt(Receipt {
            action_id: act_id_2,
            capability_id: CapabilityId::new("chat.send"),
            outcome: ReceiptOutcome::Done,
            reason_code: None,
            detail: None,
            termination_reason: Some(TerminationReason::Normal),
            progress: Some(1.0),
            delivered_units: Some(1),
            h_act: None,
            timestamp_ms: Time::from_millis(6_200),
        }),
        Time::from_millis(6_200),
        &mut doors,
    )
    .unwrap();

    let exp = core
        .state()
        .expectation_book
        .expectations
        .get(&exp_id)
        .unwrap();
    assert_eq!(exp.state, ExpectationLifecycleState::KeptLate);
    assert_ne!(exp.state, ExpectationLifecycleState::Kept);
}

struct HoldWithHActDoors(HAct);

impl Doors for HoldWithHActDoors {
    fn inbound(&mut self, _percept: &mut Percept) -> DoorDecision {
        DoorDecision::Allow
    }
    fn context(&mut self, _plan: &mut monoagens_core::ContextPlan) -> DoorDecision {
        DoorDecision::Allow
    }
    fn action(&mut self, action: &mut Action) -> DoorDecision {
        action.h_act = Some(self.0.clone());
        DoorDecision::Hold {
            rule: "r2_approval".to_owned(),
            reason: "needs owner".to_owned(),
        }
    }
}

#[test]
fn held_parks_once_pinned_to_h_act_without_duplicate_prompts_or_retries() {
    let prior = Prior::default();
    let mut sw = Switches::all_enabled();
    sw.stochastic_go_no_go = false;
    let mut core = Core::open(MemStore::new(), prior, sw, 3).unwrap();
    let mut noop = NoopDoors;

    let (exp_id, _) = open_promise_via_model(
        &mut core,
        PromiseCall {
            trigger_actor: "per_alice",
            places: &["room:1"],
            people: &["per_alice"],
            due_ms: 2_000,
            expiry_ms: 50_000,
            promise_meta: None,
            domain: None,
            now_ms: 1_000,
        },
        &mut noop,
    );

    let wake = core
        .step(Input::Tick, Time::from_millis(2_000), &mut noop)
        .unwrap();
    let req = match wake.as_slice() {
        [Effect::Model { plan, .. }] => plan.trigger_event_id.unwrap(),
        _ => panic!("expected wake"),
    };
    let mut hold_doors = HoldWithHActDoors(HAct::new("blake3:deadbeef"));
    let eff = core
        .step(
            Input::ModelResult {
                request_id: req,
                acts: vec![ThoughtAct::Act {
                    capability_id: CapabilityId::new("fs.write"),
                    places: vec![PlaceId::new("room:1")],
                    people: vec![ActorId::new("per_alice")],
                    payload: OpaquePayload::from_bytes(b"write"),
                    motivation: 4,
                    confidence_0_20: 15,
                    used_ids: vec![exp_id],
                    expected: None,
                    domain: None,
                }],
            },
            Time::from_millis(2_000),
            &mut hold_doors,
        )
        .unwrap();
    assert!(eff.is_empty(), "held action must not dispatch");

    let exp = core
        .state()
        .expectation_book
        .expectations
        .get(&exp_id)
        .unwrap();
    assert_eq!(exp.state, ExpectationLifecycleState::PendingApproval);
    assert_eq!(exp.last_h_act, Some(HAct::new("blake3:deadbeef")));
    assert_eq!(exp.retry_count, 0);

    // Subsequent ticks must NOT emit duplicate prompts or retries.
    let log_len_before = core.store().len();
    assert!(
        core.step(Input::Tick, Time::from_millis(10_000), &mut noop)
            .unwrap()
            .is_empty()
    );
    assert!(
        core.step(Input::Tick, Time::from_millis(20_000), &mut noop)
            .unwrap()
            .is_empty()
    );
    assert_eq!(core.store().len(), log_len_before);
}

#[test]
fn preempted_external_and_canceled_by_core_stop_cleanly_with_zero_retries() {
    let prior = Prior::default();
    let mut sw = Switches::all_enabled();
    sw.stochastic_go_no_go = false;
    let mut core = Core::open(MemStore::new(), prior, sw, 4).unwrap();
    let mut doors = NoopDoors;

    let (exp_id, _) = open_promise_via_model(
        &mut core,
        PromiseCall {
            trigger_actor: "per_alice",
            places: &["room:1"],
            people: &["per_alice"],
            due_ms: 5_000,
            expiry_ms: 50_000,
            promise_meta: None,
            domain: None,
            now_ms: 1_000,
        },
        &mut doors,
    );
    let wake = core
        .step(Input::Tick, Time::from_millis(5_000), &mut doors)
        .unwrap();
    let req = match wake.as_slice() {
        [Effect::Model { plan, .. }] => plan.trigger_event_id.unwrap(),
        _ => panic!("expected wake"),
    };
    let dispatch = core
        .step(
            Input::ModelResult {
                request_id: req,
                acts: vec![ThoughtAct::Act {
                    capability_id: CapabilityId::new("speech.say"),
                    places: vec![PlaceId::new("room:1")],
                    people: vec![ActorId::new("per_alice")],
                    payload: OpaquePayload::from_bytes(b"long reply"),
                    motivation: 4,
                    confidence_0_20: 15,
                    used_ids: vec![exp_id],
                    expected: None,
                    domain: None,
                }],
            },
            Time::from_millis(5_000),
            &mut doors,
        )
        .unwrap();
    let act_id = match dispatch.as_slice() {
        [Effect::Dispatch(a)] => a.action_id.clone(),
        _ => panic!("expected dispatch"),
    };

    core.step(
        Input::Receipt(Receipt {
            action_id: act_id,
            capability_id: CapabilityId::new("speech.say"),
            outcome: ReceiptOutcome::Failed,
            reason_code: Some("barge_in".to_owned()),
            detail: None,
            termination_reason: Some(TerminationReason::PreemptedExternal),
            progress: Some(0.4),
            delivered_units: Some(3),
            h_act: None,
            timestamp_ms: Time::from_millis(5_200),
        }),
        Time::from_millis(5_200),
        &mut doors,
    )
    .unwrap();

    let exp = core
        .state()
        .expectation_book
        .expectations
        .get(&exp_id)
        .unwrap();
    assert_eq!(exp.state, ExpectationLifecycleState::Cancelled);
    assert_eq!(exp.delivered_units, Some(3));
    assert_eq!(exp.retry_count, 0);
    assert_eq!(exp.next_retry_at_ms, None);
    assert!(
        core.step(Input::Tick, Time::from_millis(60_000), &mut doors)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn promise_meta_conflict_flags_reasoning_and_depends_on_orders_execution() {
    let prior = Prior::default();
    let mut sw = Switches::all_enabled();
    sw.stochastic_go_no_go = false;
    let mut core = Core::open(MemStore::new(), prior, sw, 5).unwrap();
    let mut doors = NoopDoors;

    // 1. Open promise A with conflict_key = "exclusive:calendar:10am".
    let (p_a, _) = open_promise_via_model(
        &mut core,
        PromiseCall {
            trigger_actor: "per_alice",
            places: &["room:1"],
            people: &["per_alice"],
            due_ms: 10_000,
            expiry_ms: 30_000,
            promise_meta: Some(PromiseMeta {
                promisee_ids: vec![ActorId::new("per_alice")],
                hard_deadline_ms: Some(Time::from_millis(30_000)),
                conflict_key: Some("exclusive:calendar:10am".to_owned()),
                depends_on: None,
            }),
            domain: None,
            now_ms: 1_000,
        },
        &mut doors,
    );

    // 2. Propose conflicting promise B sharing conflict_key -> conflict flagged to reasoning, NOT committed!
    let (_after_id, conflict_eff) = open_promise_via_model(
        &mut core,
        PromiseCall {
            trigger_actor: "per_bob",
            places: &["room:1"],
            people: &["per_bob"],
            due_ms: 10_000,
            expiry_ms: 30_000,
            promise_meta: Some(PromiseMeta {
                promisee_ids: vec![ActorId::new("per_bob")],
                hard_deadline_ms: Some(Time::from_millis(30_000)),
                conflict_key: Some("exclusive:calendar:10am".to_owned()),
                depends_on: None,
            }),
            domain: None,
            now_ms: 2_000,
        },
        &mut doors,
    );
    assert!(
        matches!(
            conflict_eff.as_slice(),
            [Effect::Model {
                purpose: ModelPurpose::Reason,
                ..
            }]
        ),
        "conflict must wake reasoning for renegotiation: {conflict_eff:?}"
    );
    let log = core.store().read_in_order(None, 100).unwrap();
    assert!(log.iter().any(|r| matches!(
        &r.detail,
        Some(EventDetail::ConflictFlagged {
            conflicting_expectation_id,
            conflict_key: Some(k),
            ..
        }) if *conflicting_expectation_id == p_a && k == "exclusive:calendar:10am"
    )));

    // 3. Open dependent promise C with depends_on = Some(p_a) and try to Act on C while p_a is open -> deferred!
    let (p_c, _) = open_promise_via_model(
        &mut core,
        PromiseCall {
            trigger_actor: "per_bob",
            places: &["room:2"],
            people: &["per_bob"],
            due_ms: 20_000,
            expiry_ms: 40_000,
            promise_meta: Some(PromiseMeta {
                promisee_ids: vec![ActorId::new("per_bob")],
                hard_deadline_ms: Some(Time::from_millis(40_000)),
                conflict_key: Some("step_2".to_owned()),
                depends_on: Some(p_a),
            }),
            domain: None,
            now_ms: 3_000,
        },
        &mut doors,
    );
    let deferred = core
        .step(
            Input::ModelResult {
                request_id: p_c,
                acts: vec![ThoughtAct::Act {
                    capability_id: CapabilityId::new("chat.send"),
                    places: vec![PlaceId::new("room:2")],
                    people: vec![ActorId::new("per_bob")],
                    payload: OpaquePayload::from_bytes(b"after A"),
                    motivation: 4,
                    confidence_0_20: 15,
                    used_ids: vec![p_c],
                    expected: None,
                    domain: None,
                }],
            },
            Time::from_millis(3_500),
            &mut doors,
        )
        .unwrap();
    assert!(
        deferred.is_empty(),
        "dependent act must NOT dispatch while dependency p_a is open"
    );

    // Resolve p_a -> dependent act for p_c immediately unblocks and dispatches!
    let unblocked = core
        .step(
            Input::ModelResult {
                request_id: p_a,
                acts: vec![ThoughtAct::Resolve {
                    expectation_id: p_a,
                    outcome: ResolveOutcome::Kept,
                    note: None,
                    places: vec![PlaceId::new("room:1")],
                    people: vec![ActorId::new("per_alice")],
                    domain: None,
                }],
            },
            Time::from_millis(4_000),
            &mut doors,
        )
        .unwrap();
    assert!(
        matches!(unblocked.as_slice(), [Effect::Dispatch(a)] if a.places == vec![PlaceId::new("room:2")]),
        "resolving p_a must dispatch deferred dependent act: {unblocked:?}"
    );
}

#[test]
fn promise_debt_remains_undiluted_with_ten_distant_promises() {
    let prior = Prior::default(); // H_debt = 3_600_000 ms (1 hour)
    let mut sw = Switches::all_enabled();
    sw.stochastic_go_no_go = false;
    let mut core = Core::open(MemStore::new(), prior.clone(), sw, 6).unwrap();
    let mut doors = NoopDoors;

    // 1 imminent promise due in 10 minutes.
    open_promise_via_model(
        &mut core,
        PromiseCall {
            trigger_actor: "per_alice",
            places: &["room:1"],
            people: &["per_alice"],
            due_ms: 601_000,
            expiry_ms: 700_000,
            promise_meta: None,
            domain: None,
            now_ms: 1_000,
        },
        &mut doors,
    );
    let debt_one = monoagens_core::gate::compute_promise_debt(
        &core
            .state()
            .open_promises
            .values()
            .cloned()
            .collect::<Vec<_>>(),
        Time::from_millis(1_000),
        prior.promise_debt_horizon_ms,
    );
    assert!((debt_one - 1.0).abs() < 1e-9);

    // Add 10 distant promises due next week.
    for i in 0..10 {
        let next_week = 7 * 86_400_000 + i * 1_000;
        open_promise_via_model(
            &mut core,
            PromiseCall {
                trigger_actor: "per_alice",
                places: &["room:1"],
                people: &["per_alice"],
                due_ms: next_week,
                expiry_ms: next_week + 10_000,
                promise_meta: None,
                domain: None,
                now_ms: 2_000 + i,
            },
            &mut doors,
        );
    }
    let debt_with_distant = monoagens_core::gate::compute_promise_debt(
        &core
            .state()
            .open_promises
            .values()
            .cloned()
            .collect::<Vec<_>>(),
        Time::from_millis(2_010),
        prior.promise_debt_horizon_ms,
    );
    assert!(
        (debt_with_distant - 1.0).abs() < 1e-9,
        "10 distant promises must not dilute urgent promise_debt: {debt_with_distant}"
    );
}

#[test]
fn response_prediction_answered_expired_content_miss_and_slow_place_duration() {
    let prior = Prior {
        content_miss_min_samples: 3,
        content_miss_z_threshold: 2.0,
        ..Prior::default()
    };
    let mut sw = Switches::all_enabled();
    sw.stochastic_go_no_go = false;
    let mut core = Core::open(MemStore::new(), prior, sw, 8).unwrap();
    let mut doors = NoopDoors;

    // Register body with slow_room typical latency = 5_000 ms.
    core.step(
        Input::BodyState(BodyState {
            protocol_version: Default::default(),
            body_id: BodyId::new("chat_body"),
            status: BodyStatus::Ready,
            capabilities: vec![CapabilityDescriptor {
                id: CapabilityId::new("chat.send"),
                description: "send chat".to_owned(),
                places: vec![PlaceId::new("room:slow")],
                flags: CapabilityFlags {
                    communicative: true,
                    identity_changing: false,
                    irreversible: false,
                },
                typical_latency_ms: 5_000,
                capability_schema_hash: CapabilitySchemaHash::new("h1"),
                sensed_effect: None,
            }],
            timestamp_ms: Time::from_millis(100),
        }),
        Time::from_millis(100),
        &mut doors,
    )
    .unwrap();

    // Helper: send a prompt to soul, wake, dispatch act with embedding [1,0,0], then reply.
    let mut t = 1_000u64;
    for _ in 0..3 {
        let mut p = make_percept("per_alice", "room:slow", &["per_alice", "soul"], t);
        p.addressees = vec![ActorId::new("soul")];
        p.embeddings = vec![emb([1.0, 0.0, 0.0])];
        let eff = core
            .step(
                Input::Percept(Box::new(p)),
                Time::from_millis(t),
                &mut doors,
            )
            .unwrap();
        let req = match eff.as_slice() {
            [Effect::Model { plan, .. }] => plan.trigger_event_id.unwrap(),
            _ => panic!("expected wake"),
        };
        let disp = core
            .step(
                Input::ModelResult {
                    request_id: req,
                    acts: vec![ThoughtAct::Act {
                        capability_id: CapabilityId::new("chat.send"),
                        places: vec![PlaceId::new("room:slow")],
                        people: vec![ActorId::new("per_alice")],
                        payload: OpaquePayload::from_bytes(b"explain"),
                        motivation: 4,
                        confidence_0_20: 15,
                        used_ids: vec![req],
                        expected: None,
                        domain: None,
                    }],
                },
                Time::from_millis(t),
                &mut doors,
            )
            .unwrap();
        let act_id = match disp.as_slice() {
            [Effect::Dispatch(a)] => a.action_id.clone(),
            _ => panic!("expected dispatch"),
        };
        core.set_response_embeddings(&act_id, vec![emb([1.0, 0.0, 0.0])], vec![]);

        // Ordinary reply (close in vector space, similarity ~ 0.99 -> dist ~ 0.01).
        t += 100;
        let mut rep = make_percept("per_alice", "room:slow", &["per_alice", "soul"], t);
        rep.embeddings = vec![emb([0.99, 0.14, 0.0])];
        let rep_eff = core
            .step(
                Input::Percept(Box::new(rep)),
                Time::from_millis(t),
                &mut doors,
            )
            .unwrap();
        assert!(
            matches!(
                rep_eff.as_slice(),
                [Effect::Model {
                    purpose: ModelPurpose::Reason,
                    ..
                }]
            ),
            "reply must wake reasoning as commitment"
        );
        t += 1_000;
    }

    // Now dispatch one more act and get a far reply ([0.0, 1.0, 0.0], dist = 1.0) -> ContentMiss!
    let mut p_last = make_percept("per_alice", "room:slow", &["per_alice", "soul"], t);
    p_last.addressees = vec![ActorId::new("soul")];
    let eff = core
        .step(
            Input::Percept(Box::new(p_last)),
            Time::from_millis(t),
            &mut doors,
        )
        .unwrap();
    let req = match eff.as_slice() {
        [Effect::Model { plan, .. }] => plan.trigger_event_id.unwrap(),
        _ => panic!("expected wake"),
    };
    let disp = core
        .step(
            Input::ModelResult {
                request_id: req,
                acts: vec![ThoughtAct::Act {
                    capability_id: CapabilityId::new("chat.send"),
                    places: vec![PlaceId::new("room:slow")],
                    people: vec![ActorId::new("per_alice")],
                    payload: OpaquePayload::from_bytes(b"explain again"),
                    motivation: 4,
                    confidence_0_20: 15,
                    used_ids: vec![],
                    expected: None,
                    domain: None,
                }],
            },
            Time::from_millis(t),
            &mut doors,
        )
        .unwrap();
    let act_id = match disp.as_slice() {
        [Effect::Dispatch(a)] => a.action_id.clone(),
        _ => panic!("expected dispatch"),
    };
    core.set_response_embeddings(&act_id, vec![emb([1.0, 0.0, 0.0])], vec![]);

    let mut far_rep = make_percept("per_alice", "room:slow", &["per_alice", "soul"], t + 50);
    far_rep.embeddings = vec![emb([0.0, 1.0, 0.0])];
    let miss_wake = core
        .step(
            Input::Percept(Box::new(far_rep)),
            Time::from_millis(t + 50),
            &mut doors,
        )
        .unwrap();
    assert!(matches!(
        miss_wake.as_slice(),
        [Effect::Model {
            purpose: ModelPurpose::Reason,
            ..
        }]
    ));

    let log = core.store().read_in_order(None, 500).unwrap();
    assert!(log.iter().any(|r| matches!(
        &r.detail,
        Some(EventDetail::ContentMiss { who, distance, z_score, .. })
            if *who == ActorId::new("per_alice") && *distance > 0.9 && *z_score > 2.0
    )));

    // Expiry case: dispatch another act and never reply -> Expired noticed absence.
    let mut p_exp = make_percept("per_bob", "room:slow", &["per_bob", "soul"], t + 5_000);
    p_exp.addressees = vec![ActorId::new("soul")];
    let eff_e = core
        .step(
            Input::Percept(Box::new(p_exp)),
            Time::from_millis(t + 5_000),
            &mut doors,
        )
        .unwrap();
    let req_e = match eff_e.as_slice() {
        [Effect::Model { plan, .. }] => plan.trigger_event_id.unwrap(),
        _ => panic!("expected wake"),
    };
    core.step(
        Input::ModelResult {
            request_id: req_e,
            acts: vec![ThoughtAct::Act {
                capability_id: CapabilityId::new("chat.send"),
                places: vec![PlaceId::new("room:slow")],
                people: vec![ActorId::new("per_bob")],
                payload: OpaquePayload::from_bytes(b"ping"),
                motivation: 4,
                confidence_0_20: 15,
                used_ids: vec![],
                expected: None,
                domain: None,
            }],
        },
        Time::from_millis(t + 5_000),
        &mut doors,
    )
    .unwrap();
    let due = core.next_due().unwrap();
    core.step(Input::Tick, due, &mut doors).unwrap();
    let log2 = core.store().read_in_order(None, 500).unwrap();
    assert!(log2.iter().any(|r| matches!(
        &r.detail,
        Some(EventDetail::ExpectationTransition {
            to_state: ExpectationLifecycleState::Expired,
            ..
        })
    )));
}

#[test]
fn agency_self_caused_echo_vs_person_reply_vs_unrelated_sensor_change() {
    let prior = Prior::default();
    let mut sw = Switches::all_enabled();
    sw.stochastic_go_no_go = false;
    let mut core = Core::open(MemStore::new(), prior, sw, 9).unwrap();
    let mut doors = NoopDoors;

    // 1. Unrelated sensor change with no open effect prediction -> external and appraised.
    let mut p_ext = make_percept("sensor:door", "hall", &["soul"], 1_000);
    p_ext.channel = Some(ChannelId::new("door_angle"));
    p_ext.delta_obs = Some(45.0);
    core.step(
        Input::Percept(Box::new(p_ext)),
        Time::from_millis(1_000),
        &mut doors,
    )
    .unwrap();
    let log = core.store().read_in_order(None, 100).unwrap();
    assert!(log.iter().any(|r| matches!(
        &r.detail,
        Some(EventDetail::Decision {
            reason: Some(r_reason),
            ..
        }) if *r_reason != DecisionReason::SelfCaused
    )));

    // 2. Soul acts on door actuator with SensedEffect(door_angle, window=2000, noise=1.0, delta_pred=90.0).
    let mut p_cmd = make_percept("per_alice", "hall", &["per_alice", "soul"], 2_000);
    p_cmd.addressees = vec![ActorId::new("soul")];
    let eff = core
        .step(
            Input::Percept(Box::new(p_cmd)),
            Time::from_millis(2_000),
            &mut doors,
        )
        .unwrap();
    let req = match eff.as_slice() {
        [Effect::Model { plan, .. }] => plan.trigger_event_id.unwrap(),
        _ => panic!("expected wake"),
    };
    core.step(
        Input::ModelResult {
            request_id: req,
            acts: vec![ThoughtAct::Act {
                capability_id: CapabilityId::new("door.open"),
                places: vec![PlaceId::new("hall")],
                people: vec![ActorId::new("per_alice")],
                payload: OpaquePayload::from_bytes(b"open"),
                motivation: 4,
                confidence_0_20: 15,
                used_ids: vec![],
                expected: Some(SensedEffect {
                    channel: ChannelId::new("door_angle"),
                    window_ms: 2_000,
                    noise: 1.0,
                    delta_pred: 90.0,
                }),
                domain: None,
            }],
        },
        Time::from_millis(2_000),
        &mut doors,
    )
    .unwrap();

    // 3. Sensor reports delta_obs = 91.2 within tolerance (|91.2 - 90.0| <= 2*1.0) -> self_caused, NO reasoning!
    let mut p_sensor = make_percept("sensor:door", "hall", &["soul"], 2_500);
    p_sensor.channel = Some(ChannelId::new("door_angle"));
    p_sensor.delta_obs = Some(91.2);
    let sensor_eff = core
        .step(
            Input::Percept(Box::new(p_sensor)),
            Time::from_millis(2_500),
            &mut doors,
        )
        .unwrap();
    assert!(
        sensor_eff.is_empty(),
        "self-caused sensor effect must cause no reasoning: {sensor_eff:?}"
    );

    // 4. Person replies in hall -> NEVER self_caused, wakes reasoning!
    let mut p_reply = make_percept("per_alice", "hall", &["per_alice", "soul"], 2_800);
    p_reply.addressees = vec![ActorId::new("soul")];
    let reply_eff = core
        .step(
            Input::Percept(Box::new(p_reply)),
            Time::from_millis(2_800),
            &mut doors,
        )
        .unwrap();
    assert!(
        matches!(
            reply_eff.as_slice(),
            [Effect::Model {
                purpose: ModelPurpose::Reason,
                ..
            }]
        ),
        "person's reply must never be self_caused and must wake reasoning"
    );

    let log_end = core.store().read_in_order(None, 200).unwrap();
    assert!(log_end.iter().any(|r| matches!(
        &r.detail,
        Some(EventDetail::Decision {
            reason: Some(DecisionReason::SelfCaused),
            ..
        })
    )));
}

#[test]
fn judged_silence_seeded_sample_reproducible_missed_reask_and_sibling_answer() {
    let prior = Prior {
        silence_sample_rate: 0.25,
        ..Prior::default()
    };
    let sw = Switches::all_enabled();

    // 1. Seeded sampling across 40 silent/note events is reproducible on replay (no wall clock).
    let mut core1 = Core::open(MemStore::new(), prior.clone(), sw.clone(), 42).unwrap();
    let mut doors = NoopDoors;
    for i in 0..40u64 {
        let mut p = make_percept(
            "per_alice",
            "room:1",
            &["per_alice", "per_bob", "soul"],
            1_000 + i * 100,
        );
        p.embeddings = vec![emb([0.0, 0.0, 1.0])];
        core1
            .step(
                Input::Percept(Box::new(p)),
                Time::from_millis(1_000 + i * 100),
                &mut doors,
            )
            .unwrap();
    }
    let silences1: Vec<EventId> = core1
        .state()
        .expectation_book
        .expectations
        .values()
        .filter(|e| {
            e.response_spec
                .as_ref()
                .is_some_and(|s| s.is_judged_silence)
        })
        .map(|e| e.expectation_id)
        .collect();
    assert!(!silences1.is_empty() && silences1.len() < 40);

    let mut core2 = Core::open(MemStore::new(), prior.clone(), sw.clone(), 42).unwrap();
    for i in 0..40u64 {
        let mut p = make_percept(
            "per_alice",
            "room:1",
            &["per_alice", "per_bob", "soul"],
            1_000 + i * 100,
        );
        p.embeddings = vec![emb([0.0, 0.0, 1.0])];
        core2
            .step(
                Input::Percept(Box::new(p)),
                Time::from_millis(1_000 + i * 100),
                &mut doors,
            )
            .unwrap();
    }
    let silences2: Vec<EventId> = core2
        .state()
        .expectation_book
        .expectations
        .values()
        .filter(|e| {
            e.response_spec
                .as_ref()
                .is_some_and(|s| s.is_judged_silence)
        })
        .map(|e| e.expectation_id)
        .collect();
    assert_eq!(
        silences1, silences2,
        "seeded silence sample must be deterministic"
    );

    // Verify Two-Epoch Replay bit-exactness across the log.
    let rep = replay_log(core1.store(), &sw, 42).unwrap();
    assert!(
        rep.is_bit_identical(),
        "replay diverged: {:?}",
        rep.first_divergence
    );

    // 2. Re-ask by sender triggers ONE Classify -> confirmed -> missed!
    let prior_full = Prior {
        silence_sample_rate: 1.0,
        ..Prior::default()
    };
    let mut sw_det = Switches::all_enabled();
    sw_det.stochastic_go_no_go = false;
    let mut core_r = Core::open(MemStore::new(), prior_full, sw_det, 11).unwrap();

    let mut q1 = make_percept(
        "per_alice",
        "room:2",
        &["per_alice", "per_bob", "soul"],
        1_000,
    );
    q1.embeddings = vec![emb([1.0, 0.0, 0.0])];
    core_r
        .step(
            Input::Percept(Box::new(q1)),
            Time::from_millis(1_000),
            &mut doors,
        )
        .unwrap();

    let mut q_again = make_percept(
        "per_alice",
        "room:2",
        &["per_alice", "per_bob", "soul"],
        2_000,
    );
    q_again.embeddings = vec![emb([0.98, 0.19, 0.0])];
    let clf_eff = core_r
        .step(
            Input::Percept(Box::new(q_again)),
            Time::from_millis(2_000),
            &mut doors,
        )
        .unwrap();
    let clf_req = match clf_eff.as_slice() {
        [
            Effect::Model {
                purpose: ModelPurpose::Classify,
                plan,
            },
        ] => plan.trigger_event_id.unwrap(),
        other => panic!("expected one Classify call on re-ask, got {other:?}"),
    };
    core_r
        .step(
            Input::ClassifyResult {
                request_id: clf_req,
                verdict: ClassificationVerdict {
                    protocol_version: Default::default(),
                    label: "confirmed".to_owned(),
                    score: Some(0.95),
                    rationale: None,
                },
            },
            Time::from_millis(2_100),
            &mut doors,
        )
        .unwrap();
    let log_r = core_r.store().read_in_order(None, 200).unwrap();
    assert!(log_r.iter().any(|r| matches!(
        &r.detail,
        Some(EventDetail::SilenceJudged {
            missed: true,
            reason,
            ..
        }) if reason == "reask_confirmed"
    )));

    // 3. Answered by sibling participant -> missed = false!
    let mut core_sib = Core::open(
        MemStore::new(),
        Prior {
            silence_sample_rate: 1.0,
            ..Prior::default()
        },
        Switches {
            stochastic_go_no_go: false,
            ..Switches::all_enabled()
        },
        12,
    )
    .unwrap();
    let q_s = make_percept(
        "per_alice",
        "room:3",
        &["per_alice", "per_bob", "soul"],
        1_000,
    );
    core_sib
        .step(
            Input::Percept(Box::new(q_s)),
            Time::from_millis(1_000),
            &mut doors,
        )
        .unwrap();
    let mut sib_ans = make_percept(
        "per_bob",
        "room:3",
        &["per_alice", "per_bob", "soul"],
        1_500,
    );
    sib_ans.addressees = vec![ActorId::new("per_alice")];
    core_sib
        .step(
            Input::Percept(Box::new(sib_ans)),
            Time::from_millis(1_500),
            &mut doors,
        )
        .unwrap();
    let log_s = core_sib.store().read_in_order(None, 200).unwrap();
    assert!(log_s.iter().any(|r| matches!(
        &r.detail,
        Some(EventDetail::SilenceJudged {
            missed: false,
            reason,
            ..
        }) if reason == "answered_by_participant"
    )));
}

#[test]
fn free_text_condition_bands_and_goal_reached_and_expiry() {
    let prior = Prior {
        wake_gate_lo: 0.50,
        wake_gate_hi_epsilon: 0.25, // rule of three: ceil(3 / 0.25) = 12 verdicts
        ..Prior::default()
    };
    let mut sw = Switches::all_enabled();
    sw.stochastic_go_no_go = false;
    let mut core = Core::open(MemStore::new(), prior, sw, 13).unwrap();
    let mut doors = NoopDoors;

    // Open a free-text Monitor condition with embedding [1, 0, 0].
    let mut p_init = make_percept("per_alice", "room:1", &["per_alice", "soul"], 1_000);
    p_init.addressees = vec![ActorId::new("soul")];
    let eff = core
        .step(
            Input::Percept(Box::new(p_init)),
            Time::from_millis(1_000),
            &mut doors,
        )
        .unwrap();
    let req = match eff.as_slice() {
        [Effect::Model { plan, .. }] => plan.trigger_event_id.unwrap(),
        _ => panic!("expected wake"),
    };
    core.step(
        Input::ModelResult {
            request_id: req,
            acts: vec![ThoughtAct::Expect {
                expect_kind: ExpectKind::Promise,
                about_ids: vec![req],
                places: vec![PlaceId::new("room:1")],
                people: vec![ActorId::new("per_alice")],
                condition: "deploy succeeded".to_owned(),
                due_ms: Some(Time::from_millis(100_000)),
                expiry_ms: Time::from_millis(200_000),
                recurrence: None,
                promise_meta: None,
                domain: None,
            }],
        },
        Time::from_millis(1_000),
        &mut doors,
    )
    .unwrap();
    let cond_id = *core
        .state()
        .expectation_book
        .expectations
        .keys()
        .max()
        .unwrap();
    core.set_expectation_embeddings(cond_id, vec![emb([1.0, 0.0, 0.0])]);

    // 1. Below lower band (sim = 0.0 < 0.50) -> ZERO Classify call!
    let mut p_low = make_percept("per_bob", "room:1", &["per_bob", "soul"], 2_000);
    p_low.embeddings = vec![emb([0.0, 1.0, 0.0])];
    let eff_low = core
        .step(
            Input::Percept(Box::new(p_low)),
            Time::from_millis(2_000),
            &mut doors,
        )
        .unwrap();
    assert!(
        !eff_low.iter().any(|e| matches!(
            e,
            Effect::Model {
                purpose: ModelPurpose::Classify,
                ..
            }
        )),
        "below lower band must not call Classify"
    );

    // 2. Above lower band -> ONE Classify call; negative verdict keeps expectation open!
    let mut p_mid = make_percept("per_bob", "room:1", &["per_bob", "soul"], 3_000);
    p_mid.embeddings = vec![emb([0.96, 0.28, 0.0])]; // cos = 0.96
    let eff_mid = core
        .step(
            Input::Percept(Box::new(p_mid)),
            Time::from_millis(3_000),
            &mut doors,
        )
        .unwrap();
    let clf_req = match eff_mid.as_slice() {
        [
            Effect::Model {
                purpose: ModelPurpose::Classify,
                plan,
            },
        ] => plan.trigger_event_id.unwrap(),
        other => panic!("expected one Classify call, got {other:?}"),
    };
    core.step(
        Input::ClassifyResult {
            request_id: clf_req,
            verdict: ClassificationVerdict {
                protocol_version: Default::default(),
                label: "not_met".to_owned(),
                score: Some(0.1),
                rationale: None,
            },
        },
        Time::from_millis(3_100),
        &mut doors,
    )
    .unwrap();
    assert_eq!(
        core.state()
            .expectation_book
            .expectations
            .get(&cond_id)
            .unwrap()
            .state,
        ExpectationLifecycleState::Open
    );

    // Negative verdict at 0.96 blocks adopting 0.90 upper band even if we add 12 positives at 0.95.
    assert!(!core.try_adopt_wake_gate_upper_band(0.90));

    // 3. Goal lifecycle: Reached accepted cleanly; expired goal no longer attracts events.
    let mut p_g = make_percept("per_alice", "room:2", &["per_alice", "soul"], 4_000);
    p_g.addressees = vec![ActorId::new("soul")];
    let eff_g = core
        .step(
            Input::Percept(Box::new(p_g)),
            Time::from_millis(4_000),
            &mut doors,
        )
        .unwrap();
    let req_g = match eff_g.as_slice() {
        [Effect::Model { plan, .. }] => plan.trigger_event_id.unwrap(),
        _ => panic!("expected wake"),
    };
    core.step(
        Input::ModelResult {
            request_id: req_g,
            acts: vec![
                ThoughtAct::Expect {
                    expect_kind: ExpectKind::Goal,
                    about_ids: vec![req_g],
                    places: vec![PlaceId::new("room:2")],
                    people: vec![ActorId::new("per_alice")],
                    condition: "finish migration".to_owned(),
                    due_ms: Some(Time::from_millis(10_000)),
                    expiry_ms: Time::from_millis(20_000),
                    recurrence: None,
                    promise_meta: None,
                    domain: None,
                },
                ThoughtAct::Expect {
                    expect_kind: ExpectKind::Goal,
                    about_ids: vec![req_g],
                    places: vec![PlaceId::new("room:2")],
                    people: vec![ActorId::new("per_alice")],
                    condition: "expiring goal".to_owned(),
                    due_ms: Some(Time::from_millis(5_000)),
                    expiry_ms: Time::from_millis(6_000),
                    recurrence: None,
                    promise_meta: None,
                    domain: None,
                },
            ],
        },
        Time::from_millis(4_000),
        &mut doors,
    )
    .unwrap();

    let mut goal_ids: Vec<EventId> = core
        .state()
        .expectation_book
        .expectations
        .values()
        .filter(|e| e.kind == ExpectKind::Goal)
        .map(|e| e.expectation_id)
        .collect();
    goal_ids.sort();
    let (goal_reach, goal_expire) = (goal_ids[0], goal_ids[1]);
    core.set_expectation_embeddings(goal_expire, vec![emb([0.0, 0.0, 1.0])]);

    // Resolve goal_reach via Reached.
    core.step(
        Input::ModelResult {
            request_id: req_g,
            acts: vec![ThoughtAct::Resolve {
                expectation_id: goal_reach,
                outcome: ResolveOutcome::Reached,
                note: None,
                places: vec![PlaceId::new("room:2")],
                people: vec![ActorId::new("per_alice")],
                domain: None,
            }],
        },
        Time::from_millis(4_500),
        &mut doors,
    )
    .unwrap();
    assert_eq!(
        core.state()
            .expectation_book
            .expectations
            .get(&goal_reach)
            .unwrap()
            .state,
        ExpectationLifecycleState::Reached
    );

    // Expire goal_expire at t = 6_000, then send matching event -> must NOT attract or classify!
    core.step(Input::Tick, Time::from_millis(6_000), &mut doors)
        .unwrap();
    assert_eq!(
        core.state()
            .expectation_book
            .expectations
            .get(&goal_expire)
            .unwrap()
            .state,
        ExpectationLifecycleState::Expired
    );
    let mut p_after_exp = make_percept("per_bob", "room:2", &["per_bob", "soul"], 6_500);
    p_after_exp.embeddings = vec![emb([0.0, 0.0, 1.0])];
    let eff_after_exp = core
        .step(
            Input::Percept(Box::new(p_after_exp)),
            Time::from_millis(6_500),
            &mut doors,
        )
        .unwrap();
    assert!(
        !eff_after_exp.iter().any(|e| matches!(
            e,
            Effect::Model {
                purpose: ModelPurpose::Classify,
                ..
            }
        )),
        "expired goal must no longer attract events"
    );
}
