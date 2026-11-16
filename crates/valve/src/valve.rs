//! Deterministic Cedar `Valve` implementing `monoagens_core::Doors` plus memory-write
//! enforcement, `PendingApproval` + `H_act`, 3-door taint quarantine, and quiet-hours digests.

use std::collections::{BTreeSet, HashSet};

use cedar_policy::Decision;
use monoagens_core::{
    ContextPlan, ContextSlotKind, DoorDecision, Doors, DroppedContextItem, LogSeq,
};
use monoagens_protocol::{
    Action, ActorId, CapabilityFlags, DeclassToken, EventId, OwnerSigned, Percept, PlaceId, Taint,
    Time, VisibleScope,
};

use crate::approvals::{
    ActionRiskMetadata, ApprovalGrant, ApprovalManager, HoldRequestContext, RiskTier,
    classify_action_risk, compute_h_act,
};
use crate::config::{ValveConfig, ValveMode, ValveRuleToggle};
use crate::hooks::{
    ActionDoorPolicyPlugin, SecondaryDoorHook, WideningOverride, reconcile_first_and_last,
};
use crate::policy::{MemoryWritePolicyAttrs, PolicyBundle};
use crate::quiet_hours::{
    DigestBatch, DigestItem, DigestQueue, ProactiveCapTracker, ProactiveEvaluation,
    evaluate_proactive_notification,
};
use crate::reason_tags::{ValveReasonTag, build_valve_self_snapshot};
use crate::roles::compile_attested_roles;
use crate::snapshot::{MemoryWriteRequest, ValveSnapshot};
use crate::taint::{
    TaintActionCheck, check_action_under_taint, combine_taint, evaluate_context_turn_taint,
    is_cross_audience_action, is_memory_write_taint_blocked,
};
use crate::verdict::{ValveAuditLog, ValveDoorKind};

/// Deterministic Gateway policy engine implementing [`monoagens_core::Doors`] and the
/// memory-write door.
pub struct Valve {
    bundle: PolicyBundle,
    config: ValveConfig,
    snapshot: ValveSnapshot,
    approvals: ApprovalManager,
    proactive_tracker: ProactiveCapTracker,
    digest_queue: DigestQueue,
    context_turn_taint: Option<Taint>,
    hooks: Vec<Box<dyn SecondaryDoorHook>>,
    plugins: Vec<Box<dyn ActionDoorPolicyPlugin>>,
    audit_log: ValveAuditLog,
}

impl Valve {
    /// Constructs a `Valve` from a `symcc`-verified [`PolicyBundle`], [`ValveConfig`], and initial [`ValveSnapshot`].
    #[must_use]
    pub fn new(bundle: PolicyBundle, config: ValveConfig, snapshot: ValveSnapshot) -> Self {
        Self {
            bundle,
            config,
            snapshot,
            approvals: ApprovalManager::new(),
            proactive_tracker: ProactiveCapTracker::default(),
            digest_queue: DigestQueue::new(),
            context_turn_taint: None,
            hooks: Vec::new(),
            plugins: Vec::new(),
            audit_log: ValveAuditLog::new(),
        }
    }

    /// Constructs a `Valve` using the compiled-in default Cedar bundle verified via `LocalSolver::cvc5()`.
    pub fn with_default_policies(
        config: ValveConfig,
        snapshot: ValveSnapshot,
    ) -> Result<Self, crate::policy::PolicyError> {
        let bundle = PolicyBundle::default_verified()?;
        Ok(Self::new(bundle, config, snapshot))
    }

    /// Replaces the active policy bundle after verification (fail-closed if caller passes unverified error).
    pub fn reload_policies(&mut self, bundle: PolicyBundle) {
        self.bundle = bundle;
        self.audit_log.record_config_change(
            self.snapshot.log_seq,
            self.snapshot.t_now,
            "valve.policy_reload",
            "verified policy bundle activated",
        );
    }

    /// Updates the pure `(log_seq, t_now)` snapshot evaluated by all four doors.
    pub fn update_snapshot(&mut self, snapshot: ValveSnapshot) {
        self.approvals.prune_at(snapshot.t_now);
        self.context_turn_taint = None;
        self.snapshot = snapshot;
    }

    /// Returns a shared reference to the current [`ValveSnapshot`].
    #[must_use]
    pub fn snapshot(&self) -> &ValveSnapshot {
        &self.snapshot
    }

    /// Returns a mutable reference to the current [`ValveSnapshot`].
    pub fn snapshot_mut(&mut self) -> &mut ValveSnapshot {
        &mut self.snapshot
    }

    /// Returns the active [`PolicyBundle`].
    #[must_use]
    pub fn bundle(&self) -> &PolicyBundle {
        &self.bundle
    }

    /// Returns the current [`ValveConfig`].
    #[must_use]
    pub fn config(&self) -> &ValveConfig {
        &self.config
    }

    /// Returns the effective turn taint `tau_turn` combined from snapshot, inbound, and context.
    #[must_use]
    pub fn effective_turn_taint(&self) -> Taint {
        match self.context_turn_taint {
            Some(ctx) => combine_taint(self.snapshot.turn_taint, ctx),
            None => self.snapshot.turn_taint,
        }
    }

    /// Registers an owner [`ApprovalGrant`] bound to `H_act`, `state_epoch`, and TTL.
    pub fn grant_approval(&mut self, grant: ApprovalGrant) {
        let h_act = grant.h_act.clone();
        self.approvals.register_grant(grant);
        self.audit_log.record_config_change(
            self.snapshot.log_seq,
            self.snapshot.t_now,
            "valve.approval.granted",
            format!("owner approval registered for {}", h_act.as_str()),
        );
    }

