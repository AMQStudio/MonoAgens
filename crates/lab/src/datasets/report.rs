//! Standard benchmark report generator (`REPORT.md`, self-contained `index.html`,
//! `tables/*.csv`, `charts/*.png|svg`, and failure-analysis `<item_id>.md` transcripts).
//!
//! Every score, confidence interval, token count, latency percentile, wake rate, and failure
//! example is derived strictly from persisted [`RunRecord`] and `*.outcomes.json` files.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::{Deserialize, Serialize};

use crate::LabError;
use crate::benchmark::{BenchmarkDatasetSpec, BenchmarkDomain, standard_benchmark_registry};
use crate::datasets::manifest::{
    CoreApplicability, DatasetAvailability, DatasetLane, DatasetsManifest,
    load_and_validate_manifest, resolve_template_manifest_path,
};
use crate::datasets::runner::{ItemOutcomeSummary, PreflightEstimate};
use crate::pareto::{
    ScorecardRow, ScorecardStatus, compute_convex_pareto, scorecard_rows_to_pareto_input,
};
use crate::record::{MONOAGENS_MODE_OFFLINE, RunRecord};
use crate::scenario::ensure_not_sealed_path;

/// Configuration for [`generate_benchmark_report`].
#[derive(Debug, Clone)]
pub struct ReportOptions {
    pub run_dir: PathBuf,
    pub out_dir: Option<PathBuf>,
    pub manifest_path: Option<PathBuf>,
    pub extra_notes: Vec<String>,
    pub skip_python_charts: bool,
    /// Category suite name; when set, the report covers only that category's datasets.
    pub suite: Option<String>,
    /// Optional banner repeated on every table (e.g. a partial-run label).
    pub label: Option<String>,
    /// Optional Markdown appended to `REPORT.md` and shown in `index.html`.
    pub appendix_markdown: Option<String>,
}

/// Summary returned after generating a standard benchmark report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GeneratedReportSummary {
    pub report_dir: String,
    pub markdown_path: String,
    pub html_path: String,
    pub csv_files: Vec<String>,
    pub chart_files: Vec<String>,
    pub failure_markdown_files: Vec<String>,
    pub run_records_loaded: usize,
}

#[derive(Debug, Clone)]
struct LoadedDatasetRun {
    record: RunRecord,
    source_dir: PathBuf,
    outcomes: Vec<ItemOutcomeSummary>,
}

