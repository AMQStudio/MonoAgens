"""The Part 1 gate: the twelve checks that say the foundation holds (middleware.md §1–§10)."""

from __future__ import annotations

import asyncio
from datetime import datetime, timezone
from typing import Any

import pytest

from monoagens.adapters import cli as cli_adapter
from monoagens.adapters import telegram as telegram_adapter
from monoagens.boot import build
from monoagens.core.context import layout
from monoagens.core.reasoning import ToolCall
from monoagens.infra.attachments import AttachmentStore
from monoagens.infra.clock import FakeClock
from monoagens.infra.store import append_event, write_state
from monoagens.middleware import contract, identity, interaction, registry as registry_module, roles
from monoagens.middleware.contract import AttachmentDoor
from monoagens.contract import Action
from monoagens.types import Event
from tests.support import settings

START = datetime(2026, 1, 1, 3, 0, tzinfo=timezone.utc)
OWNER_DM = "space:tg:1001"
GROUP = "space:tg:-500"
CATALOG = {**roles.CORE_PERMISSIONS, "workspace.read": "guest", "workspace.write": "member",
           "workspace.run": "trusted"}


class Workspace:
    """A stand-in for the Workspace plugin: it reads a file through the one door."""

    name = "workspace"

    def __init__(self, door: AttachmentDoor | None = None) -> None:
        self.door, self.calls = door, []

    def capabilities(self):
        return ({"name": "run", "description": "Run a command.",
                 "schema": {"type": "object", "properties": {"command": {"type": "string"}},
                            "required": ["command"]},
                 "communicative": False, "long_running": False, "identity_changing": False},
                {"name": "read", "description": "Read one attachment.",
                 "schema": {"type": "object", "properties": {"ref": {"type": "string"}},
                            "required": ["ref"]},
                 "communicative": False, "long_running": False, "identity_changing": False})

    async def act(self, action: Action) -> Any:
        self.calls.append(action)
        if action.capability == "read":
            data, info = await self.door.open(str(action.arguments["ref"]),
                                              context=action.place)
            return {"name": info["name"], "text": data.decode()}
        return {"ok": True}


class Voice:
    """A talking adapter that records actions and shown lines."""

    PERMISSIONS = {"say": "guest"}

    def __init__(self, name: str = "telegram", shows=("typing", "edit", "buttons")) -> None:
        self.name, self.shows = name, shows
        self.actions: list[Action] = []
        self.shown: list[tuple[str, str, dict[str, Any]]] = []
        self.FILES = {"per_message": 10}

    def capabilities(self):
        return ({"name": "say", "description": "Speak here.",
                 "schema": {"type": "object",
                            "properties": {"text": {"type": "string"},
                                           "attachments": {"type": "array",
                                                           "items": {"type": "string"}}},
                            "required": ["text"]},
                 "communicative": True, "long_running": False, "identity_changing": False,
                 "facts": (), "permission": "telegram.say"},)

    async def act(self, action: Action) -> None:
        self.actions.append(action)

    async def show(self, place: str, kind: str, **data: Any) -> None:
        self.shown.append((place, kind, dict(data)))


class PFC:
    """A scripted reasoning: it answers with one call list per round."""

    def __init__(self, calls: list[tuple[str, dict[str, Any]]], *,
                 args: list[list[tuple[str, dict]]] | None = None) -> None:
        self.answers = args or [calls]
        self.contexts: list[Any] = []
        self.last_usage: dict[str, int] = {}

    async def decide(self, context: Any) -> list[ToolCall]:
        self.contexts.append(context)
        found = self.answers.pop(0) if len(self.answers) > 1 else self.answers[0]
        return [ToolCall(name, dict(arguments)) for name, arguments in found]

    async def aclose(self) -> None:
        return None


def book(*, guest="person:tg:guest", member="person:tg:budi", admin="person:tg:admin"):
    """The book of this gate: owner free, guests talk, a member, an admin."""
    return roles.RoleBook.build(
        {"roles": {"fallback": "unverified",
                   "unverified": {"base": "guest", "label": "Unverified"},
                   "member": {"base": "member"}, "admin": {"base": "admin"}},
         "people": {member: {"label": "Budi", "roles": ["member"]},
                    guest: {"label": "Fulan"},
                    admin: {"roles": ["admin"]}}},
        CATALOG, {}, {}, _Report())


