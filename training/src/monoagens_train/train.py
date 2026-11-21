"""Phase 1 RTX 4060 sweep, calibration, report generation, and safetensors parity export."""

from __future__ import annotations

import argparse
import json
import time
from dataclasses import asdict, dataclass
from pathlib import Path

import numpy as np
import torch
import torch.nn.functional as F
from torch.utils.data import DataLoader, TensorDataset

from monoagens_train.calibrate import (
    compute_aurc,
    compute_brier,
    compute_ece,
    compute_macro_f1,
    compute_nll,
    compute_tc_ece,
    fit_cic_threshold,
    fit_temperature,
    probs_from_logits_k,
)
from monoagens_train.datasets import load_cached_shards, precompute_phase1_shards
from monoagens_train.licenses import assert_safe_path
from monoagens_train.model import (
    CellKind,
    MonoNeuralController,
    ScalePreset,
    architecture_for_scale,
)

SWEEP_SCALES: tuple[ScalePreset, ...] = ("150k", "500k", "1m", "3m")
SWEEP_CELLS: tuple[CellKind, ...] = ("mingru", "gated_deltanet2")


@dataclass
class SweepEntry:
    scale: str
    cell_type: str
    param_count: int
    train_seconds: float
    temperature: float
    cic_threshold: float
    nll_before_ts: float
    nll_after_ts: float
    test_macro_f1: float
    test_accuracy: float
    test_ece: float
    test_tc_ece: float
    test_brier: float
    test_aurc: float
    test_cic_coverage: float
    test_cic_selective_risk: float


def _set_seed(seed: int = 42) -> None:
    np.random.seed(seed)
    torch.manual_seed(seed)
    if torch.cuda.is_available():
        torch.cuda.manual_seed_all(seed)


def _compute_class_weights(y: np.ndarray, mask: np.ndarray, device: torch.device) -> torch.Tensor:
    active = y[mask > 0.5]
    counts = np.bincount(active, minlength=4).astype(np.float64)
    counts = np.maximum(counts, 1.0)
    inv_sqrt = np.sqrt(np.max(counts) / counts)
    inv_sqrt = np.clip(inv_sqrt, 0.5, 4.0)
    return torch.tensor(inv_sqrt, dtype=torch.float32, device=device)


@torch.no_grad()
def _collect_split_outputs(
    model: MonoNeuralController,
    split_dict: dict[str, np.ndarray],
    device: torch.device,
    batch_size: int = 128,
) -> tuple[np.ndarray, np.ndarray, np.ndarray, np.ndarray]:
    model.eval()
    e = torch.from_numpy(split_dict["e_256"])
    x = torch.from_numpy(split_dict["x_num"])
    y = split_dict["y_gate"]
    m = split_dict["mask"]

    logits_k_list: list[np.ndarray] = []
    for start in range(0, len(e), batch_size):
        eb = e[start : start + batch_size].to(device)
        xb = x[start : start + batch_size].to(device)
        out = model.forward_sequence(eb, xb, temperature=1.0)
        logits_k_list.append(out["logits_k"].cpu().numpy())

    logits_k_seq = np.concatenate(logits_k_list, axis=0)
    return logits_k_seq, y, m, split_dict["y_wait_ms"]


