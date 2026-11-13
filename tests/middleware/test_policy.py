"""Policy: a call passes, asks the owner or is refused; quiet hours; data leaving the machine."""

from __future__ import annotations

import asyncio
from datetime import datetime, timezone
from typing import Any

from monoagens.boot import build
from monoagens.core.hooks import Continue, Middleware
from monoagens.core.reasoning import ToolCall
from monoagens.infra.clock import FakeClock
from monoagens.infra.store import append_event
from monoagens.middleware import interaction, policy, privacy, roles
from monoagens.types import Action, Event
from tests.support import open_book, settings

START = datetime(2026, 1, 1, 3, 0, tzinfo=timezone.utc)  # 10:00 in Jakarta
GROUP = "space:tg:-500"
OWNER_DM = "space:tg:1001"
PLACE = OWNER_DM


class Tool:
    """A tool with declared facts and a permission, recording what it was asked to do."""

    name = "dev"

    def __init__(self, facts: tuple[str, ...] = ("runs_code",),
                 permission: str = "dev.run") -> None:
        """Store what the manifest would declare."""
        self.facts, self.permission, self.actions = facts, permission, []

    def capabilities(self):
        return ({"name": "run", "description": "Run a command.",
                 "schema": {"type": "object", "properties": {"command": {"type": "string"}},
                            "required": ["command"]},
                 "communicative": False, "long_running": False,
                 "identity_changing": False},)

    async def act(self, action: Action) -> dict[str, Any]:
        self.actions.append(action)
        return {"ok": True}


class Voice:
    """A talking adapter, so approvals and quiet hours have somewhere to go."""

    name = "telegram"
    shows = ("buttons",)

    def __init__(self) -> None:
        self.actions: list[Action] = []
        self.shown: list[tuple[str, str, dict[str, Any]]] = []

    def capabilities(self):
        return ({"name": "say", "description": "Speak here.",
                 "schema": {"type": "object", "properties": {"text": {"type": "string"}},
                            "required": ["text"]},
                 "communicative": True, "long_running": False, "identity_changing": False,
                 "facts": (), "permission": None},)

    async def act(self, action: Action) -> None:
        self.actions.append(action)

    async def show(self, place: str, kind: str, **data: Any) -> None:
        self.shown.append((place, kind, data))


async def _kernel(aconn, *, facts=("runs_code",), permission="dev.run", mode=None,
                  quiet=None) -> tuple[Any, Tool, Voice]:
    values = settings()
    if quiet:
        values["owner_settings"] = {**values["owner_settings"], "quiet_hours": quiet}
    kernel = build(aconn, clock=FakeClock(START), settings=values)
    tool, voice = Tool(facts, permission), Voice()
    kernel.register_adapter(voice)
    kernel.tools.add_tool(tool, {"run": {"facts": list(facts), "permission": permission}})
    book = open_book("person:owner", permission)
    kernel.roles = book if mode is None else _with_mode(book, mode)
    return kernel, tool, voice


def _with_mode(book: roles.RoleBook, mode: str) -> roles.RoleBook:
    """Return the same book with one approval mode for the owner."""
    role = book.roles["open"]
    return roles.RoleBook(roles={"open": roles.Role(role.name, role.base, role.allow,
                                                   role.deny, {}, mode)},
                          people=book.people, fallback=book.fallback, bases=book.bases)


async def _trigger(aconn, place: str = PLACE, who: str = "person:owner") -> Event:
    return await append_event(aconn, ts=START, kind="percept", who=who, place=place,
                              body="telegram",
                              payload={"description": "jalankan", "place_kind": "direct"})


async def _kinds(aconn) -> list[str]:
    cur = await aconn.execute("SELECT kind FROM events ORDER BY id")
    return [kind for (kind,) in await cur.fetchall()]


async def _intent(conn) -> Event:
    """Return the retry intent the answer enqueued, read back from the Log."""
    row = await (await conn.execute(
        "SELECT id, ts, place, body, payload FROM events WHERE kind = 'intent' "
        "ORDER BY id DESC LIMIT 1")).fetchone()
    assert row is not None, "no retry intent was enqueued"
    return Event(id=row[0], ts=row[1], kind="intent", who="person:owner", place=row[2],
                 body=row[3], payload=row[4], origin_actor="person:owner")


