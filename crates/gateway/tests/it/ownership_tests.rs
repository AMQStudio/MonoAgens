//! Gateway ownership: data-dir lock, admin socket, plugin supervision, config.apply and
//! turn cancel (change `tui-foundation`, tasks 1.2, 2.1-2.5, 4.1-4.3).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use monoagens_core::{
    ContextPlan, CoreConfig, DoorDecision, Doors, EventKind, Input, MemStore, NoopDoors, ReplayMode,
    Store,
};
use monoagens_gateway::{
    AdminClient, AdminServer, BodyTransportConfig, Clock, CueRecorder, DiagnosticSink,
    DoorController, FirstOutcome, GatewayConfig, GatewayHandle, GatewayOptions, GatewayOwner,
    GatewayRuntime, GatewaySeams, LockError, ModelProviderKind, OwnerMode, OwnerParams,
    PluginConfig, PluginEnvContext, PluginKind, PluginState, PluginSupervisor, RestartPolicy,
    RunLog, RunLogLevel, SystemClock, VirtualClock, acquire_data_dir_lock, acquire_plugin_lock,
};
use monoagens_protocol::{
    Action, ActorId, Audience, CURRENT_PROTOCOL_VERSION, OpaquePayload, Percept, PlaceId, Time,
};
use monoagens_reason::{
    CallOutcome, ModelCallFailure, ModelCallRecord, ModelCallSuccess, ModelProvider, ModelRequest,
    ProviderSecrets,
};
use serde_json::{Value, json};
use tempfile::TempDir;

fn test_dir() -> TempDir {
    tempfile::Builder::new()
        .prefix("own-")
        .tempdir_in(env!("CARGO_TARGET_TMPDIR"))
        .expect("tempdir under target")
}

fn fake_plugin_bin() -> PathBuf {
    // `cargo test` builds examples under `<target>/<profile>/examples`; the target dir is the
    // parent of `CARGO_TARGET_TMPDIR`.
    let target = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .parent()
        .expect("target dir");
    for profile in ["debug", "release"] {
        let bin = target.join(profile).join("examples").join("fake_plugin");
        if bin.exists() {
            return bin;
        }
    }
    panic!("fake_plugin example not built under {}", target.display());
}

fn plugin(id: &str, deps: &[&str], args: &[&str]) -> PluginConfig {
    let mut full = vec!["--body-id".to_owned(), format!("body-{id}")];
    full.extend(args.iter().map(|s| (*s).to_owned()));
    PluginConfig {
        id: id.to_owned(),
        enabled: true,
        kind: PluginKind::Adapter,
        transport: BodyTransportConfig::Stdio {
            command: fake_plugin_bin().display().to_string(),
            args: full,
        },
        dependencies: deps.iter().map(|s| (*s).to_owned()).collect(),
        optional_dependencies: Vec::new(),
    }
}

fn test_config() -> GatewayConfig {
    let mut cfg = GatewayConfig::default();
    cfg.model.provider = ModelProviderKind::Scripted;
    cfg.model.model = "mock".to_owned();
    cfg.runtime.hook_timeout_ms = 3_000;
    cfg.runtime.model_timeout_ms = 20_000;
    cfg
}

struct Booted {
    rt: GatewayRuntime,
    handle: GatewayHandle,
}

async fn boot_with<D: DoorController + 'static>(
    doors: D,
    provider: Option<Arc<dyn ModelProvider>>,
) -> Booted {
    let secrets = ProviderSecrets::default();
    let rt = GatewayRuntime::start(
        test_config(),
        &secrets,
        || Ok(MemStore::new()),
        CoreConfig::default(),
        doors,
        GatewayOptions {
            clock: Arc::new(SystemClock::new()),
            provider,
            diagnostics: DiagnosticSink::new(&secrets),
            cues: CueRecorder::new(),
            seams: GatewaySeams::default(),
        },
    )
    .await
    .expect("start gateway");
    let handle = rt.handle();
    Booted { rt, handle }
}

