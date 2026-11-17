//! `opentom` (`SeacowX/OpenToM`) Mode-2 narrator converter & `Loc_c`/`Loc_f`/`Att`/`MHop` macro-F1 scorer.

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
use crate::datasets::scoring_utils::{macro_f1_labels, normalize_qa_tokens};

pub struct OpenTomConverter;

#[derive(Deserialize)]
struct RawOpenTomRow {
    narrative: String,
    #[serde(default)]
    personality: Option<String>,
    #[serde(default)]
    plot_info: Option<RawOpenTomPlotInfo>,
    question: RawOpenTomQuestion,
}

#[derive(Deserialize)]
struct RawOpenTomPlotInfo {
    #[serde(default)]
    original_place: Option<String>,
    #[serde(default)]
    move_to_place: Option<String>,
}

#[derive(Deserialize)]
struct RawOpenTomQuestion {
    question: String,
    answer: String,
    #[serde(rename = "type")]
    question_type: String,
}

fn canonical_opentom_type(raw_type: &str, answer: &str) -> &'static str {
    match raw_type {
        "attitude" | "Att" => "Att",
        "multihop-fo" | "multihop-so" | "multihop" | "MHop" => "MHop",
        "location-coarse" | "Loc_c" => "Loc_c",
        "location-fine" | "Loc_f" => "Loc_f",
        "location-fo" | "location-so" => {
            let trimmed = answer.trim();
            if trimmed.eq_ignore_ascii_case("yes") || trimmed.eq_ignore_ascii_case("no") {
                "Loc_c"
            } else {
                "Loc_f"
            }
        }
        _ => "Loc_c",
    }
}

fn options_for_opentom(qtype: &str, plot_info: Option<&RawOpenTomPlotInfo>) -> Vec<String> {
    match qtype {
        "Loc_c" => vec!["Yes".to_owned(), "No".to_owned()],
        "Att" => vec![
            "positive".to_owned(),
            "negative".to_owned(),
            "neutral".to_owned(),
        ],
        "MHop" => vec![
            "equally accessible".to_owned(),
            "less accessible".to_owned(),
            "more accessible".to_owned(),
            "equally full".to_owned(),
            "less full".to_owned(),
            "more full".to_owned(),
        ],
        "Loc_f" => {
            let mut opts = Vec::new();
            if let Some(pi) = plot_info {
                if let Some(orig) = pi.original_place.as_deref()
                    && !orig.trim().is_empty()
                {
                    opts.push(orig.trim().to_owned());
                }
                if let Some(mv) = pi.move_to_place.as_deref()
                    && !mv.trim().is_empty()
                    && !opts.iter().any(|x| x == mv.trim())
                {
                    opts.push(mv.trim().to_owned());
                }
            }
            opts
        }
        _ => Vec::new(),
    }
}

impl Converter for OpenTomConverter {
    fn dataset_id(&self) -> &str {
        "opentom"
    }

    fn converter_version(&self) -> &str {
        "2.0.0"
    }

