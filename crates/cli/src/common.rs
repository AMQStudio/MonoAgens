use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use monoagens_gateway::{
    Clock, CueRecorder, DiagnosticSink, DoorController, EmbedBackendKind, GatewayConfig,
    GatewayOptions, GatewayRuntime, GatewaySeams, LoadedGatewayEnv, ResolvedPaths, SystemClock,
    ValveDoorController, load_approval_secrets, load_or_init_gateway_env,
    resolve_approval_mac_key, resolve_paths,
};
use monoagens_protocol::ActorId;
use monoagens_reason::{LocalEmbedSidecar, ModelProvider};
use monoagens_store::{SegmentedStore, StoreConfig};
use monoagens_valve::{Valve, ValveConfig, ValveMode, ValveSnapshot};
use serde::Serialize;

pub(crate) fn build_valve_config(cfg: &GatewayConfig) -> ValveConfig {
    ValveConfig {
        mode: match cfg.valve.mode.as_str() {
            "partial" => ValveMode::Partial,
            "off" => ValveMode::Off,
            _ => ValveMode::Enforcing,
        },
        max_chain_wakes: cfg.valve.max_chain_wakes,
        daily_token_budget: cfg.model.daily_token_budget,
        ..ValveConfig::default()
    }
}

pub(crate) fn owner_actor_from_cfg(cfg: Option<&GatewayConfig>) -> ActorId {
    cfg.and_then(|c| c.persons.iter().find(|p| p.is_owner))
        .map(|p| p.actor_id.clone())
        .unwrap_or_else(|| ActorId::new("per_owner"))
}

pub(crate) fn load_paths(
    config_path: Option<&Path>,
) -> Result<(Option<GatewayConfig>, ResolvedPaths), String> {
    let paths = resolve_paths(config_path).map_err(|e| e.to_string())?;
    let cfg = if paths.config_path.exists() {
        Some(
            monoagens_gateway::load_config_file(&paths.config_path, &paths.secrets_path)
                .map_err(|e| e.to_string())?,
        )
    } else {
        None
    };
    let effective_paths = if let Some(ref c) = cfg
        && let Some(ref data_dir) = c.data_dir
    {
        ResolvedPaths {
            config_path: paths.config_path,
            secrets_path: paths.secrets_path,
            data_dir: data_dir.clone(),
        }
    } else {
        paths
    };
    Ok((cfg, effective_paths))
}

pub(crate) fn open_store_readonly(data_dir: &Path) -> Result<SegmentedStore, String> {
    if !data_dir.exists() {
        return Err(format!(
            "store directory '{}' does not exist",
            data_dir.display()
        ));
    }
    SegmentedStore::open(StoreConfig::new(data_dir.to_path_buf()))
        .map_err(|e| format!("failed to open store '{}': {e}", data_dir.display()))
}

pub(crate) fn print_startup_not_ready(
    loaded: &LoadedGatewayEnv,
    startup_err: &dyn std::fmt::Display,
    err: &mut dyn Write,
) {
    let _ = writeln!(err, "{startup_err}");
    let _ = writeln!(
        err,
        "config path: {}\nsecrets path: {}\nrequired env vars (when using openai_compatible): MONOAGENS_MODEL_API_KEY (or OPENAI_API_KEY), MONOAGENS_MODEL_ENDPOINT",
        loaded.paths.config_path.display(),
        loaded.paths.secrets_path.display(),
    );
}

pub(crate) fn write_json_pretty<T: Serialize + ?Sized>(
    out: &mut dyn Write,
    value: &T,
) -> Result<(), String> {
    writeln!(
        out,
        "{}",
        serde_json::to_string_pretty(value).map_err(|e| e.to_string())?
    )
    .map_err(|e| e.to_string())
}

fn find_workspace_root_for_cli() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    if manifest
        .join("lab/adapters/eval_helpers/embeddinggemma2_sidecar.py")
        .is_file()
    {
        return manifest;
    }
    if let Ok(cwd) = std::env::current_dir()
        && cwd
            .join("lab/adapters/eval_helpers/embeddinggemma2_sidecar.py")
            .is_file()
    {
        return cwd;
    }
    manifest
}

pub(crate) fn maybe_start_embed_sidecar(
    cfg: &mut GatewayConfig,
    json_mode: bool,
    err: &mut dyn Write,
) -> Result<Option<LocalEmbedSidecar>, String> {
    if cfg.embed.backend != EmbedBackendKind::EmbeddingGemma2Http {
        return Ok(None);
    }
    if cfg
        .embed
        .endpoint_url
        .as_deref()
        .is_some_and(|s| !s.trim().is_empty())
        || std::env::var("MONOAGENS_EMBED_ENDPOINT").is_ok_and(|s| !s.trim().is_empty())
    {
        return Ok(None);
    }

    let ws_root = find_workspace_root_for_cli();
    let script = ws_root.join("lab/adapters/eval_helpers/embeddinggemma2_sidecar.py");
    let model_dir = cfg
        .embed
        .model_path
        .clone()
        .unwrap_or_else(|| ws_root.join("lab/datasets/embedder_models/embeddinggemma-2"));
    if !script.is_file() {
        return Err(format!(
            "embeddinggemma2_http sidecar script not found at '{}'",
            script.display()
        ));
    }
    if !model_dir.is_dir() {
        return Err(format!(
            "embeddinggemma2_http requires embed.model_path pointing to an existing directory (missing '{}')",
            model_dir.display()
        ));
    }
    cfg.embed.model_path = Some(model_dir.clone());

    if !json_mode {
        let _ = writeln!(
            err,
            "[embedder] memuat EmbeddingGemma 2 dari {} ...",
            model_dir.display()
        );
        let _ = err.flush();
    }

    let sidecar = LocalEmbedSidecar::start(&script, &model_dir, &ws_root)?;
    let url = sidecar.url().to_owned();
    let device_info = sidecar.device_info().to_owned();
    cfg.embed.endpoint_url = Some(url.clone());
    if !json_mode {
        let _ = writeln!(err, "[embedder] EmbeddingGemma 2 siap ({device_info}) di {url}");
        let _ = err.flush();
    }
    Ok(Some(sidecar))
}

