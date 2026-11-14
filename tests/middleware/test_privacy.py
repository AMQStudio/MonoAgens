"""Tests for Part 3: privacy follows people, sharing, reaching a person, threads."""

from __future__ import annotations

from dataclasses import replace
from types import SimpleNamespace
from datetime import datetime, timedelta, timezone
from typing import Any

from monoagens.core import expectations, hooks, memory, threads
from monoagens.core.context import layout
from monoagens.middleware import identity, privacy
from monoagens.middleware.contract import compose
from monoagens.infra.clock import FakeClock
from monoagens.infra.embedding import FakeEmbedder
from monoagens.core.kernel import Kernel
from monoagens.core.reasoning import Reasoning, ToolCall
from monoagens.infra.store import append_event, read_state, write_state
from monoagens.types import Action, Event
from monoagens.boot import build
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
        say = {"name": "say", "description": "Speak here.", "schema": {
                   "type": "object", "properties": {"text": {"type": "string"}},
                   "required": ["text"]},
               "communicative": True, "long_running": False, "identity_changing": False,
               "permission": None}
        run = {"name": "run", "description": "Run code.", "schema": {
                   "type": "object", "properties": {"command": {"type": "string"}},
                   "required": ["command"]},
               "communicative": False, "long_running": True, "identity_changing": False,
               "permission": "dev.run"}
        return (say, run) if self.long_running else (say,)

    async def act(self, action: Action) -> dict[str, Any]:
        """Record the action; a long action reports its completion."""
        self.actions.append(action)
        return {"ok": True} if action.kind == "run" else {}


async def _kernel(conn, *, settings_value=None, pfc=None, embedder=None) -> Kernel:
    kernel = build(conn, clock=FakeClock(START), settings=settings_value or SETTINGS,
                    pfc=pfc, embedder=embedder)
    kernel.register_adapter(Body("whatsapp"))
    kernel.register_adapter(Body("telegram"))
    kernel.register_adapter(Body("dev", long_running=True))
    return kernel


def _trigger(event_id: int, place: str, who: str = OWNER,
             text: str = "kerjakan") -> Event:
    return Event(id=event_id, ts=START, kind="percept", who=who, place=place,
                 payload={"description": text}, description=text, origin_actor=who)


async def test_a_private_chat_follows_the_person_across_bodies_once_linked(aconn):
    a = await _said(aconn, "person:wa:fulan", FULAN_A, "direct")
    await _said(aconn, "person:tg:77", FULAN_B, "direct")
    assert not await privacy.visible(aconn, context=FULAN_B, source=FULAN_A)

    await _link(aconn, "person:tg:77", "person:wa:fulan", a)
    assert await privacy.visible(aconn, context=FULAN_B, source=FULAN_A)


async def test_a_private_chat_is_never_visible_in_a_group_with_anyone_else(aconn):
    await _said(aconn, "person:fulan", FULAN_A, "direct")
    await _said(aconn, "person:charlie", GROUP, "shared")
    assert not await privacy.visible(aconn, context=GROUP, source=FULAN_A)

    await _said(aconn, "person:fulan", FULAN_B, "direct")
    assert await privacy.visible(aconn, context=FULAN_B, source=FULAN_A)


async def test_a_private_group_flows_to_a_direct_chat_with_one_member(aconn):
    await _said(aconn, "person:fulan", GROUP, "shared")
    await _said(aconn, "person:charlie", GROUP, "shared")
    await _said(aconn, "person:fulan", FULAN_A, "direct")
    assert await privacy.visible(aconn, context=FULAN_A, source=GROUP)
    await _said(aconn, "person:charlie", FULAN_B, "direct")
    assert await privacy.visible(aconn, context=FULAN_B, source=GROUP)


async def test_a_memory_needs_every_source_to_cover_the_place(aconn):
    second_group, dewi_dm, budi_dm = "space:tg:-600", "space:tg:dewi", "space:tg:budi"
    await _said(aconn, "person:charlie", GROUP, "shared", "rapat jam sembilan")
    one = await _said(aconn, "person:dewi", GROUP, "shared", "rapat jam sembilan")
    await _said(aconn, "person:budi", second_group, "shared", "rapat jam sembilan")
    two = await _said(aconn, "person:dewi", second_group, "shared", "rapat jam sembilan")
    await _said(aconn, "person:dewi", dewi_dm, "direct")
    await _said(aconn, "person:budi", budi_dm, "direct")
    await _said(aconn, OWNER, OWNER_DM, "direct")

    assert await privacy.memory_visible(aconn, context=GROUP, sources=(one,)) is True
    assert await privacy.memory_visible(aconn, context=GROUP, sources=(one, two)) is False
    assert await privacy.memory_visible(aconn, context=dewi_dm, sources=(one, two)) is True
    assert await privacy.memory_visible(aconn, context=budi_dm, sources=(one, two)) is False
    assert await privacy.memory_visible(aconn, context=OWNER_DM, sources=(one, two)) is True


async def test_the_owner_direct_chat_is_a_view_but_a_group_is_not(aconn):
    await _said(aconn, "person:fulan", FULAN_A, "direct")
    owner_dm = await _said(aconn, OWNER, "space:tg:owner", "direct")
    assert await privacy.owner_direct(aconn, "space:tg:owner")
    assert await privacy.visible(aconn, context="space:tg:owner", source=FULAN_A)

    await _say(conn=aconn, who="person:charlie", place=GROUP, kind="shared")
    assert not await privacy.visible(aconn, context=GROUP, source=FULAN_A)
    assert owner_dm


async def _say(*, conn, who: str, place: str, kind: str) -> int:
    return await _said(conn, who, place, kind)


OWNER_DM = "space:tg:owner"


async def _desk(conn, *, place: str, trigger: int, memories: tuple = (),
                text: str = "kerjakan"):
    """Lay out one desk and pass it through the context hook's lines and the cards."""
    event = _trigger(trigger, place, text=text)
    context = await layout.build_context(
        conn, event, place, "test", now=START, settings=SETTINGS, memories=memories,
        capabilities=())
    ctx = {"conn": conn, "event": event, "now": START}
    return await compose(privacy.filter_context(conn), identity.cards(conn))(context, ctx)


async def test_the_owners_direct_chat_is_private_to_the_owner(aconn):
    dm = await _said(aconn, OWNER, OWNER_DM, "direct", "rapat jam sembilan")
    asked = await _said(aconn, "person:budi", GROUP, "shared", "rapat jam berapa?")
    fact = memory.MemoryHit("- rapat jam sembilan (owner at space:tg:owner)",
                            place=OWNER_DM, sources=(dm,))

    assert await privacy.visible(aconn, context=GROUP, source=OWNER_DM) is False
    assert await privacy.visible(aconn, context=OWNER_DM, source=OWNER_DM) is True

    group_desk = await _desk(aconn, place=GROUP, trigger=asked, memories=(fact,))
    assert group_desk.item.memories == ()
    assert all(line.place != OWNER_DM for line in group_desk.item.recent_elsewhere)

    owner_desk = await _desk(aconn, place=OWNER_DM, trigger=dm, memories=(fact,))
    assert owner_desk.item.memories == (fact,)

    kernel = await _kernel(aconn)
    await kernel._apply(_trigger(dm, OWNER_DM), ToolCall("act", {
        "capability": "self.share",
        "arguments": {"item": "rapat jam sembilan", "to": GROUP}}))
    assert await privacy.visible(aconn, context=GROUP, source=OWNER_DM, before=dm) is True
    assert await privacy.visible(aconn, context=GROUP, source=OWNER_DM) is False

    shared_desk = await _desk(aconn, place=GROUP, trigger=asked, memories=(fact,))
    assert shared_desk.item.memories == (fact,)


