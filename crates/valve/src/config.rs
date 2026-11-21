//! Configurable, individually toggleable rules and operational modes for `Valve`.

use serde::{Deserialize, Serialize};

use crate::approvals::{
    APPROVAL_TTL_CHAT_S, APPROVAL_TTL_EMBODIED_S, DEDUP_COOLDOWN_S, MAX_PENDING_QUEUE,
    MAX_PROMPTS_PER_HOUR,
};
use crate::quiet_hours::{
    DIGEST_HOURS_LOCAL, EMERGENCY_URGENCY_CUTOFF, IMMEDIATE_URGENCY_CUTOFF, PROACTIVE_DAILY_CAP,
    QUIET_END_HOUR, QUIET_START_HOUR,
};
use crate::roles::{PHYSICAL_PRESENCE_TTL_S, PHYSICAL_ROOM_RADIUS_M, ROOM_PRESENCE_TTL_S};

/// Default consecutive automated-turn brake threshold per place (`C-14`).
pub const DEFAULT_MAX_CONSECUTIVE_AUTOMATED_TURNS: u32 = 4;

/// Default maximum transitive A2A delegation hop count (`C-14`).
pub const DEFAULT_MAX_A2A_HOPS: u8 = 3;

/// Operational mode recorded in `ValveSelfSnapshot` and `door_verdict` logs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValveMode {
    Enforcing,
    Partial,
    Off,
}

impl ValveMode {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Enforcing => "enforcing",
            Self::Partial => "partial",
            Self::Off => "off",
        }
    }
}

/// Identifiers for individually toggleable Valve rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValveRuleToggle {
    CedarAbac,
    AttestedRoles,
    Line2Scope,
    BudgetGuard,
    ChainBrake,
    MemoryWriteGuard,
    PendingApproval,
    TaintBlocking,
    QuietHours,
    ProactiveCap,
    PeerLoopBrake,
}

impl ValveRuleToggle {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CedarAbac => "cedar_abac",
            Self::AttestedRoles => "attested_roles",
            Self::Line2Scope => "line2_scope",
            Self::BudgetGuard => "budget_guard",
            Self::ChainBrake => "chain_brake",
            Self::MemoryWriteGuard => "memory_write_guard",
            Self::PendingApproval => "pending_approval",
            Self::TaintBlocking => "taint_blocking",
            Self::QuietHours => "quiet_hours",
            Self::ProactiveCap => "proactive_cap",
            Self::PeerLoopBrake => "peer_loop_brake",
        }
    }
}

/// Complete Valve configuration with per-rule toggles and bounded numeric limits.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ValveConfig {
    pub mode: ValveMode,
    pub cedar_enabled: bool,
    pub attested_roles_enabled: bool,
    pub line2_enabled: bool,
    pub budget_guard_enabled: bool,
    pub chain_brake_enabled: bool,
    pub memory_write_guard_enabled: bool,
    pub pending_approval_enabled: bool,
    pub taint_blocking_enabled: bool,
    pub quiet_hours_enabled: bool,
    pub proactive_cap_enabled: bool,
    pub peer_loop_brake_enabled: bool,

    pub room_presence_ttl_s: u64,
    pub physical_presence_ttl_s: u64,
    pub physical_room_radius_m: f64,

    pub daily_token_budget: u64,
    pub max_chain_wakes: u32,
    pub proactive_daily_cap: u32,
    pub max_consecutive_automated_turns: u32,
    pub max_a2a_hops: u8,

    // V2 approval lifecycle & sliding caps
    pub approval_ttl_chat_s: u64,
    pub approval_ttl_embodied_s: u64,
    pub max_prompts_per_hour: usize,
    pub max_pending_queue: usize,
    pub dedup_cooldown_s: u64,

    // V2 quiet hours & digests
    pub quiet_start_hour: u8,
    pub quiet_end_hour: u8,
    pub emergency_urgency_cutoff: f64,
    pub immediate_urgency_cutoff: f64,
    pub digest_hours_local: [u8; 3],
}

