use std::collections::BTreeSet;

use monoagens_core::{
    Core, CoreConfig, Effect, EpisodeBook, EpisodeState, EventDetail, EventKind, EventRecord,
    Input, MemStore, MemoryBasis, MemoryEntry, ModelPurpose, NoopDoors, NumericFeatures,
    RecallQuery, ReplayMode, Store,
};
use monoagens_protocol::{
    ActorFacts, ActorId, Audience, CURRENT_PROTOCOL_VERSION, ClassificationVerdict,
    ConsolidateOutput, DomainId, EmbeddingTag, EventId, OpaquePayload, OriginActor, Percept,
    PlaceId, TaggedEmbedding, Taint, ThoughtAct, Time, VisibleScope,
};

fn emb_tag() -> EmbeddingTag {
    EmbeddingTag {
        embed_model_ver: "eg2-256".to_owned(),
        dim_d: 4,
        prompt_regime_id: "t_d".to_owned(),
    }
}

fn vec_emb(vals: [f32; 4]) -> TaggedEmbedding {
    TaggedEmbedding {
        tag: emb_tag(),
        values: vals.to_vec(),
    }
}

fn make_percept(
    actor: &str,
    place: &str,
    taint: Taint,
    ts_ms: u64,
    payload: &str,
    embs: Vec<TaggedEmbedding>,
) -> Percept {
    Percept {
        protocol_version: CURRENT_PROTOCOL_VERSION,
        actor: ActorId::new(actor),
        origin_actor: Some(OriginActor::new(ActorId::new(actor))),
        place: PlaceId::new(place),
        audience: Audience::new([ActorId::new("soul"), ActorId::new(actor)]),
        taint,
        payload: OpaquePayload::from_bytes(payload.as_bytes()),
        embeddings: embs,
        addressees: vec![ActorId::new("soul")],
        named: vec![],
        reply_to: None,
        correlation_id: None,
        channel: None,
        channel_prediction: None,
        f_modal: None,
        orient: vec![],
        dialogue_act_gap: Some(0.95),
        delta_obs: None,
        length: Some(payload.len() as u64),
        negative_flag: None,
        repair: None,
        continued_attention: None,
        attachments: vec![],
        timestamp_ms: Time::from_millis(ts_ms),
    }
}

fn open_test_core(config: CoreConfig) -> Core<MemStore> {
    Core::open(MemStore::new(), config, None).expect("open core")
}

#[test]
fn basis_downgrade_validity_and_mind_inference() {
    let mut core = open_test_core(CoreConfig::default());
    let mut doors = NoopDoors;

    // 1. A source asserts something: percept itself is recorded, while what it said is `Told`.
    let p_untrusted = make_percept(
        "alice",
        "room1",
        Taint::Untrusted,
        10_000,
        "alice said X",
        vec![],
    );
    core.step(
        Input::Percept(Box::new(p_untrusted)),
        Time::from_millis(10_000),
        &mut doors,
    )
    .expect("step alice percept");
    let alice_ev = core
        .state()
        .memory_entries
        .keys()
        .copied()
        .next()
        .expect("alice event indexed");
    assert_eq!(
        core.state().memory_entries[&alice_ev].basis,
        MemoryBasis::Told
    );

    // 2. Reasoning claims `Observed` citing only a `Told` event (no witnessed percept): downgraded to `Inferred`!
    let bad_obs_id = core
        .record_belief(
            "fact:bridge_closed",
            MemoryBasis::Observed,
            None,
            Some(DomainId::new("traffic")),
            vec![alice_ev],
            Taint::Trusted,
            false,
            Time::from_millis(11_000),
        )
        .expect("record ungrounded observed");
    let b1 = &core.state().belief_book.beliefs[&bad_obs_id];
    assert_eq!(b1.requested_basis, MemoryBasis::Observed);
    assert_eq!(b1.basis, MemoryBasis::Inferred);
    assert!(b1.basis_downgraded);
    assert_eq!(b1.valid_from_ms, Time::from_millis(11_000));
    assert_eq!(b1.valid_to_ms, None);

    // 3. Direct witnessed percept (`Taint::Trusted`) stays `Observed`.
    let p_direct = make_percept(
        "sensor1",
        "room1",
        Taint::Trusted,
        12_000,
        "sensor sees bridge",
        vec![],
    );
    core.step(
        Input::Percept(Box::new(p_direct)),
        Time::from_millis(12_000),
        &mut doors,
    )
    .expect("step sensor");
    let sensor_ev = *core.state().memory_entries.keys().last().unwrap();
    let ok_obs_id = core
        .record_belief(
            "fact:bridge_status",
            MemoryBasis::Observed,
            None,
            Some(DomainId::new("traffic")),
            vec![sensor_ev],
            Taint::Trusted,
            false,
            Time::from_millis(13_000),
        )
        .expect("record grounded observed");
    let b2 = &core.state().belief_book.beliefs[&ok_obs_id];
    assert_eq!(b2.basis, MemoryBasis::Observed);
    assert!(!b2.basis_downgraded);
    assert!(
        b2.credence.is_none(),
        "observed evidence stays separate from uncertain credence"
    );

    // 4. Replacing a belief preserves old belief with `valid_to_ms` and `replaced_by`.
    let new_obs_id = core
        .record_belief(
            "fact:bridge_status",
            MemoryBasis::Observed,
            None,
            Some(DomainId::new("traffic")),
            vec![sensor_ev],
            Taint::Trusted,
            false,
            Time::from_millis(20_000),
        )
        .expect("replace belief");
    let old_b = &core.state().belief_book.beliefs[&ok_obs_id];
    assert_eq!(old_b.valid_to_ms, Some(Time::from_millis(20_000)));
    assert_eq!(old_b.replaced_by, Some(new_obs_id));
    assert!(core.state().belief_book.beliefs[&new_obs_id].is_current());

    // 5. Mind inference is never `Observed`, starts `Inferred` with credence, falls on prediction fail.
    let mind_id = core
        .record_belief(
            "mind:alice_prefers_tea",
            MemoryBasis::Observed,
            None,
            Some(DomainId::new("social")),
            vec![sensor_ev],
            Taint::Trusted,
            true,
            Time::from_millis(21_000),
        )
        .expect("mind inference");
    let mb = &core.state().belief_book.beliefs[&mind_id];
    assert_eq!(mb.basis, MemoryBasis::Inferred);
    assert!(mb.is_mind_inference);
    let init_exp = mb.credence.unwrap().expected();
    core.record_cited_outcome(
        &[mind_id],
        &[],
        false,
        1.0,
        None,
        Some(DomainId::new("social")),
        Time::from_millis(22_000),
    )
    .expect("record failed prediction");
    let after_exp = core.state().belief_book.beliefs[&mind_id]
        .credence
        .unwrap()
        .expected();
    assert!(after_exp < init_exp);
}

