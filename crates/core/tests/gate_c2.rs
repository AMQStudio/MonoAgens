use std::collections::BTreeMap;

use monoagens_core::gate::{
    BacklogEventInput, CandidateMemory, CommitmentInputs, DecisionReason, ExpectationCue,
    FrozenDialogueActHead, GateSignals, InitHabituationEntry, MAX_CHUNKS_PER_WAKE,
    PendingBeliefSummary, PlaceHistoryEntry, PromiseSummary, SalienceWeights, SignalInputs,
    ThresholdInputs, breakpoint_score, compute_deg, compute_promise_debt, compute_salience,
    compute_social_cost, compute_talk_rate, compute_threshold, curiosity_drive, decide_gate,
    effective_idle_trigger_ms, effort_control, evaluate_signals, is_backlog_event,
    schedule_backlog_chunks, sleep_pressure, switching_drive,
};
use monoagens_core::{
    ActivationTrace, Core, DecisionOutcome, EventDetail, EventKind, EventRecord, Input, MemStore,
    NoopDoors, NumericFeatures, Prior, ReplayMode, Store, Switches, closeness_from_trace,
    context_base_level, decay_exponent, petrov_from_timestamps, petrov_summary, place_familiarity,
    replay_from_checkpoint, sigmoid,
};
use monoagens_protocol::{
    ActorId, Audience, EmbeddingTag, EventId, FModal, OpaquePayload, OrientScore, OriginActor,
    Percept, PlaceId, TaggedEmbedding, Taint, Time, VisibleScope,
};

fn test_tag() -> EmbeddingTag {
    EmbeddingTag {
        embed_model_ver: "embeddinggemma-2-256d".to_owned(),
        dim_d: 3,
        prompt_regime_id: "t_clf".to_owned(),
    }
}

fn make_emb(vals: [f32; 3]) -> TaggedEmbedding {
    TaggedEmbedding {
        tag: test_tag(),
        values: vals.to_vec(),
    }
}

fn make_percept(actor: &str, place: &str, audience: &[&str], ts_ms: u64) -> Percept {
    Percept::minimal(
        ActorId::new(actor),
        PlaceId::new(place),
        Audience::new(audience.iter().map(|s| ActorId::new(*s))),
        Time::from_millis(ts_ms),
        OpaquePayload::from_bytes(b"opaque".as_slice()),
    )
}

#[test]
fn petrov_guards_avoid_nan_and_closeness_familiarity_decay() {
    let now = Time::from_millis(120_000);
    assert_eq!(petrov_summary(now, now, 0, now, 0.5), 0.0);
    assert_eq!(petrov_from_timestamps(&[], now, 0.5, 1), 0.0);

    // Identical timestamps within the 1-minute floor must not divide by zero or produce NaN.
    let ts = [
        Time::from_millis(100_000),
        Time::from_millis(100_000),
        Time::from_millis(105_000),
        Time::from_millis(119_999),
    ];
    for d in [0.0, 0.3, 0.5, 0.7, 1.0] {
        let b1 = petrov_from_timestamps(&ts, now, d, 1);
        let b2 = petrov_summary(
            Time::from_millis(119_999),
            Time::from_millis(100_000),
            4,
            now,
            d,
        );
        assert!(b1.is_finite() && !b1.is_nan(), "b1 non-finite for d={d}");
        assert!(b2.is_finite() && !b2.is_nan(), "b2 non-finite for d={d}");
    }

    // Large channel with identical first-minute events and zero prior exchanges -> closeness stays 0.
    let zero_trace = ActivationTrace::default();
    assert_eq!(closeness_from_trace(&zero_trace, now, 0.5, 5.0), 0.0);
    let crowd: Vec<&ActivationTrace> = vec![&zero_trace; 100];
    assert_eq!(place_familiarity(&zero_trace, &crowd, now, 0.5, 5.0), 0.0);
    assert_eq!(context_base_level(&zero_trace, &crowd, now, 0.5), None);

    // Long gap: closeness & familiarity decay when contact stops.
    let mut active_trace = ActivationTrace::default();
    active_trace.record(Time::from_millis(0));
    active_trace.record(Time::from_millis(60_000));
    active_trace.record(Time::from_millis(120_000));
    let t_recent = Time::from_millis(180_000);
    let t_30d = Time::from_millis(180_000 + 30 * 24 * 3_600_000);
    let d = decay_exponent(0.5);

    let c_recent = closeness_from_trace(&active_trace, t_recent, d, 5.0);
    let c_30d = closeness_from_trace(&active_trace, t_30d, d, 5.0);
    assert!(c_recent > c_30d && c_30d > 0.0);

    let f_recent = place_familiarity(&active_trace, &[&active_trace], t_recent, d, 5.0);
    let f_30d = place_familiarity(&active_trace, &[&active_trace], t_30d, d, 5.0);
    assert!(f_recent > f_30d);
}

