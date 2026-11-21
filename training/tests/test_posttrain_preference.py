"""Unit tests for Stage 4 preference & counterfactual post-training (E-70.1)."""

from pathlib import Path

import numpy as np
import pytest
import torch

from monoagens_train.augment import build_procedural_timing_shards
from monoagens_train.model import MonoNeuralController, architecture_for_scale
from monoagens_train.posttrain_preference import (
    Stage4PreferenceConfig,
    build_stage4_preference_targets,
    compute_asymmetric_kto_loss,
    compute_categorical_dpo_margin_loss,
    compute_cfpo_invariance_loss,
    compute_tabm_jsd,
    measure_cfpo_jsd,
    run_stage4_preference_posttrain,
)
from monoagens_train.train_multitask import evaluate_multitask_model


def test_stage4_losses_are_finite_and_kto_enforces_asymmetry():
    logits_k = torch.randn(4, 6, 4, 4, requires_grad=True)
    ref_logits_k = logits_k.detach().clone()
    log_pi = torch.log_softmax(logits_k.mean(dim=-2), dim=-1)
    log_ref = torch.log_softmax(ref_logits_k.mean(dim=-2), dim=-1)
    pi = torch.exp(log_pi)

    jsd = compute_tabm_jsd(logits_k)
    assert jsd.shape == (4, 6)
    assert torch.all(torch.isfinite(jsd))
    assert torch.all(jsd >= 0.0)

    pos = torch.zeros((4, 6), dtype=torch.long)
    neg = torch.full((4, 6), 3, dtype=torch.long)
    mask = torch.ones((4, 6), dtype=torch.float32)
    dpo_loss = compute_categorical_dpo_margin_loss(log_pi, log_ref, pos, neg, mask, weights=1.0 / (1.0 + jsd.detach()))
    kto_loss = compute_asymmetric_kto_loss(log_pi, log_ref, pi, pos, mask, neg, mask, lambda_d=1.0, lambda_u=2.5)
    cfpo_loss = compute_cfpo_invariance_loss(logits_k, torch.zeros_like(logits_k), mask)

    for val in (dpo_loss, kto_loss, cfpo_loss):
        assert torch.isfinite(val)
        assert float(val.item()) >= 0.0

    with pytest.raises(ValueError, match="lambda_u > lambda_d"):
        compute_asymmetric_kto_loss(log_pi, log_ref, pi, pos, mask, neg, mask, lambda_d=2.0, lambda_u=1.0)


def test_stage4_reduces_cfpo_invariance_and_keeps_param_count_and_sealed_safety(tmp_path: Path):
    shards = build_procedural_timing_shards(seq_len=12, n_train=48, n_cal=24, n_test=24, seed=7)
    arch = architecture_for_scale("150k", "mingru", has_phase2_heads=True, has_phase3_heads=True)
    model = MonoNeuralController(arch)
    params_before = sum(p.numel() for p in model.parameters())

    targets = build_stage4_preference_targets(
        shards["train"]["x_num"],
        shards["train"]["y_gate"],
        shards["train"]["mask"],
        shards["train"].get("dataset_id"),
    )
    assert float(np.sum(targets["cfpo_mask"])) > 0.0
    assert float(np.sum(targets["m_1"])) > 0.0
    assert float(np.sum(targets["m_2"])) > 0.0

    # Inject mild vector-dependent drift on timing sequences before Stage 4 so CFPO starts non-trivial
    with torch.no_grad():
        model.emb_linear.weight.add_(0.25)

    cfpo_before = measure_cfpo_jsd(model, shards["test"], device=torch.device("cpu"))
    tuned_model, entry, diag = run_stage4_preference_posttrain(
        model=model,
        shards=shards,
        phase1_test_split=shards["test"],
        device=torch.device("cpu"),
        evaluate_fn=evaluate_multitask_model,
        scale="150k",
        cell_type="mingru",
        config=Stage4PreferenceConfig(epochs=6, batch_size=16, lr=1e-3, cfpo_weight=1.5),
        repo_root=tmp_path,
    )
    params_after = sum(p.numel() for p in tuned_model.parameters())
    assert params_after == params_before
    assert entry.cic_threshold > 0.25
    assert diag["cfpo_jsd_after"] < cfpo_before
    assert np.isfinite(diag["stage4_final_loss"])

    sealed = tmp_path / "lab" / "scenarios" / "sealed" / "secret.json"
    with pytest.raises(PermissionError):
        run_stage4_preference_posttrain(
            model=model,
            shards=shards,
            phase1_test_split=shards["test"],
            device=torch.device("cpu"),
            evaluate_fn=evaluate_multitask_model,
            scale="150k",
            cell_type="mingru",
            repo_root=sealed,
        )
