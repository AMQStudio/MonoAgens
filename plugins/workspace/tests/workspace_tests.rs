#![forbid(unsafe_code)]

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::symlink;
use std::process::{Command, Stdio};

use monoagens_body_kit::PersistentBodyRuntime;
use monoagens_body_sdk::{
    ActionExecuteParams, ActionExecuteResult, ActionId, ActorId, Audience, BodyDiscoverParams,
    BodyHandler, CapabilityId, OpaquePayload, PlaceId, ReceiptOutcome, Waist2ErrorCode,
    Waist2Headers,
};
use plugin_workspace::config::WorkspaceConfig;
use plugin_workspace::{DEFAULT_PLACE_ID, WorkspaceBody};
use serde_json::{Value, json};
use tempfile::TempDir;

fn exec_cap(
    body: &mut WorkspaceBody,
    action_id: &str,
    cap: &str,
    actor: &str,
    payload: Value,
) -> ActionExecuteResult {
    body.execute(ActionExecuteParams {
        headers: Waist2Headers::new(format!("idem-{action_id}")),
        action_id: ActionId::new(action_id),
        capability_id: CapabilityId::new(cap),
        places: vec![PlaceId::new(DEFAULT_PLACE_ID)],
        audience: Audience::new([ActorId::new(actor)]),
        payload: OpaquePayload::from_bytes(payload.to_string().into_bytes()),
        timeout_ms: Some(10_000),
        h_act: None,
    })
    .expect("execute should not fail with RPC error")
}

fn parse_detail(res: &ActionExecuteResult) -> Value {
    let receipt = res.receipt.as_ref().expect("receipt expected");
    let detail = receipt.detail.as_deref().expect("detail expected");
    serde_json::from_str(detail).expect("detail should be valid JSON")
}

#[test]
fn discover_lists_all_six_capabilities_sorted_with_hashes() {
    let tmp = TempDir::new().unwrap();
    let mut body = WorkspaceBody::default().with_workspace_root(tmp.path());
    let res = body
        .discover(BodyDiscoverParams {
            headers: Waist2Headers::new("disc-1"),
            body_id: None,
        })
        .unwrap();

    assert_eq!(res.body_id.as_str(), "plugin-workspace");
    assert_eq!(res.places, vec![PlaceId::new("cap:workspace")]);
    let ids: Vec<&str> = res.capabilities.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(
        ids,
        vec![
            "workspace.edit",
            "workspace.glob",
            "workspace.grep",
            "workspace.read",
            "workspace.run",
            "workspace.write",
        ]
    );
    for cap in &res.capabilities {
        assert!(cap.capability_schema_hash.as_str().starts_with("sha256:"));
        assert_eq!(cap.places, vec![PlaceId::new("cap:workspace")]);
    }
    let run_cap = res
        .capabilities
        .iter()
        .find(|c| c.id.as_str() == "workspace.run")
        .unwrap();
    assert!(run_cap.flags.irreversible);
}

#[test]
fn unknown_capability_and_undeclared_audience_errors() {
    let tmp = TempDir::new().unwrap();
    let mut body = WorkspaceBody::default().with_workspace_root(tmp.path());

    let err = body
        .execute(ActionExecuteParams {
            headers: Waist2Headers::new("unk-1"),
            action_id: ActionId::new("act-unk"),
            capability_id: CapabilityId::new("workspace.nope"),
            places: vec![PlaceId::new(DEFAULT_PLACE_ID)],
            audience: Audience::new([ActorId::new("alice")]),
            payload: OpaquePayload::from_bytes(b"{}".to_vec()),
            timeout_ms: None,
            h_act: None,
        })
        .unwrap_err();
    assert_eq!(err.code, Waist2ErrorCode::UnknownCapability.code());

    let err = body
        .execute(ActionExecuteParams {
            headers: Waist2Headers::new("aud-1"),
            action_id: ActionId::new("act-aud"),
            capability_id: CapabilityId::new("workspace.read"),
            places: vec![PlaceId::new(DEFAULT_PLACE_ID)],
            audience: Audience::empty(),
            payload: OpaquePayload::from_bytes(b"{}".to_vec()),
            timeout_ms: None,
            h_act: None,
        })
        .unwrap_err();
    assert_eq!(err.code, Waist2ErrorCode::UndeclaredAudience.code());
}

