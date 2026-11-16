//! Expectation check declarations and deterministic evaluators.

use monoagens_core::{
    ContextPlan, DecisionOutcome, DecisionReason, EventDetail, EventKind, EventRecord,
};
use monoagens_protocol::{ActionId, CapabilityId, EventId, PlaceId, ReceiptOutcome};
use serde::{Deserialize, Serialize};

/// Tag separating structural checks from model wording checks (`specs/lab/spec.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckKind {
    Structural,
    Wording,
}

/// Declarative expectation check evaluated against a completed simulation log and captured contexts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "check", rename_all = "snake_case")]
pub enum ExpectationCheck {
    /// Verifies that an action did or did not happen in `place` within `[after_ms, before_ms]`.
    ActionInPlace {
        name: String,
        kind: CheckKind,
        place: PlaceId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        capability_id: Option<CapabilityId>,
        after_ms: u64,
        before_ms: u64,
        expected_present: bool,
    },
    /// Verifies that a captured `ContextPlan` did or did not contain a given event id or label.
    ContextContains {
        name: String,
        kind: CheckKind,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        item_event_id: Option<EventId>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
        expected_present: bool,
    },
    /// Verifies that a recorded receipt had the expected outcome.
    ReceiptOutcomeIs {
        name: String,
        kind: CheckKind,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        action_id: Option<ActionId>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        capability_id: Option<CapabilityId>,
        expected: ReceiptOutcome,
    },
    /// Verifies that a recorded gate decision had the expected outcome and optional reason.
    DecisionIs {
        name: String,
        kind: CheckKind,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target_event_id: Option<EventId>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        decision_index: Option<usize>,
        expected_outcome: DecisionOutcome,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expected_reason: Option<DecisionReason>,
    },
    /// Verifies that at least one matching event's payload contains (or never contains) `text`.
    PayloadContains {
        name: String,
        kind: CheckKind,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        place: Option<PlaceId>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        event_kind: Option<EventKind>,
        text: String,
        expected_present: bool,
    },
}

impl ExpectationCheck {
    /// Returns the human-readable check name.
    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            Self::ActionInPlace { name, .. }
            | Self::ContextContains { name, .. }
            | Self::ReceiptOutcomeIs { name, .. }
            | Self::DecisionIs { name, .. }
            | Self::PayloadContains { name, .. } => name,
        }
    }

    /// Returns whether this check is structural or wording.
    #[must_use]
    pub const fn kind(&self) -> CheckKind {
        match self {
            Self::ActionInPlace { kind, .. }
            | Self::ContextContains { kind, .. }
            | Self::ReceiptOutcomeIs { kind, .. }
            | Self::DecisionIs { kind, .. }
            | Self::PayloadContains { kind, .. } => *kind,
        }
    }
}

/// Outcome of evaluating a single [`ExpectationCheck`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckResult {
    pub name: String,
    pub kind: CheckKind,
    pub passed: bool,
    pub detail: String,
    pub violating_event_id: Option<EventId>,
}

/// Evaluates all `checks` deterministically against recorded `events` and `context_plans`.
#[must_use]
pub fn evaluate_checks(
    checks: &[ExpectationCheck],
    events: &[EventRecord],
    context_plans: &[ContextPlan],
) -> Vec<CheckResult> {
    checks
        .iter()
        .map(|check| evaluate_one(check, events, context_plans))
        .collect()
}

