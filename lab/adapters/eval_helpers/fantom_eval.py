#!/usr/bin/env python3
"""Offline-only local FANToM SentenceTransformer evaluator (`all-roberta-large-v1`).

Reads JSON on stdin:
  {"model_path": "/abs/path/to/lab/datasets/scorer_models/all-roberta-large-v1",
   "items": [{"item_id": str, "response": str, "gold": str, "wrong_belief": str}]}
Writes JSON on stdout:
  {"results": [{"item_id": str, "ok": bool}]}

Strictly enforces `HF_HUB_OFFLINE=1` and `local_files_only=True`. Never downloads weights at
runtime; fails with a clear error when `model_path` is absent so Rust reports `not_scored`
(`scorer_model_unavailable`).
"""

from __future__ import annotations

import json
import os
import sys

os.environ["HF_HUB_OFFLINE"] = "1"
os.environ["TRANSFORMERS_OFFLINE"] = "1"

from sentence_transformers import SentenceTransformer, util


def _require_local_model_dir(model_path: object) -> str:
    if not isinstance(model_path, str) or not model_path.strip():
        raise FileNotFoundError(
            "scorer_model_unavailable: 'model_path' is required and must point to a fetched local model directory"
        )
    if not os.path.isdir(model_path) or not any(os.scandir(model_path)):
        raise FileNotFoundError(
            f"scorer_model_unavailable: local SentenceTransformer weights missing at '{model_path}'"
        )
    return model_path


def main() -> None:
    payload = json.load(sys.stdin)
    model_path = _require_local_model_dir(payload.get("model_path"))
    items = payload.get("items", [])
    if not items:
        json.dump({"results": []}, sys.stdout)
        return

    model = SentenceTransformer(model_path, local_files_only=True)
    results = []
    for item in items:
        pred = (item.get("response") or item.get("prediction") or "").strip()
        gold = (item.get("gold") or "").strip()
        wrong = (item.get("wrong_belief") or item.get("wrong_answer") or "").strip()
        if not pred or not gold:
            results.append({"item_id": item.get("item_id", ""), "ok": False})
            continue
        embs = model.encode([pred, gold, wrong] if wrong else [pred, gold], convert_to_tensor=True)
        sim_gold = float(util.cos_sim(embs[0], embs[1]))
        if wrong:
            sim_wrong = float(util.cos_sim(embs[0], embs[2]))
            ok = sim_gold > sim_wrong and sim_gold >= 0.45
        else:
            ok = sim_gold >= 0.55
        results.append({"item_id": item.get("item_id", ""), "ok": ok})

    json.dump({"results": results}, sys.stdout)


if __name__ == "__main__":
    main()
