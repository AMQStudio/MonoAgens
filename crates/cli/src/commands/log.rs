use std::io::Write;
use std::path::Path;

use monoagens_core::{DecisionOutcome, DecisionTrace, EventDetail, EventKind, EventRecord, Store};
use monoagens_protocol::EventId;
use serde::Serialize;
use serde_json::json;

use crate::LogArgs;
use crate::common::{load_paths, open_store_readonly, write_json_pretty};

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
