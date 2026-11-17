//! Shared deterministic scoring, tokenization, Porter stemming, room-occupancy tracking,
//! `R-54` default sampling rules, `Embedder` twin-retrieval evaluation, and optional `lab/adapters`
//! Python helper invocation.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use monoagens_protocol::{ActorId, Audience, Percept, PlaceId};
use monoagens_reason::embed::{EmbedInput, Embedder, partitioned_cosine};
use serde_json::Value;

use crate::LabError;
use crate::datasets::framework::{
    GoldEvidenceRef, JudgeExecutionConfig, SamplingRule, ScorerMetric, cluster_bootstrap_ci,
    make_text_percept,
};
use crate::datasets::manifest::ManifestEntryKind;
use crate::datasets::ops::verify_manifest_entry;

/// Returns the `R-54` default sampling rule for `dataset_id`.
#[must_use]
pub fn default_sampling_rule(dataset_id: &str, seed: u64) -> SamplingRule {
    let (rule_name, target_n, sample_by_cluster) = match dataset_id {
        "locomo" => ("all_10_convs_1986_qas", None, false),
        "longmemeval" => ("longmemeval_s_500", None, false),
        "memory_agent_bench" => ("all_8_cr_22_ar_rows", None, true),
        "group_mem_bench" => ("all_4_domains_745_qas", None, false),
        "stale" => ("all_400_scenarios", None, false),
        "pm_bench" => ("full_80_steps", None, false),
        "capa" => ("ami_20_test_meetings", None, true),
        "proactive_bench" => ("official_test_233", None, false),
        "pitch" => ("all_384_candidates", None, false),
        "when2speak" => ("finetune_test_token_stratified_s1000", Some(1_000), false),
        "ubuntu_irc" => ("contiguous_test_5000", None, true),
        "molweni" => ("dp_500_mrc_stratified_500", Some(500), false),
        "fantom" => ("short_all_256_convs_703_sets", None, false),
        "opentom" => ("stratified_471_narratives", Some(471), true),
        "hi_tom" => ("whole_stories_1200_qas", None, true),
        "exploretom" => ("stratified_471", Some(471), false),
        "dyntom" => ("stratified_471_contexts", Some(471), true),
        "tomi" => ("stratified_471_perturbation_twins", Some(471), false),
        "multilexnorm" => ("ind_and_ind_eng_test", None, false),
        "indocollex" => ("stratified_471_full_or_test", Some(471), false),
        _ => ("full_split", None, false),
    };
    SamplingRule {
        rule_name: rule_name.to_owned(),
        seed,
        target_n,
        sample_by_cluster,
    }
}

/// Normalizes text for QA token F1 / exact match (lowercase, strip punctuation & articles).
#[must_use]
pub fn normalize_qa_tokens(text: &str, stem: bool) -> Vec<String> {
    let cleaned: String = text
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { ' ' })
        .collect();
    cleaned
        .split_whitespace()
        .filter(|tok| !matches!(*tok, "a" | "an" | "the" | "and"))
        .map(|tok| {
            if stem {
                porter_stem_light(tok)
            } else {
                tok.to_owned()
            }
        })
        .collect()
}

/// Lightweight deterministic Porter-style suffix stemmer matching `locomo/evaluation.py` needs.
#[must_use]
pub fn porter_stem_light(word: &str) -> String {
    let mut w = word.to_ascii_lowercase();
    if w.len() <= 3 {
        return w;
    }
    if let Some(stem) = w.strip_suffix("sses") {
        return format!("{stem}ss");
    }
    if let Some(stem) = w.strip_suffix("ies")
        && stem.len() > 1
    {
        return format!("{stem}i");
    }
    if w.ends_with('s') && !w.ends_with("ss") && w.len() > 3 {
        w.pop();
    }
    if let Some(stem) = w.strip_suffix("ing")
        && stem.len() >= 3
    {
        return stem.to_owned();
    }
    if let Some(stem) = w.strip_suffix("ed")
        && stem.len() >= 3
    {
        return stem.to_owned();
    }
    if let Some(stem) = w.strip_suffix("ly")
        && stem.len() >= 3
    {
        return stem.to_owned();
    }
    w
}

