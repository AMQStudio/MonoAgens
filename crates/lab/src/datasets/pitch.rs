//! `pitch` (`Social-Futures-Lab/pitch`) causal turn-by-turn intervention timing converter & scorer.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use monoagens_protocol::{ActorId, Audience, PlaceId, VisibleScope};
use serde::Deserialize;

use crate::LabError;
use crate::datasets::framework::{
    ConvertedDataset, Converter, ExternalItem, ExternalPrediction, ExternalProbe, GoldBook,
    GoldEntry, GoldEvidenceRef, HistoryMode, JudgeExecutionConfig, Scorer, ScorerReport,
    make_text_percept,
};
use crate::datasets::manifest::DatasetManifestEntry;
use crate::datasets::scoring_utils::metric_with_cluster_ci;

pub struct PitchConverter;

#[derive(Deserialize)]
struct RawPitchItem {
    knowledge_item_id: String,
    conversation_id: String,
    #[serde(default)]
    scenario: Option<String>,
    turns: Vec<RawPitchTurn>,
    neutralized_ideal_response: String,
    ground_truth: RawPitchGroundTruth,
}

#[derive(Deserialize)]
struct RawPitchTurn {
    #[serde(default)]
    turn_number: Option<usize>,
    speaker: String,
    text: String,
    #[serde(default)]
    timestamp_ms: Option<u64>,
}

#[derive(Deserialize)]
struct RawPitchGroundTruth {
    #[serde(default)]
    do_not_surface: bool,
    #[serde(default)]
    accepted_best_turns: Vec<usize>,
    #[serde(default)]
    acceptable_windows: Vec<[usize; 2]>,
    #[serde(default)]
    consensus_best_turn: Option<usize>,
    #[serde(default)]
    union_window_start: Option<usize>,
    #[serde(default)]
    union_window_end: Option<usize>,
}

impl Converter for PitchConverter {
    fn dataset_id(&self) -> &str {
        "pitch"
    }

    fn converter_version(&self) -> &str {
        "2.2.0"
    }

