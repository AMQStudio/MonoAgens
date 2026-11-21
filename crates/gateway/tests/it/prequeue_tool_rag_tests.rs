use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use monoagens_core::{CoreConfig, EventDetail, EventKind, MemStore, NoopDoors};
use monoagens_gateway::{
    Clock, CueRecorder, DiagnosticSink, EmbedBackendKind, GatewayConfig, GatewayOptions,
    GatewayRuntime, GatewaySeams, ModelProviderKind, PreQueueService, ToolRagIndex, VirtualClock,
    build_embedder_from_config, build_embedder_from_config_async,
};
use monoagens_protocol::{
    ActorId, Audience, BodyDiscoverParams, BodyDiscoverResult, BodyId, CURRENT_PROTOCOL_VERSION,
    CapabilityDescriptor, CapabilityFlags, CapabilityId, CapabilitySchemaHash, OpaquePayload,
    Percept, PerceptEmitParams, PlaceId, Time, Waist2Headers, Waist2Method, Waist2Request,
    Waist2Response,
};
use monoagens_reason::{
    EMBEDDING_GEMMA_2_DIM, EMBEDDING_GEMMA_2_VER, EmbedError, EmbedInput, Embedder,
    EmbeddingGemma2Backend, EmbeddingGemma2Config, EmbeddingGemma2Embedder, HashEmbedder,
    InfoGapClassifier, NoneEmbedder, ProviderSecrets, ScriptedEmbedder, ScriptedModelProvider,
};
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader as TokioBufReader};

fn make_unit_vec(dim: usize, hot_idx: usize) -> Vec<f32> {
    let mut v = vec![0.0_f32; dim];
    v[hot_idx % dim] = 1.0;
    v
}

fn spawn_mock_embed_sidecar_http(
    mode: MockEmbedHttpMode,
) -> (String, Arc<AtomicUsize>, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock embedder");
    let addr = listener.local_addr().unwrap();
    let hits = Arc::new(AtomicUsize::new(0));
    let hits_clone = Arc::clone(&hits);

    let handle = thread::spawn(move || match mode {
        MockEmbedHttpMode::OkOpenAiSingle => {
            let (mut stream, _) = listener.accept().unwrap();
            hits_clone.fetch_add(1, Ordering::SeqCst);
            let _body = read_http_req(&mut stream);
            let vec = make_unit_vec(EMBEDDING_GEMMA_2_DIM as usize, 3);
            let resp_body = json!({ "data": [{ "embedding": vec }] }).to_string();
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                resp_body.len(),
                resp_body
            );
            let _ = stream.write_all(resp.as_bytes());
            let _ = stream.flush();
        }
        MockEmbedHttpMode::OkLoop(count) => {
            for _ in 0..count {
                let (mut stream, _) = listener.accept().unwrap();
                hits_clone.fetch_add(1, Ordering::SeqCst);
                let body = read_http_req(&mut stream);
                let req_val: Value = serde_json::from_str(&body).unwrap();
                let prefix = req_val["task_prefix"].as_str().unwrap_or("t_d");
                let idx = match prefix {
                    "t_clf" => 1,
                    "t_q" => 2,
                    _ => 2,
                };
                let vec = make_unit_vec(EMBEDDING_GEMMA_2_DIM as usize, idx);
                let resp_body = json!({ "embedding": vec }).to_string();
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    resp_body.len(),
                    resp_body
                );
                let _ = stream.write_all(resp.as_bytes());
                let _ = stream.flush();
            }
        }
        MockEmbedHttpMode::HangOnce { delay_ms } => {
            let (mut stream, _) = listener.accept().unwrap();
            hits_clone.fetch_add(1, Ordering::SeqCst);
            let _body = read_http_req(&mut stream);
            thread::sleep(Duration::from_millis(delay_ms));
            let vec = make_unit_vec(EMBEDDING_GEMMA_2_DIM as usize, 0);
            let resp_body = json!({ "embedding": vec }).to_string();
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                resp_body.len(),
                resp_body
            );
            let _ = stream.write_all(resp.as_bytes());
            let _ = stream.flush();
        }
        MockEmbedHttpMode::Error500Once => {
            let (mut stream, _) = listener.accept().unwrap();
            hits_clone.fetch_add(1, Ordering::SeqCst);
            let _body = read_http_req(&mut stream);
            let resp_body = "sidecar overloaded";
            let resp = format!(
                "HTTP/1.1 500 Internal Server Error\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                resp_body.len(),
                resp_body
            );
            let _ = stream.write_all(resp.as_bytes());
            let _ = stream.flush();
        }
    });

    (format!("http://{addr}/embed"), hits, handle)
}

