use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;

use monoagens_cli::{ChatArgs, canonical_h_act_details, run_chat_with_provider};
use monoagens_core::{
    Core, CoreConfig, EventDetail, EventKind, EventRecord, Input, NoopDoors, NumericFeatures, Store,
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
    let mut core = Core::open(
        store,
        CoreConfig {
            seed: 7,
            ..CoreConfig::default()
        },
        None,
    )
    .expect("core open");

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
    assert!(parsed["trace"]["neural"]["gate_probs"].is_array());
    assert!(parsed["trace"]["commitment_bypass"].is_boolean());
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
fn cli_approvals_renders_3line_impact_card_real_epoch_and_wires_grant_and_deny() {
    let tmp = TempDir::new().expect("tempdir");
    let cfg_path = write_scripted_config(tmp.path());
    let data_dir = tmp.path().join("data");
    let (_decision_id, expired_action) = seed_sample_store_with_held_action(&data_dir);

    // 1. Human-mode `approvals list` renders structured 3-line impact card
    let human_out = Command::new(monoagens_bin())
        .args(["--config", cfg_path.to_str().unwrap(), "approvals", "list"])
        .output()
        .expect("approvals list human");
    assert!(human_out.status.success());
    let human_stdout = String::from_utf8_lossy(&human_out.stdout);
    let lines: Vec<&str> = human_stdout.lines().collect();
    assert_eq!(lines.len(), 3, "expected 3-line impact card, got:\n{human_stdout}");
    assert!(lines[0].contains("capability=tool.write") && lines[0].contains("action=act-cli-held-1") && lines[0].contains("expires_at_ms="));
    assert!(lines[1].contains("places=[cli:terminal]") && lines[1].contains("taint=trusted"));
    assert!(lines[2].contains("h_act=blake3:") && lines[2].contains("state_epoch=0"));

    // 2. JSON-mode `approvals list` emits canonical H_act & real state_epoch
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

    let (expected_h_act, expected_input, expected_preimage) =
        canonical_h_act_details(&expired_action, 0);
    assert_eq!(list[0]["h_act"], expected_h_act.as_str());
    assert_eq!(list[0]["state_epoch"], 0);
    let actual_input: ActionCanonicalInput =
        serde_json::from_value(list[0]["canonical_input"].clone()).expect("canonical_input");
    assert_eq!(actual_input, expected_input);
    assert_eq!(
        serde_json::to_vec(&actual_input).unwrap(),
        serde_json::to_vec(&expected_input).unwrap()
    );
    assert_eq!(
        canonical_json_bytes(expired_action.payload.as_bytes()),
        expected_input.payload
    );
    let _ = expected_preimage;

    // 3. Stale 1970-timestamp held action fails closed as expired_ttl
    let expired_grant = Command::new(monoagens_bin())
        .args([
            "--config",
            cfg_path.to_str().unwrap(),
            "approvals",
            "grant",
            "act-cli-held-1",
        ])
        .output()
        .expect("expired grant");
    assert!(!expired_grant.status.success());
    assert!(
        String::from_utf8_lossy(&expired_grant.stderr).contains("expired_ttl"),
        "stderr={}",
        String::from_utf8_lossy(&expired_grant.stderr)
    );

    // 4. Seed a fresh unexpired held action at current wall clock & grant it out-of-band
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_millis() as u64;
    let mut store = SegmentedStore::open(StoreConfig::new(data_dir)).expect("reopen store");
    let fresh_action = Action {
        protocol_version: monoagens_protocol::CURRENT_PROTOCOL_VERSION,
        action_id: ActionId::new("act-cli-fresh-1"),
        capability_id: CapabilityId::new("tool.write"),
        places: vec![PlaceId::new("cli:terminal")],
        audience: Audience::new([ActorId::new("per_owner")]),
        origin_actor: OriginActor::new(ActorId::new("per_owner")),
        taint: Taint::Trusted,
        payload: OpaquePayload::from_bytes(br#"{"b":2,"a":1}"#.as_slice()),
        correlation_id: None,
        expected_effect: None,
        h_act: None,
        timestamp_ms: Time::from_millis(now_ms),
    };
    let fresh_h_act = compute_h_act(&fresh_action, 0);
    store
        .append(EventRecord::new_root(
            EventKind::Action,
            Time::from_millis(now_ms),
            ActorId::new("per_owner"),
            PlaceId::new("cli:terminal"),
            Audience::new([ActorId::new("per_owner")]),
            OriginActor::new(ActorId::new("per_owner")),
            Taint::Trusted,
            NumericFeatures::default(),
            fresh_action.payload.clone(),
            Vec::new(),
            Some(EventDetail::Action {
                action_id: fresh_action.action_id.clone(),
                capability_id: fresh_action.capability_id.clone(),
                h_act: Some(fresh_h_act.clone()),
                correlation_id: None,
            }),
        ))
        .expect("append fresh action");
    store
        .append(EventRecord::new_root(
            EventKind::Receipt,
            Time::from_millis(now_ms + 1),
            ActorId::new("valve"),
            PlaceId::new("cli:terminal"),
            Audience::new([ActorId::new("per_owner")]),
            OriginActor::new(ActorId::new("per_owner")),
            Taint::Trusted,
            NumericFeatures::default(),
            OpaquePayload::default(),
            Vec::new(),
            Some(EventDetail::Receipt {
                action_id: fresh_action.action_id.clone(),
                capability_id: fresh_action.capability_id.clone(),
                outcome: ReceiptOutcome::Held,
                termination_reason: None,
                progress: None,
                delivered_units: None,
                h_act: Some(fresh_h_act),
            }),
        ))
        .expect("append fresh held receipt");
    drop(store);

    let grant_out = Command::new(monoagens_bin())
        .env("MONOAGENS_APPROVAL_ATTESTATION_KEY", "cli-test-secret-key")
        .args([
            "--config",
            cfg_path.to_str().unwrap(),
            "--json",
            "approvals",
            "grant",
            "act-cli-fresh-1",
        ])
        .output()
        .expect("approvals grant");
    assert!(
        grant_out.status.success(),
        "approvals grant stderr={}",
        String::from_utf8_lossy(&grant_out.stderr)
    );
    let grant_json: Value = serde_json::from_slice(&grant_out.stdout).expect("grant json");
    assert_eq!(grant_json["applied"], true);
    assert_eq!(grant_json["approved"], true);
    assert_eq!(grant_json["out_of_band_channel"], true);

    // Re-running grant on already-resolved action fails closed
    let second_grant = Command::new(monoagens_bin())
        .env("MONOAGENS_APPROVAL_ATTESTATION_KEY", "cli-test-secret-key")
        .args([
            "--config",
            cfg_path.to_str().unwrap(),
            "approvals",
            "grant",
            "act-cli-fresh-1",
        ])
        .output()
        .expect("second grant");
    assert!(!second_grant.status.success());

    // 5. Seed a post-HardForget held action with non-zero state_epoch=2 and test `approvals deny`
    let mut store = SegmentedStore::open(StoreConfig::new(tmp.path().join("data"))).expect("reopen store");
    store
        .append(EventRecord::new_root(
            EventKind::HardForget,
            Time::from_millis(now_ms + 10),
            ActorId::new("core"),
            PlaceId::new("core:forget"),
            Audience::new([ActorId::new("per_owner")]),
            OriginActor::new(ActorId::new("per_owner")),
            Taint::Trusted,
            NumericFeatures::default(),
            OpaquePayload::default(),
            Vec::new(),
            Some(EventDetail::HardForget {
                target_event_ids: vec![],
                target_actor_ids: vec![],
                epoch: 2,
                reason: "epoch_bump".to_owned(),
            }),
        ))
        .expect("append hard_forget");
    let action2 = Action {
        protocol_version: monoagens_protocol::CURRENT_PROTOCOL_VERSION,
        action_id: ActionId::new("act-cli-held-epoch2"),
        capability_id: CapabilityId::new("fs.delete"),
        places: vec![PlaceId::new("cli:terminal")],
        audience: Audience::new([ActorId::new("per_owner")]),
        origin_actor: OriginActor::new(ActorId::new("per_owner")),
        taint: Taint::Trusted,
        payload: OpaquePayload::from_bytes(br#"{"path":"/tmp/x"}"#.as_slice()),
        correlation_id: None,
        expected_effect: None,
        h_act: None,
        timestamp_ms: Time::from_millis(now_ms + 20),
    };
    let h_act2 = compute_h_act(&action2, 2);
    store
        .append(EventRecord::new_root(
            EventKind::Action,
            Time::from_millis(now_ms + 20),
            ActorId::new("per_owner"),
            PlaceId::new("cli:terminal"),
            Audience::new([ActorId::new("per_owner")]),
            OriginActor::new(ActorId::new("per_owner")),
            Taint::Trusted,
            NumericFeatures::default(),
            action2.payload.clone(),
            Vec::new(),
            Some(EventDetail::Action {
                action_id: action2.action_id.clone(),
                capability_id: action2.capability_id.clone(),
                h_act: Some(h_act2.clone()),
                correlation_id: None,
            }),
        ))
        .expect("append action2");
    store
        .append(EventRecord::new_root(
            EventKind::Receipt,
            Time::from_millis(now_ms + 21),
            ActorId::new("valve"),
            PlaceId::new("cli:terminal"),
            Audience::new([ActorId::new("per_owner")]),
            OriginActor::new(ActorId::new("per_owner")),
            Taint::Trusted,
            NumericFeatures::default(),
            OpaquePayload::default(),
            Vec::new(),
            Some(EventDetail::Receipt {
                action_id: action2.action_id.clone(),
                capability_id: action2.capability_id.clone(),
                outcome: ReceiptOutcome::Held,
                termination_reason: None,
                progress: None,
                delivered_units: None,
                h_act: Some(h_act2.clone()),
            }),
        ))
        .expect("append receipt2");
    drop(store);

    let list_epoch2 = Command::new(monoagens_bin())
        .args([
            "--config",
            cfg_path.to_str().unwrap(),
            "--json",
            "approvals",
            "list",
        ])
        .output()
        .expect("approvals list epoch 2");
    assert!(list_epoch2.status.success());
    let parsed2: Vec<Value> = serde_json::from_slice(&list_epoch2.stdout).expect("json");
    let epoch2_item = parsed2
        .iter()
        .find(|item| item["action_id"] == "act-cli-held-epoch2")
        .expect("act-cli-held-epoch2 present");
    assert_eq!(epoch2_item["state_epoch"], 2);
    assert_eq!(epoch2_item["h_act"], h_act2.as_str());

    let deny_out = Command::new(monoagens_bin())
        .env("MONOAGENS_APPROVAL_ATTESTATION_KEY", "cli-test-secret-key")
        .args([
            "--config",
            cfg_path.to_str().unwrap(),
            "--json",
            "approvals",
            "deny",
            "act-cli-held-epoch2",
        ])
        .output()
        .expect("approvals deny");
    assert!(
        deny_out.status.success(),
        "approvals deny stderr={}",
        String::from_utf8_lossy(&deny_out.stderr)
    );
    let deny_json: Value = serde_json::from_slice(&deny_out.stdout).expect("deny json");
    assert_eq!(deny_json["applied"], true);
    assert_eq!(deny_json["approved"], false);
    assert_eq!(deny_json["state_epoch"], 2);

    // Denied action no longer appears in pending approvals list
    let list_final = Command::new(monoagens_bin())
        .args([
            "--config",
            cfg_path.to_str().unwrap(),
            "--json",
            "approvals",
            "list",
        ])
        .output()
        .expect("approvals list final");
    assert!(list_final.status.success());
    let final_arr: Vec<Value> = serde_json::from_slice(&list_final.stdout).expect("json");
    assert!(
        !final_arr
            .iter()
            .any(|item| item["action_id"] == "act-cli-held-epoch2"
                || item["action_id"] == "act-cli-fresh-1")
    );
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

    // Edit [soul] core_config in monoagens.toml to an edited config file.
    let edited_cfg_path = tmp.path().join("edited_core_config.json");
    fs::write(
        &edited_cfg_path,
        r#"{
          "format_version": 1,
          "certainty_bar": 0.72,
          "online_adaptation": false
        }"#,
    )
    .unwrap();
    let cfg_toml = format!(
        "version = 1\ndata_dir = \"{}\"\n\n[soul]\nname = \"MonoTest\"\nseed = 1\ncore_config = \"{}\"\n\n[model]\nprovider = \"scripted\"\nmodel = \"scripted-test\"\ndaily_token_budget = 10000\n\n[valve]\nmode = \"enforcing\"\nmax_chain_wakes = 8\n",
        data_dir.display(),
        edited_cfg_path.display()
    );
    fs::write(&cfg_path, cfg_toml).expect("update config with edited core_config");

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

    // Session 2 under edited config.
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

    // Verify log holds two config_loaded events (default, then edited).
    let store = SegmentedStore::open(StoreConfig::new(data_dir)).expect("open store");
    let events = store.read_in_order(None, usize::MAX).expect("read events");
    let cfg_events: Vec<_> = events
        .iter()
        .filter(|e| e.kind == EventKind::ConfigLoaded)
        .collect();
    assert_eq!(cfg_events.len(), 2, "expected 2 config_loaded events");
    match &cfg_events[0].detail {
        Some(EventDetail::ConfigLoaded { config, .. }) => {
            assert_eq!(config.as_ref(), &CoreConfig::default());
        }
        other => panic!("unexpected first config detail: {other:?}"),
    }
    match &cfg_events[1].detail {
        Some(EventDetail::ConfigLoaded { config, .. }) => {
            assert!((config.certainty_bar - 0.72).abs() < 1e-12);
            assert!(!config.online_adaptation);
        }
        other => panic!("unexpected second config detail: {other:?}"),
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
fn cli_chat_fails_startup_when_soul_core_config_is_missing_or_invalid() {
    let tmp = TempDir::new().expect("tempdir");
    let cfg_path = tmp.path().join("monoagens.toml");
    let data_dir = tmp.path().join("data");
    fs::create_dir_all(&data_dir).unwrap();
    let missing_cfg = tmp.path().join("nonexistent_core_config.json");

    let cfg_toml = format!(
        "version = 1\ndata_dir = \"{}\"\n\n[soul]\nname = \"MonoTest\"\nseed = 1\ncore_config = \"{}\"\n\n[model]\nprovider = \"scripted\"\nmodel = \"scripted-test\"\ndaily_token_budget = 10000\n",
        data_dir.display(),
        missing_cfg.display()
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
    .expect_err("chat must fail when [soul] core_config path is missing");
    assert!(err.contains("nonexistent_core_config.json"), "err={err}");
}

fn build_export_learning_fixture_log(include_hard_forget: bool) -> Vec<EventRecord> {
    use monoagens_core::{Core, CoreConfig, Input, MemStore, NoopDoors};
    use monoagens_protocol::{
        ActorId, Audience, ConsolidateOutput, OpaquePayload, Percept, PlaceId, Taint, ThoughtAct,
        Time,
    };

    let cfg = CoreConfig {
        sleep_min_episodes: 1,
        sleep_idle_ms: 1_000,
        ..CoreConfig::default()
    };
    let mut core = Core::open(MemStore::new(), cfg, None).unwrap();

    // 1. Group message & lesson (multi-party audience: alice, bob, soul)
    let mut p_group = Percept::minimal(
        ActorId::new("per_alice"),
        PlaceId::new("room:group"),
        Audience::new([
            ActorId::new("per_alice"),
            ActorId::new("per_bob"),
            ActorId::new("soul"),
        ]),
        Time::from_millis(1_000),
        OpaquePayload::from_bytes(b"group discussion"),
    );
    p_group.named = vec![ActorId::new("soul")];
    p_group.addressees = vec![ActorId::new("soul")];
    p_group.taint = Taint::Trusted;
    let eff_group = core
        .step(
            Input::Percept(Box::new(p_group)),
            Time::from_millis(1_000),
            &mut NoopDoors,
        )
        .unwrap();
    let group_wake_id = match eff_group.as_slice() {
        [monoagens_core::Effect::Model { plan, .. }] => plan.trigger_event_id.unwrap(),
        other => panic!("expected group wake, got {other:?}"),
    };
    core.step(
        Input::ModelResult {
            request_id: group_wake_id,
            acts: vec![
                ThoughtAct::Think {
                    thought: "group convention remembered".to_owned(),
                    motivation: 3,
                    confidence_0_20: 16,
                    used_ids: vec![group_wake_id],
                    importance: Some(0.8),
                    basis: Some("observed".to_owned()),
                    value: Some(1.0),
                    cost: Some(0.1),
                    domain: Some(monoagens_protocol::DomainId::new("ops")),
                },
                ThoughtAct::Consolidate {
                    output: ConsolidateOutput::Lesson {
                        pattern: "group_retry_guard".to_owned(),
                        failure_signature: "sig_group".to_owned(),
                        failure_ids: vec![group_wake_id, monoagens_protocol::EventId::new(999)],
                        contrastive_success_ids: vec![],
                        domain: Some(monoagens_protocol::DomainId::new("ops")),
                    },
                },
            ],
        },
        Time::from_millis(1_100),
        &mut NoopDoors,
    )
    .unwrap();

    // 2. Private DM message & lesson (audience: secret_eve, soul)
    let mut p_priv = Percept::minimal(
        ActorId::new("per_secret_eve"),
        PlaceId::new("dm:secret_eve"),
        Audience::new([ActorId::new("per_secret_eve"), ActorId::new("soul")]),
        Time::from_millis(2_000),
        OpaquePayload::from_bytes(b"private dm"),
    );
    p_priv.named = vec![ActorId::new("soul")];
    p_priv.addressees = vec![ActorId::new("soul")];
    p_priv.taint = Taint::Trusted;
    let eff_priv = core
        .step(
            Input::Percept(Box::new(p_priv)),
            Time::from_millis(2_000),
            &mut NoopDoors,
        )
        .unwrap();
    let priv_wake_id = match eff_priv.as_slice() {
        [monoagens_core::Effect::Model { plan, .. }] => plan.trigger_event_id.unwrap(),
        other => panic!("expected private wake, got {other:?}"),
    };
    core.step(
        Input::ModelResult {
            request_id: priv_wake_id,
            acts: vec![
                ThoughtAct::Think {
                    thought: "secret private belief".to_owned(),
                    motivation: 3,
                    confidence_0_20: 18,
                    used_ids: vec![priv_wake_id],
                    importance: Some(0.9),
                    basis: Some("observed".to_owned()),
                    value: Some(1.0),
                    cost: Some(0.1),
                    domain: Some(monoagens_protocol::DomainId::new("private_ops")),
                },
                ThoughtAct::Consolidate {
                    output: ConsolidateOutput::Lesson {
                        pattern: "secret_dm_lesson".to_owned(),
                        failure_signature: "sig_secret_eve".to_owned(),
                        failure_ids: vec![priv_wake_id, monoagens_protocol::EventId::new(998)],
                        contrastive_success_ids: vec![],
                        domain: Some(monoagens_protocol::DomainId::new("private_ops")),
                    },
                },
            ],
        },
        Time::from_millis(2_100),
        &mut NoopDoors,
    )
    .unwrap();

    // Record bounded place adaptation for both group and private places.
    core.record_place_feedback(
        &PlaceId::new("room:group"),
        monoagens_core::neural::AdaptFeedback {
            arm: monoagens_core::neural::GateArm::Go,
            reward: 1.0,
            is_delayed_one_sided: false,
            context_hint: [0.5, 0.25],
        },
    );
    core.record_place_feedback(
        &PlaceId::new("dm:secret_eve"),
        monoagens_core::neural::AdaptFeedback {
            arm: monoagens_core::neural::GateArm::Silent,
            reward: 1.0,
            is_delayed_one_sided: false,
            context_hint: [0.8, 0.2],
        },
    );

    if include_hard_forget {
        core.hard_forget(
            vec![priv_wake_id],
            vec![ActorId::new("per_secret_eve")],
            "gdpr_erasure",
            Time::from_millis(5_000),
        )
        .unwrap();
    }

    let _ = Taint::Trusted;
    core.store().read_in_order(None, usize::MAX).unwrap()
}

#[test]
fn cli_export_learning_is_bit_identical_and_respects_privacy_and_hard_forget() {
    let tmp = TempDir::new().expect("tempdir");
    let cfg_path = tmp.path().join("monoagens.toml");
    let data_dir = tmp.path().join("data");
    fs::create_dir_all(&data_dir).unwrap();
    fs::write(
        &cfg_path,
        format!(
            "version = 1\ndata_dir = \"{}\"\n\n[soul]\nname = \"MonoTest\"\nseed = 1\n",
            data_dir.display()
        ),
    )
    .unwrap();

    let events = build_export_learning_fixture_log(false);
    let log_json_path = tmp.path().join("log.json");
    fs::write(&log_json_path, serde_json::to_string_pretty(&events).unwrap()).unwrap();

    // 1. Repeatability: two runs produce byte-identical Markdown and identical export_blake3.
    let out_a = tmp.path().join("export_a.md");
    let out_b = tmp.path().join("export_b.md");
    let run_a = Command::new(monoagens_bin())
        .args([
            "--config",
            cfg_path.to_str().unwrap(),
            "--json",
            "export-learning",
            "--log-json",
            log_json_path.to_str().unwrap(),
            "--out",
            out_a.to_str().unwrap(),
        ])
        .output()
        .expect("run export-learning A");
    assert!(run_a.status.success(), "{}", String::from_utf8_lossy(&run_a.stderr));
    let run_b = Command::new(monoagens_bin())
        .args([
            "--config",
            cfg_path.to_str().unwrap(),
            "--json",
            "export-learning",
            "--log-json",
            log_json_path.to_str().unwrap(),
            "--out",
            out_b.to_str().unwrap(),
        ])
        .output()
        .expect("run export-learning B");
    assert!(run_b.status.success(), "{}", String::from_utf8_lossy(&run_b.stderr));

    let bytes_a = fs::read(&out_a).unwrap();
    let bytes_b = fs::read(&out_b).unwrap();
    assert_eq!(bytes_a, bytes_b, "repeated exports must be byte-identical");
    let json_a: Value = serde_json::from_slice(&run_a.stdout).unwrap();
    let json_b: Value = serde_json::from_slice(&run_b.stdout).unwrap();
    assert_eq!(json_a["export_blake3"], json_b["export_blake3"]);

    // 2. Privacy: default excludes private DM items; --include-private includes them with Audience labels.
    let md_default = String::from_utf8(bytes_a).unwrap();
    assert!(md_default.contains("group_retry_guard"));
    assert!(md_default.contains("room:group"));
    assert!(!md_default.contains("secret_dm_lesson"));
    assert!(!md_default.contains("dm:secret_eve"));
    assert!(!md_default.contains("per_secret_eve"));

    let out_priv = tmp.path().join("export_priv.md");
    let run_priv = Command::new(monoagens_bin())
        .args([
            "--config",
            cfg_path.to_str().unwrap(),
            "export-learning",
            "--log-json",
            log_json_path.to_str().unwrap(),
            "--out",
            out_priv.to_str().unwrap(),
            "--include-private",
        ])
        .output()
        .expect("run export-learning --include-private");
    assert!(run_priv.status.success());
    let md_priv = fs::read_to_string(&out_priv).unwrap();
    assert!(md_priv.contains("secret_dm_lesson"));
    assert!(md_priv.contains("dm:secret_eve"));
    assert!(md_priv.contains("audience=[per_secret_eve,soul]"));

    // 3. HardForget: after erasing per_secret_eve, even --include-private exports zero residual traces.
    let forgotten_events = build_export_learning_fixture_log(true);
    let forgotten_log_path = tmp.path().join("forgotten_log.json");
    fs::write(
        &forgotten_log_path,
        serde_json::to_string_pretty(&forgotten_events).unwrap(),
    )
    .unwrap();
    let out_forgotten = tmp.path().join("export_forgotten.md");
    let run_forgotten = Command::new(monoagens_bin())
        .args([
            "--config",
            cfg_path.to_str().unwrap(),
            "export-learning",
            "--log-json",
            forgotten_log_path.to_str().unwrap(),
            "--out",
            out_forgotten.to_str().unwrap(),
            "--include-private",
        ])
        .output()
        .expect("run export-learning post hard_forget");
    assert!(run_forgotten.status.success());
    let md_forgotten = fs::read_to_string(&out_forgotten).unwrap();
    assert!(md_forgotten.contains("group_retry_guard"));
    assert!(!md_forgotten.contains("per_secret_eve"));
    assert!(!md_forgotten.contains("dm:secret_eve"));
    assert!(!md_forgotten.contains("secret_dm_lesson"));
    assert!(!md_forgotten.contains("private_ops"));
}
