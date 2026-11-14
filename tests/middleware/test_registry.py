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
from monoagens.core.memory.forget import forget_hard
from monoagens.infra.attachments import AttachmentStore
from monoagens.infra.clock import FakeClock
from monoagens.infra.store import append_event
from monoagens.middleware import privacy, registry
from monoagens.contract import OUTPUT_MARKER
from monoagens.middleware.registry import (CommandEntry, SkillEntry, ToolRegistry, help_text,
                                           self_block)
from monoagens.contract import Action
from monoagens.types import Event
from tests.support import open_book, publish_audience, settings

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
    PERMISSIONS = {"edit": "guest", "exec": "guest", "react": "guest", "read": "guest", "run": "guest", "say": "guest"}

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
        return action.arguments.get("_result")


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


def test_every_capability_must_declare_a_permission():
    """There are no exceptions, talking capabilities included (contract.md §3, §10)."""
    tools = ToolRegistry()
    tools.register(_Body("silent", (
        {"name": "say", "description": "Speak.", "schema": {}, "communicative": True,
         "long_running": False, "identity_changing": False, "permission": "silent.say"},
        {"name": "react", "description": "React.", "schema": {}, "communicative": False,
         "long_running": False, "identity_changing": False, "permission": "silent.react"},
    )))
    assert tools.permission("silent.say") == "silent.say"
    assert tools.permission("silent.react") == "silent.react"

    with pytest.raises(ValueError, match="quiet.read: an entry without a permission"):
        tools.register(_Body("quiet", ({"name": "read", "description": "Read.",
                                        "schema": {}, "communicative": True,
                                        "long_running": False, "identity_changing": False},)))


async def _act(kernel, event: Event, capability: str,
               arguments: dict[str, Any] | None = None) -> Any:
    """Carry one action through the registry and answer with its receipt."""
    return await kernel.tools.act(registry.own_action(event, capability, arguments or {}))


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
    result = registry._capped(long_text, tools.output_cap("workspace.read"), ref="att:abc")
    expected_n = 200 - len(OUTPUT_MARKER.format(limit=200, ref="att:abc", n=100))
    marker = OUTPUT_MARKER.format(limit=200, ref="att:abc", n=expected_n)
    assert result.endswith(marker)
    assert len(result) == 200
    assert result == long_text[:expected_n] + marker


def test_a_dict_result_keeps_its_other_keys_and_caps_its_text():
    tools = ToolRegistry()
    tools.add_tool(_Tool(), {"read": {"facts": ["reads"], "output_cap": 40,
                                      "permission": "workspace.read"}})
    capped = registry._capped({"text": "y" * 400, "path": "a.txt", "lines": 3}, 200)

    assert capped["path"] == "a.txt" and capped["lines"] == 3
    expected_n = 200 - len(OUTPUT_MARKER.format(limit=200, ref="none", n=100))
    assert capped["text"].endswith(OUTPUT_MARKER.format(limit=200, ref="none", n=expected_n))


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
                                 payload={"description": "how do I brew coffee?"})
    context = await layout.build_context(aconn, trigger, PLACE, "test", now=START,
                                         settings=SETTINGS, memories=(), capabilities=())
    ctx = {"conn": aconn, "event": trigger, "now": START}

    outcome = await self_block(kernel)(context, ctx)
    text = outcome.item.self_text

    assert "<self>" in text and "</self>" in text
    assert ("Commands: /budget, /help, /link, /plugin, /quiet, /role, /roles, /show, /unlink, "
            "/whoami, /whois") in text
    assert "coffee-brewing — How to brew coffee well." in text
    assert "tax-filing — How to file a tax return." in text
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

    text = outcome.item.self_text
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
    assert kernel.tools.permission("telegram.say") == "telegram.say"
    assert kernel.tools.facts("registry.describe") == frozenset({"reads"})
    assert kernel.tools.facts("telegram.say") == frozenset()


def test_help_text_without_a_roles_book_lists_nothing_but_tools():
    tools = ToolRegistry()
    tools.register(_Body("telegram", TELEGRAM_CAPABILITIES))

    assert help_text(tools, "person:owner") == "Commands:\nnone\n\nTools:\nnone"