    /// Convenience helper computing canonical `H_act` at `snapshot.state_epoch` and registering
    /// an owner [`ApprovalGrant`] (`900 s` chat TTL or `30 s` embodied TTL).
    pub fn approve_action(
        &mut self,
        action: &Action,
        is_embodied: bool,
        signer: ActorId,
        signature: impl Into<String>,
    ) -> ApprovalGrant {
        let h_act = compute_h_act(action, self.snapshot.state_epoch);
        let grant = ApprovalGrant::new(
            h_act,
            self.snapshot.state_epoch,
            self.snapshot.t_now,
            is_embodied,
            signer,
            signature,
        );
        self.grant_approval(grant.clone());
        grant
    }

    /// Returns a shared reference to the [`ApprovalManager`].
    #[must_use]
    pub fn approval_manager(&self) -> &ApprovalManager {
        &self.approvals
    }

    /// Returns a shared reference to the [`DigestQueue`].
    #[must_use]
    pub fn digest_queue(&self) -> &DigestQueue {
        &self.digest_queue
    }

    /// Returns a shared reference to the [`ProactiveCapTracker`].
    #[must_use]
    pub fn proactive_cap_tracker(&self) -> &ProactiveCapTracker {
        &self.proactive_tracker
    }

    /// Drains all due `3x/day` digest batches at `snapshot.t_now` (never using wall clock).
    pub fn drain_due_digests(&mut self) -> Vec<DigestBatch> {
        self.digest_queue.drain_due(self.snapshot.t_now)
    }

    /// Sets the [`ValveMode`] and logs the configuration change.
    pub fn set_mode(&mut self, mode: ValveMode) {
        let prev = self.config.mode;
        self.config.mode = mode;
        self.audit_log.record_config_change(
            self.snapshot.log_seq,
            self.snapshot.t_now,
            "valve.mode",
            format!("mode changed from {} to {}", prev.as_str(), mode.as_str()),
        );
    }

    /// Toggles an individual rule and logs the change in [`ValveAuditLog`].
    pub fn set_rule_enabled(&mut self, rule: ValveRuleToggle, enabled: bool) {
        let prev = self.config.set_rule(rule, enabled);
        self.audit_log.record_config_change(
            self.snapshot.log_seq,
            self.snapshot.t_now,
            format!("valve.toggle.{}", rule.as_str()),
            format!("{} changed from {prev} to {enabled}", rule.as_str()),
        );
    }

    /// Replaces the full [`ValveConfig`] and logs the update.
    pub fn set_config(&mut self, config: ValveConfig) {
        self.config = config;
        self.audit_log.record_config_change(
            self.snapshot.log_seq,
            self.snapshot.t_now,
            "valve.config",
            format!(
                "config updated (mode={})",
                self.config.effective_mode_label()
            ),
        );
    }

    /// Registers a secondary hook that runs between Valve's opening and closing passes.
    pub fn add_secondary_hook(&mut self, hook: Box<dyn SecondaryDoorHook>) {
        self.hooks.push(hook);
    }

    /// Registers an [`ActionDoorPolicyPlugin`] (extension point for risk tiers / quiet hours).
    pub fn add_action_plugin(&mut self, plugin: Box<dyn ActionDoorPolicyPlugin>) {
        self.plugins.push(plugin);
    }

    /// Returns the deterministic [`ValveAuditLog`].
    #[must_use]
    pub fn audit_log(&self) -> &ValveAuditLog {
        &self.audit_log
    }

    /// Returns the active `(log_seq, t_now)` coordinates of the snapshot.
    #[must_use]
    pub fn snapshot_coords(&self) -> (LogSeq, Time) {
        (self.snapshot.log_seq, self.snapshot.t_now)
    }

    /// Evaluates the 4th door (`memory-write`) using first-and-last pass ordering.
    pub fn memory_write(&mut self, req: &mut MemoryWriteRequest) -> DoorDecision {
        let (pass1, pass1_tag, pass1_rule) = self.pass1_memory_write(req);
        let (final_decision, final_tag, overrides) =
            self.run_secondary_hooks(ValveDoorKind::MemoryWrite, pass1, pass1_tag, |hook| {
                hook.on_memory_write(req)
            });
        self.record_overrides(&overrides, req.target_event_id, None);

        let rule_name = decision_rule_name(&final_decision, &pass1_rule);
        let rec = self.audit_log.record_door_decision(
            self.snapshot.log_seq,
            self.snapshot.t_now,
            ValveDoorKind::MemoryWrite,
            rule_name,
            final_tag,
            &final_decision,
        );
        rec.target_event_id = req.target_event_id;
        rec.detail = if overrides.is_empty() {
            "memory_write evaluated".to_owned()
        } else {
            "memory_write evaluated (hook widening overridden)".to_owned()
        };
        final_decision
    }

