//! `dyntom` (`GAIR-NLP/DynToM`) Mode-2 `+60,000 ms`/stage converter & `dyntom_acc` MCQ scorer.

use std::collections::BTreeMap;
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
use crate::datasets::scoring_utils::{exact_match_normalized, metric_with_cluster_ci};

pub struct DynTomConverter;

/// Real upstream `DynToM.json`: `{ "<trial_id>": { "stage": {...}, "question": { "<qid>": {...} } } }`.
#[derive(Deserialize)]
struct RawDynTomTrial {
    stage: serde_json::Value,
    question: BTreeMap<String, RawDynTomQuestion>,
}

#[derive(Deserialize)]
struct RawDynTomQuestion {
    question: String,
    #[serde(rename = "true answer")]
    answer: String,
    #[serde(default)]
    options: Vec<String>,
    #[serde(rename = "question type", default)]
    question_type: String,
}

fn build_stage_text(stage: &serde_json::Value, scenario_no: usize) -> String {
    let setting = stage
        .get("social setting")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown setting");
    let characters = stage
        .get("characters information")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let sketch = stage.get("sketch");
    let relationships = sketch
        .and_then(|s| s.get("relationships among characters"))
        .map(std::string::ToString::to_string)
        .unwrap_or_default();
    let mental = sketch
        .and_then(|s| s.get("mental states analysis in every scenario"))
        .and_then(|m| {
            m.get(format!("scenario {scenario_no}"))
                .or_else(|| m.get(scenario_no.to_string()))
        });
    let state = mental
        .map(|m| {
            let get = |k: &str| m.get(k).and_then(|v| v.as_str()).unwrap_or("");
            let influence = m
                .get("influence")
                .map(std::string::ToString::to_string)
                .unwrap_or_default();
            format!(
                "belief: {}\nemotion: {}\nintention: {}\naction: {}\ninfluence: {}",
                get("belief"),
                get("emotion"),
                get("intention"),
                get("action"),
                influence
            )
        })
        .unwrap_or_default();
    format!(
        "Scenario {scenario_no} in {setting}.\nCharacters: {characters}\nRelationships: {relationships}\nMental state: {state}"
    )
}

impl Converter for DynTomConverter {
    fn dataset_id(&self) -> &str {
        "dyntom"
    }

    fn converter_version(&self) -> &str {
        "1.1.0"
    }

