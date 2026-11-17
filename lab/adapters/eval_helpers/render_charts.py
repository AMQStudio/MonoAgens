#!/usr/bin/env python3
"""Generates publication-grade matplotlib charts for MonoAgens Lab benchmark reports.

Reads a JSON chart payload on stdin (derived strictly from RunRecords) and writes PNG files
into the requested output directory. Returns JSON mapping chart names to file paths on stdout.
"""

from __future__ import annotations

import json
import os
import sys
from typing import Any

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt  # noqa: E402


def _save(fig: plt.Figure, out_dir: str, filename: str, manifest: dict[str, str], key: str) -> None:
    path = os.path.join(out_dir, filename)
    fig.tight_layout()
    fig.savefig(path, dpi=150, bbox_inches="tight")
    plt.close(fig)
    manifest[key] = filename


def _plot_domain_bars(rows: list[dict[str, Any]], out_dir: str, manifest: dict[str, str]) -> None:
    by_domain: dict[str, list[dict[str, Any]]] = {}
    for row in rows:
        by_domain.setdefault(row["domain"], []).append(row)

    for domain, drows in by_domain.items():
        labels = []
        vals = []
        err_lo = []
        err_hi = []
        for r in drows:
            for m in r.get("metrics", []):
                val = m.get("value")
                if val is None:
                    continue
                labels.append(f"{r['dataset_id']}\n({m['metric']})")
                vals.append(float(val))
                lo = m.get("ci95_low")
                hi = m.get("ci95_high")
                err_lo.append(max(0.0, float(val) - float(lo)) if lo is not None else 0.0)
                err_hi.append(max(0.0, float(hi) - float(val)) if hi is not None else 0.0)
        if not labels:
            continue
        fig, ax = plt.subplots(figsize=(max(6.0, 1.3 * len(labels)), 4.5))
        x = list(range(len(labels)))
        ax.bar(x, vals, yerr=[err_lo, err_hi], capsize=4, color="#2563eb", edgecolor="#1e3a8a", alpha=0.88)
        ax.set_xticks(x)
        ax.set_xticklabels(labels, rotation=20, ha="right", fontsize=8.5)
        ax.set_ylabel("Score")
        ax.set_ylim(0.0, max(1.05, max(vals) * 1.15))
        ax.set_title(f"{domain.replace('_', ' ').title()} — Primary Metrics (95% CI)")
        ax.grid(axis="y", linestyle="--", alpha=0.35)
        for xi, v in zip(x, vals):
            ax.text(xi, v + 0.02, f"{v:.3f}", ha="center", va="bottom", fontsize=8)
        _save(fig, out_dir, f"domain_{domain}.png", manifest, f"domain_{domain}")


def _plot_categories(categories: list[dict[str, Any]], out_dir: str, manifest: dict[str, str]) -> None:
    if not categories:
        return
    # Group up to 8 datasets with category metrics into one or more clean subplots
    by_ds: dict[str, list[tuple[str, float]]] = {}
    for item in categories:
        by_ds.setdefault(item["dataset_id"], []).append((item["category"], float(item["value"])))
    if not by_ds:
        return
    n = len(by_ds)
    cols = min(2, n)
    rows_n = (n + cols - 1) // cols
    fig, axes = plt.subplots(rows_n, cols, figsize=(6.5 * cols, 3.8 * rows_n), squeeze=False)
    flat_axes = [ax for row in axes for ax in row]
    for ax, (ds_id, pairs) in zip(flat_axes, sorted(by_ds.items())):
        cats = [p[0] for p in pairs]
        vals = [p[1] for p in pairs]
        x = list(range(len(cats)))
        ax.bar(x, vals, color="#0d9488", edgecolor="#115e59", alpha=0.88)
        ax.set_xticks(x)
        ax.set_xticklabels(cats, rotation=22, ha="right", fontsize=8)
        ax.set_ylim(0.0, max(1.05, max(vals) * 1.15))
        ax.set_title(f"{ds_id} — Per-Category Breakdown")
        ax.set_ylabel("Score")
        ax.grid(axis="y", linestyle="--", alpha=0.35)
        for xi, v in zip(x, vals):
            ax.text(xi, v + 0.02, f"{v:.2f}", ha="center", va="bottom", fontsize=7.5)
    for ax in flat_axes[len(by_ds) :]:
        ax.axis("off")
    _save(fig, out_dir, "category_breakdowns.png", manifest, "category_breakdowns")