async def test_a_digit_string_episode_is_accepted_and_other_values_name_the_field(aconn):
    dm = await _said(aconn, OWNER, OWNER_DM, "direct", "Rapat tim besok jam sembilan pagi")
    asked = await _said(aconn, "person:budi", GROUP, "shared", "rapat jam berapa?")
    kernel = await _kernel(aconn)

    await kernel._apply(_trigger(dm, OWNER_DM), ToolCall("act", {
        "capability": "self.share",
        "arguments": {"item": "Rapat tim besok jam sembilan pagi", "episode": str(dm),
                      "to": GROUP}}))
    cur = await aconn.execute("SELECT payload FROM events WHERE kind = 'share.granted' "
                              "ORDER BY id DESC LIMIT 1")
    row = await cur.fetchone()
    assert row is not None, "the share was not granted"
    assert row[0]["episode"] == dm and isinstance(row[0]["episode"], int)
    assert row[0]["source_place"] == OWNER_DM
    assert row[0]["to"] == GROUP

    await kernel._apply(_trigger(asked, GROUP), ToolCall("act", {
        "capability": "self.share",
        "arguments": {"item": "rapat jam sembilan", "episode": "besok", "to": GROUP}}))
    await kernel.run_until_idle()
    cur = await aconn.execute("SELECT payload FROM events WHERE kind = 'act.result' "
                              "ORDER BY id DESC LIMIT 1")
    result = (await cur.fetchone())[0]
    assert "episode: expected an event id made of digits" in result["reason"]

    await kernel._apply(_trigger(asked, GROUP), ToolCall("act", {
        "capability": "self.share",
        "arguments": {"item": "rapat jam sembilan", "episode": 1.5, "to": GROUP}}))
    cur = await aconn.execute("SELECT payload FROM events WHERE kind = 'act.result' "
                              "ORDER BY id DESC LIMIT 1")
    error = (await cur.fetchone())[0]["reason"]
    assert "arguments.episode: expected string, got number" in error
    count = await (await aconn.execute(
        "SELECT count(*) FROM events WHERE kind = 'share.granted'")).fetchone()
    assert count[0] == 1


async def test_context_hook_keeps_private_lines_and_memories_inside_the_audience(aconn):
    trigger_id = await _said(aconn, "person:fulan", FULAN_A, "direct", "aku di-PHK")
    await _said(aconn, "person:charlie", GROUP, "shared")
    context = await layout.build_context(
        aconn, _trigger(trigger_id, GROUP), GROUP, "test", now=START, settings=SETTINGS,
        memories=(), capabilities=())
    filtered = await privacy.filter_context(aconn)(context, {"conn": aconn, "now": START})
    assert all(line.place != FULAN_A for line in filtered.item.recent_elsewhere)

    owner_dm = await _said(aconn, OWNER, "space:tg:owner", "direct")
    context = await layout.build_context(
        aconn, _trigger(owner_dm, "space:tg:owner"), "space:tg:owner", "test", now=START,
        settings=SETTINGS, memories=(), capabilities=())
    filtered = await privacy.filter_context(aconn)(context, {"conn": aconn, "now": START})
    assert any(line.place == FULAN_A for line in filtered.item.recent_elsewhere)


async def test_a_newcomer_never_receives_a_line_said_before_they_arrived(aconn):
    said = await _said(aconn, OWNER, GROUP, "shared", "Budi mengirim sketsa_logo.png")
    asked = await _said(aconn, OWNER, GROUP, "shared", "Budi kirim file apa ya?")
    assert await privacy.visible(aconn, context=GROUP, source=GROUP, before=said) is True
    context = await layout.build_context(
        aconn, _trigger(asked, GROUP), GROUP, "test", now=START, settings=SETTINGS,
        memories=(), capabilities=())
    desk = await privacy.filter_context(aconn)(context, {"conn": aconn, "now": START})
    assert [line.id for line in desk.item.recent] == [said]

    joined = await _said(aconn, "person:charlie", GROUP, "shared", "eh aku juga di sini")
    later = await _said(aconn, OWNER, GROUP, "shared", "Budi kirim file apa tadi?")
    assert await privacy.visible(aconn, context=GROUP, source=GROUP, before=said) is False
    assert await privacy.visible(aconn, context=GROUP, source=GROUP, before=joined) is True
    assert await privacy.visible(aconn, context=GROUP, source=GROUP, before=later) is True
    context = await layout.build_context(
        aconn, _trigger(later, GROUP), GROUP, "test", now=START, settings=SETTINGS,
        memories=(), capabilities=())
    desk = await privacy.filter_context(aconn)(context, {"conn": aconn, "now": START})
    assert [line.id for line in desk.item.recent] == [joined]


async def test_a_withheld_line_leaves_a_note_with_its_source_but_not_its_text(aconn):
    """D56: the note says how many and whose private chat; the words stay off the desk."""
    await _said(aconn, OWNER, OWNER_DM, "direct", "rapat jam sembilan")
    asked = await _said(aconn, "person:budi", GROUP, "shared", "rapat jam berapa?")
    desk = (await _desk(aconn, place=GROUP, trigger=asked)).item

    assert desk.recent_elsewhere == ()
    assert desk.withheld == (("person:owner private chat", 1),)
    prompt = _prompt(desk)
    assert "1 thing you know from person:owner private chat is not shown here" in prompt
    assert "sembilan" not in prompt


async def test_a_withheld_memory_leaves_a_note_with_its_source_but_not_its_text(aconn):
    """D56: a retrieved private memory is counted and labelled, never quoted."""
    dm = await _said(aconn, OWNER, OWNER_DM, "direct", "rapat jam sembilan")
    asked = await _said(aconn, "person:budi", GROUP, "shared", "rapat jam berapa?")
    fact = memory.MemoryHit("- rapat jam sembilan (owner at space:tg:owner)",
                            place=OWNER_DM, sources=(dm,))
    context = await layout.build_context(
        aconn, _trigger(asked, GROUP), GROUP, "test", now=START, settings=SETTINGS,
        memories=(fact,), capabilities=())
    context = replace(context, recent_elsewhere=())
    desk = (await privacy.filter_context(aconn)(context, {"conn": aconn, "now": START})).item

    assert desk.memories == ()
    assert desk.withheld == (("person:owner private chat", 1),)
    prompt = _prompt(desk)
    assert "1 thing you know from person:owner private chat is not shown here" in prompt
    assert "sembilan" not in prompt


async def test_no_withheld_note_when_nothing_was_withheld(aconn):
    asked = await _said(aconn, "person:budi", GROUP, "shared", "halo semua")
    desk = (await _desk(aconn, place=GROUP, trigger=asked)).item

    assert desk.withheld == ()
    assert "<withheld>" not in _prompt(desk)


def _prompt(desk) -> str:
    """Render the three prompt messages into one string, as the Reasoning sends them."""
    return "\n".join(message["content"] for message in
                     Reasoning(settings=SETTINGS).build_messages(desk))


