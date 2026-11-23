//! `/doctor`: independent health checks with a fix hint each. No model call is ever made; the
//! provider check only opens a TCP connection to the configured endpoint (and is skipped for
//! the `scripted` provider).

use std::fmt::Write as _;
use std::net::{TcpStream, ToSocketAddrs};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use monoagens_gateway::{
    BodyTransportConfig, EmbedBackendKind, GatewayConfig,
    ModelProviderKind, ResolvedPaths, load_config_file,
    read_data_dir_lock_pid,
};

/// Outcome of one check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DoctorCheck {
    pub name: String,
    pub status: DoctorStatus,
    pub detail: String,
    /// What to do when the check failed (empty otherwise).
    pub hint: String,
}

/// Pass, fail or skipped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DoctorStatus {
    Pass,
    Fail,
    Skipped,
}

/// What `/doctor` needs to know about this process.
#[derive(Debug, Clone)]
pub struct DoctorInput {
    pub paths: ResolvedPaths,
    /// `hosting` or `attached` (how this TUI reaches the gateway), `none` before boot.
    pub link: String,
}

fn check(name: &str, status: DoctorStatus, detail: impl Into<String>, hint: &str) -> DoctorCheck {
    DoctorCheck {
        name: name.to_owned(),
        status,
        detail: detail.into(),
        hint: hint.to_owned(),
    }
}

/// Runs every check; one failing check never stops the others.
#[must_use]
pub fn run_doctor(input: &DoctorInput) -> Vec<DoctorCheck> {
    let mut out = Vec::new();
    let cfg = match load_config_file(&input.paths.config_path, &input.paths.secrets_path) {
        Ok(cfg) => {
            out.push(check(
                "config",
                DoctorStatus::Pass,
                format!("{} parses and validates", input.paths.config_path.display()),
                "",
            ));
            Some(cfg)
        }
        Err(e) => {
            out.push(check(
                "config",
                DoctorStatus::Fail,
                e.to_string(),
                "fix the value in /settings or edit the file, then /retry",
            ));
            None
        }
    };
    let provider_kind = cfg.as_ref().map(|c| c.model.provider);
    out.push(check_secrets(&input.paths.secrets_path, provider_kind));
    out.push(check_provider(cfg.as_ref()));
    out.extend(check_plugins(cfg.as_ref()));
    out.push(check_embedder(cfg.as_ref()));
    out.push(check_data_dir(&input.paths.data_dir));
    out.push(check_lock(&input.paths.data_dir, &input.link));
    out
}

fn check_secrets(path: &Path, provider: Option<ModelProviderKind>) -> DoctorCheck {
    match std::fs::metadata(path) {
        Ok(meta) => {
            let mode = meta.permissions().mode() & 0o777;
            if mode == 0o600 {
                check("secrets", DoctorStatus::Pass, format!("{} (mode 0600)", path.display()), "")
            } else {
                check(
                    "secrets",
                    DoctorStatus::Fail,
                    format!("{} has mode {mode:04o}", path.display()),
                    &format!("run: chmod 600 {}", path.display()),
                )
            }
        }
        Err(_) if provider == Some(ModelProviderKind::Scripted) => check(
            "secrets",
            DoctorStatus::Pass,
            "no secrets file (not needed for the scripted provider)",
            "",
        ),
        Err(_) => check(
            "secrets",
            DoctorStatus::Fail,
            format!("{} not found", path.display()),
            "log in with /provider (writes a per-provider key to secrets.toml)",
        ),
    }
}

fn check_provider(cfg: Option<&monoagens_gateway::GatewayConfig>) -> DoctorCheck {
    let Some(cfg) = cfg else {
        return check("provider", DoctorStatus::Skipped, "configuration is invalid", "");
    };
    if cfg.model.provider == ModelProviderKind::Scripted {
        return check("provider", DoctorStatus::Skipped, "scripted provider (no network)", "");
    }
    let entry = cfg
        .model
        .catalog_provider
        .as_deref()
        .and_then(monoagens_reason::model::catalog::find);
    let Some(entry) = entry else {
        return check(
            "provider",
            DoctorStatus::Fail,
            "no provider selected",
            "run /provider to log in, then /model to pick a model",
        );
    };
    let endpoint = entry.effective_base_url(cfg.model.provider_base_url.as_deref());
    let Some((host, port)) = host_port(&endpoint) else {
        return check(
            "provider",
            DoctorStatus::Fail,
            format!("cannot read a host from endpoint '{endpoint}'"),
            "check the provider base URL in /provider",
        );
    };
    let addr = (host.as_str(), port).to_socket_addrs().ok().and_then(|mut a| a.next());
    let reachable = addr
        .is_some_and(|a| TcpStream::connect_timeout(&a, Duration::from_secs(3)).is_ok());
    if reachable {
        check("provider", DoctorStatus::Pass, format!("{host}:{port} is reachable"), "")
    } else {
        check(
            "provider",
            DoctorStatus::Fail,
            format!("cannot connect to {host}:{port}"),
            "check the network, the provider base URL, or a proxy",
        )
    }
}

/// Host and port of an `http(s)://` endpoint.
fn host_port(endpoint: &str) -> Option<(String, u16)> {
    let (scheme, rest) = endpoint.split_once("://")?;
    let authority = rest.split(['/', '?', '#']).next()?;
    let authority = authority.rsplit('@').next()?;
    let default_port = if scheme.eq_ignore_ascii_case("https") { 443 } else { 80 };
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) if p.parse::<u16>().is_ok() => (h, p.parse().ok()?),
        _ => (authority, default_port),
    };
    let host = host.trim_matches(['[', ']']);
    (!host.is_empty()).then(|| (host.to_owned(), port))
}

