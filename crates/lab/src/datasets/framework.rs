//! Evaluation framework for external datasets: `Converter`/`Scorer` traits with compile-time
//! `GoldBook` isolation, `VisibleScope` evidence-visibility verification, Lab-side
//! `ScriptedModelProvider` silent-`think` history loading, `reason::EmbedderProbe`, converted cache,
//! seeded stratified sampling, and cluster-bootstrap confidence intervals (`R-52`–`R-54`).

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};

use monoagens_core::{
    CapabilityCatalogEntry, Core, CoreConfig, DecisionOutcome, Effect, EventDetail, EventKind,
    Input, LogSeq, MemStore, ModelPurpose, Store,
};
use monoagens_protocol::{
    ActorId, Audience, CURRENT_PROTOCOL_VERSION, CapabilityDescriptor, CapabilityId, EmbeddingTag,
    OpaquePayload, Percept, PerceptEmitParams, PlaceId, TaggedEmbedding, Time, VisibleScope,
    Waist2Headers, Waist2Method, Waist2Request,
};
use monoagens_reason::embed::{
    EmbedContent, EmbedError, EmbedInput, EmbedModality, Embedder, TaskPrefix,
};
use monoagens_reason::model::{
    ModelOutput, ModelProvider, ProviderFailureClass, ScriptedModelProvider, ScriptedReply,
};
use monoagens_reason::{RenderExtras, render_context_plan_with};
use serde::{Deserialize, Serialize};

use crate::LabError;
use crate::datasets::manifest::{DatasetManifestEntry, DatasetsManifest};
use crate::datasets::ops::{CONVERTER_CACHE_FILE, ConvertedCacheMetadata};
use crate::guard::CostGuard;
use crate::record::{FailedItemRecord, MetricScoreInterval, PurposeCost};
use crate::scenario::ensure_not_sealed_path;

/// Silent `ThoughtBatch` JSON emitted by Lab's `ScriptedModelProvider` during static history loading.
pub const SILENT_THINK_JSON: &str = r#"{"acts":[{"act":"think","thought":"silent history ingestion","motivation":1,"confidence_0_20":10,"used_ids":[]}]}"#;

/// Controls how `history` turns interact with the probe model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HistoryMode {
    /// Static memory/ToM histories. Offline runs load them through Core's unmodified loop with a
    /// Lab `ScriptedModelProvider` returning silent `think` (`$0`, logged as `scripted_silent`);
    /// live runs load every turn through Core's loop with the live provider (`Design 8`).
    StaticSilent,
    /// Interactive/timing benchmarks: decision turns use the active probe model provider.
    Interactive,
    /// Continuous stateful sequential timeline: probes execute sequentially on a single
    /// continuous `Core` instance so `ExpectationBook` and per-place `place_memory` (`h_t`)
    /// persist across simulated days rather than resetting per probe.
    LiveSequential,
}

/// One evaluation probe attached to an [`ExternalItem`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExternalProbe {
    pub probe_id: String,
    pub percept: Percept,
    /// When `true`, clones the post-ingestion checkpoint before running this probe.
    pub isolated_checkpoint: bool,
    /// Probe `VisibleScope` used by Core recall and verified by [`assert_visiblescope_gold_evidence`].
    pub probe_scope: VisibleScope,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_capability: Option<CapabilityId>,
}

/// One converted benchmark item visible to the Lab execution driver.
///
/// Compile-time isolation: contains zero gold answers, `.trace` flags, `_abs` suffixes,
/// `is_noise`, or `privacy_conflict` labels.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExternalItem {
    pub item_id: String,
    pub cluster_id: String,
    pub strata_key: String,
    pub history: Vec<Percept>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkpoint_group: Option<String>,
    pub probes: Vec<ExternalProbe>,
    pub history_mode: HistoryMode,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<CapabilityDescriptor>,
}

/// Reference to one historical evidence turn used to check `VisibleScope::allows`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoldEvidenceRef {
    pub turn_index: usize,
    pub audience: Audience,
    pub place: PlaceId,
}

/// Ground-truth entry held strictly inside [`GoldBook`] (never passed to Core or drivers).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoldEntry {
    pub item_id: String,
    pub probe_id: String,
    pub cluster_id: String,
    pub gold_answer: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub accepted_answers: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub gold_evidence: Vec<GoldEvidenceRef>,
    /// True for cross-channel `group_mem_bench` items whose evidence lacks the asker.
    #[serde(default)]
    pub privacy_conflict: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, String>,
}

/// Ground-truth oracle store kept strictly separate from [`ExternalItem`]s.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct GoldBook {
    pub dataset_id: String,
    pub entries: Vec<GoldEntry>,
}

impl GoldBook {
    /// Looks up a ground-truth entry by `(item_id, probe_id)`.
    #[must_use]
    pub fn find(&self, item_id: &str, probe_id: &str) -> Option<&GoldEntry> {
        self.entries
            .iter()
            .find(|e| e.item_id == item_id && e.probe_id == probe_id)
    }
}

/// Complete output of a [`Converter`] run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConvertedDataset {
    pub dataset_id: String,
    pub pinned_revision: String,
    pub converter_version: String,
    pub items: Vec<ExternalItem>,
    pub gold_book: GoldBook,
}

/// Trait implemented by every behavioral dataset converter under `crates/lab/src/datasets/`.
pub trait Converter: Send + Sync {
    fn dataset_id(&self) -> &str;
    fn converter_version(&self) -> &str;
    fn convert(
        &self,
        dataset_dir: &Path,
        manifest_entry: &DatasetManifestEntry,
    ) -> Result<ConvertedDataset, LabError>;
}

/// Recorded model/Core response for one `(item_id, probe_id)`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExternalPrediction {
    pub item_id: String,
    pub probe_id: String,
    pub cluster_id: String,
    pub spoke: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action_turn_ms: Option<u64>,
    pub probe_gate_wakes: u64,
    pub history_gate_wakes: u64,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub cost_usd: f64,
}

/// Scored metric with optional cluster-bootstrap confidence interval.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScorerMetric {
    pub name: String,
    pub value: Option<f64>,
    pub ci95_low: Option<f64>,
    pub ci95_high: Option<f64>,
    pub n_clusters: Option<usize>,
    pub not_scored_reason: Option<String>,
}

/// Aggregate scorer output.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScorerReport {
    pub dataset_id: String,
    pub scorer_version: String,
    pub metrics: Vec<ScorerMetric>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub judge_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub judge_prompt_version: Option<String>,
}

impl ScorerReport {
    /// Converts metrics into [`MetricScoreInterval`] entries for [`crate::RunRecord`].
    #[must_use]
    pub fn to_intervals(&self) -> Vec<MetricScoreInterval> {
        self.metrics
            .iter()
            .map(|m| MetricScoreInterval {
                metric: m.name.clone(),
                value: m.value,
                ci95_low: m.ci95_low,
                ci95_high: m.ci95_high,
                n_clusters: m.n_clusters,
                not_scored_reason: m.not_scored_reason.clone(),
            })
            .collect()
    }
}

/// Per-item scoring outcome recorded by scorers for failure analysis and transcripts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScorerItemOutcome {
    pub item_id: String,
    pub probe_id: String,
    pub cluster_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    pub metric_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub passed: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub judge_prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub judge_verdict: Option<String>,
    #[serde(default)]
    pub failed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_reason: Option<String>,
}

/// Thread-safe side-channel collector passed via [`JudgeExecutionConfig::recorder`] so scorers
/// can emit per-item outcomes, judge prompts/verdicts, category breakdowns, and failures without
/// breaking `Scorer::score` signatures.
#[derive(Debug, Default)]
pub struct ScorerDetailRecorder {
    pub outcomes: Vec<ScorerItemOutcome>,
    pub category_metrics: BTreeMap<String, BTreeMap<String, f64>>,
    pub failed_items: Vec<FailedItemRecord>,
}

/// Optional judge and local dataset/scorer-model configuration passed to [`Scorer::score`].
#[derive(Clone, Default)]
pub struct JudgeExecutionConfig<'a> {
    pub live: bool,
    pub judge_model: Option<&'a str>,
    pub judge_prompt_version: Option<&'a str>,
    pub provider: Option<&'a dyn ModelProvider>,
    pub datasets_dir: Option<&'a Path>,
    pub manifest: Option<&'a DatasetsManifest>,
    pub recorder: Option<Arc<Mutex<ScorerDetailRecorder>>>,
}

