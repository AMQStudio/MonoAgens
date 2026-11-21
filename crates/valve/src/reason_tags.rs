//! Canonical `door_verdict` / `Receipt` reason tags and exhaustive `[SELF_GUIDE]` entries.
//!
//! Every [`ValveReasonTag`] has a compile-time exhaustive guide mapping with no Cedar
//! internals or tunable formula constants exposed.

use core::fmt;
use monoagens_core::ValveSelfSnapshot;
use serde::{Deserialize, Serialize};

use crate::approvals::ApprovalManager;
use crate::config::{ValveConfig, ValveMode};
use crate::quiet_hours::{ProactiveCapTracker, evaluate_recipient_local_time};
use crate::snapshot::ValveSnapshot;

/// Canonical hold, strip, and refusal reason tags emitted by `Valve` in `door_verdict`
/// and `Receipt.reason_code`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum ValveReasonTag {
    PendingApproval,
    QuietHoursDigest,
    StaleOrMismatchedApproval,
    OversightBudgetExhausted,
    TaintBlocked,
    NonOwnerIdentityChange,
    Line2ScopeStripped,
    UnknownAudience,
    BudgetExhausted,
    ChainBrake,
    CedarDenied,
    HookWideningOverridden,
    TaintedProcedureWrite,
    DailyCapReached,
    PeerLoopBrake,
}

impl ValveReasonTag {
    /// Complete ordered list of every canonical [`ValveReasonTag`].
    pub const ALL: [Self; 15] = [
        Self::PendingApproval,
        Self::QuietHoursDigest,
        Self::StaleOrMismatchedApproval,
        Self::OversightBudgetExhausted,
        Self::TaintBlocked,
        Self::NonOwnerIdentityChange,
        Self::Line2ScopeStripped,
        Self::UnknownAudience,
        Self::BudgetExhausted,
        Self::ChainBrake,
        Self::CedarDenied,
        Self::HookWideningOverridden,
        Self::TaintedProcedureWrite,
        Self::DailyCapReached,
        Self::PeerLoopBrake,
    ];

    /// Canonical wire/receipt reason code string.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PendingApproval => "PendingApproval",
            Self::QuietHoursDigest => "quiet_hours_digest",
            Self::StaleOrMismatchedApproval => "stale_or_mismatched_approval",
            Self::OversightBudgetExhausted => "oversight_budget_exhausted",
            Self::TaintBlocked => "taint_blocked",
            Self::NonOwnerIdentityChange => "non_owner_identity_change",
            Self::Line2ScopeStripped => "line2_scope_stripped",
            Self::UnknownAudience => "unknown_audience",
            Self::BudgetExhausted => "budget_exhausted",
            Self::ChainBrake => "chain_brake",
            Self::CedarDenied => "cedar_denied",
            Self::HookWideningOverridden => "hook_widening_overridden",
            Self::TaintedProcedureWrite => "tainted_procedure_write",
            Self::DailyCapReached => "daily_cap_reached",
            Self::PeerLoopBrake => "peer_loop_brake",
        }
    }
}

impl fmt::Display for ValveReasonTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Exhaustive `[SELF_GUIDE]` metadata entry paired with one [`ValveReasonTag`].
///
/// Mirrors `monoagens_core::SelfGuideTemplateEntry` conventions without exposing Cedar
/// policy internals or tunable threshold constants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ValveGuideEntry {
    pub tag: ValveReasonTag,
    pub canonical_str: &'static str,
    pub template_key: &'static str,
    pub summary_tag: &'static str,
    pub self_guide_cue: &'static str,
}

