use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::unix::fs::PermissionsExt;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use monoagens_core::{
    Core, CoreConfig, DoorDecision, Input, MemStore, ModelPurpose, NoopDoors,
};
use monoagens_protocol::{
    ActorId, AttachmentHandle, Audience, CapabilityDescriptor, CapabilityFlags, CapabilityId,
    CapabilitySchemaHash, EmbeddingTag, EventId, ExpectKind, ModalitySlot, OpaquePayload, Percept,
    PlaceId, ResolveOutcome, TaggedEmbedding, ThoughtAct, Time, VisibleScope,
};
use monoagens_reason::{
    AttachmentAccessContext, AttachmentError, CallOutcome, CapabilityCatalogItem, ConfigSourceKind,
    EMBEDDING_GEMMA_2_CTX_TOKENS, EMBEDDING_GEMMA_2_DIM, EMBEDDING_GEMMA_2_VER,
    EMBEDDING_GEMMA_300M_VER, EmbedError, EmbedInput, Embedder, EmbeddingGemma2Config,
    EmbeddingGemma2Embedder, EmbeddingGemma300mConfig, EmbeddingGemma300mEmbedder, HashEmbedder,
    InMemoryAttachmentStore, InfoGapClassifier, ModelOutput, ModelProvider, ModelRequest,
    MotivationLevel, NoneEmbedder, OpenAiModelConfig, OpenAiModelProvider, ParseOptions,
    ProcedureCatalogItem, ProviderFailureClass, ProviderSecrets, ScriptedEmbedder,
    ScriptedModelProvider, ScriptedReply, SecretError, SecretString, TaskPrefix,
    ToolSelectionBudget, decode_rubric, is_l2_normalized, parse_thought_output,
    parse_thought_output_with_options, partitioned_cosine, resolve_authorized_attachment,
    search_asymmetric_q_to_d, search_partitioned, select_tools,
    strict_classification_verdict_schema, strict_thought_batch_schema, validate_no_secrets_in_json,
    validate_no_secrets_in_str,
};
use serde_json::{Value, json};

#[derive(Clone)]
struct MockHttpReply {
    status_line: &'static str,
    body: String,
    delay_ms: u64,
}

fn spawn_mock_http_server(
    replies: Vec<MockHttpReply>,
) -> (String, Arc<Mutex<Vec<String>>>, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock server");
    let addr = listener.local_addr().expect("mock local_addr");
    let captured_bodies = Arc::new(Mutex::new(Vec::new()));
    let captured_clone = Arc::clone(&captured_bodies);

    let handle = thread::spawn(move || {
        for reply in replies {
            let (mut stream, _) = listener.accept().expect("accept client");
            let body_str = read_http_request_body(&mut stream);
            captured_clone.lock().expect("lock bodies").push(body_str);

            if reply.delay_ms > 0 {
                thread::sleep(Duration::from_millis(reply.delay_ms));
            }
            let response = format!(
                "HTTP/1.1 {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                reply.status_line,
                reply.body.len(),
                reply.body
            );
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
        }
    });

    (
        format!("http://{addr}/v1/chat/completions"),
        captured_bodies,
        handle,
    )
}

fn read_http_request_body(stream: &mut TcpStream) -> String {
    let mut reader = BufReader::new(stream.try_clone().expect("clone stream"));
    let mut content_length: usize = 0;
    loop {
        let mut line = String::new();
        let n = reader.read_line(&mut line).expect("read header line");
        if n == 0 || line == "\r\n" || line == "\n" {
            break;
        }
        let lower = line.to_ascii_lowercase();
        if let Some(rest) = lower.strip_prefix("content-length:") {
            content_length = rest.trim().parse().unwrap_or(0);
        }
    }
    let mut buf = vec![0_u8; content_length];
    if content_length > 0 {
        reader.read_exact(&mut buf).expect("read request body");
    }
    String::from_utf8_lossy(&buf).into_owned()
}

fn golden_multi_act_json() -> String {
    json!({
        "protocol_version": { "major": 1, "minor": 0 },
        "acts": [
            {
                "act": "think",
                "thought": "Alice asked for status.",
                "motivation": 4,
                "confidence_0_20": 16,
                "used_ids": [1, 2],
                "importance": 0.75,
                "basis": "observed",
                "value": null,
                "cost": null,
                "domain": "ops"
            },
            {
                "act": "act",
                "capability_id": "chat.speak",
                "places": ["room:1"],
                "people": ["per_alice"],
                "payload": [111, 107],
                "motivation": 4,
                "confidence_0_20": 17,
                "used_ids": [1],
                "expected": null,
                "domain": null
            },
            {
                "act": "expect",
                "expect_kind": "response",
                "about_ids": [2],
                "places": ["room:1"],
                "people": ["per_alice"],
                "condition": "alice acknowledges",
                "due_ms": 5000,
                "expiry_ms": 20000,
                "recurrence": null,
                "promise_meta": null,
                "domain": null
            },
            {
                "act": "resolve",
                "expectation_id": 1,
                "outcome": "kept",
                "note": null,
                "places": ["room:1"],
                "people": ["per_alice"],
                "domain": null
            },
            {
                "act": "revise",
                "target_id": 2,
                "new_version": 2,
                "condition": "extended wait",
                "due_ms": null,
                "expiry_ms": 30000,
                "places": ["room:1"],
                "people": ["per_alice"],
                "domain": null
            }
        ]
    })
    .to_string()
}

fn golden_consolidate_json() -> String {
    json!({
        "acts": [
            {
                "act": "consolidate",
                "output": {
                    "consolidate_kind": "gist",
                    "summary": "Handled status request in room:1",
                    "about_ids": [1, 2],
                    "places": ["room:1"],
                    "people": ["per_alice"],
                    "domain": null
                }
            },
            {
                "act": "consolidate",
                "output": {
                    "consolidate_kind": "lesson",
                    "pattern": "Verify room scope before sending files",
                    "failure_signature": "door_denied:scope",
                    "contrastive_success_ids": [10],
                    "failure_ids": [11],
                    "domain": "safety"
                }
            },
            {
                "act": "consolidate",
                "output": {
                    "consolidate_kind": "procedure",
                    "name": "reply_status",
                    "steps": ["read status", "reply concisely"],
                    "required_cap_ids": ["chat.speak"],
                    "capability_schema_hash": "hash_v1",
                    "domain": "ops"
                }
            }
        ]
    })
    .to_string()
}

