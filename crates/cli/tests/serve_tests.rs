use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use monoagens_cli::{ServeArgs, run_serve_with_provider};
use monoagens_core::{EventKind, Store};
use monoagens_reason::{ScriptedModelProvider, ScriptedReply};
use monoagens_store::{SegmentedStore, StoreConfig};
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::sync::oneshot;

fn monoagens_bin() -> &'static str {
    env!("CARGO_BIN_EXE_monoagens")
}

fn write_mock_stdio_body_script(dir: &Path) -> PathBuf {
    let script = dir.join("mock_body.py");
    let src = r#"#!/usr/bin/env python3
import argparse
import json
import os
import sys
import threading
import time

parser = argparse.ArgumentParser()
parser.add_argument("--body-id", required=True)
parser.add_argument("--cap-id", default="wa.send")
parser.add_argument("--log-file", required=True)
parser.add_argument("--emit-place", default="")
parser.add_argument("--emit-actor", default="alice")
parser.add_argument("--meta-kind", default="")
parser.add_argument("--meta-cmd", default="")
parser.add_argument("--log-env", action="append", default=[])
args = parser.parse_args()

write_lock = threading.Lock()

def log_event(entry):
    with open(args.log_file, "a", encoding="utf-8") as f:
        f.write(json.dumps(entry) + "\n")
        f.flush()

def send_json(obj):
    with write_lock:
        sys.stdout.write(json.dumps(obj) + "\n")
        sys.stdout.flush()

def emit_dynamic_percept():
    time.sleep(0.08)
    req = {
        "jsonrpc": "2.0",
        "id": 9001,
        "method": "percept/emit",
        "params": {
            "headers": {
                "idempotency_key": f"idem-{args.body_id}-1",
                "protocol_version": {"major": 1, "minor": 0},
            },
            "actor": args.emit_actor,
            "place": args.emit_place,
            "audience": [args.emit_actor, "soul"],
            "timestamp_ms": 1700000001000,
            "payload": list(b"hello dynamic room"),
            "taint": "trusted",
            "addressees": ["soul"],
            "named": ["soul"],
        },
    }
    send_json(req)

emitted = False
for raw in sys.stdin:
    line = raw.strip()
    if not line:
        continue
    msg = json.loads(line)
    if "method" not in msg:
        log_event({"type": "response", "id": msg.get("id"), "result": msg.get("result"), "error": msg.get("error")})
        continue

    method = msg["method"]
    rpc_id = msg.get("id")
    params = msg.get("params", {})

    if method == "body/discover":
        log_event({
            "type": "discover",
            "body_id": args.body_id,
            "plugin_config": os.environ.get("MONOAGENS_PLUGIN_CONFIG"),
            "plugin_state_dir": os.environ.get("MONOAGENS_PLUGIN_STATE_DIR"),
            "body_state_dir": os.environ.get("MONOAGENS_BODY_STATE_DIR"),
            "rbac_enabled": os.environ.get("MONOAGENS_RBAC_ENABLED"),
            "env": {k: os.environ.get(k) for k in args.log_env},
        })
        discover_result = {
            "body_id": args.body_id,
            "protocol_version": {"major": 1, "minor": 0},
            "places": [],
            "capabilities": [{
                "id": args.cap_id,
                "description": "Mock channel send",
                "places": [],
                "flags": {
                    "communicative": True,
                    "identity_changing": False,
                    "irreversible": False,
                },
                "typical_latency_ms": 5,
                "capability_schema_hash": "mock-h1",
                "sensed_effect": None,
            }],
        }
        meta = {}
        if args.meta_kind:
            meta["kind"] = args.meta_kind
        if args.meta_cmd:
            meta["commands"] = [{
                "name": args.meta_cmd,
                "summary": "Mock plugin command",
                "permission": f"plugin.{args.meta_cmd}",
                "base_tier": "member",
            }]
        if meta:
            discover_result["_meta"] = meta
        send_json({
            "jsonrpc": "2.0",
            "id": rpc_id,
            "result": discover_result,
        })
        if args.emit_place and not emitted:
            emitted = True
            threading.Thread(target=emit_dynamic_percept, daemon=True).start()
    elif method == "action/execute":
        log_event({
            "type": "action_execute",
            "action_id": params.get("action_id"),
            "capability_id": params.get("capability_id"),
            "places": params.get("places"),
            "payload": params.get("payload"),
        })
        send_json({
            "jsonrpc": "2.0",
            "id": rpc_id,
            "result": {
                "accepted": True,
                "receipt": {
                    "action_id": params["action_id"],
                    "capability_id": params["capability_id"],
                    "outcome": "done",
                    "reason_code": None,
                    "detail": "delivered",
                    "termination_reason": "normal",
                    "progress": 1.0,
                    "delivered_units": 1,
                    "h_act": params.get("h_act"),
                    "timestamp_ms": 1700000002000,
                },
            },
        })
    elif method == "body/shutdown":
        log_event({"type": "shutdown", "reason": params.get("reason")})
        send_json({
            "jsonrpc": "2.0",
            "id": rpc_id,
            "result": {"accepted": True},
        })
        break
    else:
        send_json({
            "jsonrpc": "2.0",
            "id": rpc_id,
            "result": {},
        })
"#;
    fs::write(&script, src).expect("write mock_body.py");
    script
}

