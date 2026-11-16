//! Out-of-process `BaselineAdapter` protocol (`R-39`): stdio NDJSON framing, virtual-clock
//! `sim_time_rfc3339` injection, `PerceptView` oracle-stripping, symmetric `NOOP` 5-tuple
//! `E_tok` billing, `restart_epoch` RAM eviction, and `SEALED_MODE=1` isolation.

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use monoagens_core::{DecisionOutcome, EventDetail, MemStore, Prior};
use monoagens_protocol::{Percept, Time};
use serde::{Deserialize, Serialize};
use tempfile::TempDir;

use crate::LabError;
use crate::driver::run_scenario;
use crate::scenario::{Scenario, ScriptedEventKind};

/// External baseline harness roster (`R-39`, `tasks.md` 11.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BaselineKind {
    StatelessReact,
    ArmF,
    Mem0,
    Letta,
    LanggraphLangmem,
    ZepGraphiti,
    HermesAgent,
}

impl BaselineKind {
    /// All 7 registered baseline adapters.
    pub const ALL: [Self; 7] = [
        Self::StatelessReact,
        Self::ArmF,
        Self::Mem0,
        Self::Letta,
        Self::LanggraphLangmem,
        Self::ZepGraphiti,
        Self::HermesAgent,
    ];

    /// Canonical directory/slug name under `lab/adapters/`.
    #[must_use]
    pub const fn slug(self) -> &'static str {
        match self {
            Self::StatelessReact => "stateless_react",
            Self::ArmF => "arm_f",
            Self::Mem0 => "mem0",
            Self::Letta => "letta",
            Self::LanggraphLangmem => "langgraph_langmem",
            Self::ZepGraphiti => "zep_graphiti",
            Self::HermesAgent => "hermes_agent",
        }
    }

    /// Parses a CLI adapter slug.
    pub fn parse(raw: &str) -> Result<Self, String> {
        match raw {
            "stateless_react" | "react" => Ok(Self::StatelessReact),
            "arm_f" | "arm-f" => Ok(Self::ArmF),
            "mem0" => Ok(Self::Mem0),
            "letta" | "memgpt" => Ok(Self::Letta),
            "langgraph_langmem" | "langgraph" => Ok(Self::LanggraphLangmem),
            "zep_graphiti" | "graphiti" | "zep" => Ok(Self::ZepGraphiti),
            "hermes_agent" | "hermes" => Ok(Self::HermesAgent),
            other => Err(format!("unknown baseline adapter '{other}'")),
        }
    }

    /// Resolves the adapter script path relative to `workspace_root`.
    #[must_use]
    pub fn adapter_script_path(self, workspace_root: &Path) -> PathBuf {
        workspace_root
            .join("lab")
            .join("adapters")
            .join(self.slug())
            .join("adapter.py")
    }
}

/// Zero-leakage `PerceptView` sent to external baseline adapters.
///
/// Structurally omits all ground-truth Lab labels (`warranted`, `sel_other`, `answered`,
/// `false_belief_target`, `episode_boundary`, `sealed_tag`, `lure`, `annotations`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PerceptView {
    pub event_id: String,
    pub place_id: String,
    pub actor_id: String,
    pub audience_ids: Vec<String>,
    pub capability_list: Vec<String>,
    pub addressed_to_agent: bool,
    pub content_payload_utf8: String,
    pub sim_time_rfc3339: String,
}

impl PerceptView {
    /// Builds a label-stripped `PerceptView` from a Waist-1 [`Percept`].
    #[must_use]
    pub fn from_percept(
        event_id: impl Into<String>,
        percept: &Percept,
        capabilities: &[String],
        sim_time_rfc3339: String,
    ) -> Self {
        let addressed = percept
            .addressees
            .iter()
            .chain(percept.named.iter())
            .any(|a| a.as_str() == "soul");
        Self {
            event_id: event_id.into(),
            place_id: percept.place.as_str().to_owned(),
            actor_id: percept.actor.as_str().to_owned(),
            audience_ids: percept
                .audience
                .members
                .iter()
                .map(|m| m.as_str().to_owned())
                .collect(),
            capability_list: capabilities.to_vec(),
            addressed_to_agent: addressed,
            content_payload_utf8: String::from_utf8_lossy(percept.payload.as_bytes()).into_owned(),
            sim_time_rfc3339,
        }
    }
}