async def test_share_with_a_person_records_consent_and_counts_as_audience(aconn):
    at = await _said(aconn, OWNER, GROUP, "shared", "ini hasil kerjaan kita")
    await _said(aconn, "person:fulan", GROUP, "shared")
    await _said(aconn, "person:charlie", "space:tg:charlie", "direct")
    assert await privacy.visible(aconn, context="space:tg:charlie", source=GROUP) is False
    kernel = await _kernel(aconn)
    await kernel._apply(_trigger(at, GROUP), ToolCall("act", {
        "capability": "self.share", "arguments": {"item": "hasil kerjaan kita",
                                                  "to": "person:charlie"}}))
    rows = await (await aconn.execute("SELECT payload FROM events WHERE kind = 'share.granted'")
                  ).fetchall()
    assert rows and rows[0][0]["persons"] == ["person:charlie"]
    assert await privacy.visible(aconn, context="space:tg:charlie", source=GROUP, before=at) is True
    assert await privacy.visible(aconn, context="space:tg:charlie", source=GROUP) is False


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
        "arguments": {"episode": str(fulan), "to": "person:budi"}}))
    await kernel.run_until_idle()
    assert seen == []  # the action hook never sees content the actor was not part of
    kinds = [row[0] for row in await (await aconn.execute(
        "SELECT kind FROM events ORDER BY id")).fetchall()]
    assert "share.granted" not in kinds
    assert "act.result" in kinds
    assert "guard.stopped" in kinds  # the consent rule is a Stop: nothing later sees it

    # Fulan's own chain may share Fulan's line: the hook sees it and consent is recorded.
    await kernel._apply(Event(id=fulan, ts=START, kind="percept", who="person:fulan",
                              place=FULAN_A, origin_actor="person:fulan"),
                        ToolCall("act", {"capability": "self.share", "arguments": {
                            "episode": str(fulan), "to": "person:budi"}}))
    assert seen
    assert await (await aconn.execute(
        "SELECT count(*) FROM events WHERE kind = 'share.granted'")).fetchone() == (1,)


async def test_share_with_a_place_grants_everyone_present(aconn):
    at = await _said(aconn, OWNER, GROUP, "shared", "ini kode kita")
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
    await kernel.run_until_idle()
    row = await (await aconn.execute(
        "SELECT payload FROM events WHERE kind = 'act.result'")).fetchone()
    assert "no way to reach budi yet" in row[0]["reason"]


async def test_a_message_to_an_unknown_place_fails_honestly(aconn):
    kernel = await _kernel(aconn)
    at = await _said(aconn, OWNER, GROUP, "shared")
    await kernel._apply(_trigger(at, GROUP), ToolCall("act", {
        "capability": "whatsapp.say", "arguments": {"text": "halo", "to": "budi_dm"}}))
    await kernel.run_until_idle()
    row = await (await aconn.execute(
        "SELECT payload FROM events WHERE kind = 'act.result'")).fetchone()
    assert "no such place: budi_dm" in row[0]["reason"]
    out = await (await aconn.execute(
        "SELECT payload FROM events WHERE kind = 'act.out'")).fetchone()
    assert out is not None and out[0]["communicative"] is True


async def test_act_offers_the_capabilities_of_every_running_tool(aconn):
    kernel = await _kernel(aconn)
    assert {"whatsapp.say", "telegram.say", "dev.run", "self.share"} <= set(
        kernel.tools.capabilities)


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
        [],
    ])
    kernel = await _kernel(aconn, pfc=pfc)
    await kernel._apply(_trigger(owner_dm, "space:tg:owner_dm"), ToolCall("act", {
        "capability": "whatsapp.say", "arguments": {
            "text": "Fulan bilang: aku di-PHK", "to": "person:budi", "via": "whatsapp"}}))
    await kernel.run_until_idle()

    # 12.16: the message ended the step, so its result does not wake Budi's desk again.
    assert [context.place for context in pfc.contexts] == [BUDI]
    for context in pfc.contexts:
        prompt = "\n".join(message["content"] for message in
                           Reasoning(settings=SETTINGS)
                           .build_messages(context))
        assert "di-PHK" not in prompt
    intent = await (await aconn.execute(
        "SELECT place, payload FROM events WHERE kind = 'act.intent'")).fetchone()
    assert intent[0] == "space:tg:owner_dm"
    assert "di-PHK" in intent[1]["arguments"]["text"]
    spoken = await (await aconn.execute(
        "SELECT place, payload FROM events WHERE kind = 'act.out'")).fetchone()
    assert spoken[0] == BUDI
    assert "di-PHK" not in spoken[1]["arguments"]["text"]


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
    await threads.after_step(kernel, job, result, [(call, {"hold_for": OWNER})])
    assert (await read_state(aconn, f"task:{task_place}"))["waiting_for"] == OWNER

    call = {"name": "act", "arguments": {"capability": "whatsapp.say",
                                         "arguments": {"to": GROUP}}}
    await threads.after_step(kernel, job, result, [(call, {"delivers": True})])
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
    call = {"name": "act", "arguments": {"capability": "whatsapp.say",
                                        "arguments": {"text": "selesai"}}}
    await threads.after_step(kernel, job, result, [(call, {})])
    row = await (await aconn.execute(
        "SELECT payload FROM events WHERE kind = 'task.closed'")).fetchone()
    assert row[0]["reason"] == "max_task_steps"


class _SleepPFC:
    """A sleep-only Reasoning that returns one recorded store_memories answer."""

    model = "sleep-model"
    last_usage: dict = {}

    def __init__(self, arguments: dict) -> None:
        """Keep the answer the fake model would return."""
        self.arguments = arguments

    async def call(self, messages, tools, *, tool_choice="required"):
        """Return the recorded answer for the sleep prompt."""
        return [ToolCall("store_memories", self.arguments)]


async def test_sleep_derives_private_content_from_the_sources(aconn):
    dm = await _said(aconn, "person:fulan", FULAN_A, "direct", "aku di-PHK")
    answer = {"items": [{"text": "Fulan di-PHK", "importance": 1.0}],
              "core_memory": "Fulan sedang di-PHK."}
    done = await memory.sleep(aconn, pfc=_SleepPFC(answer), embedder=FakeEmbedder(),
                              now=START + timedelta(minutes=35), settings=SETTINGS)
    assert done is not None
    row = await (await aconn.execute(
        "SELECT place, private, derived_from FROM memories")).fetchone()
    assert row == (FULAN_A, True, [dm])
    core = await read_state(aconn, "core_memory")
    assert core["derived_from"] == [dm]

    hits = await memory.retrieve(aconn, FakeEmbedder(), "Fulan di-PHK",
                                 now=START + timedelta(minutes=36), k=5, settings=SETTINGS)
    assert [hit.sources for hit in hits] == [(dm,)]
    group = await _said(aconn, "person:charlie", GROUP, "shared", "ada kabar?")
    owner = await _said(aconn, OWNER, OWNER_DM, "direct")
    group_desk = await _desk(aconn, place=GROUP, trigger=group, memories=tuple(hits))
    assert group_desk.item.memories == ()
    assert group_desk.item.core_memory == ""
    owner_desk = await _desk(aconn, place=OWNER_DM, trigger=owner, memories=tuple(hits))
    assert owner_desk.item.memories == tuple(hits)
    assert owner_desk.item.core_memory == "Fulan sedang di-PHK."


