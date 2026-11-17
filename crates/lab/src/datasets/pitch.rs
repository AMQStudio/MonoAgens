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
struct RawPitchFile {
    conversations: Vec<RawPitchConv>,
}

#[derive(Deserialize)]
struct RawPitchConv {
    conv_id: String,
    #[serde(default)]
    task_domain: Option<String>,
    turns: Vec<RawPitchTurn>,
    candidates: Vec<RawPitchCandidate>,
}

#[derive(Deserialize)]
struct RawPitchTurn {
    speaker: String,
    text: String,
    #[serde(default)]
    timestamp_ms: Option<u64>,
}

#[derive(Deserialize)]
struct RawPitchCandidate {
    candidate_id: String,
    content: String,
    #[serde(default)]
    ideal_turn: Option<usize>,
    #[serde(default)]
    window_start: Option<usize>,
    #[serde(default)]
    window_end: Option<usize>,
    #[serde(default)]
    do_not_surface: bool,
}

impl Converter for PitchConverter {
    fn dataset_id(&self) -> &str {
        "pitch"
    }

    fn converter_version(&self) -> &str {
        "1.0.0"
    }

    fn convert(
        &self,
        dataset_dir: &Path,
        manifest_entry: &DatasetManifestEntry,
    ) -> Result<ConvertedDataset, LabError> {
        let raw = fs::read_to_string(dataset_dir.join("data/pitch-benchmark-data.json"))?;
        let parsed: RawPitchFile = serde_json::from_str(&raw)
            .map_err(|e| LabError::Serialization(format!("pitch JSON error: {e}")))?;

        let soul = ActorId::new("soul");
        let proctor = ActorId::new("lab_proctor");
        let mut items = Vec::new();
        let mut gold_entries = Vec::new();

        for conv in parsed.conversations {
            let domain = conv.task_domain.unwrap_or_else(|| "planning".to_owned());
            let place = PlaceId::new(format!("pitch_{}", conv.conv_id));
            let mut participants: BTreeSet<ActorId> = conv
                .turns
                .iter()
                .map(|t| ActorId::new(&t.speaker))
                .collect();
            participants.insert(proctor.clone());
            participants.insert(soul.clone());
            let scene_aud = Audience::new(participants);
            let probe_aud = Audience::new([proctor.clone(), soul.clone()]);
            let base_ms: u64 = 1_700_000_000_000;

            for cand in conv.candidates {
                let item_id = format!("{}_{}", conv.conv_id, cand.candidate_id);
                let mut history = Vec::with_capacity(conv.turns.len() + 1);
                let mut ev_refs = Vec::with_capacity(conv.turns.len() + 1);

                // Stage candidate knowledge item without any timing hint.
                history.push(make_text_percept(
                    "lab_proctor",
                    place.as_str(),
                    scene_aud.members.iter().cloned(),
                    base_ms,
                    format!("Pending candidate contribution: {}", cand.content),
                ));
                ev_refs.push(GoldEvidenceRef {
                    turn_index: 0,
                    audience: scene_aud.clone(),
                    place: place.clone(),
                });

                let mut probes = Vec::with_capacity(conv.turns.len());
                for (t_idx, turn) in conv.turns.iter().enumerate() {
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

                    let probe_id = format!("turn_{t_idx}");
                    probes.push(ExternalProbe {
                        probe_id: probe_id.clone(),
                        percept: make_text_percept(
                            "lab_proctor",
                            place.as_str(),
                            probe_aud.members.iter().cloned(),
                            ts + 100,
                            format!("Turn {t_idx}: surface candidate or stay silent."),
                        ),
                        isolated_checkpoint: false,
                        probe_scope: VisibleScope::new(probe_aud.clone(), Some(place.clone())),
                        expected_capability: None,
                    });

                    let mut metadata = BTreeMap::new();
                    metadata.insert("turn_index".to_owned(), t_idx.to_string());
                    metadata.insert("do_not_surface".to_owned(), cand.do_not_surface.to_string());
                    if let Some(it) = cand.ideal_turn {
                        metadata.insert("ideal_turn".to_owned(), it.to_string());
                    }
                    if let Some(ws) = cand.window_start {
                        metadata.insert("window_start".to_owned(), ws.to_string());
                    }
                    if let Some(we) = cand.window_end {
                        metadata.insert("window_end".to_owned(), we.to_string());
                    }

                    gold_entries.push(GoldEntry {
                        item_id: item_id.clone(),
                        probe_id,
                        cluster_id: conv.conv_id.clone(),
                        gold_answer: if cand.do_not_surface {
                            "do-not-surface".to_owned()
                        } else {
                            cand.ideal_turn.unwrap_or(0).to_string()
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
                    cluster_id: conv.conv_id.clone(),
                    strata_key: format!("{domain}:dns={}", cand.do_not_surface),
                    history,
                    checkpoint_group: None,
                    probes,
                    history_mode: HistoryMode::Interactive,
                    capabilities: Vec::new(),
                });
            }
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
                let ideal = g
                    .metadata
                    .get("ideal_turn")
                    .and_then(|s| s.parse::<usize>().ok());
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
                match spoken_turn {
                    Some(t) => (Some(t) == ideal, t >= ws && t <= we),
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
