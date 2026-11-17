//! `exploretom` (`facebookresearch/ExploreToM`) Mode-1 DSL converter & `exact_match_acc` scorer.

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

pub struct ExploreTomConverter;

#[derive(Deserialize)]
struct RawExploreTomRow {
    #[serde(default)]
    row_id: Option<String>,
    story_steps: Vec<String>,
    #[serde(default)]
    actors: Vec<String>,
    question: String,
    expected_answer: String,
    #[serde(default)]
    nth_order: Option<u8>,
    #[serde(default)]
    accessibility: Option<String>,
    #[allow(dead_code)]
    #[serde(default, rename = "Human_State")]
    human_state: Option<String>,
}

impl Converter for ExploreTomConverter {
    fn dataset_id(&self) -> &str {
        "exploretom"
    }

    fn converter_version(&self) -> &str {
        "1.0.0"
    }

    fn convert(
        &self,
        dataset_dir: &Path,
        manifest_entry: &DatasetManifestEntry,
    ) -> Result<ConvertedDataset, LabError> {
        let json_p = dataset_dir.join("train.json");
        let jsonl_p = dataset_dir.join("train.jsonl");
        let rows: Vec<RawExploreTomRow> = if json_p.is_file() {
            serde_json::from_str(&fs::read_to_string(&json_p)?)
                .map_err(|e| LabError::Serialization(format!("exploretom JSON: {e}")))?
        } else if jsonl_p.is_file() {
            fs::read_to_string(&jsonl_p)?
                .lines()
                .filter(|l| !l.trim().is_empty())
                .map(|l| {
                    serde_json::from_str(l).map_err(|e| LabError::Serialization(e.to_string()))
                })
                .collect::<Result<Vec<_>, _>>()?
        } else {
            return Err(LabError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!(
                    "no train.json/train.jsonl under '{}'",
                    dataset_dir.display()
                ),
            )));
        };

        let soul = ActorId::new("soul");
        let proctor = ActorId::new("lab_proctor");
        let probe_aud = Audience::new([proctor, soul]);

        let mut items = Vec::with_capacity(rows.len());
        let mut gold_entries = Vec::with_capacity(rows.len());

        for (idx, row) in rows.into_iter().enumerate() {
            let item_id = row.row_id.unwrap_or_else(|| format!("etom_{idx:05}"));
            let (history, ev_refs) = build_mode1_tom_history(
                &row.story_steps,
                &item_id,
                &row.actors,
                1_700_000_000_000,
                1_000,
            );
            let place = PlaceId::new(format!("{item_id}_main"));
            let probe = ExternalProbe {
                probe_id: "q0".to_owned(),
                percept: make_text_percept(
                    "lab_proctor",
                    place.as_str(),
                    probe_aud.members.iter().cloned(),
                    1_700_000_000_000 + (history.len() as u64 + 1) * 1_000,
                    &row.question,
                ),
                isolated_checkpoint: true,
                probe_scope: VisibleScope::new(probe_aud.clone(), Some(place)),
                expected_capability: None,
            };

            let order_str = row.nth_order.unwrap_or(1).to_string();
            let acc_str = row.accessibility.unwrap_or_else(|| "accessible".to_owned());
            let mut metadata = BTreeMap::new();
            metadata.insert("nth_order".to_owned(), order_str.clone());
            metadata.insert("accessibility".to_owned(), acc_str.clone());

            gold_entries.push(GoldEntry {
                item_id: item_id.clone(),
                probe_id: "q0".to_owned(),
                cluster_id: item_id.clone(),
                gold_answer: row.expected_answer.clone(),
                accepted_answers: vec![row.expected_answer],
                gold_evidence: ev_refs,
                privacy_conflict: false,
                category: Some(acc_str.clone()),
                metadata,
            });

            items.push(ExternalItem {
                item_id: item_id.clone(),
                cluster_id: item_id,
                strata_key: format!("order={order_str}:{acc_str}"),
                history,
                checkpoint_group: None,
                probes: vec![probe],
                history_mode: HistoryMode::StaticSilent,
                capabilities: Vec::new(),
            });
        }

        Ok(ConvertedDataset {
            dataset_id: "exploretom".to_owned(),
            pinned_revision: manifest_entry
                .sources
                .first()
                .map_or_else(|| "pending_pin".to_owned(), |s| s.revision.clone()),
            converter_version: self.converter_version().to_owned(),
            items,
            gold_book: GoldBook {
                dataset_id: "exploretom".to_owned(),
                entries: gold_entries,
            },
        })
    }
}

pub struct ExploreTomScorer;

impl Scorer for ExploreTomScorer {
    fn dataset_id(&self) -> &str {
        "exploretom"
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
                LabError::Protocol(format!("exploretom missing gold for {}", pred.item_id))
            })?;
            let resp = pred.response_text.as_deref().unwrap_or("");
            let hit = exact_match_normalized(resp, &entry.accepted_answers);
            obs.push((pred.cluster_id.clone(), hit));
        }

        Ok(ScorerReport {
            dataset_id: "exploretom".to_owned(),
            scorer_version: self.scorer_version().to_owned(),
            metrics: vec![metric_with_cluster_ci("exact_match_acc", &obs, seed)],
            judge_model: None,
            judge_prompt_version: None,
        })
    }
}
