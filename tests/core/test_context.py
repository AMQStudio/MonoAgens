"""The context: the cached prefix, the three messages and the items behind them."""

from __future__ import annotations

import hashlib
from dataclasses import replace
from datetime import datetime, timezone

from monoagens.boot import build
from monoagens.contract import ContextItem
from monoagens.core.context import layout, parts
from monoagens.core.context.layout import PFCContext
from monoagens.core.hooks import Replace
from monoagens.core.reasoning import Reasoning
from monoagens.infra.clock import FakeClock
from monoagens.infra.store import append_event
from tests.support import context, settings

START = datetime(2026, 11, 10, 8, 0, tzinfo=timezone.utc)
SETTINGS = settings()
PLACE = "space:tg:1"
PREFIX_HASH = "8535fcd588d944ec769b7c61436d30d9600b3a811bd35e9fcb63acb26717b38f"


def _soul() -> Reasoning:
    return Reasoning(settings=SETTINGS)


def _plain() -> PFCContext:
    return context(now=START, place=PLACE, reason="x", core_memory="m")


def _rich() -> PFCContext:
    return context(
        now=START, place=PLACE, reason="addressed message", last_interaction=START,
        core_memory="Fulan sedang di-PHK.",
        doing=("goal: kirim laporan", 'step: dev.run {"command": "pytest"}'),
        expectations=("#3 kirim laporan (no intent stated)",),
        memories=("- rapat jam sembilan (person:owner at space:tg:owner; importance 0.90)",),
        withheld=(("space:tg:owner", 1),),
        results=('think: {"thought": "kirim hasil"}',),
        self_text="<self>\nCommands: /help\n</self>")


def test_the_stable_prefix_is_unchanged():
    """The cached prefix is byte-identical after the items refactor (golden hash)."""
    first = parts.messages(_soul(), _plain())[0]["content"]

    assert hashlib.sha256(first.encode()).hexdigest() == PREFIX_HASH


def test_the_three_messages_render_the_items():
    """A desk with results, doing, memories, a note and what is installed."""
    got = [message["content"] for message in parts.messages(_soul(), _rich())]

    assert got[1] == ("<core_memory>Fulan sedang di-PHK.</core_memory>\n"
                     "<self>\nCommands: /help\n</self>")
    assert got[2] == EXPECTED_VOLATILE


def test_a_hook_may_drop_an_item_and_the_text_follows():
    """The text is rendered after the last hook, so a dropped item never reaches the model."""
    desk = _rich()
    kept = tuple(item for item in desk.items if item.section != "memories")
    text = parts.messages(_soul(), replace(desk, items=kept))[-1]["content"]

    assert "rapat jam sembilan" not in text
    assert "<memories>\nnone\n</memories>" in text


async def test_the_context_hook_receives_items_and_its_replacement_is_rendered(aconn):
    """A context rule annotates items; the core renders what came back (DEC-1)."""
    kernel = build(aconn, clock=FakeClock(START), settings=SETTINGS)
    trigger = await append_event(aconn, ts=START, kind="percept", who="person:budi",
                                 place=PLACE, body="telegram", description="halo")

    async def annotate(item, ctx):
        return Replace(replace(item, items=(
            *item.items, ContextItem(section="notes", text="<x>hello</x>"))))

    kernel.hooks.context.add("plugin-note", annotate)
    desk = await layout.build_context(aconn, trigger, PLACE, "test", now=START,
                                      settings=SETTINGS, memories=(), capabilities=())
    filtered = await kernel._run_hooks("context", desk,
                                       {"conn": aconn, "event": trigger, "now": START}, trigger)
    text = parts.messages(_soul(), filtered)[-1]["content"]

    assert "<notes>\n<x>hello</x>\n</notes>" in text
    assert text.index("<notes>") < text.index("<recent>")


def test_every_section_renders_in_one_fixed_order():
    """The volatile sections keep one order, so the prompt stays cache-friendly."""
    text = parts.messages(_soul(), _rich())[-1]["content"]
    order = ["<doing>", "<expectations>", "<memories>", "<notes>", "<recent>", "<event>"]

    assert [text.index(marker) for marker in order] == sorted(
        text.index(marker) for marker in order)


def test_a_notes_section_that_a_rule_never_added_is_absent():
    """The notes section costs no tokens until something is withheld or annotated."""
    assert "<notes>" not in parts.messages(_soul(), _plain())[-1]["content"]


EXPECTED_VOLATILE = (
    "<now>2026-11-10 15:00, Tuesday, Asia/Jakarta, time since last interaction: 0m</now>\n"
    "<place>space:tg:1</place>\n<why_awake>addressed message</why_awake>\n"
    '<results>\nthink: {"thought": "kirim hasil"}\n</results>\n'
    "<doing>\ngoal: kirim laporan\nstep: dev.run {\"command\": \"pytest\"}\n</doing>\n"
    "<expectations>#3 kirim laporan (no intent stated)</expectations>\n"
    "<memories>\n- rapat jam sembilan (person:owner at space:tg:owner; importance 0.90)\n"
    "</memories>\n<notes>\n1 thing you know from space:tg:owner is not shown here\n</notes>\n"
    "<recent>\n(nothing yet)\n</recent>\n<event>none</event>"
)
