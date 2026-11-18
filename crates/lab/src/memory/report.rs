//! Memory-only report generated from phase 0 run records (`Design 5`, spec
//! "The memory report shows quality against token cost").
//!
//! The report contains MonoAgens' own results only: no scores published by others, no other
//! benchmark category.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::LabError;
use crate::memory::scoring::MemoryItemScore;
use crate::memory::suite::{DatasetArtifact, MemoryRunFile};
use crate::record::{MetricScoreInterval, RunRecord, UsageBreakdown};

/// Paths written by [`generate_memory_report`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GeneratedMemoryReport {
    pub markdown_path: PathBuf,
    pub html_path: PathBuf,
}

struct LoadedDataset {
    artifact: DatasetArtifact,
    record: RunRecord,
    items: Vec<MemoryItemScore>,
}

fn load(run_dir: &Path) -> Result<(MemoryRunFile, Vec<LoadedDataset>), LabError> {
    let run_raw = fs::read_to_string(run_dir.join("memory_run.json"))?;
    let run: MemoryRunFile =
        serde_json::from_str(&run_raw).map_err(|e| LabError::Serialization(e.to_string()))?;
    let mut datasets = Vec::new();
    for artifact in &run.datasets {
        let record_raw = fs::read_to_string(run_dir.join(&artifact.record_rel_path))?;
        let record: RunRecord = serde_json::from_str(&record_raw)
            .map_err(|e| LabError::Serialization(e.to_string()))?;
        let items: Vec<MemoryItemScore> =
            fs::read_to_string(run_dir.join(&artifact.items_rel_path))
                .ok()
                .and_then(|raw| serde_json::from_str(&raw).ok())
                .unwrap_or_default();
        datasets.push(LoadedDataset {
            artifact: artifact.clone(),
            record,
            items,
        });
    }
    Ok((run, datasets))
}

fn metric<'a>(record: &'a RunRecord, name: &str) -> Option<&'a MetricScoreInterval> {
    record
        .dataset_provenance
        .as_ref()?
        .metric_scores
        .iter()
        .find(|m| m.metric == name)
}

fn metric_value(record: &RunRecord, name: &str) -> Option<f64> {
    metric(record, name).and_then(|m| m.value)
}

fn fmt_score(value: Option<f64>) -> String {
    value.map_or_else(|| "—".to_owned(), |v| format!("{v:.3}"))
}

fn fmt_ci(record: &RunRecord, name: &str) -> String {
    match metric(record, name) {
        Some(m) if m.value.is_some() => match (m.ci95_low, m.ci95_high) {
            (Some(lo), Some(hi)) => format!("[{lo:.3}, {hi:.3}]"),
            _ => "—".to_owned(),
        },
        _ => "—".to_owned(),
    }
}

fn latencies(record: &RunRecord) -> Vec<u64> {
    record
        .dataset_provenance
        .as_ref()
        .and_then(|p| p.memory_usage.as_ref())
        .map(|u| u.latencies_ms.clone())
        .unwrap_or_default()
}

