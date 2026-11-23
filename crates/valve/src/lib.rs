//! Deterministic Cedar policy Valve (`monoagens-valve`) governing Inbound, Context,
//! Action, and memory-write doors across Waist 1.

#![forbid(unsafe_code)]

pub mod approvals;
pub mod config;
pub mod hooks;
pub mod policy;
pub mod quiet_hours;
pub mod rbac;
pub mod reason_tags;
pub mod roles;
pub mod self_guide;
pub mod snapshot;
pub mod taint;
pub mod valve;
pub mod verdict;

pub use approvals::{
    APPROVAL_TTL_CHAT_S, APPROVAL_TTL_EMBODIED_S, ActionRiskMetadata, ApprovalGrant,
    ApprovalLogRecord, ApprovalManager, ApprovalResolveError, ApprovalResolveOutcome,
    DEDUP_COOLDOWN_S, HoldRequestContext, MAX_PENDING_QUEUE, MAX_PROMPTS_PER_HOUR,
    MAX_RETRY_ON_HELD, MAX_UNTRUSTED_PROMPTS_PER_HOUR, MIN_TRUSTED_SLOTS_PER_HOUR,
    RUBBER_STAMP_LATENCY_MS, RiskTier, approval_signing_preimage, canonical_json_bytes,
    classify_action_risk, compute_h_act, default_test_approval_mac_key, derive_approval_mac_key,
    sign_approval_resolution, verify_approval_signature,
};
pub use config::{
    DEFAULT_MAX_A2A_HOPS, DEFAULT_MAX_CONSECUTIVE_AUTOMATED_TURNS, ValveConfig, ValveMode,
    ValveRuleToggle,
};
pub use hooks::{
    ActionDoorPolicyPlugin, SecondaryDoorHook, WideningOverride, reconcile_first_and_last,
};
pub use policy::{
    DEFAULT_CEDAR_POLICIES, DEFAULT_CEDAR_SCHEMA, MemoryWritePolicyAttrs, PolicyBundle,
    PolicyError, ProofStatus, SymccProofReport,
};
pub use quiet_hours::{
    DIGEST_HOURS_LOCAL, DigestBatch, DigestItem, DigestLogRecord, DigestQueue,
    EMERGENCY_URGENCY_CUTOFF, IMMEDIATE_URGENCY_CUTOFF, LocalDateKey, LocalTimeView,
    PROACTIVE_DAILY_CAP, ProactiveCapTracker, ProactiveEvaluation, QUIET_END_HOUR,
    QUIET_START_HOUR, QuietHoursError, evaluate_proactive_notification,
    evaluate_recipient_local_time, is_hour_in_quiet_window, next_digest_slot_ms,
};
pub use rbac::{
    ApprovalMode, PERMISSION_AGENT_CONVERSE, PlaceAccessRule, RbacConfig, RbacDecision,
    RoleDefinition, RoleTier, agent_converse_permitted, matches_pattern,
};
pub use reason_tags::{
    ValveGuideEntry, ValveReasonTag, build_valve_self_snapshot, valve_guide_entry,
    valve_guide_table,
};
pub use roles::{
    AttestedRoles, PHYSICAL_PRESENCE_TTL_S, PHYSICAL_ROOM_RADIUS_M, ROOM_PRESENCE_TTL_S,
    compile_attested_roles,
};
pub use self_guide::{
    ProvenanceFrame, VERIFY_BEFORE_ABSTAIN_RULE, VERIFY_BEFORE_ABSTAIN_TAG,
    classify_segment_provenance, has_authoritative_valve_block, render_valve_self_guide_framing,
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
