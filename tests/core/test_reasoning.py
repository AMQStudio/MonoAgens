"""Tests for Step 4: the Reasoning, its layered prompt, tools, and failure handling."""

from __future__ import annotations

import asyncio
import json
from datetime import datetime, timedelta, timezone

import httpx
import pytest

from monoagens.core import expectations
from monoagens.infra.clock import FakeClock
from monoagens.core.gate import Situation, appraise
from monoagens.core.identity import SelfTool
from monoagens.core.kernel import Kernel
from monoagens.core.context import layout
from monoagens.core.context.layout import PFCContext
from monoagens.core.reasoning import TOOLS, TOOL_NAMES, Reasoning, ToolCall, turn_check
from monoagens.infra.store import (active_expectation_lines, append_event, read_state,
                                  write_state)
from monoagens.adapters.telegram import CAPABILITIES as TELEGRAM_CAPABILITIES
from monoagens.types import Action, Event
from monoagens.core.context.working import doing_lines
from monoagens.boot import build
from monoagens.middleware import privacy
from tests.support import settings

SETTINGS = settings()
PLACE = "space:tg:1"
START = datetime(2026, 11, 10, 8, 0, tzinfo=timezone.utc)  # 15:00 in Jakarta


class FakeLLM:
    """Scripted OpenAI-compatible endpoint; records every request it receives."""

    def __init__(
        self,
        responses: list[list[tuple[str, dict]]] | None = None,
        *,
        status: int = 200,
    ) -> None:
        self.responses = responses or [[]]
        self.status = status
        self.requests: list[dict] = []
        self.headers: list[httpx.Headers] = []

    def handler(self, request: httpx.Request) -> httpx.Response:
        self.requests.append(json.loads(request.content))
        self.headers.append(request.headers)
        if self.status >= 400:
            return httpx.Response(self.status, json={"error": {"message": "boom"}})
        calls = self.responses[min(len(self.requests) - 1, len(self.responses) - 1)]
        tool_calls = [
            {
                "id": f"call_{i}",
                "type": "function",
                "function": {"name": name, "arguments": json.dumps(arguments)},
            }
            for i, (name, arguments) in enumerate(calls)
        ]
        return httpx.Response(
            200, json={"choices": [{"message": {"role": "assistant", "tool_calls": tool_calls}}]}
        )

    @property
    def last_messages(self) -> list[dict]:
        return self.requests[-1]["messages"]


def _pfc(fake: FakeLLM) -> Reasoning:
    return Reasoning(
        "https://llm.test/v1",
        "test-key",
        "fake-model",
        settings=SETTINGS,
        transport=httpx.MockTransport(fake.handler),
    )


class RecordingBody:
    """A body that records actions and snapshots the Log when each action arrives."""

    name = "telegram"

    def __init__(self, conn=None) -> None:
        self.conn = conn
        self.actions: list[Action] = []
        self.log_at_act: list[list[str]] = []

    def capabilities(self):
        return TELEGRAM_CAPABILITIES

    async def act(self, action: Action) -> None:
        if self.conn is not None:
            cur = await self.conn.execute(
                "SELECT kind FROM events WHERE kind NOT IN ('job.started', 'job.result') "
                "ORDER BY id")
            self.log_at_act.append([row[0] for row in await cur.fetchall()])
        self.actions.append(action)


class ResultBody(RecordingBody):
    """A body that answers every action with a result."""

    async def act(self, action: Action):
        await super().act(action)
        return {"message_id": 7}


def _msg(text: str, *, message_id: int = 1, addressed: bool = True, who: str = "person:owner") -> Event:
    return Event(
        id=None,
        ts=START,
        kind="percept",
        who=who,
        place=PLACE,
        body="telegram",
        payload={
            "description": text,
            "data": {"message_id": message_id},
            "addressed": addressed,
            "place_kind": "direct",
        },
    )


async def _feed(kernel: Kernel, event: Event) -> Event:
    """Put one event and let the single consumer process it to completion."""
    put_task = asyncio.create_task(kernel.put(event))
    await asyncio.sleep(0)
    stored = await kernel.step()
    await kernel.run_until_idle()
    await asyncio.wait_for(put_task, 1.0)
    return stored


async def _kinds(conn) -> list[str]:
    cur = await conn.execute("SELECT kind FROM events WHERE kind NOT IN "
                             "('job.started', 'job.result') ORDER BY id")
    return [row[0] for row in await cur.fetchall()]


async def test_layered_prompt_structure(aconn):
    """Stable identity, then core_memory, then tagged volatile sections in order."""
    fake = FakeLLM()
    kernel = build(aconn, clock=FakeClock(START), pfc=_pfc(fake), settings=SETTINGS)
    body = RecordingBody()
    kernel.register_adapter(body)

    seed = await append_event(aconn, ts=START, kind="decision.note", place=PLACE)
    assert seed.id is not None
    await write_state(aconn, "core_memory", {"text": "The owner likes coffee.",
                                            "derived_from": [seed.id]}, seed.id)
    await append_event(aconn, ts=START, kind="percept", who="person:owner", place=PLACE,
                       body="telegram", payload={"description": "halo"})
    await append_event(aconn, ts=START, kind="act.out", who="person:owner", place=PLACE,
                       body="telegram",
                       payload={"capability": "telegram.say", "arguments": {"text": "hai"}})

    await _feed(kernel, _msg("lagi?"))

    request = fake.requests[0]
    assert request["model"] == "fake-model"
    assert request["tool_choice"] == "required"
    assert {tool["function"]["name"] for tool in request["tools"]} == {
        "think", "act", "expect", "resolve", "revise"}
    act_tool = next(tool for tool in request["tools"]
                    if tool["function"]["name"] == "act")
    capability = act_tool["function"]["parameters"]["properties"]["capability"]
    assert capability["enum"] == ["registry.describe", "self.recall", "self.set_identity",
                                 "self.share", "telegram.react", "telegram.say"]
    description = act_tool["function"]["description"]
    assert ("telegram.say(text, attachments, to? (a speaker or a place), via? (a tool)):"
            in description)
    assert "telegram.react(emoji, message_id):" in description
    expect_tool = next(tool for tool in request["tools"]
                       if tool["function"]["name"] == "expect")
    kind = expect_tool["function"]["parameters"]["properties"]["kind"]
    assert kind["enum"] == ["promise", "wait", "predict", "guard"]
    assert fake.headers[0]["authorization"] == "Bearer test-key"

    messages = fake.last_messages
    assert [m["role"] for m in messages] == ["system", "system", "user"]

    stable = messages[0]["content"]
    assert stable.startswith("You are Mona.")
    assert "Calm and honest." in stable
    assert "Never lie." in stable
    assert "Language: id." in stable
    assert "You act only through the tools. Staying silent is a valid choice." in stable

    assert messages[1]["content"] == "<core_memory>The owner likes coffee.</core_memory>"

    volatile = messages[2]["content"]
    tags = ["<now>", "<place>", "<why_awake>", "<expectations>", "<memories>", "<recent>", "<event>"]
    positions = [volatile.index(tag) for tag in tags]
    assert positions == sorted(positions)
    assert "Asia/Jakarta" in volatile
    assert f"<place>{PLACE} (direct)</place>" in volatile
    assert "<recent>\n" in volatile
    assert "15:00 person:owner: halo" in volatile
    assert "15:00 Mona: hai" in volatile
    assert "<event>person:owner: lagi?</event>" in volatile


