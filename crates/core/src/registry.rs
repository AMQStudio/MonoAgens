//! Brain-map mechanism registry and [`Switches`] covering all 22 rows of `design.md` §18.
//!
//! Every mechanism carries a stable id, function name, `RESEARCH.md` section,
//! falsifiable causal claim ("what gets worse without it"), and reachable neutral value.
//! Setting a mechanism to its neutral value disables its effect without altering
//! the single cognitive loop's control flow.

use core::fmt;
use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

/// Canonical identifiers for the 22 cognitive mechanisms in `design.md` §18.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MechanismId {
    Salience,
    FloorCost,
    Habituation,
    DriveThreshold,
    FamiliarityReserve,
    StochasticGoNoGo,
    CommitmentBreaker,
    Patience,
    ReportBarOnThought,
    ActivationRecall,
    VectorFade,
    PredictionAndGrounding,
    ProspectiveMemory,
    Credence,
    BoundedLearning,
    MemoryTruth,
    ReportControl,
    ConsolidationAndMerge,
    PerceptionLedger,
    Agency,
    EventBoundaries,
    BacklogChunking,
}

impl MechanismId {
    /// All 22 canonical mechanisms in `design.md` §18 row order.
    pub const ALL: [Self; 22] = [
        Self::Salience,
        Self::FloorCost,
        Self::Habituation,
        Self::DriveThreshold,
        Self::FamiliarityReserve,
        Self::StochasticGoNoGo,
        Self::CommitmentBreaker,
        Self::Patience,
        Self::ReportBarOnThought,
        Self::ActivationRecall,
        Self::VectorFade,
        Self::PredictionAndGrounding,
        Self::ProspectiveMemory,
        Self::Credence,
        Self::BoundedLearning,
        Self::MemoryTruth,
        Self::ReportControl,
        Self::ConsolidationAndMerge,
        Self::PerceptionLedger,
        Self::Agency,
        Self::EventBoundaries,
        Self::BacklogChunking,
    ];

    /// Stable string identifier for serialization and Lab run records.
    #[must_use]
    pub const fn stable_id(self) -> &'static str {
        match self {
            Self::Salience => "m01_salience",
            Self::FloorCost => "m02_floor_cost",
            Self::Habituation => "m03_habituation",
            Self::DriveThreshold => "m04_drive_threshold",
            Self::FamiliarityReserve => "m05_familiarity_reserve",
            Self::StochasticGoNoGo => "m06_stochastic_go_no_go",
            Self::CommitmentBreaker => "m07_commitment_breaker",
            Self::Patience => "m08_patience",
            Self::ReportBarOnThought => "m09_report_bar_on_thought",
            Self::ActivationRecall => "m10_activation_recall",
            Self::VectorFade => "m11_vector_fade",
            Self::PredictionAndGrounding => "m12_prediction_and_grounding",
            Self::ProspectiveMemory => "m13_prospective_memory",
            Self::Credence => "m14_credence",
            Self::BoundedLearning => "m15_bounded_learning",
            Self::MemoryTruth => "m16_memory_truth",
            Self::ReportControl => "m17_report_control",
            Self::ConsolidationAndMerge => "m18_consolidation_and_merge",
            Self::PerceptionLedger => "m19_perception_ledger",
            Self::Agency => "m20_agency",
            Self::EventBoundaries => "m21_event_boundaries",
            Self::BacklogChunking => "m22_backlog_chunking",
        }
    }

    /// 1-based row index in `design.md` §18 (`1..=22`).
    #[must_use]
    pub const fn row(self) -> u8 {
        match self {
            Self::Salience => 1,
            Self::FloorCost => 2,
            Self::Habituation => 3,
            Self::DriveThreshold => 4,
            Self::FamiliarityReserve => 5,
            Self::StochasticGoNoGo => 6,
            Self::CommitmentBreaker => 7,
            Self::Patience => 8,
            Self::ReportBarOnThought => 9,
            Self::ActivationRecall => 10,
            Self::VectorFade => 11,
            Self::PredictionAndGrounding => 12,
            Self::ProspectiveMemory => 13,
            Self::Credence => 14,
            Self::BoundedLearning => 15,
            Self::MemoryTruth => 16,
            Self::ReportControl => 17,
            Self::ConsolidationAndMerge => 18,
            Self::PerceptionLedger => 19,
            Self::Agency => 20,
            Self::EventBoundaries => 21,
            Self::BacklogChunking => 22,
        }
    }
}