#[test]
fn golden_fixtures_parse_identically_across_scripted_and_mock_http_providers() {
    let batch_schema = strict_thought_batch_schema();
    assert!(
        jsonschema::validator_for(&batch_schema).is_ok(),
        "strict ThoughtBatch schema must compile as Draft 2020-12"
    );
    let clf_schema = strict_classification_verdict_schema();
    assert!(
        jsonschema::validator_for(&clf_schema).is_ok(),
        "strict ClassificationVerdict schema must compile as Draft 2020-12"
    );

    let fixtures = [
        (ModelPurpose::Reason, golden_multi_act_json()),
        (ModelPurpose::Consolidate, golden_consolidate_json()),
        (
            ModelPurpose::Reason,
            "Plain act-less reflection text from model.".to_owned(),
        ),
        (
            ModelPurpose::Classify,
            json!({
                "label": "answered_silence",
                "score": 0.85,
                "rationale": null
            })
            .to_string(),
        ),
    ];

    let mock_replies: Vec<MockHttpReply> = fixtures
        .iter()
        .map(|(_, content)| MockHttpReply {
            status_line: "200 OK",
            body: json!({
                "id": "chatcmpl-1",
                "model": "mock-gpt",
                "choices": [{
                    "index": 0,
                    "message": {
                        "role": "assistant",
                        "content": content,
                        "refusal": null
                    }
                }],
                "usage": {
                    "prompt_tokens": 120,
                    "completion_tokens": 45,
                    "total_tokens": 165,
                    "prompt_tokens_details": {
                        "cached_tokens": 64
                    }
                }
            })
            .to_string(),
            delay_ms: 0,
        })
        .collect();

    let (endpoint, captured, server_handle) = spawn_mock_http_server(mock_replies);

    let scripted = ScriptedModelProvider::from_replies(
        fixtures
            .iter()
            .map(|(_, raw)| ScriptedReply::text(raw.clone())),
    );
    let http_provider = OpenAiModelProvider::new(OpenAiModelConfig::new(
        endpoint,
        Some(SecretString::new("sk-test-fixture-token-0001")),
    ))
    .expect("construct openai provider");

    for (idx, (purpose, _)) in fixtures.iter().enumerate() {
        let req = ModelRequest::text(
            EventId::new((idx + 1) as u64),
            *purpose,
            "mock-gpt",
            "system",
            "user",
        );
        let s_res = scripted.invoke(&req).expect("scripted fixture succeeds");
        let h_res = http_provider.invoke(&req).expect("http fixture succeeds");

        assert_eq!(s_res.output, h_res.output, "fixture {idx} output diverged");
        assert_eq!(h_res.record.outcome, CallOutcome::Success);
        assert_eq!(h_res.record.model, "mock-gpt");
        let usage = h_res.record.usage.expect("usage reported");
        assert_eq!(usage.input_tokens, 120);
        assert_eq!(usage.output_tokens, 45);
        assert_eq!(usage.cached_tokens, Some(64));

        if idx == 0 {
            if let ModelOutput::ThoughtActs(parsed) = &h_res.output {
                assert!(!parsed.fallback_plain_text);
                assert_eq!(parsed.acts.len(), 5);
                assert!(matches!(parsed.acts[0], ThoughtAct::Think { .. }));
                assert!(matches!(parsed.acts[1], ThoughtAct::Act { .. }));
                assert!(matches!(parsed.acts[2], ThoughtAct::Expect { .. }));
                assert!(matches!(
                    parsed.acts[3],
                    ThoughtAct::Resolve {
                        outcome: ResolveOutcome::Kept,
                        ..
                    }
                ));
                assert!(matches!(parsed.acts[4], ThoughtAct::Revise { .. }));
            } else {
                panic!("expected ThoughtActs");
            }
        } else if idx == 2 {
            if let ModelOutput::ThoughtActs(parsed) = &h_res.output {
                assert!(parsed.fallback_plain_text);
                assert_eq!(parsed.acts.len(), 1);
                assert!(matches!(parsed.acts[0], ThoughtAct::Think { .. }));
            } else {
                panic!("expected ThoughtActs fallback");
            }
        }
    }

    server_handle.join().expect("server thread finishes");
    let req_bodies = captured.lock().expect("lock bodies");
    assert_eq!(req_bodies.len(), fixtures.len());
    for raw_req in req_bodies.iter() {
        let v: Value = serde_json::from_str(raw_req).expect("valid outgoing JSON");
        assert_eq!(
            v["store"],
            json!(false),
            "must enforce stateless store:false"
        );
        assert_eq!(v["response_format"]["json_schema"]["strict"], json!(true));
    }
}

#[test]
fn malformed_acts_produce_failed_wake_naming_the_wrong_field_and_rubric_decodes() {
    let cases = vec![
        (
            "missing thought",
            json!({
                "acts": [{
                    "act": "think",
                    "motivation": 3,
                    "confidence_0_20": 10,
                    "used_ids": []
                }]
            })
            .to_string(),
            "thought",
        ),
        (
            "non-nullable null on required field",
            json!({
                "acts": [{
                    "act": "think",
                    "thought": null,
                    "motivation": 3,
                    "confidence_0_20": 10,
                    "used_ids": []
                }]
            })
            .to_string(),
            "thought",
        ),
        (
            "out of range confidence_0_20",
            json!({
                "acts": [{
                    "act": "think",
                    "thought": "hi",
                    "motivation": 3,
                    "confidence_0_20": 21,
                    "used_ids": []
                }]
            })
            .to_string(),
            "confidence_0_20",
        ),
        (
            "out of range motivation",
            json!({
                "acts": [{
                    "act": "think",
                    "thought": "hi",
                    "motivation": 0,
                    "confidence_0_20": 10,
                    "used_ids": []
                }]
            })
            .to_string(),
            "motivation",
        ),
        (
            "empty places on act",
            json!({
                "acts": [{
                    "act": "act",
                    "capability_id": "chat.speak",
                    "places": [],
                    "people": [],
                    "payload": [1, 2],
                    "motivation": 4,
                    "confidence_0_20": 15,
                    "used_ids": []
                }]
            })
            .to_string(),
            "places",
        ),
        (
            "unknown extra field",
            json!({
                "acts": [{
                    "act": "think",
                    "thought": "hi",
                    "motivation": 3,
                    "confidence_0_20": 10,
                    "used_ids": [],
                    "rogue_field": true
                }]
            })
            .to_string(),
            "rogue_field",
        ),
        (
            "empty procedure capability_schema_hash",
            json!({
                "acts": [{
                    "act": "consolidate",
                    "output": {
                        "consolidate_kind": "procedure",
                        "name": "p1",
                        "steps": ["a"],
                        "required_cap_ids": ["c1"],
                        "capability_schema_hash": "",
                        "domain": null
                    }
                }]
            })
            .to_string(),
            "capability_schema_hash",
        ),
        ("invalid json", "{\"acts\": [".to_owned(), "json"),
        ("empty output", "   ".to_owned(), "output"),
    ];

    for (label, raw, expected_field) in cases {
        let err = parse_thought_output(&raw)
            .expect_err(&format!("case '{label}' must return FailedWake"));
        assert!(
            err.field.contains(expected_field),
            "case '{label}' expected field containing '{expected_field}', got '{}'",
            err.field
        );
    }

    // Strict explicit-null mode rejects omitted nullable fields naming the missing field.
    let omitted_nullable = json!({
        "acts": [{
            "act": "resolve",
            "expectation_id": 5,
            "outcome": "kept",
            "places": ["room:1"],
            "people": ["per_alice"]
        }]
    })
    .to_string();
    assert!(parse_thought_output(&omitted_nullable).is_ok());
    let strict_err =
        parse_thought_output_with_options(&omitted_nullable, ParseOptions::strict_explicit_nulls())
            .expect_err("strict_explicit_nulls rejects omitted optional keys");
    assert!(strict_err.field.contains("note") || strict_err.field.contains("domain"));

    // Rubric decoding.
    let decoded = decode_rubric(4, 15).expect("valid rubric");
    assert_eq!(decoded.motivation_level, MotivationLevel::PreferSpeak);
    assert!((decoded.c_raw - 0.75).abs() < 1e-12);

    // ExpectKind coverage check.
    let expect_json = json!({
        "acts": [{
            "act": "expect",
            "expect_kind": "goal",
            "about_ids": [1],
            "places": ["room:1"],
            "people": [],
            "condition": "goal met",
            "due_ms": null,
            "expiry_ms": 9999,
            "recurrence": null,
            "promise_meta": null,
            "domain": null
        }]
    })
    .to_string();
    let parsed_expect = parse_thought_output(&expect_json).expect("valid expect act");
    assert!(matches!(
        parsed_expect.acts[0],
        ThoughtAct::Expect {
            expect_kind: ExpectKind::Goal,
            ..
        }
    ));
}