#[test]
fn neutral_parity_for_all_signal_and_threshold_weights() {
    let signals = GateSignals {
        own_name: 1.0,
        dialogue_act_gap: 0.9,
        asr_clarification_boost: 0.35,
        info_gap: 1.0,
        own_info_gap: 0.8,
        feeling_of_knowing: 0.7,
        relevance: 0.6,
        commitment: 0.5,
        deg_soul: 0.4,
        max_deg_other: 0.9,
        sum_deg_listeners: 1.3,
        answered: 1.0,
        closeness_sender: 0.0,
        social_cost: 0.9,
    };

    // helpfulness = 0 => w_info == 0, w_fok == 0
    let mut prior = Prior {
        helpfulness: 0.0,
        ..Prior::default()
    };
    let w_no_help = SalienceWeights::from_prior(&prior, &Switches::all_enabled());
    assert_eq!(w_no_help.w_info, 0.0);
    assert_eq!(w_no_help.w_fok, 0.0);

    // restraint = 0 => w_sc == 0, a == 0
    prior = Prior {
        restraint: 0.0,
        ..Prior::default()
    };
    let w_no_rest = SalienceWeights::from_prior(&prior, &Switches::all_enabled());
    assert_eq!(w_no_rest.w_sc, 0.0);
    let thr_no_rest = compute_threshold(
        &ThresholdInputs {
            now_ms: Time::from_millis(10_000),
            soul_acts_in_window: 10,
            total_acts_in_window: 12,
            active_speakers: 3,
            budget_used_ratio: 0.5,
            promises: &[],
            familiarity: 0.2,
            init_habituation: None,
            is_idle_initiative: false,
        },
        &prior,
        &Switches::all_enabled(),
    );
    assert_eq!(thr_no_rest.a, 0.0);

    // Switches::floor_cost = false => w_sc == 0
    let mut sw = Switches::all_enabled();
    sw.floor_cost = false;
    let w_floor_off = SalienceWeights::from_prior(&Prior::default(), &sw);
    assert_eq!(w_floor_off.w_sc, 0.0);

    // Switches::salience = false => all w_k == 0, S == 0
    sw = Switches::all_enabled();
    sw.salience = false;
    let s_off = compute_salience(&signals, &Prior::default(), &sw);
    assert_eq!(s_off.weights, SalienceWeights::default());
    assert_eq!(s_off.raw_salience, 0.0);

    // Switches::all_neutral => a = b = c = r = 0, kappa = 0 -> S_eff = S
    let thr_neutral = compute_threshold(
        &ThresholdInputs {
            now_ms: Time::from_millis(10_000),
            soul_acts_in_window: 10,
            total_acts_in_window: 12,
            active_speakers: 3,
            budget_used_ratio: 0.9,
            promises: &[PromiseSummary {
                due_ms: Time::from_millis(20_000),
                urgency_weight: 0.8,
                is_recurring_monitor: false,
                active: true,
            }],
            familiarity: 0.0,
            init_habituation: None,
            is_idle_initiative: false,
        },
        &Prior::default(),
        &Switches::all_neutral(),
    );
    assert_eq!(thr_neutral.a, 0.0);
    assert_eq!(thr_neutral.b, 0.0);
    assert_eq!(thr_neutral.c, 0.0);
    assert_eq!(thr_neutral.r, 0.0);
    assert_eq!(thr_neutral.delta_theta_sleep, 0.0);

    // kappa = 0 neutral habituation
    let prior_k0 = Prior {
        kappa: 0.0,
        ..Prior::default()
    };
    let out_k0 = decide_gate(
        EventId::new(1),
        7,
        signals.clone(),
        compute_salience(&signals, &prior_k0, &Switches::all_enabled()),
        thr_no_rest,
        5.0,
        CommitmentInputs::default(),
        &prior_k0,
        &Switches::all_enabled(),
    );
    assert_eq!(out_k0.trace.habituation_factor, 1.0);
    assert_eq!(out_k0.trace.effective_salience, out_k0.trace.raw_salience);
}