impl fmt::Display for MechanismId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.stable_id())
    }
}

/// Reachable neutral value representation for a registered mechanism.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum NeutralValue {
    Bool(bool),
    Float(f64),
    FloatPair(f64, f64),
    FloatTriple(f64, f64, f64),
    DurationMs(u64),
    NegativeInfinity,
    ZeroTemperatureStepLimit,
    DisabledMode(String),
}

/// Errors produced when constructing or validating a mechanism registry entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistryError {
    MissingClaim {
        mechanism: String,
    },
    MissingNeutralValue {
        mechanism: String,
    },
    MissingField {
        mechanism: String,
        field: &'static str,
    },
    DuplicateEntry {
        mechanism: String,
    },
    IncompleteRegistry {
        expected: usize,
        found: usize,
    },
}

impl fmt::Display for RegistryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingClaim { mechanism } => {
                write!(
                    f,
                    "mechanism '{mechanism}' cannot be registered without a causal claim"
                )
            }
            Self::MissingNeutralValue { mechanism } => {
                write!(
                    f,
                    "mechanism '{mechanism}' cannot be registered without a neutral value"
                )
            }
            Self::MissingField { mechanism, field } => {
                write!(
                    f,
                    "mechanism '{mechanism}' is missing required registry field '{field}'"
                )
            }
            Self::DuplicateEntry { mechanism } => {
                write!(f, "duplicate mechanism registration for '{mechanism}'")
            }
            Self::IncompleteRegistry { expected, found } => {
                write!(
                    f,
                    "incomplete mechanism registry: expected {expected} entries, found {found}"
                )
            }
        }
    }
}

impl std::error::Error for RegistryError {}

/// Documented brain-map registry entry for one cognitive mechanism.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MechanismEntry {
    pub id: MechanismId,
    pub row: u8,
    pub stable_id: String,
    pub function_name: String,
    pub mechanism: String,
    pub research_section: String,
    pub causal_claim: String,
    pub neutral_value: NeutralValue,
    pub neutral_description: String,
}

impl MechanismEntry {
    /// Constructs and validates a [`MechanismEntry`], failing if `causal_claim`,
    /// `neutral_value`, or any metadata field is absent or blank.
    #[allow(clippy::too_many_arguments)]
    pub fn try_build(
        id: MechanismId,
        row: u8,
        stable_id: &str,
        function_name: &str,
        mechanism: &str,
        research_section: &str,
        causal_claim: Option<&str>,
        neutral_value: Option<NeutralValue>,
        neutral_description: Option<&str>,
    ) -> Result<Self, RegistryError> {
        let mech_label = if stable_id.trim().is_empty() {
            id.stable_id().to_owned()
        } else {
            stable_id.to_owned()
        };

        let claim = causal_claim
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| RegistryError::MissingClaim {
                mechanism: mech_label.clone(),
            })?;

        let neutral = neutral_value.ok_or_else(|| RegistryError::MissingNeutralValue {
            mechanism: mech_label.clone(),
        })?;

