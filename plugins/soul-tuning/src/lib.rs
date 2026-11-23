//! `plugin-soul-tuning`: Waist-2 (`monoagens.body.v1`) speech-mode tuning capability plugin.
//!
//! Exposes `soul_tuning.get_mode`, `soul_tuning.list_modes`, and owner-only
//! (`identity_changing: true`) `soul_tuning.set_mode` on place `cap:soul-tuning`,
//! persisting the active mode atomically in `MONOAGENS_PLUGIN_STATE_DIR` and
//! managing `soul-tuning.toml` via `MONOAGENS_PLUGIN_CONFIG`.

#![forbid(unsafe_code)]

pub mod config;

use std::collections::HashSet;
use std::path::PathBuf;

use monoagens_body_kit::{
    HonestCueThresholds, capability_schema_hash, make_refused_receipt, make_terminal_receipt,
};
use monoagens_body_sdk::{
    ActionCancelParams, ActionCancelResult, ActionExecuteParams, ActionExecuteResult, ActionId,
    ApprovalResolveParams, ApprovalResolveResult, Audience, BodyDiscoverParams, BodyDiscoverResult,
    BodyHandler, BodyId, CURRENT_PROTOCOL_VERSION, CapabilityDescriptor, CapabilityFlags,
    CapabilityId, MetaValue, PerceptEmitParams, PerceptEmitResult, PlaceId, ReceiptOutcome,
    TerminationReason, Time, Waist2Error, sort_capabilities,
};
use serde::Deserialize;
use serde_json::{Value, json};

pub use config::{
    ACTIVE_MODE_FILENAME, DEFAULT_MODE_NAME, ENV_PLUGIN_CONFIG, ENV_PLUGIN_STATE_DIR,
    MAX_EXCHANGES, MAX_EXAMPLE_TURNS, MAX_TALKATIVENESS_OFFSET, MIN_TALKATIVENESS_OFFSET,
    SOUL_TUNING_CONFIG_VERSION, SoulModeSpec, SoulTuningConfig, clamp_talkativeness_offset,
    default_modes, load_active_mode, resolve_state_dir_from_env, save_active_mode,
};

/// Default Waist-2 body identifier for `plugin-soul-tuning`.
pub const DEFAULT_BODY_ID: &str = "plugin-soul-tuning";

/// Default declared capability place owned by `plugin-soul-tuning`.
pub const DEFAULT_PLACE: &str = "cap:soul-tuning";

/// Capability ID for reading the active speech mode and its rule/offset/examples.
pub const CAP_GET_MODE: &str = "soul_tuning.get_mode";

/// Capability ID for listing all configured speech modes.
pub const CAP_LIST_MODES: &str = "soul_tuning.list_modes";

/// Capability ID for switching the active speech mode (`identity_changing: true`).
pub const CAP_SET_MODE: &str = "soul_tuning.set_mode";

#[derive(Debug, Deserialize)]
struct SetModeInput {
    mode: String,
}

/// Waist-2 body implementation for `plugin-soul-tuning`.
pub struct SoulTuningBody {
    body_id: BodyId,
    place: PlaceId,
    config: SoulTuningConfig,
    state_dir: Option<PathBuf>,
    active_mode: String,
    held_actions: HashSet<ActionId>,
    thresholds: HonestCueThresholds,
    clock_now_ms: u64,
}

impl SoulTuningBody {
    /// Creates a [`SoulTuningBody`] loading configuration from `MONOAGENS_PLUGIN_CONFIG`
    /// and state directory from `MONOAGENS_PLUGIN_STATE_DIR` / `MONOAGENS_BODY_STATE_DIR`.
    pub fn from_env() -> Result<Self, String> {
        let config = SoulTuningConfig::load_or_init_from_env()?;
        let state_dir = resolve_state_dir_from_env();
        Ok(Self::new(config, state_dir))
    }

    /// Creates a [`SoulTuningBody`] with the default `body_id` and `cap:soul-tuning` place.
    #[must_use]
    pub fn new(config: SoulTuningConfig, state_dir: Option<PathBuf>) -> Self {
        Self::with_ids(
            BodyId::new(DEFAULT_BODY_ID),
            PlaceId::new(DEFAULT_PLACE),
            config,
            state_dir,
        )
    }

