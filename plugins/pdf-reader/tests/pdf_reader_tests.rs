//! Integration tests for `plugin-pdf-reader` covering real `lopdf` PDF parsing,
//! page-aware overlapping chunking, persistent `pdf_rag_index.json`, BM25 & hybrid
//! cosine RRF retrieval, bounded `pdf.read_page`, `pdf.list`, `pdf.delete`, Waist-2
//! error handling, and `MONOAGENS_PLUGIN_CONFIG` / `MONOAGENS_PLUGIN_STATE_DIR` stdio execution.

#![forbid(unsafe_code)]

use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use lopdf::{Document, Object, Stream, dictionary};
use monoagens_body_kit::PersistentBodyRuntime;
use monoagens_body_sdk::{
    ActionExecuteParams, ActionExecuteResult, ActionId, ActorId, Audience, BodyDiscoverParams,
    BodyDiscoverResult, BodyHandler, CapabilityId, OpaquePayload, PlaceId, ReceiptOutcome, RpcId,
    Waist2ErrorCode, Waist2Headers, Waist2Method, Waist2Request, Waist2Response, sort_capabilities,
};
use plugin_pdf_reader::{
    DEFAULT_BODY_ID, DEFAULT_PLACE_ID, ENV_EMBED_ENDPOINT, ENV_PLUGIN_CONFIG, ENV_PLUGIN_STATE_DIR,
    PDF_RAG_INDEX_FILENAME, PdfRagIndex, PdfReaderBody, PdfReaderConfig,
};
use serde_json::{Value, json};
use tempfile::TempDir;

fn build_real_pdf(title: Option<&str>, page_texts: &[&str]) -> Vec<u8> {
    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let font_id = doc.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "Type1",
        "BaseFont" => "Helvetica",
        "Encoding" => "WinAnsiEncoding",
    });
    let resources_id = doc.add_object(dictionary! {
        "Font" => dictionary! { "F1" => font_id },
    });

    let mut kids: Vec<Object> = Vec::with_capacity(page_texts.len());
    for text in page_texts {
        let escaped = text
            .replace('\\', "\\\\")
            .replace('(', "\\(")
            .replace(')', "\\)");
        let stream_bytes = format!("BT /F1 12 Tf 72 720 Td ({escaped}) Tj ET").into_bytes();
        let content_id = doc.add_object(Stream::new(dictionary! {}, stream_bytes));
        let page_id = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "Contents" => content_id,
            "Resources" => resources_id,
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
        });
        kids.push(page_id.into());
    }

    doc.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => kids,
            "Count" => page_texts.len() as i64,
        }),
    );

    if let Some(title_text) = title {
        let info_id = doc.add_object(dictionary! {
            "Title" => Object::string_literal(title_text),
        });
        doc.trailer.set("Info", info_id);
    }

    let catalog_id = doc.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => pages_id,
    });
    doc.trailer.set("Root", catalog_id);

    let mut buf = Vec::new();
    doc.save_to(&mut buf).expect("save generated PDF");
    buf
}

fn exec_cap(body: &mut PdfReaderBody, seq: u64, cap: &str, payload: Value) -> Value {
    let params = ActionExecuteParams {
        headers: Waist2Headers::new(format!("test-idem-{seq}")),
        action_id: ActionId::new(format!("test-act-{seq}")),
        capability_id: CapabilityId::new(cap),
        places: vec![PlaceId::new(DEFAULT_PLACE_ID)],
        audience: Audience::new([ActorId::new("per_owner")]),
        payload: OpaquePayload::from_bytes(payload.to_string().into_bytes()),
        timeout_ms: Some(5_000),
        h_act: None,
    };
    let res = body.execute(params).expect("execute should succeed");
    assert!(res.accepted);
    let receipt = res.receipt.expect("receipt present");
    assert_eq!(
        receipt.outcome,
        ReceiptOutcome::Done,
        "unexpected receipt outcome: {:?} detail={:?}",
        receipt.outcome,
        receipt.detail
    );
    let detail = receipt.detail.expect("receipt detail present");
    serde_json::from_str(&detail).expect("detail is valid JSON")
}

