use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;

use monoagens_cli::{ChatArgs, canonical_h_act_details, run_chat_with_provider};
use monoagens_core::{
    Core, EventDetail, EventKind, EventRecord, Input, NoopDoors, NumericFeatures, Prior, Store,
    Switches,
};
use monoagens_protocol::{
    Action, ActionCanonicalInput, ActionId, ActorId, Audience, CapabilityId, OpaquePayload,
    OriginActor, Percept, PlaceId, ReceiptOutcome, Taint, TerminationReason, Time,
};
use monoagens_reason::{ScriptedModelProvider, ScriptedReply};
use monoagens_store::{SegmentedStore, StoreConfig};
use monoagens_valve::{canonical_json_bytes, compute_h_act};
use serde_json::{Value, json};
use tempfile::TempDir;

fn monoagens_bin() -> &'static str {
    env!("CARGO_BIN_EXE_monoagens")
}

fn body_cli_bin() -> PathBuf {
    let cli = PathBuf::from(monoagens_bin());
    cli.parent().expect("target dir").join("body-cli")
}

fn write_scripted_config(home: &std::path::Path) -> PathBuf {
    let cfg_path = home.join("monoagens.toml");
    let data_dir = home.join("data");
    fs::create_dir_all(&data_dir).expect("mkdir data");
    let toml = format!(
        "version = 1\ndata_dir = \"{}\"\n\n[soul]\nname = \"MonoTest\"\nseed = 1\n\n[model]\nprovider = \"scripted\"\nmodel = \"scripted-test\"\ndaily_token_budget = 10000\n\n[valve]\nmode = \"enforcing\"\nmax_chain_wakes = 8\n",
        data_dir.display()
    );
    fs::write(&cfg_path, toml).expect("write config");
    cfg_path
}