async def test_a_model_named_place_never_widens_a_memory(aconn):
    dm = await _said(aconn, "person:fulan", FULAN_A, "direct", "aku di-PHK")
    answer = {"items": [{"text": "Fulan di-PHK", "importance": 1.0,
                         "place": GROUP, "private": False}],
              "core_memory": "Fulan sedang di-PHK."}
    await memory.sleep(aconn, pfc=_SleepPFC(answer), embedder=FakeEmbedder(),
                       now=START + timedelta(minutes=35), settings=SETTINGS)
    row = await (await aconn.execute(
        "SELECT place, private, derived_from FROM memories")).fetchone()
    assert row == (FULAN_A, True, [dm])
    hits = await memory.retrieve(aconn, FakeEmbedder(), "Fulan di-PHK",
                                 now=START + timedelta(minutes=36), k=5, settings=SETTINGS)
    group = await _said(aconn, "person:charlie", GROUP, "shared", "ada kabar?")
    group_desk = await _desk(aconn, place=GROUP, trigger=group, memories=tuple(hits))
    assert group_desk.item.memories == ()
    assert group_desk.item.core_memory == ""


async def test_a_memory_without_a_known_source_is_private_to_the_owner(aconn):
    owner = await _said(aconn, OWNER, OWNER_DM, "direct", "halo")
    group = await _said(aconn, "person:charlie", GROUP, "shared", "halo semua")
    fact = memory.MemoryHit("- tanpa sumber", place=None, sources=())
    assert await privacy.memory_visible(aconn, context=GROUP, sources=()) is False
    assert await privacy.memory_visible(aconn, context=OWNER_DM, sources=()) is True
    group_desk = await _desk(aconn, place=GROUP, trigger=group, memories=(fact,))
    assert group_desk.item.memories == ()
    owner_desk = await _desk(aconn, place=OWNER_DM, trigger=owner, memories=(fact,))
    assert owner_desk.item.memories == (fact,)


async def test_person_cards_show_people_present_and_named_but_never_their_messages(aconn):
    """D55: Charlie present and Budi named; a card says how to reach Budi, not what he said."""
    await _said(aconn, "person:budi", "space:wa:budi", "direct",
                "ini sketsa_logo.png yang baru", body="whatsapp")
    await _said(aconn, "person:charlie", GROUP, "shared", "eh Mona juga ada di world ini")
    asked = await _said(aconn, OWNER, GROUP, "shared",
                        "Tolong kirim hasil kerjaan kita ke Budi")
    await write_state(aconn, "person:budi", {"name": "Budi", "closeness": 0.4}, asked)

    desk = (await _desk(aconn, place=GROUP, trigger=asked,
                        text="Tolong kirim hasil kerjaan kita ke Budi")).item
    cards = "\n".join(desk.people)
    assert "person:charlie" in cards
    assert "Budi (person:budi)" in cards
    assert "whatsapp at space:wa:budi (last " in cards
    assert "closeness 0.40" in cards
    assert "sketsa_logo" not in cards

    prompt = "\n".join(message["content"] for message in
                       Reasoning(settings=SETTINGS)
                       .build_messages(desk))
    assert "<people>" in prompt and "person:budi" in prompt
    assert "sketsa_logo" not in prompt


async def test_scenario_18_step_4_desk_line_is_sent_while_private_dm_in_group_is_refused(aconn):
    """D45 one truth: visible desk content is never refused as a leak; private DMs still are."""
    budi = await _said(aconn, "person:budi", BUDI, "direct",
                       "Ini sketsa logo versi baru: warna biru gelap.", body="whatsapp")
    owner_dm = await _said(aconn, OWNER, OWNER_DM, "direct",
                           "Kode brankas ruang rapat itu 842190, jangan kasih tahu grup.")
    await aconn.execute("INSERT INTO memories (text, derived_from) VALUES (%s, %s)",
                        ("Sketsa logo versi baru: warna biru gelap", [budi]))
    world = await _said(aconn, OWNER, "space:game:world1", "shared", "Budi tadi kirim file apa ya?")
    kernel = await _kernel(aconn)

    desk_quote = "Budi mengirim Sketsa logo versi baru: warna biru gelap"
    await kernel._apply(_trigger(world, "space:game:world1"), ToolCall("act", {
        "capability": "whatsapp.say",
        "arguments": {"text": desk_quote, "to": "space:game:world1"}}))
    out = await (await aconn.execute(
        "SELECT place, payload FROM events WHERE kind = 'act.out' ORDER BY id DESC LIMIT 1")
    ).fetchone()
    assert out is not None and out[0] == "space:game:world1"
    assert desk_quote in out[1]["arguments"]["text"]

    group = await _said(aconn, "person:charlie", GROUP, "shared", "halo semua")
    desk = (await _desk(aconn, place=GROUP, trigger=group)).item
    assert all(line.place != OWNER_DM for line in desk.recent_elsewhere)
    assert "842190" not in _prompt(desk)


async def test_delegated_reply_carries_delegator_audience_for_one_reply_only(aconn):
    """D80: a matched wait-expectation reply gets delegator X; later/Fulan lines do not."""
    await _said(aconn, "person:charlie", GROUP, "shared", "halo grup")
    asked_id = await _said(aconn, OWNER, GROUP, "shared", "tanya Budi cocok atau tidak")
    asked = Event(id=asked_id, ts=START, kind="percept", who=OWNER, place=GROUP,
                  body="telegram", origin_actor=OWNER)
    await _said(aconn, "person:budi", BUDI, "direct", "hai", body="whatsapp")
    kernel = await _kernel(aconn)

    await kernel._emit(asked, "act.out", {
        "capability": "whatsapp.say",
        "arguments": {"text": "Menurutmu cocok atau tidak?"},
        "communicative": True}, place=BUDI)
    exp_created = await expectations.create(aconn, {
        "kind": "wait", "about": "Budi menjawab",
        "cue_event": {"who": "person:budi"}, "expect": {"none": True}},
        now=START, settings=SETTINGS, source=asked)

    budi_reply = await append_event(
        aconn, ts=START, kind="percept", who="person:budi", place=BUDI, body="whatsapp",
        payload={"description": "Cocok kok, cuma warnanya agak terlalu terang.",
                 "place_kind": "direct"})
    await kernel._run_hooks("inbound", budi_reply, {"conn": aconn, "now": START},
                            budi_reply, tool=budi_reply.body)
    assert await privacy.audience(aconn, BUDI, before=budi_reply.id) == frozenset({
        "person:budi", OWNER})
    assert await privacy.visible(aconn, context=GROUP, source=BUDI, before=budi_reply.id) is False

    # Unrelated later line from Budi keeps only Budi's audience.
    await expectations.resolve(aconn, int(exp_created.payload["expectation_id"]),
                               "satisfied", now=START)
    later = await append_event(
        aconn, ts=START, kind="percept", who="person:budi", place=BUDI, body="whatsapp",
        payload={"description": "Aku mau cuti besok, rahasia ya.", "place_kind": "direct"})
    await kernel._run_hooks("inbound", later, {"conn": aconn, "now": START},
                            later, tool=later.body)
    assert await privacy.audience(aconn, BUDI, before=later.id) == frozenset({"person:budi"})
    assert await privacy.visible(aconn, context=GROUP, source=BUDI, before=later.id) is False

    # Owner chooses to share into GROUP; only the matched reply opens, never Fulan's DM.
    reply_here = Event(id=asked_id, ts=START, kind="percept", who=OWNER, place=GROUP,
                       body="telegram", origin_actor=OWNER)
    await kernel._apply(reply_here, ToolCall("act", {
        "capability": "self.share", "arguments": {"item": "jawaban Budi", "to": GROUP}}))
    assert await privacy.visible(aconn, context=GROUP, source=BUDI, before=budi_reply.id) is True
    assert await privacy.visible(aconn, context=GROUP, source=BUDI, before=later.id) is False

    fulan = await _said(aconn, "person:fulan", FULAN_A, "direct", "aku di-PHK")
    await kernel._apply(_trigger(fulan, FULAN_A), ToolCall("act", {
        "capability": "self.share",
        "arguments": {"episode": str(fulan), "to": "person:budi"}}))
    kinds = [r[0] for r in await (await aconn.execute(
        "SELECT kind FROM events WHERE kind = 'guard.stopped'")).fetchall()]
    assert kinds