impl JudgeExecutionConfig<'_> {
    /// Records one scored (or failed) item outcome for transcript & failure-analysis emission.
    pub fn record_outcome(&self, outcome: ScorerItemOutcome) {
        if let Some(rec) = &self.recorder
            && let Ok(mut guard) = rec.lock()
        {
            if outcome.failed {
                guard.failed_items.push(FailedItemRecord {
                    item_id: outcome.item_id.clone(),
                    probe_id: outcome.probe_id.clone(),
                    stage: "judge".to_owned(),
                    reason: outcome
                        .failure_reason
                        .clone()
                        .unwrap_or_else(|| "judge call failed".to_owned()),
                });
            }
            guard.outcomes.push(outcome);
        }
    }

    /// Records a per-category breakdown metric for reporting.
    pub fn record_category_metric(
        &self,
        category: impl Into<String>,
        metric: impl Into<String>,
        val: f64,
    ) {
        if let Some(rec) = &self.recorder
            && let Ok(mut guard) = rec.lock()
        {
            guard
                .category_metrics
                .entry(category.into())
                .or_default()
                .insert(metric.into(), val);
        }
    }
}

impl std::fmt::Debug for JudgeExecutionConfig<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JudgeExecutionConfig")
            .field("live", &self.live)
            .field("judge_model", &self.judge_model)
            .field("judge_prompt_version", &self.judge_prompt_version)
            .field("provider", &self.provider.map(|p| p.provider_name()))
            .field("datasets_dir", &self.datasets_dir)
            .field("manifest_present", &self.manifest.is_some())
            .field("recorder_present", &self.recorder.is_some())
            .finish()
    }
}

/// Trait implemented by every behavioral dataset scorer.
pub trait Scorer: Send + Sync {
    fn dataset_id(&self) -> &str;
    fn scorer_version(&self) -> &str;
    fn score(
        &self,
        predictions: &[ExternalPrediction],
        gold: &GoldBook,
        judge: &JudgeExecutionConfig<'_>,
        seed: u64,
    ) -> Result<ScorerReport, LabError>;
}

/// Verifies the mandatory `VisibleScope` probe-subset invariant (`crates/protocol/src/facts.rs:256`)
/// for every `(ExternalItem, ExternalProbe)` and its matching [`GoldEntry`]:
/// - Non-conflict items: every gold evidence item MUST satisfy `probe_scope.allows(...) == true`.
/// - `privacy_conflict` items: at least one cross-channel evidence item MUST satisfy `allows == false`.
pub fn assert_visiblescope_gold_evidence(
    items: &[ExternalItem],
    gold: &GoldBook,
) -> Result<(), LabError> {
    let now = Time::from_millis(0);
    for item in items {
        for probe in &item.probes {
            if probe.percept.audience.is_empty() || probe.probe_scope.audience.is_empty() {
                return Err(LabError::Protocol(format!(
                    "item '{}' probe '{}' has empty audience",
                    item.item_id, probe.probe_id
                )));
            }
            if probe.percept.audience != probe.probe_scope.audience {
                return Err(LabError::Protocol(format!(
                    "item '{}' probe '{}' percept audience {:?} disagrees with probe_scope {:?}",
                    item.item_id,
                    probe.probe_id,
                    probe.percept.audience,
                    probe.probe_scope.audience
                )));
            }
            let gold_entry = gold.find(&item.item_id, &probe.probe_id).ok_or_else(|| {
                LabError::Protocol(format!(
                    "missing GoldEntry for item '{}' probe '{}'",
                    item.item_id, probe.probe_id
                ))
            })?;
            if gold_entry.gold_evidence.is_empty() {
                continue;
            }
            if gold_entry.privacy_conflict {
                let has_blocked = gold_entry
                    .gold_evidence
                    .iter()
                    .any(|ev| !probe.probe_scope.allows(&ev.audience, None, now));
                if !has_blocked {
                    return Err(LabError::Protocol(format!(
                        "item '{}' probe '{}' marked privacy_conflict=true but VisibleScope allowed all evidence",
                        item.item_id, probe.probe_id
                    )));
                }
            } else {
                for ev in &gold_entry.gold_evidence {
                    if !probe.probe_scope.allows(&ev.audience, None, now) {
                        return Err(LabError::Protocol(format!(
                            "VisibleScope blocked non-conflict gold evidence (turn {}) for item '{}' probe '{}': probe {:?} not subset of evidence {:?}",
                            ev.turn_index,
                            item.item_id,
                            probe.probe_id,
                            probe.probe_scope.audience,
                            ev.audience
                        )));
                    }
                }
            }
        }
    }
    Ok(())
}

/// Converted dataset cache filename (payload) paired with [`CONVERTER_CACHE_FILE`].
const CONVERTED_PAYLOAD_CACHE_FILE: &str = ".monoagens_converted_payload.json";

/// Loads a cached [`ConvertedDataset`] when `converter_version` and pinned revisions match
/// `manifest_entry`; otherwise runs `converter.convert` and refreshes the cache atomically.
/// Returns `(converted, cache_hit)`.
pub fn load_or_convert_cached(
    converter: &dyn Converter,
    dataset_dir: &Path,
    manifest_entry: &DatasetManifestEntry,
) -> Result<(ConvertedDataset, bool), LabError> {
    ensure_not_sealed_path(dataset_dir)?;
    let meta_path = dataset_dir.join(CONVERTER_CACHE_FILE);
    let payload_path = dataset_dir.join(CONVERTED_PAYLOAD_CACHE_FILE);
    let expected_revs: BTreeMap<String, String> = manifest_entry
        .sources
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let key = s.role.clone().unwrap_or_else(|| format!("source_{i}"));
            (key, s.revision.clone())
        })
        .collect();

    if meta_path.is_file() && payload_path.is_file() {
        let meta_raw = fs::read_to_string(&meta_path)?;
        if let Ok(meta) = serde_json::from_str::<ConvertedCacheMetadata>(&meta_raw)
            && meta.dataset_id == manifest_entry.id
            && meta.converter_version == converter.converter_version()
            && meta.converter_version == manifest_entry.converter_version
            && meta.revisions == expected_revs
        {
            let payload_raw = fs::read_to_string(&payload_path)?;
            if let Ok(converted) = serde_json::from_str::<ConvertedDataset>(&payload_raw)
                && converted.converter_version == converter.converter_version()
            {
                return Ok((converted, true));
            }
        }
    }

    let converted = converter.convert(dataset_dir, manifest_entry)?;
    if converted.dataset_id != manifest_entry.id {
        return Err(LabError::Protocol(format!(
            "converter dataset_id '{}' != manifest id '{}'",
            converted.dataset_id, manifest_entry.id
        )));
    }
    if converted.converter_version != converter.converter_version() {
        return Err(LabError::Protocol(format!(
            "converter output version '{}' != converter.converter_version() '{}'",
            converted.converter_version,
            converter.converter_version()
        )));
    }

    let meta = ConvertedCacheMetadata {
        dataset_id: manifest_entry.id.clone(),
        converter_version: converter.converter_version().to_owned(),
        revisions: expected_revs,
    };
    fs::create_dir_all(dataset_dir)?;
    // Unique temporary names per writer: parallel runs (or tests) converting the same dataset
    // must not rename each other's half-written files. The final renames are atomic, so the
    // last complete writer wins and readers never see a partial cache.
    let unique = format!(
        "{}.{:?}.{}",
        std::process::id(),
        std::thread::current().id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos())
    )
    .replace(|c: char| !c.is_ascii_alphanumeric() && c != '.', "_");
    let tmp_payload = dataset_dir.join(format!(".monoagens_converted_payload.{unique}.tmp"));
    let tmp_meta = dataset_dir.join(format!(".monoagens_converted_cache.{unique}.tmp"));
    fs::write(
        &tmp_payload,
        serde_json::to_vec(&converted).map_err(|e| LabError::Serialization(e.to_string()))?,
    )?;
    fs::write(
        &tmp_meta,
        serde_json::to_vec_pretty(&meta).map_err(|e| LabError::Serialization(e.to_string()))?,
    )?;
    fs::rename(&tmp_payload, &payload_path)?;
    fs::rename(&tmp_meta, &meta_path)?;

    Ok((converted, false))
}

/// Memoizing [`Embedder`] wrapper for deterministic reuse of identical `(regime_id, text)`
/// embeddings across clustered benchmark items sharing conversation histories.
pub struct CachingEmbedder<'a> {
    inner: &'a dyn Embedder,
    cache: Mutex<HashMap<(String, String), TaggedEmbedding>>,
}

impl<'a> CachingEmbedder<'a> {
    /// Wraps `inner` with an in-memory `(prompt_regime_id, text)` memoization table.
    #[must_use]
    pub fn new(inner: &'a dyn Embedder) -> Self {
        Self {
            inner,
            cache: Mutex::new(HashMap::new()),
        }
    }
}

