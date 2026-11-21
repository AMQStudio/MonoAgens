//! `monoagens` CLI library implementing `chat`, `log`, `replay`, `approvals`, `link`,
//! `conformance`, and `inspector` against `gateway`, `valve`, `lab`, `conformance`, and `body-cli`.

#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use body_cli::{CliBody, TerminalOutputSink};
use clap::{Args, Parser, Subcommand, ValueEnum};
use monoagens_body_kit::{EmittedCue, PersistentBodyRuntime};
use monoagens_conformance::{ConformanceLevel, run_body_conformance, run_core_conformance};
use monoagens_core::{
    DecisionOutcome, DecisionTrace, EventDetail, EventKind, EventRecord, Input, MemStore,
    ReplayMode, Store,
};
use monoagens_gateway::{
    Clock, CueRecorder, DiagnosticSink, DoorsAdapter, GatewayConfig, GatewayOptions,
    GatewayRuntime, GatewaySeams, IdentityResolver, LifecycleCueKind, LoadedGatewayEnv,
    SystemClock, load_or_init_gateway_env, resolve_paths,
};
use monoagens_protocol::{
    Action, ActionCanonicalInput, ActionId, ActorId, BodyId, EventId, HAct, PlaceId,
    ReceiptOutcome, Time,
};
use monoagens_reason::ModelProvider;
use monoagens_store::{SegmentedStore, StoreConfig};
use monoagens_valve::{
    DEFAULT_CEDAR_POLICIES, PolicyBundle, Valve, ValveConfig, ValveMode, ValveReasonTag,
    ValveSnapshot, canonical_json_bytes, compute_h_act,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader as TokioBufReader};

/// Top-level `monoagens` CLI arguments.
#[derive(Debug, Parser)]
#[command(name = "monoagens", version, about = "MonoAgens CLI")]
pub struct Cli {
    /// Override path to `monoagens.toml`.
    #[arg(long, global = true)]
    pub config: Option<PathBuf>,

    /// Emit strictly machine-readable JSON on stdout.
    #[arg(long, global = true)]
    pub json: bool,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Boot Gateway + Valve + body-cli for interactive or single-message chat.
    Chat(ChatArgs),
    /// Query event logs or explain a single gate decision (`--explain <id>`).
    Log(LogArgs),
    /// Replay a recorded log offline with zero model calls and verify bit-identical decisions.
    Replay(ReplayArgs),
    /// Inspect pending approvals (with canonical `H_act`) or attempt resolution.
    Approvals(ApprovalsArgs),
    /// Manage canonical `κ(b, a)` cross-body identity links and bilateral challenges.
    Link(LinkArgs),
    /// Run black-box Waist-1/Waist-2 conformance suites (`monoagens-conformance`).
    Conformance(ConformanceArgs),
    /// Inspect `traceparent` traces and active Cedar Valve policy rules/toggles.
    Inspector(InspectorArgs),
}

#[derive(Debug, Args)]
pub struct ChatArgs {
    /// Send one or more messages non-interactively instead of reading stdin lines.
    #[arg(long = "message")]
    pub messages: Vec<String>,
    /// Optional data/store directory override for this session.
    #[arg(long)]
    pub data_dir: Option<PathBuf>,
}