async def test_delegated_reply_via_reply_to_grants_one_reply_only(aconn):
    """D80: a percept replying (`reply_to`) to an owner-delegated send includes owner once."""
    root_id = await _said(aconn, OWNER, GROUP, "shared", "tanya Budi ya")
    await _said(aconn, "person:budi", BUDI, "direct", "hai", body="whatsapp")
    kernel = await _kernel(aconn)

    root = Event(id=root_id, ts=START, kind="percept", who=OWNER, place=GROUP,
                 body="telegram", origin_actor=OWNER)
    out = await kernel._emit(root, "act.out", {
        "capability": "whatsapp.say", "arguments": {"text": "Jadi nggak?"},
        "communicative": True}, place=BUDI)
    await kernel._emit(out, "act.confirmed", {"capability": "whatsapp.say",
                                              "message_id": "wa-msg-9"}, place=BUDI)

    first = await append_event(
        aconn, ts=START, kind="percept", who="person:budi", place=BUDI, body="whatsapp",
        payload={"description": "Jadi jam lima.", "place_kind": "direct",
                 "reply_to": "wa-msg-9"})
    await kernel._run_hooks("inbound", first, {"conn": aconn, "now": START},
                            first, tool=first.body)
    assert await privacy.audience(aconn, BUDI, before=first.id) == frozenset({"person:budi", OWNER})

    second = await append_event(
        aconn, ts=START, kind="percept", who="person:budi", place=BUDI, body="whatsapp",
        payload={"description": "Oh iya gaji belum turun.", "place_kind": "direct",
                 "reply_to": "wa-msg-9"})
    await kernel._run_hooks("inbound", second, {"conn": aconn, "now": START},
                            second, tool=second.body)
    assert await privacy.audience(aconn, BUDI, before=second.id) == frozenset({"person:budi"})


async def test_delegation_requires_same_place_ignores_unrelated_reply_and_never_doubles(aconn):
    """P.11: D80 delegation applies only once to the one reply to the delegated message."""
    root_id = await _said(aconn, OWNER, GROUP, "shared", "tanya Budi cocok atau tidak")
    await _said(aconn, "person:charlie", GROUP, "shared", "oke")
    await _said(aconn, "person:budi", BUDI, "direct", "hai", body="whatsapp")
    kernel = await _kernel(aconn)

    asked = Event(id=root_id, ts=START, kind="percept", who=OWNER, place=GROUP,
                  body="telegram", origin_actor=OWNER)
    out = await kernel._emit(asked, "act.out", {
        "capability": "whatsapp.say", "arguments": {"text": "Jadi nggak?"},
        "communicative": True}, place=BUDI)
    await kernel._emit(out, "act.confirmed", {"capability": "whatsapp.say",
                                              "message_id": "wa-msg-10"}, place=BUDI)
    await expectations.create(aconn, {
        "kind": "wait", "about": "Budi jawab jadi",
        "cue_event": {"who": "person:budi"}, "expect": {"none": True}},
        now=START, settings=SETTINGS, source=asked)

    # 1) Unrelated message in another place matching cue never gets delegation or burns wait.
    in_group = await append_event(
        aconn, ts=START, kind="percept", who="person:budi", place=GROUP, body="telegram",
        payload={"description": "acara lain jadi", "place_kind": "shared",
                 "place_info": {"members": ["person:budi", "person:charlie", OWNER]}})
    await kernel._run_hooks("inbound", in_group, {"conn": aconn, "now": START},
                            in_group, tool=in_group.body)
    assert await privacy._delegated(aconn, in_group.id) == {"actors": frozenset(), "source_places": ()}

    # 2) Explicit reply_to an unrelated message in BUDI never claims the wait delegation.
    other_reply = await append_event(
        aconn, ts=START, kind="percept", who="person:budi", place=BUDI, body="whatsapp",
        payload={"description": "jadi kemarin itu lucu", "place_kind": "direct",
                 "reply_to": "wa-unrelated"})
    await kernel._run_hooks("inbound", other_reply, {"conn": aconn, "now": START},
                            other_reply, tool=other_reply.body)
    assert await privacy.audience(aconn, BUDI, before=other_reply.id) == frozenset({"person:budi"})

    # 3) Reply #1 answers via reply_to (without keyword): gets delegation ONCE.
    first_reply = await append_event(
        aconn, ts=START, kind="percept", who="person:budi", place=BUDI, body="whatsapp",
        payload={"description": "oke sebentar", "place_kind": "direct",
                 "reply_to": "wa-msg-10"})
    await kernel._run_hooks("inbound", first_reply, {"conn": aconn, "now": START},
                            first_reply, tool=first_reply.body)
    assert await privacy.audience(aconn, BUDI, before=first_reply.id) == frozenset({
        "person:budi", OWNER})

    # 4) Later message matching still-active wait cue must NOT grant a second delegation!
    second_msg = await append_event(
        aconn, ts=START, kind="percept", who="person:budi", place=BUDI, body="whatsapp",
        payload={"description": "jadi rahasia gajiku segini", "place_kind": "direct"})
    await kernel._run_hooks("inbound", second_msg, {"conn": aconn, "now": START},
                            second_msg, tool=second_msg.body)
    assert await privacy.audience(aconn, BUDI, before=second_msg.id) == frozenset({"person:budi"})


async def test_audience_includes_place_info_members_and_mapped_speakers(aconn):
    place = "space:tg:-901"
    ev = await append_event(
        aconn, ts=START, kind="percept", who="telegram:10", place=place, body="telegram",
        payload={"description": "halo", "place_kind": "shared",
                 "place_info": {"members": ["telegram:10", "telegram:20"]}})
    await _link(aconn, "telegram:10", "person:a", int(ev.id))
    await _link(aconn, "telegram:20", "person:b", int(ev.id))
    assert await privacy.audience(aconn, place) == frozenset({"person:a", "person:b"})

    alice_dm = "space:tg:alice"
    await append_event(
        aconn, ts=START, kind="percept", who=OWNER, place=alice_dm, body="telegram",
        payload={"description": "catatan", "place_kind": "direct",
                 "place_info": {"members": [OWNER, "person:alice"]}})
    assert await privacy.audience(aconn, alice_dm) == frozenset({"person:alice"})
    assert await privacy.owner_direct(aconn, alice_dm) is False


