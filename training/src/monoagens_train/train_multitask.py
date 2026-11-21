"""Phase 2 & Phase 3 multi-task training (Kendall uncertainty weighting + PCGrad), calibration, and safetensors parity export."""

from __future__ import annotations

import argparse
import json
import time
from dataclasses import asdict, dataclass
from pathlib import Path
from typing import Any

import numpy as np
import torch
import torch.nn as nn
import torch.nn.functional as F
from torch.utils.data import DataLoader, TensorDataset

from monoagens_train.augment import build_procedural_timing_shards
from monoagens_train.calibrate import (
    compute_aurc,
    compute_brier,
    compute_ece,
    compute_macro_f1,
    compute_tc_ece,
    fit_cic_threshold,
    fit_temperature,
    probs_from_logits_k,
)
from monoagens_train.datasets import (
    load_multitask_shards,
    precompute_phase1_shards,
    precompute_phase2_shards,
    precompute_phase3_shards,
)
from monoagens_train.model import (
    CellKind,
    MonoNeuralController,
    ScalePreset,
    architecture_for_scale,
)
from monoagens_train.posttrain_preference import (
    Stage4PreferenceConfig,
    run_stage4_preference_posttrain,
)


@dataclass
class MultiTaskReportEntry:
    phase: int
    scale: str
    cell_type: str
    param_count: int
    train_seconds: float
    temperature: float
    cic_threshold: float
    nll_before_ts: float
    nll_after_ts: float
    gate_macro_f1: float
    gate_accuracy: float
    phase1_gate_macro_f1: float
    ece: float
    tc_ece: float
    brier: float
    aurc: float
    cic_coverage: float
    cic_selective_risk: float
    p_uncertainty_mae: float | None = None
    p_uncertainty_brier: float | None = None
    p_verify_bce: float | None = None
    p_verify_f1: float | None = None
    p_encode_importance_bce: float | None = None
    p_encode_importance_f1: float | None = None
    p_memory_relevance_bce: float | None = None
    p_memory_relevance_f1: float | None = None
    p_memory_relevance_top1_acc: float | None = None


def _set_seed(seed: int = 42) -> None:
    np.random.seed(seed)
    torch.manual_seed(seed)
    if torch.cuda.is_available():
        torch.cuda.manual_seed_all(seed)


def _merge_splits(shard_dicts: list[dict[str, dict[str, np.ndarray]]], split: str) -> dict[str, np.ndarray]:
    keys = (
        "e_256",
        "x_num",
        "y_gate",
        "y_wait_ms",
        "mask",
        "y_unc",
        "m_unc",
        "y_ver",
        "m_ver",
        "y_enc",
        "m_enc",
        "y_rel",
        "m_rel",
        "dataset_id",
    )
    merged: dict[str, np.ndarray] = {}
    for k in keys:
        parts = [sd[split][k] for sd in shard_dicts if len(sd[split]["e_256"]) > 0]
        merged[k] = np.concatenate(parts, axis=0)
    return merged


def _compute_class_weights(
    y: np.ndarray,
    mask: np.ndarray,
    device: torch.device,
    wait_boost: float = 1.4,
) -> torch.Tensor:
    active = y[mask > 0.5]
    counts = np.bincount(active, minlength=4).astype(np.float64)
    counts = np.maximum(counts, 1.0)
    inv_sqrt = np.sqrt(np.max(counts) / counts)
    inv_sqrt[2] *= wait_boost
    inv_sqrt = np.clip(inv_sqrt, 0.5, 4.5)
    return torch.tensor(inv_sqrt, dtype=torch.float32, device=device)


class HomoscedasticTaskWeights(nn.Module):
    """Learnable log-variance task weights (Kendall et al., 2018), clamped for stability."""

    def __init__(self, n_tasks: int) -> None:
        super().__init__()
        self.log_vars = nn.Parameter(torch.zeros(n_tasks, dtype=torch.float32))

    def weight(self, idx: int, loss: torch.Tensor) -> torch.Tensor:
        s = self.log_vars[idx].clamp(-3.5, 3.5)
        return torch.exp(-s) * loss + 0.5 * s


