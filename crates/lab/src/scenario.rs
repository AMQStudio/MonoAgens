//! Scenario declarations for Lab test bodies, scripted events, model responses,
//! ground-truth annotations, and safe filesystem loading.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use monoagens_core::CoreConfig;
use monoagens_protocol::{
    ActionId, ActorFacts, ActorId, Audience, BodyId, BodyState, CapabilityDescriptor, CapabilityId,
    ChannelId, OpaquePayload, OwnerSigned, Percept, PlaceId, ReceiptOutcome, TerminationReason,
    ThoughtAct,
};
use serde::{Deserialize, Serialize};

use crate::LabError;
use crate::checks::ExpectationCheck;

/// Scenario partition set (`specs/lab/spec.md`, `benchmark-evaluation/spec.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScenarioSet {
    Development,
    Validation,
    Sealed,
    External,
}

/// Warranted reply ground-truth annotation (`specs/lab/spec.md` L131).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WarrantedReply {
    Speak,
    Silent,
}

/// Per-event ground-truth annotation keyed by scripted event index in `Scenario::events`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct EventAnnotation {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warranted: Option<WarrantedReply>,
    #[serde(default)]
    pub sel_other: bool,
    #[serde(default)]
    pub answered: bool,
    #[serde(default)]
    pub addressed_to_soul: bool,
    #[serde(default)]
    pub unfamiliar_place: bool,
    #[serde(default)]
    pub double_answer_eligible: bool,
    #[serde(default)]
    pub low_value_if_spoken: bool,
    #[serde(default)]
    pub non_uptake_expected_repair: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub causal_label: Option<String>,
}

/// Recall probe ground-truth annotation for 1/7/30-day gap & memory-truth metrics (Rows 10, 16).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecallProbeAnnotation {
    pub gap_days: u32,
    pub expected_event_indices: Vec<usize>,
    #[serde(default)]
    pub familiar_only_r0: bool,
    #[serde(default)]
    pub detail_asserted_without_r: bool,
    #[serde(default)]
    pub source_misattributed: bool,
    #[serde(default)]
    pub drm_lure_error: bool,
    #[serde(default)]
    pub stale_belief_presented: bool,
    #[serde(default)]
    pub blended_memory_error: bool,
}

/// Report-control probe annotation (Row 17).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReportControlProbe {
    pub wrong_volunteered: bool,
    #[serde(default)]
    pub withheld: bool,
    #[serde(default)]
    pub coarse: bool,
}

/// False-belief question probe annotation (Row 19).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FalseBeliefProbe {
    pub question_id: String,
    pub answered_correctly: bool,
}

/// Scenario-level ground-truth annotations for computing the 22 brain-map and safety metrics
/// without inventing values for unexercised rows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ScenarioAnnotations {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub event_annotations: BTreeMap<usize, EventAnnotation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub participant_count: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub daily_budget_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub human_reply_delays_ms: Vec<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recall_probes: Vec<RecallProbeAnnotation>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub report_control_probes: Vec<ReportControlProbe>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub false_belief_probes: Vec<FalseBeliefProbe>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub episode_boundaries_ms: Vec<u64>,
    #[serde(default)]
    pub boundary_tolerance_ms: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub downtime_windows_ms: Vec<(u64, u64)>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delta_ni_measured: Option<f64>,
}

/// Scripted follow-up percept emitted by a `TestBody` after an action executes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScriptedFollowUpPercept {
    pub delay_ms: u64,
    pub actor: ActorId,
    pub place: PlaceId,
    pub audience: Audience,
    pub payload: Vec<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<ChannelId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delta_obs: Option<f64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub addressees: Vec<ActorId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub named: Vec<ActorId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dialogue_act_gap: Option<f32>,
}

/// Scripted action receipt and optional sensor follow-up percepts for a `TestBody`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScriptedActionRule {
    pub capability_id: CapabilityId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub place: Option<PlaceId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload_contains: Option<String>,
    pub outcome: ReceiptOutcome,
    #[serde(default)]
    pub receipt_delay_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub termination_reason: Option<TerminationReason>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivered_units: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub follow_up_percepts: Vec<ScriptedFollowUpPercept>,
}

/// Declarative specification of a `TestBody` in a scenario.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TestBodySpec {
    pub body_id: BodyId,
    pub places: Vec<PlaceId>,
    pub capabilities: Vec<CapabilityDescriptor>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub action_scripts: Vec<ScriptedActionRule>,
}

