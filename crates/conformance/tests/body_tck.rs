use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;
use tempfile::TempDir;

fn build_fixture(tmp: &TempDir, mode: &str) -> PathBuf {
    let mock_bin = env!("CARGO_BIN_EXE_tck-mock-body");
    let path = tmp.path().join(format!("fixture_{mode}.sh"));
    let script = format!("#!/bin/sh\nexec \"{mock_bin}\" --mode {mode} \"$@\"\n");
    fs::write(&path, script).expect("write fixture wrapper");
    let mut perms = fs::metadata(&path).expect("metadata").permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&path, perms).expect("chmod");
    path
}

fn run_conformance_cli(fixture: &Path, level: &str, report_path: &Path) -> (bool, Value, Vec<u8>) {
    let exe = env!("CARGO_BIN_EXE_monoagens-conformance");
    let output = Command::new(exe)
        .args([
            "body",
            "--target",
            fixture.to_str().expect("utf8 path"),
            "--level",
            level,
            "--json-report",
            report_path.to_str().expect("utf8 report"),
        ])
        .output()
        .expect("spawn monoagens-conformance");
    let raw = fs::read(report_path).expect("read compatibility report");
    let parsed: Value = serde_json::from_slice(&raw).expect("parse compatibility report");
    (output.status.success(), parsed, raw)
}

fn check_status(report: &Value, id: &str) -> String {
    report["checks"]
        .as_array()
        .expect("checks array")
        .iter()
        .find(|c| c["id"] == id)
        .unwrap_or_else(|| panic!("missing check '{id}' in report: {report}"))["status"]
        .as_str()
        .expect("status string")
        .to_owned()
}

fn assert_broken_fixture_fails(mode: &str, level: &str, expected_failed_id: &str) {
    let tmp = TempDir::new().expect("tempdir");
    let fixture = build_fixture(&tmp, mode);
    let report_path = tmp.path().join("compatibility.json");
    let (ok, report, _raw) = run_conformance_cli(&fixture, level, &report_path);

    assert!(
        !ok,
        "broken fixture '{mode}' must exit non-zero, report: {report}"
    );
    assert_eq!(
        check_status(&report, expected_failed_id),
        "fail",
        "broken fixture '{mode}' must fail check '{expected_failed_id}', report: {report}"
    );
}

#[test]
fn good_fixture_passes_all_must_and_should_and_reports_are_byte_identical() {
    let tmp = TempDir::new().expect("tempdir");
    let good = build_fixture(&tmp, "good");

    let r1 = tmp.path().join("compat_1.json");
    let r2 = tmp.path().join("compat_2.json");

    let (ok1, parsed1, bytes1) = run_conformance_cli(&good, "should", &r1);
    let (ok2, _parsed2, bytes2) = run_conformance_cli(&good, "should", &r2);

    assert!(ok1, "good fixture failed --level should: {parsed1}");
    assert!(ok2);
    assert_eq!(
        bytes1, bytes2,
        "two runs for the same target and level must yield byte-identical compatibility.json"
    );

    for id in [
        "B-MUST-01",
        "B-MUST-02",
        "B-MUST-03",
        "B-MUST-04",
        "B-MUST-05",
        "B-MUST-06",
        "B-MUST-07",
        "B-SHOULD-01",
        "B-SHOULD-02",
        "B-SHOULD-03",
    ] {
        assert_eq!(check_status(&parsed1, id), "pass", "expected pass for {id}");
    }
}

#[test]
fn b_must_01_fails_on_unsorted_discover_fixture() {
    assert_broken_fixture_fails("unsorted_discover", "must", "B-MUST-01");
}

#[test]
fn b_must_02_fails_on_duplicate_execution_fixture() {
    assert_broken_fixture_fails("duplicate_execution", "must", "B-MUST-02");
}

#[test]
fn b_must_03_fails_on_silent_crash_fixture() {
    assert_broken_fixture_fails("silent_crash", "must", "B-MUST-03");
}

#[test]
fn b_must_04_fails_on_cross_place_execution_fixture() {
    assert_broken_fixture_fails("cross_place", "must", "B-MUST-04");
}

#[test]
fn b_must_05_fails_on_double_receipt_fixture() {
    assert_broken_fixture_fails("double_receipt", "must", "B-MUST-05");
}

#[test]
fn b_must_06_fails_on_accepted_bad_digest_fixture() {
    assert_broken_fixture_fails("bad_digest", "must", "B-MUST-06");
}

#[test]
fn b_must_07_fails_on_invented_capability_name_fixture() {
    assert_broken_fixture_fails("unknown_capability", "must", "B-MUST-07");
}

#[test]
fn b_should_01_fails_on_bad_heartbeat_fixture() {
    assert_broken_fixture_fails("bad_heartbeat", "should", "B-SHOULD-01");
}

#[test]
fn b_should_02_fails_on_artificial_delivery_delay_fixture() {
    assert_broken_fixture_fails("bad_delivery_delay", "should", "B-SHOULD-02");
}

#[test]
fn b_should_03_fails_on_accepted_undeclared_audience_fixture() {
    assert_broken_fixture_fails("accept_undeclared_audience", "should", "B-SHOULD-03");
}