fn read_http_body(stream: &mut TcpStream) -> String {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut content_len = 0usize;
    loop {
        let mut line = String::new();
        let n = reader.read_line(&mut line).unwrap();
        if n == 0 || line == "\r\n" || line == "\n" {
            break;
        }
        if let Some((k, v)) = line.split_once(':')
            && k.eq_ignore_ascii_case("content-length")
        {
            content_len = v.trim().parse::<usize>().unwrap_or(0);
        }
    }
    let mut buf = vec![0u8; content_len];
    reader.read_exact(&mut buf).unwrap();
    String::from_utf8_lossy(&buf).into_owned()
}

type RecordedEmbedRequests = Arc<Mutex<Vec<(String, Vec<String>)>>>;

fn spawn_mock_embed_server(
    expected_requests: usize,
    fail_with_500: bool,
) -> (String, RecordedEmbedRequests, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock embed server");
    let addr = listener.local_addr().unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let seen_clone = Arc::clone(&seen);

    let handle = thread::spawn(move || {
        for _ in 0..expected_requests {
            let (mut stream, _) = listener.accept().unwrap();
            let raw_body = read_http_body(&mut stream);
            let req_val: Value = serde_json::from_str(&raw_body).unwrap();
            let regime = req_val["regime"].as_str().unwrap_or("").to_owned();
            let texts: Vec<String> = req_val["texts"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|v| v.as_str().map(ToOwned::to_owned))
                .collect();
            seen_clone.lock().unwrap().push((regime.clone(), texts.clone()));

            if fail_with_500 {
                let err_body = "mock embedder failure";
                let resp = format!(
                    "HTTP/1.1 500 Internal Server Error\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    err_body.len(),
                    err_body
                );
                let _ = stream.write_all(resp.as_bytes());
                let _ = stream.flush();
                continue;
            }

            // Encode semantic topics orthogonally:
            // dim 0 = Rust / ownership / systems
            // dim 1 = WhatsApp / Signal / end-to-end messaging privacy
            // dim 2 = Indonesian coffee / brewing
            let embeddings: Vec<Vec<f32>> = texts
                .iter()
                .map(|t| {
                    let lower = t.to_lowercase();
                    if lower.contains("whatsapp")
                        || lower.contains("signal")
                        || lower.contains("secure chat")
                        || lower.contains("private messaging")
                    {
                        vec![0.0, 1.0, 0.0, 0.0]
                    } else if lower.contains("coffee") || lower.contains("kopi") {
                        vec![0.0, 0.0, 1.0, 0.0]
                    } else if lower.contains("rust") || lower.contains("memory") {
                        vec![1.0, 0.0, 0.0, 0.0]
                    } else {
                        vec![0.0, 0.0, 0.0, 1.0]
                    }
                })
                .collect();

            let resp_body = json!({ "embeddings": embeddings }).to_string();
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                resp_body.len(),
                resp_body
            );
            let _ = stream.write_all(resp.as_bytes());
            let _ = stream.flush();
        }
    });

    (format!("http://{addr}"), seen, handle)
}

#[test]
fn discover_lists_sorted_five_capabilities_and_rejects_unknown() {
    let mut body = PdfReaderBody::new(PdfReaderConfig::default(), None);
    let discover = body
        .discover(BodyDiscoverParams {
            headers: Waist2Headers::new("d-1"),
            body_id: None,
        })
        .unwrap();

    assert_eq!(discover.body_id.as_str(), DEFAULT_BODY_ID);
    assert_eq!(discover.places, vec![PlaceId::new(DEFAULT_PLACE_ID)]);
    let cap_ids: Vec<&str> = discover.capabilities.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(
        cap_ids,
        vec![
            "pdf.delete",
            "pdf.index",
            "pdf.list",
            "pdf.read_page",
            "pdf.search",
        ]
    );
    let mut sorted = discover.capabilities.clone();
    sort_capabilities(&mut sorted);
    assert_eq!(discover.capabilities, sorted);
    for cap in &discover.capabilities {
        assert!(cap.capability_schema_hash.as_str().starts_with("sha256:"));
    }

    let err = body
        .execute(ActionExecuteParams {
            headers: Waist2Headers::new("unk-1"),
            action_id: ActionId::new("act-unk"),
            capability_id: CapabilityId::new("pdf.nonexistent"),
            places: vec![PlaceId::new(DEFAULT_PLACE_ID)],
            audience: Audience::new([ActorId::new("per_owner")]),
            payload: OpaquePayload::from_bytes(b"{}".as_slice()),
            timeout_ms: Some(1_000),
            h_act: None,
        })
        .unwrap_err();
    assert_eq!(err.code, Waist2ErrorCode::UnknownCapability.code());
}

