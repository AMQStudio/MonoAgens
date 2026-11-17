//! `stale` (`STALEproj/STALE`) converter with isolated cloned checkpoints for `SR`/`PR`/`IPA`
//! and `Gemini-3.1-flash-lite` judge-gated scorer.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use monoagens_core::ModelPurpose;
use monoagens_protocol::{ActorId, Audience, EventId, PlaceId, VisibleScope};
use monoagens_reason::model::{ModelOutput, ModelRequest};
use serde::Deserialize;

use crate::LabError;
use crate::datasets::framework::{
    ConvertedDataset, Converter, ExternalItem, ExternalPrediction, ExternalProbe, GoldBook,
    GoldEntry, GoldEvidenceRef, HistoryMode, JudgeExecutionConfig, Scorer, ScorerReport,
    make_text_percept,
};
use crate::datasets::manifest::DatasetManifestEntry;
use crate::datasets::scoring_utils::{metric_with_cluster_ci, not_scored_metric};

pub struct StaleConverter;

#[derive(Deserialize)]
struct RawStaleScenario {
    #[serde(default)]
    scenario_id: Option<String>,
    #[serde(default)]
    conflict_type: Option<String>,
    sessions: Vec<Vec<RawStaleTurn>>,
    sr_query: String,
    sr_answer: String,
    pr_query: String,
    pr_answer: String,
    ipa_query: String,
    ipa_answer: String,
    #[serde(default)]
    evidence_turns: Vec<usize>,
}

#[derive(Deserialize)]
struct RawStaleTurn {
    role: String,
    content: String,
}

impl Converter for StaleConverter {
    fn dataset_id(&self) -> &str {
        "stale"
    }

    fn converter_version(&self) -> &str {
        "1.0.0"
    }

    fn convert(
        &self,
        dataset_dir: &Path,
        manifest_entry: &DatasetManifestEntry,
    ) -> Result<ConvertedDataset, LabError> {
        let raw = fs::read_to_string(dataset_dir.join("T1_T2_400_FULL.json"))?;
        let scenarios: Vec<RawStaleScenario> = serde_json::from_str(&raw)
            .map_err(|e| LabError::Serialization(format!("stale JSON error: {e}")))?;

        let soul = ActorId::new("soul");
        let user = ActorId::new("user");
        let prior_assistant = ActorId::new("prior_assistant");
        let hist_aud = Audience::new([user.clone(), prior_assistant, soul.clone()]);
        let probe_aud = Audience::new([user.clone(), soul]);

        let mut items = Vec::with_capacity(scenarios.len());
        let mut gold_entries = Vec::new();

        for (idx, sc) in scenarios.into_iter().enumerate() {
            let item_id = sc.scenario_id.unwrap_or_else(|| format!("stale_{idx:04}"));
            let place = PlaceId::new(format!("stale_{idx:04}"));
            let conflict_type = sc.conflict_type.unwrap_or_else(|| "TypeI".to_owned());
            let base_ms: u64 = 1_700_000_000_000;

            let mut history = Vec::new();
            for (s_idx, sess) in sc.sessions.into_iter().enumerate() {
                let s_ms = base_ms + (s_idx as u64) * 86_400_000;
                for (t_idx, turn) in sess.into_iter().enumerate() {
                    let actor = if turn.role == "assistant" {
                        "prior_assistant"
                    } else {
                        "user"
                    };
                    history.push(make_text_percept(
                        actor,
                        place.as_str(),
                        hist_aud.members.iter().cloned(),
                        s_ms + (t_idx as u64) * 1_000,
                        turn.content,
                    ));
                }
            }

            let gold_evidence: Vec<GoldEvidenceRef> = if sc.evidence_turns.is_empty() {
                if history.is_empty() {
                    Vec::new()
                } else {
                    vec![GoldEvidenceRef {
                        turn_index: history.len() - 1,
                        audience: hist_aud.clone(),
                        place: place.clone(),
                    }]
                }
            } else {
                sc.evidence_turns
                    .into_iter()
                    .filter(|&ti| ti < history.len())
                    .map(|ti| GoldEvidenceRef {
                        turn_index: ti,
                        audience: hist_aud.clone(),
                        place: place.clone(),
                    })
                    .collect()
            };

            let probe_ts = base_ms + 1_000_000_000;
            let specs = [
                ("sr", sc.sr_query, sc.sr_answer),
                ("pr", sc.pr_query, sc.pr_answer),
                ("ipa", sc.ipa_query, sc.ipa_answer),
            ];
            let mut probes = Vec::with_capacity(3);
            for (p_idx, (dim, q_text, a_text)) in specs.into_iter().enumerate() {
                probes.push(ExternalProbe {
                    probe_id: dim.to_owned(),
                    percept: make_text_percept(
                        "user",
                        place.as_str(),
                        probe_aud.members.iter().cloned(),
                        probe_ts + (p_idx as u64) * 1_000,
                        q_text,
                    ),
                    isolated_checkpoint: true,
                    probe_scope: VisibleScope::new(probe_aud.clone(), Some(place.clone())),
                    expected_capability: None,
                });

                let mut metadata = BTreeMap::new();
                metadata.insert("conflict_type".to_owned(), conflict_type.clone());
                metadata.insert("dimension".to_owned(), dim.to_owned());

                gold_entries.push(GoldEntry {
                    item_id: item_id.clone(),
                    probe_id: dim.to_owned(),
                    cluster_id: item_id.clone(),
                    gold_answer: a_text.clone(),
                    accepted_answers: vec![a_text],
                    gold_evidence: gold_evidence.clone(),
                    privacy_conflict: false,
                    category: Some(dim.to_owned()),
                    metadata,
                });
            }

            items.push(ExternalItem {
                item_id: item_id.clone(),
                cluster_id: item_id,
                strata_key: conflict_type,
                history,
                checkpoint_group: Some(format!("stale_{idx:04}")),
                probes,
                history_mode: HistoryMode::StaticSilent,
                capabilities: Vec::new(),
            });
        }

        Ok(ConvertedDataset {
            dataset_id: "stale".to_owned(),
            pinned_revision: manifest_entry
                .sources
                .first()
                .map_or_else(|| "pending_pin".to_owned(), |s| s.revision.clone()),
            converter_version: self.converter_version().to_owned(),
            items,
            gold_book: GoldBook {
                dataset_id: "stale".to_owned(),
                entries: gold_entries,
            },
        })
    }
}

