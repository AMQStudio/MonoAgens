//! Context-door cost and chain-wake guard for Lab runs.

use std::collections::BTreeMap;

use monoagens_core::{ContextPlan, DoorDecision, Doors};
use monoagens_protocol::{Action, EventId, Percept};
use serde::{Deserialize, Serialize};

use crate::scenario::CostGuardConfig;

/// Recorded refusal emitted by [`CostGuard`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuardRefusal {
    pub rule: String,
    pub reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger_event_id: Option<EventId>,
}

/// Door wrapper enforcing token and per-chain wake ceilings at `Doors::context`.
#[derive(Debug, Clone, Default)]
pub struct CostGuard {
    pub max_tokens: Option<u64>,
    pub max_chain_wakes: Option<u64>,
    pub tokens_used: u64,
    pub chain_wakes: BTreeMap<EventId, u64>,
    pub refusals: Vec<GuardRefusal>,
    pub captured_plans: Vec<ContextPlan>,
}

impl CostGuard {
    /// Builds a [`CostGuard`] from optional scenario guard limits.
    #[must_use]
    pub fn from_config(config: Option<&CostGuardConfig>) -> Self {
        Self {
            max_tokens: config.and_then(|c| c.max_tokens),
            max_chain_wakes: config.and_then(|c| c.max_chain_wakes),
            ..Self::default()
        }
    }

    /// Returns active door rule descriptors for the run record.
    #[must_use]
    pub fn rule_names(&self) -> Vec<String> {
        let mut rules = Vec::new();
        if let Some(limit) = self.max_tokens {
            rules.push(format!("cost_guard.max_tokens={limit}"));
        }
        if let Some(limit) = self.max_chain_wakes {
            rules.push(format!("cost_guard.max_chain_wakes={limit}"));
        }
        rules
    }

    /// Charges token usage after a model call completes.
    pub fn record_tokens(&mut self, tokens: u64) {
        self.tokens_used = self.tokens_used.saturating_add(tokens);
    }
}

impl Doors for CostGuard {
    fn inbound(&mut self, _percept: &mut Percept) -> DoorDecision {
        DoorDecision::Allow
    }

    fn context(&mut self, plan: &mut ContextPlan) -> DoorDecision {
        if let Some(limit) = self.max_tokens
            && self.tokens_used >= limit
        {
            let reason = format!("run token limit reached ({}/{limit})", self.tokens_used);
            self.refusals.push(GuardRefusal {
                rule: "cost_guard.token_limit".to_owned(),
                reason: reason.clone(),
                trigger_event_id: plan.trigger_event_id,
            });
            return DoorDecision::Refuse {
                rule: "cost_guard.token_limit".to_owned(),
                reason,
            };
        }

        if let Some(limit) = self.max_chain_wakes
            && let Some(trigger_id) = plan.trigger_event_id
        {
            let count = self.chain_wakes.entry(trigger_id).or_insert(0);
            if *count >= limit {
                let reason =
                    format!("chain wake limit reached for event {trigger_id} ({count}/{limit})");
                self.refusals.push(GuardRefusal {
                    rule: "cost_guard.chain_wake_limit".to_owned(),
                    reason: reason.clone(),
                    trigger_event_id: Some(trigger_id),
                });
                return DoorDecision::Refuse {
                    rule: "cost_guard.chain_wake_limit".to_owned(),
                    reason,
                };
            }
            *count = count.saturating_add(1);
        }

        self.captured_plans.push(plan.clone());
        DoorDecision::Allow
    }

    fn action(&mut self, _action: &mut Action) -> DoorDecision {
        DoorDecision::Allow
    }
}
