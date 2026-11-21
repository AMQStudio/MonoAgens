"""Wave C Track 3 tests: schema=2 (32d), V2.2 heads + h_actor, procedural curricula, OSI SEA loaders, and Stage 4."""

from __future__ import annotations

import json
from pathlib import Path

import blake3
import numpy as np
import pytest
import torch
from safetensors import safe_open

from monoagens_train.augment import (
    COMPUTE_TIER_DEEP,
    COMPUTE_TIER_FAST,
    COMPUTE_TIER_STANDARD,
    build_procedural_timing_shards,
    build_procedural_v22_shards,
)
from monoagens_train.datasets import build_v22_sea_episodes, load_multitask_shards
from monoagens_train.export import export_golden_fixture
from monoagens_train.features import (
    FEATURE_NAMES_V2,
    NUM_FEATURES_V1,
    NUM_FEATURES_V2,
    PerceptNumericInputs,
    PerceptNumericInputsV2,
    build_x_num,
    build_x_num_v2,
    upgrade_x_num_v1_to_v2,
)
from monoagens_train.licenses import (
    audit_manifest,
    audit_v22_registry,
    require_v22_dataset_allowed,
)
from monoagens_train.model import MonoNeuralController, architecture_for_scale
from monoagens_train.posttrain_preference import (
    Stage4PreferenceConfig,
    build_stage4_preference_targets,
    measure_cfpo_jsd,
    run_stage4_preference_posttrain,
)
from monoagens_train.train_multitask import evaluate_multitask_model, train_multitask_model


def test_x_num_v2_schema_layout_bounds_and_v1_compat():
    assert NUM_FEATURES_V1 == 18
    assert NUM_FEATURES_V2 == 32
    assert len(FEATURE_NAMES_V2) == 32

    base_inp = PerceptNumericInputs(
        is_named_soul=True,
        is_explicit_addressee=False,
        is_reply_or_expect_link=True,
        active_participant_count=6,
        dt_since_last_place_event_s=2.5,
        max_visible_mem_cos=0.72,
    )
    v1_vec = build_x_num(base_inp)
    v2_vec = build_x_num_v2(
        PerceptNumericInputsV2(
            is_named_soul=True,
            is_explicit_addressee=False,
            is_reply_or_expect_link=True,
            active_participant_count=6,
            dt_since_last_place_event_s=2.5,
            max_visible_mem_cos=0.72,
            has_recent_visible_soul_act=True,
            cos_soul_user=0.42,
            cos_prev_user_rephrase=0.88,
            cos_expected_reply=0.15,
            is_reply_to_last_soul=True,
            soul_intervened_since_sender_last=True,
            max_active_thread_cos=0.79,
            active_thread_count=3,
            sender_in_top_thread=True,
            sender_reliability=0.81,
            is_sender_automated_peer=True,
            room_has_other_automated_peers=True,
            consecutive_automated_turns=3,
            peer_expertise_margin=0.6,  # maps [-1, 1] -> 0.8
        )
    )
    assert v2_vec.shape == (32,)
    assert v2_vec.dtype == np.float32
    assert np.all(np.isfinite(v2_vec))
    assert np.all((v2_vec >= 0.0) & (v2_vec <= 1.0))
    assert np.allclose(v2_vec[:18], v1_vec)
    # Repair (18..24)
    assert v2_vec[18] == 1.0
    assert np.isclose(v2_vec[19], 0.42)
    assert np.isclose(v2_vec[20], 0.88)
    assert np.isclose(v2_vec[21], 0.15)
    assert v2_vec[22] == 1.0
    assert v2_vec[23] == 1.0
    # Thread/actor (24..28)
    assert np.isclose(v2_vec[24], 0.79)
    assert np.isclose(v2_vec[25], 0.5)
    assert v2_vec[26] == 1.0
    assert np.isclose(v2_vec[27], 0.81)
    # Peer-agent (28..32)
    assert v2_vec[28] == 1.0
    assert v2_vec[29] == 1.0
    assert np.isclose(v2_vec[30], 0.5)
    assert np.isclose(v2_vec[31], 0.8)

    # Neutral & non-finite safety
    neutral = build_x_num_v2(
        PerceptNumericInputsV2(
            cos_soul_user=float("nan"),
            sender_reliability=float("inf"),
            peer_expertise_margin=None,
        )
    )
    assert np.all(np.isfinite(neutral))
    assert np.isclose(neutral[27], 0.5)
    assert np.isclose(neutral[31], 0.5)

    # V1 -> V2 upcast
    upcast = upgrade_x_num_v1_to_v2(v1_vec[None, :])
    assert upcast.shape == (1, 32)
    assert np.allclose(upcast[0, :18], v1_vec)
    assert np.isclose(upcast[0, 27], 0.5)
    assert np.isclose(upcast[0, 31], 0.5)


