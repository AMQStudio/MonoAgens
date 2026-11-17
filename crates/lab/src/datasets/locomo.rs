//! `locomo` (`snap-research/locomo`) converter & official `evaluation.py`-aligned scorer.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use monoagens_protocol::{ActorId, Audience, PlaceId, VisibleScope};
use serde::Deserialize;

use crate::LabError;
use crate::datasets::framework::{
    ConvertedDataset, Converter, ExternalItem, ExternalPrediction, ExternalProbe, GoldBook,
    GoldEntry, GoldEvidenceRef, HistoryMode, JudgeExecutionConfig, Scorer, ScorerReport,
    make_text_percept,
};
use crate::datasets::manifest::DatasetManifestEntry;
use crate::datasets::scoring_utils::{metric_with_cluster_ci, token_f1};

pub struct LocomoConverter;

#[derive(Deserialize)]
struct RawLocomoSample {
    #[serde(default)]
    sample_id: Option<String>,
    conversation: serde_json::Map<String, serde_json::Value>,
    #[serde(default)]
    qa: Vec<RawLocomoQa>,
}

#[derive(Deserialize)]
struct RawLocomoTurn {
    speaker: String,
    #[serde(default)]
    dia_id: Option<String>,
    text: String,
    #[serde(default)]
    blip_caption: Option<String>,
}

#[derive(Deserialize)]
struct RawLocomoQa {
    question: String,
    #[serde(default)]
    answer: Option<serde_json::Value>,
    #[serde(default)]
    adversarial_answer: Option<String>,
    category: u8,
    #[serde(default)]
    evidence: Vec<String>,
}

impl Converter for LocomoConverter {
    fn dataset_id(&self) -> &str {
        "locomo"
    }

    fn converter_version(&self) -> &str {
        "1.0.0"
    }

    fn convert(
        &self,
        dataset_dir: &Path,
        manifest_entry: &DatasetManifestEntry,
    ) -> Result<ConvertedDataset, LabError> {
        let raw = fs::read_to_string(dataset_dir.join("data/locomo10.json"))?;
        let samples: Vec<RawLocomoSample> = serde_json::from_str(&raw)
            .map_err(|e| LabError::Serialization(format!("locomo10.json parse error: {e}")))?;

        let soul = ActorId::new("soul");
        let mut items = Vec::with_capacity(samples.len());
        let mut gold_entries = Vec::new();

        for (conv_idx, sample) in samples.into_iter().enumerate() {
            let conv_id = sample
                .sample_id
                .unwrap_or_else(|| format!("conv_{conv_idx}"));
            let place = PlaceId::new(format!("locomo_conv_{conv_idx}"));
            let speaker_a_str = sample
                .conversation
                .get("speaker_a")
                .and_then(|v| v.as_str())
                .unwrap_or("speaker_a")
                .to_owned();
            let speaker_b_str = sample
                .conversation
                .get("speaker_b")
                .and_then(|v| v.as_str())
                .unwrap_or("speaker_b")
                .to_owned();

            let speaker_a = ActorId::new(&speaker_a_str);
            let speaker_b = ActorId::new(&speaker_b_str);
            let hist_aud = Audience::new([speaker_a.clone(), speaker_b, soul.clone()]);
            let probe_aud = Audience::new([speaker_a.clone(), soul.clone()]);

            // Collect ordered session_1, session_2, ... keys deterministically.
            let mut session_nums: Vec<usize> = sample
                .conversation
                .keys()
                .filter_map(|k| {
                    k.strip_prefix("session_")
                        .filter(|rest| !rest.contains('_'))
                        .and_then(|n| n.parse::<usize>().ok())
                })
                .collect();
            session_nums.sort_unstable();

            let mut history = Vec::new();
            let mut dia_to_turn_idx: BTreeMap<String, usize> = BTreeMap::new();
            let base_ms: u64 = 1_700_000_000_000;

            for (s_pos, s_num) in session_nums.iter().enumerate() {
                let key = format!("session_{s_num}");
                let Some(turns_val) = sample.conversation.get(&key) else {
                    continue;
                };
                let turns: Vec<RawLocomoTurn> =
                    serde_json::from_value(turns_val.clone()).unwrap_or_default();
                let session_base_ms = base_ms + (s_pos as u64) * 86_400_000;

                for (t_idx, turn) in turns.into_iter().enumerate() {
                    let global_turn = history.len();
                    if let Some(dia_id) = turn.dia_id {
                        dia_to_turn_idx.insert(dia_id, global_turn);
                    }
                    let payload_text = match turn.blip_caption {
                        Some(cap) if !cap.trim().is_empty() => {
                            format!("{} [image: {}]", turn.text, cap)
                        }
                        _ => turn.text,
                    };
                    history.push(make_text_percept(
                        &turn.speaker,
                        place.as_str(),
                        hist_aud.members.iter().cloned(),
                        session_base_ms + (t_idx as u64) * 1_000,
                        payload_text,
                    ));
                }
            }

            let probe_base_ms = base_ms + (session_nums.len() as u64 + 1) * 86_400_000;
            let mut probes = Vec::with_capacity(sample.qa.len());
            let mut cats_seen = BTreeSet::new();

            for (q_idx, qa) in sample.qa.into_iter().enumerate() {
                let probe_id = format!("qa_{q_idx}");
                cats_seen.insert(qa.category);
                let ts = probe_base_ms + (q_idx as u64) * 60_000;
                probes.push(ExternalProbe {
                    probe_id: probe_id.clone(),
                    percept: make_text_percept(
                        &speaker_a_str,
                        place.as_str(),
                        probe_aud.members.iter().cloned(),
                        ts,
                        &qa.question,
                    ),
                    isolated_checkpoint: true,
                    probe_scope: VisibleScope::new(probe_aud.clone(), Some(place.clone())),
                    expected_capability: None,
                });

                let gold_str = qa
                    .answer
                    .as_ref()
                    .map(|v| match v {
                        serde_json::Value::String(s) => s.clone(),
                        other => other.to_string(),
                    })
                    .or(qa.adversarial_answer)
                    .unwrap_or_default();

                let gold_evidence = qa
                    .evidence
                    .iter()
                    .filter_map(|eid| dia_to_turn_idx.get(eid).copied())
                    .map(|ti| GoldEvidenceRef {
                        turn_index: ti,
                        audience: hist_aud.clone(),
                        place: place.clone(),
                    })
                    .collect();

                let mut metadata = BTreeMap::new();
                metadata.insert("category".to_owned(), qa.category.to_string());

                gold_entries.push(GoldEntry {
                    item_id: conv_id.clone(),
                    probe_id,
                    cluster_id: conv_id.clone(),
                    gold_answer: gold_str.clone(),
                    accepted_answers: vec![gold_str],
                    gold_evidence,
                    privacy_conflict: false,
                    category: Some(format!("cat_{}", qa.category)),
                    metadata,
                });
            }

            items.push(ExternalItem {
                item_id: conv_id.clone(),
                cluster_id: conv_id,
                strata_key: format!("cats_{}", cats_seen.len()),
                history,
                checkpoint_group: Some(format!("locomo_conv_{conv_idx}")),
                probes,
                history_mode: HistoryMode::StaticSilent,
                capabilities: Vec::new(),
            });
        }

        Ok(ConvertedDataset {
            dataset_id: "locomo".to_owned(),
            pinned_revision: manifest_entry
                .sources
                .first()
                .map_or_else(|| "pending_pin".to_owned(), |s| s.revision.clone()),
            converter_version: self.converter_version().to_owned(),
            items,
            gold_book: GoldBook {
                dataset_id: "locomo".to_owned(),
                entries: gold_entries,
            },
        })
    }
}