async def test_shared_place_with_unknown_or_count_membership_blocks_private_items(aconn):
    group_a, group_b = "space:tg:-902", "space:tg:-903"
    await append_event(
        aconn, ts=START, kind="percept", who=OWNER, place=group_a, body="telegram",
        payload={"description": "hai", "place_kind": "shared", "place_info": {"members": 5}})
    ev_a = await _said(aconn, "person:budi", group_a, "shared", "agenda rapat")
    ev_b = await _said(aconn, "person:budi", group_b, "shared", "rahasia budi")
    await _said(aconn, "person:fulan", FULAN_A, "direct", "rahasia fulan")

    assert privacy.UNKNOWN in await privacy.audience(aconn, group_a)
    assert privacy.UNKNOWN in await privacy.audience(aconn, group_b)
    assert await privacy.visible(aconn, context=group_a, source=FULAN_A) is False
    assert await privacy.visible(aconn, context=group_a, source=group_b) is False
    assert await privacy.visible(aconn, context=group_a, source=group_a, before=ev_a) is True
    assert await privacy.memory_visible(aconn, context=group_a, sources=(ev_b,)) is False
    assert await privacy.memory_visible(aconn, context=group_a, sources=(ev_a,)) is True


async def test_empty_direct_or_owner_only_shared_place_fails_closed(aconn):
    await _said(aconn, "person:fulan", FULAN_A, "direct", "rahasia")
    empty_dm = "space:tg:empty_dm"
    assert await privacy.owner_direct(aconn, empty_dm) is False
    assert await privacy.visible(aconn, context=empty_dm, source=FULAN_A) is False

    owner_shared = "space:tg:-905"
    await append_event(
        aconn, ts=START, kind="percept", who=OWNER, place=owner_shared, body="telegram",
        payload={"description": "uji", "place_kind": "shared",
                 "place_info": {"members": [OWNER]}})
    assert await privacy.owner_direct(aconn, owner_shared) is False
    assert await privacy.audience(aconn, owner_shared) == frozenset({OWNER})
    assert await privacy.visible(aconn, context=owner_shared, source=FULAN_A) is False
    assert await privacy.visible(aconn, context=None, source=FULAN_A) is False
    assert await privacy.visible(aconn, context=FULAN_A, source=None) is False


async def test_labels_before_d85_fall_back_to_current_members_and_newcomer_snapshot_holds(aconn):
    pre_place = "space:tg:-904"
    ev1 = await _said(aconn, "person:alice", pre_place, "shared", "pesan lama")
    await write_state(aconn, f"place:{pre_place}", {"members": ["person:alice", "person:bob"]}, ev1)
    assert await privacy.audience(aconn, pre_place, before=ev1) == frozenset(
        {"person:alice", "person:bob"})

    snap_place = "space:tg:-906"
    ev2 = await append_event(
        aconn, ts=START, kind="percept", who="person:alice", place=snap_place, body="telegram",
        payload={"description": "sebelum charlie", "place_kind": "shared",
                 "place_info": {"members": ["person:alice", "person:bob"]}})
    await append_event(
        aconn, ts=START, kind="percept", who="person:charlie", place=snap_place, body="telegram",
        payload={"description": "charlie masuk", "place_kind": "shared",
                 "place_info": {"members": ["person:alice", "person:bob", "person:charlie"]}})
    assert await privacy.visible(
        aconn, context=snap_place, source=snap_place, before=int(ev2.id)) is False


async def test_verify_context_last_checks_all_eight_sections_and_records_error(aconn):
    from monoagens.middleware import policy

    dm = await _said(aconn, OWNER, OWNER_DM, "direct", "rahasia owner")
    fulan_ev = await _said(aconn, "person:fulan", FULAN_A, "direct", "rahasia fulan")
    budi_ev = await append_event(
        aconn, ts=START, kind="percept", who="person:budi", place=GROUP, body="telegram",
        payload={"description": "halo group, tanya Andi", "place_kind": "shared",
                 "place_info": {"members": ["person:budi"]}})
    await append_event(
        aconn, ts=START, kind="percept", who="person:stranger", place="space:tg:stranger",
        body="telegram", payload={"description": "halo", "place_kind": "direct"})
    await append_event(
        aconn, ts=START, kind="percept", who="person:andi", place="space:tg:andi",
        body="telegram", payload={"description": "rahasia andi", "place_kind": "direct"})
    await write_state(aconn, "core_memory",
                      {"text": "core dari dm", "derived_from": [dm]}, dm)
    priv_exp = await expectations.create(aconn, {
        "kind": "wait", "about": "rahasia exp", "cue_event": {"who": OWNER},
        "expect": {"none": True}}, now=START, settings=SETTINGS,
        source=Event(id=dm, ts=START, kind="percept", who=OWNER, place=OWNER_DM))
    pub_exp = await expectations.create(aconn, {
        "kind": "wait", "about": "grup exp", "cue_event": {"who": "person:budi"},
        "expect": {"none": True}}, now=START, settings=SETTINGS,
        source=Event(id=budi_ev.id, ts=START, kind="percept", who="person:budi", place=GROUP))

    kernel = await _kernel(aconn)
    rule = policy.policy_for("context", "last", kernel)
    dm_event = Event(id=dm, ts=START, kind="percept", who=OWNER, place=OWNER_DM,
                     payload={"description": "rahasia owner"}, description="rahasia owner")
    fulan_event = Event(id=fulan_ev, ts=START, kind="percept", who="person:fulan", place=FULAN_A,
                        payload={"description": "rahasia fulan"}, description="rahasia fulan")
    bad = layout.PFCContext(
        now=START, place=GROUP, reason="test",
        recent=(dm_event,),
        recent_elsewhere=(fulan_event,),
        memories=(memory.MemoryHit("- rahasia mem", place=OWNER_DM, sources=(dm,)),),
        core_memory="core dari dm",
        doing=("goal: rahasia goal", "result: rahasia result"),
        expectations=(f"#{priv_exp.payload['expectation_id']} rahasia exp (no intent stated)",),
        trigger=dm_event,
        people=("- Stranger (person:stranger): telegram at space:tg:stranger; closeness 0.50",),
    )
    res = await rule(bad, {"conn": aconn, "now": START})
    cleaned = res.item
    assert cleaned.recent == ()
    assert cleaned.recent_elsewhere == ()
    assert cleaned.memories == ()
    assert cleaned.core_memory == ""
    assert cleaned.doing == ()
    assert cleaned.expectations == ()
    assert cleaned.trigger is None
    assert cleaned.people == ()

    good = layout.PFCContext(
        now=START, place=GROUP, reason="test",
        recent=(budi_ev,),
        expectations=(f"#{pub_exp.payload['expectation_id']} grup exp (no intent stated)",),
        trigger=budi_ev,
        people=(
            "- person:budi: no direct place known yet; closeness 0.00",
            "- Andi (person:andi): telegram at space:tg:andi; closeness 0.00",
        ),
    )
    res_good = await rule(good, {"conn": aconn, "now": START})
    assert isinstance(res_good, hooks.Continue)
    assert await (await aconn.execute(
        "SELECT count(*) FROM events WHERE kind = 'error'")).fetchone() == (0,)