/// Exhaustive compile-time mapping from every [`ValveReasonTag`] to its `[SELF_GUIDE]` entry.
///
/// Adding a new [`ValveReasonTag`] variant without adding a matching entry here is a
/// compile-time error.
#[must_use]
pub const fn valve_guide_entry(tag: ValveReasonTag) -> ValveGuideEntry {
    match tag {
        ValveReasonTag::PendingApproval => ValveGuideEntry {
            tag,
            canonical_str: "PendingApproval",
            template_key: "valve.guide.pending_approval",
            summary_tag: "owner_approval_held",
            self_guide_cue: "action held awaiting explicit owner approval",
        },
        ValveReasonTag::QuietHoursDigest => ValveGuideEntry {
            tag,
            canonical_str: "quiet_hours_digest",
            template_key: "valve.guide.quiet_hours_digest",
            summary_tag: "quiet_hours_batched_digest",
            self_guide_cue: "non-urgent notification deferred to recipient quiet-hours digest",
        },
        ValveReasonTag::StaleOrMismatchedApproval => ValveGuideEntry {
            tag,
            canonical_str: "stale_or_mismatched_approval",
            template_key: "valve.guide.stale_or_mismatched_approval",
            summary_tag: "approval_hash_or_epoch_mismatch",
            self_guide_cue: "approval invalid due to expired TTL, epoch shift, or action change",
        },
        ValveReasonTag::OversightBudgetExhausted => ValveGuideEntry {
            tag,
            canonical_str: "oversight_budget_exhausted",
            template_key: "valve.guide.oversight_budget_exhausted",
            summary_tag: "approval_queue_or_rate_cap",
            self_guide_cue: "approval prompt cap reached; excess requests refused",
        },
        ValveReasonTag::TaintBlocked => ValveGuideEntry {
            tag,
            canonical_str: "taint_blocked",
            template_key: "valve.guide.taint_blocked",
            summary_tag: "untrusted_provenance_quarantine",
            self_guide_cue: "privileged or cross-audience act restricted under untrusted taint",
        },
        ValveReasonTag::NonOwnerIdentityChange => ValveGuideEntry {
            tag,
            canonical_str: "non_owner_identity_change",
            template_key: "valve.guide.non_owner_identity_change",
            summary_tag: "absolute_line_1_owner_only",
            self_guide_cue: "identity modification permitted exclusively for owner",
        },
        ValveReasonTag::Line2ScopeStripped => ValveGuideEntry {
            tag,
            canonical_str: "line2_scope_stripped",
            template_key: "valve.guide.line2_scope_stripped",
            summary_tag: "absolute_line_2_scope_isolation",
            self_guide_cue: "out-of-scope context items stripped absent valid declassification",
        },
        ValveReasonTag::UnknownAudience => ValveGuideEntry {
            tag,
            canonical_str: "unknown_audience",
            template_key: "valve.guide.unknown_audience",
            summary_tag: "missing_or_unknown_audience",
            self_guide_cue: "event refused due to absent or unknown audience",
        },
        ValveReasonTag::BudgetExhausted => ValveGuideEntry {
            tag,
            canonical_str: "budget_exhausted",
            template_key: "valve.guide.budget_exhausted",
            summary_tag: "daily_token_budget_exhausted",
            self_guide_cue: "daily token budget exhausted for non-exempt wakes or monitors",
        },
        ValveReasonTag::ChainBrake => ValveGuideEntry {
            tag,
            canonical_str: "chain_brake",
            template_key: "valve.guide.chain_brake",
            summary_tag: "causal_chain_wake_limit",
            self_guide_cue: "causal chain wake limit reached",
        },
        ValveReasonTag::CedarDenied => ValveGuideEntry {
            tag,
            canonical_str: "cedar_denied",
            template_key: "valve.guide.cedar_denied",
            summary_tag: "policy_default_deny",
            self_guide_cue: "action not permitted by active authorization policy",
        },
        ValveReasonTag::HookWideningOverridden => ValveGuideEntry {
            tag,
            canonical_str: "hook_widening_overridden",
            template_key: "valve.guide.hook_widening_overridden",
            summary_tag: "first_and_last_override",
            self_guide_cue: "secondary hook widening attempt overridden by closing valve pass",
        },
        ValveReasonTag::TaintedProcedureWrite => ValveGuideEntry {
            tag,
            canonical_str: "tainted_procedure_write",
            template_key: "valve.guide.tainted_procedure_write",
            summary_tag: "tainted_procedure_write_blocked",
            self_guide_cue: "untrusted or unsigned procedure memory write blocked",
        },
        ValveReasonTag::DailyCapReached => ValveGuideEntry {
            tag,
            canonical_str: "daily_cap_reached",
            template_key: "valve.guide.daily_cap_reached",
            summary_tag: "proactive_daily_cap_reached",
            self_guide_cue: "proactive daily notification cap reached",
        },
        ValveReasonTag::PeerLoopBrake => ValveGuideEntry {
            tag,
            canonical_str: "peer_loop_brake",
            template_key: "valve.guide.peer_loop_brake",
            summary_tag: "automated_peer_loop_brake",
            self_guide_cue: "consecutive automated peer turns exceeded limit; loop brake active",
        },
    }
}