@pytest.mark.parametrize("cell_type", ("mingru", "gated_deltanet2"))
@pytest.mark.parametrize("h_actor_dim", (24, 32))
def test_v22_model_h_actor_and_v22_heads_safetensors_roundtrip(
    tmp_path: Path, cell_type: str, h_actor_dim: int
):
    base_arch = architecture_for_scale(
        "150k",
        cell_type,  # type: ignore[arg-type]
        has_phase2_heads=True,
        has_phase3_heads=True,
        feature_schema_version=2,
        h_actor_dim=h_actor_dim,
        has_v22_heads=False,
    )
    v22_arch = architecture_for_scale(
        "150k",
        cell_type,  # type: ignore[arg-type]
        has_phase2_heads=True,
        has_phase3_heads=True,
        feature_schema_version=2,
        h_actor_dim=h_actor_dim,
        has_v22_heads=True,
    )
    base_params = sum(p.numel() for p in MonoNeuralController(base_arch).parameters())
    model = MonoNeuralController(v22_arch).eval()
    v22_params = sum(p.numel() for p in model.parameters())
    # V2.2 heads (`compute_tier` + `p_user_correction`) must stay strictly lightweight (< +9K)
    assert 0 < (v22_params - base_params) < 9_000

    st_path = tmp_path / f"v22_{cell_type}_ha{h_actor_dim}.safetensors"
    digest = model.export_safetensors(st_path, temperature=1.15, cic_threshold=0.55)
    assert digest == blake3.blake3(st_path.read_bytes()).hexdigest()

    with safe_open(str(st_path), framework="pt") as sf:
        meta = sf.metadata()
        assert meta["feature_schema_version"] == "2"
        assert meta["d_in_num"] == "32"
        assert meta["has_v22_heads"] == "true"
        assert meta["h_actor_dim"] == str(h_actor_dim)
        assert meta["expected_embed_model_ver"] == "egemma2-256"
        assert meta["expected_prompt_regime_id"] == "t_d"

        keys = set(sf.keys())
        fusion_w = sf.get_tensor("stem.fusion.weight")
        assert tuple(fusion_w.shape) == (
            v22_arch.d_model,
            v22_arch.d_e + v22_arch.d_n + v22_arch.h_dim + h_actor_dim,
        )
        assert "head.compute_tier.weight" in keys
        assert tuple(sf.get_tensor("head.compute_tier.weight").shape) == (3, v22_arch.d_model)
        assert tuple(sf.get_tensor("head.compute_tier.bias").shape) == (3,)
        assert "head.p_user_correction.weight" in keys
        assert tuple(sf.get_tensor("head.p_user_correction.weight").shape) == (1, v22_arch.d_model)
        assert tuple(sf.get_tensor("head.p_user_correction.bias").shape) == (1,)

        if cell_type == "mingru":
            for suffix in ("w_z.weight", "w_z.bias", "w_h.weight", "w_h.bias"):
                assert f"actor_memory.mingru.{suffix}" in keys
        else:
            for suffix in (
                "w_erase.weight",
                "w_erase.bias",
                "w_write.weight",
                "w_write.bias",
                "w_delta.weight",
                "w_delta.bias",
            ):
                assert f"actor_memory.gdn2.{suffix}" in keys

    reloaded, _meta = MonoNeuralController.from_safetensors(st_path)
    reloaded.eval()

    e = torch.randn(2, 4, 256)
    x = torch.rand(2, 4, 32)
    with torch.no_grad():
        out_orig = model.forward_sequence(e, x, temperature=1.15)
        out_re = reloaded.forward_sequence(e, x, temperature=1.15)
        # Also verify step(..., h_actor_prev=None) defaults to zeros cleanly
        h0 = torch.zeros(2, v22_arch.h_dim)
        step_none = model.step(e[:, 0, :], x[:, 0, :], h0, temperature=1.15, h_actor_prev=None)
        step_zero = model.step(
            e[:, 0, :], x[:, 0, :], h0, temperature=1.15, h_actor_prev=torch.zeros(2, h_actor_dim)
        )
        assert torch.allclose(step_none["h_actor_next"], step_zero["h_actor_next"], atol=1e-6)

    assert out_orig["h_actor_seq"].shape == (2, 4, h_actor_dim)
    assert out_orig["compute_tier_probs"].shape == (2, 4, 3)
    assert torch.allclose(out_orig["compute_tier_probs"].sum(dim=-1), torch.ones(2, 4), atol=1e-5)
    assert out_orig["p_user_correction"].shape == (2, 4)
    assert torch.all((out_orig["p_user_correction"] >= 0.0) & (out_orig["p_user_correction"] <= 1.0))
    for k in ("gate_probs", "h_seq", "h_actor_seq", "compute_tier_probs", "p_user_correction"):
        assert torch.allclose(out_orig[k], out_re[k], atol=1e-6)