#[test]
fn told_credence_cited_vs_uncited_decay_robustness_and_domain_isolation() {
    let mut core = open_test_core(CoreConfig::default());
    let mut doors = NoopDoors;

    // Register prior (alpha_0=4, beta_0=1) for `alice`, and leave `stranger` with no prior (uniform 1,1).
    core.step(
        Input::ActorFacts(ActorFacts {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            actor_id: ActorId::new("alice"),
            alpha_0: 4.0,
            beta_0: 1.0,
            teacher_weight: 1.0,
            trusted_origin: false,
        }),
        Time::from_millis(1_000),
        &mut doors,
    )
    .expect("actor facts");

    let dom_sched = DomainId::new("schedules");
    let dom_tech = DomainId::new("tech");

    let b_alice = core
        .record_belief(
            "claim:meeting_at_3",
            MemoryBasis::Told,
            Some(ActorId::new("alice")),
            Some(dom_sched.clone()),
            vec![],
            Taint::Provisional,
            false,
            Time::from_millis(2_000),
        )
        .expect("alice told");
    let c_alice = core.state().belief_book.beliefs[&b_alice].credence.unwrap();
    assert!((c_alice.alpha - 4.0).abs() < 1e-12);
    assert!((c_alice.beta - 1.0).abs() < 1e-12);

    // No supplied prior -> uniform Beta(1, 1).
    let b_stranger = core
        .record_belief(
            "claim:stranger_note",
            MemoryBasis::Told,
            Some(ActorId::new("stranger")),
            Some(dom_sched.clone()),
            vec![],
            Taint::Untrusted,
            false,
            Time::from_millis(2_500),
        )
        .expect("stranger told");
    let c_str = core.state().belief_book.beliefs[&b_stranger]
        .credence
        .unwrap();
    assert!((c_str.alpha - 1.0).abs() < 1e-12);
    assert!((c_str.beta - 1.0).abs() < 1e-12);

    // Cited vs uncited: confirm `b_alice` 50 times while `b_stranger` is in context but uncited.
    for i in 0..50 {
        core.record_cited_outcome(
            &[b_alice],
            &[b_stranger],
            true,
            1.0,
            Some(ActorId::new("alice")),
            Some(dom_sched.clone()),
            Time::from_millis(3_000 + i * 1_000),
        )
        .expect("confirm alice");
    }
    let c_alice_confirmed = core.state().belief_book.beliefs[&b_alice].credence.unwrap();
    assert!(c_alice_confirmed.alpha > 53.9 && c_alice_confirmed.alpha <= 54.0);
    // Uncited item did not move at all!
    let c_str_after = core.state().belief_book.beliefs[&b_stranger]
        .credence
        .unwrap();
    assert!((c_str_after.alpha - 1.0).abs() < 1e-12);
    assert!((c_str_after.beta - 1.0).abs() < 1e-12);

    // One failure after many confirmations moves expected credence only slightly.
    let before_fail = c_alice_confirmed.expected();
    core.record_cited_outcome(
        &[b_alice],
        &[],
        false,
        1.0,
        Some(ActorId::new("alice")),
        Some(dom_sched.clone()),
        Time::from_millis(60_000),
    )
    .expect("single failure");
    let after_fail = core.state().belief_book.beliefs[&b_alice]
        .credence
        .unwrap()
        .expected();
    assert!(before_fail - after_fail < 0.03);
    assert!(after_fail > 0.90);

    // Decay toward prior over 30 days (`lambda_domain = 0.9` per day).
    let thirty_days = Time::from_millis(60_000 + 30 * 86_400_000);
    let decayed = core
        .state()
        .belief_book
        .item_credence_at(b_alice, thirty_days)
        .expect("decayed");
    assert!(decayed.alpha < c_alice_confirmed.alpha);
    assert!(decayed.alpha > decayed.alpha_0);

    // Domain isolation: contradicting `alice` repeatedly in `tech` lowers new `told` items in `tech`,
    // while `schedules` stays unaffected!
    let tech_claim = core
        .record_belief(
            "claim:rust_bug",
            MemoryBasis::Told,
            Some(ActorId::new("alice")),
            Some(dom_tech.clone()),
            vec![],
            Taint::Provisional,
            false,
            Time::from_millis(70_000),
        )
        .expect("tech claim");
    for i in 0..8 {
        core.record_cited_outcome(
            &[tech_claim],
            &[],
            false,
            1.0,
            Some(ActorId::new("alice")),
            Some(dom_tech.clone()),
            Time::from_millis(71_000 + i * 1_000),
        )
        .expect("contradict tech");
    }
    let new_tech = core
        .record_belief(
            "claim:new_tech",
            MemoryBasis::Told,
            Some(ActorId::new("alice")),
            Some(dom_tech.clone()),
            vec![],
            Taint::Provisional,
            false,
            Time::from_millis(90_000),
        )
        .expect("new tech claim");
    let new_sched = core
        .record_belief(
            "claim:new_sched",
            MemoryBasis::Told,
            Some(ActorId::new("alice")),
            Some(dom_sched.clone()),
            vec![],
            Taint::Provisional,
            false,
            Time::from_millis(90_500),
        )
        .expect("new sched claim");
    assert!(
        core.state().belief_book.beliefs[&new_tech]
            .credence
            .unwrap()
            .expected()
            < 0.45
    );
    assert!(
        core.state().belief_book.beliefs[&new_sched]
            .credence
            .unwrap()
            .expected()
            > 0.90
    );

    // Rebuild from log matches identically.
    let rebuilt = core.state().belief_book.rebuild_from_log();
    assert_eq!(rebuilt.beliefs, core.state().belief_book.beliefs);
    assert_eq!(
        rebuilt.source_reliability,
        core.state().belief_book.source_reliability
    );
}

