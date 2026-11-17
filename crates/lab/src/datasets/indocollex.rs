//! `indocollex` (`haryoa/indo-collex`) `monoagens_reason::Embedder` probe (`R-53` / Task 7.5).
//!
//! Evaluates `twin_retrieval_r1_256d` over Indonesian colloquialism `(transformed, original-for)` pairs.
//! Applies `R-54` default stratified sampling (`S=471`), rejects `Hash`/`None`/`Scripted`
//! outside tests, and emits [`EmbedderProbeOutcome::NotScored`] when semantic weights are absent.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use monoagens_reason::embed::{EmbedInput, Embedder};
use serde_json::Value;

use crate::LabError;
use crate::datasets::framework::{
    EmbedderProbe, EmbedderProbeOutcome, EmbedderReadiness, ScorerMetric, SplitMix64,
    validate_semantic_embedder,
};
use crate::datasets::scoring_utils::{default_sampling_rule, evaluate_twin_retrieval_r1};

/// `indocollex` component embedder probe.
#[derive(Debug, Default)]
pub struct IndoCollexProbe;

#[derive(Debug, Clone)]
struct CollexSample {
    id: String,
    stratum: String,
    transformed: String,
    original_for: String,
}

impl EmbedderProbe for IndoCollexProbe {
    fn dataset_id(&self) -> &str {
        "indocollex"
    }

    fn probe_version(&self) -> &str {
        "indocollex-embed-v1"
    }

    fn run_probe(
        &self,
        dataset_dir: &Path,
        embedder: Option<&dyn Embedder>,
        allow_test_embedders: bool,
        target_n: Option<usize>,
        seed: u64,
    ) -> Result<EmbedderProbeOutcome, LabError> {
        match validate_semantic_embedder(embedder, allow_test_embedders)? {
            EmbedderReadiness::Unconfigured(reason) => {
                return Ok(EmbedderProbeOutcome::NotScored {
                    dataset_id: self.dataset_id().to_owned(),
                    probe_version: self.probe_version().to_owned(),
                    reason,
                });
            }
            EmbedderReadiness::Ready => {}
        }

        let embedder = embedder.ok_or_else(|| {
            LabError::Protocol("indocollex probe reached Ready with None embedder".to_owned())
        })?;
        let samples = load_indocollex_twins(dataset_dir)?;
        let rule = default_sampling_rule(self.dataset_id(), seed);
        let limit = target_n.or(rule.target_n);
        let sampled = sample_collex_stratified(&samples, limit, seed);
        let tuples: Vec<(String, String, String)> = sampled
            .iter()
            .map(|s| (s.id.clone(), s.transformed.clone(), s.original_for.clone()))
            .collect();

        let (r1, chosen_ids) = evaluate_twin_retrieval_r1(&tuples, embedder)?;
        let meta_sample = embedder
            .embed(&EmbedInput::query(&tuples[0].1))
            .map_err(|e| LabError::Protocol(format!("embed metadata probe failed: {e}")))?
            .ok_or_else(|| LabError::Protocol("embedder returned None".to_owned()))?;

        Ok(EmbedderProbeOutcome::Scored {
            dataset_id: self.dataset_id().to_owned(),
            probe_version: self.probe_version().to_owned(),
            embed_model_ver: meta_sample.tag.embed_model_ver,
            dim_d: meta_sample.tag.dim_d,
            metrics: vec![ScorerMetric {
                name: "twin_retrieval_r1_256d".to_owned(),
                value: Some(r1),
                ci95_low: None,
                ci95_high: None,
                n_clusters: Some(chosen_ids.len()),
                not_scored_reason: None,
            }],
            item_count: chosen_ids.len(),
            chosen_item_ids: chosen_ids,
        })
    }
}

fn load_indocollex_twins(dataset_dir: &Path) -> Result<Vec<CollexSample>, LabError> {
    let csv_candidates = [
        dataset_dir.join("data/full.csv"),
        dataset_dir.join("data/test.csv"),
        dataset_dir.join("full.csv"),
        dataset_dir.join("test.csv"),
    ];
    if let Some(path) = csv_candidates.into_iter().find(|p| p.is_file()) {
        let text = fs::read_to_string(&path)?;
        let samples = parse_collex_csv(&text)?;
        if !samples.is_empty() {
            return Ok(samples);
        }
    }

    for name in ["indocollex.jsonl", "indocollex.json"] {
        let path = dataset_dir.join(name);
        if path.is_file() {
            let text = fs::read_to_string(&path)?;
            let mut out = Vec::new();
            if name.ends_with(".jsonl") {
                for (idx, line) in text.lines().enumerate() {
                    let t = line.trim();
                    if t.is_empty() {
                        continue;
                    }
                    let v: Value = serde_json::from_str(t)
                        .map_err(|e| LabError::Serialization(e.to_string()))?;
                    push_json_collex(&v, idx, &mut out);
                }
            } else {
                let v: Value = serde_json::from_str(&text)
                    .map_err(|e| LabError::Serialization(e.to_string()))?;
                if let Some(arr) = v.as_array() {
                    for (idx, item) in arr.iter().enumerate() {
                        push_json_collex(item, idx, &mut out);
                    }
                }
            }
            if !out.is_empty() {
                return Ok(out);
            }
        }
    }

    Err(LabError::Protocol(format!(
        "indocollex directory '{}' contains no data/full.csv or twin JSON/JSONL file",
        dataset_dir.display()
    )))
}

