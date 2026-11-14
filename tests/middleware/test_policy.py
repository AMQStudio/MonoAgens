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
    await _audiences(aconn)
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


async def _audiences(conn) -> None:
    """Report who can perceive this file's places, the one privacy fact."""
    for place, actors in ((OWNER_DM, ["person:owner"]),
                          (GROUP, ["person:owner", "person:budi"])):
        await append_event(conn, ts=START, kind="place.audience", place=place,
                           payload={"place": place, "actors": actors})


async def _trigger(aconn, place: str = PLACE, who: str = "person:owner") -> Event:
    await _audiences(aconn)
    return await append_event(aconn, ts=START, kind="percept", who=who, place=place,
                              body="telegram", payload={"description": "jalankan"})


async def _kinds(aconn) -> list[str]:
    cur = await aconn.execute("SELECT kind FROM events ORDER BY id")
    return [kind for (kind,) in await cur.fetchall()]


async def _intent(conn) -> Event:
    """Return the retry intent the answer enqueued, read back from the Log."""
    row = await (await conn.execute(
        "SELECT id, ts, who, place, body, origin_actor, payload FROM events WHERE kind = 'intent' "
        "ORDER BY id DESC LIMIT 1")).fetchone()
    assert row is not None, "no retry intent was enqueued"
    return Event(id=row[0], ts=row[1], kind="intent", who=row[2], place=row[3],
                 body=row[4], origin_actor=row[5], payload=row[6])


async def _answer(kernel, text: str) -> None:
    """Type one answer in the owner's direct chat and let the loop process it."""
    put = asyncio.create_task(kernel.put(Event(
        id=None, ts=START, kind="percept", who="person:owner", place=OWNER_DM,
        body="telegram", payload={"description": text, "place_kind": "direct"}, description=text)))
    await asyncio.sleep(0)
    await kernel.step()
    await put
    await kernel.run_until_idle()


async def _call(kernel, trigger: Event, command: str = "ls", event: Event | None = None):
    """Apply one act and let the loop finish, so nothing outlives the test."""
    outcome = await kernel._apply(event or trigger, ToolCall("act", {
        "capability": "dev.run", "arguments": {"command": command}}))
    await kernel.run_until_idle()
    return outcome


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
    assert voice.actions[0].arguments["choices"] == ["Ya", "Tidak"]
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
    await kernel.run_until_idle()

    cur = await aconn.execute("SELECT payload FROM events WHERE kind = 'act.result' "
                              "ORDER BY id DESC LIMIT 1")
    assert "the approved call was different" in (await cur.fetchone())[0]["reason"]
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
    assert "quiet hours" in (await cur.fetchone())[0]["reason"]
    assert voice.actions == []

    owner = await _trigger(aconn)
    await kernel._apply(owner, ToolCall("act", {"capability": "telegram.say",
                                                "arguments": {"text": "halo"}}))
    assert [action.arguments["text"] for action in voice.actions] == ["halo"]


async def test_leaving_no_longer_uses_text_similarity_while_context_hides_private_memory(aconn):
    kernel, tool, _ = await _kernel(aconn, facts=("network",), permission="search.run")
    await append_event(aconn, ts=START, kind="place.audience", place="space:tg:9",
                       payload={"place": "space:tg:9", "actors": ["person:owner"]})
    source = await append_event(aconn, ts=START, kind="percept", who="person:owner",
                                place="space:tg:9", body="telegram",
                                payload={"description": "rahasia kantor pindah"})
    await aconn.execute("INSERT INTO memories (text, derived_from) VALUES (%s, %s)",
                        ("rahasia kantor", [source.id]))
    await append_event(aconn, ts=START, kind="percept", who="person:budi", place=GROUP,
                       body="telegram", payload={"description": "halo semua"})
    trigger = await _trigger(aconn, place=GROUP, who="person:owner")

    await kernel._apply(trigger, ToolCall("act", {
        "capability": "dev.run", "arguments": {"command": "search rahasia kantor"}}))
    await kernel.run_until_idle()

    assert len(tool.actions) == 1
    assert await privacy.memory_visible(aconn, context=GROUP, sources=[source.id]) is False


