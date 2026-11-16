//! Run records capturing provenance, configuration, checks, metrics, and cost by purpose.
//!
//! Persisted deterministically with `postcard` and formatted for `--json` CLI output.

use std::collections::BTreeMap;

use monoagens_core::{ModelPurpose, Switches};
use serde::{Deserialize, Serialize};

use crate::LabError;
use crate::checks::CheckResult;
use crate::metrics::RunMetricsSummary;
use crate::scenario::ScenarioSet;

/// Token and call accounting grouped by [`ModelPurpose`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PurposeCost {
    pub purpose: ModelPurpose,
    pub calls: u64,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub cost_usd: f64,
}

/// Complete provenance and outcome record for one Lab run (`specs/lab/spec.md`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunRecord {
    pub schema_version: String,
    pub run_id: String,
    pub core_version: String,
    pub prior_content_version: String,
    pub prior_format_version: u32,
    pub prior_digest: String,
    /// Number of events `Core::open` appended before the first scenario input.
    pub open_offset: u64,
    pub model_id: String,
    pub embedder_id: String,
    pub door_rules: Vec<String>,
    pub switches: Switches,
    pub learning_switches: BTreeMap<String, bool>,
    pub seed: u64,
    pub scenario_id: String,
    pub scenario_set: ScenarioSet,
    pub sealed: bool,
    pub checks: Vec<CheckResult>,
    pub passed: bool,
    pub metrics: RunMetricsSummary,
    pub cost_by_purpose: Vec<PurposeCost>,
}

impl RunRecord {
    /// Derives the learning-switch summary map from [`Switches`].
    #[must_use]
    pub fn learning_switches_from(switches: &Switches) -> BTreeMap<String, bool> {
        let mut map = BTreeMap::new();
        map.insert("bounded_learning".to_owned(), switches.bounded_learning);
        map.insert("credence".to_owned(), switches.credence);
        map.insert(
            "self_mechanics_guide".to_owned(),
            switches.self_mechanics_guide,
        );
        map
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
            "prior_content_version": self.prior_content_version,
            "prior_format_version": self.prior_format_version,
            "prior_digest": self.prior_digest,
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