fn runlog_for(handle: &GatewayHandle) -> RunLog {
    let log = RunLog::new(
        Arc::clone(handle.clock()),
        handle.diagnostics().clone(),
        None,
    );
    log.attach_diagnostics(handle.diagnostics());
    log
}

fn supervisor(
    handle: &GatewayHandle,
    data_dir: &Path,
    clock: Arc<dyn Clock>,
    log: &RunLog,
    configs: Vec<PluginConfig>,
) -> PluginSupervisor {
    PluginSupervisor::new(
        handle.clone(),
        PluginEnvContext {
            config_dir: None,
            data_dir: Some(data_dir.to_path_buf()),
            rbac_enabled: false,
            plugin_env: Default::default(),
        },
        clock,
        log.clone(),
        RestartPolicy::default(),
        configs,
    )
}

async fn wait_until<F: FnMut() -> bool>(what: &str, mut cond: F) {
    for _ in 0..600 {
        if cond() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("timed out waiting for: {what}");
}

// ---------------------------------------------------------------- 2.1 data-dir lock

#[test]
fn second_lock_attempt_is_refused_naming_the_holder_pid() {
    let dir = test_dir();
    let first = acquire_data_dir_lock(dir.path()).expect("first lock");
    match acquire_data_dir_lock(dir.path()) {
        Err(LockError::Held(held)) => {
            assert_eq!(held.pid, Some(std::process::id()));
            assert!(held.to_string().contains(&std::process::id().to_string()));
        }
        other => panic!("expected Held, got {other:?}"),
    }
    drop(first);
    acquire_data_dir_lock(dir.path()).expect("lock is free again after release");
}

#[test]
fn lock_is_released_when_the_holder_process_is_killed() {
    use std::io::{BufRead, BufReader};
    use std::process::{Command, Stdio};

    let dir = test_dir();
    let lock_path = dir.path().join("gateway.lock");
    // `flock(1)` on an inherited fd takes the same kernel flock the std lock uses; `exec`
    // keeps the lock in the very process we kill.
    let mut child = Command::new("sh")
        .arg("-c")
        .arg(r#"exec 9>>"$1"; flock -n 9 || exit 3; echo locked; exec sleep 60"#)
        .arg("sh")
        .arg(&lock_path)
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn sh");
    let mut line = String::new();
    BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    if line.trim() != "locked" {
        eprintln!("skipping: flock(1) unavailable or lock not taken");
        let _ = child.kill();
        let _ = child.wait();
        return;
    }
    assert_eq!(line.trim(), "locked");
    assert!(matches!(
        acquire_data_dir_lock(dir.path()),
        Err(LockError::Held(_))
    ));
    child.kill().unwrap();
    child.wait().unwrap();
    acquire_data_dir_lock(dir.path()).expect("lock released by the kernel after kill");
}

// ---------------------------------------------------------------- 1.2 fake plugin

#[test]
fn fake_plugin_writes_boom_and_exits_one_after_about_200ms() {
    use std::process::{Command, Stdio};
    let started = std::time::Instant::now();
    let mut child = Command::new(fake_plugin_bin())
        .args(["--stderr-lines", "boom", "--crash-after-ms", "200", "--exit-code", "1"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let _keep_stdin_open = child.stdin.take();
    let out = child.wait_with_output().unwrap();
    let elapsed = started.elapsed();
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(String::from_utf8_lossy(&out.stderr).trim(), "boom");
    assert!(elapsed >= Duration::from_millis(180), "exited too early: {elapsed:?}");
    assert!(elapsed < Duration::from_millis(3_000), "exited too late: {elapsed:?}");
}

// ---------------------------------------------------------------- 2.2 / 2.3 admin socket

async fn start_owner(
    dir: &Path,
    configs: Vec<PluginConfig>,
    config_text: Option<&str>,
) -> (Booted, Arc<GatewayOwner>, PluginSupervisor, RunLog, monoagens_gateway::DataDirLock) {
    let booted = boot_with(NoopDoors, None).await;
    let log = runlog_for(&booted.handle);
    let sup = supervisor(
        &booted.handle,
        dir,
        Arc::new(SystemClock::new()),
        &log,
        configs,
    );
    let config_path = dir.join("monoagens.toml");
    if let Some(text) = config_text {
        std::fs::write(&config_path, text).unwrap();
    }
    let owner = Arc::new(GatewayOwner::new(OwnerParams {
        handle: booted.handle.clone(),
        supervisor: sup.clone(),
        runlog: log.clone(),
        mode: OwnerMode::Serve,
        config: test_config(),
        config_path,
        secrets_path: dir.join("secrets.toml"),
        data_dir: dir.to_path_buf(),
        owner_actor: ActorId::new("per_owner"),
    }));
    let lock = acquire_data_dir_lock(dir).expect("lock");
    (booted, owner, sup, log, lock)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn admin_socket_is_0600_replaces_stale_socket_and_answers_status_and_unknown_method() {
    use std::os::unix::fs::PermissionsExt;
    let dir = test_dir();
    // A leftover socket file from a dead holder.
    drop(std::os::unix::net::UnixListener::bind(dir.path().join("gateway.sock")).unwrap());
    assert!(dir.path().join("gateway.sock").exists());

    let (booted, owner, _sup, _log, lock) = start_owner(dir.path(), vec![], None).await;
    let server = AdminServer::start(&lock, dir.path(), owner).await.expect("serve");
    let mode = std::fs::metadata(server.path()).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);

    let mut client = AdminClient::connect(dir.path()).await.expect("connect");
    let status = client.call("status", Value::Null).await.expect("status");
    assert_eq!(status["owner"]["pid"], json!(std::process::id()));
    assert_eq!(status["owner"]["mode"], json!("serve"));
    assert_eq!(status["data_dir"], json!(dir.path().display().to_string()));
    assert_eq!(status["provider"], json!("scripted"));
    assert!(status["plugins"].as_array().unwrap().is_empty());
    assert_eq!(status["queue_depth"], json!(0));

    let err = client.call("no.such.method", Value::Null).await.unwrap_err();
    assert_eq!(err.code, -32601);

    server.shutdown();
    assert!(!dir.path().join("gateway.sock").exists());
    booted.rt.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn plugins_restart_through_the_socket_gives_a_new_pid_and_logs_stream() {
    let dir = test_dir();
    let (booted, owner, sup, log, lock) =
        start_owner(dir.path(), vec![plugin("fake", &[], &["--stderr-lines", "hello-from-fake"])], None)
            .await;
    let server = AdminServer::start(&lock, dir.path(), Arc::clone(&owner))
        .await
        .unwrap();
    let mut client = AdminClient::connect(dir.path()).await.unwrap();

    let sub = client.call("logs.subscribe", json!({ "backlog": 0 })).await.unwrap();
    assert!(sub["subscription"].is_u64());

    client.call("plugins.start", json!({ "id": "fake" })).await.unwrap();
    let list = client.call("plugins.list", Value::Null).await.unwrap();
    let p = &list["plugins"][0];
    assert_eq!(p["state"], json!("running"));
    let pid1 = p["pid"].as_u64().unwrap();

    client.call("plugins.restart", json!({ "id": "fake" })).await.unwrap();
    let list = client.call("plugins.list", Value::Null).await.unwrap();
    let p = &list["plugins"][0];
    assert_eq!(p["state"], json!("running"));
    let pid2 = p["pid"].as_u64().unwrap();
    assert_ne!(pid1, pid2, "restart must replace the process");

    // The stderr line arrives as a pushed logs.entry notification under source plugin:fake.
    let mut saw = false;
    for _ in 0..200 {
        let n = tokio::time::timeout(Duration::from_secs(5), client.next_notification())
            .await
            .expect("notification in time")
            .unwrap();
        if n["method"] == "logs.entry"
            && n["params"]["source"] == "plugin:fake"
            && n["params"]["message"] == "hello-from-fake"
        {
            saw = true;
            break;
        }
    }
    assert!(saw, "plugin stderr must be streamed via logs.subscribe");
    assert!(log.snapshot().iter().any(|e| e.source == "plugin:fake"));

    let err = client.call("plugins.stop", json!({ "id": "nope" })).await.unwrap_err();
    assert_eq!(err.code, -32602);
    client.call("plugins.stop", json!({ "id": "fake" })).await.unwrap();
    assert_eq!(sup.info("fake").unwrap().state, PluginState::Stopped);

    server.shutdown();
    sup.shutdown().await;
    booted.rt.shutdown().await;
}

// ---------------------------------------------------------------- 4.1 autostart order

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn autostart_orders_by_dependency_and_isolates_failures() {
    let dir = test_dir();
    let booted = boot_with(NoopDoors, None).await;
    let log = runlog_for(&booted.handle);
    let sup = supervisor(
        &booted.handle,
        dir.path(),
        Arc::new(SystemClock::new()),
        &log,
        vec![plugin("a", &["b"], &[]), plugin("b", &[], &[]), plugin("c", &["x"], &[])],
    );
    let mut events = sup.subscribe();
    sup.start_enabled().await;

    assert_eq!(sup.info("a").unwrap().state, PluginState::Running);
    assert_eq!(sup.info("b").unwrap().state, PluginState::Running);
    let c = sup.info("c").unwrap();
    assert_eq!(c.state, PluginState::Failed);
    assert!(c.reason.unwrap().contains("'x'"), "reason must name x");

    let mut seen = Vec::new();
    while let Ok(ev) = events.try_recv() {
        seen.push((ev.id, ev.state));
    }
    let pos = |id: &str, st: PluginState| {
        seen.iter().position(|(i, s)| i == id && *s == st).expect("event present")
    };
    assert!(
        pos("b", PluginState::Running) < pos("a", PluginState::Starting),
        "b must be running before a starts: {seen:?}"
    );
    sup.shutdown().await;
    booted.rt.shutdown().await;
}

// ---------------------------------------------------------------- 4.2 crash / backoff

async fn next_event_where(
    rx: &mut tokio::sync::broadcast::Receiver<monoagens_gateway::PluginStateEvent>,
    mut pred: impl FnMut(&monoagens_gateway::PluginStateEvent) -> bool,
) -> monoagens_gateway::PluginStateEvent {
    loop {
        let ev = tokio::time::timeout(Duration::from_secs(10), rx.recv())
            .await
            .expect("plugin event in time")
            .expect("event stream open");
        if pred(&ev) {
            return ev;
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn crash_is_visible_logged_and_restarted_after_the_first_backoff() {
    let dir = test_dir();
    let booted = boot_with(NoopDoors, None).await;
    let log = runlog_for(&booted.handle);
    let vclock = Arc::new(VirtualClock::new(Time::from_millis(1_000_000)));
    let sup = supervisor(
        &booted.handle,
        dir.path(),
        vclock.clone(),
        &log,
        vec![plugin("fake", &[], &["--stderr-lines", "boom", "--crash-after-ms", "300", "--exit-code", "1"])],
    );
    let mut ev = sup.subscribe();
    sup.start_enabled().await;

    let failed = next_event_where(&mut ev, |e| e.state == PluginState::Failed).await;
    assert_eq!(failed.reason.as_deref(), Some("exited 1: boom"));
    assert_eq!(failed.retry_at_ms, Some(1_001_000), "first backoff is 1 s");
    assert!(
        log.snapshot()
            .iter()
            .any(|e| e.source == "plugin:fake" && e.message == "boom"),
        "stderr line must be in the runtime log under plugin:fake"
    );
    assert_eq!(sup.info("fake").unwrap().stderr_tail, vec!["boom".to_owned()]);

    // Nothing restarts until the injected clock passes the backoff.
    tokio::time::sleep(Duration::from_millis(250)).await;
    assert_eq!(sup.info("fake").unwrap().state, PluginState::Failed);
    vclock.advance_to(Time::from_millis(1_001_000));
    let running = next_event_where(&mut ev, |e| e.state == PluginState::Running).await;
    assert!(running.pid.is_some());
    sup.shutdown().await;
    booted.rt.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn gives_up_after_three_failures_and_a_manual_start_resets() {
    let dir = test_dir();
    let booted = boot_with(NoopDoors, None).await;
    let log = runlog_for(&booted.handle);
    let vclock = Arc::new(VirtualClock::new(Time::from_millis(5_000_000)));
    let sup = supervisor(
        &booted.handle,
        dir.path(),
        vclock.clone(),
        &log,
        vec![plugin("fake", &[], &["--stderr-lines", "boom", "--crash-after-ms", "100"])],
    );
    let mut ev = sup.subscribe();
    sup.start_enabled().await;

    let f1 = next_event_where(&mut ev, |e| e.state == PluginState::Failed).await;
    assert_eq!(f1.retry_at_ms, Some(5_001_000));
    vclock.advance_to(Time::from_millis(f1.retry_at_ms.unwrap()));
    let f2 = next_event_where(&mut ev, |e| e.state == PluginState::Failed).await;
    assert_eq!(f2.retry_at_ms.map(|t| t - 5_001_000), Some(5_000), "second backoff is 5 s");
    vclock.advance_to(Time::from_millis(f2.retry_at_ms.unwrap()));
    let f3 = next_event_where(&mut ev, |e| e.state == PluginState::Failed).await;
    assert_eq!(f3.retry_at_ms, None, "third failure in the window gives up");

    // No further restart, however far the clock moves.
    vclock.advance_by(10_000_000);
    tokio::time::sleep(Duration::from_millis(400)).await;
    while let Ok(e) = ev.try_recv() {
        assert_ne!(e.state, PluginState::Starting, "must not restart after giving up");
    }
    let info = sup.info("fake").unwrap();
    assert_eq!(info.state, PluginState::Failed);
    assert!(info.reason.unwrap().starts_with("exited 1: boom"));

    // A manual start clears the history and runs again.
    assert!(matches!(sup.start("fake").await, Ok(FirstOutcome::Running)));
    sup.shutdown().await;
    booted.rt.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn user_stop_cancels_the_restart_loop() {
    let dir = test_dir();
    let booted = boot_with(NoopDoors, None).await;
    let log = runlog_for(&booted.handle);
    let vclock = Arc::new(VirtualClock::new(Time::from_millis(9_000_000)));
    let sup = supervisor(
        &booted.handle,
        dir.path(),
        vclock.clone(),
        &log,
        vec![plugin("fake", &[], &["--crash-after-ms", "100"])],
    );
    let mut ev = sup.subscribe();
    sup.start_enabled().await;
    let failed = next_event_where(&mut ev, |e| e.state == PluginState::Failed).await;
    assert!(failed.retry_at_ms.is_some());

    sup.stop("fake").await.unwrap();
    assert_eq!(sup.info("fake").unwrap().state, PluginState::Stopped);
    vclock.advance_by(100_000);
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(sup.info("fake").unwrap().state, PluginState::Stopped);
    while let Ok(e) = ev.try_recv() {
        assert_ne!(e.state, PluginState::Starting);
    }
    booted.rt.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn body_rpc_disconnect_is_treated_like_an_exit() {
    use monoagens_protocol::{BodyDiscoverResult, BodyId};
    let dir = test_dir();
    let booted = boot_with(NoopDoors, None).await;
    let log = runlog_for(&booted.handle);
    let vclock = Arc::new(VirtualClock::new(Time::from_millis(1_000)));

    let discover = serde_json::to_string(&json!({
        "jsonrpc": "2.0", "id": 1,
        "result": BodyDiscoverResult {
            body_id: BodyId::new("hangs"),
            protocol_version: CURRENT_PROTOCOL_VERSION,
            capabilities: vec![],
            places: vec![PlaceId::new("p:hangs")],
        }
    }))
    .unwrap();
    // Answers discover, then closes stdout but stays alive.
    let script = format!("read line; echo '{discover}'; exec 1>&-; sleep 30");
    let mut cfg = plugin("hangs", &[], &[]);
    cfg.transport = BodyTransportConfig::Stdio {
        command: "/bin/sh".to_owned(),
        args: vec!["-c".to_owned(), script],
    };
    let sup = supervisor(&booted.handle, dir.path(), vclock, &log, vec![cfg]);
    let mut ev = sup.subscribe();
    sup.start_enabled().await;
    let failed = next_event_where(&mut ev, |e| e.state == PluginState::Failed).await;
    assert_eq!(failed.reason.as_deref(), Some("body rpc disconnected"));
    sup.shutdown().await;
    booted.rt.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn spawn_failure_is_reported_as_failed() {
    let dir = test_dir();
    let booted = boot_with(NoopDoors, None).await;
    let log = runlog_for(&booted.handle);
    let mut cfg = plugin("ghost", &[], &[]);
    cfg.transport = BodyTransportConfig::Stdio {
        command: "/nonexistent/plugin-binary".to_owned(),
        args: vec![],
    };
    let sup = supervisor(
        &booted.handle,
        dir.path(),
        Arc::new(VirtualClock::new(Time::from_millis(1))),
        &log,
        vec![cfg],
    );
    sup.start_enabled().await;
    let info = sup.info("ghost").unwrap();
    assert_eq!(info.state, PluginState::Failed);
    assert!(info.reason.unwrap().contains("spawn"));
    booted.rt.shutdown().await;
}

// ---------------------------------------------------------------- 4.3 per-plugin lock

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn externally_held_plugin_lock_means_running_elsewhere_and_no_child() {
    let dir = test_dir();
    let marker = dir.path().join("spawned.marker");
    let _external = acquire_plugin_lock(dir.path(), "whatsapp").expect("external holder");

    let booted = boot_with(NoopDoors, None).await;
    let log = runlog_for(&booted.handle);
    let mut cfg = plugin("whatsapp", &[], &[]);
    cfg.transport = BodyTransportConfig::Stdio {
        command: "/bin/sh".to_owned(),
        args: vec!["-c".to_owned(), format!("touch {}; sleep 5", marker.display())],
    };
    let sup = supervisor(
        &booted.handle,
        dir.path(),
        Arc::new(SystemClock::new()),
        &log,
        vec![cfg],
    );
    sup.start_enabled().await;
    let info = sup.info("whatsapp").unwrap();
    assert_eq!(info.state, PluginState::RunningElsewhere);
    assert!(info.pid.is_none());
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!marker.exists(), "no child may be spawned");
    booted.rt.shutdown().await;
}

// ---------------------------------------------------------------- 2.4 config.apply

#[derive(Clone, Default)]
struct ReloadProbe {
    modes: Arc<Mutex<Vec<String>>>,
    inner: NoopDoors,
}

impl Doors for ReloadProbe {
    fn inbound(&mut self, p: &mut Percept) -> DoorDecision {
        self.inner.inbound(p)
    }
    fn context(&mut self, p: &mut ContextPlan) -> DoorDecision {
        self.inner.context(p)
    }
    fn action(&mut self, a: &mut Action) -> DoorDecision {
        self.inner.action(a)
    }
}

impl DoorController for ReloadProbe {
    fn reload_valve(&mut self, config: &GatewayConfig) -> bool {
        self.modes.lock().unwrap().push(config.valve.mode.clone());
        true
    }
}

const CONFIG_TEXT: &str = "\
version = 1
# top-level comment survives
[valve]
mode = \"enforcing\" # keep this note
max_chain_wakes = 8

[model]
provider = \"scripted\"
model = \"mock\"
";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn config_apply_writes_bak_preserves_comments_and_reloads_valve() {
    let dir = test_dir();
    let probe = ReloadProbe::default();
    let modes = Arc::clone(&probe.modes);
    let booted = boot_with(probe, None).await;
    let log = runlog_for(&booted.handle);
    let sup = supervisor(&booted.handle, dir.path(), Arc::new(SystemClock::new()), &log, vec![]);
    let config_path = dir.path().join("monoagens.toml");
    std::fs::write(&config_path, CONFIG_TEXT).unwrap();
    let hook_calls = Arc::new(AtomicUsize::new(0));
    let owner = GatewayOwner::new(OwnerParams {
        handle: booted.handle.clone(),
        supervisor: sup,
        runlog: log.clone(),
        mode: OwnerMode::Tui,
        config: monoagens_gateway::load_config_file(&config_path, &dir.path().join("secrets.toml"))
            .unwrap(),
        config_path: config_path.clone(),
        secrets_path: dir.path().join("secrets.toml"),
        data_dir: dir.path().to_path_buf(),
        owner_actor: ActorId::new("per_owner"),
    });
    let hc = Arc::clone(&hook_calls);
    owner.set_provider_rebuild_hook(Arc::new(move |_| {
        hc.fetch_add(1, Ordering::SeqCst);
    }));

    let report = owner
        .config_apply(&json!({ "valve": { "mode": "off" } }))
        .await
        .expect("apply");
    assert!(report.valve);
    assert!(!report.provider);
    assert_eq!(modes.lock().unwrap().as_slice(), ["off".to_owned()]);

    let bak = std::fs::read_to_string(dir.path().join("monoagens.toml.bak")).unwrap();
    assert_eq!(bak, CONFIG_TEXT);
    let new = std::fs::read_to_string(&config_path).unwrap();
    assert!(new.contains("mode = \"off\""));
    assert!(new.contains("# top-level comment survives"));
    assert!(new.contains("# keep this note"), "inline comment kept: {new}");
    assert!(new.contains("max_chain_wakes = 8"));

    // Provider settings go through the hook stub.
    let report = owner
        .config_apply(&json!({ "model": { "model": "other" } }))
        .await
        .unwrap();
    assert!(report.provider && !report.valve);
    assert_eq!(hook_calls.load(Ordering::SeqCst), 1);

    // Invalid patch: rejected, file untouched.
    let before = std::fs::read_to_string(&config_path).unwrap();
    let err = owner
        .config_apply(&json!({ "valve": { "mode": "bogus" } }))
        .await
        .unwrap_err();
    assert_eq!(err.code, -32000);
    assert_eq!(std::fs::read_to_string(&config_path).unwrap(), before);

    // config.get returns the raw text.
    let got = owner.config_get().unwrap();
    assert!(got["toml"].as_str().unwrap().contains("# top-level comment survives"));
    booted.rt.shutdown().await;
}

// ---------------------------------------------------------------- 2.5 turn cancel

/// Scripted-style provider that blocks `delay` before answering with a single `think`.
struct SlowProvider {
    delay: Duration,
    entered: Arc<AtomicBool>,
    finished: Arc<AtomicBool>,
}

impl ModelProvider for SlowProvider {
    fn provider_name(&self) -> &str {
        "slow_test"
    }

    fn invoke(&self, request: &ModelRequest) -> Result<ModelCallSuccess, ModelCallFailure> {
        self.entered.store(true, Ordering::SeqCst);
        std::thread::sleep(self.delay);
        self.finished.store(true, Ordering::SeqCst);
        Ok(ModelCallSuccess {
            record: ModelCallRecord {
                request_id: request.request_id,
                purpose: request.purpose,
                provider: "slow_test".to_owned(),
                model: request.model.clone(),
                usage: None,
                latency_ms: 1,
                outcome: CallOutcome::Success,
                strict_schema: true,
            },
            output: monoagens_reason::ModelOutput::ThoughtActs(monoagens_reason::ParsedModelActs {
                protocol_version: CURRENT_PROTOCOL_VERSION,
                acts: vec![monoagens_protocol::ThoughtAct::Think {
                    thought: "too late".to_owned(),
                    motivation: 3,
                    confidence_0_20: 14,
                    used_ids: vec![],
                    importance: Some(0.4),
                    basis: None,
                    value: None,
                    cost: None,
                    domain: None,
                }],
                rubrics: vec![None],
                fallback_plain_text: false,
            }),
            raw_text: "too late".to_owned(),
        })
    }
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

#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
async fn turn_cancel_records_one_failure_discards_the_late_result_and_replays_identically() {
    let entered = Arc::new(AtomicBool::new(false));
    let finished = Arc::new(AtomicBool::new(false));
    let provider = Arc::new(SlowProvider {
        delay: Duration::from_millis(1_200),
        entered: Arc::clone(&entered),
        finished: Arc::clone(&finished),
    });
    let booted = boot_with(NoopDoors, Some(provider)).await;
    let handle = booted.handle.clone();
    let log = runlog_for(&handle);
    let sup = supervisor(&handle, test_dir().path(), Arc::new(SystemClock::new()), &log, vec![]);
    let dir = test_dir();
    let owner = GatewayOwner::new(OwnerParams {
        handle: handle.clone(),
        supervisor: sup,
        runlog: log.clone(),
        mode: OwnerMode::Serve,
        config: test_config(),
        config_path: dir.path().join("monoagens.toml"),
        secrets_path: dir.path().join("secrets.toml"),
        data_dir: dir.path().to_path_buf(),
        owner_actor: ActorId::new("per_owner"),
    });

    // Nothing in flight: nothing submitted.
    assert_eq!(owner.turn_cancel(), json!({ "cancelled": false }));

    handle
        .submit_and_wait(Input::Percept(Box::new(percept("hello"))))
        .await
        .expect("percept");
    wait_until("model call in flight", || {
        entered.load(Ordering::SeqCst) && handle.in_flight_model_calls() == 1
    })
    .await;

    let t0 = std::time::Instant::now();
    assert_eq!(owner.turn_cancel(), json!({ "cancelled": true }));
    // The turn ends (failure recorded) well before the slow provider returns.
    loop {
        let events = handle.read_events().await.unwrap();
        if events.iter().any(|e| e.kind == EventKind::Failure) {
            break;
        }
        assert!(t0.elapsed() < Duration::from_secs(1), "cancel must end the turn within 1 s");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(!finished.load(Ordering::SeqCst), "provider still blocking");
    assert_eq!(owner.turn_cancel(), json!({ "cancelled": false }), "second cancel is a no-op");

    // Let the abandoned call return; its result must be dropped.
    wait_until("late result discarded", || finished.load(Ordering::SeqCst)).await;
    wait_until("late-discard log line", || {
        log.snapshot()
            .iter()
            .any(|e| e.source == "provider" && e.message.contains("late result discarded"))
    })
    .await;

    let events = handle.read_events().await.unwrap();
    let failures: Vec<_> = events.iter().filter(|e| e.kind == EventKind::Failure).collect();
    assert_eq!(failures.len(), 1, "exactly one ModelFailure");
    assert!(matches!(
        &failures[0].detail,
        Some(monoagens_core::EventDetail::Failure { detail, .. }) if detail == "cancelled_by_owner"
    ));
    assert!(
        !events.iter().any(|e| e.kind == EventKind::Thought),
        "no ModelResult may be recorded for the cancelled request"
    );
    assert_eq!(handle.in_flight_model_calls(), 0);

    // `monoagens replay` machinery: the recorded log reproduces identical decisions.
    let mut mem = MemStore::new();
    for rec in &events {
        mem.append(rec.clone()).expect("append to replay store");
    }
    let report = monoagens_core::replay_log(&mem, ReplayMode::Unpinned, None).expect("replay");
    assert!(report.is_exact(), "replay diverged: {:?}", report.mismatches.first());

    // Silence unused-import lints for helper types used only in other tests.
    let _ = (RunLogLevel::Info, ModelProviderKind::Scripted);
    booted.rt.shutdown().await;
}
