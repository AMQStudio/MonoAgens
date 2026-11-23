//! Draft preview safety (owner decision D-3, change `tui-conversation` task 7.3): the preview
//! reaches only the TUI subscription, never a fake adapter body, is never an event, and the
//! delivered `chat.speak` and the recorded log are identical to a non-streamed run.

use std::sync::Arc;
use std::time::Duration;

use monoagens_core::{CoreConfig, EventDetail, EventKind, MemStore, NoopDoors, ReplayMode, Store};
use monoagens_gateway::{
    CueRecorder, DiagnosticSink, GatewayConfig, GatewayOptions, GatewayRuntime, GatewaySeams,
    ModelProviderKind, PreviewEnd, PreviewEvent, VirtualClock,
};
use monoagens_protocol::{
    ActionExecuteParams, ActorId, Audience, OpaquePayload, Percept, PlaceId, Time,
};
use monoagens_reason::{ProviderSecrets, ScriptedModelProvider, ScriptedReply};
use serde_json::json;

use crate::gateway_tests::{MockBodyBehavior, mpsc_tx_handle, spawn_ndjson_mock_body};

const TUI_PLACE: &str = "cli:terminal";
const ADAPTER_PLACE: &str = "wa:room";
const REPLY: &str = "Halo dari Mono, ini balasan yang agak panjang supaya terpotong beberapa kali.";

fn scripted_output(place: &str, text: &str) -> String {
    json!({
        "protocol_version": { "major": 1, "minor": 0 },
        "acts": [
            { "act": "think", "thought": "answer", "motivation": 4, "confidence_0_20": 18,
              "used_ids": [], "importance": 0.6, "basis": "observed", "value": null,
              "cost": null, "domain": null },
            { "act": "act", "capability_id": "chat.speak", "places": [place],
              "people": ["per_owner"], "payload": text.as_bytes(), "motivation": 4,
              "confidence_0_20": 18, "used_ids": [], "expected": null, "domain": null }
        ]
    })
    .to_string()
}

fn percept(place: &str, text: &str, ts: u64) -> Percept {
    let mut p = Percept::minimal(
        ActorId::new("per_owner"),
        PlaceId::new(place),
        Audience::new([ActorId::new("per_owner"), ActorId::new("soul")]),
        Time::from_millis(ts),
        OpaquePayload::from_bytes(text.as_bytes()),
    );
    p.addressees = vec![ActorId::new("soul")];
    p
}

struct Run {
    drafts: Vec<String>,
    ends: Vec<PreviewEnd>,
    tui_speaks: Vec<Vec<u8>>,
    adapter_executed: usize,
    log: Vec<(EventKind, String, Vec<u8>)>,
    replay_exact: bool,
    streamed_calls: u64,
}