#[test]
fn k_corrob_taint_never_upgrades_and_conventions_discounted() {
    let mut core = open_test_core(CoreConfig::default());
    let mut doors = NoopDoors;

    // Direct observation establishes truth on `claim:door_locked`.
    let p_obs = make_percept(
        "sensor",
        "lab",
        Taint::Trusted,
        1_000,
        "door locked",
        vec![],
    );
    core.step(
        Input::Percept(Box::new(p_obs)),
        Time::from_millis(1_000),
        &mut doors,
    )
    .unwrap();
    let sensor_ev = *core.state().memory_entries.keys().last().unwrap();
    let direct_id = core
        .record_belief(
            "claim:door_locked",
            MemoryBasis::Observed,
            None,
            Some(DomainId::new("safety")),
            vec![sensor_ev],
            Taint::Trusted,
            false,
            Time::from_millis(1_100),
        )
        .unwrap();

    // Untrusted stranger echoes twice from the SAME origin (`mallory`) -> still 1 distinct source (< k_corrob=2),
    // keeps Untrusted taint, never upgrades to trusted basis, never overrides direct observation!
    let echo1 = core
        .record_belief(
            "claim:door_locked",
            MemoryBasis::Told,
            Some(ActorId::new("mallory")),
            Some(DomainId::new("safety")),
            vec![],
            Taint::Untrusted,
            false,
            Time::from_millis(2_000),
        )
        .unwrap();
    let echo2 = core
        .record_belief(
            "claim:door_locked",
            MemoryBasis::Told,
            Some(ActorId::new("mallory")),
            Some(DomainId::new("safety")),
            vec![],
            Taint::Untrusted,
            false,
            Time::from_millis(2_100),
        )
        .unwrap();

    assert_eq!(
        core.state().belief_book.active_claims["claim:door_locked"],
        direct_id
    );
    assert_eq!(
        core.state().belief_book.beliefs[&echo1].taint,
        Taint::Untrusted
    );
    assert_eq!(
        core.state().belief_book.beliefs[&echo2]
            .corroborating_actors
            .len(),
        1
    );
    assert!(
        core.state().belief_book.beliefs[&echo2]
            .is_uncorroborated_stranger_claim(2)
    );

    // Distinct independent actors for a novel claim (`bob` + `carol`) meet `k_corrob = 2`,
    // while min-trust taint remains Untrusted (never laundered to Trusted).
    let c1 = core
        .record_belief(
            "claim:novel_schedule",
            MemoryBasis::Told,
            Some(ActorId::new("bob")),
            Some(DomainId::new("sched")),
            vec![],
            Taint::Untrusted,
            false,
            Time::from_millis(3_000),
        )
        .unwrap();
    let c2 = core
        .record_belief(
            "claim:novel_schedule",
            MemoryBasis::Told,
            Some(ActorId::new("carol")),
            Some(DomainId::new("sched")),
            vec![],
            Taint::Provisional,
            false,
            Time::from_millis(3_100),
        )
        .unwrap();
    assert_eq!(
        core.state().belief_book.beliefs[&c1]
            .corroborating_actors
            .len(),
        2
    );
    assert_eq!(
        core.state().belief_book.beliefs[&c2]
            .corroborating_actors
            .len(),
        2
    );
    assert_eq!(
        core.state().belief_book.beliefs[&c2].taint,
        Taint::Untrusted
    );

    // R-32 social convention discounting: peer observation updates convention strictly below 1.0 weight.
    let place = PlaceId::new("room_conv");
    core.record_convention_observation(
        place.clone(),
        "fast_reply",
        ActorId::new("bob"),
        None,
        1.0,
        Time::from_millis(4_000),
    )
    .unwrap();
    let bias =
        core.state()
            .belief_book
            .convention_bias_at(&place, "fast_reply", Time::from_millis(4_100));
    assert!(bias > 0.0 && bias <= 0.15);
    let conv_cred = core.state().belief_book.conventions[&(place, "fast_reply".to_owned())];
    assert!(
        conv_cred.effective_evidence() < 1.0,
        "vicarious peer convention must be discounted by lambda_vic < 1"
    );
}

