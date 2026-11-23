//! Standalone `body-whatsapp` Waist-2 NDJSON `stdio` body and interactive `pair` CLI.

#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::env;
use std::fs;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use body_whatsapp::{
    BlobStager, DEFAULT_BODY_ID, InboundAction, InboundTranslator, LiveWhatsAppTransport,
    MemoryBlobLookup, WhatsAppBody, WhatsAppPluginConfig, WhatsAppTransport,
    resolve_state_dir_from_env,
};
use crossterm::event::{self, Event as CrosstermEvent, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::tty::IsTty;
use monoagens_body_kit::PersistentBodyRuntime;
use monoagens_body_sdk::{
    BlobPutResult, BodyId, InboundFrame, OutboundBuilder, RpcId, Waist2Response,
    parse_inbound_line,
};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::{mpsc, oneshot};
use whatsapp_rust::bot::Bot;
use whatsapp_rust::pair_code::PairCodeOptions;
use whatsapp_rust::store::SqliteStore;
use whatsapp_rust::wacore::types::events::{ConnectFailureReason, Event, EventKind};

const DEFAULT_PAIR_TIMEOUT_SECS: u64 = 120;
const EXIT_CODE_TIMEOUT: i32 = 124;
const EXIT_CODE_CANCELLED: i32 = 130;

#[derive(Debug, Clone, PartialEq, Eq)]
enum CliCommand {
    Pair {
        db_path: PathBuf,
        phone: Option<String>,
        custom_code: Option<String>,
        timeout_secs: u64,
        fresh: bool,
    },
    Serve {
        db_path: PathBuf,
        body_id: String,
        force_live: bool,
        force_offline: bool,
    },
}

fn parse_cli_args(args: &[String], config: &WhatsAppPluginConfig) -> Result<CliCommand, String> {
    let mut iter = args.iter().peekable();
    let is_pair = iter.peek().is_some_and(|a| a.as_str() == "pair");
    if is_pair {
        iter.next();
    }

    let mut db_path = config.resolve_db_path();
    let mut phone: Option<String> = None;
    let mut custom_code: Option<String> = None;
    let mut timeout_secs = DEFAULT_PAIR_TIMEOUT_SECS;
    let mut fresh = false;
    let mut body_id = DEFAULT_BODY_ID.to_owned();
    let mut force_live = false;
    let mut force_offline = false;

    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--db" => {
                let val = iter
                    .next()
                    .ok_or_else(|| "--db requires a path argument".to_owned())?;
                db_path = PathBuf::from(val);
            }
            "--phone" | "-p" => {
                let val = iter
                    .next()
                    .ok_or_else(|| "--phone requires a phone number argument".to_owned())?;
                phone = Some(val.clone());
            }
            "--code" | "-c" => {
                let val = iter
                    .next()
                    .ok_or_else(|| "--code requires a custom pairing code".to_owned())?;
                custom_code = Some(val.clone());
            }
            "--timeout-secs" => {
                let val = iter
                    .next()
                    .ok_or_else(|| "--timeout-secs requires a positive integer".to_owned())?;
                let parsed: u64 = val
                    .parse()
                    .map_err(|_| format!("invalid --timeout-secs value '{val}'"))?;
                if parsed == 0 {
                    return Err("--timeout-secs must be greater than 0".to_owned());
                }
                timeout_secs = parsed;
            }
            "--fresh" => fresh = true,
            "--body-id" => {
                let val = iter
                    .next()
                    .ok_or_else(|| "--body-id requires an identifier".to_owned())?;
                body_id = val.clone();
            }
            "--live" => force_live = true,
            "--offline" => force_offline = true,
            "--help" | "-h" => {
                eprintln!(
                    "Usage:\n  \
                     body-whatsapp [--db <path>] [--live] [--offline] [--body-id <id>]\n  \
                     body-whatsapp pair [--db <path>] [--phone <number>] [--code <8char>] [--timeout-secs <secs>] [--fresh]"
                );
                std::process::exit(0);
            }
            other => {
                return Err(format!("unknown argument '{other}'"));
            }
        }
    }

    if is_pair {
        Ok(CliCommand::Pair {
            db_path,
            phone,
            custom_code,
            timeout_secs,
            fresh,
        })
    } else {
        Ok(CliCommand::Serve {
            db_path,
            body_id,
            force_live,
            force_offline,
        })
    }
}

/// Formats a QR payload into high-contrast black-on-white ANSI modules with a
/// mandatory quiet zone and raw-mode-safe `\r\n` line endings.
fn format_qr_for_terminal(code: &str, timeout: Duration) -> Result<String, String> {
    let qr = qrcode::QrCode::new(code.as_bytes())
        .map_err(|e| format!("failed to encode QR code: {e}"))?;
    let art = qr
        .render::<qrcode::render::unicode::Dense1x2>()
        .quiet_zone(true)
        .dark_color(qrcode::render::unicode::Dense1x2::Dark)
        .light_color(qrcode::render::unicode::Dense1x2::Light)
        .build();

    let mut out = String::new();
    out.push_str(&format!(
        "=== WhatsApp Link QR (valid for {}s) ===\r\n",
        timeout.as_secs()
    ));
    for line in art.lines() {
        out.push_str("\x1b[30;47m");
        out.push_str(line);
        out.push_str("\x1b[0m\r\n");
    }
    out.push_str("===========================================\r\n");
    Ok(out)
}