    fn convert(
        &self,
        dataset_dir: &Path,
        manifest_entry: &DatasetManifestEntry,
    ) -> Result<ConvertedDataset, LabError> {
        let mut raw_rows: Vec<(String, RawOpenTomRow)> = Vec::new();
        for (split_name, fname) in [("Long", "opentom.json"), ("ExtraLong", "opentom_long.json")] {
            let p = dataset_dir.join(fname);
            if p.is_file() {
                let raw = fs::read_to_string(&p)?;
                let part: Vec<RawOpenTomRow> = serde_json::from_str(&raw)
                    .map_err(|e| LabError::Serialization(format!("opentom {fname}: {e}")))?;
                for r in part {
                    raw_rows.push((split_name.to_owned(), r));
                }
            }
        }
        if raw_rows.is_empty() {
            return Err(LabError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("no opentom.json under '{}'", dataset_dir.display()),
            )));
        }

        // Group QA rows by exact narrative preserving insertion order.
        let mut story_idx_map: BTreeMap<String, usize> = BTreeMap::new();
        let mut stories: Vec<(String, String, Vec<RawOpenTomRow>)> = Vec::new();
        for (split, row) in raw_rows {
            if let Some(&pos) = story_idx_map.get(&row.narrative) {
                stories[pos].2.push(row);
            } else {
                let pos = stories.len();
                story_idx_map.insert(row.narrative.clone(), pos);
                stories.push((split, row.narrative.clone(), vec![row]));
            }
        }

        let soul = ActorId::new("soul");
        let narrator = ActorId::new("narrator");
        let proctor = ActorId::new("lab_proctor");
        let hist_aud = Audience::new([narrator, proctor.clone(), soul.clone()]);
        let probe_aud = Audience::new([proctor, soul]);
        let base_ms: u64 = 1_700_000_000_000;

        let mut items = Vec::with_capacity(stories.len());
        let mut gold_entries = Vec::new();

        for (s_idx, (split, narrative, rows)) in stories.into_iter().enumerate() {
            let story_id = format!("opentom_{s_idx:04}");
            let place = PlaceId::new(story_id.clone());
            let trait_tag = rows
                .first()
                .and_then(|r| r.personality.clone())
                .unwrap_or_else(|| "neutral".to_owned());

            let history = vec![make_text_percept(
                "narrator",
                place.as_str(),
                hist_aud.members.iter().cloned(),
                base_ms,
                &narrative,
            )];
            let ev_refs = vec![GoldEvidenceRef {
                turn_index: 0,
                audience: hist_aud.clone(),
                place: place.clone(),
            }];

            let mut probes = Vec::with_capacity(rows.len());
            for (q_idx, row) in rows.into_iter().enumerate() {
                let q = row.question;
                let probe_id = format!("q_{q_idx}");
                let qtype = canonical_opentom_type(&q.question_type, &q.answer);
                let opts = options_for_opentom(qtype, row.plot_info.as_ref());
                let prompt = if opts.is_empty() {
                    q.question.clone()
                } else {
                    format!("{}\nOptions: {}", q.question, opts.join(" | "))
                };
                probes.push(ExternalProbe {
                    probe_id: probe_id.clone(),
                    percept: make_text_percept(
                        "lab_proctor",
                        place.as_str(),
                        probe_aud.members.iter().cloned(),
                        base_ms + ((q_idx + 1) as u64) * 1_000,
                        prompt,
                    ),
                    isolated_checkpoint: true,
                    probe_scope: VisibleScope::new(probe_aud.clone(), Some(place.clone())),
                    expected_capability: None,
                });

                let mut metadata = BTreeMap::new();
                metadata.insert("question_type".to_owned(), qtype.to_owned());
                metadata.insert("raw_question_type".to_owned(), q.question_type);
                metadata.insert("options".to_owned(), opts.join("||"));

                gold_entries.push(GoldEntry {
                    item_id: story_id.clone(),
                    probe_id,
                    cluster_id: story_id.clone(),
                    gold_answer: q.answer.clone(),
                    accepted_answers: vec![q.answer],
                    gold_evidence: ev_refs.clone(),
                    privacy_conflict: false,
                    category: Some(qtype.to_owned()),
                    metadata,
                });
            }

            items.push(ExternalItem {
                item_id: story_id.clone(),
                cluster_id: story_id,
                strata_key: format!("{split}:{trait_tag}"),
                history,
                checkpoint_group: None,
                probes,
                history_mode: HistoryMode::StaticSilent,
                capabilities: Vec::new(),
            });
        }

        Ok(ConvertedDataset {
            dataset_id: "opentom".to_owned(),
            pinned_revision: manifest_entry
                .sources
                .first()
                .map_or_else(|| "pending_pin".to_owned(), |s| s.revision.clone()),
            converter_version: self.converter_version().to_owned(),
            items,
            gold_book: GoldBook {
                dataset_id: "opentom".to_owned(),
                entries: gold_entries,
            },
        })
    }
}

pub struct OpenTomScorer;

