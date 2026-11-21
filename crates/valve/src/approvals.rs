//! Risk tier classification (`R_0`..`R_3`), canonical `H_act` binding, out-of-band
//! `I-HITL` keyed BLAKE3-MAC verification, event-sourced `ApprovalManager` projection,
//! and anti-approval-fatigue partitioned sliding caps (`R-42`, `R-67`).

use std::collections::{BTreeMap, BTreeSet};

use monoagens_core::{DoorDecision, EventDetail, EventKind, EventRecord};
use monoagens_protocol::{
    Action, ActionCanonicalInput, ActionId, ActorId, CapabilityFlags, HAct, PendingApproval,
    PlaceId, ReceiptOutcome, Taint, Time,
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
/// Minimum hourly slots reserved exclusively for `Taint::Trusted` owner-originated chains (`2/h`).
pub const MIN_TRUSTED_SLOTS_PER_HOUR: usize = 2;
/// Maximum hourly approval prompts permitted for untrusted (`Taint < Trusted` or non-owner) chains (`2/h`).
pub const MAX_UNTRUSTED_PROMPTS_PER_HOUR: usize = 2;
/// Latency threshold below which an approval resolution is classified as rubber-stamped (`500 ms`).
pub const RUBBER_STAMP_LATENCY_MS: u64 = 500;
/// Maximum concurrent items in `PendingApproval` queue (`3`).
pub const MAX_PENDING_QUEUE: usize = 3;
/// Deduplication cooldown in seconds before re-prompting an identical `H_act` (`600 s`).
pub const DEDUP_COOLDOWN_S: u64 = 600;

const APPROVAL_MAC_KEY_CONTEXT: &str = "monoagens.approval.mac.key.v1";
const APPROVAL_SIG_PREFIX: &str = "blake3-mac:";
const DEFAULT_TEST_APPROVAL_SECRET: &[u8] = b"monoagens.default.test.approval.attestation.key.v1";

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

/// Computes canonical signing preimage for out-of-band `approval/resolve` attestation (`I-HITL`):
/// `"monoagens.approval.v1|action_id|h_act|state_epoch|approved|resolved_at_ms"`.
#[must_use]
pub fn approval_signing_preimage(
    action_id: &ActionId,
    h_act: &HAct,
    state_epoch: u64,
    approved: bool,
    resolved_at_ms: Time,
) -> Vec<u8> {
    format!(
        "monoagens.approval.v1|{}|{}|{}|{}|{}",
        action_id.as_str(),
        h_act.as_str(),
        state_epoch,
        approved,
        resolved_at_ms.as_millis(),
    )
    .into_bytes()
}

/// Derives a 32-byte keyed BLAKE3-MAC key from an arbitrary-length owner attestation secret.
#[must_use]
pub fn derive_approval_mac_key(secret: &[u8]) -> [u8; 32] {
    blake3::derive_key(APPROVAL_MAC_KEY_CONTEXT, secret)
}

/// Deterministic fallback MAC key for offline unit/integration tests when no custom secret is configured.
#[must_use]
pub fn default_test_approval_mac_key() -> [u8; 32] {
    derive_approval_mac_key(DEFAULT_TEST_APPROVAL_SECRET)
}

/// Produces a keyed `blake3-mac:<hex>` signature over canonical `approval/resolve` fields.
#[must_use]
pub fn sign_approval_resolution(
    mac_key: &[u8; 32],
    action_id: &ActionId,
    h_act: &HAct,
    state_epoch: u64,
    approved: bool,
    resolved_at_ms: Time,
) -> String {
    let preimage =
        approval_signing_preimage(action_id, h_act, state_epoch, approved, resolved_at_ms);
    let mac = blake3::keyed_hash(mac_key, &preimage);
    format!("{APPROVAL_SIG_PREFIX}{}", mac.to_hex())
}

/// Constant-time verification of a `blake3-mac:<hex>` signature over canonical `approval/resolve` fields.
#[must_use]
pub fn verify_approval_signature(
    mac_key: &[u8; 32],
    action_id: &ActionId,
    h_act: &HAct,
    state_epoch: u64,
    approved: bool,
    resolved_at_ms: Time,
    signature: &str,
) -> bool {
    let trimmed = signature.trim();
    let Some(hex_str) = trimmed.strip_prefix(APPROVAL_SIG_PREFIX) else {
        return false;
    };
    let Some(provided_bytes) = decode_hex_32(hex_str) else {
        return false;
    };
    let preimage =
        approval_signing_preimage(action_id, h_act, state_epoch, approved, resolved_at_ms);
    let expected = blake3::keyed_hash(mac_key, &preimage);
    let provided = blake3::Hash::from_bytes(provided_bytes);
    expected == provided
}

fn decode_hex_32(hex: &str) -> Option<[u8; 32]> {
    if hex.len() != 64 {
        return None;
    }
    let bytes = hex.as_bytes();
    let mut out = [0u8; 32];
    let (chunks, _rem) = bytes.as_chunks::<2>();
    for (idx, chunk) in chunks.iter().enumerate() {
        let hi = hex_nibble(chunk[0])?;
        let lo = hex_nibble(chunk[1])?;
        out[idx] = (hi << 4) | lo;
    }
    Some(out)
}

const fn hex_nibble(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Owner approval grant bound cryptographically to `action_id`, `H_act`, `state_epoch`, and TTL.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalGrant {
    pub action_id: ActionId,
    pub h_act: HAct,
    pub state_epoch: u64,
    pub granted_at_ms: Time,
    pub expires_at_ms: Time,
    pub signer: ActorId,
    pub signature: String,
    pub is_embodied: bool,
    #[serde(default)]
    pub out_of_band_channel: bool,
}

impl ApprovalGrant {
    /// Creates an out-of-band owner-signed [`ApprovalGrant`] using `30 s` TTL for embodied
    /// actions and `900 s` TTL for chat/standard actions.
    #[must_use]
    pub fn new(
        action_id: ActionId,
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
            action_id,
            h_act,
            state_epoch,
            granted_at_ms,
            expires_at_ms: granted_at_ms.saturating_add_ms(ttl_s.saturating_mul(1000)),
            signer,
            signature: signature.into(),
            is_embodied,
            out_of_band_channel: true,
        }
    }
}

/// Append-only event-log payload persisted inside `EventKind::DoorVerdict` for restart replay.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "approval_event", rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)]
pub enum ApprovalLogRecord {
    Held {
        pending: PendingApproval,
        taint: Taint,
        is_embodied: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        action: Option<Action>,
    },
    Resolved {
        action_id: ActionId,
        h_act: HAct,
        state_epoch: u64,
        approved: bool,
        resolver: ActorId,
        resolved_at_ms: Time,
        expires_at_ms: Time,
        is_embodied: bool,
        out_of_band_channel: bool,
        signature: String,
    },
    GrantConsumed {
        h_act: HAct,
    },
}

/// Failures returned by [`ApprovalManager::resolve_out_of_band`] (always fail-closed).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalResolveError {
    NotOutOfBand,
    MissingOwner,
    ResolverNotOwner,
    MissingOrInvalidSignature,
    NoPendingApproval,
    HashMismatch,
    EpochMismatch,
    Expired,
}