async def test_filter_context_drops_cross_place_expectations_and_doing_cleanly(aconn):
    dm = await _said(aconn, OWNER, OWNER_DM, "direct", "rahasia dm")
    await append_event(
        aconn, ts=START, kind="percept", who="person:budi", place=GROUP, body="telegram",
        payload={"description": "halo", "place_kind": "shared",
                 "place_info": {"members": ["person:budi"]}})
    asked = await append_event(
        aconn, ts=START, kind="percept", who="person:budi", place=GROUP, body="telegram",
        payload={"description": "gimana?", "place_kind": "shared",
                 "place_info": {"members": ["person:budi"]}})
    global_exp = await expectations.create(aconn, {
        "kind": "wait", "about": "followup rahasia", "cue_event": {"who": OWNER},
        "expect": {"none": True}}, now=START, settings=SETTINGS,
        source=Event(id=dm, ts=START, kind="percept", who=OWNER, place=None))
    exp_id = int(global_exp.payload["expectation_id"])
    await aconn.execute("UPDATE expectations SET place = NULL WHERE id = %s", (exp_id,))

    kernel = await _kernel(aconn)
    raw = await layout.build_context(
        aconn, asked, GROUP, "test", now=START, settings=SETTINGS, memories=(), capabilities=())
    assert any(line.startswith(f"#{exp_id} ") for line in raw.expectations)
    cleaned = await kernel._run_hooks("context", raw, {"conn": aconn, "now": START}, asked)
    assert cleaned is not None
    assert not any(line.startswith(f"#{exp_id} ") for line in cleaned.expectations)
    assert await (await aconn.execute(
        "SELECT count(*) FROM events WHERE kind = 'error'")).fetchone() == (0,)


async def test_text_similarity_leaks_removed_and_safe_phrase_not_refused(aconn):
    assert not hasattr(privacy, "leaks")
    await append_event(
        aconn, ts=START, kind="percept", who="person:fulan", place=FULAN_A, body="whatsapp",
        payload={"description": "aku di-PHK kemarin", "place_kind": "direct", "imported": True})
    group = await append_event(
        aconn, ts=START, kind="percept", who="person:charlie", place=GROUP, body="whatsapp",
        payload={"description": "halo semua", "place_kind": "shared",
                 "place_info": {"members": ["person:charlie"]}})
    kernel = await _kernel(aconn)
    await kernel._apply(group, ToolCall("act", {
        "capability": "whatsapp.say",
        "arguments": {"text": "aku di-PHK kemarin", "to": GROUP}}))
    await kernel.run_until_idle()
    outs = await (await aconn.execute(
        "SELECT payload FROM events WHERE kind = 'act.out' ORDER BY id DESC")).fetchall()
    assert outs and outs[0][0].get("communicative") is True
    assert await privacy.leaving(kernel, {"capability": "web.search", "event": group},
                                 {"query": "aku di-PHK kemarin"}) is None


async def test_speaking_elsewhere_intent_omits_because_goal_and_checks_context_sources(aconn):
    await _said(aconn, "person:budi", BUDI, "direct", "hai", body="whatsapp")
    owner_dm = await _said(aconn, OWNER, OWNER_DM, "direct",
                           "Tolong bilang Budi rapat besok jam 10")
    kernel = await _kernel(aconn)

    # 1. Owner DM without any other person's private items carries drafted words, never `because`.
    await kernel._apply(_trigger(owner_dm, OWNER_DM, text="Tolong bilang Budi rapat besok jam 10"),
                        ToolCall("act", {"capability": "whatsapp.say",
                                         "arguments": {"text": "Rapat besok jam 10 ya",
                                                       "to": "person:budi", "via": "whatsapp"}}))
    await kernel.run_until_idle()
    clean_intent = await (await aconn.execute(
        "SELECT payload FROM events WHERE kind = 'intent' ORDER BY id DESC LIMIT 1")).fetchone()
    assert clean_intent is not None
    desc1 = clean_intent[0]["description"]
    assert "Rapat besok jam 10 ya" in desc1
    assert "because " not in desc1

    # 2. Once Fulan's private chat exists and is in view of Owner DM's wake, words never cross.
    await _said(aconn, "person:fulan", FULAN_A, "direct", "rahasia fulan")
    pfc = _ScriptedPFC([
        [ToolCall("act", {"capability": "whatsapp.say",
                          "arguments": {"text": "Kabar baik Budi",
                                        "to": "person:budi", "via": "whatsapp"}})],
        [],
    ])
    kernel2 = await _kernel(aconn, pfc=pfc)
    trigger2 = await _said(aconn, OWNER, OWNER_DM, "direct", "Tanyakan kabar Budi")
    await kernel2._think(
        Event(id=trigger2, ts=START, kind="percept", who=OWNER, place=OWNER_DM,
              payload={"description": "Tanyakan kabar Budi"}, description="Tanyakan kabar Budi"),
        "addressed", OWNER_DM)
    await kernel2.run_until_idle()
    intents = await (await aconn.execute(
        "SELECT payload FROM events WHERE kind = 'intent' ORDER BY id DESC LIMIT 1")).fetchone()
    assert intents is not None
    desc2 = intents[0]["description"]
    assert "Kabar baik Budi" not in desc2
    assert "its words cannot appear here" in desc2
    assert "because " not in desc2


async def test_doing_hides_goal_when_chain_root_is_not_visible_at_place(aconn):
    await _said(aconn, "person:budi", BUDI, "direct", "hai")
    root = await append_event(
        aconn, ts=START, kind="percept", who=OWNER, place=OWNER_DM, body="telegram",
        payload={"description": "rahasia gaji budi", "place_kind": "direct"})
    intent_ev = await append_event(
        aconn, ts=START, kind="intent", who=None, place=BUDI, caused_by=root.id,
        payload={"description": "tanya budi", "place_kind": "direct"})
    act_out = await append_event(
        aconn, ts=START, kind="act.out", who=None, place=BUDI, caused_by=intent_ev.id,
        payload={"capability": "dev.run", "arguments": {"command": "check"},
                 "communicative": False})
    act_res = await append_event(
        aconn, ts=START, kind="act.result", who=None, place=BUDI, caused_by=act_out.id,
        payload={"capability": "dev.run", "result": {"ok": True}})

    kernel = await _kernel(aconn)
    raw = await layout.build_context(
        aconn, act_res, BUDI, "result", now=START, settings=SETTINGS, memories=(),
        capabilities=())
    assert any("rahasia gaji budi" in line for line in raw.doing)
    cleaned = await kernel._run_hooks("context", raw, {"conn": aconn, "now": START}, act_res)
    assert cleaned is not None
    assert not any(line.startswith("goal:") for line in cleaned.doing)
    assert any(line.startswith("step:") for line in cleaned.doing)
    assert await (await aconn.execute(
        "SELECT count(*) FROM events WHERE kind = 'error'")).fetchone() == (0,)

    # Same-place chain root stays visible in <doing>.
    budi_root = await append_event(
        aconn, ts=START, kind="percept", who="person:budi", place=BUDI, body="whatsapp",
        payload={"description": "tolong cek server", "place_kind": "direct"})
    budi_out = await append_event(
        aconn, ts=START, kind="act.out", who=None, place=BUDI, caused_by=budi_root.id,
        payload={"capability": "dev.run", "arguments": {"command": "ping"},
                 "communicative": False})
    budi_res = await append_event(
        aconn, ts=START, kind="act.result", who=None, place=BUDI, caused_by=budi_out.id,
        payload={"capability": "dev.run", "result": {"ok": True}})
    raw_ok = await layout.build_context(
        aconn, budi_res, BUDI, "result", now=START, settings=SETTINGS, memories=(),
        capabilities=())
    cleaned_ok = await kernel._run_hooks(
        "context", raw_ok, {"conn": aconn, "now": START}, budi_res)
    assert cleaned_ok is not None
    assert any(line == "goal: tolong cek server" for line in cleaned_ok.doing)