#[test]
fn verbatim_vs_gist_supersession_and_min_trust_taint() {
    let mut core = open_test_core(CoreConfig::default());
    let mut doors = NoopDoors;

    let e_src = vec_emb([1.0, 0.0, 0.0, 0.0]);
    let p1 = make_percept(
        "stranger",
        "room1",
        Taint::Untrusted,
        10_000,
        "met at coffee shop",
        vec![e_src.clone()],
    );
    let eff = core
        .step(
            Input::Percept(Box::new(p1)),
            Time::from_millis(10_000),
            &mut doors,
        )
        .unwrap();
    let wake_id = eff
        .iter()
        .find_map(|e| match e {
            Effect::Model { plan, .. } => plan.trigger_event_id,
            _ => None,
        })
        .expect("wake trigger");

    // Emit ConsolidateOutput::Gist over mixed/untrusted source `wake_id`.
    let cons_effects = core
        .step(
            Input::ModelResult {
                request_id: wake_id,
                acts: vec![ThoughtAct::Consolidate {
                    output: ConsolidateOutput::Gist {
                        summary: "met at coffee shop and signed contract".to_owned(),
                        about_ids: vec![wake_id],
                        places: vec![PlaceId::new("room1")],
                        people: vec![ActorId::new("stranger")],
                        domain: None,
                    },
                }],
            },
            Time::from_millis(11_000),
            &mut doors,
        )
        .unwrap();

    // Verify the post-sleep gist verification is a `Classify` call carrying its sources.
    let verify_plan = cons_effects
        .iter()
        .find_map(|e| match e {
            Effect::Model {
                purpose: ModelPurpose::Classify,
                plan,
            } => Some(plan),
            _ => None,
        })
        .expect("post-sleep gist verification classify effect");
    let gist_id = verify_plan.trigger_event_id.expect("gist trigger");
    match &verify_plan.sections.task {
        Some(monoagens_core::ModelTask::GistFaithful { sources }) => {
            assert_eq!(
                sources.iter().map(|s| s.event_id).collect::<Vec<_>>(),
                vec![Some(wake_id)]
            );
        }
        other => panic!("gist verification must carry GistFaithful sources, got {other:?}"),
    }
    assert!(
        !cons_effects.iter().any(|e| matches!(
            e,
            Effect::Model {
                purpose: ModelPurpose::Consolidate,
                ..
            }
        )),
        "gist verification must not be requested as a new consolidation"
    );

    // Gist must inherit `min(trust)` = Untrusted (`13.2`).
    assert_eq!(
        core.state().memory_entries[&gist_id].taint,
        Taint::Untrusted
    );
    assert_eq!(
        core.state().memory_entries[&gist_id].source_ids,
        vec![wake_id]
    );

    // `reason` detects overstated detail absent from source events (`10.2`).
    core.step(
        Input::ClassifyResult {
            request_id: gist_id,
            verdict: ClassificationVerdict {
                protocol_version: Default::default(),
                label: "conflict".to_owned(),
                score: Some(0.9),
                rationale: Some("contract not in source events".to_owned()),
            },
        },
        Time::from_millis(12_000),
        &mut doors,
    )
    .unwrap();

    let gist_entry = &core.state().memory_entries[&gist_id];
    assert_eq!(gist_entry.superseded_by, Some(wake_id));

    // Recall presents the intact source event as current & recollected, and marks gist superseded.
    let scope = VisibleScope::new(
        Audience::new([ActorId::new("soul"), ActorId::new("stranger")]),
        Some(PlaceId::new("room1")),
    );
    let mut q = RecallQuery::new(scope);
    q.query_embeddings = vec![e_src];
    q.limit = 10;
    let res = core.recall(&q, Time::from_millis(13_000)).unwrap();
    let src_item = res
        .items
        .iter()
        .find(|i| i.event_id == wake_id)
        .expect("source item returned");
    assert!(src_item.is_current);
    assert!(src_item.recollected);
    if let Some(gist_item) = res.items.iter().find(|i| i.event_id == gist_id) {
        assert!(!gist_item.is_current);
        assert_eq!(gist_item.superseded_by, Some(wake_id));
    }
}

