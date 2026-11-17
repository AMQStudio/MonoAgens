//! `tomi` (`facebookresearch/ToMi`) Mode-1 bAbI converter (stripping `.trace`) & `shortcut_perturbation_acc` scorer.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use monoagens_protocol::{ActorId, Audience, PlaceId, VisibleScope};

use crate::LabError;
use crate::datasets::framework::{
    ConvertedDataset, Converter, ExternalItem, ExternalPrediction, ExternalProbe, GoldBook,
    GoldEntry, HistoryMode, JudgeExecutionConfig, Scorer, ScorerReport, make_text_percept,
};
use crate::datasets::manifest::DatasetManifestEntry;
use crate::datasets::scoring_utils::{
    build_mode1_tom_history, exact_match_normalized, metric_with_cluster_ci,
};

pub struct TomiConverter;

struct ParsedTomiStory {
    lines: Vec<String>,
    questions: Vec<(String, String)>,
}

fn parse_babi_stories(raw: &str) -> Vec<ParsedTomiStory> {
    let mut stories = Vec::new();
    let mut cur_lines: Vec<String> = Vec::new();
    let mut cur_qs: Vec<(String, String)> = Vec::new();

    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let body = trimmed.split_once(' ').map_or(trimmed, |(num, rest)| {
            if num == "1" && (!cur_lines.is_empty() || !cur_qs.is_empty()) {
                stories.push(ParsedTomiStory {
                    lines: std::mem::take(&mut cur_lines),
                    questions: std::mem::take(&mut cur_qs),
                });
            }
            rest
        });
        if let Some((q, ans_and_trace)) = body.split_once('\t') {
            let ans = ans_and_trace
                .split('\t')
                .next()
                .unwrap_or("")
                .trim()
                .to_owned();
            cur_qs.push((q.trim().to_owned(), ans));
        } else {
            cur_lines.push(body.to_owned());
        }
    }
    if !cur_lines.is_empty() || !cur_qs.is_empty() {
        stories.push(ParsedTomiStory {
            lines: cur_lines,
            questions: cur_qs,
        });
    }
    stories
}

impl Converter for TomiConverter {
    fn dataset_id(&self) -> &str {
        "tomi"
    }

    fn converter_version(&self) -> &str {
        "1.0.0"
    }

    fn convert(
        &self,
        dataset_dir: &Path,
        manifest_entry: &DatasetManifestEntry,
    ) -> Result<ConvertedDataset, LabError> {
        let raw = fs::read_to_string(dataset_dir.join("test.txt"))?;
        let stories = parse_babi_stories(&raw);
        let trace_lines: Vec<String> = fs::read_to_string(dataset_dir.join("test.trace"))
            .map(|s| s.lines().map(str::to_owned).collect())
            .unwrap_or_default();

        let soul = ActorId::new("soul");
        let proctor = ActorId::new("lab_proctor");
        let probe_aud = Audience::new([proctor, soul]);

        let mut items = Vec::with_capacity(stories.len());
        let mut gold_entries = Vec::new();

        for (s_idx, story) in stories.into_iter().enumerate() {
            let item_id = format!("tomi_{s_idx:04}");
            let (history, ev_refs) =
                build_mode1_tom_history(&story.lines, &item_id, &[], 1_700_000_000_000, 1_000);
            let place = PlaceId::new(format!("{item_id}_main"));
            let trace_tag = trace_lines
                .get(s_idx)
                .cloned()
                .unwrap_or_else(|| "false_belief".to_owned());

            let probe_base = 1_700_000_000_000 + (history.len() as u64 + 1) * 1_000;
            let mut probes = Vec::with_capacity(story.questions.len());
            for (q_idx, (q_text, ans)) in story.questions.into_iter().enumerate() {
                let probe_id = format!("q_{q_idx}");
                probes.push(ExternalProbe {
                    probe_id: probe_id.clone(),
                    percept: make_text_percept(
                        "lab_proctor",
                        place.as_str(),
                        probe_aud.members.iter().cloned(),
                        probe_base + (q_idx as u64) * 1_000,
                        q_text,
                    ),
                    isolated_checkpoint: true,
                    probe_scope: VisibleScope::new(probe_aud.clone(), Some(place.clone())),
                    expected_capability: None,
                });

                let mut metadata = BTreeMap::new();
                metadata.insert("trace_type".to_owned(), trace_tag.clone());

                gold_entries.push(GoldEntry {
                    item_id: item_id.clone(),
                    probe_id,
                    cluster_id: item_id.clone(),
                    gold_answer: ans.clone(),
                    accepted_answers: vec![ans],
                    gold_evidence: ev_refs.clone(),
                    privacy_conflict: false,
                    category: Some(trace_tag.clone()),
                    metadata,
                });
            }

            items.push(ExternalItem {
                item_id: item_id.clone(),
                cluster_id: item_id,
                strata_key: "tomi_story".to_owned(),
                history,
                checkpoint_group: None,
                probes,
                history_mode: HistoryMode::StaticSilent,
                capabilities: Vec::new(),
            });
        }

        Ok(ConvertedDataset {
            dataset_id: "tomi".to_owned(),
            pinned_revision: manifest_entry
                .sources
                .first()
                .map_or_else(|| "pending_pin".to_owned(), |s| s.revision.clone()),
            converter_version: self.converter_version().to_owned(),
            items,
            gold_book: GoldBook {
                dataset_id: "tomi".to_owned(),
                entries: gold_entries,
            },
        })
    }
}

pub struct TomiScorer;

impl Scorer for TomiScorer {
    fn dataset_id(&self) -> &str {
        "tomi"
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
                LabError::Protocol(format!(
                    "tomi missing gold for {}:{}",
                    pred.item_id, pred.probe_id
                ))
            })?;
            let resp = pred.response_text.as_deref().unwrap_or("");
            let hit = exact_match_normalized(resp, &entry.accepted_answers);
            obs.push((pred.cluster_id.clone(), hit));
        }

        Ok(ScorerReport {
            dataset_id: "tomi".to_owned(),
            scorer_version: self.scorer_version().to_owned(),
            metrics: vec![metric_with_cluster_ci(
                "shortcut_perturbation_acc",
                &obs,
                seed,
            )],
            judge_model: None,
            judge_prompt_version: None,
        })
    }
}