#[test]
fn frozen_info_gap_head_asr_boost_and_visiblescope_privacy() {
    let prior = Prior::default();
    let soul = ActorId::new("soul");
    let sender = ActorId::new("per_alice");
    let now = Time::from_millis(100_000);

    // Frozen contrastive logistic head distinguishing question vs topical statement.
    let head = FrozenDialogueActHead {
        w_gap: vec![4.0, -4.0, 0.0],
        b_gap: 0.0,
        expected_tag: Some(test_tag()),
    };

    // Private memory in narrow audience [per_bob] vs public room [per_alice, per_bob, soul].
    let mut private_trace = ActivationTrace::default();
    private_trace.record(Time::from_millis(90_000));
    let candidates = vec![CandidateMemory {
        event_id: EventId::new(50),
        audience: Audience::new([ActorId::new("per_bob")]),
        activation: private_trace,
        importance: 1.0,
        embeddings: vec![make_emb([0.0, 1.0, 0.0])],
    }];

    let public_scope = VisibleScope::new(
        Audience::new([ActorId::new("per_alice"), ActorId::new("soul")]),
        Some(PlaceId::new("room:1")),
    );

    // Statement sharing topic with private memory (e = [0.0, 1.0, 0.0]):
    // dialogue_act_gap < 0.5 AND private memory MUST NOT affect FOK or relevance!
    let stmt_emb = [make_emb([0.0, 1.0, 0.0])];
    let (stmt_sig, _) = evaluate_signals(
        &SignalInputs {
            soul_id: &soul,
            sender: &sender,
            timestamp_ms: now,
            audience: &public_scope.audience,
            addressees: &[],
            named: &[],
            orient: &[],
            reply_to_actor: None,
            f_modal: None,
            embeddings: &stmt_emb,
            precomputed_dialogue_act_gap: None,
            is_dyadic_chat: false,
            answered: 0.0,
            sender_exchange_trace: ActivationTrace::default(),
            place_history: &[],
            active_participants: &[],
            visible_scope: &public_scope,
            candidate_memories: &candidates,
            expectation_cues: &[],
            pending_beliefs: &[],
            dialogue_act_head: Some(&head),
            is_idle_probe: false,
            curiosity_override: None,
        },
        &prior,
    );
    assert!(
        stmt_sig.dialogue_act_gap < 0.1,
        "statement suppressed by head"
    );
    assert_eq!(
        stmt_sig.feeling_of_knowing, 0.0,
        "private memory must not leak into FOK"
    );
    assert_eq!(
        stmt_sig.relevance, 0.0,
        "private memory must not leak into relevance"
    );

    // Request with low ASR confidence (< 0.45) adds clarification boost; missing embeddings degrade to 0.
    let req_emb = [make_emb([1.0, 0.0, 0.0])];
    let (req_sig, _) = evaluate_signals(
        &SignalInputs {
            soul_id: &soul,
            sender: &sender,
            timestamp_ms: now,
            audience: &public_scope.audience,
            addressees: &[],
            named: std::slice::from_ref(&soul),
            orient: &[],
            reply_to_actor: None,
            f_modal: Some(FModal([
                2.0, 1.0, 0.8, 0.0, 0.0, 0.0, 100.0, 0.9, 0.1, 0.30,
            ])),
            embeddings: &req_emb,
            precomputed_dialogue_act_gap: None,
            is_dyadic_chat: false,
            answered: 0.0,
            sender_exchange_trace: ActivationTrace::default(),
            place_history: &[],
            active_participants: &[],
            visible_scope: &public_scope,
            candidate_memories: &[],
            expectation_cues: &[],
            pending_beliefs: &[],
            dialogue_act_head: Some(&head),
            is_idle_probe: false,
            curiosity_override: None,
        },
        &prior,
    );
    assert!(req_sig.dialogue_act_gap > 0.9);
    assert_eq!(
        req_sig.asr_clarification_boost,
        prior.asr_clarification_boost
    );
    assert_eq!(req_sig.info_gap, 1.0);
    assert_eq!(req_sig.own_name, 1.0);

    // Missing embeddings degrade vector signals to 0 while structural own_name still works.
    let (no_vec_sig, _) = evaluate_signals(
        &SignalInputs {
            soul_id: &soul,
            sender: &sender,
            timestamp_ms: now,
            audience: &public_scope.audience,
            addressees: &[],
            named: std::slice::from_ref(&soul),
            orient: &[],
            reply_to_actor: None,
            f_modal: None,
            embeddings: &[],
            precomputed_dialogue_act_gap: None,
            is_dyadic_chat: false,
            answered: 0.0,
            sender_exchange_trace: ActivationTrace::default(),
            place_history: &[],
            active_participants: &[],
            visible_scope: &public_scope,
            candidate_memories: &candidates,
            expectation_cues: &[],
            pending_beliefs: &[],
            dialogue_act_head: Some(&head),
            is_idle_probe: false,
            curiosity_override: None,
        },
        &prior,
    );
    assert_eq!(no_vec_sig.dialogue_act_gap, 0.0);
    assert_eq!(no_vec_sig.feeling_of_knowing, 0.0);
    assert_eq!(no_vec_sig.relevance, 0.0);
    assert_eq!(no_vec_sig.own_name, 1.0);
}