impl Embedder for CachingEmbedder<'_> {
    fn backend_name(&self) -> &str {
        self.inner.backend_name()
    }

    fn model_ver(&self) -> Option<&str> {
        self.inner.model_ver()
    }

    fn dim_d(&self) -> Option<u32> {
        self.inner.dim_d()
    }

    fn max_context_tokens(&self) -> Option<usize> {
        self.inner.max_context_tokens()
    }

    fn supported_modalities(&self) -> &'static [EmbedModality] {
        self.inner.supported_modalities()
    }

    fn tag_for(&self, task: TaskPrefix) -> Option<EmbeddingTag> {
        self.inner.tag_for(task)
    }

    fn embed(&self, input: &EmbedInput<'_>) -> Result<Option<TaggedEmbedding>, EmbedError> {
        if let EmbedContent::Text(txt) = input.content {
            let key = (input.task.regime_id().to_owned(), txt.to_owned());
            if let Ok(guard) = self.cache.lock()
                && let Some(hit) = guard.get(&key)
            {
                return Ok(Some(hit.clone()));
            }
            let res = self.inner.embed(input)?;
            if let Some(ref emb) = res
                && let Ok(mut guard) = self.cache.lock()
            {
                guard.insert(key, emb.clone());
            }
            return Ok(res);
        }
        self.inner.embed(input)
    }
}

/// One event line written into `transcripts/<dataset>/<item_id>.jsonl`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "record_type", rename_all = "snake_case")]
pub enum TranscriptRecord {
    Percept {
        phase: String,
        turn_or_probe_id: String,
        actor: String,
        place: String,
        audience: Vec<String>,
        timestamp_ms: u64,
        text: String,
        embedding_dim: Option<u32>,
        embedding_tag: Option<String>,
    },
    GateDecision {
        phase: String,
        turn_or_probe_id: String,
        target_event_id: u64,
        outcome: String,
    },
    ModelCall {
        phase: String,
        turn_or_probe_id: String,
        purpose: String,
        model: String,
        system_prompt: String,
        user_prompt: String,
        raw_response: Option<String>,
        tokens_in: u64,
        tokens_out: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cached_tokens: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reasoning_tokens: Option<u64>,
        latency_ms: u64,
        outcome: String,
        error_detail: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        strict_schema: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        failure_field: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        failure_reason: Option<String>,
    },
    Action {
        probe_id: String,
        capability_id: String,
        places: Vec<String>,
        people: Vec<String>,
        payload_text: String,
    },
}

/// Execution outcome of running an [`ExternalItem`] through Core's unmodified loop.
#[derive(Debug, Clone)]
pub struct ExternalItemExecution {
    pub predictions: Vec<ExternalPrediction>,
    pub history_reasoning_mode: String,
    pub total_history_gate_wakes: u64,
    pub cost_by_purpose: Vec<PurposeCost>,
    pub transcript_events: Vec<TranscriptRecord>,
    pub failed_items: Vec<FailedItemRecord>,
    pub decisions_go: usize,
    pub decisions_note: usize,
    pub decisions_silent: usize,
    pub decisions_door_blocked: usize,
    pub actions_dispatched: usize,
    pub percepts_count: usize,
    pub model_latencies_ms: Vec<u64>,
}

/// Executes one [`ExternalItem`] against Core's unmodified `Core::open` + `Core::step` loop.
#[allow(clippy::too_many_arguments)]
pub fn execute_external_item(
    item: &ExternalItem,
    config: &CoreConfig,
    seed: u64,
    probe_provider: &dyn ModelProvider,
    probe_model_name: &str,
    live: bool,
    guard: &mut CostGuard,
) -> Result<ExternalItemExecution, LabError> {
    execute_external_item_with_embedder(
        item,
        config,
        seed,
        probe_provider,
        probe_model_name,
        live,
        guard,
        None,
        None,
        None,
        None,
    )
}

/// Executes one [`ExternalItem`] against Core's unmodified loop, embedding every percept and
/// probe with `embedder` before Core sees it (Gateway pre-queue role) and recording transcripts.
///
/// Every model call is rendered with the production renderer
/// ([`monoagens_reason::render_context_plan_with`] plus `populate_payloads_from_store`) and parsed
/// as a strict ThoughtBatch; answers are the payloads of Core's dispatched `Act`s. When `live` is
/// `true`, every history percept is loaded through Core's loop with `probe_provider`.
#[allow(clippy::too_many_arguments)]
pub fn execute_external_item_with_embedder(
    item: &ExternalItem,
    config: &CoreConfig,
    seed: u64,
    probe_provider: &dyn ModelProvider,
    probe_model_name: &str,
    live: bool,
    guard: &mut CostGuard,
    embedder: Option<&dyn Embedder>,
    attribution: Option<&crate::memory::proxy::CallAttribution>,
    request_timeout_ms: Option<u64>,
    max_output_tokens: Option<u32>,
) -> Result<ExternalItemExecution, LabError> {
    let (exec, _state) = execute_external_item_with_model_and_state(
        item,
        config,
        seed,
        None,
        probe_provider,
        probe_model_name,
        live,
        guard,
        embedder,
        attribution,
        request_timeout_ms,
        max_output_tokens,
    )?;
    Ok(exec)
}

