"""Tests for Step 5: the gate, its score breakdown, the bands, and MVP (b)."""

from __future__ import annotations

import asyncio
import json
from datetime import datetime, timedelta, timezone

import httpx
import pytest

from monoagens.clock import FakeClock
from monoagens.gate import Gate, Verdict
from monoagens.kernel import Kernel
from monoagens.pfc import PFC
from monoagens.store import append_event, write_state
from monoagens.types import Action, Event

SOUL = {
    "name": "Mona",
    "language": "id",
    "timezone": "Asia/Jakarta",
    "persona": "Calm and honest.",
    "principles": ["Never lie."],
    "gate": {
        "weights": {
            "addressed": 1.0,
            "surprise": 0.8,
            "cooldown_penalty": -0.3,
            "crowd_penalty": -0.2,
        },
        "band": {"silent": 0.3, "wake": 0.6},
        "cooldown_minutes": 3,
        "crowd_window_minutes": 5,
    },
}
GROUP = "space:tg:-100123"
START = datetime(2026, 11, 10, 8, 0, tzinfo=timezone.utc)


class FakeLLM:
    """Scripted OpenAI-compatible endpoint; records every request it receives."""

    def __init__(self, responses: list[list[tuple[str, dict]]] | None = None) -> None:
        self.responses = responses or [[("stay_silent", {"reason": "testing"})]]
        self.requests: list[dict] = []

    def handler(self, request: httpx.Request) -> httpx.Response:
        self.requests.append(json.loads(request.content))
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


def _pfc(fake: FakeLLM) -> PFC:
    return PFC(
        "https://llm.test/v1",
        "test-key",
        "fake-model",
        soul=SOUL,
        transport=httpx.MockTransport(fake.handler),
    )


class RecordingBody:
    """A body that records the actions handed to it."""

    name = "telegram"

    def __init__(self) -> None:
        self.actions: list[Action] = []

    async def act(self, action: Action) -> None:
        self.actions.append(action)


def _gate() -> Gate:
    return Gate.from_soul(SOUL)


def _event(
    text: str = "obrolan",
    *,
    addressed: bool = False,
    place: str = GROUP,
    who: str = "person:tg:7",
    message_id: int = 1,
    place_kind: str = "group",
) -> Event:
    return Event(
        id=None,
        ts=START,
        kind="msg.in",
        who=who,
        place=place,
        body="telegram",
        payload={
            "text": text,
            "message_id": message_id,
            "addressed": addressed,
            "place_kind": place_kind,
        },
    )


async def _seed(conn, kind: str, place: str, *, ts: datetime = START, count: int = 1) -> None:
    for _ in range(count):
        await append_event(conn, ts=ts, kind=kind, who="person:tg:1", place=place, body="telegram")


async def _seed_last_spoke(conn, place: str, ts: datetime) -> None:
    seed = await append_event(conn, ts=ts, kind="decision.note", place=place)
    await write_state(conn, f"last_spoke:{place}", {"ts": ts.isoformat()}, seed.id)


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


async def test_addressed_event_always_wakes_and_pays_no_penalties(aconn):
    place = "space:tg:-100999"
    await _seed_last_spoke(aconn, place, START - timedelta(seconds=10))
    await _seed(aconn, "msg.in", place, count=20)

    verdict = await _gate().judge(aconn, _event("halo", addressed=True, place=place), now=START)

    assert verdict.decision == "wake"
    assert verdict.score == 1.0
    assert verdict.reason == "addressed message"
    assert verdict.breakdown == {
        "addressed": 1.0,
        "surprise": 0.0,
        "cooldown_penalty": 0.0,
        "crowd_penalty": 0.0,
        "score": 1.0,
    }


async def test_cooldown_penalty_applies_to_unaddressed_events_until_it_expires(aconn):
    place = "space:tg:9"
    await _seed_last_spoke(aconn, place, START - timedelta(seconds=60))

    fresh = await _gate().judge(aconn, _event(place=place), now=START)
    assert fresh.breakdown["cooldown_penalty"] == pytest.approx(2 / 3)
    assert fresh.breakdown["contributions"]["cooldown_penalty"] == pytest.approx(-0.2)
    assert fresh.score == 0.0
    assert fresh.decision == "silent"

    expired = await _gate().judge(aconn, _event(place=place), now=START + timedelta(minutes=4))
    assert expired.breakdown["cooldown_penalty"] == 0.0