#[test]
fn delta_theta_sleep_is_zero_talk_rate_shrinks_and_promise_debt_excludes_monitors() {
    let prior = Prior::default();
    let switches = Switches::all_enabled();
    let now = Time::from_millis(10_000_000);

    // Cold-start talk_rate shrinkage (A-07).
    assert_eq!(compute_talk_rate(0, 0, 4, prior.talk_rate_prior_count), 0.0);
    let tr_one = compute_talk_rate(1, 1, 2, prior.talk_rate_prior_count);
    let tr_unshrunk = compute_talk_rate(1, 1, 2, 0.0);
    assert!(tr_one < tr_unshrunk && tr_one > 0.0);
    let tr_heavy = compute_talk_rate(18, 20, 4, prior.talk_rate_prior_count);
    assert!(tr_heavy > tr_one);

    // promise_debt sums imminent non-recurring promises and excludes recurring monitors.
    let promises = [
        PromiseSummary {
            due_ms: now.saturating_add_ms(600_000),
            urgency_weight: 0.8,
            is_recurring_monitor: false,
            active: true,
        },
        PromiseSummary {
            due_ms: now.saturating_add_ms(900_000),
            urgency_weight: 0.5,
            is_recurring_monitor: false,
            active: true,
        },
        PromiseSummary {
            due_ms: now.saturating_add_ms(300_000),
            urgency_weight: 1.0,
            is_recurring_monitor: true, // recurring monitor MUST be excluded
            active: true,
        },
        PromiseSummary {
            due_ms: now.saturating_add_ms(10_000_000), // beyond horizon
            urgency_weight: 1.0,
            is_recurring_monitor: false,
            active: true,
        },
    ];
    let debt = compute_promise_debt(&promises, now, prior.promise_debt_horizon_ms);
    assert!((debt - 1.3).abs() < 1e-12, "expected 1.3, got {debt}");

    let terms = compute_threshold(
        &ThresholdInputs {
            now_ms: now,
            soul_acts_in_window: 18,
            total_acts_in_window: 20,
            active_speakers: 4,
            budget_used_ratio: 0.9,
            promises: &promises,
            familiarity: 0.3,
            init_habituation: Some(InitHabituationEntry {
                h_init: 2.0,
                last_updated_ms: now,
            }),
            is_idle_initiative: false,
        },
        &prior,
        &switches,
    );
    assert_eq!(terms.delta_theta_sleep, 0.0, "A-04 Delta theta_sleep = 0");
    assert_eq!(
        terms.delta_theta_init, 0.0,
        "external event keeps Delta theta_init = 0"
    );

    // Idle initiative gets Delta theta_init from init_habituation, still keeping Delta theta_sleep = 0.
    let idle_terms = compute_threshold(
        &ThresholdInputs {
            now_ms: now,
            soul_acts_in_window: 0,
            total_acts_in_window: 5,
            active_speakers: 2,
            budget_used_ratio: 0.0,
            promises: &[],
            familiarity: 0.5,
            init_habituation: Some(InitHabituationEntry {
                h_init: 2.0,
                last_updated_ms: now,
            }),
            is_idle_initiative: true,
        },
        &prior,
        &switches,
    );
    assert_eq!(idle_terms.delta_theta_sleep, 0.0);
    assert!((idle_terms.delta_theta_init - 0.2).abs() < 1e-12);

    // R-30 breakpoint & R-31 drives sanity.
    assert_eq!(
        breakpoint_score(
            now,
            Some(Time::from_millis(now.as_millis().saturating_sub(5_000))),
            false,
            false,
            0,
            0,
            1000,
            &prior
        ),
        1.0
    );
    assert_eq!(
        breakpoint_score(now, None, false, false, 0, 5_000, 1_000, &prior),
        0.5
    );
    assert_eq!(
        breakpoint_score(now, None, false, false, 1, 5_000, 1_000, &prior),
        0.0
    );

    let d_sl = sleep_pressure(prior.n_min_sleep * 2, 12 * 3_600_000, &prior);
    assert!(d_sl > 1.0 && d_sl <= prior.d_sleep_max);
    assert!(effective_idle_trigger_ms(60_000, d_sl) < 60_000);

    let sw_b = switching_drive(4, 3, 0.1, &prior);
    assert!(sw_b.d_switch > 0.0 && sw_b.effective_b_steps <= prior.b_steps);

    let cur = curiosity_drive(
        0.8,
        0.5,
        0.2,
        &[PendingBeliefSummary {
            alpha: 1.5,
            beta: 1.5,
            stakes: 1.0,
            depends_on_active_goal: true,
        }],
        &prior,
    );
    assert!(cur > 0.2);

    let low_stakes = effort_control(0, 1.0, 0.0, 2, 0, 0.1, 0.8, &prior);
    let high_stakes = effort_control(0, 1.0, 0.0, 2, 0, 1.0, 0.1, &prior);
    assert!(low_stakes.evc_marg < high_stakes.evc_marg);
}