#[test]
fn write_read_edit_glob_grep_roundtrip() {
    let tmp = TempDir::new().unwrap();
    let mut body = WorkspaceBody::default().with_workspace_root(tmp.path());

    // Write nested files
    let w1 = exec_cap(
        &mut body,
        "w1",
        "workspace.write",
        "alice",
        json!({ "path": "src/lib.rs", "text": "fn alpha() {}\nfn beta() {}\nfn alpha_two() {}\n" }),
    );
    assert_eq!(w1.receipt.as_ref().unwrap().outcome, ReceiptOutcome::Done);
    assert_eq!(parse_detail(&w1)["path"], "src/lib.rs");

    let w2 = exec_cap(
        &mut body,
        "w2",
        "workspace.write",
        "alice",
        json!({ "path": "README.md", "text": "hello world\nline 2\nline 3\nline 4\n" }),
    );
    assert_eq!(w2.receipt.as_ref().unwrap().outcome, ReceiptOutcome::Done);

    // Read with offset & limit
    let r1 = exec_cap(
        &mut body,
        "r1",
        "workspace.read",
        "alice",
        json!({ "path": "README.md", "offset": 1, "limit": 2 }),
    );
    assert_eq!(r1.receipt.as_ref().unwrap().outcome, ReceiptOutcome::Done);
    let r1_json = parse_detail(&r1);
    assert_eq!(r1_json["total_lines"], 4);
    assert_eq!(r1_json["offset"], 1);
    assert_eq!(r1_json["text"], "line 2\nline 3");

    // Edit non-unique fails without replace_all
    let e_fail = exec_cap(
        &mut body,
        "e-fail",
        "workspace.edit",
        "alice",
        json!({ "path": "src/lib.rs", "old": "alpha", "new": "gamma" }),
    );
    assert_eq!(
        e_fail.receipt.as_ref().unwrap().outcome,
        ReceiptOutcome::Failed
    );

    // Edit with replace_all succeeds
    let e_all = exec_cap(
        &mut body,
        "e-all",
        "workspace.edit",
        "alice",
        json!({ "path": "src/lib.rs", "old": "alpha", "new": "gamma", "replace_all": true }),
    );
    assert_eq!(
        e_all.receipt.as_ref().unwrap().outcome,
        ReceiptOutcome::Done
    );
    assert_eq!(parse_detail(&e_all)["replacements"], 2);

    // Edit single occurrence without replace_all
    let e_one = exec_cap(
        &mut body,
        "e-one",
        "workspace.edit",
        "alice",
        json!({ "path": "src/lib.rs", "old": "beta", "new": "delta" }),
    );
    assert_eq!(
        e_one.receipt.as_ref().unwrap().outcome,
        ReceiptOutcome::Done
    );
    assert_eq!(parse_detail(&e_one)["replacements"], 1);

    // Glob recursive **/*.rs and ?
    let g1 = exec_cap(
        &mut body,
        "g1",
        "workspace.glob",
        "alice",
        json!({ "pattern": "**/*.rs" }),
    );
    assert_eq!(parse_detail(&g1)["matches"], json!(["src/lib.rs"]));

    let g2 = exec_cap(
        &mut body,
        "g2",
        "workspace.glob",
        "alice",
        json!({ "pattern": "src/???.rs" }),
    );
    assert_eq!(parse_detail(&g2)["matches"], json!(["src/lib.rs"]));

    // Grep matches, count, files
    let gr_matches = exec_cap(
        &mut body,
        "gr1",
        "workspace.grep",
        "alice",
        json!({ "pattern": "gamma", "glob": "*.rs", "mode": "matches" }),
    );
    let gr_m = parse_detail(&gr_matches);
    assert_eq!(gr_m["count"], 2);
    assert_eq!(gr_m["matches"][0]["path"], "src/lib.rs");
    assert_eq!(gr_m["matches"][0]["line"], 1);

    let gr_count = exec_cap(
        &mut body,
        "gr2",
        "workspace.grep",
        "alice",
        json!({ "pattern": "gamma|hello", "mode": "count" }),
    );
    let gr_c = parse_detail(&gr_count);
    assert_eq!(gr_c["total"], 3);
    assert_eq!(gr_c["counts"]["src/lib.rs"], 2);
    assert_eq!(gr_c["counts"]["README.md"], 1);

    let gr_files = exec_cap(
        &mut body,
        "gr3",
        "workspace.grep",
        "alice",
        json!({ "pattern": "delta", "mode": "files" }),
    );
    let gr_f = parse_detail(&gr_files);
    assert_eq!(gr_f["files"], json!(["src/lib.rs"]));
}