impl ApprovalResolveError {
    /// Canonical diagnostic detail for audit traces and Waist-2 errors.
    #[must_use]
    pub const fn reason_str(self) -> &'static str {
        match self {
            Self::NotOutOfBand => "approval_resolve_rejected_in_band_channel",
            Self::MissingOwner => "approval_resolve_rejected_missing_owner",
            Self::ResolverNotOwner => "approval_resolve_rejected_non_owner",
            Self::MissingOrInvalidSignature => "approval_resolve_rejected_invalid_signature",
            Self::NoPendingApproval => "approval_resolve_rejected_unknown_or_settled_action",
            Self::HashMismatch => "approval_resolve_rejected_h_act_mismatch",
            Self::EpochMismatch => "approval_resolve_rejected_state_epoch_mismatch",
            Self::Expired => "approval_resolve_rejected_expired_ttl",
        }
    }
}

/// Successful out-of-band approval resolution ready for event persistence and optional dispatch.
#[derive(Debug, Clone, PartialEq)]
pub struct ApprovalResolveOutcome {
    pub pending: PendingApproval,
    pub grant: Option<ApprovalGrant>,
    pub log_record: ApprovalLogRecord,
    pub action: Option<Action>,
    pub latency_ms: u64,
    pub rubber_stamped: bool,
}

/// Snapshot and classification context supplied to [`ApprovalManager::try_hold`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HoldRequestContext {
    pub state_epoch: u64,
    pub t_now: Time,
    pub is_embodied: bool,
    pub reason_tag: ValveReasonTag,
    pub is_trusted_owner_chain: bool,
}