async def test_crowd_penalty_saturates_at_ten_messages(aconn):
    busy, quiet = "space:tg:10", "space:tg:5"
    await _seed(aconn, "msg.in", busy, count=10)
    await _seed(aconn, "msg.in", quiet, count=5)

    crowded = await _gate().judge(aconn, _event(place=busy), now=START)
    assert crowded.breakdown["crowd_penalty"] == 1.0
    assert crowded.breakdown["contributions"]["crowd_penalty"] == -0.2
    assert crowded.score == 0.0

    calmer = await _gate().judge(aconn, _event(place=quiet), now=START)
    assert calmer.breakdown["crowd_penalty"] == 0.5


async def test_surprise_puts_an_unaddressed_event_in_the_note_band(aconn):
    verdict = await _gate().judge(aconn, _event(place="space:tg:note"), now=START, surprise=0.5)

    assert verdict.decision == "note"
    assert verdict.score == 0.4
    assert verdict.reason == "gate score in note band"
    assert verdict.payload()["breakdown"]["contributions"]["surprise"] == 0.4


async def test_high_surprise_wakes_the_pfc_with_the_score_in_the_reason(aconn):
    verdict = await _gate().judge(aconn, _event(place="space:tg:wake"), now=START, surprise=1.0)

    assert verdict.decision == "wake"
    assert verdict.score == 0.8
    assert verdict.reason == "gate score 0.80 >= 0.6"


class NoteGate:
    """A gate stub that forces the note band, which no Step 5 feature can reach."""

    async def judge(self, conn, event, *, now, surprise=0.0):
        return Verdict("note", 0.4, {"addressed": 0.0, "score": 0.4}, "gate score in note band")


async def test_kernel_records_decision_note_with_the_score_breakdown(aconn):
    kernel = Kernel(aconn, clock=FakeClock(START), pfc=_pfc(FakeLLM()), gate=NoteGate())

    await _feed(kernel, _event(place="space:tg:note"))

    assert await _kinds(aconn) == ["msg.in", "decision.note"]
    cur = await aconn.execute("SELECT payload FROM events WHERE kind='decision.note'")
    assert await cur.fetchone() == (
        {"score": 0.4, "breakdown": {"addressed": 0.0, "score": 0.4},
         "reason": "gate score in note band"},
    )


async def test_mvp_b_twenty_group_messages_then_a_mention(aconn):
    """PLAN.md Step 5: 20 ordinary group messages -> no reply; a mention -> a reply."""
    fake = FakeLLM([[("say", {"text": "iya, ada apa?"})]])
    kernel = Kernel(aconn, clock=FakeClock(START), pfc=_pfc(fake), gate=_gate())
    body = RecordingBody()
    kernel.register_body(body)

    for i in range(20):
        await _feed(kernel, _event(f"obrolan {i}", message_id=i + 1, who=f"person:tg:{i}"))

    assert fake.requests == []
    kinds = await _kinds(aconn)
    assert kinds.count("msg.in") == 20
    assert kinds.count("msg.out") == 0
    assert kinds.count("decision.silent") == 20

    cur = await aconn.execute("SELECT payload FROM events WHERE kind='decision.silent' ORDER BY id")
    payloads = [row[0] for row in await cur.fetchall()]
    assert len(payloads) == 20
    for payload in payloads:
        assert payload["score"] == 0.0
        assert payload["reason"] == "gate score below silent threshold"
        assert payload["breakdown"]["addressed"] == 0.0
        assert set(payload["breakdown"]) == {
            "addressed", "surprise", "cooldown_penalty", "crowd_penalty",
            "contributions", "score",
        }

    await _feed(kernel, _event("mona, halo?", message_id=99, addressed=True))

    assert len(fake.requests) == 1
    assert "<why_awake>addressed message</why_awake>" in fake.requests[0]["messages"][2]["content"]
    assert (await _kinds(aconn)).count("msg.out") == 1
    assert body.actions == [
        Action(kind="say", place=GROUP, payload={"text": "iya, ada apa?"}, body="telegram")
    ]
