//! Discrete-event virtual-time Lab harness and Waist-2 `TestBody` simulator for MonoAgens.

#![forbid(unsafe_code)]

use core::fmt;
use std::io;

use monoagens_core::{CoreError, StoreError};
/// Re-export seam for `monoagens-reason` (real LLM/embedder providers land in later milestones).
pub use monoagens_reason as reason_seam;

pub mod baseline;
pub mod benchmark;
pub mod checks;
pub mod clock;
pub mod driver;
pub mod guard;
pub mod metrics;
pub mod pareto;
pub mod record;
pub mod scenario;
pub mod soul_dir;
pub mod test_body;

pub use baseline::{
    AdapterAction, AdapterStepRecord, BaselineKind, BaselineRunReport, PerceptView,
    TokenUsage5Tuple, format_sim_time_rfc3339, run_baseline_adapter,
};
pub use benchmark::{
    BenchmarkDatasetSpec, BenchmarkDomain, BenchmarkSuiteReport, ToolRagScalePoint,
    run_arm_t_restart_parity_check, run_benchmark_suite, run_tool_rag_scaling_eval,
    standard_benchmark_registry, verify_required_datasets_present,
};
pub use checks::{CheckKind, CheckResult, ExpectationCheck, evaluate_checks};
pub use clock::VirtualClock;
pub use driver::{SimOutput, replay_events_offline, run_ablation_compare, run_scenario};
pub use guard::{CostGuard, GuardRefusal};
pub use metrics::{
    DetailedMetricsReport, MetricReportEntry, PairedAblationDelta, RunMetricInputs,
    RunMetricsSummary, SeedAggregate, aggregate_metrics_across_seeds, compare_paired_ablation,
    compute_run_metrics, compute_run_metrics_full, format_metrics_table, wasserstein_1d_ms,
};
pub use pareto::{
    CONFIRMATORY_SCENARIO_MIN, CONFIRMATORY_SCENARIO_TARGET, ConvexParetoReport,
    DEFAULT_FWER_ALPHA, DEFAULT_NI_MARGIN_DELTA, HolmComparisonResult, PairedComparisonInput,
    ParetoScorecardInput, SystemConfigScore, compute_convex_pareto, evaluate_holm_bonferroni,
    kerby_rank_biserial,
};
pub use record::{PurposeCost, RunRecord};
pub use scenario::{
    CostGuardConfig, EventAnnotation, FalseBeliefProbe, RecallProbeAnnotation, ReportControlProbe,
    Scenario, ScenarioAnnotations, ScenarioSet, ScriptedActionRule, ScriptedEvent,
    ScriptedEventKind, ScriptedFollowUpPercept, ScriptedModelResponse, TestBodySpec,
    WarrantedReply, ensure_not_sealed_path, load_scenarios_from_set,
};
pub use soul_dir::{SoulDir, SoulManifest};
pub use test_body::{ScheduledWireRpc, TestBody, sha256_hex};

/// Errors produced by the Lab harness.
#[derive(Debug)]
pub enum LabError {
    Io(io::Error),
    Core(CoreError),
    Store(StoreError),
    Serialization(String),
    Protocol(String),
}

impl fmt::Display for LabError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(err) => write!(f, "I/O error: {err}"),
            Self::Core(err) => write!(f, "core error: {err}"),
            Self::Store(err) => write!(f, "store error: {err}"),
            Self::Serialization(msg) => write!(f, "serialization error: {msg}"),
            Self::Protocol(msg) => write!(f, "protocol error: {msg}"),
        }
    }
}

impl std::error::Error for LabError {}

impl From<io::Error> for LabError {
    fn from(err: io::Error) -> Self {
        Self::Io(err)
    }
}

impl From<CoreError> for LabError {
    fn from(err: CoreError) -> Self {
        Self::Core(err)
    }
}

impl From<StoreError> for LabError {
    fn from(err: StoreError) -> Self {
        Self::Store(err)
    }
}