#[test]
fn index_persist_reload_search_read_page_list_delete_end_to_end() {
    let state_dir = TempDir::new().unwrap();
    let pdf_bytes = build_real_pdf(
        Some("Multi-Topic Guide"),
        &[
            "Page 1 covers Rust memory safety, ownership, borrowing, and zero-cost abstractions.",
            "Page 2 explains WhatsApp encryption using the Signal double ratchet protocol.",
            "Page 3 shares Indonesian coffee recipes including kopi tubruk and Aceh Gayo pour over.",
        ],
    );
    let b64 = BASE64_STANDARD.encode(&pdf_bytes);

    {
        let mut body = PdfReaderBody::new(PdfReaderConfig::default(), Some(state_dir.path().to_path_buf()));
        body.set_now_ms(12_345);

        // 1. pdf.index extracts all 3 pages, persists pdf_rag_index.json, returns compact metadata.
        let idx_out = exec_cap(
            &mut body,
            1,
            "pdf.index",
            json!({
                "bytes_base64": b64,
                "doc_id": "guide-2026"
            }),
        );
        assert_eq!(idx_out["doc_id"], "guide-2026");
        assert_eq!(idx_out["title"], "Multi-Topic Guide");
        assert_eq!(idx_out["page_count"], 3);
        assert_eq!(idx_out["chunk_count"], 3);
        assert_eq!(idx_out["indexed_with_embeddings"], false);
        let idx_str = idx_out.to_string();
        assert!(!idx_str.contains("WhatsApp encryption"), "pdf.index must not dump full raw text");
        assert!(!idx_str.contains("kopi tubruk"), "pdf.index must not dump full raw text");
        assert!(state_dir.path().join(PDF_RAG_INDEX_FILENAME).exists());

        // 2. pdf.search ranks Page 2 #1 with exact citation [guide-2026 p.2].
        let search_out = exec_cap(
            &mut body,
            2,
            "pdf.search",
            json!({
                "query": "WhatsApp encryption",
                "top_k": 3
            }),
        );
        assert_eq!(search_out["total_hits"], 1);
        let hits = search_out["hits"].as_array().unwrap();
        assert_eq!(hits[0]["page"], 2);
        assert_eq!(hits[0]["citation"], "[guide-2026 p.2]");
        assert!(hits[0]["bm25_score"].as_f64().unwrap() > 0.0);
        assert!(hits[0]["semantic_score"].is_null());
    }

    // 3. Reload PdfRagIndex & PdfReaderBody from disk and verify documents & chunks are preserved.
    {
        let reloaded_index = PdfRagIndex::load_or_new(Some(state_dir.path()));
        assert_eq!(reloaded_index.documents().len(), 1);
        assert_eq!(reloaded_index.documents()["guide-2026"].page_count, 3);
        assert_eq!(reloaded_index.documents()["guide-2026"].chunks.len(), 3);

        let cfg = PdfReaderConfig {
            max_pages_per_read: 2,
            ..Default::default()
        };
        let mut body = PdfReaderBody::new(cfg, Some(state_dir.path().to_path_buf()));

        let search_out = exec_cap(
            &mut body,
            3,
            "pdf.search",
            json!({ "query": "Indonesian coffee kopi tubruk" }),
        );
        assert_eq!(search_out["hits"][0]["page"], 3);
        assert_eq!(search_out["hits"][0]["citation"], "[guide-2026 p.3]");

        // 4. pdf.read_page respects max_pages_per_read (2).
        let read_out = exec_cap(
            &mut body,
            4,
            "pdf.read_page",
            json!({
                "doc_id": "guide-2026",
                "start_page": 1,
                "end_page": 3
            }),
        );
        assert_eq!(read_out["start_page"], 1);
        assert_eq!(read_out["end_page"], 2);
        let pages = read_out["pages"].as_array().unwrap();
        assert_eq!(pages.len(), 2);
        assert_eq!(pages[0]["page"], 1);
        assert_eq!(pages[1]["page"], 2);

        // 5. pdf.list returns compact summary.
        let list_out = exec_cap(&mut body, 5, "pdf.list", json!({}));
        let docs = list_out["documents"].as_array().unwrap();
        assert_eq!(docs.len(), 1);
        assert_eq!(docs[0]["doc_id"], "guide-2026");
        assert_eq!(docs[0]["page_count"], 3);

        // 6. pdf.delete removes document & persists removal.
        let del_out = exec_cap(&mut body, 6, "pdf.delete", json!({ "doc_id": "guide-2026" }));
        assert_eq!(del_out["deleted"], true);

        let list_after = exec_cap(&mut body, 7, "pdf.list", json!({}));
        assert_eq!(list_after["documents"].as_array().unwrap().len(), 0);
    }

    let after_delete_reload = PdfRagIndex::load_or_new(Some(state_dir.path()));
    assert!(after_delete_reload.documents().is_empty());
}

