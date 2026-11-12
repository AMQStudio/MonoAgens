"""Tests for the free-text wake gate: cosine filter and the reminder wake."""

from __future__ import annotations

import asyncio
import json
from datetime import datetime, timedelta, timezone

import httpx

from monoagens import expect
from monoagens.clock import FakeClock
from monoagens.embedding import FakeEmbedder
from monoagens.kernel import Kernel
from monoagens.pfc import PFC
from monoagens.store import append_event, read_expectations
from tests.support import settings

SETTINGS = settings()
START = datetime(2026, 11, 10, 23, 0, tzinfo=timezone.utc)  # 06:00 in Jakarta
PLACE = "space:tg:1001"
GROUP = "space:tg:-1004"
WHO = "person:owner"
OTHER = "person:tg:7001"


class FakeLLM:
    """Scripted endpoint: recorded answers per request."""

    def __init__(self, responses: list[list[tuple[str, dict]]] | None = None) -> None:
        """Start with the scripted answers and no requests."""
        self.responses = responses or [[]]
        self.requests: list[dict] = []

    def handler(self, request: httpx.Request) -> httpx.Response:
        self.requests.append(json.loads(request.content))
        calls = self.responses[min(len(self.requests) - 1, len(self.responses) - 1)]
        tool_calls = [{"id": f"call_{i}", "type": "function",
                       "function": {"name": name, "arguments": json.dumps(arguments)}}
                      for i, (name, arguments) in enumerate(calls)]
        return httpx.Response(200, json={
            "choices": [{"message": {"role": "assistant", "tool_calls": tool_calls}}]})


def _pfc(fake: FakeLLM) -> PFC:
    return PFC("https://llm.test/v1", "test-key", "fake-model", settings=SETTINGS,
               transport=httpx.MockTransport(fake.handler))


class RecordingBody:
    """A body that records every action dispatched to it."""

    name = "telegram"

    def __init__(self) -> None:
        """Start with an empty action list."""
        self.actions = []

    def capabilities(self):
        from monoagens.bodies.telegram import CAPABILITIES

        return CAPABILITIES

    async def act(self, action) -> None:
        """Record one action."""
        self.actions.append(action)


class Harness:
    """A kernel, its scripted model, and the action sink of one test."""

    def __init__(self, conn, *, pfc: FakeLLM | None = None) -> None:
        """Wire the kernel with a fake embedder and, when given, a model."""
        self.fake = pfc
        self.kernel = Kernel(conn, clock=FakeClock(START),
                             pfc=_pfc(pfc) if pfc is not None else None,
                             settings=SETTINGS, embedder=FakeEmbedder())
        self.body = RecordingBody()
        self.kernel.register_body(self.body)

    async def feed(self, event) -> None:
        """Put one event through the loop and every job it starts."""
        put = asyncio.create_task(self.kernel.put(event))
        await asyncio.sleep(0)
        await self.kernel.step()
        await self.kernel.run_until_idle()
        await asyncio.wait_for(put, 1.0)


def _percept(text: str, *, place: str = GROUP, who: str = OTHER) -> object:
    from monoagens.types import Event

    return Event(id=None, ts=START, kind="percept", who=who, place=place, body="telegram",
                 payload={"description": text, "addressed": False, "place_kind": "shared"})


async def _promise(conn, *, text: str, expires: datetime | None = None) -> int:
    """Create one free-text promise through the module API."""
    source = await append_event(conn, ts=START, kind="percept", who=WHO, place=PLACE,
                                body="telegram", payload={"description": "minta dikabari"})
    data = {"about": f"kabari kalau {text}", "kind": "promise",
            "cue_event": {"kind": "percept", "place": GROUP}, "expect": {"text": text},
            "intent": "beri tahu owner", "weight": 0.8, "policy": {"once": True}}
    if expires is not None:
        data["expires_at"] = expires.isoformat()
    created = await expect.create(conn, data, now=START, settings=SETTINGS, source=source)
    return int(created.payload["expectation_id"])


async def _kinds(conn, name: str) -> list[dict]:
    cur = await conn.execute("SELECT payload FROM events WHERE kind = %s ORDER BY id", (name,))
    return [row[0] for row in await cur.fetchall()]


async def _status(conn, exp_id: int) -> str:
    rows = await read_expectations(conn, "ex.id = %s", (exp_id,))
    return rows[0]["status"]


async def test_a_line_below_lo_wakes_no_one(aconn):
    exp_id = await _promise(aconn, text="server down")
    harness = Harness(aconn, pfc=FakeLLM())

    await harness.feed(_percept("deploy staging aman"))

    assert harness.fake.requests == []  # no PFC wake
    assert await _kinds(aconn, "expectation.fired") == []
    assert await _status(aconn, exp_id) == "active"


async def test_a_line_at_or_above_lo_wakes_the_pfc_once_with_the_line(aconn):
    exp_id = await _promise(aconn, text="server down")
    harness = Harness(aconn, pfc=FakeLLM([[("act", {
        "capability": "telegram.say", "arguments": {"text": "Server down, owner."}})]]))
    line = "server down lagi nih guys"

    await harness.feed(_percept(line))

    assert len(harness.fake.requests) == 1  # exactly one wake
    assert line in harness.fake.requests[0]["messages"][2]["content"]
    reminders = await _kinds(aconn, "expectation.fired")
    assert len(reminders) == 1
    assert reminders[0]["reminder"] is True
    assert reminders[0]["expectation_id"] == exp_id
    assert reminders[0]["text"] == line
    assert await _status(aconn, exp_id) == "active"  # the PFC may still let it be
    assert [action.payload["text"] for action in harness.body.actions] == ["Server down, owner."]


async def test_a_fulfilled_promise_wakes_no_one(aconn):
    exp_id = await _promise(aconn, text="server down")
    await expect.resolve(aconn, exp_id, "satisfied", now=START)
    harness = Harness(aconn, pfc=FakeLLM())

    await harness.feed(_percept("server down lagi nih guys"))

    assert harness.fake.requests == []
    assert await _kinds(aconn, "expectation.fired") == []
    assert await _status(aconn, exp_id) == "satisfied"


async def test_a_stale_expectation_drops_on_the_wake_gate(aconn):
    exp_id = await _promise(aconn, text="server down", expires=START - timedelta(minutes=1))
    harness = Harness(aconn)

    await harness.kernel.clock.tick(harness.kernel.put)
    await harness.kernel.step()
    await harness.kernel.run_until_idle()

    assert await _status(aconn, exp_id) == "dropped"
    assert await _kinds(aconn, "expectation.fired") == []