/// Multiset token-level F1 score between `pred` and `gold`.
#[must_use]
pub fn token_f1(pred: &str, gold: &str, stem: bool) -> f64 {
    let p_toks = normalize_qa_tokens(pred, stem);
    let g_toks = normalize_qa_tokens(gold, stem);
    if p_toks.is_empty() && g_toks.is_empty() {
        return 1.0;
    }
    if p_toks.is_empty() || g_toks.is_empty() {
        return 0.0;
    }
    let mut g_counts: BTreeMap<&str, usize> = BTreeMap::new();
    for t in &g_toks {
        *g_counts.entry(t.as_str()).or_insert(0) += 1;
    }
    let mut overlap = 0usize;
    for t in &p_toks {
        if let Some(c) = g_counts.get_mut(t.as_str())
            && *c > 0
        {
            *c -= 1;
            overlap += 1;
        }
    }
    if overlap == 0 {
        return 0.0;
    }
    let prec = overlap as f64 / p_toks.len() as f64;
    let rec = overlap as f64 / g_toks.len() as f64;
    2.0 * prec * rec / (prec + rec)
}

/// Normalized substring exact match (`SubEM`) against any accepted gold answer.
#[must_use]
pub fn substring_exact_match(pred: &str, accepted: &[String]) -> f64 {
    let p_norm = normalize_qa_tokens(pred, false).join(" ");
    if p_norm.is_empty() {
        return 0.0;
    }
    for g in accepted {
        let g_norm = normalize_qa_tokens(g, false).join(" ");
        if !g_norm.is_empty() && p_norm.contains(&g_norm) {
            return 1.0;
        }
    }
    0.0
}

/// Exact normalized string match against any accepted gold answer.
#[must_use]
pub fn exact_match_normalized(pred: &str, accepted: &[String]) -> f64 {
    let p_norm = normalize_qa_tokens(pred, false).join(" ");
    for g in accepted {
        let g_norm = normalize_qa_tokens(g, false).join(" ");
        if p_norm == g_norm {
            return 1.0;
        }
    }
    0.0
}

/// Precision, Recall, F1 from `(tp, fp, fn)`.
#[must_use]
pub fn binary_prf1(tp: usize, fp: usize, fn_: usize) -> (f64, f64, f64) {
    let p = if tp + fp == 0 {
        0.0
    } else {
        tp as f64 / (tp + fp) as f64
    };
    let r = if tp + fn_ == 0 {
        0.0
    } else {
        tp as f64 / (tp + fn_) as f64
    };
    let f1 = if p + r <= 0.0 {
        0.0
    } else {
        2.0 * p * r / (p + r)
    };
    (p, r, f1)
}

/// Macro-averaged F1 across discrete string labels.
#[must_use]
pub fn macro_f1_labels(pairs: &[(String, String)]) -> f64 {
    if pairs.is_empty() {
        return 0.0;
    }
    let mut labels = BTreeSet::new();
    for (g, p) in pairs {
        labels.insert(g.as_str());
        labels.insert(p.as_str());
    }
    let mut sum_f1 = 0.0;
    let mut count = 0usize;
    for lbl in labels {
        let mut tp = 0usize;
        let mut fp = 0usize;
        let mut fn_ = 0usize;
        for (g, p) in pairs {
            match (p == lbl, g == lbl) {
                (true, true) => tp += 1,
                (true, false) => fp += 1,
                (false, true) => fn_ += 1,
                (false, false) => {}
            }
        }
        if tp + fp + fn_ > 0 {
            sum_f1 += binary_prf1(tp, fp, fn_).2;
            count += 1;
        }
    }
    if count == 0 {
        0.0
    } else {
        sum_f1 / count as f64
    }
}

/// Builds a [`ScorerMetric`] with cluster-bootstrap CI over `(cluster_id, value)` observations.
#[must_use]
pub fn metric_with_cluster_ci(
    name: &str,
    observations: &[(String, f64)],
    seed: u64,
) -> ScorerMetric {
    if let Some(ci) = cluster_bootstrap_ci(observations, seed, 200) {
        ScorerMetric {
            name: name.to_owned(),
            value: Some(ci.point_estimate),
            ci95_low: Some(ci.ci95_low),
            ci95_high: Some(ci.ci95_high),
            n_clusters: Some(ci.n_clusters),
            not_scored_reason: None,
        }
    } else {
        ScorerMetric {
            name: name.to_owned(),
            value: None,
            ci95_low: None,
            ci95_high: None,
            n_clusters: None,
            not_scored_reason: Some("no observations".to_owned()),
        }
    }
}

/// Builds a `not_scored` [`ScorerMetric`] placeholder.
#[must_use]
pub fn not_scored_metric(name: &str, reason: &str) -> ScorerMetric {
    ScorerMetric {
        name: name.to_owned(),
        value: None,
        ci95_low: None,
        ci95_high: None,
        n_clusters: None,
        not_scored_reason: Some(reason.to_owned()),
    }
}