#[test]
fn http_failure_classification_timeout_and_zero_secret_leakage_keep_loop_alive() {
    let canary_key = "sk-canary-super-secret-key-987654321";
    let replies = vec![
        MockHttpReply {
            status_line: "401 Unauthorized",
            body: format!("{{\"error\":\"invalid token {canary_key}\"}}"),
            delay_ms: 0,
        },
        MockHttpReply {
            status_line: "429 Too Many Requests",
            body: format!("{{\"error\":\"rate limit exceeded for Bearer {canary_key}\"}}"),
            delay_ms: 0,
        },
        MockHttpReply {
            status_line: "503 Service Unavailable",
            body: format!("{{\"error\":\"upstream crash with {canary_key}\"}}"),
            delay_ms: 0,
        },
        MockHttpReply {
            status_line: "200 OK",
            body: "{\"choices\":[]}".to_owned(),
            delay_ms: 250,
        },
    ];

    let (endpoint, _captured, server_handle) = spawn_mock_http_server(replies);
    let config = OpenAiModelConfig::new(endpoint, Some(SecretString::new(canary_key)));
    assert!(
        !format!("{config:?}").contains(canary_key),
        "config Debug must never leak secret"
    );

    let provider = OpenAiModelProvider::new(config).expect("construct provider");

    let expected = [
        ProviderFailureClass::Authentication,
        ProviderFailureClass::RateLimit,
        ProviderFailureClass::Server,
        ProviderFailureClass::Timeout,
    ];

    let mut core = Core::open(
        MemStore::new(),
        CoreConfig {
            seed: 42,
            ..CoreConfig::default()
        },
        None,
    )
    .expect("open core");
    let mut doors = NoopDoors;

    for (idx, exp_class) in expected.iter().enumerate() {
        let mut req = ModelRequest::text(
            EventId::new((idx + 1) as u64),
            ModelPurpose::Reason,
            "mock-gpt",
            "sys",
            "usr",
        );
        req.timeout_ms = 50;

        let failure = provider
            .invoke(&req)
            .expect_err("expected classified provider failure");
        assert_eq!(failure.class, *exp_class);
        assert_eq!(failure.record.outcome, CallOutcome::Failed(*exp_class));

        let display_str = failure.to_string();
        let debug_str = format!("{failure:?}");
        assert!(
            !display_str.contains(canary_key),
            "canary leaked in Display"
        );
        assert!(!debug_str.contains(canary_key), "canary leaked in Debug");
        assert!(
            !failure.detail.contains(canary_key),
            "canary leaked in detail"
        );

        let core_input = failure.to_core_input();
        if let Input::ModelFailure { ref reason, .. } = core_input {
            assert!(!reason.contains(canary_key), "canary leaked in Core input");
        }
        let effects = core
            .step(
                core_input,
                Time::from_millis(10_000 + idx as u64 * 100),
                &mut doors,
            )
            .expect("core loop survives ModelFailure");
        assert!(effects.is_empty());
    }

    server_handle.join().expect("server thread finishes");
}

