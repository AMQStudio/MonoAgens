//! Owner-only admin socket `<data_dir>/gateway.sock`: NDJSON JSON-RPC 2.0 over a Unix socket.
//!
//! Only the data-dir lock holder creates it (a stale socket from a dead holder is replaced),
//! mode `0600`, and every connection is checked against the owner's uid with `SO_PEERCRED`
//! (`peer_cred`). Waist-2 has no reusable framing helper (its reader is inline in the router),
//! so framing here is the same plain one-JSON-object-per-line convention, with a line cap.
//!
//! Methods: `status`, `config.get`, `config.apply`, `config.reload`, `plugins.list`, `plugins.start`,
//! `plugins.stop`, `plugins.restart`, `plugins.subscribe`, `logs.subscribe`, `cues.subscribe`,
//! `turn.submit` (text, or a full client-built `percept`), `turn.cancel`, `events.since`,
//! `commands.list`, `commands.run`, `approvals.resolve`, `providers.list`, `models.discover`,
//! `providers.test`, `secrets.set` (write-only: the key is never echoed), `usage.get` and `body.attach`,
//! plus the governance methods of `crate::governance` (`approvals.list`, `people.list`, `links.*`,
//! `policy.get`, `events.query/explain/trace`, `activity.list`, `activity.subscribe`, `lab.*`),
//! and `preview.subscribe` / `preview.unsubscribe` (owner-only draft preview, never recorded).
//! `body.attach` turns
//! the connection itself into a Waist-2 body stream (the attached TUI's chat body); it is
//! refused while another body already owns the requested id or places. Anything else answers
//! `-32601`.

use std::collections::VecDeque;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use monoagens_protocol::{ApprovalResolveParams, BodyId, Percept};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{broadcast, mpsc};
use tokio::task::JoinHandle;

use crate::lock::DataDirLock;
use crate::owner::{AdminError, ERR_INVALID_PARAMS, ERR_METHOD_NOT_FOUND, GatewayOwner};

/// File name of the admin socket inside the data dir.
pub const ADMIN_SOCKET_FILENAME: &str = "gateway.sock";
const MAX_LINE_BYTES: u64 = 4 * 1024 * 1024;
const ERR_PARSE: i32 = -32700;
const ERR_INVALID_REQUEST: i32 = -32600;

/// Longest socket path a unix socket address can hold (`sun_path` is 108 bytes incl. the NUL).
const MAX_SOCKET_PATH_BYTES: usize = 107;

/// Returns the admin socket path of `data_dir`: `<data_dir>/gateway.sock`, or, when that would
/// exceed the unix socket path limit, `<runtime dir>/monoagens-<hash of data_dir>.sock`
/// (`$XDG_RUNTIME_DIR`, else `/run/user/<uid>`; never `/tmp`). The choice is a pure function of
/// the data dir and the environment, so the owner and every attaching client agree on it; the
/// owner reports the actual path as `socket` in `status`.
#[must_use]
pub fn admin_socket_path(data_dir: &Path) -> PathBuf {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| {
            use std::os::unix::fs::MetadataExt;
            let uid = std::fs::metadata("/proc/self").ok()?.uid();
            let dir = PathBuf::from(format!("/run/user/{uid}"));
            dir.is_dir().then_some(dir)
        });
    admin_socket_path_in(data_dir, runtime.as_deref())
}

/// [`admin_socket_path`] with an explicit runtime directory.
#[must_use]
pub fn admin_socket_path_in(data_dir: &Path, runtime_dir: Option<&Path>) -> PathBuf {
    use std::os::unix::ffi::OsStrExt;
    let primary = data_dir.join(ADMIN_SOCKET_FILENAME);
    if primary.as_os_str().as_bytes().len() <= MAX_SOCKET_PATH_BYTES {
        return primary;
    }
    let Some(runtime_dir) = runtime_dir else {
        return primary;
    };
    let hash = blake3::hash(data_dir.as_os_str().as_bytes()).to_hex();
    runtime_dir.join(format!("monoagens-{}.sock", &hash.as_str()[..16]))
}

/// Running admin socket server. Dropping it (or [`AdminServer::shutdown`]) removes the socket.
pub struct AdminServer {
    path: PathBuf,
    accept: JoinHandle<()>,
}

