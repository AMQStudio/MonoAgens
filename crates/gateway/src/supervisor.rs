//! Plugin supervision shared by `monoagens serve` and a TUI hosting the gateway.
//!
//! The supervisor owns every plugin child process: it autostarts `enabled = true` plugins in
//! dependency order, pipes stderr into the runtime log (`plugin:<id>`) and a last-20-lines
//! buffer, watches for exit or body-RPC disconnect, restarts with backoff (1 s / 5 s / 30 s,
//! giving up at the 4th failure in 10 minutes), and guards each plugin with an advisory lock
//! `<data_dir>/plugins/<id>.lock` (state `running_elsewhere` when another process holds it).
//! Backoff uses the injected [`Clock`], so tests advance a [`crate::VirtualClock`] instead of
//! sleeping.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;
use std::os::unix::process::ExitStatusExt;
use std::process::{ExitStatus, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use monoagens_protocol::BodyId;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::{broadcast, oneshot, watch};
use tokio::task::JoinHandle;

use crate::clock::Clock;
use crate::config::{BodyTransportConfig, PluginConfig};
use crate::lock::{FileLock, LockError, acquire_plugin_lock};
use crate::plugins::{PluginEnvContext, build_plugin_env, resolve_plugin_boot_order};
use crate::runlog::RunLog;
use crate::runtime::GatewayHandle;

/// Number of trailing stderr lines kept per plugin.
pub const STDERR_TAIL_LINES: usize = 20;

const BODY_POLL_INTERVAL: Duration = Duration::from_millis(200);
const STOP_GRACE: Duration = Duration::from_secs(2);
const FIRST_OUTCOME_TIMEOUT: Duration = Duration::from_secs(20);
const STDERR_DRAIN: Duration = Duration::from_millis(500);

/// Lifecycle state of a supervised plugin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginState {
    Stopped,
    Starting,
    Running,
    Failed,
    /// Another process holds `<data_dir>/plugins/<id>.lock`; nothing is spawned here.
    RunningElsewhere,
}

impl PluginState {
    /// Wire label (`stopped`, `starting`, `running`, `failed`, `running_elsewhere`).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Stopped => "stopped",
            Self::Starting => "starting",
            Self::Running => "running",
            Self::Failed => "failed",
            Self::RunningElsewhere => "running_elsewhere",
        }
    }
}

/// Snapshot of one plugin for `plugins.list` / `status`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginInfo {
    pub id: String,
    pub enabled: bool,
    pub state: PluginState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body_id: Option<String>,
    /// Clock time (epoch ms) of the next scheduled restart while backing off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_at_ms: Option<u64>,
    #[serde(default)]
    pub stderr_tail: Vec<String>,
}

/// Plugin state change published on [`PluginSupervisor::subscribe`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginStateEvent {
    pub id: String,
    pub state: PluginState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_at_ms: Option<u64>,
    #[serde(default)]
    pub ts_ms: u64,
}

/// Restart backoff policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestartPolicy {
    /// Delay before restart number `n` (1-based, clamped to the last entry).
    pub delays_ms: Vec<u64>,
    /// Give up when this many failures happened inside `window_ms` (the 4th failure: delays
    /// 1 s, 5 s, 30 s are used, then no further restart).
    pub max_failures: usize,
    pub window_ms: u64,
}

impl Default for RestartPolicy {
    fn default() -> Self {
        Self {
            delays_ms: vec![1_000, 5_000, 30_000],
            max_failures: 4,
            window_ms: 600_000,
        }
    }
}

impl RestartPolicy {
    fn delay_for(&self, failures_in_window: usize) -> u64 {
        let idx = failures_in_window.saturating_sub(1);
        self.delays_ms
            .get(idx)
            .or_else(|| self.delays_ms.last())
            .copied()
            .unwrap_or(1_000)
    }
}

/// Supervisor API error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SupervisorError {
    UnknownPlugin(String),
}

impl fmt::Display for SupervisorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownPlugin(id) => write!(f, "unknown plugin '{id}'"),
        }
    }
}

impl std::error::Error for SupervisorError {}

/// What the first start attempt of a run ended in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FirstOutcome {
    Running,
    RunningElsewhere,
    Failed,
    /// Still starting when the wait gave up, or already running before this call.
    Pending,
}

struct RunCtl {
    stop: watch::Sender<bool>,
    join: JoinHandle<()>,
}