#[test]
fn rejects_path_traversal_absolute_paths_and_symlink_escape() {
    let tmp = TempDir::new().unwrap();
    let ws_root = tmp.path().join("workspaces");
    let outside_dir = tmp.path().join("outside");
    fs::create_dir_all(&outside_dir).unwrap();
    let outside_secret = outside_dir.join("secret.txt");
    fs::write(&outside_secret, "TOP_SECRET").unwrap();

    let mut body = WorkspaceBody::default().with_workspace_root(&ws_root);

    // Ensure alice's dir exists first
    let init = exec_cap(
        &mut body,
        "init",
        "workspace.write",
        "alice",
        json!({ "path": "ok.txt", "text": "ok" }),
    );
    assert_eq!(init.receipt.as_ref().unwrap().outcome, ReceiptOutcome::Done);

    // Traversal & absolute paths
    for bad_path in [
        "../etc/passwd",
        "/etc/passwd",
        "sub/../../etc/passwd",
        "..\\etc\\passwd",
        "foo\0bar",
    ] {
        let r = exec_cap(
            &mut body,
            "bad-r",
            "workspace.read",
            "alice",
            json!({ "path": bad_path }),
        );
        assert_eq!(
            r.receipt.as_ref().unwrap().outcome,
            ReceiptOutcome::Refused,
            "expected refusal for read {bad_path:?}"
        );

        let w = exec_cap(
            &mut body,
            "bad-w",
            "workspace.write",
            "alice",
            json!({ "path": bad_path, "text": "pwn" }),
        );
        assert_eq!(
            w.receipt.as_ref().unwrap().outcome,
            ReceiptOutcome::Refused,
            "expected refusal for write {bad_path:?}"
        );
    }

    // Symlink escape (file symlink and directory symlink and dangling symlink)
    let alice_dir = ws_root.join("alice");
    symlink(&outside_secret, alice_dir.join("leak.txt")).unwrap();
    symlink(&outside_dir, alice_dir.join("leak_dir")).unwrap();
    symlink(outside_dir.join("new_evil.txt"), alice_dir.join("dangling.txt")).unwrap();

    for sym_path in ["leak.txt", "leak_dir/secret.txt", "dangling.txt"] {
        let r = exec_cap(
            &mut body,
            "sym-r",
            "workspace.read",
            "alice",
            json!({ "path": sym_path }),
        );
        assert_eq!(
            r.receipt.as_ref().unwrap().outcome,
            ReceiptOutcome::Refused,
            "expected symlink refusal for read {sym_path}"
        );
        let w = exec_cap(
            &mut body,
            "sym-w",
            "workspace.write",
            "alice",
            json!({ "path": sym_path, "text": "overwrite" }),
        );
        assert_eq!(
            w.receipt.as_ref().unwrap().outcome,
            ReceiptOutcome::Refused,
            "expected symlink refusal for write {sym_path}"
        );
    }
    assert_eq!(fs::read_to_string(&outside_secret).unwrap(), "TOP_SECRET");
    assert!(!outside_dir.join("new_evil.txt").exists());
}

#[test]
fn binary_file_rejected_by_workspace_read() {
    let tmp = TempDir::new().unwrap();
    let ws_root = tmp.path().join("workspaces");
    let mut body = WorkspaceBody::default().with_workspace_root(&ws_root);

    let init = exec_cap(
        &mut body,
        "init",
        "workspace.write",
        "alice",
        json!({ "path": "keep.txt", "text": "hi" }),
    );
    assert_eq!(init.receipt.as_ref().unwrap().outcome, ReceiptOutcome::Done);

    fs::write(ws_root.join("alice").join("bin.dat"), b"ELF\x00\x01\x02").unwrap();
    let r = exec_cap(
        &mut body,
        "bin-r",
        "workspace.read",
        "alice",
        json!({ "path": "bin.dat" }),
    );
    let rcpt = r.receipt.as_ref().unwrap();
    assert_eq!(rcpt.outcome, ReceiptOutcome::Failed);
    assert_eq!(rcpt.detail.as_deref(), Some("file is not UTF-8 text"));
}