/// Executes one [`ExternalItem`] against Core's unmodified loop with an optional [`monoagens_core::NeuralModel`],
/// returning both the [`ExternalItemExecution`] and the final [`monoagens_core::CoreState`].
#[allow(clippy::too_many_arguments)]
pub fn execute_external_item_with_model_and_state(
    item: &ExternalItem,
    config: &CoreConfig,
    seed: u64,
    model: Option<Arc<monoagens_core::NeuralModel>>,
    probe_provider: &dyn ModelProvider,
    probe_model_name: &str,
    live: bool,
    guard: &mut CostGuard,
    embedder: Option<&dyn Embedder>,
    attribution: Option<&crate::memory::proxy::CallAttribution>,
    request_timeout_ms: Option<u64>,
    max_output_tokens: Option<u32>,
) -> Result<(ExternalItemExecution, monoagens_core::CoreState), LabError> {
    let cfg = CoreConfig {
        seed,
        ..config.clone()
    };
    let mut core = Core::open(MemStore::new(), cfg, model.clone())?;
    register_item_capabilities(&mut core, item);
    let silent_provider = {
        let mut p = ScriptedModelProvider::new();
        p.set_fallback(ScriptedReply::text(SILENT_THINK_JSON));
        p
    };

    let use_silent_history = item.history_mode == HistoryMode::StaticSilent && !live;
    let history_mode_label = match item.history_mode {
        HistoryMode::StaticSilent if !live => "scripted_silent",
        HistoryMode::StaticSilent => "live",
        HistoryMode::Interactive => "interactive",
        HistoryMode::LiveSequential => "live_sequential",
    };
    // Stable per-run-and-item session value so repeated prefixes hit the provider prompt cache.
    let session_id = conversation_id_for(seed, &item.item_id);

    let mut history_gate_wakes = 0_u64;
    let mut purpose_costs: BTreeMap<u8, PurposeCost> = BTreeMap::new();
    let mut last_time = Time::from_millis(0);
    let mut transcript_events = Vec::new();
    let mut failed_items = Vec::new();
    let mut decisions_go = 0usize;
    let mut decisions_note = 0usize;
    let mut decisions_silent = 0usize;
    let mut decisions_door_blocked = 0usize;
    let mut actions_dispatched = 0usize;
    let mut percepts_count = 0usize;
    let mut model_latencies_ms = Vec::new();
    let mut hist_last_seq: Option<LogSeq> = None;

    // Production Gateway steps `Input::Tick` whenever `Core::next_due()` passes (open waits,
    // expectation expiry, silence windows, sleep consolidation). Under virtual time the Lab
    // fires the same Ticks at their due times before the next input, so the time-driven parts
    // of MonoAgens run as they do in production.
    macro_rules! fire_due_ticks {
        ($until:expr, $phase_id:expr, $provider:expr, $model:expr) => {{
            let until: Time = $until;
            let mut fired = 0_u32;
            while let Some(due) = core.next_due() {
                if due > until {
                    break;
                }
                fired += 1;
                if fired > 100_000 {
                    return Err(LabError::Protocol(format!(
                        "item {}: more than 100000 due Ticks before {}",
                        item.item_id,
                        until.as_millis()
                    )));
                }
                let tick_effects = core.step(Input::Tick, due, guard)?;
                collect_new_store_events(
                    &mut core,
                    &mut hist_last_seq,
                    "history",
                    $phase_id,
                    &mut decisions_go,
                    &mut decisions_note,
                    &mut decisions_silent,
                    &mut decisions_door_blocked,
                    &mut transcript_events,
                )?;
                let mut tick_ctx = DriveEffectsContext {
                    phase: "history",
                    turn_or_probe_id: $phase_id,
                    item_id: &item.item_id,
                    probe_id: $phase_id,
                    is_silent_history: use_silent_history,
                    session_id: &session_id,
                    attribution,
                    request_timeout_ms,
                    max_output_tokens,
                    transcript_events: &mut transcript_events,
                    failed_items: &mut failed_items,
                    actions_dispatched: &mut actions_dispatched,
                    model_latencies_ms: &mut model_latencies_ms,
                };
                let (tick_wakes, _, _, _) = drive_model_effects(
                    &mut core,
                    tick_effects,
                    due,
                    guard,
                    $provider,
                    $model,
                    &mut purpose_costs,
                    &mut tick_ctx,
                )?;
                collect_new_store_events(
                    &mut core,
                    &mut hist_last_seq,
                    "history",
                    $phase_id,
                    &mut decisions_go,
                    &mut decisions_note,
                    &mut decisions_silent,
                    &mut decisions_door_blocked,
                    &mut transcript_events,
                )?;
                history_gate_wakes = history_gate_wakes.saturating_add(tick_wakes);
                if core.next_due() == Some(due) {
                    // The Tick changed nothing that moves the schedule; stop rather than spin.
                    break;
                }
            }
        }};
    }

    for (idx, percept) in item.history.iter().enumerate() {
        let mut p = percept.clone();
        p.protocol_version = CURRENT_PROTOCOL_VERSION;
        embed_percept_if_configured(&mut p, embedder)?;
        let p_text = String::from_utf8_lossy(p.payload.as_bytes()).into_owned();
        let emb_meta = p.embeddings.first().map(|e| {
            (
                e.tag.dim_d,
                format!(
                    "{}:{}:{}",
                    e.tag.embed_model_ver, e.tag.dim_d, e.tag.prompt_regime_id
                ),
            )
        });
        transcript_events.push(TranscriptRecord::Percept {
            phase: "history".to_owned(),
            turn_or_probe_id: format!("hist_{idx}"),
            actor: p.actor.as_str().to_owned(),
            place: p.place.as_str().to_owned(),
            audience: p
                .audience
                .members
                .iter()
                .map(|a| a.as_str().to_owned())
                .collect(),
            timestamp_ms: p.timestamp_ms.as_millis(),
            text: p_text,
            embedding_dim: emb_meta.as_ref().map(|m| m.0),
            embedding_tag: emb_meta.map(|m| m.1),
        });
        percepts_count += 1;

        let req = Waist2Request::new(
            None,
            Waist2Method::PerceptEmit,
            PerceptEmitParams::from_percept(
                Waist2Headers::new(format!("ext-{}-hist-{idx}", item.item_id)),
                p,
            ),
        );
        let parsed_percept = req
            .params
            .into_percept(CURRENT_PROTOCOL_VERSION)
            .map_err(|e| LabError::Protocol(e.to_string()))?;
        last_time = parsed_percept.timestamp_ms;
        {
            let tick_phase = format!("hist_{idx}");
            let (tick_provider, tick_model): (&dyn ModelProvider, &str) = if use_silent_history {
                (&silent_provider, "scripted_silent")
            } else {
                (probe_provider, probe_model_name)
            };
            fire_due_ticks!(last_time, &tick_phase, tick_provider, tick_model);
        }
        let effects = core.step(Input::Percept(Box::new(parsed_percept)), last_time, guard)?;
        collect_new_store_events(
            &mut core,
            &mut hist_last_seq,
            "history",
            &format!("hist_{idx}"),
            &mut decisions_go,
            &mut decisions_note,
            &mut decisions_silent,
            &mut decisions_door_blocked,
            &mut transcript_events,
        )?;
        let history_provider: &dyn ModelProvider = if use_silent_history {
            &silent_provider
        } else {
            probe_provider
        };
        let phase_id = format!("hist_{idx}");
        let mut drive_ctx = DriveEffectsContext {
            phase: "history",
            turn_or_probe_id: &phase_id,
            item_id: &item.item_id,
            probe_id: &phase_id,
            is_silent_history: use_silent_history,
            session_id: &session_id,
            attribution,
            request_timeout_ms,
            max_output_tokens,
            transcript_events: &mut transcript_events,
            failed_items: &mut failed_items,
            actions_dispatched: &mut actions_dispatched,
            model_latencies_ms: &mut model_latencies_ms,
        };
        let (wakes, _tok_in, _tok_out, _reply) = drive_model_effects(
            &mut core,
            effects,
            last_time,
            guard,
            history_provider,
            if use_silent_history {
                "scripted_silent"
            } else {
                probe_model_name
            },
            &mut purpose_costs,
            &mut drive_ctx,
        )?;
        collect_new_store_events(
            &mut core,
            &mut hist_last_seq,
            "history",
            &phase_id,
            &mut decisions_go,
            &mut decisions_note,
            &mut decisions_silent,
            &mut decisions_door_blocked,
            &mut transcript_events,
        )?;
        history_gate_wakes = history_gate_wakes.saturating_add(wakes);
    }

    // Time passes between the end of the history and the questions; due Ticks (sleep after the
    // last session, expiries) run before the checkpoint every probe starts from.
    if let Some(first_probe_ms) = item
        .probes
        .iter()
        .map(|p| p.percept.timestamp_ms)
        .min()
        .filter(|t| *t > last_time)
    {
        let (tick_provider, tick_model): (&dyn ModelProvider, &str) = if use_silent_history {
            (&silent_provider, "scripted_silent")
        } else {
            (probe_provider, probe_model_name)
        };
        // Strictly before the first question, so it is not preceded by a same-instant Tick.
        let until = Time::from_millis(first_probe_ms.as_millis().saturating_sub(1));
        fire_due_ticks!(until, "pre_probe", tick_provider, tick_model);
        last_time = last_time.max(until);
    }

    let mut predictions = Vec::with_capacity(item.probes.len());
    if item.history_mode == HistoryMode::LiveSequential {
        for (p_idx, probe) in item.probes.iter().enumerate() {
            let probe_ts = probe.percept.timestamp_ms.max(last_time);
            if probe_ts > last_time {
                let until = Time::from_millis(probe_ts.as_millis().saturating_sub(1));
                fire_due_ticks!(until, &probe.probe_id, probe_provider, probe_model_name);
            }
            let pred = execute_probe_on_core(
                &mut core,
                &mut hist_last_seq,
                item,
                p_idx,
                probe,
                last_time,
                history_gate_wakes,
                &session_id,
                probe_provider,
                probe_model_name,
                guard,
                embedder,
                attribution,
                request_timeout_ms,
                max_output_tokens,
                &mut purpose_costs,
                &mut transcript_events,
                &mut failed_items,
                &mut decisions_go,
                &mut decisions_note,
                &mut decisions_silent,
                &mut decisions_door_blocked,
                &mut actions_dispatched,
                &mut percepts_count,
                &mut model_latencies_ms,
            )?;
            last_time = probe_ts;
            predictions.push(pred);
        }
    } else {
        let checkpoint = core.take_snapshot(format!("post-ingestion-{}", item.item_id))?;
        let base_events = core.store().read_after_seq(None, usize::MAX)?;

        for (p_idx, probe) in item.probes.iter().enumerate() {
            let mut cloned_store = MemStore::new();
            let mut cloned_last_seq: Option<LogSeq> = None;
            for rec in &base_events {
                let eid = cloned_store.append(rec.clone())?;
                cloned_last_seq = Some(rec.seq);
                if let Some(EventDetail::PerceptMeta { embeddings, .. }) = &rec.detail {
                    for emb in embeddings {
                        cloned_store.insert_vector(eid, emb.clone());
                    }
                }
            }
            if let Some(last_rec) = base_events.last() {
                cloned_last_seq = Some(last_rec.seq);
            }
            cloned_store.snapshot(checkpoint.clone())?;
            let probe_cfg = CoreConfig {
                seed,
                ..config.clone()
            };
            let mut probe_core = Core::open(cloned_store, probe_cfg, model.clone())?;
            register_item_capabilities(&mut probe_core, item);
            // Advance cursor past any ConfigLoaded/Open events appended by Core::open
            let open_tail = probe_core
                .store()
                .read_after_seq(cloned_last_seq, usize::MAX)?;
            if let Some(last_open) = open_tail.last() {
                cloned_last_seq = Some(last_open.seq);
            }

            let pred = execute_probe_on_core(
                &mut probe_core,
                &mut cloned_last_seq,
                item,
                p_idx,
                probe,
                last_time,
                history_gate_wakes,
                &session_id,
                probe_provider,
                probe_model_name,
                guard,
                embedder,
                attribution,
                request_timeout_ms,
                max_output_tokens,
                &mut purpose_costs,
                &mut transcript_events,
                &mut failed_items,
                &mut decisions_go,
                &mut decisions_note,
                &mut decisions_silent,
                &mut decisions_door_blocked,
                &mut actions_dispatched,
                &mut percepts_count,
                &mut model_latencies_ms,
            )?;
            predictions.push(pred);
        }
    }

    let final_state = core.state().clone();
    Ok((
        ExternalItemExecution {
            predictions,
            history_reasoning_mode: history_mode_label.to_owned(),
            total_history_gate_wakes: history_gate_wakes,
            cost_by_purpose: purpose_costs.into_values().collect(),
            transcript_events,
            failed_items,
            decisions_go,
            decisions_note,
            decisions_silent,
            decisions_door_blocked,
            actions_dispatched,
            percepts_count,
            model_latencies_ms,
        },
        final_state,
    ))
}