async def test_recent_shows_only_conversational_events(aconn):
    """<recent> carries percepts, act.out, and fired expectations, never bookkeeping."""
    fake = FakeLLM()
    kernel = build(aconn, clock=FakeClock(START), pfc=_pfc(fake), settings=SETTINGS)

    await append_event(aconn, ts=START, kind="percept", who="person:owner", place=PLACE,
                       body="telegram", payload={"description": "halo pagi"})
    await append_event(aconn, ts=START, kind="decision.silent", place=PLACE,
                       payload={"reason": "internal-silence"})
    await append_event(aconn, ts=START, kind="error", place=PLACE,
                       payload={"error": "internal-error"})
    await append_event(aconn, ts=START, kind="sleep.done", place=PLACE,
                       payload={"text": "internal-sleep"})
    await append_event(aconn, ts=START, kind="expectation.created", place=PLACE,
                       payload={"about": "internal-expectation"})
    await append_event(aconn, ts=START, kind="expectation.fired", who="person:owner",
                       place=PLACE, body="telegram", payload={"about": "minum air"})
    await append_event(aconn, ts=START, kind="act.out", who="person:owner", place=PLACE,
                       body="telegram",
                       payload={"capability": "telegram.say", "arguments": {"text": "pagi juga"}})

    await _feed(kernel, _msg("masih di sana?"))

    volatile = fake.last_messages[2]["content"]
    recent = volatile.split("<recent>", 1)[1].split("</recent>", 1)[0]
    assert "15:00 person:owner: halo pagi" in recent
    assert "15:00 Mona: pagi juga" in recent
    assert "15:00 expectation: minum air" in recent
    for internal in ("internal-silence", "internal-error", "internal-sleep",
                     "internal-expectation"):
        assert internal not in recent
    assert "<event>person:owner: masih di sana?</event>" in volatile


async def test_say_is_recorded_before_the_body_acts(aconn):
    fake = FakeLLM([[("act", {"capability": "telegram.say", "arguments": {"text": "halo juga"}})]])
    kernel = build(aconn, clock=FakeClock(START), pfc=_pfc(fake), settings=SETTINGS)
    body = RecordingBody(aconn)
    kernel.register_adapter(body)

    trigger = await _feed(kernel, _msg("halo"))

    assert await _kinds(aconn) == ["percept", "model.call", "act.out", "act.confirmed"]
    cur = await aconn.execute("SELECT who, place, body, payload, caused_by FROM events WHERE kind='act.out'")
    who, place, body_name, payload, caused_by = await cur.fetchone()
    assert (who, place, body_name) == ("person:owner", PLACE, "telegram")
    assert payload == {"capability": "telegram.say", "arguments": {"text": "halo juga"},
                       "communicative": True}
    assert caused_by == trigger.id

    # act.out is already in the Log when act() is called.
    assert body.log_at_act == [["percept", "model.call", "act.out"]]
    assert body.actions == [
        Action(kind="say", place=PLACE, payload={"text": "halo juga"}, adapter="telegram")
    ]
    last_spoke = await read_state(aconn, f"last_spoke:{PLACE}")
    assert datetime.fromisoformat(last_spoke["ts"]) == START


async def test_react_is_recorded_before_the_body_acts(aconn):
    fake = FakeLLM([[("act", {"capability": "telegram.react", "arguments": {"emoji": "👍"}})]])
    kernel = build(aconn, clock=FakeClock(START), pfc=_pfc(fake), settings=SETTINGS)
    body = RecordingBody(aconn)
    kernel.register_adapter(body)

    await _feed(kernel, _msg("mantap", message_id=42))

    assert await _kinds(aconn) == ["percept", "model.call", "act.out"]
    cur = await aconn.execute("SELECT payload FROM events WHERE kind='act.out'")
    assert (await cur.fetchone())[0] == {
        "capability": "telegram.react", "arguments": {"emoji": "👍"},
        "communicative": False}
    assert body.log_at_act == [["percept", "model.call", "act.out"]]
    assert body.actions == [
        Action(kind="react", place=PLACE, payload={"emoji": "👍"}, adapter="telegram")
    ]


async def test_a_tool_result_is_recorded_as_act_result(aconn):
    fake = FakeLLM([[("act", {"capability": "telegram.say",
                              "arguments": {"text": "done"}})], []])
    kernel = build(aconn, clock=FakeClock(START), pfc=_pfc(fake), settings=SETTINGS)
    kernel.register_adapter(ResultBody())

    await _feed(kernel, _msg("halo"))

    # 12.16: the message ended the step, so its result is recorded but wakes nothing.
    assert await _kinds(aconn) == ["percept", "model.call", "act.out", "act.confirmed",
                                   "act.result", "decision.silent"]
    cur = await aconn.execute("SELECT payload FROM events WHERE kind='act.result'")
    assert (await cur.fetchone())[0] == {"capability": "telegram.say",
                                         "result": {"message_id": 7}}


