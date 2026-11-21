"""Stage 4 offline preference & counterfactual post-training (E-70.1):
- Closed-form categorical pairwise DPO + Margin weighted by TabM JSD (T-DUR)
- Asymmetric unpaired KTO (lambda_u > lambda_d)
- Counterfactual vectorless invariance (CFPO) between (e_256, x_num) and (0_256, x_num)
Adds +0 runtime parameters and preserves calibrated multi-task heads.
"""

from __future__ import annotations

import copy
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Callable

import numpy as np
import torch
import torch.nn.functional as F
from torch.utils.data import DataLoader, TensorDataset

from monoagens_train.datasets import GATE_GO, GATE_NOTE, GATE_SILENT, GATE_WAIT
from monoagens_train.licenses import assert_safe_path
from monoagens_train.model import LN_4, MonoNeuralController


@dataclass(frozen=True)
class Stage4PreferenceConfig:
    epochs: int = 4
    batch_size: int = 64
    lr: float = 2e-4
    weight_decay: float = 1e-4
    beta: float = 0.2
    margin: float = 0.25
    lambda_d: float = 1.0
    lambda_u: float = 2.5
    cfpo_weight: float = 0.6
    dpo_weight: float = 1.0
    kto_weight: float = 0.5
    kl_anchor_weight: float = 0.25
    aux_retention_weight: float = 0.5
    tdur_gamma: float = 1.0
    temperature: float = 1.0
    seed: int = 42


def _masked_mean(values: torch.Tensor, mask: torch.Tensor) -> torch.Tensor:
    denom = mask.sum().clamp_min(1.0)
    return (values * mask).sum() / denom


def compute_tabm_jsd(logits_k: torch.Tensor, temperature: float = 1.0) -> torch.Tensor:
    """Computes per-position Jensen-Shannon divergence across TabM ensemble members `[..., K, 4]`."""
    t = max(float(temperature), 1e-3)
    log_p_k = F.log_softmax(logits_k / t, dim=-1)
    p_k = torch.exp(log_p_k)
    p_mean = p_k.mean(dim=-2, keepdim=True).clamp_min(1e-7)
    log_p_mean = torch.log(p_mean)
    kl_k = (p_k * (log_p_k - log_p_mean)).sum(dim=-1)
    return kl_k.mean(dim=-1).clamp(min=0.0, max=LN_4)


def ensemble_log_probs(logits_k: torch.Tensor, temperature: float = 1.0) -> tuple[torch.Tensor, torch.Tensor]:
    """Returns `(log_probs_mean, probs_mean)` over TabM members `[..., K, 4]`."""
    t = max(float(temperature), 1e-3)
    probs_k = F.softmax(logits_k / t, dim=-1)
    probs_mean = probs_k.mean(dim=-2).clamp_min(1e-7)
    probs_mean = probs_mean / probs_mean.sum(dim=-1, keepdim=True)
    return torch.log(probs_mean), probs_mean


def compute_categorical_dpo_margin_loss(
    policy_log_probs: torch.Tensor,
    ref_log_probs: torch.Tensor,
    pos_idx: torch.Tensor,
    neg_idx: torch.Tensor,
    valid_mask: torch.Tensor,
    weights: torch.Tensor | None = None,
    beta: float = 0.2,
    margin: float = 0.25,
) -> torch.Tensor:
    """Closed-form categorical DPO + Margin loss weighted by T-DUR confidence."""
    pos_gather = pos_idx.unsqueeze(-1)
    neg_gather = neg_idx.unsqueeze(-1)
    pi_pos = policy_log_probs.gather(-1, pos_gather).squeeze(-1)
    pi_neg = policy_log_probs.gather(-1, neg_gather).squeeze(-1)
    ref_pos = ref_log_probs.gather(-1, pos_gather).squeeze(-1)
    ref_neg = ref_log_probs.gather(-1, neg_gather).squeeze(-1)

    log_ratio = (pi_pos - ref_pos) - (pi_neg - ref_neg)
    loss_el = F.softplus(float(margin) - float(beta) * log_ratio)
    if weights is not None:
        loss_el = loss_el * weights
    return _masked_mean(loss_el, valid_mask)