    fn pass1_inbound(
        &mut self,
        percept: &Percept,
    ) -> (DoorDecision, Option<ValveReasonTag>, String) {
        if self.config.mode == ValveMode::Off || !self.config.any_rule_enabled() {
            return (DoorDecision::Allow, None, "valve.off".to_owned());
        }

        if percept.audience.is_empty() {
            return (
                DoorDecision::Refuse {
                    rule: "valve.inbound.audience".to_owned(),
                    reason: ValveReasonTag::UnknownAudience.as_str().to_owned(),
                },
                Some(ValveReasonTag::UnknownAudience),
                "valve.inbound.audience".to_owned(),
            );
        }

        if self.config.is_rule_active(ValveRuleToggle::ChainBrake) {
            let chain_wakes = percept
                .reply_to
                .and_then(|id| self.snapshot.chain_wake_counts.get(&id).copied())
                .unwrap_or(self.snapshot.current_wake_hint.chain_wakes);
            if chain_wakes >= self.config.max_chain_wakes {
                return (
                    DoorDecision::Refuse {
                        rule: "valve.inbound.chain_brake".to_owned(),
                        reason: ValveReasonTag::ChainBrake.as_str().to_owned(),
                    },
                    Some(ValveReasonTag::ChainBrake),
                    "valve.inbound.chain_brake".to_owned(),
                );
            }
        }

        if self.config.is_rule_active(ValveRuleToggle::CedarAbac) {
            let roles = compile_attested_roles(
                &percept.actor,
                Some(&percept.place),
                &self.snapshot,
                &self.config,
            );
            let decision = self.bundle.eval_inbound(
                &percept.actor,
                roles,
                &percept.place,
                !percept.audience.is_empty(),
            );
            if decision == Decision::Deny {
                let tag = if !roles.known_actor && !roles.room_present && !roles.physically_present
                {
                    ValveReasonTag::UnknownAudience
                } else {
                    ValveReasonTag::CedarDenied
                };
                return (
                    DoorDecision::Refuse {
                        rule: "valve.inbound.cedar".to_owned(),
                        reason: tag.as_str().to_owned(),
                    },
                    Some(tag),
                    "valve.inbound.cedar".to_owned(),
                );
            }
        }

        // Propagate monotonic taint from admitted inbound percept across the 3-door boundary.
        let prev = self.effective_turn_taint();
        self.context_turn_taint = Some(combine_taint(prev, percept.taint));

        (DoorDecision::Allow, None, "valve.inbound.allow".to_owned())
    }

    fn pass1_context(
        &mut self,
        plan: &mut ContextPlan,
    ) -> (DoorDecision, Option<ValveReasonTag>, String) {
        // Compute turn taint `tau_turn` across context items and update live snapshot.
        let computed_taint = evaluate_context_turn_taint(self.effective_turn_taint(), plan);
        self.context_turn_taint = Some(computed_taint);

        // Always populate live typed ValveSelfSnapshot so [SELF_GUIDE] stays synchronized.
        plan.valve_self_snapshot = Some(build_valve_self_snapshot(
            &self.config,
            &self.snapshot,
            &self.approvals,
            &self.proactive_tracker,
        ));
        plan.refresh_static_prefix_digest();

        if self.config.mode == ValveMode::Off || !self.config.any_rule_enabled() {
            return (DoorDecision::Allow, None, "valve.off".to_owned());
        }

        if self.config.is_rule_active(ValveRuleToggle::ChainBrake) {
            let chain_wakes = plan
                .trigger_event_id
                .and_then(|id| self.snapshot.chain_wake_counts.get(&id).copied())
                .unwrap_or(self.snapshot.current_wake_hint.chain_wakes);
            if chain_wakes >= self.config.max_chain_wakes {
                return (
                    DoorDecision::Refuse {
                        rule: "valve.context.chain_brake".to_owned(),
                        reason: ValveReasonTag::ChainBrake.as_str().to_owned(),
                    },
                    Some(ValveReasonTag::ChainBrake),
                    "valve.context.chain_brake".to_owned(),
                );
            }
        }

        if self.config.is_rule_active(ValveRuleToggle::BudgetGuard)
            && self.snapshot.daily_tokens_used >= self.config.daily_token_budget
        {
            let hint = self.snapshot.current_wake_hint;
            let single_shot_exempt = hint.is_single_shot_promise && !hint.is_recurrence_monitor;
            if !single_shot_exempt {
                return (
                    DoorDecision::Refuse {
                        rule: "valve.context.budget_guard".to_owned(),
                        reason: ValveReasonTag::BudgetExhausted.as_str().to_owned(),
                    },
                    Some(ValveReasonTag::BudgetExhausted),
                    "valve.context.budget_guard".to_owned(),
                );
            }
        }

        if self.config.is_rule_active(ValveRuleToggle::CedarAbac) {
            let actor = self
                .snapshot
                .owner_actor_id
                .clone()
                .unwrap_or_else(|| ActorId::new("soul"));
            let roles =
                compile_attested_roles(&actor, plan.place.as_ref(), &self.snapshot, &self.config);
            let decision = self.bundle.eval_context(
                &actor,
                roles,
                plan.place.as_ref(),
                plan.visible_scope.is_some(),
            );
            if decision == Decision::Deny {
                return (
                    DoorDecision::Refuse {
                        rule: "valve.context.cedar".to_owned(),
                        reason: ValveReasonTag::Line2ScopeStripped.as_str().to_owned(),
                    },
                    Some(ValveReasonTag::Line2ScopeStripped),
                    "valve.context.cedar".to_owned(),
                );
            }
        }

        if self.config.is_rule_active(ValveRuleToggle::Line2Scope) {
            let removed_ids = self.collect_line2_disallowed_ids(plan);
            if !removed_ids.is_empty() {
                apply_context_strip(
                    plan,
                    &removed_ids,
                    ValveReasonTag::Line2ScopeStripped.as_str(),
                );
                return (
                    DoorDecision::Strip {
                        removed_ids,
                        rule: "valve.context.line2_scope".to_owned(),
                    },
                    Some(ValveReasonTag::Line2ScopeStripped),
                    "valve.context.line2_scope".to_owned(),
                );
            }
        }

        (DoorDecision::Allow, None, "valve.context.allow".to_owned())
    }