struct Slot {
    config: PluginConfig,
    state: PluginState,
    reason: Option<String>,
    pid: Option<u32>,
    body_id: Option<String>,
    retry_at_ms: Option<u64>,
    tail: VecDeque<String>,
    run: Option<RunCtl>,
}

impl Slot {
    fn new(config: PluginConfig) -> Self {
        Self {
            config,
            state: PluginState::Stopped,
            reason: None,
            pid: None,
            body_id: None,
            retry_at_ms: None,
            tail: VecDeque::new(),
            run: None,
        }
    }

    fn info(&self) -> PluginInfo {
        PluginInfo {
            id: self.config.id.clone(),
            enabled: self.config.enabled,
            state: self.state,
            reason: self.reason.clone(),
            pid: self.pid,
            body_id: self.body_id.clone(),
            retry_at_ms: self.retry_at_ms,
            stderr_tail: self.tail.iter().cloned().collect(),
        }
    }
}

#[derive(Default)]
struct Table {
    order: Vec<String>,
    slots: BTreeMap<String, Slot>,
}

struct SupInner {
    handle: GatewayHandle,
    env_ctx: PluginEnvContext,
    clock: Arc<dyn Clock>,
    runlog: RunLog,
    policy: RestartPolicy,
    table: Mutex<Table>,
    events: broadcast::Sender<PluginStateEvent>,
}

/// Owns plugin processes for the lock-holding gateway process. Cheap to clone.
#[derive(Clone)]
pub struct PluginSupervisor {
    inner: Arc<SupInner>,
}

impl PluginSupervisor {
    /// Creates a supervisor for `configs` (nothing is started yet).
    #[must_use]
    pub fn new(
        handle: GatewayHandle,
        env_ctx: PluginEnvContext,
        clock: Arc<dyn Clock>,
        runlog: RunLog,
        policy: RestartPolicy,
        configs: Vec<PluginConfig>,
    ) -> Self {
        runlog.register_secret_values(env_ctx.plugin_env.values().map(|s| s.expose()));
        let mut table = Table::default();
        for cfg in configs {
            if table.slots.contains_key(&cfg.id) {
                continue;
            }
            table.order.push(cfg.id.clone());
            table.slots.insert(cfg.id.clone(), Slot::new(cfg));
        }
        let (events, _rx) = broadcast::channel(256);
        Self {
            inner: Arc::new(SupInner {
                handle,
                env_ctx,
                clock,
                runlog,
                policy,
                table: Mutex::new(table),
                events,
            }),
        }
    }

    /// Subscribes to plugin state changes.
    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<PluginStateEvent> {
        self.inner.events.subscribe()
    }

    /// Snapshot of every plugin in configuration order.
    #[must_use]
    pub fn list(&self) -> Vec<PluginInfo> {
        let t = self.inner.table.lock().expect("supervisor table lock");
        t.order
            .iter()
            .filter_map(|id| t.slots.get(id))
            .map(Slot::info)
            .collect()
    }

    /// Snapshot of one plugin.
    #[must_use]
    pub fn info(&self, id: &str) -> Option<PluginInfo> {
        self.inner
            .table
            .lock()
            .expect("supervisor table lock")
            .slots
            .get(id)
            .map(Slot::info)
    }

    /// Starts every `enabled = true` plugin in dependency order. A plugin with a missing or
    /// cyclic hard dependency becomes `Failed` with the reason; the others still start.
    pub async fn start_enabled(&self) {
        let (configs, rbac) = {
            let t = self.inner.table.lock().expect("supervisor table lock");
            (
                t.order
                    .iter()
                    .filter_map(|id| t.slots.get(id))
                    .map(|s| s.config.clone())
                    .collect::<Vec<_>>(),
                self.inner.env_ctx.rbac_enabled,
            )
        };
        let builtin: &[&str] = if rbac { &["rbac"] } else { &[] };
        let plan = resolve_plugin_boot_order(&configs, builtin);
        for skip in &plan.skipped {
            self.inner.set_state(
                &skip.id,
                PluginState::Failed,
                Some(skip.reason.clone()),
                None,
                None,
                None,
            );
        }
        let mut bad: BTreeSet<String> = plan.skipped.iter().map(|s| s.id.clone()).collect();
        for plugin in plan.ordered {
            if let Some(dep) = plugin.dependencies.iter().find(|d| bad.contains(d.as_str())) {
                self.inner.set_state(
                    &plugin.id,
                    PluginState::Failed,
                    Some(format!("required dependency '{dep}' failed to start")),
                    None,
                    None,
                    None,
                );
                bad.insert(plugin.id.clone());
                continue;
            }
            if let Ok(FirstOutcome::Failed) = self.start(&plugin.id).await {
                bad.insert(plugin.id.clone());
            }
        }
    }

