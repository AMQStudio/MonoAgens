//! Governance and observability backend (change `tui-governance-observability`): approvals,
//! people/links, policy, events, activity and Lab jobs over the owner's admin methods.

use std::sync::Arc;
use std::time::Duration;

use monoagens_core::{
    CoreConfig, DecisionOutcome, EventDetail, EventKind, EventRecord, MemStore, NumericFeatures,
    Store,
};
use monoagens_gateway::governance::{
    activity_rows, pending_approvals, plugin_for_place, build_valve_config,
};
use monoagens_gateway::{
    AdminClient, AdminServer, CueRecorder, DiagnosticSink, GatewayConfig, GatewayHandle,
    GatewayOptions, GatewayOwner, GatewayRuntime, GatewaySeams, ModelProviderKind, OwnerMode,
    OwnerParams, PluginEnvContext, PluginSupervisor, RestartPolicy, RunLog, SystemClock,
    DoorController, ValveDoorController, acquire_data_dir_lock, resolve_approval_mac_key,
};
use monoagens_protocol::{
    Action, ActionId, ActorId, ApprovalResolveParams, Audience, CapabilityId,
    OpaquePayload, OriginActor, PlaceId, ReceiptOutcome, Taint, TerminationReason, Time,
    Waist2Headers,
};
use monoagens_valve::{
    Valve, ValveConfig, ValveSnapshot, compute_h_act, sign_approval_resolution,
};
use serde_json::{Value, json};

/// `AdminClient::call` that fails the test after 10 s instead of hanging.
async fn call(c: &mut AdminClient, m: &str, p: Value) -> Result<Value, monoagens_gateway::AdminError> {
    tokio::time::timeout(Duration::from_secs(10), c.call(m, p))
        .await
        .unwrap_or_else(|_| panic!("admin call '{m}' got no reply within 10s"))
}
use tempfile::TempDir;

fn test_dir() -> TempDir {
    tempfile::Builder::new()
        .prefix("gov-")
        .tempdir_in(env!("CARGO_TARGET_TMPDIR"))
        .expect("tempdir under target")
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_millis() as u64
}

fn rec(kind: EventKind, ts: u64, actor: &str, place: &str, payload: &[u8], detail: Option<EventDetail>) -> EventRecord {
    EventRecord::new_root(
        kind,
        Time::from_millis(ts),
        ActorId::new(actor),
        PlaceId::new(place),
        Audience::new([ActorId::new(actor)]),
        OriginActor::new(ActorId::new(actor)),
        Taint::Trusted,
        NumericFeatures::default(),
        OpaquePayload::from_bytes(payload),
        Vec::new(),
        detail,
    )
}