/// Shared Mode-1 event/line percept stream builder with dynamic room-occupancy tracking,
/// private `Tell` side-channels, and explicit `lab_proctor` observer (`R-53`).
pub fn build_mode1_tom_history(
    lines: &[String],
    place_prefix: &str,
    initial_actors: &[String],
    start_ms: u64,
    step_ms: u64,
) -> (Vec<Percept>, Vec<GoldEvidenceRef>) {
    let soul = ActorId::new("soul");
    let proctor = ActorId::new("lab_proctor");
    let mut occupants: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let default_room = format!("{place_prefix}_main");
    let room_set = occupants.entry(default_room.clone()).or_default();
    for a in initial_actors {
        room_set.insert(a.clone());
    }

    let mut percepts = Vec::with_capacity(lines.len());
    let mut evidence = Vec::with_capacity(lines.len());

    for (idx, line) in lines.iter().enumerate() {
        let ts = start_ms + (idx as u64) * step_ms;
        let lower = line.to_ascii_lowercase();
        let tokens: Vec<&str> = line.split_whitespace().collect();
        let first_actor = tokens
            .first()
            .map(|s| s.trim_matches(|c: char| !c.is_alphanumeric() && c != '_'))
            .filter(|s| !s.is_empty() && !s.chars().all(|c| c.is_ascii_digit()))
            .or_else(|| tokens.get(1).copied())
            .unwrap_or("narrator")
            .to_owned();

        // Check room entry / exit / private Tell patterns.
        let mut current_room = default_room.clone();
        if let Some(room_pos) = tokens.iter().position(|t| {
            let tl = t.trim_matches(|c: char| !c.is_alphanumeric());
            tl.eq_ignore_ascii_case("hall")
                || tl.eq_ignore_ascii_case("kitchen")
                || tl.eq_ignore_ascii_case("garden")
                || tl.eq_ignore_ascii_case("office")
                || tl.eq_ignore_ascii_case("bedroom")
                || tl.eq_ignore_ascii_case("room")
        }) {
            current_room = format!(
                "{place_prefix}_{}",
                tokens[room_pos]
                    .trim_matches(|c: char| !c.is_alphanumeric())
                    .to_ascii_lowercase()
            );
        }

        let aud_members: Vec<ActorId> = if lower.contains("privately told")
            || lower.contains("whispered to")
        {
            // Private side-channel: {sender, recipient, lab_proctor, soul}.
            let recipient = tokens
                .iter()
                .skip_while(|t| !t.eq_ignore_ascii_case("told") && !t.eq_ignore_ascii_case("to"))
                .nth(1)
                .map(|s| s.trim_matches(|c: char| !c.is_alphanumeric()))
                .filter(|s| !s.is_empty())
                .unwrap_or(&first_actor);
            vec![
                ActorId::new(&first_actor),
                ActorId::new(recipient),
                proctor.clone(),
                soul.clone(),
            ]
        } else {
            if lower.contains("exited") || lower.contains("left") {
                for set in occupants.values_mut() {
                    set.remove(&first_actor);
                }
            } else {
                occupants
                    .entry(current_room.clone())
                    .or_default()
                    .insert(first_actor.clone());
            }
            let mut members: Vec<ActorId> = occupants
                .get(&current_room)
                .into_iter()
                .flat_map(|s| s.iter().map(ActorId::new))
                .collect();
            if members.is_empty() {
                members.push(ActorId::new(&first_actor));
            }
            members.push(proctor.clone());
            members.push(soul.clone());
            members
        };

        let aud = Audience::new(aud_members);
        let place = PlaceId::new(&current_room);
        percepts.push(make_text_percept(
            &first_actor,
            place.as_str(),
            aud.members.iter().cloned(),
            ts,
            line,
        ));
        evidence.push(GoldEvidenceRef {
            turn_index: idx,
            audience: aud,
            place,
        });
    }

    (percepts, evidence)
}

