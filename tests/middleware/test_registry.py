"""The registry: tools, commands and skills, each with facts, a permission and an output cap."""

from __future__ import annotations

import asyncio
from datetime import datetime, timezone
from typing import Any

import pytest

from monoagens.adapters.cli import CAPABILITIES as CLI_CAPABILITIES
from monoagens.adapters.telegram import CAPABILITIES as TELEGRAM_CAPABILITIES
from monoagens.boot import build
from monoagens.core.context import layout
from monoagens.core.context.parts import VOLATILE
from monoagens.infra.clock import FakeClock
from monoagens.infra.store import append_event
from monoagens.middleware import registry
from monoagens.middleware.contract import OUTPUT_MARKER
from monoagens.middleware.registry import (CommandEntry, SkillEntry, ToolRegistry, help_text,
                                           self_block)
from monoagens.types import Action, Event
from tests.support import open_book, settings

START = datetime(2026, 11, 12, 8, 0, tzinfo=timezone.utc)
SETTINGS = settings()
PLACE = "space:tg:-500"


class _Body:
    """A body that declares the chat capabilities of one adapter and records its actions."""

    def __init__(self, name: str, capabilities) -> None:
        self.name, self._capabilities, self.actions = name, capabilities, []

    def capabilities(self):
        return self._capabilities

    async def act(self, action: Action) -> None:
        self.actions.append(action)


class _Tool:
    """A plugin tool: one capability, facts supplied by the manifest."""

    name = "workspace"

    def __init__(self) -> None:
        self.actions: list[Action] = []

    def capabilities(self):
        return ({"name": "read", "description": "Read a file.",
                 "schema": {"type": "object", "properties": {"path": {"type": "string"}},
                            "required": ["path"]},
                 "communicative": False, "long_running": False,
                 "identity_changing": False},)

    async def act(self, action: Action) -> Any:
        self.actions.append(action)
        return action.payload.get("_result")


def _event(kind: str = "command", *, body: str = "telegram", **payload) -> Event:
    return Event(id=None, ts=START, kind=kind, who="person:owner", place=PLACE,
                 body=body, payload=payload)


def test_a_plugin_tool_without_facts_fails_at_boot():
    tools = ToolRegistry()

    with pytest.raises(ValueError, match="workspace.read: a tool without facts"):
        tools.add_tool(_Tool(), {})


def test_a_command_without_a_permission_fails_at_boot():
    tools = ToolRegistry()

    with pytest.raises(ValueError, match="help: an entry without a permission"):
        tools.add_command(CommandEntry("help", "List things.", "", _no_handler))


async def _no_handler(kernel, event, arguments):
    return None


def test_a_capability_collision_fails_at_boot():
    tools = ToolRegistry()
    tools.register(_Body("telegram", TELEGRAM_CAPABILITIES))

    with pytest.raises(ValueError, match="adapter collision: telegram"):
        tools.register(_Body("telegram", TELEGRAM_CAPABILITIES))


def test_a_duplicate_command_fails_at_boot():
    tools = ToolRegistry()
    tools.add_command(CommandEntry("help", "List things.", "help", _no_handler))

    with pytest.raises(ValueError, match="collision: help"):
        tools.add_command(CommandEntry("help", "List things.", "help", _no_handler))


def test_only_a_talking_capability_may_declare_no_permission():
    tools = ToolRegistry()
    tools.register(_Body("silent", ({"name": "say", "description": "Speak.",
                                     "schema": {}, "communicative": True,
                                     "long_running": False, "identity_changing": False,
                                     "permission": None},)))
    assert tools.permission("silent.say") == ""

    with pytest.raises(ValueError, match="quiet.read: an entry without a permission"):
        tools.register(_Body("quiet", ({"name": "read", "description": "Read.",
                                        "schema": {}, "communicative": False,
                                        "long_running": False, "identity_changing": False},)))


async def _help_on(aconn, body: _Body) -> str:
    kernel = build(aconn, clock=FakeClock(START), settings=SETTINGS)
    kernel.register_adapter(body)
    event = _event("command", body=body.name, command="help", arguments=[])
    put = _task(kernel, event)
    await kernel.step()
    await put
    await kernel.run_until_idle()
    cur = await aconn.execute("SELECT payload FROM events WHERE kind = 'act.out' "
                              "AND payload->>'capability' = %s", (f"{body.name}.say",))
    row = await cur.fetchone()
    if row is None:
        logged = await (await aconn.execute(
            "SELECT kind, payload->>'capability' FROM events ORDER BY id")).fetchall()
        raise AssertionError(f"no command reply recorded for {body.name}: {logged}")
    return row[0]["arguments"]["text"]


def _task(kernel, event):
    """Put one event without waiting for it: the step below processes it."""
    return asyncio.create_task(kernel.put(event))


async def test_help_is_byte_identical_on_telegram_and_cli(aconn):
    telegram = await _help_on(aconn, _Body("telegram", TELEGRAM_CAPABILITIES))
    cli = await _help_on(aconn, _Body("cli", CLI_CAPABILITIES))

    assert telegram == cli
    assert telegram.startswith("Commands:\n/budget — ")
    assert "self.recall(" in telegram and "/help" in telegram
    assert "telegram.say" not in telegram and "cli.say" not in telegram