/// Generates `REPORT.md`, `index.html`, `tables/*.csv`, `charts/*`, and failure `transcripts/`
/// inside `opts.out_dir` (defaulting to `<run_dir>/report`) strictly from persisted run records.
pub fn generate_benchmark_report(opts: &ReportOptions) -> Result<GeneratedReportSummary, LabError> {
    ensure_not_sealed_path(&opts.run_dir)?;
    if !opts.run_dir.is_dir() {
        return Err(LabError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("run directory '{}' does not exist", opts.run_dir.display()),
        )));
    }

    let report_dir = opts
        .out_dir
        .clone()
        .unwrap_or_else(|| opts.run_dir.join("report"));
    ensure_not_sealed_path(&report_dir)?;
    let tables_dir = report_dir.join("tables");
    let charts_dir = report_dir.join("charts");
    let tx_out_dir = report_dir.join("transcripts");
    fs::create_dir_all(&tables_dir)?;
    fs::create_dir_all(&charts_dir)?;
    fs::create_dir_all(&tx_out_dir)?;

    let full_registry = standard_benchmark_registry();
    let registry: Vec<BenchmarkDatasetSpec> = match opts
        .suite
        .as_deref()
        .and_then(crate::datasets::suite_domain)
    {
        Some(domain) => full_registry
            .iter()
            .filter(|s| s.domain == domain)
            .cloned()
            .collect(),
        None => full_registry.clone(),
    };
    let manifest = resolve_manifest_for_report(opts, &full_registry)?;
    let loaded_runs = collect_run_records_recursive(&opts.run_dir)?;
    let preflight = load_preflight_if_present(&opts.run_dir);

    let mut by_ds: BTreeMap<String, LoadedDatasetRun> = BTreeMap::new();
    let mut invalid_ids: BTreeSet<String> = BTreeSet::new();
    for run in loaded_runs {
        let rec = &run.record;
        let offline = rec.monoagens_mode.as_deref() == Some(MONOAGENS_MODE_OFFLINE);
        if !rec.is_full_monoagens() && !offline {
            invalid_ids.insert(rec.scenario_id.clone());
        } else {
            by_ds.insert(rec.scenario_id.clone(), run);
        }
    }
    // Any record that is neither a full MonoAgens live run nor an explicit offline run is
    // excluded from the MonoAgens results and flagged in every table (`Design 8`).
    let invalid_banner =
        (!invalid_ids.is_empty()).then(|| "invalid: not full MonoAgens".to_owned());
    let banner_label = match (opts.label.as_deref(), invalid_banner.as_deref()) {
        (Some(l), Some(b)) => Some(format!("{l}; {b}")),
        (Some(l), None) => Some(l.to_owned()),
        (None, Some(b)) => Some(b.to_owned()),
        (None, None) => None,
    };

    // Copy any per-item JSONL transcripts from subdirs into report/transcripts/<dataset>/ so all
    // report links resolve locally under report/.
    for (ds_id, run) in &by_ds {
        let safe_ds = sanitize_id(ds_id);
        let src_tx_dir = run.source_dir.join("transcripts").join(&safe_ds);
        if src_tx_dir.is_dir() {
            let dst_tx_dir = tx_out_dir.join(&safe_ds);
            fs::create_dir_all(&dst_tx_dir)?;
            for entry in fs::read_dir(&src_tx_dir)? {
                let path = entry?.path();
                if path.is_file()
                    && let Some(name) = path.file_name()
                {
                    let dst = dst_tx_dir.join(name);
                    if path != dst {
                        let _ = fs::copy(&path, &dst);
                    }
                }
            }
        }
    }

    // Build canonical 23-dataset + 1 tool_rag rows directly from manifest + loaded RunRecords.
    let mut scorecard_rows: Vec<ScorecardRow> = Vec::new();
    let mut status_rows: Vec<StatusRow> = Vec::new();
    let mut cost_rows: Vec<CostRow> = Vec::new();
    let mut category_chart_rows: Vec<serde_json::Value> = Vec::new();
    let mut domain_chart_rows: Vec<serde_json::Value> = Vec::new();

    for spec in &registry {
        if !spec.requires_external_corpus {
            status_rows.push(StatusRow {
                dataset_id: spec.id.clone(),
                domain: domain_slug(spec.domain).to_owned(),
                lane: "in_tree".to_owned(),
                status: "in_tree_only".to_owned(),
                reason: spec.caveats.clone(),
                contamination_risk: "none".to_owned(),
            });
            continue;
        }
        let man_entry = manifest.as_ref().and_then(|m| m.get(&spec.id));
        let lane = man_entry.map_or(DatasetLane::CoreBehavioral, |e| e.lane);
        let core_app =
            man_entry.map_or(CoreApplicability::CoreBehavioral, |e| e.core_applicability);
        let contam = man_entry
            .map(|e| e.contamination_risk)
            .unwrap_or(crate::datasets::manifest::ContaminationRisk::Moderate);
        let avail = man_entry.map_or(DatasetAvailability::Available, |e| e.availability);

        if avail == DatasetAvailability::Unavailable {
            let reason = man_entry
                .map(|e| e.reason.clone())
                .unwrap_or_else(|| "unavailable".to_owned());
            scorecard_rows.push(ScorecardRow {
                dataset_id: spec.id.clone(),
                domain: spec.domain,
                lane,
                status: ScorecardStatus::Unavailable,
                core_applicability: core_app,
                contamination_risk: contam,
                metrics: spec
                    .primary_metrics
                    .iter()
                    .map(|m| (m.clone(), None))
                    .collect(),
                metric_intervals: Vec::new(),
                cost_usd: 0.0,
                e_tok: 0.0,
                reason: Some(reason.clone()),
            });
            status_rows.push(StatusRow {
                dataset_id: spec.id.clone(),
                domain: domain_slug(spec.domain).to_owned(),
                lane: lane.to_string(),
                status: "unavailable".to_owned(),
                reason,
                contamination_risk: contam.to_string(),
            });
            continue;
        }

        if invalid_ids.contains(&spec.id) {
            let reason = "invalid: not full MonoAgens".to_owned();
            scorecard_rows.push(ScorecardRow {
                dataset_id: spec.id.clone(),
                domain: spec.domain,
                lane,
                status: ScorecardStatus::NotScored,
                core_applicability: core_app,
                contamination_risk: contam,
                metrics: spec
                    .primary_metrics
                    .iter()
                    .map(|m| (m.clone(), None))
                    .collect(),
                metric_intervals: Vec::new(),
                cost_usd: 0.0,
                e_tok: 0.0,
                reason: Some(reason.clone()),
            });
            status_rows.push(StatusRow {
                dataset_id: spec.id.clone(),
                domain: domain_slug(spec.domain).to_owned(),
                lane: lane.to_string(),
                status: "not_scored".to_owned(),
                reason,
                contamination_risk: contam.to_string(),
            });
            continue;
        }

        if let Some(loaded) = by_ds.get(&spec.id) {
            let rec = &loaded.record;
            let prov = rec.dataset_provenance.as_ref();
            let intervals = prov.map(|p| p.metric_scores.clone()).unwrap_or_default();
            let mut metric_map: BTreeMap<String, Option<f64>> = BTreeMap::new();
            for iv in &intervals {
                metric_map.insert(iv.metric.clone(), iv.value);
            }
            // Model-judged metrics stay under their recorded `<metric>_non_official_judge`
            // names; never re-label an unofficial judge score with the official metric name.
            let any_scored = intervals.iter().any(|iv| iv.value.is_some());
            let any_unscored = intervals.iter().any(|iv| iv.value.is_none());
            let partial_flag = prov.is_some_and(|p| p.partial_run);
            let status = if !any_scored && any_unscored {
                ScorecardStatus::NotScored
            } else if partial_flag || any_unscored {
                ScorecardStatus::Partial
            } else {
                ScorecardStatus::Run
            };

            let (tok_in, tok_out, model_calls, judge_calls, cost_usd) = summarize_costs(rec);
            let e_tok = (tok_in + tok_out) as f64;
            let n_items = prov.map_or(rec.metrics.total_events, |p| p.chosen_item_ids.len());
            let n_clusters = intervals
                .iter()
                .filter_map(|i| i.n_clusters)
                .max()
                .unwrap_or(n_items);

            let total_dec = rec.metrics.decisions_go
                + rec.metrics.decisions_note
                + rec.metrics.decisions_silent
                + rec.metrics.decisions_door_blocked;
            let silence_rate = if total_dec > 0 {
                (rec.metrics.decisions_silent + rec.metrics.decisions_note) as f64
                    / total_dec as f64
            } else {
                0.0
            };
            let latencies = prov.map(|p| p.model_latencies_ms.as_slice()).unwrap_or(&[]);
            let (p50_ms, p95_ms) = latency_percentiles(latencies);
            let failed_cnt = prov.map_or(0, |p| p.failed_items.len());

            let reason = if status == ScorecardStatus::NotScored {
                intervals
                    .iter()
                    .find_map(|i| i.not_scored_reason.clone())
                    .or_else(|| Some("not_scored".to_owned()))
            } else if failed_cnt > 0 {
                Some(format!("{failed_cnt} item failure(s) recorded"))
            } else {
                None
            };

            scorecard_rows.push(ScorecardRow {
                dataset_id: spec.id.clone(),
                domain: spec.domain,
                lane,
                status,
                core_applicability: core_app,
                contamination_risk: contam,
                metrics: metric_map,
                metric_intervals: intervals.clone(),
                cost_usd,
                e_tok,
                reason: reason.clone(),
            });
            status_rows.push(StatusRow {
                dataset_id: spec.id.clone(),
                domain: domain_slug(spec.domain).to_owned(),
                lane: lane.to_string(),
                status: status_slug(status).to_owned(),
                reason: reason.unwrap_or_else(|| "ok".to_owned()),
                contamination_risk: contam.to_string(),
            });
            cost_rows.push(CostRow {
                dataset_id: spec.id.clone(),
                domain: domain_slug(spec.domain).to_owned(),
                n_items,
                n_clusters,
                percepts: rec.metrics.percepts,
                tokens_in: tok_in,
                tokens_out: tok_out,
                model_calls,
                judge_calls,
                wakes_per_event: rec.metrics.wakes_per_event,
                silence_rate,
                latency_p50_ms: p50_ms,
                latency_p95_ms: p95_ms,
                wall_elapsed_ms: rec.metrics.wall_elapsed_ms,
                cost_usd,
                failed_items: failed_cnt,
            });

            domain_chart_rows.push(serde_json::json!({
                "dataset_id": spec.id,
                "domain": domain_slug(spec.domain),
                "metrics": intervals.iter().map(|iv| serde_json::json!({
                    "metric": iv.metric,
                    "value": iv.value,
                    "ci95_low": iv.ci95_low,
                    "ci95_high": iv.ci95_high,
                })).collect::<Vec<_>>(),
            }));

            if let Some(p) = prov {
                for (cat, mmap) in &p.category_metrics {
                    for (mname, mval) in mmap {
                        category_chart_rows.push(serde_json::json!({
                            "dataset_id": spec.id,
                            "category": format!("{cat} ({mname})"),
                            "value": mval,
                        }));
                    }
                }
            }
        } else {
            let reason = if core_app == CoreApplicability::NotMappable
                && lane == DatasetLane::CoreBehavioral
            {
                man_entry
                    .map(|e| e.reason.clone())
                    .unwrap_or_else(|| "not_mappable".to_owned())
            } else {
                "not_run (no run record in run directory)".to_owned()
            };
            let status = if core_app == CoreApplicability::NotMappable
                && lane == DatasetLane::CoreBehavioral
            {
                ScorecardStatus::NotMappable
            } else {
                ScorecardStatus::NotScored
            };
            scorecard_rows.push(ScorecardRow {
                dataset_id: spec.id.clone(),
                domain: spec.domain,
                lane,
                status,
                core_applicability: core_app,
                contamination_risk: contam,
                metrics: spec
                    .primary_metrics
                    .iter()
                    .map(|m| (m.clone(), None))
                    .collect(),
                metric_intervals: Vec::new(),
                cost_usd: 0.0,
                e_tok: 0.0,
                reason: Some(reason.clone()),
            });
            status_rows.push(StatusRow {
                dataset_id: spec.id.clone(),
                domain: domain_slug(spec.domain).to_owned(),
                lane: lane.to_string(),
                status: status_slug(status).to_owned(),
                reason,
                contamination_risk: contam.to_string(),
            });
        }
    }

    let pareto_input = scorecard_rows_to_pareto_input(&scorecard_rows, Vec::new());
    let pareto = compute_convex_pareto(&pareto_input);

    // 1. Write CSV tables (`overall.csv`, `status.csv`, `cost.csv`, plus one per domain).
    let csv_files = write_csv_tables(
        &tables_dir,
        &scorecard_rows,
        &status_rows,
        &cost_rows,
        &by_ds,
    )?;
    if let Some(l) = banner_label.as_deref() {
        prepend_label_column(&report_dir, &csv_files, l)?;
    }

    // 2. Write failure analysis `<item_id>.md` files (up to 5 wrong/failed items per dataset).
    let (failure_sections, failure_md_files) = build_failure_analysis(&by_ds, &tx_out_dir)?;

    // 3. Render charts via Python matplotlib (`lab/adapters`), or pure-Rust SVG fallback.
    let pareto_points: Vec<serde_json::Value> = scorecard_rows
        .iter()
        .filter_map(|r| {
            let score = r.metrics.values().flatten().copied().next()?;
            let on_frontier = pareto
                .undominated_frontier
                .iter()
                .any(|f| f.name == r.dataset_id);
            Some(serde_json::json!({
                "dataset_id": r.dataset_id,
                "e_tok": r.e_tok,
                "score": score,
                "on_frontier": on_frontier,
            }))
        })
        .collect();
    let chart_files = generate_charts(
        &charts_dir,
        &domain_chart_rows,
        &category_chart_rows,
        &cost_rows,
        &pareto_points,
        opts.skip_python_charts,
    )?;

    // 4. Build Reproducibility metadata strictly from loaded RunRecords + git/manifest.
    let repro = build_repro_info(
        &opts.run_dir,
        manifest.as_ref(),
        &by_ds,
        preflight.as_ref(),
        &opts.extra_notes,
    );

    // 5. Render REPORT.md and self-contained index.html.
    let md = render_markdown_report(
        &repro,
        &registry,
        &scorecard_rows,
        &status_rows,
        &cost_rows,
        &by_ds,
        &failure_sections,
        &chart_files,
        &pareto,
        banner_label.as_deref(),
        opts.appendix_markdown.as_deref(),
    );
    let md_path = report_dir.join("REPORT.md");
    fs::write(&md_path, &md)?;

    let html = render_html_report(
        &repro,
        &registry,
        &scorecard_rows,
        &status_rows,
        &cost_rows,
        &by_ds,
        &failure_sections,
        &charts_dir,
        &chart_files,
        &pareto,
        banner_label.as_deref(),
        opts.appendix_markdown.as_deref(),
    )?;
    let html_path = report_dir.join("index.html");
    fs::write(&html_path, &html)?;

    Ok(GeneratedReportSummary {
        report_dir: report_dir.to_string_lossy().into_owned(),
        markdown_path: md_path.to_string_lossy().into_owned(),
        html_path: html_path.to_string_lossy().into_owned(),
        csv_files,
        chart_files,
        failure_markdown_files: failure_md_files,
        run_records_loaded: by_ds.len(),
    })
}

