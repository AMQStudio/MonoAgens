"""Batch tool calls: one round's results are awaited together, and one round wakes once."""

from __future__ import annotations

import asyncio
from datetime import datetime, timezone
from typing import Any

from monoagens.boot import build
from monoagens.core.reasoning import ToolCall
from monoagens.infra.clock import FakeClock
from monoagens.infra.store import append_event
from monoagens.types import Action, Event
from tests.support import open_book, settings

START = datetime(2026, 1, 1, 3, 0, tzinfo=timezone.utc)
PLACE = "space:tg:1001"
OTHER = "space:tg:-500"
TIMEOUT_MS = 2000


class Tool:
    """A tool with one fast and one slow capability."""

    name = "demo"

    def __init__(self) -> None:
        self.calls = 0

    def capabilities(self):
        return ({"name": "look", "description": "Look something up.",
                 "schema": {"type": "object", "properties": {"q": {"type": "string"}},
                            "required": ["q"]},
                 "communicative": False, "long_running": False, "identity_changing": False},
                {"name": "build", "description": "Build something.",
                 "schema": {"type": "object", "properties": {"q": {"type": "string"}},
                            "required": ["q"]},
                 "communicative": False, "long_running": True, "identity_changing": False})

    async def act(self, action: Action) -> Any:
        self.calls += 1
        return {"text": "hasil"}


class Voice:
    """A talking adapter, so a round can also speak."""

    name = "telegram"

    def __init__(self) -> None:
        self.actions = []

    def capabilities(self):
        return ({"name": "say", "description": "Speak.",
                 "schema": {"type": "object", "properties": {"text": {"type": "string"}},
                            "required": ["text"]},
                 "communicative": True, "long_running": False, "identity_changing": False,
                 "facts": (), "permission": None},)

    async def act(self, action: Action) -> None:
        self.actions.append(action)


class Scripted:
    """A reasoning that answers with one scripted round after another."""

    def __init__(self, rounds: list[list[tuple[str, dict]]]) -> None:
        self.rounds = list(rounds)
        self.contexts: list[Any] = []
        self.last_usage: dict[str, int] = {}

    async def decide(self, context: Any):
        self.contexts.append(context)
        found = self.rounds.pop(0) if self.rounds else []
        return [ToolCall(name, dict(arguments)) for name, arguments in found]

    async def aclose(self):
        return None


def _kernel(aconn, rounds, *, clock=None) -> tuple[Any, Scripted, Tool]:
    values = settings()
    values["constants"] = {**values["constants"], "batch_wake_timeout_ms": TIMEOUT_MS}
    pfc = Scripted(rounds)
    kernel = build(aconn, clock=clock or FakeClock(START), pfc=pfc, settings=values)
    tool = Tool()
    kernel.register_adapter(Voice())
    kernel.tools.add_tool(tool, {"look": {"facts": ["reads"], "permission": "demo.look"},
                                 "build": {"facts": ["reads"], "permission": "demo.build"}})
    kernel.roles = open_book("person:owner", "demo.look", "demo.build")
    return kernel, pfc, tool


async def _feed(kernel, *events: Event) -> None:
    """Put events and let the loop process them to completion."""
    for event in events:
        put = asyncio.create_task(kernel.put(event))
        await asyncio.sleep(0)
        await kernel.step()
        await put
    await kernel.run_until_idle()


def _round(place: str, count: int = 3, capability: str = "demo.look") -> list[tuple]:
    return [("act", {"capability": capability, "arguments": {"q": f"{index}"}})
            for index in range(count)]


async def _kinds(conn) -> list[str]:
    cur = await conn.execute("SELECT kind FROM events ORDER BY id")
    return [kind for (kind,) in await cur.fetchall()]


def _percept(place: str = PLACE, text: str = "kerjakan") -> Event:
    return Event(id=None, ts=START, kind="percept", who="person:owner", place=place,
                 body="telegram",
                 payload={"description": text, "place_kind": "direct", "addressed": True},
                 origin_actor="person:owner")


