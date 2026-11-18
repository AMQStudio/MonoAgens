//! Phase 0 memory suite: full MonoAgens on LoCoMo and LongMemEval-S, resumable per
//! conversation/history, with a pilot gate and a memory-only report.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use monoagens_core::{Prior, Switches};
use monoagens_protocol::{
    CapabilityDescriptor, CapabilityFlags, CapabilityId, CapabilitySchemaHash, OpaquePayload,
};
use monoagens_reason::embed::Embedder;
use monoagens_reason::model::{ModelProvider, OpenAiModelConfig, OpenAiModelProvider};
use serde::{Deserialize, Serialize};

use crate::LabError;
use crate::datasets::framework::{
    ConvertedDataset, Converter, GoldBook, JudgeExecutionConfig, SplitMix64, TranscriptRecord,
    execute_external_item_with_embedder, load_or_convert_cached,
};
use crate::datasets::manifest::{DatasetsManifest, load_and_validate_manifest};
use crate::datasets::ops::verify_manifest_entry;
use crate::datasets::runner::{derive_monoagens_mode, resolve_manifest_path};
use crate::datasets::{LocomoConverter, LongMemEvalConverter};
use crate::guard::CostGuard;
use crate::memory::judge::{HttpJudgeClient, JudgeRunner};
use crate::memory::prompts::{
    ANSWER_FORMAT_PROMPT_VERSION, LOCOMO_ANSWER_FORMAT_V1, LOCOMO_PROMPT_VERSION,
    LONGMEMEVAL_ANSWER_FORMAT_V1, LONGMEMEVAL_PROMPT_VERSION,
};
use crate::memory::proxy::{
    CallAttribution, LedgerEntry, RunProfile, peak_multiplier_at, read_ledger_totals,
};
use crate::memory::report;
use crate::memory::scoring::{
    MemoryItemScore, MemoryScore, ScoreContext, score_locomo, score_longmemeval,
};
use crate::record::{
    DatasetRunProvenance, MemoryUsageTotals, MetricScoreInterval, PROMPT_SOURCE_CORE_RENDER,
    PurposeCost, RunRecord, UsageBreakdown,
};
use crate::scenario::ScenarioSet;
use crate::sha256_hex;

/// Harness behavior version; changing it invalidates item checkpoints.
pub const MEMORY_HARNESS_VERSION: &str = "memory_harness_v2";
/// Per-request model timeout for memory-suite calls (matches the provider timeout).
pub const MEMORY_MODEL_TIMEOUT_MS: u64 = 600_000;
/// Configured model output cap for every MonoAgens and judge call (`model.max_output_tokens`).
pub const MEMORY_MAX_OUTPUT_TOKENS: u32 = 4096;

/// Which portion of phase 0 to run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryPhase {
    /// First LoCoMo conversation and 20 seeded stratified LongMemEval-S questions.
    Pilot,
    /// All 10 LoCoMo conversations and all 500 LongMemEval-S questions.
    Full,
}

impl MemoryPhase {
    /// Parses `--phase`.
    pub fn parse(value: &str) -> Result<Self, LabError> {
        match value {
            "pilot" => Ok(Self::Pilot),
            "0" | "full" => Ok(Self::Full),
            other => Err(LabError::Protocol(format!(
                "unknown memory phase '{other}' (expected 0)"
            ))),
        }
    }
}

/// Options for one memory-suite run.
pub struct MemorySuiteOptions {
    pub datasets_dir: PathBuf,
    pub manifest_path: Option<PathBuf>,
    pub run_dir: PathBuf,
    pub prior: Prior,
    pub seed: u64,
    pub phase: MemoryPhase,
    pub live: bool,
    /// When true, run only the render dry-run gate and exit without live model calls.
    pub render_dry_run: bool,
    pub profile: Option<RunProfile>,
    pub max_cost_usd: Option<f64>,
    /// Chat-completions endpoint of the metering proxy for MonoAgens.
    pub provider_endpoint: Option<String>,
    /// Chat-completions endpoint of the metering proxy for the judge.
    pub judge_endpoint: Option<String>,
    /// Test/offline override for the participant provider.
    pub probe_provider: Option<Arc<dyn ModelProvider>>,
    /// Test/offline override for the judge.
    pub judge: Option<Arc<dyn JudgeRunner>>,
    pub embedder: Option<Arc<dyn Embedder>>,
    /// When false, skip report generation (used by resume tests).
    pub write_report: bool,
    /// Restrict the run to these datasets (tests); `None` runs both.
    pub only_datasets: Option<Vec<String>>,
    /// Truncate selected items per dataset (tests).
    pub max_items_per_dataset: Option<usize>,
    /// Earlier run directories whose finished items and judge results are reused.
    pub reuse_from: Vec<PathBuf>,
}

/// Per-dataset artifact references.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DatasetArtifact {
    pub dataset_id: String,
    pub record_rel_path: String,
    pub items_rel_path: String,
    pub n_items: usize,
}

/// Pilot measurements and the projection for the rest of phase 0.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PilotSummary {
    pub locomo_item_ids: Vec<String>,
    pub longmemeval_item_ids: Vec<String>,
    pub calls: u64,
    pub ingest_calls: u64,
    pub answer_calls: u64,
    pub judge_calls: u64,
    pub mean_tokens_in_per_call: f64,
    pub p95_tokens_in_per_call: f64,
    pub mean_tokens_out_per_call: f64,
    pub p95_tokens_out_per_call: f64,
    pub cached_share: f64,
    pub cached_unknown_calls: u64,
    pub silence_rate: f64,
    pub errors: u64,
    pub retries: u64,
    pub rate_limit_hits: u64,
    pub latency_p50_ms: u64,
    pub latency_p95_ms: u64,
    pub list_cost_usd: f64,
    pub allowance_cost_usd: f64,
    pub projected_rest_list_cost_usd: f64,
    pub projected_rest_wall_clock_s: f64,
}

/// Run-level manifest written to `memory_run.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryRunFile {
    pub model: String,
    pub reasoning_effort: String,
    pub judge_model: String,
    pub judge_prompt_versions: BTreeMap<String, String>,
    pub answer_format_version: String,
    pub seed: u64,
    pub phase: String,
    pub live: bool,
    pub status: String,
    pub development_exposure: String,
    pub started_at_utc: String,
    pub finished_at_utc: String,
    pub datasets: Vec<DatasetArtifact>,
    pub pilot: Option<PilotSummary>,
    pub notes: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_limits: Option<crate::reason_seam::model::OutputLimitProfile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runaways_by_purpose: Option<BTreeMap<String, u64>>,
}

/// Outcome returned by [`run_memory_suite`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemorySuiteOutcome {
    pub run_dir: PathBuf,
    pub datasets: Vec<DatasetArtifact>,
    pub pilot: Option<PilotSummary>,
    pub stopped_reason: Option<String>,
    pub report_md: Option<PathBuf>,
    pub report_html: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct CompletedItem {
    dataset: String,
    item_id: String,
    cluster_id: String,
    answer_format_version: String,
    #[serde(default)]
    harness_version: String,
    predictions: Vec<crate::datasets::ExternalPrediction>,
    history_reasoning_mode: String,
    total_history_gate_wakes: u64,
    decisions_go: usize,
    decisions_note: usize,
    decisions_silent: usize,
    decisions_door_blocked: usize,
    actions_dispatched: usize,
    percepts_count: usize,
    model_latencies_ms: Vec<u64>,
    failed_items: Vec<crate::record::FailedItemRecord>,
}

fn dataset_dir(datasets_dir: &Path, entry: &crate::datasets::DatasetManifestEntry) -> PathBuf {
    datasets_dir.join(&entry.relative_dir)
}