def test_export_golden_fixture_supports_v1_and_v2_models(tmp_path: Path):
    # 1. Backward-compatible v1 fixture
    v1_mg = MonoNeuralController(architecture_for_scale("150k", "mingru"))
    v1_gdn = MonoNeuralController(architecture_for_scale("150k", "gated_deltanet2"))
    v1_hashes = export_golden_fixture(
        [("mingru", v1_mg, 1.1, 0.5), ("gated_deltanet2", v1_gdn, 1.2, 0.52)],
        tmp_path / "v1",
        fixture_prefix="neural_phase1_golden",
        phase=1,
    )
    assert set(v1_hashes) == {"mingru", "gated_deltanet2"}
    v1_json = json.loads((tmp_path / "v1" / "neural_phase1_golden.json").read_text())
    assert v1_json["feature_schema_version"] == 1
    assert len(v1_json["cases"]) == 2
    assert len(v1_json["cases"][0]["steps"][0]["x_num"]) == 18
    assert all(v == 0.0 for v in v1_json["cases"][0]["steps"][1]["e_256"])
    assert "expected_compute_tier_logits" not in v1_json["cases"][0]["steps"][0]

    # 2. V2.2 fixture (32d + h_actor + compute_tier + p_user_correction)
    v2_mg = MonoNeuralController(
        architecture_for_scale(
            "150k",
            "mingru",
            has_phase2_heads=True,
            has_phase3_heads=True,
            has_v22_heads=True,
            feature_schema_version=2,
            h_actor_dim=24,
        )
    )
    v2_gdn = MonoNeuralController(
        architecture_for_scale(
            "150k",
            "gated_deltanet2",
            has_phase2_heads=True,
            has_phase3_heads=True,
            has_v22_heads=True,
            feature_schema_version=2,
            h_actor_dim=24,
        )
    )
    v2_hashes = export_golden_fixture(
        [("mingru", v2_mg, 1.05, 0.55), ("gated_deltanet2", v2_gdn, 1.10, 0.58)],
        tmp_path / "v2",
        fixture_prefix="neural_v22_golden",
        phase=3,
    )
    assert set(v2_hashes) == {"mingru", "gated_deltanet2"}
    v2_json = json.loads((tmp_path / "v2" / "neural_v22_golden.json").read_text())
    assert v2_json["feature_schema_version"] == 2
    for case in v2_json["cases"]:
        assert case["d_in_num"] == 32
        assert case["h_actor_dim"] == 24
        assert case["has_v22_heads"] is True
        st_bytes = (tmp_path / "v2" / case["safetensors_file"]).read_bytes()
        assert blake3.blake3(st_bytes).hexdigest() == case["blake3"]
        for step_idx, step in enumerate(case["steps"]):
            assert len(step["x_num"]) == 32
            assert len(step["h_actor_prev"]) == 24
            assert len(step["expected_h_actor_next"]) == 24
            assert len(step["expected_compute_tier_logits"]) == 3
            assert len(step["expected_compute_tier_probs"]) == 3
            assert abs(sum(step["expected_compute_tier_probs"]) - 1.0) < 1e-5
            assert 0.0 <= step["expected_p_user_correction"] <= 1.0
            if step_idx > 0:
                assert step["h_actor_prev"] == case["steps"][step_idx - 1]["expected_h_actor_next"]