async def test_an_act_result_wakes_the_pfc_as_a_continued_goal(aconn):
    fake = FakeLLM([[("act", {"capability": "self.recall",
                              "arguments": {"query": "catatan"}})],
                    [("think", {"thought": "the result arrived", "motivation": 2,
                               "confidence": 0.5})], []])
    kernel = build(aconn, clock=FakeClock(START), pfc=_pfc(fake), settings=SETTINGS)
    kernel.register_adapter(ResultBody())

    await _feed(kernel, _msg("halo"))

    assert len(fake.requests) == 3  # the act, the thought, then the turn ends
    volatile = fake.requests[1]["messages"][-1]["content"]
    assert "<why_awake>goal continued</why_awake>" in volatile
    assert "<event>[act.result]</event>" in volatile


async def test_an_act_result_is_routed_to_its_task_thread(aconn):
    fake = FakeLLM([[("act", {"capability": "self.recall",
                              "arguments": {"query": "catatan"}})],
                    [("think", {"thought": "step done", "motivation": 1,
                               "confidence": 0.5})], []])
    kernel = build(aconn, clock=FakeClock(START), pfc=_pfc(fake), settings=SETTINGS)
    kernel.register_adapter(ResultBody())
    seed = await append_event(aconn, ts=START, kind="task.opened",
                              place="space:task:5", payload={"parent": PLACE})
    await write_state(aconn, "task:space:task:5", {
        "place": "space:task:5", "parent": PLACE, "audience": [], "status": "open",
        "steps": 1, "waiting_for": None, "opened_event": seed.id, "goal": "finish",
        "since": seed.id}, seed.id)

    await _feed(kernel, _msg("halo"))

    assert "<place>space:task:5 (" in fake.requests[1]["messages"][-1]["content"]


async def test_a_wake_without_act_is_recorded_as_silent(aconn):
    fake = FakeLLM([[]])
    kernel = build(aconn, clock=FakeClock(START), pfc=_pfc(fake), settings=SETTINGS)
    body = RecordingBody()
    kernel.register_adapter(body)

    await _feed(kernel, _msg("hm"))

    assert await _kinds(aconn) == ["percept", "model.call", "decision.silent"]
    cur = await aconn.execute("SELECT payload FROM events WHERE kind='decision.silent'")
    assert (await cur.fetchone())[0] == {"reason": "no act"}
    assert body.actions == []


async def test_reasoning_effort_is_sent_only_when_it_is_set(aconn):
    asked = FakeLLM()
    kernel = build(aconn, clock=FakeClock(START), settings=SETTINGS, pfc=Reasoning(
        "https://llm.test/v1", "test-key", "fake-model", effort="high", settings=SETTINGS,
        transport=httpx.MockTransport(asked.handler)))
    await _feed(kernel, _msg("halo"))
    assert asked.requests[0]["reasoning_effort"] == "high"

    plain = FakeLLM()
    kernel = build(aconn, clock=FakeClock(START), pfc=_pfc(plain), settings=SETTINGS)
    await _feed(kernel, _msg("halo lagi", message_id=2))
    assert "reasoning_effort" not in plain.requests[0]


async def test_a_model_that_only_thinks_stops_after_max_thought_rounds(aconn):
    """D51: every round is recorded, and a model that never acts ends at the prior's limit."""
    fake = FakeLLM([[("think", {"thought": "nothing to add", "motivation": 2,
                               "confidence": 0.4})]])
    kernel = build(aconn, clock=FakeClock(START), pfc=_pfc(fake),
                    settings=settings(constants={"max_thought_rounds": 2}))
    kernel.register_adapter(RecordingBody())

    await _feed(kernel, _msg("hm"))

    assert await _kinds(aconn) == ["percept", "model.call", "model.call",
                                   "decision.note", "decision.note", "decision.silent"]
    assert len(fake.requests) == 2
    assert [request["tool_choice"] for request in fake.requests] == ["required", "auto"]
    last = fake.requests[1]["messages"][-1]["content"]
    assert "<results>\nthink: " in last and "nothing to add" in last
    cur = await aconn.execute("SELECT payload FROM events WHERE kind='decision.note'")
    assert (await cur.fetchone())[0] == {
        "act": "think", "thought": "nothing to add", "motivation": 2, "confidence": 0.4}
    cur = await aconn.execute("SELECT payload FROM events WHERE kind='decision.silent'")
    assert (await cur.fetchone())[0] == {"reason": "nothing to add"}


async def test_a_thought_then_an_act_is_one_wake(aconn):
    """D51: the thought's result goes back to the model, which acts in the same wake."""
    fake = FakeLLM([[("think", {"thought": "the owner needs the file", "motivation": 4,
                               "confidence": 0.8})],
                    [("act", {"capability": "telegram.say",
                              "arguments": {"text": "Budi mengirim sketsa_logo.png"}})]])
    kernel = build(aconn, clock=FakeClock(START), pfc=_pfc(fake), settings=SETTINGS)
    body = RecordingBody()
    kernel.register_adapter(body)

    await _feed(kernel, _msg("Budi kirim file apa?"))

    assert [action.payload for action in body.actions] == [
        {"text": "Budi mengirim sketsa_logo.png"}]
    assert await _kinds(aconn) == ["percept", "model.call", "model.call",
                                   "decision.note", "act.out", "act.confirmed"]
    assert [request["tool_choice"] for request in fake.requests] == ["required", "auto"]
    assert "<results>\nthink: " in fake.requests[1]["messages"][-1]["content"]


