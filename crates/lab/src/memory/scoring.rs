//! Memory-suite scoring: official LoCoMo token F1 plus the published refusal check, and the
//! published LoCoMo / LongMemEval-S judge prompts under the Lab's own metric names.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::LabError;
use crate::datasets::framework::{
    ExternalPrediction, GoldBook, JudgeExecutionConfig, ScorerItemOutcome,
};
use crate::datasets::scoring_utils::{metric_with_cluster_ci, not_scored_metric, token_f1};
use crate::memory::judge::JudgeRunner;
use crate::memory::prompts::{
    locomo_judge_prompt, longmemeval_judge_prompt, parse_locomo_verdict, parse_longmemeval_verdict,
};
use crate::memory::proxy::CallAttribution;

/// One scored item, retained for failure analysis and transcripts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryItemScore {
    pub item_id: String,
    pub probe_id: String,
    pub cluster_id: String,
    pub category: String,
    pub question: String,
    pub gold_answer: String,
    pub answer: String,
    pub spoke: bool,
    pub silent: bool,
    #[serde(default)]
    pub is_abs: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_f1: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub judge_correct: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub judge_verdict: Option<String>,
    pub judge_failed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_reason: Option<String>,
    pub transcript_rel_path: String,
}

/// Result of scoring one benchmark.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MemoryScore {
    pub metrics: Vec<crate::datasets::ScorerMetric>,
    pub category_metrics: BTreeMap<String, BTreeMap<String, f64>>,
    pub items: Vec<MemoryItemScore>,
    pub judge_calls: u64,
    pub judge_failures: u64,
    pub silence_rate: f64,
    pub n_items: usize,
}

impl MemoryScore {
    /// Converts metrics to run-record intervals.
    #[must_use]
    pub fn to_intervals(&self) -> Vec<crate::record::MetricScoreInterval> {
        crate::datasets::ScorerReport {
            dataset_id: String::new(),
            scorer_version: String::new(),
            metrics: self.metrics.clone(),
            judge_model: None,
            judge_prompt_version: None,
        }
        .to_intervals()
    }
}

/// Judges one prompt, retrying once when the verdict cannot be parsed. A second unparsable
/// verdict is recorded as a judge failure and never counted as correct.
fn judge_verdict(
    judge: &dyn JudgeRunner,
    prompt: &str,
    attribution: &CallAttribution,
    parse: fn(&str) -> Option<bool>,
    raw_out: &mut Option<String>,
) -> Result<Option<bool>, LabError> {
    let mut last_failure = String::new();
    for attempt in 0..2 {
        let raw = match judge.judge(prompt, attribution) {
            Ok(raw) => raw,
            Err(err) => {
                let text = err.to_string();
                if text.contains("CostGuard") || text.contains("Authentication") {
                    return Err(err);
                }
                last_failure = text;
                continue;
            }
        };
        *raw_out = Some(raw.clone());
        if let Some(verdict) = parse(&raw) {
            return Ok(Some(verdict));
        }
        last_failure = format!("unparsable verdict (attempt {}): {}", attempt + 1, raw);
    }
    Err(LabError::Protocol(last_failure))
}

fn category_of(entry: &crate::datasets::GoldEntry) -> String {
    entry
        .category
        .clone()
        .or_else(|| entry.metadata.get("category").cloned())
        .unwrap_or_else(|| "unknown".to_owned())
}

