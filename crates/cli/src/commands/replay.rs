use std::fs;
use std::io::Write;
use std::path::Path;

use monoagens_core::{EventRecord, MemStore, ReplayMode, Store};
use serde_json::json;

use crate::common::{load_paths, open_store_readonly, write_json_pretty};
use crate::{ReplayArgs, ReplayModeArg};

pub(crate) fn cmd_replay(
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
            write_json_pretty(out, &payload)?;
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
            write_json_pretty(out, &payload)?;
        }
        let _ = writeln!(err, "error: {div_msg}");
        Ok(1)
    }
}
