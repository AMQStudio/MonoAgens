"""Post-hoc temperature scaling, finite-sample CIC risk thresholds, and calibration metrics."""

from __future__ import annotations

import math
from dataclasses import dataclass

import numpy as np
from scipy.optimize import minimize_scalar
from scipy.stats import beta as beta_dist


@dataclass(frozen=True)
class CalibrationResult:
    temperature: float
    cic_threshold: float
    nll_before: float
    nll_after: float
    cic_coverage: float
    cic_empirical_risk: float
    cic_upper_risk: float


def _softmax(logits: np.ndarray) -> np.ndarray:
    shifted = logits - np.max(logits, axis=-1, keepdims=True)
    exps = np.exp(shifted)
    return exps / np.sum(exps, axis=-1, keepdims=True)


def probs_from_logits_k(logits_k: np.ndarray, temperature: float) -> np.ndarray:
    """Computes ensemble-mean probabilities from [N, K, 4] member logits at `temperature`."""
    probs_k = _softmax(logits_k / float(temperature))
    return np.mean(probs_k, axis=1)


def compute_nll(probs: np.ndarray, labels: np.ndarray) -> float:
    eps = 1e-12
    idx = np.arange(len(labels))
    p_true = np.clip(probs[idx, labels], eps, 1.0)
    return float(-np.mean(np.log(p_true)))


def fit_temperature(logits_k_cal: np.ndarray, labels_cal: np.ndarray) -> tuple[float, float, float]:
    """Fits temperature `T > 0` on calibration split; guarantees NLL never degrades vs T=1.0."""
    p_before = probs_from_logits_k(logits_k_cal, 1.0)
    nll_before = compute_nll(p_before, labels_cal)

    def obj(t: float) -> float:
        return compute_nll(probs_from_logits_k(logits_k_cal, t), labels_cal)

    res = minimize_scalar(obj, bounds=(0.25, 5.0), method="bounded")
    t_opt = float(res.x)
    nll_after = float(res.fun)
    if not math.isfinite(nll_after) or nll_after > nll_before:
        return 1.0, nll_before, nll_before
    return t_opt, nll_before, nll_after


def clopper_pearson_upper(k_errors: int, n_total: int, delta: float = 0.05) -> float:
    """Exact finite-sample one-sided (1 - delta) Clopper-Pearson upper bound on error rate."""
    if n_total <= 0:
        return 1.0
    if k_errors >= n_total:
        return 1.0
    return float(beta_dist.ppf(1.0 - delta, k_errors + 1, n_total - k_errors))


def fit_cic_threshold(
    probs_cal: np.ndarray,
    labels_cal: np.ndarray,
    target_risk: float = 0.05,
    delta: float = 0.05,
    min_coverage: float = 0.20,
) -> tuple[float, float, float, float]:
    """Finds lowest CIC confidence threshold strictly above 4-class simplex floor (0.25)."""
    conf = np.max(probs_cal, axis=-1)
    preds = np.argmax(probs_cal, axis=-1)
    errors = (preds != labels_cal).astype(np.int64)
    n_all = len(labels_cal)

    best_tau = 0.5
    best_cov = 0.0
    best_emp = 1.0
    best_upper = 1.0

    candidates = np.linspace(0.30, 0.98, 69)
    found_feasible = False
    for tau in candidates:
        sel = conf >= tau
        n_sel = int(np.sum(sel))
        cov = float(n_sel / max(1, n_all))
        if n_sel < 20 or cov < min_coverage:
            continue
        k_err = int(np.sum(errors[sel]))
        emp_risk = float(k_err / n_sel)
        u_risk = clopper_pearson_upper(k_err, n_sel, delta=delta)
        if u_risk <= target_risk:
            best_tau = float(tau)
            best_cov = cov
            best_emp = emp_risk
            best_upper = u_risk
            found_feasible = True
            break

    if not found_feasible:
        # Pick threshold with lowest upper risk above min_coverage
        min_u = float("inf")
        for tau in candidates:
            sel = conf >= tau
            n_sel = int(np.sum(sel))
            cov = float(n_sel / max(1, n_all))
            if n_sel < 20 or cov < min_coverage:
                continue
            k_err = int(np.sum(errors[sel]))
            u_risk = clopper_pearson_upper(k_err, n_sel, delta=delta)
            if u_risk < min_u:
                min_u = u_risk
                best_tau = float(tau)
                best_cov = cov
                best_emp = float(k_err / n_sel)
                best_upper = u_risk

    return best_tau, best_cov, best_emp, best_upper