    /// Starts a plugin (resetting its failure history) and waits for its first start outcome.
    /// A plugin that is already running is left untouched.
    pub async fn start(&self, id: &str) -> Result<FirstOutcome, SupervisorError> {
        let rx = {
            let mut t = self.inner.table.lock().expect("supervisor table lock");
            let slot = t
                .slots
                .get_mut(id)
                .ok_or_else(|| SupervisorError::UnknownPlugin(id.to_owned()))?;
            if slot.run.as_ref().is_some_and(|r| !r.join.is_finished()) {
                return Ok(FirstOutcome::Pending);
            }
            let (stop_tx, stop_rx) = watch::channel(false);
            let (first_tx, first_rx) = oneshot::channel();
            let inner = Arc::clone(&self.inner);
            let config = slot.config.clone();
            let join = tokio::spawn(supervise(inner, config, stop_rx, first_tx));
            slot.run = Some(RunCtl { stop: stop_tx, join });
            first_rx
        };
        Ok(tokio::time::timeout(FIRST_OUTCOME_TIMEOUT, rx)
            .await
            .ok()
            .and_then(Result::ok)
            .unwrap_or(FirstOutcome::Pending))
    }

    /// Stops a plugin: cancels any pending restart, asks the body to shut down, then kills the
    /// process if it lingers. Idempotent.
    pub async fn stop(&self, id: &str) -> Result<(), SupervisorError> {
        let run = {
            let mut t = self.inner.table.lock().expect("supervisor table lock");
            let slot = t
                .slots
                .get_mut(id)
                .ok_or_else(|| SupervisorError::UnknownPlugin(id.to_owned()))?;
            slot.run.take()
        };
        if let Some(run) = run {
            let _ = run.stop.send(true);
            let _ = run.join.await;
        }
        self.inner
            .set_state(id, PluginState::Stopped, None, None, None, None);
        Ok(())
    }

    /// Stops then starts a plugin.
    pub async fn restart(&self, id: &str) -> Result<FirstOutcome, SupervisorError> {
        self.stop(id).await?;
        self.start(id).await
    }

    /// Stops every plugin (used at gateway shutdown).
    pub async fn shutdown(&self) {
        let ids: Vec<String> = self
            .inner
            .table
            .lock()
            .expect("supervisor table lock")
            .order
            .clone();
        for id in ids {
            let _ = self.stop(&id).await;
        }
    }

    /// Applies a new plugin configuration: removed plugins are stopped and dropped, changed
    /// ones restarted when they were running (or enabled), new enabled ones started. Returns
    /// the ids whose configuration changed.
    pub async fn reload(&self, new_configs: Vec<PluginConfig>) -> Vec<String> {
        let mut changed = Vec::new();
        let current: Vec<(String, PluginConfig)> = {
            let t = self.inner.table.lock().expect("supervisor table lock");
            t.order
                .iter()
                .filter_map(|id| t.slots.get(id).map(|s| (id.clone(), s.config.clone())))
                .collect()
        };
        let new_ids: BTreeSet<String> = new_configs.iter().map(|c| c.id.clone()).collect();
        for (id, _) in &current {
            if !new_ids.contains(id) {
                let _ = self.stop(id).await;
                let mut t = self.inner.table.lock().expect("supervisor table lock");
                t.slots.remove(id);
                t.order.retain(|x| x != id);
                changed.push(id.clone());
            }
        }
        let mut to_start = Vec::new();
        for cfg in new_configs {
            let existing = current.iter().find(|(id, _)| *id == cfg.id).map(|(_, c)| c);
            match existing {
                None => {
                    let id = cfg.id.clone();
                    let enabled = cfg.enabled;
                    let mut t = self.inner.table.lock().expect("supervisor table lock");
                    t.order.push(id.clone());
                    t.slots.insert(id.clone(), Slot::new(cfg));
                    drop(t);
                    changed.push(id.clone());
                    if enabled {
                        to_start.push(id);
                    }
                }
                Some(old) if *old != cfg => {
                    let id = cfg.id.clone();
                    let was_active = self
                        .inner
                        .table
                        .lock()
                        .expect("supervisor table lock")
                        .slots
                        .get(&id)
                        .is_some_and(|s| s.run.as_ref().is_some_and(|r| !r.join.is_finished()));
                    let _ = self.stop(&id).await;
                    let enabled = cfg.enabled;
                    if let Some(slot) = self
                        .inner
                        .table
                        .lock()
                        .expect("supervisor table lock")
                        .slots
                        .get_mut(&id)
                    {
                        slot.config = cfg;
                    }
                    changed.push(id.clone());
                    if enabled || was_active {
                        to_start.push(id);
                    }
                }
                Some(_) => {}
            }
        }
        for id in to_start {
            let _ = self.start(&id).await;
        }
        changed
    }
}