    fn convert(
        &self,
        dataset_dir: &Path,
        manifest_entry: &DatasetManifestEntry,
    ) -> Result<ConvertedDataset, LabError> {
        let raw = fs::read_to_string(dataset_dir.join("DynToM.json"))?;
        let trials: BTreeMap<String, RawDynTomTrial> = serde_json::from_str(&raw)
            .map_err(|e| LabError::Serialization(format!("dyntom DynToM.json: {e}")))?;

        let soul = ActorId::new("soul");
        let narrator = ActorId::new("narrator");
        let proctor = ActorId::new("lab_proctor");
        let hist_aud = Audience::new([narrator, proctor.clone(), soul.clone()]);
        let probe_aud = Audience::new([proctor, soul]);
        let base_ms: u64 = 1_700_000_000_000;

        let mut items = Vec::new();
        let mut gold_entries = Vec::new();

        for (context_id, trial) in trials {
            let place = PlaceId::new(format!("dyntom_{context_id}"));
            let scenario_count = trial
                .stage
                .get("scenario numbers")
                .and_then(serde_json::Value::as_u64)
                .map_or(5, |n| n as usize);
            let mut history = Vec::with_capacity(scenario_count);
            let mut ev_refs = Vec::with_capacity(scenario_count);
            for scenario_no in 1..=scenario_count {
                let text = build_stage_text(&trial.stage, scenario_no);
                history.push(make_text_percept(
                    "narrator",
                    place.as_str(),
                    hist_aud.members.iter().cloned(),
                    base_ms + ((scenario_no - 1) as u64) * 60_000,
                    text,
                ));
                ev_refs.push(GoldEvidenceRef {
                    turn_index: scenario_no - 1,
                    audience: hist_aud.clone(),
                    place: place.clone(),
                });
            }

            let mut probes = Vec::with_capacity(trial.question.len());
            for (q_idx, (qid, q)) in trial.question.into_iter().enumerate() {
                if q.options.is_empty() || q.answer.len() != 1 {
                    continue;
                }
                let answer_letter = q.answer.trim().to_ascii_lowercase();
                let accepted_option = q
                    .options
                    .iter()
                    .find(|o| {
                        o.trim()
                            .to_ascii_lowercase()
                            .starts_with(&format!("{answer_letter}."))
                    })
                    .map(|o| o.trim().to_owned());
                let mut accepted = vec![q.answer.trim().to_owned()];
                if let Some(opt) = accepted_option {
                    accepted.push(opt);
                }
                let mut strata = q.question_type.clone();
                if strata.is_empty() {
                    strata = "untyped".to_owned();
                }
                let prompt = format!(
                    "{}\nOptions:\n{}\nAnswer with the option letter only.",
                    q.question,
                    q.options.join("\n")
                );
                probes.push(ExternalProbe {
                    probe_id: qid.clone(),
                    percept: make_text_percept(
                        "lab_proctor",
                        place.as_str(),
                        probe_aud.members.iter().cloned(),
                        base_ms + (scenario_count as u64 + 1) * 60_000 + (q_idx as u64) * 1_000,
                        prompt,
                    ),
                    isolated_checkpoint: true,
                    probe_scope: VisibleScope::new(probe_aud.clone(), Some(place.clone())),
                    expected_capability: None,
                });

                let mut metadata = BTreeMap::new();
                metadata.insert("question_type".to_owned(), strata.clone());

                gold_entries.push(GoldEntry {
                    item_id: context_id.clone(),
                    probe_id: qid,
                    cluster_id: context_id.clone(),
                    gold_answer: q.answer.trim().to_owned(),
                    accepted_answers: accepted,
                    gold_evidence: ev_refs.clone(),
                    privacy_conflict: false,
                    category: Some(strata),
                    metadata,
                });
            }

            items.push(ExternalItem {
                item_id: context_id.clone(),
                cluster_id: context_id,
                strata_key: "dyntom_context".to_owned(),
                history,
                checkpoint_group: None,
                probes,
                history_mode: HistoryMode::StaticSilent,
                capabilities: Vec::new(),
            });
        }

        Ok(ConvertedDataset {
            dataset_id: "dyntom".to_owned(),
            pinned_revision: manifest_entry
                .sources
                .first()
                .map_or_else(|| "pending_pin".to_owned(), |s| s.revision.clone()),
            converter_version: self.converter_version().to_owned(),
            items,
            gold_book: GoldBook {
                dataset_id: "dyntom".to_owned(),
                entries: gold_entries,
            },
        })
    }
}

pub struct DynTomScorer;

impl Scorer for DynTomScorer {
    fn dataset_id(&self) -> &str {
        "dyntom"
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
        let mut obs = Vec::with_capacity(predictions.len());
        for pred in predictions {
            let entry = gold.find(&pred.item_id, &pred.probe_id).ok_or_else(|| {
                LabError::Protocol(format!(
                    "dyntom missing gold for {}:{}",
                    pred.item_id, pred.probe_id
                ))
            })?;
            let resp = pred.response_text.as_deref().unwrap_or("");
            let hit = exact_match_normalized(resp, &entry.accepted_answers);
            obs.push((pred.cluster_id.clone(), hit));
        }

        Ok(ScorerReport {
            dataset_id: "dyntom".to_owned(),
            scorer_version: self.scorer_version().to_owned(),
            metrics: vec![metric_with_cluster_ci("dyntom_acc", &obs, seed)],
            judge_model: None,
            judge_prompt_version: None,
        })
    }
}