fn seed_sample_store_with_held_action(data_dir: &std::path::Path) -> (u64, Action) {
    let store = SegmentedStore::open(StoreConfig::new(data_dir.to_path_buf())).expect("open store");
    let mut core = Core::open(store, Prior::default(), Switches::default(), 7).expect("core open");

    let mut p1 = Percept::minimal(
        ActorId::new("per_alice"),
        PlaceId::new("cli:terminal"),
        Audience::new([ActorId::new("per_alice")]),
        Time::from_millis(1_000),
        OpaquePayload::from_bytes(b"hello".as_slice()),
    );
    p1.addressees = vec![ActorId::new("soul")];
    core.step(
        Input::Percept(Box::new(p1)),
        Time::from_millis(1_000),
        &mut NoopDoors,
    )
    .expect("step percept");

    // Append a held action + receipt + door_verdict for log/approvals/inspector tests.
    let action = Action {
        protocol_version: monoagens_protocol::CURRENT_PROTOCOL_VERSION,
        action_id: ActionId::new("act-cli-held-1"),
        capability_id: CapabilityId::new("tool.write"),
        places: vec![PlaceId::new("cli:terminal")],
        audience: Audience::new([ActorId::new("per_alice")]),
        origin_actor: OriginActor::new(ActorId::new("per_alice")),
        taint: Taint::Trusted,
        payload: OpaquePayload::from_bytes(br#"{"b":2,"a":1}"#.as_slice()),
        correlation_id: None,
        expected_effect: None,
        h_act: None,
        timestamp_ms: Time::from_millis(1_500),
    };
    let h_act = compute_h_act(&action, 0);

    let events = core.store().read_in_order(None, usize::MAX).expect("read");
    let decision_id = events
        .iter()
        .find(|e| e.kind == EventKind::Decision)
        .expect("decision event")
        .id
        .as_u64();

    drop(core);
    let mut store2 =
        SegmentedStore::open(StoreConfig::new(data_dir.to_path_buf())).expect("reopen store");

    let dv = EventRecord::new_root(
        EventKind::DoorVerdict,
        Time::from_millis(1_500),
        ActorId::new("per_alice"),
        PlaceId::new("cli:terminal"),
        Audience::new([ActorId::new("per_alice")]),
        OriginActor::new(ActorId::new("per_alice")),
        Taint::Trusted,
        NumericFeatures::default(),
        OpaquePayload::default(),
        Vec::new(),
        Some(EventDetail::DoorVerdict {
            door: monoagens_core::DoorKind::Action,
            target_event_id: Some(monoagens_protocol::EventId::new(decision_id)),
            rule: "valve.action.pending_approval".to_owned(),
            verdict: "PendingApproval".to_owned(),
        }),
    );
    store2.append(dv).expect("append door_verdict");

    let act_rec = EventRecord::new_root(
        EventKind::Action,
        Time::from_millis(1_500),
        ActorId::new("per_alice"),
        PlaceId::new("cli:terminal"),
        Audience::new([ActorId::new("per_alice")]),
        OriginActor::new(ActorId::new("per_alice")),
        Taint::Trusted,
        NumericFeatures::default(),
        action.payload.clone(),
        Vec::new(),
        Some(EventDetail::Action {
            action_id: action.action_id.clone(),
            capability_id: action.capability_id.clone(),
            h_act: Some(h_act.clone()),
            correlation_id: None,
        }),
    );
    store2.append(act_rec).expect("append action");

    let rcpt_rec = EventRecord::new_root(
        EventKind::Receipt,
        Time::from_millis(1_501),
        ActorId::new("per_alice"),
        PlaceId::new("cli:terminal"),
        Audience::new([ActorId::new("per_alice")]),
        OriginActor::new(ActorId::new("per_alice")),
        Taint::Trusted,
        NumericFeatures::default(),
        OpaquePayload::default(),
        Vec::new(),
        Some(EventDetail::Receipt {
            action_id: action.action_id.clone(),
            capability_id: action.capability_id.clone(),
            outcome: ReceiptOutcome::Held,
            termination_reason: Some(TerminationReason::Normal),
            progress: Some(0.0),
            delivered_units: Some(0),
            h_act: Some(h_act),
        }),
    );
    store2.append(rcpt_rec).expect("append receipt");

    (decision_id, action)
}

#[test]
fn cli_chat_first_run_unconfigured_bootstraps_without_secrets_and_exits_nonzero() {
    let tmp = TempDir::new().expect("tempdir");
    let home = tmp.path().join("home");

    let out = Command::new(monoagens_bin())
        .env("MONOAGENS_HOME", &home)
        .env_remove("MONOAGENS_MODEL_API_KEY")
        .env_remove("OPENAI_API_KEY")
        .env_remove("MONOAGENS_MODEL_ENDPOINT")
        .args(["chat", "--message", "hello"])
        .output()
        .expect("run monoagens chat");

    assert!(
        !out.status.success(),
        "first-run unconfigured chat must exit non-zero"
    );
    let cfg_path = home.join("monoagens.toml");
    assert!(cfg_path.exists(), "default monoagens.toml must be written");
    let cfg_text = fs::read_to_string(&cfg_path).expect("read config");
    assert!(!cfg_text.contains("api_key"));
    assert!(!cfg_text.contains("sk-"));

    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("MONOAGENS_MODEL_API_KEY"),
        "stderr={stderr}"
    );
    assert!(stderr.contains("OPENAI_API_KEY"), "stderr={stderr}");
    assert!(stderr.contains("monoagens.toml"), "stderr={stderr}");
}