fn read_mock_log(path: &Path) -> Vec<Value> {
    let Ok(raw) = fs::read_to_string(path) else {
        return Vec::new();
    };
    raw.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str::<Value>(l).expect("valid log line"))
        .collect()
}

fn write_serve_config(dir: &Path, bodies_toml: &str, extra_toml: &str) -> (PathBuf, PathBuf) {
    let cfg_path = dir.join("monoagens.toml");
    let data_dir = dir.join("data");
    fs::create_dir_all(&data_dir).expect("mkdir data");
    let toml = format!(
        "version = 1\ndata_dir = \"{}\"\n\n[soul]\nname = \"MonoServeTest\"\nseed = 1\n\n[model]\nprovider = \"scripted\"\nmodel = \"scripted-test\"\ndaily_token_budget = 10000\n\n[valve]\nmode = \"enforcing\"\nmax_chain_wakes = 8\n\n{extra_toml}\n{bodies_toml}\n",
        data_dir.display()
    );
    fs::write(&cfg_path, toml).expect("write config");
    (cfg_path, data_dir)
}

#[test]
fn cli_serve_rejects_empty_bodies_without_allow_empty() {
    let tmp = TempDir::new().expect("tempdir");
    let (cfg_path, _data_dir) = write_serve_config(tmp.path(), "", "");

    let out = Command::new(monoagens_bin())
        .args(["--config", cfg_path.to_str().unwrap(), "serve"])
        .output()
        .expect("run serve");

    assert!(!out.status.success(), "serve without [[bodies]] must fail");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("--allow-empty"), "stderr={stderr}");
}

#[test]
fn cli_serve_allow_empty_starts_and_exits_cleanly() {
    let tmp = TempDir::new().expect("tempdir");
    let (cfg_path, _data_dir) = write_serve_config(tmp.path(), "", "");

    let out = Command::new(monoagens_bin())
        .args([
            "--config",
            cfg_path.to_str().unwrap(),
            "--json",
            "serve",
            "--allow-empty",
            "--exit-after-ms",
            "50",
        ])
        .output()
        .expect("run serve --allow-empty");

    assert!(
        out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let parsed: Value = serde_json::from_slice(&out.stdout).expect("pure JSON stdout");
    assert_eq!(parsed["status"], "running");
    assert_eq!(parsed["connected_count"], 0);
    assert_eq!(parsed["failed_count"], 0);
    assert_eq!(parsed["bodies"].as_array().unwrap().len(), 0);
}

#[test]
fn cli_serve_unknown_body_filter_exits_nonzero() {
    let tmp = TempDir::new().expect("tempdir");
    let script = write_mock_stdio_body_script(tmp.path());
    let log_file = tmp.path().join("body.log");
    let bodies_toml = format!(
        "[[bodies]]\nid = \"valid-wa\"\ntransport = \"stdio\"\ncommand = \"python3\"\nargs = [\"{}\", \"--body-id\", \"wa-mock\", \"--log-file\", \"{}\"]\n",
        script.display(),
        log_file.display()
    );
    let (cfg_path, _data_dir) = write_serve_config(tmp.path(), &bodies_toml, "");

    let out = Command::new(monoagens_bin())
        .args([
            "--config",
            cfg_path.to_str().unwrap(),
            "serve",
            "--body",
            "missing-body",
        ])
        .output()
        .expect("run serve unknown filter");

    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("unknown body id"), "stderr={stderr}");
}

