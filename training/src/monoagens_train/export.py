"""Reproducible `safetensors` and multi-turn golden parity fixture export for v1 and v2.2."""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any, Sequence

import numpy as np
import torch

from monoagens_train.model import MonoNeuralController


def export_safetensors_with_metadata(
    model: MonoNeuralController,
    output_path: Path,
    *,
    temperature: float = 1.0,
    cic_threshold: float = 0.5,
    expected_embed_model_ver: str | None = None,
    expected_prompt_regime_id: str | None = None,
) -> str:
    """Exports `model` to canonical F32 `safetensors` with schema v1/v2 metadata and BLAKE3 digest."""
    return model.export_safetensors(
        output_path,
        temperature=temperature,
        cic_threshold=cic_threshold,
        expected_embed_model_ver=expected_embed_model_ver,
        expected_prompt_regime_id=expected_prompt_regime_id,
    )


def export_golden_fixture(
    cases: Sequence[tuple[str, MonoNeuralController, float, float]],
    fixtures_dir: Path,
    *,
    fixture_prefix: str = "neural_v22_golden",
    phase: int | None = None,
    n_steps: int = 3,
    seed: int = 2026,
) -> dict[str, str]:
    """Exports safetensors + multi-turn JSON golden parity fixtures for v1 (18d) or v2 (32d + V2.2).

    Each item in `cases` is `(label, model, temperature, cic_threshold)`.
    Step 1 always uses a zero embedding (`e_256 = 0`) to exercise vectorless parity.
    """
    if not cases:
        raise ValueError("export_golden_fixture requires at least one model case")

    fixtures_dir.mkdir(parents=True, exist_ok=True)
    first_arch = cases[0][1].arch
    d_in_emb = first_arch.d_in_emb
    d_in_num = first_arch.d_in_num
    for label, mdl, _temp, _cic in cases:
        if mdl.arch.d_in_emb != d_in_emb or mdl.arch.d_in_num != d_in_num:
            raise ValueError(
                f"Case '{label}' input dims ({mdl.arch.d_in_emb}, {mdl.arch.d_in_num}) "
                f"differ from ({d_in_emb}, {d_in_num})"
            )

    rng = np.random.default_rng(seed)
    e_seq = rng.normal(0.0, 0.25, size=(n_steps, d_in_emb)).astype(np.float32)
    if n_steps >= 2:
        e_seq[1, :] = 0.0  # deterministic vectorless parity step
    x_seq = rng.uniform(0.0, 1.0, size=(n_steps, d_in_num)).astype(np.float32)

    results: dict[str, Any] = {
        "feature_schema_version": first_arch.feature_schema_version,
        "cases": [],
    }
    if phase is not None:
        results["phase"] = phase

    blake3_map: dict[str, str] = {}
    for label, model, temperature, cic_threshold in cases:
        st_name = (
            f"{fixture_prefix}.safetensors"
            if label == "mingru"
            else f"{fixture_prefix}_{label}.safetensors"
        )
        st_path = fixtures_dir / st_name
        digest = export_safetensors_with_metadata(
            model,
            st_path,
            temperature=temperature,
            cic_threshold=cic_threshold,
        )
        blake3_map[label] = digest

        model_cpu = model.cpu().eval()
        arch = model_cpu.arch
        h = torch.zeros(1, arch.h_dim, dtype=torch.float32)
        h_actor = (
            torch.zeros(1, arch.h_actor_dim, dtype=torch.float32)
            if arch.h_actor_dim > 0
            else None
        )
        is_v2_payload = (
            arch.feature_schema_version >= 2
            or arch.h_actor_dim > 0
            or arch.has_v22_heads
        )

        steps: list[dict[str, Any]] = []
        with torch.no_grad():
            for t in range(n_steps):
                eb = torch.from_numpy(e_seq[t : t + 1])
                xb = torch.from_numpy(x_seq[t : t + 1])
                h_actor_in = h_actor.clone() if h_actor is not None else None
                out = model_cpu.step(
                    eb,
                    xb,
                    h,
                    temperature=temperature,
                    h_actor_prev=h_actor_in,
                )
                h = out["h_next"]
                if "h_actor_next" in out:
                    h_actor = out["h_actor_next"]
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
                    "expected_cic_abstain": bool(float(np.max(probs)) < cic_threshold),
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

                if is_v2_payload:
                    step_rec["h_actor_prev"] = (
                        [float(v) for v in h_actor_in[0].numpy()]
                        if h_actor_in is not None
                        else None
                    )
                    step_rec["expected_h_actor_next"] = (
                        [float(v) for v in h_actor[0].numpy()]
                        if h_actor is not None
                        else None
                    )
                    step_rec["expected_compute_tier_logits"] = (
                        [float(v) for v in out["compute_tier_logits"][0].numpy()]
                        if "compute_tier_logits" in out
                        else None
                    )
                    step_rec["expected_compute_tier_probs"] = (
                        [float(v) for v in out["compute_tier_probs"][0].numpy()]
                        if "compute_tier_probs" in out
                        else None
                    )
                    step_rec["expected_p_user_correction"] = (
                        float(out["p_user_correction"][0].item())
                        if "p_user_correction" in out
                        else None
                    )

                steps.append(step_rec)

        results["cases"].append(
            {
                "name": label,
                "safetensors_file": st_name,
                "blake3": digest,
                "temperature": float(temperature),
                "cic_threshold": float(cic_threshold),
                "feature_schema_version": arch.feature_schema_version,
                "d_in_num": arch.d_in_num,
                "h_actor_dim": arch.h_actor_dim,
                "has_phase2_heads": arch.has_phase2_heads,
                "has_phase3_heads": arch.has_phase3_heads,
                "has_v22_heads": arch.has_v22_heads,
                "steps": steps,
            }
        )

    json_path = fixtures_dir / f"{fixture_prefix}.json"
    json_path.write_text(json.dumps(results, indent=2), encoding="utf-8")
    return blake3_map