    fn pass1_action(
        &mut self,
        action: &mut Action,
    ) -> (DoorDecision, Option<ValveReasonTag>, String) {
        if self.config.mode == ValveMode::Off || !self.config.any_rule_enabled() {
            return (DoorDecision::Allow, None, "valve.off".to_owned());
        }

        let hint = self
            .snapshot
            .action_hints
            .get(&action.action_id)
            .copied()
            .unwrap_or(self.snapshot.current_wake_hint);

        if self.config.is_rule_active(ValveRuleToggle::ChainBrake)
            && hint.chain_wakes >= self.config.max_chain_wakes
        {
            return (
                DoorDecision::Refuse {
                    rule: "valve.action.chain_brake".to_owned(),
                    reason: ValveReasonTag::ChainBrake.as_str().to_owned(),
                },
                Some(ValveReasonTag::ChainBrake),
                "valve.action.chain_brake".to_owned(),
            );
        }

        if self.config.is_rule_active(ValveRuleToggle::BudgetGuard)
            && self.snapshot.daily_tokens_used >= self.config.daily_token_budget
        {
            let single_shot_exempt = hint.is_single_shot_promise && !hint.is_recurrence_monitor;
            if !single_shot_exempt {
                return (
                    DoorDecision::Refuse {
                        rule: "valve.action.budget_guard".to_owned(),
                        reason: ValveReasonTag::BudgetExhausted.as_str().to_owned(),
                    },
                    Some(ValveReasonTag::BudgetExhausted),
                    "valve.action.budget_guard".to_owned(),
                );
            }
        }

        let flags = self
            .snapshot
            .capabilities
            .get(&action.capability_id)
            .map_or_else(
                || infer_capability_flags(action.capability_id.as_str()),
                |desc| desc.flags,
            );
        let actor = action.origin_actor.actor_id().clone();
        let roles =
            compile_attested_roles(&actor, action.places.first(), &self.snapshot, &self.config);

        // Absolute Line 1 / Cedar ABAC check (non-owner SelfModify / identity_changing always refused).
        if self.config.is_rule_active(ValveRuleToggle::CedarAbac) {
            let decision = self
                .bundle
                .eval_action(&actor, roles, &action.capability_id, flags);
            if decision == Decision::Deny {
                let tag = if flags.identity_changing && !roles.is_owner {
                    ValveReasonTag::NonOwnerIdentityChange
                } else {
                    ValveReasonTag::CedarDenied
                };
                return (
                    DoorDecision::Refuse {
                        rule: "valve.action.cedar".to_owned(),
                        reason: tag.as_str().to_owned(),
                    },
                    Some(tag),
                    "valve.action.cedar".to_owned(),
                );
            }
        }

        // V2: 6.1 & 6.2 — Risk classification, H_act / TOCTOU / TTL / epoch, and 3-door taint quarantine.
        let risk_meta = self.resolve_risk_metadata(action);
        let is_embodied = risk_meta.is_embodied
            || action
                .places
                .iter()
                .any(|p| self.snapshot.embodied_places.contains(p));
        let effective_taint = combine_taint(action.taint, self.effective_turn_taint());
        let cross_audience =
            is_cross_audience_action(action, self.snapshot.active_context_audience.as_ref());
        let has_valid_declass = cross_audience && self.has_action_declass_token(action);
        let unauthorized_cross_audience = cross_audience && !has_valid_declass;

        let tier = classify_action_risk(
            flags,
            roles.is_owner,
            unauthorized_cross_audience,
            effective_taint,
            risk_meta,
        );
        let computed_h_act = compute_h_act(action, self.snapshot.state_epoch);

        let has_valid_grant = if self.config.is_rule_active(ValveRuleToggle::PendingApproval) {
            match self.approvals.check_approval_or_toctou(
                action,
                &computed_h_act,
                self.snapshot.owner_actor_id.as_ref(),
                self.snapshot.state_epoch,
                self.snapshot.t_now,
            ) {
                Ok(valid) => valid,
                Err(tag) => {
                    return (
                        DoorDecision::Refuse {
                            rule: "valve.approval.toctou".to_owned(),
                            reason: tag.as_str().to_owned(),
                        },
                        Some(tag),
                        "valve.approval.toctou".to_owned(),
                    );
                }
            }
        } else {
            false
        };

        if self.config.is_rule_active(ValveRuleToggle::TaintBlocking) {
            match check_action_under_taint(
                effective_taint,
                tier,
                risk_meta.is_procedure_write,
                flags.identity_changing,
                cross_audience,
                has_valid_declass,
                has_valid_grant,
            ) {
                TaintActionCheck::Allow => {}
                TaintActionCheck::RefuseCrossAudience => {
                    return (
                        DoorDecision::Refuse {
                            rule: "valve.taint.cross_audience".to_owned(),
                            reason: ValveReasonTag::TaintBlocked.as_str().to_owned(),
                        },
                        Some(ValveReasonTag::TaintBlocked),
                        "valve.taint.cross_audience".to_owned(),
                    );
                }
                TaintActionCheck::RequireApproval => {
                    if self.config.is_rule_active(ValveRuleToggle::PendingApproval) {
                        let (dec, tag) = self.approvals.try_hold(
                            action,
                            computed_h_act,
                            HoldRequestContext {
                                state_epoch: self.snapshot.state_epoch,
                                t_now: self.snapshot.t_now,
                                is_embodied,
                                reason_tag: ValveReasonTag::TaintBlocked,
                            },
                            &self.config,
                        );
                        let rule = decision_rule_name(&dec, "valve.taint.quarantine");
                        return (dec, Some(tag), rule);
                    }
                    return (
                        DoorDecision::Refuse {
                            rule: "valve.taint.quarantine".to_owned(),
                            reason: ValveReasonTag::TaintBlocked.as_str().to_owned(),
                        },
                        Some(ValveReasonTag::TaintBlocked),
                        "valve.taint.quarantine".to_owned(),
                    );
                }
            }
        } else if unauthorized_cross_audience {
            return (
                DoorDecision::Refuse {
                    rule: "valve.action.cross_audience".to_owned(),
                    reason: ValveReasonTag::Line2ScopeStripped.as_str().to_owned(),
                },
                Some(ValveReasonTag::Line2ScopeStripped),
                "valve.action.cross_audience".to_owned(),
            );
        }

        if self.config.is_rule_active(ValveRuleToggle::PendingApproval) && tier >= RiskTier::R2 {
            if has_valid_grant {
                self.approvals.consume_grant(&computed_h_act);
            } else if tier == RiskTier::R3 && flags.identity_changing && !roles.is_owner {
                return (
                    DoorDecision::Refuse {
                        rule: "valve.action.line1".to_owned(),
                        reason: ValveReasonTag::NonOwnerIdentityChange.as_str().to_owned(),
                    },
                    Some(ValveReasonTag::NonOwnerIdentityChange),
                    "valve.action.line1".to_owned(),
                );
            } else {
                let (dec, tag) = self.approvals.try_hold(
                    action,
                    computed_h_act,
                    HoldRequestContext {
                        state_epoch: self.snapshot.state_epoch,
                        t_now: self.snapshot.t_now,
                        is_embodied,
                        reason_tag: ValveReasonTag::PendingApproval,
                    },
                    &self.config,
                );
                let rule = decision_rule_name(&dec, "valve.approval.pending");
                return (dec, Some(tag), rule);
            }
        } else if has_valid_grant {
            self.approvals.consume_grant(&computed_h_act);
        }

        // V2: 6.3 — Recipient-local quiet hours, proactive daily cap C_daily, emergency cutoffs, 3x/day digests.
        if hint.is_proactive_notification
            && (self.config.is_rule_active(ValveRuleToggle::QuietHours)
                || self.config.is_rule_active(ValveRuleToggle::ProactiveCap))
        {
            let recipient = self
                .snapshot
                .active_recipient
                .clone()
                .or_else(|| action.audience.members.iter().next().cloned())
                .unwrap_or_else(|| actor.clone());
            let tz_name = self
                .snapshot
                .recipient_timezones
                .get(&recipient)
                .map_or("UTC", String::as_str);
            let urgency = self
                .snapshot
                .action_urgencies
                .get(&action.action_id)
                .copied()
                .or(self.snapshot.current_urgency);

            match evaluate_proactive_notification(
                &recipient,
                tz_name,
                self.snapshot.t_now,
                urgency,
                &self.proactive_tracker,
                self.snapshot.proactive_sent_today,
                &self.config,
            ) {
                ProactiveEvaluation::AllowImmediate {
                    emergency_override,
                    local_date,
                } => {
                    self.proactive_tracker.record_sent(&recipient, local_date);
                    if emergency_override {
                        self.audit_log.record_config_change(
                            self.snapshot.log_seq,
                            self.snapshot.t_now,
                            "valve.quiet_hours.emergency_override",
                            format!(
                                "proactive notification {} bypassed quiet_hours/C_daily via emergency urgency",
                                action.action_id
                            ),
                        );
                    }
                }
                ProactiveEvaluation::HoldDigest {
                    reason,
                    scheduled_at_ms,
                } => {
                    let place = action
                        .places
                        .first()
                        .cloned()
                        .unwrap_or_else(|| PlaceId::new("digest:default"));
                    let inserted = self.digest_queue.enqueue_once(DigestItem {
                        action_id: action.action_id.clone(),
                        recipient,
                        place,
                        capability_id: action.capability_id.clone(),
                        payload: action.payload.clone(),
                        queued_at_ms: self.snapshot.t_now,
                        scheduled_at_ms,
                        reason,
                    });
                    if !inserted {
                        return (
                            DoorDecision::Refuse {
                                rule: "valve.quiet_hours.duplicate_digest".to_owned(),
                                reason: reason.as_str().to_owned(),
                            },
                            Some(reason),
                            "valve.quiet_hours.duplicate_digest".to_owned(),
                        );
                    }
                    return (
                        DoorDecision::Hold {
                            rule: "valve.quiet_hours.digest".to_owned(),
                            reason: reason.as_str().to_owned(),
                        },
                        Some(reason),
                        "valve.quiet_hours.digest".to_owned(),
                    );
                }
                ProactiveEvaluation::RefuseInvalidTz => {
                    return (
                        DoorDecision::Refuse {
                            rule: "valve.quiet_hours.invalid_tz".to_owned(),
                            reason: ValveReasonTag::CedarDenied.as_str().to_owned(),
                        },
                        Some(ValveReasonTag::CedarDenied),
                        "valve.quiet_hours.invalid_tz".to_owned(),
                    );
                }
            }
        }

        // Evaluate pluggable action-door policies.
        let mut current_decision = DoorDecision::Allow;
        let mut current_tag = None;
        let mut current_rule = "valve.action.allow".to_owned();
        for plugin in &mut self.plugins {
            if let Some((plugin_decision, plugin_tag)) =
                plugin.evaluate_action(action, &self.snapshot, &self.config)
            {
                let (merged, merged_tag, _) = reconcile_first_and_last(
                    ValveDoorKind::Action,
                    current_decision,
                    current_tag,
                    "valve.action.plugin",
                    plugin_decision,
                );
                current_decision = merged;
                current_tag = Some(plugin_tag).or(merged_tag);
                current_rule = decision_rule_name(&current_decision, &current_rule);
            }
        }

        (current_decision, current_tag, current_rule)
    }

