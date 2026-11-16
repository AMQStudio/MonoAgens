use std::collections::BTreeMap;

use monoagens_gateway::{
    ConfigError, CueRecorder, DiagnosticSink, GatewayConfig, GatewayOptions, GatewayRuntime,
    GatewaySeams, HEURISTIC_MERGE_ENABLED, HEURISTIC_MERGE_THRESHOLD_MU, IdentityError,
    IdentityResolver, IdentitySeam, OWNER_ESCALATION_GUARD, PersonAccountConfig,
    PersonMappingConfig, VirtualClock, canonical_kappa, derive_per_actor_id, parse_config_str,
};
use monoagens_protocol::{
    ActorId, Audience, BodyDiscoverParams, BodyDiscoverResult, BodyId, CURRENT_PROTOCOL_VERSION,
    OpaquePayload, Percept, PerceptEmitParams, PlaceId, Time, Waist2Headers, Waist2Method,
    Waist2Request, Waist2Response,
};
use monoagens_reason::{ProviderSecrets, ScriptedModelProvider};
use proptest::prelude::*;
use serde_json::Value;
use tempfile::TempDir;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader as TokioBufReader};

proptest! {
    #[test]
    fn proptest_kappa_and_per_actor_prevent_delimiter_and_unicode_collisions(
        b1 in "[:a-zA-Z0-9._\\-\\u{00e9}\\u{1f600}]{0,24}",
        a1 in "[:a-zA-Z0-9._\\-\\u{00e9}\\u{1f600}]{0,24}",
        b2 in "[:a-zA-Z0-9._\\-\\u{00e9}\\u{1f600}]{0,24}",
        a2 in "[:a-zA-Z0-9._\\-\\u{00e9}\\u{1f600}]{0,24}",
    ) {
        let k1 = canonical_kappa(&b1, &a1);
        let k2 = canonical_kappa(&b2, &a2);
        let id1 = derive_per_actor_id(&BodyId::new(&b1), &a1);
        let id2 = derive_per_actor_id(&BodyId::new(&b2), &a2);

        // Deterministic across repeated invocations.
        prop_assert_eq!(id1.clone(), derive_per_actor_id(&BodyId::new(&b1), &a1));
        prop_assert!(id1.as_str().starts_with("per_"));
        prop_assert_eq!(id1.as_str().len(), 4 + 32);

        if (b1, a1) != (b2, a2) {
            prop_assert_ne!(k1, k2);
            prop_assert_ne!(id1, id2);
        } else {
            prop_assert_eq!(k1, k2);
            prop_assert_eq!(id1, id2);
        }
    }
}

#[test]
fn colon_dot_empty_adversarial_pairs_never_collide() {
    let pairs = [
        ("matrix", "@alice:matrix.org"),
        ("matrix:@alice", "matrix.org"),
        ("matrix:", "alice:matrix.org"),
        ("", "matrix:@alice:matrix.org"),
        ("did:web", "example.com:user:1"),
        ("did:web:example.com", "user:1"),
        ("a.b", "c"),
        ("a", "b.c"),
        ("", ""),
    ];
    let ids: Vec<ActorId> = pairs
        .iter()
        .map(|(b, a)| derive_per_actor_id(&BodyId::new(*b), a))
        .collect();
    for i in 0..ids.len() {
        for j in (i + 1)..ids.len() {
            assert_ne!(
                ids[i], ids[j],
                "collision between {:?} and {:?}",
                pairs[i], pairs[j]
            );
        }
    }
}