        let neutral_desc = neutral_description
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| RegistryError::MissingNeutralValue {
                mechanism: mech_label.clone(),
            })?;

        if stable_id.trim().is_empty() {
            return Err(RegistryError::MissingField {
                mechanism: mech_label,
                field: "stable_id",
            });
        }
        if function_name.trim().is_empty() {
            return Err(RegistryError::MissingField {
                mechanism: mech_label,
                field: "function_name",
            });
        }
        if mechanism.trim().is_empty() {
            return Err(RegistryError::MissingField {
                mechanism: mech_label,
                field: "mechanism",
            });
        }
        if research_section.trim().is_empty() {
            return Err(RegistryError::MissingField {
                mechanism: mech_label,
                field: "research_section",
            });
        }
        if let NeutralValue::DisabledMode(ref label) = neutral
            && label.trim().is_empty()
        {
            return Err(RegistryError::MissingNeutralValue {
                mechanism: mech_label,
            });
        }

        Ok(Self {
            id,
            row,
            stable_id: stable_id.to_owned(),
            function_name: function_name.to_owned(),
            mechanism: mechanism.to_owned(),
            research_section: research_section.to_owned(),
            causal_claim: claim.to_owned(),
            neutral_value: neutral,
            neutral_description: neutral_desc.to_owned(),
        })
    }
}

/// Compile-time metadata row for each of the 22 mechanisms in `design.md` §18.
#[derive(Debug, Clone, Copy)]
struct CompileTimeSpec {
    id: MechanismId,
    function_name: &'static str,
    mechanism: &'static str,
    research_section: &'static str,
    causal_claim: &'static str,
    neutral_description: &'static str,
}

