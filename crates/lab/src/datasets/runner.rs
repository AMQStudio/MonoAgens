//! Benchmark suite execution engine: manifest-verified preflight, `--only` partial runs,
//! `--live --max-cost-usd` scripted preflight cost estimation, configurable judge routing,
//! one-row-per-dataset scorecard construction, and `RunRecord` emission.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use monoagens_core::{ModelPurpose, Prior, Switches};
use monoagens_reason::embed::Embedder;
use monoagens_reason::model::{
    CallOutcome, ModelCallFailure, ModelCallRecord, ModelCallSuccess, ModelProvider, ModelRequest,
    ProviderFailureClass, ScriptedModelProvider, ScriptedReply,
};
use monoagens_reason::secrets::{
    ENV_EMBED_API_KEY, ENV_EMBED_ENDPOINT, ENV_MODEL_API_KEY, ENV_MODEL_ENDPOINT,
    ENV_OPENAI_API_KEY, ENV_OPENAI_BASE_URL, ProviderSecrets, SecretString,
};
use serde::{Deserialize, Serialize};

use crate::LabError;
use crate::benchmark::BenchmarkDatasetSpec;
use crate::datasets::capa::{CapaConverter, CapaScorer};
use crate::datasets::dyntom::{DynTomConverter, DynTomScorer};
use crate::datasets::exploretom::{ExploreTomConverter, ExploreTomScorer};
use crate::datasets::fantom::{FantomConverter, FantomScorer};
use crate::datasets::framework::{
    Converter, EmbedderProbe, EmbedderProbeOutcome, ExternalItem, HistoryMode,
    JudgeExecutionConfig, SILENT_THINK_JSON, Scorer, execute_external_item, load_or_convert_cached,
    sample_items_stratified,
};
use crate::datasets::group_mem_bench::{GroupMemBenchConverter, GroupMemBenchScorer};
use crate::datasets::hi_tom::{HiTomConverter, HiTomScorer};
use crate::datasets::indocollex::IndoCollexProbe;
use crate::datasets::locomo::{LocomoConverter, LocomoScorer};
use crate::datasets::longmemeval::{LongMemEvalConverter, LongMemEvalScorer};
use crate::datasets::manifest::{
    CoreApplicability, DatasetAvailability, DatasetLane, DatasetsManifest,
    load_and_validate_manifest,
};
use crate::datasets::memory_agent_bench::{MemoryAgentBenchConverter, MemoryAgentBenchScorer};
use crate::datasets::molweni::{MolweniConverter, MolweniScorer};
use crate::datasets::multilexnorm::MultiLexNormProbe;
use crate::datasets::opentom::{OpenTomConverter, OpenTomScorer};
use crate::datasets::ops::verify_manifest_entry;
use crate::datasets::pitch::{PitchConverter, PitchScorer};
use crate::datasets::pm_bench::{PmBenchConverter, PmBenchScorer};
use crate::datasets::proactive_bench::{ProactiveBenchConverter, ProactiveBenchScorer};
use crate::datasets::scoring_utils::default_sampling_rule;
use crate::datasets::stale::{StaleConverter, StaleScorer};
use crate::datasets::tomi::{TomiConverter, TomiScorer};
use crate::datasets::ubuntu_irc::{UbuntuIrcConverter, UbuntuIrcScorer};
use crate::datasets::when2speak::{When2SpeakConverter, When2SpeakScorer};
use crate::guard::CostGuard;
use crate::metrics::RunMetricsSummary;
use crate::pareto::{ScorecardRow, ScorecardStatus};
use crate::record::{DatasetRunProvenance, MetricScoreInterval, PurposeCost, RunRecord};
use crate::scenario::ScenarioSet;
use crate::sha256_hex;

/// Official judge model requirement handling when configured judge differs from upstream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JudgeSubstitutionPolicy {
    RenameMetric,
    StrictFail,
}

/// Configurable OpenAI-compatible judge specification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JudgeConfig {
    pub endpoint_url: String,
    pub model: String,
    pub prompt_version: String,
    pub substitution_policy: JudgeSubstitutionPolicy,
}

/// Returns the official upstream judge model ID for judge-dependent benchmarks.
#[must_use]
pub fn official_judge_model(dataset_id: &str) -> Option<&'static str> {
    match dataset_id {
        "longmemeval" | "memory_agent_bench" | "capa" => Some("gpt-4o"),
        "group_mem_bench" => Some("gpt-5"),
        "stale" => Some("Gemini-3.1-flash-lite"),
        _ => None,
    }
}

/// Enforces official judge model alignment: either returns `metric` unchanged, renames it to
/// `<metric>_non_official_judge`, or fails under [`JudgeSubstitutionPolicy::StrictFail`].
pub fn resolve_judge_metric_name(
    dataset_id: &str,
    metric: &str,
    configured_judge_model: &str,
    policy: JudgeSubstitutionPolicy,
) -> Result<String, LabError> {
    let Some(official) = official_judge_model(dataset_id) else {
        return Ok(metric.to_owned());
    };
    if configured_judge_model.eq_ignore_ascii_case(official) {
        return Ok(metric.to_owned());
    }
    match policy {
        JudgeSubstitutionPolicy::RenameMetric => Ok(format!("{metric}_non_official_judge")),
        JudgeSubstitutionPolicy::StrictFail => Err(LabError::Protocol(format!(
            "judge substitution refused for '{dataset_id}': configured judge '{configured_judge_model}' != official '{official}'"
        ))),
    }
}

/// Per-dataset preflight estimate computed deterministically before any `--live` call.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PreflightDatasetEstimate {
    pub dataset_id: String,
    pub item_count: usize,
    pub history_percepts: u64,
    pub estimated_history_wakes: u64,
    pub estimated_probe_calls: u64,
    pub estimated_judge_calls: u64,
    pub worst_case_tokens: u64,
    pub worst_case_usd: f64,
}

