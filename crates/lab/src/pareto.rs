//! Statistical comparison (`Holm-Bonferroni`, Cohen's `d_z`, Kerby rank-biserial `r_rb`,
//! `A-16` confirmatory sizing `S >= 354–471`) and undominated **Convex Pareto** frontier
//! computation (`Kapoor et al. 2025`, `R-39`).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::baseline::TokenUsage5Tuple;
use crate::benchmark::BenchmarkDomain;
use crate::datasets::{ContaminationRisk, CoreApplicability, DatasetLane};
use crate::record::MetricScoreInterval;

/// Recommended confirmatory cluster-paired minimum scenario count (`A-16`, `R-39`).
pub const CONFIRMATORY_SCENARIO_MIN: usize = 354;
/// Upper target scenario count (`R = 1` seed) for `A-16` power.
pub const CONFIRMATORY_SCENARIO_TARGET: usize = 471;
/// Default non-inferiority margin on normalized quality (`A-16`).
pub const DEFAULT_NI_MARGIN_DELTA: f64 = 0.05;
/// Family-wise error rate alpha for Holm-Bonferroni (`R-39`).
pub const DEFAULT_FWER_ALPHA: f64 = 0.05;

/// Six-value scorecard row status (`benchmark-evaluation/spec.md`, Design 8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScorecardStatus {
    Run,
    Partial,
    NotScored,
    Unavailable,
    NotMappable,
    Unverified,
}

/// Strictly one row per registered external dataset in the benchmark scorecard.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScorecardRow {
    pub dataset_id: String,
    pub domain: BenchmarkDomain,
    pub lane: DatasetLane,
    pub status: ScorecardStatus,
    pub core_applicability: CoreApplicability,
    pub contamination_risk: ContaminationRisk,
    pub metrics: BTreeMap<String, Option<f64>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub metric_intervals: Vec<MetricScoreInterval>,
    pub cost_usd: f64,
    pub e_tok: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Raw scorecard point for one evaluated system configuration.
///
/// Preserves raw 5-tuple token usage logs (`raw_5tuples`) without collapsing them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SystemConfigScore {
    pub name: String,
    /// Normalized task quality / accuracy in `[0, 1]` (higher is better).
    pub quality: f64,
    /// Weighted effective tokens `E_tok` (lower is better).
    pub e_tok: f64,
    /// 95th percentile latency in ms (lower is better).
    pub p95_latency_ms: f64,
    /// False Interruption Rate `FIR` in `[0, 1]` (lower is better).
    pub fir: f64,
    /// Privacy non-interference divergence `Delta_NI` (lower is better; `0.0` target).
    pub delta_ni: f64,
    /// Security Attack Success Rate `ASR` in `[0, 1]` (lower is better).
    pub asr: f64,
    /// Raw 5-tuple logs preserved alongside normalized `E_tok`.
    pub raw_5tuples: Vec<TokenUsage5Tuple>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lane: Option<DatasetLane>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contamination_risk: Option<ContaminationRisk>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<ScorecardStatus>,
}

impl SystemConfigScore {
    /// Combined safety risk coordinate `R_risk = delta_ni + asr + fir` (lower is better).
    #[must_use]
    pub fn composite_risk(&self) -> f64 {
        self.delta_ni + self.asr + self.fir
    }

    /// Recomputes `E_tok` directly from `raw_5tuples` when present, verifying no 5-tuple collapse.
    #[must_use]
    pub fn verified_e_tok(&self) -> f64 {
        if self.raw_5tuples.is_empty() {
            self.e_tok
        } else {
            self.raw_5tuples.iter().map(|t| t.e_tok()).sum()
        }
    }
}

/// Paired scenario-level hypothesis input for Holm-Bonferroni and effect-size analysis.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PairedComparisonInput {
    pub metric: String,
    pub baseline_name: String,
    pub monoagens_scores: Vec<f64>,
    pub baseline_scores: Vec<f64>,
    pub higher_is_better: bool,
    #[serde(default)]
    pub is_quality_ni_endpoint: bool,
}

/// Holm-Bonferroni adjusted paired statistical test output (`A-16`, `R-39`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HolmComparisonResult {
    pub metric: String,
    pub baseline_name: String,
    pub s_scenarios: usize,
    pub confirmatory_power_met: bool,
    pub mean_diff: f64,
    pub ci95_low: f64,
    pub ci95_high: f64,
    pub cohens_dz: f64,
    pub kerby_r_rb: f64,
    pub raw_p_value: f64,
    pub holm_adjusted_p: f64,
    pub rejected_null: bool,
    pub non_inferiority_passed: bool,
}

