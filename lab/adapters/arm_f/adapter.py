"""Out-of-process Full-Transcript Control (Arm F, A-16) BaselineAdapter."""

from __future__ import annotations

import json
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


class ArmFFullTranscriptAdapter:
    def __init__(self) -> None:
        self.scenario_id = ""
        self.run_seed = 1
        self.sim_time = ""
        self.restart_epoch = 0
        self.sealed_mode = False
        self.state_dir = ""
        self.ram_transcript: List[Dict[str, Any]] = []
        self.ram_cleared_on_restart = False

    def _checkpoint_path(self) -> str:
        return os.path.join(self.state_dir, "arm_f_transcript.json")

    def _load_persisted(self) -> List[Dict[str, Any]]:
        if self.sealed_mode or not self.state_dir:
            return []
        p = self._checkpoint_path()
        if os.path.exists(p):
            with open(p, "r", encoding="utf-8") as fh:
                return json.load(fh)
        return []

    def _save_persisted(self, items: List[Dict[str, Any]]) -> None:
        if self.sealed_mode or not self.state_dir:
            return
        os.makedirs(self.state_dir, exist_ok=True)
        with open(self._checkpoint_path(), "w", encoding="utf-8") as fh:
            json.dump(items, fh)

    def handle(self, req: Dict[str, Any]) -> Dict[str, Any]:
        op = req["op"]
        if op == "init":
            self.scenario_id = req["scenario_id"]
            self.run_seed = int(req["run_seed"])
            self.sim_time = req["sim_time_rfc3339"]
            self.restart_epoch = int(req["restart_epoch"])
            self.sealed_mode = bool(req.get("sealed_mode", False)) or os.environ.get("SEALED_MODE") == "1"
            self.state_dir = req.get("state_dir") or ""
            self.ram_transcript = self._load_persisted()
            return {"ok": True, "adapter": "arm_f", "sdk_verified": True}

        if op == "restart_epoch":
            self.restart_epoch = int(req["restart_epoch"])
            self.sim_time = req["sim_time_rfc3339"]
            self.ram_transcript.clear()
            self.ram_cleared_on_restart = True
            # Reload only persisted non-RAM store after clearing ephemeral RAM.
            self.ram_transcript = self._load_persisted()
            return {
                "ok": True,
                "restart_epoch": self.restart_epoch,
                "ram_cleared_on_restart": True,
                "persisted_items": len(self.ram_transcript),
            }

        if op == "step":
            self.sim_time = req["sim_time_rfc3339"]
            self.restart_epoch = int(req["restart_epoch"])
            percept = req["percept"]
            assert_no_oracle_labels(percept)
            payload_text = percept.get("content_payload_utf8", "")
            check_sealed_barrier(self.sealed_mode, payload_text)

            entry = {
                "event_id": percept["event_id"],
                "sim_time_rfc3339": self.sim_time,
                "actor_id": percept["actor_id"],
                "place_id": percept["place_id"],
                "len": len(payload_text),
            }
            self.ram_transcript.append(entry)
            self._save_persisted(self.ram_transcript)

            addressed = bool(percept.get("addressed_to_agent", False))
            caps = percept.get("capability_list", [])
            if addressed and caps:
                decision = "act"
                action = {
                    "capability_id": caps[0],
                    "place_id": percept["place_id"],
                    "payload_utf8": f"armf-reply@{self.sim_time}",
                }
                out_vis = 28
            else:
                decision = "noop"
                action = None
                out_vis = 8

            prefix_tokens = 180
            transcript_tokens = 45 * len(self.ram_transcript)
            usage = {
                "uncached_in": 45,
                "cache_read": prefix_tokens + max(0, transcript_tokens - 45),
                "cache_write": 45,
                "out_vis": out_vis,
                "reason": 16,
            }
            return {
                "ok": True,
                "adapter": "arm_f",
                "sdk_verified": True,
                "unavailable_reason": None,
                "decision": decision,
                "action": action,
                "recorded_sim_time_rfc3339": self.sim_time,
                "restart_epoch": self.restart_epoch,
                "ram_cleared_on_restart": self.ram_cleared_on_restart,
                "ram_items": len(self.ram_transcript),
                "noop_tool_exposed": True,
                "wake_invoked_llm": True,
                "usage": usage,
                "e_tok": compute_e_tok(usage),
                "c_share": compute_c_share(usage),
                "latency_ms": 22.0 + 0.5 * len(self.ram_transcript),
                "cost_usd": compute_e_tok(usage) * 1e-6,
            }

        if op == "shutdown":
            return {"ok": True}

        raise ValueError(f"unknown op: {op}")


if __name__ == "__main__":
    adapter = ArmFFullTranscriptAdapter()
    run_ndjson_loop(adapter.handle)
