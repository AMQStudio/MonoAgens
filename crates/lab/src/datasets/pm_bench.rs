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

/// Real upstream `data/synthetic_week_v9.json`.
#[derive(Deserialize)]
struct RawPmBenchFile {
    #[allow(dead_code)]
    #[serde(default)]
    scenario_name: Option<String>,
    days: Vec<RawPmDay>,
}

#[derive(Deserialize)]
struct RawPmDay {
    name: String,
    #[serde(default)]
    tasks: Vec<RawPmTask>,
    #[serde(default)]
    lures: Vec<RawPmLure>,
    steps: Vec<RawPmStep>,
}

#[derive(Deserialize)]
struct RawPmTask {
    id: String,
    label: String,
}

#[derive(Deserialize)]
struct RawPmLure {
    id: String,
}

#[derive(Deserialize)]
struct RawPmStep {
    id: String,
    time: String,
    text: String,
    #[serde(default)]
    options: Vec<String>,
    #[serde(default)]
    updates: Vec<RawPmUpdate>,
    #[serde(default)]
    groundtruth: Option<RawPmGroundtruth>,
}

#[derive(Deserialize)]
struct RawPmUpdate {
    task_id: String,
    action: String,
    #[serde(default)]
    new_label: Option<String>,
}

#[derive(Deserialize)]
struct RawPmGroundtruth {
    #[serde(default)]
    actions: Vec<RawPmAction>,
}

#[derive(Deserialize)]
struct RawPmAction {
    id: String,
}

impl Converter for PmBenchConverter {
    fn dataset_id(&self) -> &str {
        "pm_bench"
    }

    fn converter_version(&self) -> &str {
        "1.2.0"
    }

    fn convert(
        &self,
        dataset_dir: &Path,
        manifest_entry: &DatasetManifestEntry,
    ) -> Result<ConvertedDataset, LabError> {
        let raw = fs::read_to_string(dataset_dir.join("data/synthetic_week_v9.json"))?;
        let parsed: RawPmBenchFile = serde_json::from_str(&raw)
            .map_err(|e| LabError::Serialization(format!("pm_bench parse error: {e}")))?;

        let soul = ActorId::new("soul");
        let env_actor = ActorId::new("env:virtual_week");
        let place = PlaceId::new("place:virtual_week_sim");
        let aud = Audience::new([env_actor.clone(), soul.clone()]);
        let base_ms: u64 = 1_700_000_000_000;

        let mut items = Vec::new();
        let mut gold_entries = Vec::new();
        let mut running_history = Vec::new();
        let mut running_ev = Vec::new();

        for (d_idx, day) in parsed.days.into_iter().enumerate() {
            let day_no = (d_idx + 1) as u64;
            let menu: Vec<String> = day
                .tasks
                .iter()
                .map(|t| format!("{}: {}", t.id, t.label))
                .collect();
            let menu_ids: BTreeSet<String> = day.tasks.iter().map(|t| t.id.clone()).collect();
            let day_start = base_ms + d_idx as u64 * 86_400_000;

            for (s_idx, step) in day.steps.into_iter().enumerate() {
                let (hh, mm) = step
                    .time
                    .split_once(':')
                    .and_then(|(h, m)| {
                        Some((h.trim().parse::<u64>().ok()?, m.trim().parse::<u64>().ok()?))
                    })
                    .unwrap_or((0, 0));
                let ts = day_start + hh * 3_600_000 + mm * 60_000;
                let updates: Vec<String> = step
                    .updates
                    .iter()
                    .map(|u| {
                        format!(
                            "Update: task {} {} -> {}",
                            u.task_id,
                            u.action,
                            u.new_label.as_deref().unwrap_or("")
                        )
                    })
                    .collect();
                let due: Vec<String> = step
                    .groundtruth
                    .as_ref()
                    .map(|g| g.actions.iter().map(|a| a.id.clone()).collect())
                    .unwrap_or_default();
                let step_text = format!(
                    "Day {day_no} ({}) {:02}:{:02}\n{}\nOptions: {}\n{}\nAvailable tasks (reply with the ids you perform now, comma separated, or NONE): [{}]",
                    day.name,
                    hh,
                    mm,
                    step.text,
                    step.options.join(" | "),
                    updates.join("\n"),
                    menu.join("; ")
                );
                let item_id = format!("pm_step_{d_idx:02}_{s_idx:02}");
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
                metadata.insert("due_task_ids".to_owned(), due.join(","));
                metadata.insert(
                    "menu_task_ids".to_owned(),
                    menu_ids.iter().cloned().collect::<Vec<_>>().join(","),
                );
                metadata.insert(
                    "lure_task_ids".to_owned(),
                    day.lures
                        .iter()
                        .map(|l| l.id.clone())
                        .collect::<Vec<_>>()
                        .join(","),
                );
                metadata.insert("step_id".to_owned(), step.id.clone());

                gold_entries.push(GoldEntry {
                    item_id: item_id.clone(),
                    probe_id: "choose".to_owned(),
                    cluster_id: "virtual_week_7d".to_owned(),
                    gold_answer: due.join(","),
                    accepted_answers: due,
                    gold_evidence: running_ev.clone(),
                    privacy_conflict: false,
                    category: Some(day.name.clone()),
                    metadata,
                });

                items.push(ExternalItem {
                    item_id,
                    cluster_id: "virtual_week_7d".to_owned(),
                    strata_key: format!("day_{day_no}"),
                    history: running_history.clone(),
                    checkpoint_group: None,
                    probes: vec![probe],
                    history_mode: HistoryMode::StaticSilent,
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
            let menu: BTreeSet<&str> = entry
                .metadata
                .get("menu_task_ids")
                .map(|s| s.split(',').filter(|t| !t.is_empty()).collect())
                .unwrap_or_default();
            let resp = pred
                .response_text
                .as_deref()
                .unwrap_or("")
                .to_ascii_lowercase();
            let chosen: BTreeSet<&str> = menu
                .iter()
                .copied()
                .filter(|id| resp.contains(&id.to_ascii_lowercase()))
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
