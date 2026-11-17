use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;

use serde_json::Value;
use tempfile::TempDir;

fn write_executable(dir: &Path, name: &str, body: &str) -> std::path::PathBuf {
    let path = dir.join(name);
    fs::write(&path, body).expect("write script");
    let mut perms = fs::metadata(&path).expect("metadata").permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&path, perms).expect("chmod");
    path
}

fn mock_body_wrapper(dir: &Path, mode: &str) -> std::path::PathBuf {
    let mock_bin = env!("CARGO_BIN_EXE_tck-mock-body");
    let script = format!("#!/bin/sh\nexec \"{mock_bin}\" --mode {mode} \"$@\"\n");
    write_executable(dir, &format!("fixture_{mode}.sh"), &script)
}

fn unsorted_caps_fixture_script() -> &'static str {
    r#"#!/bin/sh
while IFS= read -r line; do
  case "$line" in
    *"body/discover"*)
      printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"body_id":"fixture-bad","protocol_version":{"major":1,"minor":0},"capabilities":[{"id":"z.last","description":"Last","places":["fixture:0"],"flags":{"communicative":false,"identity_changing":false,"irreversible":false},"typical_latency_ms":1,"capability_schema_hash":"sha256:z"},{"id":"a.first","description":"First","places":["fixture:0"],"flags":{"communicative":false,"identity_changing":false,"irreversible":false},"typical_latency_ms":1,"capability_schema_hash":"sha256:a"}],"places":["fixture:0"]}}'
      ;;
    *"body/shutdown"*)
      printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{"accepted":true}}'
      ;;
    *)
      id=$(printf '%s\n' "$line" | sed -n 's/.*"id":\([0-9]*\).*/\1/p')
      [ -z "$id" ] && id=1
      printf '%s\n' "{\"jsonrpc\":\"2.0\",\"id\":$id,\"error\":{\"code\":-32601,\"message\":\"Method not found\"}}"
      ;;
  esac
done
"#
}