#[test]
fn embedder_backends_partition_non_mixing_and_startup_model_path_checks() {
    // 1. Local providers without configured or existing model_path fail at startup.
    let err_missing = EmbeddingGemma2Embedder::new(EmbeddingGemma2Config::local_http(
        "http://127.0.0.1:9999/embed",
        None,
        1000,
    ))
    .expect_err("missing model_path must fail at startup");
    assert!(matches!(err_missing, EmbedError::MissingModelPath { .. }));

    let err_nonexistent = EmbeddingGemma2Embedder::new(EmbeddingGemma2Config::local_http(
        "http://127.0.0.1:9999/embed",
        Some("/definitely/missing/embeddinggemma2.onnx".into()),
        1000,
    ))
    .expect_err("non-existent model_path must fail at startup");
    assert!(matches!(
        err_nonexistent,
        EmbedError::ModelPathNotFound { .. }
    ));

    let err_300m = EmbeddingGemma300mEmbedder::new(EmbeddingGemma300mConfig::local_stdio(
        "python3",
        vec![],
        None,
        768,
        1000,
    ))
    .expect_err("300M local stdio without model_path must fail at startup");
    assert!(matches!(err_300m, EmbedError::MissingModelPath { .. }));

    // 2. NoneEmbedder keeps loop alive with no vector.
    let none_emb = NoneEmbedder;
    assert!(
        none_emb
            .embed(&EmbedInput::query("hello"))
            .expect("none ok")
            .is_none()
    );
    assert!(none_emb.tag_for(TaskPrefix::Query).is_none());

    let mut core = Core::open(
        MemStore::new(),
        CoreConfig {
            seed: 7,
            ..CoreConfig::default()
        },
        None,
    )
    .expect("open core");
    let mut doors = NoopDoors;
    let percept = Percept::minimal(
        ActorId::new("per_alice"),
        PlaceId::new("room:1"),
        Audience::new([ActorId::new("per_alice"), ActorId::new("soul")]),
        Time::from_millis(1000),
        OpaquePayload::from_bytes(b"hello".as_slice()),
    );
    core.step(
        Input::Percept(Box::new(percept)),
        Time::from_millis(1000),
        &mut doors,
    )
    .expect("loop runs cleanly with NoneEmbedder");

    // 3. Mock HTTP sidecar + stdio sidecar for EmbeddingGemma 2 (256d, 8K ctx, normalized).
    let model_file = tempfile::NamedTempFile::new().expect("create dummy weight path marker");
    let vec_256: Vec<f32> = (0..EMBEDDING_GEMMA_2_DIM)
        .map(|i| (i as f32) + 1.0)
        .collect();
    let sidecar_replies = vec![
        MockHttpReply {
            status_line: "200 OK",
            body: json!({ "embedding": vec_256 }).to_string(),
            delay_ms: 0,
        },
        MockHttpReply {
            status_line: "200 OK",
            body: json!({ "embedding": vec_256 }).to_string(),
            delay_ms: 0,
        },
    ];
    let (sidecar_url, captured_sidecar_reqs, sidecar_srv) = spawn_mock_http_server(sidecar_replies);

    let gemma2_http = EmbeddingGemma2Embedder::new(EmbeddingGemma2Config::local_http(
        sidecar_url,
        Some(model_file.path().to_path_buf()),
        5000,
    ))
    .expect("gemma2 local http initializes with existing model_path");
    assert_eq!(
        gemma2_http.max_context_tokens(),
        Some(EMBEDDING_GEMMA_2_CTX_TOKENS)
    );

    let q_vec = gemma2_http
        .embed(&EmbedInput::query("where is the key?"))
        .expect("query embed")
        .expect("non-none embedding");
    assert_eq!(q_vec.tag.embed_model_ver, EMBEDDING_GEMMA_2_VER);
    assert_eq!(q_vec.tag.dim_d, 256);
    assert_eq!(q_vec.tag.prompt_regime_id, "t_q");
    assert!(is_l2_normalized(&q_vec.values, 1e-4));

    let img_vec = gemma2_http
        .embed(&EmbedInput::image(
            TaskPrefix::Document,
            b"PNG_BYTES",
            "image/png",
            Some("desk photo"),
        ))
        .expect("image embed")
        .expect("non-none embedding");
    assert_eq!(img_vec.tag.prompt_regime_id, "t_d");
    assert!(is_l2_normalized(&img_vec.values, 1e-4));

    sidecar_srv.join().expect("sidecar server joins");
    let captured_reqs = captured_sidecar_reqs.lock().expect("lock sidecar reqs");
    let req0: Value = serde_json::from_str(&captured_reqs[0]).expect("json req0");
    assert_eq!(req0["task_prefix"], json!("t_q"));
    assert_eq!(req0["dimensions"], json!(256));
    let req1: Value = serde_json::from_str(&captured_reqs[1]).expect("json req1");
    assert_eq!(req1["task_prefix"], json!("t_d"));
    assert_eq!(req1["modality"], json!("image"));

    // 4. Stdio sidecar verification using a small executable shell script emitting 256d JSON.
    let dir = tempfile::tempdir().expect("tempdir for stdio sidecar");
    let script_path = dir.path().join("mock_gemma2_sidecar.sh");
    let raw_256 = serde_json::to_string(&vec_256).expect("serialize 256 floats");
    let script = format!("#!/bin/sh\ncat >/dev/null\nprintf '{{\"embedding\":{raw_256}}}\\n'\n");
    std::fs::write(&script_path, script).expect("write script");
    let mut perms = std::fs::metadata(&script_path).expect("stat").permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&script_path, perms).expect("chmod");

    let gemma2_stdio = EmbeddingGemma2Embedder::new(EmbeddingGemma2Config::local_stdio(
        script_path.to_string_lossy().into_owned(),
        vec![],
        Some(model_file.path().to_path_buf()),
        5000,
    ))
    .expect("stdio sidecar initializes");
    let audio_vec = gemma2_stdio
        .embed(&EmbedInput::audio_voice(
            TaskPrefix::Sts,
            b"RIFF_AUDIO",
            "audio/wav",
            Some("halo"),
        ))
        .expect("stdio audio_voice embed")
        .expect("non-none");
    assert_eq!(audio_vec.tag.prompt_regime_id, "t_sts");
    assert_eq!(audio_vec.values.len(), 256);
    assert!(is_l2_normalized(&audio_vec.values, 1e-4));

    // 5. Partition non-mixing and mismatch reporting across embedders, dims, and regimes.
    let hash_256 = HashEmbedder::new(256);
    let h_q = hash_256
        .embed(&EmbedInput::query("status"))
        .unwrap()
        .unwrap();
    let h_d = hash_256
        .embed(&EmbedInput::document("status"))
        .unwrap()
        .unwrap();

    // Exact match succeeds; regime mismatch or model_ver mismatch fails cleanly.
    assert!(partitioned_cosine(&h_q, &h_q).is_ok());
    let regime_err = partitioned_cosine(&h_q, &h_d).expect_err("t_q vs t_d must not mix directly");
    assert!(matches!(regime_err, EmbedError::PartitionMismatch { .. }));

    let v1_300m_vec = TaggedEmbedding {
        tag: EmbeddingTag {
            embed_model_ver: EMBEDDING_GEMMA_300M_VER.to_owned(),
            dim_d: 256,
            prompt_regime_id: "t_d".to_owned(),
        },
        values: vec![1.0 / 16.0; 256],
    };
    let v2_d_vec = TaggedEmbedding {
        tag: EmbeddingTag {
            embed_model_ver: EMBEDDING_GEMMA_2_VER.to_owned(),
            dim_d: 256,
            prompt_regime_id: "t_d".to_owned(),
        },
        values: q_vec.values.clone(),
    };
    let search_res = search_asymmetric_q_to_d(
        &q_vec,
        &[(EventId::new(1), v1_300m_vec), (EventId::new(2), v2_d_vec)],
        5,
    );
    assert!(search_res.has_partition_mismatch());
    assert_eq!(search_res.matches.len(), 1);
    assert_eq!(search_res.matches[0].0, EventId::new(2));

    let strict_search = search_partitioned(
        &h_q,
        &[
            (EventId::new(9), h_d.clone()),
            (EventId::new(10), h_q.clone()),
        ],
        5,
    );
    assert!(strict_search.has_partition_mismatch());
    assert_eq!(strict_search.matches.len(), 1);
    assert_eq!(strict_search.matches[0].0, EventId::new(10));

    // Single-event scripted failure is recoverable without crashing.
    let mut scripted_emb = ScriptedEmbedder::new("scripted-v1", 4);
    scripted_emb.insert_error(
        TaskPrefix::Document,
        "broken",
        EmbedError::ScriptedFailure {
            detail: "injected".to_owned(),
        },
    );
    assert!(scripted_emb.embed(&EmbedInput::document("broken")).is_err());
}

