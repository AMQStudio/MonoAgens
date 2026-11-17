use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use std::thread;

use monoagens_lab::datasets::{
    CONVERTER_CACHE_FILE, ChecksumsRecorded, ConvertedCacheMetadata, FetchEndpointOverrides,
    LicenseStatus, ManifestFile, PinEndpointOverrides, PinnedSha256, PinnedSize, SourceKind,
    fetch_datasets, load_and_validate_manifest, pin_datasets, save_manifest_to_path,
    verify_datasets,
};
use monoagens_lab::{sha256_hex, standard_benchmark_registry};
use tempfile::TempDir;

fn lab_bin() -> &'static str {
    env!("CARGO_BIN_EXE_monoagens-lab")
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root")
        .to_path_buf()
}

struct MockRoute {
    status: u16,
    content_type: &'static str,
    body: Vec<u8>,
}

struct MockHttpServer {
    base_url: String,
    hits: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl MockHttpServer {
    fn start(routes: BTreeMap<String, MockRoute>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        listener.set_nonblocking(true).expect("set nonblocking");
        let addr = listener.local_addr().expect("local addr");
        let base_url = format!("http://{addr}");
        let hits = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let hits_c = Arc::clone(&hits);
        let stop_c = Arc::clone(&stop);
        let routes = Arc::new(routes);

        let handle = thread::spawn(move || {
            while !stop_c.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let _ = stream.set_nonblocking(false);
                        let mut buf = [0u8; 8192];
                        let n = stream.read(&mut buf).unwrap_or(0);
                        if n == 0 {
                            continue;
                        }
                        hits_c.fetch_add(1, Ordering::SeqCst);
                        let req = String::from_utf8_lossy(&buf[..n]);
                        let path = req
                            .lines()
                            .next()
                            .and_then(|l| l.split_whitespace().nth(1))
                            .unwrap_or("/");
                        let (status, ctype, body) = match routes.get(path) {
                            Some(r) => (r.status, r.content_type, r.body.clone()),
                            None => (404, "text/plain", format!("not found: {path}").into_bytes()),
                        };
                        let status_text = match status {
                            200 => "OK",
                            404 => "Not Found",
                            500 => "Internal Server Error",
                            _ => "Status",
                        };
                        let header = format!(
                            "HTTP/1.1 {status} {status_text}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        );
                        let _ = stream.write_all(header.as_bytes());
                        let _ = stream.write_all(&body);
                        let _ = stream.flush();
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(std::time::Duration::from_millis(5));
                    }
                    Err(_) => break,
                }
            }
        });

        Self {
            base_url,
            hits,
            stop,
            handle: Some(handle),
        }
    }
}

impl Drop for MockHttpServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = std::net::TcpStream::connect(self.base_url.trim_start_matches("http://"));
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

