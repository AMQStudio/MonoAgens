//! `fantom` (`skywalker023/fantom`) Mode-1 chat converter & `All` (deterministic) + `All*`
//! (`lab/adapters` ST cosine) scorer over the official `fantom_v1.json` release.

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
use crate::datasets::scoring_utils::{
    metric_with_cluster_ci, not_scored_metric, resolve_verified_scorer_model_dir,
    run_optional_python_helper, token_f1,
};

/// Manifest ID for the offline-verified SentenceTransformer scorer model (`all-roberta-large-v1`).
pub const FANTOM_SCORER_MODEL_ID: &str = "fantom_scorer_all_roberta_large_v1";

pub struct FantomConverter;

#[derive(Deserialize)]
struct RawFantomSet {
    set_id: String,
    part_id: String,
    conv_id: u64,
    short_context: String,
    #[allow(dead_code)]
    full_context: String,
    #[serde(rename = "factQA")]
    fact_qa: RawFantomFactQa,
    #[serde(rename = "beliefQAs", default)]
    belief_qas: Vec<RawFantomBeliefQa>,
    #[serde(rename = "infoAccessibilityQA_list")]
    info_accessibility_qa_list: RawFantomListQa,
    #[serde(rename = "infoAccessibilityQAs_binary", default)]
    info_accessibility_qas_binary: Vec<RawFantomBinaryQa>,
    #[serde(rename = "answerabilityQA_list")]
    answerability_qa_list: RawFantomListQa,
    #[serde(rename = "answerabilityQAs_binary", default)]
    answerability_qas_binary: Vec<RawFantomBinaryQa>,
}

#[derive(Deserialize)]
struct RawFantomFactQa {
    question: String,
    correct_answer: String,
}

#[derive(Deserialize)]
struct RawFantomBeliefQa {
    question: String,
    correct_answer: String,
    wrong_answer: String,
}

#[derive(Deserialize)]
struct RawFantomListQa {
    question: String,
    correct_answer: Vec<String>,
    #[serde(default)]
    wrong_answer: Vec<String>,
}

#[derive(Deserialize)]
struct RawFantomBinaryQa {
    question: String,
    correct_answer: String,
}

fn parse_chat_context(ctx: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for line in ctx.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some((speaker, text)) = trimmed.split_once(':') {
            let speaker = speaker.trim();
            let text = text.trim();
            if !speaker.is_empty()
                && speaker.len() <= 40
                && !text.is_empty()
                && speaker.split_whitespace().count() <= 3
            {
                out.push((speaker.to_owned(), text.to_owned()));
                continue;
            }
        }
        out.push(("narrator".to_owned(), trimmed.to_owned()));
    }
    out
}

fn fantom_list_hit(resp: &str, gold: &[String], wrong: &[String]) -> bool {
    let lower = resp.to_ascii_lowercase();
    let excluded = gold
        .iter()
        .any(|c| !lower.contains(&c.to_ascii_lowercase()));
    let included_unaware = wrong
        .iter()
        .any(|c| lower.contains(&c.to_ascii_lowercase()));
    !(excluded || included_unaware)
}

fn fantom_binary_hit(resp: &str, answer: &str) -> bool {
    let expected = answer
        .split(':')
        .next()
        .unwrap_or(answer)
        .to_ascii_lowercase();
    let first = resp
        .split_whitespace()
        .next()
        .unwrap_or("")
        .trim_matches(|c: char| !c.is_alphanumeric())
        .to_ascii_lowercase();
    first == expected
}

fn fantom_mc_hit(resp: &str, letter: &str) -> bool {
    let letter = letter.to_ascii_lowercase();
    let response = resp.to_ascii_lowercase();
    response.starts_with(&format!("({letter})"))
        || response.starts_with(&format!("{letter})"))
        || response.starts_with(&format!("{letter}."))
        || response.starts_with(&format!("{letter}:"))
        || response.starts_with(&format!("{letter},"))
        || response == letter
        || response.contains(&format!("({letter})"))
}

impl Converter for FantomConverter {
    fn dataset_id(&self) -> &str {
        "fantom"
    }

    fn converter_version(&self) -> &str {
        "2.0.0"
    }

