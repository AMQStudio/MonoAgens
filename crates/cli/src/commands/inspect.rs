use std::fs;
use std::io::Write;
use std::path::Path;

use monoagens_conformance::{run_body_conformance, run_core_conformance};
use monoagens_core::{EventRecord, ReplayMode, Store};
use monoagens_gateway::governance::{
    EventFilter, JobCtl, explain_decision, filter_events, inspector_trace_events,
    parse_event_kind, policy_report, replay_events,
};
use monoagens_store::{SegmentedStore, StoreConfig};
use serde_json::json;

use crate::common::{load_paths, open_store_readonly, write_json_pretty};
use crate::{
    ConformanceArgs, ConformanceSubcommand, InspectorArgs, LogArgs, ReplayArgs, ReplayModeArg,
};

pub(crate) fn cmd_log(
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

    let filter = EventFilter {
        place: args.place.clone(),
        source: args.source.clone(),
        kind: kind_filter,
        since_ms: args.since,
        until_ms: args.until,
    };
    let filtered = filter_events(&events, &filter);

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
        write_json_pretty(out, &rows)?;
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

fn render_explain_event(
    query_id: u64,
    events: &[EventRecord],
    json_mode: bool,
    out: &mut dyn Write,
) -> Result<u8, String> {
    let explained = explain_decision(events, query_id)?;
    if json_mode {
        write_json_pretty(out, &explained)?;
    } else {
        for line in explained.text_lines() {
            writeln!(out, "{line}").map_err(|e| e.to_string())?;
        }
    }
    Ok(0)
}

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

    let summary = replay_events(&events, mode, &JobCtl::none())?;

    if let Some(div) = &summary.first_divergence {
        let div_msg = format!(
            "replay divergence at seq={} decision_event={} target_event={} field={}: {}",
            div.seq, div.decision_event_id, div.target_event_id, div.field, div.detail
        );
        if json_mode {
            let payload = json!({
                "status": "diverged",
                "replayed_events": summary.replayed_events,
                "matched_decisions": summary.matched_decisions,
                "first_divergence": {
                    "seq": div.seq,
                    "decision_event_id": div.decision_event_id,
                    "target_event_id": div.target_event_id,
                    "field": div.field,
                    "detail": div.detail,
                },
            });
            write_json_pretty(out, &payload)?;
        }
        let _ = writeln!(err, "error: {div_msg}");
        Ok(1)
    } else {
        if json_mode {
            let payload = json!({
                "status": "ok",
                "message": "all decisions reproduced",
                "replayed_events": summary.replayed_events,
                "matched_decisions": summary.matched_decisions,
                "first_divergence": serde_json::Value::Null,
            });
            write_json_pretty(out, &payload)?;
        } else {
            writeln!(
                out,
                "all decisions reproduced (matched={}, replayed={})",
                summary.matched_decisions, summary.replayed_events
            )
            .map_err(|e| e.to_string())?;
        }
        Ok(0)
    }
}

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

pub(crate) fn cmd_inspector(
    config_path: Option<&Path>,
    json_mode: bool,
    args: InspectorArgs,
    out: &mut dyn Write,
) -> Result<u8, String> {
    let (cfg_opt, paths) = load_paths(config_path)?;
    let cfg = cfg_opt.unwrap_or_default();
    let report = policy_report(&cfg)?;

    let mut trace_events = Vec::new();
    if paths.data_dir.exists()
        && let Ok(store) = SegmentedStore::open(StoreConfig::new(paths.data_dir.clone()))
        && let Ok(events) = store.read_in_order(None, usize::MAX)
    {
        for row in inspector_trace_events(&events, args.traceparent.as_deref()) {
            trace_events.push(json!({
                "id": row.id,
                "seq": row.seq,
                "kind": row.kind,
                "timestamp_ms": row.timestamp_ms,
            }));
        }
    }

    let proof = &report.symcc_proof;
    let rules = &report.active_rule_names;
    let payload = json!({
        "valve_mode": report.valve_mode,
        "symcc_proof": proof,
        "active_rule_names": rules,
        "canonical_reason_tags": report.canonical_reason_tags,
        "traceparent_filter": args.traceparent,
        "traces": trace_events,
    });

    if json_mode {
        write_json_pretty(out, &payload)?;
    } else {
        writeln!(
            out,
            "valve_mode={} symcc_status={:?} solver={} counterexamples={}",
            report.valve_mode,
            proof.status,
            proof.solver,
            proof.counterexamples
        )
        .map_err(|e| e.to_string())?;
        for r in rules {
            writeln!(out, "  rule: {r}").map_err(|e| e.to_string())?;
        }
        writeln!(out, "  traces: {}", trace_events.len()).map_err(|e| e.to_string())?;
    }
    Ok(0)
}
