//! Standalone `body-telegram` Waist-2 NDJSON `stdio` body binary.

#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::env;
use std::io::{self, BufRead, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use body_telegram::{
    BlobResolver, BlobStager, DEFAULT_BODY_ID, DEFAULT_TELEGRAM_API_BASE, LiveTelegramTransport,
    OfflineTelegramTransport, OutboundTelegramEvent, StagedBlob, TelegramBody, TelegramPluginConfig,
    TelegramTransport, resolve_state_dir_from_env,
};
use clap::Parser;
use monoagens_body_kit::PersistentBodyRuntime;
use monoagens_body_sdk::{
    ApprovalResolveResult, BlobGetResult, BlobPutResult, BodyId, InboundFrame, OutboundBuilder,
    PlaceId, RpcId, Waist2Response, parse_inbound_line, verify_blob_get,
};
use serde_json::Value;
use tokio::sync::{mpsc, oneshot};

#[derive(Debug, Parser)]
#[command(name = "body-telegram", about = "Telegram Waist-2 body binary")]
struct CliArgs {
    /// Environment variable holding the Telegram Bot API token.
    #[arg(long, default_value = "TELEGRAM_BOT_TOKEN")]
    token_env: String,

    /// Telegram Bot API base URL (override for local mock HTTP tests).
    #[arg(long, default_value = DEFAULT_TELEGRAM_API_BASE)]
    api_base: String,

    /// Disable live Bot API polling and run in offline stdio conformance mode.
    #[arg(long)]
    offline: bool,

    /// Override Waist-2 body_id reported by body/discover.
    #[arg(long, default_value = DEFAULT_BODY_ID)]
    body_id: String,

    /// Optional extra places to advertise in body/discover.
    #[arg(long)]
    place: Vec<String>,
}

type PendingMap = Arc<Mutex<HashMap<RpcId, oneshot::Sender<Waist2Response<Value>>>>>;

#[derive(Clone)]
struct StdioRpcBridge {
    stdout_tx: mpsc::UnboundedSender<String>,
    pending: PendingMap,
    builder: Arc<Mutex<OutboundBuilder>>,
}

impl StdioRpcBridge {
    fn new(stdout_tx: mpsc::UnboundedSender<String>, pending: PendingMap, prefix: &str) -> Self {
        Self {
            stdout_tx,
            pending,
            builder: Arc::new(Mutex::new(OutboundBuilder::new(prefix))),
        }
    }

    fn call_blocking<P: serde::Serialize>(
        &self,
        make_req: impl FnOnce(&mut OutboundBuilder) -> monoagens_body_sdk::Waist2Request<P>,
    ) -> Result<Value, String> {
        let req = {
            let mut b = self.builder.lock().expect("outbound builder lock");
            make_req(&mut b)
        };
        let id = req
            .id
            .clone()
            .ok_or_else(|| "outbound request missing id".to_owned())?;
        let wire = serde_json::to_string(&req).map_err(|e| format!("encode RPC failed: {e}"))?;

        let (tx, rx) = oneshot::channel();
        self.pending.lock().expect("pending map lock").insert(id.clone(), tx);

        if self.stdout_tx.send(wire).is_err() {
            self.pending.lock().expect("pending map lock").remove(&id);
            return Err("stdout channel closed".to_owned());
        }

        let resp = rx
            .blocking_recv()
            .map_err(|_| "Gateway closed before replying to outbound RPC".to_owned())?;
        if let Some(err) = resp.error {
            return Err(format!("Gateway RPC error [{}]: {}", err.code, err.message));
        }
        resp.result
            .ok_or_else(|| "Gateway RPC response missing result".to_owned())
    }
}

impl BlobResolver for StdioRpcBridge {
    fn fetch_blob(&self, blob_ref: &str) -> Result<Vec<u8>, String> {
        let blob_ref_owned = blob_ref.to_owned();
        let val = self.call_blocking(|b| b.blob_get(blob_ref_owned, None))?;
        let res: BlobGetResult =
            serde_json::from_value(val).map_err(|e| format!("invalid blob/get result: {e}"))?;
        verify_blob_get(&res, None)
            .map_err(|e| format!("blob/get verification failed [{}]: {}", e.code, e.message))?;
        res.bytes
            .map(|p| p.0)
            .ok_or_else(|| "blob/get returned no inline bytes".to_owned())
    }
}

impl BlobStager for StdioRpcBridge {
    fn stage_blob(
        &mut self,
        bytes: &[u8],
        content_type: Option<&str>,
        scope_id: Option<&str>,
    ) -> Result<StagedBlob, String> {
        let val = self.call_blocking(|b| b.blob_put(bytes, content_type, scope_id))?;
        let res: BlobPutResult =
            serde_json::from_value(val).map_err(|e| format!("invalid blob/put result: {e}"))?;
        Ok(StagedBlob {
            blob_ref: res.blob_ref,
            sha256: res.sha256,
            expires_at_ms: res.expires_at_ms,
        })
    }
}

fn load_plugin_config() -> TelegramPluginConfig {
    TelegramPluginConfig::load_or_init_from_env().unwrap_or_else(|err| {
        eprintln!("body-telegram config warning: {err}");
        TelegramPluginConfig::default()
    })
}

fn run_offline(args: CliArgs) -> Result<(), String> {
    let config = load_plugin_config();
    let mut runtime = PersistentBodyRuntime::new(resolve_state_dir_from_env());
    let mut body = TelegramBody::new(BodyId::new(args.body_id), OfflineTelegramTransport::new())
        .with_config(config);
    if !args.place.is_empty() {
        body = body.with_places(args.place.into_iter().map(PlaceId::new).collect());
    }
    let stdin = io::stdin();
    let stdout = io::stdout();
    runtime
        .run_stdio(stdin.lock(), stdout.lock(), &mut body)
        .map_err(|e| format!("body-telegram stdio error: {e}"))
}

fn run_live(args: CliArgs, token: String) -> Result<(), String> {
    let config = load_plugin_config();
    let poll_timeout_secs = config.poll_timeout_secs;
    let transport = LiveTelegramTransport::new(args.api_base.clone(), token.clone())?;
    let mut poll_transport = LiveTelegramTransport::new(args.api_base, token)?;

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("failed to create tokio runtime: {e}"))?;

    let shutdown = Arc::new(AtomicBool::new(false));
    let pending: PendingMap = Arc::new(Mutex::new(HashMap::new()));
    let (stdout_tx, mut stdout_rx) = mpsc::unbounded_channel::<String>();
    let (req_tx, mut req_rx) = mpsc::unbounded_channel::<String>();

    let bridge = StdioRpcBridge::new(stdout_tx.clone(), Arc::clone(&pending), "tg");

    let mut body = TelegramBody::new(BodyId::new(&args.body_id), transport)
        .with_config(config)
        .with_blob_resolver(Arc::new(bridge.clone()))
        .with_shutdown_flag(Arc::clone(&shutdown));
    if !args.place.is_empty() {
        body = body.with_places(args.place.into_iter().map(PlaceId::new).collect());
    }
    if let Err(err) = body.init_bot_identity() {
        eprintln!("body-telegram warning: bot identity unavailable ({err})");
    }

    let state_runtime = Arc::new(Mutex::new(PersistentBodyRuntime::new(
        resolve_state_dir_from_env(),
    )));
    let body_shared = Arc::new(Mutex::new(body));

    // Telegram Bot API long-polling thread (detached on process shutdown so runtime drop never stalls).
    let body_poll = Arc::clone(&body_shared);
    let shutdown_poll = Arc::clone(&shutdown);
    let mut stager_poll = bridge.clone();
    let rpc_poll = bridge;
    let _poller_thread = std::thread::Builder::new()
        .name("tg-poller".to_owned())
        .spawn(move || {
            let mut offset: Option<i64> = None;
            while !shutdown_poll.load(Ordering::SeqCst) {
                match poll_transport.get_updates(offset, poll_timeout_secs) {
                    Ok(updates) => {
                        for upd in updates {
                            offset = Some(upd.update_id.saturating_add(1));
                            let translated = {
                                let mut guard = body_poll.lock().expect("body lock");
                                guard.translate_update_event(&upd, Some(&mut stager_poll))
                            };
                            match translated {
                                Ok(Some(OutboundTelegramEvent::Percept(percept_params))) => {
                                    if let Err(err) = rpc_poll.call_blocking(|b| {
                                        monoagens_body_sdk::Waist2Request::new(
                                            Some(b.next_rpc_id()),
                                            monoagens_body_sdk::Waist2Method::PerceptEmit,
                                            *percept_params,
                                        )
                                    }) {
                                        eprintln!("body-telegram percept/emit failed: {err}");
                                    }
                                }
                                Ok(Some(OutboundTelegramEvent::ApprovalResolve(outbound))) => {
                                    let action_id = outbound.params.action_id.as_str().to_owned();
                                    let approved = outbound.params.approved;
                                    let resolver = outbound.params.resolver.clone();
                                    match rpc_poll.call_blocking(|b| {
                                        monoagens_body_sdk::Waist2Request::new(
                                            Some(b.next_rpc_id()),
                                            monoagens_body_sdk::Waist2Method::ApprovalResolve,
                                            outbound.params,
                                        )
                                    }) {
                                        Ok(val) => {
                                            match serde_json::from_value::<ApprovalResolveResult>(val) {
                                                Ok(res) if res.applied => {
                                                    let mut guard =
                                                        body_poll.lock().expect("body lock");
                                                    guard.mark_approval_resolved(
                                                        &action_id,
                                                        approved,
                                                        &resolver,
                                                    );
                                                }
                                                Ok(_) => {}
                                                Err(err) => {
                                                    eprintln!(
                                                        "body-telegram invalid approval/resolve result: {err}"
                                                    );
                                                }
                                            }
                                        }
                                        Err(err) => {
                                            eprintln!("body-telegram approval/resolve failed: {err}");
                                        }
                                    }
                                }
                                Ok(None) => {}
                                Err(err) => {
                                    eprintln!("body-telegram translate_update error: {err}");
                                }
                            }
                        }
                    }
                    Err(err) => {
                        eprintln!("body-telegram getUpdates error: {err}");
                        std::thread::sleep(Duration::from_secs(2));
                    }
                }
            }
        });

    rt.block_on(async move {
        // Dedicated stdout NDJSON writer task (strictly JSON-RPC frames only).
        let writer_handle = tokio::task::spawn_blocking(move || {
            let stdout = io::stdout();
            let mut lock = stdout.lock();
            while let Some(line) = stdout_rx.blocking_recv() {
                if writeln!(lock, "{line}").is_err() || lock.flush().is_err() {
                    break;
                }
            }
        });

        // Stdin reader task multiplexing Gateway requests and responses to outbound calls.
        let pending_reader = Arc::clone(&pending);
        let shutdown_reader = Arc::clone(&shutdown);
        let reader_handle = tokio::task::spawn_blocking(move || {
            let stdin = io::stdin();
            for line_res in stdin.lock().lines() {
                match line_res {
                    Ok(line) => {
                        let trimmed = line.trim();
                        if trimmed.is_empty() {
                            continue;
                        }
                        match parse_inbound_line(trimmed) {
                            Ok(InboundFrame::Response(resp)) => {
                                if let Some(ref id) = resp.id
                                    && let Some(tx) =
                                        pending_reader.lock().expect("pending lock").remove(id)
                                {
                                    let _ = tx.send(resp);
                                }
                            }
                            Ok(InboundFrame::Request(_)) => {
                                if req_tx.send(trimmed.to_owned()).is_err() {
                                    break;
                                }
                            }
                            Err(_) => {
                                // Let PersistentBodyRuntime produce a structured JSON-RPC error.
                                if req_tx.send(trimmed.to_owned()).is_err() {
                                    break;
                                }
                            }
                        }
                    }
                    Err(err) => {
                        eprintln!("body-telegram stdin read error: {err}");
                        break;
                    }
                }
            }
            shutdown_reader.store(true, Ordering::SeqCst);
        });

        // Request dispatcher task executing Waist-2 calls from Gateway.
        let rt_exec = Arc::clone(&state_runtime);
        let body_exec = Arc::clone(&body_shared);
        let out_exec = stdout_tx.clone();
        let dispatch_handle = tokio::spawn(async move {
            while let Some(line) = req_rx.recv().await {
                let rt_c = Arc::clone(&rt_exec);
                let body_c = Arc::clone(&body_exec);
                let reply = tokio::task::spawn_blocking(move || {
                    let mut runtime = rt_c.lock().expect("runtime lock");
                    let mut body = body_c.lock().expect("body lock");
                    runtime.dispatch_line(&line, &mut *body)
                })
                .await
                .ok()
                .flatten();

                if let Some(out) = reply
                    && out_exec.send(out).is_err()
                {
                    break;
                }
            }
        });

        // Wait for stdin EOF / shutdown signal.
        let shutdown_sig = Arc::clone(&shutdown);
        tokio::select! {
            _ = reader_handle => {}
            _ = tokio::signal::ctrl_c() => {
                shutdown_sig.store(true, Ordering::SeqCst);
            }
        }

        shutdown.store(true, Ordering::SeqCst);
        drop(stdout_tx);
        let _ = dispatch_handle.await;
        let _ = writer_handle.await;
    });

    Ok(())
}

fn main() {
    let args = CliArgs::parse();
    let token = env::var(&args.token_env)
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty());

    let res = match (args.offline, token) {
        (true, _) | (false, None) => run_offline(args),
        (false, Some(tok)) => run_live(args, tok),
    };

    if let Err(err) = res {
        eprintln!("{err}");
        std::process::exit(1);
    }
}
