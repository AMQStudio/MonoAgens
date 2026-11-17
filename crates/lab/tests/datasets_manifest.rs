use std::fs;
use std::path::PathBuf;
use std::process::Command;

use monoagens_lab::datasets::{
    CoreApplicability, DatasetAvailability, LicenseStatus, ManifestEntryKind, SourceKind,
    check_registry_manifest_consistency, load_and_validate_manifest, parse_manifest_strict,
    save_manifest_to_path,
};
use monoagens_lab::standard_benchmark_registry;
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

#[test]
fn manifest_skeleton_parses_strictly_and_matches_registry() {
    let manifest_path = workspace_root().join("lab/datasets.manifest.example.toml");
    let raw = fs::read_to_string(&manifest_path)
        .expect("committed lab/datasets.manifest.example.toml must exist");
    assert!(
        !raw.contains("status = \"cleared\"") && !raw.contains("status = \"refused\""),
        "template must contain zero cleared/refused license statuses"
    );
    let registry = standard_benchmark_registry();
    let manifest = load_and_validate_manifest(&manifest_path, &registry).expect(
        "committed lab/datasets.manifest.example.toml must parse strictly and match registry",
    );

    assert!(!manifest.datasets.is_empty());
    for entry in &manifest.datasets {
        assert_eq!(
            entry.license.status,
            LicenseStatus::Pending,
            "all licenses in manifest skeleton must remain pending: {}",
            entry.id
        );
    }

    let gmb = manifest
        .get("group_mem_bench")
        .expect("group_mem_bench present");
    assert!(
        gmb.sources.len() >= 2,
        "group_mem_bench must support multi-source [[datasets.sources]]"
    );

    // Group 6 checks: 23 external + 1 local tool_rag_scaling = 24 registry entries.
    // Manifest also holds 2 non-corpus `scorer_model` entries (not counted as benchmark datasets).
    assert_eq!(registry.len(), 24);
    let dataset_entries: Vec<_> = manifest
        .datasets
        .iter()
        .filter(|d| d.kind == ManifestEntryKind::Dataset)
        .collect();
    let scorer_models: Vec<_> = manifest
        .datasets
        .iter()
        .filter(|d| d.kind == ManifestEntryKind::ScorerModel)
        .collect();
    assert_eq!(dataset_entries.len(), 23);
    assert_eq!(scorer_models.len(), 2);
    for sm_id in ["fantom_scorer_all_roberta_large_v1", "proactive_agent_rm"] {
        let sm = manifest.get(sm_id).expect("scorer model present");
        assert_eq!(sm.kind, ManifestEntryKind::ScorerModel);
        assert_eq!(sm.availability, DatasetAvailability::Available);
        assert_eq!(sm.core_applicability, CoreApplicability::NotMappable);
        assert_eq!(sm.license.status, LicenseStatus::Pending);
        assert_eq!(sm.sources[0].kind, SourceKind::Huggingface);
        assert_eq!(sm.sources[0].revision, "pending_pin");
        assert!(sm.reason.to_ascii_lowercase().contains("scorer model"));
    }
    assert!(manifest.get("dyntom").is_some());
    let exp = registry.iter().find(|s| s.id == "exploretom").unwrap();
    assert_eq!(exp.primary_metrics, vec!["exact_match_acc"]);
    let dyntom = registry.iter().find(|s| s.id == "dyntom").unwrap();
    assert_eq!(dyntom.primary_metrics, vec!["dyntom_acc"]);
    let irc = registry.iter().find(|s| s.id == "ubuntu_irc").unwrap();
    assert_eq!(irc.primary_metrics, vec!["thread_f1", "parent_speaker_p1"]);

    // Unavailable & not_mappable statuses
    for uid in ["pis", "proact", "mpbench"] {
        assert_eq!(
            manifest.get(uid).unwrap().availability,
            monoagens_lab::datasets::DatasetAvailability::Unavailable
        );
    }
    for eid in ["multilexnorm", "indocollex"] {
        let e = manifest.get(eid).unwrap();
        assert_eq!(
            e.core_applicability,
            monoagens_lab::datasets::CoreApplicability::NotMappable
        );
        assert_eq!(e.lane, monoagens_lab::datasets::DatasetLane::EmbedderProbe);
    }
}