    /// Creates a [`SoulTuningBody`] with explicit `body_id` and `place`.
    #[must_use]
    pub fn with_ids(
        body_id: BodyId,
        place: PlaceId,
        config: SoulTuningConfig,
        state_dir: Option<PathBuf>,
    ) -> Self {
        let active_mode = load_active_mode(state_dir.as_deref(), &config);
        Self {
            body_id,
            place,
            config,
            state_dir,
            active_mode,
            held_actions: HashSet::new(),
            thresholds: HonestCueThresholds::default(),
            clock_now_ms: 0,
        }
    }

    /// Returns the owned capability [`PlaceId`].
    #[must_use]
    pub fn place(&self) -> &PlaceId {
        &self.place
    }

    /// Returns the currently active mode name.
    #[must_use]
    pub fn active_mode(&self) -> &str {
        &self.active_mode
    }

    /// Returns the loaded [`SoulTuningConfig`].
    #[must_use]
    pub fn config(&self) -> &SoulTuningConfig {
        &self.config
    }

    /// Updates the local clock timestamp used on emitted receipts.
    pub fn set_now_ms(&mut self, now_ms: u64) {
        self.clock_now_ms = now_ms;
    }

    fn capabilities(&self) -> Vec<CapabilityDescriptor> {
        let get_schema = json!({
            "capability": CAP_GET_MODE,
            "input": {
                "type": "object",
                "properties": {},
                "additionalProperties": false
            },
            "place": self.place.as_str()
        });
        let list_schema = json!({
            "capability": CAP_LIST_MODES,
            "input": {
                "type": "object",
                "properties": {},
                "additionalProperties": false
            },
            "place": self.place.as_str()
        });
        let set_schema = json!({
            "capability": CAP_SET_MODE,
            "input": {
                "type": "object",
                "required": ["mode"],
                "properties": {
                    "mode": { "type": "string" }
                },
                "additionalProperties": false
            },
            "place": self.place.as_str()
        });

        let mut caps = vec![
            CapabilityDescriptor {
                id: CapabilityId::new(CAP_GET_MODE),
                description:
                    "Return the active speech mode, rule, talkativeness offset, examples, and available modes"
                        .to_owned(),
                places: vec![self.place.clone()],
                flags: CapabilityFlags {
                    communicative: false,
                    identity_changing: false,
                    irreversible: false,
                },
                typical_latency_ms: 1,
                capability_schema_hash: capability_schema_hash(&get_schema),
                sensed_effect: None,
            },
            CapabilityDescriptor {
                id: CapabilityId::new(CAP_LIST_MODES),
                description:
                    "List all configured speech modes with their rules, talkativeness offsets, and example counts"
                        .to_owned(),
                places: vec![self.place.clone()],
                flags: CapabilityFlags {
                    communicative: false,
                    identity_changing: false,
                    irreversible: false,
                },
                typical_latency_ms: 1,
                capability_schema_hash: capability_schema_hash(&list_schema),
                sensed_effect: None,
            },
            CapabilityDescriptor {
                id: CapabilityId::new(CAP_SET_MODE),
                description:
                    "Switch the active speech mode (owner-only identity_changing capability)"
                        .to_owned(),
                places: vec![self.place.clone()],
                flags: CapabilityFlags {
                    communicative: false,
                    identity_changing: true,
                    irreversible: false,
                },
                typical_latency_ms: 2,
                capability_schema_hash: capability_schema_hash(&set_schema),
                sensed_effect: None,
            },
        ];
        sort_capabilities(&mut caps);
        caps
    }

    fn execute_get_mode(&self, is_delivery_probe: bool) -> Value {
        let spec = &self.config.modes[&self.active_mode];
        let available_modes: Vec<&str> = self.config.modes.keys().map(String::as_str).collect();
        let mut detail = json!({
            "active_mode": self.active_mode,
            "rule": spec.rule,
            "talkativeness_offset": spec.talkativeness_offset,
            "examples": spec.examples,
            "available_modes": available_modes,
        });
        if is_delivery_probe && let Some(obj) = detail.as_object_mut() {
            obj.insert("d_art_ms".to_owned(), json!(self.thresholds.d_art_ms));
            obj.insert(
                "working_cue_ms".to_owned(),
                json!(self.thresholds.working_cue_ms),
            );
            obj.insert(
                "progress_note_ms".to_owned(),
                json!(self.thresholds.progress_note_ms),
            );
        }
        detail
    }

