//! Risk tier classification (`R_0`..`R_3`), canonical `H_act` binding, and
//! non-retrying `PendingApproval` lifecycle with sliding oversight caps (`R-42`).

use std::collections::{BTreeMap, BTreeSet};

use monoagens_core::DoorDecision;
use monoagens_protocol::{
    Action, ActionCanonicalInput, ActionId, ActorId, CapabilityFlags, HAct, PendingApproval,
    PlaceId, Taint, Time,
};
use serde::{Deserialize, Serialize};

use crate::config::ValveConfig;
use crate::reason_tags::ValveReasonTag;

/// Maximum automatic retries for held `PendingApproval` items (`0`: never auto-retry).
pub const MAX_RETRY_ON_HELD: u32 = 0;
/// Default approval TTL in seconds for chat/standard places (`900 s`).
pub const APPROVAL_TTL_CHAT_S: u64 = 900;
/// Short-lived approval TTL in seconds for embodied/physical actuator places (`30 s`).
pub const APPROVAL_TTL_EMBODIED_S: u64 = 30;
/// Maximum approval prompts emitted per sliding 1-hour window (`6/h`).
pub const MAX_PROMPTS_PER_HOUR: usize = 6;
/// Maximum concurrent items in `PendingApproval` queue (`3`).
pub const MAX_PENDING_QUEUE: usize = 3;
/// Deduplication cooldown in seconds before re-prompting an identical `H_act` (`600 s`).
pub const DEDUP_COOLDOWN_S: u64 = 600;

/// Four deterministic action risk tiers (`R_0..R_3`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskTier {
    /// Read-only or in-audience communicative reply.
    R0,
    /// Reversible, non-identity-changing, budgeted action.
    R1,
    /// Irreversible write, financial/credential act, `DeclassToken`, `procedure` write, or tainted call.
    R2,
    /// Non-owner `identity_changing`, unauthorized cross-audience act, or bad/stale `H_act`.
    R3,
}

/// Optional per-capability/per-action risk hints provided in [`crate::snapshot::ValveSnapshot`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ActionRiskMetadata {
    pub is_read_only: bool,
    pub is_financial_or_credential: bool,
    pub is_procedure_write: bool,
    pub is_declass_grant: bool,
    pub is_embodied: bool,
}

/// Deterministic `R_0..R_3` classification strictly from descriptor flags and snapshot facts
/// (never from model text).
#[must_use]
pub fn classify_action_risk(
    flags: CapabilityFlags,
    is_owner: bool,
    cross_audience_without_declass: bool,
    effective_taint: Taint,
    meta: ActionRiskMetadata,
) -> RiskTier {
    if (flags.identity_changing && !is_owner) || cross_audience_without_declass {
        return RiskTier::R3;
    }

    if flags.identity_changing
        || flags.irreversible
        || meta.is_financial_or_credential
        || meta.is_procedure_write
        || meta.is_declass_grant
    {
        return RiskTier::R2;
    }

    if effective_taint < Taint::Trusted && !flags.communicative && !meta.is_read_only {
        return RiskTier::R2;
    }

    if meta.is_read_only || (flags.communicative && !flags.irreversible) {
        return RiskTier::R0;
    }

    RiskTier::R1
}

/// Canonicalizes a JSON payload by sorting all object keys recursively in compact form;
/// non-JSON opaque bytes are returned verbatim.
#[must_use]
pub fn canonical_json_bytes(raw: &[u8]) -> Vec<u8> {
    let Ok(val) = serde_json::from_slice::<serde_json::Value>(raw) else {
        return raw.to_vec();
    };
    let canonical = canonicalize_json_value(val);
    serde_json::to_vec(&canonical).unwrap_or_else(|_| raw.to_vec())
}

fn canonicalize_json_value(val: serde_json::Value) -> serde_json::Value {
    match val {
        serde_json::Value::Object(map) => {
            let mut sorted: Vec<(String, serde_json::Value)> = map.into_iter().collect();
            sorted.sort_by(|a, b| a.0.cmp(&b.0));
            let mut out = serde_json::Map::with_capacity(sorted.len());
            for (k, v) in sorted {
                out.insert(k, canonicalize_json_value(v));
            }
            serde_json::Value::Object(out)
        }
        serde_json::Value::Array(arr) => {
            serde_json::Value::Array(arr.into_iter().map(canonicalize_json_value).collect())
        }
        other => other,
    }
}

/// Computes canonical action digest `H_act = BLAKE3("monoagens.act.v1" || canonical_preimage)`
/// over `ActionCanonicalInput` (with canonicalized JSON payload and sorted/deduped `places`).
#[must_use]
pub fn compute_h_act(action: &Action, state_epoch: u64) -> HAct {
    let canonical_payload = canonical_json_bytes(action.payload.as_bytes());
    let input = ActionCanonicalInput {
        capability_id: action.capability_id.clone(),
        actor: action.origin_actor.actor_id().clone(),
        places: action.places.clone(),
        state_epoch,
        payload: canonical_payload,
    };
    let digest = blake3::hash(&input.canonical_preimage());
    HAct::new(format!("blake3:{}", digest.to_hex()))
}

