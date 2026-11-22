//! `proactive_bench` (`thunlp/ProactiveAgent`) converter & deterministic `TGL` trigger-parity scorer.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use monoagens_protocol::{ActorId, Audience, PlaceId, VisibleScope};
use serde::Deserialize;

use crate::LabError;
use crate::datasets::framework::{
    ConvertedDataset, Converter, ExternalItem, ExternalPrediction, ExternalProbe, GoldBook,
    GoldEntry, GoldEvidenceRef, HistoryMode, JudgeExecutionConfig, Scorer, ScorerMetric,
    ScorerReport, make_text_percept,
};
use crate::datasets::manifest::DatasetManifestEntry;
use crate::datasets::scoring_utils::{
    binary_prf1, not_scored_metric, resolve_verified_scorer_model_dir, run_optional_python_helper,
};

/// Manifest ID for the offline-verified ProactiveAgent Reward Model (`YancyLee/ProactiveAgent`).
pub const PROACTIVE_RM_MODEL_ID: &str = "proactive_agent_rm";

pub struct ProactiveBenchConverter;

#[derive(Deserialize)]
struct RawProactiveObservation {
    time: String,
    event: String,
}

#[derive(Deserialize)]
struct RawProactiveAgentResponse {
    #[serde(default)]
    candidate_task: Vec<String>,
}

/// Real upstream `dataset/test_data/{code,writing}_NN.json` event record.
#[derive(Deserialize)]
struct RawProactiveEvent {
    observation: RawProactiveObservation,
    agent_response: RawProactiveAgentResponse,
    task_status: bool,
}

impl Converter for ProactiveBenchConverter {
    fn dataset_id(&self) -> &str {
        "proactive_bench"
    }

    fn converter_version(&self) -> &str {
        "1.3.0"
    }

    fn convert(
        &self,
        dataset_dir: &Path,
        manifest_entry: &DatasetManifestEntry,
    ) -> Result<ConvertedDataset, LabError> {
        let test_dir = dataset_dir.join("dataset/test_data");
        let mut files: Vec<PathBuf> = Vec::new();
        if test_dir.is_dir() {
            for entry in fs::read_dir(&test_dir)? {
                let path = entry?.path();
                let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
                if name.ends_with(".json") && name != "splits.json" {
                    files.push(path);
                }
            }
        }
        files.sort();

        let soul = ActorId::new("soul");
        let user = ActorId::new("user:primary");
        let aud = Audience::new([user.clone(), soul.clone()]);
        let base_ms: u64 = 1_700_000_000_000;

        let mut items = Vec::new();
        let mut gold_entries = Vec::new();

        for file in files {
            let stem = file
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("scenario")
                .to_owned();
            let domain = stem
                .split_once('_')
                .map_or_else(|| stem.clone(), |(d, _)| d.to_owned());
            let place = PlaceId::new(format!("place:{stem}"));
            let raw = fs::read_to_string(&file)?;
            let events: Vec<RawProactiveEvent> = serde_json::from_str(&raw)
                .map_err(|e| LabError::Serialization(format!("proactive_bench {stem}: {e}")))?;

            let mut history = Vec::with_capacity(events.len());
            let mut ev_refs = Vec::with_capacity(events.len());
            for (idx, ev) in events.into_iter().enumerate() {
                let ts = ev
                    .observation
                    .time
                    .trim()
                    .parse::<f64>()
                    .ok()
                    .and_then(|secs| {
                        (secs.is_finite() && secs >= 0.0).then_some((secs * 1000.0) as u64)
                    })
                    .unwrap_or(base_ms + (idx as u64) * 5_000);
                let item_id = format!("pb_{stem}_{idx:03}");
                let payload = format!(
                    "Observation: {}\n\nShould you proactively intervene and offer assistance? If yes, state the assistance you would offer; if not, answer SILENT.",
                    ev.observation.event
                );
                let reference_task = ev
                    .agent_response
                    .candidate_task
                    .first()
                    .cloned()
                    .unwrap_or_default();

                let probe = ExternalProbe {
                    probe_id: "trigger".to_owned(),
                    percept: make_text_percept(
                        "user:primary",
                        place.as_str(),
                        aud.members.iter().cloned(),
                        ts,
                        &payload,
                    ),
                    isolated_checkpoint: true,
                    probe_scope: VisibleScope::new(aud.clone(), Some(place.clone())),
                    expected_capability: None,
                };

                let mut metadata = BTreeMap::new();
                metadata.insert("need".to_owned(), ev.task_status.to_string());
                metadata.insert("domain".to_owned(), domain.clone());
                metadata.insert("scenario_id".to_owned(), stem.clone());

                gold_entries.push(GoldEntry {
                    item_id: item_id.clone(),
                    probe_id: "trigger".to_owned(),
                    cluster_id: stem.clone(),
                    gold_answer: reference_task.clone(),
                    accepted_answers: if reference_task.is_empty() {
                        Vec::new()
                    } else {
                        vec![reference_task]
                    },
                    gold_evidence: ev_refs.clone(),
                    privacy_conflict: false,
                    category: Some(domain.clone()),
                    metadata,
                });

                items.push(ExternalItem {
                    item_id,
                    cluster_id: stem.clone(),
                    strata_key: format!("{domain}:need={}", ev.task_status),
                    history: history.clone(),
                    checkpoint_group: None,
                    probes: vec![probe],
                    history_mode: HistoryMode::StaticSilent,
                    capabilities: Vec::new(),
                });

                let turn_idx = history.len();
                history.push(make_text_percept(
                    "user:primary",
                    place.as_str(),
                    aud.members.iter().cloned(),
                    ts,
                    payload,
                ));
                ev_refs.push(GoldEvidenceRef {
                    turn_index: turn_idx,
                    audience: aud.clone(),
                    place: place.clone(),
                });
            }
        }

        if items.is_empty() {
            return Err(LabError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!(
                    "no `dataset/test_data/*.json` ProactiveAgent events under '{}'",
                    dataset_dir.display()
                ),
            )));
        }

        Ok(ConvertedDataset {
            dataset_id: "proactive_bench".to_owned(),
            pinned_revision: manifest_entry
                .sources
                .first()
                .map_or_else(|| "pending_pin".to_owned(), |s| s.revision.clone()),
            converter_version: self.converter_version().to_owned(),
            items,
            gold_book: GoldBook {
                dataset_id: "proactive_bench".to_owned(),
                entries: gold_entries,
            },
        })
    }
}

