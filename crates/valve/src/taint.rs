//! Three-door taint quarantine (`R-43`) sharing `monoagens_protocol::Taint` min-trust join.

use monoagens_core::ContextPlan;
use monoagens_protocol::{Action, Audience, Taint};

use crate::approvals::RiskTier;
use crate::snapshot::{MemoryWriteRequest, ValveSnapshot};

/// Shared compaction/write minimum-trust taint join delegating directly to
/// [`monoagens_protocol::Taint::join_min`].
#[must_use]
pub fn join_compaction_taint(items: impl IntoIterator<Item = Taint>) -> Taint {
    Taint::join_min(items)
}

/// Shared monotonic taint combination delegating directly to [`monoagens_protocol::Taint::combine`].
#[must_use]
pub fn combine_taint(a: Taint, b: Taint) -> Taint {
    a.combine(b)
}

/// Computes minimum-trust turn taint `tau_turn = min_{m in ContextPlan} iota(m)` across
/// `snapshot_taint` and all segments in [`ContextPlan`].
#[must_use]
pub fn evaluate_context_turn_taint(snapshot_taint: Taint, plan: &ContextPlan) -> Taint {
    let trigger = plan.sections.trigger.iter().map(|s| s.taint);
    let working = plan.sections.working_set.iter().map(|s| s.taint);
    let recalled = plan.sections.recalled.iter().map(|s| s.taint);
    let tail = plan.sections.place_tail.iter().map(|s| s.taint);

    Taint::join_min(
        core::iter::once(snapshot_taint)
            .chain(trigger)
            .chain(working)
            .chain(recalled)
            .chain(tail),
    )
}

/// Returns `true` when `action.audience` attempts to reach actors outside the active context audience.
#[must_use]
pub fn is_cross_audience_action(action: &Action, active_audience: Option<&Audience>) -> bool {
    let Some(scope_aud) = active_audience else {
        return false;
    };
    if scope_aud.is_empty() {
        return false;
    }
    !action.audience.is_subset_of(scope_aud)
}

/// Verdict of the 3-door taint quarantine check on an action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaintActionCheck {
    Allow,
    /// Action requires `ApprovalGrant(H_act)` (tainted `>= R_2`, `procedure`, or owner `identity_changing`).
    RequireApproval,
    /// Hard refusal (cross-audience without `DeclassToken` or non-owner `identity_changing`).
    RefuseCrossAudience,
}

/// Evaluates three-door taint quarantine for an outbound [`Action`] given effective `tau_turn`.
///
/// Under `tau_turn < Trusted`:
/// - `R_0` replies remain allowed;
/// - `>= R_2`, cross-audience, `procedure`-writing, or `identity_changing` acts are held or
///   refused unless an owner `ApprovalGrant(H_act)` or valid `DeclassToken` authorizes them.
#[must_use]
pub fn check_action_under_taint(
    effective_taint: Taint,
    tier: RiskTier,
    is_procedure_write: bool,
    identity_changing: bool,
    cross_audience: bool,
    has_valid_declass: bool,
    has_valid_approval_grant: bool,
) -> TaintActionCheck {
    if effective_taint >= Taint::Trusted {
        return if cross_audience && !has_valid_declass {
            TaintActionCheck::RefuseCrossAudience
        } else {
            TaintActionCheck::Allow
        };
    }

    if cross_audience && !has_valid_declass {
        return TaintActionCheck::RefuseCrossAudience;
    }

    let privileged = tier >= RiskTier::R2
        || is_procedure_write
        || identity_changing
        || (cross_audience && !has_valid_declass);
    if !privileged {
        return TaintActionCheck::Allow;
    }

    if has_valid_approval_grant {
        TaintActionCheck::Allow
    } else {
        TaintActionCheck::RequireApproval
    }
}

/// Checks whether a memory write is blocked under taint quarantine (`procedure` write or
/// cross-audience write under `tau_turn < Trusted` without owner approval/signature).
#[must_use]
pub fn is_memory_write_taint_blocked(req: &MemoryWriteRequest, snapshot: &ValveSnapshot) -> bool {
    let effective = combine_taint(req.taint, snapshot.turn_taint);
    if effective >= Taint::Trusted {
        return false;
    }
    if req.is_procedure {
        return true;
    }
    if let Some(ctx_aud) = &snapshot.active_context_audience
        && !ctx_aud.is_empty()
        && !req.audience.is_subset_of(ctx_aud)
    {
        return true;
    }
    false
}
