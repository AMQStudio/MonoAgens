//! Deterministic Cedar policy Valve (`monoagens-valve`) governing Inbound, Context,
//! Action, and memory-write doors across Waist 1.

#![forbid(unsafe_code)]

pub mod approvals;
pub mod config;
pub mod hooks;
pub mod policy;
pub mod quiet_hours;
pub mod reason_tags;
pub mod roles;
pub mod snapshot;
pub mod taint;
pub mod valve;
pub mod verdict;

pub use approvals::{
    APPROVAL_TTL_CHAT_S, APPROVAL_TTL_EMBODIED_S, ActionRiskMetadata, ApprovalGrant,
    ApprovalManager, DEDUP_COOLDOWN_S, HoldRequestContext, MAX_PENDING_QUEUE, MAX_PROMPTS_PER_HOUR,
    MAX_RETRY_ON_HELD, RiskTier, canonical_json_bytes, classify_action_risk, compute_h_act,
};
pub use config::{ValveConfig, ValveMode, ValveRuleToggle};
pub use hooks::{
    ActionDoorPolicyPlugin, SecondaryDoorHook, WideningOverride, reconcile_first_and_last,
};
pub use policy::{
    DEFAULT_CEDAR_POLICIES, DEFAULT_CEDAR_SCHEMA, MemoryWritePolicyAttrs, PolicyBundle,
    PolicyError, ProofStatus, SymccProofReport,
};
pub use quiet_hours::{
    DIGEST_HOURS_LOCAL, DigestBatch, DigestItem, DigestQueue, EMERGENCY_URGENCY_CUTOFF,
    IMMEDIATE_URGENCY_CUTOFF, LocalDateKey, LocalTimeView, PROACTIVE_DAILY_CAP,
    ProactiveCapTracker, ProactiveEvaluation, QUIET_END_HOUR, QUIET_START_HOUR, QuietHoursError,
    evaluate_proactive_notification, evaluate_recipient_local_time, is_hour_in_quiet_window,
    next_digest_slot_ms,
};
pub use reason_tags::{
    ValveGuideEntry, ValveReasonTag, build_valve_self_snapshot, valve_guide_entry,
    valve_guide_table,
};
pub use roles::{
    AttestedRoles, PHYSICAL_PRESENCE_TTL_S, PHYSICAL_ROOM_RADIUS_M, ROOM_PRESENCE_TTL_S,
    compile_attested_roles,
};
pub use snapshot::{
    AttestedPhysicalPresence, AttestedPlacePresence, MemoryWriteRequest, ValveSnapshot,
    WakeDispatchHint,
};
pub use taint::{
    TaintActionCheck, check_action_under_taint, combine_taint, evaluate_context_turn_taint,
    is_cross_audience_action, is_memory_write_taint_blocked, join_compaction_taint,
};
pub use valve::Valve;
pub use verdict::{ValveAuditLog, ValveDoorKind, ValveVerdictRecord};
