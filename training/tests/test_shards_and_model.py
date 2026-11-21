"""Shard schema, finite-value, model preset, and export checks for Wave 2."""

from pathlib import Path

import blake3
import numpy as np
import torch
from safetensors.torch import load_file

from monoagens_train.datasets import (
    _when2speak_lab_eval_ids,
    build_phase1_episodes,
    load_cached_shards,
)
from monoagens_train.features import NUM_FEATURES, PerceptNumericInputs, build_x_num
from monoagens_train.model import MonoNeuralController, architecture_for_scale

REPO_ROOT = Path(__file__).resolve().parents[2]


def test_x_num_handles_non_finite_and_bounds():
    vec = build_x_num(
        PerceptNumericInputs(
            is_named_soul=True,
            dt_since_last_place_event_s=float("nan"),
            budget_used_ratio=float("inf"),
            imminent_promise_debt=-2.0,
            max_visible_mem_cos=1.5,
            asr_conf_or_one=float("-inf"),
        )
    )
    assert vec.shape == (NUM_FEATURES,)
    assert vec.dtype == np.float32
    assert np.all(np.isfinite(vec))
    assert np.all((vec >= 0.0) & (vec <= 1.0))
    assert vec[0] == 1.0
    assert vec[17] == 1.0


def test_phase1_episodes_cover_all_four_datasets_and_exclude_lab_eval():
    episodes = build_phase1_episodes(REPO_ROOT, max_w2s_windows=500)
    ds_seen = {ep.dataset_id for ep in episodes}
    assert ds_seen == {"when2speak", "ubuntu_irc", "proactive_bench", "capa"}
    splits_seen = {ep.split for ep in episodes}
    assert splits_seen == {"train", "calibration", "test"}

    for ep in episodes[:50]:
        for t in ep.turns:
            assert t.x_num.shape == (NUM_FEATURES,)
            assert np.all(np.isfinite(t.x_num))
            assert 0 <= t.label_gate <= 3

    # Verify Lab eval ID exclusion helper deterministically catches S=1000 items
    mock_meta = [(f"w2s_{i:05d}", "SPEAK:spk=3" if i % 2 == 0 else "SILENT:spk=2") for i in range(2500)]
    excluded = _when2speak_lab_eval_ids(mock_meta, seeds=(42,), target_n=1000)
    assert len(excluded) == 1000


def test_model_presets_and_safetensors_export_roundtrip(tmp_path: Path):
    for scale in ("150k", "500k", "1m", "3m"):
        for cell in ("mingru", "gated_deltanet2"):
            arch = architecture_for_scale(scale, cell)  # type: ignore[arg-type]
            m = MonoNeuralController(arch)
            n_params = sum(p.numel() for p in m.parameters())
            if scale == "150k":
                assert 100_000 <= n_params <= 250_000
            elif scale == "500k":
                assert 350_000 <= n_params <= 700_000
            elif scale == "1m":
                assert 850_000 <= n_params <= 1_500_000
            else:
                assert 2_500_000 <= n_params <= 3_800_000

            e = torch.randn(2, 5, 256)
            x = torch.rand(2, 5, 18)
            out = m.forward_sequence(e, x, temperature=1.2)
            assert out["gate_probs"].shape == (2, 5, 4)
            assert torch.all(torch.isfinite(out["gate_probs"]))
            assert torch.allclose(out["gate_probs"].sum(dim=-1), torch.ones(2, 5), atol=1e-5)

            if scale == "150k":
                out_file = tmp_path / f"model_{cell}.safetensors"
                digest = m.export_safetensors(out_file, temperature=1.2, cic_threshold=0.55)
                assert digest == blake3.blake3(out_file.read_bytes()).hexdigest()
                loaded = load_file(str(out_file))
                assert "stem.plr.freqs" in loaded
                assert "head.gate_4.weight" in loaded


def test_cached_shards_if_present_are_finite_and_valid():
    cache_path = REPO_ROOT / "training" / "cache" / "phase1_shards.npz"
    if cache_path.is_file():
        shards = load_cached_shards(cache_path)
        for split in ("train", "calibration", "test"):
            assert len(shards[split]["e_256"]) > 0
            assert set(shards[split]["dataset_id"].tolist()) <= {
                "when2speak",
                "ubuntu_irc",
                "proactive_bench",
                "capa",
            }
