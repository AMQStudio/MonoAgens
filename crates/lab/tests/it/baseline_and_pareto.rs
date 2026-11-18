//! Adapter conformance, baseline parity, Tool RAG scaling, `1/n` band,
//! Holm-Bonferroni, and Convex Pareto scorecard tests (Core 11.3, 11.4).

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use monoagens_core::{Prior, Switches};
use monoagens_lab::{
    BaselineKind, PairedComparisonInput, ParetoScorecardInput, PerceptView, Scenario, ScenarioSet,
    ScriptedEvent, ScriptedEventKind, SystemConfigScore, TokenUsage5Tuple, compute_convex_pareto,
    evaluate_holm_bonferroni, format_sim_time_rfc3339, run_arm_t_restart_parity_check,
    run_baseline_adapter, run_benchmark_suite, run_tool_rag_scaling_eval,
};
use monoagens_protocol::{ActorId, Audience, OpaquePayload, Percept, PlaceId, Time};
use tempfile::TempDir;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates dir")
        .parent()
        .expect("workspace root")
        .to_path_buf()
}

fn synthetic_multi_turn_scenario_with_gap() -> Scenario {
    // Turn 1 at virtual t=1_000 ms (room chatter -> MonoAgens Gate suppresses LLM),
    // Turn 2 at t=1_000 + 30 days (addressed recall prompt across 30-day virtual clock jump).
    let thirty_days_ms = 30_u64 * 24 * 60 * 60 * 1000;
    let alice = ActorId::new("alice");
    let bob = ActorId::new("bob");
    let soul = ActorId::new("soul");
    let place = PlaceId::new("room.general");

    let p1 = Percept::minimal(
        alice.clone(),
        place.clone(),
        Audience::new([alice.clone(), bob]),
        Time::from_millis(1_000),
        OpaquePayload::from_bytes(b"background chatter in room".to_vec()),
    );

    let mut p2 = Percept::minimal(
        alice.clone(),
        place,
        Audience::new([alice, soul.clone()]),
        Time::from_millis(1_000 + thirty_days_ms),
        OpaquePayload::from_bytes(b"soul please remember our launch deadline".to_vec()),
    );
    p2.addressees = vec![soul.clone()];
    p2.named = vec![soul];

    Scenario {
        id: "adapter_conformance_30d".to_owned(),
        name: "Baseline adapter parity across virtual 30-day jump + noop gating".to_owned(),
        set: ScenarioSet::Development,
        start_time_ms: 1_000,
        model_latency_ms: 5,
        model_id: "test-model".to_owned(),
        embedder_id: "test-embed".to_owned(),
        bodies: Vec::new(),
        events: vec![
            ScriptedEvent {
                at_ms: 1_000,
                body_id: None,
                event: ScriptedEventKind::Percept(Box::new(p1)),
            },
            ScriptedEvent {
                at_ms: 1_000 + thirty_days_ms,
                body_id: None,
                event: ScriptedEventKind::Percept(Box::new(p2)),
            },
        ],
        scripted_model: Vec::new(),
        checks: Vec::new(),
        switches: Some(Switches::all_enabled()),
        cost_guard: None,
        prior: None,
        annotations: Default::default(),
    }
}