/// Scores LoCoMo: per-category official token F1 (categories 1–4 headline), the official
/// category-5 refusal check, and `j_deepseek` from the published `ACCURACY_PROMPT`.
pub fn score_locomo(
    predictions: &[ExternalPrediction],
    gold: &GoldBook,
    questions: &BTreeMap<(String, String), String>,
    cached_items: &BTreeMap<(String, String), MemoryItemScore>,
    judge: Option<&dyn JudgeRunner>,
    judge_config: &JudgeExecutionConfig<'_>,
    project_root: &std::path::Path,
    seed: u64,
) -> Result<MemoryScore, LabError> {
    let mut score = MemoryScore::default();
    let judge_calls_before = judge.map(|j| j.calls()).unwrap_or(0);
    let mut f1_headline: Vec<(String, f64)> = Vec::new();
    let mut judge_headline: Vec<(String, f64)> = Vec::new();
    let mut f1_by_cat: BTreeMap<u8, Vec<(String, f64)>> = BTreeMap::new();
    let mut judge_by_cat: BTreeMap<u8, Vec<(String, f64)>> = BTreeMap::new();
    let mut refusal: Vec<(String, f64)> = Vec::new();
    let mut judge_cat5: Vec<(String, f64)> = Vec::new();
    let mut silent = 0usize;

    for pred in predictions {
        let entry = gold.find(&pred.item_id, &pred.probe_id).ok_or_else(|| {
            LabError::Protocol(format!(
                "locomo missing gold entry for {}:{}",
                pred.item_id, pred.probe_id
            ))
        })?;
        let category = category_of(entry);
        let cat: u8 = category
            .strip_prefix("cat_")
            .and_then(|s| s.parse().ok())
            .unwrap_or(4);
        let question = questions
            .get(&(pred.item_id.clone(), pred.probe_id.clone()))
            .cloned()
            .unwrap_or_default();
        let answer = pred.response_text.clone().unwrap_or_default();
        let is_silent = answer.trim().is_empty();
        if is_silent {
            silent += 1;
        }
        let transcript_rel_path = format!(
            "transcripts/locomo/{}.jsonl",
            pred.item_id
                .chars()
                .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                })
                .collect::<String>()
        );

        let f1 = if cat == 5 {
            let lower = answer.to_ascii_lowercase();
            let refused =
                lower.contains("no information available") || lower.contains("not mentioned");
            let v = if refused { 1.0 } else { 0.0 };
            refusal.push((pred.cluster_id.clone(), v));
            None
        } else {
            let gold_answer = match cat {
                3 => entry
                    .gold_answer
                    .split(';')
                    .next()
                    .unwrap_or(&entry.gold_answer)
                    .trim()
                    .to_owned(),
                _ => entry.gold_answer.clone(),
            };
            let v = if cat == 1 {
                let gold_parts: Vec<&str> = gold_answer
                    .split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .collect();
                let pred_parts: Vec<&str> = answer
                    .split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .collect();
                if gold_parts.is_empty() {
                    token_f1(&answer, &gold_answer, true)
                } else {
                    let sum: f64 = gold_parts
                        .iter()
                        .map(|gp| {
                            pred_parts
                                .iter()
                                .map(|pp| token_f1(pp, gp, true))
                                .fold(0.0_f64, f64::max)
                        })
                        .sum();
                    sum / gold_parts.len() as f64
                }
            } else {
                token_f1(&answer, &gold_answer, true)
            };
            f1_by_cat
                .entry(cat)
                .or_default()
                .push((pred.cluster_id.clone(), v));
            if (1..=4).contains(&cat) {
                f1_headline.push((pred.cluster_id.clone(), v));
            }
            Some(v)
        };
        let _ = question;

        let mut judge_correct = None;
        let mut judge_verdict_raw = None;
        let mut judge_failed = false;
        let mut failure_reason = None;
        let cached = cached_items
            .get(&(pred.item_id.clone(), pred.probe_id.clone()))
            .filter(|c| c.answer == answer && c.question == question);
        if let Some(c) = cached {
            judge_correct = c.judge_correct;
            judge_verdict_raw = c.judge_verdict.clone();
            judge_failed = c.judge_failed;
            failure_reason = c.failure_reason.clone();
            if let Some(v) = c.judge_correct {
                let val = if v { 1.0 } else { 0.0 };
                if cat == 5 {
                    judge_cat5.push((pred.cluster_id.clone(), val));
                } else {
                    judge_by_cat
                        .entry(cat)
                        .or_default()
                        .push((pred.cluster_id.clone(), val));
                    if (1..=4).contains(&cat) {
                        judge_headline.push((pred.cluster_id.clone(), val));
                    }
                }
            }
        } else if let Some(runner) = judge {
            let prompt = locomo_judge_prompt(&question, &entry.gold_answer, &answer);
            let attribution =
                CallAttribution::new("judge", "locomo", pred.cluster_id.clone(), "judge");
            match judge_verdict(
                runner,
                &prompt,
                &attribution,
                parse_locomo_verdict,
                &mut judge_verdict_raw,
            ) {
                Ok(Some(v)) => {
                    judge_correct = Some(v);
                    let val = if v { 1.0 } else { 0.0 };
                    if cat == 5 {
                        judge_cat5.push((pred.cluster_id.clone(), val));
                    } else {
                        judge_by_cat
                            .entry(cat)
                            .or_default()
                            .push((pred.cluster_id.clone(), val));
                        if (1..=4).contains(&cat) {
                            judge_headline.push((pred.cluster_id.clone(), val));
                        }
                    }
                }
                Ok(None) => {}
                Err(err) => {
                    let text = err.to_string();
                    if text.contains("CostGuard") || text.contains("Authentication") {
                        return Err(err);
                    }
                    judge_failed = true;
                    failure_reason = Some(text.clone());
                    score.judge_failures += 1;
                    judge_config.record_outcome(ScorerItemOutcome {
                        item_id: pred.item_id.clone(),
                        probe_id: pred.probe_id.clone(),
                        cluster_id: pred.cluster_id.clone(),
                        category: Some(category.clone()),
                        metric_name: "j_deepseek".to_owned(),
                        score: None,
                        passed: None,
                        judge_prompt: Some(prompt),
                        judge_verdict: judge_verdict_raw.clone(),
                        failed: true,
                        failure_reason: Some(text),
                    });
                }
            }
        }

        score.items.push(MemoryItemScore {
            item_id: pred.item_id.clone(),
            probe_id: pred.probe_id.clone(),
            cluster_id: pred.cluster_id.clone(),
            category: category.clone(),
            question: question.clone(),
            gold_answer: entry.gold_answer.clone(),
            answer,
            spoke: pred.spoke,
            silent: is_silent,
            is_abs: false,
            token_f1: f1,
            judge_correct,
            judge_verdict: judge_verdict_raw,
            judge_failed,
            failure_reason,
            transcript_rel_path,
        });
        let _ = project_root;
    }

    score.n_items = predictions.len();
    score.silence_rate = if predictions.is_empty() {
        0.0
    } else {
        silent as f64 / predictions.len() as f64
    };
    score.judge_calls = judge.map(|j| j.calls()).unwrap_or(0) - judge_calls_before;

    if judge.is_some() {
        score
            .metrics
            .push(metric_with_cluster_ci("j_deepseek", &judge_headline, seed));
    } else {
        score
            .metrics
            .push(not_scored_metric("j_deepseek", "no judge configured"));
    }
    score
        .metrics
        .push(metric_with_cluster_ci("qa_f1", &f1_headline, seed));
    for (cat, obs) in &f1_by_cat {
        score.metrics.push(metric_with_cluster_ci(
            &format!("qa_f1_cat_{cat}"),
            obs,
            seed,
        ));
    }
    if judge.is_some() {
        for (cat, obs) in &judge_by_cat {
            score.metrics.push(metric_with_cluster_ci(
                &format!("j_deepseek_cat_{cat}"),
                obs,
                seed,
            ));
        }
    }
    score
        .metrics
        .push(metric_with_cluster_ci("cat5_refusal_acc", &refusal, seed));
    if judge.is_some() {
        score.metrics.push(metric_with_cluster_ci(
            "j_deepseek_cat_5",
            &judge_cat5,
            seed,
        ));
    }
    score
        .metrics
        .push(silence_metric(score.silence_rate, score.n_items));

    for (cat, obs) in &f1_by_cat {
        if !obs.is_empty() {
            let mean = obs.iter().map(|(_, v)| v).sum::<f64>() / obs.len() as f64;
            score
                .category_metrics
                .entry(format!("cat_{cat}"))
                .or_default()
                .insert("qa_f1".to_owned(), mean);
        }
    }
    for (cat, obs) in &judge_by_cat {
        if !obs.is_empty() {
            let mean = obs.iter().map(|(_, v)| v).sum::<f64>() / obs.len() as f64;
            score
                .category_metrics
                .entry(format!("cat_{cat}"))
                .or_default()
                .insert("j_deepseek".to_owned(), mean);
        }
    }
    Ok(score)
}