#[derive(Debug, Args)]
pub struct LogArgs {
    /// Filter events by place id.
    #[arg(long)]
    pub place: Option<String>,
    /// Filter events by source/actor id.
    #[arg(long)]
    pub source: Option<String>,
    /// Filter events by canonical EventKind snake_case name (e.g. decision, percept, door_verdict).
    #[arg(long)]
    pub kind: Option<String>,
    /// Filter events with timestamp_ms >= since.
    #[arg(long)]
    pub since: Option<u64>,
    /// Filter events with timestamp_ms <= until.
    #[arg(long)]
    pub until: Option<u64>,
    /// Explain a single decision event (or decision targeting event id).
    #[arg(long)]
    pub explain: Option<u64>,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum ReplayModeArg {
    #[value(alias = "two-epoch")]
    Pinned,
    #[value(alias = "full")]
    Unpinned,
}

#[derive(Debug, Args)]
pub struct ReplayArgs {
    /// Override PRNG seed (defaults to `soul.seed` in config, or 1).
    #[arg(long)]
    pub seed: Option<u64>,
    /// Replay mode (`pinned` or `unpinned`; legacy aliases `two-epoch` and `full` accepted).
    #[arg(long, value_enum, default_value_t = ReplayModeArg::Pinned)]
    pub mode: ReplayModeArg,
    /// Optional JSON file containing an array of EventRecord values to replay offline.
    #[arg(long)]
    pub log_json: Option<PathBuf>,
}

#[derive(Debug, Args)]
pub struct ApprovalsArgs {
    #[command(subcommand)]
    pub action: Option<ApprovalsSubcommand>,
}

#[derive(Debug, Subcommand)]
pub enum ApprovalsSubcommand {
    /// List pending `Held` actions with canonical `H_act` and canonical JSON preimage.
    List,
    /// Attempt to grant a pending approval by `action_id`.
    Grant { action_id: String },
    /// Attempt to deny a pending approval by `action_id`.
    Deny { action_id: String },
}

#[derive(Debug, Args)]
pub struct LinkArgs {
    #[command(subcommand)]
    pub action: LinkSubcommand,
}

#[derive(Debug, Subcommand)]
pub enum LinkSubcommand {
    /// List configured `[[person]]` accounts and active prospective `actor_linked` links.
    List,
    /// Resolve `(body_id, account_id)` via `[[person]] -> E_link -> κ(b, a)`.
    Resolve {
        #[arg(long)]
        body: String,
        #[arg(long)]
        account: String,
    },
    /// Initiate a dual-nonce bilateral challenge (`actor_link_challenge`).
    Initiate {
        #[arg(long)]
        primary_body: String,
        #[arg(long)]
        primary_account: String,
        #[arg(long)]
        secondary_body: String,
        #[arg(long)]
        secondary_account: String,
        /// Include generated nonces in output for out-of-band delivery/testing.
        #[arg(long)]
        emit_nonces: bool,
    },
    /// Verify a bilateral challenge (`actor_linked`).
    Verify {
        #[arg(long)]
        challenge: String,
        #[arg(long)]
        nonce1: String,
        #[arg(long)]
        nonce2: String,
        #[arg(long)]
        owner_confirmed: bool,
    },
    /// Revoke an active link (`actor_unlinked`).
    Unlink {
        #[arg(long)]
        body: String,
        #[arg(long)]
        account: String,
        #[arg(long, default_value = "owner_unlinked")]
        reason: String,
    },
}

#[derive(Debug, Args)]
pub struct ConformanceArgs {
    #[command(subcommand)]
    pub target: ConformanceSubcommand,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum LevelArg {
    Must,
    Should,
    May,
}

impl From<LevelArg> for ConformanceLevel {
    fn from(value: LevelArg) -> Self {
        match value {
            LevelArg::Must => Self::Must,
            LevelArg::Should => Self::Should,
            LevelArg::May => Self::May,
        }
    }
}

#[derive(Debug, Subcommand)]
pub enum ConformanceSubcommand {
    /// Run Body TCK (`B-MUST-01..07`, etc.) against a body binary.
    Body {
        #[arg(long)]
        target: String,
        #[arg(long = "target-arg")]
        target_args: Vec<String>,
        #[arg(long, value_enum, default_value_t = LevelArg::Must)]
        level: LevelArg,
        #[arg(long)]
        json_report: Option<PathBuf>,
    },
    /// Run Core TCK (`C-MUST-01..04`, etc.) against a Core harness binary.
    Core {
        #[arg(long)]
        target: String,
        #[arg(long = "target-arg")]
        target_args: Vec<String>,
        #[arg(long, value_enum, default_value_t = LevelArg::Must)]
        level: LevelArg,
        #[arg(long)]
        json_report: Option<PathBuf>,
    },
}

#[derive(Debug, Args)]
pub struct InspectorArgs {
    /// Optional `traceparent` filter when inspecting trace records.
    #[arg(long)]
    pub traceparent: Option<String>,
}

/// Result returned by [`execute_cli`] indicating exit status (0 = success, non-zero = error).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CliOutcome {
    pub exit_code: u8,
}

/// Runs the parsed [`Cli`] command, writing stdout to `out` and errors/cues to `err`.
pub fn execute_cli(cli: Cli, out: &mut dyn Write, err: &mut dyn Write) -> CliOutcome {
    let config_opt = cli.config.as_deref();
    let json_mode = cli.json;

    let res = match cli.command {
        Commands::Chat(args) => cmd_chat(config_opt, json_mode, args, None, out, err),
        Commands::Log(args) => cmd_log(config_opt, json_mode, args, out),
        Commands::Replay(args) => cmd_replay(config_opt, json_mode, args, out, err),
        Commands::Approvals(args) => cmd_approvals(config_opt, json_mode, args, out),
        Commands::Link(args) => cmd_link(config_opt, json_mode, args, out),
        Commands::Conformance(args) => cmd_conformance(args, out),
        Commands::Inspector(args) => cmd_inspector(config_opt, json_mode, args, out),
    };

    match res {
        Ok(code) => CliOutcome { exit_code: code },
        Err(msg) => {
            let _ = writeln!(err, "error: {msg}");
            CliOutcome { exit_code: 1 }
        }
    }
}

struct SharedChatSink {
    rendered: Arc<Mutex<Vec<(String, String)>>>,
}

impl TerminalOutputSink for SharedChatSink {
    fn render_bubble(&mut self, pid: &str, text: &str) -> Result<(), String> {
        self.rendered
            .lock()
            .expect("rendered lock")
            .push((pid.to_owned(), text.to_owned()));
        Ok(())
    }