/// Owner approval grant bound cryptographically to `H_act`, `state_epoch`, and `approval_ttl_s`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalGrant {
    pub h_act: HAct,
    pub state_epoch: u64,
    pub granted_at_ms: Time,
    pub expires_at_ms: Time,
    pub signer: ActorId,
    pub signature: String,
    pub is_embodied: bool,
}

impl ApprovalGrant {
    /// Creates an owner-signed [`ApprovalGrant`] using `30 s` TTL for embodied actions
    /// and `900 s` TTL for chat/standard actions.
    #[must_use]
    pub fn new(
        h_act: HAct,
        state_epoch: u64,
        granted_at_ms: Time,
        is_embodied: bool,
        signer: ActorId,
        signature: impl Into<String>,
    ) -> Self {
        let ttl_s = if is_embodied {
            APPROVAL_TTL_EMBODIED_S
        } else {
            APPROVAL_TTL_CHAT_S
        };
        Self {
            h_act,
            state_epoch,
            granted_at_ms,
            expires_at_ms: granted_at_ms.saturating_add_ms(ttl_s.saturating_mul(1000)),
            signer,
            signature: signature.into(),
            is_embodied,
        }
    }
}

/// Stateful, deterministic `PendingApproval` + sliding-cap manager (`max_retry_on_held = 0`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalManager {
    pending: BTreeMap<ActionId, PendingApproval>,
    grants: BTreeMap<HAct, ApprovalGrant>,
    consumed_grants: BTreeSet<HAct>,
    prompt_timestamps_ms: Vec<Time>,
    dedup_last_prompt_ms: BTreeMap<HAct, Time>,
    prompts_emitted_total: u64,
}

/// Snapshot and classification context supplied to [`ApprovalManager::try_hold`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HoldRequestContext {
    pub state_epoch: u64,
    pub t_now: Time,
    pub is_embodied: bool,
    pub reason_tag: ValveReasonTag,
}

impl ApprovalManager {
    /// Creates an empty [`ApprovalManager`].
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers an owner [`ApprovalGrant`] bound to `H_act` and `state_epoch`.
    pub fn register_grant(&mut self, grant: ApprovalGrant) {
        self.pending.retain(|_, p| p.h_act != grant.h_act);
        self.grants.insert(grant.h_act.clone(), grant);
    }

    /// Returns currently active `PendingApproval` items.
    #[must_use]
    pub fn pending(&self) -> &BTreeMap<ActionId, PendingApproval> {
        &self.pending
    }

    /// Total number of prompts actually emitted (never exceeds sliding/dedup bounds).
    #[must_use]
    pub const fn prompts_emitted_total(&self) -> u64 {
        self.prompts_emitted_total
    }

    /// Prunes expired pending entries and old hourly prompt timestamps at `t_now`.
    pub fn prune_at(&mut self, t_now: Time) {
        self.pending.retain(|_, p| t_now <= p.expires_at_ms);
        let one_hour_ago = Time::from_millis(t_now.as_millis().saturating_sub(3_600_000));
        self.prompt_timestamps_ms.retain(|&ts| ts >= one_hour_ago);
    }

    /// Checks whether an action carrying `action.h_act` or matching an [`ApprovalGrant`]
    /// is validly authorized at `(t_now, state_epoch)`.
    ///
    /// Returns:
    /// - `Ok(true)` if validly approved by a matching, unexpired, same-epoch owner grant.
    /// - `Ok(false)` if no approval has been attempted yet (`action.h_act.is_none()` and no grant).
    /// - `Err(ValveReasonTag::StaleOrMismatchedApproval)` if TOCTOU mutation, epoch drift,
    ///   TTL expiration, or invalid signer occurred.
    pub fn check_approval_or_toctou(
        &mut self,
        action: &Action,
        computed_h_act: &HAct,
        owner_actor_id: Option<&ActorId>,
        state_epoch: u64,
        t_now: Time,
    ) -> Result<bool, ValveReasonTag> {
        if let Some(provided) = &action.h_act {
            if provided != computed_h_act {
                return Err(ValveReasonTag::StaleOrMismatchedApproval);
            }
            if let Some(grant) = self.grants.get(computed_h_act) {
                if !is_grant_valid(grant, computed_h_act, owner_actor_id, state_epoch, t_now) {
                    return Err(ValveReasonTag::StaleOrMismatchedApproval);
                }
                return Ok(true);
            }
            // If this exact action is currently held in PendingApproval at the same epoch/TTL
            // without a grant yet, treat as a duplicate retry of a held item (`max_retry_on_held = 0`).
            if let Some(pending) = self.pending.get(&action.action_id)
                && &pending.h_act == computed_h_act
                && pending.state_epoch == state_epoch
                && t_now <= pending.expires_at_ms
            {
                return Ok(false);
            }
            return Err(ValveReasonTag::StaleOrMismatchedApproval);
        }

        if let Some(grant) = self.grants.get(computed_h_act) {
            if !is_grant_valid(grant, computed_h_act, owner_actor_id, state_epoch, t_now) {
                return Err(ValveReasonTag::StaleOrMismatchedApproval);
            }
            return Ok(true);
        }

        Ok(false)
    }