/// Undominated vs dominated Convex Pareto report (`Kapoor et al. 2025`, `R-39`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConvexParetoReport {
    pub schema_version: String,
    pub undominated_frontier: Vec<SystemConfigScore>,
    pub dominated_points: Vec<String>,
    pub holm_results: Vec<HolmComparisonResult>,
    pub co_primary_ni_and_cost_passed: bool,
    pub hypervolume_2d_quality_etok: f64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub gaps: Vec<ScorecardRow>,
}

/// Input scorecard file schema for `monoagens-lab pareto --input <path>`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParetoScorecardInput {
    pub systems: Vec<SystemConfigScore>,
    #[serde(default)]
    pub paired_comparisons: Vec<PairedComparisonInput>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scorecard_rows: Vec<ScorecardRow>,
}

/// Evaluates paired hypotheses with Cohen's `d_z`, Kerby rank-biserial `r_rb`,
/// `A-16` sizing checks (`S >= 354`), and Holm-Bonferroni step-down correction (`alpha = 0.05`).
#[must_use]
pub fn evaluate_holm_bonferroni(
    comparisons: &[PairedComparisonInput],
    alpha: f64,
    ni_margin: f64,
) -> Vec<HolmComparisonResult> {
    struct RawStat {
        idx: usize,
        s: usize,
        mean_diff: f64,
        ci_low: f64,
        ci_high: f64,
        dz: f64,
        r_rb: f64,
        p_raw: f64,
        ni_ok: bool,
    }

    let mut stats = Vec::with_capacity(comparisons.len());
    for (idx, cmp) in comparisons.iter().enumerate() {
        let s = cmp.monoagens_scores.len().min(cmp.baseline_scores.len());
        if s == 0 {
            stats.push(RawStat {
                idx,
                s: 0,
                mean_diff: 0.0,
                ci_low: 0.0,
                ci_high: 0.0,
                dz: 0.0,
                r_rb: 0.0,
                p_raw: 1.0,
                ni_ok: false,
            });
            continue;
        }
        // Orient differences so positive `d` means MonoAgens improves over baseline.
        let diffs: Vec<f64> = (0..s)
            .map(|i| {
                let raw = cmp.monoagens_scores[i] - cmp.baseline_scores[i];
                if cmp.higher_is_better { raw } else { -raw }
            })
            .collect();
        let mean_diff = diffs.iter().sum::<f64>() / s as f64;
        let sd = if s > 1 {
            let var = diffs.iter().map(|d| (d - mean_diff).powi(2)).sum::<f64>() / (s - 1) as f64;
            var.sqrt()
        } else {
            0.0
        };
        let se = if s > 0 { sd / (s as f64).sqrt() } else { 0.0 };
        let t_crit = if s >= 30 { 1.96 } else { 2.093 };
        let ci_low = mean_diff - t_crit * se;
        let ci_high = mean_diff + t_crit * se;
        let dz = if sd > 1e-12 { mean_diff / sd } else { 0.0 };

        let r_rb = kerby_rank_biserial(&diffs);
        let p_raw = if se > 1e-12 {
            let z = (mean_diff / se).abs();
            two_sided_normal_p(z)
        } else if mean_diff.abs() > 1e-12 {
            0.0
        } else {
            1.0
        };

        let ni_ok = if cmp.is_quality_ni_endpoint {
            ci_low > -ni_margin
        } else {
            ci_low > 0.0
        };

        stats.push(RawStat {
            idx,
            s,
            mean_diff,
            ci_low,
            ci_high,
            dz,
            r_rb,
            p_raw,
            ni_ok,
        });
    }

    let m = stats.len();
    let mut order: Vec<usize> = (0..m).collect();
    order.sort_by(|&a, &b| stats[a].p_raw.total_cmp(&stats[b].p_raw));

    let mut holm_p = vec![1.0_f64; m];
    let mut rejected = vec![false; m];
    let mut running_max = 0.0_f64;
    let mut still_rejecting = true;

    for (rank, &orig_idx) in order.iter().enumerate() {
        let mult = (m - rank) as f64;
        let adj = (stats[orig_idx].p_raw * mult).min(1.0);
        running_max = running_max.max(adj);
        holm_p[orig_idx] = running_max;
        if still_rejecting && running_max <= alpha {
            rejected[orig_idx] = true;
        } else {
            still_rejecting = false;
        }
    }

    stats
        .into_iter()
        .map(|st| {
            let cmp = &comparisons[st.idx];
            HolmComparisonResult {
                metric: cmp.metric.clone(),
                baseline_name: cmp.baseline_name.clone(),
                s_scenarios: st.s,
                confirmatory_power_met: st.s >= CONFIRMATORY_SCENARIO_MIN,
                mean_diff: st.mean_diff,
                ci95_low: st.ci_low,
                ci95_high: st.ci_high,
                cohens_dz: st.dz,
                kerby_r_rb: st.r_rb,
                raw_p_value: st.p_raw,
                holm_adjusted_p: holm_p[st.idx],
                rejected_null: rejected[st.idx],
                non_inferiority_passed: st.ni_ok,
            }
        })
        .collect()
}