/// Aggregate `--live` scripted preflight estimate (`Design 7`, Task 4.5).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PreflightEstimate {
    pub datasets: Vec<PreflightDatasetEstimate>,
    pub total_history_percepts: u64,
    pub total_estimated_history_wakes: u64,
    pub total_estimated_probe_calls: u64,
    pub total_estimated_judge_calls: u64,
    pub total_worst_case_tokens: u64,
    pub total_worst_case_usd: f64,
    pub max_cost_usd: f64,
    pub within_budget: bool,
}

/// Estimates worst-case `--live` wakes, calls, tokens, and USD via a deterministic scripted
/// simulation BEFORE any paid call runs.
pub fn estimate_live_preflight(
    converted_by_id: &[(String, Vec<ExternalItem>)],
    prior: &Prior,
    switches: &Switches,
    seed: u64,
    max_cost_usd: f64,
    live_full_history: bool,
) -> Result<PreflightEstimate, LabError> {
    let silent_probe = {
        let mut p = ScriptedModelProvider::new();
        p.set_fallback(ScriptedReply::text(SILENT_THINK_JSON));
        p
    };
    let mut guard = CostGuard::default();
    let mut ds_estimates = Vec::with_capacity(converted_by_id.len());
    let mut total_hist_p = 0_u64;
    let mut total_hist_w = 0_u64;
    let mut total_probes = 0_u64;
    let mut total_judges = 0_u64;
    let mut total_tok = 0_u64;
    let mut total_usd = 0.0_f64;

    // Conservative worst-case pricing assumption for preflight bounding ($0.01 per 1K tokens).
    const WORST_CASE_USD_PER_1K_TOK: f64 = 0.01;
    const WORST_CASE_TOK_PER_PROBE: u64 = 2_048;
    const WORST_CASE_TOK_PER_JUDGE: u64 = 1_024;

    for (ds_id, items) in converted_by_id {
        let mut hist_p = 0_u64;
        let mut hist_w = 0_u64;
        let mut probe_calls = 0_u64;
        for item in items {
            hist_p = hist_p.saturating_add(item.history.len() as u64);
            probe_calls = probe_calls.saturating_add(item.probes.len() as u64);
            let exec = execute_external_item(
                item,
                prior,
                switches,
                seed,
                &silent_probe,
                "preflight-scripted",
                live_full_history,
                &mut guard,
            )?;
            hist_w = hist_w.saturating_add(exec.total_history_gate_wakes);
        }
        let judge_calls = if official_judge_model(ds_id).is_some() {
            probe_calls
        } else {
            0
        };
        let paid_history_calls = if live_full_history { hist_w } else { 0 };
        let worst_tok = (probe_calls + paid_history_calls) * WORST_CASE_TOK_PER_PROBE
            + judge_calls * WORST_CASE_TOK_PER_JUDGE;
        let worst_usd = (worst_tok as f64 / 1_000.0) * WORST_CASE_USD_PER_1K_TOK;

        total_hist_p = total_hist_p.saturating_add(hist_p);
        total_hist_w = total_hist_w.saturating_add(hist_w);
        total_probes = total_probes.saturating_add(probe_calls);
        total_judges = total_judges.saturating_add(judge_calls);
        total_tok = total_tok.saturating_add(worst_tok);
        total_usd += worst_usd;

        ds_estimates.push(PreflightDatasetEstimate {
            dataset_id: ds_id.clone(),
            item_count: items.len(),
            history_percepts: hist_p,
            estimated_history_wakes: hist_w,
            estimated_probe_calls: probe_calls,
            estimated_judge_calls: judge_calls,
            worst_case_tokens: worst_tok,
            worst_case_usd: worst_usd,
        });
    }

    Ok(PreflightEstimate {
        datasets: ds_estimates,
        total_history_percepts: total_hist_p,
        total_estimated_history_wakes: total_hist_w,
        total_estimated_probe_calls: total_probes,
        total_estimated_judge_calls: total_judges,
        total_worst_case_tokens: total_tok,
        total_worst_case_usd: total_usd,
        max_cost_usd,
        within_budget: total_usd <= max_cost_usd,
    })
}

/// Registry of dataset converters, scorers, and component embedder probes.
#[derive(Default)]
pub struct DatasetHandlerRegistry {
    pub converters: BTreeMap<String, Box<dyn Converter>>,
    pub scorers: BTreeMap<String, Box<dyn Scorer>>,
    pub embedder_probes: BTreeMap<String, Box<dyn EmbedderProbe>>,
}

impl DatasetHandlerRegistry {
    /// Registers a behavioral `(Converter, Scorer)` pair.
    pub fn register_behavioral(&mut self, converter: Box<dyn Converter>, scorer: Box<dyn Scorer>) {
        self.converters
            .insert(converter.dataset_id().to_owned(), converter);
        self.scorers.insert(scorer.dataset_id().to_owned(), scorer);
    }

    /// Registers a component `EmbedderProbe`.
    pub fn register_embedder_probe(&mut self, probe: Box<dyn EmbedderProbe>) {
        self.embedder_probes
            .insert(probe.dataset_id().to_owned(), probe);
    }