#[allow(clippy::too_many_arguments)]
fn execute_probe_on_core(
    probe_core: &mut Core<MemStore>,
    last_seq: &mut Option<LogSeq>,
    item: &ExternalItem,
    p_idx: usize,
    probe: &ExternalProbe,
    last_time: Time,
    history_gate_wakes: u64,
    session_id: &str,
    probe_provider: &dyn ModelProvider,
    probe_model_name: &str,
    guard: &mut CostGuard,
    embedder: Option<&dyn Embedder>,
    attribution: Option<&crate::memory::proxy::CallAttribution>,
    request_timeout_ms: Option<u64>,
    max_output_tokens: Option<u32>,
    purpose_costs: &mut BTreeMap<u8, PurposeCost>,
    transcript_events: &mut Vec<TranscriptRecord>,
    failed_items: &mut Vec<FailedItemRecord>,
    decisions_go: &mut usize,
    decisions_note: &mut usize,
    decisions_silent: &mut usize,
    decisions_door_blocked: &mut usize,
    actions_dispatched: &mut usize,
    percepts_count: &mut usize,
    model_latencies_ms: &mut Vec<u64>,
) -> Result<ExternalPrediction, LabError> {
    let mut p = probe.percept.clone();
    p.protocol_version = CURRENT_PROTOCOL_VERSION;
    // Evaluation probes are addressed to the Soul by definition: without an addressee the
    // Gate legitimately holds silence and no answer would ever be produced.
    p.addressees = vec![ActorId::new("soul")];
    embed_percept_if_configured(&mut p, embedder)?;
    let p_text = String::from_utf8_lossy(p.payload.as_bytes()).into_owned();
    let emb_meta = p.embeddings.first().map(|e| {
        (
            e.tag.dim_d,
            format!(
                "{}:{}:{}",
                e.tag.embed_model_ver, e.tag.dim_d, e.tag.prompt_regime_id
            ),
        )
    });
    transcript_events.push(TranscriptRecord::Percept {
        phase: "probe".to_owned(),
        turn_or_probe_id: probe.probe_id.clone(),
        actor: p.actor.as_str().to_owned(),
        place: p.place.as_str().to_owned(),
        audience: p
            .audience
            .members
            .iter()
            .map(|a| a.as_str().to_owned())
            .collect(),
        timestamp_ms: p.timestamp_ms.as_millis(),
        text: p_text,
        embedding_dim: emb_meta.as_ref().map(|m| m.0),
        embedding_tag: emb_meta.map(|m| m.1),
    });
    *percepts_count += 1;

    let req = Waist2Request::new(
        None,
        Waist2Method::PerceptEmit,
        PerceptEmitParams::from_percept(
            Waist2Headers::new(format!("ext-{}-probe-{p_idx}", item.item_id)),
            p,
        ),
    );
    let parsed_probe = req
        .params
        .into_percept(CURRENT_PROTOCOL_VERSION)
        .map_err(|e| LabError::Protocol(e.to_string()))?;
    let probe_ts = parsed_probe.timestamp_ms.max(last_time);
    let effects = probe_core.step(Input::Percept(Box::new(parsed_probe)), probe_ts, guard)?;
    collect_new_store_events(
        probe_core,
        last_seq,
        "probe",
        &probe.probe_id,
        decisions_go,
        decisions_note,
        decisions_silent,
        decisions_door_blocked,
        transcript_events,
    )?;

    let mut drive_ctx = DriveEffectsContext {
        phase: "probe",
        turn_or_probe_id: &probe.probe_id,
        item_id: &item.item_id,
        probe_id: &probe.probe_id,
        is_silent_history: false,
        session_id,
        attribution,
        request_timeout_ms,
        max_output_tokens,
        transcript_events,
        failed_items,
        actions_dispatched,
        model_latencies_ms,
    };
    let (probe_wakes, tok_in, tok_out, reply_text) = drive_model_effects(
        probe_core,
        effects,
        probe_ts,
        guard,
        probe_provider,
        probe_model_name,
        purpose_costs,
        &mut drive_ctx,
    )?;
    collect_new_store_events(
        probe_core,
        last_seq,
        "probe",
        &probe.probe_id,
        decisions_go,
        decisions_note,
        decisions_silent,
        decisions_door_blocked,
        transcript_events,
    )?;

    Ok(ExternalPrediction {
        item_id: item.item_id.clone(),
        probe_id: probe.probe_id.clone(),
        cluster_id: item.cluster_id.clone(),
        spoke: reply_text.is_some(),
        response_text: reply_text,
        action_turn_ms: Some(probe_ts.as_millis()),
        probe_gate_wakes: probe_wakes,
        history_gate_wakes,
        tokens_in: tok_in,
        tokens_out: tok_out,
        cost_usd: 0.0,
    })
}

/// Stable conversation identifier shared by every call of one run and item (`R-58`).
#[must_use]
pub fn conversation_id_for(seed: u64, item_id: &str) -> String {
    let digest = crate::sha256_hex(format!("{seed}:{item_id}").as_bytes());
    format!("ses_{}", &digest[..32])
}

fn embed_percept_if_configured(
    percept: &mut Percept,
    embedder: Option<&dyn Embedder>,
) -> Result<(), LabError> {
    let Some(emb) = embedder else {
        return Ok(());
    };
    if !percept.embeddings.is_empty() || percept.payload.is_empty() {
        return Ok(());
    }
    let Ok(text) = std::str::from_utf8(percept.payload.as_bytes()) else {
        return Ok(());
    };
    if text.trim().is_empty() {
        return Ok(());
    }
    match emb.embed(&EmbedInput::document(text)) {
        Ok(Some(tagged)) => {
            percept.embeddings.push(tagged);
            Ok(())
        }
        Ok(None) => Ok(()),
        Err(EmbedError::ContextLimitExceeded { .. }) => {
            // Truncate safely on byte boundary below 8K-token budget (~24KB) if a mega-chunk exceeds window.
            let mut end = text.len().min(20_000);
            while end > 0 && !text.is_char_boundary(end) {
                end -= 1;
            }
            let truncated = &text[..end];
            let tagged = emb
                .embed(&EmbedInput::document(truncated))
                .map_err(|e| LabError::Protocol(format!("semantic embedder failed: {e}")))?;
            if let Some(t) = tagged {
                percept.embeddings.push(t);
            }
            Ok(())
        }
        Err(err) => Err(LabError::Protocol(format!(
            "semantic embedder failed: {err}"
        ))),
    }
}

#[allow(clippy::too_many_arguments)]
fn collect_new_store_events(
    core: &mut Core<MemStore>,
    last_seq: &mut Option<LogSeq>,
    phase: &str,
    turn_or_probe_id: &str,
    decisions_go: &mut usize,
    decisions_note: &mut usize,
    decisions_silent: &mut usize,
    decisions_door_blocked: &mut usize,
    transcript_events: &mut Vec<TranscriptRecord>,
) -> Result<(), LabError> {
    let events = core.store().read_after_seq(*last_seq, usize::MAX)?;
    let mut to_index: Vec<(monoagens_protocol::EventId, TaggedEmbedding)> = Vec::new();
    for rec in &events {
        *last_seq = Some(rec.seq);
        match (&rec.kind, &rec.detail) {
            (EventKind::Percept, Some(EventDetail::PerceptMeta { embeddings, .. })) => {
                for emb in embeddings {
                    to_index.push((rec.id, emb.clone()));
                }
            }
            (
                EventKind::Decision,
                Some(EventDetail::Decision {
                    target_event_id,
                    outcome,
                    ..
                }),
            ) => {
                let outcome_lbl = match outcome {
                    DecisionOutcome::Go => {
                        *decisions_go += 1;
                        "go"
                    }
                    DecisionOutcome::Note => {
                        *decisions_note += 1;
                        "note"
                    }
                    DecisionOutcome::Silent => {
                        *decisions_silent += 1;
                        "silent"
                    }
                    DecisionOutcome::DoorBlocked => {
                        *decisions_door_blocked += 1;
                        "door_blocked"
                    }
                    DecisionOutcome::Processed => "processed",
                };
                transcript_events.push(TranscriptRecord::GateDecision {
                    phase: phase.to_owned(),
                    turn_or_probe_id: turn_or_probe_id.to_owned(),
                    target_event_id: target_event_id.as_u64(),
                    outcome: outcome_lbl.to_owned(),
                });
            }
            _ => {}
        }
    }
    for (eid, emb) in to_index {
        core.store_mut().insert_vector(eid, emb);
    }
    Ok(())
}

/// Registers an item's declared capabilities exactly as a chat body declares them, so the
/// rendered prompt advertises what the soul may dispatch.
fn register_item_capabilities(core: &mut Core<MemStore>, item: &ExternalItem) {
    if item.capabilities.is_empty() {
        return;
    }
    let members: Vec<monoagens_protocol::ActorId> = item
        .probes
        .iter()
        .flat_map(|p| p.percept.audience.members.iter().cloned())
        .chain(
            item.history
                .iter()
                .flat_map(|p| p.audience.members.iter().cloned()),
        )
        .collect();
    let audience = monoagens_protocol::Audience::new(members);
    for descriptor in &item.capabilities {
        core.register_capability_item(CapabilityCatalogEntry {
            descriptor: descriptor.clone(),
            is_static_core: true,
            owner_signed: true,
            active: true,
            audience: audience.clone(),
            embedding: None,
        });
    }
}