    fn render_cue(&mut self, _cue: &EmittedCue) -> Result<(), String> {
        Ok(())
    }
}

/// Executes `monoagens chat` with an optional injected [`ModelProvider`] (used by tests to drive
/// `body-cli` + `Valve` + `GatewayRuntime` deterministically without network).
pub fn run_chat_with_provider(
    config_path: Option<&Path>,
    json_mode: bool,
    args: ChatArgs,
    provider_override: Option<Arc<dyn ModelProvider>>,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> Result<u8, String> {
    cmd_chat(config_path, json_mode, args, provider_override, out, err)
}

fn cmd_chat(
    config_path: Option<&Path>,
    json_mode: bool,
    args: ChatArgs,
    provider_override: Option<Arc<dyn ModelProvider>>,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> Result<u8, String> {
    let loaded: LoadedGatewayEnv = load_or_init_gateway_env(config_path, |k| std::env::var(k).ok())
        .map_err(|e| e.to_string())?;

    if provider_override.is_none()
        && let Err(startup_err) = loaded.ensure_startup_ready()
    {
        let _ = writeln!(err, "{startup_err}");
        let _ = writeln!(
            err,
            "config path: {}\nsecrets path: {}\nrequired env vars (when using openai_compatible): MONOAGENS_MODEL_API_KEY (or OPENAI_API_KEY), MONOAGENS_MODEL_ENDPOINT",
            loaded.paths.config_path.display(),
            loaded.paths.secrets_path.display(),
        );
        return Ok(1);
    }

    let owner_actor = loaded
        .config
        .persons
        .iter()
        .find(|p| p.is_owner)
        .map(|p| p.actor_id.clone())
        .unwrap_or_else(|| ActorId::new("per_owner"));

    let valve_cfg = build_valve_config(&loaded.config);
    let mut snapshot = ValveSnapshot {
        owner_actor_id: Some(owner_actor.clone()),
        t_now: SystemClock::new().now(),
        ..ValveSnapshot::default()
    };
    snapshot.known_actors.insert(owner_actor.clone());

    // Build Valve (runs cedar-policy-symcc cvc5 verification) BEFORE entering Tokio runtime.
    let valve = Valve::with_default_policies(valve_cfg, snapshot)
        .map_err(|e| format!("valve policy verification failed: {e}"))?;

    let mut cfg = loaded.config.clone();
    let store_dir = args
        .data_dir
        .unwrap_or_else(|| loaded.paths.data_dir.clone());
    cfg.data_dir = Some(store_dir.clone());

    let inputs: Vec<String> = if !args.messages.is_empty() {
        args.messages
    } else {
        let stdin = io::stdin();
        stdin
            .lock()
            .lines()
            .map_while(Result::ok)
            .map(|l| l.trim().to_owned())
            .filter(|l| !l.is_empty())
            .collect()
    };

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("failed to start tokio runtime: {e}"))?;

    let rendered_bubbles = Arc::new(Mutex::new(Vec::<(String, String)>::new()));
    let rendered_clone = Arc::clone(&rendered_bubbles);

    let result: Result<(Vec<String>, Vec<EventRecord>), String> = rt.block_on(async move {
        let clock = Arc::new(SystemClock::new());
        let diagnostics = DiagnosticSink::new(&loaded.secrets);
        let cues = CueRecorder::new();
        let mut cue_rx = cues.subscribe();

        let runtime = GatewayRuntime::start_segmented_with_configured_core(
            cfg,
            &loaded.secrets,
            store_dir,
            DoorsAdapter(valve),
            GatewayOptions {
                clock: clock.clone(),
                provider: provider_override,
                diagnostics,
                cues,
                seams: GatewaySeams::default(),
            },
        )
        .await
        .map_err(|e| format!("gateway startup failed: {e}"))?;

        let handle = runtime.handle();

        // Spawn in-process body-cli speaking canonical Waist-2 NDJSON over duplex pipe.
        let (gw_stream, body_stream) = tokio::io::duplex(64 * 1024);
        let (gw_read, gw_write) = tokio::io::split(gw_stream);
        let (body_read, mut body_write) = tokio::io::split(body_stream);

        let body_actor = owner_actor.clone();
        let body_handle = tokio::spawn(async move {
            let mut body_rt = PersistentBodyRuntime::new(None);
            let mut cli_body = CliBody::new(
                BodyId::new("body-cli"),
                PlaceId::new("cli:terminal"),
                body_actor,
                4096,
                SharedChatSink {
                    rendered: rendered_clone,
                },
            );
            let mut reader = TokioBufReader::new(body_read);
            let mut line = String::new();
            loop {
                line.clear();
                match reader.read_line(&mut line).await {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {
                        if let Some(reply) = body_rt.dispatch_line(&line, &mut cli_body)
                            && (body_write.write_all(reply.as_bytes()).await.is_err()
                                || body_write.write_all(b"\n").await.is_err()
                                || body_write.flush().await.is_err())
                        {
                            break;
                        }
                    }
                }
            }
        });

        handle
            .connect_ndjson_body(gw_read, gw_write, Some(BodyId::new("body-cli")))
            .await
            .map_err(|e| format!("failed to register body-cli over Waist 2: {e}"))?;

        let helper_body = CliBody::new(
            BodyId::new("body-cli"),
            PlaceId::new("cli:terminal"),
            owner_actor,
            4096,
            SharedChatSink {
                rendered: Arc::new(Mutex::new(Vec::new())),
            },
        );

        let mut emitted_outputs = Vec::new();
        let mut rendered_cursor = 0usize;

        for (idx, msg) in inputs.iter().enumerate() {
            let now = clock.now();
            let (percept_params, _) =
                helper_body.build_user_percept(msg, None, None, now, format!("cli-chat-{idx}"));
            let mut percept = percept_params
                .into_percept(monoagens_protocol::CURRENT_PROTOCOL_VERSION)
                .map_err(|e| e.to_string())?;
            if percept.addressees.is_empty() {
                percept.addressees.push(ActorId::new("soul"));
            }
            handle
                .submit_and_wait(Input::Percept(Box::new(percept)))
                .await
                .map_err(|e| format!("chat turn failed: {e}"))?;

            // Drain any asynchronous model/action turn briefly without blocking fast responses (d_art = 0 ms).
            for _ in 0..100 {
                while let Ok(cue) = cue_rx.try_recv() {
                    let cue_label = match cue.kind {
                        LifecycleCueKind::WorkingCue => "working_cue",
                        LifecycleCueKind::ProgressNote => "progress_note",
                    };
                    if !json_mode {
                        let _ = writeln!(err, "[{cue_label}] {}", cue.detail);
                    }
                }
                let new_items = {
                    let guard = rendered_bubbles.lock().expect("rendered lock");
                    if guard.len() > rendered_cursor {
                        let slice = guard[rendered_cursor..].to_vec();
                        rendered_cursor = guard.len();
                        slice
                    } else {
                        Vec::new()
                    }
                };
                if !new_items.is_empty() {
                    for (_pid, text) in new_items {
                        emitted_outputs.push(text);
                    }
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        }

        let events = handle.read_events().await.unwrap_or_default();
        runtime.shutdown().await;
        body_handle.abort();
        Ok((emitted_outputs, events))
    });

    let (outputs, events) = result?;
    if json_mode {
        let payload = json!({
            "status": "ok",
            "responses": outputs,
            "events_recorded": events.len(),
        });
        writeln!(
            out,
            "{}",
            serde_json::to_string_pretty(&payload).map_err(|e| e.to_string())?
        )
        .map_err(|e| e.to_string())?;
    } else {
        for text in outputs {
            writeln!(out, "{text}").map_err(|e| e.to_string())?;
        }
    }
    Ok(0)
}

fn build_valve_config(cfg: &GatewayConfig) -> ValveConfig {
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

fn load_paths(
    config_path: Option<&Path>,
) -> Result<(Option<GatewayConfig>, monoagens_gateway::ResolvedPaths), String> {
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
        monoagens_gateway::ResolvedPaths {
            config_path: paths.config_path,
            secrets_path: paths.secrets_path,
            data_dir: data_dir.clone(),
        }
    } else {
        paths
    };
    Ok((cfg, effective_paths))
}

fn open_store_readonly(data_dir: &Path) -> Result<SegmentedStore, String> {
    if !data_dir.exists() {
        return Err(format!(
            "store directory '{}' does not exist",
            data_dir.display()
        ));
    }
    SegmentedStore::open(StoreConfig::new(data_dir.to_path_buf()))
        .map_err(|e| format!("failed to open store '{}': {e}", data_dir.display()))
}

fn cmd_log(
    config_path: Option<&Path>,
    json_mode: bool,
    args: LogArgs,
    out: &mut dyn Write,
) -> Result<u8, String> {
    let (_cfg, paths) = load_paths(config_path)?;
    let store = open_store_readonly(&paths.data_dir)?;
    let events = store
        .read_in_order(None, usize::MAX)
        .map_err(|e| format!("failed to read event log: {e}"))?;

    if let Some(explain_id) = args.explain {
        return render_explain_event(explain_id, &events, json_mode, out);
    }

    let kind_filter = args
        .kind
        .as_deref()
        .map(|k| parse_event_kind(k).ok_or_else(|| format!("unknown event kind '{k}'")))
        .transpose()?;

    let filtered: Vec<&EventRecord> = events
        .iter()
        .filter(|ev| args.place.as_deref().is_none_or(|p| ev.place.as_str() == p))
        .filter(|ev| {
            args.source
                .as_deref()
                .is_none_or(|s| ev.actor.as_str() == s)
        })
        .filter(|ev| kind_filter.is_none_or(|k| ev.kind == k))
        .filter(|ev| {
            args.since
                .is_none_or(|min_ms| ev.timestamp_ms.as_millis() >= min_ms)
        })
        .filter(|ev| {
            args.until
                .is_none_or(|max_ms| ev.timestamp_ms.as_millis() <= max_ms)
        })
        .collect();

    if json_mode {
        let rows: Vec<serde_json::Value> = filtered
            .iter()
            .map(|ev| {
                json!({
                    "id": ev.id.as_u64(),
                    "seq": ev.seq.as_u64(),
                    "kind": ev.kind.as_str(),
                    "timestamp_ms": ev.timestamp_ms.as_millis(),
                    "place": ev.place.as_str(),
                    "actor": ev.actor.as_str(),
                    "detail": ev.detail,
                })
            })
            .collect();
        writeln!(
            out,
            "{}",
            serde_json::to_string_pretty(&rows).map_err(|e| e.to_string())?
        )
        .map_err(|e| e.to_string())?;
    } else {
        for ev in filtered {
            writeln!(
                out,
                "seq={} id={} kind={} ts={} place={} actor={}",
                ev.seq.as_u64(),
                ev.id.as_u64(),
                ev.kind.as_str(),
                ev.timestamp_ms.as_millis(),
                ev.place.as_str(),
                ev.actor.as_str(),
            )
            .map_err(|e| e.to_string())?;
        }
    }
    Ok(0)
}

fn parse_event_kind(raw: &str) -> Option<EventKind> {
    EventKind::ALL.into_iter().find(|k| k.as_str() == raw)
}

#[derive(Debug, Serialize)]
struct ExplainDecisionPayload<'a> {
    event_id: u64,
    seq: u64,
    target_event_id: u64,
    outcome: DecisionOutcome,
    salience: f64,
    threshold: f64,
    probability: f64,
    draw: f64,
    reason: Option<monoagens_core::DecisionReason>,
    trace: Option<&'a DecisionTrace>,
    door_verdicts: Vec<DoorVerdictSummary>,
}

#[derive(Debug, Serialize)]
struct DoorVerdictSummary {
    event_id: u64,
    seq: u64,
    door: String,
    rule: String,
    reason_tag: String,
}

fn render_explain_event(
    query_id: u64,
    events: &[EventRecord],
    json_mode: bool,
    out: &mut dyn Write,
) -> Result<u8, String> {
    let target = EventId::new(query_id);
    let decision_ev = events
        .iter()
        .find(|ev| ev.id == target && matches!(ev.detail, Some(EventDetail::Decision { .. })))
        .or_else(|| {
            events.iter().find(|ev| {
                matches!(
                    ev.detail,
                    Some(EventDetail::Decision { target_event_id, .. }) if target_event_id == target
                )
            })
        })
        .ok_or_else(|| format!("no decision event found for event id {query_id}"))?;

    let Some(EventDetail::Decision {
        target_event_id,
        outcome,
        salience,
        threshold,
        probability,
        draw,
        reason,
        trace,
    }) = &decision_ev.detail
    else {
        return Err(format!("event {query_id} is not a decision event"));
    };

    let door_verdicts: Vec<DoorVerdictSummary> = events
        .iter()
        .filter_map(|ev| match &ev.detail {
            Some(EventDetail::DoorVerdict {
                door,
                target_event_id: dv_target,
                rule,
                verdict,
            }) if *dv_target == Some(*target_event_id) || *dv_target == Some(decision_ev.id) => {
                Some(DoorVerdictSummary {
                    event_id: ev.id.as_u64(),
                    seq: ev.seq.as_u64(),
                    door: format!("{door:?}").to_lowercase(),
                    rule: rule.clone(),
                    reason_tag: verdict.clone(),
                })
            }
            _ => None,
        })
        .collect();

    if json_mode {
        let payload = ExplainDecisionPayload {
            event_id: decision_ev.id.as_u64(),
            seq: decision_ev.seq.as_u64(),
            target_event_id: target_event_id.as_u64(),
            outcome: *outcome,
            salience: *salience,
            threshold: *threshold,
            probability: *probability,
            draw: *draw,
            reason: *reason,
            trace: trace.as_deref(),
            door_verdicts,
        };
        writeln!(
            out,
            "{}",
            serde_json::to_string_pretty(&payload).map_err(|e| e.to_string())?
        )
        .map_err(|e| e.to_string())?;
    } else {
        writeln!(
            out,
            "decision event={} target={} outcome={:?} salience={:.6} threshold={:.6} probability={:.6} draw={:.6} reason={:?}",
            decision_ev.id.as_u64(),
            target_event_id.as_u64(),
            outcome,
            salience,
            threshold,
            probability,
            draw,
            reason
        )
        .map_err(|e| e.to_string())?;
        if let Some(t) = trace {
            writeln!(
                out,
                "  schema_v={} online_adapt={} commitment_bypass={} cic_abstain={} wait_delta_ms={} gate_probs={:?} gate_logits={:?}",
                t.feature_schema_version,
                t.online_adaptation_applied,
                t.commitment_bypass,
                t.neural.cic_abstain,
                t.neural.wait_delta_ms,
                t.neural.gate_probs,
                t.neural.gate_logits,
            )
            .map_err(|e| e.to_string())?;
        }
        for dv in &door_verdicts {
            writeln!(
                out,
                "  door_verdict door={} rule={} reason_tag={}",
                dv.door, dv.rule, dv.reason_tag
            )
            .map_err(|e| e.to_string())?;
        }
    }
    Ok(0)
}

fn cmd_replay(
    config_path: Option<&Path>,
    json_mode: bool,
    args: ReplayArgs,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> Result<u8, String> {
    let (cfg_opt, paths) = load_paths(config_path)?;
    let _seed = args
        .seed
        .or_else(|| cfg_opt.as_ref().map(|c| c.soul.seed))
        .unwrap_or(1);

    let events: Vec<EventRecord> = if let Some(log_path) = &args.log_json {
        let raw = fs::read_to_string(log_path)
            .map_err(|e| format!("failed to read log JSON '{}': {e}", log_path.display()))?;
        serde_json::from_str(&raw)
            .map_err(|e| format!("failed to parse log JSON '{}': {e}", log_path.display()))?
    } else {
        let store = open_store_readonly(&paths.data_dir)?;
        store
            .read_in_order(None, usize::MAX)
            .map_err(|e| format!("failed to read store log: {e}"))?
    };

    let mode = match args.mode {
        ReplayModeArg::Pinned => ReplayMode::Pinned,
        ReplayModeArg::Unpinned => ReplayMode::Unpinned,
    };

    let mut mem = MemStore::new();
    for rec in &events {
        mem.append(rec.clone())
            .map_err(|e| format!("offline replay store build failed: {e}"))?;
    }
    let report = monoagens_core::replay_log(&mem, mode, None)
        .map_err(|e| format!("offline replay failed: {e}"))?;

    if report.is_exact() {
        if json_mode {
            let payload = json!({
                "status": "ok",
                "message": "all decisions reproduced",
                "replayed_events": report.recovery.replayed_events,
                "matched_decisions": report.verified_decisions,
                "first_divergence": serde_json::Value::Null,
            });
            writeln!(
                out,
                "{}",
                serde_json::to_string_pretty(&payload).map_err(|e| e.to_string())?
            )
            .map_err(|e| e.to_string())?;
        } else {
            writeln!(
                out,
                "all decisions reproduced (matched={}, replayed={})",
                report.verified_decisions, report.recovery.replayed_events
            )
            .map_err(|e| e.to_string())?;
        }
        Ok(0)
    } else {
        let div = report.mismatches.first().expect("mismatch present");
        let div_msg = format!(
            "replay divergence at seq={} decision_event={} target_event={} field={}: {}",
            div.seq.as_u64(),
            div.event_id.as_u64(),
            div.target_event_id.as_u64(),
            div.field,
            div.detail
        );
        if json_mode {
            let payload = json!({
                "status": "diverged",
                "replayed_events": report.recovery.replayed_events,
                "matched_decisions": report.verified_decisions,
                "first_divergence": {
                    "seq": div.seq.as_u64(),
                    "decision_event_id": div.event_id.as_u64(),
                    "target_event_id": div.target_event_id.as_u64(),
                    "field": div.field,
                    "detail": div.detail,
                },
            });
            writeln!(
                out,
                "{}",
                serde_json::to_string_pretty(&payload).map_err(|e| e.to_string())?
            )
            .map_err(|e| e.to_string())?;
        }
        let _ = writeln!(err, "error: {div_msg}");
        Ok(1)
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct PendingApprovalEntry {
    action_id: String,
    capability_id: String,
    places: Vec<String>,
    state_epoch: u64,
    h_act: String,
    canonical_input: ActionCanonicalInput,
    canonical_payload_utf8: String,
    canonical_preimage_hex: String,
    reason_code: Option<String>,
    timestamp_ms: u64,
}

fn to_hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        use std::fmt::Write as _;
        let _ = write!(&mut s, "{b:02x}");
    }
    s
}

/// Computes canonical `H_act` and canonical preimage representation for an [`Action`] at `state_epoch`.
#[must_use]
pub fn canonical_h_act_details(
    action: &Action,
    state_epoch: u64,
) -> (HAct, ActionCanonicalInput, Vec<u8>) {
    let h_act = compute_h_act(action, state_epoch);
    let canonical_payload = canonical_json_bytes(action.payload.as_bytes());
    let input = ActionCanonicalInput {
        capability_id: action.capability_id.clone(),
        actor: action.origin_actor.actor_id().clone(),
        places: action.places.clone(),
        state_epoch,
        payload: canonical_payload,
    };
    let preimage = input.canonical_preimage();
    (h_act, input, preimage)
}

fn cmd_approvals(
    config_path: Option<&Path>,
    json_mode: bool,
    args: ApprovalsArgs,
    out: &mut dyn Write,
) -> Result<u8, String> {
    let sub = args.action.unwrap_or(ApprovalsSubcommand::List);
    match sub {
        ApprovalsSubcommand::Grant { action_id } | ApprovalsSubcommand::Deny { action_id } => {
            Err(format!(
                "approval write path for '{action_id}' is blocked: Gateway does not yet wire a persisted PendingApproval write endpoint (DoorController::resolve_approval returns None in crates/gateway/src/seams.rs and approval/resolve is unwired in crates/gateway/src/waist2.rs)"
            ))
        }
        ApprovalsSubcommand::List => {
            let (_cfg, paths) = load_paths(config_path)?;
            let store = open_store_readonly(&paths.data_dir)?;
            let events = store
                .read_in_order(None, usize::MAX)
                .map_err(|e| format!("failed to read store log: {e}"))?;

            let mut actions: BTreeMap<ActionId, Action> = BTreeMap::new();
            let mut held: BTreeMap<ActionId, (Option<HAct>, Option<String>, Time)> =
                BTreeMap::new();
            let mut settled: BTreeSet<ActionId> = BTreeSet::new();

            for ev in &events {
                match &ev.detail {
                    Some(EventDetail::Action {
                        action_id,
                        capability_id,
                        h_act,
                        correlation_id,
                    }) => {
                        actions.insert(
                            action_id.clone(),
                            Action {
                                protocol_version: monoagens_protocol::CURRENT_PROTOCOL_VERSION,
                                action_id: action_id.clone(),
                                capability_id: capability_id.clone(),
                                places: vec![ev.place.clone()],
                                audience: ev.audience.clone(),
                                origin_actor: ev.origin_actor.clone(),
                                taint: ev.taint,
                                payload: ev.payload.clone(),
                                correlation_id: correlation_id.clone(),
                                expected_effect: None,
                                h_act: h_act.clone(),
                                timestamp_ms: ev.timestamp_ms,
                            },
                        );
                    }
                    Some(EventDetail::Receipt {
                        action_id,
                        outcome,
                        h_act,
                        ..
                    }) => {
                        if *outcome == ReceiptOutcome::Held {
                            held.insert(
                                action_id.clone(),
                                (
                                    h_act.clone(),
                                    Some("PendingApproval".to_owned()),
                                    ev.timestamp_ms,
                                ),
                            );
                        } else {
                            settled.insert(action_id.clone());
                        }
                    }
                    _ => {}
                }
            }

            let mut entries = Vec::new();
            for (aid, (rcpt_h_act, reason_code, ts)) in held {
                if settled.contains(&aid) {
                    continue;
                }
                if let Some(action) = actions.get(&aid) {
                    let (computed_h, canonical_input, preimage) =
                        canonical_h_act_details(action, 0);
                    let h_act = rcpt_h_act.unwrap_or(computed_h);
                    entries.push(PendingApprovalEntry {
                        action_id: aid.as_str().to_owned(),
                        capability_id: action.capability_id.as_str().to_owned(),
                        places: action
                            .places
                            .iter()
                            .map(|p| p.as_str().to_owned())
                            .collect(),
                        state_epoch: 0,
                        h_act: h_act.as_str().to_owned(),
                        canonical_payload_utf8: String::from_utf8_lossy(&canonical_input.payload)
                            .into_owned(),
                        canonical_input,
                        canonical_preimage_hex: to_hex(&preimage),
                        reason_code,
                        timestamp_ms: ts.as_millis(),
                    });
                }
            }

            if json_mode {
                writeln!(
                    out,
                    "{}",
                    serde_json::to_string_pretty(&entries).map_err(|e| e.to_string())?
                )
                .map_err(|e| e.to_string())?;
            } else if entries.is_empty() {
                writeln!(out, "no pending approvals").map_err(|e| e.to_string())?;
            } else {
                for entry in entries {
                    let canonical_json =
                        serde_json::to_string(&entry.canonical_input).map_err(|e| e.to_string())?;
                    writeln!(
                        out,
                        "action={} capability={} h_act={} canonical_input={}",
                        entry.action_id, entry.capability_id, entry.h_act, canonical_json
                    )
                    .map_err(|e| e.to_string())?;
                }
            }
            Ok(0)
        }
    }
}

fn cmd_link(
    config_path: Option<&Path>,
    json_mode: bool,
    args: LinkArgs,
    out: &mut dyn Write,
) -> Result<u8, String> {
    let (cfg_opt, paths) = load_paths(config_path)?;
    let cfg = cfg_opt.unwrap_or_default();
    let log_path = paths.data_dir.join("identity_events.jsonl");
    let resolver = IdentityResolver::from_gateway_config(&cfg, Some(log_path))
        .map_err(|e| format!("failed to initialize identity resolver: {e}"))?;
    let now = SystemClock::new().now();

    match args.action {
        LinkSubcommand::List => {
            let active: Vec<serde_json::Value> = resolver
                .active_links()
                .into_iter()
                .map(|l| {
                    json!({
                        "body_id": l.body_id.as_str(),
                        "account_id": l.account_id,
                        "from_actor": l.from_actor.as_str(),
                        "to_actor": l.to_actor.as_str(),
                        "linked_at_ms": l.linked_at_ms.as_millis(),
                        "retroactive_merge": l.retroactive_merge,
                        "owner_confirmed": l.owner_confirmed,
                    })
                })
                .collect();
            let configured: Vec<serde_json::Value> = cfg
                .persons
                .iter()
                .map(|p| {
                    json!({
                        "actor_id": p.actor_id.as_str(),
                        "is_owner": p.is_owner,
                        "accounts": p.accounts.iter().map(|a| json!({
                            "body_id": a.body_id.as_str(),
                            "account_id": a.account_id,
                        })).collect::<Vec<_>>(),
                    })
                })
                .collect();
            if json_mode {
                writeln!(
                    out,
                    "{}",
                    serde_json::to_string_pretty(&json!({
                        "configured_persons": configured,
                        "active_links": active,
                    }))
                    .map_err(|e| e.to_string())?
                )
                .map_err(|e| e.to_string())?;
            } else {
                writeln!(
                    out,
                    "configured_persons={} active_links={}",
                    configured.len(),
                    active.len()
                )
                .map_err(|e| e.to_string())?;
            }
            Ok(0)
        }
        LinkSubcommand::Resolve { body, account } => {
            let actor = resolver.resolve(&BodyId::new(body.clone()), &account);
            if json_mode {
                writeln!(
                    out,
                    "{}",
                    serde_json::to_string_pretty(&json!({
                        "body_id": body,
                        "account_id": account,
                        "actor_id": actor.as_str(),
                        "heuristic_merge_enabled": resolver.heuristic_merge_enabled(),
                    }))
                    .map_err(|e| e.to_string())?
                )
                .map_err(|e| e.to_string())?;
            } else {
                writeln!(out, "{}", actor.as_str()).map_err(|e| e.to_string())?;
            }
            Ok(0)
        }
        LinkSubcommand::Initiate {
            primary_body,
            primary_account,
            secondary_body,
            secondary_account,
            emit_nonces,
        } => {
            let issued = resolver
                .initiate_bilateral_challenge(
                    &BodyId::new(primary_body),
                    &primary_account,
                    &BodyId::new(secondary_body),
                    &secondary_account,
                    now,
                )
                .map_err(|e| e.to_string())?;
            if json_mode {
                let mut payload = json!({
                    "challenge_id": issued.challenge_id,
                    "primary_actor": issued.primary_actor.as_str(),
                    "secondary_actor": issued.secondary_actor.as_str(),
                    "expires_at_ms": issued.expires_at_ms.as_millis(),
                });
                if emit_nonces {
                    payload["nonce_1"] = json!(issued.nonce_1);
                    payload["nonce_2"] = json!(issued.nonce_2);
                }
                writeln!(
                    out,
                    "{}",
                    serde_json::to_string_pretty(&payload).map_err(|e| e.to_string())?
                )
                .map_err(|e| e.to_string())?;
            } else {
                writeln!(
                    out,
                    "challenge_id={} primary={} secondary={} expires_at_ms={}",
                    issued.challenge_id,
                    issued.primary_actor.as_str(),
                    issued.secondary_actor.as_str(),
                    issued.expires_at_ms.as_millis()
                )
                .map_err(|e| e.to_string())?;
            }
            Ok(0)
        }
        LinkSubcommand::Verify {
            challenge,
            nonce1,
            nonce2,
            owner_confirmed,
        } => {
            let linked = resolver
                .verify_bilateral_challenge(&challenge, &nonce1, &nonce2, owner_confirmed, now)
                .map_err(|e| e.to_string())?;
            if json_mode {
                writeln!(
                    out,
                    "{}",
                    serde_json::to_string_pretty(&json!({
                        "status": "linked",
                        "challenge_id": challenge,
                        "actor_id": linked.as_str(),
                    }))
                    .map_err(|e| e.to_string())?
                )
                .map_err(|e| e.to_string())?;
            } else {
                writeln!(out, "linked actor={}", linked.as_str()).map_err(|e| e.to_string())?;
            }
            Ok(0)
        }
        LinkSubcommand::Unlink {
            body,
            account,
            reason,
        } => {
            resolver
                .unlink_actor(&BodyId::new(body.clone()), &account, reason, now)
                .map_err(|e| e.to_string())?;
            if json_mode {
                writeln!(
                    out,
                    "{}",
                    serde_json::to_string_pretty(&json!({
                        "status": "unlinked",
                        "body_id": body,
                        "account_id": account,
                    }))
                    .map_err(|e| e.to_string())?
                )
                .map_err(|e| e.to_string())?;
            } else {
                writeln!(out, "unlinked ({body}, {account})").map_err(|e| e.to_string())?;
            }
            Ok(0)
        }
    }
}

fn cmd_conformance(args: ConformanceArgs, out: &mut dyn Write) -> Result<u8, String> {
    let (report, json_report) = match args.target {
        ConformanceSubcommand::Body {
            target,
            target_args,
            level,
            json_report,
        } => (
            run_body_conformance(&target, &target_args, level.into()).map_err(|e| e.to_string())?,
            json_report,
        ),
        ConformanceSubcommand::Core {
            target,
            target_args,
            level,
            json_report,
        } => (
            run_core_conformance(&target, &target_args, level.into()).map_err(|e| e.to_string())?,
            json_report,
        ),
    };

    let rendered =
        serde_json::to_string_pretty(&report).map_err(|e| format!("encode report failed: {e}"))?;
    if let Some(path) = json_report {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent).map_err(|e| format!("create report dir failed: {e}"))?;
        }
        fs::write(&path, format!("{rendered}\n"))
            .map_err(|e| format!("write report failed: {e}"))?;
    }
    writeln!(out, "{rendered}").map_err(|e| e.to_string())?;
    Ok(if report.passed() { 0 } else { 1 })
}

fn extract_cedar_rule_summaries(policy_src: &str) -> Vec<String> {
    let mut rules = Vec::new();
    let mut idx = 0usize;
    for raw_line in policy_src.lines() {
        let trimmed = raw_line.trim();
        if trimmed.starts_with("permit(") || trimmed.starts_with("forbid(") {
            idx += 1;
            let kind = if trimmed.starts_with("permit") {
                "permit"
            } else {
                "forbid"
            };
            rules.push(format!("default_cedar_rule_{idx:02}_{kind}"));
        }
    }
    rules
}

fn cmd_inspector(
    config_path: Option<&Path>,
    json_mode: bool,
    args: InspectorArgs,
    out: &mut dyn Write,
) -> Result<u8, String> {
    let (cfg_opt, paths) = load_paths(config_path)?;
    let cfg = cfg_opt.unwrap_or_default();
    let valve_cfg = build_valve_config(&cfg);
    let bundle = PolicyBundle::default_verified()
        .map_err(|e| format!("cedar policy verification failed: {e}"))?;
    let proof = bundle.proof_report().clone();
    let rules = extract_cedar_rule_summaries(DEFAULT_CEDAR_POLICIES);
    let reason_tags: Vec<&'static str> = ValveReasonTag::ALL.iter().map(|t| t.as_str()).collect();

    let mut trace_events = Vec::new();
    if paths.data_dir.exists()
        && let Ok(store) = SegmentedStore::open(StoreConfig::new(paths.data_dir.clone()))
        && let Ok(events) = store.read_in_order(None, usize::MAX)
    {
        for ev in events {
            let is_relevant = matches!(
                ev.detail,
                Some(
                    EventDetail::DoorVerdict { .. }
                        | EventDetail::Action { .. }
                        | EventDetail::Receipt { .. }
                )
            );
            if !is_relevant {
                continue;
            }
            let tp_match = args
                .traceparent
                .as_deref()
                .is_none_or(|tp| serde_json::to_string(&ev).is_ok_and(|s| s.contains(tp)));
            if tp_match {
                trace_events.push(json!({
                    "id": ev.id.as_u64(),
                    "seq": ev.seq.as_u64(),
                    "kind": ev.kind.as_str(),
                    "timestamp_ms": ev.timestamp_ms.as_millis(),
                }));
            }
        }
    }

    let payload = json!({
        "valve_mode": valve_cfg.mode.as_str(),
        "symcc_proof": proof,
        "active_rule_names": rules,
        "canonical_reason_tags": reason_tags,
        "traceparent_filter": args.traceparent,
        "traces": trace_events,
    });

    if json_mode {
        writeln!(
            out,
            "{}",
            serde_json::to_string_pretty(&payload).map_err(|e| e.to_string())?
        )
        .map_err(|e| e.to_string())?;
    } else {
        writeln!(
            out,
            "valve_mode={} symcc_status={:?} solver={} counterexamples={}",
            valve_cfg.mode.as_str(),
            proof.status,
            proof.solver,
            proof.counterexamples
        )
        .map_err(|e| e.to_string())?;
        for r in &rules {
            writeln!(out, "  rule: {r}").map_err(|e| e.to_string())?;
        }
        writeln!(out, "  traces: {}", trace_events.len()).map_err(|e| e.to_string())?;
    }
    Ok(0)
}
