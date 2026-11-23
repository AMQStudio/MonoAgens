use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use monoagens_gateway::{
    AdminServer, Clock, GatewayOwner, LockError, OwnerMode, OwnerParams, PluginConfig,
    PluginEnvContext, PluginKind, PluginState, PluginSupervisor, RestartPolicy, RunLog,
    acquire_data_dir_lock, load_plugin_env_secrets, resolve_plugin_boot_order,
};
use monoagens_protocol::{ActorId, BodyId};
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
    let mut selected_plugins = select_plugins(&effective_plugins, &args.bodies)?;
    if !args.bodies.is_empty() {
        // Naming a plugin with `--plugin` starts it even when it is disabled in the config.
        for p in &mut selected_plugins {
            p.enabled = true;
        }
    }
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
    let data_dir_lock = match acquire_data_dir_lock(&lock_dir) {
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
            owner_actor,
            cfg,
            clock,
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
        let clock_dyn: Arc<dyn Clock> = clock;
        let runlog = RunLog::new(
            Arc::clone(&clock_dyn),
            handle.diagnostics().clone(),
            Some(RunLog::default_file(&store_dir)),
        );
        runlog.attach_diagnostics(handle.diagnostics());
        let supervisor = PluginSupervisor::new(
            handle.clone(),
            env_ctx,
            clock_dyn,
            runlog.clone(),
            RestartPolicy::default(),
            selected_plugins.clone(),
        );
        let owner = Arc::new(GatewayOwner::new(OwnerParams {
            handle: handle.clone(),
            supervisor: supervisor.clone(),
            runlog: runlog.clone(),
            mode: OwnerMode::Serve,
            config: cfg,
            config_path: loaded.paths.config_path.clone(),
            secrets_path: loaded.paths.secrets_path.clone(),
            data_dir: store_dir.clone(),
            owner_actor,
        }));
        // Serve the owner-only admin socket so a TUI can attach (and detach) at will.
        let admin = AdminServer::start(&data_dir_lock, &store_dir, Arc::clone(&owner))
            .await
            .map_err(|e| format!("failed to serve the admin socket: {e}"))?;

        // Dependency order, stderr capture, crash backoff and per-plugin locks live in the
        // supervisor; the report below is read from its state.
        supervisor.start_enabled().await;

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
        let builtin: &[&str] = if rbac_enabled { &["rbac"] } else { &[] };
        let plan_skipped: BTreeMap<String, String> = resolve_plugin_boot_order(
            &selected_plugins
                .iter()
                .filter(|p| p.enabled)
                .cloned()
                .collect::<Vec<_>>(),
            builtin,
        )
        .skipped
        .into_iter()
        .map(|s| (s.id, s.reason))
        .collect();

        let mut connected_count = 0usize;
        let mut failed_count = 0usize;
        let mut skipped_count = 0usize;
        let mut body_reports = Vec::with_capacity(selected_plugins.len());
        for plugin in &selected_plugins {
            let kind = kind_str(&plugin.id);
            let Some(info) = supervisor.info(&plugin.id) else {
                continue;
            };
            let reason = info.reason.clone().unwrap_or_default();
            let report = match info.state {
                PluginState::Running => {
                    connected_count += 1;
                    let body_id = info.body_id.clone().unwrap_or_default();
                    let places = handle
                        .router()
                        .owned_places(&BodyId::new(&body_id))
                        .into_iter()
                        .map(|p| p.as_str().to_owned())
                        .collect();
                    ServeBodyReport {
                        config_id: plugin.id.clone(),
                        kind,
                        status: "connected",
                        body_id: Some(body_id),
                        places,
                        error: None,
                    }
                }
                PluginState::Failed
                    if plan_skipped.contains_key(&plugin.id)
                        || reason.starts_with("required dependency") =>
                {
                    skipped_count += 1;
                    ServeBodyReport {
                        config_id: plugin.id.clone(),
                        kind,
                        status: "skipped",
                        body_id: None,
                        places: Vec::new(),
                        error: Some(reason),
                    }
                }
                PluginState::Failed => {
                    failed_count += 1;
                    ServeBodyReport {
                        config_id: plugin.id.clone(),
                        kind,
                        status: "failed",
                        body_id: None,
                        places: Vec::new(),
                        error: Some(reason),
                    }
                }
                PluginState::RunningElsewhere => {
                    skipped_count += 1;
                    ServeBodyReport {
                        config_id: plugin.id.clone(),
                        kind,
                        status: "skipped",
                        body_id: None,
                        places: Vec::new(),
                        error: Some("running in another MonoAgens (plugin lock held)".to_owned()),
                    }
                }
                PluginState::Stopped | PluginState::Starting => {
                    skipped_count += 1;
                    ServeBodyReport {
                        config_id: plugin.id.clone(),
                        kind,
                        status: "skipped",
                        body_id: None,
                        places: Vec::new(),
                        error: Some(if plugin.enabled {
                            "not started".to_owned()
                        } else {
                            "disabled in the configuration".to_owned()
                        }),
                    }
                }
            };
            if !json_mode && report.status != "connected" {
                let _ = writeln!(
                    err,
                    "[warn] plugin '{}' ({kind}) {}: {}",
                    report.config_id,
                    if report.status == "failed" { "failed to connect" } else { "skipped" },
                    report.error.as_deref().unwrap_or("")
                );
                let _ = err.flush();
            }
            body_reports.push(report);
        }

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

        supervisor.shutdown().await;
        admin.shutdown();
        runtime.shutdown().await;
        if !json_mode {
            let _ = writeln!(err, "monoagens serve shut down cleanly");
            let _ = err.flush();
        }
        Ok(0)
    })
}
