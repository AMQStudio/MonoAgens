//! Guest access end to end: an unregistered adapter sender is admitted as a guest, RBAC decides
//! whether the agent wakes, gateway slash commands answer without the model, and guests are
//! recognised again after a restart because they are rebuilt from the event log.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use monoagens_core::{CoreConfig, Doors as _, EventDetail, EventKind, MemStore};
use monoagens_gateway::{
    CueRecorder, DiagnosticSink, DoorController as _, GatewayConfig, GatewayHandle, GatewayOptions, GatewayRuntime,
    GatewaySeams, ModelProviderKind, ValveDoorController, VirtualClock,
};
use monoagens_protocol::{ActorId, Audience, OpaquePayload, Percept, PlaceId, Time};
use monoagens_reason::{ProviderSecrets, ScriptedModelProvider};
use monoagens_valve::{
    PERMISSION_AGENT_CONVERSE, RbacConfig, RoleDefinition, RoleTier, Valve, ValveConfig,
    ValveSnapshot,
};

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
    let rbac = guest_rbac(&[PERMISSION_AGENT_CONVERSE]);
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