    /// Registers all 18 standard behavioral `(Converter, Scorer)` pairs and both Indonesian
    /// component `EmbedderProbe`s (`multilexnorm`, `indocollex`).
    #[must_use]
    pub fn standard() -> Self {
        let mut reg = Self::default();
        // Memory (5)
        reg.register_behavioral(Box::new(LocomoConverter), Box::new(LocomoScorer));
        reg.register_behavioral(Box::new(LongMemEvalConverter), Box::new(LongMemEvalScorer));
        reg.register_behavioral(
            Box::new(MemoryAgentBenchConverter),
            Box::new(MemoryAgentBenchScorer),
        );
        reg.register_behavioral(
            Box::new(GroupMemBenchConverter),
            Box::new(GroupMemBenchScorer),
        );
        reg.register_behavioral(Box::new(StaleConverter), Box::new(StaleScorer));
        // Prospective & timing (5 active; pis/proact unavailable)
        reg.register_behavioral(Box::new(PmBenchConverter), Box::new(PmBenchScorer));
        reg.register_behavioral(Box::new(CapaConverter), Box::new(CapaScorer));
        reg.register_behavioral(
            Box::new(ProactiveBenchConverter),
            Box::new(ProactiveBenchScorer),
        );
        reg.register_behavioral(Box::new(PitchConverter), Box::new(PitchScorer));
        reg.register_behavioral(Box::new(When2SpeakConverter), Box::new(When2SpeakScorer));
        // Group chat (2)
        reg.register_behavioral(Box::new(UbuntuIrcConverter), Box::new(UbuntuIrcScorer));
        reg.register_behavioral(Box::new(MolweniConverter), Box::new(MolweniScorer));
        // Theory of Mind (6 incl. dyntom + optional tomi)
        reg.register_behavioral(Box::new(FantomConverter), Box::new(FantomScorer));
        reg.register_behavioral(Box::new(OpenTomConverter), Box::new(OpenTomScorer));
        reg.register_behavioral(Box::new(HiTomConverter), Box::new(HiTomScorer));
        reg.register_behavioral(Box::new(ExploreTomConverter), Box::new(ExploreTomScorer));
        reg.register_behavioral(Box::new(DynTomConverter), Box::new(DynTomScorer));
        reg.register_behavioral(Box::new(TomiConverter), Box::new(TomiScorer));
        // Indonesian reason::Embedder probes (2 active; mpbench unavailable)
        reg.register_embedder_probe(Box::new(MultiLexNormProbe));
        reg.register_embedder_probe(Box::new(IndoCollexProbe));
        reg
    }
}

/// Execution options for `run_benchmark_suite_with_options`.
pub struct BenchmarkExecutionOptions<'a> {
    pub manifest_path: Option<PathBuf>,
    pub only_ids: Option<Vec<String>>,
    pub seed: u64,
    pub live: bool,
    pub live_full_history: bool,
    pub max_cost_usd: Option<f64>,
    pub out_dir: Option<PathBuf>,
    pub target_items_per_dataset: Option<usize>,
    pub allow_test_embedders: bool,
    pub probe_model_name: String,
    pub probe_provider: Option<&'a dyn ModelProvider>,
    pub judge_config: Option<JudgeConfig>,
    pub judge_provider: Option<&'a dyn ModelProvider>,
    pub embedder: Option<&'a dyn Embedder>,
    pub handlers: DatasetHandlerRegistry,
}

impl Default for BenchmarkExecutionOptions<'_> {
    fn default() -> Self {
        Self {
            manifest_path: None,
            only_ids: None,
            seed: 1,
            live: false,
            live_full_history: false,
            max_cost_usd: None,
            out_dir: None,
            target_items_per_dataset: None,
            allow_test_embedders: false,
            probe_model_name: "scripted".to_owned(),
            probe_provider: None,
            judge_config: None,
            judge_provider: None,
            embedder: None,
            handlers: DatasetHandlerRegistry::standard(),
        }
    }
}

/// Resolves the manifest path for `datasets_dir`, preferring an explicit path, then
/// `<datasets_dir>/../datasets.manifest.toml` when present, otherwise `lab/datasets.manifest.toml`.
/// Never falls back silently to the committed template `lab/datasets.manifest.example.toml`.
#[must_use]
pub fn resolve_manifest_path(explicit: Option<&Path>, datasets_dir: &Path) -> PathBuf {
    if let Some(p) = explicit {
        return p.to_path_buf();
    }
    if let Some(parent) = datasets_dir.parent() {
        let sibling = parent.join("datasets.manifest.toml");
        if sibling.is_file() {
            return sibling;
        }
    }
    PathBuf::from(crate::datasets::DEFAULT_LOCAL_MANIFEST_PATH)
}

/// Verifies required datasets against the manifest (`--suite standard` or `--only`).
pub fn verify_required_datasets_with_manifest(
    manifest_path: &Path,
    datasets_dir: &Path,
    registry: &[BenchmarkDatasetSpec],
    only_ids: Option<&[String]>,
) -> Result<(DatasetsManifest, Vec<PathBuf>), LabError> {
    let manifest = load_and_validate_manifest(manifest_path, registry)?;

    let mut targets: BTreeSet<String> = BTreeSet::new();
    if let Some(only) = only_ids {
        if only.is_empty() {
            return Err(LabError::Protocol(
                "--only requires at least one dataset id".to_owned(),
            ));
        }
        for id in only {
            if manifest.get(id).is_none() {
                return Err(LabError::Protocol(format!(
                    "unknown dataset id '{id}' in --only"
                )));
            }
            targets.insert(id.clone());
        }
    } else {
        for spec in registry {
            if spec.requires_external_corpus && spec.required_in_standard_suite {
                targets.insert(spec.id.clone());
            }
        }
    }

    let mut present = Vec::new();
    let mut bad = Vec::new();
    for id in targets {
        let entry = manifest.get(&id).expect("validated above");
        if entry.availability == DatasetAvailability::Unavailable {
            continue;
        }
        let v = verify_manifest_entry(entry, datasets_dir)?;
        if v.verified {
            present.push(datasets_dir.join(&entry.relative_dir));
        } else {
            bad.push(format!(
                "{} ({}; run `monoagens-lab datasets fetch {}`)",
                entry.id,
                v.failures.join(", "),
                entry.id
            ));
        }
    }

    if !bad.is_empty() {
        return Err(LabError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!(
                "missing required benchmark dataset(s) or unverified checksums under '{}': {}",
                datasets_dir.display(),
                bad.join("; ")
            ),
        )));
    }

    Ok((manifest, present))
}

pub type ExternalExecutionResult = (
    Vec<ScorecardRow>,
    Vec<RunRecord>,
    bool,
    Vec<String>,
    Option<PreflightEstimate>,
);

