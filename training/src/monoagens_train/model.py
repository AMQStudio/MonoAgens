"""PyTorch `MonoNeuralController` matching `crates/core/src/neural/` 1-to-1."""

from __future__ import annotations

import math
from dataclasses import dataclass
from pathlib import Path
from typing import Literal

import blake3
import torch
import torch.nn as nn
import torch.nn.functional as F
from safetensors import safe_open
from safetensors.torch import save_file

CellKind = Literal["mingru", "gated_deltanet2"]
ScalePreset = Literal["150k", "500k", "1m", "3m"]

LN_4 = math.log(4.0)


@dataclass(frozen=True)
class ControllerArchitecture:
    d_in_emb: int = 256
    d_in_num: int = 18
    plr_n_frequencies: int = 8
    d_e: int = 64
    d_n: int = 32
    d_model: int = 96
    d_ff: int = 192
    n_layers: int = 2
    tabm_k: int = 8
    h_dim: int = 64
    h_actor_dim: int = 0
    cell_type: CellKind = "mingru"
    feature_schema_version: int = 1
    max_wait_ms: float = 30000.0
    has_phase2_heads: bool = False
    has_phase3_heads: bool = False
    has_v22_heads: bool = False
    expected_embed_model_ver: str = "egemma2-256"
    expected_prompt_regime_id: str = "t_d"


def architecture_for_scale(
    scale: ScalePreset,
    cell_type: CellKind,
    *,
    has_phase2_heads: bool = False,
    has_phase3_heads: bool = False,
    has_v22_heads: bool = False,
    feature_schema_version: int = 1,
    d_in_num: int | None = None,
    h_actor_dim: int = 0,
) -> ControllerArchitecture:
    if feature_schema_version not in (1, 2):
        raise ValueError(f"Unsupported feature_schema_version: {feature_schema_version}")
    if h_actor_dim < 0:
        raise ValueError(f"h_actor_dim must be >= 0, got {h_actor_dim}")
    resolved_d_in_num = d_in_num if d_in_num is not None else (32 if feature_schema_version == 2 else 18)

    if scale == "150k":
        return ControllerArchitecture(
            d_in_num=resolved_d_in_num,
            plr_n_frequencies=8,
            d_e=64,
            d_n=32,
            d_model=96,
            d_ff=192,
            n_layers=2,
            tabm_k=8,
            h_dim=64,
            h_actor_dim=h_actor_dim,
            cell_type=cell_type,
            feature_schema_version=feature_schema_version,
            has_phase2_heads=has_phase2_heads,
            has_phase3_heads=has_phase3_heads,
            has_v22_heads=has_v22_heads,
        )
    if scale == "500k":
        return ControllerArchitecture(
            d_in_num=resolved_d_in_num,
            plr_n_frequencies=12,
            d_e=96,
            d_n=48,
            d_model=160,
            d_ff=384,
            n_layers=2,
            tabm_k=8,
            h_dim=64,
            h_actor_dim=h_actor_dim,
            cell_type=cell_type,
            feature_schema_version=feature_schema_version,
            has_phase2_heads=has_phase2_heads,
            has_phase3_heads=has_phase3_heads,
            has_v22_heads=has_v22_heads,
        )
    if scale == "1m":
        return ControllerArchitecture(
            d_in_num=resolved_d_in_num,
            plr_n_frequencies=16,
            d_e=128,
            d_n=64,
            d_model=224,
            d_ff=512,
            n_layers=3,
            tabm_k=8,
            h_dim=64,
            h_actor_dim=h_actor_dim,
            cell_type=cell_type,
            feature_schema_version=feature_schema_version,
            has_phase2_heads=has_phase2_heads,
            has_phase3_heads=has_phase3_heads,
            has_v22_heads=has_v22_heads,
        )
    if scale == "3m":
        return ControllerArchitecture(
            d_in_num=resolved_d_in_num,
            plr_n_frequencies=16,
            d_e=128,
            d_n=64,
            d_model=320,
            d_ff=768,
            n_layers=4,
            tabm_k=8,
            h_dim=64,
            h_actor_dim=h_actor_dim,
            cell_type=cell_type,
            feature_schema_version=feature_schema_version,
            has_phase2_heads=has_phase2_heads,
            has_phase3_heads=has_phase3_heads,
            has_v22_heads=has_v22_heads,
        )
    raise ValueError(f"Unknown scale preset: {scale}")