def _pcgrad_backward(
    task_losses: list[torch.Tensor],
    shared_params: list[nn.Parameter],
    head_params: list[nn.Parameter],
    rng: np.random.Generator,
) -> None:
    """Projects conflicting shared-parameter task gradients (Yu et al., 2020) while leaving heads exact."""
    active_losses = [l for l in task_losses if l.requires_grad]
    if not active_losses:
        return

    # 1. Head gradients (task-specific, zero interference) + shared gradients per task
    shared_grads_per_task: list[list[torch.Tensor]] = []
    head_grads_acc = [torch.zeros_like(p) for p in head_params]
    all_params = shared_params + head_params

    for idx, loss in enumerate(active_losses):
        retain = idx < len(active_losses) - 1
        grads = torch.autograd.grad(
            loss,
            all_params,
            retain_graph=retain,
            allow_unused=True,
        )
        s_grads: list[torch.Tensor] = []
        n_shared = len(shared_params)
        for p, g in zip(shared_params, grads[:n_shared]):
            s_grads.append(torch.zeros_like(p) if g is None else g.detach())
        for acc, g in zip(head_grads_acc, grads[n_shared:]):
            if g is not None:
                acc.add_(g.detach())
        shared_grads_per_task.append(s_grads)

    # Flatten each task's shared grad to project pairwise conflicts
    flat_tasks = [torch.cat([g.reshape(-1) for g in sg]) for sg in shared_grads_per_task]
    n_t = len(flat_tasks)
    projected: list[torch.Tensor] = []
    order = np.arange(n_t)
    for i in range(n_t):
        g_i = flat_tasks[i].clone()
        rng.shuffle(order)
        for j in order:
            if j == i:
                continue
            g_j = flat_tasks[j]
            dot_ij = torch.dot(g_i, g_j)
            if dot_ij < 0.0:
                denom = torch.dot(g_j, g_j).clamp_min(1e-12)
                g_i = g_i - (dot_ij / denom) * g_j
        projected.append(g_i)

    summed_shared = torch.stack(projected, dim=0).sum(dim=0)

    # Unflatten onto .grad
    offset = 0
    for p in shared_params:
        numel = p.numel()
        p.grad = summed_shared[offset : offset + numel].view_as(p).clone()
        offset += numel
    for p, hg in zip(head_params, head_grads_acc):
        p.grad = hg


@torch.no_grad()
def _collect_multitask_outputs(
    model: MonoNeuralController,
    split_dict: dict[str, np.ndarray],
    device: torch.device,
    batch_size: int = 128,
) -> dict[str, np.ndarray]:
    model.eval()
    e = torch.from_numpy(split_dict["e_256"])
    x = torch.from_numpy(split_dict["x_num"])

    buckets: dict[str, list[np.ndarray]] = {"logits_k": []}
    for opt_k in ("p_uncertainty", "p_verify", "p_encode_importance", "p_memory_relevance"):
        if (opt_k in ("p_uncertainty", "p_verify") and model.arch.has_phase2_heads) or (
            opt_k in ("p_encode_importance", "p_memory_relevance") and model.arch.has_phase3_heads
        ):
            buckets[opt_k] = []

    for start in range(0, len(e), batch_size):
        eb = e[start : start + batch_size].to(device)
        xb = x[start : start + batch_size].to(device)
        out = model.forward_sequence(eb, xb, temperature=1.0)
        for k in buckets:
            buckets[k].append(out[k].cpu().numpy())

    return {k: np.concatenate(v, axis=0) for k, v in buckets.items()}


def _binary_metrics(preds: np.ndarray, targets: np.ndarray, mask: np.ndarray) -> tuple[float, float]:
    active = mask > 0.5
    if not np.any(active):
        return 0.0, 0.0
    p = np.clip(preds[active].astype(np.float64), 1e-6, 1.0 - 1e-6)
    y = targets[active].astype(np.float64)
    bce = float(-np.mean(y * np.log(p) + (1.0 - y) * np.log(1.0 - p)))
    y_bin = y >= 0.5
    p_bin = p >= 0.5
    tp = int(np.sum(p_bin & y_bin))
    fp = int(np.sum(p_bin & (~y_bin)))
    fn = int(np.sum((~p_bin) & y_bin))
    denom = 2 * tp + fp + fn
    f1 = float((2 * tp) / denom) if denom > 0 else 0.0
    return bce, f1


