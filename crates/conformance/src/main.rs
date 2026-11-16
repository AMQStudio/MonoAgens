//! `monoagens-conformance` CLI entrypoint.

#![forbid(unsafe_code)]

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use monoagens_conformance::{
    CompatibilityReport, ConformanceLevel, run_body_conformance, run_core_conformance,
};

fn main() -> ExitCode {
    match run_cli(env::args().skip(1).collect()) {
        Ok(code) => code,
        Err(msg) => {
            eprintln!("error: {msg}");
            ExitCode::from(2)
        }
    }
}

fn run_cli(args: Vec<String>) -> Result<ExitCode, String> {
    let Some(subcmd) = args.first().map(String::as_str) else {
        return Err(usage());
    };

    match subcmd {
        "-h" | "--help" => {
            println!("{}", usage());
            Ok(ExitCode::SUCCESS)
        }
        "body" => run_subcmd_target("body", &args[1..]),
        "core" => run_subcmd_target("core", &args[1..]),
        other => Err(format!("unknown subcommand '{other}'\n\n{}", usage())),
    }
}

fn run_subcmd_target(subcmd: &str, args: &[String]) -> Result<ExitCode, String> {
    let mut target: Option<String> = None;
    let mut target_args: Vec<String> = Vec::new();
    let mut level = ConformanceLevel::Must;
    let mut json_report: Option<PathBuf> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-h" | "--help" => {
                println!("{}", usage());
                return Ok(ExitCode::SUCCESS);
            }
            "--target" => {
                i += 1;
                let val = args.get(i).ok_or("--target requires a command/path")?;
                target = Some(val.clone());
            }
            "--target-arg" => {
                i += 1;
                let val = args.get(i).ok_or("--target-arg requires a value")?;
                target_args.push(val.clone());
            }
            "--level" => {
                i += 1;
                let val = args
                    .get(i)
                    .ok_or("--level requires one of: must, should, may")?;
                level = ConformanceLevel::parse(val)?;
            }
            "--json-report" => {
                i += 1;
                let val = args.get(i).ok_or("--json-report requires a file path")?;
                json_report = Some(PathBuf::from(val));
            }
            other => {
                return Err(format!("unknown argument '{other}'\n\n{}", usage()));
            }
        }
        i += 1;
    }

    let target_cmd =
        target.ok_or_else(|| format!("{subcmd} subcommand requires --target <cmd>"))?;
    let report: CompatibilityReport = match subcmd {
        "body" => run_body_conformance(&target_cmd, &target_args, level),
        "core" => run_core_conformance(&target_cmd, &target_args, level),
        _ => unreachable!(),
    }
    .map_err(|err| err.to_string())?;

    let rendered =
        serde_json::to_string_pretty(&report).map_err(|e| format!("encode report failed: {e}"))?;

    if let Some(path) = json_report {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent)
                .map_err(|e| format!("failed to create report dir '{}': {e}", parent.display()))?;
        }
        fs::write(&path, format!("{rendered}\n"))
            .map_err(|e| format!("failed to write json report '{}': {e}", path.display()))?;
    }

    println!("{rendered}");

    if report.passed() {
        Ok(ExitCode::SUCCESS)
    } else {
        Ok(ExitCode::from(1))
    }
}

fn usage() -> String {
    "Usage:\n  monoagens-conformance body --target <cmd> [--target-arg <arg>...] [--level must|should|may] [--json-report <path>]\n  monoagens-conformance core --target <cmd> [--target-arg <arg>...] [--level must|should|may] [--json-report <path>]".to_owned()
}