#[test]
fn seeded_draw_determinism_convergence_and_commitment_bypass() {
    let prior = Prior::default();
    let switches = Switches::all_enabled();
    let signals = GateSignals {
        info_gap: 0.6,
        ..GateSignals::default()
    };
    let salience = compute_salience(&signals, &prior, &switches);
    let threshold = compute_threshold(
        &ThresholdInputs {
            now_ms: Time::from_millis(1000),
            soul_acts_in_window: 0,
            total_acts_in_window: 0,
            active_speakers: 2,
            budget_used_ratio: 0.0,
            promises: &[],
            familiarity: 0.5,
            init_habituation: None,
            is_idle_initiative: false,
        },
        &prior,
        &switches,
    );

    // Determinism: same event twice => identical P, draw, outcome.
    let g1 = decide_gate(
        EventId::new(77),
        12345,
        signals.clone(),
        salience,
        threshold,
        0.0,
        CommitmentInputs::default(),
        &prior,
        &switches,
    );
    let g2 = decide_gate(
        EventId::new(77),
        12345,
        signals.clone(),
        salience,
        threshold,
        0.0,
        CommitmentInputs::default(),
        &prior,
        &switches,
    );
    assert_eq!(g1.probability, g2.probability);
    assert_eq!(g1.draw, g2.draw);
    assert_eq!(g1.outcome, g2.outcome);

    // Frequency convergence near P = 0.3 over many distinct events.
    // Set S_eff - theta = tau * ln(0.3 / 0.7) so sigma((S_eff - theta)/tau) == 0.3 exactly.
    let tau = prior.tau();
    let target_diff = tau * (0.3_f64 / 0.7_f64).ln();
    let mut conv_sal = salience;
    conv_sal.raw_salience = (threshold.theta + target_diff).clamp(0.0, 1.0);
    let mut conv_thr = threshold;
    conv_thr.theta = conv_sal.raw_salience - target_diff;

    let n_events = 2000u64;
    let mut goes = 0usize;
    for id in 1..=n_events {
        let out = decide_gate(
            EventId::new(id),
            999,
            signals.clone(),
            conv_sal,
            conv_thr,
            0.0,
            CommitmentInputs::default(),
            &prior,
            &switches,
        );
        assert!((out.probability - 0.3).abs() < 1e-12);
        if out.outcome == DecisionOutcome::Go {
            goes += 1;
        }
    }
    let go_share = (goes as f64) / (n_events as f64);
    assert!(
        (go_share - 0.3).abs() < 0.03,
        "expected go share near 0.3, got {go_share}"
    );

    // Commitment bypass: zero salience still fires Go with reason Commitment.
    let zero_sal = compute_salience(&GateSignals::default(), &prior, &switches);
    let comm_out = decide_gate(
        EventId::new(5001),
        999,
        GateSignals::default(),
        zero_sal,
        threshold,
        0.0,
        CommitmentInputs {
            turn_allocated: true,
            fires_or_answers_expectation: false,
            continues_active_chain: false,
        },
        &prior,
        &switches,
    );
    assert_eq!(comm_out.outcome, DecisionOutcome::Go);
    assert_eq!(comm_out.reason, DecisionReason::Commitment);
    assert_eq!(comm_out.probability, 1.0);

    // Row-6 neutral (tau -> 0 step limit).
    let mut sw_det = Switches::all_enabled();
    sw_det.stochastic_go_no_go = false;
    let step_low = decide_gate(
        EventId::new(1),
        1,
        GateSignals::default(),
        zero_sal,
        threshold,
        0.0,
        CommitmentInputs::default(),
        &prior,
        &sw_det,
    );
    assert_eq!(step_low.outcome, DecisionOutcome::Silent);
    assert_eq!(step_low.reason, DecisionReason::BelowNoteBand);
    assert_eq!(step_low.probability, 0.0);
}