#[derive(Clone, Copy)]
enum MockEmbedHttpMode {
    OkOpenAiSingle,
    OkLoop(usize),
    HangOnce { delay_ms: u64 },
    Error500Once,
}

fn read_http_req(stream: &mut TcpStream) -> String {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut content_len = 0usize;
    loop {
        let mut line = String::new();
        let n = reader.read_line(&mut line).unwrap();
        if n == 0 || line == "\r\n" || line == "\n" {
            break;
        }
        let lower = line.to_ascii_lowercase();
        if let Some(rest) = lower.strip_prefix("content-length:") {
            content_len = rest.trim().parse().unwrap_or(0);
        }
    }
    let mut buf = vec![0u8; content_len];
    if content_len > 0 {
        reader.read_exact(&mut buf).unwrap();
    }
    String::from_utf8_lossy(&buf).into_owned()
}

#[test]
fn embedding_gemma2_sidecar_http_and_stdio_and_config_builder_produce_tagged_256d() {
    let tmp = TempDir::new().unwrap();
    let model_file = tmp.path().join("egemma2-256.onnx");
    std::fs::write(&model_file, b"stub-weights").unwrap();

    // 1. Local HTTP sidecar via Gateway config builder.
    let (endpoint, hits, server_join) = spawn_mock_embed_sidecar_http(MockEmbedHttpMode::OkLoop(2));
    let mut cfg = GatewayConfig::default();
    cfg.embed.backend = EmbedBackendKind::EmbeddingGemma2Http;
    cfg.embed.model_path = Some(model_file.clone());
    cfg.embed.endpoint_url = Some(endpoint);
    cfg.embed.timeout_ms = 2_000;

    let embedder = build_embedder_from_config(&cfg, &ProviderSecrets::default()).unwrap();
    assert_eq!(embedder.model_ver(), Some(EMBEDDING_GEMMA_2_VER));
    assert_eq!(embedder.dim_d(), Some(EMBEDDING_GEMMA_2_DIM));

    let mut w_gap = vec![0.0_f32; EMBEDDING_GEMMA_2_DIM as usize];
    w_gap[1] = 2.0;
    let info_gap = InfoGapClassifier::from_weights(
        w_gap,
        0.0,
        Some(monoagens_protocol::EmbeddingTag {
            embed_model_ver: EMBEDDING_GEMMA_2_VER.to_owned(),
            dim_d: EMBEDDING_GEMMA_2_DIM,
            prompt_regime_id: "t_clf".to_owned(),
        }),
    )
    .unwrap();

    let clock = Arc::new(VirtualClock::new(Time::from_millis(10_000)));
    let diag = DiagnosticSink::default();
    let pq = PreQueueService::new(embedder, Some(info_gap), None, diag, clock, 2_000);

    let mut percept = Percept::minimal(
        ActorId::new("per_alice"),
        PlaceId::new("room:embed"),
        Audience::new([ActorId::new("per_alice"), ActorId::new("soul")]),
        Time::from_millis(10_000),
        OpaquePayload::from_bytes(b"Can you check my calendar?"),
    );
    let report = pq.enrich_percept_sync(&mut percept);
    assert!(report.degraded_reason.is_none());
    assert_eq!(hits.load(Ordering::SeqCst), 2);
    assert_eq!(percept.embeddings.len(), 1);
    assert_eq!(
        percept.embeddings[0].tag.embed_model_ver,
        EMBEDDING_GEMMA_2_VER
    );
    assert_eq!(percept.embeddings[0].tag.dim_d, EMBEDDING_GEMMA_2_DIM);
    assert_eq!(percept.embeddings[0].tag.prompt_regime_id, "t_d");
    assert_eq!(percept.embeddings[0].values.len(), 256);
    assert!(percept.dialogue_act_gap.unwrap() > 0.8);
    server_join.join().unwrap();

    // 2. Local stdio sidecar works behind the exact same Embedder trait.
    let script_path = tmp.path().join("mock_sidecar.sh");
    let vec_json = serde_json::to_string(&make_unit_vec(256, 5)).unwrap();
    std::fs::write(
        &script_path,
        format!("#!/bin/sh\nread line\nprintf '{{\"embedding\":{vec_json}}}\\n'\n"),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&script_path).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&script_path, perms).unwrap();
    }
    let stdio_embedder = EmbeddingGemma2Embedder::new(EmbeddingGemma2Config {
        embed_model_ver: EMBEDDING_GEMMA_2_VER.to_owned(),
        dim_d: EMBEDDING_GEMMA_2_DIM,
        max_context_tokens: 8192,
        backend: EmbeddingGemma2Backend::SidecarStdio {
            program: script_path.display().to_string(),
            args: vec![],
            model_path: Some(model_file),
            timeout_ms: 2_000,
        },
    })
    .unwrap();
    let out = stdio_embedder
        .embed(&EmbedInput::document("hello stdio sidecar"))
        .unwrap()
        .unwrap();
    assert_eq!(out.tag.embed_model_ver, EMBEDDING_GEMMA_2_VER);
    assert_eq!(out.tag.dim_d, 256);
    assert_eq!(out.tag.prompt_regime_id, "t_d");

    // 3. Also verify remote OpenAI-compatible EmbeddingGemma 2 endpoint format.
    let (ep1, hits1, j1) = spawn_mock_embed_sidecar_http(MockEmbedHttpMode::OkOpenAiSingle);
    let mut cfg1 = GatewayConfig::default();
    cfg1.embed.backend = EmbedBackendKind::EmbeddingGemma2Remote;
    cfg1.embed.endpoint_url = Some(ep1);
    let rem = build_embedder_from_config(&cfg1, &ProviderSecrets::default()).unwrap();
    let _ = rem.embed(&EmbedInput::query("ping")).unwrap().unwrap();
    assert_eq!(hits1.load(Ordering::SeqCst), 1);
    j1.join().unwrap();
}

