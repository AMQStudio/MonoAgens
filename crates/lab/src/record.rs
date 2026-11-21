//! Run records capturing provenance, configuration, checks, metrics, and cost by purpose.
//!
//! Persisted deterministically with `postcard` and formatted for `--json` CLI output.

use std::collections::BTreeMap;

use monoagens_core::ModelPurpose;
use serde::{Deserialize, Serialize};

use crate::LabError;
use crate::checks::CheckResult;
use crate::metrics::RunMetricsSummary;
use crate::scenario::ScenarioSet;

/// `RunRecord.monoagens_mode` for a live run that exercised the full MonoAgens loop.
pub const MONOAGENS_MODE_FULL: &str = "full";
/// `RunRecord.monoagens_mode` for a live run that deviated from the full MonoAgens.
pub const MONOAGENS_MODE_INVALID: &str = "invalid";
/// `RunRecord.monoagens_mode` for a non-live run (never a MonoAgens benchmark result).
pub const MONOAGENS_MODE_OFFLINE: &str = "offline";
/// `DatasetRunProvenance.prompt_source` when every model call used the production renderer.
pub const PROMPT_SOURCE_CORE_RENDER: &str = "core_render";

/// Token and call accounting grouped by [`ModelPurpose`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PurposeCost {
    pub purpose: ModelPurpose,
    pub calls: u64,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub cost_usd: f64,
}

/// Confidence interval recorded per metric in external dataset run records.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MetricScoreInterval {
    pub metric: String,
    pub value: Option<f64>,
    #[serde(default)]
    pub ci95_low: Option<f64>,
    #[serde(default)]
    pub ci95_high: Option<f64>,
    #[serde(default)]
    pub n_clusters: Option<usize>,
    #[serde(default)]
    pub not_scored_reason: Option<String>,
}

/// Item-level failure recorded explicitly (never silently dropped).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FailedItemRecord {
    pub item_id: String,
    pub probe_id: String,
    pub stage: String,
    pub reason: String,
}

/// Token and cost accounting for one ledger phase of a memory-suite dataset.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct UsageBreakdown {
    pub calls: u64,
    pub uncached_input_tokens: u64,
    pub cached_input_tokens: u64,
    pub cached_unknown_calls: u64,
    pub output_tokens: u64,
    pub reasoning_tokens: u64,
    pub list_cost_usd: f64,
    pub allowance_cost_usd: f64,
}

/// Ingest/answer/judge usage recorded for a memory-suite dataset run.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MemoryUsageTotals {
    pub ingest: UsageBreakdown,
    pub answer: UsageBreakdown,
    pub judge: UsageBreakdown,
    pub latencies_ms: Vec<u64>,
    /// Ledger usage grouped by the recorded call purpose (`reason`/`classify`/`consolidate`/`judge`).
    #[serde(default)]
    pub by_purpose: BTreeMap<String, UsageBreakdown>,
    /// Calls whose provider `finish_reason` was `length`, per purpose.
    #[serde(default)]
    pub runaway_by_purpose: BTreeMap<String, u64>,
    /// Configured model output cap (`model.max_output_tokens`).
    #[serde(default)]
    pub max_output_tokens: Option<u32>,
}

/// Provenance and sampling metadata attached to external dataset runs (`benchmark-evaluation/spec.md`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DatasetRunProvenance {
    pub dataset_id: String,
    pub pinned_revision: String,
    pub manifest_digest: String,
    pub converter_version: String,
    pub scorer_version: String,
    pub sampling_rule: String,
    pub chosen_item_ids: Vec<String>,
    pub metric_scores: Vec<MetricScoreInterval>,
    #[serde(default)]
    pub judge_model: Option<String>,
    #[serde(default)]
    pub judge_prompt_version: Option<String>,
    /// Judge calls actually issued by scorers (excludes Gate/classifier wakes).
    #[serde(default)]
    pub judge_calls: u64,
    #[serde(default)]
    pub judge_tokens_in: u64,
    #[serde(default)]
    pub judge_tokens_out: u64,
    #[serde(default)]
    pub history_gate_wakes: u64,
    #[serde(default)]
    pub partial_run: bool,
    #[serde(default)]
    pub reasoning_effort: Option<String>,
    #[serde(default)]
    pub judge_reasoning_effort: Option<String>,
    #[serde(default)]
    pub extra_header_names: Vec<String>,
    #[serde(default)]
    pub provider_endpoint_host: Option<String>,
    #[serde(default)]
    pub embedder_backend: Option<String>,
    #[serde(default)]
    pub embedder_revision: Option<String>,
    /// `core_render` when every model call was rendered by `monoagens_reason`, never by the Lab.
    #[serde(default)]
    pub prompt_source: Option<String>,
    /// `none` when no MonoAgens change was informed by this benchmark.
    #[serde(default)]
    pub development_exposure: Option<String>,
    /// Ledger-derived ingest/answer/judge usage for a memory-suite run.
    #[serde(default)]
    pub memory_usage: Option<MemoryUsageTotals>,
    #[serde(default)]
    pub category_metrics: BTreeMap<String, BTreeMap<String, f64>>,
    #[serde(default)]
    pub failed_items: Vec<FailedItemRecord>,
    #[serde(default)]
    pub model_latencies_ms: Vec<u64>,
    #[serde(default)]
    pub started_at_utc: Option<String>,
    #[serde(default)]
    pub finished_at_utc: Option<String>,
}