/// Scripted event payload scheduled on the Lab virtual clock.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ScriptedEventKind {
    Percept(Box<Percept>),
    BodyState(BodyState),
    ActorFacts(ActorFacts),
    Identity(OwnerSigned<OpaquePayload>),
    CancelAction {
        action_id: ActionId,
        termination_reason: TerminationReason,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason_code: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        detail: Option<String>,
    },
    Tick,
    TriggerSleep,
    RecordCalibration {
        conf_0_20: u8,
        correct: bool,
    },
    SetDailyBudget {
        used: f64,
        limit: f64,
    },
    HardForget {
        #[serde(default)]
        event_indices: Vec<usize>,
        #[serde(default)]
        actor_ids: Vec<ActorId>,
        reason: String,
    },
}

/// A single timestamped event in a scenario script.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScriptedEvent {
    pub at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body_id: Option<BodyId>,
    pub event: ScriptedEventKind,
}

/// Scripted model response consumed deterministically by wake index.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScriptedModelResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wake_index: Option<usize>,
    pub acts: Vec<ThoughtAct>,
    #[serde(default)]
    pub tokens_in: u64,
    #[serde(default)]
    pub tokens_out: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fail_reason: Option<String>,
}

/// Optional cost-guard bounds installed at the context door.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct CostGuardConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_chain_wakes: Option<u64>,
}

/// Complete declarative Lab scenario.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Scenario {
    pub id: String,
    pub name: String,
    pub set: ScenarioSet,
    #[serde(default)]
    pub start_time_ms: u64,
    #[serde(default)]
    pub model_latency_ms: u64,
    #[serde(default = "default_model_id")]
    pub model_id: String,
    #[serde(default = "default_embedder_id")]
    pub embedder_id: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bodies: Vec<TestBodySpec>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<ScriptedEvent>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scripted_model: Vec<ScriptedModelResponse>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub checks: Vec<ExpectationCheck>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub core_config: Option<CoreConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_path: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub online_adaptation: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_guard: Option<CostGuardConfig>,
    #[serde(default)]
    pub annotations: ScenarioAnnotations,
}

fn default_model_id() -> String {
    "scripted".to_owned()
}

fn default_embedder_id() -> String {
    "none".to_owned()
}

/// Validates that `path` never references `sealed` (`lab/scenarios/sealed/` MUST NOT be read or enumerated).
pub fn ensure_not_sealed_path(path: &Path) -> Result<(), LabError> {
    for comp in path.components() {
        if comp
            .as_os_str()
            .to_string_lossy()
            .eq_ignore_ascii_case("sealed")
        {
            return Err(LabError::Protocol(format!(
                "refusing to read or enumerate sealed scenario path '{}'",
                path.display()
            )));
        }
    }
    Ok(())
}

/// Loads all `.json` scenario files in deterministic sorted filename order from `lab_scenarios_root/<set>/`.
///
/// Refuses [`ScenarioSet::Sealed`] or any path containing a `sealed` component before touching disk.
pub fn load_scenarios_from_set(
    lab_scenarios_root: &Path,
    set: ScenarioSet,
) -> Result<Vec<Scenario>, LabError> {
    ensure_not_sealed_path(lab_scenarios_root)?;
    let subdir = match set {
        ScenarioSet::Development => "development",
        ScenarioSet::Validation => "validation",
        ScenarioSet::Sealed => {
            return Err(LabError::Protocol(
                "refusing to enumerate or read lab/scenarios/sealed/".to_owned(),
            ));
        }
        ScenarioSet::External => {
            return Err(LabError::Protocol(
                "external benchmark datasets are loaded via lab/datasets.manifest.toml, not lab/scenarios/".to_owned(),
            ));
        }
    };
    let dir = lab_scenarios_root.join(subdir);
    ensure_not_sealed_path(&dir)?;
    if !dir.is_dir() {
        return Err(LabError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("scenario set directory '{}' does not exist", dir.display()),
        )));
    }
    let mut paths = Vec::new();
    for entry in fs::read_dir(&dir)? {
        let entry = entry?;
        let p = entry.path();
        if p.is_file() && p.extension().and_then(|e| e.to_str()) == Some("json") {
            paths.push(p);
        }
    }
    paths.sort();

    let mut scenarios = Vec::with_capacity(paths.len());
    for p in paths {
        let raw = fs::read_to_string(&p)?;
        let sc: Scenario = serde_json::from_str(&raw).map_err(|e| {
            LabError::Serialization(format!("failed to parse scenario '{}': {e}", p.display()))
        })?;
        if sc.set != set {
            return Err(LabError::Protocol(format!(
                "scenario '{}' declared set {:?} inside {:?} directory",
                p.display(),
                sc.set,
                set
            )));
        }
        scenarios.push(sc);
    }
    Ok(scenarios)
}