    fn convert(
        &self,
        dataset_dir: &Path,
        manifest_entry: &DatasetManifestEntry,
    ) -> Result<ConvertedDataset, LabError> {
        let raw = fs::read_to_string(dataset_dir.join("data/pitch-benchmark-data.json"))?;
        let candidates: Vec<RawPitchItem> = serde_json::from_str(&raw)
            .map_err(|e| LabError::Serialization(format!("pitch JSON error: {e}")))?;

        let soul = ActorId::new("soul");
        let proctor = ActorId::new("lab_proctor");
        let mut items = Vec::with_capacity(candidates.len());
        let mut gold_entries = Vec::new();

        for cand in candidates {
            let domain = cand.scenario.unwrap_or_else(|| "planning".to_owned());
            let place = PlaceId::new(format!("pitch_{}", cand.conversation_id));
            let mut participants: BTreeSet<ActorId> = cand
                .turns
                .iter()
                .map(|t| ActorId::new(&t.speaker))
                .collect();
            participants.insert(proctor.clone());
            participants.insert(soul.clone());
            let scene_aud = Audience::new(participants);
            let probe_aud = Audience::new([proctor.clone(), soul.clone()]);
            let base_ms: u64 = 1_700_000_000_000;

            let item_id = format!("{}_{}", cand.conversation_id, cand.knowledge_item_id);
            let mut history = Vec::with_capacity(cand.turns.len() + 1);
            let mut ev_refs = Vec::with_capacity(cand.turns.len() + 1);

            // Stage candidate knowledge item without any timing hint or raw ground-truth answer.
            history.push(make_text_percept(
                "lab_proctor",
                place.as_str(),
                scene_aud.members.iter().cloned(),
                base_ms,
                format!(
                    "Pending candidate contribution: {}",
                    cand.neutralized_ideal_response
                ),
            ));
            ev_refs.push(GoldEvidenceRef {
                turn_index: 0,
                audience: scene_aud.clone(),
                place: place.clone(),
            });

            let best_turns_str: Vec<String> = cand
                .ground_truth
                .accepted_best_turns
                .iter()
                .map(ToString::to_string)
                .collect();
            let ideal_fallback = cand
                .ground_truth
                .consensus_best_turn
                .or_else(|| cand.ground_truth.accepted_best_turns.first().copied());
            let windows_str: Vec<String> = cand
                .ground_truth
                .acceptable_windows
                .iter()
                .map(|w| format!("{}-{}", w[0], w[1]))
                .collect();

            let mut probes = Vec::with_capacity(cand.turns.len());
            for (t_idx, turn) in cand.turns.iter().enumerate() {
                let turn_num = turn.turn_number.unwrap_or(t_idx + 1);
                let ts = turn
                    .timestamp_ms
                    .unwrap_or(base_ms + ((t_idx + 1) as u64) * 1_000);
                let turn_percept = make_text_percept(
                    &turn.speaker,
                    place.as_str(),
                    scene_aud.members.iter().cloned(),
                    ts,
                    &turn.text,
                );
                history.push(turn_percept);
                ev_refs.push(GoldEvidenceRef {
                    turn_index: t_idx + 1,
                    audience: scene_aud.clone(),
                    place: place.clone(),
                });

                let probe_id = format!("turn_{turn_num}");
                probes.push(ExternalProbe {
                    probe_id: probe_id.clone(),
                    percept: make_text_percept(
                        "lab_proctor",
                        place.as_str(),
                        probe_aud.members.iter().cloned(),
                        ts + 100,
                        format!(
                            "Turn {turn_num}: should you surface the pending candidate contribution now? If yes, state the contribution; if not, answer SILENT."
                        ),
                    ),
                    isolated_checkpoint: false,
                    probe_scope: VisibleScope::new(probe_aud.clone(), Some(place.clone())),
                    expected_capability: None,
                });

                let mut metadata = BTreeMap::new();
                metadata.insert("turn_index".to_owned(), turn_num.to_string());
                metadata.insert(
                    "do_not_surface".to_owned(),
                    cand.ground_truth.do_not_surface.to_string(),
                );
                if !best_turns_str.is_empty() {
                    metadata.insert("accepted_best_turns".to_owned(), best_turns_str.join(","));
                }
                if let Some(it) = ideal_fallback {
                    metadata.insert("ideal_turn".to_owned(), it.to_string());
                }
                if !windows_str.is_empty() {
                    metadata.insert("acceptable_windows".to_owned(), windows_str.join(","));
                }
                if let Some(ws) = cand.ground_truth.union_window_start {
                    metadata.insert("window_start".to_owned(), ws.to_string());
                }
                if let Some(we) = cand.ground_truth.union_window_end {
                    metadata.insert("window_end".to_owned(), we.to_string());
                }

                gold_entries.push(GoldEntry {
                    item_id: item_id.clone(),
                    probe_id,
                    cluster_id: cand.conversation_id.clone(),
                    gold_answer: if cand.ground_truth.do_not_surface {
                        "do-not-surface".to_owned()
                    } else {
                        ideal_fallback.unwrap_or(0).to_string()
                    },
                    accepted_answers: Vec::new(),
                    gold_evidence: ev_refs.clone(),
                    privacy_conflict: false,
                    category: Some(domain.clone()),
                    metadata,
                });
            }

            items.push(ExternalItem {
                item_id,
                cluster_id: cand.conversation_id,
                strata_key: format!("{domain}:dns={}", cand.ground_truth.do_not_surface),
                history,
                checkpoint_group: None,
                probes,
                history_mode: HistoryMode::StaticSilent,
                capabilities: Vec::new(),
            });
        }

        Ok(ConvertedDataset {
            dataset_id: "pitch".to_owned(),
            pinned_revision: manifest_entry
                .sources
                .first()
                .map_or_else(|| "pending_pin".to_owned(), |s| s.revision.clone()),
            converter_version: self.converter_version().to_owned(),
            items,
            gold_book: GoldBook {
                dataset_id: "pitch".to_owned(),
                entries: gold_entries,
            },
        })
    }
}

