//! `monoagens` CLI library implementing `chat`, `log`, `replay`, `approvals`, `link`,
//! `conformance`, and `inspector` against `gateway`, `valve`, `lab`, `conformance`, and `body-cli`.

#![forbid(unsafe_code)]

use std::collections::BTreeSet;
use std::fs;
use std::io::{self, BufRead, BufReader as StdBufReader, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use body_cli::{CliBody, TerminalOutputSink};
use clap::{Args, Parser, Subcommand, ValueEnum};
use monoagens_body_kit::{EmittedCue, PersistentBodyRuntime};
use monoagens_conformance::{ConformanceLevel, run_body_conformance, run_core_conformance};
use monoagens_core::{
    ContextPlan, DecisionOutcome, DecisionReason, DecisionTrace, DoorDecision, Doors, EventDetail,
    EventKind, EventRecord, Input, LogSeq, MemStore, ReplayMode, Store,
};
use monoagens_gateway::{
    Clock, CueRecorder, DiagnosticSink, DoorController, EmbedBackendKind, GatewayConfig,
    GatewayOptions, GatewayRuntime, GatewaySeams, IdentityResolver, LifecycleCueEvent,
    LifecycleCueKind, LoadedGatewayEnv, SystemClock, ValveDoorController, load_approval_secrets,
    load_or_init_gateway_env, resolve_approval_mac_key, resolve_paths,
};
use monoagens_protocol::{
    Action, ActionCanonicalInput, ActionId, ActorId, ApprovalResolveParams, ApprovalResolveResult,
    Audience, BodyId, CapabilityDescriptor, EventId, HAct, OpaquePayload, OriginActor, Percept,
    PlaceId, Receipt, ReceiptOutcome, Taint, Time, Waist2Error, Waist2Headers,
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
    /// Deterministically replay state offline and export learned tables to canonical Markdown.
    ExportLearning(ExportLearningArgs),
}

#[derive(Debug, Default, Args)]
pub struct ChatArgs {
    /// Send one or more messages non-interactively instead of reading stdin lines.
    #[arg(long = "message")]
    pub messages: Vec<String>,
    /// Optional data/store directory override for this session.
    #[arg(long)]
    pub data_dir: Option<PathBuf>,
    /// Optional neural controller `.safetensors` model override (`soul.model_path`).
    #[arg(long)]
    pub model: Option<PathBuf>,
    /// Starting place/room id (default `cli:terminal`).
    #[arg(long)]
    pub place: Option<String>,
    /// Print per-turn neural brain decision summary (`[otak: ...]`) to stderr.
    #[arg(long)]
    pub show_brain: bool,
    /// Force interactive line-by-line REPL mode even when stdin is not a TTY.
    #[arg(long)]
    pub interactive: bool,
    /// Optional pre-registered simulated actors (REPL `/group` and `/as` also register live).
    #[arg(long, value_delimiter = ',')]
    pub actors: Vec<String>,
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

#[derive(Debug, Args)]
pub struct ExportLearningArgs {
    /// Output Markdown file path.
    #[arg(long)]
    pub out: PathBuf,
    /// Optional data/store directory override.
    #[arg(long)]
    pub data_dir: Option<PathBuf>,
    /// Optional JSON file containing an array of EventRecord values to replay offline.
    #[arg(long)]
    pub log_json: Option<PathBuf>,
    /// Optional place ID filter.
    #[arg(long)]
    pub place: Option<String>,
    /// Include private-audience (e.g. DM) learned items (excluded by default).
    #[arg(long)]
    pub include_private: bool,
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
        Commands::ExportLearning(args) => cmd_export_learning(config_opt, json_mode, args, out),
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

/// Interactive CLI chat session state for room/group/speaker simulation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplSessionState {
    pub owner_actor: ActorId,
    pub current_actor: ActorId,
    pub place: PlaceId,
    pub transport_place: PlaceId,
    pub audience: Audience,
    pub is_group: bool,
    pub show_brain: bool,
}

impl ReplSessionState {
    #[must_use]
    pub fn new(owner_actor: ActorId, place: PlaceId, show_brain: bool) -> Self {
        let audience = Audience::new([owner_actor.clone(), ActorId::new("soul")]);
        Self {
            current_actor: owner_actor.clone(),
            owner_actor,
            transport_place: place.clone(),
            place,
            audience,
            is_group: false,
            show_brain,
        }
    }

    #[must_use]
    pub fn prompt_label(&self) -> String {
        let mode = if self.is_group { "grup" } else { "dm" };
        format!("[{mode}:{} | {}]> ", self.place.as_str(), self.current_actor.as_str())
    }
}

/// Result of parsing a single REPL input line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplAction {
    Message(String),
    Notice(String),
    Quit,
}

const REPL_HELP_TEXT: &str = "\
Perintah chat interaktif MonoAgens:
  /dm                         Pindah ke DM pribadi dengan Mono (otomatis menyapa Mono)
  /room <nama_ruang>          Pindah ruang DM (mis. /room dm:catatan)
  /group <ruang> <a,b,c>      Masuk grup simulasi (mis. /group room:ta owner,budi,sari)
                              Di grup, Mono hanya otomatis disapa jika kamu menyebut @mono
  /as <nama_pengirim>         Ganti pengirim pesan (mis. /as budi atau /as owner)
  /brain                      Tampilkan / sembunyikan status otak [otak: ...]
  /status                     Lihat ruangan, pengirim, audiens, & status otak saat ini
  /help                       Tampilkan bantuan ini
  /quit atau /exit            Keluar dari chat";

/// Parses a single user input line, mutating `state` for slash commands.
#[must_use]
pub fn parse_repl_command(raw: &str, state: &mut ReplSessionState) -> ReplAction {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return ReplAction::Notice(String::new());
    }
    if !trimmed.starts_with('/') {
        return ReplAction::Message(trimmed.to_owned());
    }

    let mut parts = trimmed.split_whitespace();
    let cmd = parts.next().unwrap_or("");
    match cmd {
        "/quit" | "/exit" | "/q" => ReplAction::Quit,
        "/help" | "/h" => ReplAction::Notice(REPL_HELP_TEXT.to_owned()),
        "/brain" => {
            state.show_brain = !state.show_brain;
            ReplAction::Notice(format!(
                "Status otak [otak: ...]: {}",
                if state.show_brain { "AKTIF" } else { "MATI" }
            ))
        }
        "/dm" => {
            state.place = state.transport_place.clone();
            state.current_actor = state.owner_actor.clone();
            state.audience = Audience::new([state.owner_actor.clone(), ActorId::new("soul")]);
            state.is_group = false;
            ReplAction::Notice(format!("Pindah ke DM pribadi ({}).", state.place.as_str()))
        }
        "/room" => {
            let Some(room) = parts.next() else {
                return ReplAction::Notice("Pakai: /room <nama_ruang>".to_owned());
            };
            state.place = PlaceId::new(room);
            state.audience = Audience::new([state.current_actor.clone(), ActorId::new("soul")]);
            state.is_group = false;
            ReplAction::Notice(format!("Pindah ke ruang DM '{}'.", state.place.as_str()))
        }
        "/group" => {
            let tokens: Vec<&str> = parts.collect();
            if tokens.is_empty() {
                return ReplAction::Notice(
                    "Pakai: /group <nama_ruang> <anggota1,anggota2,...>".to_owned(),
                );
            }
            let (room_name, members_raw) = if tokens.len() >= 2 && tokens.last().is_some_and(|t| t.contains(',')) {
                (tokens[..tokens.len() - 1].join("_"), *tokens.last().expect("checked"))
            } else if tokens.len() >= 2 {
                (tokens[0].to_owned(), tokens[1])
            } else {
                (tokens[0].to_owned(), "")
            };
            let mut members: Vec<ActorId> = members_raw
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|m| {
                    if m.eq_ignore_ascii_case("owner") {
                        state.owner_actor.clone()
                    } else {
                        ActorId::new(m)
                    }
                })
                .collect();
            if members.is_empty() {
                members.push(state.owner_actor.clone());
                members.push(ActorId::new("budi"));
                members.push(ActorId::new("sari"));
            }
            if !members.contains(&state.current_actor) {
                members.push(state.current_actor.clone());
            }
            members.push(ActorId::new("soul"));
            state.place = PlaceId::new(room_name);
            state.audience = Audience::new(members);
            state.is_group = true;
            let names: Vec<&str> = state.audience.members.iter().map(ActorId::as_str).collect();
            ReplAction::Notice(format!(
                "Masuk grup '{}' (audiens: [{}]). Sebut @mono untuk menyapa Mono langsung.",
                state.place.as_str(),
                names.join(", ")
            ))
        }
        "/as" => {
            let Some(who) = parts.next() else {
                return ReplAction::Notice("Pakai: /as <nama_pengirim>".to_owned());
            };
            state.current_actor = if who.eq_ignore_ascii_case("owner") {
                state.owner_actor.clone()
            } else {
                ActorId::new(who)
            };
            if !state.audience.members.contains(&state.current_actor) {
                let mut members = state.audience.members.clone();
                members.insert(state.current_actor.clone());
                state.audience = Audience::new(members);
            }
            ReplAction::Notice(format!(
                "Sekarang berbicara sebagai '{}'.",
                state.current_actor.as_str()
            ))
        }
        "/status" => {
            let names: Vec<&str> = state.audience.members.iter().map(ActorId::as_str).collect();
            ReplAction::Notice(format!(
                "ruang={} | mode={} | pengirim={} | audiens=[{}] | otak={}",
                state.place.as_str(),
                if state.is_group { "grup" } else { "dm" },
                state.current_actor.as_str(),
                names.join(", "),
                if state.show_brain { "aktif" } else { "mati" },
            ))
        }
        other => ReplAction::Notice(format!(
            "Perintah tidak dikenal '{other}'. Ketik /help untuk daftar perintah."
        )),
    }
}

