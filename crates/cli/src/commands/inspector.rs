use std::io::Write;
use std::path::Path;

use monoagens_core::{EventDetail, Store};
use monoagens_store::{SegmentedStore, StoreConfig};
use monoagens_valve::{DEFAULT_CEDAR_POLICIES, PolicyBundle, ValveReasonTag};
use serde_json::json;

use crate::InspectorArgs;
use crate::common::{build_valve_config, load_paths, write_json_pretty};

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
