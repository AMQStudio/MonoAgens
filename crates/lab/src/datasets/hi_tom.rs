//! `hi_tom` (`ying-hui-he/Hi-ToM_dataset`) Mode-1 converter with private `Tell` audiences & `standard_acc`/`joint_acc_0_to_4` scorer.

use std::collections::BTreeMap;
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
    build_mode1_tom_history, exact_match_normalized, metric_with_cluster_ci,
};

pub struct HiTomConverter;

#[derive(Deserialize)]
struct RawHiTomItem {
    #[serde(default)]
    story_id: Option<String>,
    story: Vec<String>,
    #[serde(default)]
    actors: Vec<String>,
    question: String,
    choices: Vec<String>,
    answer: String,
    order: u8,
    #[serde(default)]
    deception: bool,
}

impl Converter for HiTomConverter {
    fn dataset_id(&self) -> &str {
        "hi_tom"
    }

    fn converter_version(&self) -> &str {
        "1.0.0"
    }

    fn convert(
        &self,
        dataset_dir: &Path,
        manifest_entry: &DatasetManifestEntry,
    ) -> Result<ConvertedDataset, LabError> {
        let raw = fs::read_to_string(dataset_dir.join("Hi-ToM_data/Hi-ToM_data.json"))?;
        let rows: Vec<RawHiTomItem> = serde_json::from_str(&raw)
            .map_err(|e| LabError::Serialization(format!("hi_tom JSON: {e}")))?;

        let soul = ActorId::new("soul");
        let proctor = ActorId::new("lab_proctor");
        let probe_aud = Audience::new([proctor, soul]);

        // Group QA items by `story_id` (whole-story clusters for Joint Accuracy).
        let mut by_story: BTreeMap<String, Vec<RawHiTomItem>> = BTreeMap::new();
        for (idx, r) in rows.into_iter().enumerate() {
            let sid = r
                .story_id
                .clone()
                .unwrap_or_else(|| format!("hitom_story_{}", idx / 5));
            by_story.entry(sid).or_default().push(r);
        }

        let mut items = Vec::with_capacity(by_story.len());
        let mut gold_entries = Vec::new();

        for (sid, mut qas) in by_story {
            qas.sort_by_key(|q| q.order);
            let first = &qas[0];
            let (history, ev_refs) = build_mode1_tom_history(
                &first.story,
                &sid,
                &first.actors,
                1_700_000_000_000,
                1_000,
            );
            let place = PlaceId::new(format!("{sid}_main"));
            let probe_base = 1_700_000_000_000 + (history.len() as u64 + 1) * 1_000;
            let deception = qas.iter().any(|q| q.deception);

            let mut probes = Vec::with_capacity(qas.len());
            for (q_idx, qa) in qas.into_iter().enumerate() {
                let probe_id = format!("order_{}", qa.order);
                let prompt = format!("{}\nChoices: {}", qa.question, qa.choices.join(" | "));
                probes.push(ExternalProbe {
                    probe_id: probe_id.clone(),
                    percept: make_text_percept(
                        "lab_proctor",
                        place.as_str(),
                        probe_aud.members.iter().cloned(),
                        probe_base + (q_idx as u64) * 1_000,
                        prompt,
                    ),
                    isolated_checkpoint: true,
                    probe_scope: VisibleScope::new(probe_aud.clone(), Some(place.clone())),
                    expected_capability: None,
                });

                let mut metadata = BTreeMap::new();
                metadata.insert("order".to_owned(), qa.order.to_string());
                metadata.insert("deception".to_owned(), qa.deception.to_string());

                gold_entries.push(GoldEntry {
                    item_id: sid.clone(),
                    probe_id,
                    cluster_id: sid.clone(),
                    gold_answer: qa.answer.clone(),
                    accepted_answers: vec![qa.answer],
                    gold_evidence: ev_refs.clone(),
                    privacy_conflict: false,
                    category: Some(format!("order_{}", qa.order)),
                    metadata,
                });
            }

            items.push(ExternalItem {
                item_id: sid.clone(),
                cluster_id: sid,
                strata_key: format!("deception={deception}"),
                history,
                checkpoint_group: None,
                probes,
                history_mode: HistoryMode::StaticSilent,
                capabilities: Vec::new(),
            });
        }

        Ok(ConvertedDataset {
            dataset_id: "hi_tom".to_owned(),
            pinned_revision: manifest_entry
                .sources
                .first()
                .map_or_else(|| "pending_pin".to_owned(), |s| s.revision.clone()),
            converter_version: self.converter_version().to_owned(),
            items,
            gold_book: GoldBook {
                dataset_id: "hi_tom".to_owned(),
                entries: gold_entries,
            },
        })
    }
}

pub struct HiTomScorer;

impl Scorer for HiTomScorer {
    fn dataset_id(&self) -> &str {
        "hi_tom"
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
        let mut std_obs = Vec::new();
        let mut by_story: BTreeMap<&str, Vec<(u8, bool, bool)>> = BTreeMap::new();

        for pred in predictions {
            let entry = gold.find(&pred.item_id, &pred.probe_id).ok_or_else(|| {
                LabError::Protocol(format!(
                    "hi_tom missing gold for {}:{}",
                    pred.item_id, pred.probe_id
                ))
            })?;
            let order: u8 = entry
                .metadata
                .get("order")
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            let deception = entry.metadata.get("deception").map(String::as_str) == Some("true");
            let resp = pred.response_text.as_deref().unwrap_or("");
            let hit = exact_match_normalized(resp, &entry.accepted_answers) > 0.5;
            std_obs.push((pred.cluster_id.clone(), if hit { 1.0 } else { 0.0 }));
            by_story
                .entry(pred.cluster_id.as_str())
                .or_default()
                .push((order, deception, hit));
        }

        let mut joint_obs = Vec::new();
        for (story_id, mut results) in by_story {
            results.sort_by_key(|r| r.0);
            let any_dec = results.iter().any(|r| r.1);
            let joint_ok = results
                .iter()
                .filter(|(ord, _, _)| !any_dec || *ord > 0)
                .all(|(_, _, ok)| *ok);
            joint_obs.push((story_id.to_owned(), if joint_ok { 1.0 } else { 0.0 }));
        }

        Ok(ScorerReport {
            dataset_id: "hi_tom".to_owned(),
            scorer_version: self.scorer_version().to_owned(),
            metrics: vec![
                metric_with_cluster_ci("joint_acc_0_to_4", &joint_obs, seed),
                metric_with_cluster_ci("standard_acc", &std_obs, seed),
            ],
            judge_model: None,
            judge_prompt_version: None,
        })
    }
}