#[test]
fn per_actor_isolation_prevents_cross_actor_access() {
    let tmp = TempDir::new().unwrap();
    let mut body = WorkspaceBody::default().with_workspace_root(tmp.path());

    let w_a = exec_cap(
        &mut body,
        "wa",
        "workspace.write",
        "tg:user:100",
        json!({ "path": "diary.txt", "text": "alice private secret" }),
    );
    assert_eq!(w_a.receipt.as_ref().unwrap().outcome, ReceiptOutcome::Done);

    // Actor B cannot read Actor A's file
    let r_b = exec_cap(
        &mut body,
        "rb",
        "workspace.read",
        "tg:user:200",
        json!({ "path": "diary.txt" }),
    );
    assert_eq!(r_b.receipt.as_ref().unwrap().outcome, ReceiptOutcome::Failed);

    // Actor B glob/grep sees nothing
    let g_b = exec_cap(
        &mut body,
        "gb",
        "workspace.glob",
        "tg:user:200",
        json!({ "pattern": "**/*" }),
    );
    assert_eq!(parse_detail(&g_b)["matches"], json!([]));

    // Attempted actor spoofing with traversal chars in _who is sanitized safely
    let w_spoof = exec_cap(
        &mut body,
        "ws",
        "workspace.write",
        "bob",
        json!({ "_who": "../tg_user_100", "path": "diary.txt", "text": "hijacked" }),
    );
    assert_eq!(w_spoof.receipt.as_ref().unwrap().outcome, ReceiptOutcome::Done);

    // Actor A's file remains intact
    let r_a = exec_cap(
        &mut body,
        "ra",
        "workspace.read",
        "tg:user:100",
        json!({ "path": "diary.txt" }),
    );
    assert_eq!(parse_detail(&r_a)["text"], "alice private secret");
}

#[test]
fn rbac_role_quotas_and_max_file_bytes_enforced() {
    let tmp = TempDir::new().unwrap();
    let mut cfg = WorkspaceConfig {
        max_file_bytes: 50,
        default_quota_bytes: 60,
        ..Default::default()
    };
    cfg.role_quotas.insert("guest".to_owned(), 0);
    cfg.role_quotas.insert("member".to_owned(), 100);

    let mut body = WorkspaceBody::with_config(cfg)
        .with_workspace_root(tmp.path())
        .with_rbac_enabled(true);

    // Guest has quota 0 -> blocked
    let guest_w = exec_cap(
        &mut body,
        "q1",
        "workspace.write",
        "guest_user",
        json!({ "_role": "guest", "path": "a.txt", "text": "hello" }),
    );
    assert_eq!(
        guest_w.receipt.as_ref().unwrap().outcome,
        ReceiptOutcome::Refused
    );
    assert_eq!(
        guest_w.receipt.as_ref().unwrap().reason_code.as_deref(),
        Some("ERR_QUOTA_EXCEEDED")
    );

    // Member succeeds within 100-byte quota and <= 50-byte max_file_bytes
    let mem_w1 = exec_cap(
        &mut body,
        "q2",
        "workspace.write",
        "member_user",
        json!({ "_role": "member", "path": "a.txt", "text": "1234567890" }),
    );
    assert_eq!(mem_w1.receipt.as_ref().unwrap().outcome, ReceiptOutcome::Done);

    // Exceeding max_file_bytes refused
    let too_big = "x".repeat(51);
    let mem_big = exec_cap(
        &mut body,
        "q3",
        "workspace.write",
        "member_user",
        json!({ "_role": "member", "path": "big.txt", "text": too_big }),
    );
    assert_eq!(
        mem_big.receipt.as_ref().unwrap().outcome,
        ReceiptOutcome::Refused
    );
    assert_eq!(
        mem_big.receipt.as_ref().unwrap().reason_code.as_deref(),
        Some("ERR_FILE_TOO_LARGE")
    );

    // Exceeding cumulative quota refused (10 + 50 + 50 = 110 > 100)
    let chunk = "a".repeat(50);
    let mem_w2 = exec_cap(
        &mut body,
        "q4",
        "workspace.write",
        "member_user",
        json!({ "_role": "member", "path": "b.txt", "text": chunk }),
    );
    assert_eq!(mem_w2.receipt.as_ref().unwrap().outcome, ReceiptOutcome::Done);

    let mem_w3 = exec_cap(
        &mut body,
        "q5",
        "workspace.write",
        "member_user",
        json!({ "_role": "member", "path": "c.txt", "text": chunk }),
    );
    assert_eq!(
        mem_w3.receipt.as_ref().unwrap().outcome,
        ReceiptOutcome::Refused
    );
    assert_eq!(
        mem_w3.receipt.as_ref().unwrap().reason_code.as_deref(),
        Some("ERR_QUOTA_EXCEEDED")
    );
}

