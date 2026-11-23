//! Event-log views shared by `monoagens log` / `inspector` / `replay` and the TUI: filters,
//! decision explanation, causal traces and offline replay. Everything here is metadata only
//! (ids, kinds, places, actors, timestamps and decision numbers); payload bytes are never
//! returned, which is the same exposure `monoagens log` has.

use std::collections::{BTreeMap, BTreeSet};

use monoagens_core::{
    DecisionOutcome, DecisionTrace, EventDetail, EventKind, EventRecord, ReplayMode,
};
use monoagens_protocol::EventId;
use serde::Serialize;

use super::export::{JobCtl, build_mem_store};

/// `monoagens log` filters.
#[derive(Debug, Clone, Default)]
pub struct EventFilter {
    pub place: Option<String>,
    pub source: Option<String>,
    pub kind: Option<EventKind>,
    pub since_ms: Option<u64>,
    pub until_ms: Option<u64>,
}

/// Parses an event kind name (`speak`-style names are not kinds; see [`EventKind::ALL`]).
#[must_use]
pub fn parse_event_kind(raw: &str) -> Option<EventKind> {
    EventKind::ALL.into_iter().find(|k| k.as_str() == raw)
}

impl EventFilter {
    /// Whether `ev` passes every set filter.
    #[must_use]
    pub fn matches(&self, ev: &EventRecord) -> bool {
        self.place.as_deref().is_none_or(|p| ev.place.as_str() == p)
            && self.source.as_deref().is_none_or(|s| ev.actor.as_str() == s)
            && self.kind.is_none_or(|k| ev.kind == k)
            && self.since_ms.is_none_or(|m| ev.timestamp_ms.as_millis() >= m)
            && self.until_ms.is_none_or(|m| ev.timestamp_ms.as_millis() <= m)
    }
}

/// Events passing `filter`, in log order.
#[must_use]
pub fn filter_events<'a>(events: &'a [EventRecord], filter: &EventFilter) -> Vec<&'a EventRecord> {
    events.iter().filter(|ev| filter.matches(ev)).collect()
}

/// Compact row for list views (no payload).
#[derive(Debug, Clone, Serialize)]
pub struct EventRow {
    pub id: u64,
    pub seq: u64,
    pub kind: String,
    pub timestamp_ms: u64,
    pub place: String,
    pub actor: String,
    /// One-line description derived from the structured detail.
    pub summary: String,
}

/// One-line, payload-free description of an event.
#[must_use]
pub fn event_summary(ev: &EventRecord) -> String {
    match &ev.detail {
        Some(EventDetail::Decision { outcome, target_event_id, .. }) => {
            format!("decision {outcome:?} for #{}", target_event_id.as_u64())
        }
        Some(EventDetail::Action { capability_id, action_id, .. }) => {
            format!("action {} ({})", capability_id.as_str(), action_id.as_str())
        }
        Some(EventDetail::Receipt { capability_id, outcome, .. }) => {
            format!("receipt {} {outcome:?}", capability_id.as_str())
        }
        Some(EventDetail::DoorVerdict { door, rule, verdict, .. }) => {
            format!("door {door:?} {rule}: {verdict}")
        }
        Some(EventDetail::Failure { component, detail, .. }) => {
            format!("failure [{component}] {detail}")
        }
        Some(EventDetail::Withheld { reason, .. }) => format!("withheld: {reason}"),
        _ => ev.kind.as_str().to_owned(),
    }
}

impl EventRow {
    #[must_use]
    pub fn from_event(ev: &EventRecord) -> Self {
        Self {
            id: ev.id.as_u64(),
            seq: ev.seq.as_u64(),
            kind: ev.kind.as_str().to_owned(),
            timestamp_ms: ev.timestamp_ms.as_millis(),
            place: ev.place.as_str().to_owned(),
            actor: ev.actor.as_str().to_owned(),
            summary: event_summary(ev),
        }
    }
}

/// Decision explanation (`monoagens log --explain`).
#[derive(Debug, Clone, Serialize)]
pub struct ExplainDecision {
    pub event_id: u64,
    pub seq: u64,
    pub target_event_id: u64,
    pub outcome: DecisionOutcome,
    pub salience: f64,
    pub threshold: f64,
    pub probability: f64,
    pub draw: f64,
    pub reason: Option<monoagens_core::DecisionReason>,
    pub trace: Option<DecisionTrace>,
    pub door_verdicts: Vec<DoorVerdictSummary>,
}

/// One door verdict attached to a decision.
#[derive(Debug, Clone, Serialize)]
pub struct DoorVerdictSummary {
    pub event_id: u64,
    pub seq: u64,
    pub door: String,
    pub rule: String,
    pub reason_tag: String,
}