#[test]
fn sleep_n_min_boundary_live_event_delta_theta_zero_and_provisional_reopen_no_spin() {
    let cfg = CoreConfig {
        sleep_min_episodes: 4,
        sleep_idle_ms: 60_000,
        seed: 42,
        ..CoreConfig::default()
    };
    let mut core = open_test_core(cfg.clone());
    let mut doors = NoopDoors;

    // 1. `N_min - 1` = 3 events: even after long idle, sleep MUST NOT trigger (`>= N_min`, not `>`).
    for i in 0..3 {
        let p = make_percept(
            "alice",
            "room1",
            Taint::Trusted,
            10_000 + i * 1_000,
            &format!("msg {i}"),
            vec![vec_emb([1.0, 0.0, 0.0, 0.0])],
        );
        core.step(
            Input::Percept(Box::new(p)),
            Time::from_millis(10_000 + i * 1_000),
            &mut doors,
        )
        .unwrap();
    }
    assert_eq!(core.state().unconsolidated_count, 3);
    let no_sleep = core
        .step(Input::Tick, Time::from_millis(500_000), &mut doors)
        .unwrap();
    assert!(no_sleep.is_empty(), "sleep must not trigger below N_min");

    // 2. Exactly `N_min` = 4 events triggers sleep once idle >= T_idle_eff.
    let p4 = make_percept(
        "alice",
        "room1",
        Taint::Trusted,
        510_000,
        "msg 3",
        vec![vec_emb([1.0, 0.0, 0.0, 0.0])],
    );
    core.step(
        Input::Percept(Box::new(p4)),
        Time::from_millis(510_000),
        &mut doors,
    )
    .unwrap();
    assert_eq!(core.state().unconsolidated_count, 4);

    let due = core.next_due().expect("sleep due at N_min");
    let sleep_effects = core.step(Input::Tick, due, &mut doors).unwrap();
    let consolidate_req = sleep_effects
        .iter()
        .find_map(|e| match e {
            Effect::Model {
                purpose: ModelPurpose::Consolidate,
                plan,
            } => Some(plan.clone()),
            _ => None,
        })
        .expect("sleep emitted Consolidate effect for provisional episode");
    let ep_id = consolidate_req.trigger_event_id.unwrap();
    assert_eq!(
        core.state().episode_book.episodes[&ep_id].state,
        EpisodeState::Provisional
    );

    // 3. While sleep is running (awaiting ModelResult), a live event arrives and is appraised
    //    at normal theta (`Delta theta_sleep = 0`, never delayed!).
    let live_t = due.saturating_add_ms(500);
    let p_live = make_percept(
        "bob",
        "room2",
        Taint::Trusted,
        live_t.as_millis(),
        "urgent live ping",
        vec![vec_emb([0.0, 1.0, 0.0, 0.0])],
    );
    let live_eff = core
        .step(Input::Percept(Box::new(p_live)), live_t, &mut doors)
        .unwrap();
    assert!(
        live_eff.iter().any(|e| matches!(
            e,
            Effect::Model {
                purpose: ModelPurpose::Reason,
                ..
            }
        )),
        "live event during sleep must wake immediately at normal theta"
    );

    // Compare threshold recorded on `p_live` against a fresh non-sleeping core: identical!
    let mut fresh_core = open_test_core(cfg);
    let p_live_cmp = make_percept(
        "bob",
        "room2",
        Taint::Trusted,
        live_t.as_millis(),
        "urgent live ping",
        vec![vec_emb([0.0, 1.0, 0.0, 0.0])],
    );
    fresh_core
        .step(Input::Percept(Box::new(p_live_cmp)), live_t, &mut doors)
        .unwrap();
    let rec_sleep_dec = core
        .store()
        .read_in_order(None, 100)
        .unwrap()
        .into_iter()
        .rev()
        .find(|r| r.kind == EventKind::Decision && r.place == PlaceId::new("room2"))
        .unwrap();
    let rec_fresh_dec = fresh_core
        .store()
        .read_in_order(None, 100)
        .unwrap()
        .into_iter()
        .rev()
        .find(|r| r.kind == EventKind::Decision && r.place == PlaceId::new("room2"))
        .unwrap();
    match (&rec_sleep_dec.detail, &rec_fresh_dec.detail) {
        (
            Some(EventDetail::Decision {
                threshold: th_s, ..
            }),
            Some(EventDetail::Decision {
                threshold: th_f, ..
            }),
        ) => {
            assert_eq!(
                th_s.to_bits(),
                th_f.to_bits(),
                "Delta theta_sleep must be 0"
            );
        }
        _ => panic!("expected Decision details"),
    }

    // 4. Complete sleep's ConsolidateOutput::Gist for `room1`: leaves `0` unconsolidated for `room1`
    //    (only `room2`'s 1 event remains, < N_min -> no spin loop!).
    let mut gist_acts = vec![ThoughtAct::Consolidate {
        output: ConsolidateOutput::Gist {
            summary: "room1 provisional summary".to_owned(),
            about_ids: consolidate_req.items.clone(),
            places: vec![PlaceId::new("room1")],
            people: vec![ActorId::new("alice")],
            domain: None,
        },
    }];
    let _ = core
        .step(
            Input::ModelResult {
                request_id: ep_id,
                acts: std::mem::take(&mut gist_acts),
            },
            due.saturating_add_ms(1_000),
            &mut doors,
        )
        .unwrap();
    let stale_gist_id = core.state().episode_book.episodes[&ep_id].gist_id.unwrap();
    assert_eq!(
        core.state().unconsolidated_count,
        1,
        "room1 tail consolidated -> 0 backlog for room1"
    );
    let next_tick = core
        .step(Input::Tick, due.saturating_add_ms(120_000), &mut doors)
        .unwrap();
    assert!(next_tick.is_empty(), "no overnight spin loop");

    // 5. Resuming event in `room1` within continuity bounds reopens `Provisional` -> `Open`
    //    and marks stale gist `superseded_by`!
    let resume_t = due.saturating_add_ms(125_000);
    let p_resume = make_percept(
        "alice",
        "room1",
        Taint::Trusted,
        resume_t.as_millis(),
        "continuing room1",
        vec![vec_emb([1.0, 0.0, 0.0, 0.0])],
    );
    core.step(Input::Percept(Box::new(p_resume)), resume_t, &mut doors)
        .unwrap();
    assert_eq!(
        core.state().episode_book.episodes[&ep_id].state,
        EpisodeState::Open
    );
    assert!(
        core.state().memory_entries[&stale_gist_id]
            .superseded_by
            .is_some(),
        "reopened provisional episode must mark stale gist superseded_by"
    );
}