async def test_dm_line_matching_group_promise_never_wakes_group(aconn):
    await _said(aconn, "person:fulan", FULAN_A, "direct", "hai")
    budi_ev = await append_event(
        aconn, ts=START, kind="percept", who="person:budi", place=GROUP, body="telegram",
        payload={"description": "halo grup", "place_kind": "shared",
                 "place_info": {"members": ["person:budi"]}})
    await expectations.create(aconn, {
        "kind": "promise", "about": "kabari kalau server down",
        "cue_event": {"kind": "percept"},
        "expect": {"text": "server down"},
        "expires_at": (START + timedelta(hours=1)).isoformat(),
    }, now=START, settings=SETTINGS,
        source=Event(id=budi_ev.id, ts=START, kind="percept", who=OWNER, place=GROUP))

    pfc = _ScriptedPFC([[], []])
    kernel = await _kernel(aconn, pfc=pfc, embedder=FakeEmbedder())
    dm_ev = Event(id=None, ts=START, kind="percept", who="person:fulan", place=FULAN_A,
                  body="whatsapp",
                  payload={"description": "server down rahasia fulan", "place_kind": "direct"}, description="server down rahasia fulan")
    kernel.enqueue(dm_ev)
    await kernel.run_until_idle()

    assert all(ctx.place != GROUP for ctx in pfc.contexts)
    for ctx in pfc.contexts:
        if ctx.place == GROUP:
            assert "rahasia fulan" not in _prompt(ctx)

    # Same-place line matching the promise still wakes GROUP as a reminder.
    group_ev = Event(id=None, ts=START, kind="percept", who="person:budi", place=GROUP,
                     body="telegram",
                     payload={"description": "server down di grup", "place_kind": "shared",
                              "place_info": {"members": ["person:budi"]}}, description="server down di grup")
    kernel.enqueue(group_ev)
    await kernel.run_until_idle()
    assert any(ctx.place == GROUP for ctx in pfc.contexts)


async def test_p15_privacy_lives_only_in_last_context_slot_and_stops_hidden_reminder(aconn):
    from monoagens.middleware import policy

    fulan_ev = await _said(aconn, "person:fulan", FULAN_A, "direct", "rahasia fulan")
    budi_ev = await append_event(
        aconn, ts=START, kind="percept", who="person:budi", place=GROUP, body="telegram",
        payload={"description": "halo grup", "place_kind": "shared",
                 "place_info": {"members": ["person:budi"]}})
    await expectations.create(aconn, {
        "kind": "promise", "about": "kabari kalau server down",
        "cue_event": {"kind": "percept"},
        "expect": {"text": "server down"},
        "expires_at": (START + timedelta(hours=1)).isoformat(),
    }, now=START, settings=SETTINGS,
        source=Event(id=budi_ev.id, ts=START, kind="percept", who=OWNER, place=GROUP))

    pfc = _ScriptedPFC([[]])
    kernel = await _kernel(aconn, pfc=pfc, embedder=FakeEmbedder())
    assert getattr(kernel, "visible", None) is None

    # First context slot no longer runs privacy filtering; last slot does.
    fulan_obj = Event(id=fulan_ev, ts=START, kind="percept", who="person:fulan", place=FULAN_A,
                      payload={"description": "rahasia fulan"}, description="rahasia fulan")
    raw = layout.PFCContext(now=START, place=GROUP, reason="test",
                            recent_elsewhere=(fulan_obj,), trigger=budi_ev)
    first_res = await policy.policy_for("context", "first", kernel)(raw, {"conn": aconn, "now": START})
    assert isinstance(first_res, hooks.Continue) or first_res.item.recent_elsewhere == (fulan_obj,)

    dm_ev = Event(id=None, ts=START, kind="percept", who="person:fulan", place=FULAN_A,
                  body="whatsapp",
                  payload={"description": "server down rahasia fulan", "place_kind": "direct"}, description="server down rahasia fulan")
    kernel.enqueue(dm_ev)
    await kernel.run_until_idle()
    assert all(ctx.place != GROUP for ctx in pfc.contexts)
    stopped = await (await aconn.execute(
        "SELECT payload FROM events WHERE kind = 'guard.stopped' AND place = %s",
        (GROUP,))).fetchall()
    assert stopped and stopped[0][0].get("hook") == "context"


async def test_p13_share_requires_item_or_episode_and_never_grants_whole_place(aconn):
    secret_before = await _said(aconn, OWNER, OWNER_DM, "direct", "rahasia pin 1111")
    meeting = await _said(aconn, OWNER, OWNER_DM, "direct", "rapat jam sembilan")
    secret_after = await _said(aconn, OWNER, OWNER_DM, "direct", "rahasia pin 2222")
    await append_event(
        aconn, ts=START, kind="percept", who="person:budi", place=GROUP, body="telegram",
        payload={"description": "kapan rapat?", "place_kind": "shared",
                 "place_info": {"members": ["person:budi"]}})
    kernel = await _kernel(aconn)

    # 1. Calling self.share without item or episode is refused with a question.
    await kernel._apply(_trigger(meeting, OWNER_DM), ToolCall("act", {
        "capability": "self.share", "arguments": {"to": GROUP}}))
    await kernel.run_until_idle()
    res = await (await aconn.execute(
        "SELECT payload FROM events WHERE kind = 'act.result' ORDER BY id DESC LIMIT 1")).fetchone()
    assert res is not None and "?" in str(res[0].get("reason") or "")
    assert await (await aconn.execute(
        "SELECT count(*) FROM events WHERE kind = 'share.granted'")).fetchone() == (0,)

    # 2. Sharing one item covers only its event, never earlier/later events or the whole place.
    await kernel._apply(_trigger(meeting, OWNER_DM), ToolCall("act", {
        "capability": "self.share",
        "arguments": {"item": "rapat jam sembilan", "to": GROUP}}))
    assert await privacy.visible(aconn, context=GROUP, source=OWNER_DM, before=meeting) is True
    assert await privacy.visible(aconn, context=GROUP, source=OWNER_DM, before=secret_before) is False
    assert await privacy.visible(aconn, context=GROUP, source=OWNER_DM, before=secret_after) is False
    assert await privacy.visible(aconn, context=GROUP, source=OWNER_DM) is False


async def test_outbound_cross_place_act_out_does_not_add_stranger_to_dm_audience(aconn):
    """Only `percept` events (who spoke there) count toward `audience`, never cross-place `act.out`."""
    await _said(aconn, OWNER, OWNER_DM, "direct", "kabari aku kalau server down")
    await append_event(
        aconn, ts=START, kind="percept", who="person:charlie", place=GROUP, body="telegram",
        payload={"description": "server down", "place_kind": "shared",
                 "place_info": {"members": ["person:charlie"]}})
    await append_event(
        aconn, ts=START, kind="act.out", who="person:charlie", place=OWNER_DM, body="telegram",
        payload={"capability": "telegram.say", "arguments": {"text": "server down di grup"},
                 "communicative": True})
    assert await privacy.audience(aconn, OWNER_DM) == frozenset({OWNER})
    assert await privacy.owner_direct(aconn, OWNER_DM) is True