fn load_dataset(
    converter: &dyn Converter,
    datasets_dir: &Path,
    manifest: &DatasetsManifest,
    dataset_id: &str,
) -> Result<ConvertedDataset, LabError> {
    let entry = manifest.get(dataset_id).ok_or_else(|| {
        LabError::Protocol(format!("manifest is missing memory dataset '{dataset_id}'"))
    })?;
    let verify = verify_manifest_entry(entry, datasets_dir)?;
    let only_stale_cache = !verify.failures.is_empty()
        && verify
            .failures
            .iter()
            .all(|f| f.contains("stale converted cache"));
    if !verify.verified && !only_stale_cache {
        return Err(LabError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!(
                "memory dataset '{dataset_id}' is not fetched/verified under '{}': {}",
                datasets_dir.display(),
                verify.failures.join("; ")
            ),
        )));
    }
    let (converted, _) =
        load_or_convert_cached(converter, &dataset_dir(datasets_dir, entry), entry)?;
    Ok(converted)
}

fn apply_answer_format(items: &mut [crate::datasets::ExternalItem], instruction: &str) {
    for item in items {
        for probe in &mut item.probes {
            let existing = String::from_utf8_lossy(probe.percept.payload.as_bytes()).into_owned();
            if existing.contains(instruction) {
                continue;
            }
            let combined = format!("{existing}\n\n{instruction}");
            probe.percept.payload = OpaquePayload::from_bytes(combined.into_bytes());
        }
    }
}

/// Gives every item an `a.speak` capability at its probe place, so the soul can dispatch an
/// answer exactly as a chat body's declared capability allows.
fn ensure_speak_capabilities(items: &mut [crate::datasets::ExternalItem]) {
    for item in items {
        if item
            .capabilities
            .iter()
            .any(|c| c.id == CapabilityId::new("a.speak"))
        {
            continue;
        }
        let Some(probe) = item.probes.first() else {
            continue;
        };
        item.capabilities.push(CapabilityDescriptor {
            id: CapabilityId::new("a.speak"),
            description: "Answer the question in the conversation, using only what you remember."
                .to_owned(),
            places: vec![probe.percept.place.clone()],
            flags: CapabilityFlags {
                communicative: true,
                identity_changing: false,
                irreversible: false,
            },
            typical_latency_ms: 1,
            capability_schema_hash: CapabilitySchemaHash::new("sha256:cap-a.speak"),
            sensed_effect: None,
        });
    }
}

fn gold_question_map(
    items: &[crate::datasets::ExternalItem],
) -> BTreeMap<(String, String), String> {
    let mut map = BTreeMap::new();
    for item in items {
        for probe in &item.probes {
            let text = String::from_utf8_lossy(probe.percept.payload.as_bytes()).into_owned();
            let question = text
                .split(&format!("\n\n{LOCOMO_ANSWER_FORMAT_V1}"))
                .next()
                .unwrap_or(&text)
                .split(&format!("\n\n{LONGMEMEVAL_ANSWER_FORMAT_V1}"))
                .next()
                .unwrap_or(&text)
                .trim()
                .to_owned();
            map.insert((item.item_id.clone(), probe.probe_id.clone()), question);
        }
    }
    map
}

pub fn select_pilot_locomo_ids(items: &[crate::datasets::ExternalItem]) -> Vec<String> {
    let mut sorted: Vec<&crate::datasets::ExternalItem> = items.iter().collect();
    sorted.sort_by(|a, b| a.item_id.cmp(&b.item_id));
    sorted
        .first()
        .map(|i| vec![i.item_id.clone()])
        .unwrap_or_default()
}

pub fn select_pilot_longmemeval_ids(
    items: &[crate::datasets::ExternalItem],
    gold: &GoldBook,
    target: usize,
    seed: u64,
) -> Vec<String> {
    let mut by_type: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    let mut ordered: Vec<&crate::datasets::ExternalItem> = items.iter().collect();
    ordered.sort_by(|a, b| a.item_id.cmp(&b.item_id));
    for (idx, item) in ordered.iter().enumerate() {
        let qtype = gold
            .entries
            .iter()
            .find(|e| e.item_id == item.item_id)
            .and_then(|e| e.category.clone())
            .unwrap_or_else(|| "unknown".to_owned());
        by_type.entry(qtype).or_default().push(idx);
    }
    let total: usize = by_type.values().map(Vec::len).sum();
    if total <= target {
        return ordered.iter().map(|i| i.item_id.clone()).collect();
    }
    let mut rng = SplitMix64::new(seed);
    for idxs in by_type.values_mut() {
        rng.shuffle(idxs);
    }
    let mut chosen: Vec<usize> = Vec::new();
    let mut remainders: Vec<(usize, usize, String)> = Vec::new();
    let mut allocated = 0usize;
    for (qtype, idxs) in &by_type {
        let num = idxs.len() * target;
        let take = (num / total).min(idxs.len());
        chosen.extend_from_slice(&idxs[..take]);
        allocated += take;
        remainders.push((num % total, take, qtype.clone()));
    }
    remainders.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.2.cmp(&b.2)));
    for (_rem, taken, qtype) in remainders {
        if allocated >= target {
            break;
        }
        let idxs = &by_type[&qtype];
        if taken < idxs.len() {
            chosen.push(idxs[taken]);
            allocated += 1;
        }
    }
    chosen.sort_unstable();
    chosen
        .into_iter()
        .map(|idx| ordered[idx].item_id.clone())
        .collect()
}

fn completed_items_dir(run_dir: &Path, dataset: &str) -> PathBuf {
    run_dir.join("items").join(dataset)
}

fn load_completed(dataset: &str, run_dir: &Path, item_id: &str) -> Option<CompletedItem> {
    let path = completed_items_dir(run_dir, dataset).join(format!("{item_id}.json"));
    let raw = fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

fn save_completed(dataset: &str, run_dir: &Path, item: &CompletedItem) -> Result<(), LabError> {
    let dir = completed_items_dir(run_dir, dataset);
    fs::create_dir_all(&dir)?;
    fs::write(
        dir.join(format!("{}.json", item.item_id)),
        serde_json::to_vec_pretty(item).map_err(|e| LabError::Serialization(e.to_string()))?,
    )?;
    Ok(())
}

fn load_cached_scores(
    dirs: &[PathBuf],
    dataset: &str,
) -> BTreeMap<(String, String), MemoryItemScore> {
    let mut out = BTreeMap::new();
    for dir in dirs {
        let Ok(entries) = fs::read_dir(completed_items_dir(dir, dataset)) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json")
                || !path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.ends_with(".score.json"))
            {
                continue;
            }
            let Ok(raw) = fs::read_to_string(&path) else {
                continue;
            };
            let Ok(items) = serde_json::from_str::<Vec<MemoryItemScore>>(&raw) else {
                continue;
            };
            for item in items {
                out.entry((item.item_id.clone(), item.probe_id.clone()))
                    .or_insert(item);
            }
        }
    }
    out
}

fn write_item_scores(
    run_dir: &Path,
    dataset: &str,
    items: &[MemoryItemScore],
) -> Result<(), LabError> {
    let dir = completed_items_dir(run_dir, dataset);
    fs::create_dir_all(&dir)?;
    let mut by_item: BTreeMap<String, Vec<MemoryItemScore>> = BTreeMap::new();
    for item in items {
        by_item
            .entry(item.item_id.clone())
            .or_default()
            .push(item.clone());
    }
    for (item_id, scores) in by_item {
        fs::write(
            dir.join(format!("{item_id}.score.json")),
            serde_json::to_vec_pretty(&scores)
                .map_err(|e| LabError::Serialization(e.to_string()))?,
        )?;
    }
    Ok(())
}

fn copy_reused_artifacts(
    source: &Path,
    run_dir: &Path,
    dataset: &str,
    item_id: &str,
) -> Result<(), LabError> {
    let item_rel = format!("items/{dataset}/{item_id}.json");
    let target = run_dir.join(&item_rel);
    if !target.is_file()
        && let Ok(bytes) = fs::read(source.join(&item_rel))
    {
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&target, bytes)?;
    }
    let transcript_rel = format!("transcripts/{dataset}/{}.jsonl", safe_name(item_id));
    let transcript_target = run_dir.join(&transcript_rel);
    if !transcript_target.is_file()
        && let Ok(bytes) = fs::read(source.join(&transcript_rel))
    {
        if let Some(parent) = transcript_target.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&transcript_target, bytes)?;
    }
    Ok(())
}

