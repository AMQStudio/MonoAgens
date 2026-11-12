"""Tests for the context parts: the prefix hash, the byte-identical prompt, and extensions."""

from __future__ import annotations

import hashlib
from dataclasses import replace
from datetime import datetime, timezone

import pytest

from monoagens.boot import build
from monoagens.core.context import layout, parts
from monoagens.core.context.layout import PFCContext
from monoagens.core.context.parts import VOLATILE, Part
from monoagens.core.hooks import Replace
from monoagens.core.reasoning import Reasoning
from monoagens.infra.clock import FakeClock
from monoagens.infra.store import append_event
from monoagens.types import Event
from tests.support import settings

START = datetime(2026, 11, 10, 8, 0, tzinfo=timezone.utc)
SETTINGS = settings()
PLACE = "space:tg:1"
PREFIX_HASH = "7ebc122f4b2988eb0ab2e33048c128a2dda8ed2fe3aa3e3c8cb424237232b448"


def _soul() -> Reasoning:
    return Reasoning("http://unused", "", "fake", settings=SETTINGS)


def _plain() -> PFCContext:
    return PFCContext(now=START, place=PLACE, place_kind="direct", reason="x",
                      core_memory="m",
                      people=("- Budi (person:budi): no direct place known yet; closeness 0.00",))


def _rich() -> PFCContext:
    event = Event(id=7, ts=START, kind="percept", who="person:budi", place=PLACE,
                  body="telegram", payload={"description": "halo, apa kabar?"})
    elsewhere = Event(id=8, ts=START, kind="act.out", who="person:owner", place="space:tg:9",
                      body="telegram", payload={"arguments": {"text": "sampai jumpa"}})
    return PFCContext(
        now=START, place=PLACE, place_kind="direct", reason="addressed message",
        core_memory="Fulan sedang di-PHK.", last_interaction=START,
        doing=('goal: kirim laporan', 'step: dev.run {"command": "pytest"}'),
        expectations=("#3 kirim laporan (no intent stated)",),
        memories=("- rapat jam sembilan (person:owner at space:tg:owner; importance 0.90)",),
        withheld=(("person:owner private chat", 1),),
        recent=[event], recent_elsewhere=[elsewhere],
        results=('think: {"thought": "kirim hasil"}',), trigger=event)


def test_the_stable_prefix_is_unchanged():
    """The cached prefix is byte-identical after the parts refactor (golden hash)."""
    first = parts.messages(_soul(), _plain())[0]["content"]

    assert hashlib.sha256(first.encode()).hexdigest() == PREFIX_HASH
    assert first == EXPECTED_STABLE


def test_the_three_messages_match_the_old_renderer():
    """A desk with results, doing, memories, a withheld note and a line elsewhere."""
    got = [message["content"] for message in parts.messages(_soul(), _rich())]

    assert got == [EXPECTED_STABLE, EXPECTED_SEMI, EXPECTED_VOLATILE]


async def test_a_middleware_block_is_appended_after_the_core_volatile_parts(aconn):
    kernel = build(aconn, clock=FakeClock(START), settings=SETTINGS)
    trigger = await append_event(aconn, ts=START, kind="percept", who="person:budi",
                                 place=PLACE, body="telegram",
                                 payload={"description": "halo", "place_kind": "direct"})

    async def block(item, ctx):
        return Replace(replace(item, blocks=item.blocks + (
            Part("plugin:x", VOLATILE, 900, text="<x>hello</x>"),)))

    kernel.hooks.context.add("plugin-block", block)
    context = await layout.build_context(aconn, trigger, PLACE, "test", now=START,
                                         settings=SETTINGS, memories=(), capabilities=())
    filtered = await kernel._run_hooks("context", context,
                                       {"conn": aconn, "event": trigger, "now": START}, trigger)
    text = parts.messages(_soul(), filtered)[-1]["content"]

    assert text.endswith("<x>hello</x>")
    assert text.index("</event>") < text.index("<x>hello</x>")


def test_a_block_may_not_touch_the_prefix_or_take_a_name():
    stable = PFCContext(now=START, place=PLACE, reason="x",
                        blocks=(Part("plugin:y", parts.STABLE, 10, text="<y>no</y>"),))
    with pytest.raises(ValueError, match="an extension block must be volatile: 'plugin:y'"):
        parts.messages(_soul(), stable)

    twice = replace(stable, blocks=(Part("now", VOLATILE, 900, text="<now>no</now>"),))
    with pytest.raises(ValueError, match="duplicate block name: 'now'"):
        parts.messages(_soul(), twice)


EXPECTED_STABLE = "You are Mona.\n\nCalm and honest.\n\nPrinciples:\n- Never lie.\n\nLanguage: id.\n\nHow you work:\n- Thinking is not acting: in the same answer, call the tool that carries the thought out.\n- What you promise for later, a reminder, or a watch ('tell me if ...') is an expectation: hold it with expect and a cue (cue_time or cue_event) that wakes you; without a cue nothing wakes you.\n- Reach someone or another place with act's to: (an id the desk shows, or an existing place id) and via: (a tool); content that came from another place must be shared first and is never repeated where its audience may not see it: say it in a private place instead.\n- If what you know may be private to someone, ask them privately instead of saying you do not know.\n- A line that may relate to an expectation is a question you answer yourself: when it fulfils one, keep it by doing it in this same turn: say what you promised before you resolve, because a resolve without the act is a broken promise; when it does not, let it be: think, or stay silent where the line was said.\n- Long work goes through a long-running capability; the task thread opens for you.\n- When a capability returns an answer, tell them that answer.\n- You act only through the tools. Staying silent is a valid choice."
EXPECTED_SEMI = '<core_memory>Fulan sedang di-PHK.</core_memory>'
EXPECTED_VOLATILE = '<now>2026-11-10 15:00, Tuesday, Asia/Jakarta, time since last interaction: 0m</now>\n<place>space:tg:1 (direct)</place>\n<why_awake>addressed message</why_awake>\n<results>\nthink: {"thought": "kirim hasil"}\n</results>\n<doing>\ngoal: kirim laporan\nstep: dev.run {"command": "pytest"}\n</doing>\n<expectations>#3 kirim laporan (no intent stated)</expectations>\n<memories>\n- rapat jam sembilan (person:owner at space:tg:owner; importance 0.90)\n</memories>\n<withheld>1 thing you know from person:owner private chat is not shown here</withheld>\n<recent>\n15:00 person:budi: halo, apa kabar?\n[space:tg:9] 15:00 Mona: sampai jumpa\n</recent>\n<event>person:budi: halo, apa kabar?</event>'