/// Finds the decision for `query_id` (the decision event itself, or the event it decided on).
///
/// # Errors
/// Returns the CLI's error text when no decision exists.
pub fn explain_decision(events: &[EventRecord], query_id: u64) -> Result<ExplainDecision, String> {
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

    Ok(ExplainDecision {
        event_id: decision_ev.id.as_u64(),
        seq: decision_ev.seq.as_u64(),
        target_event_id: target_event_id.as_u64(),
        outcome: *outcome,
        salience: *salience,
        threshold: *threshold,
        probability: *probability,
        draw: *draw,
        reason: *reason,
        trace: trace.as_deref().cloned(),
        door_verdicts,
    })
}

impl ExplainDecision {
    /// The exact lines `monoagens log --explain` prints without `--json`.
    #[must_use]
    pub fn text_lines(&self) -> Vec<String> {
        let mut lines = vec![format!(
            "decision event={} target={} outcome={:?} salience={:.6} threshold={:.6} probability={:.6} draw={:.6} reason={:?}",
            self.event_id,
            self.target_event_id,
            self.outcome,
            self.salience,
            self.threshold,
            self.probability,
            self.draw,
            self.reason
        )];
        if let Some(t) = &self.trace {
            lines.push(format!(
                "  schema_v={} online_adapt={} commitment_bypass={} cic_abstain={} wait_delta_ms={} gate_probs={:?} gate_logits={:?}",
                t.feature_schema_version,
                t.online_adaptation_applied,
                t.commitment_bypass,
                t.neural.cic_abstain,
                t.neural.wait_delta_ms,
                t.neural.gate_probs,
                t.neural.gate_logits,
            ));
        }
        for dv in &self.door_verdicts {
            lines.push(format!(
                "  door_verdict door={} rule={} reason_tag={}",
                dv.door, dv.rule, dv.reason_tag
            ));
        }
        lines
    }
}

/// `monoagens inspector --traceparent` semantics: relevant events whose serialised form
/// contains the text (all relevant events when `traceparent` is `None`).
#[must_use]
pub fn inspector_trace_events(events: &[EventRecord], traceparent: Option<&str>) -> Vec<EventRow> {
    events
        .iter()
        .filter(|ev| {
            matches!(
                ev.detail,
                Some(
                    EventDetail::DoorVerdict { .. }
                        | EventDetail::Action { .. }
                        | EventDetail::Receipt { .. }
                )
            )
        })
        .filter(|ev| {
            traceparent.is_none_or(|tp| serde_json::to_string(ev).is_ok_and(|s| s.contains(tp)))
        })
        .map(EventRow::from_event)
        .collect()
}

/// Index of the causal relations used to group events into one trace.
pub struct TraceIndex<'a> {
    events: &'a [EventRecord],
    by_id: BTreeMap<u64, usize>,
    /// cause id -> events that list it in `caused_by` or target it from their detail.
    children: BTreeMap<u64, Vec<usize>>,
    /// action id -> events carrying that action id (action, receipt, feedback, approval holds).
    by_action: BTreeMap<String, Vec<usize>>,
}

fn detail_target(d: &EventDetail) -> Option<u64> {
    match d {
        EventDetail::Decision { target_event_id, .. }
        | EventDetail::Withheld { target_event_id, .. }
        | EventDetail::Verdict { target_event_id, .. } => Some(target_event_id.as_u64()),
        EventDetail::DoorVerdict { target_event_id, .. }
        | EventDetail::Failure { target_event_id, .. } => target_event_id.map(EventId::as_u64),
        _ => None,
    }
}

/// Action ids an event refers to (including approval hold/resolve records in door verdicts).
fn event_action_ids(ev: &EventRecord) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(
        EventDetail::Action { action_id, .. }
        | EventDetail::ActionFeedback { action_id, .. }
        | EventDetail::Receipt { action_id, .. },
    ) = &ev.detail
    {
        out.push(action_id.as_str().to_owned());
    }
    if ev.kind == EventKind::DoorVerdict
        && let Ok(rec) = serde_json::from_slice::<monoagens_valve::ApprovalLogRecord>(ev.payload.as_bytes())
    {
        match rec {
            monoagens_valve::ApprovalLogRecord::Held { pending, .. } => {
                out.push(pending.action_id.as_str().to_owned());
            }
            monoagens_valve::ApprovalLogRecord::Resolved { action_id, .. } => {
                out.push(action_id.as_str().to_owned());
            }
            monoagens_valve::ApprovalLogRecord::GrantConsumed { .. } => {}
        }
    }
    out
}

