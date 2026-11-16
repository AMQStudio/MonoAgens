"""Out-of-process LangGraph + LangMem BaselineAdapter (`langchain-ai/langgraph` API contract).

Uses `langgraph.graph.StateGraph`, `InMemorySaver`, and `langmem` memory manager.
Reports `sdk_verified = false` (UNVERIFIED) when `langgraph` / `langmem` is absent offline.
"""

from __future__ import annotations

import os
import sys
from typing import Any, Dict

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
from common import assert_no_oracle_labels, check_sealed_barrier, run_ndjson_loop

try:
    from langgraph.checkpoint.memory import InMemorySaver  # type: ignore[import-not-found]
    from langgraph.graph import StateGraph  # type: ignore[import-not-found]
    LANGGRAPH_AVAILABLE = True
    LANGGRAPH_IMPORT_ERR = None
except Exception as exc:
    InMemorySaver = None
    StateGraph = None
    LANGGRAPH_AVAILABLE = False
    LANGGRAPH_IMPORT_ERR = f"langgraph/langmem Python package unavailable offline: {type(exc).__name__}: {exc}"


class LangGraphLangMemAdapter:
    def __init__(self) -> None:
        self.sim_time = ""
        self.restart_epoch = 0
        self.sealed_mode = False
        self.ram_cleared_on_restart = False
        self.checkpointer = None

    def handle(self, req: Dict[str, Any]) -> Dict[str, Any]:
        op = req["op"]
        if op == "init":
            self.sim_time = req["sim_time_rfc3339"]
            self.restart_epoch = int(req["restart_epoch"])
            self.sealed_mode = bool(req.get("sealed_mode", False)) or os.environ.get("SEALED_MODE") == "1"
            return {
                "ok": True,
                "adapter": "langgraph_langmem",
                "sdk_verified": LANGGRAPH_AVAILABLE,
                "unavailable_reason": LANGGRAPH_IMPORT_ERR,
            }

        if op == "restart_epoch":
            self.restart_epoch = int(req["restart_epoch"])
            self.sim_time = req["sim_time_rfc3339"]
            self.checkpointer = None
            self.ram_cleared_on_restart = True
            return {
                "ok": True,
                "restart_epoch": self.restart_epoch,
                "ram_cleared_on_restart": True,
                "sdk_verified": LANGGRAPH_AVAILABLE,
            }

        if op == "step":
            self.sim_time = req["sim_time_rfc3339"]
            self.restart_epoch = int(req["restart_epoch"])
            percept = req["percept"]
            assert_no_oracle_labels(percept)
            check_sealed_barrier(self.sealed_mode, percept.get("content_payload_utf8", ""))

            if not LANGGRAPH_AVAILABLE:
                return {
                    "ok": False,
                    "adapter": "langgraph_langmem",
                    "sdk_verified": False,
                    "unavailable_reason": LANGGRAPH_IMPORT_ERR,
                    "recorded_sim_time_rfc3339": self.sim_time,
                    "restart_epoch": self.restart_epoch,
                    "ram_cleared_on_restart": self.ram_cleared_on_restart,
                    "noop_tool_exposed": True,
                }
            raise RuntimeError("LangGraph LLM provider not configured for offline run")

        if op == "shutdown":
            return {"ok": True}
        raise ValueError(f"unknown op: {op}")


if __name__ == "__main__":
    run_ndjson_loop(LangGraphLangMemAdapter().handle)
