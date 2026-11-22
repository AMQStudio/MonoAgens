use std::fs;
use std::io::Write;

use monoagens_conformance::{run_body_conformance, run_core_conformance};

use crate::{ConformanceArgs, ConformanceSubcommand};

pub(crate) fn cmd_conformance(args: ConformanceArgs, out: &mut dyn Write) -> Result<u8, String> {
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
