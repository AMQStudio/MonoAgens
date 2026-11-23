use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use monoagens_gateway::{
    BodyTransportConfig, ConfigError, GatewayConfig, PluginConfig, PluginKind,
    load_plugin_env_secrets, update_config_with_backup, write_default_config,
};
use serde::Serialize;
use serde_json::json;
use toml_edit::{Array, Item, Table, value};

use crate::common::{load_paths, write_json_pretty};
use crate::{PairArgs, PluginArgs, PluginSubcommand};

#[derive(Debug, Clone, Copy)]
struct BuiltinPluginPreset {
    id: &'static str,
    binary: &'static str,
    kind: PluginKind,
    default_args: &'static [&'static str],
    default_optional_deps: &'static [&'static str],
}

const BUILTIN_PRESETS: &[BuiltinPluginPreset] = &[
    BuiltinPluginPreset {
        id: "telegram",
        binary: "body-telegram",
        kind: PluginKind::Adapter,
        default_args: &["--token-env", "TELEGRAM_BOT_TOKEN"],
        default_optional_deps: &[],
    },
    BuiltinPluginPreset {
        id: "whatsapp",
        binary: "body-whatsapp",
        kind: PluginKind::Adapter,
        default_args: &["--live"],
        default_optional_deps: &[],
    },
    BuiltinPluginPreset {
        id: "cli",
        binary: "body-cli",
        kind: PluginKind::Adapter,
        default_args: &[],
        default_optional_deps: &[],
    },
    BuiltinPluginPreset {
        id: "voice",
        binary: "body-voice",
        kind: PluginKind::Adapter,
        default_args: &[],
        default_optional_deps: &[],
    },
    BuiltinPluginPreset {
        id: "sim-actuator",
        binary: "body-sim-actuator",
        kind: PluginKind::Adapter,
        default_args: &[],
        default_optional_deps: &[],
    },
    BuiltinPluginPreset {
        id: "mcp-bridge",
        binary: "body-mcp-bridge",
        kind: PluginKind::Capability,
        default_args: &[],
        default_optional_deps: &[],
    },
    BuiltinPluginPreset {
        id: "a2a-bridge",
        binary: "plugin-a2a-bridge",
        kind: PluginKind::Capability,
        default_args: &[],
        default_optional_deps: &[],
    },
    BuiltinPluginPreset {
        id: "ros2-bridge",
        binary: "plugin-ros2-bridge",
        kind: PluginKind::Adapter,
        default_args: &[],
        default_optional_deps: &[],
    },
    BuiltinPluginPreset {
        id: "web-search",
        binary: "plugin-web-search",
        kind: PluginKind::Capability,
        default_args: &[],
        default_optional_deps: &[],
    },
    BuiltinPluginPreset {
        id: "workspace",
        binary: "plugin-workspace",
        kind: PluginKind::Capability,
        default_args: &[],
        default_optional_deps: &["rbac"],
    },
    BuiltinPluginPreset {
        id: "pdf-reader",
        binary: "plugin-pdf-reader",
        kind: PluginKind::Capability,
        default_args: &[],
        default_optional_deps: &[],
    },
    BuiltinPluginPreset {
        id: "soul-tuning",
        binary: "plugin-soul-tuning",
        kind: PluginKind::Capability,
        default_args: &[],
        default_optional_deps: &[],
    },
];

#[derive(Debug, Serialize)]
struct ConfiguredPluginSummary {
    id: String,
    kind: &'static str,
    enabled: bool,
    transport: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    command: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    socket_path: Option<String>,
    args: Vec<String>,
    dependencies: Vec<String>,
    optional_dependencies: Vec<String>,
}

#[derive(Debug, Serialize)]
struct CatalogPluginSummary {
    id: &'static str,
    kind: &'static str,
    binary: &'static str,
    discovered_path: Option<String>,
    installed: bool,
}