def test_augment_v22_curricula_all_regimes_pecah_balon_cs_repair_peer():
    shards = build_procedural_v22_shards(seq_len=24, n_train=160, n_cal=32, n_test=32, seed=99)
    tr = shards["train"]
    assert tr["e_256"].shape == (160, 24, 256)
    assert tr["x_num"].shape == (160, 24, 32)
    assert np.all(np.isfinite(tr["e_256"]))
    assert np.all(np.isfinite(tr["x_num"]))
    assert np.all((tr["x_num"] >= 0.0) & (tr["x_num"] <= 1.0))

    active = tr["mask"] > 0.5
    # Verify vectorless coverage stays strong
    norms = np.linalg.norm(tr["e_256"][active], axis=-1)
    assert np.mean(norms == 0.0) > 0.40

    # Verify all 3 compute tiers and both user_correction classes are covered
    v22_act = tr["v22_mask"] > 0.5
    tiers = set(np.unique(tr["y_compute_tier"][v22_act]).tolist())
    assert tiers == {COMPUTE_TIER_FAST, COMPUTE_TIER_STANDARD, COMPUTE_TIER_DEEP}
    uc_vals = set(np.unique(tr["y_user_correction"][v22_act]).tolist())
    assert uc_vals == {0.0, 1.0}

    # Regime 4 (indices i % 8 == 4): pecah balon intermediate bursts -> Wait (1500-4000ms), final -> Go/Silent
    r4_idx = [i for i in range(160) if i % 8 == 4]
    r4_waits = tr["y_wait_ms"][r4_idx][tr["y_gate"][r4_idx] == 2]
    assert len(r4_waits) >= len(r4_idx)
    assert np.all((r4_waits >= 1500.0) & (r4_waits <= 4000.0))

    # Regime 5 (indices i % 8 == 5): spherical code-switch drift (unit vectors, attenuated mem_cos)
    r5_idx = [i for i in range(160) if i % 8 == 5]
    r5_mask = tr["mask"][r5_idx] > 0.5
    r5_norms = np.linalg.norm(tr["e_256"][r5_idx][r5_mask], axis=-1)
    assert np.allclose(r5_norms, 1.0, atol=1e-4)
    r5_mem_cos = tr["x_num"][r5_idx, :, 13][r5_mask]
    assert np.all((r5_mem_cos >= 0.40) & (r5_mem_cos <= 0.85))

    # Regime 6 (indices i % 8 == 6): self-repair Wait + implicit repair p_user_correction=1.0 -> Deep
    r6_idx = [i for i in range(160) if i % 8 == 6]
    r6_corr = tr["y_user_correction"][r6_idx] > 0.5
    assert np.any(r6_corr)
    assert np.all(tr["y_compute_tier"][r6_idx][r6_corr] == COMPUTE_TIER_DEEP)
    assert np.all(tr["x_num"][r6_idx, :, 22][r6_corr] == 1.0)  # is_reply_to_last_soul
    assert np.all(tr["x_num"][r6_idx, :, 20][r6_corr] >= 0.60)  # cos_prev_user_rephrase
    assert np.any((tr["y_gate"][r6_idx] == 2) & (tr["mask"][r6_idx] > 0.5))

    # Regime 7 (indices i % 8 == 7): peer-agent anti-ping-pong, expertise deferral, own-expertise Go
    r7_idx = [i for i in range(160) if i % 8 == 7]
    r7_x = tr["x_num"][r7_idx]
    r7_y = tr["y_gate"][r7_idx]
    r7_m = tr["mask"][r7_idx] > 0.5
    pingpong = r7_m & (r7_x[..., 28] > 0.5) & (r7_x[..., 30] >= 2.0 / 6.0 - 1e-4)
    assert np.any(pingpong)
    assert np.all(r7_y[pingpong] == 0)  # Silent
    defer = r7_m & (r7_x[..., 31] > 0.65)
    assert np.any(defer)
    assert np.all(np.isin(r7_y[defer], [0, 2]))  # Wait or Silent
    own_exp = r7_m & (r7_x[..., 28] < 0.5) & (r7_x[..., 31] < 0.35)
    assert np.any(own_exp)
    assert np.all(r7_y[own_exp] == 3)  # Go


