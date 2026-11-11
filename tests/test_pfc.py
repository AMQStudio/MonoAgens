"""Tests for Step 4: the PFC, its layered prompt, tools, and failure handling."""

from __future__ import annotations

import asyncio
import json
from datetime import datetime, timedelta, timezone

import httpx
import pytest

from monoagens.clock import FakeClock
from monoagens.kernel import Kernel
from monoagens.pfc import PFC, TOOL_NAMES
from monoagens.store import append_event, read_state, write_state
from monoagens.bodies.telegram import CAPABILITIES as TELEGRAM_CAPABILITIES
from monoagens.types import Action, Event

SOUL = {
    "name": "Mona",
    "language": "id",
    "timezone": "Asia/Jakarta",
    "persona": "Calm and honest.",
    "principles": ["Never lie."],
    "cognition": {
        "seed": 1,
        "weights": {"name_mentioned": 0.5, "info_gap": 0.35, "feeling_of_knowing": 0.35,
                    "relevance": 0.2, "social_cost": -0.4},
    },
}
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


def _pfc(fake: FakeLLM) -> PFC:
    return PFC(
        "https://llm.test/v1",
        "test-key",
        "fake-model",
        soul=SOUL,
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
            cur = await self.conn.execute("SELECT kind FROM events ORDER BY id")
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
    await asyncio.wait_for(put_task, 1.0)
    return stored


async def _kinds(conn) -> list[str]:
    cur = await conn.execute("SELECT kind FROM events ORDER BY id")
    return [row[0] for row in await cur.fetchall()]


async def test_layered_prompt_structure(aconn):
    """Stable identity, then core_memory, then tagged volatile sections in order."""
    fake = FakeLLM()
    kernel = Kernel(aconn, clock=FakeClock(START), pfc=_pfc(fake))
    body = RecordingBody()
    kernel.register_body(body)

    seed = await append_event(aconn, ts=START, kind="decision.note", place=PLACE)
    assert seed.id is not None
    await write_state(aconn, "core_memory", {"text": "The owner likes coffee."}, seed.id)
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
    assert capability["enum"] == ["telegram.say", "telegram.react"]
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
    assert "15:00 owner: halo" in volatile
    assert "15:00 Mona: hai" in volatile
    assert "<event>owner: lagi?</event>" in volatile


async def test_recent_shows_only_conversational_events(aconn):
    """<recent> carries percepts, act.out, and fired expectations, never bookkeeping."""
    fake = FakeLLM()
    kernel = Kernel(aconn, clock=FakeClock(START), pfc=_pfc(fake))

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
    assert "15:00 owner: halo pagi" in recent
    assert "15:00 Mona: pagi juga" in recent
    assert "15:00 expectation: minum air" in recent
    for internal in ("internal-silence", "internal-error", "internal-sleep",
                     "internal-expectation"):
        assert internal not in recent
    assert "<event>owner: masih di sana?</event>" in volatile


async def test_say_is_recorded_before_the_body_acts(aconn):
    fake = FakeLLM([[("act", {"capability": "telegram.say", "arguments": {"text": "halo juga"}})]])
    kernel = Kernel(aconn, clock=FakeClock(START), pfc=_pfc(fake))
    body = RecordingBody(aconn)
    kernel.register_body(body)

    trigger = await _feed(kernel, _msg("halo"))

    assert await _kinds(aconn) == ["percept", "model.call", "act.out"]
    cur = await aconn.execute("SELECT who, place, body, payload, caused_by FROM events WHERE kind='act.out'")
    who, place, body_name, payload, caused_by = await cur.fetchone()
    assert (who, place, body_name) == ("person:owner", PLACE, "telegram")
    assert payload == {"capability": "telegram.say", "arguments": {"text": "halo juga"}}
    assert caused_by == trigger.id

    # act.out is already in the Log when act() is called.
    assert body.log_at_act == [["percept", "model.call", "act.out"]]
    assert body.actions == [
        Action(kind="say", place=PLACE, payload={"text": "halo juga"}, body="telegram")
    ]
    last_spoke = await read_state(aconn, f"last_spoke:{PLACE}")
    assert datetime.fromisoformat(last_spoke["ts"]) == START


async def test_react_is_recorded_before_the_body_acts(aconn):
    fake = FakeLLM([[("act", {"capability": "telegram.react", "arguments": {"emoji": "👍"}})]])
    kernel = Kernel(aconn, clock=FakeClock(START), pfc=_pfc(fake))
    body = RecordingBody(aconn)
    kernel.register_body(body)

    await _feed(kernel, _msg("mantap", message_id=42))

    assert await _kinds(aconn) == ["percept", "model.call", "act.out"]
    cur = await aconn.execute("SELECT payload FROM events WHERE kind='act.out'")
    assert (await cur.fetchone())[0] == {
        "capability": "telegram.react", "arguments": {"emoji": "👍"}}
    assert body.log_at_act == [["percept", "model.call", "act.out"]]
    assert body.actions == [
        Action(kind="react", place=PLACE, payload={"emoji": "👍"}, body="telegram")
    ]


async def test_an_organ_result_is_recorded_as_act_result(aconn):
    fake = FakeLLM([[("act", {"capability": "telegram.say",
                              "arguments": {"text": "done"}})]])
    kernel = Kernel(aconn, clock=FakeClock(START), pfc=_pfc(fake))
    kernel.register_body(ResultBody())

    await _feed(kernel, _msg("halo"))

    assert await _kinds(aconn) == ["percept", "model.call", "act.out", "act.result"]
    cur = await aconn.execute("SELECT payload FROM events WHERE kind='act.result'")
    assert (await cur.fetchone())[0] == {"capability": "telegram.say",
                                         "result": {"message_id": 7}}


async def test_a_wake_without_act_is_recorded_as_silent(aconn):
    fake = FakeLLM([[]])
    kernel = Kernel(aconn, clock=FakeClock(START), pfc=_pfc(fake))
    body = RecordingBody()
    kernel.register_body(body)

    await _feed(kernel, _msg("hm"))

    assert await _kinds(aconn) == ["percept", "model.call", "decision.silent"]
    cur = await aconn.execute("SELECT payload FROM events WHERE kind='decision.silent'")
    assert (await cur.fetchone())[0] == {"reason": "no act"}
    assert body.actions == []


async def test_think_is_recorded_and_silence_keeps_its_reason(aconn):
    fake = FakeLLM([[("think", {"thought": "nothing to add", "motivation": 2,
                               "confidence": 0.4})]])
    kernel = Kernel(aconn, clock=FakeClock(START), pfc=_pfc(fake))
    kernel.register_body(RecordingBody())

    await _feed(kernel, _msg("hm"))

    assert await _kinds(aconn) == ["percept", "model.call", "decision.note", "decision.silent"]
    cur = await aconn.execute("SELECT payload FROM events WHERE kind='decision.note'")
    assert (await cur.fetchone())[0] == {
        "act": "think", "thought": "nothing to add", "motivation": 2, "confidence": 0.4}
    cur = await aconn.execute("SELECT payload FROM events WHERE kind='decision.silent'")
    assert (await cur.fetchone())[0] == {"reason": "nothing to add"}


async def test_unaddressed_message_does_not_wake_the_pfc(aconn):
    fake = FakeLLM()
    kernel = Kernel(aconn, clock=FakeClock(START), pfc=_pfc(fake))

    await _feed(kernel, _msg("group chatter", addressed=False))

    assert fake.requests == []
    assert await _kinds(aconn) == ["percept", "decision.silent"]


async def test_invalid_tool_call_becomes_an_error_event_and_the_loop_lives(aconn):
    fake = FakeLLM([[("shout", {"text": "hi"})], [("act", {"capability": "telegram.say", "arguments": {"text": "still here"}})]])
    kernel = Kernel(aconn, clock=FakeClock(START), pfc=_pfc(fake))
    kernel.register_body(RecordingBody())

    await _feed(kernel, _msg("first"))
    await _feed(kernel, _msg("second", message_id=2))

    kinds = await _kinds(aconn)
    assert kinds == ["percept", "model.call", "error", "decision.silent", "percept", "model.call", "act.out"]
    cur = await aconn.execute("SELECT payload FROM events WHERE kind='error'")
    error = (await cur.fetchone())[0]
    assert "invalid tool call: shout" in error["error"]


async def test_llm_failure_becomes_an_error_event_and_the_loop_lives(aconn):
    fake = FakeLLM(status=500)
    kernel = Kernel(aconn, clock=FakeClock(START), pfc=_pfc(fake))

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
    first = Kernel(aconn, clock=clock, pfc=_pfc(first_fake))
    first.register_body(RecordingBody())
    await _feed(first, _msg("besok kita bahas apa?"))

    # Restart: a new Kernel and a new PFC on the same database.
    second_fake = FakeLLM([[("act", {"capability": "telegram.say", "arguments": {"text": "lanjut soal kopi"}})]])
    second = Kernel(aconn, clock=clock, pfc=_pfc(second_fake))
    body = RecordingBody(aconn)
    second.register_body(body)
    await _feed(second, _msg("lanjut", message_id=2))

    volatile = second_fake.last_messages[2]["content"]
    recent = volatile.split("<recent>", 1)[1].split("</recent>", 1)[0]
    assert "besok kita bahas apa?" in recent
    assert "Mona: kita bahas kopi ya" in recent
    assert "<event>owner: lanjut</event>" in volatile
    assert body.actions[0].payload == {"text": "lanjut soal kopi"}


async def test_dm_gets_a_reply_through_the_telegram_body(aconn):
    """The manual check, automated: a DM becomes an event and a tool call becomes sendMessage."""
    from monoagens.bodies.telegram import TelegramBody

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

    body = TelegramBody("token", owner_ids={111}, transport=httpx.MockTransport(handler))
    kernel = Kernel(
        aconn, clock=FakeClock(START), pfc=_pfc(FakeLLM([[("act", {"capability": "telegram.say", "arguments": {"text": "halo!"}})]]))
    )
    kernel.register_body(body)

    stop = asyncio.Event()
    tasks = [asyncio.create_task(kernel.run(stop)), asyncio.create_task(body.run(kernel.put))]
    try:
        for _ in range(2000):
            if sent:
                break
            await asyncio.sleep(0.001)
        else:  # pragma: no cover - defensive
            raise AssertionError("the DM never got a reply")
    finally:
        stop.set()
        for task in tasks:
            task.cancel()
        await asyncio.gather(*tasks, return_exceptions=True)

    assert sent == [{"chat_id": "111", "text": "halo!"}]
    assert await _kinds(aconn) == ["percept", "model.call", "act.out"]


async def test_recent_includes_other_places_prefixed(aconn):
    """<recent> lists this place first, then the same person's other places, prefixed."""
    fake = FakeLLM()
    kernel = Kernel(aconn, clock=FakeClock(START), pfc=_pfc(fake))

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
    assert "15:00 owner: di sini" in recent
    assert "[space:cli] 15:00 owner: dari cli" in recent
    assert "[space:cli] 15:00 expectation: minum air" in recent
    assert "bukan owner" not in recent
    assert "internal-elsewhere" not in recent
    assert f"[{PLACE}]" not in recent
    assert recent.index("di sini") < recent.index("[space:cli] 15:00 owner: dari cli")


async def test_recent_is_nothing_yet_when_both_parts_are_empty(aconn):
    fake = FakeLLM()
    kernel = Kernel(aconn, clock=FakeClock(START), pfc=_pfc(fake))

    await _feed(kernel, _msg("halo pertama"))

    volatile = fake.last_messages[2]["content"]
    recent = volatile.split("<recent>", 1)[1].split("</recent>", 1)[0]
    assert recent == "\n(nothing yet)\n"


async def test_tool_names_are_the_five_tools():
    assert TOOL_NAMES == {"think", "act", "expect", "resolve", "revise"}