/// Evaluates `twin_retrieval_r1_256d` directly via `monoagens_reason::Embedder`.
/// Input tuples: `(item_id, query_text, norm_target_text)`.
pub fn evaluate_twin_retrieval_r1(
    items: &[(String, String, String)],
    embedder: &dyn Embedder,
) -> Result<(f64, Vec<String>), LabError> {
    if items.is_empty() {
        return Err(LabError::Protocol(
            "embedder probe dataset contains zero items".to_owned(),
        ));
    }

    let mut q_vecs = Vec::with_capacity(items.len());
    let mut d_vecs = Vec::with_capacity(items.len());
    let mut chosen_ids = Vec::with_capacity(items.len());

    for (id, orig, norm) in items {
        let q = embedder
            .embed(&EmbedInput::query(orig))
            .map_err(|e| LabError::Protocol(format!("embed query '{id}' failed: {e}")))?
            .ok_or_else(|| LabError::Protocol("embedder returned None".to_owned()))?;
        let d = embedder
            .embed(&EmbedInput::document(norm))
            .map_err(|e| LabError::Protocol(format!("embed document '{id}' failed: {e}")))?
            .ok_or_else(|| LabError::Protocol("embedder returned None".to_owned()))?;
        q_vecs.push(q);
        d_vecs.push(d);
        chosen_ids.push(id.clone());
    }

    let mut hits = 0usize;
    for (qi, q_emb) in q_vecs.iter().enumerate() {
        let mut best_idx = 0usize;
        let mut best_sim = f32::NEG_INFINITY;
        for (di, d_emb) in d_vecs.iter().enumerate() {
            // Same embed_model_ver & dim_d, compare cosine using raw values (or query vs doc).
            let sim = if q_emb.tag.prompt_regime_id == d_emb.tag.prompt_regime_id {
                partitioned_cosine(q_emb, d_emb)
                    .map_err(|e| LabError::Protocol(format!("cosine failed: {e}")))?
            } else {
                // Asymmetric t_q -> t_d dot product over L2-normalized vectors.
                if q_emb.tag.embed_model_ver != d_emb.tag.embed_model_ver
                    || q_emb.tag.dim_d != d_emb.tag.dim_d
                {
                    return Err(LabError::Protocol("embedder partition mismatch".to_owned()));
                }
                q_emb
                    .values
                    .iter()
                    .zip(d_emb.values.iter())
                    .map(|(a, b)| a * b)
                    .sum::<f32>()
            };
            if sim > best_sim {
                best_sim = sim;
                best_idx = di;
            }
        }
        if best_idx == qi {
            hits += 1;
        }
    }

    Ok((hits as f64 / items.len() as f64, chosen_ids))
}

/// Resolves and verifies a fetched `scorer_model` directory under `datasets_dir` via the
/// manifest. Returns `Ok(None)` when `datasets_dir`/`manifest` are absent or the scorer model
/// has not been pinned, fetched, and verified locally (never triggers network downloads).
pub fn resolve_verified_scorer_model_dir(
    judge: &JudgeExecutionConfig<'_>,
    model_id: &str,
) -> Result<Option<PathBuf>, LabError> {
    let (Some(datasets_dir), Some(manifest)) = (judge.datasets_dir, judge.manifest) else {
        return Ok(None);
    };
    let Some(entry) = manifest.get(model_id) else {
        return Ok(None);
    };
    if entry.kind != ManifestEntryKind::ScorerModel {
        return Ok(None);
    }
    let verify = verify_manifest_entry(entry, datasets_dir)?;
    if !verify.verified {
        return Ok(None);
    }
    Ok(Some(datasets_dir.join(&entry.relative_dir)))
}

/// Optional invocation of a Python helper under `lab/adapters/` when explicitly requested
/// (`#[ignore]`d tests or owner environments). Forces `HF_HUB_OFFLINE=1` and
/// `TRANSFORMERS_OFFLINE=1` so Python helpers can never open Hugging Face network connections.
/// Returns `Ok(None)` when Python/packages are absent.
pub fn run_optional_python_helper(
    workspace_root: &Path,
    script_rel: &str,
    payload: &Value,
) -> Result<Option<Value>, LabError> {
    let script = workspace_root.join(script_rel);
    if !script.is_file() {
        return Ok(None);
    }
    let py = std::env::var("MONOAGENS_ADAPTER_PYTHON").unwrap_or_else(|_| "python3".to_owned());
    let Ok(mut child) = Command::new(&py)
        .arg(&script)
        .env("HF_HUB_OFFLINE", "1")
        .env("TRANSFORMERS_OFFLINE", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    else {
        return Ok(None);
    };

    if let Some(mut stdin) = child.stdin.take() {
        let bytes =
            serde_json::to_vec(payload).map_err(|e| LabError::Serialization(e.to_string()))?;
        let _ = stdin.write_all(&bytes);
    }
    let out = child.wait_with_output()?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        if stderr.contains("ModuleNotFoundError") || stderr.contains("ImportError") {
            return Ok(None);
        }
        return Err(LabError::Protocol(format!(
            "python helper '{}' failed: {}",
            script.display(),
            stderr.trim()
        )));
    }
    let parsed: Value = serde_json::from_slice(&out.stdout)
        .map_err(|e| LabError::Serialization(format!("invalid helper JSON: {e}")))?;
    Ok(Some(parsed))
}