fn percentile(values: &[u64], p: f64) -> u64 {
    if values.is_empty() {
        return 0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let idx = ((p / 100.0) * (sorted.len() as f64 - 1.0)).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

fn category_counts(items: &[MemoryItemScore]) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for item in items {
        *counts.entry(item.category.clone()).or_insert(0) += 1;
    }
    counts
}

fn silence_rate(record: &RunRecord) -> f64 {
    metric_value(record, "silence_rate").unwrap_or(0.0)
}

fn cached_share(usage: &UsageBreakdown) -> f64 {
    let total = usage.uncached_input_tokens + usage.cached_input_tokens;
    if total == 0 {
        0.0
    } else {
        usage.cached_input_tokens as f64 / total as f64
    }
}

fn no_cache_cost(usage: &UsageBreakdown) -> f64 {
    (usage.uncached_input_tokens + usage.cached_input_tokens) as f64 * 0.15 / 1_000_000.0
        + usage.output_tokens as f64 * 0.60 / 1_000_000.0
}

type CategoryRow = (String, usize, Option<f64>, Option<f64>, String);

fn locomo_rows(ds: &LoadedDataset) -> Vec<CategoryRow> {
    let counts = category_counts(&ds.items);
    let mut rows = Vec::new();
    for cat in 1..=5_u8 {
        let name = format!("cat_{cat}");
        let n = counts.get(&name).copied().unwrap_or(0);
        if n == 0 {
            continue;
        }
        let f1 = metric_value(&ds.record, &format!("qa_f1_cat_{cat}"));
        let judge = metric_value(&ds.record, &format!("judge_acc_cat_{cat}"));
        let ci = fmt_ci(&ds.record, &format!("judge_acc_cat_{cat}"));
        rows.push((name, n, f1, judge, ci));
    }
    rows
}

fn lme_rows(ds: &LoadedDataset) -> Vec<(String, usize, Option<f64>, String)> {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for item in &ds.items {
        if !item.is_abs {
            *counts.entry(item.category.clone()).or_insert(0) += 1;
        }
    }
    let mut rows = Vec::new();
    for (qtype, n) in counts {
        let name = format!("judge_acc_{qtype}");
        rows.push((
            qtype,
            n,
            metric_value(&ds.record, &name),
            fmt_ci(&ds.record, &name),
        ));
    }
    rows
}

fn failure_examples(ds: &LoadedDataset, per_category: usize) -> Vec<&MemoryItemScore> {
    let mut by_category: BTreeMap<&str, Vec<&MemoryItemScore>> = BTreeMap::new();
    for item in &ds.items {
        if item.judge_correct == Some(false) || item.judge_failed {
            by_category
                .entry(item.category.as_str())
                .or_default()
                .push(item);
        }
    }
    let mut out = Vec::new();
    for (_cat, items) in by_category {
        out.extend(items.into_iter().take(per_category));
    }
    out
}

fn write_csv(path: &Path, header: &[&str], rows: &[Vec<String>]) -> Result<(), LabError> {
    let mut out = String::new();
    out.push_str(&header.join(","));
    out.push('\n');
    for row in rows {
        let escaped: Vec<String> = row
            .iter()
            .map(|cell| {
                if cell.contains(',') || cell.contains('"') {
                    format!("\"{}\"", cell.replace('"', "\"\""))
                } else {
                    cell.clone()
                }
            })
            .collect();
        out.push_str(&escaped.join(","));
        out.push('\n');
    }
    fs::write(path, out)?;
    Ok(())
}

type Series = (String, f64, Option<(f64, f64)>, &'static str);
type Group = (String, Vec<Series>);

fn svg_bar_chart(title: &str, groups: &[Group], width: u32, height: u32) -> String {
    let max = groups
        .iter()
        .flat_map(|(_, bars)| bars.iter().map(|(_, v, _, _)| *v))
        .fold(0.0_f64, f64::max)
        .max(1.0);
    let left = 60.0_f64;
    let bottom = 70.0;
    let plot_w = f64::from(width) - left - 20.0;
    let plot_h = f64::from(height) - 50.0 - bottom;
    let group_w = plot_w / groups.len().max(1) as f64;
    let mut svg = String::new();
    let _ = writeln!(
        svg,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {width} {height}\" font-family=\"system-ui,sans-serif\" font-size=\"12\">"
    );
    let _ = writeln!(
        svg,
        "<text x=\"{left}\" y=\"22\" font-size=\"15\">{title}</text>"
    );
    for tick in 0..=5 {
        let frac = f64::from(tick) / 5.0;
        let y = 40.0 + plot_h - frac * plot_h;
        let _ = writeln!(
            svg,
            "<line x1=\"{left}\" y1=\"{y:.1}\" x2=\"{:.1}\" y2=\"{y:.1}\" stroke=\"#d0d0d0\"/><text x=\"{:.1}\" y=\"{:.1}\" text-anchor=\"end\" fill=\"#555\">{:.1}</text>",
            left + plot_w,
            left - 6.0,
            y + 4.0,
            frac * max
        );
    }
    for (gi, (label, bars)) in groups.iter().enumerate() {
        let gx = left + gi as f64 * group_w;
        let bar_w = (group_w * 0.7) / bars.len().max(1) as f64;
        for (bi, (series, value, ci, color)) in bars.iter().enumerate() {
            let x = gx + group_w * 0.15 + bi as f64 * bar_w;
            let h = (value / max) * plot_h;
            let y = 40.0 + plot_h - h;
            let _ = writeln!(
                svg,
                "<rect x=\"{x:.1}\" y=\"{y:.1}\" width=\"{:.1}\" height=\"{h:.1}\" fill=\"{color}\"/>",
                (bar_w * 0.85).max(1.0)
            );
            if let Some((lo, hi)) = ci {
                let cx = x + bar_w * 0.425;
                let y_lo = 40.0 + plot_h - (lo / max) * plot_h;
                let y_hi = 40.0 + plot_h - (hi / max) * plot_h;
                let _ = writeln!(
                    svg,
                    "<line x1=\"{cx:.1}\" y1=\"{y_lo:.1}\" x2=\"{cx:.1}\" y2=\"{y_hi:.1}\" stroke=\"#222\"/><line x1=\"{:.1}\" y1=\"{y_lo:.1}\" x2=\"{:.1}\" y2=\"{y_lo:.1}\" stroke=\"#222\"/><line x1=\"{:.1}\" y1=\"{y_hi:.1}\" x2=\"{:.1}\" y2=\"{y_hi:.1}\" stroke=\"#222\"/>",
                    cx - 5.0,
                    cx + 5.0,
                    cx - 5.0,
                    cx + 5.0
                );
            }
            let _ = writeln!(
                svg,
                "<text x=\"{:.1}\" y=\"{:.1}\" text-anchor=\"middle\" fill=\"#333\">{value:.2}</text>",
                x + bar_w * 0.425,
                y - 4.0
            );
            let _ = series;
        }
        let _ = writeln!(
            svg,
            "<text x=\"{:.1}\" y=\"{:.1}\" text-anchor=\"middle\">{label}</text>",
            gx + group_w / 2.0,
            40.0 + plot_h + 20.0
        );
    }
    let _ = writeln!(svg, "</svg>");
    svg
}

fn svg_stacked_tokens(rows: &[(String, u64, u64, u64, u64)], width: u32, height: u32) -> String {
    let max = rows
        .iter()
        .map(|(_, a, b, _, _)| a + b)
        .max()
        .unwrap_or(1)
        .max(1);
    let left = 70.0_f64;
    let plot_w = f64::from(width) - left - 30.0;
    let plot_h = f64::from(height) - 90.0;
    let row_h = plot_h / rows.len().max(1) as f64;
    let mut svg = String::new();
    let _ = writeln!(
        svg,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {width} {height}\" font-family=\"system-ui,sans-serif\" font-size=\"12\">"
    );
    let _ = writeln!(
        svg,
        "<text x=\"{left}\" y=\"22\" font-size=\"15\">Tokens per question (ingest vs answer)</text>"
    );
    for (i, (label, ingest, answer, _, _)) in rows.iter().enumerate() {
        let y = 50.0 + i as f64 * row_h;
        let w_ingest = (*ingest as f64 / max as f64) * plot_w;
        let w_answer = (*answer as f64 / max as f64) * plot_w;
        let bar_h = (row_h * 0.55).max(1.0);
        let _ = writeln!(
            svg,
            "<rect x=\"{left}\" y=\"{y:.1}\" width=\"{w_ingest:.1}\" height=\"{bar_h:.1}\" fill=\"#4e79a7\"/><rect x=\"{:.1}\" y=\"{y:.1}\" width=\"{w_answer:.1}\" height=\"{bar_h:.1}\" fill=\"#f28e2b\"/>",
            left + w_ingest
        );
        let _ = writeln!(
            svg,
            "<text x=\"{:.1}\" y=\"{:.1}\" text-anchor=\"end\" fill=\"#333\">{label}</text>",
            left - 6.0,
            y + bar_h
        );
        let _ = writeln!(
            svg,
            "<text x=\"{:.1}\" y=\"{:.1}\" fill=\"#333\">{ingest}/{answer}</text>",
            left + w_ingest + w_answer + 6.0,
            y + bar_h
        );
    }
    let _ = writeln!(
        svg,
        "<rect x=\"{left}\" y=\"{}\" width=\"12\" height=\"12\" fill=\"#4e79a7\"/><text x=\"{}\" y=\"{}\">ingest</text>",
        f64::from(height) - 22.0,
        left + 18.0,
        f64::from(height) - 12.0
    );
    let _ = writeln!(
        svg,
        "<rect x=\"{}\" y=\"{}\" width=\"12\" height=\"12\" fill=\"#f28e2b\"/><text x=\"{}\" y=\"{}\">answer</text>",
        left + 90.0,
        f64::from(height) - 22.0,
        left + 108.0,
        f64::from(height) - 12.0
    );
    let _ = writeln!(svg, "</svg>");
    svg
}

fn svg_quality_vs_tokens(points: &[(String, f64, f64)], width: u32, height: u32) -> String {
    let max_x = points.iter().map(|p| p.2).fold(1.0_f64, f64::max);
    let left = 60.0_f64;
    let bottom = 60.0;
    let plot_w = f64::from(width) - left - 30.0;
    let plot_h = f64::from(height) - 40.0 - bottom;
    let mut svg = String::new();
    let _ = writeln!(
        svg,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {width} {height}\" font-family=\"system-ui,sans-serif\" font-size=\"12\">"
    );
    let _ = writeln!(
        svg,
        "<text x=\"{left}\" y=\"22\" font-size=\"15\">Quality vs tokens per question</text>"
    );
    let _ = writeln!(
        svg,
        "<line x1=\"{left}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" stroke=\"#888\"/><line x1=\"{left}\" y1=\"40\" x2=\"{left}\" y2=\"{}\" stroke=\"#888\"/>",
        40.0 + plot_h,
        left + plot_w,
        40.0 + plot_h,
        40.0 + plot_h
    );
    for (label, score, tokens) in points {
        let x = left + (tokens / max_x) * plot_w;
        let y = 40.0 + plot_h - score.clamp(0.0, 1.0) * plot_h;
        let _ = writeln!(
            svg,
            "<circle cx=\"{x:.1}\" cy=\"{y:.1}\" r=\"5\" fill=\"#59a14f\"/><text x=\"{:.1}\" y=\"{:.1}\" fill=\"#333\">{label} ({score:.2}, {tokens:.0} tok/q)</text>",
            x + 8.0,
            y + 4.0
        );
    }
    let _ = writeln!(
        svg,
        "<text x=\"{:.1}\" y=\"{}\" text-anchor=\"middle\" fill=\"#555\">tokens / question</text>",
        left + plot_w / 2.0,
        f64::from(height) - 12.0
    );
    let _ = writeln!(svg, "</svg>");
    svg
}

fn svg_latency(latencies: &[u64], width: u32, height: u32) -> String {
    let mut buckets = [0usize; 12];
    let max = latencies.iter().copied().max().unwrap_or(1).max(1);
    for value in latencies {
        let idx = ((*value as f64 / max as f64) * 11.0).round() as usize;
        buckets[idx.min(11)] += 1;
    }
    let max_count = buckets.iter().copied().max().unwrap_or(1).max(1);
    let left = 50.0_f64;
    let bottom = 50.0;
    let plot_w = f64::from(width) - left - 20.0;
    let plot_h = f64::from(height) - 40.0 - bottom;
    let bar_w = plot_w / buckets.len() as f64;
    let mut svg = String::new();
    let _ = writeln!(
        svg,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {width} {height}\" font-family=\"system-ui,sans-serif\" font-size=\"12\">"
    );
    let _ = writeln!(
        svg,
        "<text x=\"{left}\" y=\"22\" font-size=\"15\">Model-call latency distribution (n={})</text>",
        latencies.len()
    );
    for (i, count) in buckets.iter().enumerate() {
        let h = (*count as f64 / max_count as f64) * plot_h;
        let x = left + i as f64 * bar_w;
        let y = 40.0 + plot_h - h;
        let _ = writeln!(
            svg,
            "<rect x=\"{x:.1}\" y=\"{y:.1}\" width=\"{:.1}\" height=\"{h:.1}\" fill=\"#76b7b2\"/>",
            bar_w * 0.85
        );
        let _ = writeln!(
            svg,
            "<text x=\"{:.1}\" y=\"{}\" text-anchor=\"middle\" fill=\"#555\">{}</text>",
            x + bar_w / 2.0,
            f64::from(height) - 22.0,
            (max as f64 / 11.0 * i as f64) as u64
        );
    }
    let _ = writeln!(svg, "</svg>");
    svg
}

#[derive(Debug, Default)]
struct DefectEvidence {
    classify_without_trigger: usize,
    classify_total: usize,
    classify_example: Option<String>,
    runaway_by_purpose: BTreeMap<String, usize>,
    runaway_example: Option<String>,
    schema_failures: BTreeMap<String, usize>,
    schema_example: Option<String>,
}

fn collect_defects(run_dir: &Path) -> DefectEvidence {
    let mut defects = DefectEvidence::default();
    let mut transcripts = Vec::new();
    if let Ok(entries) = fs::read_dir(run_dir.join("transcripts")) {
        for entry in entries.flatten() {
            let dir = entry.path();
            if !dir.is_dir() {
                continue;
            }
            if let Ok(files) = fs::read_dir(&dir) {
                transcripts.extend(files.flatten().map(|f| f.path()));
            }
        }
    }
    for path in transcripts {
        let Ok(raw) = fs::read_to_string(&path) else {
            continue;
        };
        for line in raw.lines() {
            let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
                continue;
            };
            if value.get("record_type").and_then(|v| v.as_str()) != Some("model_call") {
                continue;
            }
            let purpose = value.get("purpose").and_then(|v| v.as_str()).unwrap_or("");
            let user_prompt = value
                .get("user_prompt")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if purpose == "classify" {
                defects.classify_total += 1;
                if user_prompt.contains("[TRIGGER]\ntrigger: none") {
                    defects.classify_without_trigger += 1;
                    if defects.classify_example.is_none() {
                        defects.classify_example = Some(truncate(user_prompt, 240));
                    }
                }
            }
            let outcome = value.get("outcome").and_then(|v| v.as_str()).unwrap_or("");
            if outcome.starts_with("failed") {
                let detail = value
                    .get("error_detail")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");
                let kind = if detail.contains("motivation") {
                    "invalid motivation"
                } else if detail.contains("places") {
                    "invalid places"
                } else if detail.contains("label") {
                    "invalid classification label"
                } else if detail.contains("capability_id") {
                    "invalid capability_id"
                } else if detail.contains("timeout") || detail.contains("sending request") {
                    "timeout / transport"
                } else {
                    "other schema failure"
                };
                *defects.schema_failures.entry(kind.to_owned()).or_insert(0) += 1;
                if defects.schema_example.is_none() {
                    defects.schema_example = Some(format!("{kind}: {detail}"));
                }
            }
        }
    }
    if let Ok(raw) = fs::read_to_string(run_dir.join("ledger.jsonl")) {
        for line in raw.lines() {
            let Ok(entry) = serde_json::from_str::<serde_json::Value>(line) else {
                continue;
            };
            if entry.get("finish_reason").and_then(|v| v.as_str()) == Some("length") {
                let purpose = entry
                    .get("purpose")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_owned();
                *defects.runaway_by_purpose.entry(purpose).or_insert(0) += 1;
                if defects.runaway_example.is_none() {
                    defects.runaway_example = Some(format!(
                        "{} {} call hit finish_reason=length with {} output tokens in {} ms",
                        entry.get("dataset").and_then(|v| v.as_str()).unwrap_or("?"),
                        entry.get("phase").and_then(|v| v.as_str()).unwrap_or("?"),
                        entry
                            .get("output_tokens")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0),
                        entry
                            .get("latency_ms")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0)
                    ));
                }
            }
        }
    }
    defects
}