#[test]
fn conformance_runner_spawns_good_fixture_produces_deterministic_report_and_filters_levels() {
    let tmp = TempDir::new().expect("tempdir");
    let good_fixture = mock_body_wrapper(tmp.path(), "good");
    let discover_only_fixture = mock_body_wrapper(tmp.path(), "discover_only");
    let report1_path = tmp.path().join("report1.json");
    let report2_path = tmp.path().join("report2.json");
    let should_path = tmp.path().join("should.json");
    let may_path = tmp.path().join("may.json");
    let incomplete_path = tmp.path().join("incomplete.json");

    let exe = env!("CARGO_BIN_EXE_monoagens-conformance");

    // Run 1 with --level must
    let out1 = Command::new(exe)
        .args([
            "body",
            "--target",
            good_fixture.to_str().unwrap(),
            "--level",
            "must",
            "--json-report",
            report1_path.to_str().unwrap(),
        ])
        .output()
        .expect("run conformance 1");
    assert!(
        out1.status.success(),
        "expected exit 0, stderr: {}, stdout: {}",
        String::from_utf8_lossy(&out1.stderr),
        String::from_utf8_lossy(&out1.stdout)
    );

    // Run 2 with --level must
    let out2 = Command::new(exe)
        .args([
            "body",
            "--target",
            good_fixture.to_str().unwrap(),
            "--level",
            "must",
            "--json-report",
            report2_path.to_str().unwrap(),
        ])
        .output()
        .expect("run conformance 2");
    assert!(out2.status.success());

    let bytes1 = fs::read(&report1_path).expect("read report1");
    let bytes2 = fs::read(&report2_path).expect("read report2");
    assert_eq!(
        bytes1, bytes2,
        "compatibility reports must be byte-identical"
    );

    let parsed: Value = serde_json::from_slice(&bytes1).expect("parse report1");
    assert_eq!(parsed["schema_version"], "1.0");
    assert_eq!(parsed["level"], "must");
    let checks = parsed["checks"].as_array().expect("checks array");
    assert_eq!(checks.len(), 7, "--level must must filter to B-MUST-01..07");
    for check in checks {
        assert!(check["id"].as_str().unwrap().starts_with("B-MUST-"));
        assert!(!check["title"].as_str().unwrap().is_empty());
        assert_eq!(check["status"], "pass", "expected pass for {check:?}");
        assert!(!check["detail"].as_str().unwrap().is_empty());
    }

    // --level should includes B-MUST-01..07 + B-SHOULD-01..03
    let out_should = Command::new(exe)
        .args([
            "body",
            "--target",
            good_fixture.to_str().unwrap(),
            "--level",
            "should",
            "--json-report",
            should_path.to_str().unwrap(),
        ])
        .output()
        .expect("run conformance should");
    assert!(out_should.status.success());
    let parsed_should: Value =
        serde_json::from_slice(&fs::read(&should_path).expect("read should")).expect("parse");
    assert_eq!(parsed_should["level"], "should");
    let should_checks = parsed_should["checks"].as_array().expect("checks");
    assert_eq!(should_checks.len(), 10);
    assert!(
        should_checks
            .iter()
            .any(|c| c["id"] == "B-SHOULD-01" && c["status"] == "pass")
    );
    assert!(
        should_checks
            .iter()
            .any(|c| c["id"] == "B-SHOULD-02" && c["status"] == "pass")
    );
    assert!(
        should_checks
            .iter()
            .any(|c| c["id"] == "B-SHOULD-03" && c["status"] == "pass")
    );

    // --level may includes B-MAY-01 as explicit skip while all MUST+SHOULD pass
    let out_may = Command::new(exe)
        .args([
            "body",
            "--target",
            good_fixture.to_str().unwrap(),
            "--level",
            "may",
            "--json-report",
            may_path.to_str().unwrap(),
        ])
        .output()
        .expect("run conformance may");
    assert!(out_may.status.success());
    let parsed_may: Value =
        serde_json::from_slice(&fs::read(&may_path).expect("read may")).expect("parse");
    let may_checks = parsed_may["checks"].as_array().expect("checks");
    assert_eq!(may_checks.len(), 11);
    assert!(
        may_checks
            .iter()
            .any(|c| c["id"] == "B-MAY-01" && c["status"] == "skip")
    );

    // Incomplete target returning -32601 records skip with reason and exits non-zero at --level must.
    let out_inc = Command::new(exe)
        .args([
            "body",
            "--target",
            discover_only_fixture.to_str().unwrap(),
            "--level",
            "must",
            "--json-report",
            incomplete_path.to_str().unwrap(),
        ])
        .output()
        .expect("run incomplete conformance");
    assert!(!out_inc.status.success());
    let parsed_inc: Value =
        serde_json::from_slice(&fs::read(&incomplete_path).expect("read inc")).expect("parse");
    let inc_checks = parsed_inc["checks"].as_array().expect("checks");
    assert_eq!(inc_checks[0]["id"], "B-MUST-01");
    assert_eq!(inc_checks[0]["status"], "pass");
    assert!(
        inc_checks[1..]
            .iter()
            .all(|c| c["status"] == "skip" && !c["detail"].as_str().unwrap().is_empty())
    );
}

#[test]
fn conformance_runner_fails_broken_body_and_missing_target() {
    let tmp = TempDir::new().expect("tempdir");
    let bad_fixture = write_executable(tmp.path(), "bad_body.sh", unsorted_caps_fixture_script());
    let bad_report = tmp.path().join("bad_report.json");
    let exe = env!("CARGO_BIN_EXE_monoagens-conformance");

    // Broken body returning unsorted capabilities -> exit code 1, B-MUST-01 fail
    let bad_out = Command::new(exe)
        .args([
            "body",
            "--target",
            bad_fixture.to_str().unwrap(),
            "--level",
            "must",
            "--json-report",
            bad_report.to_str().unwrap(),
        ])
        .output()
        .expect("run broken conformance");
    assert!(!bad_out.status.success());
    let parsed: Value =
        serde_json::from_slice(&fs::read(&bad_report).expect("read bad report")).expect("parse");
    assert_eq!(parsed["checks"][0]["id"], "B-MUST-01");
    assert_eq!(parsed["checks"][0]["status"], "fail");

    // Missing target -> non-zero exit naming the target on stderr
    let missing_target = "/nonexistent/monoagens-missing-body-xyz";
    let missing_out = Command::new(exe)
        .args(["body", "--target", missing_target, "--level", "must"])
        .output()
        .expect("run missing conformance");
    assert!(!missing_out.status.success());
    let stderr = String::from_utf8_lossy(&missing_out.stderr);
    assert!(
        stderr.contains(missing_target),
        "stderr should name missing target '{missing_target}', got: {stderr}"
    );
}