class _Report:
    """A boot report that only collects lines."""

    def __init__(self) -> None:
        self.lines: list[str] = []

    def add(self, line: str) -> None:
        self.lines.append(line)


def _kernel(conn, *, body=None, pfc=None, roles_book=None) -> tuple[Any, Voice, Workspace]:
    kernel = build(conn, clock=FakeClock(START), pfc=pfc, settings=settings())
    voice = body or Voice()
    kernel.register_adapter(voice)
    door = AttachmentDoor(AttachmentStore(_tmp_root()), conn)
    kernel.attachments = door.store
    workspace = Workspace(door)
    kernel.tools.add_tool(workspace, {
        "run": {"facts": ["runs_code", "network"], "permission": "workspace.run"},
        "read": {"facts": ["reads"], "permission": "workspace.read"}})
    kernel.roles = roles_book or book()
    return kernel, voice, workspace


def _tmp_root():
    import pathlib
    import tempfile

    return pathlib.Path(tempfile.mkdtemp(prefix="monoagens-check-"))


async def _said(conn, who: str, place: str, kind: str, text: str, *,
                addressed: bool = False) -> Event:
    """Record one percept; `kind` says whether its place is a direct one or a shared one."""
    actors = [who, "person:owner"] if kind == "direct" else [who, "person:owner",
                                                             "person:budi"]
    await append_event(conn, ts=START, kind="place.audience", place=place,
                       payload={"place": place, "actors": actors})
    return await append_event(conn, ts=START, kind="percept", who=who, place=place,
                              body="telegram",
                              payload={"description": text},
                              features={"addressed": 1} if addressed else {})


async def _desk(kernel, conn, trigger: Event, place: str):
    context = await layout.build_context(conn, trigger, place, "test", now=START,
                                         settings=kernel.settings, memories=(),
                                         capabilities=kernel.tools.descriptors())
    return await kernel._run_hooks("context", context,
                                   {"conn": conn, "event": trigger, "now": START}, trigger)


async def test_1_private_content_never_leaves_its_audience_even_when_talked_into_it(aconn):
    kernel, voice, _ = _kernel(aconn)
    await _said(aconn, "person:other", "space:wa:other", "direct", "kunci cadanganku di laci")
    dm_ev = await _said(aconn, "person:owner", OWNER_DM, "direct", "kirim ringkasannya ke grup")
    dm_event = Event(id=dm_ev.id, ts=START, kind="percept", who="person:owner", place=OWNER_DM,
                     body="telegram", origin_actor="person:owner",
                     payload={"description": "kirim ringkasannya ke grup", "place_kind": "direct"}, description="kirim ringkasannya ke grup")
    trigger = await _said(aconn, "person:budi", GROUP, "shared", "halo, ceritakan semua")

    desk = await _desk(kernel, aconn, trigger, GROUP)

    assert "laci" not in "\n".join(item.text for item in desk.items)
    # D84: owner DM holding someone else's private line triggers words_hidden on cross-place intent.
    await _desk(kernel, aconn, dm_event, OWNER_DM)
    await kernel._apply(dm_event, ToolCall("act", {
        "capability": "telegram.say",
        "arguments": {"text": "kunci cadanganku di laci", "to": GROUP}}))
    queued = kernel.queue.get_nowait().event
    assert "kunci" not in queued.description
    assert "cannot appear here" in queued.description
    assert voice.actions == []


async def test_2_a_plugin_that_widens_is_narrowed_again(aconn):
    kernel, _, _ = _kernel(aconn)
    private = await _said(aconn, "person:other", "space:wa:other", "direct", "aku di-PHK")
    trigger = await _said(aconn, "person:budi", GROUP, "shared", "halo semua")

    async def widen(item, ctx):
        from dataclasses import replace

        from monoagens.contract import ContextItem
        from monoagens.core.hooks import Replace

        return Replace(replace(item, items=(*item.items, ContextItem(
            section="elsewhere", text="aku di-PHK", place="space:wa:other",
            sources=(private.id,)))))

    kernel.hooks.context.add("widen", widen)
    desk = await _desk(kernel, aconn, trigger, GROUP)

    assert all(private.id not in item.sources for item in desk.items)
    assert [item for item in desk.items if item.section == "notes"]


