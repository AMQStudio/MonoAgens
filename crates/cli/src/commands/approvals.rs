use std::io::Write;
use std::path::Path;

use monoagens_core::{EventKind, EventRecord, Store};
use monoagens_gateway::{
    Clock, DoorController, SystemClock, ValveDoorController, load_approval_secrets,
    resolve_approval_mac_key,
};
use monoagens_protocol::{
    Action, ActionCanonicalInput, ActionId, ActorId, ApprovalResolveParams, Audience, HAct,
    OpaquePayload, OriginActor, PlaceId, ReceiptOutcome, Taint, Waist2Headers,
};
use monoagens_valve::{Valve, ValveConfig, ValveSnapshot, canonical_json_bytes, compute_h_act};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::common::{
    build_valve_config, load_paths, open_store_readonly, owner_actor_from_cfg, write_json_pretty,
};
use crate::{ApprovalsArgs, ApprovalsSubcommand};

#[derive(Debug, Serialize, Deserialize)]
struct PendingApprovalEntry {
    action_id: String,
    capability_id: String,
    places: Vec<String>,
    audience: Vec<String>,
    taint: String,
    state_epoch: u64,
    h_act: String,
    canonical_input: ActionCanonicalInput,
    canonical_payload_utf8: String,
    canonical_preimage_hex: String,
    reason_code: Option<String>,
    timestamp_ms: u64,
    expires_at_ms: u64,
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

#[allow(clippy::too_many_arguments)]
fn resolve_cli_approval(
    data_dir: &Path,
    owner_actor: &ActorId,
    valve_cfg: &ValveConfig,
    mac_key: [u8; 32],
    action_id: &str,
    approved: bool,
    json_mode: bool,
    out: &mut dyn Write,
) -> Result<u8, String> {
    let mut store = open_store_readonly(data_dir)?;
    let events = store
        .read_in_order(None, usize::MAX)
        .map_err(|e| format!("failed to read store log: {e}"))?;

    let now = SystemClock::new().now();
    let mut snapshot = ValveSnapshot {
        owner_actor_id: Some(owner_actor.clone()),
        t_now: now,
        ..ValveSnapshot::default()
    };
    snapshot.known_actors.insert(owner_actor.clone());
    let mut valve = Valve::with_default_policies(valve_cfg.clone(), snapshot)
        .map_err(|e| format!("valve policy verification failed: {e}"))?;
    valve.set_approval_attestation_key(mac_key);

    let mut controller = ValveDoorController::new(valve);
    controller.replay_from_store_events(&events);
    controller.valve_mut().snapshot_mut().t_now = now;

    let target_id = ActionId::new(action_id);
    let pending = controller
        .valve()
        .approval_manager()
        .pending()
        .get(&target_id)
        .cloned()
        .ok_or_else(|| format!("no pending approval found for action '{action_id}'"))?;

    let signature = controller.valve().sign_approval_resolution(
        &pending.action_id,
        &pending.h_act,
        pending.state_epoch,
        approved,
        now,
    );
    let params = ApprovalResolveParams {
        headers: Waist2Headers::new(format!("cli-approval-{action_id}-{}", now.as_millis())),
        action_id: pending.action_id.clone(),
        h_act: pending.h_act.clone(),
        state_epoch: pending.state_epoch,
        approved,
        resolver: owner_actor.clone(),
        resolved_at_ms: now,
        signature: Some(signature),
        out_of_band_channel: true,
    };

    let res = controller
        .resolve_approval(&params)
        .ok_or_else(|| "approval capability unavailable".to_owned())?
        .map_err(|e| format!("approval resolution rejected: {e}"))?;

    for ev in controller.take_pending_approval_events() {
        store
            .append(ev)
            .map_err(|e| format!("failed to persist approval resolution event: {e}"))?;
    }
    if !approved && let Some(receipt) = controller.take_pending_denial_receipt(&target_id) {
        let place = pending
            .places
            .first()
            .cloned()
            .unwrap_or_else(|| PlaceId::new("cli:approvals"));
        let denial_rcpt = EventRecord::new_root(
            EventKind::Receipt,
            now,
            owner_actor.clone(),
            place,
            Audience::new([params.resolver.clone()]),
            OriginActor::new(params.resolver.clone()),
            Taint::Trusted,
            monoagens_core::NumericFeatures::default(),
            OpaquePayload::default(),
            Vec::new(),
            Some(monoagens_core::EventDetail::Receipt {
                action_id: receipt.action_id,
                capability_id: receipt.capability_id,
                outcome: ReceiptOutcome::Refused,
                termination_reason: receipt.termination_reason,
                progress: receipt.progress,
                delivered_units: receipt.delivered_units,
                h_act: receipt.h_act,
            }),
        );
        store
            .append(denial_rcpt)
            .map_err(|e| format!("failed to persist denial receipt: {e}"))?;
    }

    if json_mode {
        let payload = json!({
            "applied": res.applied,
            "action_id": pending.action_id.as_str(),
            "approved": approved,
            "state_epoch": pending.state_epoch,
            "h_act": pending.h_act.as_str(),
            "out_of_band_channel": true,
        });
        write_json_pretty(out, &payload)?;
    } else {
        let verb = if approved { "granted" } else { "denied" };
        writeln!(
            out,
            "{verb} action={} state_epoch={} h_act={}",
            pending.action_id.as_str(),
            pending.state_epoch,
            pending.h_act.as_str()
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(0)
}

pub(crate) fn cmd_approvals(
    config_path: Option<&Path>,
    json_mode: bool,
    args: ApprovalsArgs,
    out: &mut dyn Write,
) -> Result<u8, String> {
    let sub = args.action.unwrap_or(ApprovalsSubcommand::List);
    let (cfg, paths) = load_paths(config_path)?;
    let owner_actor = owner_actor_from_cfg(cfg.as_ref());
    let valve_cfg = cfg.as_ref().map(build_valve_config).unwrap_or_default();
    let approval_secrets =
        load_approval_secrets(Some(&paths.secrets_path), |k| std::env::var(k).ok())
            .map_err(|e| e.to_string())?;
    let mac_key = resolve_approval_mac_key(&approval_secrets);

    match sub {
        ApprovalsSubcommand::Grant { action_id } => resolve_cli_approval(
            &paths.data_dir,
            &owner_actor,
            &valve_cfg,
            mac_key,
            &action_id,
            true,
            json_mode,
            out,
        ),
        ApprovalsSubcommand::Deny { action_id } => resolve_cli_approval(
            &paths.data_dir,
            &owner_actor,
            &valve_cfg,
            mac_key,
            &action_id,
            false,
            json_mode,
            out,
        ),
        ApprovalsSubcommand::List => {
            let store = open_store_readonly(&paths.data_dir)?;
            let events = store
                .read_in_order(None, usize::MAX)
                .map_err(|e| format!("failed to read store log: {e}"))?;

            let mut manager = monoagens_valve::ApprovalManager::with_attestation_key(mac_key);
            let replayed_epoch =
                manager.replay_from_events(&events, Some(&owner_actor), &valve_cfg);

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
                let (_computed_h, canonical_input, preimage) =
                    canonical_h_act_details(&action, epoch);
                let taint = manager.pending_taint(aid).unwrap_or(action.taint);
                let audience: Vec<String> = action
                    .audience
                    .members
                    .iter()
                    .map(|m| m.as_str().to_owned())
                    .collect();
                entries.push(PendingApprovalEntry {
                    action_id: aid.as_str().to_owned(),
                    capability_id: pending.capability_id.as_str().to_owned(),
                    places: pending
                        .places
                        .iter()
                        .map(|p| p.as_str().to_owned())
                        .collect(),
                    audience,
                    taint: taint_label(taint).to_owned(),
                    state_epoch: epoch,
                    h_act: pending.h_act.as_str().to_owned(),
                    canonical_payload_utf8: String::from_utf8_lossy(&canonical_input.payload)
                        .into_owned(),
                    canonical_input,
                    canonical_preimage_hex: to_hex(&preimage),
                    reason_code: pending.reason.clone(),
                    timestamp_ms: pending.requested_at_ms.as_millis(),
                    expires_at_ms: pending.expires_at_ms.as_millis(),
                });
            }

            if json_mode {
                write_json_pretty(out, &entries)?;
            } else if entries.is_empty() {
                writeln!(out, "no pending approvals").map_err(|e| e.to_string())?;
            } else {
                for entry in entries {
                    let h_short = if entry.h_act.len() > 23 {
                        &entry.h_act[..23]
                    } else {
                        &entry.h_act
                    };
                    writeln!(
                        out,
                        "capability={} action={} expires_at_ms={}\n  places=[{}] audience=[{}] taint={} reason={}\n  h_act={} state_epoch={}",
                        entry.capability_id,
                        entry.action_id,
                        entry.expires_at_ms,
                        entry.places.join(","),
                        entry.audience.join(","),
                        entry.taint,
                        entry.reason_code.as_deref().unwrap_or("PendingApproval"),
                        h_short,
                        entry.state_epoch,
                    )
                    .map_err(|e| e.to_string())?;
                }
            }
            Ok(0)
        }
    }
}
