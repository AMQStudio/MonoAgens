use std::collections::HashMap;
use std::sync::Arc;

use monoagens_core::gate::DecisionReason;
use monoagens_core::memory::{
    ContextMatchTag, ImportanceInputs, MemoryBasis, MemoryEntry, RecallQuery,
    RecollectionConstraint, compute_importance,
};
use monoagens_core::neural::NeuralModel;
use monoagens_core::{
    ActivationTrace, ContextPlan, Core, CoreConfig, DecisionOutcome, DoorDecision, Doors, Effect,
    EventDetail, EventKind, EventRecord, Input, MemStore, NoopDoors, ReplayMode, Store,
    replay_from_checkpoint,
};
use monoagens_protocol::{
    Action, ActorId, AttachmentHandle, Audience, CapabilityId, DeclassToken, EmbeddingTag, EventId,
    ModalitySlot, OpaquePayload, OriginActor, OwnerSigned, Percept, PlaceId, ReceiptOutcome,
    TaggedEmbedding, Taint, ThoughtAct, Time, VisibleScope,
};
use safetensors::tensor::{Dtype, TensorView};

fn tag_v2() -> EmbeddingTag {
    EmbeddingTag {
        embed_model_ver: "embeddinggemma-2".to_owned(),
        dim_d: 3,
        prompt_regime_id: "t_q".to_owned(),
    }
}

fn tag_v1() -> EmbeddingTag {
    EmbeddingTag {
        embed_model_ver: "embeddinggemma-300m".to_owned(),
        dim_d: 3,
        prompt_regime_id: "t_q".to_owned(),
    }
}

