from __future__ import annotations

import json
from pathlib import Path

from monoagens_sdk import (
    BodyAdapter,
    HeartbeatScheduler,
    Waist2Request,
    sha256_hex,
)


class MockBody:
    def __init__(self) -> None:
        self.exec_count = 0

    def discover(self, params: dict) -> dict:
        return {
            "body_id": "body-py",
            "protocol_version": {"major": 1, "minor": 0},
            "capabilities": [],
            "places": ["cli:0"],
        }

    def execute(self, params: dict) -> dict:
        self.exec_count += 1
        return {
            "accepted": True,
            "receipt": {
                "action_id": params["action_id"],
                "capability_id": params["capability_id"],
                "outcome": "done",
                "termination_reason": "normal",
                "progress": 1.0,
                "delivered_units": 1,
                "timestamp_ms": 100,
            },
        }

    def cancel(self, params: dict) -> dict:
        return {
            "cancelled": True,
            "receipt": {
                "action_id": params["action_id"],
                "capability_id": "chat.speak",
                "outcome": "failed",
                "timestamp_ms": 101,
            },
        }

    def resolve_approval(self, params: dict) -> dict:
        return {"applied": True}


def test_smoke_round_trip_dedup_journal_blob_and_heartbeat() -> None:
    schema_path = Path(__file__).resolve().parents[3] / "crates/protocol/jsonschema/Waist2Request.schema.json"
    schema = json.loads(schema_path.read_text(encoding="utf-8"))

    req = Waist2Request(
        id=1,
        method="action/execute",
        params={
            "headers": {"idempotency_key": "idem-py-1", "protocol_version": {"major": 1, "minor": 0}},
            "action_id": "act-py-1",
            "capability_id": "chat.speak",
            "places": ["cli:0"],
            "audience": ["per_alice"],
            "payload": [104, 105],
        },
    )
    wire = req.to_json()
    parsed = Waist2Request.from_json(wire)
    assert parsed == req
    parsed_dict = json.loads(wire)
    for field in schema["required"]:
        assert field in parsed_dict
    assert parsed_dict["jsonrpc"] == schema["properties"]["jsonrpc"]["const"]
    assert parsed_dict["method"] in schema["$defs"]["Waist2Method"]["enum"]

    body = MockBody()
    adapter = BodyAdapter(body)
    for _ in range(3):
        resp = json.loads(adapter.handle_line(wire) or "{}")
        assert resp["result"]["accepted"] is True
    assert body.exec_count == 1

    q_resp = json.loads(
        adapter.handle_line(
            json.dumps(
                {
                    "jsonrpc": "2.0",
                    "id": 2,
                    "method": "state/query",
                    "params": {
                        "headers": {"idempotency_key": "idem-q", "protocol_version": {"major": 1, "minor": 0}},
                        "action_id": "act-py-1",
                    },
                }
            )
        )
        or "{}"
    )
    assert q_resp["result"]["status"]["state"] == "terminal"
    assert adapter.query_state("missing")["status"]["state"] == "unknown"

    bad_blob = json.loads(
        adapter.handle_line(
            json.dumps(
                {
                    "jsonrpc": "2.0",
                    "id": 3,
                    "method": "blob/put",
                    "params": {
                        "headers": {"idempotency_key": "idem-b", "protocol_version": {"major": 1, "minor": 0}},
                        "sha256": "0" * 64,
                        "byte_len": 3,
                        "inline_bytes": [97, 98, 99],
                    },
                }
            )
        )
        or "{}"
    )
    assert bad_blob["error"]["code"] == -32005
    assert sha256_hex(b"abc") == "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"

    hb = HeartbeatScheduler(1_000)
    assert hb.is_due(100) is True
    hb.next_request(4, "body-py", "ready", 100, "idem-hb")
    assert hb.is_due(500) is False
    assert hb.is_due(1_100) is True
