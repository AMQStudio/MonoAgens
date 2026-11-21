use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};

use monoagens_core::{DoorDecision, EventKind};
use monoagens_gateway::{
    AttachmentCustodyError, AttachmentCustodySeam, AttachmentMasterKey, AttachmentStore, Clock,
    ConfigError, CueRecorder, DiagnosticSink, GatewayAttachmentBridge, GatewayConfig,
    GatewayOptions, GatewayRuntime, GatewaySeams, STAGING_TTL_MS, ScopedAttachmentBlobReader,
    VirtualClock, load_attachment_secrets, parse_config_str, sha256_hex,
};
use monoagens_protocol::{
    ActorId, AttachmentHandle, Audience, BlobGetParams, BlobPutParams, BodyDiscoverParams,
    BodyDiscoverResult, BodyId, CURRENT_PROTOCOL_VERSION, EventId, FModal,
    MAX_INLINE_ATTACHMENT_BYTES, MetaValue, ModalitySlot, OpaquePayload, Percept,
    PerceptEmitParams, PlaceId, Time, VisibleScope, Waist2ErrorCode, Waist2Headers, Waist2Method,
    Waist2Request, Waist2Response,
};
use monoagens_reason::{
    AttachmentAccessContext, AttachmentError, ProviderSecrets, ScriptedModelProvider,
    resolve_authorized_attachment,
};
use serde_json::Value;
use tempfile::TempDir;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader as TokioBufReader};

fn collect_all_files(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    if !dir.exists() {
        return out;
    }
    let mut stack = vec![dir.to_path_buf()];
    while let Some(cur) = stack.pop() {
        if let Ok(entries) = fs::read_dir(&cur) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.is_dir() {
                    stack.push(p);
                } else if p.is_file() {
                    out.push(p);
                }
            }
        }
    }
    out.sort();
    out
}

fn scan_dir_for_needle(dir: &Path, needle: &[u8]) -> Vec<std::path::PathBuf> {
    let mut hits = Vec::new();
    for file in collect_all_files(dir) {
        let bytes = fs::read(&file).expect("read file for scan");
        if bytes.windows(needle.len()).any(|w| w == needle) {
            hits.push(file);
        }
    }
    hits
}

#[test]
fn inline_strip_boundary_256kib_and_256kib_plus_1_and_aead_at_rest() {
    let tmp = TempDir::new().unwrap();
    let master_key = AttachmentMasterKey::generate_ephemeral().unwrap();
    let mut store = AttachmentStore::open(tmp.path(), master_key).unwrap();
    let now = Time::from_millis(1_000_000);

    // Exactly 256 KiB inline succeeds, strips raw payload, and stores AEAD ciphertext on disk.
    let exact_bytes = vec![0xA5u8; MAX_INLINE_ATTACHMENT_BYTES];
    let mut percept_ok = Percept::minimal(
        ActorId::new("scope_A"),
        PlaceId::new("room:media"),
        Audience::new([ActorId::new("scope_A"), ActorId::new("soul")]),
        now,
        OpaquePayload::from_bytes(exact_bytes.clone()),
    );
    percept_ok.f_modal = Some(FModal([1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]));

    let handle = store
        .strip_inline_media_from_percept("scope_A", ModalitySlot::Image, &mut percept_ok)
        .expect("exact 256 KiB inline strip succeeds");

    assert!(
        percept_ok.payload.is_empty(),
        "raw inline payload must be stripped before Core queue"
    );
    assert_eq!(percept_ok.attachments.len(), 1);
    assert_eq!(handle.byte_len, MAX_INLINE_ATTACHMENT_BYTES as u64);

    let rel_path = store
        .rel_path_for("scope_A", &handle.handle_id)
        .unwrap()
        .expect("rel path");
    assert!(
        rel_path.starts_with("attachments/scope_A/"),
        "expected attachments/<scope_id>/<prefix>/<hex>, got {}",
        rel_path.display()
    );
    let disk_bytes = fs::read(tmp.path().join(&rel_path)).unwrap();
    assert_ne!(
        disk_bytes, exact_bytes,
        "stored file must be AEAD encrypted"
    );
    assert!(
        scan_dir_for_needle(&store.attachments_dir(), &exact_bytes[..64]).is_empty(),
        "zero plaintext bytes may ever exist unencrypted on disk"
    );

    // Decrypting round-trips identically.
    let decrypted = store
        .fetch_scoped_bytes("scope_A", &handle.handle_id, now)
        .expect("decrypt succeeds");
    assert_eq!(decrypted, exact_bytes);

    // 256 KiB + 1 inline MUST be rejected.
    let over_bytes = vec![0x5Au8; MAX_INLINE_ATTACHMENT_BYTES + 1];
    let mut percept_over = Percept::minimal(
        ActorId::new("scope_A"),
        PlaceId::new("room:media"),
        Audience::new([ActorId::new("scope_A"), ActorId::new("soul")]),
        now,
        OpaquePayload::from_bytes(over_bytes),
    );
    let err = store
        .strip_inline_media_from_percept("scope_A", ModalitySlot::Image, &mut percept_over)
        .expect_err("256 KiB + 1 inline must be rejected");
    assert!(matches!(
        err,
        AttachmentCustodyError::InlineLimitExceeded { .. }
    ));
}

