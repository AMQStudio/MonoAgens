//! Standalone black-box Core TCK (`C-MUST-01..04`, `C-SHOULD-01..04`) and broken-core fixture tests.

use std::fs;
use std::process::Command;

use monoagens_conformance::{
    CheckStatus, CompatibilityReport, ConformanceLevel, run_core_conformance,
};
use serde_json::Value;
use tempfile::TempDir;

#[test]
fn core_tck_passes_c_must_and_c_should_and_is_byte_identical() {
    let harness = env!("CARGO_BIN_EXE_tck-core-harness");
    let exe = env!("CARGO_BIN_EXE_monoagens-conformance");
    let tmp = TempDir::new().expect("tempdir");
    let report1_path = tmp.path().join("compat_core_1.json");
    let report2_path = tmp.path().join("compat_core_2.json");

    for path in [&report1_path, &report2_path] {
        let out = Command::new(exe)
            .args([
                "core",
                "--target",
                harness,
                "--target-arg",
                "--mode",
                "--target-arg",
                "good",
                "--level",
                "must",
                "--json-report",
                path.to_str().unwrap(),
            ])
            .output()
            .expect("run core TCK CLI");
        assert!(
            out.status.success(),
            "core TCK failed: stderr={}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    let bytes1 = fs::read(&report1_path).expect("read report 1");
    let bytes2 = fs::read(&report2_path).expect("read report 2");
    assert_eq!(
        bytes1, bytes2,
        "Core compatibility.json must be byte-identical across runs"
    );

    let parsed: CompatibilityReport = serde_json::from_slice(&bytes1).expect("parse compat json");
    assert!(parsed.passed());
    let ids: Vec<&str> = parsed.checks.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(
        ids,
        vec!["C-MUST-01", "C-MUST-02", "C-MUST-03", "C-MUST-04"]
    );
    assert!(parsed.checks.iter().all(|c| c.status == CheckStatus::Pass));

    // Also verify `--level should` covers 20.2 invariants C-SHOULD-01..04
    let should_rep = run_core_conformance(
        harness,
        &["--mode".to_owned(), "good".to_owned()],
        ConformanceLevel::Should,
    )
    .expect("core conformance should level");
    assert!(should_rep.passed());
    let should_ids: Vec<&str> = should_rep.checks.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(
        should_ids,
        vec![
            "C-MUST-01",
            "C-MUST-02",
            "C-MUST-03",
            "C-MUST-04",
            "C-SHOULD-01",
            "C-SHOULD-02",
            "C-SHOULD-03",
            "C-SHOULD-04",
        ]
    );
}

#[test]
fn core_tck_fails_each_deliberately_broken_core_fixture_and_missing_target() {
    let harness = env!("CARGO_BIN_EXE_tck-core-harness");
    let exe = env!("CARGO_BIN_EXE_monoagens-conformance");
    let tmp = TempDir::new().expect("tempdir");

    let cases = [
        ("leak_scope", "C-MUST-01"),
        ("forget_resurrect", "C-MUST-02"),
        ("nondeterministic", "C-MUST-03"),
        ("soak_nan", "C-MUST-04"),
    ];

    for (broken_mode, expected_failed_id) in cases {
        let rep_path = tmp.path().join(format!("bad_{broken_mode}.json"));
        let out = Command::new(exe)
            .args([
                "core",
                "--target",
                harness,
                "--target-arg",
                "--mode",
                "--target-arg",
                broken_mode,
                "--level",
                "must",
                "--json-report",
                rep_path.to_str().unwrap(),
            ])
            .output()
            .expect("run broken core fixture");
        assert!(
            !out.status.success(),
            "broken fixture '{broken_mode}' must exit non-zero"
        );
        let parsed: Value =
            serde_json::from_slice(&fs::read(&rep_path).expect("read bad report")).expect("json");
        let checks = parsed["checks"].as_array().expect("checks array");
        let target_check = checks
            .iter()
            .find(|c| c["id"] == expected_failed_id)
            .expect("expected check id present");
        assert_eq!(
            target_check["status"], "fail",
            "mode '{broken_mode}' must fail {expected_failed_id}, got: {target_check}"
        );
    }

    // Missing target must exit non-zero naming the missing target on stderr
    let missing_target = "/nonexistent/monoagens-missing-core-xyz";
    let missing_out = Command::new(exe)
        .args(["core", "--target", missing_target, "--level", "must"])
        .output()
        .expect("run missing core target");
    assert!(!missing_out.status.success());
    let stderr = String::from_utf8_lossy(&missing_out.stderr);
    assert!(
        stderr.contains(missing_target),
        "stderr must name missing target '{missing_target}', got: {stderr}"
    );
}