def compute_ece(probs: np.ndarray, labels: np.ndarray, n_bins: int = 15) -> float:
    conf = np.max(probs, axis=-1)
    acc = (np.argmax(probs, axis=-1) == labels).astype(np.float64)
    bins = np.linspace(0.0, 1.0, n_bins + 1)
    ece = 0.0
    n = len(labels)
    if n == 0:
        return 0.0
    for i in range(n_bins):
        lo, hi = bins[i], bins[i + 1]
        in_bin = (conf >= lo) & (conf <= hi if i == n_bins - 1 else conf < hi)
        cnt = int(np.sum(in_bin))
        if cnt > 0:
            ece += (cnt / n) * abs(float(np.mean(acc[in_bin])) - float(np.mean(conf[in_bin])))
    return float(ece)


def compute_tc_ece(
    probs_seq: np.ndarray,
    labels_seq: np.ndarray,
    mask_seq: np.ndarray,
    n_checkpoints: int = 4,
    n_bins: int = 10,
) -> float:
    """Multi-turn Trajectory-Checkpoint ECE (`arXiv:2609.07395`) weighted across turn-depth buckets."""
    bsz, seq_len, _ = probs_seq.shape
    t_edges = np.linspace(0, seq_len, n_checkpoints + 1, dtype=np.int64)
    total_weight = 0.0
    weighted_ece = 0.0
    for c in range(n_checkpoints):
        t0, t1 = int(t_edges[c]), int(t_edges[c + 1])
        if t1 <= t0:
            continue
        m_slice = mask_seq[:, t0:t1] > 0.5
        cnt = int(np.sum(m_slice))
        if cnt == 0:
            continue
        p_flat = probs_seq[:, t0:t1, :][m_slice]
        y_flat = labels_seq[:, t0:t1][m_slice]
        weighted_ece += cnt * compute_ece(p_flat, y_flat, n_bins=n_bins)
        total_weight += cnt
    return float(weighted_ece / max(1.0, total_weight))


def compute_brier(probs: np.ndarray, labels: np.ndarray) -> float:
    one_hot = np.eye(probs.shape[-1], dtype=np.float64)[labels]
    return float(np.mean(np.sum((probs - one_hot) ** 2, axis=-1)))


def compute_aurc(probs: np.ndarray, labels: np.ndarray) -> float:
    """Area Under Risk-Coverage curve (lower is better)."""
    n = len(labels)
    if n == 0:
        return 0.0
    conf = np.max(probs, axis=-1)
    err = (np.argmax(probs, axis=-1) != labels).astype(np.float64)
    order = np.argsort(-conf)
    sorted_err = err[order]
    cum_err = np.cumsum(sorted_err)
    k = np.arange(1, n + 1, dtype=np.float64)
    risks = cum_err / k
    coverages = k / n
    return float(np.trapezoid(risks, coverages))


def compute_macro_f1(probs: np.ndarray, labels: np.ndarray, num_classes: int = 4) -> float:
    preds = np.argmax(probs, axis=-1)
    f1s: list[float] = []
    for c in range(num_classes):
        tp = int(np.sum((preds == c) & (labels == c)))
        fp = int(np.sum((preds == c) & (labels != c)))
        fn = int(np.sum((preds != c) & (labels == c)))
        if tp + fp + fn == 0:
            continue
        denom = 2 * tp + fp + fn
        f1s.append(float((2 * tp) / denom) if denom > 0 else 0.0)
    return float(np.mean(f1s)) if f1s else 0.0