/// Stateful, deterministic `PendingApproval` + partitioned sliding-cap manager (`max_retry_on_held = 0`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApprovalManager {
    pending: BTreeMap<ActionId, PendingApproval>,
    pending_actions: BTreeMap<ActionId, Action>,
    pending_taints: BTreeMap<ActionId, Taint>,
    pending_embodied: BTreeMap<ActionId, bool>,
    grants: BTreeMap<HAct, ApprovalGrant>,
    consumed_grants: BTreeSet<HAct>,
    denied_actions: BTreeSet<ActionId>,
    prompt_timestamps_ms: Vec<Time>,
    trusted_prompt_timestamps_ms: Vec<Time>,
    untrusted_prompt_timestamps_ms: Vec<Time>,
    dedup_last_prompt_ms: BTreeMap<HAct, Time>,
    prompts_emitted_total: u64,
    state_epoch: u64,
    min_trusted_slots_per_hour: usize,
    max_untrusted_prompts_per_hour: usize,
    approval_latencies_ms: Vec<u64>,
    resolutions_total: u64,
    rubber_stamp_count: u64,
    untrusted_flood_rejections_total: u64,
    spoof_rejections_total: u64,
    #[serde(skip, default = "default_test_approval_mac_key")]
    attestation_key: [u8; 32],
}

impl Default for ApprovalManager {
    fn default() -> Self {
        Self::with_attestation_key(default_test_approval_mac_key())
    }
}

impl ApprovalManager {
    /// Creates an empty [`ApprovalManager`] using the deterministic test attestation key.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates an empty [`ApprovalManager`] pinned to a specific 32-byte keyed BLAKE3-MAC key.
    #[must_use]
    pub fn with_attestation_key(attestation_key: [u8; 32]) -> Self {
        Self {
            pending: BTreeMap::new(),
            pending_actions: BTreeMap::new(),
            pending_taints: BTreeMap::new(),
            pending_embodied: BTreeMap::new(),
            grants: BTreeMap::new(),
            consumed_grants: BTreeSet::new(),
            denied_actions: BTreeSet::new(),
            prompt_timestamps_ms: Vec::new(),
            trusted_prompt_timestamps_ms: Vec::new(),
            untrusted_prompt_timestamps_ms: Vec::new(),
            dedup_last_prompt_ms: BTreeMap::new(),
            prompts_emitted_total: 0,
            state_epoch: 0,
            min_trusted_slots_per_hour: MIN_TRUSTED_SLOTS_PER_HOUR,
            max_untrusted_prompts_per_hour: MAX_UNTRUSTED_PROMPTS_PER_HOUR,
            approval_latencies_ms: Vec::new(),
            resolutions_total: 0,
            rubber_stamp_count: 0,
            untrusted_flood_rejections_total: 0,
            spoof_rejections_total: 0,
            attestation_key,
        }
    }

    /// Replaces the keyed BLAKE3-MAC key used to verify out-of-band approval grants.
    pub fn set_attestation_key(&mut self, attestation_key: [u8; 32]) {
        self.attestation_key = attestation_key;
    }

    /// Returns the active 32-byte keyed BLAKE3-MAC key.
    #[must_use]
    pub const fn attestation_key(&self) -> &[u8; 32] {
        &self.attestation_key
    }

    /// Overrides the hourly trusted/untrusted quota partitioning bounds.
    pub fn set_quota_partitioning(
        &mut self,
        min_trusted_slots_per_hour: usize,
        max_untrusted_prompts_per_hour: usize,
    ) {
        self.min_trusted_slots_per_hour = min_trusted_slots_per_hour;
        self.max_untrusted_prompts_per_hour = max_untrusted_prompts_per_hour;
    }

    /// Registers an owner [`ApprovalGrant`] bound to `action_id`, `H_act`, and `state_epoch`.
    pub fn register_grant(&mut self, grant: ApprovalGrant) {
        self.pending.retain(|_, p| p.h_act != grant.h_act);
        self.grants.insert(grant.h_act.clone(), grant);
    }

    /// Records the full [`Action`] associated with a currently held `PendingApproval`.
    pub fn remember_held_action(&mut self, action: Action, taint: Taint, is_embodied: bool) {
        self.pending_taints.insert(action.action_id.clone(), taint);
        self.pending_embodied
            .insert(action.action_id.clone(), is_embodied);
        self.pending_actions
            .insert(action.action_id.clone(), action);
    }

    /// Returns currently active `PendingApproval` items.
    #[must_use]
    pub fn pending(&self) -> &BTreeMap<ActionId, PendingApproval> {
        &self.pending
    }

