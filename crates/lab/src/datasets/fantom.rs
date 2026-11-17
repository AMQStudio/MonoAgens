//! `fantom` (`skywalker023/fantom`) Mode-1 converter & `All` (deterministic) + `All*` (`lab/adapters` ST) scorer.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use monoagens_protocol::{ActorId, Audience, PlaceId, VisibleScope};
use serde::Deserialize;

use crate::LabError;
use crate::datasets::framework::{
    ConvertedDataset, Converter, ExternalItem, ExternalPrediction, ExternalProbe, GoldBook,
    GoldEntry, HistoryMode, JudgeExecutionConfig, Scorer, ScorerReport, make_text_percept,
};
use crate::datasets::manifest::DatasetManifestEntry;
use crate::datasets::scoring_utils::{
    build_mode1_tom_history, exact_match_normalized, metric_with_cluster_ci, not_scored_metric,
    resolve_verified_scorer_model_dir, run_optional_python_helper,
};

/// Manifest ID for the offline-verified SentenceTransformer scorer model (`all-roberta-large-v1`).
pub const FANTOM_SCORER_MODEL_ID: &str = "fantom_scorer_all_roberta_large_v1";

pub struct FantomConverter;

#[derive(Deserialize)]
struct RawFantomConv {
    conv_id: String,
    #[serde(default)]
    setting: Option<String>,
    participants: Vec<String>,
    turns: Vec<String>,
    info_sets: Vec<RawFantomInfoSet>,
}

#[derive(Deserialize)]
struct RawFantomInfoSet {
    set_id: String,
    questions: Vec<RawFantomQuestion>,
}

#[derive(Deserialize)]
struct RawFantomQuestion {
    qid: String,
    formulation: String,
    question: String,
    answer: String,
    #[serde(default)]
    wrong_belief_answer: Option<String>,
}

impl Converter for FantomConverter {
    fn dataset_id(&self) -> &str {
        "fantom"
    }

    fn converter_version(&self) -> &str {
        "1.0.0"
    }

    fn convert(
        &self,
        dataset_dir: &Path,
        manifest_entry: &DatasetManifestEntry,
    ) -> Result<ConvertedDataset, LabError> {
        let raw = fs::read_to_string(dataset_dir.join("data/fantom.json"))?;
        let convs: Vec<RawFantomConv> = serde_json::from_str(&raw)
            .map_err(|e| LabError::Serialization(format!("fantom JSON: {e}")))?;

        let soul = ActorId::new("soul");
        let proctor = ActorId::new("lab_proctor");
        let probe_aud = Audience::new([proctor, soul]);

        let mut items = Vec::new();
        let mut gold_entries = Vec::new();

        for conv in convs {
            let setting = conv.setting.unwrap_or_else(|| "short".to_owned());
            let place = PlaceId::new(format!("fantom_conv_{}_main", conv.conv_id));
            let (history, ev_refs) = build_mode1_tom_history(
                &conv.turns,
                &format!("fantom_conv_{}", conv.conv_id),
                &conv.participants,
                1_700_000_000_000,
                1_000,
            );
            let probe_base = 1_700_000_000_000 + (history.len() as u64 + 1) * 1_000;

            for info_set in conv.info_sets {
                let item_id = format!("{}_{}", conv.conv_id, info_set.set_id);
                let mut probes = Vec::with_capacity(info_set.questions.len());
                for (q_idx, q) in info_set.questions.into_iter().enumerate() {
                    probes.push(ExternalProbe {
                        probe_id: q.qid.clone(),
                        percept: make_text_percept(
                            "lab_proctor",
                            place.as_str(),
                            probe_aud.members.iter().cloned(),
                            probe_base + (q_idx as u64) * 1_000,
                            &q.question,
                        ),
                        isolated_checkpoint: true,
                        probe_scope: VisibleScope::new(probe_aud.clone(), Some(place.clone())),
                        expected_capability: None,
                    });

                    let mut metadata = BTreeMap::new();
                    metadata.insert("formulation".to_owned(), q.formulation);
                    metadata.insert("set_id".to_owned(), info_set.set_id.clone());
                    if let Some(wb) = q.wrong_belief_answer {
                        metadata.insert("wrong_belief_answer".to_owned(), wb);
                    }

                    gold_entries.push(GoldEntry {
                        item_id: item_id.clone(),
                        probe_id: q.qid,
                        cluster_id: conv.conv_id.clone(),
                        gold_answer: q.answer.clone(),
                        accepted_answers: vec![q.answer],
                        gold_evidence: ev_refs.clone(),
                        privacy_conflict: false,
                        category: Some(setting.clone()),
                        metadata,
                    });
                }

                items.push(ExternalItem {
                    item_id,
                    cluster_id: conv.conv_id.clone(),
                    strata_key: format!("{setting}:p={}", conv.participants.len()),
                    history: history.clone(),
                    checkpoint_group: Some(format!("fantom_{}", conv.conv_id)),
                    probes,
                    history_mode: HistoryMode::StaticSilent,
                    capabilities: Vec::new(),
                });
            }
        }

        Ok(ConvertedDataset {
            dataset_id: "fantom".to_owned(),
            pinned_revision: manifest_entry
                .sources
                .first()
                .map_or_else(|| "pending_pin".to_owned(), |s| s.revision.clone()),
            converter_version: self.converter_version().to_owned(),
            items,
            gold_book: GoldBook {
                dataset_id: "fantom".to_owned(),
                entries: gold_entries,
            },
        })
    }
}

