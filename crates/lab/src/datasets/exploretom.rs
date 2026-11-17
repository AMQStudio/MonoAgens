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
        "2.0.0"
    }

    fn convert(
        &self,
        dataset_dir: &Path,
        manifest_entry: &DatasetManifestEntry,
    ) -> Result<ConvertedDataset, LabError> {
        let csv_p = dataset_dir.join("ExploreToM-data-sample.csv");
        let json_p = dataset_dir.join("train.json");
        let jsonl_p = dataset_dir.join("train.jsonl");
        let rows: Vec<RawExploreTomRow> = if csv_p.is_file() {
            let mut rdr = csv::ReaderBuilder::new()
                .has_headers(true)
                .from_path(&csv_p)
                .map_err(|e| LabError::Serialization(format!("exploretom CSV open: {e}")))?;
            let headers = rdr
                .headers()
                .map_err(|e| LabError::Serialization(format!("exploretom CSV headers: {e}")))?
                .clone();
            let col_idx = |name: &str| headers.iter().position(|h| h == name);
            let idx_story = col_idx("story_structure");
            let idx_infilled = col_idx("infilled_story");
            let idx_q = col_idx("question").ok_or_else(|| {
                LabError::Serialization("exploretom CSV missing 'question'".to_owned())
            })?;
            let idx_ans = col_idx("expected_answer").ok_or_else(|| {
                LabError::Serialization("exploretom CSV missing 'expected_answer'".to_owned())
            })?;
            let idx_order = col_idx("qprop=nth_order");
            let idx_gid = col_idx("sprop=global_idx");
            let idx_fb = col_idx("sprop=is_false_belief_story_1st");

            let mut out = Vec::new();
            for (r_i, rec_res) in rdr.records().enumerate() {
                let rec = rec_res.map_err(|e| {
                    LabError::Serialization(format!("exploretom CSV row {r_i}: {e}"))
                })?;
                let story_text = idx_story
                    .and_then(|i| rec.get(i))
                    .filter(|s| !s.trim().is_empty())
                    .or_else(|| idx_infilled.and_then(|i| rec.get(i)))
                    .unwrap_or("");
                let story_steps: Vec<String> = story_text
                    .split('\n')
                    .flat_map(|line| line.split(". "))
                    .map(|s| s.trim().trim_end_matches('.').trim())
                    .filter(|s| !s.is_empty())
                    .map(|s| format!("{s}."))
                    .collect();
                let nth_order = idx_order
                    .and_then(|i| rec.get(i))
                    .and_then(|s| s.trim().parse::<u8>().ok());
                let row_id = idx_gid
                    .and_then(|i| rec.get(i))
                    .filter(|s| !s.trim().is_empty())
                    .map(|g| format!("etom_{g}_{r_i}"))
                    .or_else(|| Some(format!("etom_{r_i:05}")));
                let is_fb = idx_fb
                    .and_then(|i| rec.get(i))
                    .is_some_and(|s| s.eq_ignore_ascii_case("true"));
                out.push(RawExploreTomRow {
                    row_id,
                    story_steps,
                    actors: Vec::new(),
                    question: rec.get(idx_q).unwrap_or("").to_owned(),
                    expected_answer: rec.get(idx_ans).unwrap_or("").to_owned(),
                    nth_order,
                    accessibility: Some(if is_fb {
                        "false_belief".to_owned()
                    } else {
                        "accessible".to_owned()
                    }),
                    human_state: None,
                });
            }
            out
        } else if json_p.is_file() {
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
                    "no ExploreToM-data-sample.csv/train.json/train.jsonl under '{}'",
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