async def test_overflow_output_archives_full_text_and_reads_50mb_in_parts(aconn, tmp_path):
    class _BulkTool:
        name = "bulk"

        def __init__(self, text: str) -> None:
            self._text = text

        def capabilities(self):
            return ({"name": "dump", "description": "Emit lots of text.",
                     "schema": {"type": "object", "properties": {}},
                     "communicative": False, "long_running": False,
                     "identity_changing": False},)

        async def act(self, action: Action) -> dict[str, Any]:
            return {"output": self._text}

    full_text = ("0123456789ABCDEF" * 3_276_800)  # exact 50 MiB ASCII
    assert len(full_text) == 50 * 1024 * 1024
    kernel = build(aconn, clock=FakeClock(START), settings=SETTINGS)
    kernel.attachments = AttachmentStore(tmp_path)
    kernel.tools.add_tool(_BulkTool(full_text), {"dump": {"facts": ["reads"],
                                                          "permission": "workspace.run",
                                                          "output_cap": 8000}})
    trigger = Event(id=1, ts=START, kind="percept", who="person:owner",
                    place="space:tg:dm:owner", body="telegram", payload={})

    done = await _act(kernel, trigger, "bulk.dump")
    assert done.state == "done"
    capped = done.result
    ref = capped["full"]["ref"]
    assert capped["full"] == {"ref": ref, "size": 50 * 1024 * 1024}
    marker_idx = capped["output"].index("\n...[truncated:")
    prefix = capped["output"][:marker_idx]
    assert f"full in {ref}; continue from character {len(prefix)}]" in capped["output"]
    assert len(capped["output"]) <= 8000

    await append_event(aconn, ts=START, kind="act.result", who="self",
                       place="space:tg:dm:owner",
                       payload={"capability": "bulk.dump", "result": capped})

    from monoagens.middleware.registry import MORE_OUTPUT_CAP

    first = (await _act(kernel, trigger, "registry.more",
                        {"ref": ref, "offset": len(prefix),
                         "limit": 30_000_000})).result
    tail_offset = len(full_text) - 500
    tail = (await _act(kernel, trigger, "registry.more",
                       {"ref": ref, "offset": tail_offset, "limit": 1000})).result
    assert len(first["text"]) == MORE_OUTPUT_CAP
    assert first["remaining"] == len(full_text) - len(prefix) - MORE_OUTPUT_CAP
    assert first["next_offset"] == len(prefix) + MORE_OUTPUT_CAP
    assert full_text[len(prefix):len(prefix) + MORE_OUTPUT_CAP] == first["text"]
    assert tail["remaining"] == 0 and tail["next_offset"] is None
    assert prefix + first["text"] == full_text[:len(prefix) + MORE_OUTPUT_CAP]
    assert tail["text"] == full_text[tail_offset:]


async def test_full_output_respects_privacy_and_hard_forget(aconn, tmp_path):
    class _SecretTool:
        name = "bulk"

        def capabilities(self):
            return ({"name": "dump", "description": "Emit secret output.",
                     "schema": {"type": "object", "properties": {}},
                     "communicative": False, "long_running": False,
                     "identity_changing": False},)

        async def act(self, action: Action) -> dict[str, Any]:
            return {"output": "TOP-SECRET-CHUNK-" + ("Z" * 500)}

    kernel = build(aconn, clock=FakeClock(START), settings=SETTINGS)
    kernel.attachments = AttachmentStore(tmp_path)
    kernel.tools.add_tool(_SecretTool(), {"dump": {"facts": ["reads"],
                                                   "permission": "workspace.run",
                                                   "output_cap": 200}})
    await publish_audience(aconn, "space:tg:dm:bob", ["person:bob"], ts=START)
    dm_event = await append_event(aconn, ts=START, kind="percept", who="person:bob",
                                  place="space:tg:dm:bob", body="telegram",
                                  payload={"description": "dump it"})
    capped = (await _act(kernel, dm_event, "bulk.dump")).result
    ref = capped["full"]["ref"]
    stored = await append_event(aconn, ts=START, kind="act.result", who="self",
                                place="space:tg:dm:bob",
                                payload={"capability": "bulk.dump", "result": capped})

    await publish_audience(aconn, PLACE, ["person:alice"], ts=START)
    group_event = await append_event(aconn, ts=START, kind="percept", who="person:alice",
                                     place=PLACE, body="telegram",
                                     payload={"description": "and it?"})
    blocked = await _act(kernel, group_event, "registry.more", {"ref": ref, "offset": 0})
    assert blocked.state == "failed" and "may not see" in blocked.reason

    await privacy.grant(kernel, stored, {"items": [int(stored.id)]},
                        to=PLACE, source="space:tg:dm:bob")
    opened = await _act(kernel, group_event, "registry.more",
                        {"ref": ref, "offset": 0, "limit": 50})
    assert opened.result["text"].startswith("TOP-SECRET-CHUNK-")

    await forget_hard(aconn, [stored.id], now=START)
    removed = await kernel.attachments.sweep(aconn)
    assert removed == 1 and kernel.attachments.find(ref) is None
    gone = await _act(kernel, dm_event, "registry.more", {"ref": ref, "offset": 0})
    assert gone.state == "failed"