#[test]
fn interrupted_sleep_resumes_without_duplicating_gists() {
    let cfg = CoreConfig {
        sleep_min_episodes: 8,
        sleep_idle_ms: 60_000,
        seed: 42,
        ..CoreConfig::default()
    };
    let mut core = open_test_core(cfg.clone());
    let mut doors = NoopDoors;

    for i in 0..4 {
        let p = make_percept(
            "alice",
            "room_a",
            Taint::Trusted,
            1_000 + i * 1_000,
            "a",
            vec![vec_emb([1.0, 0.0, 0.0, 0.0])],
        );
        core.step(
            Input::Percept(Box::new(p)),
            Time::from_millis(1_000 + i * 1_000),
            &mut doors,
        )
        .unwrap();
    }
    for i in 0..4 {
        let p = make_percept(
            "bob",
            "room_b",
            Taint::Trusted,
            10_000 + i * 1_000,
            "b",
            vec![vec_emb([0.0, 1.0, 0.0, 0.0])],
        );
        core.step(
            Input::Percept(Box::new(p)),
            Time::from_millis(10_000 + i * 1_000),
            &mut doors,
        )
        .unwrap();
    }

    let due = core.next_due().unwrap();
    let eff = core.step(Input::Tick, due, &mut doors).unwrap();
    let mut plans: Vec<_> = eff
        .into_iter()
        .filter_map(|e| match e {
            Effect::Model {
                purpose: ModelPurpose::Consolidate,
                plan,
            } => Some(plan),
            _ => None,
        })
        .collect();
    assert_eq!(plans.len(), 2);

    // Consolidate ONLY the first episode, then simulate restart via snapshot + Core::open.
    let first = plans.remove(0);
    core.step(
        Input::ModelResult {
            request_id: first.trigger_event_id.unwrap(),
            acts: vec![ThoughtAct::Consolidate {
                output: ConsolidateOutput::Gist {
                    summary: "room_a gist".to_owned(),
                    about_ids: first.items.clone(),
                    places: vec![PlaceId::new("room_a")],
                    people: vec![ActorId::new("alice")],
                    domain: None,
                },
            }],
        },
        due.saturating_add_ms(500),
        &mut doors,
    )
    .unwrap();
    let ckpt = core.take_snapshot("mid_sleep").unwrap();
    let store = core.store().clone();
    drop(core);

    let mut resumed = Core::open(store, cfg, None).unwrap();
    let _ = ckpt;
    assert!(resumed.state().pending_sleep_run.is_some());

    // Next Tick resumes ONLY the remaining episode (`room_b`), never duplicating `room_a`!
    let rem_eff = resumed
        .step(Input::Tick, due.saturating_add_ms(2_000), &mut doors)
        .unwrap();
    let rem_plans: Vec<_> = rem_eff
        .into_iter()
        .filter_map(|e| match e {
            Effect::Model {
                purpose: ModelPurpose::Consolidate,
                plan,
            } => Some(plan),
            _ => None,
        })
        .collect();
    assert_eq!(rem_plans.len(), 1);
    assert_eq!(rem_plans[0].place, Some(PlaceId::new("room_b")));

    // Finish second episode -> SleepRun completes.
    resumed
        .step(
            Input::ModelResult {
                request_id: rem_plans[0].trigger_event_id.unwrap(),
                acts: vec![ThoughtAct::Consolidate {
                    output: ConsolidateOutput::Gist {
                        summary: "room_b gist".to_owned(),
                        about_ids: rem_plans[0].items.clone(),
                        places: vec![PlaceId::new("room_b")],
                        people: vec![ActorId::new("bob")],
                        domain: None,
                    },
                }],
            },
            due.saturating_add_ms(3_000),
            &mut doors,
        )
        .unwrap();
    assert!(resumed.state().pending_sleep_run.is_none());
    let gist_count = resumed
        .state()
        .memory_entries
        .values()
        .filter(|m| m.is_gist)
        .count();
    assert_eq!(gist_count, 2, "each episode must produce exactly one gist");
}