impl<'a> TraceIndex<'a> {
    #[must_use]
    pub fn new(events: &'a [EventRecord]) -> Self {
        let mut by_id = BTreeMap::new();
        let mut children: BTreeMap<u64, Vec<usize>> = BTreeMap::new();
        let mut by_action: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        for (i, ev) in events.iter().enumerate() {
            by_id.insert(ev.id.as_u64(), i);
            for c in &ev.caused_by {
                children.entry(c.as_u64()).or_default().push(i);
            }
            if let Some(t) = ev.detail.as_ref().and_then(detail_target) {
                children.entry(t).or_default().push(i);
            }
            for a in event_action_ids(ev) {
                by_action.entry(a).or_default().push(i);
            }
        }
        Self { events, by_id, children, by_action }
    }

    /// Indices of every event in the trace rooted at `root_id`, in log order.
    #[must_use]
    pub fn trace_of(&self, root_id: u64) -> Vec<usize> {
        let Some(&root) = self.by_id.get(&root_id) else {
            return Vec::new();
        };
        let mut seen: BTreeSet<usize> = BTreeSet::new();
        let mut stack = vec![root];
        while let Some(i) = stack.pop() {
            if !seen.insert(i) {
                continue;
            }
            let ev = &self.events[i];
            if let Some(kids) = self.children.get(&ev.id.as_u64()) {
                stack.extend(kids.iter().copied());
            }
            for a in event_action_ids(ev) {
                if let Some(rel) = self.by_action.get(&a) {
                    stack.extend(rel.iter().copied());
                }
            }
        }
        seen.into_iter().collect()
    }

    /// The root of the trace `event_id` belongs to: the first ancestor without causes.
    #[must_use]
    pub fn root_of(&self, event_id: u64) -> u64 {
        let mut cur = event_id;
        for _ in 0..64 {
            let Some(&i) = self.by_id.get(&cur) else {
                return cur;
            };
            let ev = &self.events[i];
            let parent = ev
                .caused_by
                .last()
                .map(|c| c.as_u64())
                .or_else(|| ev.detail.as_ref().and_then(detail_target));
            match parent {
                Some(p) if p != cur => cur = p,
                _ => return cur,
            }
        }
        cur
    }
}

/// Events of the trace that `trace` names: a numeric event id (the trace containing it) or,
/// like `monoagens inspector --traceparent`, text found in the serialised events.
#[must_use]
pub fn trace_events(events: &[EventRecord], trace: &str) -> Vec<EventRow> {
    if let Ok(id) = trace.trim().parse::<u64>() {
        let idx = TraceIndex::new(events);
        let root = idx.root_of(id);
        return idx
            .trace_of(root)
            .into_iter()
            .map(|i| EventRow::from_event(&events[i]))
            .collect();
    }
    inspector_trace_events(events, Some(trace.trim()))
}

/// Result of an offline replay (`monoagens replay`).
#[derive(Debug, Clone, Serialize)]
pub struct ReplaySummary {
    /// `ok` when every decision reproduced, else `diverged`.
    pub status: String,
    pub replayed_events: u64,
    pub matched_decisions: u64,
    pub first_divergence: Option<ReplayDivergence>,
}

/// First mismatch of a diverged replay.
#[derive(Debug, Clone, Serialize)]
pub struct ReplayDivergence {
    pub seq: u64,
    pub decision_event_id: u64,
    pub target_event_id: u64,
    pub field: String,
    pub detail: String,
}

/// Replays `events` on an in-memory copy (the live store is never touched).
///
/// # Errors
/// Returns the replay error text, or `"cancelled"`.
pub fn replay_events(
    events: &[EventRecord],
    mode: ReplayMode,
    ctl: &JobCtl<'_>,
) -> Result<ReplaySummary, String> {
    let mem = build_mem_store(events, ctl)?;
    if ctl.cancelled() {
        return Err("cancelled".to_owned());
    }
    let report = monoagens_core::replay_log(&mem, mode, None)
        .map_err(|e| format!("offline replay failed: {e}"))?;
    let replayed_events = report.recovery.replayed_events as u64;
    let matched_decisions = report.verified_decisions as u64;
    if report.is_exact() {
        Ok(ReplaySummary {
            status: "ok".to_owned(),
            replayed_events,
            matched_decisions,
            first_divergence: None,
        })
    } else {
        let div = report.mismatches.first().expect("mismatch present");
        Ok(ReplaySummary {
            status: "diverged".to_owned(),
            replayed_events,
            matched_decisions,
            first_divergence: Some(ReplayDivergence {
                seq: div.seq.as_u64(),
                decision_event_id: div.event_id.as_u64(),
                target_event_id: div.target_event_id.as_u64(),
                field: div.field.to_string(),
                detail: div.detail.to_string(),
            }),
        })
    }
}