async def test_an_expect_without_a_cue_is_corrected_in_the_same_wake(aconn):
    """D51: a refused thought act's reason is a result line; the next round fixes it."""
    fake = FakeLLM([[("expect", {"about": "ask again later", "expect": {"none": True}})],
                    [("expect", {"about": "ask again later", "kind": "promise",
                                 "cue_time": {"at": "2026-11-10T16:00:00+00:00"},
                                 "expect": {"none": True}, "policy": {"once": True}})],
                    []])
    kernel = build(aconn, clock=FakeClock(START), pfc=_pfc(fake), settings=SETTINGS)
    kernel.register_adapter(RecordingBody())

    await _feed(kernel, _msg("tanya lagi kalau aku belum jawab dalam 1 jam"))

    assert "expect refused: expect needs cue_time or cue_event" in \
        fake.requests[1]["messages"][-1]["content"]
    assert [request["tool_choice"] for request in fake.requests] == \
        ["required", "auto", "auto"]
    assert await _kinds(aconn) == ["percept", "model.call", "model.call", "model.call",
                                   "expectation.created", "decision.silent"]
    cur = await aconn.execute("SELECT about, cue_event FROM expectations")
    assert await cur.fetchall() == [("ask again later", None)]


async def test_a_second_message_to_the_same_place_in_one_turn_is_refused(aconn):
    """D51: a turn speaks once per place; the repeat is an honest result, not a message."""
    fake = FakeLLM([
        [("act", {"capability": "telegram.say", "arguments": {"text": "halo"}}),
         ("expect", {"about": "ask again later", "expect": {"none": True}})],
        [("act", {"capability": "telegram.say", "arguments": {"text": "halo lagi"}})],
        [],
    ])
    kernel = build(aconn, clock=FakeClock(START), pfc=_pfc(fake), settings=SETTINGS)
    body = RecordingBody()
    kernel.register_adapter(body)

    await _feed(kernel, _msg("jawab dong"))

    assert "expect refused: expect needs cue_time or cue_event" in \
        fake.requests[1]["messages"][-1]["content"]
    assert [action.payload for action in body.actions] == [{"text": "halo"}]
    assert await (await aconn.execute(
        "SELECT count(*) FROM events WHERE kind = 'act.out'")).fetchone() == (1,)
    cur = await aconn.execute("SELECT payload FROM events WHERE kind = 'act.result'")
    assert [row[0]["result"]["error"] for row in await cur.fetchall()] == [
        f"already said something to {PLACE} this turn"]


async def test_a_refusal_returns_even_when_the_same_round_acted(aconn):
    """12.4/D51: a refused thought act is never lost because another call took effect."""
    fake = FakeLLM([[("act", {"capability": "telegram.say", "arguments": {"text": "siap"}}),
                     ("expect", {"about": "kabari kalau server down",
                                 "expect": {"none": True}})],
                    [("expect", {"about": "kabari kalau server down",
                                 "cue_event": {"kind": "percept", "text_like": "server down"},
                                 "expect": {"none": True}, "policy": {"once": True}})],
                    []])
    kernel = build(aconn, clock=FakeClock(START), pfc=_pfc(fake), settings=SETTINGS)
    body = RecordingBody()
    kernel.register_adapter(body)

    await _feed(kernel, _msg("kabari aku kalau ada yang bilang server down"))

    assert "expect refused: expect needs cue_time or cue_event" in \
        fake.requests[1]["messages"][-1]["content"]
    assert [action.payload for action in body.actions] == [{"text": "siap"}]
    assert await _kinds(aconn) == ["percept", "model.call", "model.call", "model.call",
                                   "act.out", "act.confirmed", "expectation.created"]
    cur = await aconn.execute("SELECT about, cue_event FROM expectations")
    assert await cur.fetchall() == [("kabari kalau server down",
                                     {"kind": "percept", "text_like": "server down"})]


async def test_a_refused_round_never_runs_past_max_thought_rounds(aconn):
    """D51: a refusal buys one result round; the wake still ends at the prior's limit."""
    fake = FakeLLM([[("expect", {"about": "tanpa cue", "expect": {"none": True}})],
                    [("expect", {"about": "tanpa cue", "expect": {"none": True}})]])
    kernel = build(aconn, clock=FakeClock(START), pfc=_pfc(fake),
                    settings=settings(constants={"max_thought_rounds": 2}))
    kernel.register_adapter(RecordingBody())

    await _feed(kernel, _msg("kabari aku nanti"))

    assert len(fake.requests) == 2
    assert await _kinds(aconn) == ["percept", "model.call", "model.call", "decision.silent"]


async def test_a_chain_in_a_place_shows_every_step_and_its_latest_result(aconn):
    """D52: <doing> follows the chain where it started, not only inside task threads."""
    root = await append_event(aconn, ts=START, kind="percept", who="person:owner", place=PLACE,
                              payload={"description": "apa yang kita lakukan bulan lalu?"})
    recall = await append_event(aconn, ts=START, kind="act.out", place=PLACE,
                                caused_by=root.id,
                                payload={"capability": "self.recall", "communicative": False,
                                         "arguments": {"from": "2026-10-01",
                                                       "to": "2026-11-01"}})
    result = await append_event(aconn, ts=START, kind="act.result", place=PLACE,
                                caused_by=recall.id,
                                payload={"capability": "self.recall",
                                         "result": {"count": 1, "found": [{"text": "Kopi"}]}})
    trigger = Event(id=result.id, ts=START, kind="act.result", place=PLACE,
                    payload=result.payload)

    lines = await doing_lines(aconn, PLACE, None, SETTINGS["constants"], trigger=trigger)

    assert lines[0] == "goal: apa yang kita lakukan bulan lalu?"
    assert 'step: self.recall {"to": "2026-11-01", "from": "2026-10-01"}' in lines
    assert 'result: {"count": 1, "found": [{"text": "Kopi"}]}' in lines
    said = await append_event(aconn, ts=START, kind="act.out", place=PLACE,
                              caused_by=root.id,
                              payload={"capability": "telegram.say",
                                       "communicative": True,
                                       "arguments": {"text": "Kita buka Kopi Senja."}})
    later = await append_event(aconn, ts=START, kind="act.result", place=PLACE,
                               caused_by=said.id,
                               payload={"capability": "telegram.say", "result": {"ok": True}})
    done = Event(id=later.id, ts=START, kind="act.result", place=PLACE, payload=later.payload)
    assert await doing_lines(aconn, PLACE, None, SETTINGS["constants"], trigger=done) == ()


