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

#[derive(Deserialize)]
struct RawDynTomStory {
    context_id: String,
    stages: Vec<String>,
}

#[derive(Deserialize)]
struct RawDynTomQuestion {
    qid: String,
    context_id: String,
    stage: usize,
    task_type: String,
    dimension: String,
    question: String,
    choices: Vec<String>,
    answer: String,
}

impl Converter for DynTomConverter {
    fn dataset_id(&self) -> &str {
        "dyntom"
    }

    fn converter_version(&self) -> &str {
        "1.0.0"
    }

    fn convert(
        &self,
        dataset_dir: &Path,
        manifest_entry: &DatasetManifestEntry,
    ) -> Result<ConvertedDataset, LabError> {
        let stories_raw = fs::read_to_string(dataset_dir.join("story.json"))?;
        let qs_raw = fs::read_to_string(dataset_dir.join("question_new.json"))?;
        let stories: Vec<RawDynTomStory> = serde_json::from_str(&stories_raw)
            .map_err(|e| LabError::Serialization(format!("dyntom story.json: {e}")))?;
        let questions: Vec<RawDynTomQuestion> = serde_json::from_str(&qs_raw)
            .map_err(|e| LabError::Serialization(format!("dyntom question_new.json: {e}")))?;

        let mut qs_by_ctx: BTreeMap<String, Vec<RawDynTomQuestion>> = BTreeMap::new();
        for q in questions {
            qs_by_ctx.entry(q.context_id.clone()).or_default().push(q);
        }

        let soul = ActorId::new("soul");
        let narrator = ActorId::new("narrator");
        let proctor = ActorId::new("lab_proctor");
        let hist_aud = Audience::new([narrator, proctor.clone(), soul.clone()]);
        let probe_aud = Audience::new([proctor, soul]);
        let base_ms: u64 = 1_700_000_000_000;

        let mut items = Vec::with_capacity(stories.len());
        let mut gold_entries = Vec::new();

        for story in stories {
            let place = PlaceId::new(format!("dyntom_{}", story.context_id));
            let mut history = Vec::with_capacity(story.stages.len());
            let mut ev_refs = Vec::with_capacity(story.stages.len());
            for (s_idx, text) in story.stages.iter().enumerate() {
                history.push(make_text_percept(
                    "narrator",
                    place.as_str(),
                    hist_aud.members.iter().cloned(),
                    base_ms + (s_idx as u64) * 60_000,
                    text,
                ));
                ev_refs.push(GoldEvidenceRef {
                    turn_index: s_idx,
                    audience: hist_aud.clone(),
                    place: place.clone(),
                });
            }

            let qs = qs_by_ctx.remove(&story.context_id).unwrap_or_default();
            let mut probes = Vec::with_capacity(qs.len());
            let mut strata = "U:Belief".to_owned();
            for (q_idx, q) in qs.into_iter().enumerate() {
                strata = format!("{}:{}", q.task_type, q.dimension);
                let prompt = format!("{}\nChoices: {}", q.question, q.choices.join(" | "));
                let stage_ts = base_ms + (q.stage as u64) * 60_000 + ((q_idx + 1) as u64) * 1_000;
                probes.push(ExternalProbe {
                    probe_id: q.qid.clone(),
                    percept: make_text_percept(
                        "lab_proctor",
                        place.as_str(),
                        probe_aud.members.iter().cloned(),
                        stage_ts,
                        prompt,
                    ),
                    isolated_checkpoint: true,
                    probe_scope: VisibleScope::new(probe_aud.clone(), Some(place.clone())),
                    expected_capability: None,
                });

                let max_ev = (q.stage + 1).min(ev_refs.len());
                let mut metadata = BTreeMap::new();
                metadata.insert("task_type".to_owned(), q.task_type);
                metadata.insert("dimension".to_owned(), q.dimension);

                gold_entries.push(GoldEntry {
                    item_id: story.context_id.clone(),
                    probe_id: q.qid,
                    cluster_id: story.context_id.clone(),
                    gold_answer: q.answer.clone(),
                    accepted_answers: vec![q.answer],
                    gold_evidence: ev_refs[..max_ev].to_vec(),
                    privacy_conflict: false,
                    category: Some(strata.clone()),
                    metadata,
                });
            }

            items.push(ExternalItem {
                item_id: story.context_id.clone(),
                cluster_id: story.context_id,
                strata_key: strata,
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