struct DriveEffectsContext<'a> {
    phase: &'a str,
    turn_or_probe_id: &'a str,
    item_id: &'a str,
    probe_id: &'a str,
    is_silent_history: bool,
    /// Stable conversation identifier for this run and item.
    session_id: &'a str,
    /// Proxy attribution for this item; the phase is set per call.
    attribution: Option<&'a crate::memory::proxy::CallAttribution>,
    /// Configured provider/model timeout; replaces the renderer's 30 s default when present.
    request_timeout_ms: Option<u64>,
    /// Configured model output cap (`model.max_output_tokens`); `None` leaves it unbounded.
    max_output_tokens: Option<u32>,
    transcript_events: &'a mut Vec<TranscriptRecord>,
    failed_items: &'a mut Vec<FailedItemRecord>,
    actions_dispatched: &'a mut usize,
    model_latencies_ms: &'a mut Vec<u64>,
}

#[allow(clippy::too_many_arguments)]
fn drive_model_effects(
    core: &mut Core<MemStore>,
    initial_effects: Vec<Effect>,
    now: Time,
    guard: &mut CostGuard,
    provider: &dyn ModelProvider,
    model_name: &str,
    purpose_costs: &mut BTreeMap<u8, PurposeCost>,
    ctx: &mut DriveEffectsContext<'_>,
) -> Result<(u64, u64, u64, Option<String>), LabError> {
    let mut queue = initial_effects;
    let mut wakes = 0_u64;
    let mut tokens_in = 0_u64;
    let mut tokens_out = 0_u64;
    let mut spoken_text: Option<String> = None;

    while !queue.is_empty() {
        let mut next_queue = Vec::new();
        for effect in queue {
            match effect {
                Effect::Dispatch(action) => {
                    *ctx.actions_dispatched += 1;
                    let text = String::from_utf8_lossy(action.payload.as_bytes()).into_owned();
                    if !action.payload.is_empty() {
                        spoken_text = Some(text.clone());
                    }
                    ctx.transcript_events.push(TranscriptRecord::Action {
                        probe_id: ctx.probe_id.to_owned(),
                        capability_id: action.capability_id.as_str().to_owned(),
                        places: action
                            .places
                            .iter()
                            .map(|p| p.as_str().to_owned())
                            .collect(),
                        people: action
                            .audience
                            .members
                            .iter()
                            .map(|p| p.as_str().to_owned())
                            .collect(),
                        payload_text: text,
                    });
                }
                Effect::Model { purpose, plan } => {
                    wakes = wakes.saturating_add(1);
                    let request_id = plan
                        .trigger_event_id
                        .unwrap_or(monoagens_protocol::EventId::new(0));
                    // Production path: the Lab never authors a prompt. `RenderExtras` populates
                    // VisibleScope-authorized payloads from the real store, and the renderer
                    // produces the same prompt Gateway sends in production.
                    let mut extras = RenderExtras::default();
                    extras
                        .populate_payloads_from_store(&plan, core.store())
                        .map_err(|e| LabError::Protocol(format!("render extras failed: {e}")))?;
                    let rendered = render_context_plan_with(&plan, &extras);
                    let mut req = rendered.into_model_request(
                        request_id,
                        purpose,
                        model_name,
                        extras.attachments.clone(),
                    );
                    req.conversation_id = Some(ctx.session_id.to_owned());
                    if let Some(timeout_ms) = ctx.request_timeout_ms {
                        req.timeout_ms = timeout_ms.max(1);
                    }
                    if let Some(cap) = ctx.max_output_tokens {
                        req.max_output_tokens = Some(cap);
                    }
                    let purpose_lbl = match purpose {
                        ModelPurpose::Reason => "reason",
                        ModelPurpose::Classify => "classify",
                        ModelPurpose::Consolidate => "consolidate",
                    };
                    if let Some(attribution) = ctx.attribution {
                        let phase = if ctx.phase == "history" {
                            "ingest"
                        } else {
                            "answer"
                        };
                        req.extra_headers = crate::memory::proxy::CallAttribution {
                            participant: attribution.participant.clone(),
                            dataset: attribution.dataset.clone(),
                            conversation: attribution.conversation.clone(),
                            phase: phase.to_owned(),
                        }
                        .headers();
                        req.extra_headers
                            .push(("x-lab-purpose".to_owned(), purpose_lbl.to_owned()));
                    }
                    let system_prompt = rendered.static_prefix.clone();
                    let prompt = rendered.dynamic_context.clone();
                    let mut invoke_res = provider.invoke(&req);
                    if let Err(ref fail) = invoke_res
                        && fail.class == ProviderFailureClass::InvalidOutput
                        && fail.detail.contains("EOF while parsing")
                        && !guard.budget_exhausted()
                        && !guard.max_tokens.is_some_and(|limit| guard.tokens_used >= limit)
                    {
                        invoke_res = provider.invoke(&req);
                    }
                    let (inp, call_in, call_out, _call_cached, _call_reasoning) =
                        match invoke_res {
                            Ok(ok) => {
                                let usage = ok.record.usage;
                                let (tin, tout, treas, tcached) = if ctx.is_silent_history {
                                    (0, 0, None, None)
                                } else {
                                    usage.map_or((0, 0, None, None), |u| {
                                        (
                                            u.input_tokens,
                                            u.output_tokens,
                                            u.reasoning_tokens,
                                            u.cached_tokens,
                                        )
                                    })
                                };
                                if !ctx.is_silent_history {
                                    ctx.model_latencies_ms.push(ok.record.latency_ms);
                                }
                                ctx.transcript_events.push(TranscriptRecord::ModelCall {
                                    phase: ctx.phase.to_owned(),
                                    turn_or_probe_id: ctx.turn_or_probe_id.to_owned(),
                                    purpose: purpose_lbl.to_owned(),
                                    model: ok.record.model.clone(),
                                    system_prompt: system_prompt.clone(),
                                    user_prompt: prompt.clone(),
                                    raw_response: Some(ok.raw_text.clone()),
                                    tokens_in: tin,
                                    tokens_out: tout,
                                    cached_tokens: tcached,
                                    session_id: Some(ctx.session_id.to_owned()),
                                    reasoning_tokens: treas,
                                    latency_ms: ok.record.latency_ms,
                                    outcome: "success".to_owned(),
                                    error_detail: None,
                                    strict_schema: Some(ok.record.strict_schema),
                                    failure_field: None,
                                    failure_reason: None,
                                });
                                let inp = match ok.output {
                                    ModelOutput::ThoughtActs(parsed) => Input::ModelResult {
                                        request_id,
                                        acts: parsed.acts,
                                    },
                                    ModelOutput::Verdict(verdict) => Input::ClassifyResult {
                                        request_id,
                                        verdict,
                                    },
                                };
                                (inp, tin, tout, tcached, treas)
                            }
                            Err(fail) => {
                                if !ctx.is_silent_history {
                                    ctx.model_latencies_ms.push(fail.record.latency_ms);
                                    ctx.failed_items.push(FailedItemRecord {
                                        item_id: ctx.item_id.to_owned(),
                                        probe_id: ctx.probe_id.to_owned(),
                                        stage: "soul_model".to_owned(),
                                        reason: fail.to_string(),
                                    });
                                }
                                let fmt_rec = fail.output_format_record();
                                ctx.transcript_events.push(TranscriptRecord::ModelCall {
                                    phase: ctx.phase.to_owned(),
                                    turn_or_probe_id: ctx.turn_or_probe_id.to_owned(),
                                    purpose: purpose_lbl.to_owned(),
                                    model: fail.record.model.clone(),
                                    system_prompt: system_prompt.clone(),
                                    user_prompt: prompt.clone(),
                                    raw_response: None,
                                    tokens_in: 0,
                                    tokens_out: 0,
                                    cached_tokens: None,
                                    session_id: Some(ctx.session_id.to_owned()),
                                    reasoning_tokens: None,
                                    latency_ms: fail.record.latency_ms,
                                    outcome: format!("failed:{}", fail.class),
                                    error_detail: Some(fail.detail.clone()),
                                    strict_schema: Some(fail.record.strict_schema),
                                    failure_field: fmt_rec.as_ref().map(|r| r.field.clone()),
                                    failure_reason: fmt_rec.as_ref().map(|r| r.reason.clone()),
                                });
                                (fail.to_core_input(), 0, 0, None, None)
                            }
                        };
                    tokens_in = tokens_in.saturating_add(call_in);
                    tokens_out = tokens_out.saturating_add(call_out);
                    let call_toks = call_in.saturating_add(call_out);
                    guard.record_tokens(call_toks);
                    let call_cost_usd = if ctx.is_silent_history || call_toks == 0 {
                        0.0
                    } else {
                        (call_toks as f64 / 1_000.0) * 0.01
                    };
                    guard.record_cost_usd(call_cost_usd);
                    let key = match purpose {
                        ModelPurpose::Reason => 0,
                        ModelPurpose::Classify => 1,
                        ModelPurpose::Consolidate => 2,
                    };
                    if !ctx.is_silent_history {
                        let entry = purpose_costs.entry(key).or_insert(PurposeCost {
                            purpose,
                            calls: 0,
                            tokens_in: 0,
                            tokens_out: 0,
                            cost_usd: 0.0,
                        });
                        entry.calls += 1;
                        entry.tokens_in += call_in;
                        entry.tokens_out += call_out;
                        entry.cost_usd += call_cost_usd;
                    }

                    let follow = core.step(inp, now, guard)?;
                    next_queue.extend(follow);
                }
            }
        }
        queue = next_queue;
    }

    Ok((wakes, tokens_in, tokens_out, spoken_text))
}

