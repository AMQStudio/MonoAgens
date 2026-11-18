//! `longmemeval` (`xiaowu0162/longmemeval-cleaned`) converter & `gpt-4o` judge-gated scorer.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use monoagens_protocol::{ActorId, Audience, EventId, PlaceId, VisibleScope};
use serde::Deserialize;

use crate::LabError;
use crate::datasets::dates::parse_longmemeval_datetime;
use crate::datasets::framework::{
    ConvertedDataset, Converter, ExternalItem, ExternalPrediction, ExternalProbe, GoldBook,
    GoldEntry, GoldEvidenceRef, HistoryMode, JudgeExecutionConfig, Scorer, ScorerReport,
    make_text_percept,
};
use crate::datasets::manifest::DatasetManifestEntry;
use crate::datasets::scoring_utils::{
    invoke_judge_resilient, metric_with_cluster_ci, not_scored_metric,
};

pub struct LongMemEvalConverter;

#[derive(Deserialize)]
struct RawLmeItem {
    question_id: String,
    question_type: String,
    question: String,
    #[serde(default)]
    question_date: Option<String>,
    answer: serde_json::Value,
    #[serde(default)]
    haystack_dates: Vec<String>,
    haystack_sessions: Vec<Vec<RawLmeTurn>>,
    #[serde(default)]
    answer_session_ids: Vec<String>,
}

#[derive(Deserialize)]
struct RawLmeTurn {
    role: String,
    content: String,
    #[serde(default)]
    has_answer: Option<bool>,
}

impl Converter for LongMemEvalConverter {
    fn dataset_id(&self) -> &str {
        "longmemeval"
    }

    fn converter_version(&self) -> &str {
        "1.1.0"
    }