/// Constructs a canonical Waist-2 percept from the active REPL session state.
///
/// In DM mode, messages are automatically addressed to `soul`. In group mode, `soul` is added
/// to `addressees`/`named` only when the text explicitly mentions `@mono` or `@soul`
/// (case-insensitive), allowing natural testing of `Silent` / `Wait` / `Go` in group rooms.
pub fn build_repl_percept(
    state: &ReplSessionState,
    text: &str,
    now: monoagens_protocol::Time,
    turn_idx: usize,
) -> Result<monoagens_protocol::Percept, String> {
    let helper_body = CliBody::new(
        BodyId::new("body-cli"),
        state.transport_place.clone(),
        state.current_actor.clone(),
        4096,
        SharedChatSink {
            rendered: Arc::new(Mutex::new(Vec::new())),
        },
    );
    let (percept_params, _) =
        helper_body.build_user_percept(text, None, None, now, format!("cli-chat-{turn_idx}"));
    let mut percept = percept_params
        .into_percept(monoagens_protocol::CURRENT_PROTOCOL_VERSION)
        .map_err(|e| e.to_string())?;
    percept.place = state.transport_place.clone();
    percept.actor = state.current_actor.clone();
    percept.origin_actor = Some(OriginActor::new(state.current_actor.clone()));
    percept.audience = state.audience.clone();
    percept.taint = if state.current_actor == state.owner_actor {
        Taint::Trusted
    } else {
        Taint::Untrusted
    };

    let lower = text.to_ascii_lowercase();
    let mentions_soul = lower.contains("@mono") || lower.contains("@soul");
    if state.is_group {
        percept.addressees.clear();
        percept.named.clear();
        if mentions_soul {
            percept.addressees.push(ActorId::new("soul"));
            percept.named.push(ActorId::new("soul"));
        }
    } else if percept.addressees.is_empty() {
        percept.addressees.push(ActorId::new("soul"));
    }
    Ok(percept)
}

/// Formats a compact human-readable one-line summary of a turn's [`DecisionTrace`].
#[must_use]
pub fn format_decision_trace_summary(trace: &DecisionTrace) -> String {
    let outcome = match trace.outcome {
        DecisionOutcome::Go => "Go",
        DecisionOutcome::Silent => "Silent",
        DecisionOutcome::Note => "Note",
        DecisionOutcome::DoorBlocked => "DoorBlocked",
        DecisionOutcome::Processed => "Processed",
    };
    let tier = trace.neural.compute_tier.map_or("—", |t| match t {
        monoagens_core::ComputeTier::Fast => "Fast",
        monoagens_core::ComputeTier::Standard => "Standard",
        monoagens_core::ComputeTier::Deep => "Deep",
    });
    let mut summary = format!(
        "[otak: {outcome} | reason={:?} | tier={tier} | p_go={:.2} | p_wait={:.2}",
        trace.reason, trace.neural.gate_probs[3], trace.neural.gate_probs[2]
    );
    if trace.neural.wait_delta_ms > 0.0
        && trace.reason == monoagens_core::DecisionReason::WaitDeferral
    {
        summary.push_str(&format!(" ({:.0}ms)", trace.neural.wait_delta_ms));
    }
    if let Some(p_corr) = trace.neural.p_user_correction {
        summary.push_str(&format!(" | p_corr={p_corr:.2}"));
    }
    if trace.neural.is_neutral_fallback {
        summary.push_str(" | neutral");
    }
    summary.push(']');
    summary
}

fn find_latest_decision_since(
    events: &[EventRecord],
    since_len: usize,
) -> Option<(DecisionOutcome, Option<DecisionTrace>)> {
    events.get(since_len..)?.iter().rev().find_map(|ev| {
        if let Some(EventDetail::Decision { outcome, trace, .. }) = &ev.detail {
            Some((*outcome, trace.as_deref().cloned()))
        } else {
            None
        }
    })
}

struct CliInteractiveDoorController {
    inner: ValveDoorController,
    dynamic_actors: Arc<Mutex<BTreeSet<ActorId>>>,
}

impl CliInteractiveDoorController {
    fn sync_dynamic_actors(&mut self) {
        let guard = self.dynamic_actors.lock().expect("dynamic actors lock");
        let known = &mut self.inner.valve_mut().snapshot_mut().known_actors;
        for actor in guard.iter() {
            known.insert(actor.clone());
        }
    }
}

impl Doors for CliInteractiveDoorController {
    fn inbound(&mut self, percept: &mut Percept) -> DoorDecision {
        self.sync_dynamic_actors();
        self.inner.inbound(percept)
    }

    fn context(&mut self, plan: &mut ContextPlan) -> DoorDecision {
        self.sync_dynamic_actors();
        self.inner.context(plan)
    }

    fn action(&mut self, action: &mut Action) -> DoorDecision {
        self.inner.action(action)
    }
}

impl DoorController for CliInteractiveDoorController {
    fn sync_runtime_state(
        &mut self,
        log_seq: LogSeq,
        now: Time,
        daily_tokens_used: u64,
        capabilities: &[CapabilityDescriptor],
    ) {
        self.inner
            .sync_runtime_state(log_seq, now, daily_tokens_used, capabilities);
    }