def compute_asymmetric_kto_loss(
    policy_log_probs: torch.Tensor,
    ref_log_probs: torch.Tensor,
    policy_probs: torch.Tensor,
    desirable_idx: torch.Tensor,
    desirable_mask: torch.Tensor,
    undesirable_idx: torch.Tensor,
    undesirable_mask: torch.Tensor,
    beta: float = 0.2,
    lambda_d: float = 1.0,
    lambda_u: float = 2.5,
) -> torch.Tensor:
    """Asymmetric unpaired KTO objective with lambda_u > lambda_d for false-interrupt restraint."""
    if lambda_u <= lambda_d:
        raise ValueError(f"Expected lambda_u > lambda_d for asymmetric KTO, got {lambda_u} <= {lambda_d}")

    active = ((desirable_mask > 0.5) | (undesirable_mask > 0.5)).float()
    kl_token = (policy_probs * (policy_log_probs - ref_log_probs)).sum(dim=-1).clamp_min(0.0)
    kl_ref = _masked_mean(kl_token, active).detach()

    d_log_ratio = (
        policy_log_probs.gather(-1, desirable_idx.unsqueeze(-1)).squeeze(-1)
        - ref_log_probs.gather(-1, desirable_idx.unsqueeze(-1)).squeeze(-1)
    )
    u_log_ratio = (
        policy_log_probs.gather(-1, undesirable_idx.unsqueeze(-1)).squeeze(-1)
        - ref_log_probs.gather(-1, undesirable_idx.unsqueeze(-1)).squeeze(-1)
    )

    loss_d = float(lambda_d) * (1.0 - torch.sigmoid(float(beta) * d_log_ratio - kl_ref))
    loss_u = float(lambda_u) * (1.0 - torch.sigmoid(kl_ref - float(beta) * u_log_ratio))
    return _masked_mean(loss_d, desirable_mask) + _masked_mean(loss_u, undesirable_mask)


def compute_cfpo_invariance_loss(
    logits_full_k: torch.Tensor,
    logits_vec0_k: torch.Tensor,
    cfpo_mask: torch.Tensor,
    temperature: float = 1.0,
) -> torch.Tensor:
    """Counterfactual vectorless invariance loss: symmetric JSD((e_256, x_num) || (0_256, x_num))."""
    log_p_full, p_full = ensemble_log_probs(logits_full_k, temperature=temperature)
    log_p_vec0, p_vec0 = ensemble_log_probs(logits_vec0_k, temperature=temperature)
    p_mix = (0.5 * (p_full + p_vec0)).clamp_min(1e-7)
    log_p_mix = torch.log(p_mix)
    jsd = 0.5 * (
        (p_full * (log_p_full - log_p_mix)).sum(dim=-1)
        + (p_vec0 * (log_p_vec0 - log_p_mix)).sum(dim=-1)
    )
    return _masked_mean(jsd.clamp(min=0.0, max=LN_4), cfpo_mask)


