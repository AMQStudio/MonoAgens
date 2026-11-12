"""Tests for Part 3: privacy follows people, sharing, reaching a person, threads."""

from __future__ import annotations

from dataclasses import replace
from types import SimpleNamespace
from datetime import datetime, timedelta, timezone
from typing import Any

from monoagens.core import hooks, memory, threads
from monoagens.core.context import layout
from monoagens.middleware import privacy
from monoagens.infra.clock import FakeClock
from monoagens.infra.embedding import FakeEmbedder
from monoagens.core.kernel import Kernel
from monoagens.core.reasoning import Reasoning, ToolCall
from monoagens.infra.store import append_event, read_state, write_state
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
        say = {"name": "say", "description": "Speak here.", "schema": {
                   "type": "object", "properties": {"text": {"type": "string"}},
                   "required": ["text"]},
               "communicative": True, "long_running": False, "identity_changing": False}
        run = {"name": "run", "description": "Run code.", "schema": {
                   "type": "object", "properties": {"command": {"type": "string"}},
                   "required": ["command"]},
               "communicative": False, "long_running": True, "identity_changing": False}
        return (say, run) if self.long_running else (say,)

    async def act(self, action: Action) -> dict[str, Any]:
        """Record the action; a long action reports its completion."""
        self.actions.append(action)
        return {"ok": True} if action.kind == "run" else {}


async def _kernel(conn, *, settings_value=None, pfc=None, embedder=None) -> Kernel:
    kernel = Kernel(conn, clock=FakeClock(START), settings=settings_value or SETTINGS,
                    pfc=pfc, embedder=embedder)
    kernel.register_adapter(Body("whatsapp"))
    kernel.register_adapter(Body("telegram"))
    kernel.register_adapter(Body("dev", long_running=True))
    return kernel


def _trigger(event_id: int, place: str, who: str = OWNER,
             text: str = "kerjakan") -> Event:
    return Event(id=event_id, ts=START, kind="percept", who=who, place=place,
                 payload={"description": text}, origin_actor=who)


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
    """Lay out one desk and pass it through the core's context hook."""
    context = await layout.build_context(
        conn, _trigger(trigger, place, text=text), place, "test", now=START,
        settings=SETTINGS, memories=memories, capabilities=())
    return await hooks.audience_policy()(context, {"conn": conn, "now": START})


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
    assert await privacy.visible(aconn, context=GROUP, source=OWNER_DM) is True
    shared_desk = await _desk(aconn, place=GROUP, trigger=asked, memories=(fact,))
    assert shared_desk.item.memories == (fact,)


async def test_context_hook_keeps_private_lines_and_memories_inside_the_audience(aconn):
    trigger_id = await _said(aconn, "person:fulan", FULAN_A, "direct", "aku di-PHK")
    await _said(aconn, "person:charlie", GROUP, "shared")
    context = await layout.build_context(
        aconn, _trigger(trigger_id, GROUP), GROUP, "test", now=START, settings=SETTINGS,
        memories=(), capabilities=())
    filtered = await hooks.audience_policy()(context, {"conn": aconn, "now": START})
    assert all(line.place != FULAN_A for line in filtered.item.recent_elsewhere)

    owner_dm = await _said(aconn, OWNER, "space:tg:owner", "direct")
    context = await layout.build_context(
        aconn, _trigger(owner_dm, "space:tg:owner"), "space:tg:owner", "test", now=START,
        settings=SETTINGS, memories=(), capabilities=())
    filtered = await hooks.audience_policy()(context, {"conn": aconn, "now": START})
    assert any(line.place == FULAN_A for line in filtered.item.recent_elsewhere)


async def test_a_newcomer_never_receives_a_line_said_before_they_arrived(aconn):
    said = await _said(aconn, OWNER, GROUP, "shared", "Budi mengirim sketsa_logo.png")
    asked = await _said(aconn, OWNER, GROUP, "shared", "Budi kirim file apa ya?")
    assert await privacy.visible(aconn, context=GROUP, source=GROUP, before=said) is True
    context = await layout.build_context(
        aconn, _trigger(asked, GROUP), GROUP, "test", now=START, settings=SETTINGS,
        memories=(), capabilities=())
    desk = await hooks.audience_policy()(context, {"conn": aconn, "now": START})
    assert [line.id for line in desk.item.recent] == [said]

    joined = await _said(aconn, "person:charlie", GROUP, "shared", "eh aku juga di sini")
    later = await _said(aconn, OWNER, GROUP, "shared", "Budi kirim file apa tadi?")
    assert await privacy.visible(aconn, context=GROUP, source=GROUP, before=said) is False
    assert await privacy.visible(aconn, context=GROUP, source=GROUP, before=joined) is True
    assert await privacy.visible(aconn, context=GROUP, source=GROUP, before=later) is True
    context = await layout.build_context(
        aconn, _trigger(later, GROUP), GROUP, "test", now=START, settings=SETTINGS,
        memories=(), capabilities=())
    desk = await hooks.audience_policy()(context, {"conn": aconn, "now": START})
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
    desk = (await hooks.audience_policy()(context, {"conn": aconn, "now": START})).item

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
                     Reasoning("http://unused", "", "fake", settings=SETTINGS).build_messages(desk))


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
    assert await privacy.visible(aconn, context="space:tg:charlie", source=GROUP) is True


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


async def test_a_message_to_an_unknown_place_fails_honestly(aconn):
    kernel = await _kernel(aconn)
    at = await _said(aconn, OWNER, GROUP, "shared")
    await kernel._apply(_trigger(at, GROUP), ToolCall("act", {
        "capability": "whatsapp.say", "arguments": {"text": "halo", "to": "budi_dm"}}))
    row = await (await aconn.execute(
        "SELECT payload FROM events WHERE kind = 'act.result'")).fetchone()
    assert "no such place: budi_dm" in row[0]["result"]["error"]
    assert await (await aconn.execute(
        "SELECT count(*) FROM events WHERE kind = 'act.out'")).fetchone() == (0,)


async def test_act_offers_the_capabilities_of_every_running_organ(aconn):
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
                           Reasoning("http://unused", "", "fake", settings=SETTINGS)
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
    assert await privacy.leaks(aconn, BUDI, "Fulan bilang: aku di-PHK") is True


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
    assert "whatsapp at space:wa:budi" in cards
    assert "closeness 0.40" in cards
    assert "sketsa_logo" not in cards

    prompt = "\n".join(message["content"] for message in
                       Reasoning("http://unused", "", "fake", settings=SETTINGS)
                       .build_messages(desk))
    assert "<people>" in prompt and "person:budi" in prompt
    assert "sketsa_logo" not in prompt
