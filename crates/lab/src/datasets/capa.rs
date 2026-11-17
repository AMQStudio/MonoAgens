//! `capa` (`FKIRSTE/emnlp2026-meeting-delegation`) AMI QMSum converter & GPT-4o schema-judge scorer.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use monoagens_protocol::{ActorId, Audience, EventId, PlaceId, VisibleScope};
use serde::Deserialize;

use crate::LabError;
use crate::datasets::framework::{
    ConvertedDataset, Converter, ExternalItem, ExternalPrediction, ExternalProbe, GoldBook,
    GoldEntry, GoldEvidenceRef, HistoryMode, JudgeExecutionConfig, Scorer, ScorerReport,
    make_text_percept,
};
use crate::datasets::manifest::DatasetManifestEntry;
use crate::datasets::scoring_utils::{
    invoke_judge_resilient, metric_with_cluster_ci, not_scored_metric, run_optional_python_helper,
};
use crate::sha256_hex;

pub struct CapaConverter;

#[derive(Deserialize)]
struct RawCapaMeeting {
    meeting_id: String,
    #[serde(default)]
    split: Option<String>,
    absent_role: String,
    briefing: String,
    transcript: Vec<RawCapaTurn>,
    episodes: Vec<RawCapaEpisode>,
}

#[derive(Deserialize)]
struct RawCapaTurn {
    speaker: String,
    content: String,
}

#[derive(Deserialize)]
struct RawCapaEpisode {
    episode_id: String,
    cue_turn: usize,
    anchor_turn: usize,
    #[serde(default = "default_k")]
    window_k: usize,
    target_proposition: String,
}

fn default_k() -> usize {
    3
}

impl Converter for CapaConverter {
    fn dataset_id(&self) -> &str {
        "capa"
    }

    fn converter_version(&self) -> &str {
        "1.0.0"
    }

    fn convert(
        &self,
        dataset_dir: &Path,
        manifest_entry: &DatasetManifestEntry,
    ) -> Result<ConvertedDataset, LabError> {
        let raw = fs::read_to_string(dataset_dir.join("data/ami/test.json"))?;
        let meetings: Vec<RawCapaMeeting> = serde_json::from_str(&raw)
            .map_err(|e| LabError::Serialization(format!("capa JSON error: {e}")))?;

        let soul = ActorId::new("soul");
        let proctor = ActorId::new("lab_proctor");
        let mut items = Vec::new();
        let mut gold_entries = Vec::new();

        for mtg in meetings {
            let split = mtg.split.unwrap_or_else(|| "test".to_owned());
            let place = PlaceId::new(format!("meeting:{}", mtg.meeting_id));
            let mut participants: BTreeSet<ActorId> = mtg
                .transcript
                .iter()
                .map(|t| ActorId::new(&t.speaker))
                .collect();
            participants.insert(ActorId::new("system:briefing"));
            participants.insert(proctor.clone());
            participants.insert(soul.clone());
            let scene_aud = Audience::new(participants);
            let probe_aud = Audience::new([proctor.clone(), soul.clone()]);
            let base_ms: u64 = 1_700_000_000_000;

            for ep in mtg.episodes {
                let item_id = format!("{}_{}", mtg.meeting_id, ep.episode_id);
                let mut history = Vec::new();
                history.push(make_text_percept(
                    "system:briefing",
                    place.as_str(),
                    scene_aud.members.iter().cloned(),
                    base_ms,
                    &mtg.briefing,
                ));

                // Stream transcript strictly up to `anchor_turn` (never leaking masked future turns).
                let end_turn = ep.anchor_turn.min(mtg.transcript.len().saturating_sub(1));
                for (t_idx, turn) in mtg.transcript[..=end_turn].iter().enumerate() {
                    if turn.speaker == mtg.absent_role && t_idx >= ep.cue_turn {
                        continue;
                    }
                    history.push(make_text_percept(
                        &turn.speaker,
                        place.as_str(),
                        scene_aud.members.iter().cloned(),
                        base_ms + ((t_idx + 1) as u64) * 1_000,
                        &turn.content,
                    ));
                }

                let probe_ts = base_ms + ((end_turn + 2) as u64) * 1_000;
                let probe = ExternalProbe {
                    probe_id: "delegate".to_owned(),
                    percept: make_text_percept(
                        "lab_proctor",
                        place.as_str(),
                        probe_aud.members.iter().cloned(),
                        probe_ts,
                        format!(
                            "Opportunity window for absent participant {}: speak or remain silent.",
                            mtg.absent_role
                        ),
                    ),
                    isolated_checkpoint: true,
                    probe_scope: VisibleScope::new(probe_aud.clone(), Some(place.clone())),
                    expected_capability: None,
                };

                let gold_evidence = vec![GoldEvidenceRef {
                    turn_index: 0,
                    audience: scene_aud.clone(),
                    place: place.clone(),
                }];

                let mut metadata = BTreeMap::new();
                metadata.insert("split".to_owned(), split.clone());
                metadata.insert("absent_role".to_owned(), mtg.absent_role.clone());
                metadata.insert("window_k".to_owned(), ep.window_k.to_string());

                gold_entries.push(GoldEntry {
                    item_id: item_id.clone(),
                    probe_id: "delegate".to_owned(),
                    cluster_id: mtg.meeting_id.clone(),
                    gold_answer: ep.target_proposition.clone(),
                    accepted_answers: vec![ep.target_proposition],
                    gold_evidence,
                    privacy_conflict: false,
                    category: Some(mtg.absent_role.clone()),
                    metadata,
                });

                items.push(ExternalItem {
                    item_id,
                    cluster_id: mtg.meeting_id.clone(),
                    strata_key: format!("{split}:{}", mtg.absent_role),
                    history,
                    checkpoint_group: None,
                    probes: vec![probe],
                    history_mode: HistoryMode::Interactive,
                    capabilities: Vec::new(),
                });
            }
        }

        Ok(ConvertedDataset {
            dataset_id: "capa".to_owned(),
            pinned_revision: manifest_entry
                .sources
                .first()
                .map_or_else(|| "pending_pin".to_owned(), |s| s.revision.clone()),
            converter_version: self.converter_version().to_owned(),
            items,
            gold_book: GoldBook {
                dataset_id: "capa".to_owned(),
                entries: gold_entries,
            },
        })
    }
}