#[test]
fn hybrid_bm25_plus_cosine_rrf_with_mock_http_server_and_fallback() {
    let state_dir = TempDir::new().unwrap();
    // Page 2 is concise about WhatsApp/Signal privacy, while Page 1 mentions "chat" incidentally.
    // Query 1 ("secure chat privacy") tests BM25 + cosine RRF fusion ranking Page 2 #1.
    // Query 2 ("private messaging") has zero lexical overlap with Page 2, proving semantic RRF rescue.
    let pdf_bytes = build_real_pdf(
        Some("Architecture Notes"),
        &[
            "Rust systems overview with chat logging diagnostics and memory pools.",
            "Signal protocol provides WhatsApp end to end privacy.",
            "Indonesian coffee brewing ratios for kopi tubruk.",
        ],
    );
    let pdf_path = state_dir.path().join("arch.pdf");
    fs::write(&pdf_path, &pdf_bytes).unwrap();

    let (endpoint, seen_reqs, server_handle) = spawn_mock_embed_server(3, false);

    let cfg = PdfReaderConfig {
        embed_endpoint: Some(endpoint),
        ..Default::default()
    };
    let mut body = PdfReaderBody::new(cfg, Some(state_dir.path().to_path_buf()));

    let idx_out = exec_cap(
        &mut body,
        10,
        "pdf.index",
        json!({
            "path": pdf_path.to_str().unwrap(),
            "doc_id": "arch"
        }),
    );
    assert_eq!(idx_out["indexed_with_embeddings"], true);
    assert_eq!(idx_out["page_count"], 3);

    // Search 1: hybrid BM25 + semantic RRF fusion.
    let search_out = exec_cap(
        &mut body,
        11,
        "pdf.search",
        json!({
            "query": "secure chat privacy",
            "top_k": 3
        }),
    );
    let hits = search_out["hits"].as_array().unwrap();
    assert!(!hits.is_empty());
    assert_eq!(hits[0]["page"], 2);
    assert_eq!(hits[0]["citation"], "[arch p.2]");
    assert!(hits[0]["bm25_score"].as_f64().unwrap() > 0.0);
    assert!(hits[0]["semantic_score"].as_f64().unwrap() > 0.99);

    // Search 2: zero lexical overlap ("private messaging"), rescued by semantic cosine + RRF!
    let semantic_only_out = exec_cap(
        &mut body,
        12,
        "pdf.search",
        json!({
            "query": "private messaging",
            "top_k": 2
        }),
    );
    let sem_hits = semantic_only_out["hits"].as_array().unwrap();
    assert!(!sem_hits.is_empty());
    assert_eq!(sem_hits[0]["page"], 2);
    assert_eq!(sem_hits[0]["bm25_score"], 0.0);
    assert!(sem_hits[0]["semantic_score"].as_f64().unwrap() > 0.99);

    server_handle.join().unwrap();
    let recorded = seen_reqs.lock().unwrap();
    assert_eq!(recorded.len(), 3);
    assert_eq!(recorded[0].0, "document");
    assert_eq!(recorded[0].1.len(), 3);
    assert_eq!(recorded[1].0, "query");
    assert_eq!(recorded[1].1, vec!["secure chat privacy".to_owned()]);
    assert_eq!(recorded[2].0, "query");
    assert_eq!(recorded[2].1, vec!["private messaging".to_owned()]);

    // Verify graceful fallback to BM25 when embedding sidecar returns HTTP 500!
    let (err_endpoint, _seen2, err_handle) = spawn_mock_embed_server(1, true);
    let fallback_cfg = PdfReaderConfig {
        embed_endpoint: Some(err_endpoint),
        ..Default::default()
    };
    let mut fallback_body = PdfReaderBody::new(fallback_cfg, Some(state_dir.path().to_path_buf()));

    let fallback_search = exec_cap(
        &mut fallback_body,
        13,
        "pdf.search",
        json!({
            "query": "WhatsApp privacy",
            "top_k": 2
        }),
    );
    let fb_hits = fallback_search["hits"].as_array().unwrap();
    assert_eq!(fb_hits[0]["page"], 2);
    assert!(fb_hits[0]["semantic_score"].is_null());
    assert!(fb_hits[0]["bm25_score"].as_f64().unwrap() > 0.0);
    err_handle.join().unwrap();
}