def evaluate_multitask_model(
    model: MonoNeuralController,
    phase: int,
    scale: str,
    cell_type: str,
    param_count: int,
    train_seconds: float,
    shards: dict[str, dict[str, np.ndarray]],
    phase1_test_split: dict[str, np.ndarray],
    device: torch.device,
) -> MultiTaskReportEntry:
    """Calibrates temperature + non-vacuous CIC threshold and computes multi-task test metrics."""
    arch = model.arch
    cal_out = _collect_multitask_outputs(model, shards["calibration"], device)
    cal_mask = shards["calibration"]["mask"] > 0.5
    cal_logits_k = cal_out["logits_k"][cal_mask]
    cal_y = shards["calibration"]["y_gate"][cal_mask]

    temperature, nll_before, nll_after = fit_temperature(cal_logits_k, cal_y)
    cal_probs = probs_from_logits_k(cal_logits_k, temperature)
    cic_threshold, _cov, _emp, _up = fit_cic_threshold(
        cal_probs, cal_y, target_risk=0.05, delta=0.05, min_coverage=0.25
    )

    test_dict = shards["test"]
    test_out = _collect_multitask_outputs(model, test_dict, device)
    bsz_t, seq_t, k_t, _ = test_out["logits_k"].shape
    test_probs_seq = probs_from_logits_k(
        test_out["logits_k"].reshape(-1, k_t, 4), temperature
    ).reshape(bsz_t, seq_t, 4)

    test_active = test_dict["mask"] > 0.5
    test_probs = test_probs_seq[test_active]
    test_y = test_dict["y_gate"][test_active]
    preds = np.argmax(test_probs, axis=-1)
    conf = np.max(test_probs, axis=-1)
    accepted = conf >= cic_threshold
    cic_cov = float(np.mean(accepted)) if len(accepted) > 0 else 0.0
    cic_risk = float(np.mean(preds[accepted] != test_y[accepted])) if np.any(accepted) else 0.0

    p1_out = _collect_multitask_outputs(model, phase1_test_split, device)
    p1_bsz, p1_seq, p1_k, _ = p1_out["logits_k"].shape
    p1_probs = probs_from_logits_k(
        p1_out["logits_k"].reshape(-1, p1_k, 4), temperature
    ).reshape(p1_bsz, p1_seq, 4)[phase1_test_split["mask"] > 0.5]
    p1_y = phase1_test_split["y_gate"][phase1_test_split["mask"] > 0.5]
    p1_f1 = compute_macro_f1(p1_probs, p1_y)

    unc_mae = unc_brier = ver_bce = ver_f1 = enc_bce = enc_f1 = rel_bce = rel_f1 = rel_top1 = None
    if arch.has_phase2_heads and np.any(test_dict["m_unc"] > 0.5):
        mu = test_dict["m_unc"] > 0.5
        pu = test_out["p_uncertainty"][mu]
        yu = test_dict["y_unc"][mu]
        unc_mae = round(float(np.mean(np.abs(pu - yu))), 6)
        unc_brier = round(float(np.mean((pu - yu) ** 2)), 6)
    if arch.has_phase2_heads and np.any(test_dict["m_ver"] > 0.5):
        vb, vf = _binary_metrics(test_out["p_verify"], test_dict["y_ver"], test_dict["m_ver"])
        ver_bce, ver_f1 = round(vb, 6), round(vf, 6)
    if arch.has_phase3_heads and np.any(test_dict["m_enc"] > 0.5):
        eb, ef = _binary_metrics(
            test_out["p_encode_importance"], test_dict["y_enc"], test_dict["m_enc"]
        )
        enc_bce, enc_f1 = round(eb, 6), round(ef, 6)
    if arch.has_phase3_heads and np.any(test_dict["m_rel"] > 0.5):
        rb, rf = _binary_metrics(
            test_out["p_memory_relevance"], test_dict["y_rel"], test_dict["m_rel"]
        )
        rel_bce, rel_f1 = round(rb, 6), round(rf, 6)
        hits = []
        rel_pred = test_out["p_memory_relevance"]
        rel_true = test_dict["y_rel"]
        rel_mask = test_dict["m_rel"]
        for i in range(len(rel_true)):
            m_i = rel_mask[i] > 0.5
            if np.sum(m_i) < 2:
                continue
            ys = rel_true[i, m_i]
            ps = rel_pred[i, m_i]
            if np.max(ys) >= 0.9 and np.min(ys) <= 0.1:
                best_idx = int(np.argmax(ps))
                hits.append(1.0 if ys[best_idx] >= 0.5 else 0.0)
        rel_top1 = round(float(np.mean(hits)) if hits else 0.0, 6)

    return MultiTaskReportEntry(
        phase=phase,
        scale=scale,
        cell_type=cell_type,
        param_count=param_count,
        train_seconds=round(train_seconds, 2),
        temperature=round(temperature, 6),
        cic_threshold=round(cic_threshold, 6),
        nll_before_ts=round(nll_before, 6),
        nll_after_ts=round(nll_after, 6),
        gate_macro_f1=round(compute_macro_f1(test_probs, test_y), 6),
        gate_accuracy=round(float(np.mean(preds == test_y)), 6),
        phase1_gate_macro_f1=round(p1_f1, 6),
        ece=round(compute_ece(test_probs, test_y), 6),
        tc_ece=round(compute_tc_ece(test_probs_seq, test_dict["y_gate"], test_dict["mask"]), 6),
        brier=round(compute_brier(test_probs, test_y), 6),
        aurc=round(compute_aurc(test_probs, test_y), 6),
        cic_coverage=round(cic_cov, 6),
        cic_selective_risk=round(cic_risk, 6),
        p_uncertainty_mae=unc_mae,
        p_uncertainty_brier=unc_brier,
        p_verify_bce=ver_bce,
        p_verify_f1=ver_f1,
        p_encode_importance_bce=enc_bce,
        p_encode_importance_f1=enc_f1,
        p_memory_relevance_bce=rel_bce,
        p_memory_relevance_f1=rel_f1,
        p_memory_relevance_top1_acc=rel_top1,
    )