async def test_unaddressed_message_does_not_wake_the_pfc(aconn):
    fake = FakeLLM()
    kernel = build(aconn, clock=FakeClock(START), pfc=_pfc(fake), settings=SETTINGS)

    await _feed(kernel, _msg("group chatter", addressed=False))

    assert fake.requests == []
    assert await _kinds(aconn) == ["percept", "decision.silent"]


async def test_an_unknown_tool_is_refused_and_the_loop_lives(aconn):
    """12.13: a name outside the five tools is a refusal line, not a thrown turn."""
    fake = FakeLLM([[("shout", {"text": "hi"})], [("act", {"capability": "telegram.say", "arguments": {"text": "still here"}})]])
    kernel = build(aconn, clock=FakeClock(START), pfc=_pfc(fake), settings=SETTINGS)
    kernel.register_adapter(RecordingBody())

    await _feed(kernel, _msg("first"))
    await _feed(kernel, _msg("second", message_id=2))

    assert "shout refused: unknown tool: shout" in fake.requests[1]["messages"][-1]["content"]
    kinds = await _kinds(aconn)
    assert kinds == ["percept", "model.call", "model.call", "act.out", "act.confirmed",
                     "percept", "model.call", "act.out", "act.confirmed"]


async def test_llm_failure_becomes_an_error_event_and_the_loop_lives(aconn):
    fake = FakeLLM(status=500)
    kernel = build(aconn, clock=FakeClock(START), pfc=_pfc(fake), settings=SETTINGS)

    await _feed(kernel, _msg("halo"))

    kinds = await _kinds(aconn)
    assert kinds == ["percept", "error"]
    cur = await aconn.execute("SELECT payload, caused_by FROM events WHERE kind='error'")
    payload, caused_by = await cur.fetchone()
    assert "500" in payload["error"]
    assert caused_by is not None

    # A later, unaddressed event is still processed: the loop never died.
    await _feed(kernel, _msg("later", addressed=False, message_id=2))
    assert (await _kinds(aconn))[-1] == "decision.silent"


async def test_mvp_a_restart_keeps_the_conversation_context(aconn):
    """PLAN.md Step 4 MVP (a): after a restart, the next prompt carries the topic."""
    clock = FakeClock(START)

    first_fake = FakeLLM([[("act", {"capability": "telegram.say", "arguments": {"text": "kita bahas kopi ya"}})]])
    first = build(aconn, clock=clock, pfc=_pfc(first_fake), settings=SETTINGS)
    first.register_adapter(RecordingBody())
    await _feed(first, _msg("besok kita bahas apa?"))

    # Restart: a new Kernel and a new Reasoning on the same database.
    second_fake = FakeLLM([[("act", {"capability": "telegram.say", "arguments": {"text": "lanjut soal kopi"}})]])
    second = build(aconn, clock=clock, pfc=_pfc(second_fake), settings=SETTINGS)
    body = RecordingBody(aconn)
    second.register_adapter(body)
    await _feed(second, _msg("lanjut", message_id=2))

    volatile = second_fake.last_messages[2]["content"]
    recent = volatile.split("<recent>", 1)[1].split("</recent>", 1)[0]
    assert "besok kita bahas apa?" in recent
    assert "Mona: kita bahas kopi ya" in recent
    assert "<event>person:owner: lanjut</event>" in volatile
    assert body.actions[0].payload == {"text": "lanjut soal kopi"}


async def test_dm_gets_a_reply_through_the_telegram_body(aconn):
    """The manual check, automated: a DM becomes an event and a tool call becomes sendMessage."""
    from monoagens.adapters.telegram import TelegramBody

    update = {
        "update_id": 900,
        "message": {
            "message_id": 5,
            "date": 1762800000,
            "chat": {"id": 111, "type": "private"},
            "from": {"id": 111, "is_bot": False, "first_name": "Muqsith"},
            "text": "halo",
        },
    }
    sent: list[dict] = []

    def handler(request: httpx.Request) -> httpx.Response:
        method = request.url.path.rsplit("/", 1)[-1]
        params = json.loads(request.content or b"{}")
        if method == "getMe":
            return httpx.Response(200, json={"ok": True, "result": {"id": 42, "username": "mona_bot"}})
        if method == "getUpdates":
            delivered = [update] if params.get("offset", 0) <= 900 else []
            return httpx.Response(200, json={"ok": True, "result": delivered})
        if method == "sendMessage":
            sent.append(params)
        return httpx.Response(200, json={"ok": True, "result": True})

    body = TelegramBody("token", transport=httpx.MockTransport(handler))
    fake = FakeLLM([[("act", {"capability": "telegram.say",
                              "arguments": {"text": "halo!"}})]])
    kernel = build(aconn, clock=FakeClock(START), pfc=_pfc(fake), settings=SETTINGS)
    kernel.register_adapter(body)

    stop = asyncio.Event()
    consumer = asyncio.create_task(kernel.run(stop))
    listener = asyncio.create_task(body.run(kernel.put))
    try:
        for _ in range(2000):
            if sent:
                break
            await asyncio.sleep(0.001)
        else:  # pragma: no cover - defensive
            raise AssertionError("the DM never got a reply")
        # Let the act in flight finish, so its confirmation is committed too.
        stop.set()
        await asyncio.wait_for(consumer, 5.0)
    finally:
        listener.cancel()
        await asyncio.gather(listener, return_exceptions=True)

    assert sent == [{"chat_id": "111", "text": "halo!"}]
    assert await _kinds(aconn) == ["percept", "model.call", "act.out", "act.confirmed"]