#[test]
fn out_of_band_blob_put_get_hash_mismatch_and_60s_staging_ttl() {
    let tmp = TempDir::new().unwrap();
    let master_key = AttachmentMasterKey::generate_ephemeral().unwrap();
    let mut store = AttachmentStore::open(tmp.path(), master_key).unwrap();
    let t0 = Time::from_millis(5_000_000);

    let big_bytes = vec![0x7Cu8; MAX_INLINE_ATTACHMENT_BYTES + 4096];
    let good_sha = sha256_hex(&big_bytes);

    // Hash mismatch on blob/put -> -32005 ERR_HASH_MISMATCH.
    let put_bad = BlobPutParams {
        headers: Waist2Headers::new("bad-put"),
        sha256: sha256_hex(b"different"),
        byte_len: big_bytes.len() as u64,
        inline_bytes: Some(OpaquePayload::from_bytes(big_bytes.clone())),
    };
    let err = store
        .put_staged_blob(Some("scope_A"), put_bad, t0)
        .expect_err("hash mismatch must fail");
    assert_eq!(err.code, Waist2ErrorCode::HashMismatch.code());

    // Valid blob/put -> staged with 60 s TTL.
    let put_ok = BlobPutParams {
        headers: Waist2Headers::new("good-put"),
        sha256: good_sha.clone(),
        byte_len: big_bytes.len() as u64,
        inline_bytes: Some(OpaquePayload::from_bytes(big_bytes.clone())),
    };
    let put_res = store
        .put_staged_blob(Some("scope_A"), put_ok, t0)
        .expect("valid blob/put");
    assert_eq!(
        put_res.expires_at_ms,
        Time::from_millis(t0.as_millis() + STAGING_TTL_MS)
    );

    // Hash mismatch on blob/get -> -32005 ERR_HASH_MISMATCH.
    let get_bad = store
        .get_blob(
            Some("scope_A"),
            BlobGetParams {
                headers: Waist2Headers::new("bad-get"),
                blob_ref: put_res.blob_ref.clone(),
                expected_sha256: Some(sha256_hex(b"wrong")),
            },
            t0,
        )
        .expect_err("expected_sha256 mismatch must fail");
    assert_eq!(get_bad.code, Waist2ErrorCode::HashMismatch.code());

    // Valid blob/get at t0 + 60_000 (inclusive) succeeds.
    let get_ok = store
        .get_blob(
            Some("scope_A"),
            BlobGetParams {
                headers: Waist2Headers::new("good-get"),
                blob_ref: put_res.blob_ref.clone(),
                expected_sha256: Some(good_sha),
            },
            Time::from_millis(t0.as_millis() + STAGING_TTL_MS),
        )
        .expect("valid blob/get within 60 s TTL");
    assert_eq!(get_ok.bytes.unwrap().0, big_bytes);

    // Cross-tenant oracle check: scope_B receives the exact same error for scope_A's blob_ref
    // as for a nonexistent blob_ref!
    let cross_err = store
        .get_blob(
            Some("scope_B"),
            BlobGetParams {
                headers: Waist2Headers::new("cross-get"),
                blob_ref: put_res.blob_ref.clone(),
                expected_sha256: None,
            },
            t0,
        )
        .expect_err("cross-tenant fetch must fail");
    let nonexist_err = store
        .get_blob(
            Some("scope_B"),
            BlobGetParams {
                headers: Waist2Headers::new("missing-get"),
                blob_ref: "blob://staged/does-not-exist".to_owned(),
                expected_sha256: None,
            },
            t0,
        )
        .expect_err("missing fetch must fail");
    assert_eq!(cross_err.code, nonexist_err.code);

    // Staging TTL expiry (> 60 s) rejects get_blob and purge wipes the staged file.
    let after_ttl = Time::from_millis(t0.as_millis() + STAGING_TTL_MS + 1);
    store
        .get_blob(
            Some("scope_A"),
            BlobGetParams {
                headers: Waist2Headers::new("expired-get"),
                blob_ref: put_res.blob_ref.clone(),
                expected_sha256: None,
            },
            after_ttl,
        )
        .expect_err("expired staging blob must be rejected");
    let purged = store.purge_expired_staging(after_ttl).expect("purge");
    assert_eq!(purged, 1);
}