#[test]
fn cli_serve_isolates_broken_body_and_runs_valid_stdio_body_with_dynamic_percept() {
    let tmp = TempDir::new().expect("tempdir");
    let script = write_mock_stdio_body_script(tmp.path());
    let log_file = tmp.path().join("wa_body.jsonl");

    let extra_toml = r#"
[[person]]
actor_id = "per_owner"
owner = true
accounts = [
  { body_id = "wa-mock-body", account_id = "alice" },
]
"#;
    let bodies_toml = format!(
        r#"
[[bodies]]
id = "broken-wa"
transport = "stdio"
command = "/nonexistent/monoagens-body-bin"
args = []

[[bodies]]
id = "valid-wa"
transport = "stdio"
command = "python3"
args = ["{}", "--body-id", "wa-mock-body", "--log-file", "{}", "--emit-place", "wa:group:120363@g.us", "--emit-actor", "alice"]
"#,
        script.display(),
        log_file.display(),
    );

    let (cfg_path, data_dir) = write_serve_config(tmp.path(), &bodies_toml, extra_toml);

    let out = Command::new(monoagens_bin())
        .args([
            "--config",
            cfg_path.to_str().unwrap(),
            "--json",
            "serve",
            "--exit-after-ms",
            "500",
        ])
        .output()
        .expect("run monoagens serve");

    assert!(
        out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );

    let report: Value = serde_json::from_slice(&out.stdout).expect("pure JSON stdout");
    assert_eq!(report["status"], "running");
    assert_eq!(report["connected_count"], 1);
    assert_eq!(report["failed_count"], 1);
    let bodies = report["bodies"].as_array().expect("bodies array");
    assert_eq!(bodies.len(), 2);
    assert_eq!(bodies[0]["config_id"], "broken-wa");
    assert_eq!(bodies[0]["status"], "failed");
    assert!(bodies[0]["error"].as_str().is_some());
    assert_eq!(bodies[1]["config_id"], "valid-wa");
    assert_eq!(bodies[1]["status"], "connected");
    assert_eq!(bodies[1]["body_id"], "wa-mock-body");

    let mock_events = read_mock_log(&log_file);
    assert!(
        mock_events.iter().any(|e| e["type"] == "discover"),
        "expected discover in {mock_events:?}"
    );
    assert!(
        mock_events.iter().any(|e| {
            e["type"] == "response"
                && e["id"] == 9001
                && e["result"]["accepted"] == json!(true)
        }),
        "expected accepted percept/emit response in {mock_events:?}"
    );
    assert!(
        mock_events.iter().any(|e| e["type"] == "shutdown"),
        "expected body/shutdown in {mock_events:?}"
    );

    let store = SegmentedStore::open(StoreConfig::new(data_dir)).expect("open store");
    let events = store.read_in_order(None, usize::MAX).expect("read events");
    assert!(
        events
            .iter()
            .any(|ev| ev.kind == EventKind::Percept && ev.place.as_str() == "wa:group:120363@g.us"),
        "expected dynamic place percept persisted in store"
    );
}