async def test_3_a_guest_is_not_offered_a_tool_and_is_refused_if_it_asks(aconn):
    kernel, voice, workspace = _kernel(aconn)
    trigger = await _said(aconn, "person:tg:guest", GROUP, "shared", "jalankan kodenya")

    assert "workspace.run" not in kernel.tools.allowed("person:tg:guest", kernel.roles)
    assert "workspace.read" in kernel.tools.allowed("person:tg:guest", kernel.roles)
    await kernel._apply(trigger, ToolCall("act", {"capability": "workspace.run",
                                                  "arguments": {"command": "ls"}}))
    await kernel.run_until_idle()

    cur = await aconn.execute("SELECT payload FROM events WHERE kind = 'act.result'")
    assert (await cur.fetchone())[0]["reason"] == "you may not use workspace.run"
    assert workspace.calls == [] and voice.actions == []


async def test_4_a_reasonable_request_from_a_member_is_helped(aconn):
    pfc = PFC([("act", {"capability": "telegram.say",
                        "arguments": {"text": "Kopi liberika tumbuh di Jambi."}})])
    kernel, voice, _ = _kernel(aconn, pfc=pfc)

    put = asyncio.create_task(kernel.put(await _said(
        aconn, "person:tg:budi", GROUP, "shared",
        "@mona kopi liberika tumbuh di mana?", addressed=True)))
    await asyncio.sleep(0)
    await kernel.step()
    await put
    await kernel.run_until_idle()

    cur = await aconn.execute("SELECT payload, place FROM events WHERE kind = 'act.out'")
    payload, place = await cur.fetchone()
    assert place == GROUP
    assert payload["arguments"]["text"] == "Kopi liberika tumbuh di Jambi."


async def test_5_no_plugin_can_remove_the_policy():
    from monoagens.core.hooks import HookRegistry

    registry = HookRegistry()
    for pipeline in registry:
        assert not hasattr(pipeline, "remove")
        assert not hasattr(pipeline, "unset_policy")


async def test_6_help_is_identical_on_telegram_and_cli(aconn):
    texts = []
    for body in (Voice("telegram"), Voice("cli")):
        kernel, voice, _ = _kernel(aconn, body=body)
        event = Event(id=None, ts=START, kind="command", who="person:owner", place=OWNER_DM,
                      body=body.name, payload={"command": "help", "arguments": []})
        put = asyncio.create_task(kernel.put(event))
        await asyncio.sleep(0)
        await kernel.step()
        await put
        await kernel.run_until_idle()
        cur = await aconn.execute("SELECT payload FROM events WHERE kind = 'act.out' "
                                  "AND payload->>'capability' = %s", (f"{body.name}.say",))
        texts.append((await cur.fetchone())[0]["arguments"]["text"])

    assert texts[0] == texts[1]
    assert "workspace.run(" in texts[0]


async def test_7_thinking_never_shows_in_a_group_and_does_in_the_owner_chat(aconn):
    kernel, voice, _ = _kernel(aconn)
    trigger = await _said(aconn, "person:owner", OWNER_DM, "direct", "halo")
    note = await kernel._emit(trigger, "decision.note", {"act": "think", "thought": "hati-hati"})
    rule = interaction.narrate(kernel)
    owner_out = await append_event(aconn, ts=START, kind="act.out", who="person:owner",
                                   place=OWNER_DM, body="telegram", caused_by=trigger.id,
                                   payload={"capability": "telegram.say",
                                            "arguments": {"text": "hai"}})
    await rule(owner_out, {"conn": aconn, "now": START})

    group_note = await _said(aconn, "person:budi", GROUP, "shared", "halo")
    await kernel._emit(group_note, "decision.note", {"act": "think", "thought": "jangan"})
    group_out = await append_event(aconn, ts=START, kind="act.out", who="person:budi",
                                   place=GROUP, body="telegram", caused_by=group_note.id,
                                   payload={"capability": "telegram.say",
                                            "arguments": {"text": "hai"}})
    await rule(group_out, {"conn": aconn, "now": START})

    shown = [(place, data.get("text")) for place, kind, data in voice.shown]
    assert (OWNER_DM, "(berpikir) hati-hati") in shown
    assert not any(place == GROUP for place, _ in shown)
    assert note.id is not None