def test_the_output_cap_cuts_with_the_one_marker():
    tools = ToolRegistry()
    tool = _Tool()
    tools.add_tool(tool, {"read": {"facts": ["reads"], "output_cap": 200,
                                   "permission": "workspace.read"}})

    long_text = "x" * 400
    assert tools.output_cap("workspace.read") == 200
    result = registry._capped(long_text, tools.output_cap("workspace.read"))
    marker = OUTPUT_MARKER.format(limit=200, n=200)
    assert result.endswith(marker)
    assert len(result) == 200
    assert result == long_text[:200 - len(marker)] + marker


def test_a_dict_result_keeps_its_other_keys_and_caps_its_text():
    tools = ToolRegistry()
    tools.add_tool(_Tool(), {"read": {"facts": ["reads"], "output_cap": 40,
                                      "permission": "workspace.read"}})
    capped = registry._capped({"text": "y" * 400, "path": "a.txt", "lines": 3}, 200)

    assert capped["path"] == "a.txt" and capped["lines"] == 3
    assert capped["text"].endswith(OUTPUT_MARKER.format(limit=200, n=200))


async def test_a_skill_enters_the_self_block_by_name_only(aconn):
    kernel = build(aconn, clock=FakeClock(START), settings=SETTINGS)
    kernel.roles = open_book("person:owner", "help", "plugin.manage")
    kernel.tools.add_command(CommandEntry("plugin", "Manage plugins.", "plugin.manage",
                                          _no_handler))
    kernel.tools.add_skill(SkillEntry("coffee-brewing", "How to brew coffee well.",
                                      "SECRET BODY"))
    kernel.tools.add_skill(SkillEntry("tax-filing", "How to file a tax return.", "BODY"))
    trigger = await append_event(aconn, ts=START, kind="percept", who="person:owner",
                                 place=PLACE, body="telegram",
                                 payload={"description": "how do I brew coffee?",
                                          "place_kind": "direct"})
    context = await layout.build_context(aconn, trigger, PLACE, "test", now=START,
                                         settings=SETTINGS, memories=(), capabilities=())
    ctx = {"conn": aconn, "event": trigger, "now": START}

    outcome = await self_block(kernel)(context, ctx)
    text = "\n".join(part.text for part in outcome.item.blocks if part.layer == VOLATILE)

    assert "<self>" in text and "</self>" in text
    assert ("Commands: /budget, /help, /plugin, /quiet, /role, /roles, /show, /whoami, "
            "/whois") in text
    assert "coffee-brewing — How to brew coffee well." in text
    assert "tax-filing" not in text
    assert "SECRET BODY" not in text


async def test_the_self_block_lists_only_what_the_requester_may_use(aconn):
    kernel = build(aconn, clock=FakeClock(START), settings=SETTINGS)
    kernel.roles = open_book("person:guest-1", )
    kernel.tools.add_command(CommandEntry("plugin", "Manage plugins.", "plugin.manage",
                                          _no_handler))
    trigger = await append_event(aconn, ts=START, kind="percept", who="person:guest-1",
                                 place=PLACE, body="telegram",
                                 payload={"description": "halo", "place_kind": "direct"})
    context = await layout.build_context(aconn, trigger, PLACE, "test", now=START,
                                         settings=SETTINGS, memories=(),
                                         capabilities=kernel.tools.descriptors())
    outcome = await self_block(kernel)(context, {"conn": aconn, "event": trigger, "now": START})

    text = "\n".join(part.text for part in outcome.item.blocks if part.layer == VOLATILE)
    assert "Commands: /help" in text  # guests hold `help`, the guest tier permission
    assert "/plugin" not in text
    assert "self.share" in {f"{cap['organ']}.{cap['name']}"
                            for cap in outcome.item.capabilities}
    assert "registry.describe" in {f"{cap['organ']}.{cap['name']}"
                                   for cap in outcome.item.capabilities}


async def test_describe_returns_the_full_entry(aconn):
    kernel = build(aconn, clock=FakeClock(START), settings=SETTINGS)
    kernel.register_adapter(_Body("telegram", TELEGRAM_CAPABILITIES))

    tool = kernel.tools.describe("telegram.say")
    assert tool["tool"]["organ"] == "telegram"
    assert tool["tool"]["schema"]["required"] == ["text"]
    assert kernel.tools.describe("nope") == {"error": "unknown: nope"}


async def test_the_registry_reports_facts_and_permissions(aconn):
    kernel = build(aconn, clock=FakeClock(START), settings=SETTINGS)
    kernel.register_adapter(_Body("telegram", TELEGRAM_CAPABILITIES))

    assert kernel.tools.permission("self.share") == "privacy.share"
    assert kernel.tools.permission("telegram.say") == ""
    assert kernel.tools.facts("registry.describe") == frozenset({"reads"})
    assert kernel.tools.facts("telegram.say") == frozenset()


def test_help_text_without_a_roles_book_lists_nothing_but_tools():
    tools = ToolRegistry()
    tools.register(_Body("telegram", TELEGRAM_CAPABILITIES))

    assert help_text(tools, "person:owner") == "Commands:\nnone\n\nTools:\nnone"