async def test_recent_includes_other_places_prefixed(aconn):
    """<recent> lists this place first, then other places' lines the audience allows."""
    fake = FakeLLM()
    kernel = build(aconn, clock=FakeClock(START), pfc=_pfc(fake), settings=SETTINGS)

    await append_event(aconn, ts=START, kind="percept", who="person:owner", place=PLACE,
                       body="telegram", payload={"description": "di sini"})
    await append_event(aconn, ts=START, kind="percept", who="person:owner", place="space:cli",
                       body="cli", payload={"description": "dari cli"})
    await append_event(aconn, ts=START, kind="decision.silent", who="person:owner",
                       place="space:cli", payload={"reason": "internal-elsewhere"})
    await append_event(aconn, ts=START, kind="percept", who="person:alice", place="space:cli",
                       body="cli", payload={"description": "bukan owner"})
    await append_event(aconn, ts=START, kind="expectation.fired", who="person:owner",
                       place="space:cli", body="cli", payload={"about": "minum air"})

    await _feed(kernel, _msg("lanjut"))

    volatile = fake.last_messages[2]["content"]
    recent = volatile.split("<recent>", 1)[1].split("</recent>", 1)[0]
    assert "15:00 person:owner: di sini" in recent
    assert "[space:cli] 15:00 person:owner: dari cli" in recent
    assert "[space:cli] 15:00 expectation: minum air" in recent
    assert "[space:cli] 15:00 person:alice: bukan owner" in recent  # allowed: owner-only context
    assert "internal-elsewhere" not in recent
    assert f"[{PLACE}]" not in recent
    assert recent.index("di sini") < recent.index("[space:cli] 15:00 person:owner: dari cli")


async def test_recent_is_nothing_yet_when_both_parts_are_empty(aconn):
    fake = FakeLLM()
    kernel = build(aconn, clock=FakeClock(START), pfc=_pfc(fake), settings=SETTINGS)

    await _feed(kernel, _msg("halo pertama"))

    volatile = fake.last_messages[2]["content"]
    recent = volatile.split("<recent>", 1)[1].split("</recent>", 1)[0]
    assert recent == "\n(nothing yet)\n"


async def test_tool_names_are_the_five_tools():
    assert TOOL_NAMES == {"think", "act", "expect", "resolve", "revise"}


def test_the_expect_tool_offers_the_free_text_watch():
    expect_tool = next(tool for tool in TOOLS if tool["function"]["name"] == "expect")
    description = expect_tool["function"]["parameters"]["properties"]["expect"]["description"]
    assert '{"text": ' in description
    assert "the cue alone wakes you" in description
    assert "no matching event was seen" in description


def test_the_expect_tool_describes_the_cue_exactly():
    """12.8: every key matches_cue reads is named, and text_like is a plain phrase."""
    expect_tool = next(tool for tool in TOOLS if tool["function"]["name"] == "expect")
    description = expect_tool["function"]["parameters"]["properties"]["cue_event"]["description"]
    assert "event kind exactly as the desk shows it" in description
    assert "percept" in description
    for key in ("who", "place", "not_who", "text_like"):
        assert key in description
    assert "plain phrase" in description
    assert "NOT a regex" in description
    assert "no | alternation" in description
    assert "all of which must match" in description


async def test_the_expect_tool_accepts_a_free_text_condition():
    check = turn_check(_desk(), START, SETTINGS)
    call = ToolCall("expect", {"about": "tell the owner when the document is finished",
                               "cue_event": {"place": PLACE, "text_like": "dokumen"},
                               "expect": {"text": "the document is finished"}})
    assert await check(call) is None


def _doing(steps: int) -> tuple[str, ...]:
    return tuple([f"goal: selesaikan kode"] + [f"step: langkah {n}" for n in range(steps)]
                 + [f"earlier result #{n} (recall it)" for n in range(steps - 1)]
                 + [f"result: hasil {steps - 1}"])


def _desk(**changes) -> PFCContext:
    return PFCContext(now=START, place="space:task:1", place_kind="task", reason="work",
                      **changes)


def test_a_fifteen_step_task_keeps_the_prompt_flat():
    small = settings(constants={"thread_result_tokens": 80, "volatile_tokens_max": 400})
    pfc = Reasoning("https://llm.test/v1", "k", "m", settings=small)
    five = pfc.build_messages(_desk(doing=_doing(5)))[2]["content"]
    fifteen = pfc.build_messages(_desk(doing=_doing(15)))[2]["content"]
    assert len(fifteen) <= len(five) + 200 and len(fifteen) < 1200
    assert "result: hasil 14" in fifteen
    assert fifteen.index("goal:") < fifteen.index("result:")


def test_middleware_notes_are_rendered_with_their_event():
    event = Event(id=1, ts=START, kind="percept", who="person:owner", place=PLACE,
                  payload={"description": "halo", "notes": ["alias: Fulan = person:fulan"]})
    pfc = Reasoning("https://llm.test/v1", "k", "m", settings=SETTINGS)
    volatile = pfc.build_messages(_desk(trigger=event))[2]["content"]
    assert "<event>person:owner: halo\nnote: alias: Fulan = person:fulan</event>" in volatile


async def test_a_task_desk_shows_goal_steps_and_the_latest_result(aconn):
    task = {"goal": "selesaikan kode", "parent": PLACE, "since": 0}
    first = await append_event(aconn, ts=START, kind="act.out", place="space:task:1",
                               payload={"capability": "dev.run",
                                        "arguments": {"text": "langkah satu"}})
    await append_event(aconn, ts=START, kind="act.result", place="space:task:1",
                       payload={"capability": "dev.run", "result": {"text": "hasil satu"}})
    await append_event(aconn, ts=START, kind="act.out", place="space:task:1",
                       payload={"capability": "dev.run", "arguments": {"text": "langkah dua"}})
    await append_event(aconn, ts=START, kind="act.result", place="space:task:1",
                       payload={"capability": "dev.run", "result": {"text": "hasil dua"}})

    lines = await doing_lines(aconn, "space:task:1", task, SETTINGS["constants"])
    assert "goal: selesaikan kode" in lines
    steps = [line for line in lines if line.startswith("step: ")]
    assert steps == ['step: dev.run {"text": "langkah satu"}',
                     'step: dev.run {"text": "langkah dua"}']
    assert f"earlier result #{first.id + 1} (recall it)" in lines
    assert "result: hasil dua" in lines