async def test_a_round_of_three_calls_wakes_once(aconn):
    kernel, pfc, tool = _kernel(aconn, [_round(PLACE), []])

    await _feed(kernel, _percept())

    kinds = await _kinds(aconn)
    assert kinds.count("model.call") == 2  # the round, then exactly one follow-up
    assert kinds.count("act.result") == 3
    assert tool.calls == 3
    assert len(pfc.contexts) == 2


async def test_two_places_never_count_each_other(aconn):
    kernel, _pfc, _tool = _kernel(aconn, [])
    first = await append_event(aconn, ts=START, kind="percept", who="person:owner",
                               place=PLACE, body="telegram",
                               payload={"description": "a", "place_kind": "direct"})
    second = await append_event(aconn, ts=START, kind="percept", who="person:owner",
                                place=OTHER, body="telegram",
                                payload={"description": "b", "place_kind": "direct"})
    kernel._batches[(first.id, PLACE)] = {"trigger": first, "remaining": 2,
                                          "deadline": START.replace(year=2027)}
    kernel._batches[(second.id, OTHER)] = {"trigger": second, "remaining": 1,
                                           "deadline": START.replace(year=2027)}

    async def result_for(round_event: Event, place: str) -> Event:
        out = await append_event(aconn, ts=START, kind="act.out", place=place,
                                 payload={"capability": "telegram.say",
                                          "arguments": {"text": "x"}}, caused_by=round_event.id)
        return Event(id=None, ts=START, kind="act.result", who="person:owner", place=place,
                     body="telegram", caused_by=out.id,
                     payload={"capability": "telegram.say", "result": {}})

    first_hold = await kernel._batch_holds(await result_for(first, PLACE))
    second_hold = await kernel._batch_holds(await result_for(second, OTHER))

    assert first_hold is True  # one more result is due in the first place
    assert second_hold is False  # the second place's round is complete
    assert kernel._batches[(first.id, PLACE)]["remaining"] == 1
    assert (second.id, OTHER) not in kernel._batches


async def test_a_late_result_is_flushed_when_the_timeout_passes(aconn):
    clock = FakeClock(START)
    kernel, _pfc, _tool = _kernel(aconn, [[], []], clock=clock)
    trigger = await append_event(aconn, ts=START, kind="percept", who="person:owner",
                                 place=PLACE, body="telegram",
                                 payload={"description": "kerjakan", "place_kind": "direct"})
    key = (int(trigger.id or 0), PLACE)
    kernel._batches[key] = {"trigger": trigger, "remaining": 1, "deadline": START}
    clock.advance(TIMEOUT_MS / 1000 + 1)

    await kernel._flush_batches(PLACE)
    await kernel.run_until_idle()

    assert key not in kernel._batches
    assert (await _kinds(aconn)).count("model.call") == 1


async def test_an_act_that_never_produced_a_result_is_not_counted(aconn):
    kernel, _pfc, _tool = _kernel(aconn, [])

    kernel._track_batch(type("Job", (), {"trigger": _percept(), "place": PLACE})(),
                        [({"name": "act", "arguments": {"capability": "demo.missing"}},
                          {})])

    assert kernel._batches == {}


async def test_a_slow_call_is_not_counted_and_wakes_on_its_own(aconn):
    kernel, pfc, tool = _kernel(aconn, [[
        ("act", {"capability": "demo.look", "arguments": {"q": "a"}}),
        ("act", {"capability": "demo.look", "arguments": {"q": "b"}}),
        ("act", {"capability": "demo.build", "arguments": {"q": "c"}}),
    ], []])

    await _feed(kernel, _percept())

    kinds = await _kinds(aconn)
    assert tool.calls == 3
    assert kinds.count("act.result") == 3
    assert kinds.count("model.call") >= 2