fn resolve_command(command: &str) -> Option<PathBuf> {
    let p = Path::new(command);
    if p.components().count() > 1 || p.is_absolute() {
        return p.exists().then(|| p.to_path_buf());
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|d| d.join(command))
        .find(|c| c.is_file())
}

fn check_plugins(cfg: Option<&GatewayConfig>) -> Vec<DoctorCheck> {
    let Some(cfg) = cfg else {
        return vec![check("plugins", DoctorStatus::Skipped, "configuration is invalid", "")];
    };
    let enabled: Vec<_> = cfg.effective_plugins().into_iter().filter(|p| p.enabled).collect();
    if enabled.is_empty() {
        return vec![check("plugins", DoctorStatus::Pass, "no enabled plugins", "")];
    }
    enabled
        .into_iter()
        .map(|p| {
            let name = format!("plugin:{}", p.id);
            match p.transport {
                BodyTransportConfig::Stdio { command, .. } => match resolve_command(&command) {
                    Some(path) => {
                        let exec = std::fs::metadata(&path)
                            .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0);
                        if exec {
                            check(&name, DoctorStatus::Pass, format!("{} is executable", path.display()), "")
                        } else {
                            check(
                                &name,
                                DoctorStatus::Fail,
                                format!("{} is not an executable file", path.display()),
                                &format!("run: chmod +x {}", path.display()),
                            )
                        }
                    }
                    None => check(
                        &name,
                        DoctorStatus::Fail,
                        format!("binary '{command}' not found"),
                        "build or install the plugin, or fix its command in /plugins",
                    ),
                },
                BodyTransportConfig::UnixSocket { path } => {
                    if path.exists() {
                        check(&name, DoctorStatus::Pass, format!("socket {} exists", path.display()), "")
                    } else {
                        check(
                            &name,
                            DoctorStatus::Fail,
                            format!("socket {} does not exist", path.display()),
                            "start the plugin that serves this socket",
                        )
                    }
                }
            }
        })
        .collect()
}

fn check_embedder(cfg: Option<&GatewayConfig>) -> DoctorCheck {
    let Some(cfg) = cfg else {
        return check("embedder", DoctorStatus::Skipped, "configuration is invalid", "");
    };
    match cfg.embed.backend {
        EmbedBackendKind::None | EmbedBackendKind::Hash | EmbedBackendKind::OpenAiCompatible => check(
            "embedder",
            DoctorStatus::Skipped,
            format!("backend '{}' needs no local files", cfg.embed.backend.as_str()),
            "",
        ),
        _ => match &cfg.embed.model_path {
            Some(p) if p.exists() => {
                check("embedder", DoctorStatus::Pass, format!("{} exists", p.display()), "")
            }
            Some(p) => check(
                "embedder",
                DoctorStatus::Fail,
                format!("{} does not exist", p.display()),
                "fix embed.model_path in /settings -> Soul & Brain",
            ),
            None => check(
                "embedder",
                DoctorStatus::Pass,
                "default embedder model location (checked when it starts)",
                "",
            ),
        },
    }
}

fn check_data_dir(dir: &Path) -> DoctorCheck {
    let probe = dir.join(".doctor-probe");
    let res = std::fs::create_dir_all(dir)
        .and_then(|()| std::fs::write(&probe, b"ok"))
        .and_then(|()| std::fs::remove_file(&probe));
    match res {
        Ok(()) => check("data dir", DoctorStatus::Pass, format!("{} is writable", dir.display()), ""),
        Err(e) => check(
            "data dir",
            DoctorStatus::Fail,
            format!("{}: {e}", dir.display()),
            "fix the directory permissions or set data_dir in monoagens.toml",
        ),
    }
}

fn check_lock(data_dir: &Path, link: &str) -> DoctorCheck {
    let pid = read_data_dir_lock_pid(data_dir);
    let me = std::process::id();
    match (pid, link) {
        (Some(p), "hosting") if p == me => {
            check("lock", DoctorStatus::Pass, format!("this TUI hosts the gateway (pid {p})"), "")
        }
        (Some(p), "attached") => {
            check("lock", DoctorStatus::Pass, format!("attached to the gateway hosted by pid {p}"), "")
        }
        (Some(p), _) => check(
            "lock",
            DoctorStatus::Pass,
            format!("gateway lock last taken by pid {p} (link: {link})"),
            "",
        ),
        (None, _) => check("lock", DoctorStatus::Pass, "no gateway lock holder recorded", ""),
    }
}

/// Plain-text report, one line per check plus an indented hint for failures.
#[must_use]
pub fn format_report(checks: &[DoctorCheck]) -> String {
    let mut out = String::from("Doctor report");
    for c in checks {
        let tag = match c.status {
            DoctorStatus::Pass => "[ok]",
            DoctorStatus::Fail => "[FAIL]",
            DoctorStatus::Skipped => "[skip]",
        };
        let _ = write!(out, "\n{tag} {}: {}", c.name, c.detail);
        if c.status == DoctorStatus::Fail && !c.hint.is_empty() {
            let _ = write!(out, "\n      fix: {}", c.hint);
        }
    }
    out
}