def train_multitask_model(
    phase: int,
    scale: ScalePreset,
    cell_type: CellKind,
    shards: dict[str, dict[str, np.ndarray]],
    phase1_test_split: dict[str, np.ndarray],
    device: torch.device,
    epochs: int = 12,
    batch_size: int = 64,
    lr: float = 1e-3,
    seed: int = 42,
    init_checkpoint: Path | None = None,
    use_pcgrad: bool = True,
    modality_dropout_p: float = 0.15,
) -> tuple[MonoNeuralController, MultiTaskReportEntry]:
    _set_seed(seed)
    arch = architecture_for_scale(
        scale,
        cell_type,
        has_phase2_heads=(phase >= 2),
        has_phase3_heads=(phase >= 3),
    )
    model = MonoNeuralController(arch).to(device)
    if init_checkpoint is not None and init_checkpoint.is_file():
        model.load_matching_weights(init_checkpoint)

    param_count = sum(p.numel() for p in model.parameters())
    task_weights = HomoscedasticTaskWeights(n_tasks=6).to(device)

    tr = shards["train"]
    train_ds = TensorDataset(
        torch.from_numpy(tr["e_256"]),
        torch.from_numpy(tr["x_num"]),
        torch.from_numpy(tr["y_gate"]),
        torch.from_numpy(tr["y_wait_ms"]),
        torch.from_numpy(tr["mask"]),
        torch.from_numpy(tr["y_unc"]),
        torch.from_numpy(tr["m_unc"]),
        torch.from_numpy(tr["y_ver"]),
        torch.from_numpy(tr["m_ver"]),
        torch.from_numpy(tr["y_enc"]),
        torch.from_numpy(tr["m_enc"]),
        torch.from_numpy(tr["y_rel"]),
        torch.from_numpy(tr["m_rel"]),
    )
    loader = DataLoader(train_ds, batch_size=batch_size, shuffle=True, drop_last=False)
    class_weights = _compute_class_weights(tr["y_gate"], tr["mask"], device)

    shared_params = [
        p
        for n, p in model.named_parameters()
        if not n.startswith("head_")
    ]
    head_params = [p for n, p in model.named_parameters() if n.startswith("head_")] + list(
        task_weights.parameters()
    )

    optimizer = torch.optim.AdamW(
        [
            {"params": shared_params, "lr": lr * 0.7 if init_checkpoint else lr},
            {"params": head_params, "lr": lr * 1.2},
        ],
        weight_decay=1e-2,
    )
    scheduler = torch.optim.lr_scheduler.CosineAnnealingLR(optimizer, T_max=max(1, epochs))
    rng = np.random.default_rng(seed)

    t0 = time.perf_counter()
    for _epoch in range(epochs):
        model.train()
        task_weights.train()
        for batch in loader:
            eb, xb, yb, wb, mb, y_u, m_u, y_v, m_v, y_e, m_e, y_r, m_r = [
                t.to(device) for t in batch
            ]
            if modality_dropout_p > 0.0:
                drop_mask = (torch.rand((eb.shape[0], 1, 1), device=device) >= modality_dropout_p).to(eb.dtype)
                eb = eb * drop_mask
            optimizer.zero_grad(set_to_none=True)
            out = model.forward_sequence(eb, xb, temperature=1.0)

            logits_k = out["logits_k"]
            bsz, seq_len, k_mem, _ = logits_k.shape
            ce_per_mem = F.cross_entropy(
                logits_k.reshape(-1, 4),
                yb.unsqueeze(2).expand(bsz, seq_len, k_mem).reshape(-1),
                weight=class_weights,
                reduction="none",
            ).reshape(bsz, seq_len, k_mem)
            cls_loss = (ce_per_mem.mean(dim=-1) * mb).sum() / mb.sum().clamp_min(1.0)

            wait_pred = out["wait_delta_ms"] / arch.max_wait_ms
            wait_true = (wb / arch.max_wait_ms).clamp(0.0, 1.0)
            wait_loss = (
                F.smooth_l1_loss(wait_pred, wait_true, reduction="none") * mb
            ).sum() / mb.sum().clamp_min(1.0)

            task_losses: list[torch.Tensor] = [
                task_weights.weight(0, cls_loss),
                task_weights.weight(1, wait_loss),
            ]

            if arch.has_phase2_heads and m_u.sum() > 0:
                # BCE against fused uncertainty probability (clamped strictly in (1e-5, 1-1e-5))
                p_u = out["p_uncertainty"].clamp(1e-5, 1.0 - 1e-5)
                bce_u = -(y_u * torch.log(p_u) + (1.0 - y_u) * torch.log(1.0 - p_u))
                loss_u = (bce_u * m_u).sum() / m_u.sum().clamp_min(1.0)
                task_losses.append(task_weights.weight(2, loss_u))

            if arch.has_phase2_heads and m_v.sum() > 0:
                bce_v = F.binary_cross_entropy_with_logits(
                    out["p_verify_logit"], y_v, reduction="none"
                )
                loss_v = (bce_v * m_v).sum() / m_v.sum().clamp_min(1.0)
                task_losses.append(task_weights.weight(3, loss_v))

            if arch.has_phase3_heads and m_e.sum() > 0:
                bce_e = F.binary_cross_entropy_with_logits(
                    out["p_encode_importance_logit"], y_e, reduction="none"
                )
                loss_e = (bce_e * m_e).sum() / m_e.sum().clamp_min(1.0)
                task_losses.append(task_weights.weight(4, loss_e))

            if arch.has_phase3_heads and m_r.sum() > 0:
                bce_r = F.binary_cross_entropy_with_logits(
                    out["p_memory_relevance_logit"], y_r, reduction="none"
                )
                loss_r = (bce_r * m_r).sum() / m_r.sum().clamp_min(1.0)
                task_losses.append(task_weights.weight(5, loss_r))

            if use_pcgrad and len(task_losses) > 1:
                _pcgrad_backward(task_losses, shared_params, head_params, rng)
            else:
                torch.stack(task_losses).sum().backward()

            torch.nn.utils.clip_grad_norm_(model.parameters(), max_norm=1.0)
            optimizer.step()
        scheduler.step()
    train_seconds = time.perf_counter() - t0
    entry = evaluate_multitask_model(
        model=model,
        phase=phase,
        scale=scale,
        cell_type=cell_type,
        param_count=param_count,
        train_seconds=train_seconds,
        shards=shards,
        phase1_test_split=phase1_test_split,
        device=device,
    )
    return model.cpu(), entry