    fn execute_list_modes(&self) -> Value {
        let modes: Vec<Value> = self
            .config
            .modes
            .iter()
            .map(|(name, spec)| {
                json!({
                    "name": name,
                    "rule": spec.rule,
                    "talkativeness_offset": spec.talkativeness_offset,
                    "example_count": spec.examples.len(),
                })
            })
            .collect();
        json!({
            "active_mode": self.active_mode,
            "modes": modes,
        })
    }

    fn execute_set_mode(&mut self, params: ActionExecuteParams) -> ActionExecuteResult {
        let now = Time::from_millis(self.clock_now_ms);
        let parsed: SetModeInput = match serde_json::from_slice(params.payload.as_bytes()) {
            Ok(v) => v,
            Err(err) => {
                return refused_result(
                    params,
                    "ERR_INVALID_PAYLOAD",
                    format!("expected JSON object {{\"mode\": \"<name>\"}}: {err}"),
                    now,
                );
            }
        };

        let target_mode = parsed.mode.trim();
        if target_mode.is_empty() {
            return refused_result(
                params,
                "ERR_INVALID_PAYLOAD",
                "mode must not be empty",
                now,
            );
        }

        let Some(spec) = self.config.modes.get(target_mode).cloned() else {
            return refused_result(
                params,
                "ERR_UNKNOWN_MODE",
                format!("unknown soul tuning mode '{target_mode}'"),
                now,
            );
        };

        if let Err(err) = save_active_mode(self.state_dir.as_deref(), target_mode) {
            let receipt = make_terminal_receipt(
                params.action_id,
                params.capability_id,
                ReceiptOutcome::Failed,
                Some("ERR_STATE_PERSIST".to_owned()),
                Some(err),
                Some(TerminationReason::Normal),
                Some(0.0),
                Some(0),
                params.h_act,
                now,
            );
            return ActionExecuteResult {
                accepted: true,
                receipt: Some(receipt),
            };
        }

        let previous_mode = self.active_mode.clone();
        self.active_mode = target_mode.to_owned();

        let detail = json!({
            "previous_mode": previous_mode,
            "active_mode": self.active_mode,
            "rule": spec.rule,
            "talkativeness_offset": spec.talkativeness_offset,
        });
        let receipt = make_terminal_receipt(
            params.action_id,
            params.capability_id,
            ReceiptOutcome::Done,
            None,
            Some(detail.to_string()),
            Some(TerminationReason::Normal),
            Some(1.0),
            Some(1),
            params.h_act,
            now,
        );
        ActionExecuteResult {
            accepted: true,
            receipt: Some(receipt),
        }
    }
}

fn refused_result(
    params: ActionExecuteParams,
    reason_code: &str,
    detail: impl Into<String>,
    now: Time,
) -> ActionExecuteResult {
    let receipt = make_refused_receipt(
        params.action_id,
        params.capability_id,
        reason_code,
        detail,
        params.h_act,
        now,
    );
    ActionExecuteResult {
        accepted: false,
        receipt: Some(receipt),
    }
}

impl BodyHandler for SoulTuningBody {
    fn discover(&mut self, params: BodyDiscoverParams) -> Result<BodyDiscoverResult, Waist2Error> {
        Ok(BodyDiscoverResult {
            body_id: params.body_id.unwrap_or_else(|| self.body_id.clone()),
            protocol_version: CURRENT_PROTOCOL_VERSION,
            capabilities: self.capabilities(),
            places: vec![self.place.clone()],
        })
    }