#[test]
fn config_person_precedence_and_invariant_locks() {
    let tmp = TempDir::new().unwrap();
    let cfg_path = tmp.path().join("monoagens.toml");
    let sec_path = tmp.path().join("secrets.toml");

    // Heuristic merge = true MUST be rejected.
    let bad_heur = "version = 1\n[identity]\nheuristic_merge_enabled = true\n";
    let err =
        parse_config_str(bad_heur, &cfg_path, &sec_path).expect_err("must reject heuristic merge");
    assert!(matches!(err, ConfigError::InvalidValue { .. }));

    // owner_escalation_guard = false MUST be rejected.
    let bad_guard = "version = 1\n[identity]\nowner_escalation_guard = false\n";
    let err2 =
        parse_config_str(bad_guard, &cfg_path, &sec_path).expect_err("must reject disabling guard");
    assert!(matches!(err2, ConfigError::InvalidValue { .. }));

    // Valid [[person]] config overrides κ(b, a) derivation.
    let ok_toml = r#"
version = 1

[identity]
heuristic_merge_enabled = false
link_challenge_ttl_s = 600
retroactive_merge = false
owner_escalation_guard = true

[[person]]
actor_id = "per_owner"
owner = true
accounts = [
  { body_id = "body-cli", account_id = "local" },
  { body_id = "body-matrix", account_id = "@owner:matrix.org" },
]
"#;
    let parsed = parse_config_str(ok_toml, &cfg_path, &sec_path).expect("valid config");
    assert!(!parsed.identity.heuristic_merge_enabled);
    assert!(parsed.identity.owner_escalation_guard);
    assert!(!parsed.identity.retroactive_merge);
    assert_eq!(parsed.identity.link_challenge_ttl_s, 600);

    let resolver = IdentityResolver::from_gateway_config(&parsed, None).expect("init resolver");
    const { assert!(!HEURISTIC_MERGE_ENABLED) };
    assert!(HEURISTIC_MERGE_THRESHOLD_MU.is_infinite());
    const { assert!(OWNER_ESCALATION_GUARD) };
    assert!(!resolver.heuristic_merge_enabled());
    assert!(resolver.heuristic_merge_threshold_mu().is_infinite());

    assert_eq!(
        resolver.resolve(&BodyId::new("body-cli"), "local"),
        ActorId::new("per_owner")
    );
    assert_eq!(
        resolver.resolve(&BodyId::new("body-matrix"), "@owner:matrix.org"),
        ActorId::new("per_owner")
    );
    assert!(resolver.is_owner_account(&BodyId::new("body-cli"), "local"));
    assert!(!resolver.is_owner_account(&BodyId::new("body-matrix"), "@stranger:matrix.org"));
}