def train_and_evaluate_one(
    scale: ScalePreset,
    cell_type: CellKind,
    shards: dict[str, dict[str, np.ndarray]],
    device: torch.device,
    epochs: int = 12,
    batch_size: int = 64,
    lr: float = 1.5e-3,
    seed: int = 42,
) -> tuple[MonoNeuralController, SweepEntry]:
    _set_seed(seed)
    arch = architecture_for_scale(scale, cell_type)
    model = MonoNeuralController(arch).to(device)
    param_count = sum(p.numel() for p in model.parameters())

    tr = shards["train"]
    train_ds = TensorDataset(
        torch.from_numpy(tr["e_256"]),
        torch.from_numpy(tr["x_num"]),
        torch.from_numpy(tr["y_gate"]),
        torch.from_numpy(tr["y_wait_ms"]),
        torch.from_numpy(tr["mask"]),
    )
    loader = DataLoader(train_ds, batch_size=batch_size, shuffle=True, drop_last=False)
    class_weights = _compute_class_weights(tr["y_gate"], tr["mask"], device)

    optimizer = torch.optim.AdamW(model.parameters(), lr=lr, weight_decay=1e-2)
    scheduler = torch.optim.lr_scheduler.CosineAnnealingLR(optimizer, T_max=max(1, epochs))

    t0 = time.perf_counter()
    for _epoch in range(epochs):
        model.train()
        for eb, xb, yb, wb, mb in loader:
            eb = eb.to(device)
            xb = xb.to(device)
            yb = yb.to(device)
            wb = wb.to(device)
            mb = mb.to(device)

            optimizer.zero_grad(set_to_none=True)
            out = model.forward_sequence(eb, xb, temperature=1.0)
            # Member-averaged cross-entropy loss across TabM ensemble
            logits_k = out["logits_k"]  # [B, T, K, 4]
            bsz, seq_len, k_mem, _ = logits_k.shape
            ce_per_mem = F.cross_entropy(
                logits_k.reshape(-1, 4),
                yb.unsqueeze(2).expand(bsz, seq_len, k_mem).reshape(-1),
                weight=class_weights,
                reduction="none",
            ).reshape(bsz, seq_len, k_mem)
            ce_mean = ce_per_mem.mean(dim=-1)
            denom = mb.sum().clamp_min(1.0)
            cls_loss = (ce_mean * mb).sum() / denom

            # Auxiliary wait_delta_t regression on active turns
            wait_pred_norm = out["wait_delta_ms"] / arch.max_wait_ms
            wait_true_norm = (wb / arch.max_wait_ms).clamp(0.0, 1.0)
            wait_loss = (F.smooth_l1_loss(wait_pred_norm, wait_true_norm, reduction="none") * mb).sum() / denom

            loss = cls_loss + 0.25 * wait_loss
            loss.backward()
            torch.nn.utils.clip_grad_norm_(model.parameters(), max_norm=1.0)
            optimizer.step()
        scheduler.step()
    train_seconds = time.perf_counter() - t0

    # Calibration split: fit temperature + CIC threshold
    cal_logits_k_seq, cal_y_seq, cal_mask_seq, _ = _collect_split_outputs(
        model, shards["calibration"], device
    )
    cal_active = cal_mask_seq > 0.5
    cal_logits_k = cal_logits_k_seq[cal_active]
    cal_y = cal_y_seq[cal_active]

    temperature, nll_before, nll_after = fit_temperature(cal_logits_k, cal_y)
    cal_probs = probs_from_logits_k(cal_logits_k, temperature)
    cic_threshold, _cal_cov, _cal_emp, _cal_up = fit_cic_threshold(
        cal_probs, cal_y, target_risk=0.05, delta=0.05, min_coverage=0.25
    )

    # Held-out test split evaluation
    test_logits_k_seq, test_y_seq, test_mask_seq, _ = _collect_split_outputs(
        model, shards["test"], device
    )
    bsz_t, seq_t, k_t, _ = test_logits_k_seq.shape
    test_probs_seq = probs_from_logits_k(
        test_logits_k_seq.reshape(-1, k_t, 4), temperature
    ).reshape(bsz_t, seq_t, 4)

    test_active = test_mask_seq > 0.5
    test_probs = test_probs_seq[test_active]
    test_y = test_y_seq[test_active]

    preds = np.argmax(test_probs, axis=-1)
    conf = np.max(test_probs, axis=-1)
    accepted = conf >= cic_threshold
    cic_cov = float(np.mean(accepted)) if len(accepted) > 0 else 0.0
    cic_risk = float(np.mean(preds[accepted] != test_y[accepted])) if np.any(accepted) else 0.0

    entry = SweepEntry(
        scale=scale,
        cell_type=cell_type,
        param_count=param_count,
        train_seconds=round(train_seconds, 2),
        temperature=round(temperature, 6),
        cic_threshold=round(cic_threshold, 6),
        nll_before_ts=round(nll_before, 6),
        nll_after_ts=round(nll_after, 6),
        test_macro_f1=round(compute_macro_f1(test_probs, test_y), 6),
        test_accuracy=round(float(np.mean(preds == test_y)), 6),
        test_ece=round(compute_ece(test_probs, test_y), 6),
        test_tc_ece=round(compute_tc_ece(test_probs_seq, test_y_seq, test_mask_seq), 6),
        test_brier=round(compute_brier(test_probs, test_y), 6),
        test_aurc=round(compute_aurc(test_probs, test_y), 6),
        test_cic_coverage=round(cic_cov, 6),
        test_cic_selective_risk=round(cic_risk, 6),
    )
    return model, entry


