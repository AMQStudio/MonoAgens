//! Secondary door hooks and first-and-last narrowing-only reconciliation.
//!
//! Secondary hooks run between Valve's opening and closing passes and may ONLY narrow
//! what Valve allows. Any widening attempt is overridden and logged.

use std::collections::BTreeSet;

use monoagens_core::{ContextPlan, DoorDecision};
use monoagens_protocol::{Action, EventId, Percept};

use crate::config::ValveConfig;
use crate::reason_tags::ValveReasonTag;
use crate::snapshot::{MemoryWriteRequest, ValveSnapshot};
use crate::verdict::ValveDoorKind;

/// Secondary hook invoked between Valve's opening and closing passes.
pub trait SecondaryDoorHook: Send + Sync {
    /// Stable rule name for audit logging.
    fn name(&self) -> &str;

    fn on_inbound(&mut self, _percept: &Percept) -> DoorDecision {
        DoorDecision::Allow
    }

    fn on_context(&mut self, _plan: &ContextPlan) -> DoorDecision {
        DoorDecision::Allow
    }

    fn on_action(&mut self, _action: &Action) -> DoorDecision {
        DoorDecision::Allow
    }

    fn on_memory_write(&mut self, _req: &MemoryWriteRequest) -> DoorDecision {
        DoorDecision::Allow
    }
}

/// Pluggable action-door policy extension point (used by V2 risk tiers, `PendingApproval`,
/// taint quarantine, and quiet-hours digests without reshaping `Valve` doors).
pub trait ActionDoorPolicyPlugin: Send + Sync {
    /// Evaluates an action within the current `(log_seq, t_now)` snapshot and config.
    ///
    /// Returning `Some((decision, reason_tag))` narrows an otherwise-allowed action.
    fn evaluate_action(
        &mut self,
        _action: &Action,
        _snapshot: &ValveSnapshot,
        _config: &ValveConfig,
    ) -> Option<(DoorDecision, ValveReasonTag)> {
        None
    }

    /// Evaluates a memory-write request within the current `(log_seq, t_now)` snapshot and config.
    fn evaluate_memory_write(
        &mut self,
        _req: &MemoryWriteRequest,
        _snapshot: &ValveSnapshot,
        _config: &ValveConfig,
    ) -> Option<(DoorDecision, ValveReasonTag)> {
        None
    }
}

/// Metadata recorded when Valve's closing pass overrides a secondary hook's widening attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WideningOverride {
    pub door: ValveDoorKind,
    pub hook_name: String,
    pub attempted: String,
    pub enforced: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum DecisionStrictness {
    Allow = 0,
    Strip = 1,
    Hold = 2,
    Refuse = 3,
}

const fn strictness_of(d: &DoorDecision) -> DecisionStrictness {
    match d {
        DoorDecision::Allow => DecisionStrictness::Allow,
        DoorDecision::Strip { .. } => DecisionStrictness::Strip,
        DoorDecision::Hold { .. } => DecisionStrictness::Hold,
        DoorDecision::Refuse { .. } => DecisionStrictness::Refuse,
    }
}

const fn decision_label(d: &DoorDecision) -> &'static str {
    match d {
        DoorDecision::Allow => "allow",
        DoorDecision::Strip { .. } => "strip",
        DoorDecision::Hold { .. } => "hold",
        DoorDecision::Refuse { .. } => "refuse",
    }
}

/// Reconciles Valve's opening pass decision with a secondary hook decision so that
/// hooks can only narrow (never widen) access.
#[must_use]
pub fn reconcile_first_and_last(
    door: ValveDoorKind,
    pass1: DoorDecision,
    pass1_tag: Option<ValveReasonTag>,
    hook_name: &str,
    hook_decision: DoorDecision,
) -> (
    DoorDecision,
    Option<ValveReasonTag>,
    Option<WideningOverride>,
) {
    let p1_rank = strictness_of(&pass1);
    let hook_rank = strictness_of(&hook_decision);

    if hook_rank < p1_rank {
        let ov = WideningOverride {
            door,
            hook_name: hook_name.to_owned(),
            attempted: decision_label(&hook_decision).to_owned(),
            enforced: decision_label(&pass1).to_owned(),
        };
        return (pass1, pass1_tag, Some(ov));
    }

    if hook_rank > p1_rank {
        return (hook_decision, pass1_tag, None);
    }

    // Equal strictness tier: check Strip subset invariant or keep pass1 when both Hold/Refuse.
    match (pass1, hook_decision) {
        (DoorDecision::Allow, DoorDecision::Allow) => (DoorDecision::Allow, pass1_tag, None),
        (
            DoorDecision::Strip {
                removed_ids: p1_ids,
                rule: p1_rule,
            },
            DoorDecision::Strip {
                removed_ids: h_ids,
                rule: h_rule,
            },
        ) => {
            let p1_set: BTreeSet<EventId> = p1_ids.iter().copied().collect();
            let h_set: BTreeSet<EventId> = h_ids.iter().copied().collect();
            let widened = !p1_set.is_subset(&h_set);
            let mut merged: BTreeSet<EventId> = p1_set;
            merged.extend(h_set);
            let merged_vec: Vec<EventId> = merged.into_iter().collect();
            let merged_rule = if h_rule.is_empty() || widened || p1_rule == h_rule {
                p1_rule
            } else {
                format!("{p1_rule}+{h_rule}")
            };
            let ov = if widened {
                Some(WideningOverride {
                    door,
                    hook_name: hook_name.to_owned(),
                    attempted: "strip_omitted_valve_items".to_owned(),
                    enforced: "strip".to_owned(),
                })
            } else {
                None
            };
            (
                DoorDecision::Strip {
                    removed_ids: merged_vec,
                    rule: merged_rule,
                },
                pass1_tag,
                ov,
            )
        }
        (p1, _) => (p1, pass1_tag, None),
    }
}
