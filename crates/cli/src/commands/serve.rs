use std::io::Write;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use monoagens_gateway::BodyPluginConfig;
use monoagens_protocol::ActorId;
use monoagens_reason::ModelProvider;
use serde::Serialize;
use tokio::sync::oneshot;

use crate::ServeArgs;
use crate::common::{
    BootGatewayParams, BootedGateway, boot_gateway_runtime, load_ready_gateway_env,
    write_json_pretty,
};

/// Per-body startup outcome reported by `monoagens serve`.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ServeBodyReport {
    pub config_id: String,
    pub status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body_id: Option<String>,
    pub places: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Startup summary emitted by `monoagens serve` after connecting configured bodies.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ServeStartupReport {
    pub status: &'static str,
    pub config_path: String,
    pub data_dir: String,
    pub connected_count: usize,
    pub failed_count: usize,
    pub bodies: Vec<ServeBodyReport>,
}

fn select_bodies(
    configured: &[BodyPluginConfig],
    filter: &[String],
) -> Result<Vec<BodyPluginConfig>, String> {
    if filter.is_empty() {
        return Ok(configured.to_vec());
    }
    for id in filter {
        if !configured.iter().any(|b| b.id == *id) {
            return Err(format!(
                "unknown body id '{id}' (not found in [[bodies]])"
            ));
        }
    }
    Ok(configured
        .iter()
        .filter(|b| filter.iter().any(|id| id == &b.id))
        .cloned()
        .collect())
}

async fn wait_for_shutdown(
    exit_after_ms: Option<u64>,
    shutdown_rx: Option<oneshot::Receiver<()>>,
) {
    let ctrl_c = tokio::signal::ctrl_c();

    #[cfg(unix)]
    let sigterm = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut sig) => {
                let _ = sig.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let sigterm = std::future::pending::<()>();

    let timer = async move {
        match exit_after_ms {
            Some(ms) => tokio::time::sleep(Duration::from_millis(ms)).await,
            None => std::future::pending::<()>().await,
        }
    };

    let hook = async move {
        match shutdown_rx {
            Some(rx) => {
                let _ = rx.await;
            }
            None => std::future::pending::<()>().await,
        }
    };

    tokio::select! {
        _ = ctrl_c => {}
        _ = sigterm => {}
        _ = timer => {}
        _ = hook => {}
    }
}

pub(crate) fn cmd_serve(
    config_path: Option<&Path>,
    json_mode: bool,
    args: ServeArgs,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> Result<u8, String> {
    run_serve_with_provider(config_path, json_mode, args, None, None, out, err)
}

/// Executes `monoagens serve` with an optional injected [`ModelProvider`] and optional external
/// shutdown signal (used by integration tests).
pub fn run_serve_with_provider(
    config_path: Option<&Path>,
    json_mode: bool,
    args: ServeArgs,
    provider_override: Option<Arc<dyn ModelProvider>>,
    shutdown_rx: Option<oneshot::Receiver<()>>,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> Result<u8, String> {
    let Some(loaded) = load_ready_gateway_env(config_path, provider_override.is_some(), err)?
    else {
        return Ok(1);
    };

    let selected_bodies = select_bodies(&loaded.config.bodies, &args.bodies)?;
    if selected_bodies.is_empty() && !args.allow_empty {
        return Err(
            "no [[bodies]] configured (declare [[bodies]] in monoagens.toml or pass --allow-empty)"
                .to_owned(),
        );
    }

    let known_actors: Vec<ActorId> = loaded
        .config
        .persons
        .iter()
        .map(|p| p.actor_id.clone())
        .collect();

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("failed to start tokio runtime: {e}"))?;

    let config_path_str = loaded.paths.config_path.display().to_string();
    let exit_after_ms = args.exit_after_ms;

    rt.block_on(async move {
        let BootedGateway {
            runtime,
            store_dir,
            _embed_sidecar,
            ..
        } = boot_gateway_runtime(
            &loaded,
            BootGatewayParams {
                data_dir: args.data_dir,
                model: args.model,
                known_actors,
                provider_override,
                json_mode,
            },
            |doors| doors,
            err,
        )
        .await?;
        let data_dir_str = store_dir.display().to_string();

        let handle = runtime.handle();
        let outcomes = handle.connect_configured_bodies(&selected_bodies).await;

        let mut connected_count = 0usize;
        let mut failed_count = 0usize;
        let mut body_reports = Vec::with_capacity(outcomes.len());

        for outcome in outcomes {
            match outcome.result {
                Ok(body_id) => {
                    connected_count += 1;
                    let places = handle
                        .router()
                        .owned_places(&body_id)
                        .into_iter()
                        .map(|p| p.as_str().to_owned())
                        .collect();
                    body_reports.push(ServeBodyReport {
                        config_id: outcome.config_id,
                        status: "connected",
                        body_id: Some(body_id.as_str().to_owned()),
                        places,
                        error: None,
                    });
                }
                Err(err_val) => {
                    failed_count += 1;
                    let msg = err_val.to_string();
                    if !json_mode {
                        let _ = writeln!(
                            err,
                            "[warn] body '{}' failed to connect: {msg}",
                            outcome.config_id
                        );
                        let _ = err.flush();
                    }
                    body_reports.push(ServeBodyReport {
                        config_id: outcome.config_id,
                        status: "failed",
                        body_id: None,
                        places: Vec::new(),
                        error: Some(msg),
                    });
                }
            }
        }

        let report = ServeStartupReport {
            status: "running",
            config_path: config_path_str,
            data_dir: data_dir_str,
            connected_count,
            failed_count,
            bodies: body_reports,
        };

        if json_mode {
            write_json_pretty(out, &report)?;
            let _ = out.flush();
        } else {
            writeln!(
                out,
                "monoagens serve running (connected={}, failed={}, data_dir={})",
                report.connected_count, report.failed_count, report.data_dir
            )
            .map_err(|e| e.to_string())?;
            for b in &report.bodies {
                match (&b.body_id, &b.error) {
                    (Some(bid), _) => {
                        writeln!(
                            out,
                            "  body={} status={} body_id={} places=[{}]",
                            b.config_id,
                            b.status,
                            bid,
                            b.places.join(",")
                        )
                        .map_err(|e| e.to_string())?;
                    }
                    (None, Some(e)) => {
                        writeln!(out, "  body={} status={} error={e}", b.config_id, b.status)
                            .map_err(|e| e.to_string())?;
                    }
                    (None, None) => {
                        writeln!(out, "  body={} status={}", b.config_id, b.status)
                            .map_err(|e| e.to_string())?;
                    }
                }
            }
            let _ = out.flush();
        }

        wait_for_shutdown(exit_after_ms, shutdown_rx).await;

        runtime.shutdown().await;
        if !json_mode {
            let _ = writeln!(err, "monoagens serve shut down cleanly");
            let _ = err.flush();
        }
        Ok(0)
    })
}