async def test_recall_lists_the_month_after_vectors_faded_and_sleep_failed(aconn):
    october = datetime(2026, 10, 1, tzinfo=timezone.utc)
    first = await append_event(aconn, ts=october + timedelta(days=2), kind="percept",
                               who="person:owner", place=PLACE,
                               payload={"description": "Kita buka Kopi Senja."})
    second = await append_event(aconn, ts=october + timedelta(days=19), kind="percept",
                                who="person:owner", place=PLACE,
                                payload={"description": "Harga biji kopi naik dua belas persen."})
    tool = SelfTool(aconn, FakeClock(START), visible=privacy.visible,
                               memory_visible=privacy.memory_visible)
    args = {"from": "2026-10-01T00:00:00+00:00", "to": "2026-11-01T00:00:00+00:00"}

    result = await tool.act(Action(kind="recall", place=PLACE, payload=dict(args)))

    assert result["searched"]["from"] == args["from"]
    assert result["count"] == 2
    text = json.dumps(result["found"])
    assert "Kopi Senja" in text and "dua belas persen" in text

    await aconn.execute(
        "INSERT INTO memories (text, place, derived_from, created_at, embedding) "
        "VALUES (%s, %s, %s, %s, NULL)",
        ("Episod Oktober: kafe dan harga kopi.", PLACE, [first.id, second.id], START))
    result = await tool.act(Action(kind="recall", place=PLACE, payload=dict(args)))

    memory = next(item for item in result["found"] if item["kind"] == "memory")
    assert "Episod Oktober" in memory["text"]
    details = json.dumps(memory["details"])
    assert "Kopi Senja" in details and "dua belas persen" in details


async def test_recall_obeys_absolute_line_two(aconn):
    private = "space:tg:2002"
    group = "space:tg:-100"
    await append_event(aconn, ts=START, kind="percept", who="person:fulan", place=private,
                       payload={"description": "rahasia fulan tentang kafe"})
    await append_event(aconn, ts=START, kind="percept", who="person:charlie", place=group,
                       payload={"description": "halo semua"})
    tool = SelfTool(aconn, FakeClock(START), visible=privacy.visible,
                               memory_visible=privacy.memory_visible)

    hidden = await tool.act(Action(kind="recall", place=group,
                                    payload={"query": "rahasia fulan"}))
    assert hidden["found"] == []

    owner = "space:tg:1001"
    await append_event(aconn, ts=START, kind="percept", who="person:owner", place=owner,
                       payload={"description": "halo"})
    allowed = await tool.act(Action(kind="recall", place=owner,
                                     payload={"query": "rahasia fulan"}))
    assert "rahasia fulan" in json.dumps(allowed["found"])


async def test_forty_unread_messages_are_appraised_once(aconn):
    kernel = build(aconn, clock=FakeClock(START), pfc=_pfc(FakeLLM()), settings=SETTINGS)
    messages = [{"description": f"pesan {n}", "addressed": False} for n in range(40)]
    event = Event(id=None, ts=START, kind="percept", who="person:fulan", place=PLACE,
                  body="telegram", payload={
                      "description": "40 pesan belum dibaca", "backfill": True,
                      "sent_at": (START - timedelta(hours=8)).isoformat(),
                      "place_kind": "direct", "messages": messages})

    await _feed(kernel, event)

    kinds = await _kinds(aconn)
    assert kinds.count("decision.silent") == 1
    assert "act.out" not in kinds


async def test_backfill_signals_are_discounted_by_age():
    old = Event(id=1, ts=START - timedelta(hours=10), kind="percept", who="person:fulan",
                place=PLACE, payload={"description": "Mona", "backfill": True,
                                      "sent_at": (START - timedelta(hours=10)).isoformat()})
    fresh = Event(id=2, ts=START, kind="percept", who="person:fulan", place=PLACE,
                  payload={"description": "Mona"})
    aged = await appraise(old, situation=Situation(event=old, self_name="Mona", now=START),
                          now=START, settings=SETTINGS)
    new = await appraise(fresh, situation=Situation(event=fresh, self_name="Mona", now=START),
                         now=START, settings=SETTINGS)
    assert aged.signals["own_name"] < new.signals["own_name"]


# ---------------------------------------------- the argument door (12.13)

async def _one_promise(aconn) -> int:
    """Store one active expectation and return its id."""
    source = await append_event(aconn, ts=START, kind="percept", who="person:owner",
                                place=PLACE, body="telegram",
                                payload={"description": "kabari aku soal dokumen"})
    await expectations.create(aconn, {"about": "tell the owner when the document is finished",
                                "cue_time": {"at": (START + timedelta(hours=1)).isoformat()},
                                "expect": {"none": True}, "policy": {"once": True}},
                        now=START, settings=SETTINGS, source=source)
    cur = await aconn.execute("SELECT id FROM expectations WHERE status = 'active'")
    return (await cur.fetchone())[0]


async def test_a_resolve_with_a_text_id_is_refused_and_corrected_in_the_same_wake(aconn):
    """12.13: a non-integer id is a refusal with its field, never an exception."""
    exp_id = await _one_promise(aconn)
    fake = FakeLLM([[("resolve", {"id": "dokumen sudah selesai", "status": "satisfied"})],
                    [("resolve", {"id": exp_id, "status": "satisfied"})],
                    []])
    kernel = build(aconn, clock=FakeClock(START), pfc=_pfc(fake), settings=SETTINGS)
    kernel.register_adapter(RecordingBody())

    await _feed(kernel, _msg("dokumennya sudah selesai"))

    assert "resolve refused: id: expected integer, got string" in \
        fake.requests[1]["messages"][-1]["content"]
    assert [request["tool_choice"] for request in fake.requests] == \
        ["required", "auto", "auto"]
    cur = await aconn.execute("SELECT status FROM expectations")
    assert [row[0] for row in await cur.fetchall()] == ["satisfied"]
    assert "error" not in await _kinds(aconn)