#[test]
fn episode_merge_guards_preserve_people_and_place_boundaries() {
    let cfg = CoreConfig::default();

    // Episode merge (`R-24`): same vector similarity with different people or places MUST NOT merge!
    let mut book = EpisodeBook::new();
    let mut p_alice = BTreeSet::new();
    p_alice.insert(ActorId::new("alice"));
    let mut p_bob = BTreeSet::new();
    p_bob.insert(ActorId::new("bob"));
    let same_vec = vec![vec_emb([1.0, 0.0, 0.0, 0.0])];

    book.admit_event(
        monoagens_core::EpisodeAdmitParams {
            event_id: EventId::new(1),
            place: &PlaceId::new("room1"),
            timestamp_ms: Time::from_millis(1_000),
            participants: p_alice.clone(),
            taint: Taint::Trusted,
            embeddings: &same_vec,
            missed_expectation: false,
        },
        &cfg,
    );
    book.episodes.get_mut(&EventId::new(1)).unwrap().state = EpisodeState::Sealed;
    book.active_by_place.remove(&PlaceId::new("room1"));

    // Different people in same place and adjacent time -> refused.
    book.admit_event(
        monoagens_core::EpisodeAdmitParams {
            event_id: EventId::new(2),
            place: &PlaceId::new("room1"),
            timestamp_ms: Time::from_millis(2_000),
            participants: p_bob,
            taint: Taint::Trusted,
            embeddings: &same_vec,
            missed_expectation: false,
        },
        &cfg,
    );
    book.episodes.get_mut(&EventId::new(2)).unwrap().state = EpisodeState::Sealed;
    assert!(book.merge_eligible_episodes().is_empty());

    // Different place, same people, adjacent time -> refused.
    book.admit_event(
        monoagens_core::EpisodeAdmitParams {
            event_id: EventId::new(3),
            place: &PlaceId::new("room2"),
            timestamp_ms: Time::from_millis(2_500),
            participants: p_alice.clone(),
            taint: Taint::Trusted,
            embeddings: &same_vec,
            missed_expectation: false,
        },
        &cfg,
    );
    book.episodes.get_mut(&EventId::new(3)).unwrap().state = EpisodeState::Sealed;
    assert!(book.merge_eligible_episodes().is_empty());

    // Same place, same people, adjacent time -> merges!
    book.active_by_place.remove(&PlaceId::new("room1"));
    book.admit_event(
        monoagens_core::EpisodeAdmitParams {
            event_id: EventId::new(4),
            place: &PlaceId::new("room1"),
            timestamp_ms: Time::from_millis(3_000),
            participants: p_alice,
            taint: Taint::Trusted,
            embeddings: &same_vec,
            missed_expectation: false,
        },
        &cfg,
    );
    book.episodes.get_mut(&EventId::new(4)).unwrap().state = EpisodeState::Sealed;
    let merges = book.merge_eligible_episodes();
    assert_eq!(merges.len(), 1);
}

#[test]
fn petrov_fade_releases_vector_keeps_text_and_gist_links() {
    let cfg = CoreConfig {
        sleep_min_episodes: 2,
        sleep_idle_ms: 60_000,
        seed: 42,
        ..CoreConfig::default()
    };
    let mut core = open_test_core(cfg);
    let mut doors = NoopDoors;

    let e1 = vec_emb([1.0, 0.0, 0.0, 0.0]);
    let p1 = make_percept(
        "alice",
        "room1",
        Taint::Trusted,
        1_000,
        "rare_keyword_cobalt_archive",
        vec![e1.clone()],
    );
    core.step(
        Input::Percept(Box::new(p1)),
        Time::from_millis(1_000),
        &mut doors,
    )
    .unwrap();
    let ev1 = *core.state().memory_entries.keys().last().unwrap();
    core.store_mut().insert_vector(ev1, e1.clone());

    // Advance 31 days (> 720h, B < -3.29) and admit a silent/non-waking event so `ev1` stays unused.
    let day_31 = 1_000 + 31 * 86_400_000;
    let mut p2 = make_percept(
        "bob",
        "room2",
        Taint::Trusted,
        day_31,
        "fresh checkin",
        vec![vec_emb([0.0, 1.0, 0.0, 0.0])],
    );
    p2.audience = Audience::new([ActorId::new("soul"), ActorId::new("bob")]);
    p2.addressees.clear();
    p2.dialogue_act_gap = Some(0.0);
    core.step(
        Input::Percept(Box::new(p2)),
        Time::from_millis(day_31),
        &mut doors,
    )
    .unwrap();

    // Create a gist linking to `ev1`.
    let gist = MemoryEntry {
        event_id: EventId::new(900),
        source_ids: vec![ev1],
        is_gist: true,
        timestamp_ms: Time::from_millis(day_31),
        place: PlaceId::new("room1"),
        actor: ActorId::new("soul"),
        origin_actor: OriginActor::new(ActorId::new("soul")),
        basis: MemoryBasis::Inferred,
        taint: Taint::Trusted,
        audience: Audience::new([ActorId::new("soul"), ActorId::new("alice")]),
        attachments: vec![],
        activation: Default::default(),
        importance: Default::default(),
        embeddings: vec![e1],
        concepts: vec![],
        superseded_by: None,
        erased: false,
    };
    core.register_memory_entry(gist);

    let due = core.next_due().unwrap();
    core.step(Input::Tick, due, &mut doors).unwrap();

    // `ev1` vector was released (`B < -3.29`), `VectorReleased` event logged,
    // yet text query and gist recollection still resolve!
    assert!(core.state().memory_entries[&ev1].embeddings.is_empty());
    let logs = core.store().read_in_order(None, 200).unwrap();
    assert!(logs.iter().any(|r| r.kind == EventKind::VectorReleased));

    let scope = VisibleScope::new(
        Audience::new([ActorId::new("soul"), ActorId::new("alice")]),
        Some(PlaceId::new("room1")),
    );
    let mut q = RecallQuery::new(scope);
    q.query_event_id = Some(ev1); // text query or gist vector recall
    q.query_embeddings = vec![vec_emb([1.0, 0.0, 0.0, 0.0])];
    q.limit = 5;
    let res = core
        .recall(&q, Time::from_millis(day_31 + 100_000))
        .unwrap();
    let gist_hit = res
        .items
        .iter()
        .find(|i| i.event_id == EventId::new(900))
        .expect("gist resolves");
    assert!(
        gist_hit.recollected,
        "gist still verifies recollection via intact source link"
    );

    // Verify FTS/text lookup on `ev1` still resolves via Store::candidates_by_text.
    let mut p_txt = make_percept(
        "alice",
        "room1",
        Taint::Trusted,
        day_31 + 200_000,
        "rare_keyword_cobalt_archive",
        vec![],
    );
    p_txt.dialogue_act_gap = Some(0.0);
    let q_rec = EventRecord::new_root(
        EventKind::Percept,
        Time::from_millis(day_31 + 200_000),
        ActorId::new("alice"),
        PlaceId::new("room1"),
        Audience::new([ActorId::new("soul"), ActorId::new("alice")]),
        OriginActor::new(ActorId::new("alice")),
        Taint::Trusted,
        NumericFeatures::default(),
        p_txt.payload,
        vec![],
        None,
    );
    let q_id = core.store_mut().append(q_rec).unwrap();
    let text_hits = core.store().candidates_by_text(q_id, 10).unwrap();
    assert!(text_hits.contains(&ev1));
}