pub struct ProactiveBenchScorer;

impl Scorer for ProactiveBenchScorer {
    fn dataset_id(&self) -> &str {
        "proactive_bench"
    }

    fn scorer_version(&self) -> &str {
        "1.0.0"
    }

    fn score(
        &self,
        predictions: &[ExternalPrediction],
        gold: &GoldBook,
        judge: &JudgeExecutionConfig<'_>,
        _seed: u64,
    ) -> Result<ScorerReport, LabError> {
        let mut cd = 0usize;
        let mut fd = 0usize;
        let mut mn = 0usize;
        let mut nr = 0usize;

        let mut rm_payload = Vec::new();
        for pred in predictions {
            let entry = gold.find(&pred.item_id, &pred.probe_id).ok_or_else(|| {
                LabError::Protocol(format!("proactive_bench missing gold for {}", pred.item_id))
            })?;
            let need = entry.metadata.get("need").map(String::as_str) == Some("true");
            let predicted_spoke = if !pred.spoke {
                false
            } else {
                let upper = pred
                    .response_text
                    .as_deref()
                    .unwrap_or("")
                    .trim()
                    .to_ascii_uppercase();
                !upper.starts_with("SILENT")
            };
            match (need, predicted_spoke) {
                (true, true) => {
                    cd += 1;
                    if let Some(resp) = &pred.response_text {
                        rm_payload.push(serde_json::json!({
                            "item_id": pred.item_id,
                            "context": "",
                            "response": resp,
                            "reference": entry.gold_answer,
                        }));
                    }
                }
                (false, true) => fd += 1,
                (true, false) => mn += 1,
                (false, false) => nr += 1,
            }
            let hit = need == predicted_spoke;
            judge.record_outcome(crate::datasets::framework::ScorerItemOutcome {
                item_id: pred.item_id.clone(),
                probe_id: pred.probe_id.clone(),
                cluster_id: pred.cluster_id.clone(),
                category: entry.category.clone(),
                metric_name: "proactive_trigger".to_owned(),
                score: Some(if hit { 1.0 } else { 0.0 }),
                passed: Some(hit),
                judge_prompt: None,
                judge_verdict: None,
                failed: false,
                failure_reason: None,
            });
        }

        let (_p, _r, f1) = binary_prf1(cd, fd, mn);
        let fd_rate = if fd + nr == 0 {
            0.0
        } else {
            fd as f64 / (fd + nr) as f64
        };

        let ws_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let rm_metric = if !rm_payload.is_empty()
            && let Some(model_dir) =
                resolve_verified_scorer_model_dir(judge, PROACTIVE_RM_MODEL_ID)?
        {
            let helper_res = run_optional_python_helper(
                &ws_root,
                "lab/adapters/eval_helpers/proactive_rm.py",
                &serde_json::json!({
                    "model_path": model_dir.to_string_lossy(),
                    "items": rm_payload
                }),
            );
            match helper_res {
                Ok(Some(val)) => {
                    let mut accepted_cd = 0usize;
                    if let Some(arr) = val.get("results").and_then(|v| v.as_array()) {
                        for r in arr {
                            if r.get("accepted").and_then(|v| v.as_bool()) == Some(true) {
                                accepted_cd += 1;
                            }
                        }
                    }
                    let rejected = cd.saturating_sub(accepted_cd);
                    let (_rp, _rr, rm_f1) = binary_prf1(accepted_cd, fd + rejected, mn + rejected);
                    ScorerMetric {
                        name: "rm_f1".to_owned(),
                        value: Some(rm_f1),
                        ci95_low: None,
                        ci95_high: None,
                        n_clusters: Some(predictions.len()),
                        not_scored_reason: None,
                    }
                }
                Ok(None) => not_scored_metric("rm_f1", "scorer_model_unavailable"),
                Err(e) if e.to_string().contains("scorer_model_unavailable") => {
                    not_scored_metric("rm_f1", "scorer_model_unavailable")
                }
                Err(e) => return Err(e),
            }
        } else {
            not_scored_metric("rm_f1", "scorer_model_unavailable")
        };

        Ok(ScorerReport {
            dataset_id: "proactive_bench".to_owned(),
            scorer_version: self.scorer_version().to_owned(),
            metrics: vec![
                ScorerMetric {
                    name: "f1".to_owned(),
                    value: Some(f1),
                    ci95_low: None,
                    ci95_high: None,
                    n_clusters: Some(predictions.len()),
                    not_scored_reason: None,
                },
                ScorerMetric {
                    name: "false_detection".to_owned(),
                    value: Some(fd_rate),
                    ci95_low: None,
                    ci95_high: None,
                    n_clusters: Some(predictions.len()),
                    not_scored_reason: None,
                },
                rm_metric,
            ],
            judge_model: None,
            judge_prompt_version: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::datasets::framework::ScorerDetailRecorder;

    fn pb_gold_entry(item_id: &str, need: bool, category: &str) -> GoldEntry {
        let mut metadata = BTreeMap::new();
        metadata.insert("need".to_owned(), need.to_string());
        GoldEntry {
            item_id: item_id.to_owned(),
            probe_id: "trigger".to_owned(),
            cluster_id: "code_01".to_owned(),
            gold_answer: if need {
                "Offer to check build logs".to_owned()
            } else {
                String::new()
            },
            accepted_answers: Vec::new(),
            gold_evidence: Vec::new(),
            privacy_conflict: false,
            category: Some(category.to_owned()),
            metadata,
        }
    }

    fn pb_pred(item_id: &str, spoke: bool, response_text: Option<&str>) -> ExternalPrediction {
        ExternalPrediction {
            item_id: item_id.to_owned(),
            probe_id: "trigger".to_owned(),
            cluster_id: "code_01".to_owned(),
            spoke,
            response_text: response_text.map(str::to_owned),
            action_turn_ms: Some(1_000),
            probe_gate_wakes: 1,
            history_gate_wakes: 0,
            tokens_in: 10,
            tokens_out: 5,
            cost_usd: 0.0,
        }
    }

    #[test]
    fn chat_speak_silent_is_scored_as_non_speaking_and_recorded() {
        let gold = GoldBook {
            dataset_id: "proactive_bench".to_owned(),
            entries: vec![
                pb_gold_entry("pb_need_silent", true, "code"),
                pb_gold_entry("pb_noneed_silent", false, "writing"),
            ],
        };
        let preds = vec![
            pb_pred("pb_need_silent", true, Some("SILENT")),
            pb_pred("pb_noneed_silent", true, Some("  silent  ")),
        ];
        let recorder = Arc::new(Mutex::new(ScorerDetailRecorder::default()));
        let judge = JudgeExecutionConfig {
            recorder: Some(recorder.clone()),
            ..JudgeExecutionConfig::default()
        };

        let rep = ProactiveBenchScorer
            .score(&preds, &gold, &judge, 1)
            .expect("score proactive_bench");
        let f1 = rep.metrics.iter().find(|m| m.name == "f1").unwrap().value;
        let fd = rep
            .metrics
            .iter()
            .find(|m| m.name == "false_detection")
            .unwrap()
            .value;
        assert_eq!(f1, Some(0.0));
        assert_eq!(fd, Some(0.0));

        let guard = recorder.lock().expect("recorder lock");
        assert_eq!(guard.outcomes.len(), 2);
        assert_eq!(guard.outcomes[0].item_id, "pb_need_silent");
        assert_eq!(guard.outcomes[0].metric_name, "proactive_trigger");
        assert_eq!(guard.outcomes[0].category.as_deref(), Some("code"));
        assert_eq!(guard.outcomes[0].score, Some(0.0));
        assert_eq!(guard.outcomes[0].passed, Some(false));

        assert_eq!(guard.outcomes[1].item_id, "pb_noneed_silent");
        assert_eq!(guard.outcomes[1].metric_name, "proactive_trigger");
        assert_eq!(guard.outcomes[1].category.as_deref(), Some("writing"));
        assert_eq!(guard.outcomes[1].score, Some(1.0));
        assert_eq!(guard.outcomes[1].passed, Some(true));
    }
}
