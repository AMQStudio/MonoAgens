use std::collections::HashMap;
use std::sync::Arc;

use monoagens_core::gate::{
    BacklogEventInput, CommitmentInputs, DecisionReason, FrozenDialogueActHead, MAX_CHUNKS_PER_WAKE,
    PlaceHistoryEntry, decide_gate, is_backlog_event, schedule_backlog_chunks,
};
use monoagens_core::neural::{
    AdaptConfig, AdaptFeedback, GateArm, NeuralModel, PlaceAdaptState, cosine_similarity,
    extract_features_from_state,
};
use monoagens_core::{
    ActivationTrace, Core, CoreConfig, CoreState, DecisionOutcome, EventDetail, EventKind,
    EventRecord, Input, MemStore, NoopDoors, NumericFeatures, ReplayMode, Store,
    closeness_from_trace, context_base_level, decay_exponent, petrov_from_timestamps,
    petrov_summary, place_familiarity, replay_from_checkpoint, sigmoid,
};
use monoagens_protocol::{
    ActorId, Audience, EmbeddingTag, EventId, FModal, OpaquePayload, OriginActor, Percept, PlaceId,
    TaggedEmbedding, Taint, Time, VisibleScope,
};
use safetensors::tensor::{Dtype, TensorView};

fn test_tag() -> EmbeddingTag {
    EmbeddingTag {
        embed_model_ver: "embeddinggemma-2-256d".to_owned(),
        dim_d: 256,
        prompt_regime_id: "t_clf".to_owned(),
    }
}