/// Complete provenance and outcome record for one Lab run (`specs/lab/spec.md`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunRecord {
    pub schema_version: String,
    pub run_id: String,
    pub core_version: String,
    pub config_format_version: u32,
    pub config_digest: String,
    #[serde(default)]
    pub model_blake3: Option<String>,
    #[serde(default)]
    pub online_adaptation: bool,
    /// Number of events `Core::open` appended before the first scenario input.
    pub open_offset: u64,
    pub model_id: String,
    pub embedder_id: String,
    pub door_rules: Vec<String>,
    pub seed: u64,
    pub scenario_id: String,
    pub scenario_set: ScenarioSet,
    pub sealed: bool,
    pub checks: Vec<CheckResult>,
    pub passed: bool,
    pub metrics: RunMetricsSummary,
    pub cost_by_purpose: Vec<PurposeCost>,
    #[serde(default)]
    pub history_reasoning_mode: Option<String>,
    /// `full` only when the run exercised the complete MonoAgens loop (`Design 8`).
    #[serde(default)]
    pub monoagens_mode: Option<String>,
    #[serde(default)]
    pub dataset_provenance: Option<DatasetRunProvenance>,
}

impl RunRecord {
    /// Returns `true` only for a live run that exercised the full MonoAgens: production prompt
    /// rendering, live history, and `monoagens_mode = "full"`.
    #[must_use]
    pub fn is_full_monoagens(&self) -> bool {
        self.monoagens_mode.as_deref() == Some(MONOAGENS_MODE_FULL)
            && self.history_reasoning_mode.as_deref() != Some("scripted_silent")
            && self
                .dataset_provenance
                .as_ref()
                .and_then(|p| p.prompt_source.as_deref())
                == Some(PROMPT_SOURCE_CORE_RENDER)
    }

    /// Encodes this run record using `postcard`.
    pub fn encode_postcard(&self) -> Result<Vec<u8>, LabError> {
        postcard::to_allocvec(self).map_err(|e| LabError::Serialization(e.to_string()))
    }

    /// Decodes a run record from `postcard` bytes.
    pub fn decode_postcard(bytes: &[u8]) -> Result<Self, LabError> {
        postcard::from_bytes(bytes).map_err(|e| LabError::Serialization(e.to_string()))
    }

    /// Produces machine-readable JSON output, redacting scenario-specific checks/details
    /// when `self.sealed` is `true` (`specs/lab/spec.md`).
    #[must_use]
    pub fn to_machine_json(&self) -> serde_json::Value {
        if !self.sealed {
            return serde_json::to_value(self).unwrap_or(serde_json::Value::Null);
        }
        let passed_checks = self.checks.iter().filter(|c| c.passed).count();
        let failed_checks = self.checks.len().saturating_sub(passed_checks);
        serde_json::json!({
            "schema_version": self.schema_version,
            "run_id": self.run_id,
            "core_version": self.core_version,
            "config_format_version": self.config_format_version,
            "config_digest": self.config_digest,
            "model_blake3": self.model_blake3,
            "online_adaptation": self.online_adaptation,
            "open_offset": self.open_offset,
            "model_id": self.model_id,
            "embedder_id": self.embedder_id,
            "door_rules": self.door_rules,
            "seed": self.seed,
            "scenario_set": self.scenario_set,
            "sealed": true,
            "passed": self.passed,
            "aggregate": {
                "total_checks": self.checks.len(),
                "passed_checks": passed_checks,
                "failed_checks": failed_checks,
            },
            "metrics": self.metrics,
            "cost_by_purpose": self.cost_by_purpose,
        })
    }
}