#[test]
fn continuous_deg_and_social_cost_cases() {
    let prior = Prior::default();
    let soul = ActorId::new("soul");
    let alice = ActorId::new("per_alice");
    let bob = ActorId::new("per_bob");
    let now = Time::from_millis(500_000);
    let aud = Audience::new([alice.clone(), bob.clone(), soul.clone()]);

    // Case 1: Bob explicitly addressed => high deg(e, bob) == 1.0, low deg(e, soul) => high social_cost.
    let deg_bob = compute_deg(
        &soul,
        &alice,
        now,
        &aud,
        std::slice::from_ref(&bob),
        &[],
        &[],
        None,
        &[],
        &[],
        &[alice.clone(), bob.clone(), soul.clone()],
        false,
        &prior,
    );
    assert_eq!(deg_bob.deg_by_actor.get(&bob).copied(), Some(1.0));
    assert_eq!(deg_bob.deg_soul, 0.0);
    assert_eq!(deg_bob.sel_other, 1.0);

    let sc_stranger = compute_social_cost(deg_bob.sel_other, 0.0, 0.0, prior.lambda_c);
    let sc_close = compute_social_cost(deg_bob.sel_other, 0.0, 0.8, prior.lambda_c);
    assert_eq!(sc_stranger, 1.0);
    assert!(sc_close < sc_stranger);

    // Case 2: answered = 1 pushes social_cost to (1 - lambda_c * closeness) even if sel_other = 0.
    let sc_answered = compute_social_cost(0.0, 1.0, 0.5, prior.lambda_c);
    assert!((sc_answered - (1.0 - prior.lambda_c * 0.5)).abs() < 1e-12);

    // Case 3: Embodied orient + dyadic momentum continuity.
    let mut deg_map = BTreeMap::new();
    deg_map.insert(soul.clone(), 0.9);
    let hist = vec![PlaceHistoryEntry {
        event_id: EventId::new(1),
        actor: alice.clone(),
        timestamp_ms: Time::from_millis(490_000),
        deg_by_actor: deg_map,
    }];
    let deg_orient = compute_deg(
        &soul,
        &alice,
        now,
        &aud,
        &[],
        &[],
        &[OrientScore {
            actor: soul.clone(),
            score: 0.9,
        }],
        None,
        &[],
        &hist,
        &[alice.clone(), bob.clone(), soul.clone()],
        false,
        &prior,
    );
    assert!(deg_orient.deg_soul > 0.7);
    assert!(
        compute_deg(
            &soul,
            &alice,
            now,
            &aud,
            &[],
            &[],
            &[],
            None,
            &[],
            &hist,
            &[alice.clone(), bob.clone(), soul.clone()],
            false,
            &prior,
        )
        .deg_soul
            < prior.theta_alloc
    );
    let _ = ExpectationCue {
        expectation_id: None,
        audience: aud,
        counterparty: None,
        answers_event: false,
        embeddings: Vec::new(),
    };
}

