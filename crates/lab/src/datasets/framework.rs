//! Evaluation framework for external datasets: `Converter`/`Scorer` traits with compile-time
//! `GoldBook` isolation, `VisibleScope` evidence-visibility verification, Lab-side
//! `ScriptedModelProvider` silent-`think` history loading, `reason::EmbedderProbe`, converted cache,
//! seeded stratified sampling, and cluster-bootstrap confidence intervals (`R-52`–`R-54`).

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use monoagens_core::{Core, Effect, Input, MemStore, ModelPurpose, Prior, Store, Switches};
use monoagens_protocol::{
    ActorId, Audience, CURRENT_PROTOCOL_VERSION, CapabilityDescriptor, CapabilityId, OpaquePayload,
    Percept, PerceptEmitParams, PlaceId, Time, VisibleScope, Waist2Headers, Waist2Method,
    Waist2Request,
};
use monoagens_reason::embed::{EmbedError, EmbedInput, Embedder};
use monoagens_reason::model::{
    ModelOutput, ModelProvider, ModelRequest, ScriptedModelProvider, ScriptedReply,
};
use serde::{Deserialize, Serialize};

use crate::LabError;
use crate::datasets::manifest::{DatasetManifestEntry, DatasetsManifest};
use crate::datasets::ops::{CONVERTER_CACHE_FILE, ConvertedCacheMetadata};
use crate::guard::CostGuard;
use crate::record::{MetricScoreInterval, PurposeCost};
use crate::scenario::ensure_not_sealed_path;

/// Silent `ThoughtBatch` JSON emitted by Lab's `ScriptedModelProvider` during static history loading.
pub const SILENT_THINK_JSON: &str = r#"{"acts":[{"act":"think","thought":"silent history ingestion","motivation":1,"confidence_0_20":10,"used_ids":[]}]}"#;

/// Controls how `history` turns are processed through `Core::step`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HistoryMode {
    /// Static memory/ToM histories: Core runs its unmodified loop while Lab supplies a
    /// `ScriptedModelProvider` returning silent `think` (`$0` API cost, logged as `scripted_silent`).
    StaticSilent,
    /// Interactive/timing benchmarks: decision turns use the active probe model provider.
    Interactive,
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

/// Optional judge and local dataset/scorer-model configuration passed to [`Scorer::score`].
#[derive(Clone, Default)]
pub struct JudgeExecutionConfig<'a> {
    pub live: bool,
    pub judge_model: Option<&'a str>,
    pub judge_prompt_version: Option<&'a str>,
    pub provider: Option<&'a dyn ModelProvider>,
    pub datasets_dir: Option<&'a Path>,
    pub manifest: Option<&'a DatasetsManifest>,
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
    let tmp_payload = dataset_dir.join(".monoagens_converted_payload.json.tmp");
    let tmp_meta = dataset_dir.join(".monoagens_converted_cache.json.tmp");
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

/// Execution outcome of running an [`ExternalItem`] through Core's unmodified loop.
#[derive(Debug, Clone)]
pub struct ExternalItemExecution {
    pub predictions: Vec<ExternalPrediction>,
    pub history_reasoning_mode: String,
    pub total_history_gate_wakes: u64,
    pub cost_by_purpose: Vec<PurposeCost>,
}

