"""Identity: the birth certificate at birth, then `state: identity`."""

from __future__ import annotations

import asyncio
import json
from datetime import datetime, timezone

import httpx
import pytest

from monoagens.core import identity
from monoagens.infra.clock import FakeClock
from monoagens.infra.embedding import FakeEmbedder
from monoagens.core.kernel import Kernel
from monoagens.core.reasoning import Reasoning
from monoagens.infra.store import append_event, project_event, read_state, rebuild_state
from monoagens.types import Action, Event
from monoagens.boot import build
from monoagens.middleware import privacy
from tests.support import settings

SETTINGS = settings()
START = datetime(2026, 11, 10, 8, 0, tzinfo=timezone.utc)
PLACE = "space:tg:1"


class FakeLLM:
    """One scripted `act` call that changes the identity, then silence."""

    def __init__(self, arguments: dict) -> None:
        self.arguments, self.calls = arguments, 0

    def handler(self, request: httpx.Request) -> httpx.Response:
        self.calls += 1
        calls = [] if self.calls > 1 else [
            {"id": "call_1", "type": "function", "function": {
                "name": "act",
                "arguments": json.dumps({"capability": "self.set_identity",
                                         "arguments": self.arguments})}}]
        return httpx.Response(200, json={
            "choices": [{"message": {"tool_calls": calls}}],
            "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2}})


def _pfc(fake: FakeLLM) -> Reasoning:
    return Reasoning("https://llm.test/v1", "key", "fake-model", settings=SETTINGS,
               transport=httpx.MockTransport(fake.handler))


def _percept(who: str) -> Event:
    return Event(id=None, ts=START, kind="percept", who=who, place=PLACE, body="telegram",
                 payload={"description": "ubah dirimu", "addressed": True,
                          "place_kind": "direct"})


async def _feed(kernel: Kernel, event: Event) -> None:
    put = asyncio.create_task(kernel.put(event))
    await asyncio.sleep(0)
    await kernel.step()
    await kernel.run_until_idle()
    await asyncio.wait_for(put, 1.0)


async def _kinds(conn) -> list[str]:
    cur = await conn.execute("SELECT kind FROM events ORDER BY id")
    return [row[0] for row in await cur.fetchall()]


async def test_birth_writes_the_certificate_into_state_once(aconn):
    born = await identity.ensure(aconn, SETTINGS["identity"], now=START)

    assert born["name"] == "Mona"
    assert born["principles"] == ["Never lie."]
    assert await read_state(aconn, identity.KEY) == born
    assert await identity.ensure(aconn, {"name": "Other"}, now=START) == born
    assert (await _kinds(aconn)).count(identity.BORN) == 1


async def test_before_birth_identity_falls_back_to_the_certificate(aconn):
    assert await identity.load(aconn, SETTINGS["identity"]) == \
        identity.from_certificate(SETTINGS["identity"])


async def test_after_birth_identity_comes_from_state(aconn):
    await identity.ensure(aconn, SETTINGS["identity"], now=START)
    updated = {"name": "Mona 2", "language": "id", "persona": "Different.",
               "principles": []}
    wrote = await append_event(aconn, ts=START, kind=identity.UPDATED,
                               payload={"identity": updated})
    await project_event(aconn, wrote)

    assert await identity.load(aconn, {"name": "Stale"}) == updated


def test_self_tool_declares_an_identity_changing_capability():
    class Clock:
        def now(self):
            return START

    capability = identity.SelfTool(None, Clock(), visible=privacy.visible,
                                       memory_visible=privacy.memory_visible).capabilities()[0]
    assert capability["name"] == "set_identity"
    assert capability["identity_changing"] is True


async def test_a_non_owner_cannot_change_the_identity(aconn):
    await identity.ensure(aconn, SETTINGS["identity"], now=START)
    kernel = build(aconn, clock=FakeClock(START), pfc=_pfc(FakeLLM({"name": "Hacked"})),
                    settings=SETTINGS, embedder=FakeEmbedder())

    await _feed(kernel, _percept("person:other"))

    assert (await read_state(aconn, identity.KEY))["name"] == "Mona"
    stopped = await (await aconn.execute(
        "SELECT payload FROM events WHERE kind = 'guard.stopped'")).fetchone()
    assert stopped[0]["reason"] == "only the owner may invoke identity_changing capabilities"


async def test_the_owner_changes_the_identity_in_conversation(aconn):
    await identity.ensure(aconn, SETTINGS["identity"], now=START)
    fake = FakeLLM({"name": "Mona 2", "principles": ["Never lie.", "Be brief."]})
    kernel = build(aconn, clock=FakeClock(START), pfc=_pfc(fake), settings=SETTINGS,
                    embedder=FakeEmbedder())

    await _feed(kernel, _percept("person:owner"))

    assert (await read_state(aconn, identity.KEY))["name"] == "Mona 2"
    assert kernel.pfc.identity["principles"] == ["Never lie.", "Be brief."]
    assert identity.UPDATED in await _kinds(aconn)


async def test_set_identity_rejects_unknown_fields(aconn):
    await identity.ensure(aconn, SETTINGS["identity"], now=START)
    tool = identity.SelfTool(aconn, FakeClock(START), visible=privacy.visible,
                               memory_visible=privacy.memory_visible)
    action = Action(kind="set_identity", place=PLACE,
                    payload={"name": "Mona", "owner": "x"}, event_id=1)
    with pytest.raises(ValueError, match="unknown fields"):
        await tool.act(action)


async def test_identity_state_is_rebuilt_from_the_log(aconn):
    born = await identity.ensure(aconn, SETTINGS["identity"], now=START)

    await rebuild_state(aconn)

    assert await read_state(aconn, identity.KEY) == born