async fn run_percept_through_gateway_with_embedder(
    embedder: Arc<dyn Embedder>,
    timeout_ms: u64,
    text: &[u8],
) -> (
    Vec<monoagens_core::EventRecord>,
    Vec<monoagens_gateway::DiagnosticRecord>,
) {
    let clock = Arc::new(VirtualClock::new(Time::from_millis(1_000_000)));
    let secrets = ProviderSecrets::default();
    let mut cfg = GatewayConfig::default();
    cfg.model.provider = ModelProviderKind::Scripted;
    cfg.embed.timeout_ms = timeout_ms;

    let diagnostics = DiagnosticSink::new(&secrets);
    let info_gap =
        InfoGapClassifier::from_weights(vec![0.5; EMBEDDING_GEMMA_2_DIM as usize], -0.2, None)
            .unwrap();
    let service = Arc::new(PreQueueService::new(
        embedder,
        Some(info_gap),
        None,
        diagnostics.clone(),
        clock.clone(),
        timeout_ms,
    ));
    let seams = GatewaySeams {
        pre_queue_service: Some(service),
        ..Default::default()
    };

    let rt = GatewayRuntime::start(
        cfg,
        &secrets,
        || Ok(MemStore::new()),
        CoreConfig::default(),
        NoopDoors,
        GatewayOptions {
            clock: clock.clone(),
            provider: Some(Arc::new(ScriptedModelProvider::new())),
            diagnostics: diagnostics.clone(),
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
    let responses = Arc::new(Mutex::new(Vec::<Value>::new()));
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
                        resp_clone.lock().unwrap().push(val);
                        continue;
                    }
                    let req: Waist2Request<Value> = serde_json::from_value(val).unwrap();
                    match req.method {
                        Waist2Method::BodyDiscover => {
                            let _p: BodyDiscoverParams = serde_json::from_value(req.params).unwrap();
                            let res = BodyDiscoverResult {
                                body_id: BodyId::new("body-emb"),
                                protocol_version: CURRENT_PROTOCOL_VERSION,
                                capabilities: vec![],
                                places: vec![PlaceId::new("room:emb")],
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
        .connect_ndjson_body(gw_read, gw_write, Some(BodyId::new("body-emb")))
        .await
        .expect("connect");

    let mut p = Percept::minimal(
        ActorId::new("per_alice"),
        PlaceId::new("room:emb"),
        Audience::new([ActorId::new("per_alice"), ActorId::new("soul")]),
        clock.now(),
        OpaquePayload::from_bytes(text),
    );
    p.addressees.clear();
    let req = Waist2Request::new(
        Some(monoagens_protocol::RpcId::Int(1)),
        Waist2Method::PerceptEmit,
        PerceptEmitParams::from_percept(Waist2Headers::new("p-1"), p),
    );
    cmd_tx.send(serde_json::to_string(&req).unwrap()).unwrap();

    // Advance virtual clock past timeout_ms if waiting on a hanging embedder.
    for _ in 0..100 {
        if !responses.lock().unwrap().is_empty() {
            break;
        }
        clock.advance_by(timeout_ms / 2 + 10);
        tokio::task::yield_now().await;
        tokio::time::sleep(Duration::from_millis(5)).await;
    }

    let events = handle.read_events().await.unwrap();
    let recs = diagnostics.records();
    rt.shutdown().await;
    let _ = body_task.await;
    (events, recs)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn embedder_timeout_and_500_outage_record_diagnostics_and_forward_vectorless_percept() {
    let tmp = TempDir::new().unwrap();
    let model_file = tmp.path().join("egemma2.onnx");
    std::fs::write(&model_file, b"w").unwrap();

    // 1. Hanging embedder bounded by timeout_ms -> zero retry storm, vector-less percept accepted.
    let (ep_hang, hang_hits, hang_join) =
        spawn_mock_embed_sidecar_http(MockEmbedHttpMode::HangOnce { delay_ms: 400 });
    let mut hang_cfg = GatewayConfig::default();
    hang_cfg.embed.backend = EmbedBackendKind::EmbeddingGemma2Http;
    hang_cfg.embed.model_path = Some(model_file.clone());
    hang_cfg.embed.endpoint_url = Some(ep_hang);
    hang_cfg.embed.timeout_ms = 2_000;
    let hang_embedder = build_embedder_from_config_async(&hang_cfg, &ProviderSecrets::default())
        .await
        .unwrap();
    let (events, recs) =
        run_percept_through_gateway_with_embedder(hang_embedder, 100, b"outage test").await;
    assert_eq!(
        hang_hits.load(Ordering::SeqCst),
        1,
        "no retry storm on timeout"
    );
    let percept_ev = events
        .iter()
        .find(|e| e.kind == EventKind::Percept)
        .expect("vector-less percept recorded in Core");
    if let Some(EventDetail::PerceptMeta { embeddings, .. }) = &percept_ev.detail {
        assert!(
            embeddings.is_empty(),
            "timed-out percept must carry no vector"
        );
    }
    assert!(
        percept_ev.numeric_features.dialogue_act_gap.is_some(),
        "frozen logistic prior still scores dialogue_act_gap on outage"
    );
    assert!(
        recs.iter()
            .any(|r| r.component == "gateway.embedder" && r.message.contains("timed out")),
        "timeout diagnostic must be recorded"
    );
    let _ = hang_join.join();

    // 2. HTTP 500 error -> diagnostic recorded, vector-less percept accepted.
    let (ep_err, err_hits, err_join) =
        spawn_mock_embed_sidecar_http(MockEmbedHttpMode::Error500Once);
    let mut err_cfg = GatewayConfig::default();
    err_cfg.embed.backend = EmbedBackendKind::EmbeddingGemma2Http;
    err_cfg.embed.model_path = Some(model_file);
    err_cfg.embed.endpoint_url = Some(ep_err);
    err_cfg.embed.timeout_ms = 2_000;
    let err_embedder = build_embedder_from_config_async(&err_cfg, &ProviderSecrets::default())
        .await
        .unwrap();
    let (events2, recs2) =
        run_percept_through_gateway_with_embedder(err_embedder, 500, b"500 test").await;
    assert_eq!(err_hits.load(Ordering::SeqCst), 1, "no retry storm on 500");
    let percept_ev2 = events2
        .iter()
        .find(|e| e.kind == EventKind::Percept)
        .expect("vector-less percept recorded in Core");
    if let Some(EventDetail::PerceptMeta { embeddings, .. }) = &percept_ev2.detail {
        assert!(embeddings.is_empty());
    }
    assert!(
        recs2.iter().any(|r| r.component == "gateway.embedder"),
        "500 diagnostic must be recorded"
    );
    err_join.join().unwrap();
}

#[tokio::test]
async fn healthy_hash_embedder_tags_core_vectors_and_indexes_discovered_tools() {
    let embedder = Arc::new(HashEmbedder::with_model_ver(
        EMBEDDING_GEMMA_2_VER,
        EMBEDDING_GEMMA_2_DIM,
    ));
    let (events, _recs) =
        run_percept_through_gateway_with_embedder(embedder, 1_000, b"tagged percept").await;

    let percept_ev = events
        .iter()
        .find(|e| e.kind == EventKind::Percept)
        .expect("percept recorded");
    let Some(EventDetail::PerceptMeta { embeddings, .. }) = &percept_ev.detail else {
        panic!("expected PerceptMeta");
    };
    assert_eq!(embeddings.len(), 1);
    assert_eq!(embeddings[0].tag.embed_model_ver, EMBEDDING_GEMMA_2_VER);
    assert_eq!(embeddings[0].tag.dim_d, 256);
    assert_eq!(embeddings[0].tag.prompt_regime_id, "t_d");
    assert_eq!(embeddings[0].values.len(), 256);
}

fn generate_capability_catalog(n_caps: usize) -> Vec<CapabilityDescriptor> {
    let domains = [
        (
            "calendar",
            "Schedule meetings, check free slots, and manage calendar events",
        ),
        (
            "email",
            "Search inbox messages, read threads, and compose email replies",
        ),
        (
            "filesystem",
            "Read directory entries, inspect file metadata, and stat paths",
        ),
        (
            "database",
            "Run read-only SQL queries and inspect table schemas",
        ),
        (
            "github",
            "List pull requests, inspect diffs, and fetch repository issues",
        ),
    ];
    let mut out = Vec::with_capacity(n_caps);
    for i in 0..n_caps {
        let (dom, desc_base) = domains[i % domains.len()];
        let id = format!("mcp.{dom}.op_{i:03}");
        let schema_hash = blake3::hash(format!("schema:{id}:v1").as_bytes())
            .to_hex()
            .to_string();
        out.push(CapabilityDescriptor {
            id: CapabilityId::new(id),
            description: format!("{desc_base} (operation {i})"),
            places: vec![PlaceId::new(format!("mcp:{dom}"))],
            flags: CapabilityFlags {
                communicative: false,
                identity_changing: false,
                irreversible: false,
            },
            typical_latency_ms: 25,
            capability_schema_hash: CapabilitySchemaHash::new(schema_hash),
            sensed_effect: None,
        });
    }
    out
}

#[test]
fn tool_rag_scales_50_100_200_and_bm25_fallback_is_deterministic() {
    let clock = Arc::new(VirtualClock::new(Time::from_millis(50_000)));
    let diag = DiagnosticSink::default();

    // 1. Hybrid BM25 + 256d scaling across N_caps = 50, 100, 200.
    let mut latencies_us = Vec::new();
    for &n_caps in &[50_usize, 100, 200] {
        let embedder = Arc::new(HashEmbedder::with_model_ver(
            EMBEDDING_GEMMA_2_VER,
            EMBEDDING_GEMMA_2_DIM,
        ));
        let mut index = ToolRagIndex::new(embedder, diag.clone(), clock.clone());
        let caps = generate_capability_catalog(n_caps);
        index.index_capabilities(&BodyId::new("body-mcp"), &caps);
        assert_eq!(index.len(), n_caps);

        // Warm once, then time 25 queries and record mean/max latency.
        let _ = index.query("schedule calendar meeting slots", 8);
        let mut total_us = 0u64;
        for q_idx in 0..25 {
            let res = index.query("schedule calendar meeting slots", 8);
            assert!(res.used_vectors);
            assert_eq!(res.hits.len(), 8);
            assert_eq!(res.catalog_size, n_caps);
            // Every returned descriptor preserves its capability_schema_hash.
            for h in &res.hits {
                assert_eq!(h.descriptor.capability_schema_hash.as_str().len(), 64);
            }
            if q_idx == 0 {
                assert!(
                    res.hits[0].descriptor.id.as_str().contains("calendar"),
                    "top hit should match calendar query"
                );
            }
            total_us += res.query_latency_us;
        }
        let mean_us = total_us / 25;
        latencies_us.push((n_caps, mean_us));
        assert!(
            mean_us < 50_000,
            "Tool RAG query latency {mean_us}us at N_caps={n_caps} exceeded 50ms budget"
        );
    }
    eprintln!(
        "Tool RAG scaling latencies (N_caps, mean_us): {:?}",
        latencies_us
    );

    // 2. Embedder outage -> deterministic lexical BM25 fallback with diagnostic recorded.
    let mut failing = ScriptedEmbedder::new(EMBEDDING_GEMMA_2_VER, EMBEDDING_GEMMA_2_DIM);
    failing.set_fallback(Err(EmbedError::Timeout {
        timeout_ms: 100,
        detail: "sidecar unreachable".to_owned(),
    }));
    let diag_outage = DiagnosticSink::default();
    let mut index_outage = ToolRagIndex::new(Arc::new(failing), diag_outage.clone(), clock.clone());
    let caps_200 = generate_capability_catalog(200);
    index_outage.index_capabilities(&BodyId::new("body-mcp"), &caps_200);

    let r1 = index_outage.query("pull requests github diffs", 8);
    let r2 = index_outage.query("pull requests github diffs", 8);
    assert!(!r1.used_vectors, "outage must fall back to BM25-only");
    assert_eq!(r1.hits.len(), 8);
    assert_eq!(
        r1.hits
            .iter()
            .map(|h| h.descriptor.id.clone())
            .collect::<Vec<_>>(),
        r2.hits
            .iter()
            .map(|h| h.descriptor.id.clone())
            .collect::<Vec<_>>(),
        "BM25 fallback must produce deterministic selection"
    );
    assert!(
        r1.hits
            .iter()
            .all(|h| h.descriptor.id.as_str().contains("github")),
        "BM25 fallback must accurately select github capabilities"
    );
    assert!(
        diag_outage
            .records()
            .iter()
            .any(|r| r.component == "gateway.tool_rag"),
        "outage diagnostic must be recorded"
    );

    // 3. NoneEmbedder also produces identical deterministic BM25 selection.
    let mut index_none = ToolRagIndex::new(Arc::new(NoneEmbedder), diag, clock);
    index_none.index_capabilities(&BodyId::new("body-mcp"), &caps_200);
    let r_none = index_none.query("pull requests github diffs", 8);
    assert!(!r_none.used_vectors);
    assert_eq!(
        r1.hits
            .iter()
            .map(|h| h.descriptor.id.clone())
            .collect::<Vec<_>>(),
        r_none
            .hits
            .iter()
            .map(|h| h.descriptor.id.clone())
            .collect::<Vec<_>>()
    );
}