    /// Returns the reconstructed [`Action`] for a held `action_id`, if recorded.
    #[must_use]
    pub fn pending_action(&self, action_id: &ActionId) -> Option<&Action> {
        self.pending_actions.get(action_id)
    }

    /// Returns the recorded [`Taint`] for a held `action_id`.
    #[must_use]
    pub fn pending_taint(&self, action_id: &ActionId) -> Option<Taint> {
        self.pending_taints.get(action_id).copied()
    }

    /// Returns whether a held `action_id` was marked embodied (`30 s` TTL vs `900 s` chat TTL).
    #[must_use]
    pub fn pending_is_embodied(&self, action_id: &ActionId) -> bool {
        self.pending_embodied
            .get(action_id)
            .copied()
            .unwrap_or(false)
    }

    /// Returns all currently active unconsumed [`ApprovalGrant`]s.
    #[must_use]
    pub fn grants(&self) -> &BTreeMap<HAct, ApprovalGrant> {
        &self.grants
    }

    /// Returns the projected `state_epoch` reconstructed from event replay.
    #[must_use]
    pub const fn state_epoch(&self) -> u64 {
        self.state_epoch
    }

    /// Total number of prompts actually emitted (never exceeds sliding/dedup bounds).
    #[must_use]
    pub const fn prompts_emitted_total(&self) -> u64 {
        self.prompts_emitted_total
    }

    /// Returns recorded approval resolution latency samples in milliseconds.
    #[must_use]
    pub fn approval_latencies_ms(&self) -> &[u64] {
        &self.approval_latencies_ms
    }

    /// Total number of approval resolutions (`grant` + `deny`) recorded.
    #[must_use]
    pub const fn resolutions_total(&self) -> u64 {
        self.resolutions_total
    }

    /// Total number of approvals resolved faster than [`RUBBER_STAMP_LATENCY_MS`] (`500 ms`).
    #[must_use]
    pub const fn rubber_stamp_count(&self) -> u64 {
        self.rubber_stamp_count
    }

    /// Fraction of approval resolutions completed in `< 500 ms` (`0.0` when zero resolutions).
    #[must_use]
    pub fn rubber_stamp_rate(&self) -> f64 {
        if self.resolutions_total == 0 {
            0.0
        } else {
            self.rubber_stamp_count as f64 / self.resolutions_total as f64
        }
    }

    /// Total untrusted hold requests rejected due to hourly flood/quota partitioning.
    #[must_use]
    pub const fn untrusted_flood_rejections_total(&self) -> u64 {
        self.untrusted_flood_rejections_total
    }

    /// Total invalid/in-band/spoofed `approval/resolve` attempts rejected fail-closed.
    #[must_use]
    pub const fn spoof_rejections_total(&self) -> u64 {
        self.spoof_rejections_total
    }

    /// Number of untrusted approval prompts emitted in the 1-hour window ending at `t_now`.
    #[must_use]
    pub fn untrusted_prompts_in_last_hour(&self, t_now: Time) -> usize {
        let one_hour_ago = Time::from_millis(t_now.as_millis().saturating_sub(3_600_000));
        self.untrusted_prompt_timestamps_ms
            .iter()
            .filter(|&&ts| ts >= one_hour_ago)
            .count()
    }

    /// Number of trusted owner-chain approval prompts emitted in the 1-hour window ending at `t_now`.
    #[must_use]
    pub fn trusted_prompts_in_last_hour(&self, t_now: Time) -> usize {
        let one_hour_ago = Time::from_millis(t_now.as_millis().saturating_sub(3_600_000));
        self.trusted_prompt_timestamps_ms
            .iter()
            .filter(|&&ts| ts >= one_hour_ago)
            .count()
    }

    /// Records a spoof/in-band/invalid-signature rejection in telemetry.
    pub fn record_spoof_rejection(&mut self) {
        self.spoof_rejections_total = self.spoof_rejections_total.saturating_add(1);
    }

    /// Prunes expired pending entries and old hourly prompt timestamps at `t_now`.
    pub fn prune_at(&mut self, t_now: Time) {
        self.pending.retain(|_, p| t_now <= p.expires_at_ms);
        let one_hour_ago = Time::from_millis(t_now.as_millis().saturating_sub(3_600_000));
        self.prompt_timestamps_ms.retain(|&ts| ts >= one_hour_ago);
        self.trusted_prompt_timestamps_ms
            .retain(|&ts| ts >= one_hour_ago);
        self.untrusted_prompt_timestamps_ms
            .retain(|&ts| ts >= one_hour_ago);
    }