impl AdminServer {
    /// Binds `<data_dir>/gateway.sock` (replacing any stale socket; holding `_lock` proves no
    /// other gateway owns the directory), restricts it to mode `0600` and starts serving.
    pub async fn start(
        _lock: &DataDirLock,
        data_dir: &Path,
        owner: Arc<GatewayOwner>,
    ) -> std::io::Result<Self> {
        let path = admin_socket_path(data_dir);
        // Bind under a private temporary name, chmod, then rename into place so the final
        // path never exists with looser permissions.
        let tmp = path
            .parent()
            .unwrap_or(data_dir)
            .join(format!(".gw{}", std::process::id()));
        let _ = std::fs::remove_file(&tmp);
        let listener = UnixListener::bind(&tmp)?;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
        let uid = std::fs::metadata(&tmp)?.uid();
        let _ = std::fs::remove_file(&path);
        std::fs::rename(&tmp, &path)?;

        let accept = tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    break;
                };
                let allowed = stream.peer_cred().map(|c| c.uid() == uid).unwrap_or(false);
                if !allowed {
                    owner
                        .runlog()
                        .warn("gateway", "admin socket: refused connection from another uid");
                    continue;
                }
                let owner = Arc::clone(&owner);
                tokio::spawn(serve_connection(stream, owner));
            }
        });
        Ok(Self { path, accept })
    }

    /// Path of the socket.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Stops accepting and removes the socket file.
    pub fn shutdown(self) {
        drop(self);
    }
}

impl Drop for AdminServer {
    fn drop(&mut self) {
        self.accept.abort();
        let _ = std::fs::remove_file(&self.path);
    }
}

fn ok_line(id: &Value, result: Value) -> String {
    json!({ "jsonrpc": "2.0", "id": id, "result": result }).to_string()
}

fn err_line(id: &Value, code: i32, message: &str) -> String {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
        .to_string()
}

async fn read_limited_line<R: tokio::io::AsyncBufRead + Unpin>(
    reader: &mut R,
    buf: &mut String,
) -> std::io::Result<usize> {
    buf.clear();
    let mut limited = reader.take(MAX_LINE_BYTES);
    let n = limited.read_line(buf).await?;
    if n as u64 >= MAX_LINE_BYTES && !buf.ends_with('\n') {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "admin request line too long",
        ));
    }
    Ok(n)
}

async fn serve_connection(stream: UnixStream, owner: Arc<GatewayOwner>) {
    let (rd, mut wr) = stream.into_split();
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<String>();
    // The writer hands the write half back so `body.attach` can reuse the stream.
    let writer = tokio::spawn(async move {
        while let Some(line) = out_rx.recv().await {
            if wr.write_all(line.as_bytes()).await.is_err()
                || wr.write_all(b"\n").await.is_err()
                || wr.flush().await.is_err()
            {
                break;
            }
        }
        wr
    });

    let sub_ids = Arc::new(AtomicU64::new(1));
    let mut reader = BufReader::new(rd);
    let mut line = String::new();
    loop {
        match read_limited_line(&mut reader, &mut line).await {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let parsed: Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(e) => {
                let _ = out_tx.send(err_line(&Value::Null, ERR_PARSE, &format!("parse error: {e}")));
                continue;
            }
        };
        if parsed.get("method").and_then(Value::as_str) == Some("body.attach") {
            let rid = parsed.get("id").cloned().unwrap_or(Value::Null);
            match check_body_attach(&owner, parsed.get("params").unwrap_or(&Value::Null)) {
                Ok(body_id) => {
                    let _ = out_tx.send(ok_line(&rid, json!({ "ok": true, "body_id": body_id.as_str() })));
                    drop(out_tx);
                    let Ok(Ok(wr)) = tokio::time::timeout(Duration::from_secs(2), writer).await
                    else {
                        return;
                    };
                    match owner
                        .handle()
                        .connect_ndjson_body(reader.into_inner(), wr, Some(body_id.clone()))
                        .await
                    {
                        Ok(_) => owner
                            .runlog()
                            .info("gateway", &format!("body '{body_id}' attached over the admin socket")),
                        Err(e) => owner
                            .runlog()
                            .warn("gateway", &format!("body.attach for '{body_id}' failed: {e}")),
                    }
                    return;
                }
                Err(e) => {
                    let _ = out_tx.send(err_line(&rid, e.code, &e.message));
                    continue;
                }
            }
        }
        let owner = Arc::clone(&owner);
        let out = out_tx.clone();
        let subs = Arc::clone(&sub_ids);
        tokio::spawn(async move {
            handle_request(owner, out, subs, parsed).await;
        });
    }
    // Connection closed: stop the writer; subscription tasks end when `out_tx` closes.
    writer.abort();
}

