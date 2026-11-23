//! Generic, config-driven Role-Based Access Control (RBAC) narrowing layer.
//!
//! RBAC evaluates deterministically inside [`crate::Valve`] strictly as a
//! **narrowing** check: it never widens Cedar policies, Absolute Line 1
//! (`owner`-only `SelfModify` / `identity_changing`), Absolute Line 2
//! (`VisibleScope`), or 3-door taint quarantine.

use core::fmt;
use std::collections::{BTreeMap, BTreeSet};

use monoagens_protocol::{ActorId, PlaceId};
use serde::{Deserialize, Serialize};

/// RBAC permission that lets a message wake the agent (`agent.converse`).
pub const PERMISSION_AGENT_CONVERSE: &str = "agent.converse";

/// Ordered privilege tiers (`Guest < Member < Trusted < Admin < Owner`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoleTier {
    Guest,
    Member,
    Trusted,
    Admin,
    Owner,
}

impl RoleTier {
    /// Canonical lowercase string representation.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Guest => "guest",
            Self::Member => "member",
            Self::Trusted => "trusted",
            Self::Admin => "admin",
            Self::Owner => "owner",
        }
    }

    /// Parses a tier name case-insensitively (`guest`, `member`, `trusted`, `admin`, `owner`).
    #[must_use]
    pub fn parse_str(s: &str) -> Option<Self> {
        let trimmed = s.trim();
        if trimmed.eq_ignore_ascii_case("guest") {
            Some(Self::Guest)
        } else if trimmed.eq_ignore_ascii_case("member") {
            Some(Self::Member)
        } else if trimmed.eq_ignore_ascii_case("trusted") {
            Some(Self::Trusted)
        } else if trimmed.eq_ignore_ascii_case("admin") {
            Some(Self::Admin)
        } else if trimmed.eq_ignore_ascii_case("owner") {
            Some(Self::Owner)
        } else {
            None
        }
    }
}

impl fmt::Display for RoleTier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Per-role approval strictness (`Yolo < Default < Strict`).
///
/// When multiple roles apply to an actor, the narrowest (`Strict`) mode wins.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalMode {
    Yolo,
    #[default]
    Default,
    Strict,
}

impl ApprovalMode {
    /// Canonical lowercase string representation.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Yolo => "yolo",
            Self::Default => "default",
            Self::Strict => "strict",
        }
    }

    /// Parses an approval mode name case-insensitively (`yolo`, `default`, `strict`).
    #[must_use]
    pub fn parse_str(s: &str) -> Option<Self> {
        let trimmed = s.trim();
        if trimmed.eq_ignore_ascii_case("yolo") {
            Some(Self::Yolo)
        } else if trimmed.eq_ignore_ascii_case("default") {
            Some(Self::Default)
        } else if trimmed.eq_ignore_ascii_case("strict") {
            Some(Self::Strict)
        } else {
            None
        }
    }
}

impl fmt::Display for ApprovalMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Named RBAC role definition with tier base, pattern overrides, approval mode, and generic settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoleDefinition {
    pub name: String,
    pub base: RoleTier,
    #[serde(default)]
    pub allow: Vec<String>,
    #[serde(default)]
    pub deny: Vec<String>,
    #[serde(default)]
    pub approval: Option<ApprovalMode>,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub context: Option<String>,
    #[serde(default)]
    pub settings: BTreeMap<String, serde_json::Value>,
}

/// Per-place capability and permission narrowing rule.
///
/// `deny` blocks matching capabilities/permissions for non-owners; if `allow` is non-empty,
/// non-owners must match at least one `allow` pattern. The `Owner` is above all tiers and
/// bypasses place rules (unless placed in an explicit `ApprovalMode::Strict` role).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PlaceAccessRule {
    #[serde(default)]
    pub allow: Vec<String>,
    #[serde(default)]
    pub deny: Vec<String>,
}

/// Outcome of an RBAC narrowing evaluation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum RbacDecision {
    Allow,
    Hold { reason: String },
    Deny { reason: String },
}

/// Config-driven RBAC policy table evaluated deterministically by [`crate::Valve`].
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct RbacConfig {
    #[serde(default)]
    pub fallback_role: Option<String>,
    #[serde(default)]
    pub roles: BTreeMap<String, RoleDefinition>,
    #[serde(default)]
    pub actor_roles: BTreeMap<ActorId, Vec<String>>,
    #[serde(default)]
    pub places: BTreeMap<PlaceId, PlaceAccessRule>,
    /// Minimum [`RoleTier`] required per capability/permission ID (`None` = owner-only).
    #[serde(default)]
    pub permission_bases: BTreeMap<String, Option<RoleTier>>,
}