/// Executes one [`ExternalItem`] against Core's unmodified `Core::open` + `Core::step` loop.
///
/// - During static history ingestion (`HistoryMode::StaticSilent` unless `live_full_history` is true),
///   Lab supplies a `ScriptedModelProvider` returning silent `think` (`$0` API cost) and counts
///   Gate wakes without touching `crates/core`.
/// - Captures a post-ingestion `SnapshotCheckpoint` and clones it for each isolated probe.
#[allow(clippy::too_many_arguments)]
pub fn execute_external_item(
    item: &ExternalItem,
    prior: &Prior,
    switches: &Switches,
    seed: u64,
    probe_provider: &dyn ModelProvider,
    probe_model_name: &str,
    live_full_history: bool,
    guard: &mut CostGuard,
) -> Result<ExternalItemExecution, LabError> {
    let mut core = Core::open(MemStore::new(), prior.clone(), switches.clone(), seed)?;
    let silent_provider = {
        let mut p = ScriptedModelProvider::new();
        p.set_fallback(ScriptedReply::text(SILENT_THINK_JSON));
        p
    };

    let use_silent_history = item.history_mode == HistoryMode::StaticSilent && !live_full_history;
    let history_mode_label = if use_silent_history {
        "scripted_silent"
    } else if item.history_mode == HistoryMode::StaticSilent {
        "live_full_history"
    } else {
        "interactive"
    };

    let mut history_gate_wakes = 0_u64;
    let mut purpose_costs: BTreeMap<u8, PurposeCost> = BTreeMap::new();
    let mut last_time = Time::from_millis(0);

    for (idx, percept) in item.history.iter().enumerate() {
        let mut p = percept.clone();
        p.protocol_version = CURRENT_PROTOCOL_VERSION;
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
        let effects = core.step(Input::Percept(Box::new(parsed_percept)), last_time, guard)?;
        let history_provider: &dyn ModelProvider = if use_silent_history {
            &silent_provider
        } else {
            probe_provider
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
            use_silent_history,
        )?;
        history_gate_wakes = history_gate_wakes.saturating_add(wakes);
    }

    let checkpoint = core.take_snapshot(format!("post-ingestion-{}", item.item_id))?;
    let base_events = core.store().read_after_seq(None, usize::MAX)?;
    let mut predictions = Vec::with_capacity(item.probes.len());

    for (p_idx, probe) in item.probes.iter().enumerate() {
        let mut cloned_store = MemStore::new();
        for rec in &base_events {
            cloned_store.append(rec.clone())?;
        }
        cloned_store.snapshot(checkpoint.clone())?;
        let mut probe_core = Core::open(cloned_store, prior.clone(), switches.clone(), seed)?;

        let mut p = probe.percept.clone();
        p.protocol_version = CURRENT_PROTOCOL_VERSION;
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

        let (probe_wakes, tok_in, tok_out, reply_text) = drive_model_effects(
            &mut probe_core,
            effects,
            probe_ts,
            guard,
            probe_provider,
            probe_model_name,
            &mut purpose_costs,
            false,
        )?;

        predictions.push(ExternalPrediction {
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
        });
    }

    Ok(ExternalItemExecution {
        predictions,
        history_reasoning_mode: history_mode_label.to_owned(),
        total_history_gate_wakes: history_gate_wakes,
        cost_by_purpose: purpose_costs.into_values().collect(),
    })
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
    is_silent_history: bool,
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
                    if !action.payload.is_empty() {
                        spoken_text =
                            Some(String::from_utf8_lossy(action.payload.as_bytes()).into_owned());
                    }
                }
                Effect::Model { purpose, plan } => {
                    wakes = wakes.saturating_add(1);
                    let request_id = plan
                        .trigger_event_id
                        .unwrap_or(monoagens_protocol::EventId::new(0));
                    let prompt = serde_json::to_string(&plan.sections)
                        .unwrap_or_else(|_| format!("items={}", plan.items.len()));
                    let req = ModelRequest::text(
                        request_id,
                        purpose,
                        model_name,
                        "MonoAgens external evaluation",
                        prompt,
                    );
                    let (inp, call_in, call_out) = match provider.invoke(&req) {
                        Ok(ok) => {
                            let (tin, tout) = if is_silent_history {
                                (0, 0)
                            } else {
                                ok.record
                                    .usage
                                    .map_or((0, 0), |u| (u.input_tokens, u.output_tokens))
                            };
                            let inp = match ok.output {
                                ModelOutput::ThoughtActs(parsed) => Input::ModelResult {
                                    request_id,
                                    acts: parsed.acts,
                                },
                                ModelOutput::Verdict(_) => Input::ModelFailure {
                                    request_id,
                                    reason: "unexpected classification verdict".to_owned(),
                                },
                            };
                            (inp, tin, tout)
                        }
                        Err(fail) => (fail.to_core_input(), 0, 0),
                    };
                    tokens_in = tokens_in.saturating_add(call_in);
                    tokens_out = tokens_out.saturating_add(call_out);
                    let call_toks = call_in.saturating_add(call_out);
                    guard.record_tokens(call_toks);
                    let call_cost_usd = if is_silent_history || call_toks == 0 {
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