fn find_builtin_preset(id: &str) -> Option<&'static BuiltinPluginPreset> {
    BUILTIN_PRESETS.iter().find(|p| p.id == id)
}

fn exe_filename(bin_name: &str) -> String {
    format!("{bin_name}{}", std::env::consts::EXE_SUFFIX)
}

fn find_workspace_root() -> Option<PathBuf> {
    if let Ok(cwd) = std::env::current_dir() {
        for dir in cwd.ancestors() {
            let cargo_toml = dir.join("Cargo.toml");
            if cargo_toml.is_file()
                && std::fs::read_to_string(&cargo_toml)
                    .is_ok_and(|contents| contents.contains("[workspace]"))
            {
                return Some(dir.to_path_buf());
            }
        }
    }
    let manifest_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    if manifest_root.join("Cargo.toml").is_file() {
        return manifest_root.canonicalize().ok().or(Some(manifest_root));
    }
    None
}

fn discover_builtin_binary(preset: &BuiltinPluginPreset) -> Option<PathBuf> {
    let file_name = exe_filename(preset.binary);
    let mut candidates = Vec::new();

    if let Ok(current_exe) = std::env::current_exe()
        && let Some(exe_dir) = current_exe.parent()
    {
        candidates.push(exe_dir.join(&file_name));
        if exe_dir.file_name().is_some_and(|n| n == "deps")
            && let Some(parent_dir) = exe_dir.parent()
        {
            candidates.push(parent_dir.join(&file_name));
        }
    }

    if let Some(ws) = find_workspace_root() {
        candidates.push(ws.join("target/release").join(&file_name));
        candidates.push(ws.join("target/debug").join(&file_name));
        if preset.id == "whatsapp" {
            candidates.push(ws.join("plugins/whatsapp/target/release").join(&file_name));
            candidates.push(ws.join("plugins/whatsapp/target/debug").join(&file_name));
        }
    }

    if let Some(path_env) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path_env) {
            candidates.push(dir.join(&file_name));
        }
    }

    candidates.into_iter().find(|p| p.is_file())
}

fn validate_plugin_id(id: &str) -> Result<String, String> {
    let trimmed = id.trim();
    if trimmed.is_empty() {
        return Err("plugin id must not be empty".to_owned());
    }
    if trimmed.contains(char::is_whitespace) || trimmed.contains('.') {
        return Err(format!(
            "invalid plugin id '{trimmed}' (must not contain whitespace or '.')"
        ));
    }
    Ok(trimmed.to_owned())
}

fn plugin_config_to_summary(plugin: &PluginConfig) -> ConfiguredPluginSummary {
    let (transport, command, socket_path, args) = match &plugin.transport {
        BodyTransportConfig::Stdio { command, args } => (
            "stdio",
            Some(command.clone()),
            None,
            args.clone(),
        ),
        BodyTransportConfig::UnixSocket { path } => (
            "unix_socket",
            None,
            Some(path.display().to_string()),
            Vec::new(),
        ),
    };
    ConfiguredPluginSummary {
        id: plugin.id.clone(),
        kind: plugin.kind.as_str(),
        enabled: plugin.enabled,
        transport,
        command,
        socket_path,
        args,
        dependencies: plugin.dependencies.clone(),
        optional_dependencies: plugin.optional_dependencies.clone(),
    }
}

fn build_plugin_table(spec: &PluginConfig) -> Table {
    let mut tbl = Table::new();
    tbl["enabled"] = value(spec.enabled);
    tbl["kind"] = value(spec.kind.as_str());
    match &spec.transport {
        BodyTransportConfig::Stdio { command, args } => {
            tbl["transport"] = value("stdio");
            tbl["command"] = value(command.as_str());
            if !args.is_empty() {
                let mut arr = Array::new();
                for arg in args {
                    arr.push(arg.as_str());
                }
                tbl["args"] = value(arr);
            }
        }
        BodyTransportConfig::UnixSocket { path } => {
            tbl["transport"] = value("unix_socket");
            tbl["socket_path"] = value(path.display().to_string());
        }
    }
    if !spec.dependencies.is_empty() {
        let mut arr = Array::new();
        for dep in &spec.dependencies {
            arr.push(dep.as_str());
        }
        tbl["dependencies"] = value(arr);
    }
    if !spec.optional_dependencies.is_empty() {
        let mut arr = Array::new();
        for dep in &spec.optional_dependencies {
            arr.push(dep.as_str());
        }
        tbl["optional_dependencies"] = value(arr);
    }
    tbl
}