#[test]
fn in_tree_scorecard_reports_unavailable_and_not_mappable_gaps() {
    let tmp = TempDir::new().expect("tempdir");
    let datasets_dir = tmp.path().join("datasets");
    fs::create_dir_all(&datasets_dir).unwrap();
    let template_path = workspace_root().join("lab/datasets.manifest.example.toml");

    let out = Command::new(lab_bin())
        .args([
            "benchmark",
            "--suite",
            "in_tree_only",
            "--manifest",
            template_path.to_str().unwrap(),
            "--datasets-dir",
            datasets_dir.to_str().unwrap(),
            "--json",
        ])
        .output()
        .expect("benchmark in_tree_only");
    assert!(
        out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let rep: Value = serde_json::from_slice(&out.stdout).unwrap();
    let scorecard = rep["scorecard"].as_array().unwrap();
    assert_eq!(scorecard.len(), 23);

    for uid in ["pis", "proact", "mpbench"] {
        let row = scorecard.iter().find(|r| r["dataset_id"] == uid).unwrap();
        assert_eq!(row["status"], "unavailable");
        assert!(row["reason"].as_str().is_some_and(|s| !s.is_empty()));
    }
    for eid in ["multilexnorm", "indocollex"] {
        let row = scorecard.iter().find(|r| r["dataset_id"] == eid).unwrap();
        assert_eq!(row["lane"], "embedder_probe");
        assert_eq!(row["core_applicability"], "not_mappable");
    }
}

#[test]
fn manifest_strict_parsing_rejects_unknown_keys() {
    let raw = fs::read_to_string(workspace_root().join("lab/datasets.manifest.example.toml"))
        .expect("read manifest template");
    let with_unknown = format!("{raw}\nunknown_top_level_key = \"boom\"\n");
    let err = parse_manifest_strict(&with_unknown).expect_err("unknown top-level key must fail");
    assert!(
        err.to_string().contains("unknown field"),
        "expected unknown field error, got: {err}"
    );

    let with_nested_unknown = raw.replacen(
        "id = \"locomo\"",
        "id = \"locomo\"\nunknown_dataset_key = 123",
        1,
    );
    let err2 = parse_manifest_strict(&with_nested_unknown)
        .expect_err("unknown dataset key must fail strict parsing");
    assert!(
        err2.to_string().contains("unknown field"),
        "expected unknown field error, got: {err2}"
    );
}

#[test]
fn manifest_registry_consistency_reports_missing_and_extra_ids() {
    let manifest_path = workspace_root().join("lab/datasets.manifest.example.toml");
    let registry = standard_benchmark_registry();
    let base = load_and_validate_manifest(&manifest_path, &registry).expect("load base manifest");

    // 1. Missing id in manifest
    let mut missing = base.clone();
    missing.datasets.retain(|d| d.id != "locomo");
    let err_missing = check_registry_manifest_consistency(&missing, &registry)
        .expect_err("missing id must fail consistency check");
    assert!(
        err_missing.to_string().contains("locomo"),
        "error must name missing id 'locomo', got: {err_missing}"
    );

    // 2. Extra id in manifest
    let mut extra = base.clone();
    let mut cloned = extra.datasets[0].clone();
    cloned.id = "unexpected_extra_bench".to_owned();
    extra.datasets.push(cloned);
    let err_extra = check_registry_manifest_consistency(&extra, &registry)
        .expect_err("extra id must fail consistency check");
    assert!(
        err_extra.to_string().contains("unexpected_extra_bench"),
        "error must name extra id 'unexpected_extra_bench', got: {err_extra}"
    );

    // 3. ScorerModel colliding with a registry id must fail consistency check
    let mut collide = base;
    let mut sm = collide
        .get("fantom_scorer_all_roberta_large_v1")
        .unwrap()
        .clone();
    sm.id = "locomo".to_owned();
    collide.datasets.push(sm);
    let err_collide = check_registry_manifest_consistency(&collide, &registry)
        .expect_err("colliding scorer_model id must fail consistency check");
    assert!(
        err_collide.to_string().contains("locomo"),
        "error must name colliding id 'locomo', got: {err_collide}"
    );
}

#[test]
fn cli_datasets_list_json_and_text_reports_sources_risk_and_pending_pin() {
    let tmp = TempDir::new().expect("tempdir");
    let manifest_path = tmp.path().join("fixture.manifest.toml");
    let datasets_dir = tmp.path().join("datasets");
    fs::create_dir_all(&datasets_dir).unwrap();

    let base_path = workspace_root().join("lab/datasets.manifest.example.toml");
    let manifest = load_and_validate_manifest(&base_path, &standard_benchmark_registry())
        .expect("load base manifest");
    save_manifest_to_path(&manifest, &manifest_path).expect("write fixture manifest");

    let out_json = Command::new(lab_bin())
        .args([
            "datasets",
            "list",
            "--manifest",
            manifest_path.to_str().unwrap(),
            "--datasets-dir",
            datasets_dir.to_str().unwrap(),
            "--json",
        ])
        .output()
        .expect("run datasets list --json");
    assert!(
        out_json.status.success(),
        "datasets list --json failed: {}",
        String::from_utf8_lossy(&out_json.stderr)
    );

    let parsed: Value = serde_json::from_slice(&out_json.stdout).expect("parse JSON output");
    let rows = parsed["datasets"].as_array().expect("datasets array");
    assert_eq!(rows.len(), manifest.datasets.len());

    let gmb = rows
        .iter()
        .find(|r| r["id"] == "group_mem_bench")
        .expect("group_mem_bench row");
    assert_eq!(gmb["sources"].as_array().unwrap().len(), 2);
    assert_eq!(gmb["license_status"], "pending");
    assert_eq!(gmb["availability"], "available");
    assert_eq!(gmb["contamination_risk"], "low");
    assert_eq!(gmb["has_pending_pin"], true);
    assert_eq!(gmb["missing_owner_checksums"], true);
    assert!(
        gmb["verification_state"]
            .as_str()
            .unwrap()
            .contains("missing")
    );

    // Text mode also succeeds and prints expected fields
    let out_txt = Command::new(lab_bin())
        .args([
            "datasets",
            "list",
            "--manifest",
            manifest_path.to_str().unwrap(),
            "--datasets-dir",
            datasets_dir.to_str().unwrap(),
        ])
        .output()
        .expect("run datasets list text");
    assert!(out_txt.status.success());
    let stdout = String::from_utf8_lossy(&out_txt.stdout);
    assert!(stdout.contains("group_mem_bench"));
    assert!(stdout.contains("pending_pin="));
    assert!(stdout.contains("missing_owner_checksums"));

    // Corrupting the fixture manifest with an extra id fails CLI datasets list non-zero
    let mut bad = manifest;
    let mut extra_entry = bad.datasets[0].clone();
    extra_entry.id = "bogus_id".to_owned();
    bad.datasets.push(extra_entry);
    save_manifest_to_path(&bad, &manifest_path).unwrap();

    let out_bad = Command::new(lab_bin())
        .args([
            "datasets",
            "list",
            "--manifest",
            manifest_path.to_str().unwrap(),
            "--datasets-dir",
            datasets_dir.to_str().unwrap(),
        ])
        .output()
        .expect("run datasets list on mismatched manifest");
    assert!(!out_bad.status.success());
    let stderr = String::from_utf8_lossy(&out_bad.stderr);
    assert!(stderr.contains("bogus_id"), "stderr={stderr}");
}

#[test]
fn cli_datasets_init_creates_local_manifest_and_refuses_overwrite_and_force() {
    let tmp = TempDir::new().expect("tempdir");
    let template_path = workspace_root().join("lab/datasets.manifest.example.toml");
    let template_bytes = fs::read(&template_path).expect("read template");

    // 1. First init creates lab/datasets.manifest.toml from the committed template.
    let out_init = Command::new(lab_bin())
        .current_dir(tmp.path())
        .args(["datasets", "init"])
        .output()
        .expect("run datasets init");
    assert!(
        out_init.status.success(),
        "datasets init failed: {}",
        String::from_utf8_lossy(&out_init.stderr)
    );
    let local_manifest = tmp.path().join("lab/datasets.manifest.toml");
    assert!(local_manifest.is_file());
    assert_eq!(fs::read(&local_manifest).unwrap(), template_bytes);

    // 2. Subsequent init refuses without overwriting existing content.
    let modified = String::from_utf8(template_bytes).unwrap().replacen(
        "status = \"pending\"",
        "status = \"cleared\"",
        1,
    );
    fs::write(&local_manifest, modified.as_bytes()).unwrap();

    let out_overwrite = Command::new(lab_bin())
        .current_dir(tmp.path())
        .args(["datasets", "init"])
        .output()
        .expect("run datasets init over existing");
    assert!(!out_overwrite.status.success());
    let stderr_overwrite = String::from_utf8_lossy(&out_overwrite.stderr);
    assert!(
        stderr_overwrite.contains("refusing to overwrite"),
        "stderr={stderr_overwrite}"
    );
    assert_eq!(fs::read(&local_manifest).unwrap(), modified.as_bytes());

    // 3. --force is not allowed and leaves the existing file untouched.
    let out_force = Command::new(lab_bin())
        .current_dir(tmp.path())
        .args(["datasets", "init", "--force"])
        .output()
        .expect("run datasets init --force");
    assert!(!out_force.status.success());
    let stderr_force = String::from_utf8_lossy(&out_force.stderr);
    assert!(
        stderr_force.contains("--force is not allowed"),
        "stderr={stderr_force}"
    );
    assert_eq!(fs::read(&local_manifest).unwrap(), modified.as_bytes());
}

#[test]
fn cli_commands_fail_naming_datasets_init_when_local_manifest_absent() {
    let tmp = TempDir::new().expect("tempdir");

    let cases: &[&[&str]] = &[
        &["datasets", "list"],
        &["datasets", "pin", "locomo"],
        &["datasets", "fetch", "locomo"],
        &["datasets", "verify"],
        &["benchmark", "--suite", "standard"],
        &["benchmark", "--suite", "in_tree_only"],
    ];

    for args in cases {
        let out = Command::new(lab_bin())
            .current_dir(tmp.path())
            .args(*args)
            .output()
            .unwrap_or_else(|e| panic!("failed to spawn {:?}: {e}", args));
        assert!(
            !out.status.success(),
            "command {:?} must fail when local manifest is absent",
            args
        );
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            stderr.contains("monoagens-lab datasets init"),
            "command {:?} stderr must mention `monoagens-lab datasets init`, got: {stderr}",
            args
        );
    }
}