    fn pass1_memory_write(
        &mut self,
        req: &MemoryWriteRequest,
    ) -> (DoorDecision, Option<ValveReasonTag>, String) {
        if self.config.mode == ValveMode::Off || !self.config.any_rule_enabled() {
            return (DoorDecision::Allow, None, "valve.off".to_owned());
        }

        if self
            .config
            .is_rule_active(ValveRuleToggle::MemoryWriteGuard)
            && req.is_procedure
            && (combine_taint(req.taint, self.effective_turn_taint()) < Taint::Trusted
                || !req.owner_signed
                || !self.snapshot.is_owner(req.origin_actor.actor_id()))
        {
            return (
                DoorDecision::Refuse {
                    rule: "valve.memory_write.procedure_guard".to_owned(),
                    reason: ValveReasonTag::TaintedProcedureWrite.as_str().to_owned(),
                },
                Some(ValveReasonTag::TaintedProcedureWrite),
                "valve.memory_write.procedure_guard".to_owned(),
            );
        }

        if self.config.is_rule_active(ValveRuleToggle::TaintBlocking)
            && is_memory_write_taint_blocked(req, &self.snapshot)
        {
            let tag = if req.is_procedure {
                ValveReasonTag::TaintedProcedureWrite
            } else {
                ValveReasonTag::TaintBlocked
            };
            return (
                DoorDecision::Refuse {
                    rule: "valve.memory_write.taint_quarantine".to_owned(),
                    reason: tag.as_str().to_owned(),
                },
                Some(tag),
                "valve.memory_write.taint_quarantine".to_owned(),
            );
        }

        if self.config.is_rule_active(ValveRuleToggle::CedarAbac) {
            let roles = compile_attested_roles(
                &req.writer,
                req.places.first(),
                &self.snapshot,
                &self.config,
            );
            let effective = combine_taint(req.taint, self.effective_turn_taint());
            let tainted = effective < Taint::Trusted;
            let trusted = effective == Taint::Trusted;
            let decision = self.bundle.eval_memory_write(
                &req.writer,
                roles,
                MemoryWritePolicyAttrs {
                    item_id: "mem_write",
                    is_procedure: req.is_procedure,
                    owner_signed: req.owner_signed,
                    trusted,
                    tainted,
                },
            );
            if decision == Decision::Deny {
                let tag = if req.is_procedure {
                    ValveReasonTag::TaintedProcedureWrite
                } else {
                    ValveReasonTag::CedarDenied
                };
                return (
                    DoorDecision::Refuse {
                        rule: "valve.memory_write.cedar".to_owned(),
                        reason: tag.as_str().to_owned(),
                    },
                    Some(tag),
                    "valve.memory_write.cedar".to_owned(),
                );
            }
        }

        let mut current_decision = DoorDecision::Allow;
        let mut current_tag = None;
        let mut current_rule = "valve.memory_write.allow".to_owned();
        for plugin in &mut self.plugins {
            if let Some((plugin_decision, plugin_tag)) =
                plugin.evaluate_memory_write(req, &self.snapshot, &self.config)
            {
                let (merged, merged_tag, _) = reconcile_first_and_last(
                    ValveDoorKind::MemoryWrite,
                    current_decision,
                    current_tag,
                    "valve.memory_write.plugin",
                    plugin_decision,
                );
                current_decision = merged;
                current_tag = Some(plugin_tag).or(merged_tag);
                current_rule = decision_rule_name(&current_decision, &current_rule);
            }
        }

        (current_decision, current_tag, current_rule)
    }