#[test]
fn bilateral_challenge_ttl_attempts_no_raw_secrets_retroactive_isolation_and_replay() {
    let tmp = TempDir::new().unwrap();
    let log_path = tmp.path().join("identity_events.jsonl");

    let cfg = GatewayConfig::default();
    let resolver =
        IdentityResolver::from_gateway_config(&cfg, Some(log_path.clone())).expect("resolver");

    let body_a = BodyId::new("body-matrix");
    let acct_a = "@alice:matrix.org";
    let body_b = BodyId::new("body-whatsapp");
    let acct_b = "15551234567@s.whatsapp.net";

    let p1 = resolver.resolve(&body_a, acct_a);
    let p2_before = resolver.resolve(&body_b, acct_b);
    assert_ne!(p1, p2_before);

    // 1. Expired challenge (> 600 s) MUST fail.
    let t0 = Time::from_millis(1_000_000);
    let ch_exp = resolver
        .initiate_bilateral_challenge(&body_a, acct_a, &body_b, acct_b, t0)
        .expect("initiate");
    let exp_err = resolver
        .verify_bilateral_challenge(
            &ch_exp.challenge_id,
            &ch_exp.nonce_1,
            &ch_exp.nonce_2,
            false,
            Time::from_millis(t0.as_millis() + 600_001),
        )
        .expect_err("expired challenge must fail");
    assert!(matches!(exp_err, IdentityError::ChallengeExpired { .. }));

    // 2. Unilateral / wrong nonce fails and enforces link_max_attempts = 3.
    let ch2 = resolver
        .initiate_bilateral_challenge(&body_a, acct_a, &body_b, acct_b, t0)
        .expect("initiate 2");
    for _ in 0..3 {
        let err = resolver
            .verify_bilateral_challenge(
                &ch2.challenge_id,
                &ch2.nonce_1,
                "wrong-second-nonce",
                false,
                Time::from_millis(t0.as_millis() + 10_000),
            )
            .expect_err("wrong nonce2 must fail");
        assert!(matches!(err, IdentityError::NonceMismatch { .. }));
    }
    let locked_err = resolver
        .verify_bilateral_challenge(
            &ch2.challenge_id,
            &ch2.nonce_1,
            &ch2.nonce_2,
            false,
            Time::from_millis(t0.as_millis() + 11_000),
        )
        .expect_err("4th attempt must be blocked");
    assert!(matches!(
        locked_err,
        IdentityError::MaxAttemptsExceeded { .. }
    ));

    // 3. Valid dual-nonce challenge within 600 s links prospectively.
    let link_time = Time::from_millis(t0.as_millis() + 600_000); // exact boundary <= expires_at_ms
    let ch3 = resolver
        .initiate_bilateral_challenge(&body_a, acct_a, &body_b, acct_b, t0)
        .expect("initiate 3");
    let linked_to = resolver
        .verify_bilateral_challenge(
            &ch3.challenge_id,
            &ch3.nonce_1,
            &ch3.nonce_2,
            false,
            link_time,
        )
        .expect("valid bilateral challenge succeeds");
    assert_eq!(linked_to, p1);
    assert_eq!(resolver.resolve(&body_b, acct_b), p1);

    // 4. Raw secret nonces MUST NEVER appear in the append-only log file.
    let raw_log = std::fs::read_to_string(&log_path).expect("read log");
    for secret in [
        &ch_exp.nonce_1,
        &ch_exp.nonce_2,
        &ch2.nonce_1,
        &ch2.nonce_2,
        &ch3.nonce_1,
        &ch3.nonce_2,
    ] {
        assert!(
            !raw_log.contains(secret),
            "raw challenge nonce leaked into identity log!"
        );
    }

    // 5. Non-retroactive privacy (`retroactive_merge = false`):
    // Pre-link private history of p1 is NOT exposed to (body_b, acct_b),
    // and post-link events at >= link_time DO resolve to p1.
    let pre_link_ts = Time::from_millis(t0.as_millis() + 500);
    assert!(!resolver.can_access_historical_actor_item(&body_b, acct_b, &p1, pre_link_ts));
    assert!(resolver.can_access_historical_actor_item(&body_b, acct_b, &p2_before, pre_link_ts));
    assert!(resolver.can_access_historical_actor_item(&body_b, acct_b, &p1, link_time));

    // 6. Restart & replay determinism from raw (body_id, account_id) stream + log file.
    let raw_stream = [
        (body_a.clone(), acct_a),
        (body_b.clone(), acct_b),
        (BodyId::new("body-discord"), "bob#1234"),
    ];
    let resolved_before = resolver.resolve_stream(raw_stream.iter().map(|(b, a)| (b, *a)));

    // Restart from disk log:
    let restarted = IdentityResolver::from_gateway_config(&cfg, Some(log_path.clone()))
        .expect("reload from disk");
    let resolved_after_restart = restarted.resolve_stream(raw_stream.iter().map(|(b, a)| (b, *a)));
    assert_eq!(resolved_before, resolved_after_restart);

    // Pure replay from event list:
    let replayed = IdentityResolver::replay_from_events(
        cfg.identity.clone(),
        &cfg.persons,
        &resolver.events(),
    )
    .expect("replay");
    assert_eq!(
        resolved_before,
        replayed.resolve_stream(raw_stream.iter().map(|(b, a)| (b, *a)))
    );

    // 7. Unlink reverts (body_b, acct_b) back to its canonical κ(b, a) actor ID.
    resolver
        .unlink_actor(
            &body_b,
            acct_b,
            "user_revoked",
            Time::from_millis(2_000_000),
        )
        .expect("unlink");
    assert_eq!(resolver.resolve(&body_b, acct_b), p2_before);
}

