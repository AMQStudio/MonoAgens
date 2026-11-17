//! `molweni` (`HIT-SCIR/Molweni`) MRC (`qa_f1`) & DP (`sdrt_link_f1`) converter & scorer.
//! Strictly excludes dyadic `MuTual` from multi-party claims.

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
use crate::datasets::scoring_utils::{binary_prf1, metric_with_cluster_ci, token_f1};

pub struct MolweniConverter;

/// Real upstream layout (`MRC(withDiscourse)/test.json`): `{ "data": { "title": ..., "dialogues": [...] } }`.
#[derive(Deserialize)]
struct RawMolweniMrcRoot {
    data: RawMolweniMrcData,
}

#[derive(Deserialize)]
struct RawMolweniMrcData {
    #[allow(dead_code)]
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    dialogues: Vec<RawMolweniMrcDialogue>,
}

#[derive(Deserialize)]
struct RawMolweniMrcDialogue {
    edus: Vec<RawMolweniEdu>,
    #[allow(dead_code)]
    #[serde(default)]
    context: Option<String>,
    qas: Vec<RawMolweniQa>,
}

#[derive(Deserialize)]
struct RawMolweniEdu {
    speaker: String,
    text: String,
}

#[derive(Deserialize)]
struct RawMolweniQa {
    id: String,
    question: String,
    #[serde(default)]
    answers: Vec<RawMolweniAns>,
    #[serde(default)]
    is_impossible: bool,
}

#[derive(Deserialize)]
struct RawMolweniAns {
    text: String,
}

#[derive(Deserialize)]
struct RawMolweniDpDlg {
    id: String,
    edus: Vec<RawMolweniEdu>,
    #[serde(default)]
    relations: Vec<RawMolweniRel>,
}

#[derive(Deserialize)]
struct RawMolweniRel {
    x: usize,
    y: usize,
    #[allow(dead_code)]
    #[serde(rename = "type", default)]
    rel_type: Option<String>,
}

impl Converter for MolweniConverter {
    fn dataset_id(&self) -> &str {
        "molweni"
    }

    fn converter_version(&self) -> &str {
        "1.2.0"
    }