    fn resolve_risk_metadata(&self, action: &Action) -> ActionRiskMetadata {
        if let Some(meta) = self.snapshot.action_risk_metadata.get(&action.action_id) {
            return *meta;
        }
        if let Some(meta) = self
            .snapshot
            .capability_risk_metadata
            .get(&action.capability_id)
        {
            return *meta;
        }
        let lower = action.capability_id.as_str().to_ascii_lowercase();
        ActionRiskMetadata {
            is_read_only: lower.starts_with("memory.recall") || lower.ends_with(".read"),
            is_financial_or_credential: lower.contains("pay")
                || lower.contains("transfer")
                || lower.contains("secret")
                || lower.contains("cred"),
            is_procedure_write: lower.contains("procedure"),
            is_declass_grant: lower.contains("declass"),
            is_embodied: lower.contains("actuator") || lower.contains("ros2"),
        }
    }

    fn has_action_declass_token(&self, action: &Action) -> bool {
        let Some(owner) = self.snapshot.owner_actor_id.as_ref() else {
            return false;
        };
        let target_place = action.places.first();
        self.snapshot.logged_declass_tokens.iter().any(|signed| {
            is_valid_logged_owner_token(signed, owner, self.snapshot.t_now)
                && signed
                    .payload
                    .target_place
                    .as_ref()
                    .is_none_or(|p| Some(p) == target_place)
                && !action.audience.is_empty()
                && action
                    .audience
                    .is_subset_of(&signed.payload.target_audience)
        })
    }