/// Validates a `body.attach` request: the body id and every requested place must be free.
fn check_body_attach(owner: &GatewayOwner, params: &Value) -> Result<BodyId, AdminError> {
    let body_id = BodyId::new(
        params
            .get("body_id")
            .and_then(Value::as_str)
            .unwrap_or("body-attached"),
    );
    let places: Vec<String> = params
        .get("places")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect())
        .unwrap_or_default();
    let router = owner.handle().router();
    if router.is_body_connected(&body_id) {
        return Err(AdminError::server(format!(
            "body '{body_id}' is already connected (another TUI owns it)"
        )));
    }
    for other in router.connected_bodies() {
        for place in router.owned_places(&other) {
            if places.iter().any(|p| p == place.as_str()) {
                return Err(AdminError::server(format!(
                    "place '{place}' is already owned by body '{other}'"
                )));
            }
        }
    }
    Ok(body_id)
}

async fn handle_request(
    owner: Arc<GatewayOwner>,
    out: mpsc::UnboundedSender<String>,
    sub_ids: Arc<AtomicU64>,
    req: Value,
) {
    let id = req.get("id").cloned();
    let respond_id = id.clone().unwrap_or(Value::Null);
    let Some(method) = req.get("method").and_then(Value::as_str) else {
        if id.is_some() {
            let _ = out.send(err_line(&respond_id, ERR_INVALID_REQUEST, "missing method"));
        }
        return;
    };
    let params = req.get("params").cloned().unwrap_or(Value::Null);

    // Subscriptions: subscribe first (no gap), answer, then stream.
    if method == "logs.subscribe" {
        let backlog = params
            .get("backlog")
            .and_then(Value::as_u64)
            .map_or(0, |n| usize::try_from(n).unwrap_or(usize::MAX));
        let mut rx = owner.runlog().subscribe();
        let snapshot = owner.runlog().tail(backlog);
        let sub = sub_ids.fetch_add(1, Ordering::Relaxed);
        if id.is_some() {
            let _ = out.send(ok_line(&respond_id, json!({ "subscription": sub })));
        }
        for entry in snapshot {
            let _ = out.send(notification("logs.entry", &entry));
        }
        loop {
            tokio::select! {
                () = out.closed() => break,
                got = rx.recv() => match got {
                    Ok(entry) => {
                        if out.send(notification("logs.entry", &entry)).is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => break,
                },
            }
        }
        return;
    }
    if method == "cues.subscribe" {
        let mut rx = owner.handle().cues().subscribe();
        let sub = sub_ids.fetch_add(1, Ordering::Relaxed);
        if id.is_some() {
            let _ = out.send(ok_line(&respond_id, json!({ "subscription": sub })));
        }
        loop {
            tokio::select! {
                () = out.closed() => break,
                got = rx.recv() => match got {
                    Some(cue) => {
                        if out.send(notification("cues.event", &cue)).is_err() {
                            break;
                        }
                    }
                    None => break,
                },
            }
        }
        return;
    }
    // Owner-only draft preview (see `crate::preview`): drafts of the TUI's own turns, pushed as
    // `preview.event` notifications. Never recorded, never queued.
    if method == "preview.unsubscribe" {
        let sub = params.get("subscription").and_then(Value::as_u64);
        match sub {
            Some(sub) => {
                owner.handle().preview().unsubscribe(sub);
                if id.is_some() {
                    let _ = out.send(ok_line(&respond_id, json!({ "ok": true })));
                }
            }
            None => {
                if id.is_some() {
                    let _ = out.send(err_line(&respond_id, ERR_INVALID_PARAMS, "missing integer param 'subscription'"));
                }
            }
        }
        return;
    }
    if method == "preview.subscribe" {
        let mut sub = owner.handle().preview().subscribe();
        if id.is_some() {
            let _ = out.send(ok_line(&respond_id, json!({ "subscription": sub.id() })));
        }
        loop {
            tokio::select! {
                () = out.closed() => break,
                got = sub.recv() => match got {
                    Some(ev) => {
                        if out.send(notification("preview.event", &ev)).is_err() {
                            break;
                        }
                    }
                    None => break,
                },
            }
        }
        return;
    }
    if method == "plugins.subscribe" {
        let mut rx = owner.supervisor().subscribe();
        let sub = sub_ids.fetch_add(1, Ordering::Relaxed);
        if id.is_some() {
            let _ = out.send(ok_line(&respond_id, json!({ "subscription": sub })));
        }
        loop {
            tokio::select! {
                () = out.closed() => break,
                got = rx.recv() => match got {
                    Ok(ev) => {
                        if out.send(notification("plugins.state", &ev)).is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => break,
                },
            }
        }
        return;
    }

    if method == "activity.subscribe" {
        // Rows are derived from the recorded events (nothing is kept in memory across
        // restarts): remember what exists now, then push every row that is new or whose
        // outcome changed, polling the log once a second while the subscriber is connected.
        let mut sent: std::collections::BTreeMap<u64, String> = std::collections::BTreeMap::new();
        if let Ok(events) = owner.handle().read_events().await {
            for r in crate::governance::activity_rows(&events) {
                sent.insert(r.trace, r.outcome.as_str().to_owned());
            }
        }
        let sub = sub_ids.fetch_add(1, Ordering::Relaxed);
        if id.is_some() {
            let _ = out.send(ok_line(&respond_id, json!({ "subscription": sub })));
        }
        let mut tick = tokio::time::interval(Duration::from_millis(1000));
        loop {
            tokio::select! {
                () = out.closed() => break,
                _ = tick.tick() => {
                    let Ok(events) = owner.handle().read_events().await else { break };
                    for r in crate::governance::activity_rows(&events) {
                        if sent.get(&r.trace).is_none_or(|o| o != r.outcome.as_str()) {
                            sent.insert(r.trace, r.outcome.as_str().to_owned());
                            if out.send(notification("activity.row", &r)).is_err() {
                                return;
                            }
                        }
                    }
                }
            }
        }
        return;
    }

    let result = dispatch(&owner, method, &params).await;
    if id.is_none() {
        return; // JSON-RPC notification: no response.
    }
    let line = match result {
        Ok(v) => ok_line(&respond_id, v),
        Err(e) => err_line(&respond_id, e.code, &e.message),
    };
    let _ = out.send(line);
}

fn notification<T: serde::Serialize>(method: &str, params: &T) -> String {
    json!({ "jsonrpc": "2.0", "method": method, "params": params }).to_string()
}

fn param_str<'a>(params: &'a Value, key: &str) -> Result<&'a str, AdminError> {
    params
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| AdminError::invalid_params(format!("missing string param '{key}'")))
}

