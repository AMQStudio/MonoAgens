use std::fs;
use std::path::PathBuf;
use std::process::Command;

use monoagens_core::{CoreState, LogSeq, MemStore, Prior, SnapshotCheckpoint, Store, blake3};
use monoagens_lab::{
    RunRecord, Scenario, ScenarioAnnotations, ScenarioSet, ScriptedEvent, ScriptedEventKind,
    TestBodySpec, load_scenarios_from_set, run_scenario,
};
use monoagens_protocol::{
    ActorId, Audience, BodyId, CapabilityDescriptor, CapabilityFlags, CapabilityId,
    CapabilitySchemaHash, OpaquePayload, Percept, PlaceId, Time,
};
use monoagens_store::{compute_prior_digest, default_loaded_prior};
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

fn minimal_scenario(id: &str, prior: Option<Prior>) -> Scenario {
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
        name: "prior cli test scenario".to_owned(),
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
        switches: None,
        cost_guard: None,
        prior,
        annotations: ScenarioAnnotations::default(),
    }
}

#[test]
fn lab_run_prior_flag_updates_run_record_and_scenario_prior_wins() {
    let tmp = TempDir::new().expect("tempdir");
    let sc_path = tmp.path().join("sc_no_prior.json");
    let sc = minimal_scenario("prior_test_no_override", None);
    fs::write(&sc_path, serde_json::to_vec(&sc).unwrap()).unwrap();

    let edited_path = tmp.path().join("edited_prior.json");
    fs::write(
        &edited_path,
        r#"{
          "version": 1,
          "content_version": "2026-11-17.edited",
          "talkativeness": 0.85,
          "spontaneity": 0.5,
          "memory_persistence": 0.5,
          "helpfulness": 0.5,
          "restraint": 0.5,
          "boredom_with_repeats": 0.5,
          "certainty_bar": 0.5,
          "speaking_bar": 3
        }"#,
    )
    .unwrap();
    let edited_prior = Prior {
        content_version: "2026-11-17.edited".to_owned(),
        talkativeness: 0.85,
        ..Prior::default()
    };
    let edited_digest = compute_prior_digest(&edited_prior).unwrap();
    let default_digest = default_loaded_prior().unwrap().digest;
    assert_ne!(edited_digest, default_digest);

    // 1. Without --prior -> records built-in default digest.
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
    assert_eq!(def_json["prior_digest"], default_digest);
    assert_eq!(def_json["prior_format_version"], 1);
    assert_eq!(def_json["prior_content_version"], "2026-11-16.1");

    // 2. With --prior <edited> -> records edited prior digest and version.
    let rec_edited_path = tmp.path().join("edited.postcard");
    let out_ed = Command::new(lab_bin())
        .args([
            "run",
            "--scenario",
            sc_path.to_str().unwrap(),
            "--prior",
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
    assert_eq!(ed_json["prior_digest"], edited_digest);
    assert_eq!(ed_json["prior_content_version"], "2026-11-17.edited");
    assert_eq!(ed_json["prior_format_version"], 1);

    let decoded_rec = RunRecord::decode_postcard(&fs::read(&rec_edited_path).unwrap()).unwrap();
    assert_eq!(decoded_rec.prior_digest, edited_digest);
    assert_eq!(decoded_rec.prior_content_version, "2026-11-17.edited");

    // 3. Scenario with its own prior wins over --prior, and run record records the scenario prior's digest.
    let sc_own_prior = Prior {
        content_version: "2026-11-17.scenario-wins".to_owned(),
        talkativeness: 0.25,
        ..Prior::default()
    };
    let sc_own_digest = compute_prior_digest(&sc_own_prior).unwrap();
    let sc_own_path = tmp.path().join("sc_own_prior.json");
    let sc_own = minimal_scenario("prior_test_scenario_wins", Some(sc_own_prior));
    fs::write(&sc_own_path, serde_json::to_vec(&sc_own).unwrap()).unwrap();

    let out_own = Command::new(lab_bin())
        .args([
            "run",
            "--scenario",
            sc_own_path.to_str().unwrap(),
            "--prior",
            edited_path.to_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();
    assert!(out_own.status.success());
    let own_json: Value = serde_json::from_slice(&out_own.stdout).unwrap();
    assert_eq!(own_json["prior_digest"], sc_own_digest);
    assert_eq!(
        own_json["prior_content_version"],
        "2026-11-17.scenario-wins"
    );

    // 4. Missing explicit --prior fails non-zero naming path.
    let missing_prior = tmp.path().join("missing_prior.json");
    let out_bad = Command::new(lab_bin())
        .args([
            "run",
            "--scenario",
            sc_path.to_str().unwrap(),
            "--prior",
            missing_prior.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(!out_bad.status.success());
    let stderr = String::from_utf8_lossy(&out_bad.stderr);
    assert!(stderr.contains("missing_prior.json"), "stderr={stderr}");
}

#[test]
fn lab_compare_over_development_set_writes_run_records_per_side_and_seed() {
    let tmp = TempDir::new().expect("tempdir");
    let out_dir = tmp.path().join("compare_records");
    let root = workspace_root().join("lab/scenarios");
    let prior_path = workspace_root().join("prior/dispositions.json");
    let expected_digest = default_loaded_prior().unwrap().digest;

    let out = Command::new(lab_bin())
        .args([
            "compare",
            "--set",
            "development",
            "--root",
            root.to_str().unwrap(),
            "--mechanism",
            "habituation",
            "--prior",
            prior_path.to_str().unwrap(),
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

    // 8 development scenarios * 2 sides * 5 seeds = 80 RunRecord files.
    let mut files = Vec::new();
    for entry in fs::read_dir(&out_dir).unwrap() {
        let p = entry.unwrap().path();
        if p.extension().and_then(|s| s.to_str()) == Some("postcard") {
            files.push(p);
        }
    }
    assert_eq!(files.len(), 80, "expected 80 run records in out_dir");

    for path in &files {
        let rec = RunRecord::decode_postcard(&fs::read(path).unwrap()).expect("decode RunRecord");
        assert_eq!(rec.prior_digest, expected_digest);
        assert_eq!(rec.prior_format_version, 1);
    }
}

#[test]
fn lab_suite_replay_baseline_and_benchmark_support_prior_flag() {
    let tmp = TempDir::new().expect("tempdir");
    let dev_dir = tmp.path().join("scenarios").join("development");
    fs::create_dir_all(&dev_dir).unwrap();
    let sc = minimal_scenario("suite_prior_sc", None);
    let sc_path = dev_dir.join("suite_prior_sc.json");
    fs::write(&sc_path, serde_json::to_vec(&sc).unwrap()).unwrap();

    let prior_path = workspace_root().join("prior/dispositions.json");
    let expected_digest = default_loaded_prior().unwrap().digest;

    // suite --prior
    let suite_out = Command::new(lab_bin())
        .args([
            "suite",
            "--root",
            tmp.path().join("scenarios").to_str().unwrap(),
            "--set",
            "development",
            "--prior",
            prior_path.to_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();
    assert!(suite_out.status.success());
    let suite_json: Value = serde_json::from_slice(&suite_out.stdout).unwrap();
    assert_eq!(suite_json["runs"][0]["prior_digest"], expected_digest);

    // replay --prior
    let replay_out = Command::new(lab_bin())
        .args([
            "replay",
            "--scenario",
            sc_path.to_str().unwrap(),
            "--prior",
            prior_path.to_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();
    assert!(replay_out.status.success());

    // benchmark --suite in_tree_only --prior
    let bench_out = Command::new(lab_bin())
        .args([
            "benchmark",
            "--suite",
            "in_tree_only",
            "--prior",
            prior_path.to_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();
    assert!(bench_out.status.success());

    // baseline rejects missing --prior path
    let missing = tmp.path().join("no_such_prior.json");
    let base_bad = Command::new(lab_bin())
        .args([
            "baseline",
            "--adapter",
            "stateless_react",
            "--scenario",
            sc_path.to_str().unwrap(),
            "--prior",
            missing.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(!base_bad.status.success());
    assert!(
        String::from_utf8_lossy(&base_bad.stderr).contains("no_such_prior.json"),
        "stderr={}",
        String::from_utf8_lossy(&base_bad.stderr)
    );
}

#[test]
fn unchanged_dev_scenario_passes_with_and_without_open_prior_loaded_event() {
    let dev = load_scenarios_from_set(
        &workspace_root().join("lab/scenarios"),
        ScenarioSet::Development,
    )
    .unwrap();

    // Empty store whose initial snapshot already records the scenario's prior with
    // `prior_loaded_count = 0`, so `Core::open` appends no `prior_loaded` event (`open_offset == 0`).
    let pre_seeded_zero_open_store = |prior: Prior| -> MemStore {
        let mut state = CoreState::new();
        state.recorded_prior = Some(prior);
        state.prior_loaded_count = 0;
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

        let with_prior = run_scenario(MemStore::new(), Prior::default(), sc, 1).unwrap();
        assert_eq!(with_prior.run_record.open_offset, 1);
        assert!(
            with_prior.run_record.passed,
            "{target_id} with open_offset=1 failed: {:?}",
            with_prior.run_record.checks
        );

        let without_prior = run_scenario(
            pre_seeded_zero_open_store(sc.prior.clone().unwrap_or_default()),
            Prior::default(),
            sc,
            1,
        )
        .unwrap();
        assert_eq!(without_prior.run_record.open_offset, 0);
        assert!(
            without_prior.run_record.passed,
            "{target_id} with open_offset=0 failed: {:?}",
            without_prior.run_record.checks
        );

        assert_eq!(with_prior.events.len(), without_prior.events.len() + 1);
    }
}