async def test_an_act_with_a_mistyped_argument_is_refused_before_it_runs(aconn):
    """12.13: the capability's own schema names the bad argument for the next round."""
    fake = FakeLLM([[("act", {"capability": "self.recall",
                              "arguments": {"event_id": "yang tadi"}})],
                    [("act", {"capability": "self.recall", "arguments": {"query": "kopi"}})],
                    []])
    kernel = build(aconn, clock=FakeClock(START), pfc=_pfc(fake), settings=SETTINGS)
    kernel.register_adapter(RecordingBody())

    await _feed(kernel, _msg("apa yang tadi kita bicarakan soal kopi?"))

    assert "act refused: arguments.event_id: expected integer, got string" in \
        fake.requests[1]["messages"][-1]["content"]
    kinds = await _kinds(aconn)
    assert kinds.count("act.result") == 1
    assert "error" not in kinds
    assert [request["tool_choice"] for request in fake.requests][:2] == ["required", "auto"]


async def test_dispatch_refuses_a_mistyped_argument_without_raising(aconn):
    """12.13: a caller that bypasses the door still gets a refusal, not an exception."""
    kernel = build(aconn, clock=FakeClock(START), pfc=_pfc(FakeLLM()), settings=SETTINGS)
    kernel.register_adapter(RecordingBody())
    stored = await append_event(aconn, ts=START, kind="percept", who="person:owner",
                                place=PLACE, body="telegram",
                                payload={"description": "halo", "place_kind": "direct"})

    outcome = await kernel._apply(stored, ToolCall("act", {
        "capability": "self.recall", "arguments": {"event_id": "x"}}))
    assert outcome.acted is True

    kinds = await _kinds(aconn)
    assert "error" not in kinds
    cur = await aconn.execute("SELECT payload FROM events WHERE kind = 'act.result'")
    payloads = [row[0] for row in await cur.fetchall()]
    assert payloads == [{"capability": "self.recall", "result": {
        "error": "invalid arguments for self.recall: arguments.event_id: "
                 "expected integer, got string"}}]


async def test_the_desk_shows_the_id_that_resolve_takes(aconn):
    """12.13: the number the desk shows as #N is exactly what resolve accepts."""
    exp_id = await _one_promise(aconn)
    lines = await active_expectation_lines(aconn, PLACE)

    assert lines[0].startswith(f"#{exp_id} ")
    check = turn_check(_desk(expectations=lines), START, SETTINGS)
    assert await check(ToolCall("resolve", {"id": exp_id, "status": "satisfied"})) is None


def _reply(calls: list[tuple[str, dict]]) -> httpx.Response:
    """Build one chat-completion response carrying `calls`."""
    return httpx.Response(200, json={"choices": [{"message": {
        "role": "assistant", "tool_calls": [
            {"id": f"call_{i}", "type": "function",
             "function": {"name": name, "arguments": json.dumps(arguments)}}
            for i, (name, arguments) in enumerate(calls)]}}]})


def _pfc_with(handler) -> Reasoning:
    return Reasoning("https://llm.test/v1", "k", "m", settings=SETTINGS,
               transport=httpx.MockTransport(handler))


async def test_a_transport_failure_is_retried_once_then_answered(aconn):
    attempts = []

    def handler(request):
        attempts.append(request)
        if len(attempts) == 1:
            raise httpx.ReadError("reset by peer", request=request)
        return _reply([("act", {"capability": "telegram.say",
                                "arguments": {"text": "halo"}})])

    kernel = build(aconn, clock=FakeClock(START), pfc=_pfc_with(handler), settings=SETTINGS)
    kernel.register_adapter(RecordingBody())

    await _feed(kernel, _msg("halo"))

    kinds = await _kinds(aconn)
    assert len(attempts) == 2
    assert kinds.count("model.call") == 1 and kinds.count("act.out") == 1
    assert "error" not in kinds


async def test_a_second_transport_failure_names_the_exception(aconn):
    def handler(request):
        raise httpx.ConnectError("", request=request)   # empty text, as the live session saw

    kernel = build(aconn, clock=FakeClock(START), pfc=_pfc_with(handler), settings=SETTINGS)
    kernel.register_adapter(RecordingBody())

    await _feed(kernel, _msg("halo"))

    cur = await aconn.execute("SELECT payload FROM events WHERE kind = 'error'")
    payload = (await cur.fetchone())[0]
    assert payload["error"] == "ConnectError"


async def test_a_status_failure_names_the_code(aconn):
    def handler(request):
        return httpx.Response(503, json={"error": {"message": "down"}})

    kernel = build(aconn, clock=FakeClock(START), pfc=_pfc_with(handler), settings=SETTINGS)

    await _feed(kernel, _msg("halo"))

    cur = await aconn.execute("SELECT payload FROM events WHERE kind = 'error'")
    payload = (await cur.fetchone())[0]
    assert payload["error"].startswith("HTTPStatusError") and "(status 503)" in payload["error"]


async def test_a_pending_intent_is_on_the_place_desk(aconn):
    """12.17: the next wake in that place sees what it set out to say."""
    from monoagens.core import memory  # local: the test module has no memory import yet

    intent = await append_event(aconn, ts=START, kind="intent", who=None, place=PLACE,
                                payload={"description": "you set out to say 'halo' to person:budi "
                                                        "(from space:tg:2)"})
    asked = await append_event(aconn, ts=START + timedelta(minutes=1), kind="percept",
                               who="person:owner", place=PLACE,
                               payload={"description": "halo", "addressed": True,
                                        "place_kind": "direct"})
    context = await layout.build_context(aconn, asked, PLACE, "addressed message", now=START,
                                         settings=SETTINGS, memories=(), capabilities=())
    volatile = Reasoning("https://llm.test/v1", "k", "m",
                   settings=SETTINGS).build_messages(context)[2]["content"]

    assert "intent: you set out to say 'halo' to person:budi" in volatile
    assert intent.id is not None