#[test]
fn cross_tenant_isolation_aead_tamper_and_hard_forget_zero_residue_byte_scan() {
    let tmp = TempDir::new().unwrap();
    let master_key = AttachmentMasterKey::generate_ephemeral().unwrap();
    let mut store = AttachmentStore::open(tmp.path(), master_key).unwrap();
    let now = Time::from_millis(10_000);

    // Identical bytes stored in scope_A and scope_B must remain independent.
    let shared_secret_payload = b"CANARY-TENANT-CONFIDENTIAL-MEDIA-BYTES-998877".to_vec();

    let h_a1 = store
        .ingest_inline_bytes(
            "scope_A",
            &ActorId::new("scope_A"),
            Some(EventId::new(101)),
            ModalitySlot::Image,
            &shared_secret_payload,
            now,
        )
        .unwrap();
    let h_a2 = store
        .ingest_inline_bytes(
            "scope_A",
            &ActorId::new("scope_A"),
            Some(EventId::new(102)),
            ModalitySlot::Image,
            &shared_secret_payload,
            now,
        )
        .unwrap();
    assert_eq!(h_a1.handle_id, h_a2.handle_id);
    assert_eq!(store.ref_count("scope_A", &h_a1.handle_id).unwrap(), 2);

    let h_b = store
        .ingest_inline_bytes(
            "scope_B",
            &ActorId::new("scope_B"),
            Some(EventId::new(201)),
            ModalitySlot::Image,
            &shared_secret_payload,
            now,
        )
        .unwrap();

    // Scope-keyed BLAKE3 yields distinct handle IDs and paths for identical bytes!
    assert_ne!(h_a1.handle_id, h_b.handle_id);
    let path_a = tmp.path().join(
        store
            .rel_path_for("scope_A", &h_a1.handle_id)
            .unwrap()
            .unwrap(),
    );
    let path_b = tmp.path().join(
        store
            .rel_path_for("scope_B", &h_b.handle_id)
            .unwrap()
            .unwrap(),
    );
    assert_ne!(path_a, path_b);
    assert!(path_a.exists() && path_b.exists());

    // Zero cross-tenant oracle on fetch_scoped_bytes.
    let oracle_a_from_b = store
        .fetch_scoped_bytes("scope_B", &h_a1.handle_id, now)
        .expect_err("scope_B cannot read scope_A handle");
    let oracle_missing = store
        .fetch_scoped_bytes(
            "scope_B",
            "att:scope_B:00000000000000000000000000000000",
            now,
        )
        .expect_err("missing handle");
    assert!(matches!(
        (&oracle_a_from_b, &oracle_missing),
        (
            AttachmentCustodyError::NotFound { .. },
            AttachmentCustodyError::NotFound { .. }
        )
    ));

    // Populate decoded_cache for both scope_A and scope_B.
    assert_eq!(
        store
            .fetch_scoped_bytes("scope_A", &h_a1.handle_id, now)
            .unwrap(),
        shared_secret_payload
    );
    assert_eq!(
        store
            .fetch_scoped_bytes("scope_B", &h_b.handle_id, now)
            .unwrap(),
        shared_secret_payload
    );
    assert_eq!(store.decoded_cache_len(), 2);

    // Partial hard_forget erasing only EventId(101) decrements refcount 2 -> 1 and keeps scope_A blob.
    let partial = store.hard_forget(&[EventId::new(101)], &[], &[]).unwrap();
    assert_eq!(partial.removed_refs, 1);
    assert_eq!(partial.wiped_blobs, 0);
    assert!(path_a.exists());
    assert_eq!(store.ref_count("scope_A", &h_a1.handle_id).unwrap(), 1);

    // Full hard_forget erasing actor scope_A wipes scope_A's file, DB rows, and decoded cache
    // while leaving scope_B completely intact!
    let ciphertext_a_before = fs::read(&path_a).unwrap();
    let full = store
        .hard_forget(&[], &[ActorId::new("scope_A")], &[])
        .unwrap();
    assert_eq!(full.removed_refs, 1);
    assert_eq!(full.wiped_blobs, 1);
    assert_eq!(full.wiped_cache_entries, 1);
    assert!(full.overwritten_bytes > 0);
    assert!(!path_a.exists(), "scope_A file must be unlinked");
    assert_eq!(
        store.decoded_cache_len(),
        1,
        "only scope_B cache entry remains"
    );
    assert!(path_b.exists(), "scope_B file must remain intact");

    // Raw byte scan across attachments/ directory (including SQLite + WAL) finds ZERO residue of:
    // - plaintext canary
    // - scope_A's ciphertext
    // - scope_A's handle_id
    // - scope_A's scope identifier
    let att_dir = store.attachments_dir();
    assert!(scan_dir_for_needle(&att_dir, &shared_secret_payload).is_empty());
    assert!(scan_dir_for_needle(&att_dir, &ciphertext_a_before).is_empty());
    assert!(scan_dir_for_needle(&att_dir, h_a1.handle_id.as_bytes()).is_empty());
    assert!(scan_dir_for_needle(&att_dir, b"scope_A").is_empty());

    // scope_B still decrypts cleanly.
    assert_eq!(
        store
            .fetch_scoped_bytes("scope_B", &h_b.handle_id, now)
            .unwrap(),
        shared_secret_payload
    );

    // AEAD tamper detection: flip 1 byte in scope_B's ciphertext on disk -> must fail closed.
    // First clear cache by opening a fresh AttachmentStore handle or using fetch_scoped_bytes_uncached.
    let mut tampered = fs::read(&path_b).unwrap();
    let last_idx = tampered.len() - 1;
    tampered[last_idx] ^= 0x01;
    fs::write(&path_b, &tampered).unwrap();

    let tamper_err = store
        .fetch_scoped_bytes_uncached("scope_B", &h_b.handle_id, now)
        .expect_err("tampered AEAD file must fail closed");
    assert!(matches!(
        tamper_err,
        AttachmentCustodyError::AeadTamper { .. }
    ));
}