    /// Reconstructs `ApprovalManager` state deterministically from append-only [`EventRecord`]s,
    /// returning the projected `state_epoch` (`0` before any `HardForget`).
    pub fn replay_from_events(
        &mut self,
        events: &[EventRecord],
        owner_actor_id: Option<&ActorId>,
        config: &ValveConfig,
    ) -> u64 {
        let key = self.attestation_key;
        let min_trusted = self.min_trusted_slots_per_hour;
        let max_untrusted = self.max_untrusted_prompts_per_hour;
        *self = Self::with_attestation_key(key);
        self.set_quota_partitioning(min_trusted, max_untrusted);

        let mut recorded_actions: BTreeMap<ActionId, (Action, Taint)> = BTreeMap::new();
        let mut seen_hold_prompts: BTreeSet<ActionId> = BTreeSet::new();

        for ev in events {
            match ev.kind {
                EventKind::HardForget => {
                    if let Some(EventDetail::HardForget { epoch, .. }) = &ev.detail {
                        self.state_epoch = (*epoch).max(self.state_epoch.saturating_add(1));
                    }
                }
                EventKind::Action => {
                    if let Some(EventDetail::Action {
                        action_id,
                        capability_id,
                        h_act,
                        correlation_id,
                    }) = &ev.detail
                    {
                        let act = Action {
                            protocol_version: monoagens_protocol::CURRENT_PROTOCOL_VERSION,
                            action_id: action_id.clone(),
                            capability_id: capability_id.clone(),
                            places: vec![ev.place.clone()],
                            audience: ev.audience.clone(),
                            origin_actor: ev.origin_actor.clone(),
                            taint: ev.taint,
                            payload: ev.payload.clone(),
                            correlation_id: correlation_id.clone(),
                            expected_effect: None,
                            h_act: h_act.clone(),
                            timestamp_ms: ev.timestamp_ms,
                        };
                        recorded_actions.insert(action_id.clone(), (act, ev.taint));
                        if let Some(h) = h_act
                            && self.grants.contains_key(h)
                        {
                            self.consume_grant(h);
                        }
                    }
                }
                EventKind::DoorVerdict => {
                    if let Ok(log_rec) =
                        serde_json::from_slice::<ApprovalLogRecord>(ev.payload.as_bytes())
                    {
                        match log_rec {
                            ApprovalLogRecord::Held {
                                pending,
                                taint,
                                is_embodied,
                                action,
                            } => {
                                let aid = pending.action_id.clone();
                                if let Some(act) = action {
                                    self.pending_actions.insert(aid.clone(), act);
                                }
                                self.pending_taints.insert(aid.clone(), taint);
                                self.pending_embodied.insert(aid.clone(), is_embodied);
                                self.dedup_last_prompt_ms
                                    .insert(pending.h_act.clone(), pending.requested_at_ms);
                                if seen_hold_prompts.insert(aid.clone()) {
                                    self.record_prompt_timestamp(
                                        pending.requested_at_ms,
                                        taint == Taint::Trusted
                                            && owner_actor_id
                                                .is_some_and(|o| &ev.origin_actor.0 == o),
                                    );
                                }
                                self.pending.insert(aid, pending);
                            }
                            ApprovalLogRecord::Resolved {
                                action_id,
                                h_act,
                                state_epoch,
                                approved,
                                resolver,
                                resolved_at_ms,
                                expires_at_ms,
                                is_embodied,
                                out_of_band_channel,
                                signature,
                            } => {
                                if let Some(pending) = self.pending.get(&action_id) {
                                    let latency =
                                        resolved_at_ms.saturating_sub_ms(pending.requested_at_ms);
                                    self.record_resolution_latency(latency);
                                }
                                self.pending.remove(&action_id);
                                if approved
                                    && out_of_band_channel
                                    && owner_actor_id.is_some_and(|o| &resolver == o)
                                    && verify_approval_signature(
                                        &self.attestation_key,
                                        &action_id,
                                        &h_act,
                                        state_epoch,
                                        true,
                                        resolved_at_ms,
                                        &signature,
                                    )
                                    && !self.consumed_grants.contains(&h_act)
                                {
                                    self.grants.insert(
                                        h_act.clone(),
                                        ApprovalGrant {
                                            action_id,
                                            h_act,
                                            state_epoch,
                                            granted_at_ms: resolved_at_ms,
                                            expires_at_ms,
                                            signer: resolver,
                                            signature,
                                            is_embodied,
                                            out_of_band_channel: true,
                                        },
                                    );
                                } else if !approved {
                                    self.denied_actions.insert(action_id);
                                }
                            }
                            ApprovalLogRecord::GrantConsumed { h_act } => {
                                self.consume_grant(&h_act);
                            }
                        }
                    }
                }
                EventKind::Receipt => {
                    if let Some(EventDetail::Receipt {
                        action_id,
                        capability_id,
                        outcome,
                        h_act,
                        ..
                    }) = &ev.detail
                    {
                        if *outcome == ReceiptOutcome::Held {
                            let (act_opt, taint) = recorded_actions
                                .get(action_id)
                                .map(|(a, t)| (Some(a.clone()), *t))
                                .unwrap_or((None, ev.taint));
                            let is_embodied = {
                                let lower = capability_id.as_str().to_ascii_lowercase();
                                lower.contains("actuator") || lower.contains("ros2")
                            };
                            let ttl_s = if is_embodied {
                                config.approval_ttl_embodied_s
                            } else {
                                config.approval_ttl_chat_s
                            };
                            let computed_h = h_act.clone().unwrap_or_else(|| match &act_opt {
                                Some(act) => compute_h_act(act, self.state_epoch),
                                None => HAct::new(format!("blake3:held-{}", action_id.as_str())),
                            });
                            if !self.pending.contains_key(action_id)
                                && !self.denied_actions.contains(action_id)
                                && !self.grants.contains_key(&computed_h)
                                && !self.consumed_grants.contains(&computed_h)
                            {
                                let places = act_opt
                                    .as_ref()
                                    .map(|a| a.places.clone())
                                    .unwrap_or_else(|| vec![ev.place.clone()]);
                                let pending = PendingApproval {
                                    action_id: action_id.clone(),
                                    capability_id: capability_id.clone(),
                                    h_act: computed_h.clone(),
                                    state_epoch: self.state_epoch,
                                    requested_at_ms: ev.timestamp_ms,
                                    expires_at_ms: ev
                                        .timestamp_ms
                                        .saturating_add_ms(ttl_s.saturating_mul(1000)),
                                    places,
                                    reason: Some(ValveReasonTag::PendingApproval.as_str().to_owned()),
                                };
                                self.pending.insert(action_id.clone(), pending);
                                self.pending_taints.insert(action_id.clone(), taint);
                                self.pending_embodied.insert(action_id.clone(), is_embodied);
                                if let Some(act) = act_opt {
                                    self.pending_actions.insert(action_id.clone(), act);
                                }
                            }
                            self.dedup_last_prompt_ms
                                .insert(computed_h, ev.timestamp_ms);
                            if seen_hold_prompts.insert(action_id.clone()) {
                                let is_trusted_owner = taint == Taint::Trusted
                                    && owner_actor_id.is_some_and(|o| &ev.origin_actor.0 == o);
                                self.record_prompt_timestamp(ev.timestamp_ms, is_trusted_owner);
                            }
                        } else {
                            self.pending.remove(action_id);
                            self.pending_actions.remove(action_id);
                            if let Some(h) = h_act
                                && self.grants.contains_key(h)
                            {
                                self.consume_grant(h);
                            }
                        }
                    }
                }
                EventKind::ExpectationChanged => {
                    if let Some(EventDetail::ExpectationTransition {
                        action_id: Some(aid),
                        to_state,
                        ..
                    }) = &ev.detail
                        && matches!(
                            to_state,
                            monoagens_core::ExpectationLifecycleState::Approved
                                | monoagens_core::ExpectationLifecycleState::Rejected
                                | monoagens_core::ExpectationLifecycleState::Cancelled
                                | monoagens_core::ExpectationLifecycleState::Expired
                        )
                    {
                        self.pending.remove(aid);
                    }
                }
                _ => {}
            }
        }

        self.state_epoch
    }

