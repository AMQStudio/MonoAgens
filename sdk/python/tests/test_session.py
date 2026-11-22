from __future__ import annotations

import asyncio
import json
import socket
import tempfile
from pathlib import Path

import pytest

from monoagens_sdk import (
    AsyncBodySession,
    BodyAdapter,
    RpcFailure,
    sha256_hex,
)


class AsyncMockBody:
    def discover(self, params: dict) -> dict:
        return {
            "body_id": "body-py-mux",
            "protocol_version": {"major": 1, "minor": 0},
            "capabilities": [],
            "places": ["tg:room-1"],
        }

    async def execute(self, params: dict) -> dict:
        await asyncio.sleep(0.005)
        return {
            "accepted": True,
            "receipt": {
                "action_id": params["action_id"],
                "capability_id": params["capability_id"],
                "outcome": "done",
                "termination_reason": "normal",
                "timestamp_ms": 200,
            },
        }

    def cancel(self, params: dict) -> dict:
        return {
            "cancelled": True,
            "receipt": {
                "action_id": params["action_id"],
                "capability_id": "chat.send",
                "outcome": "failed",
                "timestamp_ms": 201,
            },
        }

    def resolve_approval(self, params: dict) -> dict:
        return {"applied": True}


async def _make_socketpair_session(adapter: BodyAdapter):
    sock_body, sock_gw = socket.socketpair()
    body_r, body_w = await asyncio.open_connection(sock=sock_body)
    gw_r, gw_w = await asyncio.open_connection(sock=sock_gw)
    session = AsyncBodySession(adapter, body_r, body_w)
    return session, gw_r, gw_w


def test_async_session_bidirectional_multiplexing() -> None:
    async def run() -> None:
        session, gw_r, gw_w = await _make_socketpair_session(BodyAdapter(AsyncMockBody()))
        gw_seen_requests: list[dict] = []
        gw_responses: dict[int, dict] = {}
        blobs: dict[str, list[int]] = {}

        async def mock_gateway() -> None:
            while True:
                raw = await gw_r.readline()
                if not raw:
                    break
                msg = json.loads(raw.decode("utf-8"))
                if isinstance(msg.get("method"), str):
                    gw_seen_requests.append(msg)
                    method = msg["method"]
                    req_id = msg["id"]
                    if method == "percept/emit":
                        await asyncio.sleep(0.01)
                        resp = {"jsonrpc": "2.0", "id": req_id, "result": {"accepted": True}}
                    elif method == "action/receipt":
                        resp = {"jsonrpc": "2.0", "id": req_id, "result": {"recorded": True}}
                    elif method == "body/heartbeat":
                        resp = {"jsonrpc": "2.0", "id": req_id, "result": {"acknowledged_at_ms": 7777}}
                    elif method == "blob/put":
                        digest = msg["params"]["sha256"]
                        ref = f"blob://staged/{digest}"
                        blobs[ref] = msg["params"]["inline_bytes"]
                        resp = {
                            "jsonrpc": "2.0",
                            "id": req_id,
                            "result": {"blob_ref": ref, "sha256": digest, "expires_at_ms": 60_000},
                        }
                    elif method == "blob/get":
                        ref = msg["params"]["blob_ref"]
                        data = blobs.get(ref, [])
                        resp = {
                            "jsonrpc": "2.0",
                            "id": req_id,
                            "result": {
                                "blob_ref": ref,
                                "sha256": sha256_hex(bytes(data)),
                                "byte_len": len(data),
                                "bytes": data,
                            },
                        }
                    else:
                        resp = {
                            "jsonrpc": "2.0",
                            "id": req_id,
                            "error": {"code": -32601, "message": "unknown"},
                        }
                    gw_w.write((json.dumps(resp) + "\n").encode("utf-8"))
                    await gw_w.drain()
                else:
                    gw_responses[int(msg["id"])] = msg

        gw_task = asyncio.create_task(mock_gateway())

        p_percept = asyncio.create_task(
            session.emit_percept(
                actor="tg:user1",
                place="tg:room-1",
                audience=["tg:user1", "owner"],
                timestamp_ms=5000,
                payload=b"hello",
            )
        )
        p_receipt = asyncio.create_task(
            session.send_receipt(
                {
                    "action_id": "act-out-py",
                    "capability_id": "chat.send",
                    "outcome": "done",
                    "timestamp_ms": 5001,
                }
            )
        )
        p_hb = asyncio.create_task(session.send_heartbeat("body-py-mux", "ready", 5002))

        gw_w.write(
            (
                json.dumps(
                    {
                        "jsonrpc": "2.0",
                        "id": 101,
                        "method": "body/discover",
                        "params": {
                            "headers": {"idempotency_key": "gw-d1", "protocol_version": {"major": 1, "minor": 0}}
                        },
                    }
                )
                + "\n"
            ).encode("utf-8")
        )
        gw_w.write(
            (
                json.dumps(
                    {
                        "jsonrpc": "2.0",
                        "id": 102,
                        "method": "action/execute",
                        "params": {
                            "headers": {"idempotency_key": "gw-e1", "protocol_version": {"major": 1, "minor": 0}},
                            "action_id": "act-in-py",
                            "capability_id": "chat.send",
                            "places": ["tg:room-1"],
                            "audience": ["tg:user1"],
                            "payload": [111, 107],
                        },
                    }
                )
                + "\n"
            ).encode("utf-8")
        )
        await gw_w.drain()

        percept_res, receipt_res, hb_res = await asyncio.gather(p_percept, p_receipt, p_hb)
        assert percept_res == {"accepted": True}
        assert receipt_res == {"recorded": True}
        assert hb_res == {"acknowledged_at_ms": 7777}

        await asyncio.sleep(0.03)
        assert gw_responses[101]["result"]["body_id"] == "body-py-mux"
        assert gw_responses[102]["result"]["accepted"] is True

        emitted = next(r for r in gw_seen_requests if r["method"] == "percept/emit")
        assert emitted["params"]["actor"] == "tg:user1"
        assert emitted["params"]["audience"] == ["tg:user1", "owner"]
        assert emitted["params"]["payload"] == [104, 101, 108, 108, 111]
        assert emitted["params"]["taint"] == "untrusted"

        put_res = await session.put_blob(b"\x01\x02\x03", "audio/ogg", "scope-py")
        assert put_res["sha256"] == sha256_hex(b"\x01\x02\x03")
        put_req = next(r for r in gw_seen_requests if r["method"] == "blob/put")
        assert put_req["params"]["headers"]["_meta"] == {"content_type": "audio/ogg", "scope_id": "scope-py"}

        got = await session.get_blob(put_res["blob_ref"], put_res["sha256"])
        assert got["bytes"] == [1, 2, 3]
        with pytest.raises(RpcFailure) as err_info:
            await session.get_blob(put_res["blob_ref"], "f" * 64)
        assert err_info.value.code == -32005

        await session.close()
        gw_w.close()
        await gw_w.wait_closed()
        await gw_task

    asyncio.run(run())