#[test]
fn hard_forget_recomputes_credence_and_preserves_two_epoch_replay() {
    let mut core = open_test_core(CoreConfig::default());
    let mut doors = NoopDoors;

    // Pre-forget percepts (so Two-Epoch Replay has gate decisions before and after erasure).
    let p_keep = make_percept("bob", "room1", Taint::Trusted, 1_000, "bob msg", vec![]);
    core.step(
        Input::Percept(Box::new(p_keep)),
        Time::from_millis(1_000),
        &mut doors,
    )
    .unwrap();
    let p_erase = make_percept(
        "alice",
        "room1",
        Taint::Untrusted,
        2_000,
        "alice secret",
        vec![],
    );
    core.step(
        Input::Percept(Box::new(p_erase)),
        Time::from_millis(2_000),
        &mut doors,
    )
    .unwrap();

    let dom = DomainId::new("finance");
    let claim_alice = core
        .record_belief(
            "claim:alice_tip",
            MemoryBasis::Told,
            Some(ActorId::new("alice")),
            Some(dom.clone()),
            vec![],
            Taint::Untrusted,
            false,
            Time::from_millis(3_000),
        )
        .unwrap();
    let claim_bob = core
        .record_belief(
            "claim:bob_tip",
            MemoryBasis::Told,
            Some(ActorId::new("bob")),
            Some(dom.clone()),
            vec![],
            Taint::Provisional,
            false,
            Time::from_millis(4_000),
        )
        .unwrap();

    // Evidence from `alice` (to be forgotten) vs `bob` (survives).
    core.record_cited_outcome(
        &[claim_alice],
        &[],
        true,
        5.0,
        Some(ActorId::new("alice")),
        Some(dom.clone()),
        Time::from_millis(5_000),
    )
    .unwrap();
    core.record_cited_outcome(
        &[claim_bob],
        &[],
        true,
        3.0,
        Some(ActorId::new("bob")),
        Some(dom.clone()),
        Time::from_millis(6_000),
    )
    .unwrap();

    // Also add an evidence event attributed to `alice` supporting `claim_bob` so forgetting `alice`
    // must recompute `claim_bob` cleanly by dropping `alice`'s contribution!
    core.record_cited_outcome(
        &[claim_bob],
        &[],
        false,
        2.0,
        Some(ActorId::new("alice")),
        Some(dom.clone()),
        Time::from_millis(7_000),
    )
    .unwrap();

    // Execute authorized `hard_forget` erasing `alice`.
    core.hard_forget(
        vec![],
        vec![ActorId::new("alice")],
        "gdpr_erasure",
        Time::from_millis(8_000),
    )
    .unwrap();

    // `alice` reliability and belief are gone; `claim_bob` credence is recomputed WITHOUT `alice`'s -2.0!
    assert!(!core.state().belief_book.beliefs.contains_key(&claim_alice));
    assert!(
        !core
            .state()
            .belief_book
            .source_reliability
            .contains_key(&(ActorId::new("alice"), dom.clone()))
    );
    let bob_cred = core.state().belief_book.beliefs[&claim_bob]
        .credence
        .unwrap();
    assert!(
        (bob_cred.alpha - 4.0).abs() < 1e-6,
        "prior 1 + 3 = 4, got {}",
        bob_cred.alpha
    );
    assert!(
        (bob_cred.beta - 1.0).abs() < 1e-6,
        "alice's negative evidence must be erased by recomputation, got {}",
        bob_cred.beta
    );

    // Post-forget percepts & Two-Epoch Replay verification.
    let p_post = make_percept(
        "bob",
        "room1",
        Taint::Trusted,
        10_000,
        "post forget msg",
        vec![],
    );
    core.step(
        Input::Percept(Box::new(p_post)),
        Time::from_millis(10_000),
        &mut doors,
    )
    .unwrap();
    let report = core.replay_verify(ReplayMode::Pinned).unwrap();
    assert!(report.is_exact());
    assert!(core.state().identity_anchor.is_none());
}