#[test]
fn stdio_binary_auto_inits_plugin_config_and_reloads_state() {
    let dir = TempDir::new().unwrap();
    let cfg_path = dir.path().join("config/pdf-reader.toml");
    let state_dir = dir.path().join("state");
    let bin = env!("CARGO_BIN_EXE_plugin-pdf-reader");

    let pdf_bytes = build_real_pdf(
        Some("Stdio Test"),
        &[
            "Page one Rust ownership.",
            "Page two WhatsApp encryption.",
            "Page three Indonesian coffee.",
        ],
    );
    let b64 = BASE64_STANDARD.encode(&pdf_bytes);

    // Run 1: auto-creates pdf-reader.toml and indexes PDF into state_dir.
    let mut child1 = Command::new(bin)
        .env(ENV_PLUGIN_CONFIG, &cfg_path)
        .env(ENV_PLUGIN_STATE_DIR, &state_dir)
        .env_remove(ENV_EMBED_ENDPOINT)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn plugin-pdf-reader");

    {
        let mut stdin = child1.stdin.take().unwrap();
        let req1 = Waist2Request::new(
            Some(RpcId::Int(1)),
            Waist2Method::BodyDiscover,
            BodyDiscoverParams {
                headers: Waist2Headers::new("stdio-disc-1"),
                body_id: None,
            },
        );
        let req2 = Waist2Request::new(
            Some(RpcId::Int(2)),
            Waist2Method::ActionExecute,
            ActionExecuteParams {
                headers: Waist2Headers::new("stdio-idx-1"),
                action_id: ActionId::new("act-stdio-idx-1"),
                capability_id: CapabilityId::new("pdf.index"),
                places: vec![PlaceId::new(DEFAULT_PLACE_ID)],
                audience: Audience::new([ActorId::new("per_owner")]),
                payload: OpaquePayload::from_bytes(
                    json!({ "bytes_base64": b64, "doc_id": "stdio-doc" })
                        .to_string()
                        .into_bytes(),
                ),
                timeout_ms: Some(5_000),
                h_act: None,
            },
        );
        writeln!(stdin, "{}", serde_json::to_string(&req1).unwrap()).unwrap();
        writeln!(stdin, "{}", serde_json::to_string(&req2).unwrap()).unwrap();
        stdin.flush().unwrap();
    }

    let out1 = child1.wait_with_output().unwrap();
    assert!(out1.status.success());
    let stdout1 = String::from_utf8(out1.stdout).unwrap();
    let lines1: Vec<&str> = stdout1.lines().collect();
    assert_eq!(lines1.len(), 2, "expected pure 2 NDJSON lines, got: {stdout1}");
    let disc_resp: Waist2Response<BodyDiscoverResult> = serde_json::from_str(lines1[0]).unwrap();
    assert_eq!(disc_resp.result.unwrap().body_id.as_str(), DEFAULT_BODY_ID);
    let idx_resp: Waist2Response<ActionExecuteResult> = serde_json::from_str(lines1[1]).unwrap();
    assert!(idx_resp.result.unwrap().accepted);

    assert!(cfg_path.exists(), "pdf-reader.toml should be auto-generated");
    assert!(state_dir.join(PDF_RAG_INDEX_FILENAME).exists());

    // Edit pdf-reader.toml to cap max_pages_per_read = 1, then respawn & verify both reload!
    fs::write(&cfg_path, "version = 1\nmax_pages_per_read = 1\n").unwrap();

    let mut child2 = Command::new(bin)
        .env(ENV_PLUGIN_CONFIG, &cfg_path)
        .env(ENV_PLUGIN_STATE_DIR, &state_dir)
        .env_remove(ENV_EMBED_ENDPOINT)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("respawn plugin-pdf-reader");

    {
        let mut stdin = child2.stdin.take().unwrap();
        let req3 = Waist2Request::new(
            Some(RpcId::Int(3)),
            Waist2Method::ActionExecute,
            ActionExecuteParams {
                headers: Waist2Headers::new("stdio-read-1"),
                action_id: ActionId::new("act-stdio-read-1"),
                capability_id: CapabilityId::new("pdf.read_page"),
                places: vec![PlaceId::new(DEFAULT_PLACE_ID)],
                audience: Audience::new([ActorId::new("per_owner")]),
                payload: OpaquePayload::from_bytes(
                    json!({ "doc_id": "stdio-doc", "start_page": 1, "end_page": 3 })
                        .to_string()
                        .into_bytes(),
                ),
                timeout_ms: Some(5_000),
                h_act: None,
            },
        );
        writeln!(stdin, "{}", serde_json::to_string(&req3).unwrap()).unwrap();
        stdin.flush().unwrap();
    }

    let out2 = child2.wait_with_output().unwrap();
    assert!(out2.status.success());
    let stdout2 = String::from_utf8(out2.stdout).unwrap();
    let resp3: Waist2Response<ActionExecuteResult> =
        serde_json::from_str(stdout2.trim()).unwrap();
    let detail: Value =
        serde_json::from_str(resp3.result.unwrap().receipt.unwrap().detail.unwrap().as_str())
            .unwrap();
    assert_eq!(detail["end_page"], 1, "reloaded max_pages_per_read=1 must cap range");
    assert_eq!(detail["pages"].as_array().unwrap().len(), 1);
}

#[test]
fn runtime_handles_idempotency_and_unknown_capability() {
    let mut rt = PersistentBodyRuntime::new(None);
    let mut body = PdfReaderBody::new(PdfReaderConfig::default(), None);

    let req = Waist2Request::new(
        Some(RpcId::Int(1)),
        Waist2Method::ActionExecute,
        ActionExecuteParams {
            headers: Waist2Headers::new("rt-unk-1"),
            action_id: ActionId::new("act-rt-unk"),
            capability_id: CapabilityId::new("pdf.bogus"),
            places: vec![PlaceId::new(DEFAULT_PLACE_ID)],
            audience: Audience::new([ActorId::new("per_owner")]),
            payload: OpaquePayload::from_bytes(b"{}".as_slice()),
            timeout_ms: Some(1_000),
            h_act: None,
        },
    );
    let line = serde_json::to_string(&req).unwrap();
    let out = rt.dispatch_line(&line, &mut body).unwrap();
    let resp: Waist2Response<Value> = serde_json::from_str(&out).unwrap();
    assert_eq!(
        resp.error.unwrap().code,
        Waist2ErrorCode::UnknownCapability.code()
    );
}
