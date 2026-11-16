"""Shared stdio NDJSON BaselineAdapter protocol helpers (R-39).

Enforces:
- Injected virtual time (`sim_time_rfc3339`) without wall-clock `datetime.now()`
- Pinned deterministic `run_seed`
- `PerceptView` oracle-label stripping check
- Explicit `noop()` / `[SILENT]` abstention tool with symmetric 5-tuple `E_tok` billing
- `restart_epoch` ephemeral RAM eviction
- `SEALED_MODE=1` zero-payload/prompt persistence & stdout barrier
"""

from __future__ import annotations

import json
import os
import sys
from typing import Any, Callable, Dict, Optional


ALPHA_CACHE_READ = 0.25
ALPHA_CACHE_WRITE = 1.25
RHO_OUT = 4.0


def compute_e_tok(usage: Dict[str, int]) -> float:
    return (
        float(usage["uncached_in"])
        + ALPHA_CACHE_READ * float(usage["cache_read"])
        + ALPHA_CACHE_WRITE * float(usage["cache_write"])
        + RHO_OUT * float(usage["out_vis"] + usage["reason"])
    )


def compute_c_share(usage: Dict[str, int]) -> float:
    total_in = usage["uncached_in"] + usage["cache_read"] + usage["cache_write"]
    return float(usage["cache_read"]) / float(max(1, total_in))


def assert_no_oracle_labels(percept: Dict[str, Any]) -> None:
    forbidden = {
        "warranted",
        "sel_other",
        "answered",
        "false_belief_target",
        "episode_boundary",
        "sealed_tag",
        "lure",
        "annotations",
    }
    present = forbidden.intersection(percept.keys())
    if present:
        raise ValueError(f"oracle label leakage in PerceptView: {sorted(present)}")


def check_sealed_barrier(sealed_mode: bool, payload_text: str, extra_output: Optional[str] = None) -> None:
    if not sealed_mode:
        return
    if os.environ.get("MONOAGENS_TEST_FORCE_SEALED_LEAK") == "1":
        # Deliberate leak simulation for negative conformance testing: write payload to stdout/disk.
        leak_dir = os.environ.get("MONOAGENS_LEAK_DIR")
        if leak_dir:
            with open(os.path.join(leak_dir, "leaked_prompt.txt"), "w", encoding="utf-8") as fh:
                fh.write(payload_text)
        raise RuntimeError(f"SEALED_MODE_LEAK: {payload_text}")
    if extra_output and payload_text and payload_text in extra_output:
        raise RuntimeError("SEALED_MODE=1 violation: payload echoed in adapter telemetry")


def run_ndjson_loop(handler: Callable[[Dict[str, Any]], Dict[str, Any]]) -> None:
    for raw_line in sys.stdin:
        line = raw_line.strip()
        if not line:
            continue
        req = json.loads(line)
        resp = handler(req)
        sys.stdout.write(json.dumps(resp, separators=(",", ":")) + "\n")
        sys.stdout.flush()
        if req.get("op") == "shutdown":
            break