    fn convert(
        &self,
        dataset_dir: &Path,
        manifest_entry: &DatasetManifestEntry,
    ) -> Result<ConvertedDataset, LabError> {
        let primary = dataset_dir.join("longmemeval_s_cleaned.json");
        let fallback = dataset_dir.join("longmemeval_oracle.json");
        let path = if primary.is_file() { primary } else { fallback };
        let raw = fs::read_to_string(&path)?;
        let items_raw: Vec<RawLmeItem> = serde_json::from_str(&raw)
            .map_err(|e| LabError::Serialization(format!("longmemeval parse error: {e}")))?;

        let soul = ActorId::new("soul");
        let user = ActorId::new("user");
        let prior_assistant = ActorId::new("prior_assistant");
        let hist_aud = Audience::new([user.clone(), prior_assistant.clone(), soul.clone()]);
        let probe_aud = Audience::new([user.clone(), soul]);

        let mut items = Vec::with_capacity(items_raw.len());
        let mut gold_entries = Vec::new();

        for (idx, raw_item) in items_raw.into_iter().enumerate() {
            // Strip `_abs` from question_id completely! Use opaque numeric index `lme_<idx>`.
            let opaque_id = format!("lme_{idx:04}");
            let place = PlaceId::new(opaque_id.clone());
            let is_abs = raw_item.question_id.ends_with("_abs");
            // Real dates: sessions in chronological order (the file does not keep them sorted),
            // turns +1 s apart within a session, strictly increasing overall, and the question
            // asked at `question_date` (moved just after the last turn when the file dates a
            // session after its question).
            if raw_item.haystack_dates.len() != raw_item.haystack_sessions.len() {
                return Err(LabError::Serialization(format!(
                    "longmemeval {}: {} sessions but {} haystack_dates",
                    raw_item.question_id,
                    raw_item.haystack_sessions.len(),
                    raw_item.haystack_dates.len()
                )));
            }
            let mut sessions: Vec<(u64, usize, Vec<RawLmeTurn>)> = Vec::new();
            for (s_idx, (session, date)) in raw_item
                .haystack_sessions
                .into_iter()
                .zip(raw_item.haystack_dates.iter())
                .enumerate()
            {
                sessions.push((parse_longmemeval_datetime(date)?, s_idx, session));
            }
            sessions.sort_by_key(|(ms, s_idx, _)| (*ms, *s_idx));

            let mut history = Vec::new();
            let mut gold_evidence = Vec::new();
            let mut last_ts: Option<u64> = None;

            for (session_ms, _, session) in sessions {
                for (t_idx, turn) in session.into_iter().enumerate() {
                    let turn_idx = history.len();
                    let (actor, addressee) = if turn.role == "assistant" {
                        (&prior_assistant, &user)
                    } else {
                        (&user, &prior_assistant)
                    };
                    let ts =
                        (session_ms + (t_idx as u64) * 1_000).max(last_ts.map_or(0, |t| t + 1_000));
                    last_ts = Some(ts);
                    let mut percept = make_text_percept(
                        actor.as_str(),
                        place.as_str(),
                        hist_aud.members.iter().cloned(),
                        ts,
                        turn.content,
                    );
                    // A two-party chat: each turn is addressed to the other party.
                    percept.addressees = vec![addressee.clone()];
                    history.push(percept);
                    if turn.has_answer == Some(true) {
                        gold_evidence.push(GoldEvidenceRef {
                            turn_index: turn_idx,
                            audience: hist_aud.clone(),
                            place: place.clone(),
                        });
                    }
                }
            }

            let after_history = last_ts.map_or(0, |t| t + 1_000);
            let question_ms = match raw_item.question_date.as_deref() {
                Some(qd) => parse_longmemeval_datetime(qd)?,
                None => after_history,
            };
            let probe_ts = question_ms.max(after_history);
            let probe_time_moved = probe_ts != question_ms;
            let probe = ExternalProbe {
                probe_id: "q0".to_owned(),
                percept: make_text_percept(
                    "user",
                    place.as_str(),
                    probe_aud.members.iter().cloned(),
                    probe_ts,
                    &raw_item.question,
                ),
                isolated_checkpoint: true,
                probe_scope: VisibleScope::new(probe_aud.clone(), Some(place)),
                expected_capability: None,
            };

            let gold_answer = match raw_item.answer {
                serde_json::Value::String(s) => s,
                other => other.to_string(),
            };
            let mut metadata = BTreeMap::new();
            metadata.insert("raw_question_id".to_owned(), raw_item.question_id);
            metadata.insert("question_type".to_owned(), raw_item.question_type.clone());
            metadata.insert("is_abs".to_owned(), is_abs.to_string());
            if let Some(qd) = raw_item.question_date {
                metadata.insert("question_date".to_owned(), qd);
            }
            metadata.insert(
                "probe_time_moved_after_history".to_owned(),
                probe_time_moved.to_string(),
            );
            if !raw_item.answer_session_ids.is_empty() {
                metadata.insert(
                    "answer_session_ids".to_owned(),
                    raw_item.answer_session_ids.join(","),
                );
            }

            gold_entries.push(GoldEntry {
                item_id: opaque_id.clone(),
                probe_id: "q0".to_owned(),
                cluster_id: opaque_id.clone(),
                gold_answer: gold_answer.clone(),
                accepted_answers: vec![gold_answer],
                gold_evidence,
                privacy_conflict: false,
                category: Some(raw_item.question_type.clone()),
                metadata,
            });

            items.push(ExternalItem {
                item_id: opaque_id.clone(),
                cluster_id: opaque_id,
                strata_key: format!("{}:abs={is_abs}", raw_item.question_type),
                history,
                checkpoint_group: None,
                probes: vec![probe],
                history_mode: HistoryMode::StaticSilent,
                capabilities: Vec::new(),
            });
        }

        Ok(ConvertedDataset {
            dataset_id: "longmemeval".to_owned(),
            pinned_revision: manifest_entry
                .sources
                .first()
                .map_or_else(|| "pending_pin".to_owned(), |s| s.revision.clone()),
            converter_version: self.converter_version().to_owned(),
            items,
            gold_book: GoldBook {
                dataset_id: "longmemeval".to_owned(),
                entries: gold_entries,
            },
        })
    }
}

