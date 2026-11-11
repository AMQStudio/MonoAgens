"""Tests for Part 3: privacy follows people, sharing, reaching a person, threads."""

from __future__ import annotations

from types import SimpleNamespace
from datetime import datetime, timezone
from typing import Any

from monoagens import hooks, memory, reach, threads
from monoagens.clock import FakeClock
from monoagens.kernel import Kernel
from monoagens.pfc import PFC, ToolCall
from monoagens.store import append_event, read_state, write_state
from monoagens.types import Action, Event
from tests.support import settings

START = datetime(2026, 11, 11, 8, 0, tzinfo=timezone.utc)
SETTINGS = settings()
OWNER = "person:owner"
FULAN_A = "space:wa:fulan1"
FULAN_B = "space:tg:fulan2"
BUDI = "space:wa:budi"
GROUP = "space:tg:-500"


async def _said(conn, who: str, place: str, kind: str, text: str = "halo",
               body: str = "telegram") -> int:
    event = await append_event(conn, ts=START, kind="percept", who=who, place=place,
                               body=body,
                               payload={"description": text, "place_kind": kind})
    return int(event.id)


async def _link(conn, native: str, person: str, after: int) -> None:
    await write_state(conn, f"alias:{native}", {"who": person}, after)


class Body:
    """A body that records the actions handed to it."""

    def __init__(self, name: str, *, long_running: bool = False) -> None:
        self.name, self.actions, self.long_running = name, [], long_running

    def capabilities(self) -> tuple[dict[str, Any], ...]:
        """Declare a chat capability and, for `dev`, a long-running one."""
        say = {"name": "say", "description": "Speak here.", "schema": {"type": "object"},
               "communicative": True, "long_running": False, "identity_changing": False}
        run = {"name": "run", "description": "Run code.", "schema": {"type": "object"},
               "communicative": False, "long_running": True, "identity_changing": False}
        return (say, run) if self.long_running else (say,)

    async def act(self, action: Action) -> dict[str, Any]:
        """Record the action; a long action reports its completion."""
        self.actions.append(action)
        return {"ok": True} if action.kind == "run" else {}


async def _kernel(conn, *, settings_value=None, pfc=None, embedder=None) -> Kernel:
    kernel = Kernel(conn, clock=FakeClock(START), settings=settings_value or SETTINGS,
                    pfc=pfc, embedder=embedder)
    kernel.register_body(Body("whatsapp"))
    kernel.register_body(Body("telegram"))
    kernel.register_body(Body("dev", long_running=True))
    return kernel


def _trigger(event_id: int, place: str, who: str = OWNER) -> Event:
    return Event(id=event_id, ts=START, kind="percept", who=who, place=place,
                 payload={"description": "kerjakan"}, origin_actor=who)


async def test_a_private_chat_follows_the_person_across_bodies_once_linked(aconn):
    a = await _said(aconn, "person:wa:fulan", FULAN_A, "direct")
    await _said(aconn, "person:tg:77", FULAN_B, "direct")
    assert not await reach.visible(aconn, context=FULAN_B, source=FULAN_A)

    await _link(aconn, "person:tg:77", "person:wa:fulan", a)
    assert await reach.visible(aconn, context=FULAN_B, source=FULAN_A)


async def test_a_private_chat_is_never_visible_in_a_group_with_anyone_else(aconn):
    await _said(aconn, "person:fulan", FULAN_A, "direct")
    await _said(aconn, "person:charlie", GROUP, "shared")
    assert not await reach.visible(aconn, context=GROUP, source=FULAN_A)

    await _said(aconn, "person:fulan", FULAN_B, "direct")
    assert await reach.visible(aconn, context=FULAN_B, source=FULAN_A)


async def test_a_private_group_flows_to_a_direct_chat_with_one_member(aconn):
    await _said(aconn, "person:fulan", GROUP, "shared")
    await _said(aconn, "person:charlie", GROUP, "shared")
    await _said(aconn, "person:fulan", FULAN_A, "direct")
    assert await reach.visible(aconn, context=FULAN_A, source=GROUP)
    await _said(aconn, "person:charlie", FULAN_B, "direct")
    assert await reach.visible(aconn, context=FULAN_B, source=GROUP)