    fn record_prompt_timestamp(&mut self, ts: Time, is_trusted_owner_chain: bool) {
        self.prompt_timestamps_ms.push(ts);
        if is_trusted_owner_chain {
            self.trusted_prompt_timestamps_ms.push(ts);
        } else {
            self.untrusted_prompt_timestamps_ms.push(ts);
        }
        self.prompts_emitted_total = self.prompts_emitted_total.saturating_add(1);
    }

    fn record_resolution_latency(&mut self, latency_ms: u64) {
        self.resolutions_total = self.resolutions_total.saturating_add(1);
        self.approval_latencies_ms.push(latency_ms);
        if latency_ms < RUBBER_STAMP_LATENCY_MS {
            self.rubber_stamp_count = self.rubber_stamp_count.saturating_add(1);
        }
    }

    /// Validates and resolves a held `PendingApproval` via an attested out-of-band owner channel
    /// (`I-HITL`), failing closed on any in-band, unsigned, forged, mismatched, or expired request.
    #[allow(clippy::too_many_arguments)]
    pub fn resolve_out_of_band(
        &mut self,
        action_id: &ActionId,
        h_act: &HAct,
        state_epoch: u64,
        approved: bool,
        resolver: &ActorId,
        resolved_at_ms: Time,
        out_of_band_channel: bool,
        signature: Option<&str>,
        owner_actor_id: Option<&ActorId>,
        config: &ValveConfig,
    ) -> Result<ApprovalResolveOutcome, ApprovalResolveError> {
        if !out_of_band_channel {
            self.record_spoof_rejection();
            return Err(ApprovalResolveError::NotOutOfBand);
        }
        let Some(owner) = owner_actor_id else {
            self.record_spoof_rejection();
            return Err(ApprovalResolveError::MissingOwner);
        };
        if resolver != owner {
            self.record_spoof_rejection();
            return Err(ApprovalResolveError::ResolverNotOwner);
        }
        let Some(sig) = signature else {
            self.record_spoof_rejection();
            return Err(ApprovalResolveError::MissingOrInvalidSignature);
        };
        if !verify_approval_signature(
            &self.attestation_key,
            action_id,
            h_act,
            state_epoch,
            approved,
            resolved_at_ms,
            sig,
        ) {
            self.record_spoof_rejection();
            return Err(ApprovalResolveError::MissingOrInvalidSignature);
        }

        let Some(pending) = self.pending.get(action_id).cloned() else {
            return Err(ApprovalResolveError::NoPendingApproval);
        };
        if &pending.h_act != h_act {
            return Err(ApprovalResolveError::HashMismatch);
        }
        if pending.state_epoch != state_epoch {
            return Err(ApprovalResolveError::EpochMismatch);
        }
        if resolved_at_ms > pending.expires_at_ms {
            self.pending.remove(action_id);
            return Err(ApprovalResolveError::Expired);
        }

        let latency_ms = resolved_at_ms.saturating_sub_ms(pending.requested_at_ms);
        let rubber_stamped = latency_ms < RUBBER_STAMP_LATENCY_MS;
        self.record_resolution_latency(latency_ms);

        self.pending.remove(action_id);
        let is_embodied = self.pending_embodied.get(action_id).copied().unwrap_or_else(|| {
            let lower = pending.capability_id.as_str().to_ascii_lowercase();
            lower.contains("actuator") || lower.contains("ros2")
        });
        let action = self.pending_actions.get(action_id).cloned();

        let ttl_s = if is_embodied {
            config.approval_ttl_embodied_s
        } else {
            config.approval_ttl_chat_s
        };
        let expires_at_ms = resolved_at_ms.saturating_add_ms(ttl_s.saturating_mul(1000));

        let grant = if approved {
            let g = ApprovalGrant {
                action_id: action_id.clone(),
                h_act: h_act.clone(),
                state_epoch,
                granted_at_ms: resolved_at_ms,
                expires_at_ms,
                signer: resolver.clone(),
                signature: sig.trim().to_owned(),
                is_embodied,
                out_of_band_channel: true,
            };
            self.grants.insert(h_act.clone(), g.clone());
            Some(g)
        } else {
            self.denied_actions.insert(action_id.clone());
            self.pending_actions.remove(action_id);
            None
        };

        let log_record = ApprovalLogRecord::Resolved {
            action_id: action_id.clone(),
            h_act: h_act.clone(),
            state_epoch,
            approved,
            resolver: resolver.clone(),
            resolved_at_ms,
            expires_at_ms,
            is_embodied,
            out_of_band_channel: true,
            signature: sig.trim().to_owned(),
        };

        Ok(ApprovalResolveOutcome {
            pending,
            grant,
            log_record,
            action,
            latency_ms,
            rubber_stamped,
        })
    }

