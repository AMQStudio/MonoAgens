//! `memory_agent_bench` (`ai-hyz/MemoryAgentBench`) converter & scorer.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use monoagens_protocol::{ActorId, Audience, EventId, PlaceId, VisibleScope};
use serde::Deserialize;

use crate::LabError;
use crate::datasets::framework::{
    ConvertedDataset, Converter, ExternalItem, ExternalPrediction, ExternalProbe, GoldBook,
    GoldEntry, GoldEvidenceRef, HistoryMode, JudgeExecutionConfig, Scorer, ScorerReport,
    make_text_percept,
};
use crate::datasets::manifest::DatasetManifestEntry;
use crate::datasets::parquet_utils::read_parquet_rows;
use crate::datasets::scoring_utils::{
    invoke_judge_resilient, metric_with_cluster_ci, not_scored_metric, substring_exact_match,
};

pub struct MemoryAgentBenchConverter;

#[derive(Deserialize)]
struct RawMabMetadata {
    #[serde(default)]
    source: Option<String>,
}

#[derive(Deserialize)]
struct RawMabRow {
    #[serde(default)]
    row_id: Option<String>,
    #[serde(default)]
    split: Option<String>,
    #[serde(default)]
    source: Option<String>,
    context: String,
    questions: Vec<String>,
    answers: Vec<serde_json::Value>,
    #[serde(default)]
    evidence_chunk_indices: Vec<Vec<usize>>,
    #[serde(default)]
    metadata: Option<RawMabMetadata>,
}

fn flatten_mab_answers(val: serde_json::Value) -> Vec<String> {
    match val {
        serde_json::Value::String(s) => vec![s],
        serde_json::Value::Array(arr) => {
            let mut out = Vec::new();
            for v in arr {
                out.extend(flatten_mab_answers(v));
            }
            out
        }
        serde_json::Value::Object(obj) => {
            if let Some(item) = obj.get("item").or_else(|| obj.get("element")) {
                flatten_mab_answers(item.clone())
            } else if let Some(list) = obj.get("list") {
                flatten_mab_answers(list.clone())
            } else {
                Vec::new()
            }
        }
        serde_json::Value::Null => Vec::new(),
        other => vec![other.to_string()],
    }
}