fn resolve_manifest_for_report(
    opts: &ReportOptions,
    registry: &[BenchmarkDatasetSpec],
) -> Result<Option<DatasetsManifest>, LabError> {
    if let Some(p) = &opts.manifest_path {
        return load_and_validate_manifest(p, registry).map(Some);
    }
    for candidate in [
        opts.run_dir.join("manifest.after.toml"),
        opts.run_dir.join("manifest.before.toml"),
        PathBuf::from(crate::datasets::DEFAULT_LOCAL_MANIFEST_PATH),
        resolve_template_manifest_path(None),
    ] {
        if candidate.is_file()
            && let Ok(m) = load_and_validate_manifest(&candidate, registry)
        {
            return Ok(Some(m));
        }
    }
    Ok(None)
}

fn collect_run_records_recursive(root: &Path) -> Result<Vec<LoadedDatasetRun>, LabError> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir)? {
            let path = entry?.path();
            if path.is_dir() {
                let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
                if name != "report" && name != "transcripts" && name != "pilot" {
                    stack.push(path);
                }
                continue;
            }
            let Some(fname) = path.file_name().and_then(|s| s.to_str()) else {
                continue;
            };
            if fname.ends_with(".json")
                && fname.contains("__seed-")
                && !fname.ends_with(".outcomes.json")
            {
                let raw = fs::read_to_string(&path)?;
                if let Ok(rec) = serde_json::from_str::<RunRecord>(&raw) {
                    let outcomes_path =
                        path.with_file_name(fname.replace(".json", ".outcomes.json"));
                    let outcomes = if outcomes_path.is_file() {
                        fs::read_to_string(&outcomes_path)
                            .ok()
                            .and_then(|s| serde_json::from_str::<Vec<ItemOutcomeSummary>>(&s).ok())
                            .unwrap_or_default()
                    } else {
                        Vec::new()
                    };
                    out.push(LoadedDatasetRun {
                        record: rec,
                        source_dir: dir.clone(),
                        outcomes,
                    });
                }
            }
        }
    }
    out.sort_by(|a, b| a.record.scenario_id.cmp(&b.record.scenario_id));
    Ok(out)
}

fn load_preflight_if_present(run_dir: &Path) -> Option<PreflightEstimate> {
    let p = run_dir.join("preflight.json");
    if !p.is_file() {
        return None;
    }
    let raw = fs::read_to_string(p).ok()?;
    serde_json::from_str(&raw).ok()
}

fn summarize_costs(rec: &RunRecord) -> (u64, u64, u64, u64, f64) {
    let mut tok_in = 0_u64;
    let mut tok_out = 0_u64;
    let mut model_calls = 0_u64;
    let judge_calls = rec.dataset_provenance.as_ref().map_or(0, |p| p.judge_calls);
    let mut cost_usd = 0.0_f64;
    for pc in &rec.cost_by_purpose {
        tok_in = tok_in.saturating_add(pc.tokens_in);
        tok_out = tok_out.saturating_add(pc.tokens_out);
        cost_usd += pc.cost_usd;
        if pc.purpose != monoagens_core::ModelPurpose::Classify {
            model_calls += pc.calls;
        }
    }
    (tok_in, tok_out, model_calls, judge_calls, cost_usd)
}

fn latency_percentiles(latencies: &[u64]) -> (u64, u64) {
    if latencies.is_empty() {
        return (0, 0);
    }
    let mut sorted = latencies.to_vec();
    sorted.sort_unstable();
    let p50 = sorted[(sorted.len() - 1) / 2];
    let p95_idx = ((0.95 * (sorted.len() as f64 - 1.0)).ceil() as usize).min(sorted.len() - 1);
    (p50, sorted[p95_idx])
}

#[derive(Debug, Clone, Serialize)]
struct StatusRow {
    dataset_id: String,
    domain: String,
    lane: String,
    status: String,
    reason: String,
    contamination_risk: String,
}

#[derive(Debug, Clone, Serialize)]
struct CostRow {
    dataset_id: String,
    domain: String,
    n_items: usize,
    n_clusters: usize,
    percepts: usize,
    tokens_in: u64,
    tokens_out: u64,
    model_calls: u64,
    judge_calls: u64,
    wakes_per_event: f64,
    silence_rate: f64,
    latency_p50_ms: u64,
    latency_p95_ms: u64,
    wall_elapsed_ms: u64,
    cost_usd: f64,
    failed_items: usize,
}

/// Prepends a `label` column to every generated CSV so partial/pilot rows are self-describing.
fn prepend_label_column(report_dir: &Path, files: &[String], label: &str) -> Result<(), LabError> {
    for rel in files {
        let path = report_dir.join(rel);
        let mut rdr = csv::ReaderBuilder::new()
            .from_path(&path)
            .map_err(|e| LabError::Serialization(format!("csv read '{}': {e}", path.display())))?;
        let mut records: Vec<csv::StringRecord> = Vec::new();
        for rec in rdr.records() {
            records.push(rec.map_err(|e| {
                LabError::Serialization(format!("csv row '{}': {e}", path.display()))
            })?);
        }
        let headers = rdr
            .headers()
            .map_err(|e| LabError::Serialization(format!("csv headers '{}': {e}", path.display())))?
            .clone();
        let tmp = path.with_extension("csv.tmp");
        let mut wtr = csv::Writer::from_path(&tmp)
            .map_err(|e| LabError::Serialization(format!("csv write '{}': {e}", tmp.display())))?;
        let mut head = csv::StringRecord::new();
        head.push_field("label");
        for h in headers.iter() {
            head.push_field(h);
        }
        wtr.write_record(&head)
            .map_err(|e| LabError::Serialization(e.to_string()))?;
        for rec in records {
            let mut out = csv::StringRecord::new();
            out.push_field(label);
            for f in rec.iter() {
                out.push_field(f);
            }
            wtr.write_record(&out)
                .map_err(|e| LabError::Serialization(e.to_string()))?;
        }
        wtr.flush()
            .map_err(|e| LabError::Serialization(e.to_string()))?;
        fs::rename(&tmp, &path)?;
    }
    Ok(())
}

