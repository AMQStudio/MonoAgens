//! `model-provider-catalog` groups 3 and 5: live provider rebuild through `config.apply`,
//! the discovery / key / usage admin methods, per-purpose routing and usage counters
//! (fake local servers only; no network, no paid calls).

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use monoagens_core::{
    ContextPlan, CoreConfig, MemStore, ModelPurpose, NoopDoors,
};
use monoagens_gateway::{
    AdminClient, AdminServer, CueRecorder, DailyBudgetTracker, DiagnosticSink, GatewayConfig,
    GatewayModelExecutor, GatewayOptions, GatewayOwner, GatewayRuntime, GatewaySeams, OwnerMode,
    OwnerParams, PluginEnvContext, PluginSupervisor, RestartPolicy, RunLog, SystemClock,
    acquire_data_dir_lock, build_model_routes, parse_config_str,
};
use monoagens_protocol::{ActorId, Audience, OpaquePayload, Percept, PlaceId, Time};
use monoagens_reason::{ProviderSecrets, SecretString};
use serde_json::{Value, json};
use tempfile::TempDir;

type Seen = Arc<Mutex<Vec<(String, Vec<(String, String)>, String)>>>;

const CHAT_REPLY: &str = r#"{"model":"m","choices":[{"message":{"content":"{\"acts\":[]}"},"finish_reason":"stop"}],"usage":{"prompt_tokens":100,"completion_tokens":50,"total_tokens":150}}"#;
const MODELS_REPLY: &str = r#"{"data":[{"id":"alpha","context_length":8000},{"id":"beta"}]}"#;

fn test_dir() -> TempDir {
    tempfile::Builder::new()
        .prefix("cat-")
        .tempdir_in(env!("CARGO_TARGET_TMPDIR"))
        .expect("tempdir under target")
}

/// Fake provider server answering any number of requests: `GET .../models` with `models`,
/// `POST .../chat/completions` with `chat` (`status`, body).
fn serve(models: &'static str, chat: (u16, &'static str)) -> (String, Seen) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    let seen: Seen = Arc::default();
    let seen_bg = Arc::clone(&seen);
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { return };
            let seen = Arc::clone(&seen_bg);
            thread::spawn(move || {
                let mut reader = BufReader::new(stream.try_clone().expect("clone"));
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 {
                    return;
                }
                let mut headers = Vec::new();
                let mut len = 0usize;
                loop {
                    let mut h = String::new();
                    reader.read_line(&mut h).expect("hdr");
                    let h = h.trim_end().to_owned();
                    if h.is_empty() {
                        break;
                    }
                    if let Some((k, v)) = h.split_once(':') {
                        let (k, v) = (k.trim().to_ascii_lowercase(), v.trim().to_owned());
                        if k == "content-length" {
                            len = v.parse().unwrap_or(0);
                        }
                        headers.push((k, v));
                    }
                }
                let mut body = vec![0u8; len];
                reader.read_exact(&mut body).expect("body");
                let line = line.trim().to_owned();
                let (status, reply) = if line.starts_with("GET") && line.contains("/models") {
                    (200, models)
                } else {
                    chat
                };
                seen.lock().expect("lock").push((
                    line,
                    headers,
                    String::from_utf8_lossy(&body).into_owned(),
                ));
                let resp = format!(
                    "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}",
                    reply.len()
                );
                let _ = stream.write_all(resp.as_bytes());
            });
        }
    });
    (format!("http://{addr}"), seen)
}

fn config_text(base: &str, model: &str, extra: &str) -> String {
    format!(
        "version = 1\n[model]\nprovider = \"ollama\"\nmodel = \"{model}\"\n[model.provider_options]\nbase_url = \"{base}/v1\"\n{extra}"
    )
}

fn parse(text: &str) -> GatewayConfig {
    parse_config_str(text, Path::new("monoagens.toml"), Path::new(".")).expect("config")
}

fn percept(text: &str) -> Percept {
    let mut p = Percept::minimal(
        ActorId::new("per_alice"),
        PlaceId::new("room:main"),
        Audience::new([ActorId::new("per_alice"), ActorId::new("soul")]),
        Time::from_millis(1_000_000),
        OpaquePayload::from_bytes(text.as_bytes()),
    );
    p.addressees = vec![ActorId::new("soul")];
    p
}