#[test]
fn owner_escalation_guard_refuses_unconfirmed_link_and_never_elevates_linked_accounts() {
    let mut cfg = GatewayConfig::default();
    cfg.persons.push(PersonMappingConfig {
        actor_id: ActorId::new("per_owner"),
        is_owner: true,
        accounts: vec![PersonAccountConfig {
            body_id: BodyId::new("body-cli"),
            account_id: "local-owner".to_owned(),
        }],
    });

    let resolver = IdentityResolver::from_gateway_config(&cfg, None).expect("resolver");
    let cli_body = BodyId::new("body-cli");
    let ext_body = BodyId::new("body-chat");
    let ext_acct = "untrusted-user";

    let t0 = Time::from_millis(5_000_000);
    let ch = resolver
        .initiate_bilateral_challenge(&cli_body, "local-owner", &ext_body, ext_acct, t0)
        .expect("initiate");

    // Even with both nonces valid, linking into an owner ActorId without explicit owner
    // confirmation MUST be refused (`owner_escalation_guard = true`).
    let denied = resolver
        .verify_bilateral_challenge(&ch.challenge_id, &ch.nonce_1, &ch.nonce_2, false, t0)
        .expect_err("unconfirmed owner link must be refused");
    assert!(matches!(
        denied,
        IdentityError::OwnerEscalationDenied { .. }
    ));
    assert!(!resolver.is_owner_account(&ext_body, ext_acct));
    assert_ne!(
        resolver.resolve(&ext_body, ext_acct),
        ActorId::new("per_owner")
    );

    // Even if explicitly owner-confirmed for routing, the external Tier-2 account STILL
    // never gains owner-only powers (`is_owner_account` remains false).
    let ch2 = resolver
        .initiate_bilateral_challenge(&cli_body, "local-owner", &ext_body, ext_acct, t0)
        .expect("initiate 2");
    let linked = resolver
        .verify_bilateral_challenge(&ch2.challenge_id, &ch2.nonce_1, &ch2.nonce_2, true, t0)
        .expect("owner-confirmed link");
    assert_eq!(linked, ActorId::new("per_owner"));
    assert!(resolver.is_owner_account(&cli_body, "local-owner"));
    assert!(!resolver.is_owner_account(&ext_body, ext_acct));
}