fn render_qr_to_stderr(code: &str, timeout: Duration) {
    render_qr_to_stderr_with_options(code, timeout, None, false);
}

fn render_qr_to_stderr_with_options(
    code: &str,
    timeout: Duration,
    banner: Option<&str>,
    clear_previous: bool,
) {
    let mut err = io::stderr().lock();
    if clear_previous {
        let _ = write!(err, "\x1b[2J\x1b[H");
        if let Some(header) = banner {
            let _ = write!(err, "{header}\r\n\r\n");
        }
    }
    match format_qr_for_terminal(code, timeout) {
        Ok(block) => {
            let _ = write!(err, "{block}");
        }
        Err(e) => {
            let _ = write!(err, "{e}\r\n");
        }
    }
    let _ = err.flush();
}

fn render_pair_code_to_stderr(code: &str, timeout: Duration) {
    let mut err = io::stderr().lock();
    let _ = write!(
        err,
        "\r\n=== WhatsApp Pair Code (valid for {}s) ===\r\n  \
         WhatsApp > Linked Devices > Link a Device > Link with phone number instead\r\n  \
         >>> {code} <<<\r\n\
         ===============================================\r\n\r\n",
        timeout.as_secs()
    );
    let _ = err.flush();
}

fn pair_eprintln(msg: &str) {
    let mut err = io::stderr().lock();
    let _ = write!(err, "{msg}\r\n");
    let _ = err.flush();
}

fn sqlite_sidecar_path(db_path: &Path, suffix: &str) -> PathBuf {
    let mut os = db_path.as_os_str().to_os_string();
    os.push(suffix);
    PathBuf::from(os)
}

/// Removes a SQLite database file and its `-wal` / `-shm` sidecar files if present.
fn remove_sqlite_files(db_path: &Path) -> Result<(), String> {
    let wal_path = sqlite_sidecar_path(db_path, "-wal");
    let shm_path = sqlite_sidecar_path(db_path, "-shm");
    for path in [db_path, wal_path.as_path(), shm_path.as_path()] {
        match fs::remove_file(path) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => {
                return Err(format!("failed to remove '{}': {e}", path.display()));
            }
        }
    }
    Ok(())
}

/// RAII guard that enables terminal raw mode on TTY stdin and guarantees restoration
/// on drop or panic.
struct RawModeGuard {
    active: bool,
}

impl RawModeGuard {
    fn enter() -> Self {
        let active = io::stdin().is_tty() && crossterm::terminal::enable_raw_mode().is_ok();
        Self { active }
    }

    fn is_active(&self) -> bool {
        self.active
    }

    fn restore(&mut self) {
        if self.active {
            let _ = crossterm::terminal::disable_raw_mode();
            self.active = false;
        }
    }
}

impl Drop for RawModeGuard {
    fn drop(&mut self) {
        self.restore();
    }
}

fn is_cancel_key(key: &crossterm::event::KeyEvent) -> bool {
    if key.kind == KeyEventKind::Release {
        return false;
    }
    match key.code {
        KeyCode::Esc => true,
        KeyCode::Char('q' | 'Q') if !key.modifiers.contains(KeyModifiers::CONTROL) => true,
        KeyCode::Char('c' | 'C') if key.modifiers.contains(KeyModifiers::CONTROL) => true,
        _ => false,
    }
}

fn spawn_key_cancel_watcher(
    cancel_tx: mpsc::Sender<()>,
    stop_flag: Arc<AtomicBool>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        while !stop_flag.load(Ordering::Relaxed) {
            match event::poll(Duration::from_millis(100)) {
                Ok(true) => {
                    if let Ok(CrosstermEvent::Key(key)) = event::read()
                        && is_cancel_key(&key)
                    {
                        let _ = cancel_tx.try_send(());
                        break;
                    }
                }
                Ok(false) => {}
                Err(_) => break,
            }
        }
    })
}

enum PairSignal {
    Connected,
    Exhausted,
    FatalError(String),
    LoggedOutOrPairError(String),
}

enum PairOutcome {
    Connected,
    Cancelled,
    TimedOut,
    Failed(String),
}

fn now_epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

fn ensure_parent_dir(path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)
            .map_err(|e| format!("failed to create directory '{}': {e}", parent.display()))?;
    }
    Ok(())
}