def export_multitask_parity_fixture(
    phase: int,
    mingru_model: MonoNeuralController,
    gdn2_model: MonoNeuralController,
    mingru_entry: MultiTaskReportEntry,
    gdn2_entry: MultiTaskReportEntry,
    fixtures_dir: Path,
) -> dict[str, str]:
    """Exports trained compact safetensors + multi-turn JSON parity fixtures for Rust `neural_parity`."""
    fixtures_dir.mkdir(parents=True, exist_ok=True)
    rng = np.random.default_rng(2026 + phase)

    e_seq = rng.normal(0.0, 0.25, size=(3, 256)).astype(np.float32)
    e_seq[1, :] = 0.0  # vectorless step
    x_seq = rng.uniform(0.0, 1.0, size=(3, 18)).astype(np.float32)

    results: dict[str, Any] = {"phase": phase, "cases": []}
    blake3_map: dict[str, str] = {}

    for label, model, entry in (
        ("mingru", mingru_model, mingru_entry),
        ("gated_deltanet2", gdn2_model, gdn2_entry),
    ):
        st_name = (
            f"neural_phase{phase}_golden.safetensors"
            if label == "mingru"
            else f"neural_phase{phase}_golden_{label}.safetensors"
        )
        st_path = fixtures_dir / st_name
        digest = model.export_safetensors(
            st_path, temperature=entry.temperature, cic_threshold=entry.cic_threshold
        )
        blake3_map[label] = digest

        model_cpu = model.cpu().eval()
        h = torch.zeros(1, model_cpu.arch.h_dim, dtype=torch.float32)
        steps = []
        with torch.no_grad():
            for t in range(3):
                eb = torch.from_numpy(e_seq[t : t + 1])
                xb = torch.from_numpy(x_seq[t : t + 1])
                out = model_cpu.step(eb, xb, h, temperature=entry.temperature)
                h = out["h_next"]
                probs = out["gate_probs"][0].numpy()
                step_rec: dict[str, Any] = {
                    "e_256": [float(v) for v in e_seq[t]],
                    "x_num": [float(v) for v in x_seq[t]],
                    "expected_gate_logits": [float(v) for v in out["gate_logits"][0].numpy()],
                    "expected_gate_probs": [float(v) for v in probs],
                    "expected_wait_delta_ms": float(out["wait_delta_ms"][0].item()),
                    "expected_epistemic_uncertainty": float(
                        out["epistemic_uncertainty"][0].item()
                    ),
                    "expected_cic_abstain": bool(float(np.max(probs)) < entry.cic_threshold),
                    "expected_h_next": [float(v) for v in h[0].numpy()],
                    "expected_p_uncertainty": (
                        float(out["p_uncertainty"][0].item()) if "p_uncertainty" in out else None
                    ),
                    "expected_p_verify": (
                        float(out["p_verify"][0].item()) if "p_verify" in out else None
                    ),
                    "expected_p_encode_importance": (
                        float(out["p_encode_importance"][0].item())
                        if "p_encode_importance" in out
                        else None
                    ),
                    "expected_p_memory_relevance": (
                        float(out["p_memory_relevance"][0].item())
                        if "p_memory_relevance" in out
                        else None
                    ),
                }
                steps.append(step_rec)
        results["cases"].append(
            {
                "name": label,
                "safetensors_file": st_name,
                "blake3": digest,
                "temperature": entry.temperature,
                "cic_threshold": entry.cic_threshold,
                "has_phase2_heads": model_cpu.arch.has_phase2_heads,
                "has_phase3_heads": model_cpu.arch.has_phase3_heads,
                "steps": steps,
            }
        )

    json_path = fixtures_dir / f"neural_phase{phase}_golden.json"
    json_path.write_text(json.dumps(results, indent=2), encoding="utf-8")
    return blake3_map