#[test]
fn dialogue_act_gap_bounded_with_none_embeddings_and_attachments_fail_closed() {
    let tag = EmbeddingTag {
        embed_model_ver: "hash-blake3-v1".to_owned(),
        dim_d: 8,
        prompt_regime_id: "t_clf".to_owned(),
    };
    let clf = InfoGapClassifier::from_weights(
        vec![1.5, -0.5, 0.25, 0.0, 0.5, -1.0, 0.75, 0.25],
        -0.4,
        Some(tag),
    )
    .expect("valid frozen head");

    // 1. With NoneEmbedder: still produces bounded [0, 1] score and never blocks.
    let none_eval = clf.score_with_embedder("can you help?", &NoneEmbedder);
    assert!(!none_eval.used_vector);
    assert!((0.0..=1.0).contains(&none_eval.score));
    assert!(none_eval.score.is_finite());

    // 2. With HashEmbedder (8d): evaluates O(d) from numbers only.
    let hash8 = HashEmbedder::new(8);
    let vec_eval = clf.score_with_embedder("can you help?", &hash8);
    assert!(vec_eval.used_vector);
    assert!((0.0..=1.0).contains(&vec_eval.score));
    assert!(vec_eval.score.is_finite());
    assert_eq!(clf.to_core_head().w_gap.len(), 8);

    // 3. Blind AttachmentHandle resolution strictly after VisibleScope + Door check.
    let alice = ActorId::new("per_alice");
    let bob = ActorId::new("per_bob");
    let soul = ActorId::new("soul");

    let mut blob_store = InMemoryAttachmentStore::new();
    blob_store.insert("blob:secret", b"SECRET_IMAGE_BYTES".to_vec());
    let handle = AttachmentHandle {
        handle_id: "blob:secret".to_owned(),
        modality_slot: ModalitySlot::Image,
        byte_len: b"SECRET_IMAGE_BYTES".len() as u64,
        expires_at_ms: Time::from_millis(50_000),
        embed_model_ver: EMBEDDING_GEMMA_2_VER.to_owned(),
    };

    let public_scope = VisibleScope::new(
        Audience::new([alice.clone(), soul.clone()]),
        Some(PlaceId::new("room:1")),
    );
    let private_bob_audience = Audience::new([bob.clone(), soul.clone()]);
    let allow = DoorDecision::Allow;

    // Scope denied -> resolves to nothing, 0 store fetches!
    let scope_err = resolve_authorized_attachment(
        &handle,
        AttachmentAccessContext {
            visible_scope: &public_scope,
            door_decision: &allow,
            item_audience: &private_bob_audience,
            item_event_id: Some(EventId::new(77)),
            now: Time::from_millis(10_000),
        },
        &blob_store,
        None,
    )
    .expect_err("private audience denied by VisibleScope");
    assert!(matches!(scope_err, AttachmentError::ScopeDenied { .. }));
    assert_eq!(blob_store.fetch_attempts(), 0);

    // Door Hold/Refuse/Strip -> resolves to nothing, 0 store fetches!
    let hold = DoorDecision::Hold {
        rule: "media_approval".to_owned(),
        reason: "PendingApproval".to_owned(),
    };
    let door_err = resolve_authorized_attachment(
        &handle,
        AttachmentAccessContext {
            visible_scope: &public_scope,
            door_decision: &hold,
            item_audience: &Audience::new([alice.clone(), soul.clone()]),
            item_event_id: Some(EventId::new(77)),
            now: Time::from_millis(10_000),
        },
        &blob_store,
        None,
    )
    .expect_err("held door must deny attachment");
    assert!(matches!(door_err, AttachmentError::DoorDenied { .. }));
    assert_eq!(blob_store.fetch_attempts(), 0);

    // Authorized scope + Allow door -> resolves safely and redacts bytes in Debug.
    let resolved = resolve_authorized_attachment(
        &handle,
        AttachmentAccessContext {
            visible_scope: &public_scope,
            door_decision: &allow,
            item_audience: &Audience::new([alice, soul]),
            item_event_id: Some(EventId::new(77)),
            now: Time::from_millis(10_000),
        },
        &blob_store,
        None,
    )
    .expect("authorized attachment resolves");
    assert_eq!(blob_store.fetch_attempts(), 1);
    assert_eq!(resolved.byte_len(), b"SECRET_IMAGE_BYTES".len());
    assert!(!format!("{resolved:?}").contains("SECRET_IMAGE_BYTES"));
}

#[test]
fn secrets_rejected_in_scenario_or_soul_files_and_loaded_from_env_or_secrets_file() {
    let scenario_with_key = json!({
        "name": "dev_chat",
        "provider": {
            "model": "gpt-5",
            "api_key": "sk-scenario-leak-should-fail"
        }
    });
    let err = validate_no_secrets_in_json(ConfigSourceKind::Scenario, &scenario_with_key)
        .expect_err("key in scenario file must fail");
    assert!(matches!(err, SecretError::SecretInConfigFile { .. }));
    let msg = err.to_string();
    assert!(
        msg.contains("environment variables") && msg.contains("secrets file"),
        "error must explain where secrets belong: {msg}"
    );

    let soul_with_key = "name = \"mono\"\nprovider_key = \"sk-soul-secret-1234567890\"";
    let soul_err = validate_no_secrets_in_str(ConfigSourceKind::Soul, soul_with_key)
        .expect_err("key in soul file must fail");
    assert!(soul_err.to_string().contains("secrets file"));

    // Clean env lookup & separate secrets file.
    let mut env = BTreeMap::new();
    env.insert("MONOAGENS_MODEL_API_KEY", "sk-env-secret-123456");
    env.insert(
        "MONOAGENS_MODEL_ENDPOINT",
        "https://api.example.invalid/v1/chat/completions",
    );
    let loaded_env = ProviderSecrets::from_lookup(|k| env.get(k).map(|v| (*v).to_owned()));
    assert_eq!(
        loaded_env.model_api_key.as_ref().map(SecretString::expose),
        Some("sk-env-secret-123456")
    );
    assert!(!format!("{loaded_env:?}").contains("sk-env-secret-123456"));

    let secrets_file = tempfile::NamedTempFile::new().expect("temp secrets file");
    std::fs::write(
        secrets_file.path(),
        "MONOAGENS_MODEL_API_KEY=sk-file-secret-abcdef\nMONOAGENS_MODEL_ENDPOINT=http://127.0.0.1:8080/v1\n",
    )
    .expect("write secrets file");
    let loaded_file =
        ProviderSecrets::from_secrets_file(secrets_file.path()).expect("load secrets file");
    assert_eq!(
        loaded_file.model_api_key.as_ref().map(SecretString::expose),
        Some("sk-file-secret-abcdef")
    );
}