fn write_csv_tables(
    tables_dir: &Path,
    scorecard_rows: &[ScorecardRow],
    status_rows: &[StatusRow],
    cost_rows: &[CostRow],
    by_ds: &BTreeMap<String, LoadedDatasetRun>,
) -> Result<Vec<String>, LabError> {
    let mut created = Vec::new();

    // 1. overall.csv
    let mut overall = String::from(
        "dataset_id,domain,lane,status,metric,value,ci95_low,ci95_high,n_items,n_clusters,contamination_risk\n",
    );
    for row in scorecard_rows {
        let loaded = by_ds.get(&row.dataset_id);
        let n_items = loaded
            .and_then(|l| l.record.dataset_provenance.as_ref())
            .map_or(0, |p| p.chosen_item_ids.len());
        if row.metric_intervals.is_empty() {
            for (m, v) in &row.metrics {
                let val_s = v.map_or_else(String::new, |x| format!("{x:.6}"));
                let _ = writeln!(
                    overall,
                    "{},{},{},{},{},{},,,{n_items},0,{}",
                    csv_esc(&row.dataset_id),
                    domain_slug(row.domain),
                    row.lane,
                    status_slug(row.status),
                    csv_esc(m),
                    val_s,
                    row.contamination_risk,
                );
            }
        } else {
            for iv in &row.metric_intervals {
                let val_s = iv.value.map_or_else(String::new, |x| format!("{x:.6}"));
                let lo_s = iv.ci95_low.map_or_else(String::new, |x| format!("{x:.6}"));
                let hi_s = iv.ci95_high.map_or_else(String::new, |x| format!("{x:.6}"));
                let n_cl = iv.n_clusters.unwrap_or(0);
                let _ = writeln!(
                    overall,
                    "{},{},{},{},{},{val_s},{lo_s},{hi_s},{n_items},{n_cl},{}",
                    csv_esc(&row.dataset_id),
                    domain_slug(row.domain),
                    row.lane,
                    status_slug(row.status),
                    csv_esc(&iv.metric),
                    row.contamination_risk,
                );
            }
        }
    }
    let overall_path = tables_dir.join("overall.csv");
    fs::write(&overall_path, &overall)?;
    created.push("tables/overall.csv".to_owned());

    // 2. status.csv
    let mut status_csv = String::from("dataset_id,domain,lane,status,contamination_risk,reason\n");
    for s in status_rows {
        let _ = writeln!(
            status_csv,
            "{},{},{},{},{},{}",
            csv_esc(&s.dataset_id),
            csv_esc(&s.domain),
            csv_esc(&s.lane),
            csv_esc(&s.status),
            csv_esc(&s.contamination_risk),
            csv_esc(&s.reason),
        );
    }
    fs::write(tables_dir.join("status.csv"), &status_csv)?;
    created.push("tables/status.csv".to_owned());

    // 3. cost.csv
    let mut cost_csv = String::from(
        "dataset_id,domain,n_items,n_clusters,percepts,tokens_in,tokens_out,model_calls,judge_calls,wakes_per_event,silence_rate,latency_p50_ms,latency_p95_ms,wall_elapsed_ms,failed_items\n",
    );
    for c in cost_rows {
        let _ = writeln!(
            cost_csv,
            "{},{},{},{},{},{},{},{},{},{:.6},{:.6},{},{},{},{}",
            csv_esc(&c.dataset_id),
            csv_esc(&c.domain),
            c.n_items,
            c.n_clusters,
            c.percepts,
            c.tokens_in,
            c.tokens_out,
            c.model_calls,
            c.judge_calls,
            c.wakes_per_event,
            c.silence_rate,
            c.latency_p50_ms,
            c.latency_p95_ms,
            c.wall_elapsed_ms,
            c.failed_items,
        );
    }
    fs::write(tables_dir.join("cost.csv"), &cost_csv)?;
    created.push("tables/cost.csv".to_owned());

    // 4. Per-domain CSVs
    let domains = [
        BenchmarkDomain::Memory,
        BenchmarkDomain::Prospective,
        BenchmarkDomain::ProactivityTiming,
        BenchmarkDomain::GroupChat,
        BenchmarkDomain::TheoryOfMind,
        BenchmarkDomain::IndonesianRegister,
        BenchmarkDomain::Security,
    ];
    for dom in domains {
        let slug = domain_slug(dom);
        let mut dom_csv = String::from(
            "dataset_id,metric,value,ci95_low,ci95_high,n_items,n_clusters,status,contamination_risk\n",
        );
        for row in scorecard_rows.iter().filter(|r| r.domain == dom) {
            let n_items = by_ds
                .get(&row.dataset_id)
                .and_then(|l| l.record.dataset_provenance.as_ref())
                .map_or(0, |p| p.chosen_item_ids.len());
            if row.metric_intervals.is_empty() {
                for (m, v) in &row.metrics {
                    let val_s = v.map_or_else(String::new, |x| format!("{x:.6}"));
                    let _ = writeln!(
                        dom_csv,
                        "{},{},{val_s},,,{n_items},0,{},{}",
                        csv_esc(&row.dataset_id),
                        csv_esc(m),
                        status_slug(row.status),
                        row.contamination_risk,
                    );
                }
            } else {
                for iv in &row.metric_intervals {
                    let val_s = iv.value.map_or_else(String::new, |x| format!("{x:.6}"));
                    let lo_s = iv.ci95_low.map_or_else(String::new, |x| format!("{x:.6}"));
                    let hi_s = iv.ci95_high.map_or_else(String::new, |x| format!("{x:.6}"));
                    let _ = writeln!(
                        dom_csv,
                        "{},{},{val_s},{lo_s},{hi_s},{n_items},{},{},{}",
                        csv_esc(&row.dataset_id),
                        csv_esc(&iv.metric),
                        iv.n_clusters.unwrap_or(0),
                        status_slug(row.status),
                        row.contamination_risk,
                    );
                }
            }
        }
        let fname = format!("tables/{slug}.csv");
        fs::write(tables_dir.join(format!("{slug}.csv")), &dom_csv)?;
        created.push(fname);
    }

    Ok(created)
}

#[derive(Debug, Clone)]
struct FailureExample {
    item_id: String,
    question: String,
    gold_answer: String,
    soul_answer: String,
    judge_verdict: String,
    md_rel_path: String,
    jsonl_rel_path: String,
}

type FailureAnalysisOutput = (BTreeMap<String, Vec<FailureExample>>, Vec<String>);

fn build_failure_analysis(
    by_ds: &BTreeMap<String, LoadedDatasetRun>,
    tx_out_dir: &Path,
) -> Result<FailureAnalysisOutput, LabError> {
    let mut by_dataset: BTreeMap<String, Vec<FailureExample>> = BTreeMap::new();
    let mut created_md = Vec::new();

    for (ds_id, loaded) in by_ds {
        let mut wrong: Vec<&ItemOutcomeSummary> = loaded
            .outcomes
            .iter()
            .filter(|o| o.failed || o.passed == Some(false) || o.score.is_some_and(|s| s < 0.5))
            .take(5)
            .collect();
        if wrong.is_empty() {
            // Also surface failed_items recorded on provenance if outcomes.json was empty.
            if let Some(prov) = &loaded.record.dataset_provenance {
                let mut exs = Vec::new();
                for fi in prov.failed_items.iter().take(5) {
                    let safe_ds = sanitize_id(ds_id);
                    let safe_it = sanitize_id(&fi.item_id);
                    let md_rel = format!("transcripts/{safe_ds}/{safe_it}.md");
                    let jsonl_rel = format!("transcripts/{safe_ds}/{safe_it}.jsonl");
                    let ds_dir = tx_out_dir.join(&safe_ds);
                    fs::create_dir_all(&ds_dir)?;
                    let md_body = format!(
                        "# Failure Example: `{ds_id}` / `{}`\n\n- **Stage:** {}\n- **Reason:** {}\n- **Transcript JSONL:** [`{safe_it}.jsonl`]({safe_it}.jsonl)\n",
                        fi.item_id, fi.stage, fi.reason
                    );
                    fs::write(ds_dir.join(format!("{safe_it}.md")), md_body)?;
                    created_md.push(md_rel.clone());
                    exs.push(FailureExample {
                        item_id: fi.item_id.clone(),
                        question: "(unavailable)".to_owned(),
                        gold_answer: "(unavailable)".to_owned(),
                        soul_answer: "(failed)".to_owned(),
                        judge_verdict: fi.reason.clone(),
                        md_rel_path: md_rel,
                        jsonl_rel_path: jsonl_rel,
                    });
                }
                if !exs.is_empty() {
                    by_dataset.insert(ds_id.clone(), exs);
                }
            }
            continue;
        }

        let safe_ds = sanitize_id(ds_id);
        let ds_dir = tx_out_dir.join(&safe_ds);
        fs::create_dir_all(&ds_dir)?;
        let mut exs = Vec::new();
        let mut seen_items = BTreeSet::new();
        for o in wrong.drain(..) {
            if !seen_items.insert(o.item_id.clone()) {
                continue;
            }
            let safe_it = sanitize_id(&o.item_id);
            let md_rel = format!("transcripts/{safe_ds}/{safe_it}.md");
            let jsonl_rel = format!("transcripts/{safe_ds}/{safe_it}.jsonl");
            let verdict_str = if o.failed {
                format!("FAILED: {}", o.failure_reason.as_deref().unwrap_or("error"))
            } else {
                o.judge_verdict
                    .clone()
                    .unwrap_or_else(|| format!("score={:.3}", o.score.unwrap_or(0.0)))
            };
            let soul_ans = if o.soul_answer.trim().is_empty() {
                "(silent / no spoken reply)".to_owned()
            } else {
                o.soul_answer.clone()
            };
            let md_body = format!(
                "# Failure Example: `{ds_id}` / `{}`\n\n- **Category:** {}\n- **Metric:** {} (score: {})\n- **Question / Probe:**\n  > {}\n- **Gold Answer:** `{}`\n- **Soul Answer:** `{}`\n- **Judge / Scorer Verdict:** `{}`\n- **Full Transcript:** [`{safe_it}.jsonl`]({safe_it}.jsonl)\n",
                o.item_id,
                o.category.as_deref().unwrap_or("n/a"),
                o.metric_name,
                o.score
                    .map_or_else(|| "none".to_owned(), |v| format!("{v:.3}")),
                o.question.replace('\n', " "),
                o.gold_answer.replace('\n', " "),
                soul_ans.replace('\n', " "),
                verdict_str.replace('\n', " "),
            );
            fs::write(ds_dir.join(format!("{safe_it}.md")), md_body)?;
            created_md.push(md_rel.clone());
            exs.push(FailureExample {
                item_id: o.item_id.clone(),
                question: o.question.clone(),
                gold_answer: o.gold_answer.clone(),
                soul_answer: soul_ans,
                judge_verdict: verdict_str,
                md_rel_path: md_rel,
                jsonl_rel_path: jsonl_rel,
            });
        }
        if !exs.is_empty() {
            by_dataset.insert(ds_id.clone(), exs);
        }
    }

    Ok((by_dataset, created_md))
}

