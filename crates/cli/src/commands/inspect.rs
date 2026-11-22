use std::fs;
use std::io::Write;
use std::path::Path;

use monoagens_conformance::{run_body_conformance, run_core_conformance};
use monoagens_core::{
    DecisionOutcome, DecisionTrace, EventDetail, EventKind, EventRecord, MemStore, ReplayMode,
    Store,
};
use monoagens_protocol::EventId;
use monoagens_store::{SegmentedStore, StoreConfig};
use monoagens_valve::{DEFAULT_CEDAR_POLICIES, PolicyBundle, ValveReasonTag};
use serde::Serialize;
use serde_json::json;

use crate::common::{build_valve_config, load_paths, open_store_readonly, write_json_pretty};
use crate::{
    ConformanceArgs, ConformanceSubcommand, InspectorArgs, LogArgs, ReplayArgs, ReplayModeArg,
};

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

fn parse_event_kind(raw: &str) -> Option<EventKind> {
    EventKind::ALL.into_iter().find(|k| k.as_str() == raw)
}

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
        write_json_pretty(out, &payload)?;
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

pub(crate) fn cmd_inspector(
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
        write_json_pretty(out, &payload)?;
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
