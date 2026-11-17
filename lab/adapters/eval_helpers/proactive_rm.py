#!/usr/bin/env python3
"""Offline-only ProactiveAgent local Reward Model (RM) scorer (`proactive_rm.py`).

Reads JSON on stdin:
  {"model_path": "/abs/path/to/lab/datasets/scorer_models/proactive-agent-rm",
   "items": [{"item_id": str, "context": str, "response": str, "reference": str}]}
Writes JSON on stdout:
  {"results": [{"item_id": str, "accepted": bool, "score": float}]}

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

import torch
from transformers import AutoModelForSequenceClassification, AutoTokenizer


def _require_local_model_dir(model_path: object) -> str:
    if not isinstance(model_path, str) or not model_path.strip():
        raise FileNotFoundError(
            "scorer_model_unavailable: 'model_path' is required and must point to a fetched local model directory"
        )
    if not os.path.isdir(model_path) or not any(os.scandir(model_path)):
        raise FileNotFoundError(
            f"scorer_model_unavailable: local ProactiveAgent-RM weights missing at '{model_path}'"
        )
    return model_path


def main() -> None:
    payload = json.load(sys.stdin)
    model_path = _require_local_model_dir(payload.get("model_path"))
    items = payload.get("items", [])
    if not items:
        json.dump({"results": []}, sys.stdout)
        return

    tokenizer = AutoTokenizer.from_pretrained(model_path, local_files_only=True)
    model = AutoModelForSequenceClassification.from_pretrained(model_path, local_files_only=True)
    model.eval()

    results = []
    with torch.no_grad():
        for item in items:
            text = f"Context: {item.get('context', '')}\nResponse: {item.get('response', '')}"
            inputs = tokenizer(text, return_tensors="pt", truncation=True, max_length=1024)
            logits = model(**inputs).logits
            score = float(torch.sigmoid(logits.squeeze()).item())
            results.append(
                {
                    "item_id": item.get("item_id", ""),
                    "accepted": score >= 0.5,
                    "score": score,
                }
            )

    json.dump({"results": results}, sys.stdout)


if __name__ == "__main__":
    main()