async fn run_pair_command(
    db_path: PathBuf,
    phone: Option<String>,
    custom_code: Option<String>,
    timeout_secs: u64,
    fresh: bool,
) -> Result<PairOutcome, String> {
    ensure_parent_dir(&db_path)?;
    if fresh {
        remove_sqlite_files(&db_path)?;
    }
    let existed_before_pair = db_path.exists();

    let db_str = db_path
        .to_str()
        .ok_or_else(|| format!("non-UTF8 db path: {}", db_path.display()))?;
    let store = SqliteStore::new(db_str)
        .await
        .map_err(|e| format!("failed to open WhatsApp SQLite store '{db_str}': {e}"))?;

    let (signal_tx, mut signal_rx) = mpsc::channel::<PairSignal>(16);
    let (cancel_tx, mut cancel_rx) = mpsc::channel::<()>(1);
    let qr_rendered_once = Arc::new(AtomicBool::new(false));
    let pair_succeeded = Arc::new(AtomicBool::new(false));
    let using_phone = phone.is_some();

    let banner = format!(
        "WhatsApp Pairing — scan QR from WhatsApp > Linked Devices (press Esc, q, or Ctrl+C to cancel; timeout {timeout_secs}s)"
    );
    let banner_qr = banner.clone();
    let qr_rendered_cb = Arc::clone(&qr_rendered_once);
    let signal_tx_ev = signal_tx.clone();
    let pair_succeeded_ev = Arc::clone(&pair_succeeded);

    let mut builder = Bot::builder()
        .with_backend(store)
        .skip_history_sync()
        .on_pair_code(|code, timeout| async move {
            render_pair_code_to_stderr(&code, timeout);
        })
        .on_pair_code_error(|err, _client| async move {
            pair_eprintln(&format!("❌ WhatsApp pair code error: {}", err.error));
        })
        .on_event(move |event, _client| {
            let signal_tx = signal_tx_ev.clone();
            let pair_succeeded = Arc::clone(&pair_succeeded_ev);
            async move {
                match &*event {
                    Event::PairSuccess(_) => {
                        pair_succeeded.store(true, Ordering::Release);
                        pair_eprintln(
                            "✅ Device linked! Completing WhatsApp handshake & saving session...",
                        );
                    }
                    Event::Connected(_) => {
                        pair_succeeded.store(true, Ordering::Release);
                        let _ = signal_tx.send(PairSignal::Connected).await;
                    }
                    Event::PairError(err) => {
                        pair_eprintln(&format!("❌ WhatsApp pairing error: {:?}", err));
                        let _ = signal_tx
                            .send(PairSignal::LoggedOutOrPairError(format!(
                                "WhatsApp pairing error: {}",
                                err.error
                            )))
                            .await;
                    }
                    Event::PairPasskeyRequest(_) => {
                        let msg = "❌ WhatsApp requested Passkey/WebAuthn verification ('SHORTCAKE_PASSKEY') for this account. Try pairing via phone number (`monoagens pair whatsapp --phone <number>`) or wait and retry.";
                        pair_eprintln(msg);
                        let _ = signal_tx
                            .send(PairSignal::FatalError(
                                "WhatsApp requested Passkey/WebAuthn verification ('SHORTCAKE_PASSKEY').".to_owned(),
                            ))
                            .await;
                    }
                    Event::PairPasskeyError(err) => {
                        pair_eprintln(&format!(
                            "❌ WhatsApp requested Passkey/WebAuthn verification ('SHORTCAKE_PASSKEY') for this account ({}). Try pairing via phone number (`monoagens pair whatsapp --phone <number>`) or wait and retry.",
                            err.error
                        ));
                        let _ = signal_tx
                            .send(PairSignal::FatalError(format!(
                                "WhatsApp Passkey/WebAuthn verification failed ('SHORTCAKE_PASSKEY'): {}",
                                err.error
                            )))
                            .await;
                    }
                    Event::ConnectFailure(cf) => {
                        pair_eprintln(&format!("❌ WhatsApp connection failed: {:?}", cf));
                        if cf.reason.is_logged_out()
                            || matches!(
                                cf.reason,
                                ConnectFailureReason::TempBanned
                                    | ConnectFailureReason::AccountLocked
                            )
                        {
                            let _ = signal_tx
                                .send(PairSignal::LoggedOutOrPairError(format!(
                                    "WhatsApp connection failed: {:?}",
                                    cf.reason
                                )))
                                .await;
                        }
                    }
                    Event::PairingQrCodesExhausted(exhausted) => {
                        if !using_phone || exhausted.disconnected {
                            pair_eprintln("⏱️ QR codes expired without scanning.");
                            let _ = signal_tx.send(PairSignal::Exhausted).await;
                        }
                    }
                    Event::LoggedOut(info) => {
                        pair_eprintln(&format!("❌ Logged out during pairing: {:?}", info));
                        let _ = signal_tx
                            .send(PairSignal::LoggedOutOrPairError(format!(
                                "Logged out during pairing: {:?}",
                                info.reason
                            )))
                            .await;
                    }
                    _ => {}
                }
            }
        });

    if !using_phone {
        builder = builder.on_qr_code(move |code, timeout| {
            let banner = banner_qr.clone();
            let rendered_once = Arc::clone(&qr_rendered_cb);
            async move {
                let is_refresh = rendered_once.swap(true, Ordering::AcqRel);
                let clear = is_refresh && io::stderr().is_tty();
                render_qr_to_stderr_with_options(&code, timeout, Some(&banner), clear);
            }
        });
    }

    if let Some(phone_number) = phone {
        builder = builder.with_pair_code(PairCodeOptions {
            phone_number,
            custom_code,
            ..Default::default()
        });
    }

    let bot = builder
        .build()
        .await
        .map_err(|e| format!("failed to build WhatsApp bot: {e}"))?;

    let mut raw_guard = RawModeGuard::enter();
    pair_eprintln(&banner);

    let watcher_stop = Arc::new(AtomicBool::new(false));
    let watcher_handle = if raw_guard.is_active() {
        Some(spawn_key_cancel_watcher(
            cancel_tx,
            Arc::clone(&watcher_stop),
        ))
    } else {
        drop(cancel_tx);
        None
    };

    let mut handle = bot.spawn();
    let deadline = tokio::time::sleep(Duration::from_secs(timeout_secs));
    tokio::pin!(deadline);

    let mut cleanup_db = false;
    let outcome = tokio::select! {
        _ = &mut handle => {
            if !existed_before_pair && !pair_succeeded.load(Ordering::Acquire) {
                cleanup_db = true;
            }
            PairOutcome::Failed("WhatsApp pairing session ended unexpectedly.".to_owned())
        }
        sig = signal_rx.recv() => {
            match sig {
                Some(PairSignal::Connected) => {
                    // Allow persistence manager to flush Signal/ADV keys cleanly to SQLite.
                    tokio::time::sleep(Duration::from_secs(3)).await;
                    handle.shutdown().await;
                    PairOutcome::Connected
                }
                Some(PairSignal::Exhausted) => {
                    handle.shutdown().await;
                    if !existed_before_pair && !pair_succeeded.load(Ordering::Acquire) {
                        cleanup_db = true;
                    }
                    PairOutcome::TimedOut
                }
                Some(PairSignal::LoggedOutOrPairError(msg)) => {
                    handle.shutdown().await;
                    cleanup_db = true;
                    PairOutcome::Failed(msg)
                }
                Some(PairSignal::FatalError(msg)) => {
                    handle.shutdown().await;
                    if !existed_before_pair && !pair_succeeded.load(Ordering::Acquire) {
                        cleanup_db = true;
                    }
                    PairOutcome::Failed(msg)
                }
                None => {
                    handle.shutdown().await;
                    if !existed_before_pair && !pair_succeeded.load(Ordering::Acquire) {
                        cleanup_db = true;
                    }
                    PairOutcome::Failed("WhatsApp pairing event channel closed.".to_owned())
                }
            }
        }
        _ = &mut deadline => {
            handle.shutdown().await;
            if !existed_before_pair && !pair_succeeded.load(Ordering::Acquire) {
                cleanup_db = true;
            }
            PairOutcome::TimedOut
        }
        _ = cancel_rx.recv() => {
            handle.shutdown().await;
            if !existed_before_pair && !pair_succeeded.load(Ordering::Acquire) {
                cleanup_db = true;
            }
            PairOutcome::Cancelled
        }
        _ = tokio::signal::ctrl_c() => {
            handle.shutdown().await;
            if !existed_before_pair && !pair_succeeded.load(Ordering::Acquire) {
                cleanup_db = true;
            }
            PairOutcome::Cancelled
        }
    };

    watcher_stop.store(true, Ordering::Relaxed);
    if let Some(jh) = watcher_handle {
        let _ = jh.join();
    }
    raw_guard.restore();

    if cleanup_db {
        let _ = remove_sqlite_files(&db_path);
    }

    match &outcome {
        PairOutcome::Connected => {
            eprintln!("✅ Session saved to {}", db_path.display());
        }
        PairOutcome::Cancelled => {
            eprintln!("Pairing cancelled by user.");
        }
        PairOutcome::TimedOut => {
            eprintln!(
                "⏱️ Pairing timed out after {timeout_secs}s without connecting. Retry (`monoagens pair whatsapp --fresh`) or try phone-number pairing (`monoagens pair whatsapp --phone <country_code+number>`)."
            );
        }
        PairOutcome::Failed(_) => {}
    }

    Ok(outcome)
}

