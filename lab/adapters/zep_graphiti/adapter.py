"""Out-of-process Zep / Graphiti BaselineAdapter (`getzep/graphiti` v0.29.3 API contract).

Uses `graphiti_core.Graphiti.add_episode(..., reference_time=sim_time)` and `EpisodeType.message`.
Reports `sdk_verified = false` (UNVERIFIED) when `graphiti_core` is absent offline.
"""

from __future__ import annotations

import os
import sys
from typing import Any, Dict

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
from common import assert_no_oracle_labels, check_sealed_barrier, run_ndjson_loop

try:
    from graphiti_core import Graphiti  # type: ignore[import-not-found]
    from graphiti_core.nodes import EpisodeType  # type: ignore[import-not-found]
    GRAPHITI_AVAILABLE = True
    GRAPHITI_IMPORT_ERR = None
except Exception as exc:
    Graphiti = None
    EpisodeType = None
    GRAPHITI_AVAILABLE = False
    GRAPHITI_IMPORT_ERR = f"graphiti_core Python package unavailable offline: {type(exc).__name__}: {exc}"


class ZepGraphitiAdapter:
    def __init__(self) -> None:
        self.sim_time = ""
        self.restart_epoch = 0
        self.sealed_mode = False
        self.ram_cleared_on_restart = False
        self.graphiti = None

    def handle(self, req: Dict[str, Any]) -> Dict[str, Any]:
        op = req["op"]
        if op == "init":
            self.sim_time = req["sim_time_rfc3339"]
            self.restart_epoch = int(req["restart_epoch"])
            self.sealed_mode = bool(req.get("sealed_mode", False)) or os.environ.get("SEALED_MODE") == "1"
            return {
                "ok": True,
                "adapter": "zep_graphiti",
                "sdk_verified": GRAPHITI_AVAILABLE,
                "unavailable_reason": GRAPHITI_IMPORT_ERR,
            }

        if op == "restart_epoch":
            self.restart_epoch = int(req["restart_epoch"])
            self.sim_time = req["sim_time_rfc3339"]
            self.graphiti = None
            self.ram_cleared_on_restart = True
            return {
                "ok": True,
                "restart_epoch": self.restart_epoch,
                "ram_cleared_on_restart": True,
                "sdk_verified": GRAPHITI_AVAILABLE,
            }

        if op == "step":
            self.sim_time = req["sim_time_rfc3339"]
            self.restart_epoch = int(req["restart_epoch"])
            percept = req["percept"]
            assert_no_oracle_labels(percept)
            check_sealed_barrier(self.sealed_mode, percept.get("content_payload_utf8", ""))

            if not GRAPHITI_AVAILABLE:
                return {
                    "ok": False,
                    "adapter": "zep_graphiti",
                    "sdk_verified": False,
                    "unavailable_reason": GRAPHITI_IMPORT_ERR,
                    "recorded_sim_time_rfc3339": self.sim_time,
                    "restart_epoch": self.restart_epoch,
                    "ram_cleared_on_restart": self.ram_cleared_on_restart,
                    "noop_tool_exposed": True,
                }
            raise RuntimeError("Neo4j/Graphiti URI not configured for offline run")

        if op == "shutdown":
            return {"ok": True}
        raise ValueError(f"unknown op: {op}")


if __name__ == "__main__":
    run_ndjson_loop(ZepGraphitiAdapter().handle)
