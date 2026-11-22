"""Thin Waist-2 (monoagens.body.v1) Python body SDK shim."""

from __future__ import annotations

import asyncio
from dataclasses import asdict, dataclass, field
import hashlib
import inspect
import json
from pathlib import Path
import sys
import time
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
    def __init__(self, code: int, message: str, data: dict[str, Any] | None = None) -> None:
        super().__init__(message)
        self.code = code
        self.message = message
        self.data = data


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
    def discover(self, params: dict[str, Any]) -> Any: ...
    def execute(self, params: dict[str, Any]) -> Any: ...
    def cancel(self, params: dict[str, Any]) -> Any: ...
    def resolve_approval(self, params: dict[str, Any]) -> Any: ...


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
        on_percept: Callable[[dict[str, Any]], Any] | None = None,
        on_question: Callable[[dict[str, Any]], Any] | None = None,
        on_shutdown: Callable[[dict[str, Any]], Any] | None = None,
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

    async def handle_line_async(self, line: str) -> str | None:
        if not line.strip():
            return None
        req = Waist2Request.from_json(line)
        try:
            result = await self._dispatch_async(req)
            return json.dumps({"jsonrpc": "2.0", "id": req.id, "result": result})
        except RpcFailure as exc:
            return json.dumps({"jsonrpc": "2.0", "id": req.id, "error": {"code": exc.code, "message": exc.message}})

    def run_stdio(self, inp: TextIO = sys.stdin, out: TextIO = sys.stdout) -> None:
        for line in inp:
            resp = self.handle_line(line)
            if resp is not None:
                out.write(resp + "\n")
                out.flush()

    def _check_headers_and_cache(self, req: Waist2Request) -> tuple[str, Any] | tuple[str, None]:
        headers = req.params.get("headers") or {}
        ver = headers.get("protocol_version") or {}
        if not headers.get("idempotency_key") or ver.get("major") != 1 or ver.get("minor", 1) > 0:
            raise RpcFailure(-32004, "ERR_UNSUPPORTED_VERSION: unsupported protocol version")
        cache_key = f"{req.method}:{headers['idempotency_key']}"
        if cache_key in self._idem:
            return cache_key, self._idem[cache_key]
        return cache_key, None

    def _dispatch_sync_builtin(self, m: str, p: dict[str, Any]) -> Any:
        if m == "body/heartbeat":
            return {"acknowledged_at_ms": p["timestamp_ms"]}
        if m == "action/receipt":
            receipt = p["receipt"]
            self._journal[receipt["action_id"]] = {"state": "terminal", "receipt": receipt}
            return {"recorded": True}
        if m == "state/query":
            return self.query_state(p["action_id"])
        if m == "blob/put":
            raw = bytes(p["inline_bytes"]) if p.get("inline_bytes") is not None else b""
            if p.get("inline_bytes") is not None and len(raw) != int(p["byte_len"]):
                raise RpcFailure(-32005, "ERR_HASH_MISMATCH: byte_len mismatch")
            digest = verify_sha256(p["sha256"], raw) if p.get("inline_bytes") is not None else p["sha256"]
            ref = f"blob://sha256/{digest}"
            self._blobs[ref] = raw
            return {"blob_ref": ref, "sha256": digest, "expires_at_ms": 60_000}
        if m == "blob/get":
            raw = self._blobs.get(p["blob_ref"], b"")
            digest = verify_sha256(p["expected_sha256"], raw) if p.get("expected_sha256") else sha256_hex(raw)
            return {"blob_ref": p["blob_ref"], "sha256": digest, "byte_len": len(raw), "bytes": list(raw)}
        raise RpcFailure(-32001, f"ERR_UNKNOWN_CAPABILITY: unknown method {m}")

    def _dispatch(self, req: Waist2Request) -> Any:
        cache_key, cached = self._check_headers_and_cache(req)
        if cached is not None:
            return cached
        m, p = req.method, req.params
        if m == "body/discover":
            res = self._handler.discover(p)
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
        elif m == "approval/resolve":
            res = self._handler.resolve_approval(p)
        elif m == "question/answer":
            res = self._on_question(p) if self._on_question else {"accepted": True}
        else:
            res = self._dispatch_sync_builtin(m, p)
        self._idem[cache_key] = res
        return res

    async def _dispatch_async(self, req: Waist2Request) -> Any:
        cache_key, cached = self._check_headers_and_cache(req)
        if cached is not None:
            return cached
        m, p = req.method, req.params
        if m == "body/discover":
            res = self._handler.discover(p)
            if inspect.isawaitable(res):
                res = await res
        elif m == "body/shutdown":
            res = self._on_shutdown(p) if self._on_shutdown else {"accepted": True}
            if inspect.isawaitable(res):
                res = await res
        elif m == "percept/emit":
            if not p.get("audience"):
                raise RpcFailure(-32006, "ERR_UNDECLARED_AUDIENCE: percept must declare a non-empty audience")
            res = self._on_percept(p) if self._on_percept else {"accepted": True}
            if inspect.isawaitable(res):
                res = await res
        elif m == "action/execute":
            self._journal[p["action_id"]] = {"state": "in_flight"}
            res = self._handler.execute(p)
            if inspect.isawaitable(res):
                res = await res
            if res.get("receipt"):
                self._journal[p["action_id"]] = {"state": "terminal", "receipt": res["receipt"]}
        elif m == "action/cancel":
            res = self._handler.cancel(p)
            if inspect.isawaitable(res):
                res = await res
            if res.get("receipt"):
                self._journal[p["action_id"]] = {"state": "terminal", "receipt": res["receipt"]}
        elif m == "approval/resolve":
            res = self._handler.resolve_approval(p)
            if inspect.isawaitable(res):
                res = await res
        elif m == "question/answer":
            res = self._on_question(p) if self._on_question else {"accepted": True}
            if inspect.isawaitable(res):
                res = await res
        else:
            res = self._dispatch_sync_builtin(m, p)
        self._idem[cache_key] = res
        return res