const fn spec_for(id: MechanismId) -> CompileTimeSpec {
    match id {
        MechanismId::Salience => CompileTimeSpec {
            id,
            function_name: "Salience",
            mechanism: "weighted signals -> S",
            research_section: "RESEARCH.md §2, §3",
            causal_claim: "when-to-speak recall, missed help",
            neutral_description: "all w_k = 0",
        },
        MechanismId::FloorCost => CompileTimeSpec {
            id,
            function_name: "Floor cost",
            mechanism: "social_cost (R-29 deg + answered)",
            research_section: "RESEARCH.md §2, §3 (legacy §2, §15)",
            causal_claim: "floor violations per 100",
            neutral_description: "w_sc = 0 (reachable)",
        },
        MechanismId::Habituation => CompileTimeSpec {
            id,
            function_name: "Habituation",
            mechanism: "partitioned h_{(p,k)}, post-eval S/(1+kappa*h_pre)",
            research_section: "RESEARCH.md §3",
            causal_claim: "wakes per repeated inconsequential stimulus",
            neutral_description: "kappa = 0",
        },
        MechanismId::DriveThreshold => CompileTimeSpec {
            id,
            function_name: "Drive threshold",
            mechanism: "theta terms a, b, c + opportunity costs (R-31)",
            research_section: "RESEARCH.md §3",
            causal_claim: "rate vs 1/n; budget overruns; overdue promises",
            neutral_description: "a = b = c = 0 (reachable)",
        },
        MechanismId::FamiliarityReserve => CompileTimeSpec {
            id,
            function_name: "Familiarity reserve",
            mechanism: "r * (1 - familiarity)",
            research_section: "RESEARCH.md §3 (legacy §3, §15)",
            causal_claim: "false speaks in new places",
            neutral_description: "r = 0",
        },
        MechanismId::StochasticGoNoGo => CompileTimeSpec {
            id,
            function_name: "Stochastic go / no-go",
            mechanism: "seeded sigma draw + tau->0 step limit",
            research_section: "RESEARCH.md §3",
            causal_claim: "distance to human timing",
            neutral_description: "tau -> 0 (S_eff >= theta)",
        },
        MechanismId::CommitmentBreaker => CompileTimeSpec {
            id,
            function_name: "Commitment breaker",
            mechanism: "bypass on turn, expectation, goal",
            research_section: "RESEARCH.md §3 (legacy §3, §15)",
            causal_claim: "unanswered allocated turns",
            neutral_description: "off",
        },
        MechanismId::Patience => CompileTimeSpec {
            id,
            function_name: "Patience",
            mechanism: "shrunk quantile wait expectation (A-08)",
            research_section: "RESEARCH.md §4",
            causal_claim: "double answers / floor collisions",
            neutral_description: "0 s",
        },
        MechanismId::ReportBarOnThought => CompileTimeSpec {
            id,
            function_name: "Report bar on thought",
            mechanism: "speaking & certainty bars (withheld logged)",
            research_section: "RESEARCH.md §4",
            causal_claim: "low-value messages",
            neutral_description: "speaking_bar = 1, certainty_bar = 0",
        },
        MechanismId::ActivationRecall => CompileTimeSpec {
            id,
            function_name: "Activation recall",
            mechanism: "w_B*B + context + importance (A-01)",
            research_section: "RESEARCH.md §5",
            causal_claim: "recall at 1/7/30 days",
            neutral_description: "w_B = 0 (similarity only)",
        },
        MechanismId::VectorFade => CompileTimeSpec {
            id,
            function_name: "Vector fade",
            mechanism: "release vectors where Petrov B < -3.29",
            research_section: "RESEARCH.md §5",
            causal_claim: "storage and query latency",
            neutral_description: "threshold = -inf",
        },
        MechanismId::PredictionAndGrounding => CompileTimeSpec {
            id,
            function_name: "Prediction and grounding",
            mechanism: "predictions after acts and chosen silences",
            research_section: "RESEARCH.md §6 (legacy §6, §17, §20)",
            causal_claim: "unnoticed non-uptake / missed silences",
            neutral_description: "none",
        },
        MechanismId::ProspectiveMemory => CompileTimeSpec {
            id,
            function_name: "Prospective memory",
            mechanism: "typed promise + RecurrenceSpec + CommitCheck",
            research_section: "RESEARCH.md §6",
            causal_claim: "promise keeping & conflict rate",
            neutral_description: "left to reasoning",
        },
        MechanismId::Credence => CompileTimeSpec {
            id,
            function_name: "Credence",
            mechanism: "beta counts (told incl. + lambda_domain)",
            research_section: "RESEARCH.md §7",
            causal_claim: "Brier / ECE calibration",
            neutral_description: "frozen",
        },
        MechanismId::BoundedLearning => CompileTimeSpec {
            id,
            function_name: "Bounded learning",
            mechanism: "sign-corrected & IPS-weighted delta rule",
            research_section: "RESEARCH.md §7 (legacy §8)",
            causal_claim: "learning plateau over weeks",
            neutral_description: "learning_rate = 0",
        },
        MechanismId::MemoryTruth => CompileTimeSpec {
            id,
            function_name: "Memory truth",
            mechanism: "basis; dual-process R/F; event beats gist; valid_to",
            research_section: "RESEARCH.md §5, §8 (legacy §12, §14)",
            causal_claim: "misattribution; confabulation; lures; stale beliefs",
            neutral_description: "each off",
        },
        MechanismId::ReportControl => CompileTimeSpec {
            id,
            function_name: "Report control",
            mechanism: "0/0-guarded P_rc vs [0,20] c, grain",
            research_section: "RESEARCH.md §4 (legacy §14)",
            causal_claim: "wrong volunteered answers",
            neutral_description: "P_rc = 0",
        },
        MechanismId::ConsolidationAndMerge => CompileTimeSpec {
            id,
            function_name: "Consolidation and merge",
            mechanism: "async {open, provisional, sealed} gists; overlap merge",
            research_section: "RESEARCH.md §8 (legacy §12, §14)",
            causal_claim: "30-day recall; spin loops; blended memories",
            neutral_description: "none; never",
        },
        MechanismId::PerceptionLedger => CompileTimeSpec {
            id,
            function_name: "Perception ledger",
            mechanism: "promisee-aware cards & 4-pillar self_card (R-35)",
            research_section: "RESEARCH.md §7 (legacy §18)",
            causal_claim: "false-belief errors & self-claim contradictions",
            neutral_description: "absent",
        },
        MechanismId::Agency => CompileTimeSpec {
            id,
            function_name: "Agency",
            mechanism: "(channel, delta_obs, delta_pred) effect predictions",
            research_section: "RESEARCH.md §6 (legacy §19)",
            causal_claim: "reactions to own echoes",
            neutral_description: "no marking",
        },
        MechanismId::EventBoundaries => CompileTimeSpec {
            id,
            function_name: "Event boundaries",
            mechanism: "[0,1] normalized EM-LLM surprise + shrinkage",
            research_section: "RESEARCH.md §8 (legacy §21)",
            causal_claim: "boundary F1",
            neutral_description: "fixed interval",
        },
        MechanismId::BacklogChunking => CompileTimeSpec {
            id,
            function_name: "Backlog chunking",
            mechanism: "stopped/down-only chunking (delta_theta_sleep = 0, A-04)",
            research_section: "RESEARCH.md §3 (legacy §15)",
            causal_claim: "bursts after downtime; sleep responsiveness",
            neutral_description: "per event",
        },
    }
}