impl RbacConfig {
    /// Returns `true` when no fallback role, roles, actor bindings, place rules, or permission bases are configured.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.fallback_role.is_none()
            && self.roles.is_empty()
            && self.actor_roles.is_empty()
            && self.places.is_empty()
            && self.permission_bases.is_empty()
    }

    /// Resolves the ordered, deduplicated list of [`RoleDefinition`]s held by `actor`
    /// (falling back to `fallback_role` if the actor has no valid assigned roles).
    #[must_use]
    pub fn effective_roles(&self, actor: &ActorId) -> Vec<&RoleDefinition> {
        let mut resolved = self.explicit_roles(actor);
        if resolved.is_empty()
            && let Some(fb) = &self.fallback_role
            && let Some(role) = self.roles.get(fb)
        {
            resolved.push(role);
        }
        resolved
    }

    /// Returns the effective [`RoleTier`] for `actor` (`RoleTier::Owner` when `actor == owner`,
    /// otherwise the highest base tier across `effective_roles`, defaulting to `RoleTier::Guest`).
    #[must_use]
    pub fn effective_tier(&self, actor: &ActorId, owner: &ActorId) -> RoleTier {
        self.effective_tier_opt(actor, Some(owner))
    }

    /// Returns the effective [`ApprovalMode`] for `actor`.
    ///
    /// Multiple roles combine monotonically toward the narrowest mode (`Strict` > `Default` > `Yolo`).
    /// For `owner`, only explicitly assigned roles (not `fallback_role`) are considered.
    #[must_use]
    pub fn effective_approval_mode(&self, actor: &ActorId, owner: &ActorId) -> ApprovalMode {
        self.effective_approval_mode_opt(actor, Some(owner))
    }

    /// Merges generic per-role settings for `actor` in ascending [`RoleTier`] order so higher-tier
    /// roles deterministically override lower-tier roles on key collisions.
    #[must_use]
    pub fn effective_settings(&self, actor: &ActorId) -> BTreeMap<String, serde_json::Value> {
        let mut roles = self.effective_roles(actor);
        roles.sort_by_key(|role| role.base);
        let mut merged = BTreeMap::new();
        for role in roles {
            for (k, v) in &role.settings {
                merged.insert(k.clone(), v.clone());
            }
        }
        merged
    }

    /// Evaluates RBAC access for `(actor, place, capability_or_permission)` against `owner`.
    #[must_use]
    pub fn evaluate(
        &self,
        actor: &ActorId,
        owner: &ActorId,
        place: Option<&PlaceId>,
        capability_or_permission: &str,
    ) -> RbacDecision {
        self.evaluate_with_optional_owner(actor, Some(owner), place, capability_or_permission)
    }

    /// Evaluates RBAC access when the snapshot owner may be absent (`None` = no actor is owner).
    #[must_use]
    pub fn evaluate_with_optional_owner(
        &self,
        actor: &ActorId,
        owner: Option<&ActorId>,
        place: Option<&PlaceId>,
        capability_or_permission: &str,
    ) -> RbacDecision {
        self.evaluate_with_default_base(actor, owner, place, capability_or_permission, None)
    }

    /// Like [`Self::evaluate_with_optional_owner`], but `default_base` is the minimum tier
    /// required when `[permission_bases]` has no entry for the permission (an explicit entry
    /// always wins).
    #[must_use]
    pub fn evaluate_with_default_base(
        &self,
        actor: &ActorId,
        owner: Option<&ActorId>,
        place: Option<&PlaceId>,
        capability_or_permission: &str,
        default_base: Option<RoleTier>,
    ) -> RbacDecision {
        let is_owner = owner.is_some_and(|o| o == actor);

        // 1. Owner is above every tier unless explicitly assigned an `ApprovalMode::Strict` role.
        if is_owner {
            if self.explicit_roles(actor).iter().any(|r| r.approval == Some(ApprovalMode::Strict)) {
                return RbacDecision::Hold {
                    reason: format!(
                        "owner role requires strict approval for '{capability_or_permission}'"
                    ),
                };
            }
            return RbacDecision::Allow;
        }

        // 2. Owner-only permission bases (`None`) always deny non-owners.
        let required_base: Option<Option<RoleTier>> = self
            .permission_bases
            .get(capability_or_permission)
            .copied()
            .or(default_base.map(Some));
        if matches!(required_base, Some(None)) {
            return RbacDecision::Deny {
                reason: format!("permission '{capability_or_permission}' is restricted to owner"),
            };
        }

        // 3. Per-place narrowing rules (`deny` overrides, non-empty `allow` requires match).
        if let Some(place_id) = place
            && let Some(place_rule) = self.places.get(place_id)
        {
            if place_rule
                .deny
                .iter()
                .any(|pat| matches_pattern(pat, capability_or_permission))
            {
                return RbacDecision::Deny {
                    reason: format!(
                        "place '{}' denies '{capability_or_permission}'",
                        place_id.as_str()
                    ),
                };
            }
            if !place_rule.allow.is_empty()
                && !place_rule
                    .allow
                    .iter()
                    .any(|pat| matches_pattern(pat, capability_or_permission))
            {
                return RbacDecision::Deny {
                    reason: format!(
                        "place '{}' does not allow '{capability_or_permission}'",
                        place_id.as_str()
                    ),
                };
            }
        }

        let roles = self.effective_roles(actor);
        let approval_mode = self.effective_approval_mode_opt(actor, owner);

        // 4. Explicit role `deny` patterns override role `allow` and tier bases.
        if let Some(denying_role) = roles.iter().find(|role| {
            role.deny
                .iter()
                .any(|pat| matches_pattern(pat, capability_or_permission))
        }) {
            return RbacDecision::Deny {
                reason: format!(
                    "role '{}' denies '{capability_or_permission}'",
                    denying_role.name
                ),
            };
        }

        // 5. Explicit role `allow` patterns permit access (subject to `ApprovalMode::Strict`).
        if roles.iter().any(|role| {
            role.allow
                .iter()
                .any(|pat| matches_pattern(pat, capability_or_permission))
        }) {
            return Self::allow_or_strict_hold(approval_mode, capability_or_permission);
        }

        // 6. Tier-base comparison against `permission_bases` (default-deny for non-owners).
        let tier = self.effective_tier_opt(actor, owner);
        match required_base {
            Some(Some(required_tier)) if tier >= required_tier => {
                Self::allow_or_strict_hold(approval_mode, capability_or_permission)
            }
            Some(Some(required_tier)) => RbacDecision::Deny {
                reason: format!(
                    "tier '{}' is below required '{}' for '{capability_or_permission}'",
                    tier.as_str(),
                    required_tier.as_str()
                ),
            },
            _ => RbacDecision::Deny {
                reason: format!(
                    "capability '{capability_or_permission}' is not granted to role/tier '{}'",
                    tier.as_str()
                ),
            },
        }
    }

    /// Returns `true` when `evaluate` does not return [`RbacDecision::Deny`].
    #[must_use]
    pub fn is_permitted(
        &self,
        actor: &ActorId,
        owner: &ActorId,
        place: Option<&PlaceId>,
        permission: &str,
    ) -> bool {
        !matches!(
            self.evaluate(actor, owner, place, permission),
            RbacDecision::Deny { .. }
        )
    }

    fn explicit_roles(&self, actor: &ActorId) -> Vec<&RoleDefinition> {
        let mut resolved = Vec::new();
        let mut seen = BTreeSet::new();
        if let Some(names) = self.actor_roles.get(actor) {
            for name in names {
                if seen.insert(name.as_str())
                    && let Some(role) = self.roles.get(name)
                {
                    resolved.push(role);
                }
            }
        }
        resolved
    }

    fn effective_tier_opt(&self, actor: &ActorId, owner: Option<&ActorId>) -> RoleTier {
        if owner.is_some_and(|o| o == actor) {
            return RoleTier::Owner;
        }
        self.effective_roles(actor)
            .into_iter()
            .map(|r| r.base)
            .max()
            .unwrap_or(RoleTier::Guest)
    }

    fn effective_approval_mode_opt(&self, actor: &ActorId, owner: Option<&ActorId>) -> ApprovalMode {
        let roles = if owner.is_some_and(|o| o == actor) {
            self.explicit_roles(actor)
        } else {
            self.effective_roles(actor)
        };
        roles
            .into_iter()
            .filter_map(|r| r.approval)
            .max()
            .unwrap_or(ApprovalMode::Default)
    }

    fn allow_or_strict_hold(approval_mode: ApprovalMode, target: &str) -> RbacDecision {
        if approval_mode == ApprovalMode::Strict {
            RbacDecision::Hold {
                reason: format!("role approval mode 'strict' holds '{target}'"),
            }
        } else {
            RbacDecision::Allow
        }
    }
}