def run_phase2_and_phase3(
    repo_root: Path,
    epochs_p2: int = 10,
    epochs_p3: int = 10,
    force_recompute_shards: bool = False,
    phase3_only: bool = False,
    stage4_preference: bool = False,
) -> tuple[dict[str, Any], dict[str, Any]]:
    cache_dir = repo_root / "training" / "cache"
    p1_cache = precompute_phase1_shards(
        repo_root,
        cache_dir / "phase1_shards.npz",
        force_recompute=force_recompute_shards,
        prompt_regime_id="t_d",
    )
    p2_cache = precompute_phase2_shards(
        repo_root,
        cache_dir / "phase2_shards.npz",
        force_recompute=force_recompute_shards,
        prompt_regime_id="t_d",
    )
    p3_cache = precompute_phase3_shards(
        repo_root,
        cache_dir / "phase3_shards.npz",
        force_recompute=force_recompute_shards,
        prompt_regime_id="t_d",
    )

    p1_shards = load_multitask_shards(p1_cache)
    p2_shards = load_multitask_shards(p2_cache)
    p3_shards = load_multitask_shards(p3_cache)

    device = torch.device("cuda" if torch.cuda.is_available() else "cpu")
    artifacts_dir = repo_root / "training" / "artifacts"
    fixtures_dir = repo_root / "crates" / "core" / "tests" / "fixtures"
    reports_dir = repo_root / "training" / "reports"
    reports_dir.mkdir(parents=True, exist_ok=True)

    # ---- PHASE 2: Joint Phase 1 + Phase 2 (`p_uncertainty`, `p_verify`) ----
    p2_best_path = artifacts_dir / "neural_phase2_best.safetensors"
    if phase3_only and p2_best_path.is_file() and (reports_dir / "phase2_report.json").is_file():
        p2_report = json.loads((reports_dir / "phase2_report.json").read_text(encoding="utf-8"))
    else:
        p12_merged = {sp: _merge_splits([p1_shards, p2_shards], sp) for sp in ("train", "calibration", "test")}
        p1_best = artifacts_dir / "neural_phase1_best.safetensors"

        p2_runs: list[tuple[MonoNeuralController, MultiTaskReportEntry]] = []
        # Train compact 150k minGRU & Gated DeltaNet-2 parity models + 1m/3m best model
        for sc, ct, warm in (
            ("150k", "mingru", None),
            ("150k", "gated_deltanet2", None),
            ("3m", "mingru", p1_best if p1_best.is_file() else None),
            ("1m", "gated_deltanet2", None),
        ):
            mdl, ent = train_multitask_model(
                phase=2,
                scale=sc,  # type: ignore[arg-type]
                cell_type=ct,  # type: ignore[arg-type]
                shards=p12_merged,
                phase1_test_split=p1_shards["test"],
                device=device,
                epochs=epochs_p2,
                init_checkpoint=warm,
                use_pcgrad=True,
            )
            p2_runs.append((mdl, ent))

        p2_best_mdl, p2_best_ent = max(
            p2_runs,
            key=lambda r: (
                r[1].phase1_gate_macro_f1
                + (r[1].p_verify_f1 or 0.0)
                - 0.5 * (r[1].p_uncertainty_brier or 0.0)
                - 0.5 * r[1].ece
            ),
        )
        p2_best_blake3 = p2_best_mdl.export_safetensors(
            p2_best_path, temperature=p2_best_ent.temperature, cic_threshold=p2_best_ent.cic_threshold
        )
        (artifacts_dir / "manifest_phase2.json").write_text(
            json.dumps(
                {
                    "artifact": str(p2_best_path.relative_to(repo_root)),
                    "blake3": p2_best_blake3,
                    "winner": asdict(p2_best_ent),
                },
                indent=2,
            ),
            encoding="utf-8",
        )

        p2_mg_mdl, p2_mg_ent = next(r for r in p2_runs if r[1].scale == "150k" and r[1].cell_type == "mingru")
        p2_gdn_mdl, p2_gdn_ent = next(
            r for r in p2_runs if r[1].scale == "150k" and r[1].cell_type == "gated_deltanet2"
        )
        p2_fix_hashes = export_multitask_parity_fixture(
            2, p2_mg_mdl, p2_gdn_mdl, p2_mg_ent, p2_gdn_ent, fixtures_dir
        )

        p2_report = {
            "device": torch.cuda.get_device_name(0) if torch.cuda.is_available() else "cpu",
            "datasets": ["when2speak", "ubuntu_irc", "proactive_bench", "capa", "longmemeval", "stale", "hi_tom", "dyntom"],
            "split_counts": {k: int(len(v["e_256"])) for k, v in p12_merged.items()},
            "winner": asdict(p2_best_ent),
            "best_artifact_blake3": p2_best_blake3,
            "fixture_blake3": p2_fix_hashes,
            "runs": [asdict(r[1]) for r in p2_runs],
        }
        (reports_dir / "phase2_report.json").write_text(json.dumps(p2_report, indent=2), encoding="utf-8")
        p2_md = [
            "# Phase 2 Multi-Task Report (`gate_4` + `wait_delta_t` + `p_uncertainty` + `p_verify`)",
            "",
            f"- **Device:** `{p2_report['device']}`",
            f"- **Split Sequences (P1+P2):** train={p2_report['split_counts']['train']}, cal={p2_report['split_counts']['calibration']}, test={p2_report['split_counts']['test']}",
            f"- **Winner:** `{p2_best_ent.scale}` + `{p2_best_ent.cell_type}` (`blake3={p2_best_blake3}`)",
            "",
            "| Scale | Cell | Params | T | CIC τ | P1 F1 | Gate F1 | ECE | TC-ECE | AURC | Unc MAE | Unc Brier | Verify F1 |",
            "|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|",
        ]
        for _m, e in p2_runs:
            p2_md.append(
                f"| {e.scale} | {e.cell_type} | {e.param_count:,} | {e.temperature:.3f} | {e.cic_threshold:.3f} | "
                f"{e.phase1_gate_macro_f1:.4f} | {e.gate_macro_f1:.4f} | {e.ece:.4f} | {e.tc_ece:.4f} | "
                f"{e.aurc:.4f} | {e.p_uncertainty_mae or 0.0:.4f} | {e.p_uncertainty_brier or 0.0:.4f} | {e.p_verify_f1 or 0.0:.4f} |"
            )
        (reports_dir / "phase2_report.md").write_text("\n".join(p2_md) + "\n", encoding="utf-8")

    # ---- PHASE 3: Joint Phase 1 + 2 + 3 (`p_encode_importance`, `p_memory_relevance`) ----
    seq_len = int(p1_shards["train"]["x_num"].shape[1])
    proc_timing_shards = build_procedural_timing_shards(seq_len=seq_len)
    p123_merged = {
        sp: _merge_splits([p1_shards, p2_shards, p3_shards, proc_timing_shards], sp)
        for sp in ("train", "calibration", "test")
    }
    p3_runs: list[tuple[MonoNeuralController, MultiTaskReportEntry]] = []
    for sc, ct, warm in (
        ("150k", "mingru", None),
        ("150k", "gated_deltanet2", None),
        ("3m", "mingru", p2_best_path),
        ("1m", "gated_deltanet2", None),
    ):
        mdl, ent = train_multitask_model(
            phase=3,
            scale=sc,  # type: ignore[arg-type]
            cell_type=ct,  # type: ignore[arg-type]
            shards=p123_merged,
            phase1_test_split=p1_shards["test"],
            device=device,
            epochs=epochs_p3,
            init_checkpoint=warm,
            use_pcgrad=True,
        )
        p3_runs.append((mdl, ent))

    p3_best_mdl, p3_best_ent = max(
        p3_runs,
        key=lambda r: (
            r[1].phase1_gate_macro_f1
            + (r[1].p_memory_relevance_f1 or 0.0)
            + (r[1].p_encode_importance_f1 or 0.0)
            + (r[1].p_verify_f1 or 0.0)
            - 0.5 * r[1].ece
        ),
    )

    stage4_diags: dict[str, dict[str, float]] = {}
    if stage4_preference:
        stage4_runs: list[tuple[MonoNeuralController, MultiTaskReportEntry]] = []
        s4_cfg = Stage4PreferenceConfig()
        for mdl, ent in p3_runs:
            s4_mdl, s4_ent, s4_diag = run_stage4_preference_posttrain(
                model=mdl,
                shards=p123_merged,
                phase1_test_split=p1_shards["test"],
                device=device,
                evaluate_fn=evaluate_multitask_model,
                scale=ent.scale,
                cell_type=ent.cell_type,
                phase=3,
                config=s4_cfg,
                repo_root=repo_root,
            )
            stage4_runs.append((s4_mdl, s4_ent))
            stage4_diags[f"{ent.scale}:{ent.cell_type}"] = s4_diag
        p3_runs = stage4_runs
        p3_best_mdl, p3_best_ent = max(
            p3_runs,
            key=lambda r: (
                r[1].phase1_gate_macro_f1
                + (r[1].p_memory_relevance_f1 or 0.0)
                + (r[1].p_encode_importance_f1 or 0.0)
                + (r[1].p_verify_f1 or 0.0)
                - 0.5 * r[1].ece
            ),
        )

    p3_best_path = artifacts_dir / "neural_phase3_best.safetensors"
    p3_best_blake3 = p3_best_mdl.export_safetensors(
        p3_best_path, temperature=p3_best_ent.temperature, cic_threshold=p3_best_ent.cic_threshold
    )
    (artifacts_dir / "manifest_phase3.json").write_text(
        json.dumps(
            {
                "artifact": str(p3_best_path.relative_to(repo_root)),
                "blake3": p3_best_blake3,
                "expected_embed_model_ver": p3_best_mdl.arch.expected_embed_model_ver,
                "expected_prompt_regime_id": p3_best_mdl.arch.expected_prompt_regime_id,
                "stage4_preference": stage4_preference,
                "winner": asdict(p3_best_ent),
            },
            indent=2,
        ),
        encoding="utf-8",
    )

    p3_mg_mdl, p3_mg_ent = next(r for r in p3_runs if r[1].scale == "150k" and r[1].cell_type == "mingru")
    p3_gdn_mdl, p3_gdn_ent = next(
        r for r in p3_runs if r[1].scale == "150k" and r[1].cell_type == "gated_deltanet2"
    )
    p3_fix_hashes = export_multitask_parity_fixture(
        3, p3_mg_mdl, p3_gdn_mdl, p3_mg_ent, p3_gdn_ent, fixtures_dir
    )

    p3_report = {
        "device": torch.cuda.get_device_name(0) if torch.cuda.is_available() else "cpu",
        "datasets": [
            "when2speak", "ubuntu_irc", "proactive_bench", "capa",
            "longmemeval", "stale", "hi_tom", "dyntom",
            "group_mem_bench", "memory_agent_bench",
        ],
        "expected_embed_model_ver": p3_best_mdl.arch.expected_embed_model_ver,
        "expected_prompt_regime_id": p3_best_mdl.arch.expected_prompt_regime_id,
        "stage4_preference": stage4_preference,
        "stage4_diagnostics": stage4_diags,
        "split_counts": {k: int(len(v["e_256"])) for k, v in p123_merged.items()},
        "winner": asdict(p3_best_ent),
        "best_artifact_blake3": p3_best_blake3,
        "fixture_blake3": p3_fix_hashes,
        "runs": [asdict(r[1]) for r in p3_runs],
    }
    (reports_dir / "phase3_report.json").write_text(json.dumps(p3_report, indent=2), encoding="utf-8")
    p3_md = [
        "# Phase 3 Full Multi-Task Report (Phase 1 + Phase 2 + `p_encode_importance` + `p_memory_relevance`)",
        "",
        f"- **Device:** `{p3_report['device']}`",
        f"- **Split Sequences (P1+P2+P3):** train={p3_report['split_counts']['train']}, cal={p3_report['split_counts']['calibration']}, test={p3_report['split_counts']['test']}",
        f"- **Winner:** `{p3_best_ent.scale}` + `{p3_best_ent.cell_type}` (`blake3={p3_best_blake3}`)",
        "",
        "| Scale | Cell | Params | T | P1 F1 | Gate F1 | ECE | TC-ECE | Unc MAE | Verify F1 | EncImp F1 | MemRel F1 | Rel Top-1 |",
        "|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|",
    ]
    for _m, e in p3_runs:
        p3_md.append(
            f"| {e.scale} | {e.cell_type} | {e.param_count:,} | {e.temperature:.3f} | "
            f"{e.phase1_gate_macro_f1:.4f} | {e.gate_macro_f1:.4f} | {e.ece:.4f} | {e.tc_ece:.4f} | "
            f"{e.p_uncertainty_mae or 0.0:.4f} | {e.p_verify_f1 or 0.0:.4f} | "
            f"{e.p_encode_importance_f1 or 0.0:.4f} | {e.p_memory_relevance_f1 or 0.0:.4f} | "
            f"{e.p_memory_relevance_top1_acc or 0.0:.4f} |"
        )
    (reports_dir / "phase3_report.md").write_text("\n".join(p3_md) + "\n", encoding="utf-8")

    return p2_report, p3_report


def main() -> None:
    parser = argparse.ArgumentParser(description="Train MonoAgens Phase 2 & Phase 3 neural heads.")
    parser.add_argument("--repo-root", type=Path, default=Path(__file__).resolve().parents[3])
    parser.add_argument("--epochs-p2", type=int, default=10)
    parser.add_argument("--epochs-p3", type=int, default=10)
    parser.add_argument("--force-recompute-shards", action="store_true")
    parser.add_argument("--phase3-only", action="store_true")
    parser.add_argument("--stage4-preference", action="store_true")
    args = parser.parse_args()

    p2_rep, p3_rep = run_phase2_and_phase3(
        repo_root=args.repo_root.resolve(),
        epochs_p2=args.epochs_p2,
        epochs_p3=args.epochs_p3,
        force_recompute_shards=args.force_recompute_shards,
        phase3_only=args.phase3_only,
        stage4_preference=args.stage4_preference,
    )
    print("PHASE 2 WINNER:", json.dumps(p2_rep["winner"], indent=2))
    print("PHASE 3 WINNER:", json.dumps(p3_rep["winner"], indent=2))


if __name__ == "__main__":
    main()