fn make_emb_256(first: f32, second: f32) -> TaggedEmbedding {
    let mut values = vec![0.0_f32; 256];
    values[0] = first;
    values[1] = second;
    TaggedEmbedding {
        tag: test_tag(),
        values,
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

fn f32_bytes(vals: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(vals.len() * 4);
    for &v in vals {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

fn build_synthetic_model(gate_bias: [f32; 4], cic_threshold: f32, tabm_k: usize) -> Arc<NeuralModel> {
    let d_in_emb = 256;
    let d_in_num = 18;
    let n_freq = 2;
    let d_e = 4;
    let d_n = 4;
    let d_model = 8;
    let d_ff = 16;
    let h_dim = 64;

    let data: Vec<(String, Vec<usize>, Vec<u8>)> = vec![
        (
            "stem.emb.linear.weight".into(),
            vec![d_e, d_in_emb],
            f32_bytes(&vec![0.0; d_e * d_in_emb]),
        ),
        (
            "stem.emb.linear.bias".into(),
            vec![d_e],
            f32_bytes(&vec![0.0; d_e]),
        ),
        (
            "stem.emb.norm.weight".into(),
            vec![d_e],
            f32_bytes(&vec![1.0; d_e]),
        ),
        (
            "stem.emb.norm.bias".into(),
            vec![d_e],
            f32_bytes(&vec![0.0; d_e]),
        ),
        (
            "stem.plr.freqs".into(),
            vec![d_in_num, n_freq],
            f32_bytes(&vec![1.0; d_in_num * n_freq]),
        ),
        (
            "stem.plr.linear.weight".into(),
            vec![d_n, d_in_num * 2 * n_freq],
            f32_bytes(&vec![0.0; d_n * d_in_num * 2 * n_freq]),
        ),
        (
            "stem.plr.linear.bias".into(),
            vec![d_n],
            f32_bytes(&vec![0.0; d_n]),
        ),
        (
            "stem.fusion.weight".into(),
            vec![d_model, d_e + d_n + h_dim],
            f32_bytes(&vec![0.0; d_model * (d_e + d_n + h_dim)]),
        ),
        (
            "stem.fusion.bias".into(),
            vec![d_model],
            f32_bytes(&vec![0.0; d_model]),
        ),
        (
            "trunk.tabm_in_scale".into(),
            vec![tabm_k, d_model],
            f32_bytes(&vec![1.0; tabm_k * d_model]),
        ),
        (
            "trunk.blocks.0.norm.weight".into(),
            vec![d_model],
            f32_bytes(&vec![1.0; d_model]),
        ),
        (
            "trunk.blocks.0.norm.bias".into(),
            vec![d_model],
            f32_bytes(&vec![0.0; d_model]),
        ),
        (
            "trunk.blocks.0.w_gate.weight".into(),
            vec![d_ff, d_model],
            f32_bytes(&vec![0.0; d_ff * d_model]),
        ),
        (
            "trunk.blocks.0.w_up.weight".into(),
            vec![d_ff, d_model],
            f32_bytes(&vec![0.0; d_ff * d_model]),
        ),
        (
            "trunk.blocks.0.w_down.weight".into(),
            vec![d_model, d_ff],
            f32_bytes(&vec![0.0; d_model * d_ff]),
        ),
        (
            "trunk.blocks.0.tabm_r".into(),
            vec![tabm_k, d_model],
            f32_bytes(&vec![1.0; tabm_k * d_model]),
        ),
        (
            "trunk.blocks.0.tabm_s".into(),
            vec![tabm_k, d_model],
            f32_bytes(&vec![1.0; tabm_k * d_model]),
        ),
        (
            "trunk.final_norm.weight".into(),
            vec![d_model],
            f32_bytes(&vec![1.0; d_model]),
        ),
        (
            "trunk.final_norm.bias".into(),
            vec![d_model],
            f32_bytes(&vec![0.0; d_model]),
        ),
        (
            "memory.mingru.w_z.weight".into(),
            vec![h_dim, d_model],
            f32_bytes(&vec![0.0; h_dim * d_model]),
        ),
        (
            "memory.mingru.w_z.bias".into(),
            vec![h_dim],
            f32_bytes(&vec![0.0; h_dim]),
        ),
        (
            "memory.mingru.w_h.weight".into(),
            vec![h_dim, d_model],
            f32_bytes(&vec![0.0; h_dim * d_model]),
        ),
        (
            "memory.mingru.w_h.bias".into(),
            vec![h_dim],
            f32_bytes(&vec![0.0; h_dim]),
        ),
        (
            "head.gate_4.weight".into(),
            vec![4, d_model],
            f32_bytes(&vec![0.0; 4 * d_model]),
        ),
        ("head.gate_4.bias".into(), vec![4], f32_bytes(&gate_bias)),
        (
            "head.wait_delta_t.weight".into(),
            vec![1, d_model],
            f32_bytes(&vec![0.0; d_model]),
        ),
        (
            "head.wait_delta_t.bias".into(),
            vec![1],
            f32_bytes(&[0.0]),
        ),
    ];

    let views: Vec<(&str, TensorView<'_>)> = data
        .iter()
        .map(|(name, shape, bytes)| {
            (
                name.as_str(),
                TensorView::new(Dtype::F32, shape.clone(), bytes).expect("tensor view"),
            )
        })
        .collect();

    let mut meta = HashMap::new();
    meta.insert("feature_schema_version".into(), "1".into());
    meta.insert("d_in_emb".into(), d_in_emb.to_string());
    meta.insert("d_in_num".into(), d_in_num.to_string());
    meta.insert("d_e".into(), d_e.to_string());
    meta.insert("d_n".into(), d_n.to_string());
    meta.insert("d_model".into(), d_model.to_string());
    meta.insert("n_layers".into(), "1".into());
    meta.insert("tabm_k".into(), tabm_k.to_string());
    meta.insert("h_dim".into(), h_dim.to_string());
    meta.insert("cell_type".into(), "mingru".into());
    meta.insert("temperature".into(), "1.0".into());
    meta.insert("cic_threshold".into(), cic_threshold.to_string());
    meta.insert("max_wait_ms".into(), "4000.0".into());

    let bytes = safetensors::serialize(views, Some(meta)).expect("serialize safetensors");
    Arc::new(NeuralModel::from_safetensors_bytes(&bytes, None).expect("load synthetic model"))
}

#[test]
fn petrov_guards_avoid_nan_and_closeness_familiarity_decay() {
    let now = Time::from_millis(120_000);
    assert_eq!(petrov_summary(now, now, 0, now, 0.5), 0.0);
    assert_eq!(petrov_from_timestamps(&[], now, 0.5, 1), 0.0);

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

    let zero_trace = ActivationTrace::default();
    assert_eq!(closeness_from_trace(&zero_trace, now, 0.5, 5.0), 0.0);
    let crowd: Vec<&ActivationTrace> = vec![&zero_trace; 100];
    assert_eq!(place_familiarity(&zero_trace, &crowd, now, 0.5, 5.0), 0.0);
    assert_eq!(context_base_level(&zero_trace, &crowd, now, 0.5), None);

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
fn v2_x_num_feature_extraction_visiblescope_privacy_and_vectorless_degradation() {
    let mut state = CoreState::new();
    let cfg = CoreConfig::default();
    let now = Time::from_millis(100_000);

    // Private memory in narrow audience [per_bob] vs public room [per_alice, soul].
    let mut priv_act = ActivationTrace::default();
    priv_act.record(Time::from_millis(90_000));
    state.upsert_memory_entry(monoagens_core::MemoryEntry {
        event_id: EventId::new(50),
        source_ids: vec![EventId::new(50)],
        is_gist: false,
        timestamp_ms: Time::from_millis(90_000),
        place: PlaceId::new("dm:bob"),
        actor: ActorId::new("per_bob"),
        origin_actor: OriginActor::new(ActorId::new("per_bob")),
        basis: monoagens_core::MemoryBasis::Observed,
        taint: Taint::Trusted,
        audience: Audience::new([ActorId::new("per_bob"), ActorId::new("soul")]),
        embeddings: vec![make_emb_256(0.0, 1.0)],
        attachments: Vec::new(),
        activation: priv_act,
        superseded_by: None,
        importance: monoagens_core::ImportanceInputs {
            i_reason: 1.0,
            ..Default::default()
        },
        concepts: Vec::new(),
        erased: false,
    });

    let mut p = make_percept("per_alice", "room:1", &["per_alice", "soul"], 100_000);
    p.named = vec![ActorId::new("soul")];
    p.embeddings = vec![make_emb_256(0.0, 1.0)];
    p.f_modal = Some(FModal([
        2.0, 1.0, 0.8, 0.0, 0.0, 0.0, 100.0, 0.9, 0.1, 0.30,
    ]));

    let x = extract_features_from_state(&state, &p, now, 0);
    assert_eq!(x[0], 1.0, "is_named_soul");
    assert_eq!(
        x[13], 0.0,
        "private memory must never leak into max_visible_mem_cos"
    );
    assert!((x[17] - 0.30).abs() < 1e-6, "asr_conf_or_one extracted");

    // Frozen dialogue act head & cosine similarity handle vectorless inputs cleanly.
    let head = FrozenDialogueActHead {
        w_gap: vec![4.0, -4.0, 0.0],
        b_gap: 0.0,
        expected_tag: None,
    };
    assert!(head.score(&[]).is_none());
    assert!(cosine_similarity(&[1.0, 0.0], &[0.0, 1.0]) == 0.0);
    assert!((sigmoid(0.0) - 0.5).abs() < 1e-12);

    // Vectorless percept leaves x_num finite and well-formed.
    let mut p_novec = make_percept("per_alice", "room:1", &["per_alice", "soul"], 100_000);
    p_novec.named = vec![ActorId::new("soul")];
    let x_novec = extract_features_from_state(&state, &p_novec, now, 0);
    assert!(x_novec.iter().all(|v| v.is_finite() && *v >= 0.0 && *v <= 1.0));
    assert_eq!(x_novec[0], 1.0);
    assert_eq!(x_novec[13], 0.0);
    let _ = (cfg, VisibleScope::new(p.audience, Some(p.place)));
}

#[test]
fn v2_neural_gate_determinism_sampling_cic_abstain_neutral_and_commitment_bypass() {
    let e_256 = vec![0.0_f32; 256];
    let x_num = [0.0_f32; 18];
    let h_prev = [0.0_f32; 64];

    // 1. Neutral fallback (`model = None`) stays Silent with NeutralFallback unless commitment fires.
    let cfg_det = CoreConfig {
        stochastic_gate_sampling: false,
        ..CoreConfig::default()
    };
    let neutral_out = decide_gate(
        EventId::new(1),
        &cfg_det,
        None,
        &e_256,
        &x_num,
        &h_prev,
        None,
        CommitmentInputs::default(),
    );
    assert_eq!(neutral_out.outcome, DecisionOutcome::Silent);
    assert_eq!(neutral_out.reason, DecisionReason::NeutralFallback);

    // Commitment bypass fires Go even under model = None.
    let comm_out = decide_gate(
        EventId::new(2),
        &cfg_det,
        None,
        &e_256,
        &x_num,
        &h_prev,
        None,
        CommitmentInputs {
            turn_allocated: true,
            ..Default::default()
        },
    );
    assert_eq!(comm_out.outcome, DecisionOutcome::Go);
    assert_eq!(comm_out.reason, DecisionReason::Commitment);
    assert_eq!(comm_out.probability, 1.0);

    // 2. CIC abstention when cic_threshold exceeds max gate probability.
    let model_cic = build_synthetic_model([0.0, 0.0, 0.0, 0.0], 0.90, 2);
    let cic_out = decide_gate(
        EventId::new(3),
        &cfg_det,
        Some(&model_cic),
        &e_256,
        &x_num,
        &h_prev,
        None,
        CommitmentInputs::default(),
    );
    assert_eq!(cic_out.outcome, DecisionOutcome::Silent);
    assert_eq!(cic_out.reason, DecisionReason::CicAbstention);

    // 3. Determinism & empirical convergence under stochastic_gate_sampling = true.
    // logits = [0, 0, 0, ln(3/7)] => P(Go) = 3 / (3 * 1 + 3/7)? Let's set [ln(0.25), ln(0.25), ln(0.20), ln(0.30)]!
    let target_logits = [
        0.25_f32.ln(),
        0.25_f32.ln(),
        0.20_f32.ln(),
        0.30_f32.ln(),
    ];
    let model_stoch = build_synthetic_model(target_logits, 0.10, 2);
    let cfg_stoch = CoreConfig {
        seed: 999,
        stochastic_gate_sampling: true,
        ..CoreConfig::default()
    };
    let g1 = decide_gate(
        EventId::new(77),
        &cfg_stoch,
        Some(&model_stoch),
        &e_256,
        &x_num,
        &h_prev,
        None,
        CommitmentInputs::default(),
    );
    let g2 = decide_gate(
        EventId::new(77),
        &cfg_stoch,
        Some(&model_stoch),
        &e_256,
        &x_num,
        &h_prev,
        None,
        CommitmentInputs::default(),
    );
    assert_eq!(g1.probability, g2.probability);
    assert_eq!(g1.draw, g2.draw);
    assert_eq!(g1.outcome, g2.outcome);

    let n_events = 2_000u64;
    let mut goes = 0usize;
    for id in 1..=n_events {
        let out = decide_gate(
            EventId::new(id),
            &cfg_stoch,
            Some(&model_stoch),
            &e_256,
            &x_num,
            &h_prev,
            None,
            CommitmentInputs::default(),
        );
        if out.outcome == DecisionOutcome::Go {
            goes += 1;
        }
    }
    let go_share = (goes as f64) / (n_events as f64);
    assert!(
        (go_share - 0.30).abs() < 0.04,
        "expected stochastic Go share near 0.30, got {go_share}"
    );

    // 4. Online adaptation toggle isolation.
    let mut adapt_state = PlaceAdaptState::default();
    let adapt_cfg = AdaptConfig {
        online_adaptation: true,
        max_adapt_delta: 0.5,
        ..Default::default()
    };
    adapt_state.record_feedback(
        AdaptFeedback {
            arm: GateArm::Go,
            reward: 1.0,
            is_delayed_one_sided: false,
            context_hint: [1.0, 0.5],
        },
        &adapt_cfg,
    );
    let cfg_adapt_off = CoreConfig {
        online_adaptation: false,
        ..cfg_det
    };
    let out_off = decide_gate(
        EventId::new(9),
        &cfg_adapt_off,
        Some(&model_stoch),
        &e_256,
        &x_num,
        &h_prev,
        Some(&adapt_state),
        CommitmentInputs::default(),
    );
    assert!(!out_off.trace.online_adaptation_applied);
}

#[test]
fn v2_backlog_chunking_caps_at_four_never_triggers_on_sleep_and_honors_commitments() {
    let cfg = CoreConfig::default();
    let now = Time::from_millis(100_000);

    assert!(!is_backlog_event(
        Time::from_millis(99_000),
        now,
        cfg.backlog_gap_ms,
        false,
    ));
    assert!(is_backlog_event(
        Time::from_millis(10_000),
        now,
        cfg.backlog_gap_ms,
        false,
    ));
    assert!(is_backlog_event(
        Time::from_millis(99_000),
        now,
        cfg.backlog_gap_ms,
        true,
    ));

    let mut events = Vec::new();
    for i in 1..=30u64 {
        events.push(BacklogEventInput {
            event_id: EventId::new(i),
            place: PlaceId::new("place-1"),
            timestamp_ms: Time::from_millis(50_000 + i * 100),
            commitments: CommitmentInputs::default(),
        });
    }
    for p in 2..=6u64 {
        let allocated = p == 6;
        events.push(BacklogEventInput {
            event_id: EventId::new(100 + p),
            place: PlaceId::new(format!("place-{p}")),
            timestamp_ms: Time::from_millis(50_000),
            commitments: CommitmentInputs {
                turn_allocated: allocated,
                ..CommitmentInputs::default()
            },
        });
    }

    let scheduled = schedule_backlog_chunks(&events, &cfg);
    assert_eq!(scheduled.handle_now.len(), MAX_CHUNKS_PER_WAKE);
    assert_eq!(scheduled.deferred.len(), 2);
    assert_eq!(scheduled.handle_now[0].place, PlaceId::new("place-6"));
    assert!(scheduled.handle_now[0].commitments.any());

    let p1_chunk = scheduled
        .handle_now
        .iter()
        .chain(scheduled.deferred.iter())
        .find(|c| c.place == PlaceId::new("place-1"))
        .expect("place-1 chunk present");
    assert_eq!(p1_chunk.event_ids.len(), 30);
    let _ = PlaceHistoryEntry {
        event_id: EventId::new(1),
        actor: ActorId::new("a"),
        audience: Audience::new([ActorId::new("a")]),
        timestamp_ms: now,
        deg_by_actor: Default::default(),
    };
}

#[test]
fn v2_trace_pinned_decision_replay_with_hard_forget_and_tamper_detection() {
    let model = build_synthetic_model([-5.0, -5.0, -5.0, 5.0], 0.1, 2);
    let cfg = CoreConfig {
        seed: 4242,
        stochastic_gate_sampling: false,
        ..CoreConfig::default()
    };
    let mut core = Core::open(MemStore::new(), cfg.clone(), Some(Arc::clone(&model))).expect("open");
    let mut doors = NoopDoors;

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

    // Simulate HardForget erasure and verify post-forget Trace-Pinned replay + tamper detection.
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
    core.store_mut().append(forget_rec).unwrap();
    let mut core = Core::open(core.store().clone(), cfg, Some(Arc::clone(&model)))
        .expect("reopen after hard_forget");
    assert_eq!(core.state().epoch, 1);
    assert!(
        !core
            .state()
            .actor_exchanges
            .contains_key(&ActorId::new("per_alice"))
    );

    let ckpt = core.take_snapshot("post-forget-epoch-1").expect("snapshot");

    let mut p3 = make_percept("per_carol", "chat:room", &["per_carol", "soul"], 4_000);
    p3.addressees = vec![ActorId::new("soul")];
    core.step(
        Input::Percept(Box::new(p3)),
        Time::from_millis(4_000),
        &mut doors,
    )
    .unwrap();

    let report = core.replay_verify(ReplayMode::Pinned).expect("replay ok");
    assert!(report.is_exact(), "mismatches: {:?}", report.mismatches);

    // Tamper a Decision event's trace schema version -> Trace-Pinned replay detects it!
    let mut tampered_store = MemStore::new();
    let mut modified_once = false;
    for mut rec in core.store().read_in_order(None, 500).unwrap() {
        if rec.seq > ckpt.cursor
            && !modified_once
            && let Some(EventDetail::Decision {
                trace: Some(ref mut tr),
                ..
            }) = rec.detail
        {
            tr.feature_schema_version = 999;
            modified_once = true;
        }
        tampered_store.append(rec).unwrap();
    }
    assert!(modified_once);
    let div_report = replay_from_checkpoint(
        &tampered_store,
        Some(&ckpt),
        ReplayMode::Pinned,
        Some(model.blake3),
    )
    .expect("replay reports tamper");
    assert!(!div_report.is_exact());
    assert_eq!(div_report.mismatches[0].field, "feature_schema_version");
}