fn neutral_value_for(id: MechanismId) -> NeutralValue {
    match id {
        MechanismId::Salience => NeutralValue::Float(0.0),
        MechanismId::FloorCost => NeutralValue::Float(0.0),
        MechanismId::Habituation => NeutralValue::Float(0.0),
        MechanismId::DriveThreshold => NeutralValue::FloatTriple(0.0, 0.0, 0.0),
        MechanismId::FamiliarityReserve => NeutralValue::Float(0.0),
        MechanismId::StochasticGoNoGo => NeutralValue::ZeroTemperatureStepLimit,
        MechanismId::CommitmentBreaker => NeutralValue::Bool(false),
        MechanismId::Patience => NeutralValue::DurationMs(0),
        MechanismId::ReportBarOnThought => NeutralValue::FloatPair(1.0, 0.0),
        MechanismId::ActivationRecall => NeutralValue::Float(0.0),
        MechanismId::VectorFade => NeutralValue::NegativeInfinity,
        MechanismId::PredictionAndGrounding => NeutralValue::Bool(false),
        MechanismId::ProspectiveMemory => {
            NeutralValue::DisabledMode("left_to_reasoning".to_owned())
        }
        MechanismId::Credence => NeutralValue::DisabledMode("frozen".to_owned()),
        MechanismId::BoundedLearning => NeutralValue::Float(0.0),
        MechanismId::MemoryTruth => NeutralValue::Bool(false),
        MechanismId::ReportControl => NeutralValue::Float(0.0),
        MechanismId::ConsolidationAndMerge => NeutralValue::Bool(false),
        MechanismId::PerceptionLedger => NeutralValue::Bool(false),
        MechanismId::Agency => NeutralValue::Bool(false),
        MechanismId::EventBoundaries => NeutralValue::DisabledMode("fixed_interval".to_owned()),
        MechanismId::BacklogChunking => NeutralValue::DisabledMode("per_event".to_owned()),
    }
}

/// Builds and validates all 22 brain-map mechanism registry entries.
pub fn brain_map_registry() -> Result<Vec<MechanismEntry>, RegistryError> {
    let mut entries = Vec::with_capacity(MechanismId::ALL.len());
    let mut seen_ids = BTreeSet::new();
    let mut seen_rows = BTreeSet::new();

    for id in MechanismId::ALL {
        let spec = spec_for(id);
        let entry = MechanismEntry::try_build(
            spec.id,
            spec.id.row(),
            spec.id.stable_id(),
            spec.function_name,
            spec.mechanism,
            spec.research_section,
            Some(spec.causal_claim),
            Some(neutral_value_for(spec.id)),
            Some(spec.neutral_description),
        )?;
        if !seen_ids.insert(entry.id) || !seen_rows.insert(entry.row) {
            return Err(RegistryError::DuplicateEntry {
                mechanism: entry.stable_id,
            });
        }
        entries.push(entry);
    }

    if entries.len() != 22 {
        return Err(RegistryError::IncompleteRegistry {
            expected: 22,
            found: entries.len(),
        });
    }
    Ok(entries)
}