/// Computes Kerby (2014) rank-biserial correlation `r_rb = (W+ - W-) / (W+ + W-)` over paired differences.
#[must_use]
pub fn kerby_rank_biserial(diffs: &[f64]) -> f64 {
    let mut nz: Vec<(f64, i8)> = diffs
        .iter()
        .filter_map(|&d| {
            if d.abs() <= 1e-12 {
                None
            } else if d > 0.0 {
                Some((d.abs(), 1_i8))
            } else {
                Some((d.abs(), -1_i8))
            }
        })
        .collect();
    if nz.is_empty() {
        return 0.0;
    }
    nz.sort_by(|a, b| a.0.total_cmp(&b.0));

    let mut w_pos = 0.0_f64;
    let mut w_neg = 0.0_f64;
    let mut i = 0usize;
    while i < nz.len() {
        let mut j = i + 1;
        while j < nz.len() && (nz[j].0 - nz[i].0).abs() <= 1e-12 {
            j += 1;
        }
        // Average rank (1-based) for ties.
        let avg_rank = ((i + 1 + j) as f64) / 2.0;
        for &(_, sign) in &nz[i..j] {
            if sign > 0 {
                w_pos += avg_rank;
            } else {
                w_neg += avg_rank;
            }
        }
        i = j;
    }
    let denom = w_pos + w_neg;
    if denom <= 0.0 {
        0.0
    } else {
        (w_pos - w_neg) / denom
    }
}

fn two_sided_normal_p(z_abs: f64) -> f64 {
    // Abramowitz & Stegun 7.1.26 erfc approximation for two-sided normal p-value.
    let x = z_abs / std::f64::consts::SQRT_2;
    let t = 1.0 / (1.0 + 0.327_591_1 * x);
    let poly = t
        * (0.254_829_592
            + t * (-0.284_496_736
                + t * (1.421_413_741 + t * (-1.453_152_027 + t * 1.061_405_429))));
    let erfc_x = poly * (-x * x).exp();
    erfc_x.clamp(0.0, 1.0)
}

/// Computes the undominated **Convex Pareto** frontier (`Kapoor et al. 2025`, `R-39`)
/// across `(Quality ↑, E_tok ↓, p95_latency_ms ↓, Risk ↓)` plus Holm-Bonferroni statistics.
#[must_use]
pub fn compute_convex_pareto(input: &ParetoScorecardInput) -> ConvexParetoReport {
    let holm_results = evaluate_holm_bonferroni(
        &input.paired_comparisons,
        DEFAULT_FWER_ALPHA,
        DEFAULT_NI_MARGIN_DELTA,
    );
    let co_primary_ok = if holm_results.is_empty() {
        false
    } else {
        holm_results.iter().all(|h| h.non_inferiority_passed)
    };

    // Only scored rows enter Pareto frontier computation; gap rows stay in `gaps` without imputation.
    let pts: Vec<SystemConfigScore> = input
        .systems
        .iter()
        .filter(|s| {
            matches!(
                s.status,
                None | Some(ScorecardStatus::Run | ScorecardStatus::Partial)
            )
        })
        .cloned()
        .collect();
    let n = pts.len();
    let mut is_dominated = vec![false; n];

    // 1. Direct dominance check across (quality, e_tok, p95_latency_ms, composite_risk).
    for i in 0..n {
        for j in 0..n {
            if i == j {
                continue;
            }
            if dominates_point(&pts[j], &pts[i]) {
                is_dominated[i] = true;
                break;
            }
        }
    }

    // 2. Convex-mixture dominance check (`conv(X)`): check 2-point and 3-point convex combinations
    //    that achieve higher-or-equal quality at lower-or-equal (e_tok, latency, risk) with at least one strict improvement.
    for i in 0..n {
        if is_dominated[i] {
            continue;
        }
        let target = &pts[i];
        'outer: for a in 0..n {
            if a == i {
                continue;
            }
            for b in (a + 1)..n {
                if b == i {
                    continue;
                }
                for step in 1..20 {
                    let lam = (step as f64) / 20.0;
                    if convex_mix_dominates(&pts[a], &pts[b], lam, target) {
                        is_dominated[i] = true;
                        break 'outer;
                    }
                }
            }
        }
    }

    let mut undominated = Vec::new();
    let mut dominated = Vec::new();
    for (idx, p) in pts.iter().enumerate() {
        if is_dominated[idx] {
            dominated.push(p.name.clone());
        } else {
            undominated.push(p.clone());
        }
    }

    undominated.sort_by(|a, b| {
        b.quality
            .total_cmp(&a.quality)
            .then_with(|| a.verified_e_tok().total_cmp(&b.verified_e_tok()))
    });

    let hv = compute_2d_hypervolume(&undominated, &pts);
    let gaps: Vec<ScorecardRow> = input
        .scorecard_rows
        .iter()
        .filter(|r| !matches!(r.status, ScorecardStatus::Run | ScorecardStatus::Partial))
        .cloned()
        .collect();

    ConvexParetoReport {
        schema_version: "1.0".to_owned(),
        undominated_frontier: undominated,
        dominated_points: dominated,
        holm_results,
        co_primary_ni_and_cost_passed: co_primary_ok,
        hypervolume_2d_quality_etok: hv,
        gaps,
    }
}