pub struct LongMemEvalScorer;

impl Scorer for LongMemEvalScorer {
    fn dataset_id(&self) -> &str {
        "longmemeval"
    }

    fn scorer_version(&self) -> &str {
        "1.0.0"
    }

    fn score(
        &self,
        predictions: &[ExternalPrediction],
        gold: &GoldBook,
        judge: &JudgeExecutionConfig<'_>,
        seed: u64,
    ) -> Result<ScorerReport, LabError> {
        let Some(jp) = judge.provider.filter(|_| judge.live) else {
            return Ok(ScorerReport {
                dataset_id: "longmemeval".to_owned(),
                scorer_version: self.scorer_version().to_owned(),
                metrics: vec![
                    not_scored_metric("overall_acc", "requires --live gpt-4o judge"),
                    not_scored_metric("abstention_acc", "requires --live gpt-4o judge"),
                ],
                judge_model: None,
                judge_prompt_version: None,
            });
        };
        let judge_model = judge.judge_model.unwrap_or("gpt-4o");
        let prompt_ver = judge.judge_prompt_version.unwrap_or("longmemeval-v1");

        let mut overall_obs = Vec::new();
        let mut abs_obs = Vec::new();
        let mut by_qtype: BTreeMap<String, Vec<f64>> = BTreeMap::new();

        for (idx, pred) in predictions.iter().enumerate() {
            let entry = gold.find(&pred.item_id, &pred.probe_id).ok_or_else(|| {
                LabError::Protocol(format!("longmemeval missing gold for {}", pred.item_id))
            })?;
            let is_abs = entry.metadata.get("is_abs").map(String::as_str) == Some("true");
            let resp = pred.response_text.as_deref().unwrap_or("");
            let user_prompt = if is_abs {
                format!(
                    "I will give you an unanswerable question, an explanation, and a response from a model. Please answer yes if the model correctly identifies the question as unanswerable.\nExplanation: {}\nModel Response: {}\nDoes the model correctly identify the question as unanswerable? Answer yes or no only.",
                    entry.gold_answer, resp
                )
            } else {
                format!(
                    "Gold Answer: {}\nModel Response: {}\nIs the model response correct? Answer yes or no only.",
                    entry.gold_answer, resp
                )
            };

            let Some(correct) = invoke_judge_resilient(
                judge,
                jp,
                EventId::new((idx + 1) as u64),
                judge_model,
                "LongMemEval official judge",
                &user_prompt,
                &pred.item_id,
                &pred.probe_id,
                &pred.cluster_id,
                entry,
                if is_abs {
                    "abstention_acc"
                } else {
                    "overall_acc"
                },
            )?
            else {
                continue;
            };
            let val = if correct { 1.0 } else { 0.0 };
            overall_obs.push((pred.cluster_id.clone(), val));
            if is_abs {
                abs_obs.push((pred.cluster_id.clone(), val));
            }
            if let Some(qt) = entry.category.as_ref() {
                by_qtype.entry(qt.clone()).or_default().push(val);
            }
        }
        for (qt, vals) in by_qtype {
            if !vals.is_empty() {
                let mean = vals.iter().sum::<f64>() / vals.len() as f64;
                judge.record_category_metric(qt, "acc", mean);
            }
        }

        let mut metrics = vec![metric_with_cluster_ci("overall_acc", &overall_obs, seed)];
        if abs_obs.is_empty() {
            metrics.push(not_scored_metric(
                "abstention_acc",
                "no _abs items in sample",
            ));
        } else {
            metrics.push(metric_with_cluster_ci("abstention_acc", &abs_obs, seed));
        }

        Ok(ScorerReport {
            dataset_id: "longmemeval".to_owned(),
            scorer_version: self.scorer_version().to_owned(),
            metrics,
            judge_model: Some(judge_model.to_owned()),
            judge_prompt_version: Some(prompt_ver.to_owned()),
        })
    }
}