/// Executes external datasets (`CoreBehavioral` and `EmbedderProbe` lanes), builds strictly one
/// [`ScorecardRow`] per registered external dataset, and emits per-dataset [`RunRecord`]s.
pub fn execute_external_datasets(
    suite: &str,
    manifest: &DatasetsManifest,
    manifest_path: &Path,
    datasets_dir: &Path,
    registry: &[BenchmarkDatasetSpec],
    prior: &Prior,
    opts: &BenchmarkExecutionOptions<'_>,
) -> Result<ExternalExecutionResult, LabError> {
    if opts.live && opts.max_cost_usd.is_none() {
        return Err(LabError::Protocol(
            "--live benchmark run requires --max-cost-usd <cap>".to_owned(),
        ));
    }

    let manifest_raw = fs::read(manifest_path).unwrap_or_default();
    let manifest_digest = format!("sha256:{}", sha256_hex(&manifest_raw));
    let loaded_prior = monoagens_store::LoadedPrior::from_prior(prior.clone())?;

    let external_specs: Vec<&BenchmarkDatasetSpec> = registry
        .iter()
        .filter(|s| s.requires_external_corpus)
        .collect();

    let only_set: Option<BTreeSet<String>> = opts
        .only_ids
        .as_ref()
        .map(|ids| ids.iter().cloned().collect());
    let is_partial = only_set.is_some();
    let mut left_out = Vec::new();
    if let Some(ref set) = only_set {
        for spec in &external_specs {
            if !set.contains(&spec.id) {
                left_out.push(spec.id.clone());
            }
        }
    }

    // Prepare converted behavioral items up front for any selected, verified behavioral datasets
    // so `--live` preflight cost/wake estimation runs BEFORE any paid model or judge call.
    let mut converted_cache: BTreeMap<
        String,
        (crate::datasets::ConvertedDataset, Vec<ExternalItem>, String),
    > = BTreeMap::new();
    let mut preflight_inputs: Vec<(String, Vec<ExternalItem>)> = Vec::new();

    for spec in &external_specs {
        if let Some(ref set) = only_set
            && !set.contains(&spec.id)
        {
            continue;
        }
        let entry = manifest.get(&spec.id).ok_or_else(|| {
            LabError::Protocol(format!("manifest missing external dataset '{}'", spec.id))
        })?;
        if entry.availability == DatasetAvailability::Unavailable
            || entry.lane != DatasetLane::CoreBehavioral
            || entry.core_applicability == CoreApplicability::NotMappable
        {
            continue;
        }
        let verify_res = verify_manifest_entry(entry, datasets_dir)?;
        if !verify_res.verified {
            continue;
        }
        if let Some(converter) = opts.handlers.converters.get(&spec.id) {
            let ds_dir = datasets_dir.join(&entry.relative_dir);
            let (converted, _) = load_or_convert_cached(converter.as_ref(), &ds_dir, entry)?;
            let mut sampling_rule = default_sampling_rule(&spec.id, opts.seed);
            if let Some(n) = opts.target_items_per_dataset {
                sampling_rule.rule_name = format!("stratified_s{n}");
                sampling_rule.target_n = Some(n);
            }
            let sampled = sample_items_stratified(&converted.items, &sampling_rule);
            let chosen_set: BTreeSet<&str> =
                sampled.chosen_item_ids.iter().map(String::as_str).collect();
            let chosen_items: Vec<ExternalItem> = converted
                .items
                .iter()
                .filter(|it| chosen_set.contains(it.item_id.as_str()))
                .cloned()
                .collect();
            preflight_inputs.push((spec.id.clone(), chosen_items.clone()));
            converted_cache.insert(
                spec.id.clone(),
                (converted, chosen_items, sampling_rule.rule_name),
            );
        }
    }

    let preflight_estimate = if opts.live {
        let cap = opts.max_cost_usd.expect("checked above");
        let est = estimate_live_preflight(
            &preflight_inputs,
            prior,
            &Switches::all_enabled(),
            opts.seed,
            cap,
            opts.live_full_history,
        )?;
        if !est.within_budget {
            return Err(LabError::Protocol(format!(
                "preflight estimated worst-case USD ${:.4} exceeds --max-cost-usd ${:.4}; aborting before any paid call",
                est.total_worst_case_usd, cap
            )));
        }
        Some(est)
    } else {
        None
    };

    let mut guard = match opts.max_cost_usd {
        Some(cap) => CostGuard::with_max_cost_usd(cap),
        None => CostGuard::default(),
    };

    let default_probe_provider = {
        let mut p = ScriptedModelProvider::new();
        p.set_fallback(ScriptedReply::text(SILENT_THINK_JSON));
        p
    };
    let probe_provider: &dyn ModelProvider = opts.probe_provider.unwrap_or(&default_probe_provider);

    let mut rows = Vec::with_capacity(external_specs.len());
    let mut run_records = Vec::new();

    if let Some(dir) = &opts.out_dir {
        fs::create_dir_all(dir)?;
    }

    for spec in &external_specs {
        let entry = manifest.get(&spec.id).expect("manifest validated");
        let base_metrics: BTreeMap<String, Option<f64>> = spec
            .primary_metrics
            .iter()
            .map(|m| (m.clone(), None))
            .collect();

        if entry.availability == DatasetAvailability::Unavailable {
            rows.push(ScorecardRow {
                dataset_id: spec.id.clone(),
                domain: spec.domain,
                lane: entry.lane,
                status: ScorecardStatus::Unavailable,
                core_applicability: entry.core_applicability,
                contamination_risk: entry.contamination_risk,
                metrics: base_metrics,
                metric_intervals: Vec::new(),
                cost_usd: 0.0,
                e_tok: 0.0,
                reason: Some(entry.reason.clone()),
            });
            continue;
        }

        if let Some(ref set) = only_set
            && !set.contains(&spec.id)
        {
            rows.push(ScorecardRow {
                dataset_id: spec.id.clone(),
                domain: spec.domain,
                lane: entry.lane,
                status: ScorecardStatus::NotScored,
                core_applicability: entry.core_applicability,
                contamination_risk: entry.contamination_risk,
                metrics: base_metrics,
                metric_intervals: Vec::new(),
                cost_usd: 0.0,
                e_tok: 0.0,
                reason: Some("excluded by --only".to_owned()),
            });
            continue;
        }

        let verify_res = verify_manifest_entry(entry, datasets_dir)?;
        if !verify_res.verified && suite != "standard" {
            rows.push(ScorecardRow {
                dataset_id: spec.id.clone(),
                domain: spec.domain,
                lane: entry.lane,
                status: ScorecardStatus::Unverified,
                core_applicability: entry.core_applicability,
                contamination_risk: entry.contamination_risk,
                metrics: base_metrics,
                metric_intervals: Vec::new(),
                cost_usd: 0.0,
                e_tok: 0.0,
                reason: Some(verify_res.failures.join("; ")),
            });
            continue;
        }

        if entry.lane == DatasetLane::EmbedderProbe {
            let Some(probe) = opts.handlers.embedder_probes.get(&spec.id) else {
                rows.push(ScorecardRow {
                    dataset_id: spec.id.clone(),
                    domain: spec.domain,
                    lane: DatasetLane::EmbedderProbe,
                    status: ScorecardStatus::NotScored,
                    core_applicability: entry.core_applicability,
                    contamination_risk: entry.contamination_risk,
                    metrics: base_metrics,
                    metric_intervals: Vec::new(),
                    cost_usd: 0.0,
                    e_tok: 0.0,
                    reason: Some("embedder probe not registered".to_owned()),
                });
                continue;
            };

            let ds_dir = datasets_dir.join(&entry.relative_dir);
            let outcome = probe.run_probe(
                &ds_dir,
                opts.embedder,
                opts.allow_test_embedders,
                opts.target_items_per_dataset,
                opts.seed,
            )?;

            match outcome {
                EmbedderProbeOutcome::NotScored { reason, .. } => {
                    let intervals: Vec<MetricScoreInterval> = spec
                        .primary_metrics
                        .iter()
                        .map(|m| MetricScoreInterval {
                            metric: m.clone(),
                            value: None,
                            ci95_low: None,
                            ci95_high: None,
                            n_clusters: None,
                            not_scored_reason: Some(reason.clone()),
                        })
                        .collect();
                    let rec = build_external_run_record(
                        &spec.id,
                        entry,
                        &manifest_digest,
                        &loaded_prior,
                        opts.seed,
                        "none",
                        opts.embedder
                            .and_then(|e| e.model_ver())
                            .unwrap_or("unconfigured"),
                        Vec::new(),
                        "none",
                        Vec::new(),
                        intervals.clone(),
                        None,
                        None,
                        0,
                        is_partial,
                        None,
                        Vec::new(),
                    );
                    write_run_record_if_dir(opts.out_dir.as_deref(), &rec)?;
                    run_records.push(rec);

                    rows.push(ScorecardRow {
                        dataset_id: spec.id.clone(),
                        domain: spec.domain,
                        lane: DatasetLane::EmbedderProbe,
                        status: ScorecardStatus::NotScored,
                        core_applicability: entry.core_applicability,
                        contamination_risk: entry.contamination_risk,
                        metrics: base_metrics,
                        metric_intervals: intervals,
                        cost_usd: 0.0,
                        e_tok: 0.0,
                        reason: Some(reason),
                    });
                }
                EmbedderProbeOutcome::Scored {
                    embed_model_ver,
                    metrics,
                    chosen_item_ids,
                    ..
                } => {
                    let mut metric_map = base_metrics;
                    let intervals: Vec<MetricScoreInterval> = metrics
                        .iter()
                        .map(|m| {
                            metric_map.insert(m.name.clone(), m.value);
                            MetricScoreInterval {
                                metric: m.name.clone(),
                                value: m.value,
                                ci95_low: m.ci95_low,
                                ci95_high: m.ci95_high,
                                n_clusters: m.n_clusters,
                                not_scored_reason: m.not_scored_reason.clone(),
                            }
                        })
                        .collect();
                    let rec = build_external_run_record(
                        &spec.id,
                        entry,
                        &manifest_digest,
                        &loaded_prior,
                        opts.seed,
                        "none",
                        &embed_model_ver,
                        Vec::new(),
                        "embedder_probe",
                        chosen_item_ids,
                        intervals.clone(),
                        None,
                        None,
                        0,
                        is_partial,
                        None,
                        Vec::new(),
                    );
                    write_run_record_if_dir(opts.out_dir.as_deref(), &rec)?;
                    run_records.push(rec);

                    rows.push(ScorecardRow {
                        dataset_id: spec.id.clone(),
                        domain: spec.domain,
                        lane: DatasetLane::EmbedderProbe,
                        status: if is_partial {
                            ScorecardStatus::Partial
                        } else {
                            ScorecardStatus::Run
                        },
                        core_applicability: entry.core_applicability,
                        contamination_risk: entry.contamination_risk,
                        metrics: metric_map,
                        metric_intervals: intervals,
                        cost_usd: 0.0,
                        e_tok: 0.0,
                        reason: None,
                    });
                }
            }
            continue;
        }

        if entry.core_applicability == CoreApplicability::NotMappable {
            rows.push(ScorecardRow {
                dataset_id: spec.id.clone(),
                domain: spec.domain,
                lane: entry.lane,
                status: ScorecardStatus::NotMappable,
                core_applicability: entry.core_applicability,
                contamination_risk: entry.contamination_risk,
                metrics: base_metrics,
                metric_intervals: Vec::new(),
                cost_usd: 0.0,
                e_tok: 0.0,
                reason: Some(entry.reason.clone()),
            });
            continue;
        }

        // CoreBehavioral lane
        let Some((converted, chosen_items, sampling_rule)) = converted_cache.remove(&spec.id)
        else {
            rows.push(ScorecardRow {
                dataset_id: spec.id.clone(),
                domain: spec.domain,
                lane: entry.lane,
                status: ScorecardStatus::NotScored,
                core_applicability: entry.core_applicability,
                contamination_risk: entry.contamination_risk,
                metrics: base_metrics,
                metric_intervals: Vec::new(),
                cost_usd: 0.0,
                e_tok: 0.0,
                reason: Some("converter not registered".to_owned()),
            });
            continue;
        };
        let Some(scorer) = opts.handlers.scorers.get(&spec.id) else {
            rows.push(ScorecardRow {
                dataset_id: spec.id.clone(),
                domain: spec.domain,
                lane: entry.lane,
                status: ScorecardStatus::NotScored,
                core_applicability: entry.core_applicability,
                contamination_risk: entry.contamination_risk,
                metrics: base_metrics,
                metric_intervals: Vec::new(),
                cost_usd: 0.0,
                e_tok: 0.0,
                reason: Some("scorer not registered".to_owned()),
            });
            continue;
        };

        let switches = Switches::all_enabled();
        let mut predictions = Vec::new();
        let mut total_hist_wakes = 0_u64;
        let mut history_mode_recorded: Option<String> = None;
        let mut total_tokens_in = 0_u64;
        let mut total_tokens_out = 0_u64;
        let mut combined_costs: BTreeMap<u8, PurposeCost> = BTreeMap::new();

        for item in &chosen_items {
            if guard.budget_exhausted() {
                return Err(LabError::Protocol(format!(
                    "CostGuard halted '--live' benchmark run at budget cap ${:.4}",
                    opts.max_cost_usd.unwrap_or(0.0)
                )));
            }
            let exec = execute_external_item(
                item,
                prior,
                &switches,
                opts.seed,
                probe_provider,
                &opts.probe_model_name,
                opts.live_full_history,
                &mut guard,
            )?;
            if history_mode_recorded.is_none() && item.history_mode == HistoryMode::StaticSilent {
                history_mode_recorded = Some(exec.history_reasoning_mode);
            }
            total_hist_wakes = total_hist_wakes.saturating_add(exec.total_history_gate_wakes);
            for pc in exec.cost_by_purpose {
                total_tokens_in = total_tokens_in.saturating_add(pc.tokens_in);
                total_tokens_out = total_tokens_out.saturating_add(pc.tokens_out);
                let key = match pc.purpose {
                    monoagens_core::ModelPurpose::Reason => 0,
                    monoagens_core::ModelPurpose::Classify => 1,
                    monoagens_core::ModelPurpose::Consolidate => 2,
                };
                let agg = combined_costs.entry(key).or_insert(PurposeCost {
                    purpose: pc.purpose,
                    calls: 0,
                    tokens_in: 0,
                    tokens_out: 0,
                    cost_usd: 0.0,
                });
                agg.calls += pc.calls;
                agg.tokens_in += pc.tokens_in;
                agg.tokens_out += pc.tokens_out;
                agg.cost_usd += pc.cost_usd;
            }
            predictions.extend(exec.predictions);
        }

        let guarded_judge = opts
            .judge_provider
            .map(|inner| CostGuardedJudgeProvider::new(inner, &mut guard));
        let judge_exec = JudgeExecutionConfig {
            live: opts.live,
            judge_model: opts.judge_config.as_ref().map(|j| j.model.as_str()),
            judge_prompt_version: opts
                .judge_config
                .as_ref()
                .map(|j| j.prompt_version.as_str()),
            provider: guarded_judge.as_ref().map(|g| g as &dyn ModelProvider),
            datasets_dir: Some(datasets_dir),
            manifest: Some(manifest),
        };
        let scored = scorer.score(&predictions, &converted.gold_book, &judge_exec, opts.seed)?;
        if let Some(gj) = guarded_judge {
            let judge_cost = gj.into_purpose_cost();
            if judge_cost.calls > 0 {
                total_tokens_in = total_tokens_in.saturating_add(judge_cost.tokens_in);
                total_tokens_out = total_tokens_out.saturating_add(judge_cost.tokens_out);
                let agg = combined_costs.entry(1).or_insert(PurposeCost {
                    purpose: ModelPurpose::Classify,
                    calls: 0,
                    tokens_in: 0,
                    tokens_out: 0,
                    cost_usd: 0.0,
                });
                agg.calls += judge_cost.calls;
                agg.tokens_in += judge_cost.tokens_in;
                agg.tokens_out += judge_cost.tokens_out;
                agg.cost_usd += judge_cost.cost_usd;
            }
        }

        let mut metric_map = base_metrics;
        let mut intervals = Vec::new();
        let mut any_scored = false;
        let mut any_not_scored = false;
        let policy = opts
            .judge_config
            .as_ref()
            .map_or(JudgeSubstitutionPolicy::RenameMetric, |j| {
                j.substitution_policy
            });

        for m in &scored.metrics {
            let final_name = if let Some(ref jm) = scored.judge_model {
                resolve_judge_metric_name(&spec.id, &m.name, jm, policy)?
            } else {
                m.name.clone()
            };
            if m.value.is_some() {
                any_scored = true;
            } else {
                any_not_scored = true;
            }
            metric_map.insert(final_name.clone(), m.value);
            intervals.push(MetricScoreInterval {
                metric: final_name,
                value: m.value,
                ci95_low: m.ci95_low,
                ci95_high: m.ci95_high,
                n_clusters: m.n_clusters,
                not_scored_reason: m.not_scored_reason.clone(),
            });
        }

        let row_status = if !any_scored && any_not_scored {
            ScorecardStatus::NotScored
        } else if is_partial || any_not_scored {
            ScorecardStatus::Partial
        } else {
            ScorecardStatus::Run
        };

        let chosen_ids: Vec<String> = chosen_items.iter().map(|i| i.item_id.clone()).collect();
        let e_tok = (total_tokens_in + total_tokens_out) as f64;
        let cost_usd = guard.cost_usd_used;

        let rec = build_external_run_record(
            &spec.id,
            entry,
            &manifest_digest,
            loaded_prior_ref(&loaded_prior),
            opts.seed,
            &opts.probe_model_name,
            opts.embedder.and_then(|e| e.model_ver()).unwrap_or("none"),
            guard.rule_names(),
            &sampling_rule,
            chosen_ids,
            intervals.clone(),
            scored.judge_model.clone(),
            scored.judge_prompt_version.clone(),
            total_hist_wakes,
            is_partial,
            history_mode_recorded,
            combined_costs.into_values().collect(),
        );
        write_run_record_if_dir(opts.out_dir.as_deref(), &rec)?;
        run_records.push(rec);

        rows.push(ScorecardRow {
            dataset_id: spec.id.clone(),
            domain: spec.domain,
            lane: DatasetLane::CoreBehavioral,
            status: row_status,
            core_applicability: entry.core_applicability,
            contamination_risk: entry.contamination_risk,
            metrics: metric_map,
            metric_intervals: intervals,
            cost_usd,
            e_tok,
            reason: if row_status == ScorecardStatus::NotScored {
                Some("requires --live judge".to_owned())
            } else {
                None
            },
        });
    }

    Ok((rows, run_records, is_partial, left_out, preflight_estimate))
}