#[test]
fn cli_chat_configured_scripted_executes_body_cli_action() {
    let tmp = TempDir::new().expect("tempdir");
    let cfg_path = write_scripted_config(tmp.path());

    let act_reply = json!({
        "protocol_version": { "major": 1, "minor": 0 },
        "acts": [
            {
                "act": "act",
                "capability_id": "chat.speak",
                "places": ["cli:terminal"],
                "people": ["per_owner"],
                "payload": b"Hello from scripted monoagens!".to_vec(),
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

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let code = run_chat_with_provider(
        Some(&cfg_path),
        false,
        ChatArgs {
            messages: vec!["hi soul".to_owned()],
            data_dir: None,
        },
        Some(provider),
        &mut stdout,
        &mut stderr,
    )
    .expect("chat run");

    assert_eq!(code, 0, "stderr={}", String::from_utf8_lossy(&stderr));
    let out_str = String::from_utf8_lossy(&stdout);
    if !out_str.contains("Hello from scripted monoagens!") {
        let store = SegmentedStore::open(StoreConfig::new(tmp.path().join("data"))).unwrap();
        let evs = store.read_in_order(None, usize::MAX).unwrap();
        panic!(
            "expected action output visible on stdout, got: {out_str:?}\nstderr: {}\nevents: {evs:#?}",
            String::from_utf8_lossy(&stderr)
        );
    }
}

#[test]
fn cli_log_explain_and_filters_emit_pure_json() {
    let tmp = TempDir::new().expect("tempdir");
    let cfg_path = write_scripted_config(tmp.path());
    let data_dir = tmp.path().join("data");
    let (decision_id, _action) = seed_sample_store_with_held_action(&data_dir);

    // 1. log --explain <id> --json
    let out = Command::new(monoagens_bin())
        .args([
            "--config",
            cfg_path.to_str().unwrap(),
            "--json",
            "log",
            "--explain",
            &decision_id.to_string(),
        ])
        .output()
        .expect("run log --explain");
    assert!(
        out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let parsed: Value = serde_json::from_slice(&out.stdout).expect("pure JSON stdout");
    assert!(parsed["trace"]["threshold_terms"].is_object());
    assert!(parsed["trace"]["signals"].is_object());
    assert!(parsed["draw"].is_number());
    assert!(parsed["probability"].is_number());
    assert_eq!(
        parsed["door_verdicts"][0]["reason_tag"].as_str(),
        Some("PendingApproval")
    );

    // 2. log filters (--kind door_verdict --place cli:terminal --source per_alice)
    let filt_out = Command::new(monoagens_bin())
        .args([
            "--config",
            cfg_path.to_str().unwrap(),
            "--json",
            "log",
            "--kind",
            "door_verdict",
            "--place",
            "cli:terminal",
            "--source",
            "per_alice",
        ])
        .output()
        .expect("run log filter");
    assert!(filt_out.status.success());
    let rows: Vec<Value> = serde_json::from_slice(&filt_out.stdout).expect("JSON array");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["kind"], "door_verdict");
}

#[test]
fn cli_replay_offline_succeeds_and_mutated_log_reports_divergence() {
    let tmp = TempDir::new().expect("tempdir");
    let cfg_path = write_scripted_config(tmp.path());
    let data_dir = tmp.path().join("data");
    let (_decision_id, _action) = seed_sample_store_with_held_action(&data_dir);

    // Clean offline replay against store (seed = 7 matches store creation above).
    let ok_out = Command::new(monoagens_bin())
        .args([
            "--config",
            cfg_path.to_str().unwrap(),
            "replay",
            "--seed",
            "7",
        ])
        .output()
        .expect("replay clean");
    assert!(
        ok_out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&ok_out.stderr)
    );
    let stdout = String::from_utf8_lossy(&ok_out.stdout);
    assert!(
        stdout.contains("all decisions reproduced"),
        "stdout={stdout}"
    );

    // Mutate a Decision event in JSON and verify non-zero exit naming divergence.
    let store = SegmentedStore::open(StoreConfig::new(data_dir)).expect("open");
    let mut events = store.read_in_order(None, usize::MAX).expect("read");
    for ev in &mut events {
        if let Some(EventDetail::Decision { salience, .. }) = &mut ev.detail {
            *salience = 0.999_123;
        }
    }
    let mutated_path = tmp.path().join("mutated_log.json");
    fs::write(
        &mutated_path,
        serde_json::to_vec(&events).expect("serialize events"),
    )
    .expect("write mutated");

    let bad_out = Command::new(monoagens_bin())
        .args([
            "--config",
            cfg_path.to_str().unwrap(),
            "--json",
            "replay",
            "--seed",
            "7",
            "--log-json",
            mutated_path.to_str().unwrap(),
        ])
        .output()
        .expect("replay mutated");
    assert!(!bad_out.status.success(), "mutated log must exit non-zero");
    let parsed: Value = serde_json::from_slice(&bad_out.stdout).expect("pure JSON stdout");
    assert_eq!(parsed["status"], "diverged");
    let stderr = String::from_utf8_lossy(&bad_out.stderr);
    assert!(stderr.contains("divergence"), "stderr={stderr}");
}

#[test]
fn cli_approvals_renders_canonical_h_act_and_grant_exits_nonzero_when_unwired() {
    let tmp = TempDir::new().expect("tempdir");
    let cfg_path = write_scripted_config(tmp.path());
    let data_dir = tmp.path().join("data");
    let (_decision_id, action) = seed_sample_store_with_held_action(&data_dir);

    let out = Command::new(monoagens_bin())
        .args([
            "--config",
            cfg_path.to_str().unwrap(),
            "--json",
            "approvals",
            "list",
        ])
        .output()
        .expect("approvals list");
    assert!(out.status.success());
    let list: Vec<Value> = serde_json::from_slice(&out.stdout).expect("JSON array");
    assert_eq!(list.len(), 1);

    let (expected_h_act, expected_input, expected_preimage) = canonical_h_act_details(&action, 0);
    assert_eq!(list[0]["h_act"], expected_h_act.as_str());
    let actual_input: ActionCanonicalInput =
        serde_json::from_value(list[0]["canonical_input"].clone()).expect("canonical_input");
    assert_eq!(actual_input, expected_input);
    assert_eq!(
        serde_json::to_vec(&actual_input).unwrap(),
        serde_json::to_vec(&expected_input).unwrap()
    );
    assert_eq!(
        canonical_json_bytes(action.payload.as_bytes()),
        expected_input.payload
    );
    let _ = expected_preimage;

    // grant must NOT fake success: exits non-zero explaining blocked write path.
    let grant_out = Command::new(monoagens_bin())
        .args([
            "--config",
            cfg_path.to_str().unwrap(),
            "approvals",
            "grant",
            "act-cli-held-1",
        ])
        .output()
        .expect("approvals grant");
    assert!(!grant_out.status.success());
    let stderr = String::from_utf8_lossy(&grant_out.stderr);
    assert!(stderr.contains("blocked"), "stderr={stderr}");
}

#[test]
fn cli_conformance_inspector_and_link_work_end_to_end() {
    let tmp = TempDir::new().expect("tempdir");
    let cfg_path = write_scripted_config(tmp.path());
    let report_path = tmp.path().join("compat.json");

    // 1. conformance body against built body-cli
    let body_bin = body_cli_bin();
    assert!(
        body_bin.exists(),
        "body-cli missing at {}",
        body_bin.display()
    );
    let conf_out = Command::new(monoagens_bin())
        .args([
            "conformance",
            "body",
            "--target",
            body_bin.to_str().unwrap(),
            "--level",
            "must",
            "--json-report",
            report_path.to_str().unwrap(),
        ])
        .output()
        .expect("conformance body");
    assert!(
        conf_out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&conf_out.stderr)
    );
    assert!(report_path.exists());
    let report: Value = serde_json::from_slice(&conf_out.stdout).expect("pure JSON stdout");
    assert_eq!(report["level"], "must");

    // 2. inspector prints active rules & symcc proof without secrets
    let insp_out = Command::new(monoagens_bin())
        .args([
            "--config",
            cfg_path.to_str().unwrap(),
            "--json",
            "inspector",
        ])
        .output()
        .expect("inspector");
    assert!(
        insp_out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&insp_out.stderr)
    );
    let insp: Value = serde_json::from_slice(&insp_out.stdout).expect("JSON");
    assert_eq!(insp["symcc_proof"]["status"], "unsat");
    assert!(!insp["active_rule_names"].as_array().unwrap().is_empty());

    // 3. link resolve -> initiate -> verify -> list
    let init_out = Command::new(monoagens_bin())
        .args([
            "--config",
            cfg_path.to_str().unwrap(),
            "--json",
            "link",
            "initiate",
            "--primary-body",
            "body-cli",
            "--primary-account",
            "alice",
            "--secondary-body",
            "body-mcp",
            "--secondary-account",
            "@alice:matrix.org",
            "--emit-nonces",
        ])
        .output()
        .expect("link initiate");
    assert!(init_out.status.success());
    let init_json: Value = serde_json::from_slice(&init_out.stdout).expect("JSON");
    let cid = init_json["challenge_id"].as_str().unwrap();
    let n1 = init_json["nonce_1"].as_str().unwrap();
    let n2 = init_json["nonce_2"].as_str().unwrap();

    let verify_out = Command::new(monoagens_bin())
        .args([
            "--config",
            cfg_path.to_str().unwrap(),
            "--json",
            "link",
            "verify",
            "--challenge",
            cid,
            "--nonce1",
            n1,
            "--nonce2",
            n2,
        ])
        .output()
        .expect("link verify");
    assert!(verify_out.status.success());
    let verify_json: Value = serde_json::from_slice(&verify_out.stdout).expect("JSON");
    assert_eq!(verify_json["status"], "linked");

    let list_out = Command::new(monoagens_bin())
        .args([
            "--config",
            cfg_path.to_str().unwrap(),
            "--json",
            "link",
            "list",
        ])
        .output()
        .expect("link list");
    assert!(list_out.status.success());
    let list_json: Value = serde_json::from_slice(&list_out.stdout).expect("JSON");
    assert_eq!(list_json["active_links"].as_array().unwrap().len(), 1);
}

#[test]
fn cli_two_session_prior_change_and_offline_replay_reproduces_both_sessions() {
    let tmp = TempDir::new().expect("tempdir");
    let cfg_path = write_scripted_config(tmp.path());
    let data_dir = tmp.path().join("data");

    let act_reply_1 = json!({
        "protocol_version": { "major": 1, "minor": 0 },
        "acts": [
            {
                "act": "act",
                "capability_id": "chat.speak",
                "places": ["cli:terminal"],
                "people": ["per_owner"],
                "payload": b"Session 1 reply".to_vec(),
                "motivation": 4,
                "confidence_0_20": 16,
                "used_ids": [],
                "expected": null,
                "domain": null
            }
        ]
    })
    .to_string();

    // Session 1 under built-in default prior (no [soul] prior in monoagens.toml).
    let mut stdout1 = Vec::new();
    let mut stderr1 = Vec::new();
    let code1 = run_chat_with_provider(
        Some(&cfg_path),
        false,
        ChatArgs {
            messages: vec!["session 1 message".to_owned()],
            data_dir: None,
        },
        Some(Arc::new(ScriptedModelProvider::from_replies([
            ScriptedReply::text(act_reply_1),
        ]))),
        &mut stdout1,
        &mut stderr1,
    )
    .expect("session 1 chat");
    assert_eq!(code1, 0, "stderr1={}", String::from_utf8_lossy(&stderr1));

    // Edit [soul] prior in monoagens.toml to an edited prior file.
    let edited_prior_path = tmp.path().join("edited_prior.json");
    fs::write(
        &edited_prior_path,
        r#"{
          "version": 1,
          "content_version": "2026-11-17.session2",
          "talkativeness": 0.95,
          "spontaneity": 0.5,
          "memory_persistence": 0.5,
          "helpfulness": 0.5,
          "restraint": 0.1,
          "boredom_with_repeats": 0.5,
          "certainty_bar": 0.5,
          "speaking_bar": 3
        }"#,
    )
    .unwrap();
    let cfg_toml = format!(
        "version = 1\ndata_dir = \"{}\"\n\n[soul]\nname = \"MonoTest\"\nseed = 1\nprior = \"{}\"\n\n[model]\nprovider = \"scripted\"\nmodel = \"scripted-test\"\ndaily_token_budget = 10000\n\n[valve]\nmode = \"enforcing\"\nmax_chain_wakes = 8\n",
        data_dir.display(),
        edited_prior_path.display()
    );
    fs::write(&cfg_path, cfg_toml).expect("update config with edited prior");

    let act_reply_2 = json!({
        "protocol_version": { "major": 1, "minor": 0 },
        "acts": [
            {
                "act": "act",
                "capability_id": "chat.speak",
                "places": ["cli:terminal"],
                "people": ["per_owner"],
                "payload": b"Session 2 reply".to_vec(),
                "motivation": 4,
                "confidence_0_20": 16,
                "used_ids": [],
                "expected": null,
                "domain": null
            }
        ]
    })
    .to_string();

    // Session 2 under edited prior.
    let mut stdout2 = Vec::new();
    let mut stderr2 = Vec::new();
    let code2 = run_chat_with_provider(
        Some(&cfg_path),
        false,
        ChatArgs {
            messages: vec!["session 2 message".to_owned()],
            data_dir: None,
        },
        Some(Arc::new(ScriptedModelProvider::from_replies([
            ScriptedReply::text(act_reply_2),
        ]))),
        &mut stdout2,
        &mut stderr2,
    )
    .expect("session 2 chat");
    assert_eq!(code2, 0, "stderr2={}", String::from_utf8_lossy(&stderr2));

    // Verify log holds two prior_loaded events (default, then edited).
    let store = SegmentedStore::open(StoreConfig::new(data_dir)).expect("open store");
    let events = store.read_in_order(None, usize::MAX).expect("read events");
    let prior_events: Vec<_> = events
        .iter()
        .filter(|e| e.kind == EventKind::PriorLoaded)
        .collect();
    assert_eq!(prior_events.len(), 2, "expected 2 prior_loaded events");
    match &prior_events[0].detail {
        Some(EventDetail::PriorLoaded { prior }) => {
            assert_eq!(prior.as_ref(), &Prior::default());
        }
        other => panic!("unexpected first prior detail: {other:?}"),
    }
    match &prior_events[1].detail {
        Some(EventDetail::PriorLoaded { prior }) => {
            assert_eq!(prior.content_version, "2026-11-17.session2");
            assert!((prior.talkativeness - 0.95).abs() < 1e-12);
        }
        other => panic!("unexpected second prior detail: {other:?}"),
    }

    // Run `monoagens replay` against the store and verify both sessions reproduce cleanly.
    let replay_out = Command::new(monoagens_bin())
        .args(["--config", cfg_path.to_str().unwrap(), "--json", "replay"])
        .output()
        .expect("run monoagens replay");
    assert!(
        replay_out.status.success(),
        "replay failed: {}",
        String::from_utf8_lossy(&replay_out.stderr)
    );
    let rep_json: Value = serde_json::from_slice(&replay_out.stdout).expect("parse replay JSON");
    assert_eq!(rep_json["status"], "ok");
    assert!(
        rep_json["matched_decisions"].as_u64().unwrap_or(0) >= 2,
        "expected >= 2 matched decisions across both sessions: {rep_json}"
    );
}

#[test]
fn cli_chat_fails_startup_when_soul_prior_is_missing_or_invalid() {
    let tmp = TempDir::new().expect("tempdir");
    let cfg_path = tmp.path().join("monoagens.toml");
    let data_dir = tmp.path().join("data");
    fs::create_dir_all(&data_dir).unwrap();
    let missing_prior = tmp.path().join("nonexistent_prior.json");

    let cfg_toml = format!(
        "version = 1\ndata_dir = \"{}\"\n\n[soul]\nname = \"MonoTest\"\nseed = 1\nprior = \"{}\"\n\n[model]\nprovider = \"scripted\"\nmodel = \"scripted-test\"\ndaily_token_budget = 10000\n",
        data_dir.display(),
        missing_prior.display()
    );
    fs::write(&cfg_path, cfg_toml).unwrap();

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let err = run_chat_with_provider(
        Some(&cfg_path),
        false,
        ChatArgs {
            messages: vec!["hi".to_owned()],
            data_dir: None,
        },
        Some(Arc::new(ScriptedModelProvider::new())),
        &mut stdout,
        &mut stderr,
    )
    .expect_err("chat must fail when [soul] prior path is missing");
    assert!(err.contains("nonexistent_prior.json"), "err={err}");
}