def test_osi_v22_registry_and_manifest_reject_nc_nd_sa_corpora(tmp_path: Path):
    report = audit_v22_registry()
    allowed = report.allowed_ids()
    rejected = report.rejected_ids()

    assert {"seadialogues", "indodialect", "permembench", "personamem_v2", "vars"} <= allowed
    assert {"culturetalk_id", "speak_or_stay_silent", "pingpong"} <= rejected

    for ok_name in ("SEADialogues", "IndoDialect", "PerMemBench", "PersonaMem-v2", "VARS"):
        entry = require_v22_dataset_allowed(ok_name)
        assert entry.allowed

    for blocked_name in ("CULTURETALK-ID", "Speak or Stay Silent", "PingPong"):
        with pytest.raises(ValueError, match="license filter"):
            require_v22_dataset_allowed(blocked_name)

    # Also verify TOML manifest audit rejects these exact SPDX licenses
    manifest_toml = tmp_path / "test_v22.manifest.toml"
    manifest_toml.write_text(
        """
schema_version = "1.0"

[[datasets]]
id = "seadialogues"
relative_dir = "seadialogues"
availability = "available"
[datasets.license]
spdx = "CC-BY-4.0"

[[datasets]]
id = "vars"
relative_dir = "vars"
availability = "available"
[datasets.license]
spdx = "Apache-2.0"

[[datasets]]
id = "culturetalk_id"
relative_dir = "culturetalk_id"
availability = "available"
[datasets.license]
spdx = "CC-BY-NC-SA-4.0"

[[datasets]]
id = "speak_or_stay_silent"
relative_dir = "speak_or_stay_silent"
availability = "available"
[datasets.license]
spdx = "CC-BY-NC-ND-4.0"

[[datasets]]
id = "pingpong"
relative_dir = "pingpong"
availability = "available"
[datasets.license]
spdx = "CC-BY-SA-4.0"
""",
        encoding="utf-8",
    )
    m_report = audit_manifest(manifest_toml)
    assert m_report.allowed_ids() == {"seadialogues", "vars"}
    assert m_report.rejected_ids() == {"culturetalk_id", "speak_or_stay_silent", "pingpong"}


def test_v22_sea_loaders_parse_local_files_and_fallback_cleanly_offline(tmp_path: Path):
    # Graceful offline fallback when datasets are not downloaded yet
    assert build_v22_sea_episodes(tmp_path, schema_version=2) == []

    # Blocked datasets raise immediately even if absent
    for blocked in ("CULTURETALK-ID", "Speak or Stay Silent", "PingPong"):
        with pytest.raises(ValueError, match="license filter"):
            build_v22_sea_episodes(tmp_path, dataset_ids=(blocked,))

    # Populate synthetic local OSI-clean records for all 5 datasets
    ds_root = tmp_path / "lab" / "datasets"
    for name in ("seadialogues", "indodialect", "vars"):
        p = ds_root / name / "sample.jsonl"
        p.parent.mkdir(parents=True, exist_ok=True)
        row = {
            "id": f"{name}_001",
            "turns": [
                {"speaker": "user", "text": "eh bentar ya", "is_burst_fragment": True},
                {"speaker": "assistant", "text": "Siap, silakan lanjut."},
                {"speaker": "user", "text": "bukan itu maksudnya, jadwal jam 3", "is_correction": True},
            ],
        }
        p.write_text(json.dumps(row) + "\n", encoding="utf-8")

    for name in ("permembench", "personamem_v2"):
        p = ds_root / name / "sample.json"
        p.parent.mkdir(parents=True, exist_ok=True)
        payload = [
            {
                "id": f"{name}_001",
                "history": [
                    {"content": "I prefer oat milk in my coffee.", "is_preference": True},
                    {"content": "Actually I switched to black coffee now.", "is_update": True},
                ],
                "question": "What coffee do I drink now?",
            }
        ]
        p.write_text(json.dumps(payload), encoding="utf-8")

    eps = build_v22_sea_episodes(tmp_path, schema_version=2)
    assert {ep.dataset_id for ep in eps} == {
        "seadialogues",
        "indodialect",
        "permembench",
        "personamem_v2",
        "vars",
    }
    for ep in eps:
        assert ep.turns
        for t in ep.turns:
            assert t.x_num.shape == (32,)
            assert np.all(np.isfinite(t.x_num))
            assert np.all((t.x_num >= 0.0) & (t.x_num <= 1.0))
            assert t.y_compute_tier in (0, 1, 2)
            assert t.y_user_correction in (0.0, 1.0)


