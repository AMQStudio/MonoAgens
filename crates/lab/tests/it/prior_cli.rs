use std::fs;
use std::path::PathBuf;
use std::process::Command;

use monoagens_core::{CoreConfig, CoreState, LogSeq, MemStore, SnapshotCheckpoint, Store, blake3};
use monoagens_lab::{
    EventAnnotation, RunRecord, Scenario, ScenarioAnnotations, ScenarioSet, ScriptedEvent,
    ScriptedEventKind, TestBodySpec, WarrantedReply, compute_neural_calibration_metrics,
    load_neural_model_from_path, load_scenarios_from_set, run_ablation_compare, run_scenario,
    run_scenario_with_model,
};
use monoagens_protocol::{
    ActorId, Audience, BodyId, CapabilityDescriptor, CapabilityFlags, CapabilityId,
    CapabilitySchemaHash, OpaquePayload, Percept, PlaceId, Time,
};
use monoagens_store::{compute_config_digest, default_loaded_core_config};
use serde_json::Value;
use tempfile::TempDir;

fn lab_bin() -> &'static str {
    env!("CARGO_BIN_EXE_monoagens-lab")
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root")
        .to_path_buf()
}

fn minimal_scenario(id: &str, core_config: Option<CoreConfig>) -> Scenario {
    let place = PlaceId::new("lab:room-1");
    let alice = ActorId::new("per_alice");
    let soul = ActorId::new("soul");
    let mut p = Percept::minimal(
        alice.clone(),
        place.clone(),
        Audience::new([alice, soul.clone()]),
        Time::from_millis(1_000),
        OpaquePayload::from_bytes(b"hello lab".as_slice()),
    );
    p.addressees = vec![soul];
    Scenario {
        id: id.to_owned(),
        name: "core_config cli test scenario".to_owned(),
        set: ScenarioSet::Development,
        start_time_ms: 0,
        model_id: "scripted".to_owned(),
        embedder_id: "none".to_owned(),
        model_latency_ms: 10,
        bodies: vec![TestBodySpec {
            body_id: BodyId::new("lab-test-body"),
            places: vec![place.clone()],
            capabilities: vec![CapabilityDescriptor {
                id: CapabilityId::new("a.speak"),
                description: "speak".to_owned(),
                places: vec![place],
                flags: CapabilityFlags {
                    communicative: true,
                    identity_changing: false,
                    irreversible: false,
                },
                typical_latency_ms: 5,
                capability_schema_hash: CapabilitySchemaHash::new("sha256:cap-a"),
                sensed_effect: None,
            }],
            action_scripts: Vec::new(),
        }],
        events: vec![ScriptedEvent {
            at_ms: 1_000,
            body_id: Some(BodyId::new("lab-test-body")),
            event: ScriptedEventKind::Percept(Box::new(p)),
        }],
        scripted_model: Vec::new(),
        checks: Vec::new(),
        core_config,
        model_path: None,
        online_adaptation: None,
        cost_guard: None,
        annotations: ScenarioAnnotations::default(),
    }
}