#[test]
fn backlog_chunking_caps_at_four_never_triggers_on_sleep_and_honors_commitments() {
    let prior = Prior::default();
    let switches = Switches::all_enabled();
    let now = Time::from_millis(100_000);

    // Live event during background sleep is NOT backlog.
    assert!(!is_backlog_event(
        Time::from_millis(99_000),
        now,
        prior.patience_place_ms,
        false,
    ));
    // Event exceeding patience_place OR after genuine stopped/down downtime IS backlog.
    assert!(is_backlog_event(
        Time::from_millis(10_000),
        now,
        prior.patience_place_ms,
        false,
    ));
    assert!(is_backlog_event(
        Time::from_millis(99_000),
        now,
        prior.patience_place_ms,
        true,
    ));

    // Six places with backlogs (including 30 messages in place-1 and an allocated turn in place-6).
    let mut events = Vec::new();
    for i in 1..=30u64 {
        events.push(BacklogEventInput {
            event_id: EventId::new(i),
            place: PlaceId::new("place-1"),
            timestamp_ms: Time::from_millis(50_000 + i * 100),
            signals: GateSignals {
                info_gap: 0.4,
                ..GateSignals::default()
            },
            commitments: CommitmentInputs::default(),
        });
    }
    for p in 2..=6u64 {
        let allocated = p == 6;
        events.push(BacklogEventInput {
            event_id: EventId::new(100 + p),
            place: PlaceId::new(format!("place-{p}")),
            timestamp_ms: Time::from_millis(50_000),
            signals: GateSignals {
                info_gap: (p as f64) * 0.15,
                ..GateSignals::default()
            },
            commitments: CommitmentInputs {
                turn_allocated: allocated,
                ..CommitmentInputs::default()
            },
        });
    }

    let scheduled = schedule_backlog_chunks(&events, now, &prior, &switches);
    assert_eq!(scheduled.handle_now.len(), MAX_CHUNKS_PER_WAKE);
    assert_eq!(scheduled.deferred.len(), 2);
    // place-6 had an allocated turn so it is prioritized first and carries commitment.
    assert_eq!(scheduled.handle_now[0].place, PlaceId::new("place-6"));
    assert!(scheduled.handle_now[0].commitments.any());

    let p1_chunk = scheduled
        .handle_now
        .iter()
        .chain(scheduled.deferred.iter())
        .find(|c| c.place == PlaceId::new("place-1"))
        .expect("place-1 chunk present");
    assert_eq!(p1_chunk.event_ids.len(), 30);
}