impl SupInner {
    fn set_state(
        &self,
        id: &str,
        state: PluginState,
        reason: Option<String>,
        pid: Option<u32>,
        body_id: Option<String>,
        retry_at_ms: Option<u64>,
    ) {
        {
            let mut t = self.table.lock().expect("supervisor table lock");
            let Some(slot) = t.slots.get_mut(id) else {
                return;
            };
            slot.state = state;
            slot.reason = reason.clone();
            slot.pid = pid;
            slot.body_id = body_id;
            slot.retry_at_ms = retry_at_ms;
        }
        let msg = match (&reason, state) {
            (Some(r), _) => format!("plugin '{id}' {}: {r}", state.as_str()),
            (None, _) => format!("plugin '{id}' {}", state.as_str()),
        };
        if state == PluginState::Failed {
            self.runlog.warn("gateway", &msg);
        } else {
            self.runlog.info("gateway", &msg);
        }
        let _ = self.events.send(PluginStateEvent {
            id: id.to_owned(),
            state,
            reason,
            pid,
            retry_at_ms,
            ts_ms: self.clock.now().as_millis(),
        });
    }

    fn push_tail(&self, id: &str, line: String) {
        let mut t = self.table.lock().expect("supervisor table lock");
        if let Some(slot) = t.slots.get_mut(id) {
            if slot.tail.len() == STDERR_TAIL_LINES {
                slot.tail.pop_front();
            }
            slot.tail.push_back(line);
        }
    }

    fn last_stderr_line(&self, id: &str) -> Option<String> {
        self.table
            .lock()
            .expect("supervisor table lock")
            .slots
            .get(id)
            .and_then(|s| s.tail.back().cloned())
    }
}

enum Attempt {
    Stopped,
    Failed(String),
}

fn describe_exit(status: &ExitStatus) -> String {
    match (status.code(), status.signal()) {
        (Some(code), _) => format!("exited {code}"),
        (None, Some(sig)) => format!("exited by signal {sig}"),
        (None, None) => "exited".to_owned(),
    }
}

async fn supervise(
    inner: Arc<SupInner>,
    config: PluginConfig,
    mut stop: watch::Receiver<bool>,
    first_tx: oneshot::Sender<FirstOutcome>,
) {
    let id = config.id.clone();
    let mut first = Some(first_tx);
    let signal_first = |first: &mut Option<oneshot::Sender<FirstOutcome>>, o: FirstOutcome| {
        if let Some(tx) = first.take() {
            let _ = tx.send(o);
        }
    };
    let mut failures: VecDeque<u64> = VecDeque::new();

    loop {
        // Second guard: a plugin started outside the gateway keeps its own lock.
        let _plugin_lock: Option<FileLock> = match inner.env_ctx.data_dir.as_deref() {
            Some(data_dir) => match acquire_plugin_lock(data_dir, &id) {
                Ok(lock) => Some(lock),
                Err(LockError::Held(_)) => {
                    inner.set_state(
                        &id,
                        PluginState::RunningElsewhere,
                        Some("running in another MonoAgens".to_owned()),
                        None,
                        None,
                        None,
                    );
                    signal_first(&mut first, FirstOutcome::RunningElsewhere);
                    return;
                }
                Err(err @ LockError::Io { .. }) => {
                    let reason = format!("plugin lock failed: {err}");
                    if !fail_and_backoff(
                        &inner, &id, &mut stop, &mut failures, reason, &mut first,
                    )
                    .await
                    {
                        return;
                    }
                    continue;
                }
            },
            None => None,
        };

        match run_once(&inner, &config, &mut stop, &mut first).await {
            Attempt::Stopped => {
                inner.set_state(&id, PluginState::Stopped, None, None, None, None);
                signal_first(&mut first, FirstOutcome::Failed);
                return;
            }
            Attempt::Failed(reason) => {
                drop(_plugin_lock);
                if !fail_and_backoff(&inner, &id, &mut stop, &mut failures, reason, &mut first)
                    .await
                {
                    return;
                }
            }
        }
    }
}

