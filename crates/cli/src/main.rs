//! `monoagens` CLI binary entrypoint.

#![forbid(unsafe_code)]

use std::io;
use std::process::ExitCode;

use clap::Parser;
use monoagens_cli::{Cli, execute_cli, is_tui_invocation};

fn main() -> ExitCode {
    let cli = Cli::parse();
    let outcome = if is_tui_invocation(&cli) {
        // `monoagens_tui::run_tui` writes directly to `io::stdout()` (raw mode + Ratatui);
        // holding `io::stdout().lock()` here would deadlock on the same thread.
        let mut stdout = io::stdout();
        let mut stderr = io::stderr();
        execute_cli(cli, &mut stdout, &mut stderr)
    } else {
        let mut stdout = io::stdout().lock();
        let mut stderr = io::stderr().lock();
        execute_cli(cli, &mut stdout, &mut stderr)
    };
    ExitCode::from(outcome.exit_code)
}