#[test]
fn decision_recomputability_and_two_epoch_replay_with_anchor_and_divergence() {
    let prior = Prior::default();
    let switches = Switches::all_enabled();
    let seed = 4242_u64;
    let mut core =
        Core::open(MemStore::new(), prior.clone(), switches.clone(), seed).expect("open");
    let mut doors = NoopDoors;

    // Step several percepts in Epoch 0.
    let mut p1 = make_percept("per_alice", "chat:room", &["per_alice", "soul"], 1_000);
    p1.named = vec![ActorId::new("soul")];
    core.step(
        Input::Percept(Box::new(p1)),
        Time::from_millis(1_000),
        &mut doors,
    )
    .unwrap();

    let p2 = make_percept("per_bob", "chat:room", &["per_bob", "soul"], 2_000);
    core.step(
        Input::Percept(Box::new(p2)),
        Time::from_millis(2_000),
        &mut doors,
    )
    .unwrap();

    // Verify every recorded Decision is recomputable from the event alone.
    let all_records = core.store().read_in_order(None, 500).unwrap();
    let decision_records: Vec<&EventRecord> = all_records
        .iter()
        .filter(|r| r.kind == EventKind::Decision)
        .collect();
    assert_eq!(decision_records.len(), 2);
    for dec_rec in &decision_records {
        let recomp = dec_rec
            .recompute_decision_outcome()
            .expect("decision trace present");
        let Some(EventDetail::Decision {
            outcome,
            salience,
            threshold,
            probability,
            reason,
            ..
        }) = dec_rec.detail
        else {
            panic!("expected Decision detail");
        };
        assert_eq!(recomp.outcome, outcome);
        assert_eq!(recomp.effective_salience.to_bits(), salience.to_bits());
        assert_eq!(recomp.threshold.to_bits(), threshold.to_bits());
        assert_eq!(recomp.probability.to_bits(), probability.to_bits());
        assert_eq!(Some(recomp.reason), reason);
    }

    // Simulate an authorized HardForget erasure event and take a clean post-erasure snapshot.
    let forget_rec = EventRecord::new_root(
        EventKind::HardForget,
        Time::from_millis(3_000),
        ActorId::new("core"),
        PlaceId::new("core:forget"),
        Audience::new([ActorId::new("soul")]),
        OriginActor::new(ActorId::new("core")),
        Taint::Trusted,
        NumericFeatures::default(),
        OpaquePayload::default(),
        Vec::new(),
        Some(EventDetail::HardForget {
            target_event_ids: vec![EventId::new(1)],
            target_actor_ids: vec![ActorId::new("per_alice")],
            epoch: 1,
            reason: "gdpr_erasure".to_owned(),
        }),
    );
    let forget_id = core.store_mut().append(forget_rec).unwrap();
    let forget_persisted = core.store().get(forget_id).unwrap().unwrap();
    // Apply erasure via reopen or state apply, then take post-erasure snapshot.
    let mut core = Core::open(core.store().clone(), prior.clone(), switches.clone(), seed)
        .expect("reopen after hard_forget");
    assert_eq!(core.state().epoch, 1);
    assert!(core.state().epoch_anchor.frozen);
    assert!(core.state().epoch_anchor.frozen_weights.is_some());
    assert!(
        !core
            .state()
            .actor_exchanges
            .contains_key(&ActorId::new("per_alice"))
    );
    let _ = forget_persisted;

    let ckpt = core.take_snapshot("post-forget-epoch-1").expect("snapshot");

    // Record Epoch 1 events after the snapshot.
    let mut p3 = make_percept("per_carol", "chat:room", &["per_carol", "soul"], 4_000);
    p3.addressees = vec![ActorId::new("soul")];
    core.step(
        Input::Percept(Box::new(p3)),
        Time::from_millis(4_000),
        &mut doors,
    )
    .unwrap();

    let p4 = make_percept("per_carol", "chat:room", &["per_carol", "soul"], 5_000);
    core.step(
        Input::Percept(Box::new(p4)),
        Time::from_millis(5_000),
        &mut doors,
    )
    .unwrap();

    // Two-Epoch Replay from post-erasure snapshot must match bit-for-bit and preserve anchor.
    let report = core
        .replay_verify(ReplayMode::TwoEpochFrozen)
        .expect("replay ok");
    assert!(
        report.is_bit_identical(),
        "divergence: {:?}",
        report.first_divergence
    );
    assert_eq!(report.epoch, 1);
    assert!(report.anchor_preserved);
    assert_eq!(report.matched_decisions, 2);
    assert_eq!(report.snapshot_cursor, Some(ckpt.cursor));

    // Also verify first-divergence reporting when a recorded Decision differs.
    let mut tampered_store = MemStore::new();
    let mut modified_once = false;
    for mut rec in core.store().read_in_order(None, 500).unwrap() {
        if rec.seq > ckpt.cursor
            && !modified_once
            && let Some(EventDetail::Decision {
                ref mut threshold, ..
            }) = rec.detail
        {
            *threshold += 0.125;
            modified_once = true;
        }
        tampered_store.append(rec).unwrap();
    }
    assert!(modified_once);
    let div_report = replay_from_checkpoint(
        &tampered_store,
        Some(&ckpt),
        &prior,
        &switches,
        seed,
        ReplayMode::TwoEpochFrozen,
    )
    .expect("replay reports divergence");
    assert!(!div_report.is_bit_identical());
    let div = div_report.first_divergence.expect("first divergence");
    assert_eq!(div.field, "threshold");
    assert_ne!(
        div.recorded.threshold.to_bits(),
        div.recomputed.threshold.to_bits()
    );
    assert!(sigmoid(0.0) == 0.5);
}
