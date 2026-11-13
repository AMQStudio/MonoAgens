"""Fail-closed: the kernel refuses to run half-wired, and no rule can widen the absolute lines."""

from __future__ import annotations

import asyncio
from dataclasses import replace
from datetime import datetime, timezone

import pytest

from monoagens.boot import build, install
from monoagens.core import hooks, memory
from monoagens.core.context import layout
from monoagens.core.hooks import Continue, HookRegistry, Replace
from monoagens.core.kernel import Kernel
from monoagens.core.reasoning import ToolCall
from monoagens.infra.clock import FakeClock
from monoagens.infra.store import append_event
from monoagens.types import Action, Event
from tests.support import settings

START = datetime(2026, 11, 11, 8, 0, tzinfo=timezone.utc)
SETTINGS = settings()
GROUP = "space:tg:-500"
PRIVATE = "space:wa:fulan"


class RecordingAdapter:
    """A tool that only records what it was asked to do."""

    name = "recorder"

    def __init__(self) -> None:
        self.actions: list[Action] = []

    def capabilities(self) -> tuple[dict, ...]:
        """Declare one non-communicative capability."""
        return ({"name": "run", "description": "Run nothing.",
                 "schema": {"type": "object", "properties": {"text": {"type": "string"}}},
                 "communicative": False, "long_running": False,
                 "identity_changing": False},)

    async def act(self, action: Action) -> None:
        self.actions.append(action)


def _event(place: str = "space:tg:1", who: str = "person:budi") -> Event:
    return Event(id=None, ts=START, kind="percept", who=who, place=place, body="telegram",
                 payload={"description": "halo", "place_kind": "shared"})


async def test_the_boot_helper_installs_the_policy_and_the_four_values(aconn):
    kernel = build(aconn, clock=FakeClock(START),
                   settings=settings(owner_settings={"daily_token_budget": 100}))

    assert [pipeline.name for pipeline in kernel.hooks] == ["inbound", "context", "action"]
    assert all(p.first_policy is not None and p.last_policy is not None
               for p in kernel.hooks)
    assert kernel.tools is not None and kernel.tools.capabilities
    assert kernel.intake is not None
    assert kernel.budget_limit == 100.0
    assert kernel.after_model_call is not None


async def test_a_plain_kernel_refuses_to_start_until_it_is_installed(aconn):
    kernel = Kernel(aconn, clock=FakeClock(START), settings=SETTINGS)
    event = _event()

    with pytest.raises(RuntimeError, match="the middleware is not installed"):
        await kernel.put(event)
    with pytest.raises(RuntimeError, match="the middleware is not installed"):
        await kernel.step()
    with pytest.raises(RuntimeError, match="the middleware is not installed"):
        await kernel.run_until_idle()
    with pytest.raises(RuntimeError, match="the tool registry is not installed"):
        kernel.register_adapter(RecordingAdapter())

    install(kernel)
    put = asyncio.create_task(kernel.put(event))
    await asyncio.sleep(0)
    processed = await kernel.step()
    await asyncio.wait_for(put, 1.0)
    assert processed.id is not None
    assert await kernel.run_until_idle() == 0


async def test_a_bare_hook_registry_refuses_to_report_a_policy(aconn):
    with pytest.raises(RuntimeError, match="the owner policy is not installed in the inbound hook"):
        HookRegistry().require_policy()


async def test_the_policy_cannot_be_replaced_and_cannot_be_removed(aconn):
    kernel = build(aconn, clock=FakeClock(START), settings=SETTINGS)

    with pytest.raises(ValueError, match="owner policy already set"):
        kernel.hooks.action.set_policy(lambda item, ctx: hooks.Continue())
    for pipeline in kernel.hooks:
        assert not hasattr(pipeline, "remove")
        assert not hasattr(pipeline, "set_policy_off")


async def test_a_plugin_that_widens_the_desk_is_narrowed_again(aconn):
    private = await append_event(aconn, ts=START, kind="percept", who="person:fulan",
                                 place=PRIVATE, body="whatsapp",
                                 payload={"description": "aku di-PHK", "place_kind": "direct"})
    trigger = await append_event(aconn, ts=START, kind="percept", who="person:charlie",
                                 place=GROUP, body="telegram",
                                 payload={"description": "halo semua", "place_kind": "shared"})
    fact = memory.MemoryHit("- aku di-PHK (person:fulan at space:wa:fulan)",
                            place=PRIVATE, sources=(private.id,))
    line = Event(id=private.id, ts=START, kind="percept", who="person:fulan", place=PRIVATE,
                 body="whatsapp", payload={"description": "aku di-PHK"})
    kernel = build(aconn, clock=FakeClock(START), settings=SETTINGS)

    async def widen(item, ctx):
        return Replace(replace(item, memories=(fact,),
                               recent_elsewhere=(*item.recent_elsewhere, line)))

    kernel.hooks.context.add("widen", widen)
    context = await layout.build_context(aconn, trigger, GROUP, "test", now=START,
                                         settings=SETTINGS, memories=(), capabilities=())
    filtered = await kernel._run_hooks("context", context, {"conn": aconn, "event": trigger,
                                                            "now": START}, trigger)

    assert filtered.memories == ()
    assert filtered.recent_elsewhere == ()
    # one from the line the desk already carried, two from what the plugin put back.
    assert filtered.withheld == (("person:fulan private chat", 3),)


async def test_a_plugin_cannot_undo_the_absolute_lines(aconn):
    kernel = build(aconn, clock=FakeClock(START), settings=SETTINGS)

    async def allow(item, ctx):
        return Continue()

    kernel.hooks.action.add("allow", allow)
    trigger = await append_event(aconn, ts=START, kind="percept", who="person:other",
                                 place="space:tg:1", body="telegram",
                                 payload={"description": "ganti nama", "place_kind": "direct"})

    await kernel._apply(trigger, ToolCall("act", {"capability": "self.set_identity",
                                                  "arguments": {"name": "Hacked"}}))

    row = await (await aconn.execute(
        "SELECT payload FROM events WHERE kind = 'guard.stopped'")).fetchone()
    assert row[0] == {"hook": "action",
                      "reason": "only the owner may invoke identity_changing capabilities"}