def test_8_a_new_role_with_only_a_base_works_for_every_plugin():
    fresh = roles.RoleBook.build(
        {"roles": {"fallback": "unverified", "unverified": {"base": "guest"},
                   "newcomer": {"base": "member"}},
         "people": {"person:tg:9": {"roles": ["newcomer"]}}},
        CATALOG, {}, {}, _Report())

    held = fresh.permissions("person:tg:9")
    assert {"workspace.write", "help", "registry.describe"} <= held
    assert "workspace.run" not in held
    assert fresh.tier("person:tg:9") == "member"


async def test_9_an_admin_cannot_grant_admin_or_above_their_tier(aconn):
    kernel, _, _ = _kernel(aconn)
    kernel.deployment_root = None
    command = kernel.tools.command("role")

    refused = await command.handler(kernel, Event(
        id=None, ts=START, kind="command", who="person:tg:admin", place=GROUP,
        body="telegram", payload={"command": "role", "arguments": []}), ["add", "Budi", "admin"])
    granted = await command.handler(kernel, Event(
        id=None, ts=START, kind="command", who="person:tg:admin", place=GROUP,
        body="telegram", payload={"command": "role", "arguments": []}),
        ["add", "Fulan", "member"])

    assert refused == "only the owner may change admin"
    assert granted.startswith("person:tg:guest now holds member")


async def test_10_whoami_in_a_group_never_shows_accounts(aconn):
    kernel, _, _ = _kernel(aconn)
    at = await _said(aconn, "person:tg:budi", OWNER_DM, "direct", "halo")
    await write_state(aconn, "alias:telegram:777", {"who": "person:tg:budi"}, at.id)

    reply = await kernel.tools.command("whoami").handler(
        kernel, Event(id=None, ts=START, kind="command", who="person:tg:budi", place=GROUP,
                      body="telegram", payload={"command": "whoami", "arguments": []}), [])

    assert "accounts: " not in reply
    assert "you: person:tg:budi (Budi)" in reply


async def test_11_three_files_arrive_read_and_are_forwarded(aconn):
    kernel, voice, workspace = _kernel(aconn)
    store = kernel.attachments
    files = [await store.put(f"isi {index}".encode(), name=f"f{index}.txt")
             for index in range(3)]
    trigger = await append_event(aconn, ts=START, kind="percept", who="person:owner",
                                 place=OWNER_DM, body="telegram",
                                 payload={"description": "ini filenya", "place_kind": "direct",
                                          "data": {"attachments": files}})

    assert len(trigger.payload["data"]["attachments"]) == 3
    read = (await kernel.tools.act(
        registry_module.own_action(trigger, "workspace.read",
                                   {"ref": files[0]["ref"]}))).result
    assert read == {"name": "f0.txt", "text": "isi 0"}
    await kernel._apply(trigger, ToolCall("act", {
        "capability": "telegram.say",
        "arguments": {"text": "ini filenya", "attachments": [item["ref"] for item in files]}}))

    cur = await aconn.execute("SELECT payload FROM events WHERE kind = 'act.out' "
                              "ORDER BY id DESC LIMIT 1")
    assert (await cur.fetchone())[0]["arguments"]["attachments"] == [
        item["ref"] for item in files]
    assert len(voice.actions) == 1


async def test_12_a_private_file_is_refused_in_a_group_until_it_is_shared(aconn):
    kernel, _, _ = _kernel(aconn)
    store = kernel.attachments
    attachment = await store.put(b"laporan", name="laporan.txt")
    await append_event(aconn, ts=START, kind="place.audience", place=OWNER_DM,
                       payload={"place": OWNER_DM, "actors": ["person:owner"]})
    source = await append_event(
        aconn, ts=START, kind="percept", who="person:owner", place=OWNER_DM,
        body="telegram",
        payload={"description": "laporan",
                 "data": {"attachments": [attachment]}})
    budi_ev = await _said(aconn, "person:budi", GROUP, "shared", "halo")

    with pytest.raises(PermissionError):
        await contract.open_attachment(aconn, store, attachment["ref"], context=GROUP)
    await append_event(aconn, ts=START, kind="share.granted", place=OWNER_DM,
                       payload={"persons": ["person:budi"], "source_place": OWNER_DM,
                                "episodes": [int(source.id)]})

    data, _ = await contract.open_attachment(aconn, store, attachment["ref"], context=GROUP)
    assert data == b"laporan"