#[tokio::test]
async fn gateway_waist2_percept_emit_resolves_canonical_kappa_actors() {
    let clock = std::sync::Arc::new(VirtualClock::new(Time::from_millis(1_000_000)));
    let secrets = ProviderSecrets::default();
    let mut cfg = GatewayConfig::default();
    cfg.model.provider = monoagens_gateway::ModelProviderKind::Scripted;
    cfg.persons.push(PersonMappingConfig {
        actor_id: ActorId::new("per_owner"),
        is_owner: true,
        accounts: vec![PersonAccountConfig {
            body_id: BodyId::new("body-matrix"),
            account_id: "@owner:matrix.org".to_owned(),
        }],
    });

    let resolver =
        std::sync::Arc::new(IdentityResolver::from_gateway_config(&cfg, None).expect("resolver"));
    let seams = GatewaySeams {
        identity: Some(resolver.clone() as std::sync::Arc<dyn IdentitySeam>),
        ..Default::default()
    };

    let rt = GatewayRuntime::start(
        cfg,
        &secrets,
        || Ok(monoagens_core::MemStore::new()),
        monoagens_core::Prior::default(),
        monoagens_core::Switches::all_neutral(),
        monoagens_core::NoopDoors,
        GatewayOptions {
            clock: clock.clone(),
            provider: Some(std::sync::Arc::new(ScriptedModelProvider::new())),
            diagnostics: DiagnosticSink::new(&secrets),
            cues: CueRecorder::new(),
            seams,
        },
    )
    .await
    .expect("start gateway");

    let handle = rt.handle();
    let (gw_stream, body_stream) = tokio::io::duplex(64 * 1024);
    let (gw_read, gw_write) = tokio::io::split(gw_stream);
    let (body_read, mut body_write) = tokio::io::split(body_stream);

    let (cmd_tx, mut cmd_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let body_task = tokio::spawn(async move {
        let mut reader = TokioBufReader::new(body_read);
        let mut line = String::new();
        loop {
            line.clear();
            tokio::select! {
                biased;
                maybe_out = cmd_rx.recv() => {
                    if let Some(s) = maybe_out {
                        let _ = body_write.write_all(s.as_bytes()).await;
                        let _ = body_write.write_all(b"\n").await;
                        let _ = body_write.flush().await;
                    }
                }
                res = reader.read_line(&mut line) => {
                    let Ok(n) = res else { break; };
                    if n == 0 { break; }
                    let trimmed = line.trim();
                    if trimmed.is_empty() { continue; }
                    let val: Value = serde_json::from_str(trimmed).unwrap();
                    if val.get("method").is_none() { continue; }
                    let req: Waist2Request<Value> = serde_json::from_value(val).unwrap();
                    match req.method {
                        Waist2Method::BodyDiscover => {
                            let _p: BodyDiscoverParams = serde_json::from_value(req.params).unwrap();
                            let res = BodyDiscoverResult {
                                body_id: BodyId::new("body-matrix"),
                                protocol_version: CURRENT_PROTOCOL_VERSION,
                                capabilities: vec![],
                                places: vec![PlaceId::new("room:mx")],
                            };
                            let resp = serde_json::to_string(&Waist2Response::ok(req.id, res)).unwrap();
                            let _ = body_write.write_all(resp.as_bytes()).await;
                            let _ = body_write.write_all(b"\n").await;
                            let _ = body_write.flush().await;
                        }
                        Waist2Method::BodyShutdown => {
                            let resp = serde_json::to_string(&Waist2Response::ok(
                                req.id,
                                monoagens_protocol::BodyShutdownResult { accepted: true },
                            )).unwrap();
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
    });

    handle
        .connect_ndjson_body(gw_read, gw_write, Some(BodyId::new("body-matrix")))
        .await
        .expect("connect");

    // Emit percept from @alice:matrix.org (hashed) and @owner:matrix.org ([[person]] mapped).
    for (rpc_id, acct) in [(1, "@alice:matrix.org"), (2, "@owner:matrix.org")] {
        let p = Percept::minimal(
            ActorId::new(acct),
            PlaceId::new("room:mx"),
            Audience::new([ActorId::new(acct), ActorId::new("soul")]),
            Time::from_millis(1_000_000 + rpc_id as u64),
            OpaquePayload::from_bytes(b"hello"),
        );
        let req = Waist2Request::new(
            Some(monoagens_protocol::RpcId::Int(rpc_id)),
            Waist2Method::PerceptEmit,
            PerceptEmitParams::from_percept(Waist2Headers::new(format!("p-{rpc_id}")), p),
        );
        cmd_tx.send(serde_json::to_string(&req).unwrap()).unwrap();
    }

    for _ in 0..100 {
        let evs = handle.read_events().await.unwrap();
        let percepts: Vec<_> = evs
            .iter()
            .filter(|e| e.kind == monoagens_core::EventKind::Percept)
            .collect();
        if percepts.len() == 2 {
            let expected_alice =
                derive_per_actor_id(&BodyId::new("body-matrix"), "@alice:matrix.org");
            assert_eq!(percepts[0].actor, expected_alice);
            assert!(percepts[0].audience.contains(&expected_alice));
            assert_eq!(percepts[1].actor, ActorId::new("per_owner"));
            assert!(percepts[1].audience.contains(&ActorId::new("per_owner")));
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }

    // Silence unused import warning if BTreeMap wasn't needed directly here.
    let _ = BTreeMap::<String, String>::new();

    rt.shutdown().await;
    let _ = body_task.await;
}