class SwiGluTabMBlock(nn.Module):
    """Pre-LN Residual SwiGLU block with TabM rank-1 ensemble adapters."""

    def __init__(self, d_model: int, d_ff: int, tabm_k: int) -> None:
        super().__init__()
        self.norm = nn.LayerNorm(d_model, eps=1e-5)
        self.w_gate = nn.Linear(d_model, d_ff, bias=False)
        self.w_up = nn.Linear(d_model, d_ff, bias=False)
        self.w_down = nn.Linear(d_ff, d_model, bias=False)
        self.tabm_r = nn.Parameter(torch.ones(tabm_k, d_model))
        self.tabm_s = nn.Parameter(torch.ones(tabm_k, d_model))
        nn.init.normal_(self.tabm_r, mean=1.0, std=0.05)
        nn.init.normal_(self.tabm_s, mean=1.0, std=0.05)

    def forward(self, x: torch.Tensor) -> torch.Tensor:
        # x: [B, K, D]
        u = self.norm(x) * self.tabm_r.unsqueeze(0)
        h = F.silu(self.w_gate(u)) * self.w_up(u)
        delta = self.w_down(h) * self.tabm_s.unsqueeze(0)
        return x + delta


class MonoNeuralController(nn.Module):
    """Unified Neural Cognitive Controller matching Rust `crates/core/src/neural/` (v1 and v2.2)."""

    def __init__(self, arch: ControllerArchitecture) -> None:
        super().__init__()
        self.arch = arch

        # Stem
        self.emb_linear = nn.Linear(arch.d_in_emb, arch.d_e)
        self.emb_norm = nn.LayerNorm(arch.d_e, eps=1e-5)
        self.plr_freqs = nn.Parameter(torch.randn(arch.d_in_num, arch.plr_n_frequencies) * 0.5)
        self.plr_linear = nn.Linear(arch.d_in_num * 2 * arch.plr_n_frequencies, arch.d_n)
        self.fusion = nn.Linear(arch.d_e + arch.d_n + arch.h_dim + arch.h_actor_dim, arch.d_model)

        # TabM + SwiGLU Trunk
        self.tabm_in_scale = nn.Parameter(torch.ones(arch.tabm_k, arch.d_model))
        nn.init.normal_(self.tabm_in_scale, mean=1.0, std=0.05)
        self.blocks = nn.ModuleList(
            [SwiGluTabMBlock(arch.d_model, arch.d_ff, arch.tabm_k) for _ in range(arch.n_layers)]
        )
        self.final_norm = nn.LayerNorm(arch.d_model, eps=1e-5)

        # Per-place recurrent working memory cell
        if arch.cell_type == "mingru":
            self.mem_w_z = nn.Linear(arch.d_model, arch.h_dim)
            self.mem_w_h = nn.Linear(arch.d_model, arch.h_dim)
        elif arch.cell_type == "gated_deltanet2":
            self.mem_w_erase = nn.Linear(arch.d_model, arch.h_dim)
            self.mem_w_write = nn.Linear(arch.d_model, arch.h_dim)
            self.mem_w_delta = nn.Linear(arch.d_model, arch.h_dim)
        else:
            raise ValueError(f"Unsupported cell_type: {arch.cell_type}")

        # Optional factorized per-actor recurrent working memory cell (V2.2)
        self.actor_mem_w_z: nn.Linear | None = None
        self.actor_mem_w_h: nn.Linear | None = None
        self.actor_mem_w_erase: nn.Linear | None = None
        self.actor_mem_w_write: nn.Linear | None = None
        self.actor_mem_w_delta: nn.Linear | None = None
        if arch.h_actor_dim > 0:
            if arch.cell_type == "mingru":
                self.actor_mem_w_z = nn.Linear(arch.d_model, arch.h_actor_dim)
                self.actor_mem_w_h = nn.Linear(arch.d_model, arch.h_actor_dim)
            else:
                self.actor_mem_w_erase = nn.Linear(arch.d_model, arch.h_actor_dim)
                self.actor_mem_w_write = nn.Linear(arch.d_model, arch.h_actor_dim)
                self.actor_mem_w_delta = nn.Linear(arch.d_model, arch.h_actor_dim)

        # Phase 1 Heads
        self.head_gate_4 = nn.Linear(arch.d_model, 4)
        self.head_wait_delta_t = nn.Linear(arch.d_model, 1)

        # Optional Phase 2 Heads
        if arch.has_phase2_heads:
            self.head_p_uncertainty = nn.Linear(arch.d_model, 1)
            self.head_p_verify = nn.Linear(arch.d_model, 1)
        else:
            self.head_p_uncertainty = None
            self.head_p_verify = None

        # Optional Phase 3 Heads
        if arch.has_phase3_heads:
            self.head_p_encode_importance = nn.Linear(arch.d_model, 1)
            self.head_p_memory_relevance = nn.Linear(arch.d_model, 1)
        else:
            self.head_p_encode_importance = None
            self.head_p_memory_relevance = None

        # Optional V2.2 Heads (`compute_tier` + `p_user_correction`)
        if arch.has_v22_heads:
            self.head_compute_tier = nn.Linear(arch.d_model, 3)
            self.head_p_user_correction = nn.Linear(arch.d_model, 1)
        else:
            self.head_compute_tier = None
            self.head_p_user_correction = None

    def step(
        self,
        e_256: torch.Tensor,
        x_num: torch.Tensor,
        h_prev: torch.Tensor,
        temperature: float = 1.0,
        h_actor_prev: torch.Tensor | None = None,
    ) -> dict[str, torch.Tensor]:
        """Single percept forward step matching Rust `forward_step` identically."""
        # e_256: [B, d_in_emb], x_num: [B, d_in_num], h_prev: [B, h_dim]
        e_proj = self.emb_norm(self.emb_linear(e_256))

        # PLR: [B, d_in_num, n_freq] -> cat([sin, cos], dim=-1) -> flatten -> Linear -> SiLU
        angles = (2.0 * math.pi) * x_num.unsqueeze(-1) * self.plr_freqs.unsqueeze(0)
        periodic = torch.cat([torch.sin(angles), torch.cos(angles)], dim=-1).flatten(1, 2)
        n_proj = F.silu(self.plr_linear(periodic))

        if self.arch.h_actor_dim > 0:
            if h_actor_prev is None:
                h_actor_prev = torch.zeros(
                    e_256.shape[0],
                    self.arch.h_actor_dim,
                    device=e_256.device,
                    dtype=e_256.dtype,
                )
            fused_in = torch.cat([e_proj, n_proj, h_prev, h_actor_prev], dim=-1)
        else:
            fused_in = torch.cat([e_proj, n_proj, h_prev], dim=-1)

        z0 = self.fusion(fused_in)  # [B, d_model]
        x_k = z0.unsqueeze(1) * self.tabm_in_scale.unsqueeze(0)  # [B, K, d_model]

        for block in self.blocks:
            x_k = block(x_k)

        x_k = self.final_norm(x_k)  # [B, K, d_model]
        h_mean = x_k.mean(dim=1)  # [B, d_model]

        if self.arch.cell_type == "mingru":
            z_t = torch.sigmoid(self.mem_w_z(h_mean))
            h_tilde = self.mem_w_h(h_mean)
            h_next = (1.0 - z_t) * h_prev + z_t * h_tilde
        else:
            b_t = torch.sigmoid(self.mem_w_erase(h_mean))
            w_t = torch.sigmoid(self.mem_w_write(h_mean))
            d_t = torch.tanh(self.mem_w_delta(h_mean))
            h_next = (1.0 - b_t) * h_prev + w_t * d_t

        h_actor_next: torch.Tensor | None = None
        if self.arch.h_actor_dim > 0 and h_actor_prev is not None:
            if self.arch.cell_type == "mingru":
                assert self.actor_mem_w_z is not None and self.actor_mem_w_h is not None
                az_t = torch.sigmoid(self.actor_mem_w_z(h_mean))
                ah_tilde = self.actor_mem_w_h(h_mean)
                h_actor_next = (1.0 - az_t) * h_actor_prev + az_t * ah_tilde
            else:
                assert (
                    self.actor_mem_w_erase is not None
                    and self.actor_mem_w_write is not None
                    and self.actor_mem_w_delta is not None
                )
                ab_t = torch.sigmoid(self.actor_mem_w_erase(h_mean))
                aw_t = torch.sigmoid(self.actor_mem_w_write(h_mean))
                ad_t = torch.tanh(self.actor_mem_w_delta(h_mean))
                h_actor_next = (1.0 - ab_t) * h_actor_prev + aw_t * ad_t

        logits_k = self.head_gate_4(x_k)  # [B, K, 4]
        gate_logits = logits_k.mean(dim=1)  # [B, 4]
        probs_k = F.softmax(logits_k / temperature, dim=-1)  # [B, K, 4]
        gate_probs = probs_k.mean(dim=1)  # [B, 4]

        if self.arch.tabm_k > 1:
            eps = 1e-12
            ent_ens = -(gate_probs * torch.log(gate_probs.clamp_min(eps))).sum(dim=-1)
            ent_mem = -(probs_k * torch.log(probs_k.clamp_min(eps))).sum(dim=-1).mean(dim=1)
            epistemic_uncertainty = ((ent_ens - ent_mem).clamp_min(0.0) / LN_4).clamp(0.0, 1.0)
        else:
            epistemic_uncertainty = torch.zeros(e_256.shape[0], device=e_256.device, dtype=e_256.dtype)

        wait_logit = self.head_wait_delta_t(h_mean).squeeze(-1)
        wait_delta_ms = (torch.sigmoid(wait_logit) * self.arch.max_wait_ms).clamp(0.0, self.arch.max_wait_ms)

        out: dict[str, torch.Tensor] = {
            "logits_k": logits_k,
            "gate_logits": gate_logits,
            "gate_probs": gate_probs,
            "epistemic_uncertainty": epistemic_uncertainty,
            "wait_delta_ms": wait_delta_ms,
            "h_next": h_next,
            "h_mean": h_mean,
        }
        if h_actor_next is not None:
            out["h_actor_next"] = h_actor_next

        if self.head_p_uncertainty is not None and self.head_p_verify is not None:
            p_unc_logit = self.head_p_uncertainty(h_mean).squeeze(-1)
            p_unc_raw = torch.sigmoid(p_unc_logit)
            out["p_uncertainty_logit"] = p_unc_logit
            out["p_uncertainty_raw"] = p_unc_raw
            out["p_uncertainty"] = (p_unc_raw + epistemic_uncertainty).clamp(0.0, 1.0)
            p_ver_logit = self.head_p_verify(h_mean).squeeze(-1)
            out["p_verify_logit"] = p_ver_logit
            out["p_verify"] = torch.sigmoid(p_ver_logit)

        if self.head_p_encode_importance is not None and self.head_p_memory_relevance is not None:
            p_enc_logit = self.head_p_encode_importance(h_mean).squeeze(-1)
            p_rel_logit = self.head_p_memory_relevance(h_mean).squeeze(-1)
            out["p_encode_importance_logit"] = p_enc_logit
            out["p_encode_importance"] = torch.sigmoid(p_enc_logit)
            out["p_memory_relevance_logit"] = p_rel_logit
            out["p_memory_relevance"] = torch.sigmoid(p_rel_logit)

        if self.head_compute_tier is not None and self.head_p_user_correction is not None:
            ct_logits = self.head_compute_tier(h_mean)  # [B, 3]
            ct_probs = F.softmax(ct_logits, dim=-1)  # [B, 3]
            uc_logit = self.head_p_user_correction(h_mean).squeeze(-1)  # [B]
            out["compute_tier_logits"] = ct_logits
            out["compute_tier_probs"] = ct_probs
            out["p_user_correction_logit"] = uc_logit
            out["p_user_correction"] = torch.sigmoid(uc_logit)

        return out

    def forward_sequence(
        self,
        e_seq: torch.Tensor,
        x_seq: torch.Tensor,
        temperature: float = 1.0,
        h_actor_init: torch.Tensor | None = None,
    ) -> dict[str, torch.Tensor]:
        """Unrolls over a batch of trajectories [B, T, D]."""
        bsz, seq_len, _ = e_seq.shape
        h = torch.zeros(bsz, self.arch.h_dim, device=e_seq.device, dtype=e_seq.dtype)
        h_actor: torch.Tensor | None = None
        if self.arch.h_actor_dim > 0:
            if h_actor_init is not None:
                h_actor = h_actor_init
            else:
                h_actor = torch.zeros(
                    bsz, self.arch.h_actor_dim, device=e_seq.device, dtype=e_seq.dtype
                )

        logits_k_all = []
        logits_all = []
        probs_all = []
        ep_all = []
        wait_all = []
        h_all = []
        h_actor_all = []
        opt_keys = (
            "p_uncertainty_logit",
            "p_uncertainty_raw",
            "p_uncertainty",
            "p_verify_logit",
            "p_verify",
            "p_encode_importance_logit",
            "p_encode_importance",
            "p_memory_relevance_logit",
            "p_memory_relevance",
            "compute_tier_logits",
            "compute_tier_probs",
            "p_user_correction_logit",
            "p_user_correction",
        )
        opt_acc: dict[str, list[torch.Tensor]] = {k: [] for k in opt_keys}

        for t in range(seq_len):
            out = self.step(
                e_seq[:, t, :],
                x_seq[:, t, :],
                h,
                temperature=temperature,
                h_actor_prev=h_actor,
            )
            h = out["h_next"]
            logits_k_all.append(out["logits_k"])
            logits_all.append(out["gate_logits"])
            probs_all.append(out["gate_probs"])
            ep_all.append(out["epistemic_uncertainty"])
            wait_all.append(out["wait_delta_ms"])
            h_all.append(h)
            if "h_actor_next" in out:
                h_actor = out["h_actor_next"]
                h_actor_all.append(h_actor)
            for k in opt_keys:
                if k in out:
                    opt_acc[k].append(out[k])

        res: dict[str, torch.Tensor] = {
            "logits_k": torch.stack(logits_k_all, dim=1),
            "gate_logits": torch.stack(logits_all, dim=1),
            "gate_probs": torch.stack(probs_all, dim=1),
            "epistemic_uncertainty": torch.stack(ep_all, dim=1),
            "wait_delta_ms": torch.stack(wait_all, dim=1),
            "h_seq": torch.stack(h_all, dim=1),
        }
        if h_actor_all:
            res["h_actor_seq"] = torch.stack(h_actor_all, dim=1)
        for k, vals in opt_acc.items():
            if vals:
                res[k] = torch.stack(vals, dim=1)
        return res

    def _canonical_param_map(self) -> dict[str, nn.Parameter | torch.Tensor]:
        canonical_to_param: dict[str, nn.Parameter | torch.Tensor] = {
            "stem.emb.linear.weight": self.emb_linear.weight,
            "stem.emb.linear.bias": self.emb_linear.bias,
            "stem.emb.norm.weight": self.emb_norm.weight,
            "stem.emb.norm.bias": self.emb_norm.bias,
            "stem.plr.freqs": self.plr_freqs,
            "stem.plr.linear.weight": self.plr_linear.weight,
            "stem.plr.linear.bias": self.plr_linear.bias,
            "stem.fusion.weight": self.fusion.weight,
            "stem.fusion.bias": self.fusion.bias,
            "trunk.tabm_in_scale": self.tabm_in_scale,
            "trunk.final_norm.weight": self.final_norm.weight,
            "trunk.final_norm.bias": self.final_norm.bias,
            "head.gate_4.weight": self.head_gate_4.weight,
            "head.gate_4.bias": self.head_gate_4.bias,
            "head.wait_delta_t.weight": self.head_wait_delta_t.weight,
            "head.wait_delta_t.bias": self.head_wait_delta_t.bias,
        }
        for idx, block in enumerate(self.blocks):
            canonical_to_param[f"trunk.blocks.{idx}.norm.weight"] = block.norm.weight
            canonical_to_param[f"trunk.blocks.{idx}.norm.bias"] = block.norm.bias
            canonical_to_param[f"trunk.blocks.{idx}.w_gate.weight"] = block.w_gate.weight
            canonical_to_param[f"trunk.blocks.{idx}.w_up.weight"] = block.w_up.weight
            canonical_to_param[f"trunk.blocks.{idx}.w_down.weight"] = block.w_down.weight
            canonical_to_param[f"trunk.blocks.{idx}.tabm_r"] = block.tabm_r
            canonical_to_param[f"trunk.blocks.{idx}.tabm_s"] = block.tabm_s

        if self.arch.cell_type == "mingru":
            canonical_to_param["memory.mingru.w_z.weight"] = self.mem_w_z.weight
            canonical_to_param["memory.mingru.w_z.bias"] = self.mem_w_z.bias
            canonical_to_param["memory.mingru.w_h.weight"] = self.mem_w_h.weight
            canonical_to_param["memory.mingru.w_h.bias"] = self.mem_w_h.bias
        else:
            canonical_to_param["memory.gdn2.w_erase.weight"] = self.mem_w_erase.weight
            canonical_to_param["memory.gdn2.w_erase.bias"] = self.mem_w_erase.bias
            canonical_to_param["memory.gdn2.w_write.weight"] = self.mem_w_write.weight
            canonical_to_param["memory.gdn2.w_write.bias"] = self.mem_w_write.bias
            canonical_to_param["memory.gdn2.w_delta.weight"] = self.mem_w_delta.weight
            canonical_to_param["memory.gdn2.w_delta.bias"] = self.mem_w_delta.bias

        if self.arch.h_actor_dim > 0:
            if self.arch.cell_type == "mingru":
                assert self.actor_mem_w_z is not None and self.actor_mem_w_h is not None
                canonical_to_param["actor_memory.mingru.w_z.weight"] = self.actor_mem_w_z.weight
                canonical_to_param["actor_memory.mingru.w_z.bias"] = self.actor_mem_w_z.bias
                canonical_to_param["actor_memory.mingru.w_h.weight"] = self.actor_mem_w_h.weight
                canonical_to_param["actor_memory.mingru.w_h.bias"] = self.actor_mem_w_h.bias
            else:
                assert (
                    self.actor_mem_w_erase is not None
                    and self.actor_mem_w_write is not None
                    and self.actor_mem_w_delta is not None
                )
                canonical_to_param["actor_memory.gdn2.w_erase.weight"] = self.actor_mem_w_erase.weight
                canonical_to_param["actor_memory.gdn2.w_erase.bias"] = self.actor_mem_w_erase.bias
                canonical_to_param["actor_memory.gdn2.w_write.weight"] = self.actor_mem_w_write.weight
                canonical_to_param["actor_memory.gdn2.w_write.bias"] = self.actor_mem_w_write.bias
                canonical_to_param["actor_memory.gdn2.w_delta.weight"] = self.actor_mem_w_delta.weight
                canonical_to_param["actor_memory.gdn2.w_delta.bias"] = self.actor_mem_w_delta.bias

        if self.head_p_uncertainty is not None and self.head_p_verify is not None:
            canonical_to_param["head.p_uncertainty.weight"] = self.head_p_uncertainty.weight
            canonical_to_param["head.p_uncertainty.bias"] = self.head_p_uncertainty.bias
            canonical_to_param["head.p_verify.weight"] = self.head_p_verify.weight
            canonical_to_param["head.p_verify.bias"] = self.head_p_verify.bias

        if self.head_p_encode_importance is not None and self.head_p_memory_relevance is not None:
            canonical_to_param["head.p_encode_importance.weight"] = (
                self.head_p_encode_importance.weight
            )
            canonical_to_param["head.p_encode_importance.bias"] = self.head_p_encode_importance.bias
            canonical_to_param["head.p_memory_relevance.weight"] = self.head_p_memory_relevance.weight
            canonical_to_param["head.p_memory_relevance.bias"] = self.head_p_memory_relevance.bias

        if self.head_compute_tier is not None and self.head_p_user_correction is not None:
            canonical_to_param["head.compute_tier.weight"] = self.head_compute_tier.weight
            canonical_to_param["head.compute_tier.bias"] = self.head_compute_tier.bias
            canonical_to_param["head.p_user_correction.weight"] = self.head_p_user_correction.weight
            canonical_to_param["head.p_user_correction.bias"] = self.head_p_user_correction.bias

        return canonical_to_param

    def canonical_state_dict(self) -> dict[str, torch.Tensor]:
        """Maps PyTorch parameters to canonical `crates/core/src/neural/mod.rs` tensor names."""
        def f32_cpu(t: torch.Tensor) -> torch.Tensor:
            return t.detach().cpu().to(torch.float32).contiguous()

        return {name: f32_cpu(param) for name, param in self._canonical_param_map().items()}

    def export_safetensors(
        self,
        output_path: Path,
        temperature: float = 1.0,
        cic_threshold: float = 0.5,
        expected_embed_model_ver: str | None = None,
        expected_prompt_regime_id: str | None = None,
    ) -> str:
        """Exports weights and canonical metadata to `safetensors` and returns the BLAKE3 hex digest."""
        arch = self.arch
        metadata = {
            "feature_schema_version": str(arch.feature_schema_version),
            "d_in_emb": str(arch.d_in_emb),
            "d_in_num": str(arch.d_in_num),
            "plr_n_frequencies": str(arch.plr_n_frequencies),
            "d_e": str(arch.d_e),
            "d_n": str(arch.d_n),
            "d_model": str(arch.d_model),
            "d_ff": str(arch.d_ff),
            "n_layers": str(arch.n_layers),
            "tabm_k": str(arch.tabm_k),
            "h_dim": str(arch.h_dim),
            "h_actor_dim": str(arch.h_actor_dim),
            "cell_type": arch.cell_type,
            "temperature": f"{float(temperature):.8f}",
            "cic_threshold": f"{float(cic_threshold):.8f}",
            "max_wait_ms": f"{float(arch.max_wait_ms):.6f}",
            "has_phase2_heads": "true" if arch.has_phase2_heads else "false",
            "has_phase3_heads": "true" if arch.has_phase3_heads else "false",
            "has_v22_heads": "true" if arch.has_v22_heads else "false",
            "expected_embed_model_ver": expected_embed_model_ver or arch.expected_embed_model_ver,
            "expected_prompt_regime_id": expected_prompt_regime_id or arch.expected_prompt_regime_id,
        }
        tensors = self.canonical_state_dict()
        output_path.parent.mkdir(parents=True, exist_ok=True)
        save_file(tensors, str(output_path), metadata=metadata)
        digest = blake3.blake3(output_path.read_bytes()).hexdigest()
        return digest

    def load_matching_weights(self, safetensors_path: Path) -> tuple[int, dict[str, str]]:
        """Loads matching canonical tensors from a prior-phase safetensors checkpoint."""
        canonical_to_param = self._canonical_param_map()
        loaded = 0
        with safe_open(str(safetensors_path), framework="pt", device="cpu") as f:
            meta = dict(f.metadata() or {})
            available = set(f.keys())
            with torch.no_grad():
                for cname, param in canonical_to_param.items():
                    if cname in available:
                        src = f.get_tensor(cname)
                        if tuple(src.shape) == tuple(param.shape):
                            param.copy_(src.to(dtype=param.dtype, device=param.device))
                            loaded += 1
        return loaded, meta

    @classmethod
    def from_safetensors(cls, safetensors_path: Path) -> tuple[MonoNeuralController, dict[str, str]]:
        """Reconstructs a `MonoNeuralController` (v1 or v2.2) directly from a safetensors artifact."""
        with safe_open(str(safetensors_path), framework="pt", device="cpu") as f:
            meta = dict(f.metadata() or {})
            freqs_shape = tuple(f.get_tensor("stem.plr.freqs").shape)
            gate0_shape = tuple(f.get_tensor("trunk.blocks.0.w_gate.weight").shape)

        arch = ControllerArchitecture(
            d_in_emb=int(meta["d_in_emb"]),
            d_in_num=int(meta["d_in_num"]),
            plr_n_frequencies=int(meta.get("plr_n_frequencies", freqs_shape[1])),
            d_e=int(meta["d_e"]),
            d_n=int(meta["d_n"]),
            d_model=int(meta["d_model"]),
            d_ff=int(meta.get("d_ff", gate0_shape[0])),
            n_layers=int(meta["n_layers"]),
            tabm_k=int(meta["tabm_k"]),
            h_dim=int(meta["h_dim"]),
            h_actor_dim=int(meta.get("h_actor_dim", "0")),
            cell_type=meta["cell_type"],  # type: ignore[arg-type]
            feature_schema_version=int(meta.get("feature_schema_version", "1")),
            max_wait_ms=float(meta.get("max_wait_ms", "30000.0")),
            has_phase2_heads=meta.get("has_phase2_heads", "false") == "true",
            has_phase3_heads=meta.get("has_phase3_heads", "false") == "true",
            has_v22_heads=meta.get("has_v22_heads", "false") == "true",
            expected_embed_model_ver=meta.get("expected_embed_model_ver", "egemma2-256"),
            expected_prompt_regime_id=meta.get("expected_prompt_regime_id", "t_d"),
        )
        model = cls(arch)
        expected_tensors = len(model._canonical_param_map())
        loaded, _ = model.load_matching_weights(safetensors_path)
        if loaded != expected_tensors:
            raise ValueError(
                f"Incomplete safetensors load for {safetensors_path}: loaded {loaded}/{expected_tensors} tensors"
            )
        return model, meta