#[test]
fn adapter_conformance_local_verified_and_offline_unverified_honest() {
    let root = workspace_root();
    let scenario = synthetic_multi_turn_scenario_with_gap();

    // 1. PerceptView serializes ONLY observable perception fields (no oracle labels).
    let pv = PerceptView {
        event_id: "evt-1".to_owned(),
        place_id: "room.general".to_owned(),
        actor_id: "alice".to_owned(),
        audience_ids: vec!["alice".to_owned()],
        capability_list: vec!["a.speak".to_owned()],
        addressed_to_agent: false,
        content_payload_utf8: "hello".to_owned(),
        sim_time_rfc3339: format_sim_time_rfc3339(Time(1_000)),
    };
    let pv_json = serde_json::to_string(&pv).expect("serialize PerceptView");
    for forbidden in [
        "warranted",
        "sel_other",
        "answered",
        "false_belief_target",
        "episode_boundary",
        "sealed_tag",
        "lure",
        "annotations",
    ] {
        assert!(
            !pv_json.contains(forbidden),
            "PerceptView unexpectedly contains oracle label '{forbidden}': {pv_json}"
        );
    }

    // 2. Both self-contained Python adapters (stateless_react, arm_f) verify cleanly offline,
    // handle virtual 30-day jump, evict RAM on restart_epoch, and incur nonzero tokens on wake-to-noop
    // while MonoAgens Gate suppresses background chatter with 0 LLM tokens.
    for kind in [BaselineKind::StatelessReact, BaselineKind::ArmF] {
        let script = kind.adapter_script_path(&root);
        let rep = run_baseline_adapter(
            kind,
            &script,
            &scenario,
            &Prior::default(),
            42,
            true,
            Some(1),
            Duration::from_secs(5),
            &[],
        )
        .expect("local adapter run should succeed");

        assert!(
            rep.sdk_verified,
            "{} should be verified offline",
            kind.slug()
        );
        assert_eq!(rep.steps.len(), 2);
        assert_eq!(
            rep.steps[0].sim_time_rfc3339, rep.steps[0].recorded_sim_time_rfc3339,
            "virtual time must match without host wall clock"
        );
        assert!(
            rep.steps[1].sim_time_rfc3339.starts_with("1970-01-31T"),
            "30-day virtual jump must reach 1970-01-31 UTC, got {}",
            rep.steps[1].sim_time_rfc3339
        );
        assert!(
            rep.steps[1].ram_cleared_on_restart,
            "{} must clear RAM on restart_epoch",
            kind.slug()
        );
        assert_eq!(rep.steps[0].decision, "noop");
        assert!(rep.steps[0].noop_tool_exposed);
        assert!(rep.steps[0].wake_invoked_llm);
        assert!(
            rep.steps[0].e_tok > 0.0,
            "{} baseline wake-to-noop must bill nonzero E_tok, got {}",
            kind.slug(),
            rep.steps[0].e_tok
        );
        assert!(rep.monoagens_zero_llm_gate_suppressions >= 1);
        assert_eq!(rep.monoagens_gate_llm_tokens, 0);
    }

    // 3. External SDK adapters absent offline MUST report sdk_verified=false (UNVERIFIED) loudly
    // and never fabricate verified scores.
    for kind in [
        BaselineKind::Mem0,
        BaselineKind::Letta,
        BaselineKind::LanggraphLangmem,
        BaselineKind::ZepGraphiti,
        BaselineKind::HermesAgent,
    ] {
        let script = kind.adapter_script_path(&root);
        let rep = run_baseline_adapter(
            kind,
            &script,
            &scenario,
            &Prior::default(),
            42,
            true,
            None,
            Duration::from_secs(5),
            &[],
        )
        .expect("absent SDK adapter process should still cleanly emit UNVERIFIED handshake");

        assert!(
            !rep.sdk_verified,
            "{} has no offline Python package and must report UNVERIFIED",
            kind.slug()
        );
        assert!(
            rep.unavailable_reason.is_some(),
            "{} must explain why it is UNVERIFIED",
            kind.slug()
        );
    }
}