#[test]
fn workspace_run_executes_in_sandbox_caps_output_and_kills_on_timeout() {
    let tmp = TempDir::new().unwrap();
    let cfg = WorkspaceConfig {
        timeout_secs: 2,
        max_output_bytes: 64,
        ..Default::default()
    };

    let mut body = WorkspaceBody::with_config(cfg).with_workspace_root(tmp.path());

    // Create sub/hello.txt
    exec_cap(
        &mut body,
        "prep",
        "workspace.write",
        "runner",
        json!({ "path": "sub/hello.txt", "text": "world" }),
    );

    // Run inside cwd="sub" with minimal safe env
    let r1 = exec_cap(
        &mut body,
        "run1",
        "workspace.run",
        "runner",
        json!({ "command": "cat hello.txt && printf '|%s' \"$HOME\"", "cwd": "sub" }),
    );
    assert_eq!(r1.receipt.as_ref().unwrap().outcome, ReceiptOutcome::Done);
    let d1 = parse_detail(&r1);
    assert_eq!(d1["exit_code"], 0);
    assert_eq!(d1["timed_out"], false);
    let stdout = d1["stdout"].as_str().unwrap();
    assert!(stdout.starts_with("world|"));
    assert!(stdout.ends_with("runner"));

    // Output capping
    let r_cap = exec_cap(
        &mut body,
        "run-cap",
        "workspace.run",
        "runner",
        json!({ "command": "printf '%0200d' 0" }),
    );
    let d_cap = parse_detail(&r_cap);
    assert_eq!(d_cap["stdout"].as_str().unwrap().len(), 64);

    // Timeout kills process
    let r_to = exec_cap(
        &mut body,
        "run-to",
        "workspace.run",
        "runner",
        json!({ "command": "echo before_sleep; sleep 10", "timeout": 1 }),
    );
    assert_eq!(r_to.receipt.as_ref().unwrap().outcome, ReceiptOutcome::Done);
    let d_to = parse_detail(&r_to);
    assert_eq!(d_to["timed_out"], true);
}