fn parse_collex_csv(text: &str) -> Result<Vec<CollexSample>, LabError> {
    let mut lines = text.lines();
    let Some(header_line) = lines.next() else {
        return Ok(Vec::new());
    };
    let headers = split_csv_line(header_line);
    let idx_no = headers
        .iter()
        .position(|h| h.eq_ignore_ascii_case("no"))
        .unwrap_or(0);
    let idx_trans = headers
        .iter()
        .position(|h| h.eq_ignore_ascii_case("transformed"))
        .ok_or_else(|| {
            LabError::Protocol("indocollex CSV missing 'transformed' column".to_owned())
        })?;
    let idx_orig = headers
        .iter()
        .position(|h| h.eq_ignore_ascii_case("original-for"))
        .ok_or_else(|| {
            LabError::Protocol("indocollex CSV missing 'original-for' column".to_owned())
        })?;
    let idx_kind = headers
        .iter()
        .position(|h| h.eq_ignore_ascii_case("transformation"));

    let mut out = Vec::new();
    for (row_idx, line) in lines.enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let cols = split_csv_line(trimmed);
        let transformed = cols.get(idx_trans).map_or("", String::as_str).trim();
        let original_for = cols.get(idx_orig).map_or("", String::as_str).trim();
        if transformed.is_empty() || original_for.is_empty() {
            continue;
        }
        let no = cols
            .get(idx_no)
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map_or_else(|| format!("row_{row_idx}"), ToOwned::to_owned);
        let stratum = idx_kind
            .and_then(|i| cols.get(i))
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .unwrap_or("default")
            .to_owned();
        out.push(CollexSample {
            id: format!("collex::{no}"),
            stratum,
            transformed: transformed.to_owned(),
            original_for: original_for.to_owned(),
        });
    }
    Ok(out)
}

fn split_csv_line(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut cur = String::new();
    let mut chars = line.chars().peekable();
    let mut in_quotes = false;
    while let Some(c) = chars.next() {
        match c {
            '"' if in_quotes && chars.peek() == Some(&'"') => {
                cur.push('"');
                chars.next();
            }
            '"' => in_quotes = !in_quotes,
            ',' if !in_quotes => {
                fields.push(std::mem::take(&mut cur));
            }
            _ => cur.push(c),
        }
    }
    fields.push(cur);
    fields
}

fn push_json_collex(v: &Value, idx: usize, out: &mut Vec<CollexSample>) {
    let Some(transformed) = v.get("transformed").and_then(Value::as_str) else {
        return;
    };
    let Some(original_for) = v.get("original-for").and_then(Value::as_str) else {
        return;
    };
    let id = v
        .get("no")
        .and_then(|n| {
            n.as_str()
                .map(ToOwned::to_owned)
                .or_else(|| n.as_u64().map(|u| u.to_string()))
        })
        .unwrap_or_else(|| format!("j{idx}"));
    let stratum = v
        .get("transformation")
        .and_then(Value::as_str)
        .unwrap_or("default");
    out.push(CollexSample {
        id: format!("collex::{id}"),
        stratum: stratum.to_owned(),
        transformed: transformed.to_owned(),
        original_for: original_for.to_owned(),
    });
}

fn sample_collex_stratified(
    samples: &[CollexSample],
    target_n: Option<usize>,
    seed: u64,
) -> Vec<&CollexSample> {
    let Some(limit) = target_n else {
        return samples.iter().collect();
    };
    if limit >= samples.len() {
        return samples.iter().collect();
    }
    let mut by_stratum: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (idx, s) in samples.iter().enumerate() {
        by_stratum.entry(s.stratum.as_str()).or_default().push(idx);
    }
    let mut rng = SplitMix64::new(seed);
    for idxs in by_stratum.values_mut() {
        rng.shuffle(idxs);
    }
    let mut chosen = Vec::with_capacity(limit);
    let mut cursor = 0usize;
    while chosen.len() < limit {
        let mut progressed = false;
        for idxs in by_stratum.values() {
            if cursor < idxs.len() {
                chosen.push(idxs[cursor]);
                progressed = true;
                if chosen.len() >= limit {
                    break;
                }
            }
        }
        if !progressed {
            break;
        }
        cursor += 1;
    }
    chosen.sort_unstable();
    chosen.into_iter().map(|i| &samples[i]).collect()
}