fn render_markdown(
    run: &MemoryRunFile,
    datasets: &[LoadedDataset],
    report_run_dir: &Path,
) -> String {
    let mut md = String::new();
    md.push_str("# MonoAgens memory benchmark — phase 0\n\n");
    let _ = writeln!(
        md,
        "> **{}**, run `{}`, seed {}. Model `{}`, reasoning effort `{}`, judge `{}`.\n",
        if run.phase == "pilot" {
            "Pilot results (small n; not the full run)"
        } else {
            "Full phase 0 results"
        },
        run.phase,
        run.seed,
        run.model,
        run.reasoning_effort,
        run.judge_model
    );

    md.push_str("## 1. Setup and reproducibility\n\n");
    md.push_str("| Item | Value |\n|---|---|\n");
    let _ = writeln!(md, "| Participant | full MonoAgens |");
    let _ = writeln!(
        md,
        "| Model / effort | `{}` / `{}` |",
        run.model, run.reasoning_effort
    );
    let _ = writeln!(md, "| Judge | `{}` |", run.judge_model);
    for (name, version) in &run.judge_prompt_versions {
        let _ = writeln!(md, "| Prompt version — {} | `{version}` |", name);
    }
    let _ = writeln!(
        md,
        "| Answer-format instruction | `{}` |",
        run.answer_format_version
    );
    let _ = writeln!(
        md,
        "| Development exposure | `{}` |",
        run.development_exposure
    );
    let _ = writeln!(
        md,
        "| Started / finished (UTC) | {} / {} |",
        run.started_at_utc, run.finished_at_utc
    );
    let _ = writeln!(md, "| Status | {} |", run.status);
    for ds in datasets {
        let _ = writeln!(
            md,
            "| {} items | {} |",
            ds.artifact.dataset_id, ds.artifact.n_items
        );
    }
    md.push('\n');
    md.push_str("Reproduction: `monoagens-lab benchmark --suite memory-comparison --phase 0 --live --max-cost-usd <cap> --out-dir <run-dir>`; answers come from the production render/parse path, history was loaded live, and every call was metered by the Lab proxy.\n\n");

    md.push_str("## 2. Results\n\n");
    for ds in datasets {
        if ds.artifact.dataset_id == "locomo" {
            md.push_str("### LoCoMo (official token F1 and `judge_acc`)\n\n");
            md.push_str(
                "| Category | n | F1 | `judge_acc` | 95% CI (judge) |\n|---|---:|---:|---:|---|\n",
            );
            for (name, n, f1, judge, ci) in locomo_rows(ds) {
                let _ = writeln!(
                    md,
                    "| {name} | {n} | {} | {} | {ci} |",
                    fmt_score(f1),
                    fmt_score(judge)
                );
            }
            let headline_n: usize = ds
                .items
                .iter()
                .filter(|i| {
                    i.category
                        .strip_prefix("cat_")
                        .and_then(|s| s.parse::<u8>().ok())
                        .is_some_and(|c| (1..=4).contains(&c))
                })
                .count();
            let _ = writeln!(
                md,
                "| **Headline (cat 1–4)** | {headline_n} | {} | {} | {} |",
                fmt_score(metric_value(&ds.record, "qa_f1")),
                fmt_score(metric_value(&ds.record, "judge_acc")),
                fmt_ci(&ds.record, "judge_acc")
            );
            let cat5_n = ds.items.iter().filter(|i| i.category == "cat_5").count();
            let _ = writeln!(
                md,
                "| **Cat 5 refusal** | {cat5_n} | {} | {} | {} |\n",
                fmt_score(metric_value(&ds.record, "cat5_refusal_acc")),
                fmt_score(metric_value(&ds.record, "judge_acc_cat_5")),
                fmt_ci(&ds.record, "judge_acc_cat_5")
            );
        } else {
            md.push_str("### LongMemEval-S (`judge_acc`)\n\n");
            md.push_str("| Question type | n | `judge_acc` | 95% CI |\n|---|---:|---:|---|\n");
            for (qtype, n, acc, ci) in lme_rows(ds) {
                let _ = writeln!(md, "| {qtype} | {n} | {} | {ci} |", fmt_score(acc));
            }
            let non_abs_n = ds.items.iter().filter(|i| !i.is_abs).count();
            let _ = writeln!(
                md,
                "| **Overall** | {non_abs_n} | {} | {} |",
                fmt_score(metric_value(&ds.record, "judge_acc")),
                fmt_ci(&ds.record, "judge_acc")
            );
            let _ = writeln!(
                md,
                "| **Task-averaged** | — | {} | — |",
                fmt_score(metric_value(&ds.record, "judge_acc_task_avg"))
            );
            let abs_n = ds.items.iter().filter(|i| i.is_abs).count();
            let _ = writeln!(
                md,
                "| **Abstention (`_abs`)** | {abs_n} | {} | {} |\n",
                fmt_score(metric_value(&ds.record, "abs_judge_acc")),
                fmt_ci(&ds.record, "abs_judge_acc")
            );
        }
    }

    md.push_str("## 3. Tokens, calls and cost\n\n");
    md.push_str("| Dataset | Phase | Calls | Uncached input | Cached read | Cached share | Output | Reasoning | List USD | Allowance USD |\n|---|---|---:|---:|---:|---:|---:|---:|---:|---:|\n");
    for ds in datasets {
        let prov = ds.record.dataset_provenance.as_ref();
        let Some(usage) = prov.and_then(|p| p.memory_usage.as_ref()) else {
            continue;
        };
        for (label, u) in [
            ("ingest", Some(&usage.ingest)),
            ("answer", Some(&usage.answer)),
            ("judge", Some(&usage.judge)),
        ] {
            let Some(u) = u else { continue };
            let _ = writeln!(
                md,
                "| {} | {label} | {} | {} | {} | {:.1}% | {} | {} | {:.4} | {:.4} |",
                ds.artifact.dataset_id,
                u.calls,
                u.uncached_input_tokens,
                u.cached_input_tokens,
                cached_share(u) * 100.0,
                u.output_tokens,
                u.reasoning_tokens,
                u.list_cost_usd,
                u.allowance_cost_usd
            );
        }
        let total_with =
            usage.ingest.list_cost_usd + usage.answer.list_cost_usd + usage.judge.list_cost_usd;
        let total_without = no_cache_cost(&usage.ingest)
            + no_cache_cost(&usage.answer)
            + no_cache_cost(&usage.judge);
        let _ = writeln!(
            md,
            "| {} | **total** | {} | {} | {} | {:.1}% | {} | {} | {:.4} (no cache {:.4}) | {:.4} |",
            ds.artifact.dataset_id,
            usage.ingest.calls + usage.answer.calls + usage.judge.calls,
            usage.ingest.uncached_input_tokens
                + usage.answer.uncached_input_tokens
                + usage.judge.uncached_input_tokens,
            usage.ingest.cached_input_tokens
                + usage.answer.cached_input_tokens
                + usage.judge.cached_input_tokens,
            if usage.ingest.uncached_input_tokens + usage.answer.uncached_input_tokens > 0 {
                (usage.ingest.cached_input_tokens + usage.answer.cached_input_tokens) as f64
                    / (usage.ingest.uncached_input_tokens
                        + usage.answer.uncached_input_tokens
                        + usage.ingest.cached_input_tokens
                        + usage.answer.cached_input_tokens) as f64
                    * 100.0
            } else {
                0.0
            },
            usage.ingest.output_tokens + usage.answer.output_tokens + usage.judge.output_tokens,
            usage.ingest.reasoning_tokens
                + usage.answer.reasoning_tokens
                + usage.judge.reasoning_tokens,
            total_with,
            total_without,
            usage.ingest.allowance_cost_usd
                + usage.answer.allowance_cost_usd
                + usage.judge.allowance_cost_usd
        );
    }
    md.push_str("\nTokens per question include ingest and answer calls (the judge excluded).\n\n");
    md.push_str("### Calls and cost by purpose\n\n");
    md.push_str("| Dataset | Purpose | Calls | Output tokens | List USD | Allowance USD | Runaway (`finish_reason=length`) |\n|---|---|---:|---:|---:|---:|---:|\n");
    for ds in datasets {
        let Some(usage) = ds
            .record
            .dataset_provenance
            .as_ref()
            .and_then(|p| p.memory_usage.as_ref())
        else {
            continue;
        };
        for (purpose, u) in &usage.by_purpose {
            let _ = writeln!(
                md,
                "| {} | {purpose} | {} | {} | {:.4} | {:.4} | {} |",
                ds.artifact.dataset_id,
                u.calls,
                u.output_tokens,
                u.list_cost_usd,
                u.allowance_cost_usd,
                usage.runaway_by_purpose.get(purpose).copied().unwrap_or(0)
            );
        }
    }
    md.push('\n');

    md.push_str("## 4. Latency and silence\n\n");
    md.push_str("| Dataset | Latency p50 (ms) | p95 (ms) | Silence rate | Calls |\n|---|---:|---:|---:|---:|\n");
    for ds in datasets {
        let lats = latencies(&ds.record);
        let calls = ds
            .record
            .dataset_provenance
            .as_ref()
            .and_then(|p| p.memory_usage.as_ref())
            .map(|u| u.ingest.calls + u.answer.calls + u.judge.calls)
            .unwrap_or(0);
        let _ = writeln!(
            md,
            "| {} | {} | {} | {:.1}% | {calls} |",
            ds.artifact.dataset_id,
            percentile(&lats, 50.0),
            percentile(&lats, 95.0),
            silence_rate(&ds.record) * 100.0
        );
    }
    md.push('\n');

    if let Some(pilot) = &run.pilot {
        md.push_str("## 5. Pilot gate\n\n");
        md.push_str("| Measure | Value |\n|---|---|\n");
        let _ = writeln!(
            md,
            "| Calls (ingest/answer/judge) | {} / {} / {} |",
            pilot.ingest_calls, pilot.answer_calls, pilot.judge_calls
        );
        let _ = writeln!(
            md,
            "| Mean / p95 input tokens per call | {:.0} / {:.0} |",
            pilot.mean_tokens_in_per_call, pilot.p95_tokens_in_per_call
        );
        let _ = writeln!(
            md,
            "| Mean / p95 output tokens per call | {:.0} / {:.0} |",
            pilot.mean_tokens_out_per_call, pilot.p95_tokens_out_per_call
        );
        let _ = writeln!(md, "| Cached share | {:.1}% |", pilot.cached_share * 100.0);
        let _ = writeln!(md, "| Silence rate | {:.1}% |", pilot.silence_rate * 100.0);
        let _ = writeln!(
            md,
            "| Errors / retries / rate limits | {} / {} / {} |",
            pilot.errors, pilot.retries, pilot.rate_limit_hits
        );
        let _ = writeln!(
            md,
            "| Latency p50 / p95 (ms) | {} / {} |",
            pilot.latency_p50_ms, pilot.latency_p95_ms
        );
        let _ = writeln!(md, "| Pilot list cost | ${:.4} |", pilot.list_cost_usd);
        let _ = writeln!(
            md,
            "| Projected rest of phase 0 | ${:.2}, {:.0} min |",
            pilot.projected_rest_list_cost_usd,
            pilot.projected_rest_wall_clock_s / 60.0
        );
        md.push('\n');
    }

    md.push_str("## 6. Failure analysis\n\n");
    for ds in datasets {
        let examples = failure_examples(ds, 5);
        if examples.is_empty() {
            continue;
        }
        let _ = writeln!(md, "### {}\n", ds.artifact.dataset_id);
        for item in examples {
            let _ = writeln!(
                md,
                "- **`{}` / `{}`** — Q: `{}` | Gold: `{}` | Answer: `{}` | Verdict: `{}` | [transcript]({})",
                item.item_id,
                item.probe_id,
                truncate(&item.question, 160),
                truncate(&item.gold_answer, 120),
                truncate(&item.answer, 160),
                item.judge_verdict
                    .as_deref()
                    .unwrap_or(if item.judge_failed {
                        "judge failure"
                    } else {
                        "—"
                    }),
                item.transcript_rel_path
            );
        }
        md.push('\n');
    }

    let defects = collect_defects(report_run_dir);
    md.push_str("## 7. Known product defects observed\n\n");
    md.push_str(
        "These are defects of MonoAgens as run, recorded rather than fixed in this change.\n\n",
    );
    let _ = writeln!(
        md,
        "- **Classify plans carry no rendered trigger/sections:** {} of {} classify calls rendered `[TRIGGER] trigger: none`, although `trigger_event_id` was set. Cause: `crates/core/src/turn.rs` (~4703/4814) builds `ContextPlan {{ trigger_event_id: Some(..), ..ContextPlan::default() }}` with empty `sections`; `crates/reason/src/render.rs` prints `trigger: none`. Classification verdicts are therefore made without the percept text. Example: `{}`",
        defects.classify_without_trigger,
        defects.classify_total,
        defects.classify_example.as_deref().unwrap_or("—")
    );
    let runaway_total: usize = defects.runaway_by_purpose.values().sum();
    let runaway_detail: Vec<String> = defects
        .runaway_by_purpose
        .iter()
        .map(|(p, n)| format!("{p}={n}"))
        .collect();
    let _ = writeln!(
        md,
        "- **No default output cap in production:** without `model.max_output_tokens` the model ran to 65,536 output tokens per call; this run used a cap of 4096. {} calls ended at the cap (finish_reason=length): {}. Example: {}",
        runaway_total,
        runaway_detail.join(", "),
        defects.runaway_example.as_deref().unwrap_or("—")
    );
    let schema_total: usize = defects.schema_failures.values().sum();
    let _ = writeln!(
        md,
        "- **Schema failures (MonoAgens + configured model behaviour, not papered over):** {} failed model outputs: {}. Example: {}",
        schema_total,
        defects
            .schema_failures
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join(", "),
        defects.schema_example.as_deref().unwrap_or("—")
    );
    md.push('\n');

    md.push_str("## 8. Caveats\n\n");
    md.push_str("- **Judge:** the judge is the configured judge model, not the published judge (gpt-4o-mini / gpt-4o); judge metrics are named for our judge and are not comparable with published numbers.\n");
    md.push_str("- **Contamination:** LoCoMo and LongMemEval-S predate this run and may be in model training data; no mitigation beyond recording `development_exposure` is claimed.\n");
    md.push_str("- **Small clusters:** LongMemEval-S has one history per question (500 clusters); LoCoMo has 10 conversations, so its cluster-bootstrap intervals are wide.\n");
    md.push_str("- **Pilot:** a pilot table reflects one LoCoMo conversation and 20 LongMemEval-S questions and is not a full-run score.\n");
    md.push_str("- No published scores from other systems appear in this report.\n");
    md
}

