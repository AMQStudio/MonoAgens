//! Benchmark suite execution engine: manifest-verified preflight, `--only` partial runs,
//! `--live --max-cost-usd` scripted preflight cost estimation, configurable judge routing,
//! one-row-per-dataset scorecard construction, and `RunRecord` emission.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use monoagens_core::{CoreConfig, ModelPurpose};
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
use crate::benchmark::{BenchmarkDatasetSpec, BenchmarkDomain, standard_benchmark_registry};
use crate::datasets::capa::{CapaConverter, CapaScorer};
use crate::datasets::dyntom::{DynTomConverter, DynTomScorer};
use crate::datasets::exploretom::{ExploreTomConverter, ExploreTomScorer};
use crate::datasets::fantom::{FantomConverter, FantomScorer};
use crate::datasets::framework::{
    CachingEmbedder, Converter, EmbedderProbe, EmbedderProbeOutcome, ExternalItem, HistoryMode,
    JudgeExecutionConfig, SILENT_THINK_JSON, Scorer, ScorerDetailRecorder, ScorerItemOutcome,
    TranscriptRecord, execute_external_item, execute_external_item_with_embedder,
    load_or_convert_cached, sample_items_stratified,
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
use crate::datasets::ops::{DatasetVerifyResult, verify_manifest_entry};
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
use crate::record::{
    DatasetRunProvenance, MONOAGENS_MODE_FULL, MONOAGENS_MODE_INVALID, MONOAGENS_MODE_OFFLINE,
    MetricScoreInterval, PROMPT_SOURCE_CORE_RENDER, PurposeCost, RunRecord,
};
use crate::scenario::ScenarioSet;
use crate::sha256_hex;

/// Category suites accepted by `--suite` (`Design 8`); each runs one `BenchmarkDomain` only.
pub const CATEGORY_SUITES: [&str; 7] = [
    "memory",
    "prospective",
    "timing",
    "group",
    "tom",
    "indonesian",
    "security",
];

/// Maps a category suite name to its single [`BenchmarkDomain`].
#[must_use]
pub fn suite_domain(suite: &str) -> Option<BenchmarkDomain> {
    match suite {
        "memory" => Some(BenchmarkDomain::Memory),
        "prospective" => Some(BenchmarkDomain::Prospective),
        "timing" => Some(BenchmarkDomain::ProactivityTiming),
        "group" => Some(BenchmarkDomain::GroupChat),
        "tom" => Some(BenchmarkDomain::TheoryOfMind),
        "indonesian" => Some(BenchmarkDomain::IndonesianRegister),
        "security" => Some(BenchmarkDomain::Security),
        _ => None,
    }
}

/// Derives `RunRecord.monoagens_mode` from what actually ran (`Design 8`).
///
/// A live run is `full` only when the provider is real, every switch is on, history was loaded
/// through the live model, and prompts came from the production renderer. Otherwise a live run is
/// recorded `invalid`; a non-live run is `offline` and makes no MonoAgens claim.
#[must_use]
pub fn derive_monoagens_mode(
    live: bool,
    provider_is_live: bool,
    history_reasoning_mode: Option<&str>,
    prompt_source: Option<&str>,
) -> &'static str {
    if !live {
        return MONOAGENS_MODE_OFFLINE;
    }
    let full = provider_is_live
        && history_reasoning_mode != Some("scripted_silent")
        && prompt_source == Some(PROMPT_SOURCE_CORE_RENDER);
    if full {
        MONOAGENS_MODE_FULL
    } else {
        MONOAGENS_MODE_INVALID
    }
}

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
    config: &CoreConfig,
    seed: u64,
    max_cost_usd: f64,
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
                config,
                seed,
                &silent_probe,
                "preflight-scripted",
                false,
                &mut guard,
            )?;
            hist_w = hist_w.saturating_add(exec.total_history_gate_wakes);
        }
        let judge_calls = if official_judge_model(ds_id).is_some() {
            probe_calls
        } else {
            0
        };
        // Live runs load history through the live model, so every history wake is a paid call.
        let paid_history_calls = hist_w;
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
    pub max_cost_usd: Option<f64>,
    pub out_dir: Option<PathBuf>,
    /// Root scanned by `--suite standard` for the latest per-category run records.
    pub runs_root: Option<PathBuf>,
    pub target_items_per_dataset: Option<usize>,
    pub pilot_prefix_n: Option<usize>,
    pub preflight_only: bool,
    pub allow_test_embedders: bool,
    pub require_semantic_embedder: bool,
    pub probe_model_name: String,
    pub probe_provider: Option<&'a dyn ModelProvider>,
    pub judge_config: Option<JudgeConfig>,
    pub judge_provider: Option<&'a dyn ModelProvider>,
    pub embedder: Option<&'a dyn Embedder>,
    pub reasoning_effort: Option<String>,
    pub judge_reasoning_effort: Option<String>,
    pub extra_header_names: Vec<String>,
    pub provider_endpoint_host: Option<String>,
    pub embedder_backend: Option<String>,
    pub embedder_revision: Option<String>,
    pub handlers: DatasetHandlerRegistry,
}