def build_stage4_preference_targets(
    x_num: np.ndarray,
    y_gate: np.ndarray,
    mask: np.ndarray,
    dataset_ids: np.ndarray | None = None,
) -> dict[str, np.ndarray]:
    """Constructs preference pairs, KTO desirability targets, and CFPO timing masks from v1 or v2 shards."""
    bsz, seq_len, feat_dim = x_num.shape
    is_v2 = feat_dim >= 32
    active = mask > 0.5
    direct = (x_num[..., 0] > 0.5) | (x_num[..., 1] > 0.5) | (x_num[..., 3] > 0.5)

    # V2.2 regime cues (`dev-07` habituation, `dev-10` self-repair/correction, `dev-13` peer-agent)
    if is_v2:
        has_recent_soul = x_num[..., 18] > 0.5
        cos_rephrase = x_num[..., 20]
        is_reply_last_soul = x_num[..., 22] > 0.5
        soul_intervened = x_num[..., 23] > 0.5
        sender_in_top_thread = x_num[..., 26] > 0.5
        is_auto_peer = x_num[..., 28] > 0.5
        has_other_peers = x_num[..., 29] > 0.5
        consec_auto = x_num[..., 30]
        peer_margin = x_num[..., 31]

        is_peer_pingpong = is_auto_peer & (consec_auto >= (2.0 / 6.0 - 1e-4))
        is_peer_deferral = has_other_peers & (peer_margin > 0.65)
        is_peer_own_expertise = has_other_peers & (~is_auto_peer) & (peer_margin < 0.35)
        is_repair_correction = (
            has_recent_soul
            & (is_reply_last_soul | soul_intervened)
            & (cos_rephrase >= 0.55)
        )
        is_self_repair_wait = (
            (y_gate == GATE_WAIT)
            & ((x_num[..., 15] >= 0.35) | sender_in_top_thread)
        )
        is_habituation_silent = (
            has_recent_soul
            & (y_gate == GATE_SILENT)
            & (~is_reply_last_soul)
            & (cos_rephrase < 0.45)
        )
    else:
        is_peer_pingpong = np.zeros((bsz, seq_len), dtype=bool)
        is_peer_deferral = np.zeros((bsz, seq_len), dtype=bool)
        is_peer_own_expertise = np.zeros((bsz, seq_len), dtype=bool)
        is_repair_correction = np.zeros((bsz, seq_len), dtype=bool)
        is_self_repair_wait = np.zeros((bsz, seq_len), dtype=bool)
        is_habituation_silent = np.zeros((bsz, seq_len), dtype=bool)

    unaddressed = (~direct) | is_peer_pingpong | is_peer_deferral

    pos_1 = np.zeros((bsz, seq_len), dtype=np.int64)
    neg_1 = np.zeros((bsz, seq_len), dtype=np.int64)
    m_1 = np.zeros((bsz, seq_len), dtype=np.float32)

    pos_2 = np.zeros((bsz, seq_len), dtype=np.int64)
    neg_2_i = np.zeros((bsz, seq_len), dtype=np.int64)
    m_2 = np.zeros((bsz, seq_len), dtype=np.float32)

    kto_des_idx = np.clip(y_gate, 0, 3).astype(np.int64)
    kto_des_mask = active.astype(np.float32)
    kto_und_idx = np.full((bsz, seq_len), GATE_GO, dtype=np.int64)
    kto_und_mask = np.zeros((bsz, seq_len), dtype=np.float32)

    # Silent habituation (dev-07) / unaddressed chatter / peer ping-pong (dev-13): Silent > Wait > Go
    is_silent = active & (y_gate == GATE_SILENT)
    pos_1[is_silent] = GATE_SILENT
    neg_1[is_silent] = GATE_WAIT
    m_1[is_silent] = np.where(
        is_habituation_silent[is_silent] | is_peer_pingpong[is_silent], 1.25, 1.0
    )
    pos_2[is_silent] = GATE_SILENT
    neg_2_i[is_silent] = GATE_GO
    m_2[is_silent] = np.where(
        is_habituation_silent[is_silent] | is_peer_pingpong[is_silent] | is_peer_deferral[is_silent],
        1.5,
        1.0,
    )
    kto_und_idx[is_silent] = GATE_GO
    kto_und_mask[is_silent] = 1.0

    # Open-floor loose mention / pecah balon / self-repair (dev-10) / peer deferral (dev-13): Wait > Silent > Go
    is_wait = active & (y_gate == GATE_WAIT)
    pos_1[is_wait] = GATE_WAIT
    neg_1[is_wait] = GATE_SILENT
    m_1[is_wait] = np.where(is_self_repair_wait[is_wait] | is_peer_deferral[is_wait], 1.25, 1.0)
    pos_2[is_wait] = GATE_WAIT
    neg_2_i[is_wait] = GATE_GO
    m_2[is_wait] = np.where(is_self_repair_wait[is_wait] | is_peer_deferral[is_wait], 1.5, 1.0)
    kto_und_idx[is_wait] = GATE_GO
    kto_und_mask[is_wait] = np.where(
        unaddressed[is_wait] | is_self_repair_wait[is_wait] | is_peer_deferral[is_wait],
        1.0,
        0.5,
    )

    # Direct dyadic / repair wake (dev-10) / own-expertise peer wake (dev-13): Go > Wait > Silent
    is_go = active & (y_gate == GATE_GO)
    pos_1[is_go] = GATE_GO
    neg_1[is_go] = GATE_WAIT
    m_1[is_go] = np.where(is_repair_correction[is_go] | is_peer_own_expertise[is_go], 1.25, 1.0)
    pos_2[is_go] = GATE_GO
    neg_2_i[is_go] = GATE_SILENT
    m_2[is_go] = np.where(
        direct[is_go] | is_repair_correction[is_go] | is_peer_own_expertise[is_go], 1.0, 0.7
    )
    kto_und_idx[is_go] = GATE_SILENT
    kto_und_mask[is_go] = np.where(
        direct[is_go] | is_repair_correction[is_go] | is_peer_own_expertise[is_go], 1.0, 0.4
    )

    # Internal note: Note > Silent, Note > Go
    is_note = active & (y_gate == GATE_NOTE)
    pos_1[is_note] = GATE_NOTE
    neg_1[is_note] = GATE_SILENT
    m_1[is_note] = 1.0
    pos_2[is_note] = GATE_NOTE
    neg_2_i[is_note] = GATE_GO
    m_2[is_note] = 1.0
    kto_und_idx[is_note] = GATE_GO
    kto_und_mask[is_note] = 1.0

    # Counterfactual vectorless invariance mask: timing-dominated habituation, peer-brake & procedural timing
    cfpo_mask = (
        active
        & (
            (unaddressed & ((y_gate == GATE_SILENT) | (y_gate == GATE_WAIT)))
            | is_peer_pingpong
            | is_peer_deferral
            | is_habituation_silent
        )
    ).astype(np.float32)
    if dataset_ids is not None:
        proc_ep = np.array([str(ds).startswith("procedural_timing") for ds in dataset_ids], dtype=bool)
        if len(proc_ep) == bsz:
            cfpo_mask[proc_ep] = np.where(active[proc_ep], 1.0, cfpo_mask[proc_ep])

    return {
        "pos_1": pos_1,
        "neg_1": neg_1,
        "m_1": m_1,
        "pos_2": pos_2,
        "neg_2": neg_2_i,
        "m_2": m_2,
        "kto_des_idx": kto_des_idx,
        "kto_des_mask": kto_des_mask,
        "kto_und_idx": kto_und_idx,
        "kto_und_mask": kto_und_mask,
        "cfpo_mask": cfpo_mask,
    }


