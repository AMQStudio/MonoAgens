//! Governance and observability backend shared by the CLI (`approvals`, `link`, `inspector`,
//! `log`, `replay`, `export-learning`) and the owner's admin methods behind the TUI panels
//! (`/approvals`, `/people`, `/policy`, `/activity`, `/events`, `/status`).
//!
//! Every function here is pure over event slices or the identity log; the admin handlers in
//! this module only read the event log through the owner task and call them. Nothing here
//! writes the live event log: approvals resolve through `approvals.resolve` (the valve path),
//! replay and export run on an in-memory copy.

pub mod activity;
pub mod approvals;
pub mod events;
pub mod export;
pub mod inspector;
pub mod jobs;
pub mod links;

use serde_json::{Value, json};

use crate::owner::{AdminError, ERR_METHOD_NOT_FOUND, GatewayOwner};

pub use activity::{ActivityRow, Outcome, activity_rows, is_adapter_place};
pub use approvals::{
    PendingApprovalEntry, build_valve_config, canonical_h_act_details, owner_actor_from_cfg,
    pending_approvals, plugin_for_place,
};
pub use events::{
    EventFilter, EventRow, ExplainDecision, ReplaySummary, TraceIndex, event_summary,
    explain_decision, filter_events, inspector_trace_events, parse_event_kind, replay_events,
    trace_events,
};
pub use export::{ExportLearning, JobCtl, build_mem_store, export_learning};
pub use inspector::{CedarRule, PolicyReport, extract_cedar_rule_summaries, policy_report};
pub use jobs::Jobs;

/// Methods served by [`governance_call`] (for docs and tests).
pub const GOVERNANCE_METHODS: &[&str] = &[
    "approvals.list",
    "people.list",
    "links.initiate",
    "links.verify",
    "links.unlink",
    "policy.get",
    "events.query",
    "events.explain",
    "events.trace",
    "activity.list",
    "lab.replay",
    "lab.export_learning",
    "lab.job",
    "lab.cancel",
];

fn p_str<'a>(params: &'a Value, key: &str) -> Option<&'a str> {
    params.get(key).and_then(Value::as_str)
}

fn p_u64(params: &Value, key: &str) -> Option<u64> {
    params.get(key).and_then(Value::as_u64)
}

fn req_str<'a>(params: &'a Value, key: &str) -> Result<&'a str, AdminError> {
    p_str(params, key)
        .ok_or_else(|| AdminError::invalid_params(format!("missing string param '{key}'")))
}

fn to_json<T: serde::Serialize>(v: &T) -> Result<Value, AdminError> {
    serde_json::to_value(v).map_err(|e| AdminError::server(format!("failed to encode result: {e}")))
}

/// Link changes are owner-only: a request naming an `actor` other than the owner is refused.
/// (The admin socket is owner-only by mode 0600 and `SO_PEERCRED`; an in-process client is the
/// owner. `actor` lets a caller that relays someone else's request be refused explicitly.)
fn require_owner(owner: &GatewayOwner, params: &Value) -> Result<(), AdminError> {
    match p_str(params, "actor") {
        Some(a) if a != owner.owner_actor().as_str() => Err(AdminError::server(format!(
            "only the owner can change links (caller '{a}' is not the owner)"
        ))),
        _ => Ok(()),
    }
}

/// Dispatches one governance method. Unknown names answer method-not-found.
pub async fn governance_call(
    owner: &GatewayOwner,
    method: &str,
    params: &Value,
) -> Result<Value, AdminError> {
    match method {
        "approvals.list" => {
            let entries = owner.approvals_pending().await?;
            Ok(json!({ "approvals": to_json(&entries)? }))
        }
        "people.list" => {
            let cfg = owner.config_snapshot();
            let resolver = links::open_resolver(&cfg, owner.data_dir()).map_err(AdminError::server)?;
            let mut list = links::link_list_json(&cfg, &resolver);
            let roles = |actor: &str| -> Vec<String> {
                cfg.rbac
                    .as_ref()
                    .and_then(|r| {
                        r.actor_roles
                            .get(&monoagens_protocol::ActorId::new(actor))
                            .cloned()
                    })
                    .unwrap_or_default()
            };
            if let Some(persons) = list["configured_persons"].as_array_mut() {
                for p in persons {
                    let r = roles(p["actor_id"].as_str().unwrap_or_default());
                    p["roles"] = json!(r);
                }
            }
            list["fallback_role"] = json!(cfg.rbac.as_ref().and_then(|r| r.fallback_role.clone()));
            Ok(list)
        }
        "links.initiate" => {
            require_owner(owner, params)?;
            let cfg = owner.config_snapshot();
            let resolver = links::open_resolver(&cfg, owner.data_dir()).map_err(AdminError::server)?;
            links::link_initiate_json(
                &resolver,
                (req_str(params, "primary_body")?, req_str(params, "primary_account")?),
                (req_str(params, "secondary_body")?, req_str(params, "secondary_account")?),
                params.get("emit_nonces").and_then(Value::as_bool).unwrap_or(true),
                owner.handle().clock().now(),
            )
            .map_err(AdminError::server)
        }
        "links.verify" => {
            require_owner(owner, params)?;
            let cfg = owner.config_snapshot();
            let resolver = links::open_resolver(&cfg, owner.data_dir()).map_err(AdminError::server)?;
            links::link_verify_json(
                &resolver,
                req_str(params, "challenge")?,
                req_str(params, "nonce1")?,
                req_str(params, "nonce2")?,
                params.get("owner_confirmed").and_then(Value::as_bool).unwrap_or(false),
                owner.handle().clock().now(),
            )
            .map_err(AdminError::server)
        }
        "links.unlink" => {
            require_owner(owner, params)?;
            let cfg = owner.config_snapshot();
            let resolver = links::open_resolver(&cfg, owner.data_dir()).map_err(AdminError::server)?;
            links::link_unlink_json(
                &resolver,
                req_str(params, "body")?,
                req_str(params, "account")?,
                p_str(params, "reason").unwrap_or("unlinked from the TUI"),
                owner.handle().clock().now(),
            )
            .map_err(AdminError::server)
        }
        "policy.get" => {
            let cfg = owner.config_snapshot();
            let report = tokio::task::spawn_blocking(move || policy_report(&cfg))
                .await
                .map_err(|e| AdminError::server(format!("policy task failed: {e}")))?
                .map_err(AdminError::server)?;
            to_json(&report)
        }
        "events.query" => {
            let events = read_events(owner).await?;
            events_query(&events, params)
        }
        "events.explain" => {
            let events = read_events(owner).await?;
            let id = p_u64(params, "id")
                .ok_or_else(|| AdminError::invalid_params("missing integer param 'id'"))?;
            let e = explain_decision(&events, id).map_err(AdminError::server)?;
            let mut v = to_json(&e)?;
            v["lines"] = json!(e.text_lines());
            Ok(v)
        }
        "events.trace" => {
            let events = read_events(owner).await?;
            let rows = trace_events(&events, req_str(params, "trace")?);
            Ok(json!({ "events": to_json(&rows)? }))
        }
        "activity.list" => {
            let events = read_events(owner).await?;
            Ok(activity_list(&events, params))
        }
        "lab.replay" | "lab.export_learning" | "lab.job" | "lab.cancel" => {
            jobs::lab_call(owner, method, params).await
        }
        other => Err(AdminError::new(
            ERR_METHOD_NOT_FOUND,
            format!("method not found: {other}"),
        )),
    }
}

