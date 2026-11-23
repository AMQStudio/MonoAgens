//! Pending approvals: the single implementation behind `monoagens approvals list` and the
//! admin method `approvals.list` (the TUI `/approvals` panel). Pending holds are rebuilt by
//! replaying the event log into an [`ApprovalManager`], exactly as the CLI always did.

use monoagens_core::EventRecord;
use monoagens_protocol::{
    Action, ActionCanonicalInput, ActorId, Audience, HAct, OpaquePayload, OriginActor, Taint,
};
use monoagens_valve::{
    ApprovalManager, ValveConfig, ValveMode, canonical_json_bytes, compute_h_act,
};
use serde::{Deserialize, Serialize};

use crate::config::GatewayConfig;

/// One pending approval as listed by the CLI and by `approvals.list`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingApprovalEntry {
    pub action_id: String,
    pub capability_id: String,
    pub places: Vec<String>,
    pub audience: Vec<String>,
    pub taint: String,
    pub state_epoch: u64,
    pub h_act: String,
    pub canonical_input: ActionCanonicalInput,
    pub canonical_payload_utf8: String,
    pub canonical_preimage_hex: String,
    pub reason_code: Option<String>,
    pub timestamp_ms: u64,
    pub expires_at_ms: u64,
    /// Actor that requested the held action (additive, panel only).
    #[serde(default)]
    pub requester: String,
    /// `tui` for local terminal places, otherwise the adapter plugin id derived from the place
    /// (additive, panel only).
    #[serde(default)]
    pub origin: String,
}

fn to_hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        use std::fmt::Write as _;
        let _ = write!(&mut s, "{b:02x}");
    }
    s
}

/// Computes canonical `H_act` and canonical preimage representation for an [`Action`] at `state_epoch`.
#[must_use]
pub fn canonical_h_act_details(
    action: &Action,
    state_epoch: u64,
) -> (HAct, ActionCanonicalInput, Vec<u8>) {
    let h_act = compute_h_act(action, state_epoch);
    let canonical_payload = canonical_json_bytes(action.payload.as_bytes());
    let input = ActionCanonicalInput {
        capability_id: action.capability_id.clone(),
        actor: action.origin_actor.actor_id().clone(),
        places: action.places.clone(),
        state_epoch,
        payload: canonical_payload,
    };
    let preimage = input.canonical_preimage();
    (h_act, input, preimage)
}

fn taint_label(taint: Taint) -> &'static str {
    match taint {
        Taint::Trusted => "trusted",
        Taint::Provisional => "provisional",
        Taint::Untrusted => "untrusted",
    }
}

/// Plugin id (or `tui`) a place belongs to, from the place id prefix.
#[must_use]
pub fn plugin_for_place(place: &str) -> String {
    match place.split(':').next().unwrap_or(place) {
        "cli" | "tui" | "lab" => "tui".to_owned(),
        "wa" => "whatsapp".to_owned(),
        "tg" => "telegram".to_owned(),
        other => other.to_owned(),
    }
}

/// Valve configuration the CLI and the admin methods derive from the gateway config.
#[must_use]
pub fn build_valve_config(cfg: &GatewayConfig) -> ValveConfig {
    ValveConfig {
        mode: match cfg.valve.mode.as_str() {
            "partial" => ValveMode::Partial,
            "off" => ValveMode::Off,
            _ => ValveMode::Enforcing,
        },
        max_chain_wakes: cfg.valve.max_chain_wakes,
        daily_token_budget: cfg.model.daily_token_budget,
        rbac: cfg.rbac.clone(),
        ..ValveConfig::default()
    }
}

/// The owner actor named by `[[person]] is_owner`, or `per_owner`.
#[must_use]
pub fn owner_actor_from_cfg(cfg: Option<&GatewayConfig>) -> ActorId {
    cfg.and_then(|c| c.persons.iter().find(|p| p.is_owner))
        .map(|p| p.actor_id.clone())
        .unwrap_or_else(|| ActorId::new("per_owner"))
}

/// Replays `events` and lists every pending approval.
#[must_use]
pub fn pending_approvals(
    events: &[EventRecord],
    owner_actor: &ActorId,
    valve_cfg: &ValveConfig,
    mac_key: [u8; 32],
) -> Vec<PendingApprovalEntry> {
    let mut manager = ApprovalManager::with_attestation_key(mac_key);
    let replayed_epoch = manager.replay_from_events(events, Some(owner_actor), valve_cfg);

    let mut entries = Vec::new();
    for (aid, pending) in manager.pending() {
        let action = manager
            .pending_action(aid)
            .cloned()
            .unwrap_or_else(|| Action {
                protocol_version: monoagens_protocol::CURRENT_PROTOCOL_VERSION,
                action_id: pending.action_id.clone(),
                capability_id: pending.capability_id.clone(),
                places: pending.places.clone(),
                audience: Audience::new([owner_actor.clone()]),
                origin_actor: OriginActor::new(owner_actor.clone()),
                taint: manager.pending_taint(aid).unwrap_or(Taint::Provisional),
                payload: OpaquePayload::default(),
                correlation_id: None,
                expected_effect: None,
                h_act: Some(pending.h_act.clone()),
                timestamp_ms: pending.requested_at_ms,
            });
        let epoch = pending.state_epoch.max(replayed_epoch);
        let (_computed_h, canonical_input, preimage) = canonical_h_act_details(&action, epoch);
        let taint = manager.pending_taint(aid).unwrap_or(action.taint);
        let audience: Vec<String> = action
            .audience
            .members
            .iter()
            .map(|m| m.as_str().to_owned())
            .collect();
        let places: Vec<String> = pending
            .places
            .iter()
            .map(|p| p.as_str().to_owned())
            .collect();
        let origin = places
            .first()
            .map_or_else(|| "tui".to_owned(), |p| plugin_for_place(p));
        entries.push(PendingApprovalEntry {
            action_id: aid.as_str().to_owned(),
            capability_id: pending.capability_id.as_str().to_owned(),
            places,
            audience,
            taint: taint_label(taint).to_owned(),
            state_epoch: epoch,
            h_act: pending.h_act.as_str().to_owned(),
            canonical_payload_utf8: String::from_utf8_lossy(&canonical_input.payload).into_owned(),
            canonical_input,
            canonical_preimage_hex: to_hex(&preimage),
            reason_code: pending.reason.clone(),
            timestamp_ms: pending.requested_at_ms.as_millis(),
            expires_at_ms: pending.expires_at_ms.as_millis(),
            requester: action.origin_actor.actor_id().as_str().to_owned(),
            origin,
        });
    }
    entries
}