@torch.no_grad()
def measure_cfpo_jsd(
    model: MonoNeuralController,
    split_dict: dict[str, np.ndarray],
    device: torch.device,
    temperature: float = 1.0,
    batch_size: int = 128,
) -> float:
    """Measures mean counterfactual vectorless JSD on active timing-dominated tokens."""
    model.eval()
    targets = build_stage4_preference_targets(
        split_dict["x_num"],
        split_dict["y_gate"],
        split_dict["mask"],
        split_dict.get("dataset_id"),
    )
    e = torch.from_numpy(split_dict["e_256"])
    x = torch.from_numpy(split_dict["x_num"])
    cf_mask = torch.from_numpy(targets["cfpo_mask"])
    if float(cf_mask.sum().item()) <= 0.0:
        return 0.0

    total_weighted = 0.0
    total_mass = 0.0
    for start in range(0, len(e), batch_size):
        eb = e[start : start + batch_size].to(device)
        xb = x[start : start + batch_size].to(device)
        mb = cf_mask[start : start + batch_size].to(device)
        if float(mb.sum().item()) <= 0.0:
            continue
        out_full = model.forward_sequence(eb, xb, temperature=1.0)
        out_zero = model.forward_sequence(torch.zeros_like(eb), xb, temperature=1.0)
        loss_val = compute_cfpo_invariance_loss(
            out_full["logits_k"], out_zero["logits_k"], mb, temperature=temperature
        )
        m_sum = float(mb.sum().item())
        total_weighted += float(loss_val.item()) * m_sum
        total_mass += m_sum
    return float(total_weighted / max(1.0, total_mass))