struct CostGuardedJudgeProvider<'a> {
    inner: &'a dyn ModelProvider,
    guard: Mutex<&'a mut CostGuard>,
    classify_cost: Mutex<PurposeCost>,
}

impl<'a> CostGuardedJudgeProvider<'a> {
    fn new(inner: &'a dyn ModelProvider, guard: &'a mut CostGuard) -> Self {
        Self {
            inner,
            guard: Mutex::new(guard),
            classify_cost: Mutex::new(PurposeCost {
                purpose: ModelPurpose::Classify,
                calls: 0,
                tokens_in: 0,
                tokens_out: 0,
                cost_usd: 0.0,
            }),
        }
    }

    fn into_purpose_cost(self) -> PurposeCost {
        self.classify_cost
            .into_inner()
            .unwrap_or_else(|e| e.into_inner())
    }
}

impl ModelProvider for CostGuardedJudgeProvider<'_> {
    fn provider_name(&self) -> &str {
        self.inner.provider_name()
    }

    fn invoke(&self, request: &ModelRequest) -> Result<ModelCallSuccess, ModelCallFailure> {
        {
            let guard = self.guard.lock().unwrap_or_else(|e| e.into_inner());
            if guard.budget_exhausted() {
                return Err(ModelCallFailure {
                    class: ProviderFailureClass::RateLimit,
                    record: Box::new(ModelCallRecord {
                        request_id: request.request_id,
                        purpose: request.purpose,
                        provider: self.provider_name().to_owned(),
                        model: request.model.clone(),
                        usage: None,
                        latency_ms: 0,
                        outcome: CallOutcome::Failed(ProviderFailureClass::RateLimit),
                    }),
                    status_code: None,
                    failed_wake: None,
                    detail: format!(
                        "CostGuard halted '--live' judge call at USD cap ${:.4}",
                        guard.max_cost_usd.unwrap_or(0.0)
                    ),
                });
            }
            if let Some(limit) = guard.max_tokens
                && guard.tokens_used >= limit
            {
                return Err(ModelCallFailure {
                    class: ProviderFailureClass::RateLimit,
                    record: Box::new(ModelCallRecord {
                        request_id: request.request_id,
                        purpose: request.purpose,
                        provider: self.provider_name().to_owned(),
                        model: request.model.clone(),
                        usage: None,
                        latency_ms: 0,
                        outcome: CallOutcome::Failed(ProviderFailureClass::RateLimit),
                    }),
                    status_code: None,
                    failed_wake: None,
                    detail: format!(
                        "CostGuard halted '--live' judge call at token limit ({}/{limit})",
                        guard.tokens_used
                    ),
                });
            }
        }

        let res = self.inner.invoke(request)?;
        let (tin, tout) = res
            .record
            .usage
            .map_or((0, 0), |u| (u.input_tokens, u.output_tokens));
        let total_tok = tin.saturating_add(tout);
        let usd = if total_tok == 0 {
            0.0
        } else {
            (total_tok as f64 / 1_000.0) * 0.01
        };

        {
            let mut guard = self.guard.lock().unwrap_or_else(|e| e.into_inner());
            guard.record_tokens(total_tok);
            guard.record_cost_usd(usd);
        }
        {
            let mut pc = self.classify_cost.lock().unwrap_or_else(|e| e.into_inner());
            pc.calls += 1;
            pc.tokens_in = pc.tokens_in.saturating_add(tin);
            pc.tokens_out = pc.tokens_out.saturating_add(tout);
            pc.cost_usd += usd;
        }

        Ok(res)
    }
}