    fn execute(&mut self, params: ActionExecuteParams) -> Result<ActionExecuteResult, Waist2Error> {
        if params.audience.is_empty() {
            return Err(Waist2Error::undeclared_audience());
        }

        let cap = params.capability_id.as_str();
        if !matches!(cap, CAP_GET_MODE | CAP_LIST_MODES | CAP_SET_MODE) {
            return Err(Waist2Error::unknown_capability(&params.capability_id));
        }

        let place_ok = !params.places.is_empty() && params.places.iter().all(|p| p == &self.place);
        if !place_ok {
            return Ok(refused_result(
                params,
                "ERR_UNKNOWN_PLACE",
                "action targets a place not owned by plugin-soul-tuning",
                Time::from_millis(self.clock_now_ms),
            ));
        }

        if params.h_act.is_some() && params.payload.as_bytes() == b"hold" {
            self.held_actions.insert(params.action_id.clone());
            let receipt = make_terminal_receipt(
                params.action_id,
                params.capability_id,
                ReceiptOutcome::Held,
                Some("PendingApproval".to_owned()),
                Some("awaiting owner approval before soul-tuning execution".to_owned()),
                Some(TerminationReason::Normal),
                Some(0.0),
                Some(0),
                params.h_act,
                Time::from_millis(self.clock_now_ms),
            );
            return Ok(ActionExecuteResult {
                accepted: true,
                receipt: Some(receipt),
            });
        }

        let is_delivery_probe = params.payload.as_bytes() == b"delivery-timing-probe"
            || params.headers.meta.get("org.monoagens.delivery_probe")
                == Some(&MetaValue::Bool(true));

        let now = Time::from_millis(self.clock_now_ms);
        match cap {
            CAP_GET_MODE => {
                let detail = self.execute_get_mode(is_delivery_probe);
                let receipt = make_terminal_receipt(
                    params.action_id,
                    params.capability_id,
                    ReceiptOutcome::Done,
                    None,
                    Some(detail.to_string()),
                    Some(TerminationReason::Normal),
                    Some(1.0),
                    Some(1),
                    params.h_act,
                    now,
                );
                Ok(ActionExecuteResult {
                    accepted: true,
                    receipt: Some(receipt),
                })
            }
            CAP_LIST_MODES => {
                let detail = self.execute_list_modes();
                let receipt = make_terminal_receipt(
                    params.action_id,
                    params.capability_id,
                    ReceiptOutcome::Done,
                    None,
                    Some(detail.to_string()),
                    Some(TerminationReason::Normal),
                    Some(1.0),
                    Some(1),
                    params.h_act,
                    now,
                );
                Ok(ActionExecuteResult {
                    accepted: true,
                    receipt: Some(receipt),
                })
            }
            CAP_SET_MODE => Ok(self.execute_set_mode(params)),
            _ => Err(Waist2Error::unknown_capability(&params.capability_id)),
        }
    }

    fn cancel(&mut self, params: ActionCancelParams) -> Result<ActionCancelResult, Waist2Error> {
        self.held_actions.remove(&params.action_id);
        let receipt = make_terminal_receipt(
            params.action_id,
            CapabilityId::new(CAP_GET_MODE),
            ReceiptOutcome::Failed,
            Some("canceled_by_core".to_owned()),
            params.reason,
            Some(TerminationReason::CanceledByCore),
            Some(0.0),
            Some(0),
            None,
            Time::from_millis(self.clock_now_ms),
        );
        Ok(ActionCancelResult {
            cancelled: true,
            receipt: Some(receipt),
        })
    }

    fn resolve_approval(
        &mut self,
        params: ApprovalResolveParams,
    ) -> Result<ApprovalResolveResult, Waist2Error> {
        let applied = self.held_actions.remove(&params.action_id);
        Ok(ApprovalResolveResult { applied })
    }

    fn on_percept(&mut self, params: PerceptEmitParams) -> Result<PerceptEmitResult, Waist2Error> {
        if params.audience.as_ref().is_none_or(Audience::is_empty) {
            return Err(Waist2Error::undeclared_audience());
        }
        if params.place != self.place {
            return Err(Waist2Error::invalid_receipt_transition(format!(
                "percept place '{}' is not owned by plugin-soul-tuning",
                params.place
            )));
        }
        Ok(PerceptEmitResult { accepted: true })
    }
}