/// Readiness state of a semantic [`Embedder`] for non-fixture component probes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EmbedderReadiness {
    Ready,
    Unconfigured(String),
}

/// Enforces that component embedder probes reject `HashEmbedder`, `NoneEmbedder`, and
/// `ScriptedEmbedder` outside unit/fixture tests, and returns `Unconfigured("embedder_unconfigured")`
/// when local semantic model weights are absent (`None` or `MissingModelPath`).
pub fn validate_semantic_embedder(
    embedder: Option<&dyn Embedder>,
    allow_test_embedders: bool,
) -> Result<EmbedderReadiness, LabError> {
    let Some(embedder) = embedder else {
        return Ok(EmbedderReadiness::Unconfigured(
            "embedder_unconfigured".to_owned(),
        ));
    };

    let backend = embedder.backend_name().to_ascii_lowercase();
    let is_non_semantic = backend == "hash"
        || backend == "none"
        || backend == "scripted"
        || embedder.model_ver().is_none();

    if is_non_semantic && !allow_test_embedders {
        return Err(LabError::Protocol(format!(
            "embedder probe rejects non-semantic backend '{}' outside unit tests (requires EmbeddingGemma 2 or configured semantic embedder)",
            embedder.backend_name()
        )));
    }

    match embedder.embed(&EmbedInput::query("semantic probe readiness check")) {
        Ok(Some(_)) => Ok(EmbedderReadiness::Ready),
        Ok(None) => {
            if allow_test_embedders {
                Ok(EmbedderReadiness::Unconfigured(
                    "embedder_unconfigured".to_owned(),
                ))
            } else {
                Err(LabError::Protocol(
                    "embedder returned None for semantic probe".to_owned(),
                ))
            }
        }
        Err(EmbedError::MissingModelPath { .. } | EmbedError::ModelPathNotFound { .. }) => Ok(
            EmbedderReadiness::Unconfigured("embedder_unconfigured".to_owned()),
        ),
        Err(other) => Err(LabError::Protocol(format!(
            "semantic embedder check failed: {other}"
        ))),
    }
}

/// Outcome of an [`EmbedderProbe`] execution.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum EmbedderProbeOutcome {
    Scored {
        dataset_id: String,
        probe_version: String,
        embed_model_ver: String,
        dim_d: u32,
        metrics: Vec<ScorerMetric>,
        item_count: usize,
        chosen_item_ids: Vec<String>,
    },
    NotScored {
        dataset_id: String,
        probe_version: String,
        reason: String,
    },
}

/// Component probe trait wired directly to `monoagens_reason::Embedder` (`multilexnorm`, `indocollex`).
pub trait EmbedderProbe: Send + Sync {
    fn dataset_id(&self) -> &str;
    fn probe_version(&self) -> &str;
    fn run_probe(
        &self,
        dataset_dir: &Path,
        embedder: Option<&dyn Embedder>,
        allow_test_embedders: bool,
        target_n: Option<usize>,
        seed: u64,
    ) -> Result<EmbedderProbeOutcome, LabError>;
}

/// Seeded stratified item-sampling rule (`R-54`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SamplingRule {
    pub rule_name: String,
    pub seed: u64,
    pub target_n: Option<usize>,
    /// When true, samples whole `cluster_id` units rather than splitting items inside a cluster.
    pub sample_by_cluster: bool,
}

/// Recorded result of [`sample_items_stratified`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SamplingResult {
    pub rule_name: String,
    pub seed: u64,
    pub total_available: usize,
    pub chosen_item_ids: Vec<String>,
}

/// Deterministically samples `items` under `rule`, returning identical item ids across arms for
/// the same `seed` and `items`.
#[must_use]
pub fn sample_items_stratified(items: &[ExternalItem], rule: &SamplingRule) -> SamplingResult {
    let total = items.len();
    let Some(target_n) = rule.target_n else {
        return SamplingResult {
            rule_name: rule.rule_name.clone(),
            seed: rule.seed,
            total_available: total,
            chosen_item_ids: items.iter().map(|i| i.item_id.clone()).collect(),
        };
    };
    if target_n >= total {
        return SamplingResult {
            rule_name: rule.rule_name.clone(),
            seed: rule.seed,
            total_available: total,
            chosen_item_ids: items.iter().map(|i| i.item_id.clone()).collect(),
        };
    }

    let chosen_indices = if rule.sample_by_cluster {
        sample_clustered_indices(items, target_n, rule.seed)
    } else {
        sample_proportional_strata_indices(items, target_n, rule.seed)
    };

    let chosen_item_ids = chosen_indices
        .into_iter()
        .map(|idx| items[idx].item_id.clone())
        .collect();

    SamplingResult {
        rule_name: rule.rule_name.clone(),
        seed: rule.seed,
        total_available: total,
        chosen_item_ids,
    }
}

fn sample_proportional_strata_indices(
    items: &[ExternalItem],
    target_n: usize,
    seed: u64,
) -> Vec<usize> {
    let mut by_stratum: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (idx, item) in items.iter().enumerate() {
        by_stratum
            .entry(item.strata_key.as_str())
            .or_default()
            .push(idx);
    }

    let mut rng = SplitMix64::new(seed);
    for idxs in by_stratum.values_mut() {
        rng.shuffle(idxs);
    }

    let total = items.len();
    let mut chosen = Vec::with_capacity(target_n);
    let mut remainders: Vec<(&str, usize, usize)> = Vec::new();
    let mut allocated = 0usize;

    for (stratum, idxs) in &by_stratum {
        let num = idxs.len() * target_n;
        let q = num / total;
        let rem = num % total;
        let take = q.min(idxs.len());
        chosen.extend_from_slice(&idxs[..take]);
        allocated += take;
        remainders.push((stratum, take, rem));
    }

    remainders.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| a.0.cmp(b.0)));
    for (stratum, already_taken, _) in remainders {
        if allocated >= target_n {
            break;
        }
        let idxs = &by_stratum[stratum];
        if already_taken < idxs.len() {
            chosen.push(idxs[already_taken]);
            allocated += 1;
        }
    }

    chosen.sort_unstable();
    chosen
}

fn sample_clustered_indices(items: &[ExternalItem], target_n: usize, seed: u64) -> Vec<usize> {
    let mut by_cluster: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (idx, item) in items.iter().enumerate() {
        by_cluster
            .entry(item.cluster_id.as_str())
            .or_default()
            .push(idx);
    }
    let mut clusters: Vec<(&str, Vec<usize>)> = by_cluster.into_iter().collect();
    let mut rng = SplitMix64::new(seed);
    rng.shuffle(&mut clusters);

    let mut chosen = Vec::new();
    for (_cid, idxs) in clusters {
        if !chosen.is_empty() && chosen.len() + idxs.len() > target_n {
            break;
        }
        chosen.extend(idxs);
        if chosen.len() >= target_n {
            break;
        }
    }
    chosen.sort_unstable();
    chosen
}

/// Cluster-bootstrap 95% confidence interval (`R-54`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClusterBootstrapCi {
    pub point_estimate: f64,
    pub ci95_low: f64,
    pub ci95_high: f64,
    pub n_items: usize,
    pub n_clusters: usize,
}