async def test_registry_more_denies_without_place_and_caps_limit(aconn, tmp_path):
    """P.8: registry.more refuses empty/None context place and caps returned slice size."""
    from monoagens.middleware.registry import MORE_OUTPUT_CAP

    class _LargeTool:
        name = "bulk"

        def capabilities(self):
            return ({"name": "emit", "description": "Emit >2x MORE_OUTPUT_CAP chars.",
                     "schema": {"type": "object", "properties": {}},
                     "communicative": False, "long_running": False,
                     "identity_changing": False},)

        async def act(self, action: Action) -> dict[str, Any]:
            return {"output": "A" * (MORE_OUTPUT_CAP * 3)}

    kernel = build(aconn, clock=FakeClock(START), settings=SETTINGS)
    kernel.attachments = AttachmentStore(tmp_path)
    kernel.tools.add_tool(_LargeTool(), {"emit": {"facts": ["reads"],
                                                  "permission": "workspace.run",
                                                  "output_cap": 100}})
    dm_event = await append_event(aconn, ts=START, kind="percept", who="person:owner",
                                  place="space:tg:dm:owner", body="telegram",
                                  payload={"place_kind": "direct"})
    capped = (await _act(kernel, dm_event, "bulk.emit")).result
    ref = capped["full"]["ref"]
    await append_event(aconn, ts=START, kind="act.result", who="self",
                       place="space:tg:dm:owner",
                       payload={"capability": "bulk.emit", "result": capped})

    no_place_event = Event(id=None, ts=START, kind="percept", who="person:owner",
                           place="", body="telegram", payload={})
    denied_act = await _act(kernel, no_place_event, "registry.more", {"ref": ref, "offset": 0})
    assert denied_act.state == "failed" and "place" in denied_act.reason
    denied_direct = await kernel.tools.more(ref, offset=0, limit=100, context=None)
    assert "error" in denied_direct

    sliced = (await _act(kernel, dm_event, "registry.more",
                         {"ref": ref, "offset": 0, "limit": 999_999})).result
    assert len(sliced["text"]) == MORE_OUTPUT_CAP
    assert sliced["next_offset"] == MORE_OUTPUT_CAP
    assert sliced["remaining"] == MORE_OUTPUT_CAP * 2



async def test_an_action_for_a_place_another_body_owns_is_refused(aconn):
    """§2.4.4: a place is reached through the body that senses it, never behind its back."""
    kernel = build(aconn, clock=FakeClock(START), settings=SETTINGS)
    kernel.register_adapter(_Body("telegram", TELEGRAM_CAPABILITIES))
    kernel.register_adapter(_Body("cli", CLI_CAPABILITIES))
    kernel.tools.place_body[PLACE] = "cli"

    receipt = await _act(kernel, Event(id=None, ts=START, kind="percept", who="person:owner",
                                      place=PLACE, body="cli", payload={}),
                         "telegram.say", {"text": "hai"})

    assert receipt.state == "refused"
    assert receipt.reason == "this place is reached through cli"


async def test_a_place_no_body_has_sensed_yet_is_carried_by_its_declaring_body(aconn):
    """§2.4.4: only a place *another* body owns is refused; an unreported place has no owner."""
    kernel = build(aconn, clock=FakeClock(START), settings=SETTINGS)
    body = _Body("telegram", TELEGRAM_CAPABILITIES)
    kernel.register_adapter(body)

    receipt = await _act(kernel, Event(id=None, ts=START, kind="percept", place="space:ghost",
                                      payload={}), "telegram.say", {"text": "hai"})

    assert receipt.state == "done"
    assert [action.arguments["text"] for action in body.actions] == ["hai"]