async fn dispatch(owner: &GatewayOwner, method: &str, params: &Value) -> Result<Value, AdminError> {
    match method {
        "status" => Ok(owner.status()),
        "config.get" => owner.config_get(),
        "config.apply" => {
            let patch = params.get("patch").unwrap_or(params);
            let report = owner.config_apply(patch).await?;
            Ok(json!({ "ok": true, "reloaded": report }))
        }
        "config.reload" => {
            let report = owner.config_reload().await?;
            Ok(json!({ "ok": true, "reloaded": report }))
        }
        "plugins.list" => Ok(owner.plugins_list()),
        "plugins.start" => {
            owner.plugins_start(param_str(params, "id")?).await?;
            Ok(json!({ "ok": true }))
        }
        "plugins.stop" => {
            owner.plugins_stop(param_str(params, "id")?).await?;
            Ok(json!({ "ok": true }))
        }
        "plugins.restart" => {
            owner.plugins_restart(param_str(params, "id")?).await?;
            Ok(json!({ "ok": true }))
        }
        "turn.submit" if params.get("percept").is_some() => {
            let percept: Percept = serde_json::from_value(params["percept"].clone())
                .map_err(|e| AdminError::invalid_params(format!("invalid percept: {e}")))?;
            owner.turn_submit_percept(percept).await
        }
        "events.since" => {
            let from = params
                .get("from")
                .and_then(Value::as_u64)
                .map_or(0, |n| usize::try_from(n).unwrap_or(usize::MAX));
            owner.events_since(from).await
        }
        "commands.list" => Ok(owner.commands_list()),
        "commands.run" => Ok(owner.command_run(
            param_str(params, "text")?,
            params.get("actor").and_then(Value::as_str),
            params.get("place").and_then(Value::as_str),
        )),
        "approvals.resolve" => {
            let p: ApprovalResolveParams = serde_json::from_value(params.clone())
                .map_err(|e| AdminError::invalid_params(format!("invalid approval params: {e}")))?;
            owner.approvals_resolve(p).await
        }
        "turn.submit" => {
            let text = param_str(params, "text")?;
            owner
                .turn_submit(
                    text,
                    params.get("actor").and_then(Value::as_str),
                    params.get("place").and_then(Value::as_str),
                    params.get("channel").and_then(Value::as_str),
                )
                .await
        }
        "turn.cancel" => Ok(owner.turn_cancel()),
        "providers.list" => owner.providers_list(),
        "models.discover" => {
            owner
                .models_discover(
                    param_str(params, "provider")?,
                    params.get("refresh").and_then(Value::as_bool).unwrap_or(false),
                )
                .await
        }
        "providers.test" => {
            owner
                .providers_test(
                    param_str(params, "provider")?,
                    params.get("api_key").and_then(Value::as_str),
                )
                .await
        }
        "secrets.set" if params.get("env").is_some() || params.get("key").is_some() => {
            let (section, name) = match params.get("env") {
                Some(_) => ("env", param_str(params, "env")?),
                None => ("top", param_str(params, "key")?),
            };
            owner.secrets_set_named(section, name, params.get("value").and_then(Value::as_str))
        }
        "secrets.set" => {
            owner
                .secrets_set(param_str(params, "provider")?, param_str(params, "api_key")?)
                .await
        }
        "usage.get" => Ok(owner.usage_get()),
        "capabilities.list" => Ok(owner.capabilities_list()),
        other if crate::governance::GOVERNANCE_METHODS.contains(&other) => {
            crate::governance::governance_call(owner, other, params).await
        }
        other => Err(AdminError::new(
            ERR_METHOD_NOT_FOUND,
            format!("method not found: {other}"),
        )),
    }
}

