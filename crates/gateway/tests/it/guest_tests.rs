//! Guest access end to end: an unregistered adapter sender is admitted as a guest, RBAC decides
//! whether the agent wakes, gateway slash commands answer without the model, and guests are
//! recognised again after a restart because they are rebuilt from the event log.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use monoagens_core::{CoreConfig, Doors as _, EventDetail, EventKind, MemStore};
use monoagens_gateway::{
    AdminClient, AdminServer, BodyTransportConfig, CapabilityCache, CueRecorder, DiagnosticSink,
    DoorController as _, EmbedBackendKind, GatewayConfig, GatewayHandle, GatewayOptions,
    GatewayOwner, GatewayRuntime, GatewaySeams, ModelProviderKind, OwnerMode, OwnerParams,
    PluginConfig, PluginEnvContext, PluginKind, PluginSupervisor, PreQueueService, RestartPolicy,
    RunLog, SystemClock, ValveDoorController, VirtualClock, acquire_data_dir_lock,
    build_embedder_from_config, load_config_file,
};
use monoagens_protocol::{
    ActionExecuteParams, ActionExecuteResult, ActorId, Audience, BodyDiscoverParams,
    BodyDiscoverResult, BodyId, BodyShutdownResult, CURRENT_PROTOCOL_VERSION,
    CapabilityDescriptor, CapabilityFlags, CapabilityId, CapabilitySchemaHash, OpaquePayload,
    Percept, PlaceId, Receipt, ReceiptOutcome, TerminationReason, Time, Waist2Method,
    Waist2Request, Waist2Response,
};
use monoagens_reason::{ProviderSecrets, ScriptedModelProvider};
use monoagens_valve::{
    PERMISSION_AGENT_CONVERSE, RbacConfig, RoleDefinition, RoleTier, Valve, ValveConfig,
    ValveSnapshot,
};
use serde_json::json;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader as TokioBufReader};
use tokio::sync::Notify;

use crate::gateway_tests::{MockBodyBehavior, mpsc_tx_handle, spawn_ndjson_mock_body};

const SENDER: &str = "wa:user:6285111242960@s.whatsapp.net";
const PLACE: &str = "wa:dm:6285111242960@s.whatsapp.net";

fn guest_rbac(guest_allow: &[&str]) -> RbacConfig {
    let mut cfg = RbacConfig {
        fallback_role: Some("guest".to_owned()),
        ..RbacConfig::default()
    };
    cfg.roles.insert(
        "guest".to_owned(),
        RoleDefinition {
            name: "guest".to_owned(),
            base: RoleTier::Guest,
            allow: guest_allow.iter().map(|s| (*s).to_owned()).collect(),
            deny: Vec::new(),
            approval: None,
            label: None,
            context: None,
            settings: BTreeMap::new(),
        },
    );
    cfg
}

async fn controller(rbac: &RbacConfig) -> ValveDoorController {
    let rbac = rbac.clone();
    tokio::task::spawn_blocking(move || build_controller(&rbac))
        .await
        .expect("valve task")
}

fn build_controller(rbac: &RbacConfig) -> ValveDoorController {
    let mut snap = ValveSnapshot::new(monoagens_core::LogSeq::ZERO, Time::from_millis(1_000_000));
    snap.owner_actor_id = Some(ActorId::new("per_owner"));
    snap.known_actors.insert(ActorId::new("per_owner"));
    snap.known_actors.insert(ActorId::new("soul"));
    let cfg = ValveConfig {
        guest_access: true,
        rbac: Some(rbac.clone()),
        ..ValveConfig::default()
    };
    ValveDoorController::new(Valve::with_default_policies(cfg, snap).expect("valve"))
}

fn percept(text: &str) -> Percept {
    let mut p = Percept::minimal(
        ActorId::new(SENDER),
        PlaceId::new(PLACE),
        Audience::new([ActorId::new(SENDER), ActorId::new("soul")]),
        Time::from_millis(1_000_000),
        OpaquePayload::from_bytes(text.as_bytes()),
    );
    p.addressees = vec![ActorId::new("soul")];
    p
}

