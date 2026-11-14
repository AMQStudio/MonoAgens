"""Supervisor tests: a failing adapter records an error and restarts without stopping the soul."""

from __future__ import annotations

import asyncio
import contextlib
from datetime import datetime, timezone
from typing import Any

import httpx

from monoagens.__main__ import supervise_adapter
from monoagens.adapters import telegram as telegram_adapter
from monoagens.boot import build
from monoagens.infra import config
from monoagens.infra.clock import FakeClock
from monoagens.types import Action, Event
from tests.support import settings

START = datetime(2026, 11, 13, 16, 0, tzinfo=timezone.utc)


class _HealthyAdapter:
    """Adapter that emits one live percept and stays alive until cancelled."""

    name = "cli"
    shows = ("typing",)
    reads = ("text",)
    gaps = {}

    def __init__(self) -> None:
        self.starts = 0
        self.emitted = asyncio.Event()

    def capabilities(self) -> tuple[dict[str, Any], ...]:
        return ({"name": "say", "description": "Speak.",
                 "schema": {"type": "object", "properties": {"text": {"type": "string"}},
                            "required": ["text"]},
                 "communicative": True, "long_running": False,
                 "identity_changing": False, "facts": (), "permission": None},)

    async def act(self, action: Action) -> Any:
        del action
        return None

    async def run(self, put) -> None:
        self.starts += 1
        await put(Event(id=None, ts=START, kind="percept", who="person:owner",
                        place="space:cli", body=self.name,
                        payload={"description": "masih hidup", "place_kind": "direct"}, description="masih hidup"))
        self.emitted.set()
        await asyncio.Future()


class _CrashingAdapter:
    """Adapter whose `run()` raises immediately N times, then stays healthy."""

    name = "discord"
    shows = ("typing",)
    reads = ("text",)
    gaps = {}

    def __init__(self, fail_times: int = 2) -> None:
        self.fail_times = fail_times
        self.attempts = 0
        self.recovered = asyncio.Event()

    def capabilities(self) -> tuple[dict[str, Any], ...]:
        return ({"name": "say", "description": "Speak.",
                 "schema": {"type": "object", "properties": {"text": {"type": "string"}},
                            "required": ["text"]},
                 "communicative": True, "long_running": False,
                 "identity_changing": False, "facts": (), "permission": None},)

    async def act(self, action: Action) -> Any:
        del action
        return None

    async def run(self, put) -> None:
        self.attempts += 1
        if self.attempts <= self.fail_times:
            raise RuntimeError(f"boom attempt {self.attempts}")
        await put(Event(id=None, ts=START, kind="percept", who="discord:1",
                        place="space:dc:1", body=self.name,
                        payload={"description": "pulih", "place_kind": "shared"}, description="pulih"))
        self.recovered.set()
        await asyncio.Future()


async def test_crashing_adapter_records_error_restarts_alone_and_keeps_others_running(aconn):
    values = settings(runtime={
        "tick_seconds": 30,
        "sleep": {"idle_minutes": 30, "retry_minutes": 30, "hour": 2},
        "adapter_retry": {"initial_seconds": 0.01, "max_seconds": 0.05, "factor": 2.0},
    })
    assert config.adapter_retry(values) == {
        "initial_seconds": 0.01, "max_seconds": 0.05, "factor": 2.0,
    }
    kernel = build(aconn, clock=FakeClock(START), settings=values)
    failing = _CrashingAdapter(fail_times=2)
    healthy = _HealthyAdapter()
    kernel.register_adapter(failing)
    kernel.register_adapter(healthy)

    stop = asyncio.Event()
    tasks = [
        asyncio.create_task(kernel.run(stop)),
        asyncio.create_task(supervise_adapter(kernel, failing, stop)),
        asyncio.create_task(supervise_adapter(kernel, healthy, stop)),
    ]
    try:
        await asyncio.wait_for(healthy.emitted.wait(), timeout=2.0)
        await asyncio.wait_for(failing.recovered.wait(), timeout=2.0)
        await asyncio.sleep(0.05)
    finally:
        stop.set()
        for task in tasks:
            task.cancel()
        await asyncio.gather(*tasks, return_exceptions=True)

    assert healthy.starts == 1
    assert failing.attempts == 3

    cur = await aconn.execute(
        "SELECT kind, body, payload FROM events ORDER BY id")
    rows = await cur.fetchall()
    errors = [(body, payload) for kind, body, payload in rows if kind == "error"]
    percepts = [(body, payload["description"]) for kind, body, payload in rows if kind == "percept"]
    assert len(errors) == 2
    assert all(body == "discord" and payload["adapter"] == "discord" for body, payload in errors)
    assert all("boom attempt" in payload["error"] for _, payload in errors)
    assert ("cli", "masih hidup") in percepts
    assert ("discord", "pulih") in percepts


async def test_telegram_getme_timeout_retries_via_supervisor_until_connected(aconn):
    values = settings(runtime={
        "tick_seconds": 30,
        "sleep": {"idle_minutes": 30, "retry_minutes": 30, "hour": 2},
        "adapter_retry": {"initial_seconds": 0.01, "max_seconds": 0.04, "factor": 2.0},
    })
    kernel = build(aconn, clock=FakeClock(START), settings=values)
    calls: list[str] = []
    delivered = asyncio.Event()

    def handler(request: httpx.Request) -> httpx.Response:
        method = request.url.path.rsplit("/", 1)[-1]
        calls.append(method)
        if method == "getMe" and calls.count("getMe") <= 2:
            raise httpx.ConnectTimeout("timed out connecting to Telegram")
        if method == "getMe":
            return httpx.Response(200, json={
                "ok": True, "result": {"id": 999, "is_bot": True, "username": "mona_bot"},
            })
        if method == "getUpdates":
            if calls.count("getUpdates") == 1:
                return httpx.Response(200, json={"ok": True, "result": [{
                    "update_id": 1,
                    "message": {
                        "message_id": 10,
                        "date": int(START.timestamp()),
                        "chat": {"id": 42, "type": "private"},
                        "from": {"id": 42, "first_name": "Budi"},
                        "text": "halo sesudah timeout",
                    },
                }]})
            delivered.set()
            return httpx.Response(200, json={"ok": True, "result": []})
        return httpx.Response(200, json={"ok": True, "result": True})

    body = telegram_adapter.TelegramBody(
        "tok", clock=FakeClock(START), poll_timeout=1,
        retry_base=0.01, transport=httpx.MockTransport(handler),
    )
    kernel.register_adapter(body)
    stop = asyncio.Event()
    tasks = [
        asyncio.create_task(kernel.run(stop)),
        asyncio.create_task(supervise_adapter(kernel, body, stop)),
    ]
    try:
        await asyncio.wait_for(delivered.wait(), timeout=2.0)
        await asyncio.sleep(0.05)
    finally:
        stop.set()
        for task in tasks:
            task.cancel()
        await asyncio.gather(*tasks, return_exceptions=True)
        with contextlib.suppress(Exception):
            await body.aclose()

    assert calls.count("getMe") == 3
    cur = await aconn.execute("SELECT kind, body, payload FROM events ORDER BY id")
    rows = await cur.fetchall()
    errors = [payload for kind, body, payload in rows if kind == "error" and body == "telegram"]
    assert len(errors) == 2
    assert any("ConnectTimeout" in err["error"] for err in errors)
    percepts = [payload["description"] for kind, body, payload in rows
                if kind == "percept" and body == "telegram"]
    assert "halo sesudah timeout" in percepts