async def _answer(kernel, text: str) -> None:
    """Type one answer in the owner's direct chat and let the loop process it."""
    put = asyncio.create_task(kernel.put(Event(
        id=None, ts=START, kind="percept", who="person:owner", place=OWNER_DM,
        body="telegram", payload={"description": text, "place_kind": "direct"})))
    await asyncio.sleep(0)
    await kernel.step()
    await put
    await kernel.run_until_idle()


async def _call(kernel, trigger: Event, command: str = "ls", event: Event | None = None):
    return await kernel._apply(event or trigger, ToolCall("act", {
        "capability": "dev.run", "arguments": {"command": command}}))


async def test_a_capability_without_its_permission_is_refused_by_name(aconn):
    kernel, tool, _ = await _kernel(aconn, permission="dev.run")
    kernel.roles = roles.RoleBook.empty()  # the owner keeps everything, others nothing
    trigger = await _trigger(aconn, who="person:other")

    await _call(kernel, trigger)

    assert tool.actions == []
    cur = await aconn.execute("SELECT payload FROM events WHERE kind = 'guard.stopped'")
    assert (await cur.fetchone())[0]["reason"] == "you may not use dev.run"


async def test_the_mode_matrix_decides_pass_and_ask(aconn):
    cases = [((), None, "pass"), (("runs_code",), None, "ask"),
             (("runs_code",), "yolo", "pass"), (("runs_code",), "strict", "ask"),
             (("reads",), "strict", "ask"), (("reads",), None, "pass"),
             (("reads",), "yolo", "pass")]
    for facts, mode, expected in cases:
        for table in ("events", "state", "expectations", "memories"):
            await aconn.execute(f"TRUNCATE TABLE {table} CASCADE")
        kernel, tool, _ = await _kernel(aconn, facts=facts, mode=mode)
        trigger = await _trigger(aconn)
        await _call(kernel, trigger)
        asked = "approval.requested" in await _kinds(aconn)
        assert asked is (expected == "ask"), (facts, mode)
        assert bool(tool.actions) is (expected == "pass"), (facts, mode)


async def test_a_sensitive_call_asks_even_in_yolo(aconn):
    kernel, tool, _ = await _kernel(aconn, facts=("reads",), mode="yolo")

    async def mark(item, ctx):
        ctx["call_facts"] = {"sensitive"}
        return Continue()

    kernel.hooks.action.add("guard", mark)
    trigger = await _trigger(aconn)
    await _call(kernel, trigger)

    assert tool.actions == []
    cur = await aconn.execute("SELECT payload FROM events WHERE kind = 'approval.requested'")
    assert (await cur.fetchone())[0]["reason"] == "the call is marked sensitive"


async def test_the_owners_yes_leads_to_exactly_one_retry(aconn):
    kernel, tool, voice = await _kernel(aconn)
    trigger = await _trigger(aconn)
    await _call(kernel, trigger)
    assert voice.actions and voice.actions[0].kind == "say"
    assert voice.actions[0].payload["choices"] == ["Ya", "Tidak"]
    (request_id,) = [int((await (await aconn.execute(
        "SELECT id FROM events WHERE kind = 'approval.requested'")).fetchone())[0])]

    await _answer(kernel, "ya")

    queued = await _intent(aconn)
    assert queued.payload["approved"] == request_id
    await _call(kernel, trigger, event=queued)

    assert len(tool.actions) == 1
    assert (await _kinds(aconn)).count("approval.requested") == 1
    assert (await _kinds(aconn)).count("approval.answered") == 1