def _rpc_key(rpc_id: int | str) -> tuple[str, int | str]:
    if isinstance(rpc_id, int) and not isinstance(rpc_id, bool):
        return ("int", rpc_id)
    return ("str", str(rpc_id))


class AsyncBodySession:
    """Bidirectional multiplexed Waist-2 NDJSON session over asyncio streams."""

    def __init__(
        self,
        adapter: BodyAdapter,
        reader: asyncio.StreamReader,
        writer: asyncio.StreamWriter,
        *,
        default_timeout: float | None = 30.0,
        id_prefix: str = "py",
    ) -> None:
        self._adapter = adapter
        self._reader = reader
        self._writer = writer
        self._default_timeout = default_timeout
        self._id_prefix = id_prefix
        self._next_rpc_id = 1
        self._next_idem_seq = 1
        self._pending: dict[tuple[str, int | str], asyncio.Future[Any]] = {}
        self._inbound_tasks: set[asyncio.Task[None]] = set()
        self._write_lock = asyncio.Lock()
        self._closed = False
        self._reader_task: asyncio.Task[None] | None = None
        try:
            asyncio.get_running_loop()
        except RuntimeError:
            pass
        else:
            self.start()

    @classmethod
    async def connect_unix(
        cls,
        socket_path: str | Path,
        adapter: BodyAdapter,
        *,
        default_timeout: float | None = 30.0,
        id_prefix: str = "py",
    ) -> AsyncBodySession:
        reader, writer = await asyncio.open_unix_connection(str(socket_path))
        session = cls(adapter, reader, writer, default_timeout=default_timeout, id_prefix=id_prefix)
        session.start()
        return session

    @classmethod
    async def from_stdio(
        cls,
        adapter: BodyAdapter,
        *,
        default_timeout: float | None = 30.0,
        id_prefix: str = "py",
    ) -> AsyncBodySession:
        loop = asyncio.get_running_loop()
        reader = asyncio.StreamReader()
        protocol = asyncio.StreamReaderProtocol(reader)
        await loop.connect_read_pipe(lambda: protocol, sys.stdin.buffer)
        w_transport, w_protocol = await loop.connect_write_pipe(
            lambda: asyncio.streams.FlowControlMixin(loop=loop),
            sys.stdout.buffer,
        )
        writer = asyncio.StreamWriter(w_transport, w_protocol, reader, loop)
        session = cls(adapter, reader, writer, default_timeout=default_timeout, id_prefix=id_prefix)
        session.start()
        return session

    def start(self) -> None:
        if self._reader_task is None and not self._closed:
            self._reader_task = asyncio.create_task(self._read_loop())

    @property
    def is_closed(self) -> bool:
        return self._closed

    async def __aenter__(self) -> AsyncBodySession:
        self.start()
        return self

    async def __aexit__(self, *_exc: object) -> None:
        await self.close()

    def _next_idempotency_key(self, tag: str) -> str:
        seq = self._next_idem_seq
        self._next_idem_seq += 1
        return f"{self._id_prefix}-{tag}-{seq}"

    def _build_headers(self, tag: str, existing: dict[str, Any] | None = None) -> dict[str, Any]:
        src = dict(existing or {})
        headers: dict[str, Any] = {
            "idempotency_key": src.get("idempotency_key") or self._next_idempotency_key(tag),
            "protocol_version": dict(src.get("protocol_version") or PROTOCOL_VERSION),
        }
        if src.get("traceparent") is not None:
            headers["traceparent"] = src["traceparent"]
        if src.get("_meta"):
            headers["_meta"] = dict(src["_meta"])
        return headers

    async def _write_line(self, line: str) -> None:
        if self._closed:
            raise RpcFailure(-32003, "body session is closed")
        async with self._write_lock:
            if self._closed:
                raise RpcFailure(-32003, "body session is closed")
            self._writer.write((line + "\n").encode("utf-8"))
            await self._writer.drain()

    async def _handle_inbound_request(self, obj: dict[str, Any], raw_line: str) -> None:
        has_id = "id" in obj and obj["id"] is not None
        try:
            out = await self._adapter.handle_line_async(raw_line)
            if out is not None and has_id and not self._closed:
                await self._write_line(out)
        except Exception as exc:  # noqa: BLE001
            if has_id and not self._closed:
                code = exc.code if isinstance(exc, RpcFailure) else -32003
                err_resp = json.dumps(
                    {
                        "jsonrpc": "2.0",
                        "id": obj["id"],
                        "error": {"code": code, "message": str(exc)},
                    },
                    separators=(",", ":"),
                )
                try:
                    await self._write_line(err_resp)
                except RpcFailure:
                    pass

    async def _read_loop(self) -> None:
        try:
            while not self._closed:
                raw = await self._reader.readline()
                if not raw:
                    break
                line = raw.decode("utf-8", errors="replace")
                if not line.strip():
                    continue
                try:
                    obj = json.loads(line)
                except json.JSONDecodeError as exc:
                    if not self._closed:
                        parse_err = json.dumps(
                            {
                                "jsonrpc": "2.0",
                                "id": None,
                                "error": {"code": -32700, "message": f"parse error: {exc}"},
                            },
                            separators=(",", ":"),
                        )
                        try:
                            await self._write_line(parse_err)
                        except RpcFailure:
                            break
                    continue

                if not isinstance(obj, dict):
                    continue

                if isinstance(obj.get("method"), str):
                    task = asyncio.create_task(self._handle_inbound_request(obj, line))
                    self._inbound_tasks.add(task)
                    task.add_done_callback(self._inbound_tasks.discard)
                elif obj.get("id") is not None:
                    key = _rpc_key(obj["id"])
                    fut = self._pending.pop(key, None)
                    if fut is not None and not fut.done():
                        err = obj.get("error")
                        if err is not None:
                            fut.set_exception(
                                RpcFailure(int(err["code"]), str(err["message"]), err.get("data"))
                            )
                        else:
                            fut.set_result(obj.get("result"))
        except (OSError, ConnectionError, asyncio.CancelledError):
            pass
        finally:
            self._fail_pending("body stream closed")

    def _fail_pending(self, reason: str) -> None:
        self._closed = True
        pending = list(self._pending.values())
        self._pending.clear()
        for fut in pending:
            if not fut.done():
                fut.set_exception(RpcFailure(-32003, reason))

    async def close(self, reason: str = "body session closed") -> None:
        if self._closed and self._reader_task is None:
            return
        self._fail_pending(reason)
        if self._reader_task is not None:
            self._reader_task.cancel()
            try:
                await self._reader_task
            except asyncio.CancelledError:
                pass
            self._reader_task = None
        for task in list(self._inbound_tasks):
            task.cancel()
        if self._inbound_tasks:
            await asyncio.gather(*self._inbound_tasks, return_exceptions=True)
            self._inbound_tasks.clear()
        self._writer.close()
        try:
            await self._writer.wait_closed()
        except OSError:
            pass

    async def call(
        self,
        method: str,
        params: dict[str, Any],
        *,
        timeout: float | None = ...,  # type: ignore[assignment]
    ) -> Any:
        self.start()
        if self._closed:
            raise RpcFailure(-32003, "body session is closed")
        req_id = self._next_rpc_id
        self._next_rpc_id += 1
        key = _rpc_key(req_id)

        headers = self._build_headers(method.replace("/", "-"), params.get("headers"))
        full_params = {**params, "headers": headers}
        wire = json.dumps(
            {"jsonrpc": "2.0", "id": req_id, "method": method, "params": full_params},
            separators=(",", ":"),
        )

        loop = asyncio.get_running_loop()
        fut: asyncio.Future[Any] = loop.create_future()
        self._pending[key] = fut
        try:
            await self._write_line(wire)
        except Exception:
            self._pending.pop(key, None)
            raise

        effective_timeout = self._default_timeout if timeout is ... else timeout
        try:
            if effective_timeout is not None:
                return await asyncio.wait_for(fut, timeout=effective_timeout)
            return await fut
        except TimeoutError as exc:
            self._pending.pop(key, None)
            raise RpcFailure(-32003, f"RPC '{method}' timed out after {effective_timeout}s") from exc
        except asyncio.CancelledError:
            self._pending.pop(key, None)
            raise

    async def emit_percept(
        self,
        params: dict[str, Any] | None = None,
        *,
        actor: str | None = None,
        place: str | None = None,
        audience: list[str] | None = None,
        timestamp_ms: int | None = None,
        payload: bytes | bytearray | list[int] | None = None,
        taint: str | None = None,
        idempotency_key: str | None = None,
        timeout: float | None = ...,  # type: ignore[assignment]
        **extra: Any,
    ) -> dict[str, Any]:
        body: dict[str, Any] = dict(params or {})
        if actor is not None:
            body["actor"] = actor
        if place is not None:
            body["place"] = place
        if audience is not None:
            body["audience"] = audience
        if timestamp_ms is not None:
            body["timestamp_ms"] = timestamp_ms
        if payload is not None:
            body["payload"] = payload
        if taint is not None:
            body["taint"] = taint
        elif "taint" not in body:
            body["taint"] = "untrusted"
        body.update(extra)

        aud = body.get("audience")
        if not isinstance(aud, list) or len(aud) == 0:
            raise RpcFailure(-32006, "ERR_UNDECLARED_AUDIENCE: percept must declare a non-empty audience")

        raw_payload = body.get("payload")
        if isinstance(raw_payload, (bytes, bytearray)):
            body["payload"] = list(raw_payload)

        if idempotency_key is not None:
            headers = dict(body.get("headers") or {})
            headers["idempotency_key"] = idempotency_key
            body["headers"] = headers

        return await self.call("percept/emit", body, timeout=timeout)

    async def send_receipt(
        self,
        receipt: dict[str, Any],
        idempotency_key: str | None = None,
        *,
        timeout: float | None = ...,  # type: ignore[assignment]
    ) -> dict[str, Any]:
        headers = self._build_headers(
            "receipt",
            {"idempotency_key": idempotency_key} if idempotency_key is not None else None,
        )
        return await self.call("action/receipt", {"headers": headers, "receipt": receipt}, timeout=timeout)

    async def send_heartbeat(
        self,
        body_id: str,
        status: str,
        timestamp_ms: int | None = None,
        *,
        timeout: float | None = ...,  # type: ignore[assignment]
    ) -> dict[str, Any]:
        ts = int(time.time() * 1000) if timestamp_ms is None else timestamp_ms
        return await self.call(
            "body/heartbeat",
            {"body_id": body_id, "status": status, "timestamp_ms": ts},
            timeout=timeout,
        )

    async def put_blob(
        self,
        data: bytes,
        content_type: str | None = None,
        scope_id: str | None = None,
        *,
        timeout: float | None = ...,  # type: ignore[assignment]
    ) -> dict[str, Any]:
        digest = sha256_hex(data)
        meta: dict[str, Any] = {}
        if content_type is not None:
            meta["content_type"] = content_type
        if scope_id is not None:
            meta["scope_id"] = scope_id
        headers = self._build_headers("blob-put", {"_meta": meta} if meta else None)
        return await self.call(
            "blob/put",
            {
                "headers": headers,
                "sha256": digest,
                "byte_len": len(data),
                "inline_bytes": list(data),
            },
            timeout=timeout,
        )

    async def get_blob(
        self,
        blob_ref: str,
        expected_sha256: str | None = None,
        *,
        timeout: float | None = ...,  # type: ignore[assignment]
    ) -> dict[str, Any]:
        params: dict[str, Any] = {"blob_ref": blob_ref}
        if expected_sha256 is not None:
            params["expected_sha256"] = expected_sha256
        res = await self.call("blob/get", params, timeout=timeout)
        raw_bytes = res.get("bytes") or []
        if expected_sha256 is not None and str(res.get("sha256", "")).lower() != expected_sha256.lower():
            raise RpcFailure(
                -32005,
                f"ERR_HASH_MISMATCH: expected {expected_sha256}, got {res.get('sha256')}",
            )
        verify_sha256(expected_sha256 or str(res["sha256"]), bytes(raw_bytes))
        return {
            "blob_ref": res["blob_ref"],
            "sha256": res["sha256"],
            "byte_len": res["byte_len"],
            "bytes": list(raw_bytes),
        }