async fn wait_until<F: FnMut() -> bool>(what: &str, mut cond: F) {
    for _ in 0..600 {
        if cond() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("timed out waiting for {what}");
}

struct Env {
    rt: GatewayRuntime,
    owner: Arc<GatewayOwner>,
    log: RunLog,
    dir: TempDir,
}

async fn boot(text: &str) -> Env {
    let dir = test_dir();
    let config_path = dir.path().join("monoagens.toml");
    std::fs::write(&config_path, text).unwrap();
    let cfg = parse(text);
    let secrets = ProviderSecrets::default();
    let rt = GatewayRuntime::start(
        cfg.clone(),
        &secrets,
        || Ok(MemStore::new()),
        CoreConfig::default(),
        NoopDoors,
        GatewayOptions {
            clock: Arc::new(SystemClock::new()),
            provider: None,
            diagnostics: DiagnosticSink::new(&secrets),
            cues: CueRecorder::new(),
            seams: GatewaySeams::default(),
        },
    )
    .await
    .expect("start gateway");
    let handle = rt.handle();
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
            data_dir: Some(dir.path().to_path_buf()),
            rbac_enabled: false,
            plugin_env: Default::default(),
        },
        Arc::new(SystemClock::new()),
        log.clone(),
        RestartPolicy::default(),
        vec![],
    );
    let owner = Arc::new(GatewayOwner::new(OwnerParams {
        handle,
        supervisor: sup,
        runlog: log.clone(),
        mode: OwnerMode::Serve,
        config: cfg,
        config_path,
        secrets_path: dir.path().join("secrets.toml"),
        data_dir: dir.path().to_path_buf(),
        owner_actor: ActorId::new("per_owner"),
    }));
    Env { rt, owner, log, dir }
}

fn models_seen(seen: &Seen) -> Vec<String> {
    seen.lock()
        .unwrap()
        .iter()
        .filter(|(l, _, _)| l.starts_with("POST"))
        .filter_map(|(_, _, b)| serde_json::from_str::<Value>(b).ok())
        .filter_map(|v| v["model"].as_str().map(str::to_owned))
        .collect()
}

// ---------------------------------------------------------------- 3.1

#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
async fn config_apply_switches_the_model_for_the_next_request() {
    let (base, seen) = serve(MODELS_REPLY, (200, CHAT_REPLY));
    let env = boot(&config_text(&base, "model-one", "")).await;
    let handle = env.owner.handle().clone();

    handle
        .submit_and_wait(monoagens_core::Input::Percept(Box::new(percept("first"))))
        .await
        .expect("first turn");
    wait_until("first model request", || !models_seen(&seen).is_empty()).await;
    assert!(models_seen(&seen).iter().all(|m| m == "model-one"), "{:?}", models_seen(&seen));
    wait_until("first call settled", || handle.in_flight_model_calls() == 0).await;
    let before = models_seen(&seen).len();

    let report = env
        .owner
        .config_apply(&json!({ "model": { "model": "model-two" } }))
        .await
        .expect("apply");
    assert!(report.provider);
    assert!(env.log.snapshot().iter().any(|e| e.source == "provider" && e.message.contains("model-two")));

    handle
        .submit_and_wait(monoagens_core::Input::Percept(Box::new(percept("second"))))
        .await
        .expect("second turn");
    wait_until("second model request", || models_seen(&seen).len() > before).await;
    let after: Vec<String> = models_seen(&seen)[before..].to_vec();
    assert!(after.iter().all(|m| m == "model-two"), "{after:?}");
    env.rt.shutdown().await;
}

// ---------------------------------------------------------------- 3.2