#[test]
fn tool_selection_respects_static_core_budget_and_excludes_schema_drifted_procedures() {
    let alice = ActorId::new("per_alice");
    let soul = ActorId::new("soul");
    let public_aud = Audience::new([alice, soul]);
    let scope = VisibleScope::new(public_aud.clone(), Some(PlaceId::new("room:1")));

    let mut caps = Vec::new();
    for i in 0..60 {
        caps.push(CapabilityCatalogItem {
            descriptor: CapabilityDescriptor {
                id: CapabilityId::new(format!("cap.{i:02}")),
                description: if i == 42 {
                    "send status email notification".to_owned()
                } else {
                    format!("generic tool {i}")
                },
                places: vec![PlaceId::new("room:1")],
                flags: CapabilityFlags::default(),
                typical_latency_ms: 50,
                capability_schema_hash: CapabilitySchemaHash::new(format!("hash_{i:02}")),
                sensed_effect: None,
            },
            is_static_core: i < 2,
            owner_signed: i < 2,
            active: true,
            audience: public_aud.clone(),
            embedding: None,
        });
    }

    let procedures = vec![
        ProcedureCatalogItem {
            procedure_id: EventId::new(100),
            name: "valid_status_proc".to_owned(),
            steps: vec!["call cap.42".to_owned()],
            required_cap_ids: vec![CapabilityId::new("cap.42")],
            capability_schema_hash: CapabilitySchemaHash::new("hash_42"),
            audience: public_aud.clone(),
            active: true,
            retired: false,
            embedding: None,
        },
        ProcedureCatalogItem {
            procedure_id: EventId::new(101),
            name: "stale_status_proc".to_owned(),
            steps: vec!["call cap.42".to_owned()],
            required_cap_ids: vec![CapabilityId::new("cap.42")],
            capability_schema_hash: CapabilitySchemaHash::new("stale_old_hash"),
            audience: public_aud,
            active: true,
            retired: false,
            embedding: None,
        },
    ];

    let selected = select_tools(
        "send status email",
        None,
        &caps,
        &procedures,
        &scope,
        Time::from_millis(1000),
        ToolSelectionBudget::default(),
    );
    assert_eq!(selected.static_core.len(), 2);
    assert_eq!(selected.dynamic_caps.len(), 8);
    assert_eq!(selected.dynamic_caps[0].id, CapabilityId::new("cap.42"));
    assert_eq!(selected.procedures.len(), 1);
    assert_eq!(selected.procedures[0].name, "valid_status_proc");
    assert_eq!(selected.excluded_stale_procedures, 1);
}

/// Task 24.4: Configured usage paths are read; absent cached-token fields record `None` (`unknown`,
/// never `0`); `{conversation_id}` extra header travels on every request.
#[test]
fn configured_usage_paths_and_conversation_header_are_recorded() {
    use monoagens_reason::{ExtraHeaderValue, UsageFieldPaths};

    let thought = json!({
        "acts": [{
            "act": "think",
            "thought": "cache telemetry probe",
            "motivation": 3,
            "confidence_0_20": 15,
            "used_ids": []
        }]
    })
    .to_string();

    let custom_usage_body = json!({
        "model": "neutral-model-a",
        "choices": [{"index": 0, "message": {"role": "assistant", "content": thought}}],
        "usage": {
            "prompt_tokens": 9200,
            "completion_tokens": 300,
            "total_tokens": 9500,
            "custom_metrics": {
                "tokens": {
                    "reasoning": 120,
                    "cache_read": 8000
                }
            }
        }
    })
    .to_string();
    let bare_body = json!({
        "model": "neutral-model-a",
        "choices": [{"index": 0, "message": {"role": "assistant", "content": thought}}],
        "usage": {"prompt_tokens": 10, "completion_tokens": 2, "total_tokens": 12}
    })
    .to_string();

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock server");
    let addr = listener.local_addr().expect("addr");
    let captured: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let captured_srv = Arc::clone(&captured);
    let bodies = [custom_usage_body, bare_body];
    let handle = thread::spawn(move || {
        for body in bodies {
            let (mut stream, _) = listener.accept().expect("accept client");
            let mut reader = BufReader::new(stream.try_clone().expect("clone stream"));
            let mut raw = String::new();
            let mut content_length = 0_usize;
            loop {
                let mut line = String::new();
                let n = reader.read_line(&mut line).expect("read header");
                if n == 0 || line == "\r\n" || line == "\n" {
                    break;
                }
                raw.push_str(&line);
                if let Some(rest) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    content_length = rest.trim().parse().unwrap_or(0);
                }
            }
            let mut req_body = vec![0_u8; content_length];
            if content_length > 0 {
                reader.read_exact(&mut req_body).expect("read body");
            }
            raw.push_str(&String::from_utf8_lossy(&req_body));
            captured_srv.lock().expect("lock").push(raw);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
        }
    });

    let mut cfg = OpenAiModelConfig::new(
        format!("http://{addr}/v1/chat/completions"),
        Some(SecretString::new("sk-test")),
    );
    cfg.extra_headers = vec![(
        "x-conversation-session".to_owned(),
        ExtraHeaderValue::ConversationId,
    )];
    cfg.usage_paths = UsageFieldPaths {
        cached_tokens: "custom_metrics.tokens.cache_read".to_owned(),
        reasoning_tokens: "custom_metrics.tokens.reasoning".to_owned(),
        ..UsageFieldPaths::default()
    };
    let provider = OpenAiModelProvider::new(cfg).expect("provider");

    let mut cached = Vec::new();
    for idx in 0..2_u64 {
        let mut req = ModelRequest::text(
            EventId::new(idx + 1),
            ModelPurpose::Reason,
            "neutral-model-a",
            "system",
            "user",
        );
        req.conversation_id = Some("conv_stable_item_1".to_owned());
        let res = provider.invoke(&req).expect("mock response parses");
        cached.push(res.record.usage.expect("usage").cached_tokens);
    }
    handle.join().expect("server thread finishes");

    assert_eq!(cached, vec![Some(8_000), None]);
    let telemetry = provider.telemetry_snapshot();
    assert_eq!(telemetry.call_cached_tokens, vec![Some(8_000), None]);
    assert_eq!(telemetry.cumulative_cached_tokens, 8_000);
    assert_eq!(telemetry.cached_tokens_unknown_calls, 1);

    let requests = captured.lock().expect("lock captured");
    assert_eq!(requests.len(), 2);
    for raw in requests.iter() {
        assert!(
            raw.to_ascii_lowercase()
                .contains("x-conversation-session: conv_stable_item_1"),
            "missing stable conversation header in request: {raw}"
        );
    }
}