def test_async_session_validation_errors_and_eof_cleanup() -> None:
    async def run() -> None:
        session, gw_r, gw_w = await _make_socketpair_session(BodyAdapter(AsyncMockBody()))

        with pytest.raises(RpcFailure) as exc_info:
            await session.emit_percept(
                actor="tg:1",
                place="tg:room-1",
                audience=[],
                timestamp_ms=1,
                payload=b"x",
            )
        assert exc_info.value.code == -32006

        async def gw_responder() -> None:
            while True:
                raw = await gw_r.readline()
                if not raw:
                    break
                msg = json.loads(raw.decode("utf-8"))
                if msg.get("method") == "body/heartbeat":
                    err_line = json.dumps(
                        {
                            "jsonrpc": "2.0",
                            "id": msg["id"],
                            "error": {"code": -32004, "message": "ERR_UNSUPPORTED_VERSION"},
                        }
                    )
                    gw_w.write((err_line + "\n").encode("utf-8"))
                    await gw_w.drain()
                elif msg.get("method") == "action/receipt":
                    # Simulate sudden gateway EOF before answering receipt.
                    gw_w.close()
                    await gw_w.wait_closed()
                    break

        gw_task = asyncio.create_task(gw_responder())

        with pytest.raises(RpcFailure) as rpc_err:
            await session.send_heartbeat("body-py-mux", "ready", 10)
        assert rpc_err.value.code == -32004

        with pytest.raises(RpcFailure) as eof_err:
            await session.send_receipt(
                {
                    "action_id": "act-eof",
                    "capability_id": "chat.send",
                    "outcome": "done",
                    "timestamp_ms": 20,
                }
            )
        assert eof_err.value.code == -32003
        assert session.is_closed is True

        await gw_task
        await session.close()

    asyncio.run(run())


def test_async_session_connect_unix() -> None:
    async def run() -> None:
        with tempfile.TemporaryDirectory() as tmp:
            sock_path = Path(tmp) / "body.sock"

            async def handle_client(reader: asyncio.StreamReader, writer: asyncio.StreamWriter) -> None:
                while True:
                    raw = await reader.readline()
                    if not raw:
                        break
                    msg = json.loads(raw.decode("utf-8"))
                    if msg.get("method") == "percept/emit":
                        resp = {"jsonrpc": "2.0", "id": msg["id"], "result": {"accepted": True}}
                        writer.write((json.dumps(resp) + "\n").encode("utf-8"))
                        await writer.drain()
                writer.close()

            server = await asyncio.start_unix_server(handle_client, path=str(sock_path))
            try:
                async with await AsyncBodySession.connect_unix(sock_path, BodyAdapter(AsyncMockBody())) as session:
                    res = await session.emit_percept(
                        {
                            "actor": "tg:9",
                            "place": "tg:room-9",
                            "audience": ["tg:9"],
                            "timestamp_ms": 55,
                            "payload": [65],
                        }
                    )
                    assert res == {"accepted": True}
            finally:
                server.close()
                await server.wait_closed()

    asyncio.run(run())