/// Computes a deterministic cluster-bootstrap 95% confidence interval over `(cluster_id, value)`.
#[must_use]
pub fn cluster_bootstrap_ci(
    observations: &[(String, f64)],
    seed: u64,
    n_resamples: usize,
) -> Option<ClusterBootstrapCi> {
    if observations.is_empty() {
        return None;
    }
    let mut by_cluster: BTreeMap<&str, Vec<f64>> = BTreeMap::new();
    let mut total_sum = 0.0_f64;
    for (cid, val) in observations {
        by_cluster.entry(cid.as_str()).or_default().push(*val);
        total_sum += *val;
    }
    let point_estimate = total_sum / observations.len() as f64;
    let clusters: Vec<Vec<f64>> = by_cluster.into_values().collect();
    let k = clusters.len();
    if k <= 1 || n_resamples == 0 {
        return Some(ClusterBootstrapCi {
            point_estimate,
            ci95_low: point_estimate,
            ci95_high: point_estimate,
            n_items: observations.len(),
            n_clusters: k,
        });
    }

    let mut rng = SplitMix64::new(seed ^ 0x9E37_79B9_7F4A_7C15);
    let mut means = Vec::with_capacity(n_resamples);
    for _ in 0..n_resamples {
        let mut sum = 0.0_f64;
        let mut count = 0usize;
        for _ in 0..k {
            let pick = (rng.next_u64() as usize) % k;
            for &v in &clusters[pick] {
                sum += v;
                count += 1;
            }
        }
        means.push(if count == 0 {
            point_estimate
        } else {
            sum / count as f64
        });
    }
    means.sort_by(f64::total_cmp);
    let lo_idx = ((0.025 * (n_resamples as f64 - 1.0)).floor() as usize).min(n_resamples - 1);
    let hi_idx = ((0.975 * (n_resamples as f64 - 1.0)).ceil() as usize).min(n_resamples - 1);

    Some(ClusterBootstrapCi {
        point_estimate,
        ci95_low: means[lo_idx],
        ci95_high: means[hi_idx],
        n_items: observations.len(),
        n_clusters: k,
    })
}

/// Deterministic seeded PRNG (`SplitMix64`) for reproducible dataset sampling and bootstrap CIs.
#[derive(Debug, Clone)]
pub struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    #[must_use]
    pub fn new(seed: u64) -> Self {
        Self {
            state: seed.wrapping_add(0x9E37_79B9_7F4A_7C15),
        }
    }

    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    pub fn shuffle<T>(&mut self, slice: &mut [T]) {
        if slice.len() <= 1 {
            return;
        }
        for i in (1..slice.len()).rev() {
            let j = (self.next_u64() as usize) % (i + 1);
            slice.swap(i, j);
        }
    }
}

/// Convenience helper for building synthetic/dataset percepts in converters.
#[must_use]
pub fn make_text_percept(
    actor: impl Into<String>,
    place: impl Into<String>,
    audience: impl IntoIterator<Item = ActorId>,
    timestamp_ms: u64,
    text: impl AsRef<str>,
) -> Percept {
    Percept::minimal(
        ActorId::new(actor),
        PlaceId::new(place),
        Audience::new(audience),
        Time::from_millis(timestamp_ms),
        OpaquePayload::from_bytes(text.as_ref().as_bytes().to_vec()),
    )
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use monoagens_protocol::{CapabilityFlags, CapabilitySchemaHash, PlaceId};
    use monoagens_reason::model::{ModelCallFailure, ModelCallSuccess, ModelRequest};

    use super::*;

    struct CountingProvider {
        inner: ScriptedModelProvider,
        calls: AtomicUsize,
    }

    impl CountingProvider {
        fn from_replies(replies: impl IntoIterator<Item = ScriptedReply>) -> Self {
            Self {
                inner: ScriptedModelProvider::from_replies(replies),
                calls: AtomicUsize::new(0),
            }
        }

        fn call_count(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    impl ModelProvider for CountingProvider {
        fn provider_name(&self) -> &str {
            self.inner.provider_name()
        }

        fn invoke(&self, request: &ModelRequest) -> Result<ModelCallSuccess, ModelCallFailure> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.inner.invoke(request)
        }
    }

    fn speak_probe_item() -> ExternalItem {
        let soul = ActorId::new("soul");
        let proctor = ActorId::new("lab_proctor");
        let place = PlaceId::new("room:test");
        let aud = Audience::new([proctor, soul]);
        let cap = CapabilityDescriptor {
            id: CapabilityId::new("chat.speak"),
            capability_schema_hash: CapabilitySchemaHash::new("h1"),
            places: vec![place.clone()],
            flags: CapabilityFlags {
                communicative: true,
                identity_changing: false,
                irreversible: false,
            },
            typical_latency_ms: 50,
            description: "Speak in chat".to_owned(),
            sensed_effect: None,
        };
        ExternalItem {
            item_id: "eof_retry_item".to_owned(),
            cluster_id: "eof_retry_item".to_owned(),
            strata_key: "test".to_owned(),
            history: Vec::new(),
            checkpoint_group: None,
            probes: vec![ExternalProbe {
                probe_id: "p0".to_owned(),
                percept: make_text_percept(
                    "lab_proctor",
                    place.as_str(),
                    aud.members.iter().cloned(),
                    1_700_000_000_000,
                    "Please reply.",
                ),
                isolated_checkpoint: true,
                probe_scope: VisibleScope::new(aud, Some(place)),
                expected_capability: None,
            }],
            history_mode: HistoryMode::StaticSilent,
            capabilities: vec![cap],
        }
    }

    #[test]
    fn truncated_json_eof_retries_once_and_recovers() {
        let item = speak_probe_item();
        let valid_reply = serde_json::json!({
            "acts": [{
                "act": "act",
                "capability_id": "chat.speak",
                "places": ["room:test"],
                "people": ["lab_proctor"],
                "payload": b"Recovered answer.".to_vec(),
                "motivation": 5,
                "confidence_0_20": 18,
                "used_ids": []
            }]
        })
        .to_string();
        let provider = CountingProvider::from_replies([
            ScriptedReply::failure(
                ProviderFailureClass::InvalidOutput,
                "json parse error: EOF while parsing a value at line 1 column 128",
            ),
            ScriptedReply::text(valid_reply),
        ]);
        let mut guard = CostGuard::with_max_cost_usd(1.0);
        let exec = execute_external_item(
            &item,
            &CoreConfig::default(),
            1,
            &provider,
            "mock-model",
            true,
            &mut guard,
        )
        .unwrap();

        assert_eq!(provider.call_count(), 2);
        assert!(exec.failed_items.is_empty(), "{:?}", exec.failed_items);
        assert_eq!(exec.predictions.len(), 1);
        assert!(exec.predictions[0].spoke);
        assert_eq!(
            exec.predictions[0].response_text.as_deref(),
            Some("Recovered answer.")
        );
    }

    #[test]
    fn truncated_json_eof_does_not_retry_more_than_once_or_when_budget_exhausted() {
        let item = speak_probe_item();

        // Exhausted budget in drive_model_effects -> no retry (1 call only).
        let exhausted_provider = CountingProvider::from_replies([
            ScriptedReply::failure(
                ProviderFailureClass::InvalidOutput,
                "EOF while parsing an object at line 1 column 10",
            ),
            ScriptedReply::text(SILENT_THINK_JSON),
        ]);
        let mut core = Core::open(MemStore::new(), CoreConfig::default(), None).unwrap();
        let mut exhausted_guard = CostGuard::with_max_cost_usd(0.0);
        let mut purpose_costs = BTreeMap::new();
        let mut transcript_events = Vec::new();
        let mut failed_items = Vec::new();
        let mut actions_dispatched = 0usize;
        let mut model_latencies_ms = Vec::new();
        let mut ctx = DriveEffectsContext {
            phase: "probe",
            turn_or_probe_id: "p0",
            item_id: &item.item_id,
            probe_id: "p0",
            is_silent_history: false,
            session_id: "ses_test",
            attribution: None,
            request_timeout_ms: None,
            max_output_tokens: None,
            transcript_events: &mut transcript_events,
            failed_items: &mut failed_items,
            actions_dispatched: &mut actions_dispatched,
            model_latencies_ms: &mut model_latencies_ms,
        };
        let _ = drive_model_effects(
            &mut core,
            vec![Effect::Model {
                purpose: ModelPurpose::Reason,
                plan: Default::default(),
            }],
            Time::from_millis(1_700_000_000_000),
            &mut exhausted_guard,
            &exhausted_provider,
            "mock-model",
            &mut purpose_costs,
            &mut ctx,
        )
        .unwrap();
        assert_eq!(exhausted_provider.call_count(), 1);
        assert_eq!(failed_items.len(), 1);

        // Two consecutive EOF failures -> retries at most once (2 calls total), then fails wake.
        let double_fail_provider = CountingProvider::from_replies([
            ScriptedReply::failure(
                ProviderFailureClass::InvalidOutput,
                "EOF while parsing a string at line 1 column 42",
            ),
            ScriptedReply::failure(
                ProviderFailureClass::InvalidOutput,
                "EOF while parsing a string at line 1 column 55",
            ),
            ScriptedReply::text(SILENT_THINK_JSON),
        ]);
        let mut guard = CostGuard::with_max_cost_usd(1.0);
        let exec_double = execute_external_item(
            &item,
            &CoreConfig::default(),
            1,
            &double_fail_provider,
            "mock-model",
            true,
            &mut guard,
        )
        .unwrap();
        assert_eq!(double_fail_provider.call_count(), 2);
        assert_eq!(exec_double.failed_items.len(), 1);
    }
}