async fn start(rbac: &RbacConfig) -> (GatewayRuntime, GatewayHandle) {
    let mut cfg = GatewayConfig::default();
    cfg.model.provider = ModelProviderKind::Scripted;
    cfg.model.model = "mock".to_owned();
    cfg.runtime.action_timeout_ms = 2_000;
    cfg.rbac = Some(rbac.clone());
    let clock = Arc::new(VirtualClock::new(Time::from_millis(1_000_000)));
    let secrets = ProviderSecrets::default();
    let rt = GatewayRuntime::start(
        cfg,
        &secrets,
        || Ok(MemStore::new()),
        CoreConfig::default(),
        controller(rbac).await,
        GatewayOptions {
            clock,
            provider: Some(Arc::new(ScriptedModelProvider::new())),
            diagnostics: DiagnosticSink::new(&secrets),
            cues: CueRecorder::new(),
            seams: GatewaySeams::default(),
        },
    )
    .await
    .expect("start gateway");
    let handle = rt.handle();
    (rt, handle)
}

async fn wait_for<F: Fn(&[monoagens_core::EventRecord]) -> bool>(handle: &GatewayHandle, f: F) {
    for _ in 0..400 {
        let events = handle.read_events().await.expect("events");
        if f(&events) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("condition not reached within 10 s");
}

fn reply_texts(executed: &[monoagens_protocol::ActionExecuteParams]) -> Vec<String> {
    executed
        .iter()
        .map(|e| {
            serde_json::from_slice::<serde_json::Value>(e.payload.as_bytes())
                .ok()
                .and_then(|v| v.get("text").and_then(|t| t.as_str().map(str::to_owned)))
                .unwrap_or_default()
        })
        .collect()
}

#[tokio::test]
async fn guest_commands_are_answered_without_the_model_and_halo_is_refused() {
    let rbac = guest_rbac(&[]);
    let (rt, handle) = start(&rbac).await;
    let (body_task, ctrl) = spawn_ndjson_mock_body(
        &handle,
        "body-whatsapp",
        PLACE,
        "chat.speak",
        MockBodyBehavior::ImmediateDone,
    );
    let tx = ctrl.wait_connected().await;

    mpsc_tx_handle::BodyControl::send_percept(&tx, 1, percept("/whoami"));
    mpsc_tx_handle::BodyControl::send_percept(&tx, 2, percept("/help"));
    mpsc_tx_handle::BodyControl::send_percept(&tx, 3, percept("halo"));

    // `halo` is refused for lack of `agent.converse`, recorded in the event log.
    wait_for(&handle, |events| {
        events.iter().any(|e| {
            matches!(
                &e.detail,
                Some(EventDetail::DoorVerdict { verdict, .. }) if verdict == "refuse: agent_not_permitted"
            )
        })
    })
    .await;
    tokio::time::sleep(Duration::from_millis(200)).await;

    let events = handle.read_events().await.expect("events");
    assert!(
        !events
            .iter()
            .any(|e| matches!(e.kind, EventKind::ModelCall | EventKind::WakeStarted)),
        "no model call may happen for a guest without agent.converse"
    );
    // The two commands never reached the core: only `halo` is a recorded percept.
    assert_eq!(events.iter().filter(|e| e.kind == EventKind::Percept).count(), 1);

    rt.shutdown().await;
    let executed = body_task.await.expect("body task");
    let texts = reply_texts(&executed);
    assert_eq!(texts.len(), 2, "one reply per command: {texts:?}");
    let whoami = texts.iter().find(|t| t.contains("account:")).expect("whoami reply");
    assert!(whoami.contains(SENDER), "{whoami}");
    assert!(whoami.contains("role: guest"), "{whoami}");
    assert!(whoami.contains("use the agent: no"), "{whoami}");
    let help = texts.iter().find(|t| t.contains("Available commands")).expect("help reply");
    assert!(help.contains("/whoami") && help.contains("/help"), "{help}");
    assert!(!help.contains("/roles"), "guests must not see admin commands: {help}");
    assert!(executed.iter().all(|e| e.capability_id.as_str() == "chat.speak"));
}

#[tokio::test]
async fn guest_facts_are_rebuilt_from_events_after_a_restart() {
    let rbac = guest_rbac(&[PERMISSION_AGENT_CONVERSE]);
    let (rt, handle) = start(&rbac).await;
    let (body_task, ctrl) = spawn_ndjson_mock_body(
        &handle,
        "body-whatsapp",
        PLACE,
        "chat.speak",
        MockBodyBehavior::ImmediateDone,
    );
    let tx = ctrl.wait_connected().await;
    mpsc_tx_handle::BodyControl::send_percept(&tx, 1, percept("halo"));
    wait_for(&handle, |events| {
        events.iter().any(|e| e.kind == EventKind::Percept && e.actor.as_str() == SENDER)
    })
    .await;
    let events = handle.read_events().await.expect("events");
    rt.shutdown().await;
    let _ = body_task.await;

    // A fresh controller (a restart) knows no guest until the log is replayed.
    let mut restarted = controller(&rbac).await;
    assert!(restarted.valve().snapshot().guest_actors.is_empty());
    restarted.replay_from_store_events(&events);
    assert!(restarted.valve().snapshot().guest_actors.contains(&ActorId::new(SENDER)));
    let mut p = percept("halo again");
    assert_eq!(restarted.inbound(&mut p), monoagens_core::DoorDecision::Allow);
}

#[tokio::test]
async fn guest_with_agent_converse_wakes_the_agent_and_gets_a_reply() {
    let rbac = guest_rbac(&[PERMISSION_AGENT_CONVERSE, "chat.speak"]);
    let mut cfg = GatewayConfig::default();
    cfg.model.provider = ModelProviderKind::Scripted;
    cfg.model.model = "mock".to_owned();
    cfg.runtime.action_timeout_ms = 2_000;
    cfg.rbac = Some(rbac.clone());
    let reply = serde_json::json!({
        "protocol_version": { "major": 1, "minor": 0 },
        "acts": [
            {
                "act": "think", "thought": "greet the guest", "motivation": 4,
                "confidence_0_20": 18, "used_ids": [], "importance": 0.6,
                "basis": "observed", "value": null, "cost": null, "domain": null
            },
            {
                "act": "act", "capability_id": "chat.speak", "places": [PLACE],
                "people": [SENDER], "payload": [104, 105], "motivation": 4,
                "confidence_0_20": 18, "used_ids": [], "expected": null, "domain": null
            }
        ]
    })
    .to_string();
    let secrets = ProviderSecrets::default();
    let rt = GatewayRuntime::start(
        cfg,
        &secrets,
        || Ok(MemStore::new()),
        CoreConfig::default(),
        controller(&rbac).await,
        GatewayOptions {
            clock: Arc::new(VirtualClock::new(Time::from_millis(1_000_000))),
            provider: Some(Arc::new(ScriptedModelProvider::from_replies([
                monoagens_reason::ScriptedReply::text(reply),
            ]))),
            diagnostics: DiagnosticSink::new(&secrets),
            cues: CueRecorder::new(),
            seams: GatewaySeams::default(),
        },
    )
    .await
    .expect("start gateway");
    let handle = rt.handle();
    let (body_task, ctrl) = spawn_ndjson_mock_body(
        &handle,
        "body-whatsapp",
        PLACE,
        "chat.speak",
        MockBodyBehavior::ImmediateDone,
    );
    let tx = ctrl.wait_connected().await;
    mpsc_tx_handle::BodyControl::send_percept(&tx, 1, percept("halo"));
    wait_for(&handle, |events| {
        events.iter().any(|e| {
            matches!(
                &e.detail,
                Some(EventDetail::Receipt { capability_id, outcome: monoagens_protocol::ReceiptOutcome::Done, .. })
                    if capability_id.as_str() == "chat.speak"
            )
        })
    })
    .await;
    rt.shutdown().await;
    let executed = body_task.await.expect("body task");
    assert_eq!(executed.len(), 1);
    assert_eq!(executed[0].capability_id.as_str(), "chat.speak");
}

fn make_cap(
    id: &str,
    description: &str,
    places: &[&str],
    communicative: bool,
    identity_changing: bool,
    irreversible: bool,
) -> CapabilityDescriptor {
    CapabilityDescriptor {
        id: CapabilityId::new(id),
        description: description.to_owned(),
        places: places.iter().map(|p| PlaceId::new(*p)).collect(),
        flags: CapabilityFlags {
            communicative,
            identity_changing,
            irreversible,
        },
        typical_latency_ms: 10,
        capability_schema_hash: CapabilitySchemaHash::new(format!("hash:{id}")),
        sensed_effect: None,
    }
}

fn spawn_mock_body_with_caps(
    handle: &GatewayHandle,
    body_id: &str,
    places: &[&str],
    caps: Vec<CapabilityDescriptor>,
) -> (
    tokio::task::JoinHandle<Vec<ActionExecuteParams>>,
    mpsc_tx_handle::BodyControl,
    Arc<Notify>,
) {
    let (gw_stream, body_stream) = tokio::io::duplex(64 * 1024);
    let (gw_read, gw_write) = tokio::io::split(gw_stream);
    let (body_read, mut body_write) = tokio::io::split(body_stream);
    let (cmd_tx, mut cmd_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let bid = BodyId::new(body_id);
    let pids: Vec<PlaceId> = places.iter().map(|p| PlaceId::new(*p)).collect();
    let executed_notify = Arc::new(Notify::new());
    let notify_clone = Arc::clone(&executed_notify);

    let body_task = tokio::spawn(async move {
        let mut reader = TokioBufReader::new(body_read);
        let mut line = String::new();
        let mut executed = Vec::new();
        loop {
            line.clear();
            tokio::select! {
                biased;
                maybe_outbound = cmd_rx.recv() => {
                    if let Some(json_line) = maybe_outbound {
                        let _ = body_write.write_all(json_line.as_bytes()).await;
                        let _ = body_write.write_all(b"\n").await;
                        let _ = body_write.flush().await;
                    }
                }
                read_res = reader.read_line(&mut line) => {
                    let Ok(n) = read_res else { break; };
                    if n == 0 {
                        break;
                    }
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        continue;
                    }
                    let raw_val: serde_json::Value =
                        serde_json::from_str(trimmed).expect("valid json line");
                    if raw_val.get("method").is_none() {
                        continue;
                    }
                    let req: Waist2Request<serde_json::Value> =
                        serde_json::from_value(raw_val).expect("valid waist2 req");
                    match req.method {
                        Waist2Method::BodyDiscover => {
                            let _params: BodyDiscoverParams =
                                serde_json::from_value(req.params).expect("discover params");
                            let res = BodyDiscoverResult {
                                body_id: bid.clone(),
                                protocol_version: CURRENT_PROTOCOL_VERSION,
                                capabilities: caps.clone(),
                                places: pids.clone(),
                            };
                            let resp =
                                serde_json::to_string(&Waist2Response::ok(req.id, res)).unwrap();
                            let _ = body_write.write_all(resp.as_bytes()).await;
                            let _ = body_write.write_all(b"\n").await;
                            let _ = body_write.flush().await;
                        }
                        Waist2Method::ActionExecute => {
                            let params: ActionExecuteParams =
                                serde_json::from_value(req.params).expect("exec params");
                            executed.push(params.clone());
                            let receipt = Receipt {
                                action_id: params.action_id.clone(),
                                capability_id: params.capability_id.clone(),
                                outcome: ReceiptOutcome::Done,
                                reason_code: None,
                                detail: Some("completed".to_owned()),
                                termination_reason: Some(TerminationReason::Normal),
                                progress: Some(1.0),
                                delivered_units: Some(1),
                                h_act: params.h_act.clone(),
                                timestamp_ms: Time::from_millis(1_000_050),
                            };
                            let res = ActionExecuteResult {
                                accepted: true,
                                receipt: Some(receipt),
                            };
                            let resp =
                                serde_json::to_string(&Waist2Response::ok(req.id, res)).unwrap();
                            let _ = body_write.write_all(resp.as_bytes()).await;
                            let _ = body_write.write_all(b"\n").await;
                            let _ = body_write.flush().await;
                            notify_clone.notify_one();
                        }
                        Waist2Method::BodyShutdown => {
                            let resp = serde_json::to_string(&Waist2Response::ok(
                                req.id,
                                BodyShutdownResult { accepted: true },
                            ))
                            .unwrap();
                            let _ = body_write.write_all(resp.as_bytes()).await;
                            let _ = body_write.write_all(b"\n").await;
                            let _ = body_write.flush().await;
                            break;
                        }
                        _ => {}
                    }
                }
            }
        }
        executed
    });

    let h = handle.clone();
    let expected_bid = BodyId::new(body_id);
    let ctrl = mpsc_tx_handle::BodyControl {
        cmd_tx,
        connect_fut: tokio::spawn(async move {
            h.connect_ndjson_body(gw_read, gw_write, Some(expected_bid))
                .await
                .expect("connect ndjson body")
        }),
    };
    (body_task, ctrl, executed_notify)
}

#[tokio::test]
async fn guest_dm_with_28_capabilities_and_no_embed_model_wakes_and_delivers_reply() {
    let rbac = guest_rbac(&[PERMISSION_AGENT_CONVERSE, "chat.speak"]);
    let mut cfg = GatewayConfig::default();
    cfg.model.provider = ModelProviderKind::Scripted;
    cfg.model.model = "mock".to_owned();
    cfg.embed.backend = EmbedBackendKind::None;
    cfg.runtime.action_timeout_ms = 2_000;
    cfg.rbac = Some(rbac.clone());

    let clock = Arc::new(VirtualClock::new(Time::from_millis(1_000_000)));
    let secrets = ProviderSecrets::default();
    let diagnostics = DiagnosticSink::new(&secrets);
    let embedder = build_embedder_from_config(&cfg, &secrets).expect("none embedder");
    let pre_queue = Arc::new(PreQueueService::new(
        embedder,
        None,
        None,
        diagnostics.clone(),
        clock.clone(),
        2_000,
    ));

    let reply = serde_json::json!({
        "protocol_version": { "major": 1, "minor": 0 },
        "acts": [
            {
                "act": "think", "thought": "reply to guest DM", "motivation": 4,
                "confidence_0_20": 18, "used_ids": [], "importance": 0.6,
                "basis": "observed", "value": null, "cost": null, "domain": null
            },
            {
                "act": "act", "capability_id": "chat.speak", "places": [PLACE],
                "people": [SENDER], "payload": [111, 107], "motivation": 4,
                "confidence_0_20": 18, "used_ids": [], "expected": null, "domain": null
            }
        ]
    })
    .to_string();

    let rt = GatewayRuntime::start(
        cfg,
        &secrets,
        || Ok(MemStore::new()),
        CoreConfig::default(),
        controller(&rbac).await,
        GatewayOptions {
            clock,
            provider: Some(Arc::new(ScriptedModelProvider::from_replies([
                monoagens_reason::ScriptedReply::text(reply),
            ]))),
            diagnostics,
            cues: CueRecorder::new(),
            seams: GatewaySeams {
                pre_queue_service: Some(pre_queue),
                ..Default::default()
            },
        },
    )
    .await
    .expect("start gateway");
    let handle = rt.handle();

    let mut caps = vec![make_cap("chat.speak", "Send chat messages", &[PLACE], true, false, false)];
    for i in 0..27 {
        caps.push(make_cap(
            &format!("mcp.tools.op_{i:02}"),
            &format!("Extra tool operation {i}"),
            &[PLACE],
            false,
            false,
            false,
        ));
    }
    assert_eq!(caps.len(), 28);

    let (body_task, ctrl, _) =
        spawn_mock_body_with_caps(&handle, "body-whatsapp", &[PLACE], caps);
    let tx = ctrl.wait_connected().await;

    let dm = percept("testtt");
    assert_eq!(dm.addressees, vec![ActorId::new("soul")]);
    mpsc_tx_handle::BodyControl::send_percept(&tx, 1, dm);

    wait_for(&handle, |events| {
        events.iter().any(|e| {
            matches!(
                &e.detail,
                Some(EventDetail::Receipt { capability_id, outcome: ReceiptOutcome::Done, .. })
                    if capability_id.as_str() == "chat.speak"
            )
        })
    })
    .await;

    rt.shutdown().await;
    let executed = body_task.await.expect("body task");
    assert_eq!(executed.len(), 1);
    assert_eq!(executed[0].capability_id.as_str(), "chat.speak");
}

#[tokio::test]
async fn multi_body_guest_help_deduplicates_and_excludes_workspace_and_identity_changing() {
    let rbac = guest_rbac(&["chat.speak", "approval.prompt", "soul_tuning.set_mode"]);
    let (rt, handle) = start(&rbac).await;

    let (wa_task, wa_ctrl, wa_done) = spawn_mock_body_with_caps(
        &handle,
        "body-whatsapp",
        &[PLACE],
        vec![
            make_cap("chat.speak", "Send WhatsApp messages", &[PLACE], true, false, false),
            make_cap("approval.prompt", "Prompt for approval", &[PLACE], true, false, false),
        ],
    );
    let (tg_task, tg_ctrl, _) = spawn_mock_body_with_caps(
        &handle,
        "body-telegram",
        &["tg:dm:1"],
        vec![
            make_cap("chat.speak", "Send Telegram messages", &["tg:dm:1"], true, false, false),
            make_cap("approval.prompt", "Prompt for approval", &["tg:dm:1"], true, false, false),
        ],
    );
    let (ws_task, ws_ctrl, _) = spawn_mock_body_with_caps(
        &handle,
        "body-workspace",
        &["ws:local"],
        vec![
            make_cap("workspace.read", "Read files", &["ws:local"], false, false, false),
            make_cap("workspace.write", "Write files", &["ws:local"], false, false, true),
            make_cap(
                "soul_tuning.set_mode",
                "Change soul mode",
                &["ws:local"],
                false,
                true,
                false,
            ),
            make_cap(
                "soul_tuning.get_mode",
                "View soul mode",
                &["ws:local"],
                false,
                false,
                false,
            ),
        ],
    );

    let wa_tx = wa_ctrl.wait_connected().await;
    let _tg_tx = tg_ctrl.wait_connected().await;
    let _ws_tx = ws_ctrl.wait_connected().await;

    mpsc_tx_handle::BodyControl::send_percept(&wa_tx, 1, percept("/help"));
    tokio::time::timeout(Duration::from_secs(5), wa_done.notified())
        .await
        .expect("guest /help reply");

    rt.shutdown().await;
    let executed = wa_task.await.expect("wa body task");
    let _ = tg_task.await;
    let _ = ws_task.await;

    let texts = reply_texts(&executed);
    assert_eq!(texts.len(), 1, "expected one /help reply: {texts:?}");
    let help = &texts[0];
    assert_eq!(help.matches("approval.prompt").count(), 1, "{help}");
    assert_eq!(help.matches("chat.speak").count(), 1, "{help}");
    assert!(
        help.contains("Permitted capabilities (2): approval.prompt, chat.speak"),
        "{help}"
    );
    assert!(!help.contains("workspace."), "{help}");
    assert!(!help.contains("soul_tuning.set_mode"), "{help}");
    assert!(!help.contains("soul_tuning.get_mode"), "{help}");
}

fn test_owner(
    handle: GatewayHandle,
    cfg: GatewayConfig,
    dir: &Path,
    plugins: Vec<PluginConfig>,
) -> GatewayOwner {
    let log = RunLog::new(
        Arc::clone(handle.clock()),
        handle.diagnostics().clone(),
        None,
    );
    log.attach_diagnostics(handle.diagnostics());
    let sup = PluginSupervisor::new(
        handle.clone(),
        PluginEnvContext {
            config_dir: None,
            data_dir: Some(dir.to_path_buf()),
            rbac_enabled: true,
            plugin_env: Default::default(),
        },
        Arc::clone(handle.clock()),
        log.clone(),
        RestartPolicy::default(),
        plugins,
    );
    GatewayOwner::new(OwnerParams {
        handle,
        supervisor: sup,
        runlog: log,
        mode: OwnerMode::Serve,
        config: cfg,
        config_path: dir.join("monoagens.toml"),
        secrets_path: dir.join("secrets.toml"),
        data_dir: dir.to_path_buf(),
        owner_actor: ActorId::new("per_owner"),
    })
}

#[tokio::test]
async fn capabilities_list_returns_core_connected_and_stopped_cached_plugins() {
    let dir = tempfile::Builder::new()
        .prefix("gw-caps-")
        .tempdir_in(env!("CARGO_TARGET_TMPDIR"))
        .expect("tempdir");

    let mut cache = CapabilityCache::default();
    cache.update_plugin(
        "telegram",
        vec![
            make_cap("chat.speak", "Send Telegram messages", &["tg:1"], true, false, false),
            make_cap("chat.send_media", "Send media", &["tg:1"], true, false, false),
        ],
    );
    cache.save(dir.path());

    let mut cfg = GatewayConfig::default();
    cfg.model.provider = ModelProviderKind::Scripted;
    cfg.model.model = "mock".to_owned();
    cfg.plugins = vec![
        PluginConfig {
            id: "whatsapp".to_owned(),
            enabled: true,
            kind: PluginKind::Adapter,
            transport: BodyTransportConfig::Stdio {
                command: "true".to_owned(),
                args: Vec::new(),
            },
            dependencies: Vec::new(),
            optional_dependencies: Vec::new(),
        },
        PluginConfig {
            id: "telegram".to_owned(),
            enabled: false,
            kind: PluginKind::Adapter,
            transport: BodyTransportConfig::Stdio {
                command: "true".to_owned(),
                args: Vec::new(),
            },
            dependencies: Vec::new(),
            optional_dependencies: Vec::new(),
        },
    ];

    let secrets = ProviderSecrets::default();
    let rt = GatewayRuntime::start(
        cfg.clone(),
        &secrets,
        || Ok(MemStore::new()),
        CoreConfig::default(),
        controller(&guest_rbac(&[])).await,
        GatewayOptions {
            clock: Arc::new(SystemClock::new()),
            provider: Some(Arc::new(ScriptedModelProvider::new())),
            diagnostics: DiagnosticSink::new(&secrets),
            cues: CueRecorder::new(),
            seams: GatewaySeams::default(),
        },
    )
    .await
    .expect("start gateway");
    let handle = rt.handle();

    let (wa_task, wa_ctrl, _) = spawn_mock_body_with_caps(
        &handle,
        "whatsapp",
        &[PLACE],
        vec![
            make_cap("chat.speak", "Send WhatsApp messages", &[PLACE], true, false, false),
            make_cap("approval.prompt", "Prompt for approval", &[PLACE], true, false, false),
        ],
    );
    let (extra_task, extra_ctrl, _) = spawn_mock_body_with_caps(
        &handle,
        "body-extra",
        &["extra:1"],
        vec![
            make_cap("custom.foo_bar", "Custom action", &["extra:1"], false, false, false),
            make_cap(
                "soul_tuning.set_mode",
                "Change soul mode",
                &["extra:1"],
                false,
                true,
                false,
            ),
        ],
    );
    let _wa_tx = wa_ctrl.wait_connected().await;
    let _extra_tx = extra_ctrl.wait_connected().await;

    let owner = test_owner(handle.clone(), cfg.clone(), dir.path(), cfg.effective_plugins());
    let listed = owner.capabilities_list();
    let groups = listed["groups"].as_array().expect("groups array");

    assert_eq!(groups[0]["id"], "core");
    assert_eq!(groups[0]["label"], "Core Agent");
    assert_eq!(groups[0]["kind"], "core");
    assert_eq!(groups[0]["running"], true);
    assert!(groups[0]["body_id"].is_null());
    assert_eq!(groups[0]["capabilities"][0]["id"], "agent.converse");
    assert_eq!(groups[0]["capabilities"][0]["display_name"], "Allow Conversation");
    assert_eq!(groups[0]["capabilities"][0]["risk_tier"], "R0");
    assert_eq!(groups[0]["capabilities"][0]["owner_only"], false);

    let wa = groups.iter().find(|g| g["id"] == "whatsapp").expect("whatsapp group");
    assert_eq!(wa["running"], true);
    assert_eq!(wa["body_id"], "whatsapp");
    let wa_caps = wa["capabilities"].as_array().unwrap();
    let approval = wa_caps
        .iter()
        .find(|c| c["id"] == "approval.prompt")
        .expect("approval.prompt");
    assert_eq!(approval["display_name"], "Approval Prompts");
    assert_eq!(approval["owner_only"], true);
    assert_eq!(approval["risk_tier"], "R0");

    let tg = groups.iter().find(|g| g["id"] == "telegram").expect("telegram group");
    assert_eq!(tg["running"], false);
    assert!(tg["body_id"].is_null());
    let tg_caps = tg["capabilities"].as_array().unwrap();
    assert!(tg_caps.iter().any(|c| c["id"] == "chat.send_media"));

    let extra = groups
        .iter()
        .find(|g| g["id"] == "body-extra")
        .expect("extra body group");
    assert_eq!(extra["running"], true);
    let extra_caps = extra["capabilities"].as_array().unwrap();
    let custom = extra_caps
        .iter()
        .find(|c| c["id"] == "custom.foo_bar")
        .expect("custom.foo_bar");
    assert_eq!(custom["display_name"], "Foo Bar");
    assert_eq!(custom["risk_tier"], "R1");
    assert_eq!(custom["owner_only"], false);
    let set_mode = extra_caps
        .iter()
        .find(|c| c["id"] == "soul_tuning.set_mode")
        .expect("soul_tuning.set_mode");
    assert_eq!(set_mode["display_name"], "Change Soul Mode");
    assert_eq!(set_mode["risk_tier"], "R3");
    assert_eq!(set_mode["owner_only"], true);

    let flat = listed["capabilities"].as_array().expect("flat capabilities");
    let chat_speak_count = flat.iter().filter(|c| c["id"] == "chat.speak").count();
    assert_eq!(chat_speak_count, 1, "flat capabilities must be deduplicated: {flat:?}");
    assert!(flat.iter().any(|c| c["id"] == "agent.converse" && c["plugin_id"] == "core"));
    assert!(flat.iter().any(|c| c["id"] == "chat.send_media" && c["plugin_id"] == "telegram"));

    // Live body capabilities were persisted to capability_cache.json.
    let disk_cache = CapabilityCache::load(dir.path());
    assert!(disk_cache.get("whatsapp").is_some());

    rt.shutdown().await;
    let _ = wa_task.await;
    let _ = extra_task.await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn config_reload_applies_valid_disk_edits_and_fails_closed_on_invalid_toml() {
    let dir = tempfile::Builder::new()
        .prefix("gw-reload-")
        .tempdir_in(env!("CARGO_TARGET_TMPDIR"))
        .expect("tempdir");
    let config_path = dir.path().join("monoagens.toml");
    let secrets_path = dir.path().join("secrets.toml");

    let initial_toml = r#"version = 1

[valve]
mode = "enforcing"
guest_access = true

[roles]
fallback = "guest"

[roles.guest]
base = "guest"
allow = []

[model]
provider = "scripted"
model = "mock"
"#;
    std::fs::write(&config_path, initial_toml).unwrap();
    let cfg = load_config_file(&config_path, &secrets_path).expect("load initial config");
    let rbac = cfg.rbac.clone().expect("initial rbac");

    let secrets = ProviderSecrets::default();
    let rt = GatewayRuntime::start(
        cfg.clone(),
        &secrets,
        || Ok(MemStore::new()),
        CoreConfig::default(),
        controller(&rbac).await,
        GatewayOptions {
            clock: Arc::new(SystemClock::new()),
            provider: Some(Arc::new(ScriptedModelProvider::new())),
            diagnostics: DiagnosticSink::new(&secrets),
            cues: CueRecorder::new(),
            seams: GatewaySeams::default(),
        },
    )
    .await
    .expect("start gateway");
    let handle = rt.handle();
    let (body_task, ctrl) = spawn_ndjson_mock_body(
        &handle,
        "body-whatsapp",
        PLACE,
        "chat.speak",
        MockBodyBehavior::ImmediateDone,
    );
    let _tx = ctrl.wait_connected().await;

    let _lock = acquire_data_dir_lock(dir.path()).expect("data dir lock");
    let owner = Arc::new(test_owner(handle.clone(), cfg, dir.path(), Vec::new()));
    let server = AdminServer::start(&_lock, dir.path(), Arc::clone(&owner))
        .await
        .expect("admin server");
    let mut client = AdminClient::connect(dir.path()).await.expect("admin client");

    // Initially guest does not see chat.speak in /help.
    let before_help = owner.command_run("/help", Some(SENDER), Some(PLACE));
    assert!(
        !before_help["reply_text"]
            .as_str()
            .unwrap_or("")
            .contains("chat.speak"),
        "{before_help:?}"
    );

    // Edit monoagens.toml on disk and call config.reload over the admin socket.
    let updated_toml = r#"version = 1

[valve]
mode = "partial"
guest_access = true

[roles]
fallback = "guest"

[roles.guest]
base = "guest"
allow = ["agent.converse", "chat.speak"]

[model]
provider = "scripted"
model = "mock"
"#;
    std::fs::write(&config_path, updated_toml).unwrap();

    let reloaded = client
        .call("config.reload", json!({}))
        .await
        .expect("config.reload");
    assert_eq!(reloaded["ok"], true);
    assert_eq!(reloaded["reloaded"]["valve"], true);
    assert!(!dir.path().join("monoagens.toml.bak").exists());
    assert_eq!(owner.config_snapshot().valve.mode, "partial");

    let after_help = owner.command_run("/help", Some(SENDER), Some(PLACE));
    assert!(
        after_help["reply_text"]
            .as_str()
            .unwrap_or("")
            .contains("chat.speak"),
        "{after_help:?}"
    );

    // Invalid monoagens.toml fails closed and preserves prior live config.
    std::fs::write(&config_path, "version = 1\n[valve\nbroken").unwrap();
    let err = client
        .call("config.reload", json!({}))
        .await
        .expect_err("invalid toml must fail closed");
    assert_eq!(err.code, -32000);
    assert_eq!(owner.config_snapshot().valve.mode, "partial");
    assert_eq!(
        owner.config_snapshot().rbac.as_ref().unwrap().roles["guest"].allow,
        vec!["agent.converse".to_owned(), "chat.speak".to_owned()]
    );
    assert!(!dir.path().join("monoagens.toml.bak").exists());

    // Invalid secrets.toml also fails closed without mutating live config.
    std::fs::write(&config_path, updated_toml).unwrap();
    std::fs::write(&secrets_path, "[providers.openai\nbroken").unwrap();
    let sec_err = client
        .call("config.reload", json!({}))
        .await
        .expect_err("invalid secrets.toml must fail closed");
    assert_eq!(sec_err.code, -32000);
    assert_eq!(owner.config_snapshot().valve.mode, "partial");

    drop(client);
    server.shutdown();
    rt.shutdown().await;
    let _ = body_task.await;
}
