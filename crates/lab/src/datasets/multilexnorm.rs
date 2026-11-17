//! `multilexnorm` (`ind` + `ind_eng`) `monoagens_reason::Embedder` probe (`R-53` / Task 7.5).
//!
//! Evaluates `twin_retrieval_r1_256d` over lexical-normalization twin pairs `(slang, canonical)`.
//! Rejects `Hash`/`None`/`Scripted` outside explicit unit/fixture tests and returns
//! [`EmbedderProbeOutcome::NotScored`] when semantic weights are unconfigured.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use monoagens_reason::embed::{EmbedInput, Embedder};
use serde_json::Value;

use crate::LabError;
use crate::datasets::framework::{
    EmbedderProbe, EmbedderProbeOutcome, EmbedderReadiness, ScorerMetric, SplitMix64,
    validate_semantic_embedder,
};
use crate::datasets::scoring_utils::{default_sampling_rule, evaluate_twin_retrieval_r1};

/// `multilexnorm` component embedder probe.
#[derive(Debug, Default)]
pub struct MultiLexNormProbe;

#[derive(Debug, Clone)]
struct TwinSample {
    id: String,
    stratum: String,
    orig: String,
    norm: String,
}

impl EmbedderProbe for MultiLexNormProbe {
    fn dataset_id(&self) -> &str {
        "multilexnorm"
    }

    fn probe_version(&self) -> &str {
        "multilexnorm-embed-v1"
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
            LabError::Protocol("multilexnorm probe reached Ready with None embedder".to_owned())
        })?;
        let samples = load_multilexnorm_twins(dataset_dir)?;
        let rule = default_sampling_rule(self.dataset_id(), seed);
        let limit = target_n.or(rule.target_n);
        let sampled = sample_twins_stratified(&samples, limit, seed);
        let tuples: Vec<(String, String, String)> = sampled
            .iter()
            .map(|s| (s.id.clone(), s.orig.clone(), s.norm.clone()))
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

fn load_multilexnorm_twins(dataset_dir: &Path) -> Result<Vec<TwinSample>, LabError> {
    let mut out = Vec::new();
    for lang in ["ind", "ind_eng"] {
        let candidates = [
            dataset_dir.join("data").join(lang).join("test.norm"),
            dataset_dir.join(lang).join("test.norm"),
            dataset_dir.join("data").join(lang).join("dev.norm"),
            dataset_dir.join(lang).join("dev.norm"),
            dataset_dir.join(format!("{lang}_test.norm")),
        ];
        if let Some(path) = candidates.into_iter().find(|p| p.is_file()) {
            let text = fs::read_to_string(&path)?;
            parse_norm_file(lang, &text, &mut out);
        }
    }

    if out.is_empty() {
        // JSON / JSONL fallback for pre-extracted twin files.
        let mut json_files: Vec<PathBuf> = Vec::new();
        for name in [
            "multilexnorm.json",
            "multilexnorm.jsonl",
            "twins.json",
            "twins.jsonl",
        ] {
            let p = dataset_dir.join(name);
            if p.is_file() {
                json_files.push(p);
            }
        }
        for path in json_files {
            let text = fs::read_to_string(&path)?;
            if path.extension().and_then(|e| e.to_str()) == Some("jsonl") {
                for (idx, line) in text.lines().enumerate() {
                    let t = line.trim();
                    if t.is_empty() {
                        continue;
                    }
                    let v: Value = serde_json::from_str(t)
                        .map_err(|e| LabError::Serialization(e.to_string()))?;
                    push_json_twin(&v, idx, &mut out);
                }
            } else {
                let v: Value = serde_json::from_str(&text)
                    .map_err(|e| LabError::Serialization(e.to_string()))?;
                if let Some(arr) = v.as_array() {
                    for (idx, item) in arr.iter().enumerate() {
                        push_json_twin(item, idx, &mut out);
                    }
                }
            }
        }
    }

    if out.is_empty() {
        return Err(LabError::Protocol(format!(
            "multilexnorm directory '{}' contains no .norm or twin files for ind/ind_eng",
            dataset_dir.display()
        )));
    }
    Ok(out)
}

fn parse_norm_file(lang: &str, text: &str, out: &mut Vec<TwinSample>) {
    let mut orig_tokens: Vec<String> = Vec::new();
    let mut norm_tokens: Vec<String> = Vec::new();
    let mut all_sentences: Vec<TwinSample> = Vec::new();
    let mut changed_sentences: Vec<TwinSample> = Vec::new();
    let mut sent_idx = 0usize;

    let flush_sentence = |orig_toks: &mut Vec<String>,
                          norm_toks: &mut Vec<String>,
                          idx: &mut usize,
                          all: &mut Vec<TwinSample>,
                          changed: &mut Vec<TwinSample>| {
        if orig_toks.is_empty() {
            return;
        }
        let orig = orig_toks.join(" ");
        let norm = norm_toks.join(" ");
        let sample = TwinSample {
            id: format!("{lang}::s{idx}"),
            stratum: lang.to_owned(),
            orig: orig.clone(),
            norm: norm.clone(),
        };
        if orig != norm {
            changed.push(sample.clone());
        }
        all.push(sample);
        orig_toks.clear();
        norm_toks.clear();
        *idx += 1;
    };

    for raw_line in text.lines() {
        let trimmed = raw_line.trim();
        if trimmed.is_empty() {
            flush_sentence(
                &mut orig_tokens,
                &mut norm_tokens,
                &mut sent_idx,
                &mut all_sentences,
                &mut changed_sentences,
            );
            continue;
        }
        let mut parts = trimmed.split('\t');
        let orig = parts.next().unwrap_or("").trim();
        let norm = parts.next().unwrap_or(orig).trim();
        if orig.is_empty() {
            continue;
        }
        orig_tokens.push(orig.to_owned());
        norm_tokens.push(if norm.is_empty() {
            orig.to_owned()
        } else {
            norm.to_owned()
        });
    }
    flush_sentence(
        &mut orig_tokens,
        &mut norm_tokens,
        &mut sent_idx,
        &mut all_sentences,
        &mut changed_sentences,
    );

    if !changed_sentences.is_empty() {
        out.extend(changed_sentences);
    } else {
        out.extend(all_sentences);
    }
}

fn push_json_twin(v: &Value, idx: usize, out: &mut Vec<TwinSample>) {
    let Some(orig) = v
        .get("orig")
        .or_else(|| v.get("slang"))
        .or_else(|| v.get("input"))
        .and_then(Value::as_str)
    else {
        return;
    };
    let Some(norm) = v
        .get("norm")
        .or_else(|| v.get("canonical"))
        .or_else(|| v.get("target"))
        .and_then(Value::as_str)
    else {
        return;
    };
    let lang = v.get("lang").and_then(Value::as_str).unwrap_or("ind");
    let id = v
        .get("id")
        .and_then(Value::as_str)
        .map_or_else(|| format!("{lang}::j{idx}"), ToOwned::to_owned);
    out.push(TwinSample {
        id,
        stratum: lang.to_owned(),
        orig: orig.to_owned(),
        norm: norm.to_owned(),
    });
}

fn sample_twins_stratified(
    samples: &[TwinSample],
    target_n: Option<usize>,
    seed: u64,
) -> Vec<&TwinSample> {
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
