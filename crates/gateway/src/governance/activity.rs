//! Adapter activity rows, derived on demand from recorded events (no in-process state: the feed
//! is rebuilt from the log after any restart).

use monoagens_core::{DecisionOutcome, EventDetail, EventKind, EventRecord};
use monoagens_protocol::ReceiptOutcome;
use serde::Serialize;

use super::approvals::plugin_for_place;
use super::events::TraceIndex;

/// What became of one adapter message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Replied,
    Held,
    Denied,
    Silent,
    Failed,
}

impl Outcome {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Replied => "replied",
            Self::Held => "held",
            Self::Denied => "denied",
            Self::Silent => "silent",
            Self::Failed => "failed",
        }
    }
}

/// One row of the feed: message in -> turn -> outcome, joined by causal trace.
#[derive(Debug, Clone, Serialize)]
pub struct ActivityRow {
    /// Id of the message-in event (the trace root); Enter explains this event.
    pub trace: u64,
    pub seq: u64,
    pub timestamp_ms: u64,
    pub plugin: String,
    pub place: String,
    pub actor: String,
    /// Payload-free one-line summary (message text is never shown here).
    pub summary: String,
    pub outcome: Outcome,
}

/// Whether a place belongs to an adapter (not the local terminal or Lab).
#[must_use]
pub fn is_adapter_place(place: &str) -> bool {
    match place.split_once(':') {
        Some((prefix, _)) => !matches!(prefix, "cli" | "tui" | "lab"),
        None => false,
    }
}

/// Derives the rows for every adapter message that has a decision, oldest first.
#[must_use]
pub fn activity_rows(events: &[EventRecord]) -> Vec<ActivityRow> {
    let idx = TraceIndex::new(events);
    let mut resolved_actions = std::collections::BTreeSet::new();
    for ev in events {
        if ev.kind == EventKind::DoorVerdict
            && let Ok(monoagens_valve::ApprovalLogRecord::Resolved { action_id, .. }) =
                serde_json::from_slice(ev.payload.as_bytes())
        {
            resolved_actions.insert(action_id.as_str().to_owned());
        }
    }
    let mut rows = Vec::new();
    for ev in events {
        if ev.kind != EventKind::Percept || !is_adapter_place(ev.place.as_str()) {
            continue;
        }
        let trace = idx.trace_of(ev.id.as_u64());
        let mut decided = false;
        let (mut failed, mut held, mut denied, mut replied) = (false, false, false, false);
        for &i in &trace {
            let t = &events[i];
            match &t.detail {
                Some(EventDetail::Decision { outcome, target_event_id, .. })
                    if *target_event_id == ev.id =>
                {
                    decided = true;
                    if *outcome == DecisionOutcome::DoorBlocked {
                        denied = true;
                    }
                }
                Some(EventDetail::Failure { .. }) => failed = true,
                Some(EventDetail::Receipt { outcome, action_id, .. }) => match outcome {
                    ReceiptOutcome::Held => {
                        if !resolved_actions.contains(action_id.as_str()) {
                            held = true;
                        }
                    }
                    ReceiptOutcome::Failed => failed = true,
                    ReceiptOutcome::Refused => denied = true,
                    ReceiptOutcome::Done => replied = true,
                    ReceiptOutcome::Unknown => {}
                },
                Some(EventDetail::Action { capability_id, .. })
                    if capability_id.as_str() != crate::waist2::CAPABILITY_APPROVAL_PROMPT =>
                {
                    replied = true;
                }
                _ => {}
            }
            if t.kind == EventKind::DoorVerdict
                && let Ok(monoagens_valve::ApprovalLogRecord::Held { pending, .. }) =
                    serde_json::from_slice(t.payload.as_bytes())
                && !resolved_actions.contains(pending.action_id.as_str())
            {
                held = true;
            }
        }
        if !decided && !held && !failed {
            continue; // the turn has not concluded yet
        }
        let outcome = if failed {
            Outcome::Failed
        } else if held {
            Outcome::Held
        } else if denied {
            Outcome::Denied
        } else if replied {
            Outcome::Replied
        } else {
            Outcome::Silent
        };
        rows.push(ActivityRow {
            trace: ev.id.as_u64(),
            seq: ev.seq.as_u64(),
            timestamp_ms: ev.timestamp_ms.as_millis(),
            plugin: plugin_for_place(ev.place.as_str()),
            place: ev.place.as_str().to_owned(),
            actor: ev.actor.as_str().to_owned(),
            summary: format!("message in, {} events in trace", trace.len()),
            outcome,
        });
    }
    rows
}
