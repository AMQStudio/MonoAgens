//! Adapter readiness: can this plugin start at all?
//!
//! A plugin MAY provide a `status --json` subcommand printing
//! `{"ready": bool, "needs": [..], "detail": ".."}` where every `needs` item is `pairing` or
//! `secret:<ENV_NAME>`. Plugins without it are judged by the secrets their arguments name
//! (`--token-env NAME`, `--*-env=NAME`). The gateway stays platform-neutral: it only runs the
//! check and reads the result; it never learns what "pairing" means.

use std::process::Stdio;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::io::AsyncReadExt;
use tokio::process::Command;

use crate::config::{BodyTransportConfig, PluginConfig, PluginKind};
use crate::plugins::{PluginEnvContext, build_plugin_env};

/// How long a readiness check may run before the argument-based fallback is used.
pub const READINESS_TIMEOUT: Duration = Duration::from_secs(5);

/// Result of a readiness check.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Readiness {
    pub ready: bool,
    #[serde(default)]
    pub needs: Vec<String>,
    #[serde(default)]
    pub detail: String,
}

impl Readiness {
    fn ok() -> Self {
        Self { ready: true, needs: Vec::new(), detail: String::new() }
    }

    /// Human text of what is missing: `needs pairing`, `needs TELEGRAM_BOT_TOKEN`.
    #[must_use]
    pub fn reason(&self) -> String {
        describe_needs(&self.needs)
    }
}

/// `needs pairing`, `needs NAME`, or both joined with `, `.
#[must_use]
pub fn describe_needs(needs: &[String]) -> String {
    let parts: Vec<String> = needs
        .iter()
        .map(|n| n.strip_prefix("secret:").unwrap_or(n).to_owned())
        .collect();
    if parts.is_empty() {
        "not ready".to_owned()
    } else {
        format!("needs {}", parts.join(", "))
    }
}

/// Environment variable names an adapter's arguments say it reads secrets from.
#[must_use]
pub fn env_names_from_args(args: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let mut iter = args.iter().peekable();
    while let Some(a) = iter.next() {
        if let Some((flag, value)) = a.split_once('=') {
            if flag.starts_with("--") && flag.ends_with("-env") && !value.is_empty() {
                out.push(value.to_owned());
            }
        } else if a.starts_with("--")
            && a.ends_with("-env")
            && let Some(v) = iter.peek()
            && !v.starts_with('-')
        {
            out.push((*v).clone());
        }
    }
    out
}

fn fallback(config: &PluginConfig, env_ctx: &PluginEnvContext) -> Readiness {
    let BodyTransportConfig::Stdio { args, .. } = &config.transport else {
        return Readiness::ok();
    };
    let provided = build_plugin_env(&config.id, env_ctx);
    let mut needs = Vec::new();
    for name in env_names_from_args(args) {
        let set = std::env::var(&name).is_ok_and(|v| !v.trim().is_empty())
            || provided.iter().any(|(k, v)| *k == name && !v.trim().is_empty());
        if !set {
            needs.push(format!("secret:{name}"));
        }
    }
    Readiness { ready: needs.is_empty(), detail: String::new(), needs }
}

/// Runs the plugin's readiness check. Only stdio adapters are checked; anything else is
/// considered ready.
pub async fn check_readiness(config: &PluginConfig, env_ctx: &PluginEnvContext) -> Readiness {
    let BodyTransportConfig::Stdio { command, args } = &config.transport else {
        return Readiness::ok();
    };
    if config.kind != PluginKind::Adapter {
        return Readiness::ok();
    }
    match run_status(config, command, args, env_ctx).await {
        Some(r) => r,
        None => fallback(config, env_ctx),
    }
}

async fn run_status(
    config: &PluginConfig,
    command: &str,
    args: &[String],
    env_ctx: &PluginEnvContext,
) -> Option<Readiness> {
    let mut cmd = Command::new(command);
    cmd.arg("status")
        .arg("--json")
        .args(args)
        .envs(build_plugin_env(&config.id, env_ctx))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let mut child = cmd.spawn().ok()?;
    let mut stdout = child.stdout.take()?;
    let run = async {
        let mut buf = Vec::new();
        stdout.read_to_end(&mut buf).await.ok()?;
        let status = child.wait().await.ok()?;
        Some((status.success(), buf))
    };
    let (success, buf) = tokio::time::timeout(READINESS_TIMEOUT, run).await.ok()??;
    if !success {
        return None;
    }
    let text = String::from_utf8_lossy(&buf);
    let line = text.lines().rev().find(|l| l.trim_start().starts_with('{'))?;
    serde_json::from_str::<Readiness>(line.trim()).ok()
}