pub struct StaleScorer;

impl Scorer for StaleScorer {
    fn dataset_id(&self) -> &str {
        "stale"
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
                dataset_id: "stale".to_owned(),
                scorer_version: self.scorer_version().to_owned(),
                metrics: vec![
                    not_scored_metric(
                        "state_resolution",
                        "requires --live Gemini-3.1-flash-lite judge",
                    ),
                    not_scored_metric(
                        "premise_resistance",
                        "requires --live Gemini-3.1-flash-lite judge",
                    ),
                    not_scored_metric(
                        "implicit_policy_adaptation",
                        "requires --live Gemini-3.1-flash-lite judge",
                    ),
                ],
                judge_model: None,
                judge_prompt_version: None,
            });
        };
        let judge_model = judge.judge_model.unwrap_or("Gemini-3.1-flash-lite");
        let prompt_ver = judge.judge_prompt_version.unwrap_or("stale-v1");

        let mut sr_obs = Vec::new();
        let mut pr_obs = Vec::new();
        let mut ipa_obs = Vec::new();

        for (idx, pred) in predictions.iter().enumerate() {
            let entry = gold.find(&pred.item_id, &pred.probe_id).ok_or_else(|| {
                LabError::Protocol(format!(
                    "stale missing gold for {}:{}",
                    pred.item_id, pred.probe_id
                ))
            })?;
            let resp = pred.response_text.as_deref().unwrap_or("");
            let req = ModelRequest::text(
                EventId::new((idx + 1) as u64),
                ModelPurpose::Classify,
                judge_model,
                "STALE judge",
                format!(
                    "Dimension: {}\nRubric/Gold: {}\nResponse: {}\nPassed? yes or no.",
                    pred.probe_id, entry.gold_answer, resp
                ),
            );
            let res = jp
                .invoke(&req)
                .map_err(|e| LabError::Protocol(e.to_string()))?;
            let val = if matches!(res.output, ModelOutput::Verdict(v) if v.label.eq_ignore_ascii_case("yes"))
            {
                1.0
            } else {
                0.0
            };
            match pred.probe_id.as_str() {
                "sr" => sr_obs.push((pred.cluster_id.clone(), val)),
                "pr" => pr_obs.push((pred.cluster_id.clone(), val)),
                "ipa" => ipa_obs.push((pred.cluster_id.clone(), val)),
                _ => {}
            }
        }

        Ok(ScorerReport {
            dataset_id: "stale".to_owned(),
            scorer_version: self.scorer_version().to_owned(),
            metrics: vec![
                metric_with_cluster_ci("state_resolution", &sr_obs, seed),
                metric_with_cluster_ci("premise_resistance", &pr_obs, seed),
                metric_with_cluster_ci("implicit_policy_adaptation", &ipa_obs, seed),
            ],
            judge_model: Some(judge_model.to_owned()),
            judge_prompt_version: Some(prompt_ver.to_owned()),
        })
    }
}
