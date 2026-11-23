use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use monoagens_gateway::{
    LockError, PluginConfig, PluginEnvContext, PluginKind, acquire_data_dir_lock,
    load_plugin_env_secrets,
};
use monoagens_protocol::ActorId;
use monoagens_reason::ModelProvider;
use serde::Serialize;
use tokio::sync::oneshot;

use crate::ServeArgs;
use crate::common::{
    BootGatewayParams, BootedGateway, boot_gateway_runtime, load_ready_gateway_env,
    write_json_pretty,
};

/// Per-plugin startup outcome reported by `monoagens serve`.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ServeBodyReport {
    pub config_id: String,
    pub kind: &'static str,
    pub status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body_id: Option<String>,
    pub places: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Startup summary emitted by `monoagens serve` after connecting configured plugins.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ServeStartupReport {
    pub status: &'static str,
    pub config_path: String,
    pub data_dir: String,
    pub connected_count: usize,
    pub failed_count: usize,
    pub skipped_count: usize,
    pub rbac_enabled: bool,
    pub indexed_commands: Vec<String>,
    pub bodies: Vec<ServeBodyReport>,
}

fn select_plugins(
    configured: &[PluginConfig],
    filter: &[String],
) -> Result<Vec<PluginConfig>, String> {
    if filter.is_empty() {
        return Ok(configured.to_vec());
    }
    for id in filter {
        if !configured.iter().any(|p| p.id == *id) {
            return Err(format!(
                "unknown body id '{id}' (not found in [plugins.*] or [[bodies]])"
            ));
        }
    }
    Ok(configured
        .iter()
        .filter(|p| filter.iter().any(|id| id == &p.id))
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

    let effective_plugins = loaded.config.effective_plugins();
    let selected_plugins = select_plugins(&effective_plugins, &args.bodies)?;
    if selected_plugins.is_empty() && !args.allow_empty {
        return Err(
            "no plugins or [[bodies]] configured (declare [plugins.<id>] or [[bodies]] in monoagens.toml or pass --allow-empty)"
                .to_owned(),
        );
    }

    let known_actors: Vec<ActorId> = loaded
        .config
        .persons
        .iter()
        .map(|p| p.actor_id.clone())
        .collect();

    // One gateway per data dir: hold `<data_dir>/gateway.lock` for the life of this process.
    let lock_dir = args
        .data_dir
        .clone()
        .unwrap_or_else(|| loaded.paths.data_dir.clone());
    let _data_dir_lock = match acquire_data_dir_lock(&lock_dir) {
        Ok(lock) => lock,
        Err(LockError::Held(held)) => {
            let who = held
                .pid
                .map_or_else(|| "another process".to_owned(), |pid| format!("pid {pid}"));
            let _ = writeln!(
                err,
                "error: data dir '{}' is already hosted by {who} (lock '{}'). \
                 Attach to it (open the TUI on the same data dir) or stop that process first.",
                lock_dir.display(),
                held.path.display()
            );
            let _ = err.flush();
            return Ok(1);
        }
        Err(other) => return Err(format!("failed to take the data-dir lock: {other}")),
    };

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("failed to start tokio runtime: {e}"))?;

    let config_path_str = loaded.paths.config_path.display().to_string();
    let config_dir = loaded
        .paths
        .config_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .or_else(|| Some(PathBuf::from(".")));
    let rbac_enabled = loaded.config.rbac.is_some();
    let plugin_env =
        load_plugin_env_secrets(&loaded.paths.secrets_path).map_err(|e| e.to_string())?;
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

        let env_ctx = PluginEnvContext {
            config_dir,
            data_dir: Some(store_dir.clone()),
            rbac_enabled,
            plugin_env,
        };

        let handle = runtime.handle();
        let startup = handle
            .connect_configured_plugins(&selected_plugins, &env_ctx)
            .await;

        let kind_by_id: BTreeMap<String, PluginKind> = selected_plugins
            .iter()
            .map(|p| (p.id.clone(), p.kind))
            .collect();
        let kind_str = |id: &str| {
            kind_by_id
                .get(id)
                .copied()
                .unwrap_or(PluginKind::Adapter)
                .as_str()
        };

        let connected_count = startup.connected.len();
        let failed_count = startup.failed.len();
        let skipped_count = startup.skipped.len();

        let mut reports_by_id: BTreeMap<String, ServeBodyReport> = BTreeMap::new();
        for outcome in startup.connected {
            if let Ok(body_id) = outcome.result {
                let places = handle
                    .router()
                    .owned_places(&body_id)
                    .into_iter()
                    .map(|p| p.as_str().to_owned())
                    .collect();
                let kind = kind_str(&outcome.config_id);
                reports_by_id.insert(
                    outcome.config_id.clone(),
                    ServeBodyReport {
                        config_id: outcome.config_id,
                        kind,
                        status: "connected",
                        body_id: Some(body_id.as_str().to_owned()),
                        places,
                        error: None,
                    },
                );
            }
        }
        for outcome in startup.failed {
            if let Err(err_val) = outcome.result {
                let msg = err_val.to_string();
                let kind = kind_str(&outcome.config_id);
                if !json_mode {
                    let _ = writeln!(
                        err,
                        "[warn] plugin '{}' ({kind}) failed to connect: {msg}",
                        outcome.config_id
                    );
                    let _ = err.flush();
                }
                reports_by_id.insert(
                    outcome.config_id.clone(),
                    ServeBodyReport {
                        config_id: outcome.config_id,
                        kind,
                        status: "failed",
                        body_id: None,
                        places: Vec::new(),
                        error: Some(msg),
                    },
                );
            }
        }
        for skip in startup.skipped {
            let kind = kind_str(&skip.id);
            if !json_mode {
                let _ = writeln!(
                    err,
                    "[warn] plugin '{}' ({kind}) skipped: {}",
                    skip.id, skip.reason
                );
                let _ = err.flush();
            }
            reports_by_id.insert(
                skip.id.clone(),
                ServeBodyReport {
                    config_id: skip.id,
                    kind,
                    status: "skipped",
                    body_id: None,
                    places: Vec::new(),
                    error: Some(skip.reason),
                },
            );
        }

        // Preserve declared configuration order in the emitted report.
        let mut body_reports = Vec::with_capacity(reports_by_id.len());
        for plugin in &selected_plugins {
            if let Some(entry) = reports_by_id.remove(&plugin.id) {
                body_reports.push(entry);
            }
        }
        body_reports.extend(reports_by_id.into_values());

        let indexed_commands: Vec<String> = {
            let reg = handle.command_registry();
            let guard = reg.lock().unwrap_or_else(|e| e.into_inner());
            guard
                .all_commands()
                .into_iter()
                .map(|c| c.name.clone())
                .collect()
        };

        let report = ServeStartupReport {
            status: "running",
            config_path: config_path_str,
            data_dir: data_dir_str,
            connected_count,
            failed_count,
            skipped_count,
            rbac_enabled,
            indexed_commands,
            bodies: body_reports,
        };

        if json_mode {
            write_json_pretty(out, &report)?;
            let _ = out.flush();
        } else {
            writeln!(
                out,
                "monoagens serve running (connected={}, failed={}, skipped={}, rbac={}, data_dir={})",
                report.connected_count,
                report.failed_count,
                report.skipped_count,
                if report.rbac_enabled { "on" } else { "off" },
                report.data_dir
            )
            .map_err(|e| e.to_string())?;
            for b in &report.bodies {
                match (&b.body_id, &b.error) {
                    (Some(bid), _) => {
                        writeln!(
                            out,
                            "  plugin={} kind={} status={} body_id={} places=[{}]",
                            b.config_id,
                            b.kind,
                            b.status,
                            bid,
                            b.places.join(",")
                        )
                        .map_err(|e| e.to_string())?;
                    }
                    (None, Some(e)) => {
                        writeln!(
                            out,
                            "  plugin={} kind={} status={} reason={e}",
                            b.config_id, b.kind, b.status
                        )
                        .map_err(|e| e.to_string())?;
                    }
                    (None, None) => {
                        writeln!(
                            out,
                            "  plugin={} kind={} status={}",
                            b.config_id, b.kind, b.status
                        )
                        .map_err(|e| e.to_string())?;
                    }
                }
            }
            if !report.indexed_commands.is_empty() {
                let cmds: Vec<String> = report
                    .indexed_commands
                    .iter()
                    .map(|c| format!("/{c}"))
                    .collect();
                writeln!(out, "  commands=[{}]", cmds.join(", "))
                    .map_err(|e| e.to_string())?;
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