fn ensure_plugin_table_exists(config_path: &Path, id: &str) -> Result<(), String> {
    if !config_path.exists() {
        return Err(format!(
            "config file '{}' does not exist",
            config_path.display()
        ));
    }
    let raw = std::fs::read_to_string(config_path)
        .map_err(|e| format!("failed to read '{}': {e}", config_path.display()))?;
    let doc = raw
        .parse::<toml_edit::DocumentMut>()
        .map_err(|e| format!("invalid TOML in '{}': {e}", config_path.display()))?;
    let exists = doc
        .get("plugins")
        .and_then(Item::as_table_like)
        .and_then(|t| t.get(id))
        .is_some_and(|i| !i.is_none());
    if !exists {
        return Err(format!(
            "plugin '{id}' is not configured in [plugins.{id}] of '{}'",
            config_path.display()
        ));
    }
    Ok(())
}

fn cmd_plugin_list(
    config_path: Option<&Path>,
    json_mode: bool,
    out: &mut dyn Write,
) -> Result<u8, String> {
    let (cfg_opt, paths) = load_paths(config_path)?;
    let configured_raw = cfg_opt
        .as_ref()
        .map(GatewayConfig::effective_plugins)
        .unwrap_or_default();
    let configured: Vec<ConfiguredPluginSummary> = configured_raw
        .iter()
        .map(plugin_config_to_summary)
        .collect();

    let catalog: Vec<CatalogPluginSummary> = BUILTIN_PRESETS
        .iter()
        .map(|preset| CatalogPluginSummary {
            id: preset.id,
            kind: preset.kind.as_str(),
            binary: preset.binary,
            discovered_path: discover_builtin_binary(preset)
                .map(|p| p.display().to_string()),
            installed: configured.iter().any(|c| c.id == preset.id),
        })
        .collect();

    if json_mode {
        write_json_pretty(
            out,
            &json!({
                "config_path": paths.config_path.display().to_string(),
                "configured": configured,
                "catalog": catalog,
            }),
        )?;
        return Ok(0);
    }

    writeln!(out, "config: {}", paths.config_path.display()).map_err(|e| e.to_string())?;
    if configured.is_empty() {
        writeln!(out, "configured plugins: (none)").map_err(|e| e.to_string())?;
    } else {
        writeln!(out, "configured plugins ({}):", configured.len()).map_err(|e| e.to_string())?;
        for p in &configured {
            let target = p
                .command
                .as_deref()
                .or(p.socket_path.as_deref())
                .unwrap_or("");
            writeln!(
                out,
                "  - {} kind={} enabled={} {}={} deps=[{}] opt_deps=[{}]",
                p.id,
                p.kind,
                p.enabled,
                if p.socket_path.is_some() {
                    "socket"
                } else {
                    "command"
                },
                target,
                p.dependencies.join(","),
                p.optional_dependencies.join(","),
            )
            .map_err(|e| e.to_string())?;
        }
    }

    writeln!(out, "built-in catalog ({}):", catalog.len()).map_err(|e| e.to_string())?;
    for item in &catalog {
        let status = if item.installed { "installed" } else { "available" };
        let path_note = item.discovered_path.as_deref().unwrap_or("not built");
        writeln!(
            out,
            "  - {} kind={} bin={} status={} path={}",
            item.id, item.kind, item.binary, status, path_note
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(0)
}

#[allow(clippy::too_many_arguments)]
fn cmd_plugin_install(
    config_path: Option<&Path>,
    json_mode: bool,
    names: Vec<String>,
    command: Option<String>,
    kind: Option<String>,
    args: Vec<String>,
    deps: Vec<String>,
    opt_deps: Vec<String>,
    socket: Option<PathBuf>,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> Result<u8, String> {
    if names.is_empty() {
        return Err("provide at least one plugin id to install".to_owned());
    }
    if command.is_some() && socket.is_some() {
        return Err("cannot specify both --command and --socket".to_owned());
    }
    if names.len() > 1 && (command.is_some() || socket.is_some()) {
        return Err(
            "--command and --socket can only be used when installing a single plugin".to_owned(),
        );
    }

    let parsed_kind = match kind {
        Some(raw) => Some(PluginKind::parse(&raw).ok_or_else(|| {
            format!("invalid --kind '{raw}' (expected 'adapter' or 'capability')")
        })?),
        None => None,
    };

    let (_, paths) = load_paths(config_path)?;
    let mut specs = Vec::with_capacity(names.len());

    for raw_name in names {
        let id = validate_plugin_id(&raw_name)?;
        let preset = find_builtin_preset(&id);

        if preset.is_none() && command.is_none() && socket.is_none() {
            let known: Vec<&str> = BUILTIN_PRESETS.iter().map(|p| p.id).collect();
            return Err(format!(
                "unknown built-in plugin '{id}' (known presets: {}); pass --command <path> or --socket <path> for custom plugins",
                known.join(", ")
            ));
        }

        let resolved_kind = parsed_kind
            .or_else(|| preset.map(|p| p.kind))
            .unwrap_or(PluginKind::Adapter);

        let resolved_args = if !args.is_empty() {
            args.clone()
        } else if let Some(p) = preset {
            p.default_args.iter().map(|s| (*s).to_owned()).collect()
        } else {
            Vec::new()
        };

        let resolved_deps = deps.clone();
        let resolved_opt_deps = if !opt_deps.is_empty() {
            opt_deps.clone()
        } else if let Some(p) = preset {
            p.default_optional_deps
                .iter()
                .map(|s| (*s).to_owned())
                .collect()
        } else {
            Vec::new()
        };

        let transport = if let Some(sock) = &socket {
            BodyTransportConfig::UnixSocket { path: sock.clone() }
        } else if let Some(cmd) = &command {
            let trimmed = cmd.trim();
            if trimmed.is_empty() {
                return Err("--command must not be empty".to_owned());
            }
            BodyTransportConfig::Stdio {
                command: trimmed.to_owned(),
                args: resolved_args,
            }
        } else {
            let p = preset.expect("checked preset is Some when command and socket are None");
            let cmd = match discover_builtin_binary(p) {
                Some(path) => path.display().to_string(),
                None => {
                    if !json_mode {
                        let _ = writeln!(
                            err,
                            "[warn] built-in binary '{}' not found near current_exe or target/; recording command = '{}'",
                            p.binary, p.binary
                        );
                    }
                    p.binary.to_owned()
                }
            };
            BodyTransportConfig::Stdio {
                command: cmd,
                args: resolved_args,
            }
        };

        specs.push(PluginConfig {
            id,
            enabled: true,
            kind: resolved_kind,
            transport,
            dependencies: resolved_deps,
            optional_dependencies: resolved_opt_deps,
        });
    }

    if !paths.config_path.exists() {
        write_default_config(&paths.config_path, &GatewayConfig::default())
            .map_err(|e| e.to_string())?;
    }

    let specs_for_update = specs.clone();
    update_config_with_backup(&paths.config_path, &paths.secrets_path, |doc| {
        let plugins_item = doc.entry("plugins").or_insert_with(|| {
            let mut t = Table::new();
            t.set_implicit(true);
            Item::Table(t)
        });
        let plugins_tbl = plugins_item
            .as_table_mut()
            .ok_or_else(|| ConfigError::InvalidType {
                key: "plugins".to_owned(),
                expected: "table [plugins]",
            })?;
        for spec in &specs_for_update {
            plugins_tbl.insert(&spec.id, Item::Table(build_plugin_table(spec)));
        }
        Ok(())
    })
    .map_err(|e| e.to_string())?;

    if json_mode {
        let summaries: Vec<ConfiguredPluginSummary> =
            specs.iter().map(plugin_config_to_summary).collect();
        write_json_pretty(
            out,
            &json!({
                "status": "ok",
                "config_path": paths.config_path.display().to_string(),
                "installed": summaries,
            }),
        )?;
    } else {
        for spec in &specs {
            writeln!(
                out,
                "installed plugin '{}' (kind={}) into {}",
                spec.id,
                spec.kind.as_str(),
                paths.config_path.display()
            )
            .map_err(|e| e.to_string())?;
        }
    }
    Ok(0)
}

fn cmd_plugin_set_enabled(
    config_path: Option<&Path>,
    json_mode: bool,
    raw_id: String,
    enabled: bool,
    out: &mut dyn Write,
) -> Result<u8, String> {
    let id = validate_plugin_id(&raw_id)?;
    let (_, paths) = load_paths(config_path)?;
    ensure_plugin_table_exists(&paths.config_path, &id)?;

    let id_clone = id.clone();
    update_config_with_backup(&paths.config_path, &paths.secrets_path, |doc| {
        let plugins_tbl = doc
            .get_mut("plugins")
            .and_then(Item::as_table_mut)
            .ok_or_else(|| ConfigError::MissingField {
                key: format!("plugins.{id_clone}"),
            })?;
        let plugin_item = plugins_tbl.get_mut(&id_clone).ok_or_else(|| {
            ConfigError::MissingField {
                key: format!("plugins.{id_clone}"),
            }
        })?;
        let tbl = plugin_item
            .as_table_like_mut()
            .ok_or_else(|| ConfigError::InvalidType {
                key: format!("plugins.{id_clone}"),
                expected: "table [plugins.<id>]",
            })?;
        tbl.insert("enabled", value(enabled));
        Ok(())
    })
    .map_err(|e| e.to_string())?;

    if json_mode {
        write_json_pretty(
            out,
            &json!({
                "status": "ok",
                "id": id,
                "enabled": enabled,
                "config_path": paths.config_path.display().to_string(),
            }),
        )?;
    } else {
        writeln!(
            out,
            "{} plugin '{}' in {}",
            if enabled { "enabled" } else { "disabled" },
            id,
            paths.config_path.display()
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(0)
}

fn cmd_plugin_remove(
    config_path: Option<&Path>,
    json_mode: bool,
    raw_id: String,
    out: &mut dyn Write,
) -> Result<u8, String> {
    let id = validate_plugin_id(&raw_id)?;
    let (_, paths) = load_paths(config_path)?;
    ensure_plugin_table_exists(&paths.config_path, &id)?;

    let id_clone = id.clone();
    update_config_with_backup(&paths.config_path, &paths.secrets_path, |doc| {
        let plugins_tbl = doc
            .get_mut("plugins")
            .and_then(Item::as_table_mut)
            .ok_or_else(|| ConfigError::MissingField {
                key: format!("plugins.{id_clone}"),
            })?;
        plugins_tbl.remove(&id_clone);
        if plugins_tbl.is_empty() {
            plugins_tbl.set_implicit(true);
        }
        Ok(())
    })
    .map_err(|e| e.to_string())?;

    if json_mode {
        write_json_pretty(
            out,
            &json!({
                "status": "ok",
                "id": id,
                "removed": true,
                "config_path": paths.config_path.display().to_string(),
            }),
        )?;
    } else {
        writeln!(
            out,
            "removed plugin '{}' from {}",
            id,
            paths.config_path.display()
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(0)
}

pub(crate) fn cmd_plugin(
    config_path: Option<&Path>,
    json_mode: bool,
    args: PluginArgs,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> Result<u8, String> {
    match args.action {
        PluginSubcommand::List => cmd_plugin_list(config_path, json_mode, out),
        PluginSubcommand::Install {
            names,
            command,
            kind,
            args,
            deps,
            opt_deps,
            socket,
        } => cmd_plugin_install(
            config_path,
            json_mode,
            names,
            command,
            kind,
            args,
            deps,
            opt_deps,
            socket,
            out,
            err,
        ),
        PluginSubcommand::Enable { id } => {
            cmd_plugin_set_enabled(config_path, json_mode, id, true, out)
        }
        PluginSubcommand::Disable { id } => {
            cmd_plugin_set_enabled(config_path, json_mode, id, false, out)
        }
        PluginSubcommand::Remove { id } => cmd_plugin_remove(config_path, json_mode, id, out),
    }
}

pub(crate) fn cmd_pair(config_path: Option<&Path>, args: PairArgs) -> Result<u8, String> {
    let plugin_id = validate_plugin_id(&args.plugin)?;
    let (cfg_opt, paths) = load_paths(config_path)?;

    let configured_plugin = cfg_opt
        .as_ref()
        .and_then(|cfg| cfg.effective_plugins().into_iter().find(|p| p.id == plugin_id));

    let binary = if let Some(plugin) = configured_plugin {
        match plugin.transport {
            BodyTransportConfig::Stdio { command, .. } => command,
            BodyTransportConfig::UnixSocket { path } => {
                return Err(format!(
                    "plugin '{plugin_id}' uses unix_socket ({}) and has no CLI binary for pairing",
                    path.display()
                ));
            }
        }
    } else if let Some(preset) = find_builtin_preset(&plugin_id) {
        discover_builtin_binary(preset)
            .ok_or_else(|| {
                format!(
                    "built-in binary '{}' for plugin '{plugin_id}' was not found; build it or install with `monoagens plugin install {plugin_id} --command <path>`",
                    preset.binary
                )
            })?
            .display()
            .to_string()
    } else {
        return Err(format!(
            "plugin '{plugin_id}' is not configured in monoagens.toml and is not a built-in preset"
        ));
    };

    let config_dir = paths
        .config_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let plugin_config = config_dir.join("plugins").join(format!("{plugin_id}.toml"));
    let state_dir = paths.data_dir.join("plugins").join(&plugin_id);
    std::fs::create_dir_all(&state_dir).map_err(|e| {
        format!(
            "failed to create plugin state directory '{}': {e}",
            state_dir.display()
        )
    })?;

    let db_path = args.db.unwrap_or_else(|| state_dir.join("session.db"));
    let plugin_env = load_plugin_env_secrets(&paths.secrets_path).map_err(|e| e.to_string())?;

    let mut cmd = Command::new(&binary);
    cmd.arg("pair").arg("--db").arg(&db_path);
    if let Some(phone) = &args.phone {
        cmd.arg("--phone").arg(phone);
    }
    if !args.extra_args.is_empty() {
        cmd.args(&args.extra_args);
    }
    cmd.env("MONOAGENS_PLUGIN_CONFIG", &plugin_config);
    cmd.env("MONOAGENS_PLUGIN_STATE_DIR", &state_dir);
    cmd.env("MONOAGENS_BODY_STATE_DIR", &state_dir);
    for (key, secret) in &plugin_env {
        let value = secret.expose();
        if !value.is_empty() && std::env::var_os(key).is_none() {
            cmd.env(key, value);
        }
    }
    cmd.stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());

    let status = cmd
        .status()
        .map_err(|e| format!("failed to launch '{binary}' for pairing: {e}"))?;

    Ok(match status.code() {
        Some(code) => u8::try_from(code.clamp(0, 255)).unwrap_or(1),
        None => 1,
    })
}