#[test]
fn lab_run_config_flag_updates_run_record_and_scenario_config_wins() {
    let tmp = TempDir::new().expect("tempdir");
    let sc_path = tmp.path().join("sc_no_cfg.json");
    let sc = minimal_scenario("cfg_test_no_override", None);
    fs::write(&sc_path, serde_json::to_vec(&sc).unwrap()).unwrap();

    let edited_path = tmp.path().join("edited_core_config.json");
    fs::write(
        &edited_path,
        r#"{
          "format_version": 1,
          "certainty_bar": 0.72,
          "online_adaptation": false
        }"#,
    )
    .unwrap();
    let edited_cfg = CoreConfig {
        certainty_bar: 0.72,
        online_adaptation: false,
        ..CoreConfig::default()
    };
    let edited_digest = compute_config_digest(&edited_cfg).unwrap();
    let default_digest = default_loaded_core_config().unwrap().digest;
    assert_ne!(edited_digest, default_digest);

    // 1. Without --config -> records built-in default digest.
    let rec_default_path = tmp.path().join("default.postcard");
    let out_def = Command::new(lab_bin())
        .args([
            "run",
            "--scenario",
            sc_path.to_str().unwrap(),
            "--record-out",
            rec_default_path.to_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();
    assert!(
        out_def.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out_def.stderr)
    );
    let def_json: Value = serde_json::from_slice(&out_def.stdout).unwrap();
    assert_eq!(def_json["config_digest"], default_digest);
    assert_eq!(def_json["config_format_version"], 1);
    assert_eq!(def_json["online_adaptation"], true);

    // 2. With --config <edited> -> records edited config digest and online_adaptation=false.
    let rec_edited_path = tmp.path().join("edited.postcard");
    let out_ed = Command::new(lab_bin())
        .args([
            "run",
            "--scenario",
            sc_path.to_str().unwrap(),
            "--config",
            edited_path.to_str().unwrap(),
            "--record-out",
            rec_edited_path.to_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();
    assert!(
        out_ed.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out_ed.stderr)
    );
    let ed_json: Value = serde_json::from_slice(&out_ed.stdout).unwrap();
    assert_eq!(ed_json["config_digest"], edited_digest);
    assert_eq!(ed_json["config_format_version"], 1);
    assert_eq!(ed_json["online_adaptation"], false);

    let decoded_rec = RunRecord::decode_postcard(&fs::read(&rec_edited_path).unwrap()).unwrap();
    assert_eq!(decoded_rec.config_digest, edited_digest);
    assert!(!decoded_rec.online_adaptation);

    // 3. Scenario with its own core_config wins over --config.
    let sc_own_cfg = CoreConfig {
        certainty_bar: 0.65,
        online_adaptation: false,
        ..CoreConfig::default()
    };
    let sc_own_digest = compute_config_digest(&sc_own_cfg).unwrap();
    let sc_own_path = tmp.path().join("sc_own_cfg.json");
    let sc_own = minimal_scenario("cfg_test_scenario_wins", Some(sc_own_cfg));
    fs::write(&sc_own_path, serde_json::to_vec(&sc_own).unwrap()).unwrap();

    let out_own = Command::new(lab_bin())
        .args([
            "run",
            "--scenario",
            sc_own_path.to_str().unwrap(),
            "--config",
            edited_path.to_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();
    assert!(out_own.status.success());
    let own_json: Value = serde_json::from_slice(&out_own.stdout).unwrap();
    assert_eq!(own_json["config_digest"], sc_own_digest);

    // 4. Missing explicit --config fails non-zero naming path.
    let missing_cfg = tmp.path().join("missing_core_config.json");
    let out_bad = Command::new(lab_bin())
        .args([
            "run",
            "--scenario",
            sc_path.to_str().unwrap(),
            "--config",
            missing_cfg.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(!out_bad.status.success());
    let stderr = String::from_utf8_lossy(&out_bad.stderr);
    assert!(stderr.contains("missing_core_config.json"), "stderr={stderr}");
}

#[test]
fn lab_compare_over_development_set_writes_run_records_per_side_and_seed() {
    let tmp = TempDir::new().expect("tempdir");
    let out_dir = tmp.path().join("compare_records");
    let root = workspace_root().join("lab/scenarios");
    let cfg_path = tmp.path().join("core_config.json");
    fs::write(&cfg_path, r#"{"format_version": 1}"#).unwrap();
    let expected_digest = default_loaded_core_config().unwrap().digest;
    let model_path = workspace_root().join("crates/core/tests/fixtures/neural_phase1_golden.safetensors");

    let out = Command::new(lab_bin())
        .args([
            "compare",
            "--set",
            "development",
            "--root",
            root.to_str().unwrap(),
            "--ablation",
            "neutral",
            "--model",
            model_path.to_str().unwrap(),
            "--config",
            cfg_path.to_str().unwrap(),
            "--seeds",
            "1,2,3,4,5",
            "--out-dir",
            out_dir.to_str().unwrap(),
            "--json",
        ])
        .output()
        .expect("run compare --set development");
    assert!(
        out.status.success(),
        "compare failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let deltas: Vec<Value> = serde_json::from_slice(&out.stdout).expect("json deltas");
    assert!(!deltas.is_empty());

    // 13 development scenarios * 2 sides * 5 seeds = 130 RunRecord files.
    let mut files = Vec::new();
    for entry in fs::read_dir(&out_dir).unwrap() {
        let p = entry.unwrap().path();
        if p.extension().and_then(|s| s.to_str()) == Some("postcard") {
            files.push(p);
        }
    }
    assert_eq!(files.len(), 130, "expected 130 run records in out_dir");

    for path in &files {
        let rec = RunRecord::decode_postcard(&fs::read(path).unwrap()).expect("decode RunRecord");
        assert_eq!(rec.config_digest, expected_digest);
        assert_eq!(rec.config_format_version, 1);
    }
}

#[test]
fn lab_suite_replay_baseline_and_benchmark_support_config_flag() {
    let tmp = TempDir::new().expect("tempdir");
    let dev_dir = tmp.path().join("scenarios").join("development");
    fs::create_dir_all(&dev_dir).unwrap();
    let sc = minimal_scenario("suite_cfg_sc", None);
    let sc_path = dev_dir.join("suite_cfg_sc.json");
    fs::write(&sc_path, serde_json::to_vec(&sc).unwrap()).unwrap();

    let cfg_path = tmp.path().join("core_config.json");
    fs::write(&cfg_path, r#"{"format_version": 1}"#).unwrap();
    let expected_digest = default_loaded_core_config().unwrap().digest;

    // suite --config
    let suite_out = Command::new(lab_bin())
        .args([
            "suite",
            "--root",
            tmp.path().join("scenarios").to_str().unwrap(),
            "--set",
            "development",
            "--config",
            cfg_path.to_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();
    assert!(suite_out.status.success());
    let suite_json: Value = serde_json::from_slice(&suite_out.stdout).unwrap();
    assert_eq!(suite_json["runs"][0]["config_digest"], expected_digest);

    // replay --config
    let replay_out = Command::new(lab_bin())
        .args([
            "replay",
            "--scenario",
            sc_path.to_str().unwrap(),
            "--config",
            cfg_path.to_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();
    assert!(replay_out.status.success());

    // benchmark --suite in_tree_only --config
    let template_manifest = workspace_root().join("lab/datasets.manifest.example.toml");
    let bench_datasets_dir = tmp.path().join("datasets");
    fs::create_dir_all(&bench_datasets_dir).unwrap();
    let bench_out = Command::new(lab_bin())
        .args([
            "benchmark",
            "--suite",
            "in_tree_only",
            "--manifest",
            template_manifest.to_str().unwrap(),
            "--datasets-dir",
            bench_datasets_dir.to_str().unwrap(),
            "--config",
            cfg_path.to_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();
    assert!(
        bench_out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&bench_out.stderr)
    );

    // baseline rejects missing --config path
    let missing = tmp.path().join("no_such_core_config.json");
    let base_bad = Command::new(lab_bin())
        .args([
            "baseline",
            "--adapter",
            "stateless_react",
            "--scenario",
            sc_path.to_str().unwrap(),
            "--config",
            missing.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(!base_bad.status.success());
    assert!(
        String::from_utf8_lossy(&base_bad.stderr).contains("no_such_core_config.json"),
        "stderr={}",
        String::from_utf8_lossy(&base_bad.stderr)
    );
}

#[test]
fn unchanged_dev_scenario_passes_with_and_without_open_config_loaded_event() {
    let dev = load_scenarios_from_set(
        &workspace_root().join("lab/scenarios"),
        ScenarioSet::Development,
    )
    .unwrap();

    // Empty store whose initial snapshot already records the scenario's CoreConfig with
    // `config_loaded_count = 0`, so `Core::open` appends no `config_loaded` event (`open_offset == 0`).
    let pre_seeded_zero_open_store = |cfg: CoreConfig| -> MemStore {
        let mut state = CoreState::new();
        state.recorded_config = Some(cfg);
        state.config_loaded_count = 0;
        let bytes = state.encode_snapshot();
        let blake3 = blake3::hash(&bytes);
        let mut s = MemStore::new();
        s.snapshot(SnapshotCheckpoint {
            snapshot_id: "zero-open-offset".to_owned(),
            cursor: LogSeq::ZERO,
            epoch: 0,
            bytes,
            blake3,
        })
        .unwrap();
        s
    };

    for target_id in ["dev-03-promises-downtime", "dev-05-sleep-credence-report"] {
        let sc = dev
            .iter()
            .find(|s| s.id == target_id)
            .unwrap_or_else(|| panic!("missing scenario {target_id}"));

        let with_cfg = run_scenario(MemStore::new(), CoreConfig::default(), sc, 1).unwrap();
        assert_eq!(with_cfg.run_record.open_offset, 1);
        assert!(
            with_cfg.run_record.passed,
            "{target_id} with open_offset=1 failed: {:?}",
            with_cfg.run_record.checks
        );

        let without_cfg = run_scenario(
            pre_seeded_zero_open_store(sc.core_config.clone().unwrap_or_default()),
            CoreConfig::default(),
            sc,
            1,
        )
        .unwrap();
        assert_eq!(without_cfg.run_record.open_offset, 0);
        assert!(
            without_cfg.run_record.passed,
            "{target_id} with open_offset=0 failed: {:?}",
            without_cfg.run_record.checks
        );

        assert_eq!(with_cfg.events.len(), without_cfg.events.len() + 1);
    }
}

#[test]
fn neural_controller_calibration_metrics_cic_neutral_and_online_adaptation_sweep() {
    let model_path =
        workspace_root().join("crates/core/tests/fixtures/neural_phase1_golden.safetensors");
    let model = load_neural_model_from_path(&model_path, None).expect("load golden model");

    let place = PlaceId::new("lab:cal");
    let alice = ActorId::new("per_alice");
    let soul = ActorId::new("soul");

    let mut p1 = Percept::minimal(
        alice.clone(),
        place.clone(),
        Audience::new([alice.clone(), soul.clone()]),
        Time::from_millis(1_000),
        OpaquePayload::from_bytes(b"addressed question".as_slice()),
    );
    p1.addressees = vec![soul.clone()];
    let p2 = Percept::minimal(
        alice,
        place.clone(),
        Audience::new([ActorId::new("per_alice"), soul]),
        Time::from_millis(2_000),
        OpaquePayload::from_bytes(b"ambient chatter".as_slice()),
    );

    let mut sc = minimal_scenario("neural_cal_sweep", None);
    sc.events = vec![
        ScriptedEvent {
            at_ms: 1_000,
            body_id: Some(BodyId::new("lab-test-body")),
            event: ScriptedEventKind::Percept(Box::new(p1)),
        },
        ScriptedEvent {
            at_ms: 2_000,
            body_id: Some(BodyId::new("lab-test-body")),
            event: ScriptedEventKind::Percept(Box::new(p2)),
        },
    ];
    sc.annotations.event_annotations.insert(
        0,
        EventAnnotation {
            warranted: Some(WarrantedReply::Speak),
            ..EventAnnotation::default()
        },
    );
    sc.annotations.event_annotations.insert(
        1,
        EventAnnotation {
            warranted: Some(WarrantedReply::Silent),
            ..EventAnnotation::default()
        },
    );

    // Run with Neural Controller vs Neutral Fallback
    let neural_out =
        run_scenario_with_model(MemStore::new(), CoreConfig::default(), Some(&model), &sc, 7)
            .expect("neural run");
    let neutral_out =
        run_scenario_with_model(MemStore::new(), CoreConfig::default(), None, &sc, 7)
            .expect("neutral run");

    let nc = compute_neural_calibration_metrics(&neural_out.events, Some(&sc))
        .expect("neural calibration metrics");
    assert_eq!(nc.n_decisions, 2);
    assert_eq!(nc.n_labeled, 2);
    assert!(nc.ece.is_some_and(|v| v.is_finite() && (0.0..=1.0).contains(&v)));
    assert!(nc.tc_ece.is_some_and(|v| v.is_finite() && (0.0..=1.0).contains(&v)));
    assert!(nc.brier.is_some_and(|v| v.is_finite() && (0.0..=1.0).contains(&v)));
    assert!(nc.aurc.is_some_and(|v| v.is_finite() && (0.0..=1.0).contains(&v)));
    assert!((nc.neutral_fallback_rate - 0.0).abs() < 1e-12);
    assert!((nc.cic_coverage + nc.cic_abstention_rate - 1.0).abs() < 1e-12);

    let nc_neu = compute_neural_calibration_metrics(&neutral_out.events, Some(&sc))
        .expect("neutral calibration metrics");
    assert!((nc_neu.neutral_fallback_rate - 1.0).abs() < 1e-12);

    // Paired sweep: online_adaptation true vs false across 5 seeds
    let sweep = run_ablation_compare(
        &sc,
        CoreConfig {
            online_adaptation: true,
            ..CoreConfig::default()
        },
        Some(&model),
        CoreConfig {
            online_adaptation: false,
            ..CoreConfig::default()
        },
        Some(&model),
        &[1, 2, 3, 4, 5],
    )
    .expect("online_adaptation sweep");
    assert!(!sweep.is_empty());
}