pub struct FantomScorer;

impl Scorer for FantomScorer {
    fn dataset_id(&self) -> &str {
        "fantom"
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
        // Group predictions by info-piece set `item_id`.
        let mut by_set: BTreeMap<&str, Vec<&ExternalPrediction>> = BTreeMap::new();
        for p in predictions {
            by_set.entry(p.item_id.as_str()).or_default().push(p);
        }

        let mut all_5_obs = Vec::new();
        let mut st_payload = Vec::new();
        let mut set_clusters: BTreeMap<String, String> = BTreeMap::new();
        let mut set_5_ok: BTreeMap<String, bool> = BTreeMap::new();

        for (item_id, preds) in &by_set {
            let mut five_ok = true;
            let mut cluster_id = String::new();
            for p in preds {
                let g = gold.find(item_id, &p.probe_id).ok_or_else(|| {
                    LabError::Protocol(format!("fantom missing gold for {item_id}:{}", p.probe_id))
                })?;
                cluster_id = g.cluster_id.clone();
                let form = g
                    .metadata
                    .get("formulation")
                    .map(String::as_str)
                    .unwrap_or("");
                let resp = p.response_text.as_deref().unwrap_or("");
                if form == "BeliefQ[Dist.]" {
                    st_payload.push(serde_json::json!({
                        "item_id": item_id,
                        "probe_id": p.probe_id,
                        "cluster_id": cluster_id,
                        "response": resp,
                        "gold": g.gold_answer,
                        "wrong_belief": g.metadata.get("wrong_belief_answer").cloned().unwrap_or_default(),
                    }));
                } else {
                    let hit = if form.ends_with("[List]") {
                        let g_set: BTreeSet<String> = g
                            .gold_answer
                            .split(',')
                            .map(|s| s.trim().to_ascii_lowercase())
                            .filter(|s| !s.is_empty())
                            .collect();
                        let p_set: BTreeSet<String> = resp
                            .split([',', ' '])
                            .map(|s| s.trim().to_ascii_lowercase())
                            .filter(|s| g_set.contains(s) || s.len() > 2)
                            .collect();
                        !g_set.is_empty() && g_set.is_subset(&p_set)
                    } else {
                        exact_match_normalized(resp, &g.accepted_answers) > 0.5
                    };
                    five_ok &= hit;
                }
            }
            set_clusters.insert((*item_id).to_owned(), cluster_id.clone());
            set_5_ok.insert((*item_id).to_owned(), five_ok);
            all_5_obs.push((cluster_id, if five_ok { 1.0 } else { 0.0 }));
        }

        // Offline-only local SentenceTransformer helper for All* (6/6 simultaneous).
        let ws_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let all_star_metric = if st_payload.is_empty() {
            metric_with_cluster_ci("fantom_all_star", &all_5_obs, seed)
        } else if let Some(model_dir) =
            resolve_verified_scorer_model_dir(judge, FANTOM_SCORER_MODEL_ID)?
        {
            let helper_res = run_optional_python_helper(
                &ws_root,
                "lab/adapters/eval_helpers/fantom_eval.py",
                &serde_json::json!({
                    "model_path": model_dir.to_string_lossy(),
                    "items": st_payload
                }),
            );
            match helper_res {
                Ok(Some(val)) => {
                    let mut dist_ok: BTreeMap<String, bool> = BTreeMap::new();
                    if let Some(arr) = val.get("results").and_then(|v| v.as_array()) {
                        for r in arr {
                            if let (Some(iid), Some(ok)) = (
                                r.get("item_id").and_then(|v| v.as_str()),
                                r.get("ok").and_then(|v| v.as_bool()),
                            ) {
                                let cur = dist_ok.entry(iid.to_owned()).or_insert(true);
                                *cur &= ok;
                            }
                        }
                    }
                    let star_obs: Vec<(String, f64)> = set_5_ok
                        .iter()
                        .map(|(iid, &ok5)| {
                            let ok6 = ok5 && dist_ok.get(iid).copied().unwrap_or(false);
                            (set_clusters[iid].clone(), if ok6 { 1.0 } else { 0.0 })
                        })
                        .collect();
                    metric_with_cluster_ci("fantom_all_star", &star_obs, seed)
                }
                Ok(None) => not_scored_metric("fantom_all_star", "scorer_model_unavailable"),
                Err(e) if e.to_string().contains("scorer_model_unavailable") => {
                    not_scored_metric("fantom_all_star", "scorer_model_unavailable")
                }
                Err(e) => return Err(e),
            }
        } else {
            not_scored_metric("fantom_all_star", "scorer_model_unavailable")
        };

        Ok(ScorerReport {
            dataset_id: "fantom".to_owned(),
            scorer_version: self.scorer_version().to_owned(),
            metrics: vec![
                all_star_metric,
                metric_with_cluster_ci("fantom_all", &all_5_obs, seed),
            ],
            judge_model: None,
            judge_prompt_version: None,
        })
    }
}