impl Default for ValveConfig {
    fn default() -> Self {
        Self {
            mode: ValveMode::Enforcing,
            cedar_enabled: true,
            attested_roles_enabled: true,
            line2_enabled: true,
            budget_guard_enabled: true,
            chain_brake_enabled: true,
            memory_write_guard_enabled: true,
            pending_approval_enabled: true,
            taint_blocking_enabled: true,
            quiet_hours_enabled: false,
            proactive_cap_enabled: true,
            peer_loop_brake_enabled: true,
            room_presence_ttl_s: ROOM_PRESENCE_TTL_S,
            physical_presence_ttl_s: PHYSICAL_PRESENCE_TTL_S,
            physical_room_radius_m: PHYSICAL_ROOM_RADIUS_M,
            daily_token_budget: 100_000,
            max_chain_wakes: 8,
            proactive_daily_cap: PROACTIVE_DAILY_CAP,
            max_consecutive_automated_turns: DEFAULT_MAX_CONSECUTIVE_AUTOMATED_TURNS,
            max_a2a_hops: DEFAULT_MAX_A2A_HOPS,
            approval_ttl_chat_s: APPROVAL_TTL_CHAT_S,
            approval_ttl_embodied_s: APPROVAL_TTL_EMBODIED_S,
            max_prompts_per_hour: MAX_PROMPTS_PER_HOUR,
            max_pending_queue: MAX_PENDING_QUEUE,
            dedup_cooldown_s: DEDUP_COOLDOWN_S,
            quiet_start_hour: QUIET_START_HOUR,
            quiet_end_hour: QUIET_END_HOUR,
            emergency_urgency_cutoff: EMERGENCY_URGENCY_CUTOFF,
            immediate_urgency_cutoff: IMMEDIATE_URGENCY_CUTOFF,
            digest_hours_local: DIGEST_HOURS_LOCAL,
        }
    }
}

impl ValveConfig {
    /// Returns a configuration with `ValveMode::Off` and all rule toggles disabled.
    #[must_use]
    pub fn off() -> Self {
        Self {
            mode: ValveMode::Off,
            cedar_enabled: false,
            attested_roles_enabled: false,
            line2_enabled: false,
            budget_guard_enabled: false,
            chain_brake_enabled: false,
            memory_write_guard_enabled: false,
            pending_approval_enabled: false,
            taint_blocking_enabled: false,
            quiet_hours_enabled: false,
            proactive_cap_enabled: false,
            peer_loop_brake_enabled: false,
            ..Self::default()
        }
    }

    /// Returns `true` if `mode != ValveMode::Off` and the specified rule is enabled.
    #[must_use]
    pub const fn is_rule_active(&self, rule: ValveRuleToggle) -> bool {
        if matches!(self.mode, ValveMode::Off) {
            return false;
        }
        match rule {
            ValveRuleToggle::CedarAbac => self.cedar_enabled,
            ValveRuleToggle::AttestedRoles => self.attested_roles_enabled,
            ValveRuleToggle::Line2Scope => self.line2_enabled,
            ValveRuleToggle::BudgetGuard => self.budget_guard_enabled,
            ValveRuleToggle::ChainBrake => self.chain_brake_enabled,
            ValveRuleToggle::MemoryWriteGuard => self.memory_write_guard_enabled,
            ValveRuleToggle::PendingApproval => self.pending_approval_enabled,
            ValveRuleToggle::TaintBlocking => self.taint_blocking_enabled,
            ValveRuleToggle::QuietHours => self.quiet_hours_enabled,
            ValveRuleToggle::ProactiveCap => self.proactive_cap_enabled,
            ValveRuleToggle::PeerLoopBrake => self.peer_loop_brake_enabled,
        }
    }

    /// Returns `true` if any enforcement rule is enabled.
    #[must_use]
    pub const fn any_rule_enabled(&self) -> bool {
        self.cedar_enabled
            || self.line2_enabled
            || self.budget_guard_enabled
            || self.chain_brake_enabled
            || self.memory_write_guard_enabled
            || self.pending_approval_enabled
            || self.taint_blocking_enabled
            || self.quiet_hours_enabled
            || self.proactive_cap_enabled
            || self.peer_loop_brake_enabled
    }

    /// Returns the effective mode label (`"off"` when `ValveMode::Off` or no rules are enabled).
    #[must_use]
    pub const fn effective_mode_label(&self) -> &'static str {
        if matches!(self.mode, ValveMode::Off) || !self.any_rule_enabled() {
            "off"
        } else {
            self.mode.as_str()
        }
    }

    /// Sets an individual rule toggle, returning the previous value.
    pub fn set_rule(&mut self, rule: ValveRuleToggle, enabled: bool) -> bool {
        let slot = match rule {
            ValveRuleToggle::CedarAbac => &mut self.cedar_enabled,
            ValveRuleToggle::AttestedRoles => &mut self.attested_roles_enabled,
            ValveRuleToggle::Line2Scope => &mut self.line2_enabled,
            ValveRuleToggle::BudgetGuard => &mut self.budget_guard_enabled,
            ValveRuleToggle::ChainBrake => &mut self.chain_brake_enabled,
            ValveRuleToggle::MemoryWriteGuard => &mut self.memory_write_guard_enabled,
            ValveRuleToggle::PendingApproval => &mut self.pending_approval_enabled,
            ValveRuleToggle::TaintBlocking => &mut self.taint_blocking_enabled,
            ValveRuleToggle::QuietHours => &mut self.quiet_hours_enabled,
            ValveRuleToggle::ProactiveCap => &mut self.proactive_cap_enabled,
            ValveRuleToggle::PeerLoopBrake => &mut self.peer_loop_brake_enabled,
        };
        let prev = *slot;
        *slot = enabled;
        prev
    }
}
