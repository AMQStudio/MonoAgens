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
from monoagens.types import Action, Event

SOUL = {
    "name": "Mona",
    "language": "id",
    "timezone": "Asia/Jakarta",
    "persona": "Calm and honest.",
    "principles": ["Never lie."],
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
        self.responses = responses or [[("stay_silent", {"reason": "testing"})]]
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

    async def act(self, action: Action) -> None:
        if self.conn is not None:
            cur = await self.conn.execute("SELECT kind FROM events ORDER BY id")
            self.log_at_act.append([row[0] for row in await cur.fetchall()])
        self.actions.append(action)


def _msg(text: str, *, message_id: int = 1, addressed: bool = True, who: str = "person:owner") -> Event:
    return Event(
        id=None,
        ts=START,
        kind="msg.in",
        who=who,
        place=PLACE,
        body="telegram",
        payload={
            "text": text,
            "message_id": message_id,
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
    await append_event(aconn, ts=START, kind="msg.in", who="person:owner", place=PLACE,
                       body="telegram", payload={"text": "halo"})
    await append_event(aconn, ts=START, kind="msg.out", who="person:owner", place=PLACE,
                       body="telegram", payload={"text": "hai"})

    await _feed(kernel, _msg("lagi?"))

    request = fake.requests[0]
    assert request["model"] == "fake-model"
    assert request["tool_choice"] == "required"
    assert {tool["function"]["name"] for tool in request["tools"]} == {"say", "react", "stay_silent"}
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


async def test_say_is_recorded_before_the_body_acts(aconn):
    fake = FakeLLM([[("say", {"text": "halo juga"})]])
    kernel = Kernel(aconn, clock=FakeClock(START), pfc=_pfc(fake))
    body = RecordingBody(aconn)
    kernel.register_body(body)

    trigger = await _feed(kernel, _msg("halo"))

    assert await _kinds(aconn) == ["msg.in", "msg.out"]
    cur = await aconn.execute("SELECT who, place, body, payload, caused_by FROM events WHERE kind='msg.out'")
    who, place, body_name, payload, caused_by = await cur.fetchone()
    assert (who, place, body_name) == ("person:owner", PLACE, "telegram")
    assert payload == {"text": "halo juga"}
    assert caused_by == trigger.id

    # msg.out is already in the Log when act() is called.
    assert body.log_at_act == [["msg.in", "msg.out"]]
    assert body.actions == [
        Action(kind="say", place=PLACE, payload={"text": "halo juga"}, body="telegram")
    ]
    last_spoke = await read_state(aconn, f"last_spoke:{PLACE}")
    assert datetime.fromisoformat(last_spoke["ts"]) == START


async def test_react_is_recorded_before_the_body_acts(aconn):
    fake = FakeLLM([[("react", {"emoji": "👍"})]])
    kernel = Kernel(aconn, clock=FakeClock(START), pfc=_pfc(fake))
    body = RecordingBody(aconn)
    kernel.register_body(body)

    await _feed(kernel, _msg("mantap", message_id=42))

    assert await _kinds(aconn) == ["msg.in", "react.out"]
    cur = await aconn.execute("SELECT payload FROM events WHERE kind='react.out'")
    assert (await cur.fetchone())[0] == {"emoji": "👍", "message_id": 42}
    assert body.log_at_act == [["msg.in", "react.out"]]
    assert body.actions == [
        Action(kind="react", place=PLACE, payload={"emoji": "👍", "message_id": 42}, body="telegram")
    ]


async def test_stay_silent_is_recorded_and_calls_no_body(aconn):
    fake = FakeLLM([[("stay_silent", {"reason": "nothing to add"})]])
    kernel = Kernel(aconn, clock=FakeClock(START), pfc=_pfc(fake))
    body = RecordingBody()
    kernel.register_body(body)

    await _feed(kernel, _msg("hm"))

    assert await _kinds(aconn) == ["msg.in", "decision.silent"]
    cur = await aconn.execute("SELECT payload FROM events WHERE kind='decision.silent'")
    assert (await cur.fetchone())[0] == {"reason": "nothing to add"}
    assert body.actions == []


async def test_unaddressed_message_does_not_wake_the_pfc(aconn):
    fake = FakeLLM()
    kernel = Kernel(aconn, clock=FakeClock(START), pfc=_pfc(fake))

    await _feed(kernel, _msg("group chatter", addressed=False))

    assert fake.requests == []
    assert await _kinds(aconn) == ["msg.in", "decision.silent"]


async def test_invalid_tool_call_becomes_an_error_event_and_the_loop_lives(aconn):
    fake = FakeLLM([[("shout", {"text": "hi"})], [("say", {"text": "still here"})]])
    kernel = Kernel(aconn, clock=FakeClock(START), pfc=_pfc(fake))
    kernel.register_body(RecordingBody())

    await _feed(kernel, _msg("first"))
    await _feed(kernel, _msg("second", message_id=2))

    kinds = await _kinds(aconn)
    assert kinds == ["msg.in", "error", "msg.in", "msg.out"]
    cur = await aconn.execute("SELECT payload FROM events WHERE kind='error'")
    error = (await cur.fetchone())[0]
    assert "invalid tool call: shout" in error["error"]


async def test_llm_failure_becomes_an_error_event_and_the_loop_lives(aconn):
    fake = FakeLLM(status=500)
    kernel = Kernel(aconn, clock=FakeClock(START), pfc=_pfc(fake))

    await _feed(kernel, _msg("halo"))

    kinds = await _kinds(aconn)
    assert kinds == ["msg.in", "error"]
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

    first_fake = FakeLLM([[("say", {"text": "kita bahas kopi ya"})]])
    first = Kernel(aconn, clock=clock, pfc=_pfc(first_fake))
    first.register_body(RecordingBody())
    await _feed(first, _msg("besok kita bahas apa?"))

    # Restart: a new Kernel and a new PFC on the same database.
    second_fake = FakeLLM([[("say", {"text": "lanjut soal kopi"})]])
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
        aconn, clock=FakeClock(START), pfc=_pfc(FakeLLM([[("say", {"text": "halo!"})]]))
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
    assert await _kinds(aconn) == ["msg.in", "msg.out"]


async def test_tool_names_are_the_three_step_four_tools():
    assert TOOL_NAMES == {"say", "react", "stay_silent"}