def run_stage4_preference_posttrain(
    model: MonoNeuralController,
    shards: dict[str, dict[str, np.ndarray]],
    phase1_test_split: dict[str, np.ndarray],
    device: torch.device,
    evaluate_fn: Callable[..., Any],
    scale: str,
    cell_type: str,
    phase: int = 3,
    config: Stage4PreferenceConfig | None = None,
    repo_root: Path | None = None,
) -> tuple[MonoNeuralController, Any, dict[str, float]]:
    """Fine-tunes a trained Phase 3 model with Stage 4 DPO/Margin (T-DUR) + KTO + CFPO."""
    if repo_root is not None:
        assert_safe_path(repo_root, repo_root=repo_root)

    cfg = config or Stage4PreferenceConfig()
    torch.manual_seed(cfg.seed)
    np.random.seed(cfg.seed)

    model = copy.deepcopy(model).to(device)
    ref_model = copy.deepcopy(model).to(device).eval()
    for p in ref_model.parameters():
        p.requires_grad_(False)

    param_count = sum(p.numel() for p in model.parameters())
    tr = shards["train"]
    targets = build_stage4_preference_targets(
        tr["x_num"],
        tr["y_gate"],
        tr["mask"],
        tr.get("dataset_id"),
    )

    cfpo_jsd_before = measure_cfpo_jsd(
        model, shards["test"], device=device, temperature=cfg.temperature
    )

    has_aux = all(k in tr for k in ("y_unc", "m_unc", "y_ver", "m_ver", "y_enc", "m_enc", "y_rel", "m_rel"))
    has_v22 = all(k in tr for k in ("y_compute_tier", "y_user_correction", "v22_mask"))
    zeros_aux = np.zeros_like(tr["mask"], dtype=np.float32)
    ones_ct = np.ones_like(tr["y_gate"], dtype=np.int64)

    train_ds = TensorDataset(
        torch.from_numpy(tr["e_256"]),
        torch.from_numpy(tr["x_num"]),
        torch.from_numpy(tr["y_gate"]),
        torch.from_numpy(tr["mask"]),
        torch.from_numpy(targets["pos_1"]),
        torch.from_numpy(targets["neg_1"]),
        torch.from_numpy(targets["m_1"]),
        torch.from_numpy(targets["pos_2"]),
        torch.from_numpy(targets["neg_2"]),
        torch.from_numpy(targets["m_2"]),
        torch.from_numpy(targets["kto_des_idx"]),
        torch.from_numpy(targets["kto_des_mask"]),
        torch.from_numpy(targets["kto_und_idx"]),
        torch.from_numpy(targets["kto_und_mask"]),
        torch.from_numpy(targets["cfpo_mask"]),
        torch.from_numpy(tr["y_unc"] if has_aux else zeros_aux),
        torch.from_numpy(tr["m_unc"] if has_aux else zeros_aux),
        torch.from_numpy(tr["y_ver"] if has_aux else zeros_aux),
        torch.from_numpy(tr["m_ver"] if has_aux else zeros_aux),
        torch.from_numpy(tr["y_enc"] if has_aux else zeros_aux),
        torch.from_numpy(tr["m_enc"] if has_aux else zeros_aux),
        torch.from_numpy(tr["y_rel"] if has_aux else zeros_aux),
        torch.from_numpy(tr["m_rel"] if has_aux else zeros_aux),
        torch.from_numpy(tr["y_compute_tier"] if has_v22 else ones_ct),
        torch.from_numpy(tr["y_user_correction"] if has_v22 else zeros_aux),
        torch.from_numpy(tr["v22_mask"] if has_v22 else zeros_aux),
    )
    train_loader = DataLoader(train_ds, batch_size=cfg.batch_size, shuffle=True)

    optimizer = torch.optim.AdamW(model.parameters(), lr=cfg.lr, weight_decay=cfg.weight_decay)
    scheduler = torch.optim.lr_scheduler.CosineAnnealingLR(optimizer, T_max=max(1, cfg.epochs))

    t0 = time.perf_counter()
    last_loss = 0.0
    for _epoch in range(cfg.epochs):
        model.train()
        for (
            eb,
            xb,
            yg,
            mb,
            p1,
            n1,
            m1,
            p2,
            n2,
            m2,
            k_des_i,
            k_des_m,
            k_und_i,
            k_und_m,
            cf_m,
            y_unc,
            m_unc,
            y_ver,
            m_ver,
            y_enc,
            m_enc,
            y_rel,
            m_rel,
            y_ct,
            y_uc,
            m_v22,
        ) in train_loader:
            eb = eb.to(device)
            xb = xb.to(device)
            yg = yg.to(device)
            mb = mb.to(device)
            p1, n1, m1 = p1.to(device), n1.to(device), m1.to(device)
            p2, n2, m2 = p2.to(device), n2.to(device), m2.to(device)
            k_des_i, k_des_m = k_des_i.to(device), k_des_m.to(device)
            k_und_i, k_und_m = k_und_i.to(device), k_und_m.to(device)
            cf_m = cf_m.to(device)
            y_unc, m_unc = y_unc.to(device), m_unc.to(device)
            y_ver, m_ver = y_ver.to(device), m_ver.to(device)
            y_enc, m_enc = y_enc.to(device), m_enc.to(device)
            y_rel, m_rel = y_rel.to(device), m_rel.to(device)
            y_ct, y_uc, m_v22 = y_ct.to(device), y_uc.to(device), m_v22.to(device)

            optimizer.zero_grad(set_to_none=True)
            out_pol = model.forward_sequence(eb, xb, temperature=1.0)
            with torch.no_grad():
                out_ref = ref_model.forward_sequence(eb, xb, temperature=1.0)

            logits_k = out_pol["logits_k"]
            ref_logits_k = out_ref["logits_k"]

            log_pi, pi = ensemble_log_probs(logits_k, temperature=cfg.temperature)
            log_ref, _ = ensemble_log_probs(ref_logits_k, temperature=cfg.temperature)

            # T-DUR weighting via TabM ensemble Jensen-Shannon disagreement
            tabm_jsd = compute_tabm_jsd(logits_k, temperature=cfg.temperature).detach()
            tdur_weights = 1.0 / (1.0 + float(cfg.tdur_gamma) * (tabm_jsd / LN_4))

            loss_dpo = 0.5 * (
                compute_categorical_dpo_margin_loss(
                    log_pi,
                    log_ref,
                    p1,
                    n1,
                    m1,
                    weights=tdur_weights,
                    beta=cfg.beta,
                    margin=cfg.margin,
                )
                + compute_categorical_dpo_margin_loss(
                    log_pi,
                    log_ref,
                    p2,
                    n2,
                    m2,
                    weights=tdur_weights,
                    beta=cfg.beta,
                    margin=cfg.margin,
                )
            )
            loss_kto = compute_asymmetric_kto_loss(
                log_pi,
                log_ref,
                pi,
                k_des_i,
                k_des_m,
                k_und_i,
                k_und_m,
                beta=cfg.beta,
                lambda_d=cfg.lambda_d,
                lambda_u=cfg.lambda_u,
            )
            out_vec0 = model.forward_sequence(torch.zeros_like(eb), xb, temperature=1.0)
            loss_cfpo = compute_cfpo_invariance_loss(
                logits_k, out_vec0["logits_k"], cf_m, temperature=cfg.temperature
            )

            # Reference KL + label-preservation anchor so auxiliary/gate heads remain sharp
            ref_probs = torch.exp(log_ref)
            kl_anchor = (ref_probs * (log_ref - log_pi)).sum(dim=-1)
            ce_anchor = F.nll_loss(
                log_pi.reshape(-1, 4), yg.reshape(-1), reduction="none"
            ).reshape_as(mb)
            loss_anchor = _masked_mean(kl_anchor + 0.25 * ce_anchor, mb)

            # Retain Phase 2, Phase 3 & V2.2 auxiliary head calibration while refining gate_4
            loss_aux = torch.zeros((), device=device)
            if has_aux and model.arch.has_phase2_heads:
                if float(m_unc.sum().item()) > 0.0:
                    pu = out_pol["p_uncertainty"].clamp(1e-5, 1.0 - 1e-5)
                    loss_aux = loss_aux + _masked_mean(F.smooth_l1_loss(pu, y_unc, reduction="none", beta=0.05), m_unc)
                if float(m_ver.sum().item()) > 0.0:
                    pv = out_pol["p_verify"].clamp(1e-5, 1.0 - 1e-5)
                    loss_aux = loss_aux + _masked_mean(F.binary_cross_entropy(pv, y_ver, reduction="none"), m_ver)
            if has_aux and model.arch.has_phase3_heads:
                if float(m_enc.sum().item()) > 0.0:
                    pe = out_pol["p_encode_importance"].clamp(1e-5, 1.0 - 1e-5)
                    loss_aux = loss_aux + _masked_mean(F.binary_cross_entropy(pe, y_enc, reduction="none"), m_enc)
                if float(m_rel.sum().item()) > 0.0:
                    pr = out_pol["p_memory_relevance"].clamp(1e-5, 1.0 - 1e-5)
                    loss_aux = loss_aux + _masked_mean(F.binary_cross_entropy(pr, y_rel, reduction="none"), m_rel)
            if has_v22 and model.arch.has_v22_heads and float(m_v22.sum().item()) > 0.0:
                ce_ct = F.cross_entropy(
                    out_pol["compute_tier_logits"].reshape(-1, 3),
                    y_ct.reshape(-1),
                    reduction="none",
                ).reshape_as(m_v22)
                bce_uc = F.binary_cross_entropy_with_logits(
                    out_pol["p_user_correction_logit"],
                    y_uc,
                    reduction="none",
                )
                loss_aux = loss_aux + _masked_mean(ce_ct + bce_uc, m_v22)

            total_loss = (
                cfg.dpo_weight * loss_dpo
                + cfg.kto_weight * loss_kto
                + cfg.cfpo_weight * loss_cfpo
                + cfg.kl_anchor_weight * loss_anchor
                + cfg.aux_retention_weight * loss_aux
            )
            total_loss.backward()
            torch.nn.utils.clip_grad_norm_(model.parameters(), max_norm=1.0)
            optimizer.step()
            last_loss = float(total_loss.item())
        scheduler.step()

    train_seconds = time.perf_counter() - t0
    cfpo_jsd_after = measure_cfpo_jsd(
        model, shards["test"], device=device, temperature=cfg.temperature
    )
    entry = evaluate_fn(
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
    diag = {
        "stage4_final_loss": round(last_loss, 6),
        "cfpo_jsd_before": round(cfpo_jsd_before, 6),
        "cfpo_jsd_after": round(cfpo_jsd_after, 6),
        "lambda_d": cfg.lambda_d,
        "lambda_u": cfg.lambda_u,
        "beta": cfg.beta,
        "margin": cfg.margin,
        "feature_schema_version": float(model.arch.feature_schema_version),
        "h_actor_dim": float(model.arch.h_actor_dim),
        "has_v22_heads": 1.0 if model.arch.has_v22_heads else 0.0,
    }
    return model.cpu(), entry, diag