def _plot_cost_tokens(costs: list[dict[str, Any]], out_dir: str, manifest: dict[str, str]) -> None:
    if not costs:
        return
    labels = [c["dataset_id"] for c in costs]
    tok_in = [float(c.get("tokens_in", 0)) for c in costs]
    tok_out = [float(c.get("tokens_out", 0)) for c in costs]
    if sum(tok_in) + sum(tok_out) <= 0:
        return
    fig, ax = plt.subplots(figsize=(max(7.5, 0.55 * len(labels)), 4.6))
    x = list(range(len(labels)))
    ax.bar(x, tok_in, label="Input tokens", color="#3b82f6")
    ax.bar(x, tok_out, bottom=tok_in, label="Output tokens", color="#f59e0b")
    ax.set_xticks(x)
    ax.set_xticklabels(labels, rotation=30, ha="right", fontsize=8)
    ax.set_ylabel("Tokens")
    ax.set_title("Tokens In / Out per Dataset")
    ax.legend()
    ax.grid(axis="y", linestyle="--", alpha=0.35)
    _save(fig, out_dir, "tokens_per_dataset.png", manifest, "tokens_per_dataset")


def _plot_wake_silence(costs: list[dict[str, Any]], out_dir: str, manifest: dict[str, str]) -> None:
    active = [c for c in costs if c.get("percepts", 0) > 0]
    if not active:
        return
    labels = [c["dataset_id"] for c in active]
    wakes = [float(c.get("wakes_per_event", 0.0)) for c in active]
    silence = [float(c.get("silence_rate", 0.0)) for c in active]
    fig, ax = plt.subplots(figsize=(max(7.5, 0.6 * len(labels)), 4.6))
    x = list(range(len(labels)))
    w = 0.38
    ax.bar([i - w / 2 for i in x], wakes, width=w, label="Wakes / percept", color="#8b5cf6")
    ax.bar([i + w / 2 for i in x], silence, width=w, label="Silence rate", color="#10b981")
    ax.set_xticks(x)
    ax.set_xticklabels(labels, rotation=30, ha="right", fontsize=8)
    ax.set_ylabel("Rate")
    ax.set_ylim(0.0, max(1.05, max(wakes + silence) * 1.15))
    ax.set_title("Gate Wake Rate & Silence Rate per Dataset")
    ax.legend()
    ax.grid(axis="y", linestyle="--", alpha=0.35)
    _save(fig, out_dir, "wake_silence_rates.png", manifest, "wake_silence_rates")


def _plot_pareto(pareto_points: list[dict[str, Any]], out_dir: str, manifest: dict[str, str]) -> None:
    if not pareto_points:
        return
    fig, ax = plt.subplots(figsize=(7.5, 5.0))
    frontier_pts = []
    for pt in pareto_points:
        x = float(pt["e_tok"])
        y = float(pt["score"])
        on_front = bool(pt.get("on_frontier", False))
        color = "#dc2626" if on_front else "#64748b"
        marker = "*" if on_front else "o"
        size = 130 if on_front else 55
        ax.scatter([x], [y], c=color, marker=marker, s=size, zorder=3)
        ax.annotate(pt["dataset_id"], (x, y), textcoords="offset points", xytext=(5, 4), fontsize=7.5)
        if on_front:
            frontier_pts.append((x, y))
    if len(frontier_pts) >= 2:
        frontier_pts.sort(key=lambda t: t[0])
        ax.plot([p[0] for p in frontier_pts], [p[1] for p in frontier_pts], "--", color="#dc2626", alpha=0.75, label="Pareto frontier")
        ax.legend()
    ax.set_xlabel("Total Tokens (E_tok)")
    ax.set_ylabel("Primary Score")
    ax.set_title("MonoAgens Cost-vs-Quality Pareto Frontier (Scored Rows Only)")
    ax.grid(True, linestyle="--", alpha=0.35)
    _save(fig, out_dir, "pareto_frontier.png", manifest, "pareto_frontier")


def main() -> None:
    payload = json.load(sys.stdin)
    out_dir = payload["charts_dir"]
    os.makedirs(out_dir, exist_ok=True)
    manifest: dict[str, str] = {}
    _plot_domain_bars(payload.get("domain_rows", []), out_dir, manifest)
    _plot_categories(payload.get("category_rows", []), out_dir, manifest)
    _plot_cost_tokens(payload.get("cost_rows", []), out_dir, manifest)
    _plot_wake_silence(payload.get("cost_rows", []), out_dir, manifest)
    _plot_pareto(payload.get("pareto_points", []), out_dir, manifest)
    json.dump({"charts": manifest}, sys.stdout)


if __name__ == "__main__":
    main()