    fn convert(
        &self,
        dataset_dir: &Path,
        manifest_entry: &DatasetManifestEntry,
    ) -> Result<ConvertedDataset, LabError> {
        let soul = ActorId::new("soul");
        let proctor = ActorId::new("lab_proctor");
        let base_ms: u64 = 1_700_000_000_000;

        let mut items = Vec::new();
        let mut gold_entries = Vec::new();

        // 1. `MRC(withDiscourse)/test.json` (real upstream path).
        let mrc_path = dataset_dir.join("MRC(withDiscourse)/test.json");
        if mrc_path.is_file() {
            let raw = fs::read_to_string(&mrc_path)?;
            let parsed: RawMolweniMrcRoot = serde_json::from_str(&raw)
                .map_err(|e| LabError::Serialization(format!("molweni MRC JSON: {e}")))?;

            for (d_idx, dialogue) in parsed.data.dialogues.into_iter().enumerate() {
                let dlg_id = format!("mrc_{d_idx:04}");
                let place = PlaceId::new(dlg_id.clone());
                let mut speakers: BTreeSet<ActorId> = dialogue
                    .edus
                    .iter()
                    .map(|e| ActorId::new(&e.speaker))
                    .collect();
                speakers.insert(proctor.clone());
                speakers.insert(soul.clone());
                let hist_aud = Audience::new(speakers);
                let probe_aud = Audience::new([proctor.clone(), soul.clone()]);

                let mut history = Vec::with_capacity(dialogue.edus.len());
                let mut ev_refs = Vec::with_capacity(dialogue.edus.len());
                for (e_idx, edu) in dialogue.edus.iter().enumerate() {
                    history.push(make_text_percept(
                        &edu.speaker,
                        place.as_str(),
                        hist_aud.members.iter().cloned(),
                        base_ms + (e_idx as u64) * 1_000,
                        &edu.text,
                    ));
                    ev_refs.push(GoldEvidenceRef {
                        turn_index: e_idx,
                        audience: hist_aud.clone(),
                        place: place.clone(),
                    });
                }

                let mut probes = Vec::with_capacity(dialogue.qas.len());
                for (q_idx, qa) in dialogue.qas.into_iter().enumerate() {
                    let probe_id = qa.id.clone();
                    probes.push(ExternalProbe {
                        probe_id: probe_id.clone(),
                        percept: make_text_percept(
                            "lab_proctor",
                            place.as_str(),
                            probe_aud.members.iter().cloned(),
                            base_ms + ((history.len() + q_idx + 1) as u64) * 1_000,
                            &qa.question,
                        ),
                        isolated_checkpoint: true,
                        probe_scope: VisibleScope::new(probe_aud.clone(), Some(place.clone())),
                        expected_capability: None,
                    });

                    let accepted: Vec<String> = qa.answers.into_iter().map(|a| a.text).collect();
                    let gold_answer = accepted.first().cloned().unwrap_or_default();
                    let mut metadata = BTreeMap::new();
                    metadata.insert("task".to_owned(), "mrc".to_owned());
                    metadata.insert("is_impossible".to_owned(), qa.is_impossible.to_string());

                    gold_entries.push(GoldEntry {
                        item_id: dlg_id.clone(),
                        probe_id,
                        cluster_id: dlg_id.clone(),
                        gold_answer,
                        accepted_answers: accepted,
                        gold_evidence: ev_refs.clone(),
                        privacy_conflict: false,
                        category: Some("mrc".to_owned()),
                        metadata,
                    });
                }

                items.push(ExternalItem {
                    item_id: dlg_id.clone(),
                    cluster_id: dlg_id,
                    strata_key: "mrc".to_owned(),
                    history,
                    checkpoint_group: None,
                    probes,
                    history_mode: HistoryMode::StaticSilent,
                    capabilities: Vec::new(),
                });
            }
        }

        // 2. DP/test.json
        let dp_path = dataset_dir.join("DP/test.json");
        if dp_path.is_file() {
            let raw = fs::read_to_string(&dp_path)?;
            let dlgs: Vec<RawMolweniDpDlg> = serde_json::from_str(&raw)
                .map_err(|e| LabError::Serialization(format!("molweni DP JSON: {e}")))?;

            for dlg in dlgs {
                let dlg_id = format!("dp_{}", dlg.id);
                let place = PlaceId::new(dlg_id.clone());
                let mut speakers: BTreeSet<ActorId> =
                    dlg.edus.iter().map(|e| ActorId::new(&e.speaker)).collect();
                speakers.insert(proctor.clone());
                speakers.insert(soul.clone());
                let hist_aud = Audience::new(speakers);
                let probe_aud = Audience::new([proctor.clone(), soul.clone()]);

                let mut history = Vec::with_capacity(dlg.edus.len());
                let mut ev_refs = Vec::with_capacity(dlg.edus.len());
                for (e_idx, edu) in dlg.edus.iter().enumerate() {
                    history.push(make_text_percept(
                        &edu.speaker,
                        place.as_str(),
                        hist_aud.members.iter().cloned(),
                        base_ms + (e_idx as u64) * 1_000,
                        format!("[edu_{e_idx}] {}", edu.text),
                    ));
                    ev_refs.push(GoldEvidenceRef {
                        turn_index: e_idx,
                        audience: hist_aud.clone(),
                        place: place.clone(),
                    });
                }

                let probe = ExternalProbe {
                    probe_id: "dp_links".to_owned(),
                    percept: make_text_percept(
                        "lab_proctor",
                        place.as_str(),
                        probe_aud.members.iter().cloned(),
                        base_ms + ((history.len() + 1) as u64) * 1_000,
                        "Emit directed SDRT dependency edges (x->y) between EDUs.",
                    ),
                    isolated_checkpoint: true,
                    probe_scope: VisibleScope::new(probe_aud.clone(), Some(place.clone())),
                    expected_capability: None,
                };

                let edge_strs: Vec<String> = dlg
                    .relations
                    .iter()
                    .map(|r| format!("{}->{}", r.x, r.y))
                    .collect();
                let mut metadata = BTreeMap::new();
                metadata.insert("task".to_owned(), "dp".to_owned());

                gold_entries.push(GoldEntry {
                    item_id: dlg_id.clone(),
                    probe_id: "dp_links".to_owned(),
                    cluster_id: dlg_id.clone(),
                    gold_answer: edge_strs.join(","),
                    accepted_answers: edge_strs,
                    gold_evidence: ev_refs,
                    privacy_conflict: false,
                    category: Some("dp".to_owned()),
                    metadata,
                });

                items.push(ExternalItem {
                    item_id: dlg_id.clone(),
                    cluster_id: dlg_id,
                    strata_key: "dp".to_owned(),
                    history,
                    checkpoint_group: None,
                    probes: vec![probe],
                    history_mode: HistoryMode::StaticSilent,
                    capabilities: Vec::new(),
                });
            }
        }

        if items.is_empty() {
            return Err(LabError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!(
                    "no Molweni MRC/test.json or DP/test.json under '{}'",
                    dataset_dir.display()
                ),
            )));
        }

        Ok(ConvertedDataset {
            dataset_id: "molweni".to_owned(),
            pinned_revision: manifest_entry
                .sources
                .first()
                .map_or_else(|| "pending_pin".to_owned(), |s| s.revision.clone()),
            converter_version: self.converter_version().to_owned(),
            items,
            gold_book: GoldBook {
                dataset_id: "molweni".to_owned(),
                entries: gold_entries,
            },
        })
    }
}

