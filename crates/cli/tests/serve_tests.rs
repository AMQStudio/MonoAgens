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
import sys
import threading
import time

parser = argparse.ArgumentParser()
parser.add_argument("--body-id", required=True)
parser.add_argument("--cap-id", default="wa.send")
parser.add_argument("--log-file", required=True)
parser.add_argument("--emit-place", default="")
parser.add_argument("--emit-actor", default="alice")
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
        log_event({"type": "discover", "body_id": args.body_id})
        send_json({
            "jsonrpc": "2.0",
            "id": rpc_id,
            "result": {
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
            },
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