pub struct LocomoScorer;

impl Scorer for LocomoScorer {
    fn dataset_id(&self) -> &str {
        "locomo"
    }

    fn scorer_version(&self) -> &str {
        "1.0.0"
    }

    fn score(
        &self,
        predictions: &[ExternalPrediction],
        gold: &GoldBook,
        _judge: &JudgeExecutionConfig<'_>,
        seed: u64,
    ) -> Result<ScorerReport, LabError> {
        let mut qa_obs = Vec::new();
        let mut cat5_obs = Vec::new();

        for pred in predictions {
            let entry = gold.find(&pred.item_id, &pred.probe_id).ok_or_else(|| {
                LabError::Protocol(format!(
                    "locomo missing gold entry for {}:{}",
                    pred.item_id, pred.probe_id
                ))
            })?;
            let cat: u8 = entry
                .metadata
                .get("category")
                .and_then(|s| s.parse().ok())
                .unwrap_or(4);
            let resp = pred.response_text.as_deref().unwrap_or("");
            if cat == 5 {
                let lower = resp.to_ascii_lowercase();
                let refused =
                    lower.contains("no information available") || lower.contains("not mentioned");
                cat5_obs.push((pred.cluster_id.clone(), if refused { 1.0 } else { 0.0 }));
            } else if cat == 1 {
                // Multi-hop: comma-separated sub-answers averaged per evaluation.py.
                let gold_parts: Vec<&str> = entry
                    .gold_answer
                    .split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .collect();
                let pred_parts: Vec<&str> = resp
                    .split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .collect();
                let score = if gold_parts.is_empty() {
                    token_f1(resp, &entry.gold_answer, true)
                } else {
                    let sum: f64 = gold_parts
                        .iter()
                        .map(|gp| {
                            pred_parts
                                .iter()
                                .map(|pp| token_f1(pp, gp, true))
                                .fold(0.0_f64, f64::max)
                        })
                        .sum();
                    sum / gold_parts.len() as f64
                };
                qa_obs.push((pred.cluster_id.clone(), score));
            } else {
                qa_obs.push((
                    pred.cluster_id.clone(),
                    token_f1(resp, &entry.gold_answer, true),
                ));
            }
        }

        Ok(ScorerReport {
            dataset_id: "locomo".to_owned(),
            scorer_version: self.scorer_version().to_owned(),
            metrics: vec![
                metric_with_cluster_ci("qa_f1", &qa_obs, seed),
                metric_with_cluster_ci("cat5_adversarial_acc", &cat5_obs, seed),
            ],
            judge_model: None,
            judge_prompt_version: None,
        })
    }
}