impl Default for BenchmarkExecutionOptions<'_> {
    fn default() -> Self {
        Self {
            manifest_path: None,
            only_ids: None,
            seed: 1,
            live: false,
            max_cost_usd: None,
            out_dir: None,
            runs_root: None,
            target_items_per_dataset: None,
            pilot_prefix_n: None,
            preflight_only: false,
            allow_test_embedders: false,
            require_semantic_embedder: false,
            probe_model_name: "scripted".to_owned(),
            probe_provider: None,
            judge_config: None,
            judge_provider: None,
            embedder: None,
            reasoning_effort: None,
            judge_reasoning_effort: None,
            extra_header_names: Vec::new(),
            provider_endpoint_host: None,
            embedder_backend: None,
            embedder_revision: None,
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
/// A stale converted cache is regenerable by `load_or_convert_cached`; it never blocks a run.
fn only_regenerable_cache_failures(v: &DatasetVerifyResult) -> bool {
    !v.failures.is_empty()
        && v.failures
            .iter()
            .all(|f| f.contains("stale converted cache"))
}

pub fn verify_required_datasets_with_manifest(
    manifest_path: &Path,
    datasets_dir: &Path,
    registry: &[BenchmarkDatasetSpec],
    only_ids: Option<&[String]>,
) -> Result<(DatasetsManifest, Vec<PathBuf>), LabError> {
    // Registry↔manifest consistency always uses the full standard registry, even when the caller
    // asks for one category; `registry` below only selects which required dataset ids to verify.
    let manifest = load_and_validate_manifest(manifest_path, &standard_benchmark_registry())?;

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
        if v.verified || only_regenerable_cache_failures(&v) {
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
    config: &CoreConfig,
    opts: &BenchmarkExecutionOptions<'_>,
) -> Result<ExternalExecutionResult, LabError> {
    if opts.live && opts.max_cost_usd.is_none() {
        return Err(LabError::Protocol(
            "--live benchmark run requires --max-cost-usd <cap>".to_owned(),
        ));
    }
    if opts.require_semantic_embedder {
        let valid = match opts.embedder {
            Some(emb) => {
                let backend = emb.backend_name().to_ascii_lowercase();
                let is_non_semantic = backend == "hash"
                    || backend == "none"
                    || backend == "scripted"
                    || emb.model_ver().is_none();
                !is_non_semantic || opts.allow_test_embedders
            }
            None => false,
        };
        if !valid {
            return Err(LabError::Protocol(
                "refusing to start standard benchmark without a configured semantic embedder (EmbeddingGemma 2); run `monoagens-lab datasets pin embeddinggemma-2 && monoagens-lab datasets fetch --record-checksums embeddinggemma-2` or pass --embed-endpoint"
                    .to_owned(),
            ));
        }
    }

    let caching_embedder = opts.embedder.map(CachingEmbedder::new);
    let active_embedder: Option<&dyn Embedder> =
        caching_embedder.as_ref().map(|c| c as &dyn Embedder);

    let manifest_raw = fs::read(manifest_path).unwrap_or_default();
    let manifest_digest = format!("sha256:{}", sha256_hex(&manifest_raw));
    let loaded_config = monoagens_store::LoadedCoreConfig::from_config(config.clone())?;

    let suite_scope = suite_domain(suite);
    let external_specs: Vec<&BenchmarkDatasetSpec> = registry
        .iter()
        .filter(|s| s.requires_external_corpus)
        .filter(|s| suite_scope.is_none_or(|dom| s.domain == dom))
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
        if !verify_res.verified && !only_regenerable_cache_failures(&verify_res) {
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
            let mut chosen_items: Vec<ExternalItem> = converted
                .items
                .iter()
                .filter(|it| chosen_set.contains(it.item_id.as_str()))
                .cloned()
                .collect();
            let mut rule_name = sampling_rule.rule_name;
            if let Some(pilot_n) = opts.pilot_prefix_n {
                chosen_items = take_pilot_prefix(&spec.id, chosen_items, pilot_n);
                rule_name = format!("{rule_name}__pilot_prefix_{}", chosen_items.len());
            }
            preflight_inputs.push((spec.id.clone(), chosen_items.clone()));
            converted_cache.insert(spec.id.clone(), (converted, chosen_items, rule_name));
        }
    }

    let preflight_estimate = if opts.live || opts.preflight_only {
        let cap = opts.max_cost_usd.unwrap_or(1_000_000.0);
        let est = estimate_live_preflight(
            &preflight_inputs,
            config,
            opts.seed,
            cap,
        )?;
        if let Some(dir) = &opts.out_dir {
            fs::create_dir_all(dir)?;
            fs::write(
                dir.join("preflight.json"),
                serde_json::to_vec_pretty(&est)
                    .map_err(|e| LabError::Serialization(e.to_string()))?,
            )?;
        }
        if opts.live && !est.within_budget {
            return Err(LabError::Protocol(format!(
                "preflight estimated worst-case USD ${:.4} exceeds --max-cost-usd ${:.4}; aborting before any paid call",
                est.total_worst_case_usd, cap
            )));
        }
        Some(est)
    } else {
        None
    };

    if opts.preflight_only {
        return Ok((
            Vec::new(),
            Vec::new(),
            is_partial,
            left_out,
            preflight_estimate,
        ));
    }

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
        if !verify_res.verified
            && !only_regenerable_cache_failures(&verify_res)
            && suite_domain(suite).is_none()
        {
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
                active_embedder,
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
                    let mut rec = build_external_run_record(
                        &spec.id,
                        entry,
                        &manifest_digest,
                        &loaded_config,
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
                        MONOAGENS_MODE_OFFLINE,
                        None,
                    );
                    if let Some(prov) = rec.dataset_provenance.as_mut() {
                        prov.embedder_backend = opts
                            .embedder_backend
                            .clone()
                            .or_else(|| active_embedder.map(|e| e.backend_name().to_owned()));
                        prov.embedder_revision = opts.embedder_revision.clone();
                    }
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
                    let mut rec = build_external_run_record(
                        &spec.id,
                        entry,
                        &manifest_digest,
                        &loaded_config,
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
                        MONOAGENS_MODE_OFFLINE,
                        None,
                    );
                    if let Some(prov) = rec.dataset_provenance.as_mut() {
                        prov.embedder_backend = opts
                            .embedder_backend
                            .clone()
                            .or_else(|| active_embedder.map(|e| e.backend_name().to_owned()));
                        prov.embedder_revision = opts.embedder_revision.clone();
                    }
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

        let ds_start = Instant::now();
        let started_at_utc = utc_iso_now();
        let mut predictions = Vec::new();
        let mut total_hist_wakes = 0_u64;
        let mut history_mode_recorded: Option<String> = None;
        let mut total_tokens_in = 0_u64;
        let mut total_tokens_out = 0_u64;
        let mut combined_costs: BTreeMap<u8, PurposeCost> = BTreeMap::new();
        let mut total_decisions_go = 0usize;
        let mut total_decisions_note = 0usize;
        let mut total_decisions_silent = 0usize;
        let mut total_decisions_door_blocked = 0usize;
        let mut total_actions_dispatched = 0usize;
        let mut total_percepts_count = 0usize;
        let mut model_latencies_ms = Vec::new();
        let mut failed_items = Vec::new();
        let mut transcripts_by_item: BTreeMap<String, Vec<TranscriptRecord>> = BTreeMap::new();

        for item in &chosen_items {
            if guard.budget_exhausted() {
                return Err(LabError::Protocol(format!(
                    "CostGuard halted '--live' benchmark run at budget cap ${:.4}",
                    opts.max_cost_usd.unwrap_or(0.0)
                )));
            }
            let exec = execute_external_item_with_embedder(
                item,
                config,
                opts.seed,
                probe_provider,
                &opts.probe_model_name,
                opts.live,
                &mut guard,
                active_embedder,
                None,
                None,
                None,
            )?;
            if history_mode_recorded.is_none() && item.history_mode == HistoryMode::StaticSilent {
                history_mode_recorded = Some(exec.history_reasoning_mode);
            }
            total_hist_wakes = total_hist_wakes.saturating_add(exec.total_history_gate_wakes);
            total_decisions_go += exec.decisions_go;
            total_decisions_note += exec.decisions_note;
            total_decisions_silent += exec.decisions_silent;
            total_decisions_door_blocked += exec.decisions_door_blocked;
            total_actions_dispatched += exec.actions_dispatched;
            total_percepts_count += exec.percepts_count;
            model_latencies_ms.extend(exec.model_latencies_ms);
            failed_items.extend(exec.failed_items);
            transcripts_by_item.insert(item.item_id.clone(), exec.transcript_events);

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
        let detail_recorder = Arc::new(Mutex::new(ScorerDetailRecorder::default()));
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
            recorder: Some(Arc::clone(&detail_recorder)),
        };
        let scored = scorer.score(&predictions, &converted.gold_book, &judge_exec, opts.seed)?;
        let mut judge_calls_total = 0_u64;
        let mut judge_tokens_in_total = 0_u64;
        let mut judge_tokens_out_total = 0_u64;
        if let Some(gj) = guarded_judge {
            let judge_cost = gj.into_purpose_cost();
            judge_calls_total = judge_cost.calls;
            judge_tokens_in_total = judge_cost.tokens_in;
            judge_tokens_out_total = judge_cost.tokens_out;
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

        let detail = detail_recorder
            .lock()
            .map(|d| {
                (
                    d.outcomes.clone(),
                    d.category_metrics.clone(),
                    d.failed_items.clone(),
                )
            })
            .unwrap_or_default();
        let (scorer_outcomes, category_metrics, scorer_failures) = detail;
        failed_items.extend(scorer_failures);

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

        // Every model call above went through the production renderer; record that and derive
        // whether this run actually exercised the full MonoAgens (`Design 8`).
        let prompt_source = Some(PROMPT_SOURCE_CORE_RENDER);
        let provider_is_live = probe_provider.provider_name() != "scripted";
        let monoagens_mode = derive_monoagens_mode(
            opts.live,
            provider_is_live,
            history_mode_recorded.as_deref(),
            prompt_source,
        );
        // A deviating live run is never a MonoAgens result: keep its metrics out of the scorecard
        // and the Pareto frontier by marking the row `not_scored`.
        let invalid_run = monoagens_mode == MONOAGENS_MODE_INVALID;
        let row_status = if invalid_run || (!any_scored && any_not_scored) {
            ScorecardStatus::NotScored
        } else if is_partial || any_not_scored {
            ScorecardStatus::Partial
        } else {
            ScorecardStatus::Run
        };

        let chosen_ids: Vec<String> = chosen_items.iter().map(|i| i.item_id.clone()).collect();
        let e_tok = (total_tokens_in + total_tokens_out) as f64;
        let cost_usd = guard.cost_usd_used;
        let finished_at_utc = utc_iso_now();
        let wall_elapsed_ms = ds_start.elapsed().as_millis() as u64;

        let mut rec = build_external_run_record(
            &spec.id,
            entry,
            &manifest_digest,
            loaded_config_ref(&loaded_config),
            opts.seed,
            &opts.probe_model_name,
            active_embedder
                .and_then(|e| e.model_ver())
                .unwrap_or("none"),
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
            monoagens_mode,
            prompt_source,
        );
        rec.metrics.percepts = total_percepts_count;
        rec.metrics.decisions_go = total_decisions_go;
        rec.metrics.decisions_note = total_decisions_note;
        rec.metrics.decisions_silent = total_decisions_silent;
        rec.metrics.decisions_door_blocked = total_decisions_door_blocked;
        rec.metrics.actions_dispatched = total_actions_dispatched;
        rec.metrics.wakes_per_event = if total_percepts_count > 0 {
            total_decisions_go as f64 / total_percepts_count as f64
        } else {
            0.0
        };
        rec.metrics.wall_elapsed_ms = wall_elapsed_ms;
        if let Some(prov) = rec.dataset_provenance.as_mut() {
            prov.reasoning_effort = opts.reasoning_effort.clone();
            prov.judge_reasoning_effort = opts.judge_reasoning_effort.clone();
            prov.extra_header_names = opts.extra_header_names.clone();
            prov.provider_endpoint_host = opts.provider_endpoint_host.clone();
            prov.embedder_backend = opts
                .embedder_backend
                .clone()
                .or_else(|| active_embedder.map(|e| e.backend_name().to_owned()));
            prov.embedder_revision = opts.embedder_revision.clone();
            prov.judge_calls = judge_calls_total;
            prov.judge_tokens_in = judge_tokens_in_total;
            prov.judge_tokens_out = judge_tokens_out_total;
            prov.category_metrics = category_metrics;
            prov.failed_items = failed_items;
            prov.model_latencies_ms = model_latencies_ms;
            prov.started_at_utc = Some(started_at_utc);
            prov.finished_at_utc = Some(finished_at_utc);
        }
        write_run_record_if_dir(opts.out_dir.as_deref(), &rec)?;
        write_item_transcripts_and_outcomes(
            opts.out_dir.as_deref(),
            &spec.id,
            opts.seed,
            &chosen_items,
            &predictions,
            &converted.gold_book,
            &scorer_outcomes,
            &transcripts_by_item,
        )?;
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
            reason: if invalid_run {
                Some("invalid: not full MonoAgens".to_owned())
            } else if row_status == ScorecardStatus::NotScored {
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
                        strict_schema: request.response_schema
                            != crate::reason_seam::model::ResponseSchemaMode::None,
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
                        strict_schema: request.response_schema
                            != crate::reason_seam::model::ResponseSchemaMode::None,
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

fn loaded_config_ref(lc: &monoagens_store::LoadedCoreConfig) -> &monoagens_store::LoadedCoreConfig {
    lc
}

#[allow(clippy::too_many_arguments)]
fn build_external_run_record(
    dataset_id: &str,
    entry: &crate::datasets::manifest::DatasetManifestEntry,
    manifest_digest: &str,
    loaded_config: &monoagens_store::LoadedCoreConfig,
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
    monoagens_mode: &str,
    prompt_source: Option<&str>,
) -> RunRecord {
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
        config_format_version: u32::from(loaded_config.config.format_version),
        config_digest: loaded_config.digest.clone(),
        model_blake3: loaded_config
            .config
            .expected_blake3
            .as_ref()
            .map(monoagens_core::blake3::to_hex),
        online_adaptation: loaded_config.config.online_adaptation,
        open_offset: 0,
        model_id: model_id.to_owned(),
        embedder_id: embedder_id.to_owned(),
        door_rules,
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
        monoagens_mode: Some(monoagens_mode.to_owned()),
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
            judge_calls: 0,
            judge_tokens_in: 0,
            judge_tokens_out: 0,
            history_gate_wakes,
            partial_run,
            reasoning_effort: None,
            judge_reasoning_effort: None,
            extra_header_names: Vec::new(),
            provider_endpoint_host: None,
            embedder_backend: None,
            embedder_revision: None,
            prompt_source: prompt_source.map(str::to_owned),
            development_exposure: None,
            memory_usage: None,
            category_metrics: BTreeMap::new(),
            failed_items: Vec::new(),
            model_latencies_ms: Vec::new(),
            started_at_utc: None,
            finished_at_utc: None,
        }),
    }
}

/// Per-item outcome summary persisted alongside the RunRecord for failure analysis & reporting.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ItemOutcomeSummary {
    pub dataset_id: String,
    pub item_id: String,
    pub probe_id: String,
    pub cluster_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    pub question: String,
    pub gold_answer: String,
    pub soul_answer: String,
    pub spoke: bool,
    pub metric_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub passed: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub judge_verdict: Option<String>,
    #[serde(default)]
    pub failed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_reason: Option<String>,
    pub transcript_rel_path: String,
}

#[allow(clippy::too_many_arguments)]
fn write_item_transcripts_and_outcomes(
    out_dir: Option<&Path>,
    dataset_id: &str,
    seed: u64,
    items: &[ExternalItem],
    predictions: &[crate::datasets::ExternalPrediction],
    gold: &crate::datasets::GoldBook,
    scorer_outcomes: &[ScorerItemOutcome],
    transcripts_by_item: &BTreeMap<String, Vec<TranscriptRecord>>,
) -> Result<(), LabError> {
    let Some(dir) = out_dir else {
        return Ok(());
    };
    let safe_ds: String = dataset_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let tx_dir = dir.join("transcripts").join(&safe_ds);
    fs::create_dir_all(&tx_dir)?;

    let mut outcome_map: BTreeMap<(&str, &str), &ScorerItemOutcome> = BTreeMap::new();
    for out in scorer_outcomes {
        outcome_map.insert((out.item_id.as_str(), out.probe_id.as_str()), out);
    }
    let mut pred_map: BTreeMap<(&str, &str), &crate::datasets::ExternalPrediction> =
        BTreeMap::new();
    for p in predictions {
        pred_map.insert((p.item_id.as_str(), p.probe_id.as_str()), p);
    }

    let mut summaries = Vec::new();
    for item in items {
        let safe_item: String = item
            .item_id
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        let tx_rel = format!("transcripts/{safe_ds}/{safe_item}.jsonl");
        let tx_path = dir.join(&tx_rel);
        let mut lines = Vec::new();
        if let Some(evts) = transcripts_by_item.get(&item.item_id) {
            for ev in evts {
                if let Ok(line) = serde_json::to_string(ev) {
                    lines.push(line);
                }
            }
        }
        // Clearly separated post-scoring GoldBook + Judge verdict entries.
        for probe in &item.probes {
            let question = String::from_utf8_lossy(probe.percept.payload.as_bytes()).into_owned();
            let pred_opt = pred_map
                .get(&(item.item_id.as_str(), probe.probe_id.as_str()))
                .copied();
            let soul_ans = pred_opt
                .and_then(|p| p.response_text.clone())
                .unwrap_or_default();
            let spoke = pred_opt.is_some_and(|p| p.spoke);
            let gold_opt = gold.find(&item.item_id, &probe.probe_id);
            let out_opt = outcome_map
                .get(&(item.item_id.as_str(), probe.probe_id.as_str()))
                .copied();

            if let Some(g) = gold_opt {
                lines.push(
                    serde_json::json!({
                        "record_type": "gold_after_scoring",
                        "probe_id": probe.probe_id,
                        "gold_answer": g.gold_answer,
                        "accepted_answers": g.accepted_answers,
                        "category": g.category,
                        "privacy_conflict": g.privacy_conflict,
                    })
                    .to_string(),
                );
            }
            if let Some(o) = out_opt {
                lines.push(
                    serde_json::json!({
                        "record_type": "score_verdict",
                        "probe_id": probe.probe_id,
                        "metric_name": o.metric_name,
                        "score": o.score,
                        "passed": o.passed,
                        "judge_prompt": o.judge_prompt,
                        "judge_verdict": o.judge_verdict,
                        "failed": o.failed,
                        "failure_reason": o.failure_reason,
                    })
                    .to_string(),
                );
            }

            let gold_ans = gold_opt.map(|g| g.gold_answer.clone()).unwrap_or_default();
            let category = out_opt
                .and_then(|o| o.category.clone())
                .or_else(|| gold_opt.and_then(|g| g.category.clone()));
            let (metric_name, score, passed, judge_verdict, failed, failure_reason) =
                if let Some(o) = out_opt {
                    (
                        o.metric_name.clone(),
                        o.score,
                        o.passed,
                        o.judge_verdict.clone(),
                        o.failed,
                        o.failure_reason.clone(),
                    )
                } else {
                    let norm_match = if let Some(g) = gold_opt {
                        let s_low = soul_ans.to_ascii_lowercase();
                        let g_low = g.gold_answer.to_ascii_lowercase();
                        !g_low.is_empty() && (s_low == g_low || s_low.contains(&g_low))
                    } else {
                        false
                    };
                    (
                        "heuristic_match".to_owned(),
                        Some(if norm_match { 1.0 } else { 0.0 }),
                        Some(norm_match),
                        None,
                        false,
                        None,
                    )
                };

            summaries.push(ItemOutcomeSummary {
                dataset_id: dataset_id.to_owned(),
                item_id: item.item_id.clone(),
                probe_id: probe.probe_id.clone(),
                cluster_id: item.cluster_id.clone(),
                category,
                question,
                gold_answer: gold_ans,
                soul_answer: soul_ans,
                spoke,
                metric_name,
                score,
                passed,
                judge_verdict,
                failed,
                failure_reason,
                transcript_rel_path: tx_rel.clone(),
            });
        }

        fs::write(&tx_path, lines.join("\n") + "\n")?;
    }

    fs::write(
        dir.join(format!("{safe_ds}__seed-{seed}.outcomes.json")),
        serde_json::to_vec_pretty(&summaries)
            .map_err(|e| LabError::Serialization(e.to_string()))?,
    )?;
    Ok(())
}

/// Selects a deterministic pilot prefix from the already-sampled R-54 items.
/// For story/conversation-unit datasets (`hi_tom`, `fantom`, `opentom`, `dyntom`, `tomi`,
/// `locomo`, `capa`), keeps whole `cluster_id` units (stopping once `max_items` is reached,
/// always keeping at least 1 complete cluster).
fn take_pilot_prefix(
    dataset_id: &str,
    items: Vec<ExternalItem>,
    max_items: usize,
) -> Vec<ExternalItem> {
    if items.len() <= max_items || max_items == 0 {
        return items;
    }
    let whole_cluster_unit = matches!(
        dataset_id,
        "hi_tom" | "fantom" | "opentom" | "dyntom" | "tomi" | "locomo" | "capa"
    );
    if !whole_cluster_unit {
        return items.into_iter().take(max_items).collect();
    }
    let mut allowed_clusters: BTreeSet<String> = BTreeSet::new();
    let mut count = 0usize;
    for it in &items {
        if allowed_clusters.contains(&it.cluster_id) {
            count += 1;
            continue;
        }
        if !allowed_clusters.is_empty() && count >= max_items {
            break;
        }
        allowed_clusters.insert(it.cluster_id.clone());
        count += 1;
    }
    items
        .into_iter()
        .filter(|it| allowed_clusters.contains(&it.cluster_id))
        .collect()
}

/// Default root scanned by `--suite standard` for the latest per-category runs.
pub const DEFAULT_RUNS_ROOT: &str = "lab/runs";

/// Collects the newest persisted [`RunRecord`] per dataset id under `root` (recursively,
/// skipping `report/` and `transcripts/`).
fn collect_latest_run_records(
    root: &Path,
) -> Result<BTreeMap<String, (RunRecord, SystemTime)>, LabError> {
    let mut latest: BTreeMap<String, (RunRecord, SystemTime)> = BTreeMap::new();
    if !root.is_dir() {
        return Ok(latest);
    }
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir)? {
            let path = entry?.path();
            if path.is_dir() {
                let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
                if name != "report" && name != "transcripts" {
                    stack.push(path);
                }
                continue;
            }
            let Some(fname) = path.file_name().and_then(|s| s.to_str()) else {
                continue;
            };
            if !fname.ends_with(".json")
                || !fname.contains("__seed-")
                || fname.ends_with(".outcomes.json")
            {
                continue;
            }
            let Ok(raw) = fs::read_to_string(&path) else {
                continue;
            };
            let Ok(rec) = serde_json::from_str::<RunRecord>(&raw) else {
                continue;
            };
            let mtime = fs::metadata(&path)
                .and_then(|m| m.modified())
                .unwrap_or(UNIX_EPOCH);
            let ds = rec.scenario_id.clone();
            match latest.get(&ds) {
                Some((_, existing)) if *existing >= mtime => {}
                _ => {
                    latest.insert(ds, (rec, mtime));
                }
            }
        }
    }
    Ok(latest)
}