/// Records a failure and waits out the backoff. Returns `false` when the loop must end
/// (gave up, or the user stopped the plugin meanwhile).
async fn fail_and_backoff(
    inner: &Arc<SupInner>,
    id: &str,
    stop: &mut watch::Receiver<bool>,
    failures: &mut VecDeque<u64>,
    reason: String,
    first: &mut Option<oneshot::Sender<FirstOutcome>>,
) -> bool {
    let now = inner.clock.now().as_millis();
    failures.push_back(now);
    while failures
        .front()
        .is_some_and(|t| now.saturating_sub(*t) > inner.policy.window_ms)
    {
        failures.pop_front();
    }
    let n = failures.len();
    if n >= inner.policy.max_failures {
        inner.set_state(id, PluginState::Failed, Some(reason), None, None, None);
        if let Some(tx) = first.take() {
            let _ = tx.send(FirstOutcome::Failed);
        }
        inner.runlog.error(
            "gateway",
            &format!(
                "plugin '{id}' failed {n} times within {} s; giving up until started manually",
                inner.policy.window_ms / 1_000
            ),
        );
        return false;
    }
    let delay = inner.policy.delay_for(n);
    let deadline = now.saturating_add(delay);
    inner.set_state(
        id,
        PluginState::Failed,
        Some(reason),
        None,
        None,
        Some(deadline),
    );
    if let Some(tx) = first.take() {
        let _ = tx.send(FirstOutcome::Failed);
    }
    inner.runlog.info(
        "gateway",
        &format!("plugin '{id}' restarting in {} ms", delay),
    );
    tokio::select! {
        biased;
        () = wait_stop(stop) => {
            inner.set_state(id, PluginState::Stopped, None, None, None, None);
            false
        }
        () = inner.clock.sleep_until(monoagens_protocol::Time::from_millis(deadline)) => true,
    }
}