/// Minimal admin client (NDJSON JSON-RPC over the owner socket), used by tests and by the
/// attach path of later changes.
pub struct AdminClient {
    reader: BufReader<OwnedReadHalf>,
    writer: OwnedWriteHalf,
    next_id: i64,
    pending_notifications: VecDeque<Value>,
}

impl AdminClient {
    /// Connects to `<data_dir>/gateway.sock`.
    pub async fn connect(data_dir: &Path) -> std::io::Result<Self> {
        Self::connect_path(&admin_socket_path(data_dir)).await
    }

    /// Connects to an explicit socket path.
    pub async fn connect_path(path: &Path) -> std::io::Result<Self> {
        let stream = UnixStream::connect(path).await?;
        let (rd, wr) = stream.into_split();
        Ok(Self {
            reader: BufReader::new(rd),
            writer: wr,
            next_id: 1,
            pending_notifications: VecDeque::new(),
        })
    }

    async fn read_value(&mut self) -> std::io::Result<Value> {
        let mut line = String::new();
        loop {
            line.clear();
            if read_limited_line(&mut self.reader, &mut line).await? == 0 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "admin socket closed",
                ));
            }
            if !line.trim().is_empty() {
                return serde_json::from_str(line.trim())
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e));
            }
        }
    }

    /// Calls `method` and returns its result, or the JSON-RPC error as an [`AdminError`].
    pub async fn call(&mut self, method: &str, params: Value) -> Result<Value, AdminError> {
        let id = self.next_id;
        self.next_id += 1;
        let req = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        let io_err = |e: std::io::Error| AdminError::server(format!("admin io: {e}"));
        self.writer
            .write_all(format!("{req}\n").as_bytes())
            .await
            .map_err(io_err)?;
        self.writer.flush().await.map_err(io_err)?;
        loop {
            let v = self.read_value().await.map_err(io_err)?;
            if v.get("id") == Some(&json!(id)) {
                if let Some(err) = v.get("error") {
                    return Err(AdminError::new(
                        err.get("code")
                            .and_then(Value::as_i64)
                            .and_then(|c| i32::try_from(c).ok())
                            .unwrap_or(ERR_INVALID_PARAMS),
                        err.get("message").and_then(Value::as_str).unwrap_or(""),
                    ));
                }
                return Ok(v.get("result").cloned().unwrap_or(Value::Null));
            }
            if v.get("method").is_some() {
                self.pending_notifications.push_back(v);
            }
        }
    }

    /// Next pushed notification (`logs.entry`, `plugins.state`).
    pub async fn next_notification(&mut self) -> std::io::Result<Value> {
        if let Some(v) = self.pending_notifications.pop_front() {
            return Ok(v);
        }
        loop {
            let v = self.read_value().await?;
            if v.get("method").is_some() {
                return Ok(v);
            }
        }
    }
}