def test_train_multitask_and_stage4_preference_v22_smoke(tmp_path: Path):
    shards_v2 = build_procedural_timing_shards(
        seq_len=12, n_train=64, n_cal=24, n_test=24, seed=17, schema_version=2
    )
    shards_v1_test = build_procedural_timing_shards(
        seq_len=12, n_train=8, n_cal=8, n_test=16, seed=18, schema_version=1
    )["test"]

    # Verify schema=2 preference target builder covers dev-07, dev-10, dev-13
    targets = build_stage4_preference_targets(
        shards_v2["train"]["x_num"],
        shards_v2["train"]["y_gate"],
        shards_v2["train"]["mask"],
        shards_v2["train"]["dataset_id"],
    )
    assert float(np.sum(targets["m_1"] > 1.0)) > 0.0
    assert float(np.sum(targets["m_2"] > 1.0)) > 0.0
    assert float(np.sum(targets["cfpo_mask"])) > 0.0

    # Train V2.2 model (`schema=2`, `has_v22_heads=True`, `h_actor_dim=24`) with 18d Phase 1 test split upcast
    model, entry = train_multitask_model(
        phase=3,
        scale="150k",
        cell_type="mingru",
        shards=shards_v2,
        phase1_test_split=shards_v1_test,
        device=torch.device("cpu"),
        epochs=2,
        batch_size=16,
        lr=1e-3,
        seed=42,
        feature_schema_version=2,
        has_v22_heads=True,
        h_actor_dim=24,
    )
    assert entry.feature_schema_version == 2
    assert entry.h_actor_dim == 24
    assert entry.has_v22_heads is True
    assert entry.cic_threshold > 0.25
    assert entry.compute_tier_acc is not None
    assert entry.compute_tier_macro_f1 is not None
    assert entry.p_user_correction_bce is not None
    assert entry.p_user_correction_f1 is not None

    # Inject anisotropic vector-dependent drift so CFPO starts non-trivial, then run Stage 4
    params_before = sum(p.numel() for p in model.parameters())
    gen = torch.Generator().manual_seed(123)
    with torch.no_grad():
        drift = torch.randn(model.emb_linear.weight.shape, generator=gen) * 0.6
        model.emb_linear.weight.add_(drift)
        model.emb_norm.weight.mul_(2.0)
    cfpo_before = measure_cfpo_jsd(model, shards_v2["test"], device=torch.device("cpu"))
    assert cfpo_before > 0.02

    tuned_model, s4_entry, s4_diag = run_stage4_preference_posttrain(
        model=model,
        shards=shards_v2,
        phase1_test_split=shards_v1_test,
        device=torch.device("cpu"),
        evaluate_fn=evaluate_multitask_model,
        scale="150k",
        cell_type="mingru",
        phase=3,
        config=Stage4PreferenceConfig(epochs=6, batch_size=16, lr=1e-3, cfpo_weight=2.0),
        repo_root=tmp_path,
    )
    assert sum(p.numel() for p in tuned_model.parameters()) == params_before
    assert s4_entry.cic_threshold > 0.25
    assert s4_diag["cfpo_jsd_after"] < cfpo_before
    assert s4_diag["feature_schema_version"] == 2.0
    assert s4_diag["h_actor_dim"] == 24.0
    assert s4_diag["has_v22_heads"] == 1.0

    # Export Stage 4 V2.2 model and verify safetensors metadata preservation
    out_st = tmp_path / "v22_stage4.safetensors"
    tuned_model.export_safetensors(
        out_st,
        temperature=s4_entry.temperature,
        cic_threshold=s4_entry.cic_threshold,
    )
    with safe_open(str(out_st), framework="pt") as sf:
        meta = sf.metadata()
        assert meta["feature_schema_version"] == "2"
        assert meta["d_in_num"] == "32"
        assert meta["has_v22_heads"] == "true"
        assert meta["h_actor_dim"] == "24"
        assert meta["expected_embed_model_ver"] == "egemma2-256"
        assert meta["expected_prompt_regime_id"] == "t_d"

    # Also verify load_multitask_shards upcasts a saved v1 .npz shard cleanly to schema=2
    v1_npz = tmp_path / "v1_shards.npz"
    v1_sh = build_procedural_timing_shards(seq_len=8, n_train=4, n_cal=2, n_test=2, schema_version=1)
    arrays = {}
    for sp in ("train", "calibration", "test"):
        arrays[f"{sp}_e256"] = v1_sh[sp]["e_256"]
        arrays[f"{sp}_xnum"] = v1_sh[sp]["x_num"]
        arrays[f"{sp}_ygate"] = v1_sh[sp]["y_gate"]
        arrays[f"{sp}_ywait"] = v1_sh[sp]["y_wait_ms"]
        arrays[f"{sp}_mask"] = v1_sh[sp]["mask"]
        arrays[f"{sp}_ds"] = v1_sh[sp]["dataset_id"]
    np.savez(v1_npz, **arrays)
    up_sh = load_multitask_shards(v1_npz, target_schema_version=2)
    assert up_sh["train"]["x_num"].shape == (4, 8, 32)
    assert up_sh["train"]["y_compute_tier"].shape == (4, 8)
    assert up_sh["train"]["v22_mask"].shape == (4, 8)