/// Returns the full exhaustive [`ValveGuideEntry`] table for all canonical tags.
#[must_use]
pub fn valve_guide_table() -> Vec<ValveGuideEntry> {
    ValveReasonTag::ALL
        .iter()
        .copied()
        .map(valve_guide_entry)
        .collect()
}

/// Builds a typed read-only [`ValveSelfSnapshot`] reflecting live [`ValveConfig`],
/// [`ValveSnapshot`], [`ApprovalManager`], and [`ProactiveCapTracker`] state.
#[must_use]
pub fn build_valve_self_snapshot(
    config: &ValveConfig,
    snapshot: &ValveSnapshot,
    approvals: &ApprovalManager,
    proactive_tracker: &ProactiveCapTracker,
) -> ValveSelfSnapshot {
    if config.mode == ValveMode::Off || !config.any_rule_enabled() {
        return ValveSelfSnapshot {
            valve_mode: config.effective_mode_label().to_owned(),
            quiet_hours_active: false,
            pending_approval_count: 0,
            daily_cap_remaining: None,
            taint_gating_active: false,
            opaque_tags: Vec::new(),
        };
    }

    let local_view = snapshot.active_recipient.as_ref().and_then(|recipient| {
        let tz = snapshot
            .recipient_timezones
            .get(recipient)
            .map_or("UTC", String::as_str);
        evaluate_recipient_local_time(
            snapshot.t_now,
            tz,
            config.quiet_start_hour,
            config.quiet_end_hour,
        )
        .ok()
        .map(|view| (recipient, view))
    });

    let quiet_hours_active = config.quiet_hours_enabled
        && (local_view
            .as_ref()
            .map_or(snapshot.quiet_hours_active, |(_, v)| v.in_quiet_hours)
            || snapshot.quiet_hours_active);

    let pending_approval_count = if config.pending_approval_enabled {
        snapshot
            .pending_approvals
            .len()
            .max(approvals.pending().len())
    } else {
        0
    };

    let daily_cap_remaining = if config.proactive_cap_enabled {
        let used = local_view
            .as_ref()
            .map_or(snapshot.proactive_sent_today, |(recipient, v)| {
                proactive_tracker
                    .count_for(recipient, v.date)
                    .max(snapshot.proactive_sent_today)
            });
        Some(config.proactive_daily_cap.saturating_sub(used))
    } else {
        None
    };

    let taint_gating_active = config.taint_blocking_enabled;

    let mut opaque_tags = Vec::new();
    if config.pending_approval_enabled && pending_approval_count > 0 {
        opaque_tags.push(ValveReasonTag::PendingApproval.as_str().to_owned());
    }
    if config.pending_approval_enabled && pending_approval_count >= config.max_pending_queue {
        opaque_tags.push(ValveReasonTag::OversightBudgetExhausted.as_str().to_owned());
    }
    if quiet_hours_active {
        opaque_tags.push(ValveReasonTag::QuietHoursDigest.as_str().to_owned());
    }
    if taint_gating_active {
        opaque_tags.push(ValveReasonTag::TaintBlocked.as_str().to_owned());
    }
    opaque_tags.push(crate::self_guide::VERIFY_BEFORE_ABSTAIN_TAG.to_owned());
    if config.proactive_cap_enabled && daily_cap_remaining == Some(0) {
        opaque_tags.push(ValveReasonTag::DailyCapReached.as_str().to_owned());
    }
    if config.budget_guard_enabled && snapshot.daily_tokens_used >= config.daily_token_budget {
        opaque_tags.push(ValveReasonTag::BudgetExhausted.as_str().to_owned());
    }
    if config.peer_loop_brake_enabled
        && snapshot
            .consecutive_automated_turns_by_place
            .values()
            .any(|&c| c >= config.max_consecutive_automated_turns)
    {
        opaque_tags.push(ValveReasonTag::PeerLoopBrake.as_str().to_owned());
    }

    ValveSelfSnapshot {
        valve_mode: config.effective_mode_label().to_owned(),
        quiet_hours_active,
        pending_approval_count,
        daily_cap_remaining,
        taint_gating_active,
        opaque_tags,
    }
}