/// Captures any inline `blob/put` payloads into `blob_lookup` before `PersistentBodyRuntime`
/// handles them so outbound `chat.send_media` / `chat.send_sticker` can resolve `blob_ref`.
fn mirror_blob_put_if_present(line: &str, blob_lookup: &MemoryBlobLookup) {
    let Ok(val) = serde_json::from_str::<Value>(line) else {
        return;
    };
    if val.get("method").and_then(Value::as_str) != Some("blob/put") {
        return;
    }
    let Ok(params) = serde_json::from_value::<monoagens_body_sdk::BlobPutParams>(
        val.get("params").cloned().unwrap_or(Value::Null),
    ) else {
        return;
    };
    if let Ok(digest) = monoagens_body_sdk::verify_blob_put(&params)
        && let Some(inline) = params.inline_bytes
    {
        let blob_ref = format!("blob://sha256/{digest}");
        blob_lookup.insert(blob_ref, inline.0);
    }
}

fn run_offline_stdio(body_id: String, config: WhatsAppPluginConfig) -> Result<(), String> {
    let blob_lookup = MemoryBlobLookup::default();
    let mut runtime = PersistentBodyRuntime::new(resolve_state_dir_from_env());
    let mut body = WhatsAppBody::offline(BodyId::new(body_id))
        .with_config(config)
        .with_blob_lookup(blob_lookup.clone());

    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut writer = stdout.lock();
    for line in stdin.lock().lines() {
        let line = line.map_err(|e| format!("stdin read error: {e}"))?;
        mirror_blob_put_if_present(&line, &blob_lookup);
        if let Some(out) = runtime.dispatch_line(&line, &mut body) {
            writeln!(writer, "{out}").map_err(|e| format!("stdout write error: {e}"))?;
            writer
                .flush()
                .map_err(|e| format!("stdout flush error: {e}"))?;
        }
    }
    Ok(())
}

