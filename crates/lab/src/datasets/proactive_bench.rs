//! `proactive_bench` (`thunlp/ProactiveAgent`) converter & deterministic `TGL` trigger-parity scorer.

use std::collections::BTreeMap;
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
use crate::datasets::scoring_utils::{
    binary_prf1, not_scored_metric, resolve_verified_scorer_model_dir, run_optional_python_helper,
};

/// Manifest ID for the offline-verified ProactiveAgent Reward Model (`YancyLee/ProactiveAgent`).
pub const PROACTIVE_RM_MODEL_ID: &str = "proactive_agent_rm";

pub struct ProactiveBenchConverter;

#[derive(Deserialize)]
struct RawProactiveEvent {
    #[serde(default)]
    event_id: Option<String>,
    domain: String,
    scenario_id: String,
    #[serde(default)]
    timestamp_ms: Option<u64>,
    observation: String,
    event: String,
    need: bool,
    #[serde(default)]
    reference_task: Option<String>,
}

impl Converter for ProactiveBenchConverter {
    fn dataset_id(&self) -> &str {
        "proactive_bench"
    }

    fn converter_version(&self) -> &str {
        "1.0.0"
    }

    fn convert(
        &self,
        dataset_dir: &Path,
        manifest_entry: &DatasetManifestEntry,
    ) -> Result<ConvertedDataset, LabError> {
        let raw = fs::read_to_string(dataset_dir.join("dataset/test_data/test_data.jsonl"))?;
        let soul = ActorId::new("soul");
        let user = ActorId::new("user:primary");
        let aud = Audience::new([user, soul]);

        let mut items = Vec::new();
        let mut gold_entries = Vec::new();
        let mut scenario_histories: BTreeMap<
            String,
            (Vec<monoagens_protocol::Percept>, Vec<GoldEvidenceRef>),
        > = BTreeMap::new();

        for (idx, line) in raw.lines().filter(|l| !l.trim().is_empty()).enumerate() {
            let ev: RawProactiveEvent = serde_json::from_str(line)
                .map_err(|e| LabError::Serialization(format!("proactive_bench JSONL: {e}")))?;
            let item_id = ev.event_id.unwrap_or_else(|| format!("pb_{idx:04}"));
            let cluster_id = format!("{}:{}", ev.domain, ev.scenario_id);
            let place = PlaceId::new(format!("place:{cluster_id}"));
            let ts = ev
                .timestamp_ms
                .unwrap_or(1_700_000_000_000 + (idx as u64) * 5_000);
            let payload = format!("Observation: {}\nEvent: {}", ev.observation, ev.event);

            let (hist, ev_refs) = scenario_histories.entry(cluster_id.clone()).or_default();
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
            metadata.insert("need".to_owned(), ev.need.to_string());
            metadata.insert("domain".to_owned(), ev.domain.clone());

            gold_entries.push(GoldEntry {
                item_id: item_id.clone(),
                probe_id: "trigger".to_owned(),
                cluster_id: cluster_id.clone(),
                gold_answer: ev
                    .reference_task
                    .clone()
                    .unwrap_or_else(|| if ev.need { "SPEAK" } else { "SILENT" }.to_owned()),
                accepted_answers: ev.reference_task.into_iter().collect(),
                gold_evidence: ev_refs.clone(),
                privacy_conflict: false,
                category: Some(ev.domain.clone()),
                metadata,
            });

            items.push(ExternalItem {
                item_id,
                cluster_id,
                strata_key: format!("{}:need={}", ev.domain, ev.need),
                history: hist.clone(),
                checkpoint_group: None,
                probes: vec![probe],
                history_mode: HistoryMode::Interactive,
                capabilities: Vec::new(),
            });

            let turn_idx = hist.len();
            hist.push(make_text_percept(
                "user:primary",
                place.as_str(),
                aud.members.iter().cloned(),
                ts,
                payload,
            ));
            ev_refs.push(GoldEvidenceRef {
                turn_index: turn_idx,
                audience: aud.clone(),
                place,
            });
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
            match (need, pred.spoke) {
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