def export_golden_parity_fixture(
    mingru_model: MonoNeuralController,
    gdn2_model: MonoNeuralController,
    mingru_entry: SweepEntry,
    gdn2_entry: SweepEntry,
    fixtures_dir: Path,
) -> dict[str, str]:
    """Exports trained compact safetensors + multi-turn JSON parity fixtures for Rust `neural_parity`."""
    fixtures_dir.mkdir(parents=True, exist_ok=True)
    rng = np.random.default_rng(2026)

    # Multi-turn (3 turns) percept sequence: turn 0 normal, turn 1 vectorless (e_256=0), turn 2 normal
    e_seq = rng.normal(0.0, 0.25, size=(3, 256)).astype(np.float32)
    e_seq[1, :] = 0.0
    x_seq = rng.uniform(0.0, 1.0, size=(3, 18)).astype(np.float32)

    results: dict[str, object] = {"cases": []}
    blake3_map: dict[str, str] = {}

    for label, model, entry in (
        ("mingru", mingru_model, mingru_entry),
        ("gated_deltanet2", gdn2_model, gdn2_entry),
    ):
        st_name = f"neural_phase1_golden_{label}.safetensors"
        if label == "mingru":
            st_name = "neural_phase1_golden.safetensors"
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
                steps.append(
                    {
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
                    }
                )
        results["cases"].append(
            {
                "name": label,
                "safetensors_file": st_name,
                "blake3": digest,
                "temperature": entry.temperature,
                "cic_threshold": entry.cic_threshold,
                "steps": steps,
            }
        )

    json_path = fixtures_dir / "neural_phase1_golden.json"
    json_path.write_text(json.dumps(results, indent=2), encoding="utf-8")
    return blake3_map