    fn collect_line2_disallowed_ids(&self, plan: &ContextPlan) -> Vec<EventId> {
        let mut candidate_ids = BTreeSet::new();
        for &id in &plan.items {
            candidate_ids.insert(id);
        }
        for seg in &plan.sections.recalled {
            if let Some(id) = seg.event_id {
                candidate_ids.insert(id);
            }
        }
        for seg in &plan.sections.working_set {
            if let Some(id) = seg.event_id {
                candidate_ids.insert(id);
            }
        }
        for seg in &plan.sections.place_tail {
            if let Some(id) = seg.event_id {
                candidate_ids.insert(id);
            }
        }

        if candidate_ids.is_empty() {
            return Vec::new();
        }

        let Some(scope) = &plan.visible_scope else {
            return candidate_ids.into_iter().collect();
        };

        let context_place = plan.place.as_ref().or(scope.target_place.as_ref());
        let mut disallowed = Vec::new();

        for id in candidate_ids {
            let allowed_by_audience =
                self.snapshot
                    .item_audiences
                    .get(&id)
                    .is_some_and(|item_aud| {
                        !scope.audience.is_empty()
                            && !item_aud.is_empty()
                            && scope.audience.is_subset_of(item_aud)
                    });

            if allowed_by_audience {
                continue;
            }

            let allowed_by_declass = self.has_valid_logged_declass_token(id, scope, context_place);
            if !allowed_by_declass {
                disallowed.push(id);
            }
        }

        disallowed
    }

    fn has_valid_logged_declass_token(
        &self,
        item_id: EventId,
        scope: &VisibleScope,
        context_place: Option<&PlaceId>,
    ) -> bool {
        let Some(owner) = self.snapshot.owner_actor_id.as_ref() else {
            return false;
        };

        self.snapshot.logged_declass_tokens.iter().any(|signed| {
            is_valid_logged_owner_token(signed, owner, self.snapshot.t_now)
                && signed.payload.authorizes(
                    item_id,
                    &scope.audience,
                    context_place,
                    self.snapshot.t_now,
                )
        })
    }

    fn run_secondary_hooks<F>(
        &mut self,
        door: ValveDoorKind,
        pass1: DoorDecision,
        pass1_tag: Option<ValveReasonTag>,
        mut call_hook: F,
    ) -> (DoorDecision, Option<ValveReasonTag>, Vec<WideningOverride>)
    where
        F: FnMut(&mut dyn SecondaryDoorHook) -> DoorDecision,
    {
        let mut current = pass1;
        let mut current_tag = pass1_tag;
        let mut overrides = Vec::new();

        for hook in &mut self.hooks {
            let hook_name = hook.name().to_owned();
            let hook_decision = call_hook(hook.as_mut());
            let (reconciled, reconciled_tag, ov) =
                reconcile_first_and_last(door, current, current_tag, &hook_name, hook_decision);
            current = reconciled;
            current_tag = reconciled_tag;
            if let Some(o) = ov {
                overrides.push(o);
            }
        }

        (current, current_tag, overrides)
    }

    fn record_overrides(
        &mut self,
        overrides: &[WideningOverride],
        target_event_id: Option<EventId>,
        target_action_id: Option<monoagens_protocol::ActionId>,
    ) {
        for ov in overrides {
            let synthetic = DoorDecision::Refuse {
                rule: format!("valve.closing_override.{}", ov.hook_name),
                reason: ValveReasonTag::HookWideningOverridden.as_str().to_owned(),
            };
            let rec = self.audit_log.record_door_decision(
                self.snapshot.log_seq,
                self.snapshot.t_now,
                ov.door,
                format!("valve.closing_override.{}", ov.hook_name),
                Some(ValveReasonTag::HookWideningOverridden),
                &synthetic,
            );
            rec.target_event_id = target_event_id;
            rec.target_action_id = target_action_id.clone();
            rec.detail = format!(
                "hook '{}' attempted to widen '{}' to '{}'",
                ov.hook_name, ov.enforced, ov.attempted
            );
        }
    }
}

impl Doors for Valve {
    fn inbound(&mut self, percept: &mut Percept) -> DoorDecision {
        let (pass1, pass1_tag, pass1_rule) = self.pass1_inbound(percept);
        let (final_decision, final_tag, overrides) =
            self.run_secondary_hooks(ValveDoorKind::Inbound, pass1, pass1_tag, |hook| {
                hook.on_inbound(percept)
            });
        self.record_overrides(&overrides, percept.reply_to, None);

        let rule_name = decision_rule_name(&final_decision, &pass1_rule);
        let rec = self.audit_log.record_door_decision(
            self.snapshot.log_seq,
            self.snapshot.t_now,
            ValveDoorKind::Inbound,
            rule_name,
            final_tag,
            &final_decision,
        );
        rec.target_event_id = percept.reply_to;
        rec.detail = if overrides.is_empty() {
            "inbound evaluated".to_owned()
        } else {
            "inbound evaluated (hook widening overridden)".to_owned()
        };
        final_decision
    }