    fn sync_state_epoch(&mut self, state_epoch: u64) {
        self.inner.sync_state_epoch(state_epoch);
    }

    fn replay_from_store_events(&mut self, events: &[EventRecord]) {
        self.inner.replay_from_store_events(events);
    }

    fn sync_item_audiences(&mut self, events: &[EventRecord]) {
        self.inner.sync_item_audiences(events);
    }

    fn resolve_approval(
        &mut self,
        params: &ApprovalResolveParams,
    ) -> Option<Result<ApprovalResolveResult, Waist2Error>> {
        self.inner.resolve_approval(params)
    }

    fn record_in_band_rejection_audit(&mut self, params: &ApprovalResolveParams, detail: &str) {
        self.inner.record_in_band_rejection_audit(params, detail);
    }

    fn take_pending_approval_events(&mut self) -> Vec<EventRecord> {
        self.inner.take_pending_approval_events()
    }

    fn take_approved_action(&mut self, action_id: &ActionId) -> Option<Action> {
        self.inner.take_approved_action(action_id)
    }

    fn take_pending_denial_receipt(&mut self, action_id: &ActionId) -> Option<Receipt> {
        self.inner.take_pending_denial_receipt(action_id)
    }
}

fn register_session_actors(
    dynamic_actors: &Arc<Mutex<BTreeSet<ActorId>>>,
    session: &ReplSessionState,
) {
    let mut guard = dynamic_actors.lock().expect("dynamic actors lock");
    if session.current_actor.as_str() != "soul" {
        guard.insert(session.current_actor.clone());
    }
    for member in &session.audience.members {
        if member.as_str() != "soul" {
            guard.insert(member.clone());
        }
    }
}

struct CliSidecarGuard {
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
    if manifest.join("lab/adapters/eval_helpers/embeddinggemma2_sidecar.py").is_file() {
        return manifest;
    }
    if let Ok(cwd) = std::env::current_dir()
        && cwd.join("lab/adapters/eval_helpers/embeddinggemma2_sidecar.py").is_file()
    {
        return cwd;
    }
    manifest
}