pub(crate) fn load_ready_gateway_env(
    config_path: Option<&Path>,
    has_provider_override: bool,
    err: &mut dyn Write,
) -> Result<Option<LoadedGatewayEnv>, String> {
    let loaded =
        load_or_init_gateway_env(config_path, |k| std::env::var(k).ok()).map_err(|e| e.to_string())?;
    if !has_provider_override
        && let Err(startup_err) = loaded.ensure_startup_ready()
    {
        print_startup_not_ready(&loaded, &startup_err, err);
        return Ok(None);
    }
    Ok(Some(loaded))
}

pub(crate) struct BootGatewayParams {
    pub data_dir: Option<PathBuf>,
    pub model: Option<PathBuf>,
    pub known_actors: Vec<ActorId>,
    pub provider_override: Option<Arc<dyn ModelProvider>>,
    pub json_mode: bool,
}

pub(crate) struct BootedGateway {
    pub runtime: GatewayRuntime,
    pub owner_actor: ActorId,
    pub store_dir: PathBuf,
    pub cfg: GatewayConfig,
    pub clock: Arc<SystemClock>,
    pub cues: CueRecorder,
    pub _embed_sidecar: Option<LocalEmbedSidecar>,
}

/// Boots Valve (via `spawn_blocking` so `cedar-policy-symcc` can drive its internal runtime),
/// optional local embedder sidecar, and [`GatewayRuntime`] with shared defaults.
pub(crate) async fn boot_gateway_runtime<D, F>(
    loaded: &LoadedGatewayEnv,
    params: BootGatewayParams,
    wrap_doors: F,
    err: &mut dyn Write,
) -> Result<BootedGateway, String>
where
    D: DoorController + 'static,
    F: FnOnce(ValveDoorController) -> D,
{
    let owner_actor = owner_actor_from_cfg(Some(&loaded.config));
    let valve_cfg = build_valve_config(&loaded.config);
    let mut snapshot = ValveSnapshot {
        owner_actor_id: Some(owner_actor.clone()),
        t_now: SystemClock::new().now(),
        ..ValveSnapshot::default()
    };
    snapshot.known_actors.insert(owner_actor.clone());
    for actor in params.known_actors {
        snapshot.known_actors.insert(actor);
    }

    let approval_secrets =
        load_approval_secrets(Some(&loaded.paths.secrets_path), |k| std::env::var(k).ok())
            .map_err(|e| e.to_string())?;
    let mac_key = resolve_approval_mac_key(&approval_secrets);

    // Build Valve (runs cedar-policy-symcc cvc5 verification) outside the async runtime context.
    let mut valve =
        tokio::task::spawn_blocking(move || Valve::with_default_policies(valve_cfg, snapshot))
            .await
            .map_err(|e| format!("valve policy verification task failed: {e}"))?
            .map_err(|e| format!("valve policy verification failed: {e}"))?;
    valve.set_approval_attestation_key(mac_key);
    let door_controller = wrap_doors(ValveDoorController::new(valve));

    let mut cfg = loaded.config.clone();
    let store_dir = params
        .data_dir
        .unwrap_or_else(|| loaded.paths.data_dir.clone());
    cfg.data_dir = Some(store_dir.clone());
    if let Some(model_override) = params.model {
        cfg.soul.model_path = Some(model_override);
    }
    let embed_sidecar = if params.provider_override.is_none() {
        maybe_start_embed_sidecar(&mut cfg, params.json_mode, err)?
    } else {
        None
    };

    let clock = Arc::new(SystemClock::new());
    let diagnostics = DiagnosticSink::new(&loaded.secrets);
    let cues = CueRecorder::new();

    let runtime = GatewayRuntime::start_segmented_with_configured_core(
        cfg.clone(),
        &loaded.secrets,
        store_dir.clone(),
        door_controller,
        GatewayOptions {
            clock: clock.clone(),
            provider: params.provider_override,
            diagnostics,
            cues: cues.clone(),
            seams: GatewaySeams::default(),
        },
    )
    .await
    .map_err(|e| format!("gateway startup failed: {e}"))?;

    Ok(BootedGateway {
        runtime,
        owner_actor,
        store_dir,
        cfg,
        clock,
        cues,
        _embed_sidecar: embed_sidecar,
    })
}