fn generate_charts(
    charts_dir: &Path,
    domain_rows: &[serde_json::Value],
    category_rows: &[serde_json::Value],
    cost_rows: &[CostRow],
    pareto_points: &[serde_json::Value],
    skip_python: bool,
) -> Result<Vec<String>, LabError> {
    let ws_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let script = ws_root.join("lab/adapters/eval_helpers/render_charts.py");
    let py = std::env::var("MONOAGENS_ADAPTER_PYTHON").unwrap_or_else(|_| {
        let venv_py = ws_root.join("lab/adapters/.venv/bin/python");
        if venv_py.is_file() {
            venv_py.to_string_lossy().into_owned()
        } else {
            "python3".to_owned()
        }
    });

    if !skip_python && script.is_file() {
        let payload = serde_json::json!({
            "charts_dir": charts_dir.to_string_lossy(),
            "domain_rows": domain_rows,
            "category_rows": category_rows,
            "cost_rows": cost_rows,
            "pareto_points": pareto_points,
        });
        if let Ok(mut child) = Command::new(&py)
            .arg(&script)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
        {
            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(&serde_json::to_vec(&payload).unwrap_or_default());
            }
            if let Ok(out) = child.wait_with_output()
                && out.status.success()
                && let Ok(val) = serde_json::from_slice::<serde_json::Value>(&out.stdout)
                && let Some(obj) = val.get("charts").and_then(|v| v.as_object())
            {
                let files: Vec<String> = obj
                    .values()
                    .filter_map(|v| v.as_str().map(|f| format!("charts/{f}")))
                    .collect();
                if !files.is_empty() {
                    return Ok(files);
                }
            }
        }
    }

    // Pure-Rust SVG chart fallback when matplotlib is absent (e.g. offline minimal test env).
    let mut svg_files = Vec::new();
    let mut bars: Vec<(String, f64)> = Vec::new();
    for dr in domain_rows {
        let ds_id = dr.get("dataset_id").and_then(|v| v.as_str()).unwrap_or("");
        if let Some(arr) = dr.get("metrics").and_then(|v| v.as_array()) {
            for m in arr {
                if let (Some(name), Some(val)) = (
                    m.get("metric").and_then(|v| v.as_str()),
                    m.get("value").and_then(|v| v.as_f64()),
                ) {
                    bars.push((format!("{ds_id}:{name}"), val));
                }
            }
        }
    }
    if !bars.is_empty() {
        let svg = render_simple_bar_svg("MonoAgens Benchmark Primary Metrics", &bars);
        fs::write(charts_dir.join("overall_metrics.svg"), svg)?;
        svg_files.push("charts/overall_metrics.svg".to_owned());
    }
    if !cost_rows.is_empty() {
        let wake_bars: Vec<(String, f64)> = cost_rows
            .iter()
            .map(|c| (c.dataset_id.clone(), c.wakes_per_event))
            .collect();
        let svg = render_simple_bar_svg("Gate Wakes per Percept", &wake_bars);
        fs::write(charts_dir.join("wake_rates.svg"), svg)?;
        svg_files.push("charts/wake_rates.svg".to_owned());
    }
    Ok(svg_files)
}

fn render_simple_bar_svg(title: &str, bars: &[(String, f64)]) -> String {
    let width = 760usize;
    let height = 100 + bars.len() * 34;
    let mut out = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}"><rect width="100%" height="100%" fill="#ffffff"/><text x="20" y="32" font-family="sans-serif" font-size="16" font-weight="bold" fill="#0f172a">{}</text>"##,
        html_esc(title)
    );
    let max_val = bars
        .iter()
        .map(|(_, v)| *v)
        .fold(1.0_f64, f64::max)
        .max(1e-6);
    for (idx, (lbl, val)) in bars.iter().enumerate() {
        let y = 55 + idx * 34;
        let bar_w = ((*val / max_val) * 380.0).clamp(2.0, 380.0);
        let _ = write!(
            out,
            r##"<text x="20" y="{}" font-family="monospace" font-size="11" fill="#334155">{}</text><rect x="280" y="{}" width="{bar_w:.1}" height="20" fill="#2563eb" rx="3"/><text x="{}" y="{}" font-family="monospace" font-size="11" fill="#0f172a">{val:.3}</text>"##,
            y + 14,
            html_esc(lbl),
            y,
            286.0 + bar_w,
            y + 14,
        );
    }
    out.push_str("</svg>");
    out
}

struct ReproInfo {
    git_commit: String,
    prior_digest: String,
    manifest_digest: String,
    model_id: String,
    provider_host: String,
    reasoning_effort: String,
    judge_model: String,
    judge_reasoning_effort: String,
    extra_header_names: String,
    embedder_id: String,
    embedder_backend: String,
    embedder_revision: String,
    seeds: String,
    history_reasoning_mode: String,
    monoagens_mode: String,
    lab_version: String,
    started_at: String,
    finished_at: String,
    preflight_summary: String,
    notes: Vec<String>,
}

fn build_repro_info(
    _run_dir: &Path,
    _manifest: Option<&DatasetsManifest>,
    by_ds: &BTreeMap<String, LoadedDatasetRun>,
    preflight: Option<&PreflightEstimate>,
    extra_notes: &[String],
) -> ReproInfo {
    let git_commit = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_owned());

    let mut prior_digests = BTreeSet::new();
    let mut manifest_digests = BTreeSet::new();
    let mut models = BTreeSet::new();
    let mut hosts = BTreeSet::new();
    let mut efforts = BTreeSet::new();
    let mut judges = BTreeSet::new();
    let mut j_efforts = BTreeSet::new();
    let mut headers = BTreeSet::new();
    let mut embedders = BTreeSet::new();
    let mut emb_backends = BTreeSet::new();
    let mut emb_revs = BTreeSet::new();
    let mut seeds = BTreeSet::new();
    let mut hist_modes = BTreeSet::new();
    let mut monoagens_modes = BTreeSet::new();
    let mut lab_vers = BTreeSet::new();
    let mut starts = BTreeSet::new();
    let mut finishes = BTreeSet::new();

    for loaded in by_ds.values() {
        let r = &loaded.record;
        prior_digests.insert(r.prior_digest.clone());
        if r.model_id != "none" {
            models.insert(r.model_id.clone());
        }
        embedders.insert(r.embedder_id.clone());
        seeds.insert(r.seed.to_string());
        lab_vers.insert(r.core_version.clone());
        if let Some(hm) = &r.history_reasoning_mode {
            hist_modes.insert(hm.clone());
        }
        if let Some(mm) = &r.monoagens_mode {
            monoagens_modes.insert(mm.clone());
        }
        if let Some(p) = &r.dataset_provenance {
            manifest_digests.insert(p.manifest_digest.clone());
            if let Some(h) = &p.provider_endpoint_host {
                hosts.insert(h.clone());
            }
            if let Some(e) = &p.reasoning_effort {
                efforts.insert(e.clone());
            }
            if let Some(jm) = &p.judge_model {
                judges.insert(jm.clone());
            }
            if let Some(je) = &p.judge_reasoning_effort {
                j_efforts.insert(je.clone());
            }
            for hn in &p.extra_header_names {
                headers.insert(hn.clone());
            }
            if let Some(eb) = &p.embedder_backend {
                emb_backends.insert(eb.clone());
            }
            if let Some(er) = &p.embedder_revision {
                emb_revs.insert(er.clone());
            }
            if let Some(s) = &p.started_at_utc {
                starts.insert(s.clone());
            }
            if let Some(f) = &p.finished_at_utc {
                finishes.insert(f.clone());
            }
        }
    }

    let join_or = |set: BTreeSet<String>, fallback: &str| {
        if set.is_empty() {
            fallback.to_owned()
        } else {
            set.into_iter().collect::<Vec<_>>().join(", ")
        }
    };

    let preflight_summary = preflight.map_or_else(
        || "none saved".to_owned(),
        |pf| {
            format!(
                "history_percepts={}, estimated_wakes={}, probe_calls={}, judge_calls={}, worst_case_tokens={}, worst_case_usd=${:.2}",
                pf.total_history_percepts,
                pf.total_estimated_history_wakes,
                pf.total_estimated_probe_calls,
                pf.total_estimated_judge_calls,
                pf.total_worst_case_tokens,
                pf.total_worst_case_usd,
            )
        },
    );

    ReproInfo {
        git_commit,
        prior_digest: join_or(prior_digests, "unknown"),
        manifest_digest: join_or(manifest_digests, "unknown"),
        model_id: join_or(models, "none"),
        provider_host: join_or(hosts, "unspecified"),
        reasoning_effort: join_or(efforts, "unspecified"),
        judge_model: join_or(judges, "none"),
        judge_reasoning_effort: join_or(j_efforts, "unspecified"),
        extra_header_names: join_or(headers, "none"),
        embedder_id: join_or(embedders, "none"),
        embedder_backend: join_or(emb_backends, "unspecified"),
        embedder_revision: join_or(emb_revs, "unspecified"),
        seeds: join_or(seeds, "1"),
        history_reasoning_mode: join_or(hist_modes, "scripted_silent"),
        monoagens_mode: join_or(monoagens_modes, "absent"),
        lab_version: join_or(lab_vers, env!("CARGO_PKG_VERSION")),
        started_at: starts
            .into_iter()
            .next()
            .unwrap_or_else(|| "unknown".to_owned()),
        finished_at: finishes
            .into_iter()
            .next_back()
            .unwrap_or_else(|| "unknown".to_owned()),
        preflight_summary,
        notes: extra_notes.to_vec(),
    }
}