#[test]
fn attachment_secrets_loading_rejects_inline_keys_and_scoped_reader_enforces_doors() {
    let tmp = TempDir::new().unwrap();
    let cfg_path = tmp.path().join("monoagens.toml");
    let sec_path = tmp.path().join("secrets.toml");

    // Inline attachment master key in monoagens.toml MUST be rejected.
    let err = parse_config_str(
        "version = 1\n[attachments]\nmaster_key = \"super-secret-inline\"\n",
        &cfg_path,
        &sec_path,
    )
    .expect_err("inline master_key must be rejected");
    assert!(matches!(err, ConfigError::Secrets(_)));

    // Missing secret fails closed.
    assert!(AttachmentMasterKey::from_secrets(&Default::default()).is_err());

    // Load from secrets.toml or env var.
    fs::write(
        &sec_path,
        "[attachments]\nmaster_key = \"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\"\n",
    )
    .unwrap();
    let loaded = load_attachment_secrets(Some(&sec_path), |_| None).expect("load secrets");
    let mk = AttachmentMasterKey::from_secrets(&loaded).expect("derive master key");

    // Also check DiagnosticSink redacts the attachment master key.
    let diag = DiagnosticSink::default();
    diag.register_secret(loaded.master_key.as_ref().unwrap());
    let scrubbed =
        diag.redact("error using 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef");
    assert!(!scrubbed.contains("0123456789abcdef0123456789abcdef"));

    // ScopedAttachmentBlobReader + reason::resolve_authorized_attachment enforces VisibleScope & Doors.
    let mut store = AttachmentStore::open(tmp.path(), mk).unwrap();
    let now = Time::from_millis(20_000);
    let media = b"authorized-png-bytes".to_vec();
    let handle = store
        .ingest_inline_bytes(
            "per_alice",
            &ActorId::new("per_alice"),
            Some(EventId::new(1)),
            ModalitySlot::Image,
            &media,
            now,
        )
        .unwrap();
    let shared_store = Arc::new(Mutex::new(store));
    let reader = ScopedAttachmentBlobReader::new(shared_store, "per_alice", now);

    // Denied by VisibleScope -> fails closed before decryption.
    let scope_bob = VisibleScope::new(Audience::new([ActorId::new("per_bob")]), None);
    let denied = resolve_authorized_attachment(
        &handle,
        AttachmentAccessContext {
            visible_scope: &scope_bob,
            door_decision: &DoorDecision::Allow,
            item_audience: &Audience::new([ActorId::new("per_alice")]),
            item_event_id: Some(EventId::new(1)),
            now,
        },
        &reader,
        None,
    )
    .expect_err("scope mismatch must fail");
    assert!(matches!(denied, AttachmentError::ScopeDenied { .. }));

    // Allowed by VisibleScope + Allow door -> resolves bytes cleanly.
    let scope_alice = VisibleScope::new(Audience::new([ActorId::new("per_alice")]), None);
    let resolved = resolve_authorized_attachment(
        &handle,
        AttachmentAccessContext {
            visible_scope: &scope_alice,
            door_decision: &DoorDecision::Allow,
            item_audience: &Audience::new([ActorId::new("per_alice")]),
            item_event_id: Some(EventId::new(1)),
            now,
        },
        &reader,
        None,
    )
    .expect("authorized attachment resolves");
    assert_eq!(resolved.byte_len(), media.len());
}