#[test]
fn cli_serve_body_filter_connects_only_selected_body() {
    let tmp = TempDir::new().expect("tempdir");
    let script = write_mock_stdio_body_script(tmp.path());
    let log_a = tmp.path().join("a.jsonl");
    let log_b = tmp.path().join("b.jsonl");

    let bodies_toml = format!(
        r#"
[[bodies]]
id = "body-a"
transport = "stdio"
command = "python3"
args = ["{}", "--body-id", "a-mock", "--log-file", "{}"]

[[bodies]]
id = "body-b"
transport = "stdio"
command = "python3"
args = ["{}", "--body-id", "b-mock", "--log-file", "{}"]
"#,
        script.display(),
        log_a.display(),
        script.display(),
        log_b.display(),
    );
    let (cfg_path, _data_dir) = write_serve_config(tmp.path(), &bodies_toml, "");

    let out = Command::new(monoagens_bin())
        .args([
            "--config",
            cfg_path.to_str().unwrap(),
            "--json",
            "serve",
            "--body",
            "body-b",
            "--exit-after-ms",
            "150",
        ])
        .output()
        .expect("run serve --body body-b");

    assert!(
        out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let report: Value = serde_json::from_slice(&out.stdout).expect("json");
    assert_eq!(report["connected_count"], 1);
    assert_eq!(report["failed_count"], 0);
    let bodies = report["bodies"].as_array().unwrap();
    assert_eq!(bodies.len(), 1);
    assert_eq!(bodies[0]["config_id"], "body-b");
    assert_eq!(bodies[0]["body_id"], "b-mock");

    assert!(read_mock_log(&log_a).is_empty(), "body-a must not be started");
    assert!(
        read_mock_log(&log_b)
            .iter()
            .any(|e| e["type"] == "shutdown"),
        "body-b must receive shutdown"
    );
}

#[cfg(unix)]
#[test]
fn cli_serve_sigterm_triggers_clean_body_shutdown() {
    let tmp = TempDir::new().expect("tempdir");
    let script = write_mock_stdio_body_script(tmp.path());
    let log_file = tmp.path().join("sigterm_body.jsonl");

    let bodies_toml = format!(
        r#"
[[bodies]]
id = "valid-wa"
transport = "stdio"
command = "python3"
args = ["{}", "--body-id", "wa-sigterm", "--log-file", "{}"]
"#,
        script.display(),
        log_file.display(),
    );
    let (cfg_path, _data_dir) = write_serve_config(tmp.path(), &bodies_toml, "");

    let mut child = Command::new(monoagens_bin())
        .args(["--config", cfg_path.to_str().unwrap(), "serve"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn monoagens serve");

    let stdout = child.stdout.take().expect("stdout");
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    reader.read_line(&mut line).expect("read startup line");
    assert!(
        line.contains("monoagens serve running"),
        "unexpected startup line: {line}"
    );

    let pid = child.id().to_string();
    let kill_status = Command::new("kill")
        .args(["-TERM", &pid])
        .status()
        .expect("send SIGTERM");
    assert!(kill_status.success());

    let status = child.wait().expect("wait serve exit");
    assert!(status.success(), "serve should exit 0 on SIGTERM: {status:?}");

    let mock_events = read_mock_log(&log_file);
    assert!(
        mock_events.iter().any(|e| e["type"] == "shutdown"),
        "SIGTERM must trigger body/shutdown on connected body: {mock_events:?}"
    );
}

#[test]
fn cli_serve_routes_action_back_to_dynamically_claimed_place_end_to_end() {
    let tmp = TempDir::new().expect("tempdir");
    let script = write_mock_stdio_body_script(tmp.path());
    let log_file = tmp.path().join("e2e_body.jsonl");

    let extra_toml = r#"
[[person]]
actor_id = "per_owner"
owner = true
accounts = [
  { body_id = "wa-e2e-body", account_id = "alice" },
]
"#;
    let bodies_toml = format!(
        r#"
[[bodies]]
id = "wa-e2e"
transport = "stdio"
command = "python3"
args = ["{}", "--body-id", "wa-e2e-body", "--cap-id", "wa.send", "--log-file", "{}", "--emit-place", "wa:group:120363@g.us", "--emit-actor", "alice"]
"#,
        script.display(),
        log_file.display(),
    );
    let (cfg_path, _data_dir) = write_serve_config(tmp.path(), &bodies_toml, extra_toml);

    let act_reply = json!({
        "protocol_version": { "major": 1, "minor": 0 },
        "acts": [
            {
                "act": "act",
                "capability_id": "wa.send",
                "places": ["wa:group:120363@g.us"],
                "people": ["per_owner"],
                "payload": b"Hello dynamic WhatsApp group!".to_vec(),
                "motivation": 4,
                "confidence_0_20": 16,
                "used_ids": [],
                "expected": null,
                "domain": null
            }
        ]
    })
    .to_string();
    let provider = Arc::new(ScriptedModelProvider::from_replies([ScriptedReply::text(
        act_reply,
    )]));

    let (tx, rx) = oneshot::channel();
    let log_poll = log_file.clone();
    let watcher = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            let entries = read_mock_log(&log_poll);
            if entries.iter().any(|e| e["type"] == "action_execute") {
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
        let _ = tx.send(());
    });

    let mut out = Vec::new();
    let mut err = Vec::new();
    let code = run_serve_with_provider(
        Some(&cfg_path),
        true,
        ServeArgs {
            exit_after_ms: Some(4_000),
            ..ServeArgs::default()
        },
        Some(provider),
        Some(rx),
        &mut out,
        &mut err,
    )
    .expect("run_serve_with_provider");
    watcher.join().expect("watcher join");

    assert_eq!(code, 0, "stderr={}", String::from_utf8_lossy(&err));
    let report: Value = serde_json::from_slice(&out).expect("json report");
    assert_eq!(report["connected_count"], 1);

    let events = read_mock_log(&log_file);
    let exec = events
        .iter()
        .find(|e| e["type"] == "action_execute")
        .unwrap_or_else(|| panic!("expected action_execute in {events:?}"));
    assert_eq!(exec["capability_id"], "wa.send");
    assert_eq!(exec["places"], json!(["wa:group:120363@g.us"]));
    assert!(
        events.iter().any(|e| e["type"] == "shutdown"),
        "expected clean body/shutdown in {events:?}"
    );
}

#[test]
fn cli_serve_unified_plugins_rbac_env_injection_and_dependency_skip() {
    let tmp = TempDir::new().expect("tempdir");
    let script = write_mock_stdio_body_script(tmp.path());
    let tg_log = tmp.path().join("tg_plugin.jsonl");
    let cap_log = tmp.path().join("cap_plugin.jsonl");
    let skipped_log = tmp.path().join("skipped_plugin.jsonl");

    let extra_toml = format!(
        r#"
[roles]
fallback = "guest"

[roles.guest]
base = "guest"
allow = ["command.help", "identity.whoami"]

[roles.member]
base = "member"
allow = ["chat.*", "plugin.search"]

[roles.owner]
base = "owner"
allow = ["*"]

[[person]]
actor_id = "per_owner"
owner = true
roles = ["owner"]
accounts = [
  {{ body_id = "tg-mock", account_id = "tg:user:1001" }},
]

[plugins.tg-adapter]
kind = "adapter"
transport = "stdio"
command = "python3"
args = ["{}", "--body-id", "tg-mock", "--cap-id", "chat.speak", "--log-file", "{}", "--meta-kind", "adapter"]

[plugins.cap-search]
kind = "capability"
dependencies = ["tg-adapter", "rbac"]
transport = "stdio"
command = "python3"
args = ["{}", "--body-id", "cap-search-mock", "--cap-id", "search.web", "--log-file", "{}", "--meta-kind", "capability", "--meta-cmd", "search"]

[plugins.cap-missing-dep]
kind = "capability"
dependencies = ["nonexistent-dep"]
transport = "stdio"
command = "python3"
args = ["{}", "--body-id", "cap-missing-mock", "--cap-id", "missing.run", "--log-file", "{}"]
"#,
        script.display(),
        tg_log.display(),
        script.display(),
        cap_log.display(),
        script.display(),
        skipped_log.display(),
    );

    let (cfg_path, data_dir) = write_serve_config(tmp.path(), "", &extra_toml);

    let out = Command::new(monoagens_bin())
        .args([
            "--config",
            cfg_path.to_str().unwrap(),
            "--json",
            "serve",
            "--exit-after-ms",
            "300",
        ])
        .output()
        .expect("run monoagens serve with [plugins.*] and [roles]");

    assert!(
        out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );

    let report: Value = serde_json::from_slice(&out.stdout).expect("pure JSON stdout");
    assert_eq!(report["status"], "running");
    assert_eq!(report["connected_count"], 2);
    assert_eq!(report["failed_count"], 0);
    assert_eq!(report["skipped_count"], 1);
    assert_eq!(report["rbac_enabled"], true);

    let cmds: Vec<&str> = report["indexed_commands"]
        .as_array()
        .expect("indexed_commands array")
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert!(cmds.contains(&"help"), "expected 'help' in {cmds:?}");
    assert!(cmds.contains(&"whoami"), "expected 'whoami' in {cmds:?}");
    assert!(cmds.contains(&"roles"), "expected 'roles' in {cmds:?}");
    assert!(!cmds.contains(&"approve"), "unexpected 'approve' in {cmds:?}");
    assert!(!cmds.contains(&"deny"), "unexpected 'deny' in {cmds:?}");
    assert!(cmds.contains(&"search"), "expected 'search' in {cmds:?}");

    let entries = report["bodies"].as_array().expect("bodies array");
    assert_eq!(entries.len(), 3);

    let find_entry = |id: &str| -> &Value {
        entries
            .iter()
            .find(|e| e["config_id"] == id)
            .unwrap_or_else(|| panic!("missing '{id}' in {entries:?}"))
    };
    let tg_entry = find_entry("tg-adapter");
    assert_eq!(tg_entry["kind"], "adapter");
    assert_eq!(tg_entry["status"], "connected");
    assert_eq!(tg_entry["body_id"], "tg-mock");

    let cap_entry = find_entry("cap-search");
    assert_eq!(cap_entry["kind"], "capability");
    assert_eq!(cap_entry["status"], "connected");
    assert_eq!(cap_entry["body_id"], "cap-search-mock");

    let skipped_entry = find_entry("cap-missing-dep");
    assert_eq!(skipped_entry["kind"], "capability");
    assert_eq!(skipped_entry["status"], "skipped");
    assert!(
        skipped_entry["error"]
            .as_str()
            .is_some_and(|s| s.contains("nonexistent-dep")),
        "expected missing dependency reason in {skipped_entry:?}"
    );

    assert!(
        read_mock_log(&skipped_log).is_empty(),
        "skipped plugin must never be spawned"
    );

    for (plugin_id, log_path) in [("tg-adapter", &tg_log), ("cap-search", &cap_log)] {
        let events = read_mock_log(log_path);
        let discover = events
            .iter()
            .find(|e| e["type"] == "discover")
            .unwrap_or_else(|| panic!("expected discover for {plugin_id} in {events:?}"));
        let expected_cfg = tmp
            .path()
            .join("plugins")
            .join(format!("{plugin_id}.toml"));
        let expected_state = data_dir.join("plugins").join(plugin_id);
        assert_eq!(
            discover["plugin_config"],
            json!(expected_cfg.display().to_string())
        );
        assert_eq!(
            discover["plugin_state_dir"],
            json!(expected_state.display().to_string())
        );
        assert_eq!(
            discover["body_state_dir"],
            json!(expected_state.display().to_string())
        );
        assert_eq!(discover["rbac_enabled"], json!("1"));
        assert!(
            events.iter().any(|e| e["type"] == "shutdown"),
            "expected clean shutdown for {plugin_id}: {events:?}"
        );
    }

    // Verify `--plugin` CLI flag selects a single `[plugins.<id>]` entry identically to `--body`.
    let filtered_log = tmp.path().join("tg_filtered.jsonl");
    let single_out = Command::new(monoagens_bin())
        .args([
            "--config",
            cfg_path.to_str().unwrap(),
            "--json",
            "serve",
            "--plugin",
            "tg-adapter",
            "--exit-after-ms",
            "150",
        ])
        .output()
        .expect("run serve --plugin tg-adapter");
    assert!(
        single_out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&single_out.stderr)
    );
    let _ = filtered_log;
    let single_report: Value = serde_json::from_slice(&single_out.stdout).expect("filtered json");
    assert_eq!(single_report["connected_count"], 1);
    assert_eq!(single_report["skipped_count"], 0);
    assert_eq!(single_report["bodies"].as_array().unwrap().len(), 1);
    assert_eq!(single_report["bodies"][0]["config_id"], "tg-adapter");
}

#[test]
fn cli_serve_forwards_secrets_toml_env_into_spawned_plugins_without_shell_export() {
    let tmp = TempDir::new().expect("tempdir");
    let script = write_mock_stdio_body_script(tmp.path());
    let log_file = tmp.path().join("env_plugin.jsonl");

    let extra_toml = format!(
        r#"
[plugins.env-check]
kind = "adapter"
transport = "stdio"
command = "python3"
args = ["{}", "--body-id", "env-mock", "--log-file", "{}", "--log-env", "TEST_PLUGIN_TOKEN", "--log-env", "TEST_OVERRIDE_VAR"]
"#,
        script.display(),
        log_file.display(),
    );
    let (cfg_path, _data_dir) = write_serve_config(tmp.path(), "", &extra_toml);

    let secrets_path = tmp.path().join("secrets.toml");
    let secret_val = "tg-secret-from-secrets-toml-998877";
    fs::write(
        &secrets_path,
        format!(
            "[env]\nTEST_PLUGIN_TOKEN = \"{secret_val}\"\nTEST_OVERRIDE_VAR = \"from-secrets-toml\"\n"
        ),
    )
    .expect("write secrets.toml");

    let out = Command::new(monoagens_bin())
        .env_remove("TEST_PLUGIN_TOKEN")
        .env("TEST_OVERRIDE_VAR", "from-process-env")
        .args([
            "--config",
            cfg_path.to_str().unwrap(),
            "--json",
            "serve",
            "--exit-after-ms",
            "250",
        ])
        .output()
        .expect("run serve with [env] secrets");

    assert!(
        out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout_str = String::from_utf8_lossy(&out.stdout);
    let stderr_str = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stdout_str.contains(secret_val) && !stderr_str.contains(secret_val),
        "serve output must never leak [env] secret value"
    );

    let report: Value = serde_json::from_slice(&out.stdout).expect("json report");
    assert_eq!(report["connected_count"], 1);
    assert_eq!(report["failed_count"], 0);

    let events = read_mock_log(&log_file);
    let discover = events
        .iter()
        .find(|e| e["type"] == "discover")
        .unwrap_or_else(|| panic!("expected discover event in {events:?}"));
    assert_eq!(discover["env"]["TEST_PLUGIN_TOKEN"], json!(secret_val));
    assert_eq!(
        discover["env"]["TEST_OVERRIDE_VAR"],
        json!("from-process-env")
    );
}

#[test]
fn cli_serve_refuses_a_second_host_on_the_same_data_dir_naming_the_holder_pid() {
    let tmp = TempDir::new().expect("tempdir");
    let (cfg_path, _data_dir) = write_serve_config(tmp.path(), "", "");

    let mut first = Command::new(monoagens_bin())
        .args([
            "--config",
            cfg_path.to_str().unwrap(),
            "serve",
            "--allow-empty",
            "--exit-after-ms",
            "20000",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn first serve");
    let mut line = String::new();
    BufReader::new(first.stdout.as_mut().expect("stdout"))
        .read_line(&mut line)
        .expect("first serve startup line");
    assert!(line.contains("monoagens serve running"), "line={line}");

    let second = Command::new(monoagens_bin())
        .args(["--config", cfg_path.to_str().unwrap(), "serve", "--allow-empty"])
        .output()
        .expect("run second serve");
    assert_eq!(second.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&second.stderr);
    assert!(
        stderr.contains(&format!("pid {}", first.id())),
        "stderr must name the holder pid {}: {stderr}",
        first.id()
    );
    assert!(stderr.contains("Attach"), "stderr must carry a hint: {stderr}");

    // The lock dies with its holder, and a new serve boots normally.
    first.kill().expect("kill first");
    first.wait().expect("reap first");
    let third = Command::new(monoagens_bin())
        .args([
            "--config",
            cfg_path.to_str().unwrap(),
            "serve",
            "--allow-empty",
            "--exit-after-ms",
            "50",
        ])
        .output()
        .expect("run third serve");
    assert!(
        third.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&third.stderr)
    );
}
