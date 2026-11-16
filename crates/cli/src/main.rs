//! `monoagens` CLI binary entrypoint.

#![forbid(unsafe_code)]

use std::io;
use std::process::ExitCode;

use clap::Parser;
use monoagens_cli::{Cli, execute_cli};

fn main() -> ExitCode {
    let cli = Cli::parse();
    let mut stdout = io::stdout().lock();
    let mut stderr = io::stderr().lock();
    let outcome = execute_cli(cli, &mut stdout, &mut stderr);
    ExitCode::from(outcome.exit_code)
}