async def test_a_public_search_still_passes(aconn):
    kernel, tool, _ = await _kernel(aconn, facts=("network",), permission="search.run")
    trigger = await _trigger(aconn)

    await kernel._apply(trigger, ToolCall("act", {
        "capability": "dev.run", "arguments": {"command": "search cuaca jakarta"}}))

    assert len(tool.actions) == 1


def _context(trigger: Event, *, recent=()):
    """Build a minimal desk with the given recent lines."""
    from monoagens.core.context.layout import PFCContext

    return context(now=START, place=GROUP, reason="test", recent=tuple(recent),
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
    await _audiences(aconn)
    await append_event(aconn, ts=START, kind="percept", who="person:owner", place=OWNER_DM,
                       body="telegram", payload={"description": "halo"})

    loop_msg = Event(id=None, ts=START, kind="percept", who="person:owner", place=GROUP,
                     body="telegram", origin_actor="person:owner",
                     description="kerjakan terus", features={"addressed": 1})
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
    assert prompt_act.arguments["choices"] == ["Lanjutkan", "Berhenti"]
    notice = prompt_act.arguments["text"]
    assert "Pekerjaan dihentikan" in notice and "max_wakes_per_hour" in notice

    reply = Event(id=None, ts=START, kind="percept", who="person:owner", place=OWNER_DM,
                  body="telegram", origin_actor="person:owner",
                  payload={"description": "Lanjutkan", "place_kind": "direct", "addressed": True}, description="Lanjutkan")
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


async def test_approved_retry_keeps_requester_origin_and_is_single_use(aconn):
    kernel, tool, _ = await _kernel(aconn)
    kernel.roles = open_book("person:alice", "dev.run")
    await append_event(aconn, ts=START, kind="percept", who="person:owner", place=OWNER_DM,
                       body="telegram", payload={"description": "hi", "place_kind": "direct"})
    trigger = await append_event(
        aconn, ts=START, kind="percept", who="person:alice", place=GROUP, body="telegram",
        origin_actor="person:alice",
        payload={"description": "run it", "place_kind": "shared"})
    await _call(kernel, trigger, command="ls")
    assert tool.actions == []

    await _answer(kernel, "ya")
    queued = await _intent(aconn)
    assert queued.origin_actor == "person:alice"

    await _call(kernel, trigger, command="ls", event=queued)
    assert len(tool.actions) == 1
    assert (await _kinds(aconn)).count("approval.consumed") == 1

    # Second identical act with the same approval requires a new approval.
    await _call(kernel, trigger, command="ls", event=queued)
    assert len(tool.actions) == 1
    assert (await _kinds(aconn)).count("approval.requested") == 2


async def test_brake_continue_keeps_held_chain_origin_actor(aconn):
    values = settings()
    values["owner_settings"] = {"max_task_steps": 30, "max_wakes_per_hour": 2}
    pfc = _LoopPFC()
    kernel = build(aconn, clock=FakeClock(START), pfc=pfc, settings=values)
    kernel.register_adapter(Voice())
    kernel.tools.add_tool(Tool(facts=("reads",)), {"run": {"facts": ["reads"],
                                                          "permission": "dev.run"}})
    kernel.roles = open_book("person:alice", "dev.run")
    await _audiences(aconn)
    await append_event(aconn, ts=START, kind="percept", who="person:owner", place=OWNER_DM,
                       body="telegram", payload={"description": "halo", "place_kind": "direct"})
    msg = Event(id=None, ts=START, kind="percept", who="person:alice", place=GROUP,
                body="telegram", origin_actor="person:alice",
                payload={"description": "go", "place_kind": "shared", "addressed": True}, description="go")
    await _drive(kernel, msg)

    reply = Event(id=None, ts=START, kind="percept", who="person:owner", place=OWNER_DM,
                  body="telegram", origin_actor="person:owner",
                  payload={"description": "Lanjutkan", "place_kind": "direct", "addressed": True}, description="Lanjutkan")
    put = asyncio.create_task(kernel.put(reply))
    await asyncio.sleep(0)
    await kernel.step()
    await put
    row = await (await aconn.execute(
        "SELECT origin_actor FROM events WHERE kind = 'intent' ORDER BY id DESC LIMIT 1"
    )).fetchone()
    # Wait until idle so resume intent is recorded if processed, or check queued item first
    if row is None:
        queued = kernel.queue.get_nowait().event
        assert queued.origin_actor == "person:alice"
        assert queued.who == "person:alice"
    else:
        assert row[0] == "person:alice"


async def test_typed_approval_requires_direct_place_30m_and_newest_only(aconn):
    from datetime import timedelta

    clock = FakeClock(START)
    values = settings()
    kernel = build(aconn, clock=clock, settings=values)
    tool, voice = Tool(("runs_code",), "dev.run"), Voice()
    kernel.register_adapter(voice)
    kernel.tools.add_tool(tool, {"run": {"facts": ["runs_code"], "permission": "dev.run"}})
    kernel.roles = open_book("person:owner", "dev.run")
    await _audiences(aconn)
    await append_event(aconn, ts=START, kind="percept", who="person:owner", place=OWNER_DM,
                       body="telegram", payload={"description": "hi"})
    trigger = await _trigger(aconn)

    # Request 1 in GROUP; owner types "ya" in GROUP (no reply_to) -> must NOT count.
    await _call(kernel, trigger, command="echo 1")
    await _drive(kernel, Event(id=None, ts=clock.now(), kind="percept", who="person:owner",
                               place=GROUP, body="telegram", origin_actor="person:owner",
                               payload={"description": "ya", "place_kind": "shared"}, description="ya"))
    assert (await _kinds(aconn)).count("approval.answered") == 0

    # Request 2 is newer; answering "ya" in DM answers #2, never #1 on a second "ya".
    await _call(kernel, trigger, command="echo 2")
    req_rows = await (await aconn.execute(
        "SELECT id FROM events WHERE kind = 'approval.requested' ORDER BY id")).fetchall()
    first_id, second_id = int(req_rows[0][0]), int(req_rows[1][0])
    await _drive(kernel, Event(id=None, ts=clock.now(), kind="percept", who="person:owner",
                               place=OWNER_DM, body="telegram", origin_actor="person:owner",
                               payload={"description": "ya", "place_kind": "direct"}, description="ya"))
    ans_rows = await (await aconn.execute(
        "SELECT payload->>'request' FROM events WHERE kind = 'approval.answered'")).fetchall()
    assert [int(r[0]) for r in ans_rows] == [second_id]

    await _drive(kernel, Event(id=None, ts=clock.now(), kind="percept", who="person:owner",
                               place=OWNER_DM, body="telegram", origin_actor="person:owner",
                               payload={"description": "ya", "place_kind": "direct"}, description="ya"))
    ans_rows = await (await aconn.execute(
        "SELECT payload->>'request' FROM events WHERE kind = 'approval.answered'")).fetchall()
    assert [int(r[0]) for r in ans_rows] == [second_id]

    # After 30 minutes, #1 expires with approval.expired and even a reply cannot approve it.
    clock.advance(timedelta(minutes=31))
    await _drive(kernel, Event(id=None, ts=clock.now(), kind="percept", who="person:owner",
                               place=OWNER_DM, body="telegram", origin_actor="person:owner",
                               payload={"description": "ya", "place_kind": "direct"}, description="ya"))
    exp_rows = await (await aconn.execute(
        "SELECT payload->>'request' FROM events WHERE kind = 'approval.expired'")).fetchall()
    assert [int(r[0]) for r in exp_rows] == [first_id]
    ans_rows = await (await aconn.execute(
        "SELECT payload->>'request' FROM events WHERE kind = 'approval.answered'")).fetchall()
    assert [int(r[0]) for r in ans_rows] == [second_id]