/// Scores LongMemEval-S: `acc_j_deepseek` per question type, overall and task-averaged, with
/// abstention (`_abs`) reported apart.
pub fn score_longmemeval(
    predictions: &[ExternalPrediction],
    gold: &GoldBook,
    questions: &BTreeMap<(String, String), String>,
    cached_items: &BTreeMap<(String, String), MemoryItemScore>,
    judge: Option<&dyn JudgeRunner>,
    _judge_config: &JudgeExecutionConfig<'_>,
    _project_root: &std::path::Path,
    seed: u64,
) -> Result<MemoryScore, LabError> {
    let mut score = MemoryScore::default();
    let judge_calls_before = judge.map(|j| j.calls()).unwrap_or(0);
    let mut overall: Vec<(String, f64)> = Vec::new();
    let mut by_type: BTreeMap<String, Vec<(String, f64)>> = BTreeMap::new();
    let mut abstention: Vec<(String, f64)> = Vec::new();
    let mut silent = 0usize;

    for pred in predictions {
        let entry = gold.find(&pred.item_id, &pred.probe_id).ok_or_else(|| {
            LabError::Protocol(format!(
                "longmemeval missing gold for {}:{}",
                pred.item_id, pred.probe_id
            ))
        })?;
        let question_type = entry
            .category
            .clone()
            .or_else(|| entry.metadata.get("question_type").cloned())
            .unwrap_or_else(|| "unknown".to_owned());
        let is_abs = entry.metadata.get("is_abs").map(String::as_str) == Some("true");
        let answer = pred.response_text.clone().unwrap_or_default();
        let is_silent = answer.trim().is_empty();
        if is_silent {
            silent += 1;
        }
        let transcript_rel_path = format!(
            "transcripts/longmemeval/{}.jsonl",
            pred.item_id
                .chars()
                .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                })
                .collect::<String>()
        );

        let mut judge_correct = None;
        let mut judge_verdict_raw = None;
        let mut judge_failed = false;
        let mut failure_reason = None;
        let question = questions
            .get(&(pred.item_id.clone(), pred.probe_id.clone()))
            .cloned()
            .unwrap_or_default();
        let cached = cached_items
            .get(&(pred.item_id.clone(), pred.probe_id.clone()))
            .filter(|c| c.answer == answer && c.question == question);
        if let Some(c) = cached {
            judge_correct = c.judge_correct;
            judge_verdict_raw = c.judge_verdict.clone();
            judge_failed = c.judge_failed;
            failure_reason = c.failure_reason.clone();
            if let Some(v) = c.judge_correct {
                let val = if v { 1.0 } else { 0.0 };
                if is_abs {
                    abstention.push((pred.cluster_id.clone(), val));
                } else {
                    overall.push((pred.cluster_id.clone(), val));
                    by_type
                        .entry(question_type.clone())
                        .or_default()
                        .push((pred.cluster_id.clone(), val));
                }
            }
        } else if let Some(runner) = judge {
            let prompt = longmemeval_judge_prompt(
                &question_type,
                &question,
                &entry.gold_answer,
                &answer,
                is_abs,
            );
            let attribution =
                CallAttribution::new("judge", "longmemeval", pred.cluster_id.clone(), "judge");
            match judge_verdict(
                runner,
                &prompt,
                &attribution,
                parse_longmemeval_verdict,
                &mut judge_verdict_raw,
            ) {
                Ok(Some(v)) => {
                    judge_correct = Some(v);
                    let val = if v { 1.0 } else { 0.0 };
                    if is_abs {
                        abstention.push((pred.cluster_id.clone(), val));
                    } else {
                        overall.push((pred.cluster_id.clone(), val));
                        by_type
                            .entry(question_type.clone())
                            .or_default()
                            .push((pred.cluster_id.clone(), val));
                    }
                }
                Ok(None) => {}
                Err(err) => {
                    let text = err.to_string();
                    if text.contains("CostGuard") || text.contains("Authentication") {
                        return Err(err);
                    }
                    judge_failed = true;
                    failure_reason = Some(text.clone());
                    score.judge_failures += 1;
                }
            }
        }

        score.items.push(MemoryItemScore {
            item_id: pred.item_id.clone(),
            probe_id: pred.probe_id.clone(),
            cluster_id: pred.cluster_id.clone(),
            category: question_type,
            question,
            gold_answer: entry.gold_answer.clone(),
            answer,
            spoke: pred.spoke,
            silent: is_silent,
            is_abs,
            token_f1: None,
            judge_correct,
            judge_verdict: judge_verdict_raw,
            judge_failed,
            failure_reason,
            transcript_rel_path,
        });
    }

    score.n_items = predictions.len();
    score.silence_rate = if predictions.is_empty() {
        0.0
    } else {
        silent as f64 / predictions.len() as f64
    };
    score.judge_calls = judge.map(|j| j.calls()).unwrap_or(0) - judge_calls_before;

    if judge.is_some() {
        score
            .metrics
            .push(metric_with_cluster_ci("acc_j_deepseek", &overall, seed));
        let type_means: Vec<f64> = by_type
            .values()
            .filter(|obs| !obs.is_empty())
            .map(|obs| obs.iter().map(|(_, v)| v).sum::<f64>() / obs.len() as f64)
            .collect();
        let task_avg = if type_means.is_empty() {
            None
        } else {
            Some(type_means.iter().sum::<f64>() / type_means.len() as f64)
        };
        score.metrics.push(crate::datasets::ScorerMetric {
            name: "acc_j_deepseek_task_avg".to_owned(),
            value: task_avg,
            ci95_low: task_avg,
            ci95_high: task_avg,
            n_clusters: None,
            not_scored_reason: None,
        });
        if abstention.is_empty() {
            score
                .metrics
                .push(not_scored_metric("abs_acc_j_deepseek", "no _abs items"));
        } else {
            score.metrics.push(metric_with_cluster_ci(
                "abs_acc_j_deepseek",
                &abstention,
                seed,
            ));
        }
        for (qt, obs) in &by_type {
            score.metrics.push(metric_with_cluster_ci(
                &format!("acc_j_deepseek_{qt}"),
                obs,
                seed,
            ));
        }
    } else {
        score
            .metrics
            .push(not_scored_metric("acc_j_deepseek", "no judge configured"));
    }
    score
        .metrics
        .push(silence_metric(score.silence_rate, score.n_items));

    for (qt, obs) in &by_type {
        if !obs.is_empty() {
            let mean = obs.iter().map(|(_, v)| v).sum::<f64>() / obs.len() as f64;
            score
                .category_metrics
                .entry(qt.clone())
                .or_default()
                .insert("acc_j_deepseek".to_owned(), mean);
        }
    }
    Ok(score)
}

fn silence_metric(rate: f64, n: usize) -> crate::datasets::ScorerMetric {
    crate::datasets::ScorerMetric {
        name: "silence_rate".to_owned(),
        value: Some(rate),
        ci95_low: None,
        ci95_high: None,
        n_clusters: Some(n),
        not_scored_reason: None,
    }
}