fn split_context_chunks(context: &str) -> Vec<String> {
    let double_nl: Vec<String> = context
        .split("\n\n")
        .map(strip_memorize_wrapper)
        .filter(|s| !s.is_empty())
        .collect();
    if double_nl.len() > 1 || double_nl.first().is_none_or(|c| c.len() <= 4000) {
        return double_nl;
    }
    // Split oversized single-block contexts along newline boundaries to preserve 8K budget.
    let mut chunks = Vec::new();
    let mut current = String::new();
    for line in double_nl[0].lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if !current.is_empty() && current.len() + trimmed.len() + 1 > 2000 {
            chunks.push(std::mem::take(&mut current));
        }
        if !current.is_empty() {
            current.push('\n');
        }
        current.push_str(trimmed);
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

fn strip_memorize_wrapper(chunk: &str) -> String {
    chunk
        .trim_start_matches("Please memorize it and I will ask some questions:")
        .trim_start_matches("Please memorize it:")
        .trim()
        .to_owned()
}

impl Converter for MemoryAgentBenchConverter {
    fn dataset_id(&self) -> &str {
        "memory_agent_bench"
    }

    fn converter_version(&self) -> &str {
        "2.0.0"
    }

    fn convert(
        &self,
        dataset_dir: &Path,
        manifest_entry: &DatasetManifestEntry,
    ) -> Result<ConvertedDataset, LabError> {
        let mut rows: Vec<(String, RawMabRow)> = Vec::new();
        for (split_name, candidates) in [
            (
                "Accurate_Retrieval",
                [
                    "data/Accurate_Retrieval-00000-of-00001.parquet",
                    "Accurate_Retrieval-00000-of-00001.parquet",
                    "Accurate_Retrieval.parquet",
                    "Accurate_Retrieval.json",
                    "Accurate_Retrieval.jsonl",
                ],
            ),
            (
                "Conflict_Resolution",
                [
                    "data/Conflict_Resolution-00000-of-00001.parquet",
                    "Conflict_Resolution-00000-of-00001.parquet",
                    "Conflict_Resolution.parquet",
                    "Conflict_Resolution.json",
                    "Conflict_Resolution.jsonl",
                ],
            ),
        ] {
            for cand in candidates {
                let p = dataset_dir.join(cand);
                if p.is_file() {
                    if cand.ends_with(".parquet") {
                        for val in read_parquet_rows(&p)? {
                            let r: RawMabRow = serde_json::from_value(val)
                                .map_err(|e| LabError::Serialization(e.to_string()))?;
                            rows.push((split_name.to_owned(), r));
                        }
                    } else if cand.ends_with(".jsonl") {
                        let raw = fs::read_to_string(&p)?;
                        for line in raw.lines().filter(|l| !l.trim().is_empty()) {
                            let r: RawMabRow = serde_json::from_str(line)
                                .map_err(|e| LabError::Serialization(e.to_string()))?;
                            rows.push((split_name.to_owned(), r));
                        }
                    } else {
                        let raw = fs::read_to_string(&p)?;
                        let parsed: Vec<RawMabRow> = serde_json::from_str(&raw)
                            .map_err(|e| LabError::Serialization(e.to_string()))?;
                        for r in parsed {
                            rows.push((split_name.to_owned(), r));
                        }
                    }
                    break;
                }
            }
        }

        if rows.is_empty() {
            return Err(LabError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!(
                    "no Accurate_Retrieval/Conflict_Resolution Parquet/JSON/JSONL files in '{}'",
                    dataset_dir.display()
                ),
            )));
        }

        let soul = ActorId::new("soul");
        let stream_source = ActorId::new("stream_source");
        let proctor = ActorId::new("lab_proctor");
        let user = ActorId::new("user");
        let prior_assistant = ActorId::new("prior_assistant");

        let mut items = Vec::with_capacity(rows.len());
        let mut gold_entries = Vec::new();

        for (r_idx, (default_split, row)) in rows.into_iter().enumerate() {
            let split = row.split.unwrap_or(default_split);
            let source_name = row
                .source
                .or_else(|| row.metadata.and_then(|m| m.source))
                .unwrap_or_else(|| "unknown".to_owned());
            let is_lme = source_name.to_ascii_lowercase().contains("lme")
                || source_name.to_ascii_lowercase().contains("longmemeval");
            let item_id = row.row_id.unwrap_or_else(|| format!("mab_{split}_{r_idx}"));
            let place = PlaceId::new(format!("mab_{split}_{r_idx}"));

            let (hist_aud, probe_actor, probe_aud) = if is_lme {
                (
                    Audience::new([user.clone(), prior_assistant.clone(), soul.clone()]),
                    "user",
                    Audience::new([user.clone(), soul.clone()]),
                )
            } else {
                (
                    Audience::new([stream_source.clone(), proctor.clone(), soul.clone()]),
                    "lab_proctor",
                    Audience::new([proctor.clone(), soul.clone()]),
                )
            };

            let chunks = split_context_chunks(&row.context);
            let base_ms: u64 = 1_700_000_000_000;
            let mut history = Vec::with_capacity(chunks.len());
            for (c_idx, chunk) in chunks.iter().enumerate() {
                let actor = if is_lme && c_idx % 2 == 1 {
                    "prior_assistant"
                } else if is_lme {
                    "user"
                } else {
                    "stream_source"
                };
                history.push(make_text_percept(
                    actor,
                    place.as_str(),
                    hist_aud.members.iter().cloned(),
                    base_ms + (c_idx as u64) * 60_000,
                    chunk,
                ));
            }

            let probe_base = base_ms + (chunks.len() as u64 + 1) * 60_000;
            let mut probes = Vec::with_capacity(row.questions.len());
            for (q_idx, q) in row.questions.into_iter().enumerate() {
                let probe_id = format!("q_{q_idx}");
                probes.push(ExternalProbe {
                    probe_id: probe_id.clone(),
                    percept: make_text_percept(
                        probe_actor,
                        place.as_str(),
                        probe_aud.members.iter().cloned(),
                        probe_base + (q_idx as u64) * 1_000,
                        &q,
                    ),
                    isolated_checkpoint: true,
                    probe_scope: VisibleScope::new(probe_aud.clone(), Some(place.clone())),
                    expected_capability: None,
                });

                let ans_val = row
                    .answers
                    .get(q_idx)
                    .cloned()
                    .unwrap_or(serde_json::Value::Null);
                let accepted = flatten_mab_answers(ans_val);
                let gold_answer = accepted.first().cloned().unwrap_or_default();

                let ev_indices = row
                    .evidence_chunk_indices
                    .get(q_idx)
                    .cloned()
                    .unwrap_or_else(|| if history.is_empty() { vec![] } else { vec![0] });
                let gold_evidence = ev_indices
                    .into_iter()
                    .filter(|&i| i < history.len())
                    .map(|ti| GoldEvidenceRef {
                        turn_index: ti,
                        audience: hist_aud.clone(),
                        place: place.clone(),
                    })
                    .collect();

                let mut metadata = BTreeMap::new();
                metadata.insert("split".to_owned(), split.clone());
                metadata.insert("source".to_owned(), source_name.clone());
                metadata.insert("is_lme".to_owned(), is_lme.to_string());

                gold_entries.push(GoldEntry {
                    item_id: item_id.clone(),
                    probe_id,
                    cluster_id: item_id.clone(),
                    gold_answer,
                    accepted_answers: accepted,
                    gold_evidence,
                    privacy_conflict: false,
                    category: Some(split.clone()),
                    metadata,
                });
            }

            items.push(ExternalItem {
                item_id: item_id.clone(),
                cluster_id: item_id,
                strata_key: format!("{split}:{source_name}"),
                history,
                checkpoint_group: None,
                probes,
                history_mode: HistoryMode::StaticSilent,
                capabilities: Vec::new(),
            });
        }

        Ok(ConvertedDataset {
            dataset_id: "memory_agent_bench".to_owned(),
            pinned_revision: manifest_entry
                .sources
                .first()
                .map_or_else(|| "pending_pin".to_owned(), |s| s.revision.clone()),
            converter_version: self.converter_version().to_owned(),
            items,
            gold_book: GoldBook {
                dataset_id: "memory_agent_bench".to_owned(),
                entries: gold_entries,
            },
        })
    }
}