pub struct CapaScorer;

impl Scorer for CapaScorer {
    fn dataset_id(&self) -> &str {
        "capa"
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
        let silence_obs: Vec<(String, f64)> = predictions
            .iter()
            .map(|p| (p.cluster_id.clone(), if !p.spoke { 1.0 } else { 0.0 }))
            .collect();

        let Some(jp) = judge.provider.filter(|_| judge.live) else {
            return Ok(ScorerReport {
                dataset_id: "capa".to_owned(),
                scorer_version: self.scorer_version().to_owned(),
                metrics: vec![
                    not_scored_metric(
                        "credited_recovery",
                        "requires --live gpt-4o schema judge / capa/judge.py",
                    ),
                    metric_with_cluster_ci("silence_rate", &silence_obs, seed),
                ],
                judge_model: None,
                judge_prompt_version: None,
            });
        };
        let judge_model = judge.judge_model.unwrap_or("gpt-4o");
        let capa_dataset_dir = judge.datasets_dir.and_then(|root| {
            let rel = judge
                .manifest
                .and_then(|m| m.get("capa"))
                .map_or("capa", |e| e.relative_dir.as_str());
            let dir = root.join(rel);
            dir.is_dir().then_some(dir)
        });

        let mut helper_items = Vec::with_capacity(predictions.len());
        for pred in predictions {
            let entry = gold.find(&pred.item_id, &pred.probe_id).ok_or_else(|| {
                LabError::Protocol(format!("capa missing gold for {}", pred.item_id))
            })?;
            let absent_role = entry
                .metadata
                .get("absent_role")
                .map(String::as_str)
                .unwrap_or("absent participant");
            let resp = if pred.spoke {
                pred.response_text.as_deref().unwrap_or("")
            } else {
                ""
            };
            helper_items.push(serde_json::json!({
                "item_id": pred.item_id,
                "absent_role": absent_role,
                "target_proposition": entry.gold_answer,
                "response_text": resp,
            }));
        }

        let ws_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let built =
            build_capa_judge_requests(&ws_root, capa_dataset_dir.as_deref(), &helper_items)?;
        let prompt_ver = judge
            .judge_prompt_version
            .unwrap_or(built.prompt_version.as_str());

        let mut recovery_obs = Vec::with_capacity(predictions.len());
        for (idx, (pred, req_spec)) in predictions.iter().zip(built.requests.iter()).enumerate() {
            let entry = gold.find(&pred.item_id, &pred.probe_id).expect("checked");
            if !pred.spoke || req_spec.skip {
                recovery_obs.push((pred.cluster_id.clone(), 0.0));
                judge.record_outcome(crate::datasets::framework::ScorerItemOutcome {
                    item_id: pred.item_id.clone(),
                    probe_id: pred.probe_id.clone(),
                    cluster_id: pred.cluster_id.clone(),
                    category: entry.category.clone(),
                    metric_name: "credited_recovery".to_owned(),
                    score: Some(0.0),
                    passed: Some(false),
                    judge_prompt: None,
                    judge_verdict: Some("skipped (soul stayed silent)".to_owned()),
                    failed: false,
                    failure_reason: None,
                });
                continue;
            }
            if let Some(ok) = invoke_judge_resilient(
                judge,
                jp,
                EventId::new((idx + 1) as u64),
                judge_model,
                &req_spec.system_prompt,
                &req_spec.user_prompt,
                &pred.item_id,
                &pred.probe_id,
                &pred.cluster_id,
                entry,
                "credited_recovery",
            )? {
                recovery_obs.push((pred.cluster_id.clone(), if ok { 1.0 } else { 0.0 }));
            }
        }

        Ok(ScorerReport {
            dataset_id: "capa".to_owned(),
            scorer_version: self.scorer_version().to_owned(),
            metrics: vec![
                metric_with_cluster_ci("credited_recovery", &recovery_obs, seed),
                metric_with_cluster_ci("silence_rate", &silence_obs, seed),
            ],
            judge_model: Some(judge_model.to_owned()),
            judge_prompt_version: Some(prompt_ver.to_owned()),
        })
    }
}