#[test]
fn extra_headers_support_literal_env_and_conversation_id_and_guard_protocol_headers() {
    use monoagens_reason::ExtraHeaderValue;

    for forbidden in ["Authorization", "content-type", "Content-Length", "HOST"] {
        let mut cfg = OpenAiModelConfig::new("http://127.0.0.1:1/v1/chat/completions", None);
        cfg.extra_headers = vec![(
            forbidden.to_owned(),
            ExtraHeaderValue::Literal(SecretString::new("bad")),
        )];
        let err = OpenAiModelProvider::new(cfg).expect_err("must reject protocol header");
        assert!(
            err.to_ascii_lowercase()
                .contains(&forbidden.to_ascii_lowercase()),
            "error must name forbidden header '{forbidden}': {err}"
        );
    }

    let env_key = "MONOAGENS_TEST_EXTRA_HEADER_VAL";
    // SAFETY: test-local env var set before provider construction in single-threaded test.
    unsafe {
        std::env::set_var(env_key, "env-secret-value");
    }

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock");
    let addr = listener.local_addr().expect("addr");
    let captured: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let cap_clone = Arc::clone(&captured);
    let body_str = json!({
        "id": "cmpl-hdr",
        "model": "neutral-model-b",
        "choices": [{
            "index": 0,
            "message": { "role": "assistant", "content": golden_multi_act_json() }
        }],
        "usage": {
            "prompt_tokens": 100,
            "completion_tokens": 20,
            "total_tokens": 120,
            "prompt_tokens_details": { "cached_tokens": 40 },
            "completion_tokens_details": { "reasoning_tokens": 5 }
        }
    })
    .to_string();
    let handle = thread::spawn(move || {
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().expect("accept");
            let mut reader = BufReader::new(stream.try_clone().expect("clone stream"));
            let mut raw = String::new();
            let mut content_length = 0_usize;
            loop {
                let mut line = String::new();
                let n = reader.read_line(&mut line).expect("read header");
                if n == 0 || line == "\r\n" || line == "\n" {
                    break;
                }
                raw.push_str(&line);
                if let Some(rest) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    content_length = rest.trim().parse().unwrap_or(0);
                }
            }
            let mut req_body = vec![0_u8; content_length];
            if content_length > 0 {
                reader.read_exact(&mut req_body).expect("read body");
            }
            raw.push_str(&String::from_utf8_lossy(&req_body));
            cap_clone.lock().expect("lock captured").push(raw);
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body_str.len(),
                body_str
            );
            let _ = stream.write_all(resp.as_bytes());
            let _ = stream.flush();
        }
    });

    let mut cfg = OpenAiModelConfig::new(format!("http://{addr}/v1/chat/completions"), None);
    cfg.extra_headers = vec![
        (
            "x-literal-hdr".to_owned(),
            ExtraHeaderValue::Literal(SecretString::new("lit-secret-value")),
        ),
        (
            "x-env-hdr".to_owned(),
            ExtraHeaderValue::Env(env_key.to_owned()),
        ),
        ("x-conv-hdr".to_owned(), ExtraHeaderValue::ConversationId),
    ];
    let dbg = format!("{cfg:?}");
    assert!(!dbg.contains("lit-secret-value"));
    assert!(!dbg.contains("env-secret-value"));
    assert!(dbg.contains("x-literal-hdr"));
    assert!(dbg.contains("x-env-hdr"));
    assert!(dbg.contains("x-conv-hdr"));

    let provider = OpenAiModelProvider::new(cfg).expect("provider");
    let mut req1 = ModelRequest::text(
        EventId::new(1),
        ModelPurpose::Reason,
        "neutral-model-b",
        "sys",
        "usr",
    );
    req1.conversation_id = Some("conv-alpha".to_owned());
    let res1 = provider.invoke(&req1).expect("call 1");
    let u1 = res1.record.usage.expect("usage");
    assert_eq!(u1.cached_tokens, Some(40));
    assert_eq!(u1.reasoning_tokens, Some(5));

    let mut req2 = ModelRequest::text(
        EventId::new(2),
        ModelPurpose::Reason,
        "neutral-model-b",
        "sys",
        "usr",
    );
    req2.conversation_id = Some("conv-beta".to_owned());
    let _ = provider.invoke(&req2).expect("call 2");
    handle.join().expect("server finishes");

    let reqs = captured.lock().expect("captured");
    assert_eq!(reqs.len(), 2);
    let r1 = reqs[0].to_ascii_lowercase();
    let r2 = reqs[1].to_ascii_lowercase();
    assert!(r1.contains("x-literal-hdr: lit-secret-value"));
    assert!(r1.contains("x-env-hdr: env-secret-value"));
    assert!(r1.contains("x-conv-hdr: conv-alpha"));
    assert!(r2.contains("x-conv-hdr: conv-beta"));
}

#[test]
fn source_scan_provider_and_gateway_sources_are_provider_neutral() {
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root");
    let scan_dirs = [
        workspace_root.join("crates/reason/src"),
        workspace_root.join("crates/gateway/src"),
        workspace_root.join("crates/lab/src"),
    ];
    let forbidden_providers = [
        "opencode",
        "x-opencode-session",
        "anthropic",
        "claude",
        "openrouter",
        "deepseek",
        "qwen",
        "llama",
        "mistral",
        "sonnet",
        "opus",
        "haiku",
    ];
    let strict_model_names = ["gemini"];
    let mut violations = Vec::new();
    let mut stack = scan_dirs.to_vec();
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).expect("read_dir") {
            let entry = entry.expect("entry");
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
                let text = std::fs::read_to_string(&path).expect("read .rs");
                let lower = text.to_ascii_lowercase();
                for term in forbidden_providers {
                    if lower.contains(term) {
                        violations.push(format!("{}: contains '{term}'", path.display()));
                    }
                }
                let p_str = path.to_string_lossy();
                let is_metric_or_dataset_const = p_str
                    .ends_with("crates/lab/src/datasets/report.rs")
                    || p_str.ends_with("crates/lab/src/datasets/stale.rs")
                    || p_str.ends_with("crates/lab/src/datasets/runner.rs");
                if !is_metric_or_dataset_const {
                    for term in strict_model_names {
                        if lower.contains(term) {
                            violations.push(format!("{}: contains '{term}'", path.display()));
                        }
                    }
                }
            }
        }
    }
    assert!(
        violations.is_empty(),
        "provider-neutrality scan failed:\n{}",
        violations.join("\n")
    );
}