async def test_the_owner_direct_chat_is_a_view_but_a_group_is_not(aconn):
    await _said(aconn, "person:fulan", FULAN_A, "direct")
    owner_dm = await _said(aconn, OWNER, "space:tg:owner", "direct")
    assert await reach.owner_direct(aconn, "space:tg:owner")
    assert await reach.visible(aconn, context="space:tg:owner", source=FULAN_A)

    await _say(conn=aconn, who="person:charlie", place=GROUP, kind="shared")
    assert not await reach.visible(aconn, context=GROUP, source=FULAN_A)
    assert owner_dm


async def _say(*, conn, who: str, place: str, kind: str) -> int:
    return await _said(conn, who, place, kind)


async def test_context_hook_keeps_private_lines_and_memories_inside_the_audience(aconn):
    trigger_id = await _said(aconn, "person:fulan", FULAN_A, "direct", "aku di-PHK")
    await _said(aconn, "person:charlie", GROUP, "shared")
    context = await memory.build_context(
        aconn, _trigger(trigger_id, GROUP), GROUP, "test", now=START, settings=SETTINGS,
        memories=(), capabilities=())
    filtered = await hooks.audience_policy()(context, {"conn": aconn, "now": START})
    assert all(line.place != FULAN_A for line in filtered.item.recent_elsewhere)

    owner_dm = await _said(aconn, OWNER, "space:tg:owner", "direct")
    context = await memory.build_context(
        aconn, _trigger(owner_dm, "space:tg:owner"), "space:tg:owner", "test", now=START,
        settings=SETTINGS, memories=(), capabilities=())
    filtered = await hooks.audience_policy()(context, {"conn": aconn, "now": START})
    assert any(line.place == FULAN_A for line in filtered.item.recent_elsewhere)


async def test_share_with_a_person_records_consent_and_counts_as_audience(aconn):
    at = await _said(aconn, OWNER, GROUP, "shared", "ini hasil kerjaan kita")
    await _said(aconn, "person:fulan", GROUP, "shared")
    await _said(aconn, "person:charlie", "space:tg:charlie", "direct")
    assert await reach.visible(aconn, context="space:tg:charlie", source=GROUP) is False
    kernel = await _kernel(aconn)
    await kernel._apply(_trigger(at, GROUP), ToolCall("act", {
        "capability": "self.share", "arguments": {"item": "hasil kerjaan kita",
                                                  "to": "person:charlie"}}))
    rows = await (await aconn.execute("SELECT payload FROM events WHERE kind = 'share.granted'")
                  ).fetchall()
    assert rows and rows[0][0]["persons"] == ["person:charlie"]
    assert await reach.visible(aconn, context="space:tg:charlie", source=GROUP) is True


async def test_only_a_person_who_was_part_of_it_can_share_it(aconn):
    fulan = await _said(aconn, "person:fulan", FULAN_A, "direct", "aku di-PHK")
    await _said(aconn, OWNER, GROUP, "shared", "ini hasil kerjaan kita")
    kernel = await _kernel(aconn)
    seen: list[Any] = []

    async def spy(item, ctx):
        seen.append(item)
        return hooks.Continue()

    kernel.hooks.action.add("spy", spy)
    await kernel._apply(_trigger(fulan, FULAN_A), ToolCall("act", {
        "capability": "self.share",
        "arguments": {"episode": fulan, "to": "person:budi"}}))
    assert seen == []  # the action hook never sees content the actor was not part of
    kinds = [row[0] for row in await (await aconn.execute(
        "SELECT kind FROM events ORDER BY id")).fetchall()]
    assert "share.granted" not in kinds
    assert "act.result" in kinds

    # Fulan's own chain may share Fulan's line: the hook sees it and consent is recorded.
    await kernel._apply(Event(id=fulan, ts=START, kind="percept", who="person:fulan",
                              place=FULAN_A, origin_actor="person:fulan"),
                        ToolCall("act", {"capability": "self.share", "arguments": {
                            "episode": fulan, "to": "person:budi"}}))
    assert seen
    assert await (await aconn.execute(
        "SELECT count(*) FROM events WHERE kind = 'share.granted'")).fetchone() == (1,)