#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
async fn models_discover_serves_cache_first_then_refreshes_through_the_socket() {
    let (base, seen) = serve(MODELS_REPLY, (200, CHAT_REPLY));
    let env = boot(&config_text(&base, "alpha", "")).await;
    let lock = acquire_data_dir_lock(env.dir.path()).expect("lock");
    let server = AdminServer::start(&lock, env.dir.path(), Arc::clone(&env.owner))
        .await
        .expect("serve");
    let mut client = AdminClient::connect(env.dir.path()).await.expect("connect");

    // No cache yet: instant, empty, no request to the provider.
    let r = client
        .call("models.discover", json!({ "provider": "ollama" }))
        .await
        .expect("cache read");
    assert_eq!(r["models"], json!([]));
    assert_eq!(r["cached"], json!(true));
    assert!(seen.lock().unwrap().is_empty());

    // Refresh asks the provider and writes the cache.
    let r = client
        .call("models.discover", json!({ "provider": "ollama", "refresh": true }))
        .await
        .expect("refresh");
    assert_eq!(r["models"][0]["id"], json!("alpha"));
    assert_eq!(r["models"][0]["context_length"], json!(8000));
    assert_eq!(r["models"][1]["id"], json!("beta"));
    assert_eq!(r["stale"], json!(false));
    assert!(r["error"].is_null());

    // Now the cache answers instantly with the same list.
    let r = client
        .call("models.discover", json!({ "provider": "ollama" }))
        .await
        .expect("cache read");
    assert_eq!(r["models"].as_array().unwrap().len(), 2);
    assert!(r["fetched_at_ms"].as_u64().unwrap() > 0);

    // Unknown provider is an invalid-params error.
    let err = client
        .call("models.discover", json!({ "provider": "nope" }))
        .await
        .unwrap_err();
    assert_eq!(err.code, -32602);

    server.shutdown();
    env.rt.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
async fn models_discover_failure_returns_stale_cache_and_an_error() {
    let (base, _seen) = serve(MODELS_REPLY, (200, CHAT_REPLY));
    let env = boot(&config_text(&base, "alpha", "")).await;
    env.owner
        .models_discover("ollama", true)
        .await
        .expect("prime cache");
    // Point the provider at a closed port: refresh fails, the cached list stays visible.
    let dead = {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap()
    };
    env.owner
        .config_apply(&json!({ "model": { "provider_options": { "base_url": format!("http://{dead}/v1") } } }))
        .await
        .expect("apply");
    let r = env.owner.models_discover("ollama", true).await.expect("discover");
    assert_eq!(r["stale"], json!(true));
    assert_eq!(r["models"].as_array().unwrap().len(), 2);
    assert!(r["error"].as_str().is_some_and(|e| !e.is_empty()));
    assert!(env.log.snapshot().iter().any(|e| e.source == "discovery"));
    env.rt.shutdown().await;
}

// ---------------------------------------------------------------- key admin methods

#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
async fn secrets_set_writes_0600_keeps_other_content_and_never_logs_the_key() {
    let (base, seen) = serve(MODELS_REPLY, (200, CHAT_REPLY));
    let env = boot(&config_text(&base, "alpha", "")).await;
    let secrets_path = env.dir.path().join("secrets.toml");
    std::fs::write(&secrets_path, "model_api_key = \"legacy\"\n").unwrap();

    // Test the key against the server (does not save).
    let t = env
        .owner
        .providers_test("ollama", Some("sk-test-123456"))
        .await
        .expect("test");
    assert_eq!(t["ok"], json!(true));
    assert_eq!(t["model_count"], json!(2));
    assert!(t["latency_ms"].is_u64());
    assert!(!std::fs::read_to_string(&secrets_path).unwrap().contains("sk-test"));
    {
        let seen = seen.lock().unwrap();
        let auth = seen.last().unwrap().1.iter().find(|(k, _)| k == "authorization");
        assert_eq!(auth.map(|(_, v)| v.as_str()), Some("Bearer sk-test-123456"));
    }

    let r = env
        .owner
        .secrets_set("ollama", "sk-test-123456")
        .await
        .expect("set");
    assert_eq!(r["ok"], json!(true));
    assert!(r.to_string().find("sk-test").is_none(), "key must not be echoed");
    let mode = std::fs::metadata(&secrets_path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
    let raw = std::fs::read_to_string(&secrets_path).unwrap();
    assert!(raw.contains("model_api_key = \"legacy\""), "{raw}");
    assert!(raw.contains("[providers.ollama]"), "{raw}");

    // The new key is visible to providers.list (never its value) and survives a reload.
    let list = env.owner.providers_list().expect("list");
    let row = list["providers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == "ollama")
        .unwrap()
        .clone();
    assert_eq!(row["key_set"], json!(true));
    assert_eq!(row["key_source"], json!("secrets"));
    assert!(!list.to_string().contains("sk-test"));
    assert!(
        !env.log.snapshot().iter().any(|e| e.message.contains("sk-test-123456")),
        "key leaked into the run log"
    );

    // Bad keys are rejected.
    assert_eq!(
        env.owner.secrets_set("ollama", "a\nb").await.unwrap_err().code,
        -32602
    );
    assert_eq!(env.owner.secrets_set("nope", "k").await.unwrap_err().code, -32602);
    env.rt.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
async fn providers_test_reports_http_failure_without_saving() {
    let (base, _seen) = serve("{\"error\":\"nope\"}", (200, CHAT_REPLY));
    let env = boot(&config_text(&base, "alpha", "")).await;
    let t = env.owner.providers_test("ollama", Some("k")).await.expect("test");
    assert_eq!(t["ok"], json!(false));
    assert!(t["error"].as_str().is_some());
    assert!(!env.dir.path().join("secrets.toml").exists());
    env.rt.shutdown().await;
}

// ---------------------------------------------------------------- 5.1 purposes

#[test]
fn purpose_config_routes_classify_to_its_model_and_reason_to_the_main_model() {
    let (base, seen) = serve(MODELS_REPLY, (200, CHAT_REPLY));
    let cfg = parse(&config_text(
        &base,
        "big",
        "[model.purposes.classify]\nmodel = \"small\"\n",
    ));
    assert_eq!(cfg.model.purposes["classify"].model, "small");
    assert_eq!(cfg.model.purposes["classify"].catalog_provider, None);

    let routes = build_model_routes(&cfg, &ProviderSecrets::default(), None).expect("routes");
    let exec = GatewayModelExecutor::with_routes(
        routes,
        &cfg,
        DailyBudgetTracker::new(Time::from_millis(1), 1_000),
        DiagnosticSink::new(&ProviderSecrets::default()),
        CueRecorder::new(),
        Arc::new(SystemClock::new()),
    );
    let store = MemStore::new();
    let plan = ContextPlan::default();
    let classify = exec.prepare_request(ModelPurpose::Classify, &plan, &store);
    let reason = exec.prepare_request(ModelPurpose::Reason, &plan, &store);
    let consolidate = exec.prepare_request(ModelPurpose::Consolidate, &plan, &store);
    assert_eq!(classify.model, "small");
    assert_eq!(reason.model, "big");
    assert_eq!(consolidate.model, "big", "absent purpose falls back to [model]");

    // The titler follows classify.
    assert_eq!(exec.live_model().route_for(ModelPurpose::Classify).1, "small");

    // And the request that reaches the provider carries the model.
    let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
    rt.block_on(async {
        let _ = exec.execute_prepared(classify).await;
        let _ = exec.execute_prepared(reason).await;
    });
    let mut m = models_seen(&seen);
    m.sort();
    assert_eq!(m, vec!["big".to_owned(), "small".to_owned()]);
    std::thread::spawn(move || drop(rt)).join().unwrap();
}

#[test]
fn purpose_config_rejects_unknown_purpose_and_missing_model() {
    for bad in [
        "version = 1\n[model.purposes.titler]\nmodel = \"x\"\n",
        "version = 1\n[model.purposes.classify]\nprovider = \"ollama\"\n",
        "version = 1\n[model.purposes.classify]\nmodel = \"x\"\nprovider = \"nope\"\n",
    ] {
        assert!(
            parse_config_str(bad, Path::new("monoagens.toml"), Path::new(".")).is_err(),
            "{bad}"
        );
    }
}

// ---------------------------------------------------------------- 5.2 prices / 5.3 usage

#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
async fn usage_counts_per_purpose_and_cost_follows_known_prices() {
    let (base, seen) = serve(MODELS_REPLY, (200, CHAT_REPLY));
    let env = boot(&config_text(&base, "alpha", "")).await;
    let handle = env.owner.handle().clone();

    // No price known: cost hidden (null), usage still counted.
    handle
        .submit_and_wait(monoagens_core::Input::Percept(Box::new(percept("hello"))))
        .await
        .expect("turn");
    wait_until("model call", || !models_seen(&seen).is_empty()).await;
    wait_until("usage recorded", || {
        env.owner.usage_get()["purposes"].as_array().is_some_and(|p| !p.is_empty())
    })
    .await;
    let u = env.owner.usage_get();
    assert!(u["cost_usd"].is_null(), "{u}");
    let rows = u["purposes"].as_array().unwrap();
    assert_eq!(rows[0]["input_tokens"], json!(100));
    assert_eq!(rows[0]["output_tokens"], json!(50));
    assert_eq!(u["budget"]["used"], json!(150));
    assert_eq!(u["budget"]["left"].as_u64().unwrap() + 150, u["budget"]["limit"].as_u64().unwrap());
    assert!(env.owner.status()["budget"]["cost_usd"].is_null());

    // User-entered price: $2/M in, $4/M out -> 100*2e-6 + 50*4e-6 = 0.0004 per call.
    env.owner
        .config_apply(&json!({ "model": { "prices": { "ollama/alpha": {
            "input_per_mtok": 2.0, "output_per_mtok": 4.0 } } } }))
        .await
        .expect("apply prices");
    let u = env.owner.usage_get();
    let calls = rows.iter().map(|r| r["calls"].as_u64().unwrap()).sum::<u64>() as f64;
    let cost = u["cost_usd"].as_f64().expect("priced");
    assert!((cost - 0.0004 * calls).abs() < 1e-9, "cost {cost} calls {calls}");
    assert_eq!(env.owner.status()["budget"]["cost_usd"].as_f64(), Some(cost));
    env.rt.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
async fn price_from_discovery_cache_is_used() {
    const PRICED: &str = r#"{"data":[{"id":"alpha","pricing":{"prompt":"0.000002","completion":"0.000004"}}]}"#;
    let (base, seen) = serve(PRICED, (200, CHAT_REPLY));
    let env = boot(&config_text(&base, "alpha", "")).await;
    let handle = env.owner.handle().clone();
    let r = env.owner.models_discover("ollama", true).await.expect("discover");
    assert!((r["models"][0]["price_in_per_mtok"].as_f64().unwrap() - 2.0).abs() < 1e-9);
    handle
        .submit_and_wait(monoagens_core::Input::Percept(Box::new(percept("hello"))))
        .await
        .expect("turn");
    wait_until("usage recorded", || {
        env.owner.usage_get()["purposes"].as_array().is_some_and(|p| !p.is_empty())
    })
    .await;
    assert!(env.owner.usage_get()["cost_usd"].is_f64());
    let _ = seen;
    env.rt.shutdown().await;
}

#[test]
fn secret_string_never_prints() {
    assert_eq!(format!("{:?}", SecretString::new("sk-x")), "[REDACTED]");
}

// ---------------------------------------------------------------- 4.3 provider errors

#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
async fn rate_limited_turn_records_failure_with_message_and_a_provider_log_entry() {
    const BODY: &str = r#"{"error":{"message":"slow down please"}}"#;
    let (base, _seen) = serve(MODELS_REPLY, (429, BODY));
    let env = boot(&config_text(&base, "alpha", "")).await;
    let handle = env.owner.handle().clone();
    handle
        .submit_and_wait(monoagens_core::Input::Percept(Box::new(percept("hello"))))
        .await
        .expect("turn");
    let mut detail = None;
    for _ in 0..300 {
        let events = handle.read_events().await.unwrap();
        detail = events.iter().find_map(|e| match &e.detail {
            Some(monoagens_core::EventDetail::Failure { detail, .. }) => Some(detail.clone()),
            _ => None,
        });
        if detail.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let detail = detail.expect("failure event");
    assert!(detail.contains("rate_limit") && detail.contains("429") && detail.contains("slow down please"), "{detail}");
    assert!(
        env.log
            .snapshot()
            .iter()
            .any(|e| e.source == "provider" && e.message.contains("429")),
        "provider log entry with status 429"
    );
    env.rt.shutdown().await;
}