fn make_emb(tag: EmbeddingTag, vals: [f32; 3]) -> TaggedEmbedding {
    TaggedEmbedding {
        tag,
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

#[derive(Default)]
struct ActionPolicyDoors {
    refuse_capability: Option<String>,
    refused_count: usize,
    context_calls: usize,
}

impl Doors for ActionPolicyDoors {
    fn inbound(&mut self, _percept: &mut Percept) -> DoorDecision {
        DoorDecision::Allow
    }

    fn context(&mut self, _plan: &mut ContextPlan) -> DoorDecision {
        self.context_calls += 1;
        DoorDecision::Allow
    }

    fn action(&mut self, action: &mut Action) -> DoorDecision {
        if self
            .refuse_capability
            .as_deref()
            .is_some_and(|cap| cap == action.capability_id.as_str())
        {
            self.refused_count += 1;
            DoorDecision::Refuse {
                rule: "test_refuse_recall".to_owned(),
                reason: "policy denied".to_owned(),
            }
        } else {
            DoorDecision::Allow
        }
    }
}

fn build_wait_neural_model() -> Arc<NeuralModel> {
    let f32_bytes = |vals: &[f32]| -> Vec<u8> {
        let mut out = Vec::with_capacity(vals.len() * 4);
        for &v in vals {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out
    };
    let (d_in_emb, d_in_num, n_freq, d_e, d_n, d_model, d_ff, tabm_k, h_dim) =
        (256, 18, 2, 4, 4, 8, 16, 2, 64);
    let data: Vec<(&str, Vec<usize>, Vec<u8>)> = vec![
        ("stem.emb.linear.weight", vec![d_e, d_in_emb], f32_bytes(&vec![0.0; d_e * d_in_emb])),
        ("stem.emb.linear.bias", vec![d_e], f32_bytes(&vec![0.0; d_e])),
        ("stem.emb.norm.weight", vec![d_e], f32_bytes(&vec![1.0; d_e])),
        ("stem.emb.norm.bias", vec![d_e], f32_bytes(&vec![0.0; d_e])),
        ("stem.plr.freqs", vec![d_in_num, n_freq], f32_bytes(&vec![1.0; d_in_num * n_freq])),
        ("stem.plr.linear.weight", vec![d_n, d_in_num * 2 * n_freq], f32_bytes(&vec![0.0; d_n * d_in_num * 2 * n_freq])),
        ("stem.plr.linear.bias", vec![d_n], f32_bytes(&vec![0.0; d_n])),
        ("stem.fusion.weight", vec![d_model, d_e + d_n + h_dim], f32_bytes(&vec![0.0; d_model * (d_e + d_n + h_dim)])),
        ("stem.fusion.bias", vec![d_model], f32_bytes(&vec![0.0; d_model])),
        ("trunk.tabm_in_scale", vec![tabm_k, d_model], f32_bytes(&vec![1.0; tabm_k * d_model])),
        ("trunk.blocks.0.norm.weight", vec![d_model], f32_bytes(&vec![1.0; d_model])),
        ("trunk.blocks.0.norm.bias", vec![d_model], f32_bytes(&vec![0.0; d_model])),
        ("trunk.blocks.0.w_gate.weight", vec![d_ff, d_model], f32_bytes(&vec![0.0; d_ff * d_model])),
        ("trunk.blocks.0.w_up.weight", vec![d_ff, d_model], f32_bytes(&vec![0.0; d_ff * d_model])),
        ("trunk.blocks.0.w_down.weight", vec![d_model, d_ff], f32_bytes(&vec![0.0; d_model * d_ff])),
        ("trunk.blocks.0.tabm_r", vec![tabm_k, d_model], f32_bytes(&vec![1.0; tabm_k * d_model])),
        ("trunk.blocks.0.tabm_s", vec![tabm_k, d_model], f32_bytes(&vec![1.0; tabm_k * d_model])),
        ("trunk.final_norm.weight", vec![d_model], f32_bytes(&vec![1.0; d_model])),
        ("trunk.final_norm.bias", vec![d_model], f32_bytes(&vec![0.0; d_model])),
        ("memory.mingru.w_z.weight", vec![h_dim, d_model], f32_bytes(&vec![0.0; h_dim * d_model])),
        ("memory.mingru.w_z.bias", vec![h_dim], f32_bytes(&vec![0.0; h_dim])),
        ("memory.mingru.w_h.weight", vec![h_dim, d_model], f32_bytes(&vec![0.0; h_dim * d_model])),
        ("memory.mingru.w_h.bias", vec![h_dim], f32_bytes(&vec![0.0; h_dim])),
        ("head.gate_4.weight", vec![4, d_model], f32_bytes(&vec![0.0; 4 * d_model])),
        // Force deterministic Wait (`[Silent, Note, Wait, Go]`)
        ("head.gate_4.bias", vec![4], f32_bytes(&[-10.0, -10.0, 20.0, -10.0])),
        ("head.wait_delta_t.weight", vec![1, d_model], f32_bytes(&vec![0.0; d_model])),
        ("head.wait_delta_t.bias", vec![1], f32_bytes(&[0.0])),
    ];
    let views: Vec<(&str, TensorView<'_>)> = data
        .iter()
        .map(|(n, s, b)| (*n, TensorView::new(Dtype::F32, s.clone(), b).unwrap()))
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
    meta.insert("cic_threshold".into(), "0.5".into());
    meta.insert("max_wait_ms".into(), "4000.0".into());
    let bytes = safetensors::serialize(views, Some(meta)).unwrap();
    Arc::new(NeuralModel::from_safetensors_bytes(&bytes, None).unwrap())
}

#[test]
fn patience_yield_timeout_allocated_immediate_and_neural_wait() {
    let wait_model = build_wait_neural_model();
    let cfg = CoreConfig {
        stochastic_gate_sampling: false,
        ..CoreConfig::default()
    };
    let mut doors = NoopDoors;

    // 1. Allocated turn never waits even under a Wait-biased neural model.
    let mut core_alloc = Core::open(MemStore::new(), cfg.clone(), Some(Arc::clone(&wait_model))).unwrap();
    let mut p_alloc = make_percept("per_alice", "room:1", &["per_alice", "soul"], 1_000);
    p_alloc.addressees = vec![ActorId::new("soul")];
    let eff_alloc = core_alloc
        .step(
            Input::Percept(Box::new(p_alloc)),
            Time::from_millis(1_000),
            &mut doors,
        )
        .unwrap();
    assert!(
        matches!(eff_alloc.as_slice(), [Effect::Model { .. }]),
        "allocated turn must wake immediately: {eff_alloc:?}"
    );
    assert_eq!(core_alloc.next_due(), None);

    // 2. Unallocated Wait opens a wait; third-party answers first -> no model call, Yielded.
    let mut core_yield = Core::open(MemStore::new(), cfg.clone(), Some(Arc::clone(&wait_model))).unwrap();
    let mut p_open = make_percept(
        "per_alice",
        "room:2",
        &["per_alice", "per_bob", "soul"],
        1_000,
    );
    p_open.named = vec![ActorId::new("soul")];
    p_open.dialogue_act_gap = Some(1.0);
    let eff1 = core_yield
        .step(
            Input::Percept(Box::new(p_open)),
            Time::from_millis(1_000),
            &mut doors,
        )
        .unwrap();
    assert!(
        eff1.is_empty(),
        "unallocated Go must open wait instead of immediate model call"
    );
    let due = core_yield.next_due().expect("wait expectation scheduled");
    assert!(due > Time::from_millis(1_000));
    let p_open_id = core_yield
        .store()
        .read_in_order(None, 200)
        .unwrap()
        .into_iter()
        .find(|r| r.kind == EventKind::Percept)
        .expect("p_open percept")
        .id;

    // Bob answers 400ms later (inside the patience window); switch to neutral fallback for Bob's reply.
    core_yield.set_neural_model(None);
    let mut p_reply = make_percept(
        "per_bob",
        "room:2",
        &["per_alice", "per_bob", "soul"],
        1_400,
    );
    p_reply.reply_to = Some(p_open_id);
    let eff2 = core_yield
        .step(
            Input::Percept(Box::new(p_reply)),
            Time::from_millis(1_400),
            &mut doors,
        )
        .unwrap();
    assert!(
        eff2.is_empty(),
        "third-party reply must yield with zero model call"
    );
    assert!(
        core_yield.state().open_waits.is_empty(),
        "wait should be cancelled"
    );

    let all_records = core_yield.store().read_in_order(None, 200).unwrap();
    assert!(
        all_records.iter().any(|r| matches!(
            &r.detail,
            Some(EventDetail::Decision {
                target_event_id,
                outcome: DecisionOutcome::Silent,
                reason: Some(DecisionReason::Yielded),
                ..
            }) if *target_event_id == p_open_id
        )),
        "expected Silent/Yielded decision recorded for yielded event"
    );
    assert!(
        all_records.iter().any(|r| matches!(
            &r.detail,
            Some(EventDetail::Withheld {
                target_event_id,
                reason,
                ..
            }) if *target_event_id == p_open_id && reason == "yielded"
        )),
        "expected Withheld marker for yielded silence"
    );

    // 3. Nobody answers -> Tick at due_ms wakes reasoning with Effect::Model.
    let mut core_timeout = Core::open(MemStore::new(), cfg, Some(wait_model)).unwrap();
    let mut p_wait = make_percept(
        "per_alice",
        "room:3",
        &["per_alice", "per_bob", "soul"],
        2_000,
    );
    p_wait.named = vec![ActorId::new("soul")];
    p_wait.dialogue_act_gap = Some(1.0);
    let eff_init = core_timeout
        .step(
            Input::Percept(Box::new(p_wait)),
            Time::from_millis(2_000),
            &mut doors,
        )
        .unwrap();
    assert!(eff_init.is_empty());
    let wait_due = core_timeout.next_due().expect("wait scheduled");

    // Tick before due -> no event, no effect.
    let eff_early = core_timeout
        .step(
            Input::Tick,
            Time::from_millis(wait_due.as_millis() - 1),
            &mut doors,
        )
        .unwrap();
    assert!(eff_early.is_empty());

    // Tick at due -> wakes reasoning!
    let eff_due = core_timeout
        .step(Input::Tick, wait_due, &mut doors)
        .unwrap();
    assert!(
        matches!(eff_due.as_slice(), [Effect::Model { .. }]),
        "expected Effect::Model after patience window expired: {eff_due:?}"
    );
    assert_eq!(core_timeout.next_due(), None);
}

#[test]
fn decision_trace_pinned_withheld_markers_and_replay() {
    let cfg = CoreConfig {
        seed: 777,
        ..CoreConfig::default()
    };
    let mut core = Core::open(MemStore::new(), cfg, None).unwrap();
    let mut doors = NoopDoors;

    // Event addressed to Bob (not soul) under neutral fallback stays Silent with trace & Withheld audit marker.
    let mut p = make_percept(
        "per_alice",
        "room:1",
        &["per_alice", "per_bob", "soul"],
        1_000,
    );
    p.addressees = vec![ActorId::new("per_bob")];
    core.step(
        Input::Percept(Box::new(p)),
        Time::from_millis(1_000),
        &mut doors,
    )
    .unwrap();

    let records = core.store().read_in_order(None, 200).unwrap();
    let dec = records
        .iter()
        .find(|r| r.kind == EventKind::Decision)
        .expect("decision present");
    let Some(EventDetail::Decision {
        outcome,
        reason,
        trace: Some(tr),
        ..
    }) = &dec.detail
    else {
        panic!("expected Decision detail with trace");
    };
    assert_eq!(tr.outcome, *outcome);
    assert_eq!(Some(tr.reason), *reason);
    assert_eq!(tr.reason, DecisionReason::NeutralFallback);

    assert!(
        records.iter().any(|r| r.kind == EventKind::Withheld),
        "expected auditable Withheld event on neutral/floor silence"
    );

    let report = replay_from_checkpoint(core.store(), None, ReplayMode::Pinned, None).unwrap();
    assert!(report.is_exact(), "{:?}", report.mismatches);
}

#[test]
fn pre_step5_visiblescope_non_interference_delta_ni_zero_and_declass_token() {
    let cfg = CoreConfig {
        seed: 2026,
        ..CoreConfig::default()
    };
    let tag = tag_v2();

    // Run A: empty private history.
    let mut core_empty = Core::open(MemStore::new(), cfg.clone(), None).unwrap();
    // Run B: identical public history PLUS a highly relevant private secret in [per_owner] only.
    let mut core_priv = Core::open(MemStore::new(), cfg, None).unwrap();
    let mut doors = NoopDoors;

    let mut priv_trace = ActivationTrace::default();
    priv_trace.record(Time::from_millis(500));
    let secret_id = EventId::new(900);
    let secret_emb = make_emb(tag.clone(), [1.0, 0.0, 0.0]);
    core_priv
        .store_mut()
        .insert_vector(secret_id, secret_emb.clone());
    core_priv.register_memory_entry(MemoryEntry {
        event_id: secret_id,
        source_ids: vec![secret_id],
        is_gist: false,
        timestamp_ms: Time::from_millis(500),
        place: PlaceId::new("dm:owner"),
        actor: ActorId::new("per_owner"),
        origin_actor: OriginActor::new(ActorId::new("per_owner")),
        basis: MemoryBasis::Observed,
        taint: Taint::Trusted,
        audience: Audience::new([ActorId::new("per_owner"), ActorId::new("soul")]),
        attachments: Vec::new(),
        activation: priv_trace,
        importance: ImportanceInputs {
            i_reason: 1.0,
            linked_to_commitment: true,
            i_surprise: 1.0,
        },
        embeddings: vec![secret_emb],
        concepts: vec!["secret_topic".to_owned()],
        superseded_by: None,
        erased: false,
    });

    let mut pub_percept = make_percept("per_bob", "room:public", &["per_bob", "soul"], 1_000);
    pub_percept.embeddings = vec![make_emb(tag.clone(), [1.0, 0.0, 0.0])];
    pub_percept.named = vec![ActorId::new("soul")];

    core_empty
        .step(
            Input::Percept(Box::new(pub_percept.clone())),
            Time::from_millis(1_000),
            &mut doors,
        )
        .unwrap();
    core_priv
        .step(
            Input::Percept(Box::new(pub_percept)),
            Time::from_millis(1_000),
            &mut doors,
        )
        .unwrap();

    let dec_empty = core_empty
        .store()
        .read_in_order(None, 50)
        .unwrap()
        .into_iter()
        .find(|r| r.kind == EventKind::Decision)
        .unwrap();
    let dec_priv = core_priv
        .store()
        .read_in_order(None, 50)
        .unwrap()
        .into_iter()
        .find(|r| r.kind == EventKind::Decision)
        .unwrap();

    let (
        Some(EventDetail::Decision {
            outcome: o1,
            salience: s1,
            threshold: t1,
            probability: p1,
            draw: d1,
            trace: Some(tr1),
            ..
        }),
        Some(EventDetail::Decision {
            outcome: o2,
            salience: s2,
            threshold: t2,
            probability: p2,
            draw: d2,
            trace: Some(tr2),
            ..
        }),
    ) = (&dec_empty.detail, &dec_priv.detail)
    else {
        panic!("expected full Decision traces");
    };

    // Exact non-interference (Delta_NI == 0): bit-identical gate numbers and identical neural traces.
    assert_eq!(o1, o2);
    assert_eq!(s1.to_bits(), s2.to_bits());
    assert_eq!(t1.to_bits(), t2.to_bits());
    assert_eq!(p1.to_bits(), p2.to_bits());
    assert_eq!(d1.to_bits(), d2.to_bits());
    assert_eq!(tr1, tr2);

    // Direct recall in public scope returns zero private items and leaves secret use count untouched (count == 1).
    let public_scope = VisibleScope::new(
        Audience::new([ActorId::new("per_bob"), ActorId::new("soul")]),
        Some(PlaceId::new("room:public")),
    );
    let mut query = RecallQuery::new(public_scope.clone());
    query.query_embeddings = vec![make_emb(tag.clone(), [1.0, 0.0, 0.0])];
    query.record_uses = true;
    let res_before = core_priv.recall(&query, Time::from_millis(2_000)).unwrap();
    assert!(
        !res_before.items.iter().any(|i| i.event_id == secret_id),
        "private secret_id must never appear in public recall"
    );
    assert_eq!(
        core_priv
            .state()
            .memory_entries
            .get(&secret_id)
            .unwrap()
            .activation
            .count,
        1,
        "rejected private item must never gain activation"
    );

    // Add an unrelated private secret_id_2 that is NOT named in DeclassToken.
    let secret_id_2 = EventId::new(901);
    core_priv
        .store_mut()
        .insert_vector(secret_id_2, make_emb(tag.clone(), [1.0, 0.0, 0.0]));
    core_priv.register_memory_entry(MemoryEntry {
        event_id: secret_id_2,
        source_ids: vec![secret_id_2],
        is_gist: false,
        timestamp_ms: Time::from_millis(500),
        place: PlaceId::new("dm:owner"),
        actor: ActorId::new("per_owner"),
        origin_actor: OriginActor::new(ActorId::new("per_owner")),
        basis: MemoryBasis::Observed,
        taint: Taint::Trusted,
        audience: Audience::new([ActorId::new("per_owner"), ActorId::new("soul")]),
        attachments: Vec::new(),
        activation: ActivationTrace::default(),
        importance: ImportanceInputs::default(),
        embeddings: vec![make_emb(tag.clone(), [1.0, 0.0, 0.0])],
        concepts: Vec::new(),
        superseded_by: None,
        erased: false,
    });

    // Owner-signed DeclassToken widens scope ONLY for `secret_id` in `room:public`.
    let token = OwnerSigned {
        payload: DeclassToken {
            token_id: "tok-1".to_owned(),
            issuer: ActorId::new("per_owner"),
            scope: vec![secret_id],
            expiry: Time::from_millis(10_000),
            target_place: Some(PlaceId::new("room:public")),
            target_audience: Audience::new([ActorId::new("per_bob"), ActorId::new("soul")]),
            signature: "sig-inner".to_owned(),
        },
        signer: ActorId::new("per_owner"),
        signed_at_ms: Time::from_millis(1_500),
        signature: "sig-outer".to_owned(),
    };
    core_priv
        .log_declass_token(token, Time::from_millis(1_500))
        .unwrap();

    // Target place gets `secret_id` (plus visible public percept), NEVER `secret_id_2`.
    let scope_public_with_tok = core_priv.state().visible_scope_for(
        Audience::new([ActorId::new("per_bob"), ActorId::new("soul")]),
        Some(PlaceId::new("room:public")),
        Time::from_millis(2_000),
    );
    let mut q_tok = RecallQuery::new(scope_public_with_tok);
    q_tok.query_embeddings = vec![make_emb(tag.clone(), [1.0, 0.0, 0.0])];
    let res_public = core_priv.recall(&q_tok, Time::from_millis(2_000)).unwrap();
    let ids: Vec<EventId> = res_public.items.iter().map(|i| i.event_id).collect();
    assert!(
        ids.contains(&secret_id),
        "authorized secret_id must be visible"
    );
    assert!(
        !ids.contains(&secret_id_2),
        "unauthorized secret_id_2 must remain hidden"
    );

    // Another place (`room:other`) MUST NOT see `secret_id`!
    let scope_other_place = core_priv.state().visible_scope_for(
        Audience::new([ActorId::new("per_bob"), ActorId::new("soul")]),
        Some(PlaceId::new("room:other")),
        Time::from_millis(2_000),
    );
    let mut q_other = RecallQuery::new(scope_other_place);
    q_other.query_embeddings = vec![make_emb(tag, [1.0, 0.0, 0.0])];
    let res_other = core_priv
        .recall(&q_other, Time::from_millis(2_000))
        .unwrap();
    assert!(!res_other.items.iter().any(|i| i.event_id == secret_id));
}

#[test]
fn recall_activation_no_vector_long_gap_faded_links_superseded_importance_and_capability() {
    let tag = tag_v2();
    let cfg = CoreConfig {
        seed: 9,
        ..CoreConfig::default()
    };
    let mut core = Core::open(MemStore::new(), cfg.clone(), None).unwrap();

    let aud = Audience::new([ActorId::new("per_alice"), ActorId::new("soul")]);
    let scope = VisibleScope::new(aud.clone(), Some(PlaceId::new("room:1")));

    // 1. w_B = 0 neutral ranking: item A has 50 recent uses & lower cosine (0.6);
    // item B has 1 old use & higher cosine (0.95). With w_B = 0, B MUST beat A!
    let mut trace_heavy = ActivationTrace::default();
    for k in 1..=50u64 {
        trace_heavy.record(Time::from_millis(10_000 + k * 100));
    }
    let mut trace_old = ActivationTrace::default();
    trace_old.record(Time::from_millis(1_000));

    let id_a = EventId::new(101);
    let id_b = EventId::new(102);
    core.store_mut()
        .insert_vector(id_a, make_emb(tag.clone(), [0.6, 0.8, 0.0]));
    core.store_mut()
        .insert_vector(id_b, make_emb(tag.clone(), [0.95, 0.31225, 0.0]));
    core.register_memory_entry(MemoryEntry {
        event_id: id_a,
        source_ids: vec![id_a],
        is_gist: false,
        timestamp_ms: Time::from_millis(15_000),
        place: PlaceId::new("room:1"),
        actor: ActorId::new("per_alice"),
        origin_actor: OriginActor::new(ActorId::new("per_alice")),
        basis: MemoryBasis::Observed,
        taint: Taint::Trusted,
        audience: aud.clone(),
        attachments: vec![AttachmentHandle {
            handle_id: "att-blind-1".to_owned(),
            modality_slot: ModalitySlot::Image,
            byte_len: 512,
            expires_at_ms: Time::from_millis(9_999_999),
            embed_model_ver: "embeddinggemma-2".to_owned(),
        }],
        activation: trace_heavy,
        importance: ImportanceInputs::default(),
        embeddings: vec![make_emb(tag.clone(), [0.6, 0.8, 0.0])],
        concepts: Vec::new(),
        superseded_by: None,
        erased: false,
    });
    core.register_memory_entry(MemoryEntry {
        event_id: id_b,
        source_ids: vec![id_b],
        is_gist: false,
        timestamp_ms: Time::from_millis(1_000),
        place: PlaceId::new("room:1"),
        actor: ActorId::new("per_alice"),
        origin_actor: OriginActor::new(ActorId::new("per_alice")),
        basis: MemoryBasis::Told,
        taint: Taint::Provisional,
        audience: aud.clone(),
        attachments: Vec::new(),
        activation: trace_old,
        importance: ImportanceInputs::default(),
        embeddings: vec![make_emb(tag.clone(), [0.95, 0.31225, 0.0])],
        concepts: Vec::new(),
        superseded_by: None,
        erased: false,
    });

    // With Petrov activation enabled, heavily-used id_a beats id_b.
    let mut q_vec = RecallQuery::new(scope.clone());
    q_vec.query_embeddings = vec![make_emb(tag.clone(), [1.0, 0.0, 0.0])];
    let res_act = core.recall(&q_vec, Time::from_millis(20_000)).unwrap();
    assert_eq!(res_act.items[0].event_id, id_a);
    assert_eq!(res_act.items[0].attachments[0].handle_id, "att-blind-1");

    // 2. Importance ordering: promise-linked beats routine at equal age & similarity.
    let mut tr_eq = ActivationTrace::default();
    tr_eq.record(Time::from_millis(10_000));
    let id_routine = EventId::new(103);
    let id_promise = EventId::new(104);
    let emb_eq = make_emb(tag.clone(), [0.0, 1.0, 0.0]);
    core.store_mut().insert_vector(id_routine, emb_eq.clone());
    core.store_mut().insert_vector(id_promise, emb_eq.clone());
    core.register_memory_entry(MemoryEntry {
        event_id: id_routine,
        source_ids: vec![id_routine],
        is_gist: false,
        timestamp_ms: Time::from_millis(10_000),
        place: PlaceId::new("room:1"),
        actor: ActorId::new("per_alice"),
        origin_actor: OriginActor::new(ActorId::new("per_alice")),
        basis: MemoryBasis::Observed,
        taint: Taint::Trusted,
        audience: aud.clone(),
        attachments: Vec::new(),
        activation: tr_eq,
        importance: ImportanceInputs::default(),
        embeddings: vec![emb_eq.clone()],
        concepts: Vec::new(),
        superseded_by: None,
        erased: false,
    });
    core.register_memory_entry(MemoryEntry {
        event_id: id_promise,
        source_ids: vec![id_promise],
        is_gist: false,
        timestamp_ms: Time::from_millis(10_000),
        place: PlaceId::new("room:1"),
        actor: ActorId::new("per_alice"),
        origin_actor: OriginActor::new(ActorId::new("per_alice")),
        basis: MemoryBasis::Observed,
        taint: Taint::Trusted,
        audience: aud.clone(),
        attachments: Vec::new(),
        activation: tr_eq,
        importance: ImportanceInputs {
            i_reason: 0.0,
            linked_to_commitment: true,
            i_surprise: 0.0,
        },
        embeddings: vec![emb_eq],
        concepts: Vec::new(),
        superseded_by: None,
        erased: false,
    });
    assert_eq!(
        compute_importance(ImportanceInputs {
            i_reason: 0.2,
            linked_to_commitment: true,
            i_surprise: 0.4,
        }),
        1.0
    );
    let mut q_imp = RecallQuery::new(scope.clone());
    q_imp.query_embeddings = vec![make_emb(tag.clone(), [0.0, 1.0, 0.0])];
    let res_imp = core.recall(&q_imp, Time::from_millis(20_000)).unwrap();
    let pos_promise = res_imp
        .items
        .iter()
        .position(|i| i.event_id == id_promise)
        .unwrap();
    let pos_routine = res_imp
        .items
        .iter()
        .position(|i| i.event_id == id_routine)
        .unwrap();
    assert!(pos_promise < pos_routine);

    // 3. Use recording raises later base-level activation B.
    let b_before = core
        .state()
        .memory_entries
        .get(&id_b)
        .unwrap()
        .activation
        .base_level(Time::from_millis(25_000), 0.5);
    let mut q_use = RecallQuery::new(scope.clone());
    q_use.query_embeddings = vec![make_emb(tag.clone(), [0.95, 0.31225, 0.0])];
    q_use.record_uses = true;
    core.recall(&q_use, Time::from_millis(24_000)).unwrap();
    let b_after = core
        .state()
        .memory_entries
        .get(&id_b)
        .unwrap()
        .activation
        .base_level(Time::from_millis(25_000), 0.5);
    assert!(b_after > b_before, "use must raise base-level B");

    // 4. Superseded marking: new item ranks as current; old item marked superseded with pointer.
    core.supersede_memory(id_a, id_b);
    let res_sup = core.recall(&q_vec, Time::from_millis(25_000)).unwrap();
    assert_eq!(res_sup.items[0].event_id, id_b);
    assert!(res_sup.items[0].is_current);
    let old_item = res_sup.items.iter().find(|i| i.event_id == id_a).unwrap();
    assert!(!old_item.is_current);
    assert_eq!(old_item.superseded_by, Some(id_b));

    // 5. No-vector recall (text + activation + importance) + faded vectors still verifiable via source links
    //    + long-gap gist recall not blocked by recollection.
    let mut doors = NoopDoors;
    let fact_rec = EventRecord::new_root(
        EventKind::Percept,
        Time::from_millis(1_000),
        ActorId::new("per_alice"),
        PlaceId::new("room:text"),
        aud.clone(),
        OriginActor::new(ActorId::new("per_alice")),
        Taint::Trusted,
        Default::default(),
        OpaquePayload::from_bytes(b"zephyr_orchid_42".as_slice()),
        Vec::new(),
        None,
    );
    let fact_id = core.store_mut().append(fact_rec.clone()).unwrap();
    // Index vector, then release it to simulate Petrov fade (B < -3.29) -> record stays in log!
    core.store_mut()
        .insert_vector(fact_id, make_emb(tag.clone(), [0.0, 0.0, 1.0]));
    core.store_mut().release_vector(fact_id).unwrap();

    // Long-gap gist at 45 days referencing `fact_id` (faded vector!) vs unanchored gist (missing source).
    let t_45d = 45 * 24 * 3_600_000;
    let gist_anchored_id = EventId::new(201);
    let gist_unanchored_id = EventId::new(202);
    let mut tr_gist = ActivationTrace::default();
    tr_gist.record(Time::from_millis(1_000));
    core.store_mut()
        .insert_vector(gist_anchored_id, make_emb(tag.clone(), [0.0, 0.0, 1.0]));
    core.store_mut()
        .insert_vector(gist_unanchored_id, make_emb(tag.clone(), [0.0, 0.0, 1.0]));
    core.register_memory_entry(MemoryEntry {
        event_id: gist_anchored_id,
        source_ids: vec![fact_id],
        is_gist: true,
        timestamp_ms: Time::from_millis(1_000),
        place: PlaceId::new("room:text"),
        actor: ActorId::new("soul"),
        origin_actor: OriginActor::new(ActorId::new("per_alice")),
        basis: MemoryBasis::Inferred,
        taint: Taint::Trusted,
        audience: aud.clone(),
        attachments: Vec::new(),
        activation: tr_gist,
        importance: ImportanceInputs::default(),
        embeddings: vec![make_emb(tag.clone(), [0.0, 0.0, 1.0])],
        concepts: Vec::new(),
        superseded_by: None,
        erased: false,
    });
    core.register_memory_entry(MemoryEntry {
        event_id: gist_unanchored_id,
        source_ids: vec![EventId::new(999_999)], // missing/forgotten source
        is_gist: true,
        timestamp_ms: Time::from_millis(1_000),
        place: PlaceId::new("room:text"),
        actor: ActorId::new("soul"),
        origin_actor: OriginActor::new(ActorId::new("per_alice")),
        basis: MemoryBasis::Inferred,
        taint: Taint::Trusted,
        audience: aud.clone(),
        attachments: Vec::new(),
        activation: tr_gist,
        importance: ImportanceInputs::default(),
        embeddings: vec![make_emb(tag.clone(), [0.0, 0.0, 1.0])],
        concepts: Vec::new(),
        superseded_by: None,
        erased: false,
    });

    let mut q_long = RecallQuery::new(VisibleScope::new(
        aud.clone(),
        Some(PlaceId::new("room:text")),
    ));
    q_long.query_embeddings = vec![make_emb(tag, [0.0, 0.0, 1.0])];
    q_long.recollection_constraint = Some(RecollectionConstraint {
        people: vec![ActorId::new("per_alice")],
        place: Some(PlaceId::new("room:text")),
        time_range: None,
    });
    let res_long = core.recall(&q_long, Time::from_millis(t_45d)).unwrap();
    let anchored = res_long
        .items
        .iter()
        .find(|i| i.event_id == gist_anchored_id)
        .expect("45-day anchored gist returned");
    assert!(
        anchored.recollected,
        "faded source still verifies R through source_ids"
    );
    assert!(anchored.familiarity > 0.0);

    let unanchored = res_long
        .items
        .iter()
        .find(|i| i.event_id == gist_unanchored_id)
        .expect("45-day unanchored gist NOT blocked by R=0");
    assert!(!unanchored.recollected);
    assert_eq!(unanchored.context_match, ContextMatchTag::FamiliarOnly);
    assert!(unanchored.familiarity > 0.0);

    // Pure text-only recall (no query embeddings).
    let query_text_ev = EventRecord::new_root(
        EventKind::Percept,
        Time::from_millis(t_45d + 1_000),
        ActorId::new("per_alice"),
        PlaceId::new("room:text"),
        aud.clone(),
        OriginActor::new(ActorId::new("per_alice")),
        Taint::Trusted,
        Default::default(),
        OpaquePayload::from_bytes(b"recall zephyr_orchid_42".as_slice()),
        Vec::new(),
        None,
    );
    let query_text_id = core.store_mut().append(query_text_ev).unwrap();
    let mut q_novec = RecallQuery::new(VisibleScope::new(
        aud.clone(),
        Some(PlaceId::new("room:text")),
    ));
    q_novec.query_event_id = Some(query_text_id);
    let res_novec = core
        .recall(&q_novec, Time::from_millis(t_45d + 1_000))
        .unwrap();
    assert!(
        res_novec.items.iter().any(|i| i.event_id == fact_id),
        "no-vector text recall must return matching fact"
    );

    // 6. Explicit `memory.recall` capability path through Action door.
    let mut p_trigger = make_percept(
        "per_alice",
        "room:text",
        &["per_alice", "soul"],
        t_45d + 2_000,
    );
    p_trigger.addressees = vec![ActorId::new("soul")];
    let trigger_effects = core
        .step(
            Input::Percept(Box::new(p_trigger)),
            Time::from_millis(t_45d + 2_000),
            &mut doors,
        )
        .unwrap();
    assert_eq!(trigger_effects.len(), 1);

    // Ensure `fact_id` is not crowded out of `n_rec_max = 4` by raising its importance via commitment
    // or using a dedicated fresh query with `prior.n_rec_max` capacity.
    let mut core_cap = Core::open(
        core.store().clone(),
        CoreConfig {
            max_recalled_memories: 16,
            ..cfg
        },
        None,
    )
    .unwrap();
    let recall_effects = core_cap
        .step(
            Input::ModelResult {
                request_id: query_text_id,
                acts: vec![ThoughtAct::Act {
                    capability_id: CapabilityId::new("memory.recall"),
                    places: vec![PlaceId::new("room:text")],
                    people: vec![ActorId::new("per_alice")],
                    payload: OpaquePayload::from_bytes(b"zephyr_orchid_42".as_slice()),
                    motivation: 4,
                    confidence_0_20: 14,
                    used_ids: vec![query_text_id],
                    expected: None,
                    domain: None,
                }],
            },
            Time::from_millis(t_45d + 3_000),
            &mut doors,
        )
        .unwrap();
    assert!(
        matches!(
            recall_effects.as_slice(),
            [Effect::Model { plan, .. }] if plan.items.contains(&fact_id)
        ),
        "allowed memory.recall act must wake next turn with recalled item in ContextPlan: {recall_effects:?}"
    );

    // Refused by Action door -> Receipt(Refused), nothing recalled, no Model effect.
    let mut refusing_doors = ActionPolicyDoors {
        refuse_capability: Some("memory.recall".to_owned()),
        ..Default::default()
    };
    let refused_effects = core_cap
        .step(
            Input::ModelResult {
                request_id: query_text_id,
                acts: vec![ThoughtAct::Act {
                    capability_id: CapabilityId::new("memory.recall"),
                    places: vec![PlaceId::new("room:text")],
                    people: vec![ActorId::new("per_alice")],
                    payload: OpaquePayload::default(),
                    motivation: 4,
                    confidence_0_20: 14,
                    used_ids: vec![query_text_id],
                    expected: None,
                    domain: None,
                }],
            },
            Time::from_millis(t_45d + 4_000),
            &mut refusing_doors,
        )
        .unwrap();
    assert!(refused_effects.is_empty());
    assert_eq!(refusing_doors.refused_count, 1);
    let last_receipts: Vec<EventRecord> = core_cap
        .store()
        .read_in_order(None, 500)
        .unwrap()
        .into_iter()
        .filter(|r| r.kind == EventKind::Receipt)
        .collect();
    assert!(matches!(
        &last_receipts.last().unwrap().detail,
        Some(EventDetail::Receipt {
            outcome: ReceiptOutcome::Refused,
            ..
        })
    ));
}

#[test]
fn cross_embedder_query_partition_non_mixing_and_text_partitioning() {
    let cfg = CoreConfig {
        seed: 11,
        ..CoreConfig::default()
    };
    let mut core = Core::open(MemStore::new(), cfg, None).unwrap();
    let aud = Audience::new([ActorId::new("per_alice"), ActorId::new("soul")]);

    let old_v1_id = EventId::new(301);
    let new_v2_id = EventId::new(302);
    let incompatible_dim_id = EventId::new(303);

    let v1_emb = make_emb(tag_v1(), [1.0, 0.0, 0.0]);
    let v2_emb = make_emb(tag_v2(), [0.0, 1.0, 0.0]);
    let wrong_regime_emb = TaggedEmbedding {
        tag: EmbeddingTag {
            embed_model_ver: "embeddinggemma-2".to_owned(),
            dim_d: 3,
            prompt_regime_id: "t_doc_different".to_owned(),
        },
        values: vec![1.0, 0.0, 0.0],
    };

    core.store_mut().insert_vector(old_v1_id, v1_emb.clone());
    core.store_mut().insert_vector(new_v2_id, v2_emb.clone());
    core.store_mut()
        .insert_vector(incompatible_dim_id, wrong_regime_emb.clone());

    let mut tr = ActivationTrace::default();
    tr.record(Time::from_millis(5_000));
    core.register_memory_entry(MemoryEntry {
        event_id: old_v1_id,
        source_ids: vec![old_v1_id],
        is_gist: false,
        timestamp_ms: Time::from_millis(5_000),
        place: PlaceId::new("room:bridge"),
        actor: ActorId::new("per_alice"),
        origin_actor: OriginActor::new(ActorId::new("per_alice")),
        basis: MemoryBasis::Observed,
        taint: Taint::Trusted,
        audience: aud.clone(),
        attachments: Vec::new(),
        activation: tr,
        importance: ImportanceInputs::default(),
        embeddings: vec![v1_emb],
        concepts: vec!["project_apollo".to_owned()],
        superseded_by: None,
        erased: false,
    });
    core.register_memory_entry(MemoryEntry {
        event_id: new_v2_id,
        source_ids: vec![new_v2_id],
        is_gist: false,
        timestamp_ms: Time::from_millis(6_000),
        place: PlaceId::new("room:bridge"),
        actor: ActorId::new("per_alice"),
        origin_actor: OriginActor::new(ActorId::new("per_alice")),
        basis: MemoryBasis::Observed,
        taint: Taint::Trusted,
        audience: aud.clone(),
        attachments: Vec::new(),
        activation: tr,
        importance: ImportanceInputs::default(),
        embeddings: vec![v2_emb],
        concepts: vec!["project_apollo".to_owned()],
        superseded_by: None,
        erased: false,
    });
    let aud_other = Audience::new([ActorId::new("per_other"), ActorId::new("soul")]);
    core.register_memory_entry(MemoryEntry {
        event_id: incompatible_dim_id,
        source_ids: vec![incompatible_dim_id],
        is_gist: false,
        timestamp_ms: Time::from_millis(6_500),
        place: PlaceId::new("room:other"),
        actor: ActorId::new("per_other"),
        origin_actor: OriginActor::new(ActorId::new("per_other")),
        basis: MemoryBasis::Observed,
        taint: Taint::Trusted,
        audience: aud_other.clone(),
        attachments: Vec::new(),
        activation: tr,
        importance: ImportanceInputs::default(),
        embeddings: vec![wrong_regime_emb],
        concepts: Vec::new(),
        superseded_by: None,
        erased: false,
    });

    // Incompatible prompt_regime_id partition in `room:other` MUST NOT match a `tag_v2` query.
    let mut q_incompat = RecallQuery::new(VisibleScope::new(
        aud_other,
        Some(PlaceId::new("room:other")),
    ));
    q_incompat.partition_tag = Some(tag_v2());
    q_incompat.query_embeddings = vec![make_emb(tag_v2(), [1.0, 0.0, 0.0])];
    let res_incompat = core.recall(&q_incompat, Time::from_millis(10_000)).unwrap();
    assert!(
        !res_incompat
            .items
            .iter()
            .any(|i| i.event_id == incompatible_dim_id),
        "incompatible prompt_regime_id partition must not mix"
    );

    // Query with tag_v2([1, 0, 0]) in room:bridge:
    // old_v1_id has tag_v1([1, 0, 0]) -> cosine MUST NEVER mix across incompatible embedder versions!
    let mut q_part = RecallQuery::new(VisibleScope::new(aud, Some(PlaceId::new("room:bridge"))));
    q_part.partition_tag = Some(tag_v2());
    q_part.query_embeddings = vec![make_emb(tag_v2(), [1.0, 0.0, 0.0])];
    q_part.wake_place = Some(PlaceId::new("room:bridge"));
    q_part.seed_concepts = vec!["project_apollo".to_owned()];

    let res_part = core.recall(&q_part, Time::from_millis(10_000)).unwrap();
    for item in &res_part.items {
        if item.event_id == old_v1_id {
            assert_eq!(
                item.cosine, 0.0,
                "incompatible v1 vector must never mix cosine into v2 query"
            );
        }
    }

    // 3. Text candidate partitioning where the store reports incompatible embedding tags.
    let mut text_store = MemStore::new();
    let txt_aud = Audience::new([ActorId::new("per_alice"), ActorId::new("soul")]);
    let txt_v1 = EventRecord::new_root(
        EventKind::Percept,
        Time::from_millis(100),
        ActorId::new("per_alice"),
        PlaceId::new("room:txt"),
        txt_aud.clone(),
        OriginActor::new(ActorId::new("per_alice")),
        Taint::Trusted,
        Default::default(),
        OpaquePayload::from_bytes(b"apollo_beacon_signal".as_slice()),
        Vec::new(),
        None,
    );
    let txt_v1_id = text_store.append(txt_v1).unwrap();
    text_store.insert_vector(txt_v1_id, make_emb(tag_v1(), [1.0, 0.0, 0.0]));

    let txt_v2 = EventRecord::new_root(
        EventKind::Percept,
        Time::from_millis(200),
        ActorId::new("per_alice"),
        PlaceId::new("room:txt"),
        txt_aud.clone(),
        OriginActor::new(ActorId::new("per_alice")),
        Taint::Trusted,
        Default::default(),
        OpaquePayload::from_bytes(b"apollo_beacon_signal".as_slice()),
        Vec::new(),
        None,
    );
    let txt_v2_id = text_store.append(txt_v2).unwrap();
    text_store.insert_vector(txt_v2_id, make_emb(tag_v2(), [0.0, 1.0, 0.0]));

    let txt_query = EventRecord::new_root(
        EventKind::Percept,
        Time::from_millis(300),
        ActorId::new("per_alice"),
        PlaceId::new("room:txt"),
        txt_aud,
        OriginActor::new(ActorId::new("per_alice")),
        Taint::Trusted,
        Default::default(),
        OpaquePayload::from_bytes(b"apollo_beacon_signal".as_slice()),
        Vec::new(),
        None,
    );
    let txt_query_id = text_store.append(txt_query).unwrap();

    let partitioned_hits = text_store
        .candidates_by_text_partitioned(txt_query_id, Some(&tag_v2()), 20)
        .unwrap();
    assert_eq!(partitioned_hits, vec![txt_v2_id]);
}