#[test]
fn output_limits_per_purpose_normal_at_limit_and_empty_at_limit() {
    use monoagens_reason::{OutputLimitProfile, PRODUCT_DEFAULT_MAX_OUTPUT_TOKENS};

    assert_eq!(PRODUCT_DEFAULT_MAX_OUTPUT_TOKENS, 16_384);
    assert_eq!(OutputLimitProfile::default().reason, 16_384);
    assert_eq!(OutputLimitProfile::default().classify, 16_384);
    assert_eq!(OutputLimitProfile::default().consolidate, 16_384);

    let normal_body = json!({
        "model": "neutral-profile-model",
        "choices": [{
            "index": 0,
            "finish_reason": "stop",
            "message": { "role": "assistant", "content": golden_multi_act_json() }
        }],
        "usage": { "prompt_tokens": 100, "completion_tokens": 40, "total_tokens": 140 }
    })
    .to_string();
    let at_limit_valid_body = json!({
        "model": "neutral-profile-model",
        "choices": [{
            "index": 0,
            "finish_reason": "length",
            "message": {
                "role": "assistant",
                "content": json!({"label": "matched", "score": 0.9, "rationale": null}).to_string()
            }
        }],
        "usage": { "prompt_tokens": 100, "completion_tokens": 512, "total_tokens": 612 }
    })
    .to_string();
    let empty_at_limit_body = json!({
        "model": "neutral-profile-model",
        "choices": [{
            "index": 0,
            "finish_reason": "length",
            "message": { "role": "assistant", "content": "" }
        }],
        "usage": {
            "prompt_tokens": 100,
            "completion_tokens": 1024,
            "total_tokens": 1124,
            "completion_tokens_details": { "reasoning_tokens": 1024 }
        }
    })
    .to_string();

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock");
    let addr = listener.local_addr().expect("addr");
    let captured_bodies: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
    let cap_clone = Arc::clone(&captured_bodies);
    let responses = [normal_body, at_limit_valid_body, empty_at_limit_body];
    let handle = thread::spawn(move || {
        for body_str in responses {
            let (mut stream, _) = listener.accept().expect("accept");
            let mut reader = BufReader::new(stream.try_clone().expect("clone stream"));
            let mut content_length = 0_usize;
            loop {
                let mut line = String::new();
                let n = reader.read_line(&mut line).expect("read header");
                if n == 0 || line == "\r\n" || line == "\n" {
                    break;
                }
                if let Some(rest) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    content_length = rest.trim().parse().unwrap_or(0);
                }
            }
            let mut req_body = vec![0_u8; content_length];
            if content_length > 0 {
                reader.read_exact(&mut req_body).expect("read body");
            }
            let val: Value = serde_json::from_slice(&req_body).expect("json body");
            cap_clone.lock().expect("lock").push(val);
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body_str.len(),
                body_str
            );
            let _ = stream.write_all(resp.as_bytes());
            let _ = stream.flush();
        }
    });

    let mut cfg = OpenAiModelConfig::new(format!("http://{addr}/v1/chat/completions"), None);
    cfg.output_limit_param = "max_output_tokens".to_owned();
    cfg.profile_name = Some("neutral-fast".to_owned());
    cfg.output_limits = OutputLimitProfile {
        reason: 1024,
        classify: 512,
        consolidate: 2048,
    };
    let provider = OpenAiModelProvider::new(cfg).expect("provider");

    // 1. Normal Reason call
    let req_normal = ModelRequest::text(
        EventId::new(1),
        ModelPurpose::Reason,
        "neutral-profile-model",
        "sys",
        "usr",
    );
    provider.invoke(&req_normal).expect("normal call succeeds");
    assert_eq!(provider.telemetry_snapshot().runaways_reason, 0);

    // 2. Classify call at the limit with valid output -> parsed + recorded as classify runaway
    let req_at_limit = ModelRequest::text(
        EventId::new(2),
        ModelPurpose::Classify,
        "neutral-profile-model",
        "sys",
        "usr",
    );
    let ok_limit = provider
        .invoke(&req_at_limit)
        .expect("valid output at limit still parses");
    assert!(matches!(ok_limit.output, ModelOutput::Verdict(_)));
    assert_eq!(provider.telemetry_snapshot().runaways_classify, 1);

    // 3. Reason call at the limit with empty visible output -> error names reasoning used the whole limit + setting + profile
    let req_empty = ModelRequest::text(
        EventId::new(3),
        ModelPurpose::Reason,
        "neutral-profile-model",
        "sys",
        "usr",
    );
    let err = provider
        .invoke(&req_empty)
        .expect_err("empty output at limit must fail");
    assert!(
        err.detail.contains("reasoning used the whole limit"),
        "unexpected error: {}",
        err.detail
    );
    assert!(err.detail.contains("output_limits.reason"));
    assert!(err.detail.contains("neutral-fast"));
    assert_eq!(provider.telemetry_snapshot().runaways_reason, 1);

    handle.join().expect("server finishes");
    let bodies = captured_bodies.lock().expect("bodies");
    assert_eq!(bodies[0]["max_output_tokens"], json!(1024));
    assert_eq!(bodies[1]["max_output_tokens"], json!(512));
    assert_eq!(bodies[2]["max_output_tokens"], json!(1024));
}

#[test]
fn pilot_output_format_failure_fixtures_record_and_reject_in_strict_schema() {
    let batch_schema = strict_thought_batch_schema();
    let batch_validator = jsonschema::validator_for(&batch_schema).expect("valid batch schema");
    let clf_schema = strict_classification_verdict_schema();
    let clf_validator = jsonschema::validator_for(&clf_schema).expect("valid classify schema");

    let reason_fixtures = [
        (
            "acts[0].motivation",
            "1..=5",
            json!({
                "protocol_version": {"major": 1, "minor": 0},
                "acts": [{
                    "act": "think",
                    "thought": "test",
                    "motivation": 0,
                    "confidence_0_20": 10,
                    "used_ids": [],
                    "importance": null,
                    "basis": null,
                    "value": null
                }]
            }),
        ),
        (
            "acts[0].places",
            "at least one place",
            json!({
                "protocol_version": {"major": 1, "minor": 0},
                "acts": [{
                    "act": "act",
                    "capability_id": "a.speak",
                    "places": [],
                    "people": [],
                    "payload": [104, 105],
                    "motivation": 3,
                    "confidence_0_20": 15,
                    "used_ids": [],
                    "expected": null,
                    "domain": null
                }]
            }),
        ),
        (
            "acts[0].capability_id",
            "must not be empty",
            json!({
                "protocol_version": {"major": 1, "minor": 0},
                "acts": [{
                    "act": "act",
                    "capability_id": "",
                    "places": ["room"],
                    "people": [],
                    "payload": [104, 105],
                    "motivation": 3,
                    "confidence_0_20": 15,
                    "used_ids": [],
                    "expected": null,
                    "domain": null
                }]
            }),
        ),
    ];

    for (expected_field, expected_reason_substr, payload) in reason_fixtures {
        assert!(
            !batch_validator.is_valid(&payload),
            "tightened ThoughtBatch schema must reject invalid {expected_field}"
        );
        let provider = ScriptedModelProvider::from_replies([ScriptedReply::RawText {
            text: payload.to_string(),
            usage: monoagens_reason::TokenUsage::default(),
            latency_ms: 1,
        }]);
        let req = ModelRequest::text(
            EventId::new(1),
            ModelPurpose::Reason,
            "neutral-test-model",
            "sys",
            "usr",
        );
        let err = provider.invoke(&req).expect_err("must fail parse");
        let rec = err
            .output_format_record()
            .expect("must produce OutputFormatFailureRecord");
        assert_eq!(rec.purpose, ModelPurpose::Reason);
        assert_eq!(rec.field, expected_field);
        assert!(
            rec.reason.contains(expected_reason_substr),
            "unexpected reason for {expected_field}: {}",
            rec.reason
        );
        assert!(rec.strict_schema);
    }

    let classify_payload = json!({
        "protocol_version": {"major": 1, "minor": 0},
        "label": "",
        "score": 0.5,
        "rationale": null
    });
    assert!(
        !clf_validator.is_valid(&classify_payload),
        "tightened ClassificationVerdict schema must reject empty label"
    );
    let provider = ScriptedModelProvider::from_replies([ScriptedReply::RawText {
        text: classify_payload.to_string(),
        usage: monoagens_reason::TokenUsage::default(),
        latency_ms: 1,
    }]);
    let req = ModelRequest::text(
        EventId::new(2),
        ModelPurpose::Classify,
        "neutral-test-model",
        "sys",
        "usr",
    );
    let err = provider.invoke(&req).expect_err("must fail parse");
    let rec = err
        .output_format_record()
        .expect("must produce OutputFormatFailureRecord");
    assert_eq!(rec.purpose, ModelPurpose::Classify);
    assert_eq!(rec.field, "label");
    assert!(rec.reason.contains("non-empty string"));
    assert!(rec.strict_schema);
}