pub struct MemoryAgentBenchScorer;

impl Scorer for MemoryAgentBenchScorer {
    fn dataset_id(&self) -> &str {
        "memory_agent_bench"
    }

    fn scorer_version(&self) -> &str {
        "1.0.0"
    }

    fn score(
        &self,
        predictions: &[ExternalPrediction],
        gold: &GoldBook,
        judge: &JudgeExecutionConfig<'_>,
        seed: u64,
    ) -> Result<ScorerReport, LabError> {
        let mut ar_obs = Vec::new();
        let mut cr_obs = Vec::new();
        let mut lme_needed_offline = false;
        let mut used_judge = false;
        let judge_model = judge.judge_model.unwrap_or("gpt-4o");

        for (idx, pred) in predictions.iter().enumerate() {
            let entry = gold.find(&pred.item_id, &pred.probe_id).ok_or_else(|| {
                LabError::Protocol(format!(
                    "memory_agent_bench missing gold for {}:{}",
                    pred.item_id, pred.probe_id
                ))
            })?;
            let split = entry
                .metadata
                .get("split")
                .map(String::as_str)
                .unwrap_or("");
            let is_lme = entry.metadata.get("is_lme").map(String::as_str) == Some("true");
            let resp = pred.response_text.as_deref().unwrap_or("");

            if split == "Conflict_Resolution" {
                let hit = substring_exact_match(resp, &entry.accepted_answers);
                cr_obs.push((pred.cluster_id.clone(), hit));
                judge.record_outcome(crate::datasets::framework::ScorerItemOutcome {
                    item_id: pred.item_id.clone(),
                    probe_id: pred.probe_id.clone(),
                    cluster_id: pred.cluster_id.clone(),
                    category: entry.category.clone(),
                    metric_name: "selective_forgetting_acc".to_owned(),
                    score: Some(hit),
                    passed: Some(hit > 0.5),
                    judge_prompt: None,
                    judge_verdict: None,
                    failed: false,
                    failure_reason: None,
                });
            } else if is_lme {
                if !judge.live {
                    lme_needed_offline = true;
                } else if let Some(jp) = judge.provider {
                    used_judge = true;
                    let prompt = format!(
                        "Gold: {}\nResponse: {}\nCorrect? yes or no.",
                        entry.gold_answer, resp
                    );
                    if let Some(ok) = invoke_judge_resilient(
                        judge,
                        jp,
                        EventId::new((idx + 1) as u64),
                        judge_model,
                        "MemoryAgentBench LME(S*) judge",
                        &prompt,
                        &pred.item_id,
                        &pred.probe_id,
                        &pred.cluster_id,
                        entry,
                        "ar_acc",
                    )? {
                        ar_obs.push((pred.cluster_id.clone(), if ok { 1.0 } else { 0.0 }));
                    }
                }
            } else {
                let hit = substring_exact_match(resp, &entry.accepted_answers);
                ar_obs.push((pred.cluster_id.clone(), hit));
                judge.record_outcome(crate::datasets::framework::ScorerItemOutcome {
                    item_id: pred.item_id.clone(),
                    probe_id: pred.probe_id.clone(),
                    cluster_id: pred.cluster_id.clone(),
                    category: entry.category.clone(),
                    metric_name: "ar_acc".to_owned(),
                    score: Some(hit),
                    passed: Some(hit > 0.5),
                    judge_prompt: None,
                    judge_verdict: None,
                    failed: false,
                    failure_reason: None,
                });
            }
        }

        let ar_metric = if lme_needed_offline {
            not_scored_metric("ar_acc", "LME(S*) subset requires --live gpt-4o judge")
        } else {
            metric_with_cluster_ci("ar_acc", &ar_obs, seed)
        };
        let cr_metric = metric_with_cluster_ci("selective_forgetting_acc", &cr_obs, seed);

        Ok(ScorerReport {
            dataset_id: "memory_agent_bench".to_owned(),
            scorer_version: self.scorer_version().to_owned(),
            metrics: vec![ar_metric, cr_metric],
            judge_model: if used_judge {
                Some(judge_model.to_owned())
            } else {
                None
            },
            judge_prompt_version: if used_judge {
                Some(judge.judge_prompt_version.unwrap_or("mab-v1").to_owned())
            } else {
                None
            },
        })
    }
}