fn safe_name(text: &str) -> String {
    text.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn write_transcript(
    run_dir: &Path,
    dataset: &str,
    item_id: &str,
    events: &[TranscriptRecord],
) -> Result<String, LabError> {
    let dir = run_dir.join("transcripts").join(dataset);
    fs::create_dir_all(&dir)?;
    let rel = format!("transcripts/{dataset}/{}.jsonl", safe_name(item_id));
    let mut lines = Vec::new();
    for event in events {
        lines.push(
            serde_json::to_string(event).map_err(|e| LabError::Serialization(e.to_string()))?,
        );
    }
    fs::write(run_dir.join(&rel), lines.join("\n") + "\n")?;
    Ok(rel)
}

fn append_scoring_lines(
    run_dir: &Path,
    score: &MemoryScore,
    gold: &GoldBook,
) -> Result<(), LabError> {
    for item in &score.items {
        let path = run_dir.join(&item.transcript_rel_path);
        let Ok(existing) = fs::read_to_string(&path) else {
            continue;
        };
        if existing.contains("\"record_type\":\"score_verdict\"") {
            continue;
        }
        let mut lines = Vec::new();
        if let Some(g) = gold.find(&item.item_id, &item.probe_id) {
            lines.push(
                serde_json::json!({
                    "record_type": "gold_after_scoring",
                    "probe_id": item.probe_id,
                    "gold_answer": g.gold_answer,
                    "accepted_answers": g.accepted_answers,
                    "category": g.category,
                })
                .to_string(),
            );
        }
        lines.push(
            serde_json::json!({
                "record_type": "score_verdict",
                "probe_id": item.probe_id,
                "metric_name": "judge_acc",
                "score": item.judge_correct.map(|c| if c { 1.0 } else { 0.0 }),
                "passed": item.judge_correct,
                "judge_verdict": item.judge_verdict,
                "failed": item.judge_failed,
                "failure_reason": item.failure_reason,
            })
            .to_string(),
        );
        let mut content = existing;
        content.push_str(&lines.join("\n"));
        content.push('\n');
        fs::write(&path, content)?;
    }
    Ok(())
}

fn ledger_breakdown(
    entries: &[LedgerEntry],
    participant: &str,
    dataset: &str,
    phase: Option<&str>,
) -> (UsageBreakdown, Vec<u64>) {
    let mut breakdown = UsageBreakdown::default();
    let mut latencies = Vec::new();
    for entry in entries {
        if entry.participant != participant || entry.dataset != dataset {
            continue;
        }
        if phase.is_some_and(|p| entry.phase != p) {
            continue;
        }
        breakdown.calls += 1;
        breakdown.uncached_input_tokens += entry.uncached_input_tokens;
        match entry.cached_input_tokens {
            Some(cached) => breakdown.cached_input_tokens += cached,
            None => breakdown.cached_unknown_calls += 1,
        }
        breakdown.output_tokens += entry.output_tokens;
        breakdown.reasoning_tokens += entry.reasoning_tokens.unwrap_or(0);
        breakdown.list_cost_usd += entry.list_cost_usd;
        breakdown.allowance_cost_usd += entry.allowance_cost_usd;
        latencies.push(entry.latency_ms);
    }
    (breakdown, latencies)
}

fn purpose_breakdowns(
    entries: &[LedgerEntry],
    dataset: &str,
) -> (BTreeMap<String, UsageBreakdown>, BTreeMap<String, u64>) {
    let mut by_purpose: BTreeMap<String, UsageBreakdown> = BTreeMap::new();
    let mut runaway: BTreeMap<String, u64> = BTreeMap::new();
    for entry in entries {
        if entry.dataset != dataset {
            continue;
        }
        let purpose = if entry.purpose.is_empty() {
            entry.phase.clone()
        } else {
            entry.purpose.clone()
        };
        let slot = by_purpose.entry(purpose.clone()).or_default();
        slot.calls += 1;
        slot.uncached_input_tokens += entry.uncached_input_tokens;
        match entry.cached_input_tokens {
            Some(cached) => slot.cached_input_tokens += cached,
            None => slot.cached_unknown_calls += 1,
        }
        slot.output_tokens += entry.output_tokens;
        slot.reasoning_tokens += entry.reasoning_tokens.unwrap_or(0);
        slot.list_cost_usd += entry.list_cost_usd;
        slot.allowance_cost_usd += entry.allowance_cost_usd;
        if entry.finish_reason.as_deref() == Some("length") {
            *runaway.entry(purpose).or_insert(0) += 1;
        }
    }
    (by_purpose, runaway)
}

fn read_ledger_entries(path: &Path) -> Vec<LedgerEntry> {
    let Ok(raw) = fs::read_to_string(path) else {
        return Vec::new();
    };
    raw.lines()
        .filter_map(|line| serde_json::from_str::<LedgerEntry>(line).ok())
        .collect()
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

fn pct(values: &mut [u64], p: f64) -> u64 {
    if values.is_empty() {
        return 0;
    }
    values.sort_unstable();
    let idx = ((p / 100.0) * (values.len() as f64 - 1.0)).round() as usize;
    values[idx.min(values.len() - 1)]
}

/// Runs phase 0 (pilot or full) and writes the memory report.
pub fn run_memory_suite(opts: &MemorySuiteOptions) -> Result<MemorySuiteOutcome, LabError> {
    if opts.live && opts.profile.is_none() {
        return Err(LabError::Protocol(
            "--live run requires --profile <path> (no provider defaults exist in code)".to_owned(),
        ));
    }
    if opts.render_dry_run && opts.profile.is_none() {
        return Err(LabError::Protocol(
            "--render-dry-run requires --profile <path> (no provider defaults exist in code)"
                .to_owned(),
        ));
    }
    if opts.live && opts.max_cost_usd.is_none() {
        return Err(LabError::Protocol(
            "--live memory run requires --max-cost-usd <cap>".to_owned(),
        ));
    }
    if opts.render_dry_run && opts.embedder.is_none() {
        return Err(LabError::Protocol(
            "render dry run requires a configured EmbeddingGemma 2 embedder (no hash embedder)"
                .to_owned(),
        ));
    }
    let profile_model = opts
        .profile
        .as_ref()
        .map_or_else(|| "test-model".to_owned(), |p| p.model.clone());
    let profile_effort = opts
        .profile
        .as_ref()
        .map_or_else(|| "high".to_owned(), |p| p.reasoning_effort.clone());
    let profile_limits = opts
        .profile
        .as_ref()
        .map(|p| p.output_limits.clone())
        .unwrap_or_default();
    let profile_limit_param = opts.profile.as_ref().map_or_else(
        || crate::reason_seam::model::DEFAULT_OUTPUT_LIMIT_PARAM.to_owned(),
        |p| p.output_limit_param.clone(),
    );
    let manifest_path = resolve_manifest_path(opts.manifest_path.as_deref(), &opts.datasets_dir);
    let manifest =
        load_and_validate_manifest(&manifest_path, &crate::standard_benchmark_registry())?;
    fs::create_dir_all(&opts.run_dir)?;

    let started_at = utc_iso_now();
    let mut notes = Vec::new();
    if opts.phase == MemoryPhase::Pilot {
        notes.push(format!(
            "The full phase 0 run reuses this pilot's finished items and judge results with `--reuse-items-from {}`; only the remaining 9 conversations and 480 questions are executed.",
            opts.run_dir.display()
        ));
    } else if !opts.reuse_from.is_empty() {
        notes.push(format!(
            "Reused finished items and judge results from: {}.",
            opts.reuse_from
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if peak_multiplier_at(SystemTime::now()) > 1.0 {
        let warning =
            "peak-hour window active (2x list price); phase 0 was approved outside peak hours";
        eprintln!("warning: {warning}");
        notes.push(warning.to_owned());
    }

    let live_provider = if let Some(p) = &opts.probe_provider {
        Some(Arc::clone(p))
    } else if let Some(endpoint) = &opts.provider_endpoint {
        let mut cfg = OpenAiModelConfig::new(endpoint.clone(), None);
        cfg.reasoning_effort = Some(
            profile_effort
                .parse()
                .map_err(|e: String| LabError::Protocol(format!("reasoning effort: {e}")))?,
        );
        cfg.output_limits = profile_limits.clone();
        cfg.output_limit_param = profile_limit_param.clone();
        cfg.profile_name = opts.profile.as_ref().map(|p| p.name.clone());
        cfg.user_agent = Some(format!("monoagens-lab/{}", env!("CARGO_PKG_VERSION")));
        cfg.max_retries = 0;
        cfg.default_timeout_ms = 600_000;
        Some(
            Arc::new(OpenAiModelProvider::new(cfg).map_err(LabError::Protocol)?)
                as Arc<dyn ModelProvider>,
        )
    } else {
        None
    };
    let judge: Option<Arc<dyn JudgeRunner>> = if let Some(j) = &opts.judge {
        Some(Arc::clone(j))
    } else if let Some(endpoint) = &opts.judge_endpoint {
        Some(
            Arc::new(HttpJudgeClient::new(endpoint.clone(), &profile_model)?)
                as Arc<dyn JudgeRunner>,
        )
    } else {
        None
    };

    let mut artifacts = Vec::new();
    let mut total_wall_ms = 0u64;

    let datasets: [(&str, Box<dyn Converter>, &str); 2] = [
        ("locomo", Box::new(LocomoConverter), LOCOMO_ANSWER_FORMAT_V1),
        (
            "longmemeval",
            Box::new(LongMemEvalConverter),
            LONGMEMEVAL_ANSWER_FORMAT_V1,
        ),
    ];

    let mut pilot_ids_by_dataset: BTreeMap<String, Vec<String>> = BTreeMap::new();

    if opts.live || opts.render_dry_run {
        let dry_prior = monoagens_store::load_prior_or_default(None)?.prior;
        let mut dry_samples: Vec<(String, crate::datasets::ExternalItem)> = Vec::new();
        for (dataset_id, converter, instruction) in &datasets {
            if opts
                .only_datasets
                .as_ref()
                .is_some_and(|ids| !ids.iter().any(|id| id == dataset_id))
            {
                continue;
            }
            let mut converted = load_dataset(
                converter.as_ref(),
                &opts.datasets_dir,
                &manifest,
                dataset_id,
            )?;
            apply_answer_format(&mut converted.items, instruction);
            ensure_speak_capabilities(&mut converted.items);
            if converted.items.is_empty() {
                continue;
            }
            converted.items.sort_by(|a, b| a.item_id.cmp(&b.item_id));
            let mut sample = converted.items.remove(0);
            sample.history.truncate(60);
            sample.probes.truncate(1);
            dry_samples.push(((*dataset_id).to_owned(), sample));
        }
        let dry_refs: Vec<(String, &crate::datasets::ExternalItem)> = dry_samples
            .iter()
            .map(|(ds, it)| (ds.clone(), it))
            .collect();
        let dry_paths = run_render_dry_run_gate_with_embedder(
            &dry_refs,
            &opts.run_dir,
            &dry_prior,
            opts.seed,
            opts.embedder.as_deref(),
        )?;
        notes.push(format!(
            "Render dry run passed ({} files) with profile limits classify={}, reason={}, consolidate={}.",
            dry_paths.len(),
            profile_limits.classify,
            profile_limits.reason,
            profile_limits.consolidate
        ));
        if opts.render_dry_run && !opts.live {
            return Ok(MemorySuiteOutcome {
                run_dir: opts.run_dir.clone(),
                datasets: Vec::new(),
                pilot: None,
                stopped_reason: None,
                report_md: None,
                report_html: None,
            });
        }
    }
    let Some(provider) = live_provider else {
        return Err(LabError::Protocol(
            "memory suite requires a live provider (proxy endpoint) or a test provider".to_owned(),
        ));
    };
    let judge_config = JudgeExecutionConfig {
        live: opts.live,
        judge_model: Some(&profile_model),
        judge_prompt_version: Some(LONGMEMEVAL_PROMPT_VERSION),
        provider: None,
        datasets_dir: Some(&opts.datasets_dir),
        manifest: Some(&manifest),
        recorder: None,
    };

    let mut stopped_reason: Option<String> = None;
    // The Lab proxy ledger is the cost source of truth; the framework guard's internal price
    // table is only a coarse safety stop, so it is not given the run cap here.
    let mut guard = CostGuard::default();

    for (dataset_id, converter, instruction) in &datasets {
        if opts
            .only_datasets
            .as_ref()
            .is_some_and(|ids| !ids.iter().any(|id| id == dataset_id))
        {
            continue;
        }
        let mut converted = load_dataset(
            converter.as_ref(),
            &opts.datasets_dir,
            &manifest,
            dataset_id,
        )?;
        apply_answer_format(&mut converted.items, instruction);
        ensure_speak_capabilities(&mut converted.items);
        converted.items.sort_by(|a, b| a.item_id.cmp(&b.item_id));
        let pilot_ids = if opts.phase == MemoryPhase::Pilot {
            let ids = if *dataset_id == "locomo" {
                select_pilot_locomo_ids(&converted.items)
            } else {
                select_pilot_longmemeval_ids(&converted.items, &converted.gold_book, 20, opts.seed)
            };
            pilot_ids_by_dataset.insert((*dataset_id).to_owned(), ids.clone());
            ids
        } else {
            Vec::new()
        };
        let mut selected: Vec<&crate::datasets::ExternalItem> = converted
            .items
            .iter()
            .filter(|item| {
                opts.phase == MemoryPhase::Full
                    || pilot_ids.is_empty()
                    || pilot_ids.contains(&item.item_id)
            })
            .collect();
        if let Some(limit) = opts.max_items_per_dataset {
            selected.truncate(limit);
        }
        let questions = gold_question_map(&converted.items);
        let mut completed: Vec<CompletedItem> = Vec::new();
        let mut executed_any = false;
        let ds_start = Instant::now();

        for item in &selected {
            if guard.budget_exhausted() {
                stopped_reason = Some("cost cap reached".to_owned());
                break;
            }
            let mut reused = None;
            for dir in std::iter::once(&opts.run_dir).chain(opts.reuse_from.iter()) {
                if let Some(done) = load_completed(dataset_id, dir, &item.item_id)
                    && done.answer_format_version == ANSWER_FORMAT_PROMPT_VERSION
                    && done.harness_version == MEMORY_HARNESS_VERSION
                {
                    reused = Some((dir.clone(), done));
                    break;
                }
            }
            if let Some((source_dir, done)) = reused {
                if source_dir != opts.run_dir {
                    copy_reused_artifacts(&source_dir, &opts.run_dir, dataset_id, &item.item_id)?;
                }
                completed.push(done);
                continue;
            }
            let attribution =
                CallAttribution::new("monoagens", *dataset_id, item.item_id.clone(), "answer");
            executed_any = true;
            let exec = execute_external_item_with_embedder(
                item,
                &opts.prior,
                &Switches::all_enabled(),
                opts.seed,
                provider.as_ref(),
                &profile_model,
                opts.live,
                &mut guard,
                opts.embedder.as_deref(),
                Some(&attribution),
                Some(MEMORY_MODEL_TIMEOUT_MS),
                Some(MEMORY_MAX_OUTPUT_TOKENS),
            )?;
            let completed_item = CompletedItem {
                dataset: (*dataset_id).to_owned(),
                item_id: item.item_id.clone(),
                cluster_id: item.cluster_id.clone(),
                answer_format_version: ANSWER_FORMAT_PROMPT_VERSION.to_owned(),
                harness_version: MEMORY_HARNESS_VERSION.to_owned(),
                predictions: exec.predictions.clone(),
                history_reasoning_mode: exec.history_reasoning_mode.clone(),
                total_history_gate_wakes: exec.total_history_gate_wakes,
                decisions_go: exec.decisions_go,
                decisions_note: exec.decisions_note,
                decisions_silent: exec.decisions_silent,
                decisions_door_blocked: exec.decisions_door_blocked,
                actions_dispatched: exec.actions_dispatched,
                percepts_count: exec.percepts_count,
                model_latencies_ms: exec.model_latencies_ms.clone(),
                failed_items: exec.failed_items.clone(),
            };
            write_transcript(
                &opts.run_dir,
                dataset_id,
                &item.item_id,
                &exec.transcript_events,
            )?;
            save_completed(dataset_id, &opts.run_dir, &completed_item)?;
            if let Some(failure) = exec.failed_items.iter().find(|f| {
                f.reason.contains("rate_limit")
                    || f.reason.contains("authentication")
                    || f.reason.contains("429")
            }) {
                stopped_reason = Some(format!("provider limit reached: {}", failure.reason));
                completed.push(completed_item);
                break;
            }
            completed.push(completed_item);
            if let Some(cap) = opts.max_cost_usd {
                let spent = read_ledger_totals(&opts.run_dir.join("ledger.jsonl"), None)
                    .map(|t| {
                        if t.allowance_cost_usd > 0.0 {
                            t.allowance_cost_usd
                        } else {
                            t.list_cost_usd
                        }
                    })
                    .unwrap_or(0.0);
                if spent >= cap {
                    stopped_reason = Some(format!("cost cap reached (${spent:.4} >= ${cap:.4})"));
                    break;
                }
            }
        }

        let predictions: Vec<crate::datasets::ExternalPrediction> = completed
            .iter()
            .flat_map(|c| c.predictions.clone())
            .collect();
        let selected_questions: BTreeMap<(String, String), String> = questions
            .iter()
            .filter(|((item_id, _), _)| selected.iter().any(|i| &i.item_id == item_id))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        let mut cached_dirs = vec![opts.run_dir.clone()];
        cached_dirs.extend(opts.reuse_from.iter().cloned());
        let cached_items = load_cached_scores(&cached_dirs, dataset_id);
        // A resumed run with no new items reuses the recorded scores and does not call the judge
        // again (the previous pilot doubled judge calls on resume).
        let score_ctx = ScoreContext {
            gold: &converted.gold_book,
            questions: &selected_questions,
            cached_items: &cached_items,
            judge: judge.as_deref(),
            judge_config: &judge_config,
            project_root: &opts.run_dir,
            seed: opts.seed,
        };
        let score = if !executed_any {
            fs::read_to_string(
                opts.run_dir
                    .join(format!("items/{dataset_id}__scores.json")),
            )
            .ok()
            .and_then(|raw| serde_json::from_str::<MemoryScore>(&raw).ok())
            .unwrap_or(if *dataset_id == "locomo" {
                score_locomo(&predictions, &score_ctx)?
            } else {
                score_longmemeval(&predictions, &score_ctx)?
            })
        } else if *dataset_id == "locomo" {
            score_locomo(&predictions, &score_ctx)?
        } else {
            score_longmemeval(&predictions, &score_ctx)?
        };
        if executed_any {
            append_scoring_lines(&opts.run_dir, &score, &converted.gold_book)?;
        }
        write_item_scores(&opts.run_dir, dataset_id, &score.items)?;

        // Read the ledger only after every participant and judge call for this dataset has landed.
        let dataset_ledger_path = opts.run_dir.join("ledger.jsonl");
        let all_ledger_entries = read_ledger_entries(&dataset_ledger_path);
        let (ingest, _ingest_latencies) =
            ledger_breakdown(&all_ledger_entries, "monoagens", dataset_id, Some("ingest"));
        let (answer, mut answer_latencies) =
            ledger_breakdown(&all_ledger_entries, "monoagens", dataset_id, Some("answer"));
        let (judge_usage, _) =
            ledger_breakdown(&all_ledger_entries, "judge", dataset_id, Some("judge"));
        let mut all_latencies = answer_latencies.clone();
        for item in &completed {
            for latency in &item.model_latencies_ms {
                all_latencies.push(*latency);
            }
        }
        answer_latencies.sort_unstable();
        let (by_purpose, runaway_by_purpose) = purpose_breakdowns(&all_ledger_entries, dataset_id);
        let usage = MemoryUsageTotals {
            ingest,
            answer,
            judge: judge_usage,
            latencies_ms: all_latencies.clone(),
            by_purpose,
            runaway_by_purpose,
            max_output_tokens: Some(MEMORY_MAX_OUTPUT_TOKENS),
        };

        let intervals: Vec<MetricScoreInterval> = score.to_intervals();
        let mut category_metrics = score.category_metrics.clone();
        category_metrics.insert(
            "silence".to_owned(),
            BTreeMap::from([("silence_rate".to_owned(), score.silence_rate)]),
        );
        let loaded_prior = monoagens_store::LoadedPrior::from_prior(opts.prior.clone())?;
        let pinned_revision = manifest
            .get(dataset_id)
            .map(|e| {
                e.sources
                    .iter()
                    .map(|s| s.revision.clone())
                    .collect::<Vec<_>>()
                    .join("+")
            })
            .unwrap_or_default();
        let manifest_raw = fs::read(&manifest_path).unwrap_or_default();
        let monoagens_mode = derive_monoagens_mode(
            opts.live,
            provider.provider_name() != "scripted",
            &Switches::all_enabled(),
            Some("live"),
            Some(PROMPT_SOURCE_CORE_RENDER),
        );
        let mut rec = RunRecord {
            schema_version: "1.0".to_owned(),
            run_id: format!("memory-p0-{dataset_id}-seed-{}", opts.seed),
            core_version: env!("CARGO_PKG_VERSION").to_owned(),
            prior_content_version: loaded_prior.prior.content_version.clone(),
            prior_format_version: loaded_prior.prior.version,
            prior_digest: loaded_prior.digest.clone(),
            open_offset: 0,
            model_id: profile_model.clone(),
            embedder_id: opts
                .embedder
                .as_ref()
                .and_then(|e| e.model_ver())
                .unwrap_or("none")
                .to_owned(),
            door_rules: guard.rule_names(),
            switches: Switches::all_enabled(),
            learning_switches: RunRecord::learning_switches_from(&Switches::all_enabled()),
            seed: opts.seed,
            scenario_id: (*dataset_id).to_owned(),
            scenario_set: ScenarioSet::External,
            sealed: false,
            checks: Vec::new(),
            passed: true,
            metrics: crate::metrics::RunMetricsSummary {
                total_events: completed.len(),
                percepts: completed.iter().map(|c| c.percepts_count).sum(),
                decisions_go: completed.iter().map(|c| c.decisions_go).sum(),
                decisions_note: completed.iter().map(|c| c.decisions_note).sum(),
                decisions_silent: completed.iter().map(|c| c.decisions_silent).sum(),
                decisions_door_blocked: completed.iter().map(|c| c.decisions_door_blocked).sum(),
                actions_dispatched: completed.iter().map(|c| c.actions_dispatched).sum(),
                receipts_recorded: 0,
                wakes_per_event: 0.0,
                simulated_span_ms: 0,
                wall_elapsed_ms: ds_start.elapsed().as_millis() as u64,
                brain_map_metrics: None,
                detailed: None,
            },
            cost_by_purpose: vec![
                PurposeCost {
                    purpose: monoagens_core::ModelPurpose::Reason,
                    calls: usage.ingest.calls,
                    tokens_in: usage.ingest.uncached_input_tokens
                        + usage.ingest.cached_input_tokens,
                    tokens_out: usage.ingest.output_tokens,
                    cost_usd: usage.ingest.list_cost_usd,
                },
                PurposeCost {
                    purpose: monoagens_core::ModelPurpose::Classify,
                    calls: usage.answer.calls,
                    tokens_in: usage.answer.uncached_input_tokens
                        + usage.answer.cached_input_tokens,
                    tokens_out: usage.answer.output_tokens,
                    cost_usd: usage.answer.list_cost_usd,
                },
            ],
            history_reasoning_mode: Some("live".to_owned()),
            monoagens_mode: Some(monoagens_mode.to_owned()),
            dataset_provenance: Some(DatasetRunProvenance {
                dataset_id: (*dataset_id).to_owned(),
                pinned_revision,
                manifest_digest: format!("sha256:{}", sha256_hex(&manifest_raw)),
                converter_version: converted.converter_version.clone(),
                scorer_version: "memory-p0-1.0.0".to_owned(),
                sampling_rule: if opts.phase == MemoryPhase::Pilot {
                    "pilot".to_owned()
                } else {
                    "full".to_owned()
                },
                chosen_item_ids: completed.iter().map(|c| c.item_id.clone()).collect(),
                metric_scores: intervals,
                judge_model: Some(profile_model.clone()),
                judge_prompt_version: Some(if *dataset_id == "locomo" {
                    LOCOMO_PROMPT_VERSION.to_owned()
                } else {
                    LONGMEMEVAL_PROMPT_VERSION.to_owned()
                }),
                judge_calls: usage.judge.calls,
                judge_tokens_in: usage.judge.uncached_input_tokens
                    + usage.judge.cached_input_tokens,
                judge_tokens_out: usage.judge.output_tokens,
                history_gate_wakes: completed.iter().map(|c| c.total_history_gate_wakes).sum(),
                partial_run: opts.phase == MemoryPhase::Pilot,
                reasoning_effort: Some(profile_effort.clone()),
                judge_reasoning_effort: Some(profile_effort.clone()),
                extra_header_names: Vec::new(),
                provider_endpoint_host: opts
                    .provider_endpoint
                    .as_deref()
                    .and_then(|e| e.split("//").nth(1))
                    .and_then(|rest| rest.split('/').next())
                    .map(str::to_owned),
                embedder_backend: opts.embedder.as_ref().map(|e| e.backend_name().to_owned()),
                embedder_revision: None,
                prompt_source: Some(PROMPT_SOURCE_CORE_RENDER.to_owned()),
                development_exposure: Some("none".to_owned()),
                memory_usage: Some(usage),
                category_metrics,
                failed_items: completed
                    .iter()
                    .flat_map(|c| c.failed_items.clone())
                    .collect(),
                model_latencies_ms: all_latencies,
                started_at_utc: Some(started_at.clone()),
                finished_at_utc: Some(utc_iso_now()),
            }),
        };
        rec.metrics.wakes_per_event = if rec.metrics.percepts > 0 {
            rec.metrics.decisions_go as f64 / rec.metrics.percepts as f64
        } else {
            0.0
        };
        rec.passed = stopped_reason.is_none();

        let records_dir = opts.run_dir.join("records");
        fs::create_dir_all(&records_dir)?;
        let record_rel = format!("records/{}__seed-{}.json", dataset_id, opts.seed);
        fs::write(
            opts.run_dir.join(&record_rel),
            serde_json::to_vec_pretty(&rec).map_err(|e| LabError::Serialization(e.to_string()))?,
        )?;
        let items_rel = format!("items/{}__scores.json", dataset_id);
        fs::write(
            opts.run_dir.join(&items_rel),
            serde_json::to_vec_pretty(&score.items)
                .map_err(|e| LabError::Serialization(e.to_string()))?,
        )?;
        total_wall_ms += ds_start.elapsed().as_millis() as u64;
        let n_items = predictions.len();
        artifacts.push(DatasetArtifact {
            dataset_id: (*dataset_id).to_owned(),
            record_rel_path: record_rel,
            items_rel_path: items_rel,
            n_items,
        });
        if stopped_reason.is_some() {
            break;
        }
    }

    let status = match &stopped_reason {
        Some(reason) => format!("stopped: {reason}"),
        None => "complete".to_owned(),
    };
    let judge_prompt_versions = BTreeMap::from([
        ("locomo".to_owned(), LOCOMO_PROMPT_VERSION.to_owned()),
        (
            "longmemeval".to_owned(),
            LONGMEMEVAL_PROMPT_VERSION.to_owned(),
        ),
        (
            "answer_format".to_owned(),
            ANSWER_FORMAT_PROMPT_VERSION.to_owned(),
        ),
    ]);
    let pilot = if opts.phase == MemoryPhase::Pilot {
        Some(build_pilot_summary(
            &opts.run_dir,
            &artifacts,
            &pilot_ids_by_dataset,
            total_wall_ms,
        )?)
    } else {
        None
    };
    let run_file = MemoryRunFile {
        model: profile_model.clone(),
        reasoning_effort: profile_effort,
        judge_model: profile_model,
        judge_prompt_versions,
        answer_format_version: ANSWER_FORMAT_PROMPT_VERSION.to_owned(),
        seed: opts.seed,
        phase: if opts.phase == MemoryPhase::Pilot {
            "pilot".to_owned()
        } else {
            "full".to_owned()
        },
        live: opts.live,
        status,
        development_exposure: "none".to_owned(),
        started_at_utc: started_at,
        finished_at_utc: utc_iso_now(),
        datasets: artifacts.clone(),
        pilot: pilot.clone(),
        notes,
        output_limits: Some(profile_limits),
        runaways_by_purpose: Some(count_runaways_by_purpose(
            &opts.run_dir.join("ledger.jsonl"),
        )),
    };
    fs::write(
        opts.run_dir.join("memory_run.json"),
        serde_json::to_vec_pretty(&run_file).map_err(|e| LabError::Serialization(e.to_string()))?,
    )?;

    let (report_md, report_html) = if opts.write_report {
        let generated = report::generate_memory_report(&opts.run_dir)?;
        (Some(generated.markdown_path), Some(generated.html_path))
    } else {
        (None, None)
    };

    Ok(MemorySuiteOutcome {
        run_dir: opts.run_dir.clone(),
        datasets: artifacts,
        pilot,
        stopped_reason,
        report_md,
        report_html,
    })
}

fn build_pilot_summary(
    run_dir: &Path,
    artifacts: &[DatasetArtifact],
    pilot_ids: &BTreeMap<String, Vec<String>>,
    wall_ms: u64,
) -> Result<PilotSummary, LabError> {
    let entries = read_ledger_entries(&run_dir.join("ledger.jsonl"));
    let mut calls = 0u64;
    let mut ingest_calls = 0u64;
    let mut answer_calls = 0u64;
    let mut judge_calls = 0u64;
    let mut errors = 0u64;
    let mut cached = 0u64;
    let mut uncached = 0u64;
    let mut cached_unknown = 0u64;
    let mut tokens_in_per_call = Vec::new();
    let mut tokens_out_per_call = Vec::new();
    let mut latencies = Vec::new();
    let mut list_cost = 0.0;
    let mut allowance_cost = 0.0;
    for entry in &entries {
        calls += 1;
        match entry.phase.as_str() {
            "ingest" => ingest_calls += 1,
            "answer" => answer_calls += 1,
            "judge" => judge_calls += 1,
            _ => {}
        }
        if entry.status != "ok" {
            errors += 1;
        }
        uncached += entry.uncached_input_tokens;
        match entry.cached_input_tokens {
            Some(c) => cached += c,
            None => cached_unknown += 1,
        }
        tokens_in_per_call
            .push(entry.uncached_input_tokens + entry.cached_input_tokens.unwrap_or(0));
        tokens_out_per_call.push(entry.output_tokens);
        latencies.push(entry.latency_ms);
        list_cost += entry.list_cost_usd;
        allowance_cost += entry.allowance_cost_usd;
    }
    let mean = |vals: &[u64]| {
        if vals.is_empty() {
            0.0
        } else {
            vals.iter().sum::<u64>() as f64 / vals.len() as f64
        }
    };
    let mut t_in = tokens_in_per_call.clone();
    let mut t_out = tokens_out_per_call.clone();
    let cached_share = if uncached + cached > 0 {
        cached as f64 / (uncached + cached) as f64
    } else {
        0.0
    };
    let total_questions: usize = artifacts.iter().map(|a| a.n_items).sum();
    let mut silent_total = 0usize;
    let mut item_total = 0usize;
    for artifact in artifacts {
        let Ok(raw) = fs::read_to_string(run_dir.join(&artifact.items_rel_path)) else {
            continue;
        };
        let Ok(items) = serde_json::from_str::<Vec<MemoryItemScore>>(&raw) else {
            continue;
        };
        silent_total += items.iter().filter(|i| i.silent).count();
        item_total += items.len();
    }
    let silence_rate = if item_total == 0 {
        0.0
    } else {
        silent_total as f64 / item_total as f64
    };
    let rest_questions = (10_usize + 500).saturating_sub(total_questions);
    let all_input: f64 = tokens_in_per_call.iter().map(|v| *v as f64).sum();
    let all_output: f64 = tokens_out_per_call.iter().map(|v| *v as f64).sum();
    let per_question_tokens = if total_questions > 0 {
        (all_input + all_output) / total_questions as f64
    } else {
        0.0
    };
    let projected_rest = per_question_tokens
        * rest_questions as f64
        * (list_cost / (all_input + all_output).max(1.0));
    let mean_wall_per_question = if total_questions > 0 {
        wall_ms as f64 / 1000.0 / total_questions as f64
    } else {
        0.0
    };
    Ok(PilotSummary {
        locomo_item_ids: pilot_ids.get("locomo").cloned().unwrap_or_default(),
        longmemeval_item_ids: pilot_ids.get("longmemeval").cloned().unwrap_or_default(),
        calls,
        ingest_calls,
        answer_calls,
        judge_calls,
        mean_tokens_in_per_call: mean(&t_in),
        p95_tokens_in_per_call: pct(&mut t_in, 95.0) as f64,
        mean_tokens_out_per_call: mean(&t_out),
        p95_tokens_out_per_call: pct(&mut t_out, 95.0) as f64,
        cached_share,
        cached_unknown_calls: cached_unknown,
        silence_rate,
        errors,
        retries: entries.iter().filter(|e| e.attempts > 1).count() as u64,
        rate_limit_hits: entries
            .iter()
            .filter(|e| e.http_status == Some(429))
            .count() as u64,
        latency_p50_ms: pct(&mut latencies, 50.0),
        latency_p95_ms: pct(&mut latencies, 95.0),
        list_cost_usd: list_cost,
        allowance_cost_usd: allowance_cost,
        projected_rest_list_cost_usd: projected_rest,
        projected_rest_wall_clock_s: mean_wall_per_question * rest_questions as f64,
    })
}

fn count_runaways_by_purpose(ledger_path: &Path) -> BTreeMap<String, u64> {
    let mut counts = BTreeMap::from([
        ("reason".to_owned(), 0_u64),
        ("classify".to_owned(), 0_u64),
        ("consolidate".to_owned(), 0_u64),
    ]);
    let Ok(raw) = fs::read_to_string(ledger_path) else {
        return counts;
    };
    for line in raw.lines() {
        let Ok(entry) = serde_json::from_str::<crate::memory::proxy::LedgerEntry>(line) else {
            continue;
        };
        if entry.finish_reason.as_deref() == Some("length") {
            *counts.entry(entry.purpose).or_insert(0) += 1;
        }
    }
    counts
}

/// Validates a single rendered dry-run request, blocking live runs on empty triggers or missing capabilities.
pub fn validate_rendered_dry_run_request(
    purpose: &str,
    item_id: &str,
    turn_or_probe_id: &str,
    system_prompt: &str,
    user_prompt: &str,
) -> Result<(), LabError> {
    if user_prompt.contains("trigger: none") {
        return Err(LabError::Protocol(format!(
            "render dry run blocked live run: empty trigger ('trigger: none') for purpose '{purpose}', item '{item_id}', turn '{turn_or_probe_id}'"
        )));
    }
    if user_prompt.contains("\"detail_kind\"") || system_prompt.contains("\"detail_kind\"") {
        return Err(LabError::Protocol(format!(
            "render dry run blocked live run: Core bookkeeping records (detail_kind) leaked into the prompt for purpose '{purpose}', item '{item_id}', turn '{turn_or_probe_id}'"
        )));
    }
    let task_block = user_prompt
        .split_once("[TASK]\n")
        .and_then(|(_, rest)| rest.split_once("[/TASK]"))
        .map(|(block, _)| block);
    match (purpose, task_block) {
        ("reason", Some(_)) => {
            return Err(LabError::Protocol(format!(
                "render dry run blocked live run: a reasoning request carries a [TASK] block, item '{item_id}', turn '{turn_or_probe_id}'"
            )));
        }
        ("classify" | "consolidate", None) => {
            return Err(LabError::Protocol(format!(
                "render dry run blocked live run: {purpose} request has no [TASK] block (the model would not know what to judge), item '{item_id}', turn '{turn_or_probe_id}'"
            )));
        }
        ("classify", Some(block)) => {
            let has_material = ["condition:\n> ", "goal:\n> "]
                .iter()
                .any(|m| block.contains(m) && !block.contains(&format!("{m}\n")))
                || block.contains("earlier:\n[")
                || block.contains("sources:\n[");
            if !has_material || !block.contains("label:") {
                return Err(LabError::Protocol(format!(
                    "render dry run blocked live run: classify [TASK] lacks the material it judges or its labels, item '{item_id}', turn '{turn_or_probe_id}'"
                )));
            }
        }
        ("consolidate", Some(block)) if !block.contains("events:\n[") => {
            return Err(LabError::Protocol(format!(
                "render dry run blocked live run: consolidate [TASK] lists no episode events, item '{item_id}', turn '{turn_or_probe_id}'"
            )));
        }
        _ => {}
    }
    let no_static =
        system_prompt.contains("[STATIC_CORE_CAPABILITIES]\n- none\n[/STATIC_CORE_CAPABILITIES]");
    let no_dynamic =
        user_prompt.contains("[DYNAMIC_CAPABILITIES]\n- none\n[/DYNAMIC_CAPABILITIES]");
    if no_static && no_dynamic {
        return Err(LabError::Protocol(format!(
            "render dry run blocked live run: no capabilities rendered for purpose '{purpose}', item '{item_id}', turn '{turn_or_probe_id}'"
        )));
    }
    Ok(())
}

/// Renders a no-network scripted dry run over sample items before any `--live` run (`24.6`).
pub fn run_render_dry_run_gate(
    samples: &[(String, &crate::datasets::ExternalItem)],
    run_dir: &Path,
    prior: &monoagens_core::Prior,
    seed: u64,
) -> Result<Vec<PathBuf>, LabError> {
    run_render_dry_run_gate_with_embedder(samples, run_dir, prior, seed, None)
}

struct DryRunScriptedProvider {
    condition_text: String,
    opened_expectation: std::sync::atomic::AtomicBool,
}

impl ModelProvider for DryRunScriptedProvider {
    fn provider_name(&self) -> &str {
        "dry-run-scripted"
    }

    fn invoke(
        &self,
        request: &crate::reason_seam::model::ModelRequest,
    ) -> Result<
        crate::reason_seam::model::ModelCallSuccess,
        crate::reason_seam::model::ModelCallFailure,
    > {
        use std::sync::atomic::Ordering;

        let raw = if request.purpose == monoagens_core::ModelPurpose::Classify {
            serde_json::json!({
                "protocol_version": {"major": 1, "minor": 0},
                "label": "note",
                "rationale": "dry-run scripted classify",
                "score": 0.0
            })
            .to_string()
        } else if request.purpose == monoagens_core::ModelPurpose::Consolidate {
            // A gist over the episode events listed in [TASK], so gist verification runs too.
            let user = request
                .messages
                .iter()
                .filter(|m| m.role == crate::reason_seam::model::MessageRole::User)
                .flat_map(|m| m.parts.iter())
                .filter_map(|part| match part {
                    crate::reason_seam::model::MessagePart::Text(t) => Some(t.as_str()),
                    crate::reason_seam::model::MessagePart::Attachment(_) => None,
                })
                .collect::<Vec<_>>()
                .join("\n");
            let task = user
                .split_once("[TASK]\n")
                .and_then(|(_, rest)| rest.split_once("[/TASK]"))
                .map_or("", |(block, _)| block);
            let about_ids: Vec<u64> = task
                .lines()
                .filter_map(|l| l.strip_prefix("id="))
                .filter_map(|l| l.split_whitespace().next())
                .filter_map(|id| id.parse().ok())
                .collect();
            serde_json::json!({
                "protocol_version": {"major": 1, "minor": 0},
                "acts": [{
                    "act": "consolidate",
                    "output": {
                        "consolidate_kind": "gist",
                        "summary": "dry-run scripted gist of the episode",
                        "about_ids": about_ids,
                        "places": [],
                        "people": [],
                        "domain": null
                    }
                }]
            })
            .to_string()
        } else if request.purpose == monoagens_core::ModelPurpose::Reason
            && !self.opened_expectation.swap(true, Ordering::SeqCst)
        {
            serde_json::json!({
                "protocol_version": {"major": 1, "minor": 0},
                "acts": [{
                    "act": "expect",
                    "expect_kind": "goal",
                    "about_ids": [request.request_id.as_u64()],
                    "places": [],
                    "people": [],
                    "condition": self.condition_text,
                    "due_ms": null,
                    "expiry_ms": 9_999_999_999_999_i64,
                    "recurrence": null,
                    "promise_meta": null,
                    "domain": null
                }]
            })
            .to_string()
        } else {
            serde_json::json!({
                "protocol_version": {"major": 1, "minor": 0},
                "acts": [{
                    "act": "think",
                    "thought": "dry-run scripted check",
                    "motivation": 2,
                    "confidence_0_20": 12,
                    "used_ids": []
                }]
            })
            .to_string()
        };

        let output = if request.purpose == monoagens_core::ModelPurpose::Classify {
            let verdict = crate::reason_seam::parse_classification_verdict(&raw).map_err(|fw| {
                crate::reason_seam::model::ModelCallFailure {
                    class: crate::reason_seam::model::ProviderFailureClass::InvalidOutput,
                    record: Box::new(crate::reason_seam::model::ModelCallRecord {
                        request_id: request.request_id,
                        purpose: request.purpose,
                        provider: self.provider_name().to_owned(),
                        model: request.model.clone(),
                        usage: None,
                        latency_ms: 1,
                        outcome: crate::reason_seam::model::CallOutcome::Failed(
                            crate::reason_seam::model::ProviderFailureClass::InvalidOutput,
                        ),
                        strict_schema: true,
                    }),
                    status_code: Some(200),
                    failed_wake: Some(Box::new(fw.clone())),
                    detail: fw.to_string(),
                }
            })?;
            crate::reason_seam::model::ModelOutput::Verdict(verdict)
        } else {
            let acts = crate::reason_seam::parse_thought_output(&raw).map_err(|fw| {
                crate::reason_seam::model::ModelCallFailure {
                    class: crate::reason_seam::model::ProviderFailureClass::InvalidOutput,
                    record: Box::new(crate::reason_seam::model::ModelCallRecord {
                        request_id: request.request_id,
                        purpose: request.purpose,
                        provider: self.provider_name().to_owned(),
                        model: request.model.clone(),
                        usage: None,
                        latency_ms: 1,
                        outcome: crate::reason_seam::model::CallOutcome::Failed(
                            crate::reason_seam::model::ProviderFailureClass::InvalidOutput,
                        ),
                        strict_schema: true,
                    }),
                    status_code: Some(200),
                    failed_wake: Some(Box::new(fw.clone())),
                    detail: fw.to_string(),
                }
            })?;
            crate::reason_seam::model::ModelOutput::ThoughtActs(acts)
        };

        Ok(crate::reason_seam::model::ModelCallSuccess {
            record: crate::reason_seam::model::ModelCallRecord {
                request_id: request.request_id,
                purpose: request.purpose,
                provider: self.provider_name().to_owned(),
                model: request.model.clone(),
                usage: Some(crate::reason_seam::model::TokenUsage::default()),
                latency_ms: 1,
                outcome: crate::reason_seam::model::CallOutcome::Success,
                strict_schema: true,
            },
            output,
            raw_text: raw,
        })
    }
}

struct DryRunExample {
    dataset_id: String,
    turn_or_probe_id: String,
    system_prompt: String,
    user_prompt: String,
}

/// Renders a no-network scripted dry run over sample items with an optional semantic embedder,
/// writing `dry_run/<dataset>__dry_run.jsonl` and `dry_run/summary.md`.
pub fn run_render_dry_run_gate_with_embedder(
    samples: &[(String, &crate::datasets::ExternalItem)],
    run_dir: &Path,
    prior: &monoagens_core::Prior,
    seed: u64,
    embedder: Option<&dyn Embedder>,
) -> Result<Vec<PathBuf>, LabError> {
    use crate::datasets::framework::{TranscriptRecord, execute_external_item_with_embedder};
    use std::sync::atomic::AtomicBool;

    let dry_dir = run_dir.join("dry_run");
    fs::create_dir_all(&dry_dir)?;
    let mut written = Vec::new();
    let switches = monoagens_core::Switches::all_enabled();
    let mut counts_by_dataset: BTreeMap<String, BTreeMap<String, usize>> = BTreeMap::new();
    let mut examples_by_purpose: BTreeMap<String, DryRunExample> = BTreeMap::new();

    for (dataset_id, item) in samples {
        let condition_text = item
            .history
            .iter()
            .map(|p| String::from_utf8_lossy(p.payload.as_bytes()).into_owned())
            .find(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "user follows up on the conversation".to_owned());
        let provider = DryRunScriptedProvider {
            condition_text,
            opened_expectation: AtomicBool::new(false),
        };
        let mut guard = crate::CostGuard::default();
        let exec = execute_external_item_with_embedder(
            item,
            prior,
            &switches,
            seed,
            &provider,
            "dry-run-scripted",
            true,
            &mut guard,
            embedder,
            None,
            Some(30_000),
            Some(1_024),
        )?;
        let out_path = dry_dir.join(format!("{dataset_id}__dry_run.jsonl"));
        let mut lines = String::new();
        let ds_counts = counts_by_dataset.entry(dataset_id.clone()).or_default();
        for rec in &exec.transcript_events {
            if let TranscriptRecord::ModelCall {
                purpose,
                turn_or_probe_id,
                system_prompt,
                user_prompt,
                ..
            } = rec
            {
                validate_rendered_dry_run_request(
                    purpose,
                    &item.item_id,
                    turn_or_probe_id,
                    system_prompt,
                    user_prompt,
                )?;
                *ds_counts.entry(purpose.clone()).or_insert(0) += 1;
                examples_by_purpose
                    .entry(purpose.clone())
                    .or_insert_with(|| DryRunExample {
                        dataset_id: dataset_id.clone(),
                        turn_or_probe_id: turn_or_probe_id.clone(),
                        system_prompt: system_prompt.clone(),
                        user_prompt: user_prompt.clone(),
                    });
                let line = serde_json::to_string(rec)
                    .map_err(|e| LabError::Serialization(e.to_string()))?;
                lines.push_str(&line);
                lines.push('\n');
            }
        }
        fs::write(&out_path, lines)?;
        written.push(out_path);

        if embedder.is_some() && ds_counts.get("classify").copied().unwrap_or(0) == 0 {
            return Err(LabError::Protocol(format!(
                "render dry run failed for dataset '{dataset_id}' (item '{}'): no 'classify' request was rendered after {} history turns and {} probes (reason: free-text expectation did not trigger Classify; check embedder embeddings or wake_gate_lo)",
                item.item_id,
                item.history.len(),
                item.probes.len()
            )));
        }
    }

    let mut summary =
        String::from("# Render Dry-Run Summary\n\n## Counts by Purpose and Dataset\n\n");
    summary.push_str("| Dataset | reason | classify | consolidate | Total |\n");
    summary.push_str("| --- | ---: | ---: | ---: | ---: |\n");
    for (dataset_id, counts) in &counts_by_dataset {
        let reason_n = counts.get("reason").copied().unwrap_or(0);
        let classify_n = counts.get("classify").copied().unwrap_or(0);
        let consolidate_n = counts.get("consolidate").copied().unwrap_or(0);
        let total = reason_n + classify_n + consolidate_n;
        summary.push_str(&format!(
            "| {dataset_id} | {reason_n} | {classify_n} | {consolidate_n} | {total} |\n"
        ));
    }
    summary.push_str("\n## Example Requests (One Per Purpose)\n");
    for (purpose, ex) in &examples_by_purpose {
        summary.push_str(&format!(
            "\n### Purpose: `{purpose}` (dataset: `{}`, turn/probe: `{}`)\n\n",
            ex.dataset_id, ex.turn_or_probe_id
        ));
        summary.push_str("**System Prompt**\n\n```text\n");
        summary.push_str(&ex.system_prompt);
        summary.push_str("\n```\n\n**User Prompt**\n\n```text\n");
        summary.push_str(&ex.user_prompt);
        summary.push_str("\n```\n");
    }
    fs::write(dry_dir.join("summary.md"), summary)?;

    Ok(written)
}