    fn convert(
        &self,
        dataset_dir: &Path,
        manifest_entry: &DatasetManifestEntry,
    ) -> Result<ConvertedDataset, LabError> {
        let raw = fs::read_to_string(dataset_dir.join("fantom_v1.json"))?;
        let sets: Vec<RawFantomSet> = serde_json::from_str(&raw)
            .map_err(|e| LabError::Serialization(format!("fantom JSON: {e}")))?;

        let soul = ActorId::new("soul");
        let proctor = ActorId::new("lab_proctor");
        let probe_aud = Audience::new([proctor.clone(), soul.clone()]);
        let base_ms: u64 = 1_700_000_000_000;

        // Conversation context cache: every info set shares its conversation's short context.
        let mut conv_cache: BTreeMap<
            u64,
            (Vec<monoagens_protocol::Percept>, Vec<GoldEvidenceRef>),
        > = BTreeMap::new();

        let mut items = Vec::new();
        let mut gold_entries = Vec::new();

        for set in sets {
            let (history, ev_refs) = conv_cache
                .entry(set.conv_id)
                .or_insert_with(|| {
                    let turns = parse_chat_context(&set.short_context);
                    let mut speakers: BTreeSet<ActorId> =
                        turns.iter().map(|(s, _)| ActorId::new(s)).collect();
                    speakers.insert(proctor.clone());
                    speakers.insert(soul.clone());
                    let aud = Audience::new(speakers);
                    let place = PlaceId::new(format!("fantom_conv_{}", set.conv_id));
                    let mut percepts = Vec::with_capacity(turns.len());
                    let mut evidence = Vec::with_capacity(turns.len());
                    for (idx, (speaker, text)) in turns.iter().enumerate() {
                        percepts.push(make_text_percept(
                            speaker,
                            place.as_str(),
                            aud.members.iter().cloned(),
                            base_ms + (idx as u64) * 1_000,
                            text,
                        ));
                        evidence.push(GoldEvidenceRef {
                            turn_index: idx,
                            audience: aud.clone(),
                            place: place.clone(),
                        });
                    }
                    (percepts, evidence)
                })
                .clone();

            let place = PlaceId::new(format!("fantom_conv_{}", set.conv_id));
            let item_id = set.set_id.clone();
            let mut probes = Vec::new();
            let probe_ts = base_ms + ((history.len() + 1) as u64) * 1_000;
            let mut push_probe = |probe_id: String,
                                  question: String,
                                  formulation: &str,
                                  gold_answer: String,
                                  accepted: Vec<String>,
                                  wrong_names: Vec<String>,
                                  wrong_belief: Option<String>| {
                let probe_at = probe_ts + probes.len() as u64 * 1_000;
                probes.push(ExternalProbe {
                    probe_id: probe_id.clone(),
                    percept: make_text_percept(
                        "lab_proctor",
                        place.as_str(),
                        probe_aud.members.iter().cloned(),
                        probe_at,
                        question,
                    ),
                    isolated_checkpoint: true,
                    probe_scope: VisibleScope::new(probe_aud.clone(), Some(place.clone())),
                    expected_capability: None,
                });
                let mut metadata = BTreeMap::new();
                metadata.insert("formulation".to_owned(), formulation.to_owned());
                metadata.insert("set_id".to_owned(), item_id.clone());
                metadata.insert("part_id".to_owned(), set.part_id.clone());
                if !wrong_names.is_empty() {
                    metadata.insert("wrong_names".to_owned(), wrong_names.join(","));
                }
                if let Some(wb) = wrong_belief {
                    metadata.insert("wrong_belief_answer".to_owned(), wb);
                }
                gold_entries.push(GoldEntry {
                    item_id: item_id.clone(),
                    probe_id,
                    cluster_id: set.conv_id.to_string(),
                    gold_answer,
                    accepted_answers: accepted,
                    gold_evidence: ev_refs.clone(),
                    privacy_conflict: false,
                    category: Some("short".to_owned()),
                    metadata,
                });
            };

            push_probe(
                "fact".to_owned(),
                set.fact_qa.question.clone(),
                "FactQ",
                set.fact_qa.correct_answer.clone(),
                vec![set.fact_qa.correct_answer.clone()],
                Vec::new(),
                None,
            );

            for (b_idx, b) in set.belief_qas.iter().enumerate() {
                push_probe(
                    format!("belief{b_idx}_dist"),
                    b.question.clone(),
                    "BeliefQ[Dist.]",
                    b.correct_answer.clone(),
                    vec![b.correct_answer.clone()],
                    Vec::new(),
                    Some(b.wrong_answer.clone()),
                );
                // Official `All` uses the multiple-choice form of the same belief question.
                let options = format!("(a) {}\n(b) {}", b.wrong_answer, b.correct_answer);
                push_probe(
                    format!("belief{b_idx}_mc"),
                    format!(
                        "{}\n{options}\nAnswer with the option letter only.",
                        b.question
                    ),
                    "BeliefQ[MC]",
                    "b".to_owned(),
                    vec!["b".to_owned(), b.correct_answer.clone()],
                    Vec::new(),
                    None,
                );
            }

            push_probe(
                "answerability_list".to_owned(),
                format!(
                    "Target: {}\n{}",
                    set.fact_qa.question, set.answerability_qa_list.question
                ),
                "Answerability[List]",
                set.answerability_qa_list.correct_answer.join(", "),
                set.answerability_qa_list.correct_answer.clone(),
                set.answerability_qa_list.wrong_answer.clone(),
                None,
            );
            for (b_idx, b) in set.answerability_qas_binary.iter().enumerate() {
                if b.correct_answer == "no:long" {
                    continue;
                }
                push_probe(
                    format!("answerability_bin_{b_idx}"),
                    format!(
                        "Target: {}\n{} Answer yes or no.",
                        set.fact_qa.question, b.question
                    ),
                    "Answerability[Binary]",
                    b.correct_answer.split(':').next().unwrap_or("").to_owned(),
                    vec![b.correct_answer.split(':').next().unwrap_or("").to_owned()],
                    Vec::new(),
                    None,
                );
            }

            push_probe(
                "info_accessibility_list".to_owned(),
                format!(
                    "Information: {} {}\n{}",
                    set.fact_qa.question,
                    set.fact_qa.correct_answer,
                    set.info_accessibility_qa_list.question
                ),
                "InfoAccessibility[List]",
                set.info_accessibility_qa_list.correct_answer.join(", "),
                set.info_accessibility_qa_list.correct_answer.clone(),
                set.info_accessibility_qa_list.wrong_answer.clone(),
                None,
            );
            for (b_idx, b) in set.info_accessibility_qas_binary.iter().enumerate() {
                if b.correct_answer == "no:long" {
                    continue;
                }
                push_probe(
                    format!("info_accessibility_bin_{b_idx}"),
                    format!(
                        "Information: {} {}\n{} Answer yes or no.",
                        set.fact_qa.question, set.fact_qa.correct_answer, b.question
                    ),
                    "InfoAccessibility[Binary]",
                    b.correct_answer.split(':').next().unwrap_or("").to_owned(),
                    vec![b.correct_answer.split(':').next().unwrap_or("").to_owned()],
                    Vec::new(),
                    None,
                );
            }

            drop(push_probe);
            items.push(ExternalItem {
                item_id,
                cluster_id: set.conv_id.to_string(),
                strata_key: "short".to_owned(),
                history,
                checkpoint_group: Some(format!("fantom_{}", set.conv_id)),
                probes,
                history_mode: HistoryMode::StaticSilent,
                capabilities: Vec::new(),
            });
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

/// Official `All` formulation set (ALL* additionally requires `BeliefQ[Dist.]`; `FactQ` in neither).
const FANTOM_ALL_FORMULATIONS: [&str; 5] = [
    "BeliefQ[MC]",
    "Answerability[List]",
    "Answerability[Binary]",
    "InfoAccessibility[List]",
    "InfoAccessibility[Binary]",
];

impl Scorer for FantomScorer {
    fn dataset_id(&self) -> &str {
        "fantom"
    }

    fn scorer_version(&self) -> &str {
        "2.0.0"
    }

    fn score(
        &self,
        predictions: &[ExternalPrediction],
        gold: &GoldBook,
        judge: &JudgeExecutionConfig<'_>,
        seed: u64,
    ) -> Result<ScorerReport, LabError> {
        let mut st_payload = Vec::new();
        let mut set_clusters: BTreeMap<String, String> = BTreeMap::new();
        let mut all_ok: BTreeMap<String, bool> = BTreeMap::new();
        let mut form_scores: BTreeMap<String, Vec<f64>> = BTreeMap::new();
        let mut fact_f1: Vec<f64> = Vec::new();

        for p in predictions {
            let entry = gold.find(&p.item_id, &p.probe_id).ok_or_else(|| {
                LabError::Protocol(format!(
                    "fantom missing gold for {}:{}",
                    p.item_id, p.probe_id
                ))
            })?;
            let form = entry
                .metadata
                .get("formulation")
                .map(String::as_str)
                .unwrap_or("");
            set_clusters.insert(p.item_id.clone(), entry.cluster_id.clone());
            let resp = p.response_text.as_deref().unwrap_or("");

            if form == "FactQ" {
                let f1 = entry
                    .accepted_answers
                    .iter()
                    .map(|g| token_f1(resp, g, false))
                    .fold(0.0_f64, f64::max);
                fact_f1.push(f1);
                judge.record_outcome(crate::datasets::framework::ScorerItemOutcome {
                    item_id: p.item_id.clone(),
                    probe_id: p.probe_id.clone(),
                    cluster_id: p.cluster_id.clone(),
                    category: Some(form.to_owned()),
                    metric_name: "fantom_fact_word_f1".to_owned(),
                    score: Some(f1),
                    passed: Some(f1 > 0.5),
                    judge_prompt: None,
                    judge_verdict: None,
                    failed: false,
                    failure_reason: None,
                });
                continue;
            }

            if form == "BeliefQ[Dist.]" {
                st_payload.push(serde_json::json!({
                    "item_id": p.item_id,
                    "probe_id": p.probe_id,
                    "cluster_id": entry.cluster_id,
                    "response": resp,
                    "gold": entry.gold_answer,
                    "wrong_belief": entry.metadata.get("wrong_belief_answer").cloned().unwrap_or_default(),
                }));
                continue;
            }

            let wrong_names: Vec<String> = entry
                .metadata
                .get("wrong_names")
                .map(|s| s.split(',').map(str::to_owned).collect())
                .unwrap_or_default();
            let hit = match form {
                "Answerability[List]" | "InfoAccessibility[List]" => {
                    fantom_list_hit(resp, &entry.accepted_answers, &wrong_names)
                }
                "Answerability[Binary]" | "InfoAccessibility[Binary]" => {
                    fantom_binary_hit(resp, &entry.gold_answer)
                }
                "BeliefQ[MC]" => fantom_mc_hit(resp, &entry.gold_answer),
                _ => false,
            };
            form_scores
                .entry(form.to_owned())
                .or_default()
                .push(if hit { 1.0 } else { 0.0 });
            if FANTOM_ALL_FORMULATIONS.contains(&form) {
                let cur = all_ok.entry(p.item_id.clone()).or_insert(true);
                *cur &= hit;
            }
            judge.record_outcome(crate::datasets::framework::ScorerItemOutcome {
                item_id: p.item_id.clone(),
                probe_id: p.probe_id.clone(),
                cluster_id: p.cluster_id.clone(),
                category: Some(form.to_owned()),
                metric_name: "fantom_all".to_owned(),
                score: Some(if hit { 1.0 } else { 0.0 }),
                passed: Some(hit),
                judge_prompt: None,
                judge_verdict: None,
                failed: false,
                failure_reason: None,
            });
        }

        for (form, vals) in &form_scores {
            if !vals.is_empty() {
                let mean = vals.iter().sum::<f64>() / vals.len() as f64;
                judge.record_category_metric(form.clone(), "acc", mean);
            }
        }
        if !fact_f1.is_empty() {
            let mean = fact_f1.iter().sum::<f64>() / fact_f1.len() as f64;
            judge.record_category_metric("FactQ", "word_f1", mean);
        }

        // `All` = every official five-formulation question in the set is correct.
        let all_obs: Vec<(String, f64)> = all_ok
            .iter()
            .map(|(iid, &ok)| {
                (
                    set_clusters.get(iid).cloned().unwrap_or_default(),
                    if ok { 1.0 } else { 0.0 },
                )
            })
            .collect();

        // `All*` = `All` plus the local SentenceTransformer belief-distribution question(s).
        let ws_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let all_star_metric = if st_payload.is_empty() {
            metric_with_cluster_ci("fantom_all_star", &all_obs, seed)
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
                    let star_obs: Vec<(String, f64)> = all_ok
                        .iter()
                        .map(|(iid, &ok)| {
                            let ok6 = ok && dist_ok.get(iid).copied().unwrap_or(false);
                            (
                                set_clusters.get(iid).cloned().unwrap_or_default(),
                                if ok6 { 1.0 } else { 0.0 },
                            )
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
                metric_with_cluster_ci("fantom_all", &all_obs, seed),
            ],
            judge_model: None,
            judge_prompt_version: None,
        })
    }
}
