//! `pm_bench` (`genglinliu/PMBench`) 80-step interactive Virtual Week converter & `Set-F1`/`false_alarm_rate` scorer.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use monoagens_protocol::{ActorId, Audience, PlaceId, VisibleScope};
use serde::Deserialize;

use crate::LabError;
use crate::datasets::framework::{
    ConvertedDataset, Converter, ExternalItem, ExternalPrediction, ExternalProbe, GoldBook,
    GoldEntry, GoldEvidenceRef, HistoryMode, JudgeExecutionConfig, Scorer, ScorerMetric,
    ScorerReport, make_text_percept,
};
use crate::datasets::manifest::DatasetManifestEntry;
use crate::datasets::scoring_utils::binary_prf1;

pub struct PmBenchConverter;

#[derive(Deserialize)]
struct RawPmBenchFile {
    steps: Vec<RawPmStep>,
}

#[derive(Deserialize)]
struct RawPmStep {
    step_id: usize,
    day: u64,
    hour: u64,
    minute: u64,
    narrative: String,
    menu_task_ids: Vec<String>,
    due_task_ids: Vec<String>,
    #[serde(default)]
    lure_task_ids: Vec<String>,
}

impl Converter for PmBenchConverter {
    fn dataset_id(&self) -> &str {
        "pm_bench"
    }

    fn converter_version(&self) -> &str {
        "1.0.0"
    }

    fn convert(
        &self,
        dataset_dir: &Path,
        manifest_entry: &DatasetManifestEntry,
    ) -> Result<ConvertedDataset, LabError> {
        let raw = fs::read_to_string(dataset_dir.join("data/virtual_week.json"))?;
        let parsed: RawPmBenchFile = serde_json::from_str(&raw)
            .map_err(|e| LabError::Serialization(format!("pm_bench parse error: {e}")))?;

        let soul = ActorId::new("soul");
        let env_actor = ActorId::new("env:virtual_week");
        let place = PlaceId::new("place:virtual_week_sim");
        let aud = Audience::new([env_actor, soul]);

        let mut items = Vec::with_capacity(parsed.steps.len());
        let mut gold_entries = Vec::with_capacity(parsed.steps.len());
        let mut running_history = Vec::new();
        let mut running_ev = Vec::new();

        for step in parsed.steps {
            let ts = 1_700_000_000_000
                + step.day * 86_400_000
                + step.hour * 3_600_000
                + step.minute * 60_000;
            let step_text = format!(
                "Day {} {:02}:{:02} | {} | Available task handles: [{}]",
                step.day,
                step.hour,
                step.minute,
                step.narrative,
                step.menu_task_ids.join(", ")
            );
            let item_id = format!("pm_step_{:03}", step.step_id);
            let history_snapshot = running_history.clone();
            let ev_snapshot = running_ev.clone();

            let probe = ExternalProbe {
                probe_id: "choose".to_owned(),
                percept: make_text_percept(
                    "env:virtual_week",
                    place.as_str(),
                    aud.members.iter().cloned(),
                    ts,
                    &step_text,
                ),
                isolated_checkpoint: true,
                probe_scope: VisibleScope::new(aud.clone(), Some(place.clone())),
                expected_capability: None,
            };

            let mut metadata = BTreeMap::new();
            metadata.insert("due_task_ids".to_owned(), step.due_task_ids.join(","));
            metadata.insert("lure_task_ids".to_owned(), step.lure_task_ids.join(","));

            gold_entries.push(GoldEntry {
                item_id: item_id.clone(),
                probe_id: "choose".to_owned(),
                cluster_id: "virtual_week_7d".to_owned(),
                gold_answer: step.due_task_ids.join(","),
                accepted_answers: step.due_task_ids,
                gold_evidence: ev_snapshot,
                privacy_conflict: false,
                category: Some("virtual_week".to_owned()),
                metadata,
            });

            items.push(ExternalItem {
                item_id,
                cluster_id: "virtual_week_7d".to_owned(),
                strata_key: format!("day_{}", step.day),
                history: history_snapshot,
                checkpoint_group: None,
                probes: vec![probe],
                history_mode: HistoryMode::Interactive,
                capabilities: Vec::new(),
            });

            let turn_idx = running_history.len();
            running_history.push(make_text_percept(
                "env:virtual_week",
                place.as_str(),
                aud.members.iter().cloned(),
                ts,
                step_text,
            ));
            running_ev.push(GoldEvidenceRef {
                turn_index: turn_idx,
                audience: aud.clone(),
                place: place.clone(),
            });
        }

        Ok(ConvertedDataset {
            dataset_id: "pm_bench".to_owned(),
            pinned_revision: manifest_entry
                .sources
                .first()
                .map_or_else(|| "pending_pin".to_owned(), |s| s.revision.clone()),
            converter_version: self.converter_version().to_owned(),
            items,
            gold_book: GoldBook {
                dataset_id: "pm_bench".to_owned(),
                entries: gold_entries,
            },
        })
    }
}

pub struct PmBenchScorer;

impl Scorer for PmBenchScorer {
    fn dataset_id(&self) -> &str {
        "pm_bench"
    }

    fn scorer_version(&self) -> &str {
        "1.0.0"
    }

    fn score(
        &self,
        predictions: &[ExternalPrediction],
        gold: &GoldBook,
        _judge: &JudgeExecutionConfig<'_>,
        _seed: u64,
    ) -> Result<ScorerReport, LabError> {
        let mut tp = 0usize;
        let mut fp = 0usize;
        let mut fn_ = 0usize;

        for pred in predictions {
            let entry = gold.find(&pred.item_id, &pred.probe_id).ok_or_else(|| {
                LabError::Protocol(format!("pm_bench missing gold for {}", pred.item_id))
            })?;
            let due: BTreeSet<&str> = entry
                .accepted_answers
                .iter()
                .map(String::as_str)
                .filter(|s| !s.is_empty())
                .collect();
            let resp = pred.response_text.as_deref().unwrap_or("");
            let chosen: BTreeSet<&str> = resp
                .split(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != '-')
                .filter(|s| !s.is_empty() && s.starts_with("task"))
                .collect();

            for c in &chosen {
                if due.contains(c) {
                    tp += 1;
                } else {
                    fp += 1;
                }
            }
            for d in &due {
                if !chosen.contains(d) {
                    fn_ += 1;
                }
            }
        }

        let (_p, _r, set_f1) = binary_prf1(tp, fp, fn_);
        let n_steps = predictions.len().max(1) as f64;
        let false_alarm_rate = fp as f64 / n_steps;

        Ok(ScorerReport {
            dataset_id: "pm_bench".to_owned(),
            scorer_version: self.scorer_version().to_owned(),
            metrics: vec![
                ScorerMetric {
                    name: "set_f1".to_owned(),
                    value: Some(set_f1),
                    ci95_low: None,
                    ci95_high: None,
                    n_clusters: Some(1),
                    not_scored_reason: None,
                },
                ScorerMetric {
                    name: "false_alarm_rate".to_owned(),
                    value: Some(false_alarm_rate),
                    ci95_low: None,
                    ci95_high: None,
                    n_clusters: Some(1),
                    not_scored_reason: None,
                },
            ],
            judge_model: None,
            judge_prompt_version: None,
        })
    }
}