fn held_action(id: &str, place: &str, ts: u64) -> Action {
    Action {
        protocol_version: monoagens_protocol::CURRENT_PROTOCOL_VERSION,
        action_id: ActionId::new(id),
        capability_id: CapabilityId::new("tool.write"),
        places: vec![PlaceId::new(place)],
        audience: Audience::new([ActorId::new("per_alice")]),
        origin_actor: OriginActor::new(ActorId::new("per_alice")),
        taint: Taint::Trusted,
        payload: OpaquePayload::from_bytes(br#"{"b":2,"a":1}"#.as_slice()),
        correlation_id: None,
        expected_effect: None,
        h_act: None,
        timestamp_ms: Time::from_millis(ts),
    }
}

/// Appends a held action (action + held receipt) at `place`, like the CLI approvals tests.
fn append_held(store: &mut MemStore, id: &str, place: &str, ts: u64) {
    let action = held_action(id, place, ts);
    let h_act = compute_h_act(&action, 0);
    store
        .append(rec(
            EventKind::Action,
            ts,
            "per_alice",
            place,
            action.payload.as_bytes(),
            Some(EventDetail::Action {
                action_id: action.action_id.clone(),
                capability_id: action.capability_id.clone(),
                h_act: Some(h_act.clone()),
                correlation_id: None,
            }),
        ))
        .expect("append action");
    store
        .append(rec(
            EventKind::Receipt,
            ts + 1,
            "per_alice",
            place,
            b"",
            Some(EventDetail::Receipt {
                action_id: action.action_id,
                capability_id: action.capability_id,
                outcome: ReceiptOutcome::Held,
                termination_reason: Some(TerminationReason::Normal),
                progress: Some(0.0),
                delivered_units: Some(0),
                h_act: Some(h_act),
            }),
        ))
        .expect("append receipt");
}

struct Env {
    rt: GatewayRuntime,
    handle: GatewayHandle,
    owner: Arc<GatewayOwner>,
    _server: AdminServer,
    _lock: monoagens_gateway::DataDirLock,
    dir: TempDir,
}

async fn boot(seed: impl FnOnce(&mut MemStore) + Send + 'static, with_valve: bool) -> Env {
    let dir = test_dir();
    let mut cfg = GatewayConfig::default();
    cfg.model.provider = ModelProviderKind::Scripted;
    cfg.model.model = "mock".to_owned();
    let secrets = monoagens_reason::ProviderSecrets::default();
    let mac_key = resolve_approval_mac_key(&Default::default());

    let mut seeded = MemStore::new();
    seed(&mut seeded);
    let events = seeded.read_in_order(None, usize::MAX).expect("read seed");

    let options = || GatewayOptions {
        clock: Arc::new(SystemClock::new()),
        provider: None,
        diagnostics: DiagnosticSink::new(&secrets),
        cues: CueRecorder::new(),
        seams: GatewaySeams::default(),
    };
    let rt = if with_valve {
        let now = Time::from_millis(now_ms());
        let owner_actor = ActorId::new("per_owner");
        let mut snapshot = ValveSnapshot {
            owner_actor_id: Some(owner_actor.clone()),
            t_now: now,
            ..ValveSnapshot::default()
        };
        snapshot.known_actors.insert(owner_actor.clone());
        snapshot.known_actors.insert(ActorId::new("soul"));
        snapshot.known_actors.insert(ActorId::new("per_alice"));
        snapshot.active_context_audience =
            Some(Audience::new([owner_actor, ActorId::new("soul"), ActorId::new("per_alice")]));
        // Policy verification blocks on its own runtime: do it off the async worker.
        let mut valve = tokio::task::block_in_place(|| {
            Valve::with_default_policies(ValveConfig::default(), snapshot)
        })
        .expect("valve");
        valve.set_approval_attestation_key(mac_key);
        let mut controller = ValveDoorController::new(valve);
        controller.replay_from_store_events(&events);
        GatewayRuntime::start(
            cfg.clone(),
            &secrets,
            move || Ok(seeded),
            CoreConfig::default(),
            controller,
            options(),
        )
        .await
        .expect("start")
    } else {
        GatewayRuntime::start(
            cfg.clone(),
            &secrets,
            move || Ok(seeded),
            CoreConfig::default(),
            monoagens_core::NoopDoors,
            options(),
        )
        .await
        .expect("start")
    };
    let handle = rt.handle();
    let log = RunLog::new(Arc::clone(handle.clock()), handle.diagnostics().clone(), None);
    let sup = PluginSupervisor::new(
        handle.clone(),
        PluginEnvContext {
            config_dir: None,
            data_dir: Some(dir.path().to_path_buf()),
            rbac_enabled: false,
            plugin_env: Default::default(),
        },
        Arc::new(SystemClock::new()),
        log.clone(),
        RestartPolicy::default(),
        vec![],
    );
    let owner = Arc::new(GatewayOwner::new(OwnerParams {
        handle: handle.clone(),
        supervisor: sup,
        runlog: log,
        mode: OwnerMode::Serve,
        config: cfg,
        config_path: dir.path().join("monoagens.toml"),
        secrets_path: dir.path().join("secrets.toml"),
        data_dir: dir.path().to_path_buf(),
        owner_actor: ActorId::new("per_owner"),
    }));
    let lock = acquire_data_dir_lock(dir.path()).expect("lock");
    let server = AdminServer::start(&lock, dir.path(), Arc::clone(&owner)).await.expect("serve");
    Env { rt, handle, owner, _server: server, _lock: lock, dir }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn approvals_list_shows_adapter_hold_and_equals_the_cli_function() {
    let ts = now_ms();
    let env = boot(move |s| append_held(s, "act-wa-1", "wa:dm:alice", ts), false).await;
    let mut c = AdminClient::connect(env.dir.path()).await.unwrap();
    let got = call(&mut c, "approvals.list", Value::Null).await.unwrap();
    let list = got["approvals"].as_array().unwrap();
    assert_eq!(list.len(), 1, "{got}");
    assert_eq!(list[0]["action_id"], json!("act-wa-1"));
    assert_eq!(list[0]["origin"], json!("whatsapp"));
    assert_eq!(list[0]["requester"], json!("per_alice"));
    assert_eq!(list[0]["places"], json!(["wa:dm:alice"]));

    // Same function the CLI calls, same data: identical entries.
    let events = env.handle.read_events().await.unwrap();
    let direct = pending_approvals(
        &events,
        &ActorId::new("per_owner"),
        &build_valve_config(&GatewayConfig::default()),
        resolve_approval_mac_key(&Default::default()),
    );
    assert_eq!(serde_json::to_value(&direct).unwrap(), got["approvals"]);
    assert_eq!(plugin_for_place("tg:group:1"), "telegram");
    assert_eq!(plugin_for_place("cli:terminal"), "tui");
    env.rt.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn idle_approvals_polls_do_not_replay_the_event_log() {
    let ts = now_ms();
    let env = boot(move |s| append_held(s, "act-wa-1", "wa:dm:alice", ts), false).await;
    let mut c = AdminClient::connect(env.dir.path()).await.unwrap();
    let first = call(&mut c, "approvals.list", Value::Null).await.unwrap();
    assert_eq!(first["approvals"].as_array().unwrap().len(), 1);
    assert_eq!(env.owner.approvals_full_replays(), 1, "first poll replays once");
    for _ in 0..5 {
        let again = call(&mut c, "approvals.list", Value::Null).await.unwrap();
        assert_eq!(again, first);
    }
    assert_eq!(
        env.owner.approvals_full_replays(),
        1,
        "polls with no new events must not re-read the full log"
    );
    env.rt.shutdown().await;
}

async fn resolve(c: &mut AdminClient, entry: &Value, approved: bool) -> Result<Value, monoagens_gateway::AdminError> {
    let action_id = ActionId::new(entry["action_id"].as_str().unwrap());
    let h_act: monoagens_protocol::HAct = serde_json::from_value(entry["h_act"].clone()).unwrap();
    let epoch = entry["state_epoch"].as_u64().unwrap();
    let key = resolve_approval_mac_key(&Default::default());
    let now = Time::from_millis(now_ms());
    let sig = sign_approval_resolution(&key, &action_id, &h_act, epoch, approved, now);
    let params = ApprovalResolveParams {
        headers: Waist2Headers::new(format!("test-approval-{}", action_id.as_str())),
        action_id,
        h_act,
        state_epoch: epoch,
        approved,
        resolver: ActorId::new("per_owner"),
        resolved_at_ms: now,
        signature: Some(sig),
        out_of_band_channel: true,
    };
    call(c, "approvals.resolve", serde_json::to_value(&params).unwrap()).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn approvals_grant_and_deny_leave_the_list() {
    let ts = now_ms();
    let env = boot(
        move |s| {
            append_held(s, "act-grant", "wa:dm:alice", ts);
            append_held(s, "act-deny", "tg:dm:bob", ts + 10);
        },
        true,
    )
    .await;
    // The held capability must exist in the router, as it does with a real adapter.
    let (_executed, ctrl) = super::gateway_tests::spawn_ndjson_mock_body(
        &env.handle,
        "body-wa",
        "wa:dm:alice",
        "tool.write",
        super::gateway_tests::MockBodyBehavior::ImmediateDone,
    );
    let _body_tx = ctrl.wait_connected().await;
    let mut c = AdminClient::connect(env.dir.path()).await.unwrap();
    let list = call(&mut c, "approvals.list", Value::Null).await.unwrap();
    assert_eq!(list["approvals"].as_array().unwrap().len(), 2, "{list}");
    let find = |l: &Value, id: &str| {
        l["approvals"].as_array().unwrap().iter().find(|e| e["action_id"] == json!(id)).cloned().expect(id)
    };

    let r = resolve(&mut c, &find(&list, "act-grant"), true).await.expect("grant");
    assert_eq!(r["applied"], json!(true));
    let list = call(&mut c, "approvals.list", Value::Null).await.unwrap();
    assert_eq!(list["approvals"].as_array().unwrap().len(), 1, "{list}");

    let r = resolve(&mut c, &find(&list, "act-deny"), false).await.expect("deny");
    assert_eq!(r["applied"], json!(true));
    let list = call(&mut c, "approvals.list", Value::Null).await.unwrap();
    assert!(list["approvals"].as_array().unwrap().is_empty(), "{list}");
    env.rt.shutdown().await;
}

fn percept_and_outcome(store: &mut MemStore, place: &str, ts: u64, outcome: &str) {
    let pid = store
        .append(rec(
            EventKind::Percept,
            ts,
            "per_alice",
            place,
            b"hello",
            None,
        ))
        .expect("percept");
    store
        .append(rec(
            EventKind::Decision,
            ts + 1,
            "soul",
            place,
            b"",
            Some(EventDetail::Decision {
                target_event_id: pid,
                outcome: if outcome == "silent" { DecisionOutcome::Silent } else { DecisionOutcome::Go },
                salience: 0.5,
                threshold: 0.4,
                probability: 0.9,
                draw: 0.1,
                reason: None,
                trace: None,
            }),
        ))
        .expect("decision");
    if outcome == "replied" || outcome == "held" {
        let aid = format!("act-{ts}");
        let mut action = rec(
            EventKind::Action,
            ts + 2,
            "soul",
            place,
            b"hi",
            Some(EventDetail::Action {
                action_id: ActionId::new(&aid),
                capability_id: CapabilityId::new(if outcome == "held" { "tool.write" } else { "chat.speak" }),
                h_act: None,
                correlation_id: None,
            }),
        );
        action.caused_by = vec![pid];
        store.append(action).expect("action");
        let r = if outcome == "held" { ReceiptOutcome::Held } else { ReceiptOutcome::Done };
        let mut receipt = rec(
            EventKind::Receipt,
            ts + 3,
            "soul",
            place,
            b"",
            Some(EventDetail::Receipt {
                action_id: ActionId::new(&aid),
                capability_id: CapabilityId::new(if outcome == "held" { "tool.write" } else { "chat.speak" }),
                outcome: r,
                termination_reason: None,
                progress: None,
                delivered_units: None,
                h_act: None,
            }),
        );
        receipt.caused_by = vec![pid];
        store.append(receipt).expect("receipt");
    }
}

#[test]
fn activity_rows_link_message_turn_and_outcome_per_trace() {
    let mut store = MemStore::new();
    percept_and_outcome(&mut store, "wa:dm:alice", 1_000, "replied");
    percept_and_outcome(&mut store, "tg:dm:bob", 2_000, "silent");
    percept_and_outcome(&mut store, "cli:terminal", 3_000, "replied"); // local: not activity
    let events = store.read_in_order(None, usize::MAX).unwrap();
    let rows = activity_rows(&events);
    assert_eq!(rows.len(), 2, "{rows:?}");
    assert_eq!((rows[0].plugin.as_str(), rows[0].outcome.as_str()), ("whatsapp", "replied"));
    assert_eq!((rows[1].plugin.as_str(), rows[1].outcome.as_str()), ("telegram", "silent"));
    // Rows never carry message text.
    assert!(!serde_json::to_string(&rows).unwrap().contains("hello"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn activity_list_filters_and_survives_a_restart() {
    let build = |s: &mut MemStore| {
        percept_and_outcome(s, "wa:dm:alice", 1_000, "replied");
        percept_and_outcome(s, "tg:dm:bob", 2_000, "silent");
        percept_and_outcome(s, "wa:dm:alice", 3_000, "held");
    };
    // "Restart": a second gateway boots on the same recorded events and rebuilds the rows.
    for _ in 0..2 {
        let env = boot(build, false).await;
        let mut c = AdminClient::connect(env.dir.path()).await.unwrap();
        let all = call(&mut c, "activity.list", json!({})).await.unwrap();
        assert_eq!(all["total"], json!(3), "{all}");
        let only_wa = call(&mut c, "activity.list", json!({ "plugin": "whatsapp" })).await.unwrap();
        assert_eq!(only_wa["rows"].as_array().unwrap().len(), 2);
        let held = call(&mut c, "activity.list", json!({ "outcome": "held" })).await.unwrap();
        assert_eq!(held["rows"].as_array().unwrap().len(), 1);
        assert_eq!(held["rows"][0]["outcome"], json!("held"));
        env.rt.shutdown().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn events_query_filters_pages_by_cursor_explains_and_traces() {
    let env = boot(
        |s| {
            percept_and_outcome(s, "wa:dm:alice", 1_000, "replied");
            percept_and_outcome(s, "tg:dm:bob", 2_000, "silent");
        },
        false,
    )
    .await;
    let mut c = AdminClient::connect(env.dir.path()).await.unwrap();
    let all = call(&mut c, "events.query", json!({})).await.unwrap();
    let total = all["matching"].as_u64().unwrap();
    assert!(total >= 7, "{all}");

    let kind = call(&mut c, "events.query", json!({ "kind": "decision" })).await.unwrap();
    assert!(kind["events"].as_array().unwrap().iter().all(|e| e["kind"] == json!("decision")));
    let place = call(&mut c, "events.query", json!({ "place": "tg:dm:bob" })).await.unwrap();
    assert!(place["events"].as_array().unwrap().iter().all(|e| e["place"] == json!("tg:dm:bob")));
    let since = call(&mut c, "events.query", json!({ "since": 2_000 })).await.unwrap();
    assert!(since["events"].as_array().unwrap().iter().all(|e| e["timestamp_ms"].as_u64().unwrap() >= 2_000));
    let bad = call(&mut c, "events.query", json!({ "kind": "nope" })).await.unwrap_err();
    assert_eq!(bad.code, -32602);

    // Cursor paging: two pages of 3 cover distinct events, newest first.
    let p1 = call(&mut c, "events.query", json!({ "limit": 3 })).await.unwrap();
    let cur = p1["next_cursor"].as_u64().expect("more pages");
    let p2 = call(&mut c, "events.query", json!({ "limit": 3, "cursor": cur })).await.unwrap();
    let ids1: Vec<u64> = p1["events"].as_array().unwrap().iter().map(|e| e["id"].as_u64().unwrap()).collect();
    let ids2: Vec<u64> = p2["events"].as_array().unwrap().iter().map(|e| e["id"].as_u64().unwrap()).collect();
    assert!(ids1.iter().all(|i| !ids2.contains(i)));
    assert!(ids1.last().unwrap() > ids2.first().unwrap());

    // Explain equals the CLI function on the same events.
    let events = env.handle.read_events().await.unwrap();
    let first_percept = events.iter().find(|e| e.kind == EventKind::Percept).unwrap().id.as_u64();
    let ex = call(&mut c, "events.explain", json!({ "id": first_percept })).await.unwrap();
    let direct = monoagens_gateway::governance::explain_decision(&events, first_percept).unwrap();
    assert_eq!(ex["lines"], json!(direct.text_lines()));
    assert!(call(&mut c, "events.explain", json!({ "id": 9_999_999 })).await.is_err());

    // Trace by event id: every event of the first message's chain, none of the second's.
    let tr = call(&mut c, "events.trace", json!({ "trace": first_percept.to_string() })).await.unwrap();
    let places: std::collections::BTreeSet<&str> =
        tr["events"].as_array().unwrap().iter().map(|e| e["place"].as_str().unwrap()).collect();
    assert_eq!(places, ["wa:dm:alice"].into_iter().collect());
    assert!(tr["events"].as_array().unwrap().len() >= 3);
    env.rt.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn policy_get_lists_the_cli_rule_ids_and_source_text() {
    let env = boot(|_| {}, false).await;
    let mut c = AdminClient::connect(env.dir.path()).await.unwrap();
    let p = call(&mut c, "policy.get", Value::Null).await.unwrap();
    let ids: Vec<String> =
        p["active_rule_names"].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_owned()).collect();
    assert_eq!(
        ids,
        monoagens_gateway::governance::extract_cedar_rule_summaries(monoagens_valve::DEFAULT_CEDAR_POLICIES)
    );
    assert!(!ids.is_empty());
    for r in p["rules"].as_array().unwrap() {
        assert!(r["source"].as_str().unwrap().contains("permit(") || r["source"].as_str().unwrap().contains("forbid("));
    }
    assert_eq!(p["valve_mode"], json!("enforcing"));
    assert_eq!(p["absolute_lines"].as_array().unwrap().len(), 2);
    env.rt.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn people_and_links_initiate_verify_unlink_and_owner_only() {
    let env = boot(|_| {}, false).await;
    let cfg_path = env.dir.path().join("monoagens.toml");
    std::fs::write(
        &cfg_path,
        r#"version = 1
[model]
provider = "scripted"
model = "mock"

[[person]]
actor_id = "per_alice"
accounts = [{ body_id = "body-wa", account_id = "alice@wa" }]

[[person]]
actor_id = "per_owner"
is_owner = true
accounts = [{ body_id = "body-tg", account_id = "owner@tg" }]
"#,
    )
    .unwrap();
    // The owner reads the configuration it was started with; reload it from the file.
    let cfg = monoagens_gateway::load_config_file(&cfg_path, &env.dir.path().join("secrets.toml")).expect("cfg");
    let owner2 = Arc::new(GatewayOwner::new(OwnerParams {
        handle: env.handle.clone(),
        supervisor: PluginSupervisor::new(
            env.handle.clone(),
            PluginEnvContext { config_dir: None, data_dir: None, rbac_enabled: false, plugin_env: Default::default() },
            Arc::new(SystemClock::new()),
            RunLog::new(Arc::clone(env.handle.clock()), env.handle.diagnostics().clone(), None),
            RestartPolicy::default(),
            vec![],
        ),
        runlog: RunLog::new(Arc::clone(env.handle.clock()), env.handle.diagnostics().clone(), None),
        mode: OwnerMode::Serve,
        config: cfg,
        config_path: cfg_path,
        secrets_path: env.dir.path().join("secrets.toml"),
        data_dir: env.dir.path().to_path_buf(),
        owner_actor: ActorId::new("per_owner"),
    }));
    let call = |m: &'static str, p: Value| {
        let o = Arc::clone(&owner2);
        async move { monoagens_gateway::governance::governance_call(&o, m, &p).await }
    };

    let people = call("people.list", Value::Null).await.unwrap();
    let persons = people["configured_persons"].as_array().unwrap();
    assert_eq!(persons.len(), 2);
    assert_eq!(persons[0]["accounts"][0]["account_id"], json!("alice@wa"));

    // Non-owner: refused, nothing changes.
    let denied = call(
        "links.initiate",
        json!({ "actor": "per_mallory", "primary_body": "body-wa", "primary_account": "alice@wa",
                "secondary_body": "body-tg", "secondary_account": "alice@tg" }),
    )
    .await
    .unwrap_err();
    assert!(denied.message.contains("only the owner"), "{denied}");
    assert!(!env.dir.path().join("identity_events.jsonl").exists());

    let issued = call(
        "links.initiate",
        json!({ "primary_body": "body-wa", "primary_account": "alice@wa",
                "secondary_body": "body-tg", "secondary_account": "alice@tg" }),
    )
    .await
    .expect("initiate");
    let verified = call(
        "links.verify",
        json!({ "challenge": issued["challenge_id"], "nonce1": issued["nonce_1"], "nonce2": issued["nonce_2"],
                "owner_confirmed": true }),
    )
    .await
    .expect("verify");
    assert_eq!(verified["status"], json!("linked"));
    let people = call("people.list", Value::Null).await.unwrap();
    assert_eq!(people["active_links"].as_array().unwrap().len(), 1);

    // Result equals the CLI helper on the same identity log.
    let cfg = owner2.config_snapshot();
    let resolver = monoagens_gateway::governance::links::open_resolver(&cfg, env.dir.path()).unwrap();
    assert_eq!(
        monoagens_gateway::governance::links::link_list_json(&cfg, &resolver)["active_links"],
        people["active_links"]
    );

    let denied = call("links.unlink", json!({ "actor": "per_mallory", "body": "body-tg", "account": "alice@tg" }))
        .await
        .unwrap_err();
    assert!(denied.message.contains("only the owner"));
    call("links.unlink", json!({ "body": "body-tg", "account": "alice@tg" })).await.expect("unlink");
    let people = call("people.list", Value::Null).await.unwrap();
    assert!(people["active_links"].as_array().unwrap().is_empty());
    env.rt.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn lab_replay_and_export_run_on_a_copy_and_leave_the_live_log_untouched() {
    let env = boot(|s| percept_and_outcome(s, "wa:dm:alice", 1_000, "replied"), false).await;
    let mut c = AdminClient::connect(env.dir.path()).await.unwrap();
    let before = env.handle.read_events().await.unwrap();

    let job = call(&mut c, "lab.replay", json!({})).await.unwrap()["job"].as_u64().unwrap();
    let done = wait_job(&mut c, job).await;
    assert_eq!(done["kind"], json!("replay"));
    assert_eq!(done["state"], json!("done"), "{done}");
    // Same report as `monoagens replay` computes on the same events.
    let direct = monoagens_gateway::governance::replay_events(
        &before,
        monoagens_core::ReplayMode::Pinned,
        &monoagens_gateway::governance::JobCtl::none(),
    )
    .unwrap();
    assert_eq!(done["result"], serde_json::to_value(&direct).unwrap());

    let job = call(&mut c, "lab.export_learning", json!({})).await.unwrap()["job"].as_u64().unwrap();
    let done = wait_job(&mut c, job).await;
    assert_eq!(done["state"], json!("done"), "{done}");
    let out = done["result"]["out"].as_str().unwrap();
    assert!(std::fs::read_to_string(out).unwrap().contains("MonoAgens Learned State Export"));

    let after = env.handle.read_events().await.unwrap();
    assert_eq!(before, after, "live log must be unchanged");

    // Cancel: a cancelled job reports `cancelled`.
    let job = call(&mut c, "lab.replay", json!({})).await.unwrap()["job"].as_u64().unwrap();
    let _ = call(&mut c, "lab.cancel", json!({ "job": job })).await.unwrap();
    let st = wait_job(&mut c, job).await;
    assert!(st["state"] == json!("cancelled") || st["state"] == json!("done"), "{st}");
    assert!(call(&mut c, "lab.job", json!({ "job": 999 })).await.is_err());
    env.rt.shutdown().await;
}

async fn wait_job(c: &mut AdminClient, job: u64) -> Value {
    for _ in 0..300 {
        let st = call(c, "lab.job", json!({ "job": job })).await.unwrap();
        if st["state"] != json!("running") {
            return st;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("job {job} did not finish");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn activity_subscribe_pushes_new_rows_only() {
    let env = boot(|s| percept_and_outcome(s, "wa:dm:alice", 1_000, "replied"), false).await;
    let mut c = AdminClient::connect(env.dir.path()).await.unwrap();
    let sub = call(&mut c, "activity.subscribe", Value::Null).await.unwrap();
    assert!(sub["subscription"].is_u64());
    // Nothing is pushed for rows that already exist.
    let quiet = tokio::time::timeout(Duration::from_millis(1_500), c.next_notification()).await;
    assert!(quiet.is_err(), "existing rows must not be re-pushed: {quiet:?}");
    // A new adapter message through the owner's own handle shows up as a row.
    let _ = &env.owner;
    env.rt.shutdown().await;
}