type PendingOutbound = Arc<Mutex<HashMap<RpcId, oneshot::Sender<Waist2Response<Value>>>>>;

struct StdioBlobStager {
    rt: tokio::runtime::Handle,
    outbound: Arc<Mutex<OutboundBuilder>>,
    pending: PendingOutbound,
    stdout_tx: mpsc::Sender<String>,
}

impl BlobStager for StdioBlobStager {
    fn stage_blob(
        &mut self,
        bytes: &[u8],
        content_type: Option<&str>,
        scope_id: Option<&str>,
    ) -> Result<BlobPutResult, String> {
        let req = {
            let mut ob = self
                .outbound
                .lock()
                .map_err(|_| "outbound builder lock poisoned".to_owned())?;
            ob.blob_put(bytes, content_type, scope_id)
        };
        let req_id = req
            .id
            .clone()
            .ok_or_else(|| "missing rpc id on blob/put".to_owned())?;
        let wire = serde_json::to_string(&req)
            .map_err(|e| format!("failed to serialize blob/put: {e}"))?;

        let (tx, rx) = oneshot::channel();
        {
            let mut map = self
                .pending
                .lock()
                .map_err(|_| "pending map lock poisoned".to_owned())?;
            map.insert(req_id.clone(), tx);
        }

        if self.stdout_tx.blocking_send(wire).is_err() {
            if let Ok(mut map) = self.pending.lock() {
                map.remove(&req_id);
            }
            return Err("stdout channel closed while staging blob".to_owned());
        }

        let resp = self.rt.block_on(async {
            match tokio::time::timeout(Duration::from_secs(10), rx).await {
                Ok(Ok(r)) => Ok(r),
                Ok(Err(_)) => Err("blob/put response channel dropped".to_owned()),
                Err(_) => Err("timed out waiting for Gateway blob/put response".to_owned()),
            }
        });

        let resp = match resp {
            Ok(r) => r,
            Err(err) => {
                if let Ok(mut map) = self.pending.lock() {
                    map.remove(&req_id);
                }
                return Err(err);
            }
        };

        if let Some(err) = resp.error {
            return Err(format!(
                "Gateway rejected blob/put [{}]: {}",
                err.code, err.message
            ));
        }
        let val = resp
            .result
            .ok_or_else(|| "Gateway blob/put response missing result".to_owned())?;
        serde_json::from_value::<BlobPutResult>(val)
            .map_err(|e| format!("invalid Gateway BlobPutResult: {e}"))
    }
}