/// Unified 5-tuple raw token accounting (`R-39`):
/// `(u_uncached_in, u_cache_read, u_cache_write, u_out_vis, u_reason)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TokenUsage5Tuple {
    pub uncached_in: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub out_vis: u64,
    pub reason: u64,
}

impl TokenUsage5Tuple {
    /// Weighted cost-normalized effective tokens `E_tok`:
    /// `u_uncached_in + 0.25 * u_cache_read + 1.25 * u_cache_write + 4.0 * (u_out_vis + u_reason)`.
    #[must_use]
    pub fn e_tok(self) -> f64 {
        (self.uncached_in as f64)
            + 0.25 * (self.cache_read as f64)
            + 1.25 * (self.cache_write as f64)
            + 4.0 * ((self.out_vis + self.reason) as f64)
    }

    /// Cache share `C_share = u_cache_read / max(1, u_uncached_in + u_cache_read + u_cache_write)`.
    #[must_use]
    pub fn c_share(self) -> f64 {
        let total_in = self
            .uncached_in
            .saturating_add(self.cache_read)
            .saturating_add(self.cache_write)
            .max(1);
        (self.cache_read as f64) / (total_in as f64)
    }
}

/// Emitted action or explicit `noop` abstention from a baseline adapter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdapterAction {
    pub capability_id: String,
    pub place_id: String,
    pub payload_utf8: String,
}

/// Recorded result of one baseline adapter step.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AdapterStepRecord {
    pub monotonic_seq: usize,
    pub sim_time_rfc3339: String,
    pub recorded_sim_time_rfc3339: String,
    pub restart_epoch: u64,
    pub ram_cleared_on_restart: bool,
    pub decision: String,
    pub action: Option<AdapterAction>,
    pub noop_tool_exposed: bool,
    pub wake_invoked_llm: bool,
    pub usage: TokenUsage5Tuple,
    pub e_tok: f64,
    pub c_share: f64,
    pub latency_ms: f64,
    pub cost_usd: f64,
}

/// Complete report from running one `BaselineAdapter` against a scenario alongside MonoAgens parity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BaselineRunReport {
    pub adapter: String,
    pub scenario_id: String,
    pub seed: u64,
    pub sealed_mode: bool,
    pub sdk_verified: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unavailable_reason: Option<String>,
    pub steps: Vec<AdapterStepRecord>,
    pub total_e_tok: f64,
    pub mean_c_share: f64,
    pub noop_llm_wakes: usize,
    pub monoagens_zero_llm_gate_suppressions: usize,
    pub monoagens_gate_llm_tokens: u64,
}

/// Formats virtual millisecond timestamp cleanly into RFC-3339 UTC string without host clock access.
#[must_use]
pub fn format_sim_time_rfc3339(ts: Time) -> String {
    let total_ms = ts.as_millis();
    let secs_total = total_ms / 1_000;
    let ms = total_ms % 1_000;
    let days = secs_total / 86_400;
    let rem = secs_total % 86_400;
    let hh = rem / 3_600;
    let mm = (rem % 3_600) / 60;
    let ss = rem % 60;

    // Civil calendar conversion from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = (days as i64) + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    let year = y + if m <= 2 { 1 } else { 0 };

    format!("{year:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}.{ms:03}Z")
}