fn truncate(text: &str, max: usize) -> String {
    let mut out: String = text.chars().take(max).collect();
    if text.chars().count() > max {
        out.push('…');
    }
    out.replace('|', "\\|").replace('\n', " ")
}

fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn render_html(
    run: &MemoryRunFile,
    datasets: &[LoadedDataset],
    charts: &BTreeMap<String, String>,
) -> String {
    let mut html = String::new();
    html.push_str("<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><title>MonoAgens memory benchmark — phase 0</title><style>body{font-family:system-ui,sans-serif;max-width:1000px;margin:2rem auto;padding:0 1rem;color:#222}table{border-collapse:collapse;margin:1rem 0;width:100%}th,td{border:1px solid #ccc;padding:.3rem .5rem;text-align:right}th:first-child,td:first-child{text-align:left}code{background:#f3f3f3;padding:0 .2rem}.muted{color:#666}img,svg{max-width:100%}</style></head><body>");
    html.push_str("<h1>MonoAgens memory benchmark — phase 0</h1>");
    let _ = writeln!(
        html,
        "<p class=\"muted\">{} · model <code>{}</code> · effort <code>{}</code> · judge <code>{}</code> · seed {}</p>",
        escape_html(&run.phase),
        escape_html(&run.model),
        escape_html(&run.reasoning_effort),
        escape_html(&run.judge_model),
        run.seed
    );
    html.push_str("<h2>Setup</h2><table><tr><th>Item</th><th>Value</th></tr>");
    let _ = writeln!(
        html,
        "<tr><td>Participant</td><td>full MonoAgens</td></tr><tr><td>Development exposure</td><td>{}</td></tr><tr><td>Answer format</td><td><code>{}</code></td></tr><tr><td>Started / finished</td><td>{} / {}</td></tr><tr><td>Status</td><td>{}</td></tr>",
        escape_html(&run.development_exposure),
        escape_html(&run.answer_format_version),
        escape_html(&run.started_at_utc),
        escape_html(&run.finished_at_utc),
        escape_html(&run.status)
    );
    for (name, version) in &run.judge_prompt_versions {
        let _ = writeln!(
            html,
            "<tr><td>Prompt — {}</td><td><code>{}</code></td></tr>",
            escape_html(name),
            escape_html(version)
        );
    }
    html.push_str("</table>");

    html.push_str("<h2>Results</h2>");
    for ds in datasets {
        if ds.artifact.dataset_id == "locomo" {
            html.push_str("<h3>LoCoMo</h3><table><tr><th>Category</th><th>n</th><th>F1</th><th>judge_acc</th><th>95% CI</th></tr>");
            for (name, n, f1, judge, ci) in locomo_rows(ds) {
                let _ = writeln!(
                    html,
                    "<tr><td>{}</td><td>{n}</td><td>{}</td><td>{}</td><td>{ci}</td></tr>",
                    escape_html(&name),
                    fmt_score(f1),
                    fmt_score(judge)
                );
            }
            html.push_str("</table>");
        } else {
            html.push_str("<h3>LongMemEval-S</h3><table><tr><th>Question type</th><th>n</th><th>judge_acc</th><th>95% CI</th></tr>");
            for (qtype, n, acc, ci) in lme_rows(ds) {
                let _ = writeln!(
                    html,
                    "<tr><td>{}</td><td>{n}</td><td>{}</td><td>{ci}</td></tr>",
                    escape_html(&qtype),
                    fmt_score(acc)
                );
            }
            let _ = writeln!(
                html,
                "<tr><td><b>Overall</b></td><td>—</td><td>{}</td><td>{}</td></tr><tr><td><b>Task-averaged</b></td><td>—</td><td>{}</td><td>—</td></tr><tr><td><b>Abstention</b></td><td>—</td><td>{}</td><td>{}</td></tr>",
                fmt_score(metric_value(&ds.record, "judge_acc")),
                fmt_ci(&ds.record, "judge_acc"),
                fmt_score(metric_value(&ds.record, "judge_acc_task_avg")),
                fmt_score(metric_value(&ds.record, "abs_judge_acc")),
                fmt_ci(&ds.record, "abs_judge_acc")
            );
            html.push_str("</table>");
        }
    }

    html.push_str("<h2>Tokens and cost</h2><table><tr><th>Dataset</th><th>Phase</th><th>Calls</th><th>Uncached</th><th>Cached</th><th>Output</th><th>Reasoning</th><th>List USD</th></tr>");
    for ds in datasets {
        let Some(usage) = ds
            .record
            .dataset_provenance
            .as_ref()
            .and_then(|p| p.memory_usage.as_ref())
        else {
            continue;
        };
        for (label, u) in [
            ("ingest", &usage.ingest),
            ("answer", &usage.answer),
            ("judge", &usage.judge),
        ] {
            let _ = writeln!(
                html,
                "<tr><td>{}</td><td>{label}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{:.4}</td></tr>",
                escape_html(&ds.artifact.dataset_id),
                u.calls,
                u.uncached_input_tokens,
                u.cached_input_tokens,
                u.output_tokens,
                u.reasoning_tokens,
                u.list_cost_usd
            );
        }
    }
    html.push_str("</table>");

    html.push_str("<h2>Latency and silence</h2><table><tr><th>Dataset</th><th>p50 ms</th><th>p95 ms</th><th>Silence rate</th></tr>");
    for ds in datasets {
        let lats = latencies(&ds.record);
        let _ = writeln!(
            html,
            "<tr><td>{}</td><td>{}</td><td>{}</td><td>{:.1}%</td></tr>",
            escape_html(&ds.artifact.dataset_id),
            percentile(&lats, 50.0),
            percentile(&lats, 95.0),
            silence_rate(&ds.record) * 100.0
        );
    }
    html.push_str("</table>");

    for (name, svg) in charts {
        let _ = writeln!(html, "<h2>{}</h2>{}", escape_html(name), svg);
    }

    html.push_str("<h2>Known product defects observed</h2><ul><li>Classify plans carry no rendered trigger/sections (core turn.rs ~4703/4814; reason render_trigger): verdicts are made without the percept text.</li><li>Production has no default output cap; this run used 4096 and counts calls ending at the cap as runaway.</li><li>Schema failures are counted per type in REPORT.md rather than papered over.</li></ul><h2>Caveats</h2><ul><li>The judge is the configured judge model, not the published judge; its metrics are named <code>judge_acc</code>.</li><li>Public benchmarks may be in model training data; <code>development_exposure</code> is recorded.</li><li>LoCoMo has 10 clusters, so its intervals are wide; a pilot is not a full-run score.</li><li>No published scores of other systems appear here.</li></ul>");
    html.push_str("</body></html>");
    html
}

