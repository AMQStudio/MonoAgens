"""Thin Waist-2 (monoagens.body.v1) Python body SDK shim."""

from __future__ import annotations

from dataclasses import asdict, dataclass, field
import hashlib
import json
import sys
from typing import Any, Callable, Protocol, TextIO

OPENRPC_SCHEMA_PATH = "../../crates/protocol/openrpc/monoagens.body.v1.openrpc.json"
PROTOCOL_VERSION = {"major": 1, "minor": 0}
WAIST2_METHODS = (
    "body/discover",
    "body/heartbeat",
    "body/shutdown",
    "percept/emit",
    "action/execute",
    "action/cancel",
    "action/receipt",
    "approval/resolve",
    "question/answer",
    "state/query",
    "blob/put",
    "blob/get",
)


class RpcFailure(Exception):
    def __init__(self, code: int, message: str) -> None:
        super().__init__(message)
        self.code = code
        self.message = message


@dataclass(slots=True)
class Waist2Headers:
    idempotency_key: str
    protocol_version: dict[str, int] = field(default_factory=lambda: dict(PROTOCOL_VERSION))
    traceparent: str | None = None
    _meta: dict[str, Any] = field(default_factory=dict)


@dataclass(slots=True)
class Waist2Request:
    method: str
    params: dict[str, Any]
    id: int | str | None = None
    jsonrpc: str = "2.0"

    def to_json(self) -> str:
        return json.dumps(asdict(self), separators=(",", ":"))

    @classmethod
    def from_json(cls, raw: str) -> Waist2Request:
        data = json.loads(raw)
        return cls(
            jsonrpc=data["jsonrpc"],
            id=data.get("id"),
            method=data["method"],
            params=data["params"],
        )


class BodyHandler(Protocol):
    def discover(self, params: dict[str, Any]) -> dict[str, Any]: ...
    def execute(self, params: dict[str, Any]) -> dict[str, Any]: ...
    def cancel(self, params: dict[str, Any]) -> dict[str, Any]: ...
    def resolve_approval(self, params: dict[str, Any]) -> dict[str, Any]: ...


def sha256_hex(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def verify_sha256(expected: str, data: bytes) -> str:
    actual = sha256_hex(data)
    if actual.lower() != expected.lower():
        raise RpcFailure(-32005, f"ERR_HASH_MISMATCH: expected {expected}, got {actual}")
    return actual


class HeartbeatScheduler:
    def __init__(self, interval_ms: int) -> None:
        self.interval_ms = interval_ms
        self._last_sent_ms: int | None = None

    def is_due(self, now_ms: int) -> bool:
        return self._last_sent_ms is None or (now_ms - self._last_sent_ms) >= self.interval_ms

    def next_request(
        self, req_id: int | str, body_id: str, status: str, now_ms: int, key: str
    ) -> Waist2Request:
        self._last_sent_ms = now_ms
        return Waist2Request(
            id=req_id,
            method="body/heartbeat",
            params={
                "headers": {"idempotency_key": key, "protocol_version": dict(PROTOCOL_VERSION)},
                "body_id": body_id,
                "status": status,
                "timestamp_ms": now_ms,
            },
        )


class BodyAdapter:
    def __init__(
        self,
        handler: BodyHandler,
        on_percept: Callable[[dict[str, Any]], dict[str, Any]] | None = None,
        on_question: Callable[[dict[str, Any]], dict[str, Any]] | None = None,
        on_shutdown: Callable[[dict[str, Any]], dict[str, Any]] | None = None,
    ) -> None:
        self._handler = handler
        self._on_percept = on_percept
        self._on_question = on_question
        self._on_shutdown = on_shutdown
        self._idem: dict[str, Any] = {}
        self._journal: dict[str, dict[str, Any]] = {}
        self._blobs: dict[str, bytes] = {}

    def query_state(self, action_id: str) -> dict[str, Any]:
        return {"action_id": action_id, "status": self._journal.get(action_id, {"state": "unknown"})}

    def handle_line(self, line: str) -> str | None:
        if not line.strip():
            return None
        req = Waist2Request.from_json(line)
        try:
            return json.dumps({"jsonrpc": "2.0", "id": req.id, "result": self._dispatch(req)})
        except RpcFailure as exc:
            return json.dumps({"jsonrpc": "2.0", "id": req.id, "error": {"code": exc.code, "message": exc.message}})

    def run_stdio(self, inp: TextIO = sys.stdin, out: TextIO = sys.stdout) -> None:
        for line in inp:
            resp = self.handle_line(line)
            if resp is not None:
                out.write(resp + "\n")
                out.flush()

    def _dispatch(self, req: Waist2Request) -> Any:
        headers = req.params.get("headers") or {}
        ver = headers.get("protocol_version") or {}
        if not headers.get("idempotency_key") or ver.get("major") != 1 or ver.get("minor", 1) > 0:
            raise RpcFailure(-32004, "ERR_UNSUPPORTED_VERSION: unsupported protocol version")
        cache_key = f"{req.method}:{headers['idempotency_key']}"
        if cache_key in self._idem:
            return self._idem[cache_key]
        m, p = req.method, req.params
        if m == "body/discover":
            res = self._handler.discover(p)
        elif m == "body/heartbeat":
            res = {"acknowledged_at_ms": p["timestamp_ms"]}
        elif m == "body/shutdown":
            res = self._on_shutdown(p) if self._on_shutdown else {"accepted": True}
        elif m == "percept/emit":
            if not p.get("audience"):
                raise RpcFailure(-32006, "ERR_UNDECLARED_AUDIENCE: percept must declare a non-empty audience")
            res = self._on_percept(p) if self._on_percept else {"accepted": True}
        elif m == "action/execute":
            self._journal[p["action_id"]] = {"state": "in_flight"}
            res = self._handler.execute(p)
            if res.get("receipt"):
                self._journal[p["action_id"]] = {"state": "terminal", "receipt": res["receipt"]}
        elif m == "action/cancel":
            res = self._handler.cancel(p)
            if res.get("receipt"):
                self._journal[p["action_id"]] = {"state": "terminal", "receipt": res["receipt"]}
        elif m == "action/receipt":
            receipt = p["receipt"]
            self._journal[receipt["action_id"]] = {"state": "terminal", "receipt": receipt}
            res = {"recorded": True}
        elif m == "approval/resolve":
            res = self._handler.resolve_approval(p)
        elif m == "question/answer":
            res = self._on_question(p) if self._on_question else {"accepted": True}
        elif m == "state/query":
            res = self.query_state(p["action_id"])
        elif m == "blob/put":
            raw = bytes(p["inline_bytes"]) if p.get("inline_bytes") is not None else b""
            if p.get("inline_bytes") is not None and len(raw) != int(p["byte_len"]):
                raise RpcFailure(-32005, "ERR_HASH_MISMATCH: byte_len mismatch")
            digest = verify_sha256(p["sha256"], raw) if p.get("inline_bytes") is not None else p["sha256"]
            ref = f"blob://sha256/{digest}"
            self._blobs[ref] = raw
            res = {"blob_ref": ref, "sha256": digest, "expires_at_ms": 60_000}
        elif m == "blob/get":
            raw = self._blobs.get(p["blob_ref"], b"")
            digest = verify_sha256(p["expected_sha256"], raw) if p.get("expected_sha256") else sha256_hex(raw)
            res = {"blob_ref": p["blob_ref"], "sha256": digest, "byte_len": len(raw), "bytes": list(raw)}
        else:
            raise RpcFailure(-32001, f"ERR_UNKNOWN_CAPABILITY: unknown method {m}")
        self._idem[cache_key] = res
        return res