    /// Consumes a verified grant upon dispatch so it cannot be replayed across turns.
    pub fn consume_grant(&mut self, h_act: &HAct) {
        self.grants.remove(h_act);
        self.consumed_grants.insert(h_act.clone());
    }

    /// Attempts to hold an unapproved action in `PendingApproval` subject to sliding caps
    /// (`max_pending_queue`, `max_prompts_per_hour`, `dedup_cooldown_s`, `max_retry_on_held = 0`).
    pub fn try_hold(
        &mut self,
        action: &mut Action,
        computed_h_act: HAct,
        ctx: HoldRequestContext,
        config: &ValveConfig,
    ) -> (DoorDecision, ValveReasonTag) {
        self.prune_at(ctx.t_now);

        // Zero retry on held items: if already held or within dedup cooldown, settle as refused.
        if self.pending.contains_key(&action.action_id) {
            return (
                DoorDecision::Refuse {
                    rule: "valve.approval.no_retry_on_held".to_owned(),
                    reason: ValveReasonTag::OversightBudgetExhausted.as_str().to_owned(),
                },
                ValveReasonTag::OversightBudgetExhausted,
            );
        }

        if let Some(&last_prompt) = self.dedup_last_prompt_ms.get(&computed_h_act) {
            let cooldown_ms = config.dedup_cooldown_s.saturating_mul(1000);
            if ctx.t_now.saturating_sub_ms(last_prompt) < cooldown_ms {
                return (
                    DoorDecision::Refuse {
                        rule: "valve.approval.dedup_cooldown".to_owned(),
                        reason: ValveReasonTag::OversightBudgetExhausted.as_str().to_owned(),
                    },
                    ValveReasonTag::OversightBudgetExhausted,
                );
            }
        }

        if self.pending.len() >= config.max_pending_queue {
            return (
                DoorDecision::Refuse {
                    rule: "valve.approval.max_pending_queue".to_owned(),
                    reason: ValveReasonTag::OversightBudgetExhausted.as_str().to_owned(),
                },
                ValveReasonTag::OversightBudgetExhausted,
            );
        }

        if self.prompt_timestamps_ms.len() >= config.max_prompts_per_hour {
            return (
                DoorDecision::Refuse {
                    rule: "valve.approval.max_prompts_per_hour".to_owned(),
                    reason: ValveReasonTag::OversightBudgetExhausted.as_str().to_owned(),
                },
                ValveReasonTag::OversightBudgetExhausted,
            );
        }

        let ttl_s = if ctx.is_embodied {
            config.approval_ttl_embodied_s
        } else {
            config.approval_ttl_chat_s
        };
        let expires_at_ms = ctx.t_now.saturating_add_ms(ttl_s.saturating_mul(1000));

        let mut places: Vec<PlaceId> = action.places.clone();
        places.sort();
        places.dedup();

        let pending = PendingApproval {
            action_id: action.action_id.clone(),
            capability_id: action.capability_id.clone(),
            h_act: computed_h_act.clone(),
            state_epoch: ctx.state_epoch,
            requested_at_ms: ctx.t_now,
            expires_at_ms,
            places,
            reason: Some(ctx.reason_tag.as_str().to_owned()),
        };

        action.h_act = Some(computed_h_act.clone());
        self.pending.insert(action.action_id.clone(), pending);
        self.prompt_timestamps_ms.push(ctx.t_now);
        self.dedup_last_prompt_ms.insert(computed_h_act, ctx.t_now);
        self.prompts_emitted_total = self.prompts_emitted_total.saturating_add(1);

        (
            DoorDecision::Hold {
                rule: "valve.approval.pending".to_owned(),
                reason: ctx.reason_tag.as_str().to_owned(),
            },
            ctx.reason_tag,
        )
    }
}

fn is_grant_valid(
    grant: &ApprovalGrant,
    expected_h_act: &HAct,
    owner_actor_id: Option<&ActorId>,
    state_epoch: u64,
    t_now: Time,
) -> bool {
    let Some(owner) = owner_actor_id else {
        return false;
    };
    if &grant.signer != owner || grant.signature.trim().is_empty() {
        return false;
    }
    if &grant.h_act != expected_h_act {
        return false;
    }
    if grant.state_epoch != state_epoch {
        return false;
    }
    t_now >= grant.granted_at_ms && t_now <= grant.expires_at_ms
}