/// Mechanism toggle switches for all 22 brain-map rows plus `self_mechanics_guide`.
///
/// Leaving any mechanism at its neutral value keeps the cognitive loop on the same
/// single execution path (`ARCHITECTURE.md`: "The loop stays one loop whatever is switched off").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Switches {
    /// Row 1: weighted signals -> salience `S` (`false` => all `w_k = 0`).
    pub salience: bool,
    /// Row 2: graded address `social_cost` (`false` => `w_sc = 0`).
    pub floor_cost: bool,
    /// Row 3: partitioned habituation `h_{(p,k)}` (`false` => `kappa = 0`).
    pub habituation: bool,
    /// Row 4: threshold drive terms `a, b, c` + opportunity costs (`false` => `a = b = c = 0`).
    pub drive_threshold: bool,
    /// Row 5: familiarity reserve `r * (1 - familiarity)` (`false` => `r = 0`).
    pub familiarity_reserve: bool,
    /// Row 6: stochastic go/no-go temperature (`false` => `tau -> 0` deterministic step limit).
    pub stochastic_go_no_go: bool,
    /// Row 7: commitment circuit breaker (`false` => off).
    pub commitment_breaker: bool,
    /// Row 8: shrunk quantile `wait` patience (`false` => `0 s`).
    pub patience: bool,
    /// Row 9: speaking & certainty bars on thought (`false` => `1 and 0`).
    pub report_bar_on_thought: bool,
    /// Row 10: Petrov activation weighting in recall (`false` => `w_B = 0`, similarity only).
    pub activation_recall: bool,
    /// Row 11: vector fade release for `B < -3.29` (`false` => threshold `-inf`).
    pub vector_fade: bool,
    /// Row 12: predictions after acts and chosen silences (`false` => none).
    pub prediction_and_grounding: bool,
    /// Row 13: typed promises, `RecurrenceSpec`, `CommitCheck` (`false` => left to reasoning).
    pub prospective_memory: bool,
    /// Row 14: Beta credence updates (`false` => frozen).
    pub credence: bool,
    /// Row 15: sign-corrected IPS-weighted delta learning (`false` => `learning_rate = 0`).
    pub bounded_learning: bool,
    /// Row 16: dual-process R/F, basis, event-beats-gist, `valid_to` (`false` => each off).
    pub memory_truth: bool,
    /// Row 17: `0/0`-guarded report control `P_rc` (`false` => `P_rc = 0`).
    pub report_control: bool,
    /// Row 18: async `{open, provisional, sealed}` consolidation & merge (`false` => none; never).
    pub consolidation_and_merge: bool,
    /// Row 19: promisee-aware ledger cards & 4-pillar `self_card` (`false` => absent).
    pub perception_ledger: bool,
    /// Row 20: `(channel, delta_obs, delta_pred)` agency predictions (`false` => no marking).
    pub agency: bool,
    /// Row 21: `[0,1]` EM-LLM boundary surprise + shrinkage (`false` => fixed interval).
    pub event_boundaries: bool,
    /// Row 22: `stopped/down`-only backlog chunking (`false` => per event).
    pub backlog_chunking: bool,
    /// Derived `[SELF_GUIDE]` prompt slot (`design.md` §9, neutral `false`).
    pub self_mechanics_guide: bool,
}

impl Default for Switches {
    fn default() -> Self {
        Self::all_enabled()
    }
}

impl Switches {
    /// Returns a `Switches` configuration with all 22 mechanisms enabled and `self_mechanics_guide = false`.
    #[must_use]
    pub const fn all_enabled() -> Self {
        Self {
            salience: true,
            floor_cost: true,
            habituation: true,
            drive_threshold: true,
            familiarity_reserve: true,
            stochastic_go_no_go: true,
            commitment_breaker: true,
            patience: true,
            report_bar_on_thought: true,
            activation_recall: true,
            vector_fade: true,
            prediction_and_grounding: true,
            prospective_memory: true,
            credence: true,
            bounded_learning: true,
            memory_truth: true,
            report_control: true,
            consolidation_and_merge: true,
            perception_ledger: true,
            agency: true,
            event_boundaries: true,
            backlog_chunking: true,
            self_mechanics_guide: false,
        }
    }

