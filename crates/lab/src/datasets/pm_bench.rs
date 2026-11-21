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

impl PmBenchConverter {
    /// Converts `pm_bench` using the requested [`HistoryMode`] (`StaticSilent` by default, or
    /// opt-in `LiveSequential` for continuous 7-day timeline execution preserving `ExpectationBook`
    /// and per-place `place_memory` across simulated days).
    pub fn convert_with_history_mode(
        &self,
        dataset_dir: &Path,
        manifest_entry: &DatasetManifestEntry,
        history_mode: HistoryMode,
    ) -> Result<ConvertedDataset, LabError> {
        let raw = fs::read_to_string(dataset_dir.join("data/synthetic_week_v9.json"))?;
        let parsed: RawPmBenchFile = serde_json::from_str(&raw)
            .map_err(|e| LabError::Serialization(format!("pm_bench parse error: {e}")))?;

        let soul = ActorId::new("soul");
        let env_actor = ActorId::new("env:virtual_week");
        let place = PlaceId::new("place:virtual_week_sim");
        let aud = Audience::new([env_actor.clone(), soul.clone()]);
        let base_ms: u64 = 1_700_000_000_000;
        let is_sequential = history_mode == HistoryMode::LiveSequential;

        let mut items = Vec::new();
        let mut sequential_probes = Vec::new();
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
                let step_key = format!("pm_step_{d_idx:02}_{s_idx:02}");
                let (item_id, probe_id) = if is_sequential {
                    ("pm_bench_live_sequential".to_owned(), step_key)
                } else {
                    (step_key, "choose".to_owned())
                };
                let probe = ExternalProbe {
                    probe_id: probe_id.clone(),
                    percept: make_text_percept(
                        "env:virtual_week",
                        place.as_str(),
                        aud.members.iter().cloned(),
                        ts,
                        &step_text,
                    ),
                    isolated_checkpoint: !is_sequential,
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
                    probe_id,
                    cluster_id: "virtual_week_7d".to_owned(),
                    gold_answer: due.join(","),
                    accepted_answers: due,
                    gold_evidence: running_ev.clone(),
                    privacy_conflict: false,
                    category: Some(day.name.clone()),
                    metadata,
                });

                if is_sequential {
                    sequential_probes.push(probe);
                } else {
                    items.push(ExternalItem {
                        item_id,
                        cluster_id: "virtual_week_7d".to_owned(),
                        strata_key: format!("day_{day_no}"),
                        history: running_history.clone(),
                        checkpoint_group: None,
                        probes: vec![probe],
                        history_mode,
                        capabilities: Vec::new(),
                    });
                }

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