/// Loads Lab provider secrets and endpoints from an optional `secrets.toml` (falling back to
/// `./secrets.toml` if present) merged with process environment variables (env takes precedence).
pub fn load_lab_provider_secrets(secrets_path: Option<&Path>) -> Result<ProviderSecrets, LabError> {
    let resolved_path: Option<&Path> = match secrets_path {
        Some(p) => {
            crate::scenario::ensure_not_sealed_path(p)?;
            if !p.exists() {
                return Err(LabError::Io(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("secrets file not found: '{}'", p.display()),
                )));
            }
            Some(p)
        }
        None => {
            let default_path = Path::new("secrets.toml");
            default_path.exists().then_some(default_path)
        }
    };

    let file_secrets = if let Some(path) = resolved_path {
        let raw = fs::read_to_string(path)?;
        parse_lab_secrets_toml(&raw)?
    } else {
        ProviderSecrets::default()
    };

    let env_secrets = ProviderSecrets::from_env();
    Ok(ProviderSecrets {
        model_api_key: env_secrets.model_api_key.or(file_secrets.model_api_key),
        model_endpoint: env_secrets.model_endpoint.or(file_secrets.model_endpoint),
        embed_api_key: env_secrets.embed_api_key.or(file_secrets.embed_api_key),
        embed_endpoint: env_secrets.embed_endpoint.or(file_secrets.embed_endpoint),
    })
}