    /// Returns a `Switches` configuration where every mechanism is at its reachable neutral value.
    #[must_use]
    pub const fn all_neutral() -> Self {
        Self {
            salience: false,
            floor_cost: false,
            habituation: false,
            drive_threshold: false,
            familiarity_reserve: false,
            stochastic_go_no_go: false,
            commitment_breaker: false,
            patience: false,
            report_bar_on_thought: false,
            activation_recall: false,
            vector_fade: false,
            prediction_and_grounding: false,
            prospective_memory: false,
            credence: false,
            bounded_learning: false,
            memory_truth: false,
            report_control: false,
            consolidation_and_merge: false,
            perception_ledger: false,
            agency: false,
            event_boundaries: false,
            backlog_chunking: false,
            self_mechanics_guide: false,
        }
    }

    /// Returns `true` if `id` is currently set to its neutral value.
    #[must_use]
    pub const fn is_neutral(&self, id: MechanismId) -> bool {
        !self.is_enabled(id)
    }

    /// Returns `true` if `id` is currently enabled (non-neutral).
    #[must_use]
    pub const fn is_enabled(&self, id: MechanismId) -> bool {
        match id {
            MechanismId::Salience => self.salience,
            MechanismId::FloorCost => self.floor_cost,
            MechanismId::Habituation => self.habituation,
            MechanismId::DriveThreshold => self.drive_threshold,
            MechanismId::FamiliarityReserve => self.familiarity_reserve,
            MechanismId::StochasticGoNoGo => self.stochastic_go_no_go,
            MechanismId::CommitmentBreaker => self.commitment_breaker,
            MechanismId::Patience => self.patience,
            MechanismId::ReportBarOnThought => self.report_bar_on_thought,
            MechanismId::ActivationRecall => self.activation_recall,
            MechanismId::VectorFade => self.vector_fade,
            MechanismId::PredictionAndGrounding => self.prediction_and_grounding,
            MechanismId::ProspectiveMemory => self.prospective_memory,
            MechanismId::Credence => self.credence,
            MechanismId::BoundedLearning => self.bounded_learning,
            MechanismId::MemoryTruth => self.memory_truth,
            MechanismId::ReportControl => self.report_control,
            MechanismId::ConsolidationAndMerge => self.consolidation_and_merge,
            MechanismId::PerceptionLedger => self.perception_ledger,
            MechanismId::Agency => self.agency,
            MechanismId::EventBoundaries => self.event_boundaries,
            MechanismId::BacklogChunking => self.backlog_chunking,
        }
    }

    /// Moves `id` to its neutral value (`false`).
    pub fn set_neutral(&mut self, id: MechanismId) {
        self.set_enabled(id, false);
    }

    /// Enables or neutralizes `id`.
    pub fn set_enabled(&mut self, id: MechanismId, enabled: bool) {
        match id {
            MechanismId::Salience => self.salience = enabled,
            MechanismId::FloorCost => self.floor_cost = enabled,
            MechanismId::Habituation => self.habituation = enabled,
            MechanismId::DriveThreshold => self.drive_threshold = enabled,
            MechanismId::FamiliarityReserve => self.familiarity_reserve = enabled,
            MechanismId::StochasticGoNoGo => self.stochastic_go_no_go = enabled,
            MechanismId::CommitmentBreaker => self.commitment_breaker = enabled,
            MechanismId::Patience => self.patience = enabled,
            MechanismId::ReportBarOnThought => self.report_bar_on_thought = enabled,
            MechanismId::ActivationRecall => self.activation_recall = enabled,
            MechanismId::VectorFade => self.vector_fade = enabled,
            MechanismId::PredictionAndGrounding => self.prediction_and_grounding = enabled,
            MechanismId::ProspectiveMemory => self.prospective_memory = enabled,
            MechanismId::Credence => self.credence = enabled,
            MechanismId::BoundedLearning => self.bounded_learning = enabled,
            MechanismId::MemoryTruth => self.memory_truth = enabled,
            MechanismId::ReportControl => self.report_control = enabled,
            MechanismId::ConsolidationAndMerge => self.consolidation_and_merge = enabled,
            MechanismId::PerceptionLedger => self.perception_ledger = enabled,
            MechanismId::Agency => self.agency = enabled,
            MechanismId::EventBoundaries => self.event_boundaries = enabled,
            MechanismId::BacklogChunking => self.backlog_chunking = enabled,
        }
    }