async def test_a_retry_that_differs_is_asked_once_more_then_refused(aconn):
    kernel, tool, _ = await _kernel(aconn)
    trigger = await _trigger(aconn)
    await _call(kernel, trigger)
    await _answer(kernel, "ya")
    queued = await _intent(aconn)

    await _call(kernel, trigger, command="rm -rf /", event=queued)
    assert (await _kinds(aconn)).count("approval.requested") == 2
    assert tool.actions == []
    (second,) = [int((await (await aconn.execute(
        "SELECT id FROM events WHERE kind = 'approval.requested' ORDER BY id DESC LIMIT 1")
    ).fetchone())[0])]
    retry = Event(id=None, ts=START, kind="intent", who="person:owner", place=PLACE,
                  body="telegram", origin_actor="person:owner",
                  payload={"approved": second, "capability": "dev.run",
                           "arguments": {"command": "rm -rf /"}})

    await _call(kernel, trigger, command="rm -rf /usr", event=retry)

    cur = await aconn.execute("SELECT payload FROM events WHERE kind = 'act.result' "
                              "ORDER BY id DESC LIMIT 1")
    assert "the approved call was different" in (await cur.fetchone())[0]["result"]["error"]
    assert (await _kinds(aconn)).count("approval.requested") == 2


async def test_no_leads_to_no_retry(aconn):
    kernel, tool, _ = await _kernel(aconn)
    trigger = await _trigger(aconn)
    await _call(kernel, trigger)

    await _answer(kernel, "tidak")

    assert kernel.queue.empty()
    assert tool.actions == []
    assert (await _kinds(aconn)).count("approval.answered") == 1


async def test_quiet_hours_refuse_a_group_and_allow_the_owners_own_chat(aconn):
    kernel, _, voice = await _kernel(aconn, facts=(), quiet={"from": "00:00", "to": "23:59"})
    group = await append_event(aconn, ts=START, kind="percept", who="person:budi",
                               place=GROUP, body="telegram",
                               payload={"description": "halo", "place_kind": "shared"})

    await kernel._apply(group, ToolCall("act", {"capability": "telegram.say",
                                                "arguments": {"text": "halo"}}))
    cur = await aconn.execute("SELECT payload FROM events WHERE kind = 'act.result'")
    assert "quiet hours" in (await cur.fetchone())[0]["result"]["error"]
    assert voice.actions == []

    owner = await _trigger(aconn)
    await kernel._apply(owner, ToolCall("act", {"capability": "telegram.say",
                                                "arguments": {"text": "halo"}}))
    assert [action.payload["text"] for action in voice.actions] == ["halo"]


async def test_a_network_call_carrying_a_private_memory_is_refused(aconn):
    kernel, _, _ = await _kernel(aconn, facts=("network",), permission="search.run")
    source = await append_event(aconn, ts=START, kind="percept", who="person:owner",
                                place="space:tg:9", body="telegram",
                                payload={"description": "rahasia kantor pindah", "place_kind":
                                         "direct"})
    await aconn.execute("INSERT INTO memories (text, derived_from) VALUES (%s, %s)",
                        ("rahasia kantor", [source.id]))
    await append_event(aconn, ts=START, kind="percept", who="person:budi", place=GROUP,
                       body="telegram",
                       payload={"description": "halo semua", "place_kind": "shared"})
    trigger = await _trigger(aconn, place=GROUP, who="person:owner")

    await kernel._apply(trigger, ToolCall("act", {
        "capability": "dev.run", "arguments": {"command": "search rahasia kantor"}}))

    cur = await aconn.execute("SELECT payload FROM events WHERE kind = 'act.result'")
    assert "private content off the machine" in (await cur.fetchone())[0]["result"]["error"]


async def test_a_public_search_still_passes(aconn):
    kernel, tool, _ = await _kernel(aconn, facts=("network",), permission="search.run")
    trigger = await _trigger(aconn)

    await kernel._apply(trigger, ToolCall("act", {
        "capability": "dev.run", "arguments": {"command": "search cuaca jakarta"}}))

    assert len(tool.actions) == 1


async def test_a_topic_rule_drops_the_line_and_counts_it(aconn):
    kernel, _, _ = await _kernel(aconn)
    trigger = await _trigger(aconn)
    line = Event(id=1, ts=START, kind="percept", who="person:budi", place=GROUP,
                 body="telegram", payload={"description": "aku sedang sakit"})
    other = Event(id=2, ts=START, kind="percept", who="person:budi", place=GROUP,
                  body="telegram", payload={"description": "kapan rapat?"})
    context = _context(trigger, recent=(line, other))

    outcome = await privacy.topic_rule(kernel, ["sakit"])(context, {"conn": aconn})

    assert outcome.item.recent == (other,)
    assert outcome.item.withheld == ((privacy.TOPIC_LABEL, 1),)