#[test]
fn datasets_pin_resolves_github_and_hf_shas_sizes_without_clearing_license() {
    let tmp = TempDir::new().expect("tempdir");
    let manifest_path = tmp.path().join("datasets.manifest.toml");
    let registry = standard_benchmark_registry();
    let base = load_and_validate_manifest(
        &workspace_root().join("lab/datasets.manifest.example.toml"),
        &registry,
    )
    .unwrap();
    save_manifest_to_path(&base, &manifest_path).unwrap();

    let gh_sha = "1111111111111111111111111111111111111111";
    let hf_sha = "2222222222222222222222222222222222222222";
    let hf_lfs_sha = sha256_hex(b"synthetic-longmemeval-bytes");

    let mut routes = BTreeMap::new();
    routes.insert(
        "/repos/snap-research/locomo/commits/HEAD".to_owned(),
        MockRoute {
            status: 200,
            content_type: "application/json",
            body: format!(r#"{{"sha":"{gh_sha}"}}"#).into_bytes(),
        },
    );
    routes.insert(
        format!("/repos/snap-research/locomo/git/trees/{gh_sha}?recursive=1"),
        MockRoute {
            status: 200,
            content_type: "application/json",
            body: r#"{"tree":[{"path":"data/locomo10.json","type":"blob","size":12345}]}"#
                .as_bytes()
                .to_vec(),
        },
    );
    routes.insert(
        "/api/datasets/xiaowu0162/longmemeval-cleaned".to_owned(),
        MockRoute {
            status: 200,
            content_type: "application/json",
            body: format!(r#"{{"sha":"{hf_sha}"}}"#).into_bytes(),
        },
    );
    routes.insert(
        format!("/api/datasets/xiaowu0162/longmemeval-cleaned/tree/{hf_sha}?recursive=1"),
        MockRoute {
            status: 200,
            content_type: "application/json",
            body: format!(
                r#"[{{"path":"longmemeval_s_cleaned.json","type":"file","size":999,"lfs":{{"oid":"{hf_lfs_sha}","size":27}} }}]"#
            )
            .into_bytes(),
        },
    );
    routes.insert(
        "/repos/xiaowu0162/LongMemEval/commits/HEAD".to_owned(),
        MockRoute {
            status: 200,
            content_type: "application/json",
            body: format!(r#"{{"sha":"{gh_sha}"}}"#).into_bytes(),
        },
    );
    routes.insert(
        format!("/repos/xiaowu0162/LongMemEval/git/trees/{gh_sha}?recursive=1"),
        MockRoute {
            status: 200,
            content_type: "application/json",
            body: r#"{"tree":[]}"#.as_bytes().to_vec(),
        },
    );

    let srv = MockHttpServer::start(routes);
    let overrides = PinEndpointOverrides {
        github_api_base: srv.base_url.clone(),
        hf_api_base: srv.base_url.clone(),
        bitbucket_api_base: srv.base_url.clone(),
        timeout_secs: 5,
    };

    let res = pin_datasets(
        &manifest_path,
        &registry,
        &["locomo".to_owned(), "longmemeval".to_owned()],
        &overrides,
    )
    .expect("pin must succeed against loopback mock");
    assert_eq!(res.len(), 2);

    let updated = load_and_validate_manifest(&manifest_path, &registry).unwrap();
    let locomo = updated.get("locomo").unwrap();
    assert_eq!(locomo.sources[0].revision, gh_sha);
    assert_eq!(locomo.files[0].size, PinnedSize::Bytes(12345));
    assert!(locomo.files[0].sha256.is_pending());
    assert_eq!(locomo.license.status, LicenseStatus::Pending);

    let lme = updated.get("longmemeval").unwrap();
    assert_eq!(lme.sources[0].revision, hf_sha);
    assert_eq!(lme.files[0].size, PinnedSize::Bytes(27));
    assert_eq!(lme.files[0].sha256, PinnedSha256::Sha256(hf_lfs_sha));
    assert_eq!(lme.checksums_recorded, ChecksumsRecorded::Source);
    assert_eq!(lme.license.status, LicenseStatus::Pending);

    // Ensure no dataset directory was created on disk by pin.
    assert!(!tmp.path().join("datasets").exists());
}

#[test]
fn datasets_fetch_license_gate_atomic_multisource_record_checksums_and_verify() {
    let tmp = TempDir::new().expect("tempdir");
    let manifest_path = tmp.path().join("datasets.manifest.toml");
    let datasets_dir = tmp.path().join("datasets");
    let registry = standard_benchmark_registry();
    let mut manifest = load_and_validate_manifest(
        &workspace_root().join("lab/datasets.manifest.example.toml"),
        &registry,
    )
    .unwrap();

    // Synthetic payloads for loopback HTTP fixture server.
    let locomo_bytes = br#"[{"sample_id":"s1","conversation":{},"qa":[]}]"#.to_vec();
    let gmb_conv_bytes = br#"{"chan_1":[]}"#.to_vec();
    let gmb_q_bytes = br#"{"qid":"q1","question":"hi?"}"#.to_vec();
    let url_bytes = br#"{"seed":42}"#.to_vec();

    let gh_rev = "aaaa1111aaaa1111aaaa1111aaaa1111aaaa1111";
    let hf_rev = "bbbb2222bbbb2222bbbb2222bbbb2222bbbb2222";

    let mut routes = BTreeMap::new();
    routes.insert(
        format!("/snap-research/locomo/{gh_rev}/data/locomo10.json"),
        MockRoute {
            status: 200,
            content_type: "application/json",
            body: locomo_bytes.clone(),
        },
    );
    routes.insert(
        format!(
            "/datasets/kimperyang/GroupMemBench/resolve/{hf_rev}/data/final/Finance/synthetic_domain_channels_rolevariants_Finance.json"
        ),
        MockRoute {
            status: 200,
            content_type: "application/json",
            body: gmb_conv_bytes.clone(),
        },
    );
    routes.insert(
        format!("/UCSB-NLP-Chang/GroupMemBench/{gh_rev}/questions/Finance/multi_hop.jsonl"),
        MockRoute {
            status: 200,
            content_type: "application/json",
            body: gmb_q_bytes.clone(),
        },
    );
    routes.insert(
        "/seed-artifacts/tomi_seed.json".to_owned(),
        MockRoute {
            status: 200,
            content_type: "application/json",
            body: url_bytes.clone(),
        },
    );
    let srv = MockHttpServer::start(routes);

    // Pin revisions in manifest, keep licenses pending first.
    {
        let loc = manifest.get_mut("locomo").unwrap();
        loc.sources[0].revision = gh_rev.to_owned();
    }
    {
        let gmb = manifest.get_mut("group_mem_bench").unwrap();
        gmb.sources[0].revision = hf_rev.to_owned();
        gmb.sources[1].revision = gh_rev.to_owned();
        gmb.files = vec![
            ManifestFile {
                path: "data/final/Finance/synthetic_domain_channels_rolevariants_Finance.json"
                    .to_owned(),
                source_role: Some("conversations".to_owned()),
                sha256: PinnedSha256::Sha256(sha256_hex(&gmb_conv_bytes)),
                size: PinnedSize::Bytes(gmb_conv_bytes.len() as u64),
                download_url: None,
                extract: None,
            },
            ManifestFile {
                path: "questions/Finance/multi_hop.jsonl".to_owned(),
                source_role: Some("questions_and_eval".to_owned()),
                sha256: PinnedSha256::Sha256(sha256_hex(&gmb_q_bytes)),
                size: PinnedSize::Bytes(gmb_q_bytes.len() as u64),
                download_url: None,
                extract: None,
            },
        ];
        gmb.checksums_recorded = ChecksumsRecorded::Source;
    }
    {
        let tomi = manifest.get_mut("tomi").unwrap();
        tomi.sources[0].kind = SourceKind::Url;
        tomi.sources[0].location = format!("{}/seed-artifacts", srv.base_url);
        tomi.sources[0].revision = "seed-v1".to_owned();
        tomi.files[0].path = "tomi_seed.json".to_owned();
        tomi.files[0].sha256 = PinnedSha256::Sha256(sha256_hex(&url_bytes));
        tomi.files[0].size = PinnedSize::Bytes(url_bytes.len() as u64);
        tomi.checksums_recorded = ChecksumsRecorded::Source;
    }
    save_manifest_to_path(&manifest, &manifest_path).unwrap();

    let overrides = FetchEndpointOverrides {
        github_raw_base: srv.base_url.clone(),
        hf_raw_base: srv.base_url.clone(),
        fetch_date: Some("2026-11-17".to_owned()),
        timeout_secs: 5,
    };

    // 1. Pending license must refuse with zero HTTP requests.
    let err_pending = fetch_datasets(
        &manifest_path,
        &datasets_dir,
        &registry,
        &["locomo".to_owned()],
        true,
        &overrides,
    )
    .expect_err("pending license must refuse fetch");
    assert!(err_pending.to_string().contains("locomo"));
    assert!(err_pending.to_string().contains("pending"));
    assert_eq!(srv.hits.load(Ordering::SeqCst), 0);

    // 2. Refused license must also refuse.
    manifest.get_mut("locomo").unwrap().license.status = LicenseStatus::Refused;
    save_manifest_to_path(&manifest, &manifest_path).unwrap();
    let err_refused = fetch_datasets(
        &manifest_path,
        &datasets_dir,
        &registry,
        &["locomo".to_owned()],
        true,
        &overrides,
    )
    .expect_err("refused license must refuse fetch");
    assert!(err_refused.to_string().contains("refused"));
    assert_eq!(srv.hits.load(Ordering::SeqCst), 0);

    // 3. Cleared license + --record-checksums records TOFU hashes & date, then second call refuses.
    manifest.get_mut("locomo").unwrap().license.status = LicenseStatus::Cleared;
    manifest.get_mut("group_mem_bench").unwrap().license.status = LicenseStatus::Cleared;
    manifest.get_mut("tomi").unwrap().license.status = LicenseStatus::Cleared;
    save_manifest_to_path(&manifest, &manifest_path).unwrap();

    let fetched_tofu = fetch_datasets(
        &manifest_path,
        &datasets_dir,
        &registry,
        &["locomo".to_owned()],
        true,
        &overrides,
    )
    .expect("TOFU fetch must succeed");
    assert_eq!(fetched_tofu[0].files_fetched, 1);
    assert!(fetched_tofu[0].recorded_owner_checksums);
    assert!(!datasets_dir.join("locomo/.partial").exists());

    let updated = load_and_validate_manifest(&manifest_path, &registry).unwrap();
    let loc_after = updated.get("locomo").unwrap();
    assert_eq!(loc_after.checksums_recorded, ChecksumsRecorded::Owner);
    assert_eq!(
        loc_after.checksums_recorded_date.as_deref(),
        Some("2026-11-17")
    );
    assert_eq!(
        loc_after.files[0].sha256,
        PinnedSha256::Sha256(sha256_hex(&locomo_bytes))
    );

    let err_rerecord = fetch_datasets(
        &manifest_path,
        &datasets_dir,
        &registry,
        &["locomo".to_owned()],
        true,
        &overrides,
    )
    .expect_err("re-recording checksums must refuse");
    assert!(err_rerecord.to_string().contains("datasets verify"));

    // 4. Multi-source group_mem_bench and URL seed fetch succeed atomically.
    let multi = fetch_datasets(
        &manifest_path,
        &datasets_dir,
        &registry,
        &["group_mem_bench".to_owned(), "tomi".to_owned()],
        false,
        &overrides,
    )
    .expect("multi-source and url fetch must succeed");
    assert_eq!(multi[0].files_fetched, 2);
    assert_eq!(multi[1].files_fetched, 1);

    // 5. Verify passes cleanly on fetched datasets.
    let v_ok = verify_datasets(
        &manifest_path,
        &datasets_dir,
        &registry,
        &[
            "locomo".to_owned(),
            "group_mem_bench".to_owned(),
            "tomi".to_owned(),
        ],
    )
    .expect("verify call");
    assert!(v_ok.all_verified, "expected all verified: {:?}", v_ok);

    // 6. One-byte alteration fails verify non-zero with both hashes reported.
    let loc_file = datasets_dir.join("locomo/data/locomo10.json");
    let mut tampered = fs::read(&loc_file).unwrap();
    let expected_hash = sha256_hex(&tampered);
    tampered[0] ^= 0xFF;
    let actual_hash = sha256_hex(&tampered);
    fs::write(&loc_file, &tampered).unwrap();

    let out_tampered = Command::new(lab_bin())
        .args([
            "datasets",
            "verify",
            "locomo",
            "--manifest",
            manifest_path.to_str().unwrap(),
            "--datasets-dir",
            datasets_dir.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(!out_tampered.status.success());
    let stderr = String::from_utf8_lossy(&out_tampered.stderr);
    assert!(stderr.contains("data/locomo10.json"), "stderr={stderr}");
    assert!(stderr.contains(&expected_hash), "stderr={stderr}");
    assert!(stderr.contains(&actual_hash), "stderr={stderr}");

    // Restore file, then test missing file failure.
    fs::write(&loc_file, &locomo_bytes).unwrap();
    fs::remove_file(datasets_dir.join("group_mem_bench/questions/Finance/multi_hop.jsonl"))
        .unwrap();
    let v_missing = verify_datasets(
        &manifest_path,
        &datasets_dir,
        &registry,
        &["group_mem_bench".to_owned()],
    )
    .unwrap();
    assert!(!v_missing.all_verified);
    assert!(
        v_missing.results[0].failures[0].contains("missing dataset file"),
        "{:?}",
        v_missing.results[0].failures
    );

    // 7. Stale converter cache version fails verify.
    let stale_cache = ConvertedCacheMetadata {
        dataset_id: "locomo".to_owned(),
        converter_version: "0.0.1-stale".to_owned(),
        revisions: BTreeMap::from([("primary".to_owned(), gh_rev.to_owned())]),
    };
    fs::write(
        datasets_dir.join("locomo").join(CONVERTER_CACHE_FILE),
        serde_json::to_vec(&stale_cache).unwrap(),
    )
    .unwrap();
    let v_stale = verify_datasets(
        &manifest_path,
        &datasets_dir,
        &registry,
        &["locomo".to_owned()],
    )
    .unwrap();
    assert!(!v_stale.all_verified);
    assert!(
        v_stale.results[0].failures[0].contains("stale converted cache"),
        "{:?}",
        v_stale.results[0].failures
    );

    // 8. Interrupted / failed download removes `.partial/` and leaves no partial file in final path.
    let mut bad_manifest = load_and_validate_manifest(&manifest_path, &registry).unwrap();
    let pitch = bad_manifest.get_mut("pitch").unwrap();
    pitch.license.status = LicenseStatus::Cleared;
    pitch.sources[0].revision = gh_rev.to_owned();
    pitch.files[0].sha256 = PinnedSha256::Sha256(sha256_hex(b"expected-pitch"));
    pitch.files[0].size = PinnedSize::Bytes(14);
    save_manifest_to_path(&bad_manifest, &manifest_path).unwrap();

    let err_404 = fetch_datasets(
        &manifest_path,
        &datasets_dir,
        &registry,
        &["pitch".to_owned()],
        false,
        &overrides,
    )
    .expect_err("404 download must fail");
    assert!(err_404.to_string().contains("404"));
    assert!(!datasets_dir.join("pitch").exists());
}

#[test]
fn non_network_commands_never_open_connections_under_deny_all_proxy() {
    let tmp = TempDir::new().expect("tempdir");
    let manifest_path = tmp.path().join("datasets.manifest.toml");
    let datasets_dir = tmp.path().join("datasets");
    fs::create_dir_all(&datasets_dir).unwrap();

    let registry = standard_benchmark_registry();
    let mut manifest = load_and_validate_manifest(
        &workspace_root().join("lab/datasets.manifest.example.toml"),
        &registry,
    )
    .unwrap();

    // Point sources to a live loopback server that fails the test if ANY connection arrives.
    let srv = MockHttpServer::start(BTreeMap::new());
    for d in &mut manifest.datasets {
        for s in &mut d.sources {
            if s.kind == SourceKind::Url {
                s.location = srv.base_url.clone();
            }
        }
    }
    save_manifest_to_path(&manifest, &manifest_path).unwrap();

    // 1. benchmark --suite in_tree_only with deny-all proxy env
    let out_bench = Command::new(lab_bin())
        .args([
            "benchmark",
            "--suite",
            "in_tree_only",
            "--datasets-dir",
            datasets_dir.to_str().unwrap(),
            "--json",
        ])
        .env("HTTP_PROXY", "http://127.0.0.1:1")
        .env("HTTPS_PROXY", "http://127.0.0.1:1")
        .env("ALL_PROXY", "http://127.0.0.1:1")
        .env("MONOAGENS_GITHUB_API_BASE", &srv.base_url)
        .env("MONOAGENS_HF_API_BASE", &srv.base_url)
        .output()
        .unwrap();
    assert!(
        out_bench.status.success(),
        "benchmark in_tree_only failed: {}",
        String::from_utf8_lossy(&out_bench.stderr)
    );

    // 2. datasets list with deny-all proxy env
    let out_list = Command::new(lab_bin())
        .args([
            "datasets",
            "list",
            "--manifest",
            manifest_path.to_str().unwrap(),
            "--datasets-dir",
            datasets_dir.to_str().unwrap(),
            "--json",
        ])
        .env("HTTP_PROXY", "http://127.0.0.1:1")
        .env("HTTPS_PROXY", "http://127.0.0.1:1")
        .env("ALL_PROXY", "http://127.0.0.1:1")
        .env("MONOAGENS_GITHUB_API_BASE", &srv.base_url)
        .env("MONOAGENS_HF_API_BASE", &srv.base_url)
        .output()
        .unwrap();
    assert!(
        out_list.status.success(),
        "datasets list failed: {}",
        String::from_utf8_lossy(&out_list.stderr)
    );

    // 3. datasets verify (even when unverified/missing) makes zero network calls
    let out_verify = Command::new(lab_bin())
        .args([
            "datasets",
            "verify",
            "locomo",
            "--manifest",
            manifest_path.to_str().unwrap(),
            "--datasets-dir",
            datasets_dir.to_str().unwrap(),
        ])
        .env("HTTP_PROXY", "http://127.0.0.1:1")
        .env("HTTPS_PROXY", "http://127.0.0.1:1")
        .env("ALL_PROXY", "http://127.0.0.1:1")
        .env("MONOAGENS_GITHUB_API_BASE", &srv.base_url)
        .env("MONOAGENS_HF_API_BASE", &srv.base_url)
        .output()
        .unwrap();
    // Exits 1 (unverified), NOT crash, and zero network connections opened.
    assert_eq!(out_verify.status.code(), Some(1));
    assert_eq!(
        srv.hits.load(Ordering::SeqCst),
        0,
        "non-pin/non-fetch commands must never open network connections"
    );
}

#[test]
fn scorer_model_entries_pin_fetch_and_verify_via_hf_models_api() {
    let tmp = TempDir::new().expect("tempdir");
    let manifest_path = tmp.path().join("datasets.manifest.toml");
    let datasets_dir = tmp.path().join("datasets");
    let registry = standard_benchmark_registry();
    let base = load_and_validate_manifest(
        &workspace_root().join("lab/datasets.manifest.example.toml"),
        &registry,
    )
    .unwrap();
    save_manifest_to_path(&base, &manifest_path).unwrap();

    let model_sha = "3333333333333333333333333333333333333333";
    let config_bytes = br#"{"architectures":["RobertaModel"]}"#.to_vec();
    let weights_bytes = b"synthetic-safetensors-payload".to_vec();
    let weights_sha = sha256_hex(&weights_bytes);

    let mut routes = BTreeMap::new();
    routes.insert(
        "/api/models/sentence-transformers/all-roberta-large-v1".to_owned(),
        MockRoute {
            status: 200,
            content_type: "application/json",
            body: format!(r#"{{"sha":"{model_sha}"}}"#).into_bytes(),
        },
    );
    routes.insert(
        format!(
            "/api/models/sentence-transformers/all-roberta-large-v1/tree/{model_sha}?recursive=1"
        ),
        MockRoute {
            status: 200,
            content_type: "application/json",
            body: format!(
                r#"[{{"path":"config.json","type":"file","size":{}}},{{"path":"model.safetensors","type":"file","size":{},"lfs":{{"oid":"{weights_sha}","size":{}}}}}]"#,
                config_bytes.len(),
                weights_bytes.len(),
                weights_bytes.len()
            )
            .into_bytes(),
        },
    );
    routes.insert(
        format!("/sentence-transformers/all-roberta-large-v1/resolve/{model_sha}/config.json"),
        MockRoute {
            status: 200,
            content_type: "application/json",
            body: config_bytes.clone(),
        },
    );
    routes.insert(
        format!(
            "/sentence-transformers/all-roberta-large-v1/resolve/{model_sha}/model.safetensors"
        ),
        MockRoute {
            status: 200,
            content_type: "application/octet-stream",
            body: weights_bytes,
        },
    );
    let srv = MockHttpServer::start(routes);

    let pin_res = pin_datasets(
        &manifest_path,
        &registry,
        &["fantom_scorer_all_roberta_large_v1".to_owned()],
        &PinEndpointOverrides {
            github_api_base: srv.base_url.clone(),
            hf_api_base: srv.base_url.clone(),
            bitbucket_api_base: srv.base_url.clone(),
            timeout_secs: 5,
        },
    )
    .expect("pin scorer_model via HF /api/models/");
    assert_eq!(pin_res[0].updated_sources, 1);
    assert_eq!(pin_res[0].updated_files, 2);

    let mut pinned = load_and_validate_manifest(&manifest_path, &registry).unwrap();
    let sm = pinned
        .get_mut("fantom_scorer_all_roberta_large_v1")
        .unwrap();
    assert_eq!(sm.sources[0].revision, model_sha);
    assert_eq!(sm.files.len(), 2);
    assert_eq!(sm.license.status, LicenseStatus::Pending);

    // Pending license refuses fetch.
    let err_pending = fetch_datasets(
        &manifest_path,
        &datasets_dir,
        &registry,
        &["fantom_scorer_all_roberta_large_v1".to_owned()],
        true,
        &FetchEndpointOverrides {
            github_raw_base: srv.base_url.clone(),
            hf_raw_base: srv.base_url.clone(),
            fetch_date: Some("2026-11-17".to_owned()),
            timeout_secs: 5,
        },
    )
    .expect_err("pending scorer_model license must refuse fetch");
    assert!(err_pending.to_string().contains("pending"));

    // Clear license and fetch with --record-checksums (verifies LFS sha256 + records config.json sha256).
    sm.license.status = LicenseStatus::Cleared;
    save_manifest_to_path(&pinned, &manifest_path).unwrap();

    let fetched = fetch_datasets(
        &manifest_path,
        &datasets_dir,
        &registry,
        &["fantom_scorer_all_roberta_large_v1".to_owned()],
        true,
        &FetchEndpointOverrides {
            github_raw_base: srv.base_url.clone(),
            hf_raw_base: srv.base_url.clone(),
            fetch_date: Some("2026-11-17".to_owned()),
            timeout_secs: 5,
        },
    )
    .expect("fetch scorer_model");
    assert_eq!(fetched[0].files_fetched, 2);

    let v = verify_datasets(
        &manifest_path,
        &datasets_dir,
        &registry,
        &["fantom_scorer_all_roberta_large_v1".to_owned()],
    )
    .unwrap();
    assert!(v.all_verified, "expected verified scorer_model: {v:?}");
}
