use std::io::{BufRead, BufReader as StdBufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use monoagens_gateway::{
    EmbedBackendKind, GatewayConfig, LoadedGatewayEnv, ResolvedPaths, resolve_paths,
};
use monoagens_protocol::ActorId;
use monoagens_store::{SegmentedStore, StoreConfig};
use monoagens_valve::{ValveConfig, ValveMode};
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

pub(crate) struct CliSidecarGuard {
    child: Child,
}

impl Drop for CliSidecarGuard {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
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
) -> Result<Option<CliSidecarGuard>, String> {
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
    if !script.is_file() {
        return Err(format!(
            "embeddinggemma2_http sidecar script not found at '{}'",
            script.display()
        ));
    }

    let model_dir = cfg
        .embed
        .model_path
        .clone()
        .unwrap_or_else(|| ws_root.join("lab/datasets/embedder_models/embeddinggemma-2"));
    if !model_dir.is_dir() {
        return Err(format!(
            "embeddinggemma2_http requires embed.model_path pointing to an existing directory (missing '{}')",
            model_dir.display()
        ));
    }
    cfg.embed.model_path = Some(model_dir.clone());

    let py = std::env::var("MONOAGENS_ADAPTER_PYTHON").unwrap_or_else(|_| {
        for cand in [
            ws_root.join("lab/adapters/.venv/bin/python"),
            ws_root.join("training/.venv/bin/python"),
        ] {
            if cand.is_file() {
                return cand.to_string_lossy().into_owned();
            }
        }
        "python3".to_owned()
    });

    if !json_mode {
        let _ = writeln!(
            err,
            "[embedder] memuat EmbeddingGemma 2 dari {} ...",
            model_dir.display()
        );
        let _ = err.flush();
    }

    let mut child = Command::new(&py)
        .arg(&script)
        .arg("--model-path")
        .arg(&model_dir)
        .arg("--port")
        .arg("0")
        .env("HF_HUB_OFFLINE", "1")
        .env("TRANSFORMERS_OFFLINE", "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("failed to spawn EmbeddingGemma 2 sidecar via '{py}': {e}"))?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "EmbeddingGemma 2 sidecar stdout unavailable".to_owned())?;
    let mut reader = StdBufReader::new(stdout);
    let mut first_line = String::new();
    reader
        .read_line(&mut first_line)
        .map_err(|e| format!("failed reading EmbeddingGemma 2 sidecar readiness line: {e}"))?;

    if !first_line.starts_with("READY ") {
        let _ = child.kill();
        let out = child.wait_with_output().ok();
        let stderr_msg = out
            .map(|o| String::from_utf8_lossy(&o.stderr).into_owned())
            .unwrap_or_default();
        return Err(format!(
            "EmbeddingGemma 2 sidecar failed to start (stdout='{}', stderr='{}')",
            first_line.trim(),
            stderr_msg.trim()
        ));
    }

    let rest = first_line.trim().strip_prefix("READY ").unwrap_or("");
    let mut parts = rest.splitn(2, ' ');
    let url = parts.next().unwrap_or("").trim().to_owned();
    let device_info = parts.next().unwrap_or("unknown").trim().to_owned();
    if url.is_empty() {
        let _ = child.kill();
        return Err("EmbeddingGemma 2 sidecar returned empty URL".to_owned());
    }
    cfg.embed.endpoint_url = Some(url.clone());
    if !json_mode {
        let _ = writeln!(err, "[embedder] EmbeddingGemma 2 siap ({device_info}) di {url}");
        let _ = err.flush();
    }
    Ok(Some(CliSidecarGuard { child }))
}