    /// Checks whether an action carrying `action.h_act` or matching an [`ApprovalGrant`]
    /// is validly authorized at `(t_now, state_epoch)`.
    ///
    /// Returns:
    /// - `Ok(true)` if validly approved by a matching, unexpired, same-epoch, out-of-band MAC-signed owner grant.
    /// - `Ok(false)` if no approval has been attempted yet (`action.h_act.is_none()` and no grant).
    /// - `Err(ValveReasonTag::StaleOrMismatchedApproval)` if TOCTOU mutation, epoch drift,
    ///   TTL expiration, in-band origin, or invalid MAC signature occurred.
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
                if !is_grant_valid(
                    grant,
                    &action.action_id,
                    computed_h_act,
                    owner_actor_id,
                    state_epoch,
                    t_now,
                    &self.attestation_key,
                ) {
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
            if !is_grant_valid(
                grant,
                &action.action_id,
                computed_h_act,
                owner_actor_id,
                state_epoch,
                t_now,
                &self.attestation_key,
            ) {
                return Err(ValveReasonTag::StaleOrMismatchedApproval);
            }
            return Ok(true);
        }

        Ok(false)
    }

    /// Consumes a verified grant upon dispatch so it cannot be replayed across turns.
    pub fn consume_grant(&mut self, h_act: &HAct) {
        if let Some(grant) = self.grants.remove(h_act) {
            self.pending_actions.remove(&grant.action_id);
        }
        self.consumed_grants.insert(h_act.clone());
    }

    /// Attempts to hold an unapproved action in `PendingApproval` subject to partitioned sliding caps
    /// (`max_pending_queue`, `max_prompts_per_hour`, `min_trusted_slots_per_hour`,
    /// `max_untrusted_prompts_per_hour`, `dedup_cooldown_s`, `max_retry_on_held = 0`).
    pub fn try_hold(
        &mut self,
        action: &mut Action,
        computed_h_act: HAct,
        ctx: HoldRequestContext,
        config: &ValveConfig,
    ) -> (DoorDecision, ValveReasonTag) {
        self.prune_at(ctx.t_now);
        self.state_epoch = ctx.state_epoch;

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

        let total_in_hour = self.prompt_timestamps_ms.len();
        if total_in_hour >= config.max_prompts_per_hour {
            return (
                DoorDecision::Refuse {
                    rule: "valve.approval.max_prompts_per_hour".to_owned(),
                    reason: ValveReasonTag::OversightBudgetExhausted.as_str().to_owned(),
                },
                ValveReasonTag::OversightBudgetExhausted,
            );
        }

        if !ctx.is_trusted_owner_chain {
            let trusted_in_hour = self.trusted_prompt_timestamps_ms.len();
            let untrusted_in_hour = self.untrusted_prompt_timestamps_ms.len();
            let reserved_trusted_needed = self
                .min_trusted_slots_per_hour
                .saturating_sub(trusted_in_hour);
            let remaining_total = config.max_prompts_per_hour.saturating_sub(total_in_hour);
            if untrusted_in_hour >= self.max_untrusted_prompts_per_hour
                || remaining_total <= reserved_trusted_needed
            {
                self.untrusted_flood_rejections_total =
                    self.untrusted_flood_rejections_total.saturating_add(1);
                return (
                    DoorDecision::Refuse {
                        rule: "valve.approval.untrusted_prompts_per_hour".to_owned(),
                        reason: ValveReasonTag::OversightBudgetExhausted.as_str().to_owned(),
                    },
                    ValveReasonTag::OversightBudgetExhausted,
                );
            }
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
        self.pending_actions
            .insert(action.action_id.clone(), action.clone());
        self.pending_taints
            .insert(action.action_id.clone(), action.taint);
        self.pending_embodied
            .insert(action.action_id.clone(), ctx.is_embodied);
        self.record_prompt_timestamp(ctx.t_now, ctx.is_trusted_owner_chain);
        self.dedup_last_prompt_ms.insert(computed_h_act, ctx.t_now);

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
    expected_action_id: &ActionId,
    expected_h_act: &HAct,
    owner_actor_id: Option<&ActorId>,
    state_epoch: u64,
    t_now: Time,
    mac_key: &[u8; 32],
) -> bool {
    let Some(owner) = owner_actor_id else {
        return false;
    };
    if !grant.out_of_band_channel {
        return false;
    }
    if &grant.signer != owner {
        return false;
    }
    if &grant.action_id != expected_action_id || &grant.h_act != expected_h_act {
        return false;
    }
    if grant.state_epoch != state_epoch {
        return false;
    }
    if !(t_now >= grant.granted_at_ms && t_now <= grant.expires_at_ms) {
        return false;
    }
    verify_approval_signature(
        mac_key,
        &grant.action_id,
        &grant.h_act,
        grant.state_epoch,
        true,
        grant.granted_at_ms,
        &grant.signature,
    )
}