#[derive(Deserialize)]
struct CapaJudgeBuilderResponse {
    prompt_version: String,
    requests: Vec<CapaJudgeRequestSpec>,
}

#[derive(Deserialize)]
struct CapaJudgeRequestSpec {
    skip: bool,
    system_prompt: String,
    user_prompt: String,
}

fn build_capa_judge_requests(
    ws_root: &Path,
    dataset_dir: Option<&Path>,
    items: &[serde_json::Value],
) -> Result<CapaJudgeBuilderResponse, LabError> {
    let payload = serde_json::json!({
        "dataset_dir": dataset_dir.map(|p| p.to_string_lossy().into_owned()),
        "items": items,
    });
    if let Some(val) =
        run_optional_python_helper(ws_root, "lab/adapters/eval_helpers/capa_judge.py", &payload)?
    {
        let parsed: CapaJudgeBuilderResponse = serde_json::from_value(val).map_err(|e| {
            LabError::Serialization(format!("invalid capa_judge.py request builder JSON: {e}"))
        })?;
        if parsed.requests.len() == items.len() {
            return Ok(parsed);
        }
    }

    // Fallback pure-Rust CAPA prompt/schema builder (reads pinned files when present, never calls APIs).
    let default_system = "You are the CAPA meeting delegation judge. Decide whether a delegate's spoken utterance accurately recovers the target proposition for the absent participant without hallucination or contradiction. Return a classification verdict with label 'yes' if credited without hallucination, or 'no' otherwise.";
    let mut system_prompt = default_system.to_owned();
    let mut user_template: Option<String> = None;
    let mut schema_str = r#"{"additionalProperties":false,"properties":{"credited":{"type":"boolean"},"hallucinated":{"type":"boolean"}},"required":["credited","hallucinated"],"type":"object"}"#.to_owned();
    let mut loaded_pinned = false;

    if let Some(dir) = dataset_dir {
        for rel in [
            "prompts/delegate_judge.txt",
            "prompts/judge.txt",
            "capa/judge_prompt.txt",
        ] {
            let p = dir.join(rel);
            if p.is_file()
                && let Ok(text) = fs::read_to_string(&p)
            {
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    loaded_pinned = true;
                    if trimmed.contains("{target_proposition}")
                        || trimmed.contains("{response_text}")
                    {
                        user_template = Some(trimmed.to_owned());
                    } else {
                        system_prompt = trimmed.to_owned();
                    }
                    break;
                }
            }
        }
        for rel in [
            "capa/delegate_judge_schema.json",
            "capa/judge_schema.json",
            "prompts/judge_schema.json",
        ] {
            let p = dir.join(rel);
            if p.is_file()
                && let Ok(raw) = fs::read_to_string(&p)
                && serde_json::from_str::<serde_json::Value>(&raw).is_ok()
            {
                schema_str = raw.trim().to_owned();
                loaded_pinned = true;
                break;
            }
        }
    }

    let prompt_version = if loaded_pinned {
        let mut combined = system_prompt.clone();
        if let Some(t) = &user_template {
            combined.push_str(t);
        }
        combined.push_str(&schema_str);
        format!("capa-pinned-{}", &sha256_hex(combined.as_bytes())[..12])
    } else {
        "capa-judge-v1".to_owned()
    };

    let requests = items
        .iter()
        .map(|item| {
            let resp = item
                .get("response_text")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if resp.trim().is_empty() {
                return CapaJudgeRequestSpec {
                    skip: true,
                    system_prompt: system_prompt.clone(),
                    user_prompt: String::new(),
                };
            }
            let target = item
                .get("target_proposition")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let absent_role = item
                .get("absent_role")
                .and_then(|v| v.as_str())
                .unwrap_or("absent participant");
            let user_prompt = if let Some(tmpl) = &user_template {
                tmpl.replace("{target_proposition}", target)
                    .replace("{response_text}", resp)
                    .replace("{absent_role}", absent_role)
            } else {
                format!(
                    "Evaluate CAPA meeting delegation recovery.\nAbsent role: {absent_role}\nTarget proposition: {target}\nSpoken utterance: {resp}\nCAPA criteria schema: {schema_str}\nAnswer with verdict label 'yes' if credited without hallucination, or 'no' otherwise."
                )
            };
            CapaJudgeRequestSpec {
                skip: false,
                system_prompt: system_prompt.clone(),
                user_prompt,
            }
        })
        .collect();

    Ok(CapaJudgeBuilderResponse {
        prompt_version,
        requests,
    })
}