async def test_share_with_a_place_grants_everyone_present(aconn):
    at = await _said(aconn, OWNER, GROUP, "shared")
    await _said(aconn, "person:fulan", GROUP, "shared")
    kernel = await _kernel(aconn)
    await kernel._apply(_trigger(at, GROUP), ToolCall("act", {
        "capability": "self.share", "arguments": {"item": "kode", "to": GROUP}}))
    row = await (await aconn.execute(
        "SELECT payload FROM events WHERE kind = 'share.granted'")).fetchone()
    assert row[0]["persons"] == ["person:fulan", OWNER]


async def test_a_message_to_another_place_becomes_an_intent_there(aconn):
    await _said(aconn, "person:budi", BUDI, "direct", body="whatsapp")
    kernel = await _kernel(aconn)
    at = await _said(aconn, OWNER, GROUP, "shared")
    await kernel._apply(_trigger(at, GROUP), ToolCall("act", {
        "capability": "whatsapp.say",
        "arguments": {"text": "halo Budi", "to": "person:budi", "via": "whatsapp"}}))
    row = await (await aconn.execute(
        "SELECT place, payload FROM events WHERE kind = 'act.intent'")).fetchone()
    assert row[0] == GROUP
    assert row[1]["to"] == "person:budi"
    assert row[1]["target"] == BUDI
    assert await (await aconn.execute(
        "SELECT count(*) FROM events WHERE kind = 'act.out'")).fetchone() == (0,)
    queued = kernel.queue.get_nowait()
    assert queued.event.kind == "intent"
    assert queued.event.place == BUDI
    assert queued.event.payload["place_kind"] == "direct"


async def test_act_to_an_unreachable_person_fails_honestly(aconn):
    kernel = await _kernel(aconn)
    at = await _said(aconn, OWNER, GROUP, "shared")
    await kernel._apply(_trigger(at, GROUP), ToolCall("act", {
        "capability": "whatsapp.say", "arguments": {"text": "halo", "to": "person:budi"}}))
    row = await (await aconn.execute(
        "SELECT payload FROM events WHERE kind = 'act.result'")).fetchone()
    assert "no way to reach budi yet" in row[0]["result"]["error"]


async def test_act_offers_the_capabilities_of_every_running_organ(aconn):
    kernel = await _kernel(aconn)
    assert {"whatsapp.say", "telegram.say", "dev.run", "self.share"} <= set(
        kernel.organs.capabilities)


class _ScriptedPFC:
    """Replay one tool-call list per decide call and keep every desk it was given."""

    def __init__(self, answers: list[list[ToolCall]]) -> None:
        """Start with the answers in call order."""
        self.answers, self.contexts, self.last_usage = list(answers), [], {}

    async def decide(self, context: Any) -> list[ToolCall]:
        """Return the next answer and remember what the desk showed."""
        self.contexts.append(context)
        return self.answers.pop(0)