/// Builds the `--suite standard` index: strictly one row per registered dataset, taken from the
/// latest per-category run under `opts.runs_root` (default `lab/runs`). Never executes a dataset
/// and never mixes categories in one execution.
pub fn build_standard_index(
    manifest_path: &Path,
    registry: &[BenchmarkDatasetSpec],
    opts: &BenchmarkExecutionOptions<'_>,
) -> Result<ExternalExecutionResult, LabError> {
    let manifest = load_and_validate_manifest(manifest_path, &standard_benchmark_registry())?;
    let runs_root = opts
        .runs_root
        .clone()
        .unwrap_or_else(|| PathBuf::from(DEFAULT_RUNS_ROOT));
    crate::scenario::ensure_not_sealed_path(&runs_root)?;
    let latest = collect_latest_run_records(&runs_root)?;

    let only_set: Option<BTreeSet<String>> = opts
        .only_ids
        .as_ref()
        .map(|ids| ids.iter().cloned().collect());
    let is_partial = only_set.is_some();
    let mut left_out = Vec::new();
    let mut rows = Vec::new();
    let mut records = Vec::new();

    for spec in registry.iter().filter(|s| s.requires_external_corpus) {
        let entry = manifest.get(&spec.id).ok_or_else(|| {
            LabError::Protocol(format!("manifest missing external dataset '{}'", spec.id))
        })?;
        let base_metrics: BTreeMap<String, Option<f64>> = spec
            .primary_metrics
            .iter()
            .map(|m| (m.clone(), None))
            .collect();
        let excluded_by_only = only_set.as_ref().is_some_and(|set| !set.contains(&spec.id));
        if excluded_by_only {
            left_out.push(spec.id.clone());
        }

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

        let loaded = latest.get(&spec.id);
        let offline_ok = loaded
            .is_some_and(|(rec, _)| rec.monoagens_mode.as_deref() == Some(MONOAGENS_MODE_OFFLINE));
        let valid = loaded.is_some_and(|(rec, _)| rec.is_full_monoagens()) || offline_ok;

        if excluded_by_only {
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

        let Some((rec, _)) = loaded else {
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
                reason: Some("no per-category run recorded".to_owned()),
            });
            continue;
        };

        if !valid {
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
                reason: Some("invalid: not full MonoAgens".to_owned()),
            });
            continue;
        }

        let prov = rec.dataset_provenance.as_ref();
        let intervals = prov.map(|p| p.metric_scores.clone()).unwrap_or_default();
        let mut metric_map: BTreeMap<String, Option<f64>> = BTreeMap::new();
        for iv in &intervals {
            metric_map.insert(iv.metric.clone(), iv.value);
        }
        let any_scored = intervals.iter().any(|iv| iv.value.is_some());
        let any_unscored = intervals.iter().any(|iv| iv.value.is_none());
        let partial_flag = prov.is_some_and(|p| p.partial_run);
        let status = if !any_scored && any_unscored {
            ScorecardStatus::NotScored
        } else if partial_flag || any_unscored {
            ScorecardStatus::Partial
        } else {
            ScorecardStatus::Run
        };
        let cost_usd: f64 = rec.cost_by_purpose.iter().map(|p| p.cost_usd).sum();
        let e_tok: f64 = rec
            .cost_by_purpose
            .iter()
            .map(|p| (p.tokens_in + p.tokens_out) as f64)
            .sum();
        rows.push(ScorecardRow {
            dataset_id: spec.id.clone(),
            domain: spec.domain,
            lane: entry.lane,
            status,
            core_applicability: entry.core_applicability,
            contamination_risk: entry.contamination_risk,
            metrics: metric_map,
            metric_intervals: intervals,
            cost_usd,
            e_tok,
            reason: None,
        });
        records.push(rec.clone());
    }

    Ok((rows, records, is_partial, left_out, None))
}

fn utc_iso_now() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let h = rem / 3600;
    let m = (rem % 3600) / 60;
    let s = rem % 60;
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = y + if mo <= 2 { 1 } else { 0 };
    format!("{year:04}-{mo:02}-{d:02}T{h:02}:{m:02}:{s:02}Z")
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
