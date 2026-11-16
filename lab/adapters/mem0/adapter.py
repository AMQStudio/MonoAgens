"""Out-of-process Mem0 BaselineAdapter (mem0ai/mem0 v2.2.1 API contract).

Uses `mem0.Memory` / `mem0.MemoryClient` (`add`, `search`, `get_all`, `delete_all`, `reset`).
If `mem0` is unavailable offline, reports `sdk_verified = false` (UNVERIFIED) without fabricating scores.
"""

from __future__ import annotations

import os
import sys
from typing import Any, Dict

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
from common import assert_no_oracle_labels, check_sealed_barrier, run_ndjson_loop

try:
    from mem0 import Memory as Mem0Memory  # type: ignore[import-not-found]
    MEM0_AVAILABLE = True
    MEM0_IMPORT_ERR = None
except Exception as exc:  # pragma: no cover - verified absent offline
    Mem0Memory = None
    MEM0_AVAILABLE = False
    MEM0_IMPORT_ERR = f"mem0 Python package unavailable offline: {type(exc).__name__}: {exc}"


class Mem0Adapter:
    def __init__(self) -> None:
        self.sim_time = ""
        self.restart_epoch = 0
        self.sealed_mode = False
        self.ram_cleared_on_restart = False
        self.client = None

    def handle(self, req: Dict[str, Any]) -> Dict[str, Any]:
        op = req["op"]
        if op == "init":
            self.sim_time = req["sim_time_rfc3339"]
            self.restart_epoch = int(req["restart_epoch"])
            self.sealed_mode = bool(req.get("sealed_mode", False)) or os.environ.get("SEALED_MODE") == "1"
            if MEM0_AVAILABLE and self.client is None:
                self.client = Mem0Memory()
            return {
                "ok": True,
                "adapter": "mem0",
                "sdk_verified": MEM0_AVAILABLE,
                "unavailable_reason": MEM0_IMPORT_ERR,
            }

        if op == "restart_epoch":
            self.restart_epoch = int(req["restart_epoch"])
            self.sim_time = req["sim_time_rfc3339"]
            self.client = None
            self.ram_cleared_on_restart = True
            return {
                "ok": True,
                "restart_epoch": self.restart_epoch,
                "ram_cleared_on_restart": True,
                "sdk_verified": MEM0_AVAILABLE,
            }

        if op == "step":
            self.sim_time = req["sim_time_rfc3339"]
            self.restart_epoch = int(req["restart_epoch"])
            percept = req["percept"]
            assert_no_oracle_labels(percept)
            payload_text = percept.get("content_payload_utf8", "")
            check_sealed_barrier(self.sealed_mode, payload_text)

            if not MEM0_AVAILABLE:
                return {
                    "ok": False,
                    "adapter": "mem0",
                    "sdk_verified": False,
                    "unavailable_reason": MEM0_IMPORT_ERR,
                    "recorded_sim_time_rfc3339": self.sim_time,
                    "restart_epoch": self.restart_epoch,
                    "ram_cleared_on_restart": self.ram_cleared_on_restart,
                    "noop_tool_exposed": True,
                }

            user_id = percept["actor_id"]
            self.client.add(
                [{"role": "user", "content": payload_text}],
                user_id=user_id,
                metadata={"sim_time_rfc3339": self.sim_time, "place_id": percept["place_id"]},
            )
            _hits = self.client.search(payload_text, filters={"user_id": user_id})
            return {
                "ok": True,
                "adapter": "mem0",
                "sdk_verified": True,
                "decision": "noop",
                "action": None,
                "recorded_sim_time_rfc3339": self.sim_time,
                "restart_epoch": self.restart_epoch,
                "ram_cleared_on_restart": self.ram_cleared_on_restart,
                "noop_tool_exposed": True,
                "wake_invoked_llm": True,
            }

        if op == "shutdown":
            return {"ok": True}

        raise ValueError(f"unknown op: {op}")


if __name__ == "__main__":
    run_ndjson_loop(Mem0Adapter().handle)