fn parse_lab_secrets_toml(raw: &str) -> Result<ProviderSecrets, LabError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(ProviderSecrets::default());
    }
    if trimmed.starts_with('{') {
        return ProviderSecrets::parse_secrets_str(trimmed)
            .map_err(|e| LabError::Serialization(e.to_string()));
    }
    let val: toml::Value = toml::from_str(trimmed)
        .map_err(|e| LabError::Serialization(format!("invalid secrets.toml: {e}")))?;
    let mut flat = BTreeMap::new();
    flatten_toml_strings(&val, "", &mut flat);

    let pick =
        |keys: &[&str]| -> Option<String> { keys.iter().find_map(|k| flat.get(*k).cloned()) };
    Ok(ProviderSecrets {
        model_api_key: pick(&[
            "model.api_key",
            "model_api_key",
            ENV_MODEL_API_KEY,
            "openai.api_key",
            "openai_api_key",
            ENV_OPENAI_API_KEY,
            "api_key",
        ])
        .map(SecretString::new),
        model_endpoint: pick(&[
            "model.endpoint",
            "model.endpoint_url",
            "model_endpoint",
            ENV_MODEL_ENDPOINT,
            "openai.base_url",
            "openai_base_url",
            ENV_OPENAI_BASE_URL,
            "endpoint",
        ]),
        embed_api_key: pick(&[
            "embed.api_key",
            "embed_api_key",
            ENV_EMBED_API_KEY,
            "model.api_key",
            "model_api_key",
            ENV_MODEL_API_KEY,
            "openai_api_key",
            ENV_OPENAI_API_KEY,
            "api_key",
        ])
        .map(SecretString::new),
        embed_endpoint: pick(&[
            "embed.endpoint",
            "embed.endpoint_url",
            "embed_endpoint",
            ENV_EMBED_ENDPOINT,
            "model.endpoint",
            "model.endpoint_url",
            "model_endpoint",
            ENV_MODEL_ENDPOINT,
            "openai_base_url",
            ENV_OPENAI_BASE_URL,
            "endpoint",
        ]),
    })
}