fn evaluate_one(
    check: &ExpectationCheck,
    events: &[EventRecord],
    context_plans: &[ContextPlan],
) -> CheckResult {
    match check {
        ExpectationCheck::ActionInPlace {
            name,
            kind,
            place,
            capability_id,
            after_ms,
            before_ms,
            expected_present,
        } => {
            let matching: Vec<&EventRecord> = events
                .iter()
                .filter(|rec| rec.kind == EventKind::Action)
                .filter(|rec| &rec.place == place)
                .filter(|rec| {
                    let ts = rec.timestamp_ms.as_millis();
                    ts >= *after_ms && ts <= *before_ms
                })
                .filter(|rec| match capability_id {
                    Some(cap) => matches!(
                        &rec.detail,
                        Some(EventDetail::Action { capability_id: c, .. }) if c == cap
                    ),
                    None => true,
                })
                .collect();

            if *expected_present {
                if let Some(first) = matching.first() {
                    CheckResult {
                        name: name.clone(),
                        kind: *kind,
                        passed: true,
                        detail: format!(
                            "found {} matching action(s) in '{place}' (first event {})",
                            matching.len(),
                            first.id
                        ),
                        violating_event_id: None,
                    }
                } else {
                    CheckResult {
                        name: name.clone(),
                        kind: *kind,
                        passed: false,
                        detail: format!(
                            "expected action in '{place}' within [{after_ms}, {before_ms}]ms, found none"
                        ),
                        violating_event_id: None,
                    }
                }
            } else if let Some(violation) = matching.first() {
                CheckResult {
                    name: name.clone(),
                    kind: *kind,
                    passed: false,
                    detail: format!(
                        "expected silence in '{place}' within [{after_ms}, {before_ms}]ms, violated by event {} at {}",
                        violation.id, violation.timestamp_ms
                    ),
                    violating_event_id: Some(violation.id),
                }
            } else {
                CheckResult {
                    name: name.clone(),
                    kind: *kind,
                    passed: true,
                    detail: format!(
                        "verified absence of actions in '{place}' within [{after_ms}, {before_ms}]ms"
                    ),
                    violating_event_id: None,
                }
            }
        }

        ExpectationCheck::ContextContains {
            name,
            kind,
            item_event_id,
            label,
            expected_present,
        } => {
            let matched = context_plans.iter().any(|plan| {
                let id_match = item_event_id
                    .is_none_or(|id| plan.items.contains(&id) || plan.trigger_event_id == Some(id));
                let label_match = label.as_ref().is_none_or(|lbl| {
                    plan.place.as_ref().is_some_and(|p| p.as_str() == lbl)
                        || plan
                            .visible_scope
                            .as_ref()
                            .is_some_and(|vs| vs.audience.members.iter().any(|a| a.as_str() == lbl))
                });
                id_match && label_match
            });

            if *expected_present {
                CheckResult {
                    name: name.clone(),
                    kind: *kind,
                    passed: matched,
                    detail: if matched {
                        format!(
                            "captured {} context plan(s) matching criteria",
                            context_plans.len()
                        )
                    } else {
                        format!(
                            "no captured context plan matched criteria across {} plan(s)",
                            context_plans.len()
                        )
                    },
                    violating_event_id: None,
                }
            } else {
                CheckResult {
                    name: name.clone(),
                    kind: *kind,
                    passed: !matched,
                    detail: if !matched {
                        "verified context plans excluded forbidden item/label".to_owned()
                    } else {
                        "context plan unexpectedly contained forbidden item/label".to_owned()
                    },
                    violating_event_id: None,
                }
            }
        }

        ExpectationCheck::ReceiptOutcomeIs {
            name,
            kind,
            action_id,
            capability_id,
            expected,
        } => {
            let found = events.iter().find_map(|rec| {
                if let Some(EventDetail::Receipt {
                    action_id: rec_aid,
                    capability_id: rec_cap,
                    outcome,
                    ..
                }) = &rec.detail
                {
                    let aid_ok = action_id.as_ref().is_none_or(|a| a == rec_aid);
                    let cap_ok = capability_id.as_ref().is_none_or(|c| c == rec_cap);
                    if aid_ok && cap_ok {
                        return Some((rec.id, *outcome));
                    }
                }
                None
            });

            match found {
                Some((ev_id, actual)) if actual == *expected => CheckResult {
                    name: name.clone(),
                    kind: *kind,
                    passed: true,
                    detail: format!("receipt event {ev_id} matched expected outcome {expected:?}"),
                    violating_event_id: None,
                },
                Some((ev_id, actual)) => CheckResult {
                    name: name.clone(),
                    kind: *kind,
                    passed: false,
                    detail: format!(
                        "receipt event {ev_id} had outcome {actual:?}, expected {expected:?}"
                    ),
                    violating_event_id: Some(ev_id),
                },
                None => CheckResult {
                    name: name.clone(),
                    kind: *kind,
                    passed: false,
                    detail: format!("no matching receipt recorded (expected {expected:?})"),
                    violating_event_id: None,
                },
            }
        }

        ExpectationCheck::DecisionIs {
            name,
            kind,
            target_event_id,
            decision_index,
            expected_outcome,
            expected_reason,
        } => {
            let decisions: Vec<&EventRecord> = events
                .iter()
                .filter(|rec| rec.kind == EventKind::Decision)
                .collect();

            let matched_rec = if let Some(target) = target_event_id {
                decisions.into_iter().find(|rec| {
                    matches!(
                        &rec.detail,
                        Some(EventDetail::Decision { target_event_id: tid, .. }) if tid == target
                    )
                })
            } else if let Some(idx) = decision_index {
                decisions.get(*idx).copied()
            } else {
                decisions.first().copied()
            };

            match matched_rec {
                Some(rec) => {
                    if let Some(EventDetail::Decision {
                        outcome, reason, ..
                    }) = &rec.detail
                    {
                        let outcome_ok = outcome == expected_outcome;
                        let reason_ok = expected_reason.is_none_or(|r| *reason == Some(r));
                        let passed = outcome_ok && reason_ok;
                        CheckResult {
                            name: name.clone(),
                            kind: *kind,
                            passed,
                            detail: format!(
                                "decision event {}: outcome={outcome:?}, reason={reason:?}",
                                rec.id
                            ),
                            violating_event_id: if passed { None } else { Some(rec.id) },
                        }
                    } else {
                        CheckResult {
                            name: name.clone(),
                            kind: *kind,
                            passed: false,
                            detail: format!("decision event {} missing detail", rec.id),
                            violating_event_id: Some(rec.id),
                        }
                    }
                }
                None => CheckResult {
                    name: name.clone(),
                    kind: *kind,
                    passed: false,
                    detail: "no matching decision event recorded".to_owned(),
                    violating_event_id: None,
                },
            }
        }

        ExpectationCheck::PayloadContains {
            name,
            kind,
            place,
            event_kind,
            text,
            expected_present,
        } => {
            let matches: Vec<&EventRecord> = events
                .iter()
                .filter(|rec| place.as_ref().is_none_or(|p| &rec.place == p))
                .filter(|rec| event_kind.is_none_or(|k| rec.kind == k))
                .filter(|rec| {
                    String::from_utf8_lossy(rec.payload.as_bytes()).contains(text.as_str())
                })
                .collect();

            if *expected_present {
                if let Some(first) = matches.first() {
                    CheckResult {
                        name: name.clone(),
                        kind: *kind,
                        passed: true,
                        detail: format!("found payload substring '{text}' in event {}", first.id),
                        violating_event_id: None,
                    }
                } else {
                    CheckResult {
                        name: name.clone(),
                        kind: *kind,
                        passed: false,
                        detail: format!("payload substring '{text}' not found in matching events"),
                        violating_event_id: None,
                    }
                }
            } else if let Some(violation) = matches.first() {
                CheckResult {
                    name: name.clone(),
                    kind: *kind,
                    passed: false,
                    detail: format!(
                        "forbidden payload substring '{text}' found in event {}",
                        violation.id
                    ),
                    violating_event_id: Some(violation.id),
                }
            } else {
                CheckResult {
                    name: name.clone(),
                    kind: *kind,
                    passed: true,
                    detail: format!("verified absence of payload substring '{text}'"),
                    violating_event_id: None,
                }
            }
        }
    }
}