def run_phase1_sweep(
    repo_root: Path,
    epochs: int = 12,
    max_w2s_windows: int = 4500,
    force_recompute_shards: bool = False,
) -> dict[str, object]:
    cache_path = assert_safe_path(
        repo_root / "training" / "cache" / "phase1_shards.npz", repo_root=repo_root
    )
    resolved_cache_path = precompute_phase1_shards(
        repo_root=repo_root,
        cache_path=cache_path,
        max_w2s_windows=max_w2s_windows,
        force_recompute=force_recompute_shards,
        prompt_regime_id="t_d",
        embed_model_ver="egemma2-256",
    )
    shards = load_cached_shards(resolved_cache_path)

    device = torch.device("cuda" if torch.cuda.is_available() else "cpu")
    entries: list[SweepEntry] = []
    trained_models: dict[tuple[str, str], tuple[MonoNeuralController, SweepEntry]] = {}

    for scale in SWEEP_SCALES:
        for cell in SWEEP_CELLS:
            model, entry = train_and_evaluate_one(
                scale=scale,
                cell_type=cell,
                shards=shards,
                device=device,
                epochs=epochs,
            )
            entries.append(entry)
            trained_models[(scale, cell)] = (model.cpu(), entry)

    # Select winning model: highest (macro_f1 - 0.5 * tc_ece - 0.5 * aurc)
    winner_entry = max(
        entries,
        key=lambda e: e.test_macro_f1 - 0.5 * e.test_tc_ece - 0.5 * e.test_aurc,
    )
    winner_model, _ = trained_models[(winner_entry.scale, winner_entry.cell_type)]

    artifacts_dir = repo_root / "training" / "artifacts"
    best_path = artifacts_dir / "neural_phase1_best.safetensors"
    best_blake3 = winner_model.export_safetensors(
        best_path,
        temperature=winner_entry.temperature,
        cic_threshold=winner_entry.cic_threshold,
    )
    manifest_path = artifacts_dir / "manifest.json"
    manifest_data = {
        "artifact": str(best_path.relative_to(repo_root)),
        "blake3": best_blake3,
        "winner": asdict(winner_entry),
    }
    manifest_path.write_text(json.dumps(manifest_data, indent=2), encoding="utf-8")

    # Export compact 150k trained fixtures (both minGRU and Gated DeltaNet-2) for Rust parity tests
    mg_model, mg_entry = trained_models[("150k", "mingru")]
    gdn_model, gdn_entry = trained_models[("150k", "gated_deltanet2")]
    fixtures_dir = repo_root / "crates" / "core" / "tests" / "fixtures"
    fixture_hashes = export_golden_parity_fixture(
        mg_model, gdn_model, mg_entry, gdn_entry, fixtures_dir
    )

    # Write markdown + json reports
    reports_dir = repo_root / "training" / "reports"
    reports_dir.mkdir(parents=True, exist_ok=True)
    report_json = {
        "device": torch.cuda.get_device_name(0) if torch.cuda.is_available() else "cpu",
        "split_counts": {k: int(len(v["e_256"])) for k, v in shards.items()},
        "winner": asdict(winner_entry),
        "best_artifact_blake3": best_blake3,
        "fixture_blake3": fixture_hashes,
        "sweep": [asdict(e) for e in entries],
    }
    (reports_dir / "phase1_sweep_report.json").write_text(
        json.dumps(report_json, indent=2), encoding="utf-8"
    )

    lines = [
        "# Phase 1 Sweep Report (`150k`–`3M` × `minGRU` vs `Gated DeltaNet-2`)",
        "",
        f"- **Device:** `{report_json['device']}`",
        f"- **Split Sequences:** train={report_json['split_counts']['train']}, "
        f"calibration={report_json['split_counts']['calibration']}, "
        f"test={report_json['split_counts']['test']}",
        f"- **Winning Config:** `{winner_entry.scale}` + `{winner_entry.cell_type}` "
        f"({winner_entry.param_count:,} params, `T={winner_entry.temperature}`, "
        f"`cic_threshold={winner_entry.cic_threshold}`)",
        f"- **Best Artifact:** `{manifest_data['artifact']}` (`blake3={best_blake3}`)",
        "",
        "| Scale | Cell | Params | Time(s) | T | CIC τ | Macro-F1 | Acc | ECE | TC-ECE | Brier | AURC | CIC Cov | CIC Risk |",
        "|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|",
    ]
    for e in entries:
        lines.append(
            f"| {e.scale} | {e.cell_type} | {e.param_count:,} | {e.train_seconds:.2f} | "
            f"{e.temperature:.3f} | {e.cic_threshold:.3f} | {e.test_macro_f1:.4f} | "
            f"{e.test_accuracy:.4f} | {e.test_ece:.4f} | {e.test_tc_ece:.4f} | "
            f"{e.test_brier:.4f} | {e.test_aurc:.4f} | {e.test_cic_coverage:.4f} | "
            f"{e.test_cic_selective_risk:.4f} |"
        )
    (reports_dir / "phase1_sweep_report.md").write_text("\n".join(lines) + "\n", encoding="utf-8")
    return report_json


def main() -> None:
    parser = argparse.ArgumentParser(description="Run MonoAgens Phase 1 neural controller sweep.")
    parser.add_argument("--repo-root", type=Path, default=Path(__file__).resolve().parents[3])
    parser.add_argument("--epochs", type=int, default=12)
    parser.add_argument("--max-w2s-windows", type=int, default=4500)
    parser.add_argument("--force-recompute-shards", action="store_true")
    args = parser.parse_args()

    report = run_phase1_sweep(
        repo_root=args.repo_root.resolve(),
        epochs=args.epochs,
        max_w2s_windows=args.max_w2s_windows,
        force_recompute_shards=args.force_recompute_shards,
    )
    print(json.dumps(report["winner"], indent=2))


if __name__ == "__main__":
    main()