/// Generates the memory-only report for a run directory.
pub fn generate_memory_report(run_dir: &Path) -> Result<GeneratedMemoryReport, LabError> {
    let (run, datasets) = load(run_dir)?;
    let report_dir = run_dir.join("report");
    let tables_dir = report_dir.join("tables");
    let charts_dir = report_dir.join("charts");
    fs::create_dir_all(&tables_dir)?;
    fs::create_dir_all(&charts_dir)?;

    // CSV tables.
    let mut locomo_rows_csv = Vec::new();
    let mut lme_rows_csv = Vec::new();
    let mut token_rows = Vec::new();
    let mut latency_rows = Vec::new();
    let mut purpose_rows = Vec::new();
    for ds in &datasets {
        if ds.artifact.dataset_id == "locomo" {
            for (name, n, f1, judge, ci) in locomo_rows(ds) {
                locomo_rows_csv.push(vec![
                    name,
                    n.to_string(),
                    fmt_score(f1),
                    fmt_score(judge),
                    ci,
                ]);
            }
        } else {
            for (qtype, n, acc, ci) in lme_rows(ds) {
                lme_rows_csv.push(vec![qtype, n.to_string(), fmt_score(acc), ci]);
            }
        }
        let usage_opt = ds
            .record
            .dataset_provenance
            .as_ref()
            .and_then(|p| p.memory_usage.as_ref());
        if let Some(usage) = usage_opt {
            for (phase, u) in [
                ("ingest", &usage.ingest),
                ("answer", &usage.answer),
                ("judge", &usage.judge),
            ] {
                token_rows.push(vec![
                    ds.artifact.dataset_id.clone(),
                    phase.to_owned(),
                    u.calls.to_string(),
                    u.uncached_input_tokens.to_string(),
                    u.cached_input_tokens.to_string(),
                    u.output_tokens.to_string(),
                    u.reasoning_tokens.to_string(),
                    format!("{:.6}", u.list_cost_usd),
                    format!("{:.6}", u.allowance_cost_usd),
                ]);
            }
        }
        if let Some(usage) = usage_opt {
            for (purpose, u) in &usage.by_purpose {
                purpose_rows.push(vec![
                    ds.artifact.dataset_id.clone(),
                    purpose.clone(),
                    u.calls.to_string(),
                    u.output_tokens.to_string(),
                    format!("{:.6}", u.list_cost_usd),
                    format!("{:.6}", u.allowance_cost_usd),
                    usage
                        .runaway_by_purpose
                        .get(purpose)
                        .copied()
                        .unwrap_or(0)
                        .to_string(),
                ]);
            }
        }
        let lats = latencies(&ds.record);
        latency_rows.push(vec![
            ds.artifact.dataset_id.clone(),
            percentile(&lats, 50.0).to_string(),
            percentile(&lats, 95.0).to_string(),
            format!("{:.4}", silence_rate(&ds.record)),
            lats.len().to_string(),
        ]);
    }
    write_csv(
        &tables_dir.join("locomo_categories.csv"),
        &["category", "n", "qa_f1", "judge_acc", "ci95"],
        &locomo_rows_csv,
    )?;
    write_csv(
        &tables_dir.join("longmemeval_types.csv"),
        &["question_type", "n", "judge_acc", "ci95"],
        &lme_rows_csv,
    )?;
    write_csv(
        &tables_dir.join("tokens.csv"),
        &[
            "dataset",
            "phase",
            "calls",
            "uncached_input",
            "cached_input",
            "output",
            "reasoning",
            "list_usd",
            "allowance_usd",
        ],
        &token_rows,
    )?;
    write_csv(
        &tables_dir.join("tokens_by_purpose.csv"),
        &[
            "dataset",
            "purpose",
            "calls",
            "output_tokens",
            "list_usd",
            "allowance_usd",
            "runaway",
        ],
        &purpose_rows,
    )?;
    write_csv(
        &tables_dir.join("latency.csv"),
        &["dataset", "p50_ms", "p95_ms", "silence_rate", "n_calls"],
        &latency_rows,
    )?;

    // Charts.
    let mut charts: BTreeMap<String, String> = BTreeMap::new();
    let locomo = datasets.iter().find(|d| d.artifact.dataset_id == "locomo");
    if let Some(ds) = locomo {
        let groups: Vec<Group> = locomo_rows(ds)
            .into_iter()
            .map(|(name, _n, f1, judge, _ci)| {
                (
                    name,
                    vec![
                        ("F1".to_owned(), f1.unwrap_or(0.0), None, "#4e79a7"),
                        ("judge".to_owned(), judge.unwrap_or(0.0), None, "#e15759"),
                    ],
                )
            })
            .collect();
        charts.insert(
            "LoCoMo per-category scores".to_owned(),
            svg_bar_chart("LoCoMo per-category scores", &groups, 900, 320),
        );
    }
    if let Some(ds) = datasets
        .iter()
        .find(|d| d.artifact.dataset_id == "longmemeval")
    {
        let groups: Vec<Group> = lme_rows(ds)
            .into_iter()
            .map(|(qtype, _n, acc, _ci)| {
                (
                    short_type(&qtype),
                    vec![("judge".to_owned(), acc.unwrap_or(0.0), None, "#59a14f")],
                )
            })
            .collect();
        charts.insert(
            "LongMemEval-S per-question-type accuracy".to_owned(),
            svg_bar_chart(
                "LongMemEval-S per-question-type accuracy",
                &groups,
                900,
                320,
            ),
        );
    }
    let mut token_bars = Vec::new();
    for ds in &datasets {
        let Some(usage) = ds
            .record
            .dataset_provenance
            .as_ref()
            .and_then(|p| p.memory_usage.as_ref())
        else {
            continue;
        };
        let n = ds.artifact.n_items.max(1) as u64;
        let ingest_q = (usage.ingest.uncached_input_tokens
            + usage.ingest.cached_input_tokens
            + usage.ingest.output_tokens)
            / n;
        let answer_q = (usage.answer.uncached_input_tokens
            + usage.answer.cached_input_tokens
            + usage.answer.output_tokens)
            / n;
        token_bars.push((
            ds.artifact.dataset_id.clone(),
            ingest_q,
            answer_q,
            usage.ingest.calls,
            usage.answer.calls,
        ));
    }
    charts.insert(
        "Tokens per question".to_owned(),
        svg_stacked_tokens(&token_bars, 900, 220),
    );
    let mut points = Vec::new();
    for ds in &datasets {
        let n = ds.artifact.n_items.max(1) as f64;
        let Some(usage) = ds
            .record
            .dataset_provenance
            .as_ref()
            .and_then(|p| p.memory_usage.as_ref())
        else {
            continue;
        };
        let tokens = (usage.ingest.uncached_input_tokens
            + usage.ingest.cached_input_tokens
            + usage.ingest.output_tokens
            + usage.answer.uncached_input_tokens
            + usage.answer.cached_input_tokens
            + usage.answer.output_tokens) as f64
            / n;
        let score = metric_value(&ds.record, "judge_acc").unwrap_or(0.0);
        points.push((ds.artifact.dataset_id.clone(), score, tokens));
    }
    charts.insert(
        "Quality vs tokens".to_owned(),
        svg_quality_vs_tokens(&points, 900, 300),
    );
    let all_latencies: Vec<u64> = datasets.iter().flat_map(|d| latencies(&d.record)).collect();
    charts.insert(
        "Latency distribution".to_owned(),
        svg_latency(&all_latencies, 900, 260),
    );
    for (name, svg) in &charts {
        let file = format!("{}.svg", safe_file(name));
        fs::write(charts_dir.join(&file), svg)?;
    }

    let markdown = render_markdown(&run, &datasets, run_dir);
    let markdown_path = report_dir.join("REPORT.md");
    fs::write(&markdown_path, &markdown)?;
    let html = render_html(&run, &datasets, &charts);
    let html_path = report_dir.join("index.html");
    fs::write(&html_path, html)?;

    Ok(GeneratedMemoryReport {
        markdown_path,
        html_path,
    })
}

fn short_type(qtype: &str) -> String {
    qtype
        .replace("single-session-", "ss-")
        .replace("multi-session", "multi")
        .replace("temporal-reasoning", "temporal")
        .replace("knowledge-update", "k-update")
}

fn safe_file(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}
