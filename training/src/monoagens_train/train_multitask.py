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
    build_proactive_group_episodes,
    build_v22_sea_episodes,
    load_multitask_shards,
    precompute_multitask_shards,
    precompute_phase1_shards,
    precompute_phase2_shards,
    precompute_phase3_shards,
)
from monoagens_train.export import export_golden_fixture
from monoagens_train.features import (
    NUM_FEATURES_V2,
    NUM_FEATURES_V3,
    populate_v3_sequence_dynamics,
    upgrade_x_num_to_schema,
    upgrade_x_num_v1_to_v2,
)
from monoagens_train.info_gap_head import train_embeddinggemma_info_gap_head
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
    feature_schema_version: int = 1
    h_actor_dim: int = 0
    has_v22_heads: bool = False
    compute_tier_acc: float | None = None
    compute_tier_macro_f1: float | None = None
    p_user_correction_bce: float | None = None
    p_user_correction_f1: float | None = None


def _set_seed(seed: int = 42) -> None:
    np.random.seed(seed)
    torch.manual_seed(seed)
    if torch.cuda.is_available():
        torch.cuda.manual_seed_all(seed)


def _merge_splits(
    shard_dicts: list[dict[str, dict[str, np.ndarray]]],
    split: str,
    target_schema_version: int | None = None,
) -> dict[str, np.ndarray]:
    non_empty = [sd[split] for sd in shard_dicts if len(sd[split]["e_256"]) > 0]
    need_v3 = (
        target_schema_version == 3
        or any(d["x_num"].shape[-1] == NUM_FEATURES_V3 for d in non_empty)
    )
    need_v2 = (
        not need_v3
        and (
            target_schema_version == 2
            or any(d["x_num"].shape[-1] == NUM_FEATURES_V2 for d in non_empty)
        )
    )
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
        "y_compute_tier",
        "y_user_correction",
        "v22_mask",
        "dataset_id",
    )
    merged: dict[str, np.ndarray] = {}
    for k in keys:
        parts: list[np.ndarray] = []
        for d in non_empty:
            if k == "x_num":
                if need_v3:
                    parts.append(
                        d["x_num"]
                        if d["x_num"].shape[-1] == NUM_FEATURES_V3
                        else populate_v3_sequence_dynamics(d["x_num"], d["e_256"], d["y_gate"], d["mask"])
                    )
                elif need_v2:
                    parts.append(upgrade_x_num_v1_to_v2(d["x_num"]))
                else:
                    parts.append(d["x_num"])
            elif k in d:
                parts.append(d[k])
            elif k == "y_compute_tier":
                parts.append(np.ones_like(d["y_gate"], dtype=np.int64))
            else:
                parts.append(np.zeros_like(d["mask"], dtype=np.float32))
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
    x_raw = split_dict["x_num"]
    if x_raw.shape[-1] != model.arch.d_in_num:
        target_sv = (
            3
            if model.arch.d_in_num == NUM_FEATURES_V3
            else (2 if model.arch.d_in_num == NUM_FEATURES_V2 else 1)
        )
        x_raw = upgrade_x_num_to_schema(x_raw, target_sv)
    x = torch.from_numpy(x_raw)

    buckets: dict[str, list[np.ndarray]] = {"logits_k": []}
    for opt_k in (
        "p_uncertainty",
        "p_verify",
        "p_encode_importance",
        "p_memory_relevance",
        "compute_tier_probs",
        "p_user_correction",
    ):
        if (
            (opt_k in ("p_uncertainty", "p_verify") and model.arch.has_phase2_heads)
            or (opt_k in ("p_encode_importance", "p_memory_relevance") and model.arch.has_phase3_heads)
            or (opt_k in ("compute_tier_probs", "p_user_correction") and model.arch.has_v22_heads)
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

    ct_acc = ct_f1 = uc_bce = uc_f1 = None
    if arch.has_v22_heads and "v22_mask" in test_dict and np.any(test_dict["v22_mask"] > 0.5):
        mv = test_dict["v22_mask"] > 0.5
        ct_probs = test_out["compute_tier_probs"][mv]
        ct_true = test_dict["y_compute_tier"][mv]
        ct_acc = round(float(np.mean(np.argmax(ct_probs, axis=-1) == ct_true)), 6)
        ct_f1 = round(compute_macro_f1(ct_probs, ct_true, num_classes=3), 6)
        ucb, ucf = _binary_metrics(
            test_out["p_user_correction"],
            test_dict["y_user_correction"],
            test_dict["v22_mask"],
        )
        uc_bce, uc_f1 = round(ucb, 6), round(ucf, 6)

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
        feature_schema_version=arch.feature_schema_version,
        h_actor_dim=arch.h_actor_dim,
        has_v22_heads=arch.has_v22_heads,
        compute_tier_acc=ct_acc,
        compute_tier_macro_f1=ct_f1,
        p_user_correction_bce=uc_bce,
        p_user_correction_f1=uc_f1,
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
    feature_schema_version: int = 1,
    has_v22_heads: bool = False,
    has_v3_world_head: bool = False,
    h_actor_dim: int = 0,
) -> tuple[MonoNeuralController, MultiTaskReportEntry]:
    _set_seed(seed)
    arch = architecture_for_scale(
        scale,
        cell_type,
        has_phase2_heads=(phase >= 2),
        has_phase3_heads=(phase >= 3),
        has_v22_heads=has_v22_heads,
        has_v3_world_head=has_v3_world_head,
        feature_schema_version=feature_schema_version,
        h_actor_dim=h_actor_dim,
    )
    model = MonoNeuralController(arch).to(device)
    if init_checkpoint is not None and init_checkpoint.is_file():
        model.load_matching_weights(init_checkpoint)

    param_count = sum(p.numel() for p in model.parameters())
    task_weights = HomoscedasticTaskWeights(n_tasks=9).to(device)

    tr = shards["train"]
    x_tr = tr["x_num"]
    if x_tr.shape[-1] != arch.d_in_num:
        target_sv = (
            3
            if arch.d_in_num == NUM_FEATURES_V3
            else (2 if arch.d_in_num == NUM_FEATURES_V2 else 1)
        )
        x_tr = upgrade_x_num_to_schema(x_tr, target_sv)
    zeros_f = np.zeros_like(tr["mask"], dtype=np.float32)
    ones_i64 = np.ones_like(tr["y_gate"], dtype=np.int64)
    train_ds = TensorDataset(
        torch.from_numpy(tr["e_256"]),
        torch.from_numpy(x_tr),
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
        torch.from_numpy(tr.get("y_compute_tier", ones_i64)),
        torch.from_numpy(tr.get("y_user_correction", zeros_f)),
        torch.from_numpy(tr.get("v22_mask", zeros_f)),
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
    for epoch in range(epochs):
        model.train()
        task_weights.train()
        for batch in loader:
            eb, xb, yb, wb, mb, y_u, m_u, y_v, m_v, y_e, m_e, y_r, m_r, y_ct, y_uc, m_v22 = [
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

            timing_group = task_weights.weight(0, cls_loss) + task_weights.weight(1, wait_loss)
            aux_losses: list[torch.Tensor] = []

            if arch.has_phase2_heads and m_u.sum() > 0:
                # BCE against fused uncertainty probability (clamped strictly in (1e-5, 1-1e-5))
                p_u = out["p_uncertainty"].clamp(1e-5, 1.0 - 1e-5)
                bce_u = -(y_u * torch.log(p_u) + (1.0 - y_u) * torch.log(1.0 - p_u))
                loss_u = (bce_u * m_u).sum() / m_u.sum().clamp_min(1.0)
                aux_losses.append(task_weights.weight(2, loss_u))

            if arch.has_phase2_heads and m_v.sum() > 0:
                bce_v = F.binary_cross_entropy_with_logits(
                    out["p_verify_logit"], y_v, reduction="none"
                )
                loss_v = (bce_v * m_v).sum() / m_v.sum().clamp_min(1.0)
                aux_losses.append(task_weights.weight(3, loss_v))

            if arch.has_phase3_heads and m_e.sum() > 0:
                bce_e = F.binary_cross_entropy_with_logits(
                    out["p_encode_importance_logit"], y_e, reduction="none"
                )
                loss_e = (bce_e * m_e).sum() / m_e.sum().clamp_min(1.0)
                aux_losses.append(task_weights.weight(4, loss_e))

            if arch.has_phase3_heads and m_r.sum() > 0:
                bce_r = F.binary_cross_entropy_with_logits(
                    out["p_memory_relevance_logit"], y_r, reduction="none"
                )
                loss_r = (bce_r * m_r).sum() / m_r.sum().clamp_min(1.0)
                aux_losses.append(task_weights.weight(5, loss_r))

            v22_losses: list[torch.Tensor] = []
            if arch.has_v22_heads and m_v22.sum() > 0:
                ce_ct = F.cross_entropy(
                    out["compute_tier_logits"].reshape(-1, 3),
                    y_ct.reshape(-1),
                    reduction="none",
                ).reshape_as(m_v22)
                loss_ct = (ce_ct * m_v22).sum() / m_v22.sum().clamp_min(1.0)
                v22_losses.append(task_weights.weight(6, loss_ct))

                bce_uc = F.binary_cross_entropy_with_logits(
                    out["p_user_correction_logit"], y_uc, reduction="none"
                )
                loss_uc = (bce_uc * m_v22).sum() / m_v22.sum().clamp_min(1.0)
                v22_losses.append(task_weights.weight(7, loss_uc))

            # Self-supervised predictive coding world-model loss over raw next-turn transitions
            if arch.has_v3_world_head and "world_pred_logits" in out and seq_len > 1:
                m_trans = mb[:, :-1] * mb[:, 1:]
                if m_trans.sum() > 0:
                    wp_logits = out["world_pred_logits"][:, :-1, :]  # [B, T-1, 3]
                    next_reply = (yb[:, 1:] >= 2).to(wp_logits.dtype)
                    next_gap = (
                        xb[:, 1:, 32] if xb.shape[-1] >= 36 else (yb[:, 1:] >= 2).to(wp_logits.dtype) * 0.8
                    )
                    next_shift = xb[:, 1:, 34] if xb.shape[-1] >= 36 else torch.full_like(next_reply, 0.5)
                    wp_targets = torch.stack([next_reply, next_gap, next_shift], dim=-1).clamp(0.0, 1.0)
                    bce_world = F.binary_cross_entropy_with_logits(
                        wp_logits, wp_targets, reduction="none"
                    ).mean(dim=-1)
                    loss_world = (bce_world * m_trans).sum() / m_trans.sum().clamp_min(1.0)
                    v22_losses.append(task_weights.weight(8, loss_world))

            # Group into <=3 orthogonal task families (Timing, Memory/Verify, V2.2 Chat)
            # so PCGrad still projects inter-family conflicts at ~3x higher throughput.
            task_losses: list[torch.Tensor] = [timing_group]
            if aux_losses:
                task_losses.append(torch.stack(aux_losses).sum())
            if v22_losses:
                task_losses.append(torch.stack(v22_losses).sum())

            if use_pcgrad and len(task_losses) > 1:
                _pcgrad_backward(task_losses, shared_params, head_params, rng)
            else:
                torch.stack(task_losses).sum().backward()

            torch.nn.utils.clip_grad_norm_(model.parameters(), max_norm=1.0)
            optimizer.step()
        scheduler.step()
        if epochs >= 4 and (epoch == 0 or (epoch + 1) % 2 == 0 or epoch + 1 == epochs):
            print(
                f"[train_multitask] {scale}/{cell_type} epoch {epoch + 1}/{epochs} ({time.perf_counter() - t0:.1f}s)",
                flush=True,
            )
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
    *,
    fixture_prefix: str | None = None,
) -> dict[str, str]:
    """Exports trained compact safetensors + multi-turn JSON parity fixtures for Rust `neural_parity`."""
    prefix = fixture_prefix or (
        "neural_v22_golden"
        if mingru_model.arch.feature_schema_version >= 2
        else f"neural_phase{phase}_golden"
    )
    return export_golden_fixture(
        cases=(
            ("mingru", mingru_model, mingru_entry.temperature, mingru_entry.cic_threshold),
            ("gated_deltanet2", gdn2_model, gdn2_entry.temperature, gdn2_entry.cic_threshold),
        ),
        fixtures_dir=fixtures_dir,
        fixture_prefix=prefix,
        phase=phase,
        n_steps=3,
        seed=2026 + phase,
    )


def run_phase2_and_phase3(
    repo_root: Path,
    epochs_p2: int = 10,
    epochs_p3: int = 10,
    force_recompute_shards: bool = False,
    phase3_only: bool = False,
    stage4_preference: bool = False,
    feature_schema_version: int = 1,
    has_v22_heads: bool = False,
    has_v3_world_head: bool = False,
    fit_info_gap_head: bool = False,
    h_actor_dim: int = 0,
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

    p1_shards = load_multitask_shards(p1_cache, target_schema_version=feature_schema_version)
    p2_shards = load_multitask_shards(p2_cache, target_schema_version=feature_schema_version)
    p3_shards = load_multitask_shards(p3_cache, target_schema_version=feature_schema_version)

    device = torch.device("cuda" if torch.cuda.is_available() else "cpu")
    artifacts_dir = repo_root / "training" / "artifacts"
    fixtures_dir = repo_root / "crates" / "core" / "tests" / "fixtures"
    reports_dir = repo_root / "training" / "reports"
    reports_dir.mkdir(parents=True, exist_ok=True)

    # ---- PHASE 2: Joint Phase 1 + Phase 2 (`p_uncertainty`, `p_verify`) ----
    p2_best_path = artifacts_dir / "neural_phase2_best.safetensors"
    if phase3_only and p2_best_path.is_file() and (reports_dir / "phase2_report.json").is_file():
        p2_report = json.loads((reports_dir / "phase2_report.json").read_text(encoding="utf-8"))
    elif phase3_only:
        p2_report = {"skipped": True, "feature_schema_version": feature_schema_version}
    else:
        p12_merged = {
            sp: _merge_splits([p1_shards, p2_shards], sp, target_schema_version=feature_schema_version)
            for sp in ("train", "calibration", "test")
        }
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
                feature_schema_version=feature_schema_version,
                has_v22_heads=False,
                h_actor_dim=h_actor_dim,
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

    # ---- PHASE 3: Joint Phase 1 + 2 + 3 (+ optional V2.2/V3 heads & h_actor) ----
    seq_len = int(p1_shards["train"]["x_num"].shape[1])
    proc_timing_shards = build_procedural_timing_shards(
        seq_len=seq_len,
        n_train=1280 if feature_schema_version >= 2 else 640,
        n_cal=160 if feature_schema_version >= 2 else 80,
        n_test=160 if feature_schema_version >= 2 else 80,
        schema_version=feature_schema_version,
    )
    shard_sources = [p1_shards, p2_shards, p3_shards, proc_timing_shards]
    if feature_schema_version >= 2:
        sea_eps = build_v22_sea_episodes(repo_root, max_seq_len=seq_len, schema_version=2)
        if sea_eps:
            sea_cache = precompute_multitask_shards(
                repo_root=repo_root,
                cache_path=cache_dir / "v22_sea_shards.npz",
                episodes=sea_eps,
                max_seq_len=seq_len,
                force_recompute=force_recompute_shards,
                prompt_regime_id="t_d",
            )
            shard_sources.append(load_multitask_shards(sea_cache, target_schema_version=feature_schema_version))
        pg_eps = build_proactive_group_episodes(max_seq_len=seq_len, schema_version=feature_schema_version)
        if pg_eps:
            pg_cache = precompute_multitask_shards(
                repo_root=repo_root,
                cache_path=cache_dir / "v22_proactive_group_shards.npz",
                episodes=pg_eps,
                max_seq_len=seq_len,
                force_recompute=True,
                prompt_regime_id="t_d",
            )
            shard_sources.append(load_multitask_shards(pg_cache, target_schema_version=feature_schema_version))

    p123_merged = {
        sp: _merge_splits(shard_sources, sp, target_schema_version=feature_schema_version)
        for sp in ("train", "calibration", "test")
    }
    p3_runs: list[tuple[MonoNeuralController, MultiTaskReportEntry]] = []
    for sc, ct, warm in (
        ("150k", "mingru", None),
        ("150k", "gated_deltanet2", None),
        ("3m", "mingru", p2_best_path if p2_best_path.is_file() else None),
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
            batch_size=128,
            init_checkpoint=warm,
            use_pcgrad=True,
            feature_schema_version=feature_schema_version,
            has_v22_heads=has_v22_heads,
            has_v3_world_head=has_v3_world_head,
            h_actor_dim=h_actor_dim,
        )
        print(
            f"[train_multitask] finished {sc}/{ct} (params={ent.param_count:,}, p1_f1={ent.phase1_gate_macro_f1:.4f}, gate_f1={ent.gate_macro_f1:.4f}, ece={ent.ece:.4f})",
            flush=True,
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

    info_gap_metrics: dict[str, float] | None = None
    if fit_info_gap_head or feature_schema_version >= 3:
        ig_res = train_embeddinggemma_info_gap_head(repo_root)
        info_gap_metrics = {
            "train_acc": round(ig_res.train_acc, 4),
            "val_acc": round(ig_res.val_acc, 4),
            "val_brier": round(ig_res.val_brier, 6),
        }
        p3_best_mdl.set_info_gap_head(ig_res.w_gap, ig_res.b_gap)

    p3_best_name = (
        "neural_v3_best.safetensors"
        if feature_schema_version == 3
        else ("neural_v22_best.safetensors" if feature_schema_version == 2 else "neural_phase3_best.safetensors")
    )
    p3_best_path = artifacts_dir / p3_best_name
    p3_best_blake3 = p3_best_mdl.export_safetensors(
        p3_best_path, temperature=p3_best_ent.temperature, cic_threshold=p3_best_ent.cic_threshold
    )
    manifest_name = (
        "manifest_v3.json"
        if feature_schema_version == 3
        else ("manifest_v22.json" if feature_schema_version == 2 else "manifest_phase3.json")
    )
    (artifacts_dir / manifest_name).write_text(
        json.dumps(
            {
                "artifact": str(p3_best_path.relative_to(repo_root)),
                "blake3": p3_best_blake3,
                "feature_schema_version": p3_best_mdl.arch.feature_schema_version,
                "has_v22_heads": p3_best_mdl.arch.has_v22_heads,
                "has_info_gap_head": p3_best_mdl.arch.has_info_gap_head,
                "has_v3_world_head": p3_best_mdl.arch.has_v3_world_head,
                "info_gap_metrics": info_gap_metrics,
                "h_actor_dim": p3_best_mdl.arch.h_actor_dim,
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
        3,
        p3_mg_mdl,
        p3_gdn_mdl,
        p3_mg_ent,
        p3_gdn_ent,
        fixtures_dir,
        fixture_prefix="neural_v3_golden" if feature_schema_version == 3 else None,
    )

    p3_report = {
        "device": torch.cuda.get_device_name(0) if torch.cuda.is_available() else "cpu",
        "datasets": [
            "when2speak", "ubuntu_irc", "proactive_bench", "capa",
            "longmemeval", "stale", "hi_tom", "dyntom",
            "group_mem_bench", "memory_agent_bench",
        ],
        "feature_schema_version": feature_schema_version,
        "has_v22_heads": has_v22_heads,
        "h_actor_dim": h_actor_dim,
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
    report_stem = (
        "v3_report"
        if feature_schema_version == 3
        else ("v22_report" if feature_schema_version == 2 else "phase3_report")
    )
    (reports_dir / f"{report_stem}.json").write_text(json.dumps(p3_report, indent=2), encoding="utf-8")
    p3_md = [
        f"# Phase 3 Full Multi-Task Report (schema={feature_schema_version}, v22_heads={has_v22_heads}, h_actor_dim={h_actor_dim})",
        "",
        f"- **Device:** `{p3_report['device']}`",
        f"- **Split Sequences:** train={p3_report['split_counts']['train']}, cal={p3_report['split_counts']['calibration']}, test={p3_report['split_counts']['test']}",
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
    (reports_dir / f"{report_stem}.md").write_text("\n".join(p3_md) + "\n", encoding="utf-8")

    return p2_report, p3_report


def main() -> None:
    parser = argparse.ArgumentParser(description="Train MonoAgens Phase 2, Phase 3 & V2.2 neural heads.")
    parser.add_argument("--repo-root", type=Path, default=Path(__file__).resolve().parents[3])
    parser.add_argument("--epochs-p2", type=int, default=10)
    parser.add_argument("--epochs-p3", type=int, default=10)
    parser.add_argument("--force-recompute-shards", action="store_true")
    parser.add_argument("--phase3-only", action="store_true")
    parser.add_argument("--stage4-preference", action="store_true")
    parser.add_argument("--feature-schema-version", type=int, choices=(1, 2, 3), default=1)
    parser.add_argument("--has-v22-heads", action="store_true")
    parser.add_argument("--has-v3-world-head", action="store_true")
    parser.add_argument("--fit-info-gap-head", action="store_true")
    parser.add_argument("--h-actor-dim", type=int, default=0)
    args = parser.parse_args()

    p2_rep, p3_rep = run_phase2_and_phase3(
        repo_root=args.repo_root.resolve(),
        epochs_p2=args.epochs_p2,
        epochs_p3=args.epochs_p3,
        force_recompute_shards=args.force_recompute_shards,
        phase3_only=args.phase3_only,
        stage4_preference=args.stage4_preference,
        feature_schema_version=args.feature_schema_version,
        has_v22_heads=args.has_v22_heads,
        has_v3_world_head=args.has_v3_world_head,
        fit_info_gap_head=args.fit_info_gap_head,
        h_actor_dim=args.h_actor_dim,
    )
    print("PHASE 2 WINNER:", json.dumps(p2_rep.get("winner", p2_rep), indent=2))
    print("PHASE 3 WINNER:", json.dumps(p3_rep["winner"], indent=2))


if __name__ == "__main__":
    main()
