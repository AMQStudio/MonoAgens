//! A refusal recorded by the core consumes its action id, so the next action's terminal
//! receipt is not a duplicate (`duplicate terminal receipt for action 'act-N'`).

use std::sync::Arc;
use std::time::Duration;

use monoagens_core::{CoreConfig, EventDetail, MemStore, NoopDoors};
use monoagens_gateway::{
    CueRecorder, DiagnosticSink, GatewayConfig, GatewayOptions, GatewayRuntime, GatewaySeams,
    ModelProviderKind, VirtualClock,
};
use monoagens_protocol::{ActorId, Audience, OpaquePayload, Percept, PlaceId, ReceiptOutcome, Time};
use monoagens_reason::{ProviderSecrets, ScriptedModelProvider, ScriptedReply};
use serde_json::json;

use crate::gateway_tests::{MockBodyBehavior, mpsc_tx_handle, spawn_ndjson_mock_body};

fn reply(capability: &str) -> String {
    json!({
        "protocol_version": { "major": 1, "minor": 0 },
        "acts": [
            {
                "act": "think", "thought": "answer", "motivation": 4, "confidence_0_20": 18,
                "used_ids": [], "importance": 0.6, "basis": "observed", "value": null,
                "cost": null, "domain": null
            },
            {
                "act": "act", "capability_id": capability, "places": ["room:chat"],
                "people": ["per_alice"], "payload": [104, 105], "motivation": 4,
                "confidence_0_20": 18, "used_ids": [], "expected": null, "domain": null
            }
        ]
    })
    .to_string()
}

fn percept(text: &str, ts: u64) -> Percept {
    let mut p = Percept::minimal(
        ActorId::new("per_alice"),
        PlaceId::new("room:chat"),
        Audience::new([ActorId::new("per_alice"), ActorId::new("soul")]),
        Time::from_millis(ts),
        OpaquePayload::from_bytes(text.as_bytes()),
    );
    p.addressees = vec![ActorId::new("soul")];
    p
}

#[tokio::test]
async fn a_refused_action_does_not_make_the_next_receipt_a_duplicate() {
    let mut cfg = GatewayConfig::default();
    cfg.model.provider = ModelProviderKind::Scripted;
    cfg.model.model = "mock".to_owned();
    cfg.runtime.action_timeout_ms = 2_000;
    let secrets = ProviderSecrets::default();
    let diagnostics = DiagnosticSink::new(&secrets);
    let rt = GatewayRuntime::start(
        cfg,
        &secrets,
        || Ok(MemStore::new()),
        CoreConfig::default(),
        NoopDoors,
        GatewayOptions {
            clock: Arc::new(VirtualClock::new(Time::from_millis(2_000_000))),
            provider: Some(Arc::new(ScriptedModelProvider::from_replies([
                // Turn 1 uses a capability nobody disclosed (the core refuses it itself) ...
                ScriptedReply::text(reply("not.disclosed")),
                // ... turn 2 speaks normally.
                ScriptedReply::text(reply("chat.speak")),
            ]))),
            diagnostics: diagnostics.clone(),
            cues: CueRecorder::new(),
            seams: GatewaySeams::default(),
        },
    )
    .await
    .expect("start gateway");
    let handle = rt.handle();
    let (body_task, ctrl) = spawn_ndjson_mock_body(
        &handle,
        "body-chat",
        "room:chat",
        "chat.speak",
        MockBodyBehavior::ImmediateDone,
    );
    let tx = ctrl.wait_connected().await;

    mpsc_tx_handle::BodyControl::send_percept(&tx, 1, percept("one", 2_000_000));
    for _ in 0..400 {
        let events = handle.read_events().await.expect("events");
        if events.iter().any(|e| {
            matches!(&e.detail, Some(EventDetail::Receipt { outcome: ReceiptOutcome::Refused, .. }))
        }) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    mpsc_tx_handle::BodyControl::send_percept(&tx, 2, percept("two", 2_000_100));

    let mut receipts = Vec::new();
    for _ in 0..400 {
        let events = handle.read_events().await.expect("events");
        receipts = events
            .iter()
            .filter_map(|e| match &e.detail {
                Some(EventDetail::Receipt { action_id, outcome, .. }) => {
                    Some((action_id.as_str().to_owned(), *outcome))
                }
                _ => None,
            })
            .collect();
        if receipts.iter().any(|(_, o)| *o == ReceiptOutcome::Done) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    rt.shutdown().await;
    let _ = body_task.await;

    assert_eq!(receipts.len(), 2, "{receipts:?}");
    assert_eq!(receipts[0].1, ReceiptOutcome::Refused);
    assert_eq!(receipts[1].1, ReceiptOutcome::Done, "{receipts:?}");
    assert_ne!(receipts[0].0, receipts[1].0, "action ids must differ: {receipts:?}");
    let log: String = diagnostics
        .records()
        .iter()
        .map(|r| r.message.clone())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!log.contains("duplicate terminal receipt"), "{log}");
}

/// The legacy-provider migration reports itself on the start that performs it and never again.
#[test]
fn legacy_provider_migration_is_reported_on_the_first_start_only() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let cfg_path = dir.path().join("monoagens.toml");
    std::fs::write(
        &cfg_path,
        "version = 1\n[model]\nprovider = \"openai_compatible\"\nmodel = \"m\"\n",
    )
    .expect("write config");
    std::fs::write(
        dir.path().join("secrets.toml"),
        "model_api_key = \"k-1\"\nmodel_endpoint = \"http://127.0.0.1:9/v1\"\n",
    )
    .expect("write secrets");

    let first = monoagens_gateway::load_or_init_gateway_env(Some(&cfg_path), |_| None).expect("first start");
    let migration = first.migration.as_ref().expect("migrated on the first start");
    let notice = migration.notice(&first.paths);
    assert!(notice.contains(&migration.provider), "{notice}");
    assert!(notice.contains("monoagens.toml.bak"), "{notice}");
    assert!(notice.contains("secrets.toml.bak"), "{notice}");

    let second = monoagens_gateway::load_or_init_gateway_env(Some(&cfg_path), |_| None).expect("second start");
    assert!(second.migration.is_none(), "the notice must not repeat");
}
