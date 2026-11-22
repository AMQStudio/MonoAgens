//! `when2speak` (`duke-trust-lab/When2Speak`) 8-turn window converter & `macro_f1`/`fir`/`mir` scorer.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use monoagens_protocol::{ActorId, Audience, PlaceId, VisibleScope};
use serde::Deserialize;

use crate::LabError;
use crate::datasets::framework::{
    ConvertedDataset, Converter, ExternalItem, ExternalPrediction, ExternalProbe, GoldBook,
    GoldEntry, GoldEvidenceRef, HistoryMode, JudgeExecutionConfig, Scorer, ScorerItemOutcome,
    ScorerMetric, ScorerReport, make_text_percept,
};
use crate::datasets::manifest::DatasetManifestEntry;
use crate::datasets::scoring_utils::binary_prf1;

pub struct When2SpeakConverter;

#[derive(Deserialize)]
struct RawW2sRow {
    #[serde(default)]
    window_id: Option<String>,
    messages: Vec<RawW2sMsg>,
}

#[derive(Deserialize)]
struct RawW2sMsg {
    role: String,
    content: String,
    #[serde(default)]
    speaker: Option<String>,
}

impl Converter for When2SpeakConverter {
    fn dataset_id(&self) -> &str {
        "when2speak"
    }

    fn converter_version(&self) -> &str {
        "1.2.0"
    }

    fn convert(
        &self,
        dataset_dir: &Path,
        manifest_entry: &DatasetManifestEntry,
    ) -> Result<ConvertedDataset, LabError> {
        let raw = fs::read_to_string(dataset_dir.join("finetune_test_token.jsonl"))?;
        let soul = ActorId::new("soul");
        let proctor = ActorId::new("lab_proctor");
        let base_ms: u64 = 1_700_000_000_000;

        let mut items = Vec::new();
        let mut gold_entries = Vec::new();

        for (idx, line) in raw.lines().filter(|l| !l.trim().is_empty()).enumerate() {
            let row: RawW2sRow = serde_json::from_str(line)
                .map_err(|e| LabError::Serialization(format!("when2speak JSONL: {e}")))?;
            if row.messages.is_empty() {
                continue;
            }
            let item_id = row.window_id.unwrap_or_else(|| format!("w2s_{idx:05}"));
            let place = PlaceId::new(item_id.clone());

            let last_msg = row.messages.last().expect("non-empty");
            let label = if last_msg.role == "assistant" {
                last_msg.content.trim()
            } else {
                ">"
            };
            let is_speak = label.starts_with('<');
            let prefix_len = if last_msg.role == "assistant" {
                row.messages.len() - 1
            } else {
                row.messages.len()
            };

            let mut speakers: BTreeSet<ActorId> = BTreeSet::new();
            for (t_idx, m) in row.messages[..prefix_len].iter().enumerate() {
                let spk = m
                    .speaker
                    .clone()
                    .unwrap_or_else(|| format!("Speaker_{}", t_idx % 4));
                speakers.insert(ActorId::new(spk));
            }
            let speaker_count = speakers.len();
            speakers.insert(proctor.clone());
            speakers.insert(soul.clone());
            let hist_aud = Audience::new(speakers);
            let probe_aud = Audience::new([proctor.clone(), soul.clone()]);

            let mut history = Vec::with_capacity(prefix_len);
            let mut ev_refs = Vec::with_capacity(prefix_len);
            for (t_idx, m) in row.messages[..prefix_len].iter().enumerate() {
                let spk = m
                    .speaker
                    .clone()
                    .unwrap_or_else(|| format!("Speaker_{}", t_idx % 4));
                history.push(make_text_percept(
                    &spk,
                    place.as_str(),
                    hist_aud.members.iter().cloned(),
                    base_ms + (t_idx as u64) * 1_000,
                    &m.content,
                ));
                ev_refs.push(GoldEvidenceRef {
                    turn_index: t_idx,
                    audience: hist_aud.clone(),
                    place: place.clone(),
                });
            }

            let probe = ExternalProbe {
                probe_id: "turn_decision".to_owned(),
                percept: make_text_percept(
                    "lab_proctor",
                    place.as_str(),
                    probe_aud.members.iter().cloned(),
                    base_ms + (prefix_len as u64 + 1) * 1_000,
                    "Will the next turn be a SPEAK turn? Answer SPEAK or SILENT.",
                ),
                isolated_checkpoint: true,
                probe_scope: VisibleScope::new(probe_aud, Some(place)),
                expected_capability: None,
            };

            let mut metadata = BTreeMap::new();
            metadata.insert(
                "gold_label".to_owned(),
                if is_speak { "SPEAK" } else { "SILENT" }.to_owned(),
            );
            metadata.insert("speaker_count".to_owned(), speaker_count.to_string());

            gold_entries.push(GoldEntry {
                item_id: item_id.clone(),
                probe_id: "turn_decision".to_owned(),
                cluster_id: item_id.clone(),
                gold_answer: if is_speak { "<" } else { ">" }.to_owned(),
                accepted_answers: vec![if is_speak { "<" } else { ">" }.to_owned()],
                gold_evidence: ev_refs,
                privacy_conflict: false,
                category: Some(if is_speak { "SPEAK" } else { "SILENT" }.to_owned()),
                metadata,
            });

            items.push(ExternalItem {
                item_id: item_id.clone(),
                cluster_id: item_id,
                strata_key: format!(
                    "{}:spk={speaker_count}",
                    if is_speak { "SPEAK" } else { "SILENT" }
                ),
                history,
                checkpoint_group: None,
                probes: vec![probe],
                history_mode: HistoryMode::StaticSilent,
                capabilities: Vec::new(),
            });
        }

        Ok(ConvertedDataset {
            dataset_id: "when2speak".to_owned(),
            pinned_revision: manifest_entry
                .sources
                .first()
                .map_or_else(|| "pending_pin".to_owned(), |s| s.revision.clone()),
            converter_version: self.converter_version().to_owned(),
            items,
            gold_book: GoldBook {
                dataset_id: "when2speak".to_owned(),
                entries: gold_entries,
            },
        })
    }
}