#[allow(clippy::too_many_arguments)]
fn render_markdown_report(
    repro: &ReproInfo,
    registry: &[BenchmarkDatasetSpec],
    scorecard_rows: &[ScorecardRow],
    status_rows: &[StatusRow],
    cost_rows: &[CostRow],
    by_ds: &BTreeMap<String, LoadedDatasetRun>,
    failure_sections: &BTreeMap<String, Vec<FailureExample>>,
    chart_files: &[String],
    pareto: &crate::pareto::ConvexParetoReport,
    label: Option<&str>,
    appendix: Option<&str>,
) -> String {
    let mut md = String::new();
    md.push_str("# MonoAgens Lab Benchmark Report\n\n");
    if let Some(l) = label {
        let _ = writeln!(md, "> **{l}**\n");
    }

    // 1. Summary
    let run_cnt = scorecard_rows
        .iter()
        .filter(|r| matches!(r.status, ScorecardStatus::Run | ScorecardStatus::Partial))
        .count();
    let total_tok_in: u64 = cost_rows.iter().map(|c| c.tokens_in).sum();
    let total_tok_out: u64 = cost_rows.iter().map(|c| c.tokens_out).sum();
    let total_model_calls: u64 = cost_rows.iter().map(|c| c.model_calls).sum();
    let total_judge_calls: u64 = cost_rows.iter().map(|c| c.judge_calls).sum();
    let total_failed: usize = cost_rows.iter().map(|c| c.failed_items).sum();

    md.push_str("## 1. Summary\n\n");
    let _ = writeln!(
        md,
        "- **External datasets scored/partial:** {run_cnt} / {} registered external datasets",
        scorecard_rows.len()
    );
    let _ = writeln!(
        md,
        "- **Total tokens:** {total_tok_in} input + {total_tok_out} output = {} total tokens",
        total_tok_in + total_tok_out
    );
    let _ = writeln!(
        md,
        "- **Total model calls:** {total_model_calls} soul calls, {total_judge_calls} judge calls, {total_failed} failed items"
    );
    let _ = writeln!(
        md,
        "- **Pareto frontier (scored rows only):** {} points ({} gaps excluded without imputation)",
        pareto.undominated_frontier.len(),
        pareto.gaps.len()
    );
    md.push('\n');

    // 2. Reproducibility
    md.push_str("## 2. Reproducibility\n\n");
    if let Some(l) = label {
        let _ = writeln!(md, "**{l}**\n");
    }
    md.push_str("| Field | Value |\n|---|---|\n");
    let _ = writeln!(md, "| Git commit | `{}` |", repro.git_commit);
    let _ = writeln!(md, "| Lab version | `{}` |", repro.lab_version);
    let _ = writeln!(md, "| Prior digest | `{}` |", repro.prior_digest);
    let _ = writeln!(md, "| Manifest digest | `{}` |", repro.manifest_digest);
    let _ = writeln!(
        md,
        "| Soul model | `{}` (reasoning effort: `{}`) |",
        repro.model_id, repro.reasoning_effort
    );
    let _ = writeln!(
        md,
        "| Judge model | `{}` (reasoning effort: `{}`) |",
        repro.judge_model, repro.judge_reasoning_effort
    );
    let _ = writeln!(md, "| Provider host | `{}` |", repro.provider_host);
    let _ = writeln!(
        md,
        "| Extra header names (values redacted) | `{}` |",
        repro.extra_header_names
    );
    let _ = writeln!(
        md,
        "| Embedder | `{}` (backend: `{}`, revision: `{}`) |",
        repro.embedder_id, repro.embedder_backend, repro.embedder_revision
    );
    let _ = writeln!(
        md,
        "| History reasoning mode | `{}` |",
        repro.history_reasoning_mode
    );
    let _ = writeln!(md, "| MonoAgens mode | `{}` |", repro.monoagens_mode);
    let _ = writeln!(md, "| Seeds | `{}` |", repro.seeds);
    let _ = writeln!(
        md,
        "| Start / End UTC | `{}` -> `{}` |",
        repro.started_at, repro.finished_at
    );
    let _ = writeln!(md, "| Preflight estimate | {} |", repro.preflight_summary);
    for note in &repro.notes {
        let _ = writeln!(md, "| Note | {} |", note.replace('|', "\\|"));
    }
    md.push('\n');

    // 3. Per-domain results
    md.push_str("## 3. Results by Domain\n\n");
    let domains = [
        (BenchmarkDomain::Memory, "Memory"),
        (
            BenchmarkDomain::Prospective,
            "Prospective Memory & Delegation",
        ),
        (BenchmarkDomain::ProactivityTiming, "Proactivity & Timing"),
        (BenchmarkDomain::GroupChat, "Group Chat"),
        (BenchmarkDomain::TheoryOfMind, "Theory of Mind"),
        (BenchmarkDomain::IndonesianRegister, "Indonesian Probes"),
        (BenchmarkDomain::Security, "Security"),
    ];
    for (dom, title) in domains {
        let dom_rows: Vec<&ScorecardRow> =
            scorecard_rows.iter().filter(|r| r.domain == dom).collect();
        if dom_rows.is_empty() {
            continue;
        }
        let _ = writeln!(md, "### {title}\n");
        if let Some(l) = label {
            let _ = writeln!(md, "**{l}**\n");
        }
        md.push_str("| Dataset | Metric | Score | 95% CI | n items | n clusters | Status | Contamination |\n");
        md.push_str("|---|---|---|---|---:|---:|---|---|\n");
        for row in dom_rows {
            let n_items = by_ds
                .get(&row.dataset_id)
                .and_then(|l| l.record.dataset_provenance.as_ref())
                .map_or(0, |p| p.chosen_item_ids.len());
            if row.metric_intervals.is_empty() {
                for (m, v) in &row.metrics {
                    let s = v.map_or_else(|| "—".to_owned(), |x| format!("{x:.4}"));
                    let _ = writeln!(
                        md,
                        "| `{}` | `{}` | {} | — | {} | 0 | `{}` | `{}` |",
                        row.dataset_id,
                        m,
                        s,
                        n_items,
                        status_slug(row.status),
                        row.contamination_risk
                    );
                }
            } else {
                for iv in &row.metric_intervals {
                    let s = iv
                        .value
                        .map_or_else(|| "—".to_owned(), |x| format!("{x:.4}"));
                    let ci = match (iv.ci95_low, iv.ci95_high) {
                        (Some(lo), Some(hi)) => format!("[{lo:.4}, {hi:.4}]"),
                        _ => "—".to_owned(),
                    };
                    let n_cl = iv.n_clusters.unwrap_or(0);
                    let _ = writeln!(
                        md,
                        "| `{}` | `{}` | {} | {} | {} | {} | `{}` | `{}` |",
                        row.dataset_id,
                        iv.metric,
                        s,
                        ci,
                        n_items,
                        n_cl,
                        status_slug(row.status),
                        row.contamination_risk
                    );
                }
            }
        }
        md.push('\n');
    }

    // 4. Per-category breakdowns
    let has_cats = by_ds.values().any(|l| {
        l.record
            .dataset_provenance
            .as_ref()
            .is_some_and(|p| !p.category_metrics.is_empty())
    });
    if has_cats {
        md.push_str("## 4. Per-Category Breakdowns\n\n");
        for (ds_id, loaded) in by_ds {
            let Some(prov) = &loaded.record.dataset_provenance else {
                continue;
            };
            if prov.category_metrics.is_empty() {
                continue;
            }
            let _ = writeln!(md, "### `{ds_id}`\n");
            if let Some(l) = label {
                let _ = writeln!(md, "**{l}**\n");
            }
            md.push_str("| Category | Metric | Value |\n|---|---|---:|\n");
            for (cat, mmap) in &prov.category_metrics {
                for (mname, mval) in mmap {
                    let _ = writeln!(md, "| `{cat}` | `{mname}` | {mval:.4} |");
                }
            }
            md.push('\n');
        }
    }

    // 5. Status & Gap table
    md.push_str("## 5. Status & Gap Table (All 23 External + In-Tree Tool RAG)\n\n");
    if let Some(l) = label {
        let _ = writeln!(md, "**{l}**\n");
    }
    md.push_str("| Dataset | Domain | Lane | Status | Contamination | Reason / Notes |\n|---|---|---|---|---|---|\n");
    for s in status_rows {
        let _ = writeln!(
            md,
            "| `{}` | `{}` | `{}` | `{}` | `{}` | {} |",
            s.dataset_id,
            s.domain,
            s.lane,
            s.status,
            s.contamination_risk,
            s.reason.replace('|', "\\|")
        );
    }
    let _ = registry;
    md.push('\n');

    // 6. Cost & Behaviour
    md.push_str("## 6. Cost & Behaviour\n\n");
    if let Some(l) = label {
        let _ = writeln!(md, "**{l}**\n");
    }
    md.push_str("| Dataset | Items | Clusters | Percepts | Tokens In | Tokens Out | Soul Calls | Judge Calls | Wakes/Percept | Silence Rate | Latency p50/p95 (ms) | Failed Items |\n");
    md.push_str("|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|\n");
    for c in cost_rows {
        let _ = writeln!(
            md,
            "| `{}` | {} | {} | {} | {} | {} | {} | {} | {:.3} | {:.3} | {}/{} | {} |",
            c.dataset_id,
            c.n_items,
            c.n_clusters,
            c.percepts,
            c.tokens_in,
            c.tokens_out,
            c.model_calls,
            c.judge_calls,
            c.wakes_per_event,
            c.silence_rate,
            c.latency_p50_ms,
            c.latency_p95_ms,
            c.failed_items,
        );
    }
    md.push('\n');

    // 7. Charts
    if !chart_files.is_empty() {
        md.push_str("## 7. Charts\n\n");
        for cf in chart_files {
            let _ = writeln!(md, "![{cf}]({cf})\n");
        }
    }

    // 8. Failure Analysis
    md.push_str("## 8. Failure Analysis\n\n");
    if failure_sections.is_empty() {
        md.push_str("No item failures or low-scoring examples were recorded.\n\n");
    } else {
        for (ds_id, exs) in failure_sections {
            let _ = writeln!(md, "### `{ds_id}`\n");
            for ex in exs {
                let _ = writeln!(
                    md,
                    "- **`{}`** ([summary]({}), [jsonl]({})) — Q: `{}` | Gold: `{}` | Soul: `{}` | Verdict: `{}`",
                    ex.item_id,
                    ex.md_rel_path,
                    ex.jsonl_rel_path,
                    truncate_inline(&ex.question, 120),
                    truncate_inline(&ex.gold_answer, 120),
                    truncate_inline(&ex.soul_answer, 120),
                    truncate_inline(&ex.judge_verdict, 120),
                );
            }
            md.push('\n');
        }
    }

    // 9. Caveats
    md.push_str("## 9. Methodological Caveats\n\n");
    md.push_str("- **Non-official judge substitution:** Official judges (`gpt-4o`, `gpt-5`, `Gemini-3.1-flash-lite`) were not used; judge-scored metrics are emitted under `<metric>_non_official_judge`.\n");
    let _ = writeln!(
        md,
        "- **History reasoning mode:** recorded mode(s): `{}`. Offline suites load static memory/ToM histories through Core's unmodified loop with a silent scripted model; live memory runs load every turn through the live model.",
        repro.history_reasoning_mode
    );
    md.push_str("- **Contamination & small clusters:** Public benchmarks released before 2026 carry moderate-to-high contamination risk; clustered corpora (`locomo`, `memory_agent_bench`, `group_mem_bench`, `capa`, `pitch`, `fantom`, `molweni`, `hi_tom`, `dyntom`) report cluster-bootstrap 95% CIs.\n");
    md.push_str("- **License terms:** `locomo`, `opentom`, `exploretom`, and `tomi` are `CC-BY-NC-4.0` (non-commercial research use only); `opentom` requests ran under ZDR (`0 days` retention) on OpenCode Go.\n");
    md.push_str("- **Gaps & cost cap:** `pis`, `proact`, and `mpbench` remain `unavailable` without silent substitution. Owner waived the USD cost cap (`--max-cost-usd 1000000`).\n");
    if let Some(app) = appendix {
        md.push_str("\n---\n\n");
        md.push_str(app);
        md.push('\n');
    }

    md
}