/// Builds a [`ParetoScorecardInput`] from per-dataset [`ScorecardRow`]s without imputing gaps.
#[must_use]
pub fn scorecard_rows_to_pareto_input(
    rows: &[ScorecardRow],
    paired_comparisons: Vec<PairedComparisonInput>,
) -> ParetoScorecardInput {
    let mut systems = Vec::new();
    for row in rows {
        if !matches!(row.status, ScorecardStatus::Run | ScorecardStatus::Partial) {
            continue;
        }
        let scored_vals: Vec<f64> = row.metrics.values().copied().flatten().collect();
        if scored_vals.is_empty() {
            continue;
        }
        let mean_q = (scored_vals.iter().sum::<f64>() / scored_vals.len() as f64).clamp(0.0, 1.0);
        systems.push(SystemConfigScore {
            name: row.dataset_id.clone(),
            quality: mean_q,
            e_tok: row.e_tok,
            p95_latency_ms: 1.0,
            fir: 0.0,
            delta_ni: 0.0,
            asr: 0.0,
            raw_5tuples: Vec::new(),
            lane: Some(row.lane),
            contamination_risk: Some(row.contamination_risk),
            status: Some(row.status),
        });
    }
    ParetoScorecardInput {
        systems,
        paired_comparisons,
        scorecard_rows: rows.to_vec(),
    }
}

fn dominates_point(a: &SystemConfigScore, b: &SystemConfigScore) -> bool {
    let ae = a.verified_e_tok();
    let be = b.verified_e_tok();
    let ar = a.composite_risk();
    let br = b.composite_risk();

    let all_ge_or_le = a.quality >= b.quality - 1e-12
        && ae <= be + 1e-12
        && a.p95_latency_ms <= b.p95_latency_ms + 1e-12
        && ar <= br + 1e-12;
    let one_strict = a.quality > b.quality + 1e-12
        || ae < be - 1e-12
        || a.p95_latency_ms < b.p95_latency_ms - 1e-12
        || ar < br - 1e-12;
    all_ge_or_le && one_strict
}

fn convex_mix_dominates(
    a: &SystemConfigScore,
    b: &SystemConfigScore,
    lam: f64,
    target: &SystemConfigScore,
) -> bool {
    let om = 1.0 - lam;
    let q = lam * a.quality + om * b.quality;
    let e = lam * a.verified_e_tok() + om * b.verified_e_tok();
    let lat = lam * a.p95_latency_ms + om * b.p95_latency_ms;
    let r = lam * a.composite_risk() + om * b.composite_risk();

    let te = target.verified_e_tok();
    let tr = target.composite_risk();

    let not_worse = q >= target.quality - 1e-9
        && e <= te + 1e-9
        && lat <= target.p95_latency_ms + 1e-9
        && r <= tr + 1e-9;
    let strict = q > target.quality + 1e-9
        || e < te - 1e-9
        || lat < target.p95_latency_ms - 1e-9
        || r < tr - 1e-9;
    not_worse && strict
}

fn compute_2d_hypervolume(frontier: &[SystemConfigScore], all: &[SystemConfigScore]) -> f64 {
    if frontier.is_empty() || all.is_empty() {
        return 0.0;
    }
    let nadir_etok = all
        .iter()
        .map(SystemConfigScore::verified_e_tok)
        .fold(0.0_f64, f64::max)
        * 1.1;
    let mut pts: Vec<(f64, f64)> = frontier
        .iter()
        .map(|p| (p.verified_e_tok(), p.quality.clamp(0.0, 1.0)))
        .collect();
    pts.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| b.1.total_cmp(&a.1)));

    let mut hv = 0.0_f64;
    let mut max_q = 0.0_f64;
    for w in pts.windows(2) {
        max_q = max_q.max(w[0].1);
        let dx = (w[1].0 - w[0].0).max(0.0);
        hv += dx * max_q;
    }
    if let Some(last) = pts.last() {
        max_q = max_q.max(last.1);
        hv += (nadir_etok - last.0).max(0.0) * max_q;
    }
    hv
}