pub struct When2SpeakScorer;

impl Scorer for When2SpeakScorer {
    fn dataset_id(&self) -> &str {
        "when2speak"
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
        let mut tp = 0usize;
        let mut fp = 0usize;
        let mut tn = 0usize;
        let mut fn_ = 0usize;

        for pred in predictions {
            let entry = gold.find(&pred.item_id, &pred.probe_id).ok_or_else(|| {
                LabError::Protocol(format!("when2speak missing gold for {}", pred.item_id))
            })?;
            let gold_speak = entry.metadata.get("gold_label").map(String::as_str) == Some("SPEAK");
            let predicted_speak = if !pred.spoke {
                false
            } else {
                let upper = pred
                    .response_text
                    .as_deref()
                    .unwrap_or("")
                    .trim()
                    .to_ascii_uppercase();
                !(upper == "SILENT"
                    || upper == ">"
                    || upper.starts_with("SILENT")
                    || upper.starts_with('>'))
            };
            match (gold_speak, predicted_speak) {
                (true, true) => tp += 1,
                (false, true) => fp += 1,
                (false, false) => tn += 1,
                (true, false) => fn_ += 1,
            }
            let hit = gold_speak == predicted_speak;
            judge.record_outcome(ScorerItemOutcome {
                item_id: pred.item_id.clone(),
                probe_id: pred.probe_id.clone(),
                cluster_id: pred.cluster_id.clone(),
                category: entry.category.clone(),
                metric_name: "when2speak_turn".to_owned(),
                score: Some(if hit { 1.0 } else { 0.0 }),
                passed: Some(hit),
                judge_prompt: None,
                judge_verdict: None,
                failed: false,
                failure_reason: None,
            });
        }

        let (_, _, f1_speak) = binary_prf1(tp, fp, fn_);
        let (_, _, f1_silent) = binary_prf1(tn, fn_, fp);
        let macro_f1 = 0.5 * (f1_speak + f1_silent);
        let fir = if fp + tn == 0 {
            0.0
        } else {
            fp as f64 / (fp + tn) as f64
        };
        let mir = if fn_ + tp == 0 {
            0.0
        } else {
            fn_ as f64 / (fn_ + tp) as f64
        };

        Ok(ScorerReport {
            dataset_id: "when2speak".to_owned(),
            scorer_version: self.scorer_version().to_owned(),
            metrics: vec![
                ScorerMetric {
                    name: "macro_f1".to_owned(),
                    value: Some(macro_f1),
                    ci95_low: None,
                    ci95_high: None,
                    n_clusters: Some(predictions.len()),
                    not_scored_reason: None,
                },
                ScorerMetric {
                    name: "fir".to_owned(),
                    value: Some(fir),
                    ci95_low: None,
                    ci95_high: None,
                    n_clusters: Some(predictions.len()),
                    not_scored_reason: None,
                },
                ScorerMetric {
                    name: "mir".to_owned(),
                    value: Some(mir),
                    ci95_low: None,
                    ci95_high: None,
                    n_clusters: Some(predictions.len()),
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
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::datasets::framework::ScorerDetailRecorder;

    fn w2s_gold_entry(item_id: &str, gold_speak: bool) -> GoldEntry {
        let mut metadata = BTreeMap::new();
        metadata.insert(
            "gold_label".to_owned(),
            if gold_speak { "SPEAK" } else { "SILENT" }.to_owned(),
        );
        GoldEntry {
            item_id: item_id.to_owned(),
            probe_id: "turn_decision".to_owned(),
            cluster_id: item_id.to_owned(),
            gold_answer: if gold_speak { "<" } else { ">" }.to_owned(),
            accepted_answers: vec![if gold_speak { "<" } else { ">" }.to_owned()],
            gold_evidence: Vec::new(),
            privacy_conflict: false,
            category: Some(if gold_speak { "SPEAK" } else { "SILENT" }.to_owned()),
            metadata,
        }
    }

    fn w2s_pred(item_id: &str, spoke: bool, response_text: Option<&str>) -> ExternalPrediction {
        ExternalPrediction {
            item_id: item_id.to_owned(),
            probe_id: "turn_decision".to_owned(),
            cluster_id: item_id.to_owned(),
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
    fn chat_speak_silent_is_scored_as_silent_and_recorded_in_detail_recorder() {
        let gold = GoldBook {
            dataset_id: "when2speak".to_owned(),
            entries: vec![
                w2s_gold_entry("item_tn", false),
                w2s_gold_entry("item_fn", true),
                w2s_gold_entry("item_tn_gt", false),
                w2s_gold_entry("item_tp", true),
            ],
        };
        let preds = vec![
            w2s_pred("item_tn", true, Some("SILENT")),
            w2s_pred("item_fn", true, Some("  silent  ")),
            w2s_pred("item_tn_gt", true, Some(">")),
            w2s_pred("item_tp", true, Some("SPEAK: here is my reply")),
        ];

        let recorder = Arc::new(Mutex::new(ScorerDetailRecorder::default()));
        let judge = JudgeExecutionConfig {
            recorder: Some(Arc::clone(&recorder)),
            ..JudgeExecutionConfig::default()
        };

        let rep = When2SpeakScorer.score(&preds, &gold, &judge, 1).unwrap();
        let fir = rep
            .metrics
            .iter()
            .find(|m| m.name == "fir")
            .and_then(|m| m.value)
            .unwrap();
        let mir = rep
            .metrics
            .iter()
            .find(|m| m.name == "mir")
            .and_then(|m| m.value)
            .unwrap();
        let macro_f1 = rep
            .metrics
            .iter()
            .find(|m| m.name == "macro_f1")
            .and_then(|m| m.value)
            .unwrap();

        // Confusion matrix: TP=1 (item_tp), FP=0, TN=2 (item_tn, item_tn_gt), FN=1 (item_fn)
        assert!((fir - 0.0).abs() < 1e-12, "fir={fir}");
        assert!((mir - 0.5).abs() < 1e-12, "mir={mir}");
        // f1_speak = 2*1/(2*1 + 0 + 1) = 2/3, f1_silent = 2*2/(2*2 + 1 + 0) = 4/5, macro_f1 = 11/15
        let expected_macro_f1 = 0.5 * ((2.0 / 3.0) + (4.0 / 5.0));
        assert!(
            (macro_f1 - expected_macro_f1).abs() < 1e-12,
            "macro_f1={macro_f1}"
        );

        let guard = recorder.lock().unwrap();
        assert_eq!(guard.outcomes.len(), 4);
        assert_eq!(guard.outcomes[0].item_id, "item_tn");
        assert_eq!(guard.outcomes[0].metric_name, "when2speak_turn");
        assert_eq!(guard.outcomes[0].score, Some(1.0));
        assert_eq!(guard.outcomes[0].passed, Some(true));
        assert_eq!(guard.outcomes[0].category.as_deref(), Some("SILENT"));

        assert_eq!(guard.outcomes[1].item_id, "item_fn");
        assert_eq!(guard.outcomes[1].metric_name, "when2speak_turn");
        assert_eq!(guard.outcomes[1].score, Some(0.0));
        assert_eq!(guard.outcomes[1].passed, Some(false));
        assert_eq!(guard.outcomes[1].category.as_deref(), Some("SPEAK"));

        assert_eq!(guard.outcomes[2].item_id, "item_tn_gt");
        assert_eq!(guard.outcomes[2].score, Some(1.0));
        assert_eq!(guard.outcomes[2].passed, Some(true));

        assert_eq!(guard.outcomes[3].item_id, "item_tp");
        assert_eq!(guard.outcomes[3].score, Some(1.0));
        assert_eq!(guard.outcomes[3].passed, Some(true));
    }
}