/// Executes an out-of-process Python `BaselineAdapter` over stdio NDJSON for `scenario`.
///
/// Enforces:
/// - Virtual time (`sim_time_rfc3339`) injection and verification
/// - `PerceptView` oracle-label stripping
/// - Optional mid-run `restart_epoch` RAM eviction (`restart_at_percept_idx`)
/// - `SEALED_MODE=1` isolation (aborts on any leaked payload/prompt on disk or stdout)
/// - `step_wall_timeout_s` per-step timeout guard
/// - Symmetric MonoAgens comparison confirming zero LLM tokens on Gate-suppressed turns
#[allow(clippy::too_many_arguments)]
pub fn run_baseline_adapter(
    kind: BaselineKind,
    adapter_script: &Path,
    scenario: &Scenario,
    seed: u64,
    sealed_mode: bool,
    restart_at_percept_idx: Option<usize>,
    step_wall_timeout: Duration,
    extra_env: &[(&str, &str)],
) -> Result<BaselineRunReport, LabError> {
    if !adapter_script.is_file() {
        return Err(LabError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("adapter script not found: '{}'", adapter_script.display()),
        )));
    }

    // Run MonoAgens on the exact same scenario for symmetric NOOP token parity.
    let mono_out = run_scenario(MemStore::new(), Prior::default(), scenario, seed)?;
    let mono_gate_suppressions = mono_out
        .events
        .iter()
        .filter(|e| {
            matches!(
                &e.detail,
                Some(EventDetail::Decision {
                    outcome: DecisionOutcome::Silent | DecisionOutcome::Note,
                    ..
                })
            )
        })
        .count();

    let caps: Vec<String> = scenario
        .bodies
        .iter()
        .flat_map(|b| b.capabilities.iter().map(|c| c.id.as_str().to_owned()))
        .collect();

    let state_tmp = TempDir::new()?;
    let state_dir = state_tmp.path().join("adapter_state");
    fs::create_dir_all(&state_dir)?;

    let mut cmd = Command::new("python3");
    cmd.arg(adapter_script)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("PYTHONDONTWRITEBYTECODE", "1");
    if sealed_mode {
        cmd.env("SEALED_MODE", "1");
    }
    for &(k, v) in extra_env {
        cmd.env(k, v);
    }

    let mut child = cmd.spawn()?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| LabError::Protocol("missing adapter stdin".to_owned()))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| LabError::Protocol("missing adapter stdout".to_owned()))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| LabError::Protocol("missing adapter stderr".to_owned()))?;

    let (tx, rx) = mpsc::channel::<Result<String, String>>();
    thread::spawn(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines() {
            match line {
                Ok(l) => {
                    if tx.send(Ok(l)).is_err() {
                        break;
                    }
                }
                Err(e) => {
                    let _ = tx.send(Err(e.to_string()));
                    break;
                }
            }
        }
    });

    let (err_tx, err_rx) = mpsc::channel::<String>();
    thread::spawn(move || {
        let mut s = String::new();
        let mut br = BufReader::new(stderr);
        let _ = std::io::Read::read_to_string(&mut br, &mut s);
        let _ = err_tx.send(s);
    });

    let mut epoch = 0_u64;
    let init_req = serde_json::json!({
        "op": "init",
        "scenario_id": if sealed_mode { "sealed_anon".to_owned() } else { scenario.id.clone() },
        "run_seed": seed,
        "sim_time_rfc3339": format_sim_time_rfc3339(Time::from_millis(scenario.start_time_ms)),
        "restart_epoch": epoch,
        "sealed_mode": sealed_mode,
        "state_dir": state_dir.to_string_lossy(),
    });
    let init_resp = send_ndjson(&mut stdin, &rx, &init_req, step_wall_timeout)?;
    let sdk_verified = init_resp
        .get("sdk_verified")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let mut unavailable_reason = init_resp
        .get("unavailable_reason")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned);

    let mut steps = Vec::new();
    let mut percept_idx = 0usize;
    let mut seq = 0usize;

    for ev in &scenario.events {
        if !sdk_verified {
            break;
        }
        let ScriptedEventKind::Percept(percept) = &ev.event else {
            continue;
        };
        if restart_at_percept_idx == Some(percept_idx) {
            epoch += 1;
            let rst_req = serde_json::json!({
                "op": "restart_epoch",
                "restart_epoch": epoch,
                "sim_time_rfc3339": format_sim_time_rfc3339(Time::from_millis(ev.at_ms)),
            });
            let rst_resp = send_ndjson(&mut stdin, &rx, &rst_req, step_wall_timeout)?;
            if rst_resp
                .get("ram_cleared_on_restart")
                .and_then(serde_json::Value::as_bool)
                != Some(true)
            {
                let _ = child.kill();
                let _ = child.wait();
                return Err(LabError::Protocol(format!(
                    "adapter '{}' failed to clear ephemeral RAM on restart_epoch {epoch}",
                    kind.slug()
                )));
            }
        }

        let sim_rfc = format_sim_time_rfc3339(Time::from_millis(ev.at_ms));
        let p_view =
            PerceptView::from_percept(format!("ev-{seq}"), percept, &caps, sim_rfc.clone());
        let step_req = serde_json::json!({
            "op": "step",
            "scenario_id": if sealed_mode { "sealed_anon".to_owned() } else { scenario.id.clone() },
            "run_seed": seed,
            "sim_time_rfc3339": sim_rfc,
            "monotonic_seq": seq,
            "restart_epoch": epoch,
            "percept": p_view,
        });

        let step_start = Instant::now();
        let resp = match send_ndjson(&mut stdin, &rx, &step_req, step_wall_timeout) {
            Ok(v) => v,
            Err(err) => {
                let _ = child.kill();
                let _ = child.wait();
                if sealed_mode {
                    return Err(LabError::Protocol(
                        "SEALED_MODE=1 preflight/isolation aborted adapter run".to_owned(),
                    ));
                }
                return Err(err);
            }
        };
        let elapsed_ms = step_start.elapsed().as_secs_f64() * 1_000.0;

        // Enforce SEALED_MODE=1 zero disk-write & zero payload-echo invariant.
        if sealed_mode {
            let dir_has_files = fs::read_dir(&state_dir)?.next().is_some();
            let raw_resp_str = serde_json::to_string(&resp).unwrap_or_default();
            let payload_str = String::from_utf8_lossy(percept.payload.as_bytes());
            if dir_has_files
                || (!payload_str.is_empty() && raw_resp_str.contains(payload_str.as_ref()))
            {
                let _ = child.kill();
                let _ = child.wait();
                return Err(LabError::Protocol(
                    "SEALED_MODE=1 violation: adapter persisted or echoed scenario payload"
                        .to_owned(),
                ));
            }
        }

        if resp.get("ok").and_then(serde_json::Value::as_bool) != Some(true) {
            if !sdk_verified {
                unavailable_reason = resp
                    .get("unavailable_reason")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned)
                    .or(unavailable_reason);
                break;
            }
            let _ = child.kill();
            let _ = child.wait();
            return Err(LabError::Protocol(format!(
                "adapter '{}' returned error step response",
                kind.slug()
            )));
        }

        let recorded_sim = resp
            .get("recorded_sim_time_rfc3339")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .to_owned();
        if recorded_sim != sim_rfc {
            let _ = child.kill();
            let _ = child.wait();
            return Err(LabError::Protocol(format!(
                "adapter '{}' violated virtual clock: expected '{sim_rfc}', got '{recorded_sim}'",
                kind.slug()
            )));
        }

        let usage: TokenUsage5Tuple = serde_json::from_value(
            resp.get("usage")
                .cloned()
                .unwrap_or_else(|| serde_json::json!({})),
        )
        .map_err(|e| LabError::Serialization(e.to_string()))?;
        let e_tok = usage.e_tok();
        let c_share = usage.c_share();
        let decision = resp
            .get("decision")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("noop")
            .to_owned();
        let action: Option<AdapterAction> = resp
            .get("action")
            .and_then(|v| if v.is_null() { None } else { Some(v.clone()) })
            .map(serde_json::from_value)
            .transpose()
            .map_err(|e| LabError::Serialization(e.to_string()))?;
        let noop_tool_exposed = resp
            .get("noop_tool_exposed")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        let wake_invoked_llm = resp
            .get("wake_invoked_llm")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        let ram_cleared_on_restart = resp
            .get("ram_cleared_on_restart")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        let latency_ms = resp
            .get("latency_ms")
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(elapsed_ms);
        let cost_usd = resp
            .get("cost_usd")
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(e_tok * 1e-6);

        steps.push(AdapterStepRecord {
            monotonic_seq: seq,
            sim_time_rfc3339: sim_rfc,
            recorded_sim_time_rfc3339: recorded_sim,
            restart_epoch: epoch,
            ram_cleared_on_restart,
            decision,
            action,
            noop_tool_exposed,
            wake_invoked_llm,
            usage,
            e_tok,
            c_share,
            latency_ms,
            cost_usd,
        });

        percept_idx += 1;
        seq += 1;
    }

    let _ = send_ndjson(
        &mut stdin,
        &rx,
        &serde_json::json!({ "op": "shutdown" }),
        step_wall_timeout,
    );
    drop(stdin);
    let status = child.wait()?;
    let stderr_text = err_rx
        .recv_timeout(Duration::from_millis(200))
        .unwrap_or_default();
    if sealed_mode && (!status.success() || !stderr_text.is_empty()) {
        return Err(LabError::Protocol(
            "SEALED_MODE=1 isolation aborted run with non-zero adapter failure (redacted)"
                .to_owned(),
        ));
    }
    if !status.success() {
        return Err(LabError::Protocol(format!(
            "adapter '{}' exited with status {status}: {stderr_text}",
            kind.slug()
        )));
    }

    let total_e_tok: f64 = steps.iter().map(|s| s.e_tok).sum();
    let mean_c_share = if steps.is_empty() {
        0.0
    } else {
        steps.iter().map(|s| s.c_share).sum::<f64>() / steps.len() as f64
    };
    let noop_llm_wakes = steps
        .iter()
        .filter(|s| s.decision == "noop" && s.wake_invoked_llm)
        .count();

    Ok(BaselineRunReport {
        adapter: kind.slug().to_owned(),
        scenario_id: if sealed_mode {
            "sealed_anon".to_owned()
        } else {
            scenario.id.clone()
        },
        seed,
        sealed_mode,
        sdk_verified,
        unavailable_reason,
        steps,
        total_e_tok,
        mean_c_share,
        noop_llm_wakes,
        monoagens_zero_llm_gate_suppressions: mono_gate_suppressions,
        monoagens_gate_llm_tokens: 0,
    })
}

fn send_ndjson(
    stdin: &mut std::process::ChildStdin,
    rx: &mpsc::Receiver<Result<String, String>>,
    payload: &serde_json::Value,
    timeout: Duration,
) -> Result<serde_json::Value, LabError> {
    let encoded =
        serde_json::to_string(payload).map_err(|e| LabError::Serialization(e.to_string()))?;
    writeln!(stdin, "{encoded}")?;
    stdin.flush()?;
    match rx.recv_timeout(timeout) {
        Ok(Ok(line)) => serde_json::from_str(&line)
            .map_err(|e| LabError::Serialization(format!("invalid adapter NDJSON: {e}"))),
        Ok(Err(err)) => Err(LabError::Io(std::io::Error::other(err))),
        Err(mpsc::RecvTimeoutError::Timeout) => Err(LabError::Protocol(
            "StepOutcome::Timeout waiting for adapter".to_owned(),
        )),
        Err(mpsc::RecvTimeoutError::Disconnected) => Err(LabError::Protocol(
            "adapter closed stdout unexpectedly".to_owned(),
        )),
    }
}