#[test]
fn adapter_sealed_mode_barrier_aborts_on_leak() {
    let root = workspace_root();
    let mut scenario = synthetic_multi_turn_scenario_with_gap();
    if let ScriptedEventKind::Percept(ref mut p) = scenario.events[0].event {
        p.payload = OpaquePayload::from_bytes(b"TOP_SECRET_SEALED_SENTINEL".to_vec());
    }
    let leak_dir = TempDir::new().expect("temp leak dir");
    let leak_str = leak_dir.path().to_str().expect("utf8 temp path");
    let script = BaselineKind::StatelessReact.adapter_script_path(&root);

    let err = run_baseline_adapter(
        BaselineKind::StatelessReact,
        &script,
        &scenario,
        &Prior::default(),
        7,
        true,
        None,
        Duration::from_secs(5),
        &[
            ("MONOAGENS_TEST_FORCE_SEALED_LEAK", "1"),
            ("MONOAGENS_LEAK_DIR", leak_str),
        ],
    )
    .expect_err("SEALED_MODE=1 barrier must fail loudly on leaked payload/prompt");

    let msg = err.to_string();
    assert!(
        msg.contains("SEALED_MODE"),
        "expected SEALED_MODE error detail, got: {msg}"
    );
}

#[test]
fn pareto_holm_and_benchmark_suite_semantics() {
    // 1. Confirmatory power check S >= 354, Holm-Bonferroni FWER, d_z, r_rb, and Convex Pareto hull.
    let s = 360_usize;
    let mono_q: Vec<f64> = (0..s).map(|i| 0.86 + ((i % 7) as f64) * 0.005).collect();
    let base_q: Vec<f64> = (0..s).map(|i| 0.85 + ((i % 7) as f64) * 0.005).collect();
    let mono_cost: Vec<f64> = (0..s).map(|i| 320.0 + (i % 10) as f64 * 4.0).collect();
    let base_cost: Vec<f64> = (0..s).map(|i| 980.0 + (i % 10) as f64 * 8.0).collect();

    let raw_mono = vec![TokenUsage5Tuple {
        uncached_in: 180,
        cache_read: 640,
        cache_write: 60,
        out_vis: 20,
        reason: 0,
    }];
    let raw_react = vec![TokenUsage5Tuple {
        uncached_in: 720,
        cache_read: 160,
        cache_write: 40,
        out_vis: 45,
        reason: 30,
    }];
    let raw_full = vec![TokenUsage5Tuple {
        uncached_in: 1200,
        cache_read: 100,
        cache_write: 80,
        out_vis: 70,
        reason: 60,
    }];

    let input = ParetoScorecardInput {
        systems: vec![
            SystemConfigScore {
                name: "MonoAgens".to_owned(),
                quality: 0.875,
                e_tok: raw_mono[0].e_tok(),
                p95_latency_ms: 42.0,
                fir: 0.01,
                delta_ni: 0.0,
                asr: 0.0,
                raw_5tuples: raw_mono,
                lane: None,
                contamination_risk: None,
                status: None,
            },
            SystemConfigScore {
                name: "StatelessReAct".to_owned(),
                quality: 0.610,
                e_tok: raw_react[0].e_tok(),
                p95_latency_ms: 190.0,
                fir: 0.28,
                delta_ni: 0.0,
                asr: 0.15,
                raw_5tuples: raw_react,
                lane: None,
                contamination_risk: None,
                status: None,
            },
            SystemConfigScore {
                name: "FullContextNaive".to_owned(),
                quality: 0.865,
                e_tok: raw_full[0].e_tok(),
                p95_latency_ms: 480.0,
                fir: 0.22,
                delta_ni: 0.14,
                asr: 0.18,
                raw_5tuples: raw_full,
                lane: None,
                contamination_risk: None,
                status: None,
            },
        ],
        paired_comparisons: vec![
            PairedComparisonInput {
                metric: "quality".to_owned(),
                baseline_name: "FullContextNaive".to_owned(),
                monoagens_scores: mono_q,
                baseline_scores: base_q,
                higher_is_better: true,
                is_quality_ni_endpoint: true,
            },
            PairedComparisonInput {
                metric: "e_tok".to_owned(),
                baseline_name: "FullContextNaive".to_owned(),
                monoagens_scores: mono_cost,
                baseline_scores: base_cost,
                higher_is_better: false,
                is_quality_ni_endpoint: false,
            },
        ],
        scorecard_rows: Vec::new(),
    };

    let report = compute_convex_pareto(&input);
    assert!(report.co_primary_ni_and_cost_passed);
    assert!(
        report
            .undominated_frontier
            .iter()
            .any(|s| s.name == "MonoAgens")
    );
    assert!(
        report
            .dominated_points
            .contains(&"StatelessReAct".to_owned())
            || report
                .dominated_points
                .contains(&"FullContextNaive".to_owned()),
        "at least one inferior baseline must be dominated: {:?}",
        report.dominated_points
    );
    assert_eq!(report.holm_results[0].s_scenarios, 360);
    assert!(report.holm_results[0].confirmatory_power_met);
    assert!(report.hypervolume_2d_quality_etok > 0.0);
    // Raw 5-tuples preserved (never collapsed to single token count)
    assert_eq!(report.undominated_frontier[0].raw_5tuples.len(), 1);

    // 2. Underpowered S < 354 is explicitly flagged as confirmatory_power_met == false
    let small = evaluate_holm_bonferroni(
        &[PairedComparisonInput {
            metric: "e_tok".to_owned(),
            baseline_name: "Tiny".to_owned(),
            monoagens_scores: vec![100.0; 20],
            baseline_scores: vec![200.0; 20],
            higher_is_better: false,
            is_quality_ni_endpoint: false,
        }],
        0.05,
        0.05,
    );
    assert!(!small[0].confirmatory_power_met);

    // 3. Tool RAG scaling (N_caps=50..200) and Arm-T restart parity check
    let prior = Prior::default();
    let scaling = run_tool_rag_scaling_eval(&prior);
    assert_eq!(scaling.len(), 4);
    assert!(scaling.last().unwrap().schema_token_savings_ratio > 0.80);
    assert!(run_arm_t_restart_parity_check(&prior).expect("Arm-T restart parity"));

    // 4. In-tree benchmark suite reports 1/n [0.5, 2.0] and excludes MuTual from multi-party claims
    let empty_root = TempDir::new().expect("temp datasets root");
    let empty_dir = empty_root.path().join("datasets");
    fs::create_dir_all(&empty_dir).unwrap();
    let template_manifest =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../lab/datasets.manifest.example.toml");
    fs::copy(
        &template_manifest,
        empty_root.path().join("datasets.manifest.toml"),
    )
    .expect("copy template manifest");
    let in_tree = run_benchmark_suite("in_tree_only", &empty_dir, Some(&input), &prior)
        .expect("in_tree suite");
    assert_eq!(in_tree.one_over_n_band_check, (0.5, 2.0));
    assert!(
        in_tree
            .excluded_from_multiparty_claims
            .iter()
            .any(|s| s.contains("MuTual"))
    );

    // 5. `--suite memory` strictly fails non-zero naming missing/unverified required datasets
    let std_err = run_benchmark_suite("memory", &empty_dir, None, &prior)
        .expect_err("category benchmark suite must fail loudly when external datasets are missing");
    let err_str = std_err.to_string();
    assert!(
        err_str.contains("locomo") && err_str.contains("longmemeval"),
        "missing dataset error must name missing required datasets, got: {err_str}"
    );
}

#[test]
fn cli_benchmark_category_exits_nonzero_when_datasets_missing() {
    let empty_root = TempDir::new().expect("temp datasets root");
    let empty_dir = empty_root.path().join("datasets");
    fs::create_dir_all(&empty_dir).unwrap();
    let template_manifest =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../lab/datasets.manifest.example.toml");
    let out = Command::new(env!("CARGO_BIN_EXE_monoagens-lab"))
        .args([
            "benchmark",
            "--suite",
            "memory",
            "--manifest",
            template_manifest.to_str().unwrap(),
            "--datasets-dir",
            empty_dir.to_str().unwrap(),
        ])
        .output()
        .expect("spawn monoagens-lab binary");

    assert!(
        !out.status.success(),
        "benchmark --suite memory must exit non-zero when datasets are missing"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("missing required benchmark dataset"),
        "expected missing dataset error on stderr, got: {stderr}"
    );
}
