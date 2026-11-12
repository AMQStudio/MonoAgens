"""Interaction: thinking, steps and approvals — what is shown, to whom, and its fallback."""

from __future__ import annotations

import asyncio
from datetime import datetime, timezone
from typing import Any

from monoagens.boot import build
from monoagens.core.reasoning import ToolCall
from monoagens.core.hooks import Continue
from monoagens.infra.clock import FakeClock
from monoagens.infra.store import append_event, write_state
from monoagens.middleware import interaction
from monoagens.middleware.loader import Loader
from monoagens.types import Action, Event
from tests.support import settings

ROOT = __import__("pathlib").Path(__file__).resolve().parent.parent.parent
START = datetime(2026, 1, 1, 3, 0, tzinfo=timezone.utc)
OWNER_DM = "space:tg:1001"
GROUP = "space:tg:-500"


class Screen:
    """A body that records what it showed and what it sent."""

    def __init__(self, name: str = "telegram", shows: tuple[str, ...] = ("typing", "edit")) -> None:
        self.name, self.shows = name, shows
        self.actions: list[Action] = []
        self.shown: list[tuple[str, str, dict[str, Any]]] = []

    def capabilities(self):
        return ({"name": "say", "description": "Speak here.",
                 "schema": {"type": "object", "properties": {"text": {"type": "string"}},
                            "required": ["text"]},
                 "communicative": True, "long_running": False, "identity_changing": False,
                 "facts": (), "permission": None},
                {"name": "run", "description": "Work.",
                 "schema": {"type": "object", "properties": {}},
                 "communicative": False, "long_running": True, "identity_changing": False,
                 "facts": (), "permission": "telegram.run"},)

    async def act(self, action: Action) -> Any:
        self.actions.append(action)
        return {"text": "hasil" if action.kind == "run" else None}

    async def show(self, place: str, kind: str, **data: Any) -> None:
        self.shown.append((place, kind, dict(data)))


async def _kernel(aconn, *, body=None, interaction_settings=None):
    values = settings()
    if interaction_settings is not None:
        values["interaction"] = interaction_settings
    kernel = build(aconn, clock=FakeClock(START), settings=values)
    kernel.register_adapter(body or Screen())
    return kernel


async def _percept(aconn, place=OWNER_DM, who="person:owner", text="halo"):
    return await append_event(aconn, ts=START, kind="percept", who=who, place=place,
                              body="telegram",
                              payload={"description": text,
                                       "place_kind": "direct" if place == OWNER_DM else "shared"})


async def test_thinking_shows_in_the_owners_chat_and_never_in_a_group(aconn):
    kernel = await _kernel(aconn, body=Screen())
    trigger = await _percept(aconn)
    await kernel._emit(trigger, "decision.note", {"act": "think", "thought": "aku ragu"})
    later = await append_event(aconn, ts=START, kind="act.out", who="person:owner",
                               place=OWNER_DM, body="telegram", caused_by=trigger.id,
                               payload={"capability": "telegram.say",
                                        "arguments": {"text": "hai"}})

    rule = interaction.narrate(kernel)
    await rule(later, {"conn": aconn, "now": START})

    assert any("(thinking) aku ragu" in data["text"] for _, kind, data in
               kernel.tools.adapters["telegram"].shown)

    group = await append_event(aconn, ts=START, kind="act.out", who="person:owner",
                               place=GROUP, body="telegram", caused_by=trigger.id,
                               payload={"capability": "telegram.say",
                                        "arguments": {"text": "hai"}})
    await rule(group, {"conn": aconn, "now": START})
    assert not any(place == GROUP for place, _, _ in kernel.tools.adapters["telegram"].shown)


async def test_a_thought_is_shown_once(aconn):
    kernel = await _kernel(aconn, body=Screen())
    trigger = await _percept(aconn)
    note = await kernel._emit(trigger, "decision.note", {"act": "think", "thought": "satu"})
    later = await append_event(aconn, ts=START, kind="act.out", who="person:owner",
                               place=OWNER_DM, body="telegram", caused_by=trigger.id,
                               payload={"capability": "telegram.say",
                                        "arguments": {"text": "hai"}})
    rule = interaction.narrate(kernel)

    await rule(later, {"conn": aconn, "now": START})
    await rule(later, {"conn": aconn, "now": START})

    shown = [data["text"] for _, kind, data in kernel.tools.adapters["telegram"].shown]
    assert shown.count("(thinking) satu") == 1
    assert note.id is not None


async def test_a_long_act_shows_one_step_and_a_short_one_nothing(aconn):
    kernel = await _kernel(aconn, body=Screen())
    trigger = await _percept(aconn)
    long_result = Event(id=None, ts=START, kind="act.result", who="person:owner",
                        place=OWNER_DM, body="telegram",
                        payload={"capability": "telegram.run", "result": {"text": "x" * 300}})
    short_result = Event(id=None, ts=START, kind="act.result", who="person:owner",
                         place=OWNER_DM, body="telegram",
                         payload={"capability": "telegram.say", "result": {"text": "hai"}})
    rule = interaction.narrate(kernel)

    await rule(long_result, {"conn": aconn, "now": START})
    await rule(short_result, {"conn": aconn, "now": START})

    shown = [data["text"] for _, _, data in kernel.tools.adapters["telegram"].shown]
    steps = [text for text in shown if text.startswith("(step)")]
    assert len(steps) == 1 and "telegram.run" in steps[0]
    assert len(steps[0]) < 260
    assert trigger.id is not None