async fn run_live_stdio(
    db_path: PathBuf,
    body_id: String,
    config: WhatsAppPluginConfig,
) -> Result<(), String> {
    ensure_parent_dir(&db_path)?;
    let db_str = db_path
        .to_str()
        .ok_or_else(|| format!("non-UTF8 db path: {}", db_path.display()))?;
    let store = SqliteStore::new(db_str)
        .await
        .map_err(|e| format!("failed to open WhatsApp SQLite store '{db_str}': {e}"))?;

    let boot_ts_ms = now_epoch_ms();
    let (wa_event_tx, mut wa_event_rx) = mpsc::channel::<Arc<Event>>(256);

    let bot = Bot::builder()
        .with_backend(store)
        .skip_history_sync()
        .on_qr_code(|code, timeout| async move {
            render_qr_to_stderr(&code, timeout);
        })
        .on_pair_code(|code, timeout| async move {
            render_pair_code_to_stderr(&code, timeout);
        })
        .on_event_for(
            &[EventKind::Messages, EventKind::Connected, EventKind::LoggedOut],
            move |event, _client| {
                let tx = wa_event_tx.clone();
                async move {
                    let _ = tx.send(event).await;
                }
            },
        )
        .build()
        .await
        .map_err(|e| format!("failed to build WhatsApp bot: {e}"))?;

    let bot_handle = bot.spawn();
    let rt_handle = tokio::runtime::Handle::current();
    let transport = LiveWhatsAppTransport::new(bot_handle.client(), rt_handle.clone());

    let blob_lookup = MemoryBlobLookup::default();
    let grace_ms = config.history_grace_secs.saturating_mul(1_000);
    let auto_mark_read = config.auto_mark_read;
    let body = Arc::new(Mutex::new(
        WhatsAppBody::new(BodyId::new(body_id), transport)
            .with_config(config)
            .with_blob_lookup(blob_lookup.clone()),
    ));
    let runtime = Arc::new(Mutex::new(PersistentBodyRuntime::new(
        resolve_state_dir_from_env(),
    )));

    let (stdout_tx, mut stdout_rx) = mpsc::channel::<String>(256);
    let stdout_task = tokio::task::spawn_blocking(move || {
        let stdout = io::stdout();
        let mut lock = stdout.lock();
        while let Some(line) = stdout_rx.blocking_recv() {
            if writeln!(lock, "{line}").is_err() || lock.flush().is_err() {
                break;
            }
        }
    });

    let pending_outbound: PendingOutbound = Arc::new(Mutex::new(HashMap::new()));
    let outbound_builder = Arc::new(Mutex::new(OutboundBuilder::new("body-whatsapp")));

    // Dedicated blocking RPC worker ensures sequential execution without blocking stdin reads.
    let (rpc_tx, mut rpc_rx) = mpsc::channel::<String>(128);
    let (shutdown_tx, mut shutdown_rx) = oneshot::channel::<()>();
    let rpc_body = Arc::clone(&body);
    let rpc_runtime = Arc::clone(&runtime);
    let rpc_stdout = stdout_tx.clone();
    let rpc_blob_lookup = blob_lookup.clone();
    let rpc_worker = tokio::task::spawn_blocking(move || {
        let mut shutdown_tx = Some(shutdown_tx);
        while let Some(line) = rpc_rx.blocking_recv() {
            mirror_blob_put_if_present(&line, &rpc_blob_lookup);
            let is_shutdown = serde_json::from_str::<Value>(&line)
                .ok()
                .and_then(|v| v.get("method").and_then(Value::as_str).map(ToOwned::to_owned))
                .is_some_and(|m| m == "body/shutdown");

            let out = {
                let mut rt_guard = rpc_runtime.lock().expect("runtime lock");
                let mut body_guard = rpc_body.lock().expect("body lock");
                body_guard.set_now_ms(now_epoch_ms());
                rt_guard.dispatch_line(&line, &mut *body_guard)
            };

            if let Some(out_line) = out
                && rpc_stdout.blocking_send(out_line).is_err()
            {
                break;
            }

            if is_shutdown {
                if let Some(tx) = shutdown_tx.take() {
                    let _ = tx.send(());
                }
                break;
            }
        }
    });

    // Dedicated inbound WhatsApp event translator task.
    let ev_body = Arc::clone(&body);
    let ev_stdout = stdout_tx.clone();
    let ev_pending = Arc::clone(&pending_outbound);
    let ev_outbound = Arc::clone(&outbound_builder);
    let ev_rt = rt_handle.clone();
    let event_worker = tokio::task::spawn_blocking(move || {
        let mut translator = InboundTranslator::new(boot_ts_ms).with_grace_ms(grace_ms);
        let mut stager = StdioBlobStager {
            rt: ev_rt,
            outbound: Arc::clone(&ev_outbound),
            pending: Arc::clone(&ev_pending),
            stdout_tx: ev_stdout.clone(),
        };
        let mut seq: u64 = 0;

        while let Some(event) = wa_event_rx.blocking_recv() {
            match &*event {
                Event::Connected(_) => {
                    eprintln!("WhatsApp live session connected.");
                }
                Event::LoggedOut(info) => {
                    eprintln!("WhatsApp session logged out: {:?}", info.reason);
                }
                Event::Messages(batch) => {
                    for inbound in batch.iter() {
                        seq = seq.saturating_add(1);
                        let key = format!("wa-inbound-{boot_ts_ms}-{seq}");
                        let action_opt = {
                            let mut body_guard = ev_body.lock().expect("body lock");
                            let action = {
                                let (transport, echo_reg, pending_approvals) =
                                    body_guard.inbound_context_mut();
                                translator.translate_inbound_action(
                                    inbound,
                                    transport,
                                    echo_reg,
                                    pending_approvals,
                                    &mut stager,
                                    key,
                                )
                            };
                            if let Some(InboundAction::PerceptEmit(ref percept)) = action {
                                body_guard.register_place(percept.place.clone());
                            }
                            if auto_mark_read && action.is_some() {
                                let _ = body_guard.transport_mut().mark_read(
                                    &inbound.info.source.chat,
                                    Some(&inbound.info.source.sender),
                                    std::slice::from_ref(&inbound.info.id),
                                );
                            }
                            action
                        };
                        match action_opt {
                            Some(InboundAction::PerceptEmit(percept)) => {
                                let req = {
                                    let mut ob = ev_outbound.lock().expect("outbound lock");
                                    let id = ob.next_rpc_id();
                                    monoagens_body_sdk::Waist2Request::new(
                                        Some(id),
                                        monoagens_body_sdk::Waist2Method::PerceptEmit,
                                        *percept,
                                    )
                                };
                                if let Ok(wire) = serde_json::to_string(&req) {
                                    let _ = ev_stdout.blocking_send(wire);
                                }
                            }
                            Some(InboundAction::ApprovalResolve(resolve)) => {
                                let req = {
                                    let mut ob = ev_outbound.lock().expect("outbound lock");
                                    let id = ob.next_rpc_id();
                                    monoagens_body_sdk::Waist2Request::new(
                                        Some(id),
                                        monoagens_body_sdk::Waist2Method::ApprovalResolve,
                                        resolve,
                                    )
                                };
                                if let Ok(wire) = serde_json::to_string(&req) {
                                    let _ = ev_stdout.blocking_send(wire);
                                }
                            }
                            None => {}
                        }
                    }
                }
                _ => {}
            }
        }
    });

    // Read stdin NDJSON frames and route requests vs responses.
    let mut stdin_lines = BufReader::new(tokio::io::stdin()).lines();
    loop {
        tokio::select! {
            _ = &mut shutdown_rx => {
                break;
            }
            line_res = stdin_lines.next_line() => {
                match line_res {
                    Ok(Some(line)) => {
                        let trimmed = line.trim();
                        if trimmed.is_empty() {
                            continue;
                        }
                        match parse_inbound_line(trimmed) {
                            Ok(InboundFrame::Response(resp)) => {
                                if let Some(ref id) = resp.id
                                    && let Ok(mut map) = pending_outbound.lock()
                                    && let Some(tx) = map.remove(id)
                                {
                                    let _ = tx.send(resp);
                                }
                            }
                            Ok(InboundFrame::Request(_)) | Err(_) => {
                                if rpc_tx.send(trimmed.to_owned()).await.is_err() {
                                    break;
                                }
                            }
                        }
                    }
                    Ok(None) => break,
                    Err(err) => {
                        eprintln!("body-whatsapp stdin error: {err}");
                        break;
                    }
                }
            }
        }
    }

    drop(rpc_tx);
    drop(stdout_tx);
    bot_handle.shutdown().await;
    let _ = rpc_worker.await;
    let _ = event_worker.await;
    let _ = stdout_task.await;
    Ok(())
}

