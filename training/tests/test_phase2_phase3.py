"""License audit, anti-leakage split isolation, and Phase 2/3 head roundtrip tests."""

from pathlib import Path

import blake3
import numpy as np
import torch
from safetensors import safe_open

from _helpers import resolve_data_repo_root
from monoagens_train.datasets import (
    PHASE2_DATASETS,
    PHASE3_DATASETS,
    _dyntom_lab_eval_clusters,
    build_phase2_episodes,
    build_phase3_episodes,
)
from monoagens_train.features import NUM_FEATURES
from monoagens_train.licenses import require_dataset_allowed
from monoagens_train.model import MonoNeuralController, architecture_for_scale

REPO_ROOT = resolve_data_repo_root(Path(__file__).resolve().parents[2])


def test_phase2_and_phase3_datasets_pass_osi_audit():
    manifest = REPO_ROOT / "lab" / "datasets.manifest.toml"
    for ds_id in (*PHASE2_DATASETS, *PHASE3_DATASETS):
        entry = require_dataset_allowed(manifest, ds_id)
        assert entry.allowed, f"Dataset {ds_id} unexpectedly blocked: {entry.reason}"


def test_phase2_episodes_and_strict_split_isolation():
    eps = build_phase2_episodes(REPO_ROOT)
    assert {e.dataset_id for e in eps} == set(PHASE2_DATASETS)
    assert {e.split for e in eps} == {"train", "calibration", "test"}

    # Verify cluster_ids are strictly disjoint across train/calibration/test
    split_clusters: dict[str, set[str]] = {"train": set(), "calibration": set(), "test": set()}
    for ep in eps:
        split_clusters[ep.split].add(ep.cluster_id)
        for t in ep.turns[:24]:
            assert t.x_num.shape == (NUM_FEATURES,)
            assert np.all(np.isfinite(t.x_num))
            assert 0.0 <= t.y_uncertainty <= 1.0
            assert 0.0 <= t.y_verify <= 1.0

    assert not (split_clusters["train"] & split_clusters["calibration"])
    assert not (split_clusters["train"] & split_clusters["test"])
    assert not (split_clusters["calibration"] & split_clusters["test"])

    # Verify DynToM Lab S=471 eval clusters are completely excluded
    import json

    trials = json.loads((REPO_ROOT / "lab" / "datasets" / "dyntom" / "DynToM.json").read_text())
    lab_eval = _dyntom_lab_eval_clusters(trials, seeds=(1,))
    assert len(lab_eval) == 471
    dyntom_train_clusters = {
        ep.cluster_id.removeprefix("dyntom:") for ep in eps if ep.dataset_id == "dyntom"
    }
    assert not (dyntom_train_clusters & lab_eval)


def test_phase3_episodes_and_disjoint_splits():
    eps = build_phase3_episodes(REPO_ROOT)
    assert {e.dataset_id for e in eps} == set(PHASE3_DATASETS)
    assert {e.split for e in eps} == {"train", "calibration", "test"}

    split_clusters: dict[str, set[str]] = {"train": set(), "calibration": set(), "test": set()}
    for ep in eps:
        split_clusters[ep.split].add(ep.cluster_id)
        for t in ep.turns[:24]:
            assert t.x_num.shape == (NUM_FEATURES,)
            assert np.all(np.isfinite(t.x_num))
            assert 0.0 <= t.y_encode_importance <= 1.0
            assert 0.0 <= t.y_memory_relevance <= 1.0

    assert not (split_clusters["train"] & split_clusters["calibration"])
    assert not (split_clusters["train"] & split_clusters["test"])
    assert not (split_clusters["calibration"] & split_clusters["test"])


def test_phase2_and_phase3_heads_and_warmstart_roundtrip(tmp_path: Path):
    p1_arch = architecture_for_scale("150k", "mingru")
    p1 = MonoNeuralController(p1_arch)
    p1_path = tmp_path / "p1.safetensors"
    p1.export_safetensors(p1_path, temperature=1.1, cic_threshold=0.5)

    p3_arch = architecture_for_scale(
        "150k", "mingru", has_phase2_heads=True, has_phase3_heads=True
    )
    p3 = MonoNeuralController(p3_arch)
    loaded, meta = p3.load_matching_weights(p1_path)
    assert loaded > 0
    assert meta["has_phase2_heads"] == "false"

    e = torch.randn(2, 4, 256)
    x = torch.rand(2, 4, 18)
    out = p3.forward_sequence(e, x, temperature=1.1)
    for k in ("p_uncertainty", "p_verify", "p_encode_importance", "p_memory_relevance"):
        assert k in out
        assert out[k].shape == (2, 4)
        assert torch.all(torch.isfinite(out[k]))
        assert torch.all((out[k] >= 0.0) & (out[k] <= 1.0))

    p3_path = tmp_path / "p3.safetensors"
    digest = p3.export_safetensors(p3_path, temperature=1.1, cic_threshold=0.5)
    assert digest == blake3.blake3(p3_path.read_bytes()).hexdigest()
    with safe_open(str(p3_path), framework="pt") as f:
        m3 = f.metadata()
        assert m3["has_phase2_heads"] == "true"
        assert m3["has_phase3_heads"] == "true"
        keys = set(f.keys())
        assert "head.p_uncertainty.weight" in keys
        assert "head.p_verify.weight" in keys
        assert "head.p_encode_importance.weight" in keys
        assert "head.p_memory_relevance.weight" in keys


def test_procedural_timing_shards_cover_habituation_wait_and_vectorless():
    from monoagens_train.augment import build_procedural_timing_shards

    shards = build_procedural_timing_shards(seq_len=24, n_train=64, n_cal=16, n_test=16)
    tr = shards["train"]
    assert tr["e_256"].shape == (64, 24, 256)
    assert tr["x_num"].shape == (64, 24, 18)
    assert np.all(np.isfinite(tr["e_256"]))
    assert np.all(np.isfinite(tr["x_num"]))
    assert np.all((tr["x_num"] >= 0.0) & (tr["x_num"] <= 1.0))

    active = tr["mask"] > 0.5
    classes = set(np.unique(tr["y_gate"][active]).tolist())
    assert {0, 2, 3}.issubset(classes)
    # Verify vectorless examples are well represented
    row_norms = np.linalg.norm(tr["e_256"][active], axis=-1)
    assert np.mean(row_norms == 0.0) > 0.4
    # Verify Wait examples carry positive wait_delta_ms
    wait_mask = active & (tr["y_gate"] == 2)
    assert np.all(tr["y_wait_ms"][wait_mask] >= 1500.0)


def test_fit_cic_threshold_is_non_vacuous_above_simplex_floor():
    from monoagens_train.calibrate import fit_cic_threshold

    rng = np.random.default_rng(123)
    n = 1000
    labels = rng.integers(0, 4, size=n)
    probs = np.full((n, 4), 0.12, dtype=np.float64)
    # 78% accurate confident predictions, 22% near-uniform ambiguous predictions
    for i in range(n):
        if i < 780:
            probs[i] = 0.05
            probs[i, labels[i]] = 0.85
        else:
            wrong = (labels[i] + 1) % 4
            probs[i] = 0.18
            probs[i, wrong] = 0.46
    tau, cov, _emp, upper = fit_cic_threshold(probs, labels, target_risk=0.05, delta=0.05)
    assert tau > 0.25
    assert tau >= 0.30
    assert cov >= 0.20
    assert upper <= 0.05
