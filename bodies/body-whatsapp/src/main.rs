//! Standalone `body-whatsapp` Waist-2 NDJSON `stdio` body and interactive `pair` CLI.

#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::env;
use std::fs;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use body_whatsapp::{
    BlobStager, DEFAULT_BODY_ID, InboundTranslator, LiveWhatsAppTransport, MemoryBlobLookup,
    WhatsAppBody,
};
use monoagens_body_kit::{PersistentBodyRuntime, load_state_dir_from_env};
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
use whatsapp_rust::wacore::types::events::{Event, EventKind};

#[derive(Debug, Clone)]
enum CliCommand {
    Pair {
        db_path: PathBuf,
        phone: Option<String>,
        custom_code: Option<String>,
    },
    Serve {
        db_path: PathBuf,
        body_id: String,
        force_live: bool,
        force_offline: bool,
    },
}

fn default_db_path() -> PathBuf {
    let base = env::var_os("XDG_DATA_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            env::var_os("HOME")
                .filter(|v| !v.is_empty())
                .map(|h| PathBuf::from(h).join(".local/share"))
        })
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("monoagens/whatsapp/session.db")
}

fn parse_cli_args(args: &[String]) -> Result<CliCommand, String> {
    let mut iter = args.iter().peekable();
    let is_pair = iter.peek().is_some_and(|a| a.as_str() == "pair");
    if is_pair {
        iter.next();
    }

    let mut db_path = default_db_path();
    let mut phone: Option<String> = None;
    let mut custom_code: Option<String> = None;
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
                     body-whatsapp pair [--db <path>] [--phone <number>] [--code <8char>]"
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

fn render_qr_to_stderr(code: &str, timeout: Duration) {
    let mut err = io::stderr().lock();
    let _ = writeln!(
        err,
        "\n=== WhatsApp Link QR (valid for {}s) ===",
        timeout.as_secs()
    );
    if let Ok(qr) = qrcode::QrCode::new(code.as_bytes()) {
        let art = qr
            .render::<qrcode::render::unicode::Dense1x2>()
            .dark_color(qrcode::render::unicode::Dense1x2::Light)
            .light_color(qrcode::render::unicode::Dense1x2::Dark)
            .build();
        let _ = writeln!(err, "{art}");
    }
    let _ = writeln!(err, "QR raw: {code}");
    let _ = writeln!(err, "===========================================\n");
    let _ = err.flush();
}

fn render_pair_code_to_stderr(code: &str, timeout: Duration) {
    let mut err = io::stderr().lock();
    let _ = writeln!(
        err,
        "\n=== WhatsApp Pair Code (valid for {}s) ===\n  \
         WhatsApp > Linked Devices > Link a Device > Link with phone number instead\n  \
         >>> {code} <<<\n\
         ===============================================\n",
        timeout.as_secs()
    );
    let _ = err.flush();
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
) -> Result<(), String> {
    ensure_parent_dir(&db_path)?;
    let db_str = db_path
        .to_str()
        .ok_or_else(|| format!("non-UTF8 db path: {}", db_path.display()))?;
    let store = SqliteStore::new(db_str)
        .await
        .map_err(|e| format!("failed to open WhatsApp SQLite store '{db_str}': {e}"))?;

    let (connected_tx, mut connected_rx) = mpsc::channel::<()>(1);
    let connected_tx_cb = connected_tx.clone();

    let mut builder = Bot::builder()
        .with_backend(store)
        .skip_history_sync()
        .on_qr_code(|code, timeout| async move {
            render_qr_to_stderr(&code, timeout);
        })
        .on_pair_code(|code, timeout| async move {
            render_pair_code_to_stderr(&code, timeout);
        })
        .on_pair_code_error(|err, _client| async move {
            eprintln!("WhatsApp pair code error: {}", err.error);
        })
        .on_connected(move |_client| {
            let tx = connected_tx_cb.clone();
            async move {
                eprintln!("WhatsApp paired and connected successfully.");
                let _ = tx.send(()).await;
            }
        })
        .on_logged_out(|info| async move {
            eprintln!("WhatsApp logged out during pairing: {:?}", info.reason);
        });

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

    let mut handle = bot.spawn();
    tokio::select! {
        _ = &mut handle => {
            eprintln!("WhatsApp pairing session ended.");
        }
        _ = connected_rx.recv() => {
            // Allow persistence manager to flush paired credentials to SQLite cleanly.
            tokio::time::sleep(Duration::from_secs(2)).await;
            handle.shutdown().await;
            eprintln!("Session saved to {}", db_path.display());
        }
        _ = tokio::signal::ctrl_c() => {
            eprintln!("Interrupted; shutting down pairing session.");
            handle.shutdown().await;
        }
    }
    Ok(())
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

fn run_offline_stdio(body_id: String) -> Result<(), String> {
    let blob_lookup = MemoryBlobLookup::default();
    let mut runtime = PersistentBodyRuntime::new(load_state_dir_from_env());
    let mut body = WhatsAppBody::offline(BodyId::new(body_id)).with_blob_lookup(blob_lookup.clone());

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

async fn run_live_stdio(db_path: PathBuf, body_id: String) -> Result<(), String> {
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
    let body = Arc::new(Mutex::new(
        WhatsAppBody::new(BodyId::new(body_id), transport).with_blob_lookup(blob_lookup.clone()),
    ));
    let runtime = Arc::new(Mutex::new(PersistentBodyRuntime::new(
        load_state_dir_from_env(),
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
        let mut translator = InboundTranslator::new(boot_ts_ms);
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
                        let key = format!("wa-percept-{boot_ts_ms}-{seq}");
                        let percept_opt = {
                            let mut body_guard = ev_body.lock().expect("body lock");
                            let echo_snapshot = body_guard.echo_registry().clone();
                            let p = translator.translate_inbound(
                                inbound,
                                body_guard.transport_mut(),
                                &echo_snapshot,
                                &mut stager,
                                key,
                            );
                            if let Some(ref percept) = p {
                                body_guard.register_place(percept.place.clone());
                            }
                            p
                        };
                        if let Some(percept) = percept_opt {
                            let req = {
                                let mut ob = ev_outbound.lock().expect("outbound lock");
                                let id = ob.next_rpc_id();
                                monoagens_body_sdk::Waist2Request::new(
                                    Some(id),
                                    monoagens_body_sdk::Waist2Method::PerceptEmit,
                                    percept,
                                )
                            };
                            if let Ok(wire) = serde_json::to_string(&req) {
                                let _ = ev_stdout.blocking_send(wire);
                            }
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
    let args: Vec<String> = env::args().skip(1).collect();
    let cmd = match parse_cli_args(&args) {
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
        } => {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("failed to build tokio runtime");
            if let Err(err) = rt.block_on(run_pair_command(db_path, phone, custom_code)) {
                eprintln!("body-whatsapp pair failed: {err}");
                std::process::exit(1);
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
                if let Err(err) = run_offline_stdio(body_id) {
                    eprintln!("body-whatsapp stdio error: {err}");
                    std::process::exit(1);
                }
            } else {
                let rt = tokio::runtime::Builder::new_multi_thread()
                    .enable_all()
                    .build()
                    .expect("failed to build tokio runtime");
                if let Err(err) = rt.block_on(run_live_stdio(db_path, body_id)) {
                    eprintln!("body-whatsapp live stdio error: {err}");
                    std::process::exit(1);
                }
            }
        }
    }
}
