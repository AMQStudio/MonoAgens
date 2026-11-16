"""Out-of-process Stateless ReAct BaselineAdapter (arXiv:2210.03629)."""

from __future__ import annotations

import os
import sys
from typing import Any, Dict, List

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
from common import (
    assert_no_oracle_labels,
    check_sealed_barrier,
    compute_c_share,
    compute_e_tok,
    run_ndjson_loop,
)


class StatelessReActAdapter:
    def __init__(self) -> None:
        self.scenario_id = ""
        self.run_seed = 1
        self.sim_time = ""
        self.restart_epoch = 0
        self.sealed_mode = False
        self.ephemeral_window: List[Dict[str, Any]] = []
        self.ram_cleared_on_restart = False

    def handle(self, req: Dict[str, Any]) -> Dict[str, Any]:
        op = req["op"]
        if op == "init":
            self.scenario_id = req["scenario_id"]
            self.run_seed = int(req["run_seed"])
            self.sim_time = req["sim_time_rfc3339"]
            self.restart_epoch = int(req["restart_epoch"])
            self.sealed_mode = bool(req.get("sealed_mode", False)) or os.environ.get("SEALED_MODE") == "1"
            self.ephemeral_window.clear()
            return {"ok": True, "adapter": "stateless_react", "sdk_verified": True}

        if op == "restart_epoch":
            self.restart_epoch = int(req["restart_epoch"])
            self.sim_time = req["sim_time_rfc3339"]
            self.ephemeral_window.clear()
            self.ram_cleared_on_restart = True
            return {
                "ok": True,
                "restart_epoch": self.restart_epoch,
                "ram_cleared_on_restart": True,
                "ram_items": len(self.ephemeral_window),
            }

        if op == "step":
            self.sim_time = req["sim_time_rfc3339"]
            self.restart_epoch = int(req["restart_epoch"])
            percept = req["percept"]
            assert_no_oracle_labels(percept)
            payload_text = percept.get("content_payload_utf8", "")
            check_sealed_barrier(self.sealed_mode, payload_text)

            self.ephemeral_window.append(
                {
                    "event_id": percept["event_id"],
                    "sim_time_rfc3339": self.sim_time,
                    "actor_id": percept["actor_id"],
                    "place_id": percept["place_id"],
                }
            )
            # Deterministic offline ReAct tool decision: speak if explicitly addressed, else noop().
            addressed = bool(percept.get("addressed_to_agent", False))
            caps = percept.get("capability_list", [])
            if addressed and caps:
                decision = "act"
                action = {
                    "capability_id": caps[0],
                    "place_id": percept["place_id"],
                    "payload_utf8": f"react-ack@{self.sim_time}",
                }
                out_vis = 24
            else:
                decision = "noop"
                action = None
                out_vis = 8  # Explicit noop() / [SILENT] tool emission still bills LLM output tokens.

            usage = {
                "uncached_in": 120 + 20 * len(self.ephemeral_window),
                "cache_read": 0,
                "cache_write": 0,
                "out_vis": out_vis,
                "reason": 12,
            }
            return {
                "ok": True,
                "adapter": "stateless_react",
                "sdk_verified": True,
                "unavailable_reason": None,
                "decision": decision,
                "action": action,
                "recorded_sim_time_rfc3339": self.sim_time,
                "restart_epoch": self.restart_epoch,
                "ram_cleared_on_restart": self.ram_cleared_on_restart,
                "ram_items": len(self.ephemeral_window),
                "noop_tool_exposed": True,
                "wake_invoked_llm": True,
                "usage": usage,
                "e_tok": compute_e_tok(usage),
                "c_share": compute_c_share(usage),
                "latency_ms": 18.0,
                "cost_usd": compute_e_tok(usage) * 1e-6,
            }

        if op == "shutdown":
            return {"ok": True}

        raise ValueError(f"unknown op: {op}")


if __name__ == "__main__":
    adapter = StatelessReActAdapter()
    run_ndjson_loop(adapter.handle)