#[test]
fn subprocess_config_auto_init_reload_and_state_dir_precedence() {
    let tmp = TempDir::new().unwrap();
    let cfg_path = tmp.path().join("cfg").join("workspace.toml");
    let plugin_state = tmp.path().join("plugin_state");
    let body_state = tmp.path().join("body_state");

    let bin = env!("CARGO_BIN_EXE_plugin-workspace");

    // 1. First spawn auto-initializes workspace.toml and uses MONOAGENS_PLUGIN_STATE_DIR
    let discover_req = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "body/discover",
        "params": {
            "headers": { "protocol_version": { "major": 1, "minor": 0 }, "idempotency_key": "k1" }
        }
    });
    let write_req = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "action/execute",
        "params": {
            "headers": { "protocol_version": { "major": 1, "minor": 0 }, "idempotency_key": "k2" },
            "action_id": "act-1",
            "capability_id": "workspace.write",
            "places": ["cap:workspace"],
            "audience": ["per_alice"],
            "payload": serde_json::to_vec(&json!({ "path": "note.txt", "text": "persisted" })).unwrap()
        }
    });

    let mut child = Command::new(bin)
        .env("MONOAGENS_PLUGIN_CONFIG", &cfg_path)
        .env("MONOAGENS_PLUGIN_STATE_DIR", &plugin_state)
        .env("MONOAGENS_BODY_STATE_DIR", &body_state)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    {
        let stdin = child.stdin.as_mut().unwrap();
        writeln!(stdin, "{discover_req}").unwrap();
        writeln!(stdin, "{write_req}").unwrap();
    }
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success());
    assert!(cfg_path.exists(), "workspace.toml should be auto-initialized");
    assert!(
        plugin_state.join("workspaces").join("per_alice").join("note.txt").exists(),
        "MONOAGENS_PLUGIN_STATE_DIR must take precedence over MONOAGENS_BODY_STATE_DIR"
    );
    assert!(!body_state.exists());

    // 2. Modify workspace.toml to disable workspace.run and verify reload on next spawn
    let toml_text = fs::read_to_string(&cfg_path).unwrap();
    fs::write(&cfg_path, toml_text.replace("run_enabled = true", "run_enabled = false")).unwrap();

    let run_req = json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "action/execute",
        "params": {
            "headers": { "protocol_version": { "major": 1, "minor": 0 }, "idempotency_key": "k3" },
            "action_id": "act-2",
            "capability_id": "workspace.run",
            "places": ["cap:workspace"],
            "audience": ["per_alice"],
            "payload": serde_json::to_vec(&json!({ "command": "echo hi" })).unwrap()
        }
    });

    let mut child2 = Command::new(bin)
        .env("MONOAGENS_PLUGIN_CONFIG", &cfg_path)
        .env("MONOAGENS_PLUGIN_STATE_DIR", &plugin_state)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    {
        let mut stdin = child2.stdin.take().unwrap();
        writeln!(stdin, "{run_req}").unwrap();
    }
    let reader = BufReader::new(child2.stdout.take().unwrap());
    let lines: Vec<String> = reader.lines().map(Result::unwrap).collect();
    let status2 = child2.wait().unwrap();
    assert!(status2.success());
    assert_eq!(lines.len(), 1);
    let resp: Value = serde_json::from_str(&lines[0]).unwrap();
    assert_eq!(resp["result"]["receipt"]["outcome"], "refused");
    assert_eq!(resp["result"]["receipt"]["reason_code"], "ERR_RUN_DISABLED");
}

#[test]
fn persistent_runtime_handles_idempotency_and_held_approval() {
    let tmp = TempDir::new().unwrap();
    let mut runtime = PersistentBodyRuntime::new(Some(tmp.path().join("state")));
    let mut body = WorkspaceBody::default().with_workspace_root(tmp.path().join("ws"));

    let req_line = json!({
        "jsonrpc": "2.0",
        "id": 10,
        "method": "action/execute",
        "params": {
            "headers": { "protocol_version": { "major": 1, "minor": 0 }, "idempotency_key": "held-key" },
            "action_id": "act-held",
            "capability_id": "workspace.run",
            "places": ["cap:workspace"],
            "audience": ["per_owner"],
            "payload": b"hold".to_vec(),
            "h_act": "blake3:test"
        }
    })
    .to_string();

    let resp_str = runtime.dispatch_line(&req_line, &mut body).unwrap();
    let resp: Value = serde_json::from_str(&resp_str).unwrap();
    assert_eq!(resp["result"]["receipt"]["outcome"], "held");

    let approve_line = json!({
        "jsonrpc": "2.0",
        "id": 11,
        "method": "approval/resolve",
        "params": {
            "headers": { "protocol_version": { "major": 1, "minor": 0 }, "idempotency_key": "app-1" },
            "action_id": "act-held",
            "h_act": "blake3:test",
            "state_epoch": 1,
            "approved": true,
            "resolver": "per_owner",
            "resolved_at_ms": 1234
        }
    })
    .to_string();
    let app_str = runtime.dispatch_line(&approve_line, &mut body).unwrap();
    let app_resp: Value = serde_json::from_str(&app_str).unwrap();
    assert_eq!(app_resp["result"]["applied"], true);

    // Duplicate approval must return applied: false
    let app_dup = json!({
        "jsonrpc": "2.0",
        "id": 12,
        "method": "approval/resolve",
        "params": {
            "headers": { "protocol_version": { "major": 1, "minor": 0 }, "idempotency_key": "app-2" },
            "action_id": "act-held",
            "h_act": "blake3:test",
            "state_epoch": 1,
            "approved": true,
            "resolver": "per_owner",
            "resolved_at_ms": 1235
        }
    })
    .to_string();
    // Duplicate approval must be rejected with -32003 ERR_INVALID_RECEIPT_TRANSITION
    let dup_str = runtime.dispatch_line(&app_dup, &mut body).unwrap();
    let dup_resp: Value = serde_json::from_str(&dup_str).unwrap();
    assert_eq!(dup_resp["error"]["code"], -32003);
}