    fn context(&mut self, plan: &mut ContextPlan) -> DoorDecision {
        let (pass1, pass1_tag, pass1_rule) = self.pass1_context(plan);
        let (final_decision, final_tag, overrides) =
            self.run_secondary_hooks(ValveDoorKind::Context, pass1, pass1_tag, |hook| {
                hook.on_context(plan)
            });
        self.record_overrides(&overrides, plan.trigger_event_id, None);

        if let DoorDecision::Strip { removed_ids, .. } = &final_decision {
            apply_context_strip(
                plan,
                removed_ids,
                final_tag.map_or("context_stripped", ValveReasonTag::as_str),
            );
        }

        let rule_name = decision_rule_name(&final_decision, &pass1_rule);
        let rec = self.audit_log.record_door_decision(
            self.snapshot.log_seq,
            self.snapshot.t_now,
            ValveDoorKind::Context,
            rule_name,
            final_tag,
            &final_decision,
        );
        rec.target_event_id = plan.trigger_event_id;
        rec.detail = if overrides.is_empty() {
            "context evaluated".to_owned()
        } else {
            "context evaluated (hook widening overridden)".to_owned()
        };
        final_decision
    }

    fn action(&mut self, action: &mut Action) -> DoorDecision {
        let (pass1, pass1_tag, pass1_rule) = self.pass1_action(action);
        let (final_decision, final_tag, overrides) =
            self.run_secondary_hooks(ValveDoorKind::Action, pass1, pass1_tag, |hook| {
                hook.on_action(action)
            });
        self.record_overrides(&overrides, None, Some(action.action_id.clone()));

        let rule_name = decision_rule_name(&final_decision, &pass1_rule);
        let rec = self.audit_log.record_door_decision(
            self.snapshot.log_seq,
            self.snapshot.t_now,
            ValveDoorKind::Action,
            rule_name,
            final_tag,
            &final_decision,
        );
        rec.target_action_id = Some(action.action_id.clone());
        rec.detail = if overrides.is_empty() {
            "action evaluated".to_owned()
        } else {
            "action evaluated (hook widening overridden)".to_owned()
        };
        final_decision
    }
}

fn is_valid_logged_owner_token(
    signed: &OwnerSigned<DeclassToken>,
    owner: &ActorId,
    now: Time,
) -> bool {
    if signed.signature.trim().is_empty() || signed.payload.signature.trim().is_empty() {
        return false;
    }
    if &signed.signer != owner || &signed.payload.issuer != owner {
        return false;
    }
    if signed.payload.token_id.trim().is_empty() || signed.payload.scope.is_empty() {
        return false;
    }
    now <= signed.payload.expiry
}

fn apply_context_strip(plan: &mut ContextPlan, removed_ids: &[EventId], reason: &str) {
    if removed_ids.is_empty() {
        return;
    }
    let removed_set: BTreeSet<EventId> = removed_ids.iter().copied().collect();
    plan.items.retain(|id| !removed_set.contains(id));

    let mut existing_dropped: HashSet<(ContextSlotKind, Option<EventId>)> = plan
        .dropped_items
        .iter()
        .map(|d| (d.slot, d.event_id))
        .collect();

    plan.sections.recalled.retain(|seg| {
        let keep = seg.event_id.is_none_or(|id| !removed_set.contains(&id));
        if !keep && existing_dropped.insert((ContextSlotKind::Recalled, seg.event_id)) {
            plan.dropped_items.push(DroppedContextItem {
                slot: ContextSlotKind::Recalled,
                event_id: seg.event_id,
                estimated_bytes: seg.estimated_bytes,
                reason: reason.to_owned(),
            });
        }
        keep
    });
    plan.sections.working_set.retain(|seg| {
        let keep = seg.event_id.is_none_or(|id| !removed_set.contains(&id));
        if !keep && existing_dropped.insert((ContextSlotKind::WorkingSet, seg.event_id)) {
            plan.dropped_items.push(DroppedContextItem {
                slot: ContextSlotKind::WorkingSet,
                event_id: seg.event_id,
                estimated_bytes: seg.estimated_bytes,
                reason: reason.to_owned(),
            });
        }
        keep
    });
    plan.sections.place_tail.retain(|seg| {
        let keep = seg.event_id.is_none_or(|id| !removed_set.contains(&id));
        if !keep && existing_dropped.insert((ContextSlotKind::PlaceTail, seg.event_id)) {
            plan.dropped_items.push(DroppedContextItem {
                slot: ContextSlotKind::PlaceTail,
                event_id: seg.event_id,
                estimated_bytes: seg.estimated_bytes,
                reason: reason.to_owned(),
            });
        }
        keep
    });
}

fn infer_capability_flags(cap_id: &str) -> CapabilityFlags {
    let lower = cap_id.to_ascii_lowercase();
    let identity_changing = lower.contains("self_modify")
        || lower.contains("self.modify")
        || lower.contains("identity");
    CapabilityFlags {
        communicative: lower.contains("speak") || lower.contains("chat") || lower.contains("send"),
        identity_changing,
        irreversible: identity_changing
            || lower.contains("delete")
            || lower.contains("transfer")
            || lower.contains("pay")
            || lower.contains("write"),
    }
}

fn decision_rule_name(decision: &DoorDecision, fallback: &str) -> String {
    match decision {
        DoorDecision::Allow => fallback.to_owned(),
        DoorDecision::Strip { rule, .. }
        | DoorDecision::Hold { rule, .. }
        | DoorDecision::Refuse { rule, .. } => rule.clone(),
    }
}