        if is_sequential {
            items.push(ExternalItem {
                item_id: "pm_bench_live_sequential".to_owned(),
                cluster_id: "virtual_week_7d".to_owned(),
                strata_key: "week_7d".to_owned(),
                history: Vec::new(),
                checkpoint_group: None,
                probes: sequential_probes,
                history_mode: HistoryMode::LiveSequential,
                capabilities: Vec::new(),
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
        self.convert_with_history_mode(dataset_dir, manifest_entry, HistoryMode::StaticSilent)
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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Arc;

    use monoagens_core::{CoreConfig, ModelPurpose, NeuralModel};
    use monoagens_reason::model::{
        ModelCallFailure, ModelCallSuccess, ModelProvider, ModelRequest, ScriptedModelProvider,
        ScriptedReply,
    };

    use super::*;
    use crate::CostGuard;
    use crate::benchmark::BenchmarkDomain;
    use crate::datasets::framework::{
        assert_visiblescope_gold_evidence, execute_external_item_with_model_and_state,
    };
    use crate::datasets::manifest::{
        ChecksumsRecorded, ContaminationRisk, CoreApplicability, DatasetAvailability, DatasetLane,
        DatasetLicense, LicenseStatus, ManifestEntryKind,
    };

    fn test_manifest_entry() -> DatasetManifestEntry {
        DatasetManifestEntry {
            id: "pm_bench".to_owned(),
            kind: ManifestEntryKind::Dataset,
            domain: BenchmarkDomain::Prospective,
            relative_dir: "pm_bench".to_owned(),
            sources: Vec::new(),
            license: DatasetLicense {
                spdx: "MIT".to_owned(),
                terms: "test".to_owned(),
                status: LicenseStatus::Cleared,
            },
            availability: DatasetAvailability::Available,
            core_applicability: CoreApplicability::CoreBehavioral,
            lane: DatasetLane::CoreBehavioral,
            contamination_risk: ContaminationRisk::Low,
            contamination_note: "test".to_owned(),
            reason: "pm_bench LiveSequential test".to_owned(),
            split: None,
            files: Vec::new(),
            checksums_recorded: ChecksumsRecorded::Na,
            checksums_recorded_date: None,
            converter_version: "1.2.0".to_owned(),
            scorer_version: "1.0.0".to_owned(),
        }
    }

    #[test]
    fn pm_bench_live_sequential_preserves_expectation_book_and_place_memory_across_days() {
        let dir = tempfile::tempdir().expect("tempdir");
        let data_dir = dir.path().join("data");
        fs::create_dir_all(&data_dir).expect("mkdir data");

        let two_day_json = r#"{
          "scenario_name": "two_day_live_seq_test",
          "days": [
            {
              "name": "Monday",
              "tasks": [{"id": "task_alpha", "label": "Prepare weekly brief"}],
              "lures": [],
              "steps": [
                {
                  "id": "d1_s1",
                  "time": "09:00",
                  "text": "Morning check-in on Monday.",
                  "options": ["A) Start"],
                  "updates": [],
                  "groundtruth": {"actions": [{"id": "task_alpha"}]}
                }
              ]
            },
            {
              "name": "Tuesday",
              "tasks": [{"id": "task_beta", "label": "Send follow-up"}],
              "lures": [],
              "steps": [
                {
                  "id": "d2_s1",
                  "time": "10:00",
                  "text": "Morning check-in on Tuesday.",
                  "options": ["A) Follow up"],
                  "updates": [],
                  "groundtruth": {"actions": [{"id": "task_beta"}]}
                }
              ]
            }
          ]
        }"#;
        fs::write(data_dir.join("synthetic_week_v9.json"), two_day_json).expect("write fixture");

        let manifest = test_manifest_entry();
        let static_conv = PmBenchConverter
            .convert(dir.path(), &manifest)
            .expect("static convert");
        assert_eq!(static_conv.items.len(), 2);
        assert!(
            static_conv
                .items
                .iter()
                .all(|it| it.history_mode == HistoryMode::StaticSilent)
        );

        let seq_conv = PmBenchConverter
            .convert_with_history_mode(dir.path(), &manifest, HistoryMode::LiveSequential)
            .expect("live_sequential convert");
        assert_eq!(seq_conv.items.len(), 1);
        let item = &seq_conv.items[0];
        assert_eq!(item.history_mode, HistoryMode::LiveSequential);
        assert_eq!(item.probes.len(), 2);
        assert!(!item.probes[0].isolated_checkpoint);
        assert_visiblescope_gold_evidence(&seq_conv.items, &seq_conv.gold_book)
            .expect("visiblescope");

        let model_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|p| p.parent())
            .expect("workspace root")
            .join("crates/core/tests/fixtures/neural_phase1_golden.safetensors");
        let model = Arc::new(
            NeuralModel::from_safetensors_bytes(
                &fs::read(&model_path).expect("read golden model"),
                None,
            )
            .expect("load golden model"),
        );

        struct DayRoutingProvider;

        impl ModelProvider for DayRoutingProvider {
            fn provider_name(&self) -> &str {
                "day_routing_test"
            }

            fn invoke(&self, request: &ModelRequest) -> Result<ModelCallSuccess, ModelCallFailure> {
                let prompt_dbg = format!("{:?}", request.messages);
                let reply = if request.purpose == ModelPurpose::Reason
                    && prompt_dbg.contains("Day 2 (Tuesday)")
                {
                    r#"{"acts":[
                      {"act":"act","capability_id":"chat.speak","places":["place:virtual_week_sim"],"people":[],"payload":[116,97,115,107,95,98,101,116,97],"motivation":4,"confidence_0_20":18,"used_ids":[]}
                    ]}"#
                } else if request.purpose == ModelPurpose::Reason
                    && prompt_dbg.contains("Day 1 (Monday)")
                {
                    r#"{"acts":[
                      {"act":"expect","expect_kind":"promise","about_ids":[],"places":["place:virtual_week_sim"],"people":["env:virtual_week"],"condition":"deliver weekly brief","due_ms":1700518400000,"expiry_ms":1700604800000},
                      {"act":"act","capability_id":"chat.speak","places":["place:virtual_week_sim"],"people":[],"payload":[116,97,115,107,95,97,108,112,104,97],"motivation":4,"confidence_0_20":18,"used_ids":[]}
                    ]}"#
                } else {
                    crate::datasets::SILENT_THINK_JSON
                };
                ScriptedModelProvider::from_replies([ScriptedReply::text(reply)]).invoke(request)
            }
        }

        let provider = DayRoutingProvider;

        let mut guard = CostGuard::default();
        let (exec, final_state) = execute_external_item_with_model_and_state(
            item,
            &CoreConfig::default(),
            1,
            Some(model),
            &provider,
            "scripted",
            false,
            &mut guard,
            None,
            None,
            None,
            None,
        )
        .expect("execute LiveSequential pm_bench");

        assert_eq!(exec.history_reasoning_mode, "live_sequential");
        assert_eq!(exec.predictions.len(), 2);
        assert!(
            final_state
                .expectation_book
                .expectations
                .values()
                .any(|e| e.condition == "deliver weekly brief" && e.is_active()),
            "Day 1 ExpectationBook entry must persist into Day 2 in LiveSequential mode"
        );
        let h_place = final_state
            .place_memory
            .get(&PlaceId::new("place:virtual_week_sim"))
            .expect("place_memory persisted for virtual_week_sim");
        assert!(
            h_place.iter().any(|&v| v.abs() > 1e-6),
            "per-place recurrent place_memory h_t must be non-zero across simulated days"
        );

        let scored = PmBenchScorer
            .score(
                &exec.predictions,
                &seq_conv.gold_book,
                &JudgeExecutionConfig::default(),
                1,
            )
            .expect("score LiveSequential pm_bench");
        let f1 = scored
            .metrics
            .iter()
            .find(|m| m.name == "set_f1")
            .and_then(|m| m.value)
            .unwrap();
        assert!((f1 - 1.0).abs() < 1e-9);
    }
}