pub(crate) async fn read_events(
    owner: &GatewayOwner,
) -> Result<Vec<monoagens_core::EventRecord>, AdminError> {
    owner
        .handle()
        .read_events()
        .await
        .map_err(|e| AdminError::server(e.to_string()))
}

/// `events.query`: `{events:[EventRow], next_cursor, matching}` newest first. Params: `place`,
/// `source`, `kind`, `since`, `until` (epoch ms), `trace`, `cursor` (return events with a
/// smaller `seq`), `limit` (default 200, max 1000).
pub fn events_query(
    events: &[monoagens_core::EventRecord],
    params: &Value,
) -> Result<Value, AdminError> {
    let kind = match p_str(params, "kind") {
        Some(k) => Some(
            parse_event_kind(k)
                .ok_or_else(|| AdminError::invalid_params(format!("unknown event kind '{k}'")))?,
        ),
        None => None,
    };
    let filter = EventFilter {
        place: p_str(params, "place").map(str::to_owned),
        source: p_str(params, "source").map(str::to_owned),
        kind,
        since_ms: p_u64(params, "since"),
        until_ms: p_u64(params, "until"),
    };
    let trace_ids: Option<std::collections::BTreeSet<u64>> = p_str(params, "trace")
        .filter(|t| !t.trim().is_empty())
        .map(|t| trace_events(events, t).into_iter().map(|r| r.id).collect());
    let limit = usize::try_from(p_u64(params, "limit").unwrap_or(200).clamp(1, 1000)).unwrap_or(200);
    let cursor = p_u64(params, "cursor");
    let matching: Vec<&monoagens_core::EventRecord> = events
        .iter()
        .rev()
        .filter(|ev| filter.matches(ev))
        .filter(|ev| trace_ids.as_ref().is_none_or(|ids| ids.contains(&ev.id.as_u64())))
        .collect();
    let total = matching.len();
    let page: Vec<&monoagens_core::EventRecord> = matching
        .into_iter()
        .filter(|ev| cursor.is_none_or(|c| ev.seq.as_u64() < c))
        .take(limit + 1)
        .collect();
    let has_more = page.len() > limit;
    let page: Vec<EventRow> = page.into_iter().take(limit).map(EventRow::from_event).collect();
    let next_cursor = if has_more { page.last().map(|r| r.seq) } else { None };
    Ok(json!({ "events": page, "next_cursor": next_cursor, "matching": total }))
}

/// `activity.list`: `{rows:[ActivityRow], total}` oldest first (the newest `limit`), filtered by
/// `plugin`, `place` and `outcome`.
#[must_use]
pub fn activity_list(events: &[monoagens_core::EventRecord], params: &Value) -> Value {
    let plugin = p_str(params, "plugin");
    let place = p_str(params, "place");
    let outcome = p_str(params, "outcome");
    let limit = usize::try_from(p_u64(params, "limit").unwrap_or(500).clamp(1, 5000)).unwrap_or(500);
    let rows: Vec<ActivityRow> = activity_rows(events)
        .into_iter()
        .filter(|r| plugin.is_none_or(|p| r.plugin == p))
        .filter(|r| place.is_none_or(|p| r.place == p))
        .filter(|r| outcome.is_none_or(|o| r.outcome.as_str() == o))
        .collect();
    let total = rows.len();
    let skip = total.saturating_sub(limit);
    let rows: Vec<ActivityRow> = rows.into_iter().skip(skip).collect();
    json!({ "rows": rows, "total": total })
}