fn flatten_toml_strings(val: &toml::Value, prefix: &str, out: &mut BTreeMap<String, String>) {
    match val {
        toml::Value::Table(tbl) => {
            for (k, v) in tbl {
                let next = if prefix.is_empty() {
                    k.clone()
                } else {
                    format!("{prefix}.{k}")
                };
                flatten_toml_strings(v, &next, out);
            }
        }
        toml::Value::String(s) => {
            let trimmed = s.trim();
            if !trimmed.is_empty() && !prefix.is_empty() {
                out.insert(prefix.to_owned(), trimmed.to_owned());
            }
        }
        _ => {}
    }
}

fn loaded_prior_ref(lp: &monoagens_store::LoadedPrior) -> &monoagens_store::LoadedPrior {
    lp
}

#[allow(clippy::too_many_arguments)]
fn build_external_run_record(
    dataset_id: &str,
    entry: &crate::datasets::manifest::DatasetManifestEntry,
    manifest_digest: &str,
    loaded_prior: &monoagens_store::LoadedPrior,
    seed: u64,
    model_id: &str,
    embedder_id: &str,
    door_rules: Vec<String>,
    sampling_rule: &str,
    chosen_item_ids: Vec<String>,
    metric_scores: Vec<MetricScoreInterval>,
    judge_model: Option<String>,
    judge_prompt_version: Option<String>,
    history_gate_wakes: u64,
    partial_run: bool,
    history_reasoning_mode: Option<String>,
    cost_by_purpose: Vec<PurposeCost>,
) -> RunRecord {
    let switches = Switches::all_enabled();
    let pinned_revision = entry
        .sources
        .iter()
        .map(|s| s.revision.as_str())
        .collect::<Vec<_>>()
        .join("+");
    RunRecord {
        schema_version: "1.0".to_owned(),
        run_id: format!("ext-{dataset_id}-seed-{seed}"),
        core_version: env!("CARGO_PKG_VERSION").to_owned(),
        prior_content_version: loaded_prior.prior.content_version.clone(),
        prior_format_version: loaded_prior.prior.version,
        prior_digest: loaded_prior.digest.clone(),
        open_offset: 0,
        model_id: model_id.to_owned(),
        embedder_id: embedder_id.to_owned(),
        door_rules,
        switches: switches.clone(),
        learning_switches: RunRecord::learning_switches_from(&switches),
        seed,
        scenario_id: dataset_id.to_owned(),
        scenario_set: ScenarioSet::External,
        sealed: false,
        checks: Vec::new(),
        passed: true,
        metrics: RunMetricsSummary {
            total_events: chosen_item_ids.len(),
            percepts: chosen_item_ids.len(),
            decisions_go: 0,
            decisions_note: 0,
            decisions_silent: 0,
            decisions_door_blocked: 0,
            actions_dispatched: 0,
            receipts_recorded: 0,
            wakes_per_event: 0.0,
            simulated_span_ms: 0,
            wall_elapsed_ms: 0,
            brain_map_metrics: None,
            detailed: None,
        },
        cost_by_purpose,
        history_reasoning_mode,
        dataset_provenance: Some(DatasetRunProvenance {
            dataset_id: dataset_id.to_owned(),
            pinned_revision,
            manifest_digest: manifest_digest.to_owned(),
            converter_version: entry.converter_version.clone(),
            scorer_version: entry.scorer_version.clone(),
            sampling_rule: sampling_rule.to_owned(),
            chosen_item_ids,
            metric_scores,
            judge_model,
            judge_prompt_version,
            history_gate_wakes,
            partial_run,
        }),
    }
}

fn write_run_record_if_dir(out_dir: Option<&Path>, rec: &RunRecord) -> Result<(), LabError> {
    let Some(dir) = out_dir else {
        return Ok(());
    };
    let safe_id: String = rec
        .scenario_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    fs::write(
        dir.join(format!("{safe_id}__seed-{}.postcard", rec.seed)),
        rec.encode_postcard()?,
    )?;
    fs::write(
        dir.join(format!("{safe_id}__seed-{}.json", rec.seed)),
        serde_json::to_vec_pretty(rec).map_err(|e| LabError::Serialization(e.to_string()))?,
    )?;
    Ok(())
}