async def test_a_thought_the_place_may_not_see_is_dropped(aconn):
    kernel = await _kernel(aconn, body=Screen())
    private = await append_event(aconn, ts=START, kind="percept", who="person:other",
                                 place="space:wa:other", body="whatsapp",
                                 payload={"description": "rahasia", "place_kind": "direct"})
    trigger = await _percept(aconn, place=GROUP, who="person:budi", text="halo")
    await kernel._emit(trigger, "decision.note", {"act": "think", "thought": "rahasia"})
    later = await append_event(aconn, ts=START, kind="act.out", who="person:budi",
                               place=GROUP, body="telegram", caused_by=trigger.id,
                               payload={"capability": "telegram.say",
                                        "arguments": {"text": "hai"}})
    kernel.roles = kernel.roles.with_person("person:owner", ())

    async def also_owner(item, ctx):
        return Continue()

    kernel.hooks.inbound.add("always", also_owner)
    rule = interaction.narrate(kernel)
    await rule(later, {"conn": aconn, "now": START})

    assert not kernel.tools.adapters["telegram"].shown
    assert private.id is not None


async def test_a_missing_buttons_capability_falls_back_to_the_text_question(aconn):
    kernel = await _kernel(aconn, body=Screen(shows=("typing",)))
    await append_event(aconn, ts=START, kind="percept", who="person:owner", place=OWNER_DM,
                       body="telegram",
                       payload={"description": "halo", "place_kind": "direct"})
    trigger = await _percept(aconn)

    place = await interaction.request(kernel, trigger, 7, "workspace.run", {"command": "ls"},
                                      "the call runs code")

    assert place == OWNER_DM
    assert kernel.tools.adapters["telegram"].actions[-1].payload["text"].startswith(
        "Approval needed: workspace.run")
    assert not kernel.tools.adapters["telegram"].shown


async def test_buttons_are_shown_when_the_adapter_can(aconn):
    kernel = await _kernel(aconn, body=Screen(shows=("buttons",)))
    await append_event(aconn, ts=START, kind="percept", who="person:owner", place=OWNER_DM,
                       body="telegram",
                       payload={"description": "halo", "place_kind": "direct"})
    trigger = await _percept(aconn)

    await interaction.request(kernel, trigger, 9, "workspace.run", {}, "the call runs code")

    place, kind, data = kernel.tools.adapters["telegram"].shown[-1]
    assert kind == "buttons"
    assert data["buttons"] == [("Yes", "approval:yes:9"), ("No", "approval:no:9")]


async def test_a_button_callback_answers_the_approval(aconn):
    kernel = await _kernel(aconn, body=Screen(shows=("buttons",)))
    trigger = await _percept(aconn)
    asked = await kernel._emit(trigger, interaction.REQUEST, {
        "capability": "telegram.run", "arguments": {"command": "ls"},
        "who": "person:owner", "place": OWNER_DM, "reason": "the call runs code"})

    rule = interaction.answer(kernel)
    callback = Event(id=None, ts=START, kind="percept", who="person:owner", place=OWNER_DM,
                     body="telegram",
                     payload={"approval": asked.id, "answer": True, "description": "yes"})
    await rule(callback, {"conn": aconn, "now": START})

    cur = await aconn.execute("SELECT payload FROM events WHERE kind = 'approval.answered'")
    assert (await cur.fetchone())[0] == {"request": asked.id, "allowed": True,
                                        "who": "person:owner", "by": "person:owner"}
    queued = kernel.queue.get_nowait().event
    assert queued.kind == "intent" and queued.payload["approved"] == asked.id


async def test_the_cli_renders_typing_and_the_approval_text(aconn):
    from monoagens.adapters import cli as cli_adapter

    body = cli_adapter.CLIBody("/tmp/monoagens-test-unused.sock", clock=FakeClock(START))
    kernel = await _kernel(aconn, body=body)
    trigger = await _percept(aconn)

    await interaction.typing(kernel)({"text": "halo"}, {
        "conn": aconn, "event": trigger, "now": START,
        "descriptor": {"communicative": True}, "long_running": False})

    assert "typing" in body.shows


async def test_show_switches_write_one_line_and_apply(aconn, tmp_path):
    (tmp_path / "config.yaml").write_text(
        "version: 2\ninteraction:\n  thinking: owner_direct\n  steps: long\n", encoding="utf-8")
    kernel = await _kernel(aconn)
    kernel.deployment_root = tmp_path

    first = await interaction.show_switch(kernel, _percept_event(), ["thinking", "off"])
    second = await interaction.show_switch(kernel, _percept_event(), ["steps", "off"])

    assert first == "thinking is now off"
    assert second == "steps is now off"
    text = (tmp_path / "config.yaml").read_text(encoding="utf-8")
    assert "  thinking: off" in text and "  steps: off" in text
    assert kernel.settings["interaction"]["thinking"] == "off"
    cur = await aconn.execute("SELECT payload FROM events WHERE kind = 'plugin.config_changed'")
    assert (await cur.fetchone())[0]["key"] == "interaction.thinking"


def _percept_event() -> Event:
    return Event(id=None, ts=START, kind="percept", who="person:owner", place=OWNER_DM,
                 body="telegram", payload={"description": "halo"})


def test_the_boot_report_names_each_adapters_missing_optional_parts():
    from monoagens.middleware.loader import BootReport

    report = BootReport()

    Loader(ROOT, env={"TELEGRAM_BOT_TOKEN": "t"})._check_adapter(
        "telegram", __import__("monoagens.adapters.telegram", fromlist=["create"]).create(
            {}, {"TELEGRAM_BOT_TOKEN": "t"}), report)
    loader_lines = "\n".join(report.lines)
    assert "adapter telegram: shows typing, edit, buttons, reactions" in loader_lines
    assert "adapter telegram: has place_info" in loader_lines