#[allow(clippy::too_many_arguments)]
fn render_html_report(
    repro: &ReproInfo,
    _registry: &[BenchmarkDatasetSpec],
    scorecard_rows: &[ScorecardRow],
    status_rows: &[StatusRow],
    cost_rows: &[CostRow],
    by_ds: &BTreeMap<String, LoadedDatasetRun>,
    failure_sections: &BTreeMap<String, Vec<FailureExample>>,
    charts_dir: &Path,
    chart_files: &[String],
    pareto: &crate::pareto::ConvexParetoReport,
    label: Option<&str>,
    appendix: Option<&str>,
) -> Result<String, LabError> {
    let mut html = String::new();
    html.push_str("<!DOCTYPE html><html lang=\"en\"><head><meta charset=\"utf-8\">");
    html.push_str("<title>MonoAgens Lab Benchmark Report</title>");
    html.push_str("<style>");
    html.push_str("body{font-family:system-ui,-apple-system,sans-serif;max-width:1280px;margin:2rem auto;padding:0 1.5rem;color:#0f172a;background:#f8fafc;line-height:1.5}");
    html.push_str("h1,h2,h3{color:#0f172a}table{border-collapse:collapse;width:100%;margin:1rem 0;background:#fff;font-size:0.9rem}");
    html.push_str("th,td{border:1px solid #cbd5e1;padding:0.45rem 0.65rem;text-align:left}th{background:#f1f5f9}");
    html.push_str(".card{background:#fff;border:1px solid #e2e8f0;border-radius:8px;padding:1rem 1.25rem;margin:1rem 0}");
    html.push_str("img.chart{max-width:100%;height:auto;border:1px solid #e2e8f0;border-radius:6px;background:#fff;margin:0.75rem 0}");
    html.push_str("code{background:#f1f5f9;padding:0.1rem 0.3rem;border-radius:4px}");
    html.push_str("</style></head><body>");
    html.push_str("<h1>MonoAgens Lab Benchmark Report</h1>");

    let run_cnt = scorecard_rows
        .iter()
        .filter(|r| matches!(r.status, ScorecardStatus::Run | ScorecardStatus::Partial))
        .count();
    let total_tok_in: u64 = cost_rows.iter().map(|c| c.tokens_in).sum();
    let total_tok_out: u64 = cost_rows.iter().map(|c| c.tokens_out).sum();

    html.push_str("<div class=\"card\"><h2>1. Summary &amp; Reproducibility</h2><table>");
    let kv = [
        (
            "External datasets scored/partial",
            format!("{run_cnt} / {}", scorecard_rows.len()),
        ),
        (
            "Total tokens (in / out / total)",
            format!(
                "{total_tok_in} / {total_tok_out} / {}",
                total_tok_in + total_tok_out
            ),
        ),
        (
            "Pareto frontier / gaps",
            format!(
                "{} frontier / {} gaps",
                pareto.undominated_frontier.len(),
                pareto.gaps.len()
            ),
        ),
        ("Git commit", repro.git_commit.clone()),
        ("Prior digest", repro.prior_digest.clone()),
        ("Manifest digest", repro.manifest_digest.clone()),
        (
            "Soul model & effort",
            format!("{} (effort={})", repro.model_id, repro.reasoning_effort),
        ),
        (
            "Judge model & effort",
            format!(
                "{} (effort={})",
                repro.judge_model, repro.judge_reasoning_effort
            ),
        ),
        ("Provider host", repro.provider_host.clone()),
        ("Extra header names", repro.extra_header_names.clone()),
        (
            "Embedder",
            format!(
                "{} (backend={}, rev={})",
                repro.embedder_id, repro.embedder_backend, repro.embedder_revision
            ),
        ),
        (
            "History reasoning mode",
            repro.history_reasoning_mode.clone(),
        ),
        ("MonoAgens mode", repro.monoagens_mode.clone()),
        ("Seeds", repro.seeds.clone()),
        (
            "Start / End UTC",
            format!("{} -> {}", repro.started_at, repro.finished_at),
        ),
        ("Preflight estimate", repro.preflight_summary.clone()),
    ];
    for (k, v) in kv {
        let _ = write!(
            html,
            "<tr><th>{}</th><td><code>{}</code></td></tr>",
            html_esc(k),
            html_esc(&v)
        );
    }
    for note in &repro.notes {
        let _ = write!(html, "<tr><th>Note</th><td>{}</td></tr>", html_esc(note));
    }
    html.push_str("</table></div>");

    html.push_str("<div class=\"card\"><h2>2. Results by Domain</h2><table>");
    html.push_str("<tr><th>Domain</th><th>Dataset</th><th>Metric</th><th>Score</th><th>95% CI</th><th>Items</th><th>Clusters</th><th>Status</th><th>Contamination</th></tr>");
    for row in scorecard_rows {
        let n_items = by_ds
            .get(&row.dataset_id)
            .and_then(|l| l.record.dataset_provenance.as_ref())
            .map_or(0, |p| p.chosen_item_ids.len());
        if row.metric_intervals.is_empty() {
            for (m, v) in &row.metrics {
                let s = v.map_or_else(|| "—".to_owned(), |x| format!("{x:.4}"));
                let _ = write!(
                    html,
                    "<tr><td>{}</td><td><code>{}</code></td><td><code>{}</code></td><td>{s}</td><td>—</td><td>{n_items}</td><td>0</td><td>{}</td><td>{}</td></tr>",
                    domain_slug(row.domain),
                    html_esc(&row.dataset_id),
                    html_esc(m),
                    status_slug(row.status),
                    row.contamination_risk,
                );
            }
        } else {
            for iv in &row.metric_intervals {
                let s = iv
                    .value
                    .map_or_else(|| "—".to_owned(), |x| format!("{x:.4}"));
                let ci = match (iv.ci95_low, iv.ci95_high) {
                    (Some(lo), Some(hi)) => format!("[{lo:.4}, {hi:.4}]"),
                    _ => "—".to_owned(),
                };
                let _ = write!(
                    html,
                    "<tr><td>{}</td><td><code>{}</code></td><td><code>{}</code></td><td>{s}</td><td>{ci}</td><td>{n_items}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
                    domain_slug(row.domain),
                    html_esc(&row.dataset_id),
                    html_esc(&iv.metric),
                    iv.n_clusters.unwrap_or(0),
                    status_slug(row.status),
                    row.contamination_risk,
                );
            }
        }
    }
    html.push_str("</table></div>");

    html.push_str("<div class=\"card\"><h2>3. Status &amp; Gap Table</h2><table>");
    html.push_str("<tr><th>Dataset</th><th>Domain</th><th>Lane</th><th>Status</th><th>Contamination</th><th>Reason</th></tr>");
    for s in status_rows {
        let _ = write!(
            html,
            "<tr><td><code>{}</code></td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
            html_esc(&s.dataset_id),
            html_esc(&s.domain),
            html_esc(&s.lane),
            html_esc(&s.status),
            html_esc(&s.contamination_risk),
            html_esc(&s.reason),
        );
    }
    html.push_str("</table></div>");

    html.push_str("<div class=\"card\"><h2>4. Cost &amp; Behaviour</h2><table>");
    html.push_str("<tr><th>Dataset</th><th>Items</th><th>Percepts</th><th>Tokens In</th><th>Tokens Out</th><th>Soul Calls</th><th>Judge Calls</th><th>Wakes/Percept</th><th>Silence Rate</th><th>p50/p95 ms</th><th>Failed</th></tr>");
    for c in cost_rows {
        let _ = write!(
            html,
            "<tr><td><code>{}</code></td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{:.3}</td><td>{:.3}</td><td>{}/{}</td><td>{}</td></tr>",
            html_esc(&c.dataset_id),
            c.n_items,
            c.percepts,
            c.tokens_in,
            c.tokens_out,
            c.model_calls,
            c.judge_calls,
            c.wakes_per_event,
            c.silence_rate,
            c.latency_p50_ms,
            c.latency_p95_ms,
            c.failed_items,
        );
    }
    html.push_str("</table></div>");

    // Embedded self-contained charts
    if !chart_files.is_empty() {
        html.push_str("<div class=\"card\"><h2>5. Charts</h2>");
        for cf in chart_files {
            let fname = cf.strip_prefix("charts/").unwrap_or(cf);
            let full = charts_dir.join(fname);
            if full.is_file() {
                let bytes = fs::read(&full)?;
                if fname.ends_with(".svg") {
                    html.push_str(&String::from_utf8_lossy(&bytes));
                } else {
                    let b64 = encode_base64(&bytes);
                    let _ = write!(
                        html,
                        "<img class=\"chart\" src=\"data:image/png;base64,{b64}\" alt=\"{}\"/>",
                        html_esc(fname)
                    );
                }
            }
        }
        html.push_str("</div>");
    }

    html.push_str("<div class=\"card\"><h2>6. Failure Analysis</h2>");
    if failure_sections.is_empty() {
        html.push_str("<p>No item failures or low-scoring examples recorded.</p>");
    } else {
        html.push_str("<table><tr><th>Dataset</th><th>Item</th><th>Question</th><th>Gold</th><th>Soul</th><th>Verdict</th><th>Transcript</th></tr>");
        for (ds_id, exs) in failure_sections {
            for ex in exs {
                let _ = write!(
                    html,
                    "<tr><td><code>{}</code></td><td><code>{}</code></td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td><a href=\"{}\">.md</a> / <a href=\"{}\">.jsonl</a></td></tr>",
                    html_esc(ds_id),
                    html_esc(&ex.item_id),
                    html_esc(&truncate_inline(&ex.question, 120)),
                    html_esc(&truncate_inline(&ex.gold_answer, 120)),
                    html_esc(&truncate_inline(&ex.soul_answer, 120)),
                    html_esc(&truncate_inline(&ex.judge_verdict, 120)),
                    html_esc(&ex.md_rel_path),
                    html_esc(&ex.jsonl_rel_path),
                );
            }
        }
        html.push_str("</table>");
    }
    if let Some(l) = label {
        let banner = format!(
            "<div class=\"card\" style=\"border-left:6px solid #dc2626\"><strong>{}</strong></div>",
            html_esc(l)
        );
        if let Some(pos) = html.find("</h1>") {
            html.insert_str(pos + 5, &banner);
        } else {
            html.insert_str(0, &banner);
        }
        html = html.replace(
            "<table>",
            &format!(
                "<table><caption style=\"caption-side:top;text-align:left;font-weight:bold;padding:4px 0\">{}</caption>",
                html_esc(l)
            ),
        );
    }
    if let Some(app) = appendix {
        html.push_str("<div class=\"card\"><h2>Appendix</h2><pre style=\"white-space:pre-wrap\">");
        html.push_str(&html_esc(app));
        html.push_str("</pre></div>");
    }
    html.push_str("</div></body></html>");
    Ok(html)
}