fn argmax_lexical_choice(resp: &str, options: &[&str]) -> String {
    if options.is_empty() {
        return resp.trim().to_ascii_lowercase();
    }
    let resp_toks: BTreeSet<String> = normalize_qa_tokens(resp, false).into_iter().collect();
    let mut best_opt = options[0];
    let mut best_score = 0usize;
    let mut tie = false;
    for &opt in options {
        let opt_toks: BTreeSet<String> = normalize_qa_tokens(opt, false).into_iter().collect();
        let overlap = resp_toks.intersection(&opt_toks).count();
        if overlap > best_score {
            best_score = overlap;
            best_opt = opt;
            tie = false;
        } else if overlap == best_score && overlap > 0 {
            tie = true;
        }
    }
    if best_score == 0 || tie {
        "__unmatched__".to_owned()
    } else {
        best_opt.trim().to_ascii_lowercase()
    }
}

impl Scorer for OpenTomScorer {
    fn dataset_id(&self) -> &str {
        "opentom"
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
        let mut by_type: BTreeMap<&str, Vec<(String, String)>> = BTreeMap::new();
        for pred in predictions {
            let entry = gold.find(&pred.item_id, &pred.probe_id).ok_or_else(|| {
                LabError::Protocol(format!(
                    "opentom missing gold for {}:{}",
                    pred.item_id, pred.probe_id
                ))
            })?;
            let qtype = match entry
                .metadata
                .get("question_type")
                .map(String::as_str)
                .unwrap_or("Loc_c")
            {
                "location-coarse" | "Loc_c" => "Loc_c",
                "location-fine" | "Loc_f" => "Loc_f",
                "attitude" | "Att" => "Att",
                "multihop" | "MHop" => "MHop",
                other => other,
            };
            let opts: Vec<&str> = entry
                .metadata
                .get("options")
                .map(|s| s.split("||").filter(|o| !o.is_empty()).collect())
                .unwrap_or_default();
            let resp = pred.response_text.as_deref().unwrap_or("");
            let pred_lbl = if opts.is_empty() {
                normalize_qa_tokens(resp, false).join(" ")
            } else {
                argmax_lexical_choice(resp, &opts)
            };
            let gold_lbl = if opts.is_empty() {
                normalize_qa_tokens(&entry.gold_answer, false).join(" ")
            } else {
                entry.gold_answer.trim().to_ascii_lowercase()
            };
            let hit = pred_lbl == gold_lbl;
            judge.record_outcome(crate::datasets::framework::ScorerItemOutcome {
                item_id: pred.item_id.clone(),
                probe_id: pred.probe_id.clone(),
                cluster_id: pred.cluster_id.clone(),
                category: Some(qtype.to_owned()),
                metric_name: qtype.to_owned(),
                score: Some(if hit { 1.0 } else { 0.0 }),
                passed: Some(hit),
                judge_prompt: None,
                judge_verdict: None,
                failed: false,
                failure_reason: None,
            });
            by_type.entry(qtype).or_default().push((gold_lbl, pred_lbl));
        }

        let metric_for = |qtype: &str, name: &str| -> ScorerMetric {
            match by_type.get(qtype) {
                Some(pairs) if !pairs.is_empty() => ScorerMetric {
                    name: name.to_owned(),
                    value: Some(macro_f1_labels(pairs)),
                    ci95_low: None,
                    ci95_high: None,
                    n_clusters: Some(pairs.len()),
                    not_scored_reason: None,
                },
                _ => ScorerMetric {
                    name: name.to_owned(),
                    value: None,
                    ci95_low: None,
                    ci95_high: None,
                    n_clusters: None,
                    not_scored_reason: Some(format!("no {qtype} items in sample")),
                },
            }
        };

        Ok(ScorerReport {
            dataset_id: "opentom".to_owned(),
            scorer_version: self.scorer_version().to_owned(),
            metrics: vec![
                metric_for("Loc_c", "macro_f1_coarse"),
                metric_for("Loc_f", "macro_f1_fine"),
                metric_for("Att", "attitude_f1"),
                metric_for("MHop", "mhop_f1"),
            ],
            judge_model: None,
            judge_prompt_version: None,
        })
    }
}