    /// Returns the validated compile-time brain-map registry of all 22 mechanisms.
    pub fn registry() -> Result<Vec<MechanismEntry>, RegistryError> {
        brain_map_registry()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        MechanismEntry, MechanismId, NeutralValue, RegistryError, Switches, brain_map_registry,
    };

    #[test]
    fn registry_contains_all_22_mechanisms_with_non_empty_fields() {
        let entries = brain_map_registry().expect("22 valid registry entries");
        assert_eq!(entries.len(), 22);

        for (idx, entry) in entries.iter().enumerate() {
            assert_eq!(entry.row as usize, idx + 1);
            assert!(!entry.stable_id.trim().is_empty());
            assert!(!entry.function_name.trim().is_empty());
            assert!(!entry.mechanism.trim().is_empty());
            assert!(
                entry.research_section.contains("RESEARCH.md §"),
                "entry {} missing RESEARCH.md section: {}",
                entry.stable_id,
                entry.research_section
            );
            assert!(!entry.causal_claim.trim().is_empty());
            assert!(!entry.neutral_description.trim().is_empty());
        }
    }

    #[test]
    fn all_neutral_sets_every_mechanism_to_neutral() {
        let neutral = Switches::all_neutral();
        assert!(!neutral.self_mechanics_guide);
        for id in MechanismId::ALL {
            assert!(
                neutral.is_neutral(id),
                "expected mechanism {id} to be neutral in Switches::all_neutral()"
            );
        }

        let mut enabled = Switches::all_enabled();
        for id in MechanismId::ALL {
            assert!(!enabled.is_neutral(id));
            enabled.set_neutral(id);
            assert!(enabled.is_neutral(id));
        }
        assert_eq!(enabled, neutral);
    }

    #[test]
    fn constructing_entry_without_claim_or_neutral_fails() {
        let missing_claim = MechanismEntry::try_build(
            MechanismId::Salience,
            1,
            "m01_salience",
            "Salience",
            "weighted signals -> S",
            "RESEARCH.md §2, §3",
            None,
            Some(NeutralValue::Float(0.0)),
            Some("all w_k = 0"),
        );
        assert!(matches!(
            missing_claim,
            Err(RegistryError::MissingClaim { .. })
        ));

        let blank_claim = MechanismEntry::try_build(
            MechanismId::Salience,
            1,
            "m01_salience",
            "Salience",
            "weighted signals -> S",
            "RESEARCH.md §2, §3",
            Some("   "),
            Some(NeutralValue::Float(0.0)),
            Some("all w_k = 0"),
        );
        assert!(matches!(
            blank_claim,
            Err(RegistryError::MissingClaim { .. })
        ));

        let missing_neutral = MechanismEntry::try_build(
            MechanismId::Habituation,
            3,
            "m03_habituation",
            "Habituation",
            "partitioned h",
            "RESEARCH.md §3",
            Some("wakes per repeated stimulus"),
            None,
            Some("kappa = 0"),
        );
        assert!(matches!(
            missing_neutral,
            Err(RegistryError::MissingNeutralValue { .. })
        ));
    }
}