pub struct MolweniScorer;

fn parse_directed_edges(text: &str) -> BTreeSet<(usize, usize)> {
    let mut out = BTreeSet::new();
    for tok in text.split(|c: char| c.is_whitespace() || c == ',' || c == ';') {
        if let Some((a, b)) = tok.trim().split_once("->")
            && let (Ok(x), Ok(y)) = (a.trim().parse::<usize>(), b.trim().parse::<usize>())
        {
            out.insert((x, y));
        }
    }
    out
}

impl Scorer for MolweniScorer {
    fn dataset_id(&self) -> &str {
        "molweni"
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
        let mut mrc_obs = Vec::new();
        let mut dp_tp = 0usize;
        let mut dp_fp = 0usize;
        let mut dp_fn = 0usize;
        let mut dp_clusters = BTreeSet::new();

        for pred in predictions {
            let entry = gold.find(&pred.item_id, &pred.probe_id).ok_or_else(|| {
                LabError::Protocol(format!(
                    "molweni missing gold for {}:{}",
                    pred.item_id, pred.probe_id
                ))
            })?;
            let task = entry
                .metadata
                .get("task")
                .map(String::as_str)
                .unwrap_or("mrc");
            let resp = pred.response_text.as_deref().unwrap_or("");

            if task == "dp" {
                dp_clusters.insert(pred.cluster_id.clone());
                let gold_edges: BTreeSet<(usize, usize)> = entry
                    .accepted_answers
                    .iter()
                    .flat_map(|s| parse_directed_edges(s))
                    .collect();
                let pred_edges = parse_directed_edges(resp);
                let tp = gold_edges.intersection(&pred_edges).count();
                dp_tp += tp;
                dp_fp += pred_edges.len().saturating_sub(tp);
                dp_fn += gold_edges.len().saturating_sub(tp);
            } else {
                let is_imp =
                    entry.metadata.get("is_impossible").map(String::as_str) == Some("true");
                let f1 = if is_imp {
                    let lower = resp.to_ascii_lowercase();
                    let abstained = !pred.spoke
                        || lower.contains("unanswerable")
                        || lower.contains("no information")
                        || lower.contains("cannot answer");
                    if abstained { 1.0 } else { 0.0 }
                } else {
                    entry
                        .accepted_answers
                        .iter()
                        .map(|g| token_f1(resp, g, false))
                        .fold(0.0_f64, f64::max)
                };
                mrc_obs.push((pred.cluster_id.clone(), f1));
            }
        }

        let (_, _, sdrt_link_f1) = binary_prf1(dp_tp, dp_fp, dp_fn);

        Ok(ScorerReport {
            dataset_id: "molweni".to_owned(),
            scorer_version: self.scorer_version().to_owned(),
            metrics: vec![
                metric_with_cluster_ci("qa_f1", &mrc_obs, seed),
                ScorerMetric {
                    name: "sdrt_link_f1".to_owned(),
                    value: Some(sdrt_link_f1),
                    ci95_low: None,
                    ci95_high: None,
                    n_clusters: Some(dp_clusters.len()),
                    not_scored_reason: None,
                },
            ],
            judge_model: None,
            judge_prompt_version: None,
        })
    }
}
