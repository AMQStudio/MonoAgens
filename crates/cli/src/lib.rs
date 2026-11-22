//! `monoagens` CLI library implementing `chat`, `serve`, `log`, `replay`, `approvals`,
//! `link`, `conformance`, `inspector`, and `export-learning` against `gateway`, `valve`,
//! `conformance`, and `body-cli`.

#![forbid(unsafe_code)]

pub mod commands;
mod common;

use std::io::Write;
use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};
use monoagens_conformance::ConformanceLevel;

pub use commands::approvals::canonical_h_act_details;
pub use commands::chat::{
    ReplAction, ReplSessionState, build_repl_percept, format_decision_trace_summary,
    parse_repl_command, run_chat_with_provider,
};
pub use commands::serve::{ServeBodyReport, ServeStartupReport, run_serve_with_provider};

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
    /// Run a long-lived Gateway daemon connected to configured `[[bodies]]` plugins.
    Serve(ServeArgs),
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

#[derive(Debug, Default, Args)]
pub struct ServeArgs {
    /// Optional data/store directory override for this daemon.
    #[arg(long)]
    pub data_dir: Option<PathBuf>,
    /// Optional neural controller `.safetensors` model override (`soul.model_path`).
    #[arg(long)]
    pub model: Option<PathBuf>,
    /// Connect only the specified `[[bodies]]` config id(s); repeatable.
    #[arg(long = "body")]
    pub bodies: Vec<String>,
    /// Allow starting the daemon with zero configured or selected `[[bodies]]`.
    #[arg(long)]
    pub allow_empty: bool,
    /// Optional bounded runtime duration in milliseconds before graceful shutdown.
    #[arg(long)]
    pub exit_after_ms: Option<u64>,
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
        Commands::Chat(args) => commands::chat::cmd_chat(config_opt, json_mode, args, None, out, err),
        Commands::Serve(args) => commands::serve::cmd_serve(config_opt, json_mode, args, out, err),
        Commands::Log(args) => commands::log::cmd_log(config_opt, json_mode, args, out),
        Commands::Replay(args) => commands::replay::cmd_replay(config_opt, json_mode, args, out, err),
        Commands::Approvals(args) => commands::approvals::cmd_approvals(config_opt, json_mode, args, out),
        Commands::Link(args) => commands::link::cmd_link(config_opt, json_mode, args, out),
        Commands::Conformance(args) => commands::conformance::cmd_conformance(args, out),
        Commands::Inspector(args) => commands::inspector::cmd_inspector(config_opt, json_mode, args, out),
        Commands::ExportLearning(args) => {
            commands::export_learning::cmd_export_learning(config_opt, json_mode, args, out)
        }
    };

    match res {
        Ok(code) => CliOutcome { exit_code: code },
        Err(msg) => {
            let _ = writeln!(err, "error: {msg}");
            CliOutcome { exit_code: 1 }
        }
    }
}