/// Matches a capability/permission pattern supporting exact match, `"*"`, `"prefix.*"`, and `"prefix*"`.
#[must_use]
pub fn matches_pattern(pattern: &str, target: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    if let Some(prefix) = pattern.strip_suffix('*') {
        return target.starts_with(prefix);
    }
    pattern == target
}

/// `true` when `actor` may wake the agent (`agent.converse`).
///
/// The owner always may. With an RBAC table the permission is resolved like any other; an
/// unregistered sender (a guest) additionally needs at least tier `member` unless
/// `[permission_bases]` says otherwise. Without a table only registered senders are served once
/// guest access is on.
#[must_use]
pub fn agent_converse_permitted(
    rbac: Option<&RbacConfig>,
    owner: Option<&ActorId>,
    actor: &ActorId,
    place: Option<&PlaceId>,
    registered: bool,
    guest_access: bool,
) -> bool {
    if owner.is_some_and(|o| o == actor) {
        return true;
    }
    match rbac {
        Some(cfg) => {
            let default_base = if registered || !guest_access {
                Some(RoleTier::Guest)
            } else {
                Some(RoleTier::Member)
            };
            !matches!(
                cfg.evaluate_with_default_base(
                    actor,
                    owner,
                    place,
                    PERMISSION_AGENT_CONVERSE,
                    default_base,
                ),
                RbacDecision::Deny { .. }
            )
        }
        None => registered || !guest_access,
    }
}