#[tokio::test]
async fn gateway_waist2_strips_inline_media_and_handles_oob_blob_put_get() {
    let tmp = TempDir::new().unwrap();
    let clock = Arc::new(VirtualClock::new(Time::from_millis(1_000_000)));
    let secrets = ProviderSecrets::default();
    let mut cfg = GatewayConfig::default();
    cfg.model.provider = monoagens_gateway::ModelProviderKind::Scripted;

    let store = Arc::new(Mutex::new(
        AttachmentStore::open(
            tmp.path(),
            AttachmentMasterKey::generate_ephemeral().unwrap(),
        )
        .unwrap(),
    ));
    let bridge = Arc::new(Mutex::new(GatewayAttachmentBridge::new(
        Arc::clone(&store),
        clock.clone(),
    )));

    let seams = GatewaySeams {
        attachments: Some(bridge as Arc<Mutex<dyn AttachmentCustodySeam>>),
        ..Default::default()
    };

    let rt = GatewayRuntime::start(
        cfg,
        &secrets,
        || Ok(monoagens_core::MemStore::new()),
        monoagens_core::CoreConfig::default(),
        monoagens_core::NoopDoors,
        GatewayOptions {
            clock: clock.clone(),
            provider: Some(Arc::new(ScriptedModelProvider::new())),
            diagnostics: DiagnosticSink::new(&secrets),
            cues: CueRecorder::new(),
            seams,
        },
    )
    .await
    .expect("start gateway");

    let handle = rt.handle();
    let (gw_stream, body_stream) = tokio::io::duplex(2 * 1024 * 1024);
    let (gw_read, gw_write) = tokio::io::split(gw_stream);
    let (body_read, mut body_write) = tokio::io::split(body_stream);

    let (cmd_tx, mut cmd_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let responses = Arc::new(Mutex::new(BTreeMap::<i64, Value>::new()));
    let resp_clone = Arc::clone(&responses);

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
                    if val.get("method").is_none() {
                        if let Some(id) = val.get("id").and_then(Value::as_i64) {
                            resp_clone.lock().unwrap().insert(id, val);
                        }
                        continue;
                    }
                    let req: Waist2Request<Value> = serde_json::from_value(val).unwrap();
                    match req.method {
                        Waist2Method::BodyDiscover => {
                            let _p: BodyDiscoverParams = serde_json::from_value(req.params).unwrap();
                            let res = BodyDiscoverResult {
                                body_id: BodyId::new("body-media"),
                                protocol_version: CURRENT_PROTOCOL_VERSION,
                                capabilities: vec![],
                                places: vec![PlaceId::new("room:media")],
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
        .connect_ndjson_body(gw_read, gw_write, Some(BodyId::new("body-media")))
        .await
        .expect("connect");

    // 1. Out-of-band blob/put + blob/get + percept/emit promotion (> 256 KiB).
    let big_media = vec![0x99u8; MAX_INLINE_ATTACHMENT_BYTES + 1];
    let big_sha = sha256_hex(&big_media);
    let put_req = Waist2Request::new(
        Some(monoagens_protocol::RpcId::Int(10)),
        Waist2Method::BlobPut,
        BlobPutParams {
            headers: Waist2Headers::new("oob-put"),
            sha256: big_sha.clone(),
            byte_len: big_media.len() as u64,
            inline_bytes: Some(OpaquePayload::from_bytes(big_media.clone())),
        },
    );
    cmd_tx
        .send(serde_json::to_string(&put_req).unwrap())
        .unwrap();

    let put_resp = loop {
        if let Some(v) = responses.lock().unwrap().remove(&10) {
            break v;
        }
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    };
    let blob_ref = put_resp["result"]["blob_ref"].as_str().unwrap().to_owned();

    let get_req = Waist2Request::new(
        Some(monoagens_protocol::RpcId::Int(11)),
        Waist2Method::BlobGet,
        BlobGetParams {
            headers: Waist2Headers::new("oob-get"),
            blob_ref: blob_ref.clone(),
            expected_sha256: Some(big_sha),
        },
    );
    cmd_tx
        .send(serde_json::to_string(&get_req).unwrap())
        .unwrap();
    let get_resp = loop {
        if let Some(v) = responses.lock().unwrap().remove(&11) {
            break v;
        }
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    };
    assert_eq!(
        get_resp["result"]["byte_len"].as_u64(),
        Some(big_media.len() as u64)
    );

    // Emit percept referencing staged blob_ref; Gateway promotes it to scope custody.
    let mut p_oob = Percept::minimal(
        ActorId::new("per_alice"),
        PlaceId::new("room:media"),
        Audience::new([ActorId::new("per_alice"), ActorId::new("soul")]),
        clock.now(),
        OpaquePayload::from_bytes(b"see attached image"),
    );
    p_oob.attachments.push(AttachmentHandle {
        handle_id: blob_ref,
        modality_slot: ModalitySlot::Image,
        byte_len: big_media.len() as u64,
        expires_at_ms: Time::from_millis(2_000_000),
        embed_model_ver: "egemma2-256".to_owned(),
    });
    let emit_oob = Waist2Request::new(
        Some(monoagens_protocol::RpcId::Int(12)),
        Waist2Method::PerceptEmit,
        PerceptEmitParams::from_percept(Waist2Headers::new("emit-oob"), p_oob),
    );
    cmd_tx
        .send(serde_json::to_string(&emit_oob).unwrap())
        .unwrap();

    // 2. Inline 256 KiB + 1 MUST be rejected on percept/emit.
    let over_p = Percept::minimal(
        ActorId::new("per_alice"),
        PlaceId::new("room:media"),
        Audience::new([ActorId::new("per_alice"), ActorId::new("soul")]),
        clock.now(),
        OpaquePayload::from_bytes(big_media),
    );
    let emit_over = Waist2Request::new(
        Some(monoagens_protocol::RpcId::Int(13)),
        Waist2Method::PerceptEmit,
        PerceptEmitParams::from_percept(Waist2Headers::new("emit-over"), over_p),
    );
    cmd_tx
        .send(serde_json::to_string(&emit_over).unwrap())
        .unwrap();

    let over_resp = loop {
        if let Some(v) = responses.lock().unwrap().remove(&13) {
            break v;
        }
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    };
    assert!(
        over_resp.get("error").is_some(),
        "256 KiB + 1 inline must fail"
    );

    // 3. Inline <= 256 KiB media stripped cleanly before Core's queue.
    let mut inline_p = Percept::minimal(
        ActorId::new("per_alice"),
        PlaceId::new("room:media"),
        Audience::new([ActorId::new("per_alice"), ActorId::new("soul")]),
        clock.now(),
        OpaquePayload::from_bytes(vec![0xFFu8; MAX_INLINE_ATTACHMENT_BYTES]),
    );
    inline_p.f_modal = Some(FModal([1.0; 10]));
    let emit_inline = Waist2Request::new(
        Some(monoagens_protocol::RpcId::Int(14)),
        Waist2Method::PerceptEmit,
        PerceptEmitParams::from_percept(Waist2Headers::new("emit-inline"), inline_p),
    );
    cmd_tx
        .send(serde_json::to_string(&emit_inline).unwrap())
        .unwrap();

    let _ = loop {
        if let Some(v) = responses.lock().unwrap().remove(&14) {
            break v;
        }
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    };

    let events = handle.read_events().await.unwrap();
    let percepts: Vec<_> = events
        .iter()
        .filter(|e| e.kind == EventKind::Percept)
        .collect();
    assert_eq!(percepts.len(), 2);
    assert_eq!(percepts[0].attachments.len(), 1);
    assert!(
        percepts[0].attachments[0]
            .handle_id
            .starts_with("att:per_alice:")
    );
    assert!(
        percepts[1].payload.is_empty(),
        "inline media must be stripped before Core"
    );
    assert_eq!(percepts[1].attachments.len(), 1);
    assert_eq!(
        percepts[1].attachments[0].byte_len,
        MAX_INLINE_ATTACHMENT_BYTES as u64
    );

    let _ = MetaValue::Null;
    let _ = BTreeMap::<String, MetaValue>::new();
    rt.shutdown().await;
    let _ = body_task.await;
}