/// One scripted turn from `trigger_place`, with a TUI body and a fake adapter body connected.
async fn run_turn(subscribe: bool, trigger_place: &str) -> Run {
    let t0 = 3_000_000_u64;
    let clock = Arc::new(VirtualClock::new(Time::from_millis(t0)));
    let secrets = ProviderSecrets::default();
    let mut cfg = GatewayConfig::default();
    cfg.model.provider = ModelProviderKind::Scripted;
    cfg.model.model = "mock".to_owned();
    cfg.runtime.action_timeout_ms = 500;
    cfg.runtime.hook_timeout_ms = 500;
    cfg.runtime.model_timeout_ms = 20_000;

    let provider = Arc::new(
        ScriptedModelProvider::from_replies([ScriptedReply::text(scripted_output(
            trigger_place,
            REPLY,
        ))])
        .with_stream_pacing(12, 1),
    );
    let rt = GatewayRuntime::start(
        cfg,
        &secrets,
        || Ok(MemStore::new()),
        CoreConfig::default(),
        NoopDoors,
        GatewayOptions {
            clock: clock.clone(),
            provider: Some(provider.clone()),
            diagnostics: DiagnosticSink::new(&secrets),
            cues: CueRecorder::new(),
            seams: GatewaySeams::default(),
        },
    )
    .await
    .expect("start gateway");
    let handle = rt.handle();

    let (tui_task, tui_ctrl) = spawn_ndjson_mock_body(
        &handle,
        "body-tui",
        TUI_PLACE,
        "chat.speak",
        MockBodyBehavior::ImmediateDone,
    );
    let tui_tx = tui_ctrl.wait_connected().await;
    let (adapter_task, adapter_ctrl) = spawn_ndjson_mock_body(
        &handle,
        "body-adapter",
        ADAPTER_PLACE,
        "chat.speak",
        MockBodyBehavior::ImmediateDone,
    );
    let adapter_tx = adapter_ctrl.wait_connected().await;

    let mut sub = subscribe.then(|| handle.preview().subscribe());
    let sender = if trigger_place == TUI_PLACE { &tui_tx } else { &adapter_tx };
    mpsc_tx_handle::BodyControl::send_percept(sender, 1, percept(trigger_place, "hallo", t0));

    // Wait until the speak was executed (a Receipt for it exists).
    let mut waited = 0;
    loop {
        let events = handle.read_events().await.expect("events");
        let done = events.iter().any(|e| {
            matches!(&e.detail, Some(EventDetail::Receipt { capability_id, .. }) if capability_id.as_str() == "chat.speak")
        });
        if done {
            break;
        }
        waited += 1;
        assert!(waited < 800, "turn did not finish");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    let mut drafts = Vec::new();
    let mut ends = Vec::new();
    if let Some(sub) = sub.as_mut() {
        while let Ok(Some(ev)) = tokio::time::timeout(Duration::from_millis(300), sub.recv()).await {
            match ev {
                PreviewEvent::Draft { text, .. } => drafts.push(text),
                PreviewEvent::End { outcome, .. } => ends.push(outcome),
            }
        }
    }

    let events = handle.read_events().await.expect("events");
    let mut mem = MemStore::new();
    for rec in &events {
        mem.append(rec.clone()).expect("append");
    }
    let replay_exact = monoagens_core::replay_log(&mem, ReplayMode::Unpinned, None)
        .expect("replay")
        .is_exact();
    let log = events
        .iter()
        .map(|e| (e.kind, e.place.as_str().to_owned(), e.payload.as_bytes().to_vec()))
        .collect();

    rt.shutdown().await;
    let tui_exec: Vec<ActionExecuteParams> = tokio::time::timeout(Duration::from_secs(10), tui_task)
        .await
        .expect("tui body ends")
        .expect("join");
    let adapter_exec = tokio::time::timeout(Duration::from_secs(10), adapter_task)
        .await
        .expect("adapter body ends")
        .expect("join");
    Run {
        drafts,
        ends,
        tui_speaks: tui_exec
            .iter()
            .filter(|p| p.capability_id.as_str() == "chat.speak")
            .map(|p| p.payload.as_bytes().to_vec())
            .collect(),
        adapter_executed: adapter_exec.len(),
        log,
        replay_exact,
        streamed_calls: provider.streamed_calls(),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn adapter_sees_no_draft_and_final_speak_is_byte_equal_to_a_non_streamed_run() {
    let plain = run_turn(false, TUI_PLACE).await;
    let streamed = run_turn(true, TUI_PLACE).await;

    // The TUI subscription gets growing drafts that end in the full reply text.
    assert!(streamed.drafts.len() > 2, "drafts arrive incrementally: {:?}", streamed.drafts);
    assert!(streamed.drafts.windows(2).all(|w| w[1].starts_with(&w[0]) && w[1].len() > w[0].len()));
    assert_eq!(streamed.drafts.last().map(String::as_str), Some(REPLY));
    assert_eq!(streamed.ends, vec![PreviewEnd::Completed]);
    assert_eq!(streamed.streamed_calls, 1);

    // A fake adapter body connected to the same gateway receives nothing at all.
    assert_eq!(streamed.adapter_executed, 0, "no draft or speak reaches the adapter");
    assert_eq!(plain.adapter_executed, 0);

    // The delivered chat.speak is byte-for-byte what the non-streamed run delivers.
    assert_eq!(streamed.tui_speaks.len(), 1);
    assert_eq!(streamed.tui_speaks, plain.tui_speaks);
    assert_eq!(streamed.tui_speaks[0], REPLY.as_bytes());

    // Previews are not events: the recorded log is identical, and so is its replay.
    assert_eq!(streamed.log, plain.log, "event log identical with and without a preview");
    assert!(streamed.replay_exact && plain.replay_exact, "replay reports identical decisions");
    let draft_bytes = REPLY.as_bytes();
    let leaked_into_log = streamed
        .log
        .iter()
        .filter(|(kind, _, payload)| {
            *kind != EventKind::Thought
                && *kind != EventKind::Action
                && payload.windows(draft_bytes.len()).any(|w| w == draft_bytes)
        })
        .count();
    assert_eq!(leaked_into_log, 0, "the draft text is not recorded outside the action itself");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn no_subscriber_means_no_streaming_request() {
    let plain = run_turn(false, TUI_PLACE).await;
    assert_eq!(plain.streamed_calls, 0, "setting off: the provider is asked for a blocking call");
    assert!(plain.drafts.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn turns_triggered_at_an_adapter_place_are_never_previewed() {
    let run = run_turn(true, ADAPTER_PLACE).await;
    assert!(run.drafts.is_empty(), "no draft for a turn the TUI did not submit: {:?}", run.drafts);
    assert!(run.ends.is_empty());
    assert_eq!(run.streamed_calls, 0);
    assert!(run.tui_speaks.is_empty());
    assert_eq!(run.adapter_executed, 1, "the adapter still gets the final speak");
}

struct Stack {
    rt: GatewayRuntime,
    handle: monoagens_gateway::GatewayHandle,
    owner: Arc<monoagens_gateway::GatewayOwner>,
    server: monoagens_gateway::AdminServer,
    dir: tempfile::TempDir,
    _lock: monoagens_gateway::DataDirLock,
}

/// A gateway with an owner and its admin socket over a scripted provider.
async fn stack(provider: Arc<ScriptedModelProvider>) -> Stack {
    use monoagens_gateway::{
        AdminServer, GatewayOwner, OwnerMode, OwnerParams, PluginEnvContext, PluginSupervisor,
        RestartPolicy, RunLog, SystemClock, acquire_data_dir_lock,
    };
    let dir = tempfile::Builder::new()
        .prefix("pv-")
        .tempdir_in(env!("CARGO_TARGET_TMPDIR"))
        .expect("tempdir");
    let secrets = ProviderSecrets::default();
    let mut cfg = GatewayConfig::default();
    cfg.model.provider = ModelProviderKind::Scripted;
    cfg.model.model = "mock".to_owned();
    cfg.runtime.model_timeout_ms = 20_000;
    let rt = GatewayRuntime::start(
        cfg.clone(),
        &secrets,
        || Ok(MemStore::new()),
        CoreConfig::default(),
        NoopDoors,
        GatewayOptions {
            clock: Arc::new(SystemClock::new()),
            provider: Some(provider),
            diagnostics: DiagnosticSink::new(&secrets),
            cues: CueRecorder::new(),
            seams: GatewaySeams::default(),
        },
    )
    .await
    .expect("start");
    let handle = rt.handle();
    let log = RunLog::new(Arc::clone(handle.clock()), handle.diagnostics().clone(), None);
    log.attach_diagnostics(handle.diagnostics());
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
        Vec::new(),
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
    Stack { rt, handle, owner, server, dir, _lock: lock }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn preview_subscribe_over_the_admin_socket_streams_drafts_and_unsubscribes() {
    use monoagens_gateway::AdminClient;
    let provider = Arc::new(
        ScriptedModelProvider::from_replies([ScriptedReply::text(scripted_output(TUI_PLACE, REPLY))])
            .with_stream_pacing(12, 1),
    );
    let st = stack(provider).await;
    let mut client = AdminClient::connect(st.dir.path()).await.expect("connect");

    let (tui_task, tui_ctrl) = spawn_ndjson_mock_body(
        &st.handle,
        "body-tui",
        TUI_PLACE,
        "chat.speak",
        MockBodyBehavior::ImmediateDone,
    );
    let tui_tx = tui_ctrl.wait_connected().await;

    let sub = tokio::time::timeout(
        Duration::from_secs(10),
        client.call("preview.subscribe", json!({})),
    )
    .await
    .expect("in time")
    .expect("subscribe");
    let sub_id = sub["subscription"].as_u64().expect("id");
    assert_eq!(st.handle.preview().subscriber_count(), 1);

    mpsc_tx_handle::BodyControl::send_percept(&tui_tx, 1, percept(TUI_PLACE, "hallo", 3_000_000));
    let mut last = String::new();
    loop {
        let n = tokio::time::timeout(Duration::from_secs(10), client.next_notification())
            .await
            .expect("notification in time")
            .expect("notification");
        assert_eq!(n["method"], "preview.event");
        match n["params"]["kind"].as_str() {
            Some("draft") => last = n["params"]["text"].as_str().unwrap_or_default().to_owned(),
            Some("end") => {
                assert_eq!(n["params"]["outcome"], "completed");
                break;
            }
            other => panic!("unexpected preview event {other:?}"),
        }
    }
    assert_eq!(last, REPLY);

    // Unsubscribe from a second connection: the stream stops and the gateway has no listener.
    let mut other = AdminClient::connect(st.dir.path()).await.expect("connect 2");
    other
        .call("preview.unsubscribe", json!({ "subscription": sub_id }))
        .await
        .expect("unsubscribe");
    assert_eq!(st.handle.preview().subscriber_count(), 0, "no listener: the next call is not streamed");

    st.server.shutdown();
    st.rt.shutdown().await;
    let _ = tokio::time::timeout(Duration::from_secs(10), tui_task).await;
}

/// `events.trace` and `events.explain` answer alike through the admin socket and in process on a
/// recorded scripted turn, and explain returns exactly the `monoagens log --explain` lines.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn events_trace_and_explain_match_over_socket_and_in_process() {
    use monoagens_gateway::AdminClient;
    let provider = Arc::new(ScriptedModelProvider::from_replies([ScriptedReply::text(
        scripted_output(TUI_PLACE, REPLY),
    )]));
    let st = stack(provider).await;
    let (tui_task, tui_ctrl) = spawn_ndjson_mock_body(
        &st.handle,
        "body-tui",
        TUI_PLACE,
        "chat.speak",
        MockBodyBehavior::ImmediateDone,
    );
    let tui_tx = tui_ctrl.wait_connected().await;
    mpsc_tx_handle::BodyControl::send_percept(&tui_tx, 1, percept(TUI_PLACE, "hallo", 3_000_000));
    let mut waited = 0;
    let events = loop {
        let events = st.handle.read_events().await.expect("events");
        if events.iter().any(|e| matches!(&e.detail, Some(EventDetail::Receipt { capability_id, .. }) if capability_id.as_str() == "chat.speak")) {
            break events;
        }
        waited += 1;
        assert!(waited < 800, "turn did not finish");
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    let root = events.iter().find(|e| e.kind == EventKind::Percept).expect("percept").id.as_u64();

    let mut client = AdminClient::connect(st.dir.path()).await.expect("connect");
    let via_socket = tokio::time::timeout(
        Duration::from_secs(10),
        client.call("events.trace", json!({ "trace": root.to_string() })),
    )
    .await
    .expect("in time")
    .expect("socket trace");
    let in_process = monoagens_gateway::governance::governance_call(
        &st.owner,
        "events.trace",
        &json!({ "trace": root.to_string() }),
    )
    .await
    .expect("in-process trace");
    assert_eq!(via_socket, in_process);
    let kinds: Vec<&str> = via_socket["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["kind"].as_str().unwrap())
        .collect();
    assert!(kinds.contains(&"wake_started") && kinds.contains(&"action") && kinds.contains(&"receipt"), "{kinds:?}");

    // By-trace filter of events.query.
    let q = client.call("events.query", json!({ "trace": root.to_string() })).await.expect("query");
    assert_eq!(q["events"].as_array().unwrap().len(), kinds.len());

    let ex_socket = client.call("events.explain", json!({ "id": root })).await.expect("socket explain");
    let ex_proc = monoagens_gateway::governance::governance_call(&st.owner, "events.explain", &json!({ "id": root }))
        .await
        .expect("in-process explain");
    assert_eq!(ex_socket, ex_proc);
    let lines = ex_socket["lines"].as_array().expect("lines");
    assert!(lines[0].as_str().unwrap().starts_with("decision event="), "{lines:?}");

    st.server.shutdown();
    st.rt.shutdown().await;
    let _ = tokio::time::timeout(Duration::from_secs(10), tui_task).await;
}