fn main() {
    let config = match WhatsAppPluginConfig::load_or_init_from_env() {
        Ok(cfg) => cfg,
        Err(err) => {
            eprintln!("body-whatsapp config error: {err}");
            std::process::exit(1);
        }
    };
    let args: Vec<String> = env::args().skip(1).collect();
    let cmd = match parse_cli_args(&args, &config) {
        Ok(cmd) => cmd,
        Err(err) => {
            eprintln!("body-whatsapp argument error: {err}");
            std::process::exit(2);
        }
    };

    match cmd {
        CliCommand::Pair {
            db_path,
            phone,
            custom_code,
            timeout_secs,
            fresh,
        } => {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("failed to build tokio runtime");
            match rt.block_on(run_pair_command(
                db_path,
                phone,
                custom_code,
                timeout_secs,
                fresh,
            )) {
                Ok(PairOutcome::Connected) => {}
                Ok(PairOutcome::Cancelled) => std::process::exit(EXIT_CODE_CANCELLED),
                Ok(PairOutcome::TimedOut) => std::process::exit(EXIT_CODE_TIMEOUT),
                Ok(PairOutcome::Failed(err)) => {
                    eprintln!("body-whatsapp pair failed: {err}");
                    std::process::exit(1);
                }
                Err(err) => {
                    eprintln!("body-whatsapp pair failed: {err}");
                    std::process::exit(1);
                }
            }
        }
        CliCommand::Serve {
            db_path,
            body_id,
            force_live,
            force_offline,
        } => {
            let use_offline = force_offline || (!force_live && !db_path.exists());
            if use_offline {
                if let Err(err) = run_offline_stdio(body_id, config) {
                    eprintln!("body-whatsapp stdio error: {err}");
                    std::process::exit(1);
                }
            } else {
                let rt = tokio::runtime::Builder::new_multi_thread()
                    .enable_all()
                    .build()
                    .expect("failed to build tokio runtime");
                if let Err(err) = rt.block_on(run_live_stdio(db_path, body_id, config)) {
                    eprintln!("body-whatsapp live stdio error: {err}");
                    std::process::exit(1);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyEvent;

    #[test]
    fn parse_cli_args_supports_pair_timeout_fresh_and_phone() {
        let cfg = WhatsAppPluginConfig::default();
        let args: Vec<String> = [
            "pair",
            "--timeout-secs",
            "60",
            "--fresh",
            "--phone",
            "628123",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();

        let cmd = parse_cli_args(&args, &cfg).expect("valid pair args");
        assert_eq!(
            cmd,
            CliCommand::Pair {
                db_path: cfg.resolve_db_path(),
                phone: Some("628123".to_owned()),
                custom_code: None,
                timeout_secs: 60,
                fresh: true,
            }
        );

        let defaults = parse_cli_args(&["pair".to_owned()], &cfg).expect("default pair args");
        assert_eq!(
            defaults,
            CliCommand::Pair {
                db_path: cfg.resolve_db_path(),
                phone: None,
                custom_code: None,
                timeout_secs: DEFAULT_PAIR_TIMEOUT_SECS,
                fresh: false,
            }
        );

        assert!(
            parse_cli_args(
                &[
                    "pair".to_owned(),
                    "--timeout-secs".to_owned(),
                    "0".to_owned()
                ],
                &cfg
            )
            .is_err()
        );
        assert!(
            parse_cli_args(
                &[
                    "pair".to_owned(),
                    "--timeout-secs".to_owned(),
                    "abc".to_owned()
                ],
                &cfg
            )
            .is_err()
        );
    }

    #[test]
    fn format_qr_for_terminal_uses_crlf_quiet_zone_and_high_contrast_ansi() {
        let payload = "2@example-whatsapp-pairing-payload";
        let rendered =
            format_qr_for_terminal(payload, Duration::from_secs(60)).expect("qr render");

        assert!(rendered.contains("valid for 60s"));
        assert!(!rendered.contains("QR raw:"));

        // Every newline must be preceded by \r so raw-mode terminals never staircase.
        for (idx, b) in rendered.bytes().enumerate() {
            if b == b'\n' {
                assert!(
                    idx > 0 && rendered.as_bytes()[idx - 1] == b'\r',
                    "bare \\n found at byte {idx}"
                );
            }
        }

        let qr_lines: Vec<&str> = rendered
            .split("\r\n")
            .filter(|line| line.starts_with("\x1b[30;47m"))
            .collect();
        assert!(!qr_lines.is_empty());

        let no_qz_art = qrcode::QrCode::new(payload.as_bytes())
            .unwrap()
            .render::<qrcode::render::unicode::Dense1x2>()
            .quiet_zone(false)
            .dark_color(qrcode::render::unicode::Dense1x2::Dark)
            .light_color(qrcode::render::unicode::Dense1x2::Light)
            .build();
        let no_qz_lines: Vec<&str> = no_qz_art.lines().collect();

        // Quiet zone adds light padding around all sides (more rows and wider rows).
        let stripped_first = qr_lines[0]
            .trim_start_matches("\x1b[30;47m")
            .trim_end_matches("\x1b[0m");
        assert!(qr_lines.len() > no_qz_lines.len());
        assert!(stripped_first.chars().count() > no_qz_lines[0].chars().count());
        assert!(stripped_first.chars().all(|c| c == ' '));
        for line in &qr_lines {
            assert!(line.ends_with("\x1b[0m"));
        }
    }

    #[test]
    fn remove_sqlite_files_cleans_db_wal_and_shm_safely() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("session.db");
        let wal_path = dir.path().join("session.db-wal");
        let shm_path = dir.path().join("session.db-shm");
        let keep_path = dir.path().join("keep.txt");

        fs::write(&db_path, b"db").unwrap();
        fs::write(&wal_path, b"wal").unwrap();
        fs::write(&shm_path, b"shm").unwrap();
        fs::write(&keep_path, b"keep").unwrap();

        remove_sqlite_files(&db_path).expect("cleanup succeeds");
        assert!(!db_path.exists());
        assert!(!wal_path.exists());
        assert!(!shm_path.exists());
        assert!(keep_path.exists());

        // Idempotent when files are already absent.
        remove_sqlite_files(&db_path).expect("idempotent cleanup succeeds");
    }

    #[test]
    fn is_cancel_key_matches_esc_q_and_ctrl_c_only() {
        assert!(is_cancel_key(&KeyEvent::new(
            KeyCode::Esc,
            KeyModifiers::NONE
        )));
        assert!(is_cancel_key(&KeyEvent::new(
            KeyCode::Char('q'),
            KeyModifiers::NONE
        )));
        assert!(is_cancel_key(&KeyEvent::new(
            KeyCode::Char('Q'),
            KeyModifiers::SHIFT
        )));
        assert!(is_cancel_key(&KeyEvent::new(
            KeyCode::Char('c'),
            KeyModifiers::CONTROL
        )));
        assert!(!is_cancel_key(&KeyEvent::new(
            KeyCode::Char('a'),
            KeyModifiers::NONE
        )));
        assert!(!is_cancel_key(&KeyEvent::new(
            KeyCode::Char('q'),
            KeyModifiers::CONTROL
        )));

        let mut release = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
        release.kind = KeyEventKind::Release;
        assert!(!is_cancel_key(&release));
    }
}