pub struct PitchScorer;

impl Scorer for PitchScorer {
    fn dataset_id(&self) -> &str {
        "pitch"
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
        // Group turn predictions by item_id; first spoken turn is the predicted intervention turn.
        let mut by_item: BTreeMap<&str, Vec<&ExternalPrediction>> = BTreeMap::new();
        for p in predictions {
            by_item.entry(p.item_id.as_str()).or_default().push(p);
        }

        let mut best_obs = Vec::new();
        let mut win_obs = Vec::new();

        for (item_id, preds) in by_item {
            let mut spoken_turn: Option<usize> = None;
            let mut first_gold: Option<&GoldEntry> = None;
            for p in &preds {
                let g = gold.find(item_id, &p.probe_id).ok_or_else(|| {
                    LabError::Protocol(format!("pitch missing gold for {item_id}:{}", p.probe_id))
                })?;
                if first_gold.is_none() {
                    first_gold = Some(g);
                }
                let ti: usize = g
                    .metadata
                    .get("turn_index")
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);
                if p.spoke && spoken_turn.is_none() {
                    spoken_turn = Some(ti);
                }
            }
            let g = first_gold.expect("non-empty preds");
            let dns = g.metadata.get("do_not_surface").map(String::as_str) == Some("true");
            let (best_hit, win_hit) = if dns {
                let ok = spoken_turn.is_none();
                (ok, ok)
            } else {
                let accepted_best: Vec<usize> = g
                    .metadata
                    .get("accepted_best_turns")
                    .map(|s| {
                        s.split(',')
                            .filter_map(|x| x.trim().parse::<usize>().ok())
                            .collect()
                    })
                    .filter(|v: &Vec<usize>| !v.is_empty())
                    .or_else(|| {
                        g.metadata
                            .get("ideal_turn")
                            .and_then(|s| s.parse::<usize>().ok())
                            .map(|t| vec![t])
                    })
                    .unwrap_or_default();
                let windows: Vec<(usize, usize)> = g
                    .metadata
                    .get("acceptable_windows")
                    .map(|s| {
                        s.split(',')
                            .filter_map(|seg| {
                                let (a, b) = seg.split_once('-')?;
                                Some((a.trim().parse().ok()?, b.trim().parse().ok()?))
                            })
                            .collect()
                    })
                    .filter(|v: &Vec<(usize, usize)>| !v.is_empty())
                    .unwrap_or_else(|| {
                        let ws = g
                            .metadata
                            .get("window_start")
                            .and_then(|s| s.parse::<usize>().ok())
                            .unwrap_or(0);
                        let we = g
                            .metadata
                            .get("window_end")
                            .and_then(|s| s.parse::<usize>().ok())
                            .unwrap_or(0);
                        vec![(ws, we)]
                    });
                match spoken_turn {
                    Some(t) => (
                        accepted_best.contains(&t),
                        windows.iter().any(|&(ws, we)| t >= ws && t <= we),
                    ),
                    None => (false, false),
                }
            };
            best_obs.push((g.cluster_id.clone(), if best_hit { 1.0 } else { 0.0 }));
            win_obs.push((g.cluster_id.clone(), if win_hit { 1.0 } else { 0.0 }));
        }

        Ok(ScorerReport {
            dataset_id: "pitch".to_owned(),
            scorer_version: self.scorer_version().to_owned(),
            metrics: vec![
                metric_with_cluster_ci("live_best_turn_acc", &best_obs, seed),
                metric_with_cluster_ci("in_window_acc", &win_obs, seed),
            ],
            judge_model: None,
            judge_prompt_version: None,
        })
    }
}