fn maybe_start_chat_embed_sidecar(
    cfg: &mut GatewayConfig,
    json_mode: bool,
    err: &mut dyn Write,
) -> Result<Option<CliSidecarGuard>, String> {
    if cfg.embed.backend != EmbedBackendKind::EmbeddingGemma2Http {
        return Ok(None);
    }
    if cfg.embed.endpoint_url.as_deref().is_some_and(|s| !s.trim().is_empty())
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
        let _ = writeln!(err, "[embedder] memuat EmbeddingGemma 2 dari {} ...", model_dir.display());
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

async fn enrich_and_submit_percept(
    handle: &monoagens_gateway::GatewayHandle,
    mut percept: Percept,
) -> Result<monoagens_gateway::TurnOutcome, monoagens_gateway::GatewayError> {
    if let Some(pq) = handle.pre_queue_service() {
        let _ = pq.enrich_percept_async(&mut percept).await;
    }
    handle.submit_and_wait(Input::Percept(Box::new(percept))).await
}

fn drain_async_chat_outputs(
    cue_rx: &mut tokio::sync::mpsc::UnboundedReceiver<LifecycleCueEvent>,
    rendered_bubbles: &Arc<Mutex<Vec<(String, String)>>>,
    rendered_cursor: &mut usize,
    emitted_outputs: &mut Vec<String>,
    out: &mut dyn Write,
    err: &mut dyn Write,
    leading_newline: bool,
) -> bool {
    let mut printed_any = false;
    while let Ok(cue) = cue_rx.try_recv() {
        let cue_label = match cue.kind {
            LifecycleCueKind::WorkingCue => "working_cue",
            LifecycleCueKind::ProgressNote => "progress_note",
        };
        if leading_newline && !printed_any {
            let _ = writeln!(err);
        }
        let _ = writeln!(err, "[{cue_label}] {}", cue.detail);
        let _ = err.flush();
        printed_any = true;
    }
    let new_items = {
        let guard = rendered_bubbles.lock().expect("rendered lock");
        if guard.len() > *rendered_cursor {
            let slice = guard[*rendered_cursor..].to_vec();
            *rendered_cursor = guard.len();
            slice
        } else {
            Vec::new()
        }
    };
    if !new_items.is_empty() {
        for (_pid, text) in new_items {
            if leading_newline && !printed_any {
                let _ = writeln!(out);
            }
            let _ = writeln!(out, "Mono> {text}");
            let _ = out.flush();
            emitted_outputs.push(text);
            printed_any = true;
        }
    }
    printed_any
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
    for actor in &args.actors {
        let trimmed = actor.trim();
        if !trimmed.is_empty() {
            snapshot.known_actors.insert(ActorId::new(trimmed));
        }
    }

    let approval_secrets =
        load_approval_secrets(Some(&loaded.paths.secrets_path), |k| std::env::var(k).ok())
            .map_err(|e| e.to_string())?;
    let mac_key = resolve_approval_mac_key(&approval_secrets);

    let dynamic_actors = Arc::new(Mutex::new(BTreeSet::new()));
    {
        let mut guard = dynamic_actors.lock().expect("dynamic actors lock");
        guard.insert(owner_actor.clone());
        for actor in &snapshot.known_actors {
            guard.insert(actor.clone());
        }
    }

    // Build Valve (runs cedar-policy-symcc cvc5 verification) BEFORE entering Tokio runtime.
    let mut valve = Valve::with_default_policies(valve_cfg, snapshot)
        .map_err(|e| format!("valve policy verification failed: {e}"))?;
    valve.set_approval_attestation_key(mac_key);
    let door_controller = CliInteractiveDoorController {
        inner: ValveDoorController::new(valve),
        dynamic_actors: Arc::clone(&dynamic_actors),
    };

    let mut cfg = loaded.config.clone();
    let store_dir = args
        .data_dir
        .unwrap_or_else(|| loaded.paths.data_dir.clone());
    cfg.data_dir = Some(store_dir.clone());
    if let Some(model_override) = args.model {
        cfg.soul.model_path = Some(model_override);
    }
    let _embed_sidecar_guard = if provider_override.is_none() {
        maybe_start_chat_embed_sidecar(&mut cfg, json_mode, err)?
    } else {
        None
    };

    let start_place = PlaceId::new(
        args.place
            .as_deref()
            .unwrap_or("cli:terminal"),
    );
    let is_interactive =
        args.interactive || (args.messages.is_empty() && !json_mode && io::stdin().is_terminal());
    let show_brain = args.show_brain || is_interactive;
    let max_wait_polls: usize = if provider_override.is_some() { 100 } else { 12_000 }; // 500ms scripted vs 60s live LLM
    let neural_model_label = cfg
        .soul
        .model_path
        .as_ref()
        .map_or_else(|| "neutral (tanpa model)".to_owned(), |p| p.display().to_string());

    let batch_inputs: Vec<String> = if is_interactive {
        Vec::new()
    } else if !args.messages.is_empty() {
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

    let result: Result<(Vec<String>, Vec<EventRecord>), String> = rt.block_on(async {
        let clock = Arc::new(SystemClock::new());
        let diagnostics = DiagnosticSink::new(&loaded.secrets);
        let cues = CueRecorder::new();
        let mut cue_rx = cues.subscribe();

        let runtime = GatewayRuntime::start_segmented_with_configured_core(
            cfg,
            &loaded.secrets,
            store_dir,
            door_controller,
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
        let body_place = start_place.clone();
        let body_handle = tokio::spawn(async move {
            let mut body_rt = PersistentBodyRuntime::new(None);
            let mut cli_body = CliBody::new(
                BodyId::new("body-cli"),
                body_place,
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

        let mut session = ReplSessionState::new(owner_actor, start_place, show_brain);
        let mut emitted_outputs = Vec::new();
        let mut rendered_cursor = 0usize;

        if is_interactive {
            let _ = writeln!(out, "=== MonoAgens Interactive Chat (V2.2) ===");
            let _ = writeln!(out, "Otak Sistem 1 : {neural_model_label}");
            let _ = writeln!(out, "Ketik pesan lalu Enter. Ketik /help untuk perintah ruangan/grup, /quit untuk keluar.\n");
            let _ = out.flush();

            let (line_tx, mut line_rx) = tokio::sync::mpsc::unbounded_channel::<io::Result<String>>();
            std::thread::spawn(move || {
                let stdin = io::stdin();
                let mut locked = stdin.lock();
                loop {
                    let mut buf = String::new();
                    match locked.read_line(&mut buf) {
                        Ok(0) => break,
                        Ok(_) => {
                            if line_tx.send(Ok(buf)).is_err() {
                                break;
                            }
                        }
                        Err(e) => {
                            let _ = line_tx.send(Err(e));
                            break;
                        }
                    }
                }
            });

            let mut turn_idx = 0usize;
            let mut async_tick = tokio::time::interval(Duration::from_millis(50));
            async_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

            'repl: loop {
                let _ = write!(out, "{}", session.prompt_label());
                let _ = out.flush();

                let raw = loop {
                    tokio::select! {
                        maybe_line = line_rx.recv() => {
                            match maybe_line {
                                Some(Ok(line)) => break line,
                                Some(Err(e)) => return Err(format!("stdin read error: {e}")),
                                None => break 'repl,
                            }
                        }
                        _ = async_tick.tick() => {
                            if drain_async_chat_outputs(
                                &mut cue_rx,
                                &rendered_bubbles,
                                &mut rendered_cursor,
                                &mut emitted_outputs,
                                out,
                                err,
                                true,
                            ) {
                                let _ = write!(out, "{}", session.prompt_label());
                                let _ = out.flush();
                            }
                        }
                    }
                };

                match parse_repl_command(&raw, &mut session) {
                    ReplAction::Quit => break,
                    ReplAction::Notice(msg) => {
                        register_session_actors(&dynamic_actors, &session);
                        if !msg.is_empty() {
                            let _ = writeln!(out, "{msg}");
                            let _ = out.flush();
                        }
                        continue;
                    }
                    ReplAction::Message(msg) => {
                        register_session_actors(&dynamic_actors, &session);
                        let before_events = handle.read_events().await.unwrap_or_default().len();
                        let now = clock.now();
                        let percept = build_repl_percept(&session, &msg, now, turn_idx)?;
                        turn_idx += 1;

                        enrich_and_submit_percept(&handle, percept)
                            .await
                            .map_err(|e| format!("chat turn failed: {e}"))?;

                        let post_events = handle.read_events().await.unwrap_or_default();
                        let decision = find_latest_decision_since(&post_events, before_events);
                        if session.show_brain
                            && let Some((outcome, ref tr_opt)) = decision
                        {
                            if let Some(tr) = tr_opt {
                                let _ = writeln!(err, "{}", format_decision_trace_summary(tr));
                            } else {
                                let _ = writeln!(err, "[otak: {outcome:?} | ditahan Valve inbound]");
                            }
                            let _ = err.flush();
                        }

                        let wait_armed = decision.as_ref().is_some_and(|(_, tr_opt)| {
                            tr_opt
                                .as_ref()
                                .is_some_and(|tr| tr.reason == DecisionReason::WaitDeferral)
                        });
                        if session.show_brain && wait_armed {
                            let _ = writeln!(
                                err,
                                "[otak] menunggu sebentar; Mono akan bantu otomatis jika belum ada yang menjawab."
                            );
                            let _ = err.flush();
                        }

                        let expects_reply = decision
                            .as_ref()
                            .is_none_or(|(outcome, _)| *outcome == DecisionOutcome::Go);
                        if expects_reply {
                            let mut got_reply = false;
                            let mut failure_msg: Option<String> = None;
                            for poll in 0..max_wait_polls {
                                let prev_out_len = emitted_outputs.len();
                                let _ = drain_async_chat_outputs(
                                    &mut cue_rx,
                                    &rendered_bubbles,
                                    &mut rendered_cursor,
                                    &mut emitted_outputs,
                                    out,
                                    err,
                                    false,
                                );
                                if emitted_outputs.len() > prev_out_len {
                                    got_reply = true;
                                    break;
                                }
                                if poll > 0 && poll % 40 == 0 {
                                    let cur_events = handle.read_events().await.unwrap_or_default();
                                    if let Some(slice) = cur_events.get(before_events..) {
                                        let mut has_action = false;
                                        let mut thought_text: Option<String> = None;
                                        let mut thought_processed = false;
                                        for ev in slice {
                                            if let Some(EventDetail::Failure {
                                                component, detail, ..
                                            }) = &ev.detail
                                            {
                                                failure_msg =
                                                    Some(format!("[error {component}] {detail}"));
                                                break;
                                            }
                                            match ev.kind {
                                                EventKind::Action => has_action = true,
                                                EventKind::Thought => {
                                                    let t = String::from_utf8_lossy(
                                                        ev.payload.as_bytes(),
                                                    )
                                                    .trim()
                                                    .to_owned();
                                                    if !t.is_empty() {
                                                        thought_text = Some(t);
                                                    }
                                                }
                                                EventKind::Decision => {
                                                    if let Some(EventDetail::Decision {
                                                        outcome: DecisionOutcome::Processed,
                                                        ..
                                                    }) = &ev.detail
                                                    {
                                                        thought_processed = true;
                                                    }
                                                }
                                                _ => {}
                                            }
                                        }
                                        if failure_msg.is_some() {
                                            break;
                                        }
                                        if thought_processed && !has_action {
                                            if let Some(t) = thought_text {
                                                let _ = writeln!(err, "[pikiran Mono (tanpa kirim pesan)] {t}");
                                            } else {
                                                let _ = writeln!(err, "[otak] Mono selesai berpikir dan memilih tidak mengirim pesan.");
                                            }
                                            let _ = err.flush();
                                            got_reply = true;
                                            break;
                                        }
                                    }
                                }
                                tokio::time::sleep(Duration::from_millis(5)).await;
                            }
                            if let Some(msg) = failure_msg {
                                let _ = writeln!(err, "{msg}");
                                let _ = err.flush();
                            } else if !got_reply {
                                let _ = writeln!(
                                    err,
                                    "[info] belum ada balasan dalam batas waktu (cek log/endpoint LLM)."
                                );
                                let _ = err.flush();
                            }
                        }
                    }
                }
            }
        } else {
            for (idx, msg) in batch_inputs.iter().enumerate() {
                let before_events = handle.read_events().await.unwrap_or_default().len();
                let now = clock.now();
                let percept = build_repl_percept(&session, msg, now, idx)?;
                enrich_and_submit_percept(&handle, percept)
                    .await
                    .map_err(|e| format!("chat turn failed: {e}"))?;

                let post_events = handle.read_events().await.unwrap_or_default();
                let decision = find_latest_decision_since(&post_events, before_events);
                if session.show_brain
                    && !json_mode
                    && let Some((outcome, ref tr_opt)) = decision
                {
                    if let Some(tr) = tr_opt {
                        let _ = writeln!(err, "{}", format_decision_trace_summary(tr));
                    } else {
                        let _ = writeln!(err, "[otak: {outcome:?}]");
                    }
                }

                let expects_reply = decision
                    .as_ref()
                    .is_none_or(|(outcome, _)| *outcome == DecisionOutcome::Go);
                if !expects_reply {
                    continue;
                }

                // Drain any asynchronous model/action turn briefly without blocking fast responses (d_art = 0 ms).
                for _ in 0..max_wait_polls {
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
    } else if !is_interactive {
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
    audience: Vec<String>,
    taint: String,
    state_epoch: u64,
    h_act: String,
    canonical_input: ActionCanonicalInput,
    canonical_payload_utf8: String,
    canonical_preimage_hex: String,
    reason_code: Option<String>,
    timestamp_ms: u64,
    expires_at_ms: u64,
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

fn taint_label(taint: Taint) -> &'static str {
    match taint {
        Taint::Trusted => "trusted",
        Taint::Provisional => "provisional",
        Taint::Untrusted => "untrusted",
    }
}

fn owner_actor_from_cfg(cfg: Option<&GatewayConfig>) -> ActorId {
    cfg.and_then(|c| c.persons.iter().find(|p| p.is_owner))
        .map(|p| p.actor_id.clone())
        .unwrap_or_else(|| ActorId::new("per_owner"))
}

#[allow(clippy::too_many_arguments)]
fn resolve_cli_approval(
    data_dir: &Path,
    owner_actor: &ActorId,
    valve_cfg: &ValveConfig,
    mac_key: [u8; 32],
    action_id: &str,
    approved: bool,
    json_mode: bool,
    out: &mut dyn Write,
) -> Result<u8, String> {
    let mut store = open_store_readonly(data_dir)?;
    let events = store
        .read_in_order(None, usize::MAX)
        .map_err(|e| format!("failed to read store log: {e}"))?;

    let now = SystemClock::new().now();
    let mut snapshot = ValveSnapshot {
        owner_actor_id: Some(owner_actor.clone()),
        t_now: now,
        ..ValveSnapshot::default()
    };
    snapshot.known_actors.insert(owner_actor.clone());
    let mut valve = Valve::with_default_policies(valve_cfg.clone(), snapshot)
        .map_err(|e| format!("valve policy verification failed: {e}"))?;
    valve.set_approval_attestation_key(mac_key);

    let mut controller = ValveDoorController::new(valve);
    controller.replay_from_store_events(&events);
    controller.valve_mut().snapshot_mut().t_now = now;

    let target_id = ActionId::new(action_id);
    let pending = controller
        .valve()
        .approval_manager()
        .pending()
        .get(&target_id)
        .cloned()
        .ok_or_else(|| format!("no pending approval found for action '{action_id}'"))?;

    let signature = controller.valve().sign_approval_resolution(
        &pending.action_id,
        &pending.h_act,
        pending.state_epoch,
        approved,
        now,
    );
    let params = ApprovalResolveParams {
        headers: Waist2Headers::new(format!("cli-approval-{action_id}-{}", now.as_millis())),
        action_id: pending.action_id.clone(),
        h_act: pending.h_act.clone(),
        state_epoch: pending.state_epoch,
        approved,
        resolver: owner_actor.clone(),
        resolved_at_ms: now,
        signature: Some(signature),
        out_of_band_channel: true,
    };

    let res = controller
        .resolve_approval(&params)
        .ok_or_else(|| "approval capability unavailable".to_owned())?
        .map_err(|e| format!("approval resolution rejected: {e}"))?;

    for ev in controller.take_pending_approval_events() {
        store
            .append(ev)
            .map_err(|e| format!("failed to persist approval resolution event: {e}"))?;
    }
    if !approved && let Some(receipt) = controller.take_pending_denial_receipt(&target_id) {
        let place = pending
            .places
            .first()
            .cloned()
            .unwrap_or_else(|| PlaceId::new("cli:approvals"));
        let denial_rcpt = EventRecord::new_root(
            EventKind::Receipt,
            now,
            owner_actor.clone(),
            place,
            Audience::new([params.resolver.clone()]),
            OriginActor::new(params.resolver.clone()),
            Taint::Trusted,
            monoagens_core::NumericFeatures::default(),
            OpaquePayload::default(),
            Vec::new(),
            Some(monoagens_core::EventDetail::Receipt {
                action_id: receipt.action_id,
                capability_id: receipt.capability_id,
                outcome: ReceiptOutcome::Refused,
                termination_reason: receipt.termination_reason,
                progress: receipt.progress,
                delivered_units: receipt.delivered_units,
                h_act: receipt.h_act,
            }),
        );
        store
            .append(denial_rcpt)
            .map_err(|e| format!("failed to persist denial receipt: {e}"))?;
    }

    if json_mode {
        let payload = json!({
            "applied": res.applied,
            "action_id": pending.action_id.as_str(),
            "approved": approved,
            "state_epoch": pending.state_epoch,
            "h_act": pending.h_act.as_str(),
            "out_of_band_channel": true,
        });
        writeln!(
            out,
            "{}",
            serde_json::to_string_pretty(&payload).map_err(|e| e.to_string())?
        )
        .map_err(|e| e.to_string())?;
    } else {
        let verb = if approved { "granted" } else { "denied" };
        writeln!(
            out,
            "{verb} action={} state_epoch={} h_act={}",
            pending.action_id.as_str(),
            pending.state_epoch,
            pending.h_act.as_str()
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(0)
}

fn cmd_approvals(
    config_path: Option<&Path>,
    json_mode: bool,
    args: ApprovalsArgs,
    out: &mut dyn Write,
) -> Result<u8, String> {
    let sub = args.action.unwrap_or(ApprovalsSubcommand::List);
    let (cfg, paths) = load_paths(config_path)?;
    let owner_actor = owner_actor_from_cfg(cfg.as_ref());
    let valve_cfg = cfg.as_ref().map(build_valve_config).unwrap_or_default();
    let approval_secrets =
        load_approval_secrets(Some(&paths.secrets_path), |k| std::env::var(k).ok())
            .map_err(|e| e.to_string())?;
    let mac_key = resolve_approval_mac_key(&approval_secrets);

    match sub {
        ApprovalsSubcommand::Grant { action_id } => {
            resolve_cli_approval(
                &paths.data_dir,
                &owner_actor,
                &valve_cfg,
                mac_key,
                &action_id,
                true,
                json_mode,
                out,
            )
        }
        ApprovalsSubcommand::Deny { action_id } => {
            resolve_cli_approval(
                &paths.data_dir,
                &owner_actor,
                &valve_cfg,
                mac_key,
                &action_id,
                false,
                json_mode,
                out,
            )
        }
        ApprovalsSubcommand::List => {
            let store = open_store_readonly(&paths.data_dir)?;
            let events = store
                .read_in_order(None, usize::MAX)
                .map_err(|e| format!("failed to read store log: {e}"))?;

            let mut manager = monoagens_valve::ApprovalManager::with_attestation_key(mac_key);
            let replayed_epoch =
                manager.replay_from_events(&events, Some(&owner_actor), &valve_cfg);

            let mut entries = Vec::new();
            for (aid, pending) in manager.pending() {
                let action = manager.pending_action(aid).cloned().unwrap_or_else(|| Action {
                    protocol_version: monoagens_protocol::CURRENT_PROTOCOL_VERSION,
                    action_id: pending.action_id.clone(),
                    capability_id: pending.capability_id.clone(),
                    places: pending.places.clone(),
                    audience: Audience::new([owner_actor.clone()]),
                    origin_actor: OriginActor::new(owner_actor.clone()),
                    taint: manager.pending_taint(aid).unwrap_or(Taint::Provisional),
                    payload: OpaquePayload::default(),
                    correlation_id: None,
                    expected_effect: None,
                    h_act: Some(pending.h_act.clone()),
                    timestamp_ms: pending.requested_at_ms,
                });
                let epoch = pending.state_epoch.max(replayed_epoch);
                let (_computed_h, canonical_input, preimage) =
                    canonical_h_act_details(&action, epoch);
                let taint = manager.pending_taint(aid).unwrap_or(action.taint);
                let audience: Vec<String> = action
                    .audience
                    .members
                    .iter()
                    .map(|m| m.as_str().to_owned())
                    .collect();
                entries.push(PendingApprovalEntry {
                    action_id: aid.as_str().to_owned(),
                    capability_id: pending.capability_id.as_str().to_owned(),
                    places: pending
                        .places
                        .iter()
                        .map(|p| p.as_str().to_owned())
                        .collect(),
                    audience,
                    taint: taint_label(taint).to_owned(),
                    state_epoch: epoch,
                    h_act: pending.h_act.as_str().to_owned(),
                    canonical_payload_utf8: String::from_utf8_lossy(&canonical_input.payload)
                        .into_owned(),
                    canonical_input,
                    canonical_preimage_hex: to_hex(&preimage),
                    reason_code: pending.reason.clone(),
                    timestamp_ms: pending.requested_at_ms.as_millis(),
                    expires_at_ms: pending.expires_at_ms.as_millis(),
                });
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
                    let h_short = if entry.h_act.len() > 23 {
                        &entry.h_act[..23]
                    } else {
                        &entry.h_act
                    };
                    writeln!(
                        out,
                        "capability={} action={} expires_at_ms={}\n  places=[{}] audience=[{}] taint={} reason={}\n  h_act={} state_epoch={}",
                        entry.capability_id,
                        entry.action_id,
                        entry.expires_at_ms,
                        entry.places.join(","),
                        entry.audience.join(","),
                        entry.taint,
                        entry.reason_code.as_deref().unwrap_or("PendingApproval"),
                        h_short,
                        entry.state_epoch,
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

fn is_private_audience(aud: &Audience, soul_id: &ActorId) -> bool {
    if aud.is_empty() {
        return true;
    }
    let non_soul_count = aud.members.iter().filter(|a| *a != soul_id).count();
    non_soul_count <= 1
}

fn format_audience_label(aud: &Audience) -> String {
    if aud.is_empty() {
        return "empty".to_owned();
    }
    let members: Vec<&str> = aud.members.iter().map(ActorId::as_str).collect();
    members.join(",")
}

fn place_is_private(
    place: &PlaceId,
    participants: Option<&BTreeSet<ActorId>>,
    soul_id: &ActorId,
) -> bool {
    match participants {
        Some(set) if !set.is_empty() => set.iter().filter(|a| *a != soul_id).count() <= 1,
        _ => place.as_str().starts_with("dm:"),
    }
}

fn cmd_export_learning(
    config_path: Option<&Path>,
    json_mode: bool,
    args: ExportLearningArgs,
    out: &mut dyn Write,
) -> Result<u8, String> {
    use std::fmt::Write as _;

    let (_cfg_opt, paths) = load_paths(config_path)?;
    let events: Vec<EventRecord> = if let Some(log_path) = &args.log_json {
        let raw = fs::read_to_string(log_path)
            .map_err(|e| format!("failed to read log JSON '{}': {e}", log_path.display()))?;
        serde_json::from_str(&raw)
            .map_err(|e| format!("failed to parse log JSON '{}': {e}", log_path.display()))?
    } else {
        let data_dir = args.data_dir.clone().unwrap_or(paths.data_dir);
        let store = open_store_readonly(&data_dir)?;
        store
            .read_in_order(None, usize::MAX)
            .map_err(|e| format!("failed to read store log: {e}"))?
    };

    let mut mem = MemStore::new();
    for rec in &events {
        mem.append(rec.clone())
            .map_err(|e| format!("offline replay store build failed: {e}"))?;
    }
    let report = monoagens_core::replay_log(&mem, ReplayMode::Unpinned, None)
        .map_err(|e| format!("offline replay failed: {e}"))?;
    let state = &report.state;
    let soul_id = &state.soul_actor_id;

    let model_blake3 = events
        .iter()
        .rev()
        .find_map(|ev| match &ev.detail {
            Some(EventDetail::Decision {
                trace: Some(tr), ..
            }) => tr.model_blake3.map(|h| monoagens_core::blake3::to_hex(&h)),
            _ => None,
        })
        .unwrap_or_else(|| "none".to_owned());

    let place_filter = args.place.as_deref().map(PlaceId::new);
    let mut body = String::new();

    // Section 2: Per-place adaptation equalizer.
    let mut adapt_count = 0usize;
    body.push_str("## 2. Per-Place Adaptation Equalizer\n\n");
    for (place, ad) in &state.place_adapt {
        if let Some(pf) = &place_filter
            && place != pf
        {
            continue;
        }
        let participants = state.place_participants.get(place);
        let is_priv = place_is_private(place, participants, soul_id);
        if is_priv && !args.include_private {
            continue;
        }
        let aud_label = participants.map_or_else(
            || if is_priv { "private" } else { "group" }.to_owned(),
            |set| {
                set.iter()
                    .map(ActorId::as_str)
                    .collect::<Vec<_>>()
                    .join(",")
            },
        );
        let _ = writeln!(
            body,
            "- **place={}** audience=[{}] logit_offsets=[{:.4}, {:.4}, {:.4}, {:.4}] film_scale_deltas=[{:.4}, {:.4}] film_shift_deltas=[{:.4}, {:.4}] arm_pulls={:?} last_updated_ms={}",
            place.as_str(),
            aud_label,
            ad.logit_offsets[0],
            ad.logit_offsets[1],
            ad.logit_offsets[2],
            ad.logit_offsets[3],
            ad.film_scale_deltas[0],
            ad.film_scale_deltas[1],
            ad.film_shift_deltas[0],
            ad.film_shift_deltas[1],
            ad.arm_pulls,
            ad.last_updated_ms.map_or_else(|| "none".to_owned(), |t| t.as_millis().to_string()),
        );
        adapt_count += 1;
    }
    if adapt_count == 0 {
        body.push_str("- none\n");
    }
    body.push('\n');

    // Section 3: Learned procedures.
    let mut proc_count = 0usize;
    body.push_str("## 3. Learned Procedures\n\n");
    for (pid, proc_rec) in &state.skill_book.procedures {
        if let Some(pf) = &place_filter
            && !proc_rec.places.is_empty()
            && !proc_rec.places.contains(pf)
        {
            continue;
        }
        if !args.include_private && is_private_audience(&proc_rec.audience, soul_id) {
            continue;
        }
        let caps: Vec<&str> = proc_rec
            .required_cap_ids
            .iter()
            .map(|c| c.as_str())
            .collect();
        let _ = writeln!(
            body,
            "- **procedure_id={}** name={} state={:?} reliability={:.4} trials={} successes={} cusum={:.4} retired_reason={} required_caps=[{}] steps=[{}] audience=[{}]",
            pid.as_u64(),
            proc_rec.name,
            proc_rec.state,
            proc_rec.posterior_mean(),
            proc_rec.trial_count,
            proc_rec.success_count,
            proc_rec.cusum_stat,
            proc_rec.retired_reason.as_deref().unwrap_or("none"),
            caps.join(","),
            proc_rec.steps.join(" -> "),
            format_audience_label(&proc_rec.audience),
        );
        proc_count += 1;
    }
    if proc_count == 0 {
        body.push_str("- none\n");
    }
    body.push('\n');

    // Section 4: Contrastive lessons.
    let mut lesson_count = 0usize;
    body.push_str("## 4. Contrastive Lessons\n\n");
    for (lid, lesson) in &state.skill_book.lessons {
        if !args.include_private && is_private_audience(&lesson.audience, soul_id) {
            continue;
        }
        if let Some(pf) = &place_filter {
            let matches_place = lesson
                .failure_ids
                .iter()
                .chain(lesson.contrastive_success_ids.iter())
                .filter_map(|id| state.memory_entries.get(id))
                .any(|m| &m.place == pf);
            if !matches_place {
                continue;
            }
        }
        let fids: Vec<String> = lesson
            .failure_ids
            .iter()
            .map(|id| id.as_u64().to_string())
            .collect();
        let cids: Vec<String> = lesson
            .contrastive_success_ids
            .iter()
            .map(|id| id.as_u64().to_string())
            .collect();
        let _ = writeln!(
            body,
            "- **lesson_id={}** pattern={} signature={} domain={} credence={:.4} active={} audience=[{}] failure_ids=[{}] contrastive_success_ids=[{}]",
            lid.as_u64(),
            lesson.pattern,
            lesson.failure_signature,
            lesson.domain.as_ref().map_or("none", |d| d.as_str()),
            lesson.credence.expected(),
            lesson.active,
            format_audience_label(&lesson.audience),
            fids.join(","),
            cids.join(","),
        );
        lesson_count += 1;
    }
    if lesson_count == 0 {
        body.push_str("- none\n");
    }
    body.push('\n');

    // Section 5: Beliefs & conventions.
    let mut belief_count = 0usize;
    body.push_str("## 5. Beliefs & Conventions\n\n");
    for (bid, belief) in &state.belief_book.beliefs {
        if belief.erased {
            continue;
        }
        if !args.include_private && is_private_audience(&belief.audience, soul_id) {
            continue;
        }
        if let Some(pf) = &place_filter {
            let matches_place = belief
                .source_event_ids
                .iter()
                .filter_map(|id| state.memory_entries.get(id))
                .any(|m| &m.place == pf);
            if !matches_place {
                continue;
            }
        }
        let cred = belief
            .credence
            .as_ref()
            .map_or_else(|| "none".to_owned(), |c| format!("{:.4}", c.expected()));
        let _ = writeln!(
            body,
            "- **belief_id={}** claim_key={} basis={:?} credence={} domain={} taint={:?} superseded_by={} audience=[{}]",
            bid.as_u64(),
            belief.claim_key,
            belief.basis,
            cred,
            belief.domain.as_str(),
            belief.taint,
            belief
                .replaced_by
                .map_or_else(|| "none".to_owned(), |id| id.as_u64().to_string()),
            format_audience_label(&belief.audience),
        );
        belief_count += 1;
    }
    for ((place, pattern_id), cred) in &state.belief_book.conventions {
        if let Some(pf) = &place_filter
            && place != pf
        {
            continue;
        }
        let participants = state.place_participants.get(place);
        if place_is_private(place, participants, soul_id) && !args.include_private {
            continue;
        }
        let _ = writeln!(
            body,
            "- **convention** place={} pattern_id={} credence={:.4}",
            place.as_str(),
            pattern_id,
            cred.expected(),
        );
        belief_count += 1;
    }
    if belief_count == 0 {
        body.push_str("- none\n");
    }
    body.push('\n');

    // Section 6: Consolidated episodes & gists.
    let mut episode_count = 0usize;
    body.push_str("## 6. Consolidated Episodes & Gists\n\n");
    for (ep_id, ep) in &state.episode_book.episodes {
        if let Some(pf) = &place_filter
            && &ep.place != pf
        {
            continue;
        }
        let gist_entry = ep.gist_id.and_then(|gid| state.memory_entries.get(&gid));
        if gist_entry.is_some_and(|m| m.erased) {
            continue;
        }
        let aud = gist_entry
            .map(|m| m.audience.clone())
            .unwrap_or_else(|| Audience::new(ep.participants.iter().cloned()));
        if !args.include_private && is_private_audience(&aud, soul_id) {
            continue;
        }
        let ev_ids: Vec<String> = ep.event_ids.iter().map(|id| id.as_u64().to_string()).collect();
        let _ = writeln!(
            body,
            "- **episode_id={}** place={} state={} events=[{}] gist_id={} taint={:?} audience=[{}]",
            ep_id.as_u64(),
            ep.place.as_str(),
            ep.state.as_str(),
            ev_ids.join(","),
            ep.gist_id
                .map_or_else(|| "none".to_owned(), |id| id.as_u64().to_string()),
            ep.taint,
            format_audience_label(&aud),
        );
        episode_count += 1;
    }
    for (mid, entry) in &state.memory_entries {
        if !entry.is_gist || entry.erased {
            continue;
        }
        if let Some(pf) = &place_filter
            && &entry.place != pf
        {
            continue;
        }
        if !args.include_private && is_private_audience(&entry.audience, soul_id) {
            continue;
        }
        let src_ids: Vec<String> = entry
            .source_ids
            .iter()
            .map(|id| id.as_u64().to_string())
            .collect();
        let _ = writeln!(
            body,
            "- **gist_id={}** place={} sources=[{}] concepts=[{}] taint={:?} audience=[{}]",
            mid.as_u64(),
            entry.place.as_str(),
            src_ids.join(","),
            entry.concepts.join(","),
            entry.taint,
            format_audience_label(&entry.audience),
        );
        episode_count += 1;
    }
    if episode_count == 0 {
        body.push_str("- none\n");
    }

    let export_blake3 = monoagens_core::blake3::to_hex(&monoagens_core::blake3::hash(body.as_bytes()));
    let mut doc = String::new();
    doc.push_str("# MonoAgens Learned State Export\n\n");
    doc.push_str("## 1. Audit & Reproducibility\n\n");
    let _ = writeln!(doc, "- `log_seq`: {}", state.last_seq.as_u64());
    let _ = writeln!(doc, "- `state_epoch`: {}", state.epoch);
    let _ = writeln!(doc, "- `model_blake3`: {model_blake3}");
    let _ = writeln!(
        doc,
        "- `online_adaptation`: {}",
        report.active_config.online_adaptation
    );
    let _ = writeln!(doc, "- `include_private`: {}", args.include_private);
    let _ = writeln!(
        doc,
        "- `place_filter`: {}",
        args.place.as_deref().unwrap_or("all")
    );
    let _ = writeln!(doc, "- `export_blake3`: {export_blake3}\n");
    doc.push_str(&body);

    if let Some(parent) = args.out.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).map_err(|e| format!("failed to create export dir: {e}"))?;
    }
    fs::write(&args.out, doc.as_bytes())
        .map_err(|e| format!("failed to write export markdown '{}': {e}", args.out.display()))?;

    if json_mode {
        writeln!(
            out,
            "{}",
            serde_json::to_string_pretty(&json!({
                "status": "ok",
                "out": args.out.display().to_string(),
                "export_blake3": export_blake3,
                "log_seq": state.last_seq.as_u64(),
                "state_epoch": state.epoch,
                "model_blake3": model_blake3,
                "online_adaptation": report.active_config.online_adaptation,
                "include_private": args.include_private,
                "counts": {
                    "place_adapt": adapt_count,
                    "procedures": proc_count,
                    "lessons": lesson_count,
                    "beliefs": belief_count,
                    "episodes_and_gists": episode_count,
                },
            }))
            .map_err(|e| e.to_string())?
        )
        .map_err(|e| e.to_string())?;
    } else {
        writeln!(
            out,
            "exported learned state to {} (blake3={} seq={} epoch={})",
            args.out.display(),
            export_blake3,
            state.last_seq.as_u64(),
            state.epoch
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(0)
}

#[cfg(test)]
mod repl_tests {
    use super::*;
    use monoagens_core::{ComputeTier, DecisionReason, GateClass, NeuralOutputs};
    use monoagens_protocol::Time;

    #[test]
    fn repl_commands_switch_group_speaker_dm_and_brain_toggle() {
        let mut st = ReplSessionState::new(
            ActorId::new("per_owner"),
            PlaceId::new("cli:terminal"),
            true,
        );
        assert!(!st.is_group);
        assert!(st.show_brain);

        let a1 = parse_repl_command("/group room:ta owner,budi,sari", &mut st);
        assert!(matches!(a1, ReplAction::Notice(ref s) if s.contains("room:ta")));
        assert!(st.is_group);
        assert_eq!(st.place.as_str(), "room:ta");
        assert!(st.audience.members.contains(&ActorId::new("budi")));
        assert!(st.audience.members.contains(&ActorId::new("soul")));

        let a2 = parse_repl_command("/as budi", &mut st);
        assert!(matches!(a2, ReplAction::Notice(ref s) if s.contains("budi")));
        assert_eq!(st.current_actor.as_str(), "budi");

        // Casual group chatter does NOT auto-address soul, while keeping deliverable channel place
        let p_casual = build_repl_percept(&st, "besok bimbingan jam berapa?", Time::from_millis(1000), 0)
            .expect("percept");
        assert_eq!(p_casual.place.as_str(), "cli:terminal");
        assert!(p_casual.addressees.is_empty());
        assert!(p_casual.named.is_empty());
        assert_eq!(p_casual.taint, Taint::Untrusted);

        // Mentioning @mono addresses and names soul
        let p_mention = build_repl_percept(&st, "eh @Mono tolong cek dong", Time::from_millis(2000), 1)
            .expect("percept");
        assert_eq!(p_mention.place.as_str(), "cli:terminal");
        assert_eq!(p_mention.addressees, vec![ActorId::new("soul")]);
        assert_eq!(p_mention.named, vec![ActorId::new("soul")]);

        // Back to DM restores trusted owner + auto-address
        let _ = parse_repl_command("/dm", &mut st);
        assert!(!st.is_group);
        assert_eq!(st.current_actor.as_str(), "per_owner");
        let p_dm = build_repl_percept(&st, "halo", Time::from_millis(3000), 2).expect("dm");
        assert_eq!(p_dm.addressees, vec![ActorId::new("soul")]);
        assert_eq!(p_dm.taint, Taint::Trusted);

        let _ = parse_repl_command("/brain", &mut st);
        assert!(!st.show_brain);
        assert_eq!(parse_repl_command("/quit", &mut st), ReplAction::Quit);
    }

    #[test]
    fn interactive_door_controller_admits_dynamic_repl_actors_without_startup_flag() {
        let owner = ActorId::new("per_owner");
        let mut snapshot = ValveSnapshot {
            owner_actor_id: Some(owner.clone()),
            t_now: Time::from_millis(1000),
            ..ValveSnapshot::default()
        };
        snapshot.known_actors.insert(owner.clone());
        let valve = Valve::with_default_policies(ValveConfig::default(), snapshot).expect("valve");
        let dynamic_actors = Arc::new(Mutex::new(BTreeSet::new()));
        let mut door = CliInteractiveDoorController {
            inner: ValveDoorController::new(valve),
            dynamic_actors: Arc::clone(&dynamic_actors),
        };

        let mut st = ReplSessionState::new(owner, PlaceId::new("cli:terminal"), true);
        let _ = parse_repl_command("/group tugas_pameran muqsith,obby,nejad", &mut st);
        let _ = parse_repl_command("/as muqsith", &mut st);

        // Before dynamic registration, unknown actor is blocked by strict Valve policy.
        let mut p_before =
            build_repl_percept(&st, "halo gess", Time::from_millis(1500), 0).expect("percept");
        assert!(matches!(door.inbound(&mut p_before), DoorDecision::Refuse { .. }));

        // After REPL actor sync (no --actors flag needed), inbound admits the simulated group turn.
        register_session_actors(&dynamic_actors, &st);
        let mut p_after =
            build_repl_percept(&st, "halo gess", Time::from_millis(2000), 1).expect("percept");
        assert!(matches!(door.inbound(&mut p_after), DoorDecision::Allow));
    }

    #[test]
    fn decision_trace_summary_renders_tier_probs_and_correction() {
        let trace = DecisionTrace {
            neural: NeuralOutputs {
                gate_logits: [0.0, 0.0, 0.1, 2.0],
                gate_probs: [0.05, 0.05, 0.10, 0.80],
                predicted_class: GateClass::Go,
                wait_delta_ms: 0.0,
                epistemic_uncertainty: 0.02,
                cic_abstain: false,
                h_next: Vec::new(),
                h_actor_next: Vec::new(),
                p_uncertainty: Some(0.1),
                p_verify: Some(0.2),
                p_encode_importance: None,
                p_memory_relevance: None,
                compute_tier_probs: Some([0.1, 0.2, 0.7]),
                compute_tier: Some(ComputeTier::Deep),
                p_user_correction: Some(0.87),
                world_pred: None,
                model_blake3: None,
                feature_schema_version: 2,
                is_neutral_fallback: false,
                online_adaptation_applied: true,
            },
            model_blake3: None,
            feature_schema_version: 2,
            online_adaptation_applied: true,
            commitment_bypass: false,
            self_caused: false,
            draw: 0.2,
            probability: 0.8,
            outcome: DecisionOutcome::Go,
            reason: DecisionReason::NeuralGo,
        };
        let line = format_decision_trace_summary(&trace);
        assert!(line.contains("Go"));
        assert!(line.contains("tier=Deep"));
        assert!(line.contains("p_go=0.80"));
        assert!(line.contains("p_corr=0.87"));
    }

    #[test]
    fn prequeue_service_enriches_repl_percept_embeddings_with_t_d_tag() {
        let embedder = Arc::new(monoagens_reason::HashEmbedder::with_model_ver("egemma2-256", 256));
        let clock = Arc::new(SystemClock::new());
        let diagnostics = DiagnosticSink::new(&monoagens_reason::ProviderSecrets::default());
        let pq = monoagens_gateway::PreQueueService::new(embedder, None, None, diagnostics, clock, 2000);
        let st = ReplSessionState::new(
            ActorId::new("per_owner"),
            PlaceId::new("cli:terminal"),
            true,
        );
        let mut percept =
            build_repl_percept(&st, "progres foto lantai 3 sudah 70%", Time::from_millis(1000), 0)
                .expect("percept");
        assert!(percept.embeddings.is_empty());
        let report = pq.enrich_percept_sync(&mut percept);
        assert!(report.embedded_tag.is_some());
        assert_eq!(percept.embeddings.len(), 1);
        assert_eq!(percept.embeddings[0].tag.embed_model_ver, "egemma2-256");
        assert_eq!(percept.embeddings[0].tag.prompt_regime_id, "t_d");
        assert_eq!(percept.embeddings[0].values.len(), 256);
    }
}