async fn run_once(
    inner: &Arc<SupInner>,
    config: &PluginConfig,
    stop: &mut watch::Receiver<bool>,
    first: &mut Option<oneshot::Sender<FirstOutcome>>,
) -> Attempt {
    let id = config.id.as_str();
    inner.set_state(id, PluginState::Starting, None, None, None, None);

    let mut child: Option<tokio::process::Child> = None;
    let mut stderr_task: Option<JoinHandle<()>> = None;
    let mut stdio = None;
    if let BodyTransportConfig::Stdio { command, args } = &config.transport {
        let envs = build_plugin_env(id, &inner.env_ctx);
        let mut cmd = Command::new(command);
        cmd.args(args)
            .envs(envs)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut c = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => return Attempt::Failed(format!("spawn '{command}' failed: {e}")),
        };
        let (Some(stdin), Some(stdout)) = (c.stdin.take(), c.stdout.take()) else {
            let _ = c.start_kill();
            return Attempt::Failed("child missing piped stdio".to_owned());
        };
        if let Some(stderr) = c.stderr.take() {
            let inner2 = Arc::clone(inner);
            let id2 = id.to_owned();
            stderr_task = Some(tokio::spawn(async move {
                let source = format!("plugin:{id2}");
                let mut reader = BufReader::new(stderr);
                let mut buf = Vec::new();
                loop {
                    buf.clear();
                    match reader.read_until(b'\n', &mut buf).await {
                        Ok(0) | Err(_) => break,
                        Ok(_) => {
                            let text = String::from_utf8_lossy(&buf);
                            let line = text.trim_end_matches(['\n', '\r']).to_owned();
                            inner2.runlog.info(&source, &line);
                            inner2.push_tail(&id2, inner2.runlog_mask(&line));
                        }
                    }
                }
            }));
        }
        child = Some(c);
        stdio = Some((stdin, stdout));
    }
    let pid = child.as_ref().and_then(tokio::process::Child::id);

    let connect = async {
        match (&config.transport, stdio.take()) {
            (BodyTransportConfig::Stdio { .. }, Some((stdin, stdout))) => {
                inner.handle.connect_ndjson_body(stdout, stdin, None).await
            }
            (BodyTransportConfig::UnixSocket { path }, _) => {
                inner.handle.connect_unix_body(path).await
            }
            (BodyTransportConfig::Stdio { .. }, None) => {
                Err(crate::waist2::BodyRpcError::Transport("missing stdio".to_owned()))
            }
        }
    };
    let body_id: BodyId = tokio::select! {
        biased;
        () = wait_stop(stop) => {
            kill_child(&mut child).await;
            return Attempt::Stopped;
        }
        res = connect => match res {
            Ok(b) => b,
            Err(err) => {
                kill_child(&mut child).await;
                drain_stderr(stderr_task).await;
                let mut reason = format!("connect failed: {err}");
                if let Some(last) = inner.last_stderr_line(id) {
                    reason.push_str(&format!(" ({last})"));
                }
                return Attempt::Failed(reason);
            }
        },
    };

    inner.set_state(
        id,
        PluginState::Running,
        None,
        pid,
        Some(body_id.as_str().to_owned()),
        None,
    );
    if let Some(tx) = first.take() {
        let _ = tx.send(FirstOutcome::Running);
    }

    loop {
        let wait_child = async {
            match child.as_mut() {
                Some(c) => c.wait().await,
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            biased;
            () = wait_stop(stop) => {
                graceful_stop(inner, &body_id, &mut child).await;
                drain_stderr(stderr_task).await;
                return Attempt::Stopped;
            }
            status = wait_child => {
                drain_stderr(stderr_task).await;
                let mut reason = match status {
                    Ok(st) => describe_exit(&st),
                    Err(e) => format!("wait failed: {e}"),
                };
                if let Some(last) = inner.last_stderr_line(id) {
                    reason.push_str(&format!(": {last}"));
                }
                return Attempt::Failed(reason);
            }
            () = tokio::time::sleep(BODY_POLL_INTERVAL) => {
                if inner.handle.router().is_body_connected(&body_id) {
                    continue;
                }
                // Body RPC gone. If the process is exiting on its own, report it as an exit.
                let exit = match child.as_mut() {
                    Some(c) => tokio::time::timeout(Duration::from_millis(300), c.wait())
                        .await
                        .ok(),
                    None => None,
                };
                let mut reason = match exit {
                    Some(Ok(st)) => describe_exit(&st),
                    _ => {
                        kill_child(&mut child).await;
                        "body rpc disconnected".to_owned()
                    }
                };
                drain_stderr(stderr_task).await;
                if let Some(last) = inner.last_stderr_line(id) {
                    reason.push_str(&format!(": {last}"));
                }
                return Attempt::Failed(reason);
            }
        }
    }
}

impl SupInner {
    fn runlog_mask(&self, line: &str) -> String {
        // Reuse the runtime log's masking so the buffered tail never holds secrets either.
        self.runlog.mask(line)
    }
}

async fn wait_stop(rx: &mut watch::Receiver<bool>) {
    // Also resolves when the supervisor (sender) is gone.
    let _ = rx.wait_for(|v| *v).await;
}

async fn kill_child(child: &mut Option<tokio::process::Child>) {
    if let Some(c) = child.as_mut() {
        let _ = c.start_kill();
        let _ = tokio::time::timeout(STOP_GRACE, c.wait()).await;
    }
}

async fn drain_stderr(task: Option<JoinHandle<()>>) {
    if let Some(t) = task {
        let _ = tokio::time::timeout(STDERR_DRAIN, t).await;
    }
}

async fn graceful_stop(
    inner: &Arc<SupInner>,
    body_id: &BodyId,
    child: &mut Option<tokio::process::Child>,
) {
    let _ = tokio::time::timeout(
        STOP_GRACE,
        inner
            .handle
            .router()
            .shutdown_body(body_id, "supervisor_stop", 1_500),
    )
    .await;
    if let Some(c) = child.as_mut()
        && tokio::time::timeout(STOP_GRACE, c.wait()).await.is_err()
    {
        let _ = c.start_kill();
        let _ = tokio::time::timeout(STOP_GRACE, c.wait()).await;
    }
    inner
        .handle
        .router()
        .handle_body_disconnect(body_id, "stopped by owner");
}