fn domain_slug(dom: BenchmarkDomain) -> &'static str {
    match dom {
        BenchmarkDomain::Memory => "memory",
        BenchmarkDomain::Prospective => "prospective",
        BenchmarkDomain::ProactivityTiming => "proactivity_timing",
        BenchmarkDomain::GroupChat => "group_chat",
        BenchmarkDomain::TheoryOfMind => "theory_of_mind",
        BenchmarkDomain::IndonesianRegister => "indonesian_register",
        BenchmarkDomain::ToolRag => "tool_rag",
        BenchmarkDomain::Security => "security",
    }
}

fn status_slug(st: ScorecardStatus) -> &'static str {
    match st {
        ScorecardStatus::Run => "run",
        ScorecardStatus::Partial => "partial",
        ScorecardStatus::NotScored => "not_scored",
        ScorecardStatus::Unavailable => "unavailable",
        ScorecardStatus::NotMappable => "not_mappable",
        ScorecardStatus::Unverified => "unverified",
    }
}

fn sanitize_id(id: &str) -> String {
    id.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn csv_esc(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_owned()
    }
}

fn html_esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn truncate_inline(s: &str, max_chars: usize) -> String {
    let clean = s.replace(['\n', '\r', '|'], " ");
    let mut chars = clean.chars();
    let taken: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{taken}...")
    } else {
        taken
    }
}

fn encode_base64(input: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[((n >> 18) & 63) as usize] as char);
        out.push(TABLE[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            TABLE[((n >> 6) & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}