async def test_a_paraphrase_of_a_private_line_is_not_on_the_other_places_desk(aconn):
    await _said(aconn, "person:fulan", FULAN_A, "direct", "aku di-PHK")
    await aconn.execute(
        "INSERT INTO memories (who, place, text, importance, private) "
        "VALUES (%s, %s, %s, 0.9, true)", ("person:fulan", FULAN_A, "aku di-PHK"))
    await _said(aconn, "person:budi", BUDI, "direct", body="whatsapp")
    owner_dm = await _said(aconn, OWNER, "space:tg:owner_dm", "direct")
    pfc = _ScriptedPFC([
        [ToolCall("act", {"capability": "whatsapp.say",
                          "arguments": {"text": "Halo Budi, ada kabar?"}})],
    ])
    kernel = await _kernel(aconn, pfc=pfc)
    await kernel._apply(_trigger(owner_dm, "space:tg:owner_dm"), ToolCall("act", {
        "capability": "whatsapp.say", "arguments": {
            "text": "Fulan bilang: aku di-PHK", "to": "person:budi", "via": "whatsapp"}}))
    await kernel.run_until_idle()

    assert len(pfc.contexts) == 1
    assert pfc.contexts[0].place == BUDI
    prompt = "\n".join(message["content"] for message in
                       PFC("http://unused", "", "fake", settings=SETTINGS)
                       .build_messages(pfc.contexts[0]))
    assert "di-PHK" not in prompt
    intent = await (await aconn.execute(
        "SELECT place, payload FROM events WHERE kind = 'act.intent'")).fetchone()
    assert intent[0] == "space:tg:owner_dm"
    assert "di-PHK" in intent[1]["arguments"]["text"]
    spoken = await (await aconn.execute(
        "SELECT place, payload FROM events WHERE kind = 'act.out'")).fetchone()
    assert spoken[0] == BUDI
    assert "di-PHK" not in spoken[1]["arguments"]["text"]
    assert await reach.leaks(aconn, FULAN_A, BUDI, "Fulan bilang: aku di-PHK") is True


async def test_a_long_action_opens_a_task_thread_and_it_closes_when_done(aconn):
    kernel = await _kernel(aconn)
    at = await _said(aconn, OWNER, GROUP, "shared")
    await kernel._apply(_trigger(at, GROUP), ToolCall("act", {
        "capability": "dev.run", "arguments": {"command": "pytest"}}))
    opened = await (await aconn.execute(
        "SELECT place, payload FROM events WHERE kind = 'task.opened'")).fetchone()
    task_place = opened[0]
    assert (await read_state(aconn, f"task:{task_place}"))["status"] == "open"

    job = SimpleNamespace(name="pfc", place=task_place)
    result = await append_event(aconn, ts=START, kind="job.result", place=task_place,
                                payload={"job": "pfc"})
    call = {"name": "act", "arguments": {"capability": "whatsapp.say",
                                         "arguments": {"to": "person:owner"}}}
    await threads.after_step(kernel, job, result, [call])
    assert (await read_state(aconn, f"task:{task_place}"))["waiting_for"] == OWNER

    call = {"name": "act", "arguments": {"capability": "whatsapp.say",
                                         "arguments": {"to": GROUP}}}
    await threads.after_step(kernel, job, result, [call])
    assert (await read_state(aconn, f"task:{task_place}"))["status"] == "closed"
    assert await (await aconn.execute(
        "SELECT count(*) FROM events WHERE kind = 'task.closed'")).fetchone() == (1,)


async def test_open_task_threads_are_limited_by_the_prior(aconn):
    kernel = await _kernel(aconn, settings_value=settings(constants={"max_open_tasks": 0}))
    at = await _said(aconn, OWNER, GROUP, "shared")
    await kernel._apply(_trigger(at, GROUP), ToolCall("act", {
        "capability": "dev.run", "arguments": {"command": "pytest"}}))
    assert await (await aconn.execute(
        "SELECT count(*) FROM events WHERE kind = 'task.opened'")).fetchone() == (0,)


async def test_a_chain_that_exceeds_max_task_steps_closes(aconn):
    kernel = await _kernel(aconn, settings_value=settings(constants={"max_task_steps": 1}))
    at = await _said(aconn, OWNER, GROUP, "shared")
    await kernel._apply(_trigger(at, GROUP), ToolCall("act", {
        "capability": "dev.run", "arguments": {"command": "pytest"}}))
    task_place = (await (await aconn.execute(
        "SELECT place FROM events WHERE kind = 'task.opened'")).fetchone())[0]
    job = SimpleNamespace(name="pfc", place=task_place)
    result = await append_event(aconn, ts=START, kind="job.result", place=task_place,
                                payload={"job": "pfc"})
    await threads.after_step(kernel, job, result, [
        {"name": "act", "arguments": {"capability": "whatsapp.say",
                                      "arguments": {"text": "selesai"}}}])
    row = await (await aconn.execute(
        "SELECT payload FROM events WHERE kind = 'task.closed'")).fetchone()
    assert row[0]["reason"] == "max_task_steps"