async def test_narrowing_never_adds_anything(aconn):
    kernel, _, _ = await _kernel(aconn)
    trigger = await _trigger(aconn)
    context = _context(trigger, recent=(Event(id=1, ts=START, kind="percept",
                                              payload={"description": "biasa saja"}),))

    outcome = await privacy.narrowing(kernel, ["sakit"])(context, {"conn": aconn})

    assert isinstance(outcome, Continue)


def _context(trigger: Event, *, recent=()):
    """Build a minimal desk with the given recent lines."""
    from monoagens.core.context.layout import PFCContext

    return PFCContext(now=START, place=GROUP, reason="test", recent=tuple(recent),
                      trigger=trigger)


class _LoopPFC:
    """Scripted reasoning that loops forever calling `dev.run` until stopped."""

    last_usage: dict[str, int] = {}

    def __init__(self) -> None:
        self.calls = 0

    async def decide(self, context: Any):
        if str(getattr(context, "place", "")) == OWNER_DM:
            return []
        self.calls += 1
        return [ToolCall("act", {"capability": "dev.run", "arguments": {"command": "echo loop"}})]

    async def aclose(self) -> None:
        return None


async def _drive(kernel, *events: Event) -> None:
    """Feed events through the kernel queue until idle."""
    for event in events:
        put = asyncio.create_task(kernel.put(event))
        await asyncio.sleep(0)
        await kernel.step()
        await put
    await kernel.run_until_idle()


async def test_brake_stops_endless_loop_tells_owner_once_and_can_resume(aconn):
    values = settings()
    values["owner_settings"] = {"max_task_steps": 30, "max_wakes_per_hour": 3}
    pfc = _LoopPFC()
    kernel = build(aconn, clock=FakeClock(START), pfc=pfc, settings=values)
    voice = Voice()
    kernel.register_adapter(voice)
    tool = Tool(facts=("reads",))
    kernel.tools.add_tool(tool, {"run": {"facts": ["reads"], "permission": "dev.run"}})
    kernel.roles = open_book("person:owner", "dev.run")
    await append_event(aconn, ts=START, kind="percept", who="person:owner", place=OWNER_DM,
                       body="telegram",
                       payload={"description": "halo", "place_kind": "direct"})

    loop_msg = Event(id=None, ts=START, kind="percept", who="person:owner", place=GROUP,
                     body="telegram", origin_actor="person:owner",
                     payload={"description": "kerjakan terus", "place_kind": "shared",
                              "addressed": True})
    await _drive(kernel, loop_msg)

    assert pfc.calls == 3
    cur = await aconn.execute("SELECT kind, payload FROM events ORDER BY id")
    events = await cur.fetchall()
    kinds = [row[0] for row in events]
    assert kinds.count("brake.held") == 1
    assert kinds.count("guard.stopped") == 1
    assert len(voice.actions) == 1
    prompt_act = voice.actions[0]
    assert prompt_act.kind == "say" and prompt_act.place == OWNER_DM
    assert prompt_act.payload["choices"] == ["Lanjutkan", "Berhenti"]
    notice = prompt_act.payload["text"]
    assert "Pekerjaan dihentikan" in notice and "max_wakes_per_hour" in notice

    reply = Event(id=None, ts=START, kind="percept", who="person:owner", place=OWNER_DM,
                  body="telegram", origin_actor="person:owner",
                  payload={"description": "Lanjutkan", "place_kind": "direct", "addressed": True})
    await _drive(kernel, reply)

    cur = await aconn.execute("SELECT kind FROM events ORDER BY id")
    after = [row[0] for row in await cur.fetchall()]
    assert "brake.resumed" in after
    assert after.count("model.call") > 3
    assert after.count("brake.held") == 2


def test_config_yaml_includes_owner_brake_defaults():
    import pathlib, yaml

    loaded = yaml.safe_load(pathlib.Path("config.yaml").read_text(encoding="utf-8"))
    assert loaded["owner_settings"]["max_task_steps"] == 30
    assert loaded["owner_settings"]["max_wakes_per_hour"] == 20
