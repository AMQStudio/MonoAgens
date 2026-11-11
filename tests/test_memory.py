"""Tests for Step 7: embeddings, hybrid retrieval, sleep, and memories in the prompt."""

from __future__ import annotations

import asyncio
import json
from datetime import datetime, timedelta, timezone

import httpx
import pytest

from monoagens import expect, memory
from monoagens.clock import FakeClock
from monoagens.kernel import Kernel
from monoagens.memory import FakeEmbedder, cosine
from monoagens.pfc import PFC, ToolCall
from monoagens.store import append_event, read_state
from monoagens.types import Event

PLACE = "space:tg:1"
START = datetime(2026, 11, 10, 8, 0, tzinfo=timezone.utc)
SOUL = {
    "name": "Mona",
    "language": "id",
    "timezone": "Asia/Jakarta",
    "cognition": {
        "seed": 1,
        "weights": {"name_mentioned": 0.5, "info_gap": 0.35, "feeling_of_knowing": 0.35,
                    "relevance": 0.2, "social_cost": -0.4},
    },
    "sleep": {"idle_minutes": 30, "retry_minutes": 30, "hour": 2},
    "memory": {"retrieve_k": 5},
    "expectations": {"default_expires_days": 30, "max_active": 200},
}
SLEEP_ARGS = {
    "items": [
        {"text": "Muqsith suka kopi tubruk tanpa gula", "importance": 0.9,
         "who": "person:owner", "place": PLACE},
        {"text": "Anjing Muqsith bernama Bruno", "importance": 0.3,
         "who": "person:owner", "place": PLACE},
    ],
    "core_memory": "Muqsith tinggal di Bandung dan suka kopi tubruk.",
}


class FakePFC:
    """A sleep-only PFC: answers store_memories and records the request."""

    def __init__(self, arguments: dict) -> None:
        self.arguments = arguments
        self.requests: list[tuple[list[dict], list[dict]]] = []

    async def call(self, messages, tools, *, tool_choice="required"):
        self.requests.append((messages, tools))
        return [ToolCall("store_memories", self.arguments)]


class FailingPFC:
    """A PFC whose every call fails, like a broken LLM endpoint."""

    def __init__(self) -> None:
        self.calls = 0

    async def call(self, messages, tools, *, tool_choice="required"):
        self.calls += 1
        raise RuntimeError("llm down")


class FakeLLM:
    """Scripted OpenAI-compatible endpoint for the kernel MVP path."""

    def __init__(self, responses: list[list[tuple[str, dict]]]) -> None:
        self.responses = responses
        self.requests: list[dict] = []

    def handler(self, request: httpx.Request) -> httpx.Response:
        self.requests.append(json.loads(request.content))
        calls = self.responses[min(len(self.requests) - 1, len(self.responses) - 1)]
        return httpx.Response(200, json={"choices": [{"message": {"tool_calls": [
            {"id": f"call_{i}", "type": "function", "function": {"name": name, "arguments": json.dumps(args)}}
            for i, (name, args) in enumerate(calls)]}}]})


def _pfc(fake: FakeLLM) -> PFC:
    return PFC("https://llm.test/v1", "key", "fake",
               soul=SOUL, transport=httpx.MockTransport(fake.handler))


async def _add_memory(conn, text: str, *, importance=0.5, accessed=START, valid_to=None) -> int:
    vector = (await FakeEmbedder().embed([text]))[0]
    cur = await conn.execute(
        "INSERT INTO memories (who, place, text, embedding, importance, last_accessed_at, valid_to) "
        "VALUES (%s, %s, %s, %s, %s, %s, %s) RETURNING id",
        ("person:owner", PLACE, text, vector, importance, accessed, valid_to))
    return (await cur.fetchone())[0]


async def _seed_events(conn) -> int:
    await append_event(conn, ts=START, kind="percept", who="person:owner", place=PLACE, body="telegram",
                       payload={"text": "Muqsith suka kopi tubruk tanpa gula"})
    last = await append_event(conn, ts=START + timedelta(minutes=1), kind="act.out", who="person:owner",
                              place=PLACE, body="telegram", payload={"text": "baik, dicatat"})
    return last.id


async def _feed(kernel: Kernel, event: Event) -> Event:
    put_task = asyncio.create_task(kernel.put(event))
    await asyncio.sleep(0)
    stored = await kernel.step()
    await asyncio.wait_for(put_task, 1.0)
    return stored


async def test_fake_embedder_is_deterministic_and_word_sensitive():
    embedder = FakeEmbedder()
    first, again, other = await embedder.embed(["kopi tubruk", "kopi tubruk", "anjing bruno"])
    assert first == again and len(first) == 384 and first != other
    message_vector = await embedder.embed(["kopi tubruk manis"])
    assert cosine(message_vector[0], first) > cosine(first, other)


async def test_retrieval_fuses_ranks_and_weighs_importance_and_recency(aconn):
    old = START - timedelta(hours=100)
    fresh = await _add_memory(aconn, "Kopi tubruk tanpa gula", importance=0.9, accessed=old)
    weak = await _add_memory(aconn, "Kopi tubruk tanpa gula", importance=0.1, accessed=old)
    await _add_memory(aconn, "Kopi instan", importance=0.9, accessed=START - timedelta(hours=2000))
    recent = await _add_memory(aconn, "Kopi susu", importance=0.9, accessed=START)
    await _add_memory(aconn, "Anjing bernama Bruno", importance=1.0)
    invalid = await _add_memory(aconn, "Kopi hitam", importance=1.0, valid_to=START - timedelta(hours=1))

    results = await memory.retrieve(aconn, FakeEmbedder(), "kopi", k=4, now=START)

    assert len(results) == 4
    assert results[0].startswith("- Kopi susu")          # recency beats an older equal rank
    strong = next(r for r in results if "importance 0.90" in r and "tubruk" in r)
    weak_line = next(r for r in results if "importance 0.10" in r)
    assert results.index(strong) < results.index(weak_line)  # importance breaks ties
    assert all("Kopi hitam" not in line for line in results)  # valid_to IS NULL only
    accessed = await (await aconn.execute("SELECT last_accessed_at FROM memories WHERE id = %s",
                                          (fresh,))).fetchone()
    assert accessed[0] == START                            # retrieval refreshes last_accessed_at
    assert (await (await aconn.execute("SELECT count(*) FROM memories WHERE id = %s",
                                          (invalid,))).fetchone())[0] == 1


async def test_sleep_writes_memories_core_memory_and_never_touches_raw_events(aconn):
    max_before = await _seed_events(aconn)
    before = await (await aconn.execute(
        "SELECT id, ts, kind, who, place, body, payload FROM events ORDER BY id")).fetchall()

    pfc = FakePFC(SLEEP_ARGS)
    done = await memory.sleep(aconn, pfc=pfc, embedder=FakeEmbedder(),
                              now=START + timedelta(hours=1), soul=SOUL)
    assert done is not None and done.kind == "sleep.done"
    assert [tool["function"]["name"] for tool in pfc.requests[0][1]] == ["store_memories"]
    assert "Muqsith suka kopi tubruk" in pfc.requests[0][0][1]["content"]

    after = await (await aconn.execute(
        "SELECT id, ts, kind, who, place, body, payload FROM events WHERE id <= %s ORDER BY id",
        (max_before,))).fetchall()
    assert after == before  # the Log is untouched
    assert done.payload["last_event_id"] == max_before

    rows = await (await aconn.execute(
        "SELECT text, embedding, importance, source_events FROM memories ORDER BY id")).fetchall()
    assert [row[0] for row in rows] == [item["text"] for item in SLEEP_ARGS["items"]]
    assert all(len(row[1].to_list()) == 384 for row in rows)
    assert all(max_before in row[3] for row in rows)
    assert (await read_state(aconn, "core_memory"))["text"] == SLEEP_ARGS["core_memory"]
    assert (await read_state(aconn, "sleep:last_event"))["id"] == max_before

    # Nothing new since the last sleep: sleep does nothing.
    assert await memory.sleep(aconn, pfc=FakePFC(SLEEP_ARGS), embedder=FakeEmbedder(),
                              now=START + timedelta(hours=2), soul=SOUL) is None


async def test_sleep_drops_expired_expectations(aconn):
    source_id = await _seed_events(aconn)
    source = Event(id=source_id, ts=START, kind="percept", who="person:owner", place=PLACE, body="telegram",
                   payload={"text": "remind me"})
    await expect.create(aconn, {"about": "rapat pagi", "cue_time": {"every": "1d"},
                                "expect": {"none": True},
                                "expires_at": (START - timedelta(days=1)).isoformat()},
                        now=START, soul=SOUL, source=source)

    await memory.sleep(aconn, pfc=FakePFC(SLEEP_ARGS), embedder=FakeEmbedder(),
                       now=START + timedelta(hours=1), soul=SOUL)

    status = await (await aconn.execute("SELECT status FROM expectations")).fetchone()
    assert status[0] == "dropped"
    payload = await (await aconn.execute(
        "SELECT payload FROM events WHERE kind = 'expectation.resolved'")).fetchone()
    assert payload[0]["status"] == "dropped"


async def test_sleep_due_needs_new_events_and_idleness(aconn):
    clock = FakeClock(START)
    assert await memory.sleep_due(aconn, clock.now(), SOUL) is False  # no events at all
    await _seed_events(aconn)
    assert await memory.sleep_due(aconn, START + timedelta(minutes=10), SOUL) is False
    assert await memory.sleep_due(aconn, START + timedelta(minutes=31), SOUL) is True


async def test_mvp_d_sleep_then_a_differently_worded_question_finds_the_memory(aconn):
    """PLAN.md Step 7 MVP (d): after sleep there are memories and core_memory, the
    raw events are untouched, and a question phrased differently finds the memory."""
    max_before = await _seed_events(aconn)
    before = await (await aconn.execute(
        "SELECT id, kind, payload FROM events WHERE id <= %s ORDER BY id", (max_before,))).fetchall()
    clock = FakeClock(START)
    fake = FakeLLM([[("store_memories", SLEEP_ARGS)], []])
    kernel = Kernel(aconn, clock=clock, pfc=_pfc(fake), soul=SOUL, embedder=FakeEmbedder())

    # Idle for 45 minutes: the tick path runs sleep.
    clock.set_time(START + timedelta(minutes=45))
    await kernel.put(Event(id=None, ts=clock.now(), kind="tick"))
    await kernel.step()

    memories = await (await aconn.execute("SELECT text FROM memories ORDER BY id")).fetchall()
    assert len(memories) == 2
    assert (await read_state(aconn, "core_memory"))["text"] == SLEEP_ARGS["core_memory"]
    untouched = await (await aconn.execute(
        "SELECT id, kind, payload FROM events WHERE id <= %s ORDER BY id", (max_before,))).fetchall()
    assert untouched == before

    # A new question, worded differently, must retrieve the coffee memory first.
    clock.set_time(START + timedelta(hours=2))
    await _feed(kernel, Event(id=None, ts=clock.now(), kind="percept", who="person:owner", place=PLACE,
                              body="telegram", payload={"text": "Apa kopi favorit saya?", "addressed": True,
                                                        "place_kind": "direct"}))
    assert len(fake.requests) == 2  # sleep, then the reply turn
    volatile = fake.requests[-1]["messages"][2]["content"]
    section = volatile.split("<memories>", 1)[1].split("</memories>", 1)[0]
    assert "kopi tubruk" in section
    assert section.index("kopi tubruk") < section.index("Bruno")

    accessed = await (await aconn.execute(
        "SELECT last_accessed_at FROM memories WHERE text LIKE %s",
        ("%kopi tubruk%",))).fetchone()
    assert accessed[0] == clock.now()


async def test_sleep_due_runs_at_the_configured_hour_once_per_day(aconn):
    night = datetime(2026, 11, 10, 19, 0, tzinfo=timezone.utc)  # 02:00 in Jakarta
    await append_event(aconn, ts=night - timedelta(minutes=1), kind="percept", who="person:owner",
                       place=PLACE, body="telegram", payload={"text": "masih bangun"})
    assert await memory.sleep_due(aconn, night, SOUL) is True  # the hour, though not idle
    await memory.sleep(aconn, pfc=FakePFC(SLEEP_ARGS), embedder=FakeEmbedder(), now=night, soul=SOUL)

    await append_event(aconn, ts=night + timedelta(minutes=5), kind="percept", who="person:owner",
                       place=PLACE, body="telegram", payload={"text": "pagi"})
    assert await memory.sleep_due(aconn, night + timedelta(minutes=10), SOUL) is False  # already slept today
    tomorrow = night + timedelta(days=1)
    await append_event(aconn, ts=tomorrow - timedelta(minutes=1), kind="percept", who="person:owner",
                       place=PLACE, body="telegram", payload={"text": "begadang lagi"})
    assert await memory.sleep_due(aconn, tomorrow, SOUL) is True


async def test_failing_sleep_retries_at_most_once_per_retry_window(aconn):
    await _seed_events(aconn)
    clock = FakeClock(START + timedelta(minutes=31))
    pfc = FailingPFC()
    kernel = Kernel(aconn, clock=clock, pfc=pfc, soul=SOUL, embedder=FakeEmbedder())

    for _ in range(20):  # 20 ticks, 30 seconds apart: 10 minutes in total.
        await kernel.put(Event(id=None, ts=clock.now(), kind="tick"))
        await kernel.step()
        clock.advance(30)

    assert pfc.calls == 1
    rows = await (await aconn.execute(
        "SELECT ts, payload FROM events WHERE kind = 'sleep.failed'")).fetchall()
    assert len(rows) == 1 and "llm down" in rows[0][1]["error"]
    assert await read_state(aconn, "sleep:last_event") is None  # nothing consolidated

    clock.set_time(rows[0][0] + timedelta(minutes=31))  # the retry window has passed
    await kernel.put(Event(id=None, ts=clock.now(), kind="tick"))
    await kernel.step()
    assert pfc.calls == 2


async def test_internal_events_never_make_sleep_due(aconn):
    await _seed_events(aconn)
    await memory.sleep(aconn, pfc=FakePFC(SLEEP_ARGS), embedder=FakeEmbedder(),
                       now=START + timedelta(hours=1), soul=SOUL)
    for offset, kind in enumerate(("decision.silent", "error", "expectation.fired")):
        await append_event(aconn, ts=START + timedelta(hours=2, minutes=offset), kind=kind,
                           who="person:owner", place=PLACE, body="telegram",
                           payload={"reason": "quiet"})

    # Idle long enough, but only bookkeeping happened since sleep: not due.
    assert await memory.sleep_due(aconn, START + timedelta(hours=3), SOUL) is False

    await append_event(aconn, ts=START + timedelta(hours=3), kind="percept", who="person:owner",
                       place=PLACE, body="telegram", payload={"text": "halo"})
    assert await memory.sleep_due(aconn, START + timedelta(hours=3, minutes=31), SOUL) is True


async def test_retrieval_ignores_an_empty_query(aconn):
    await _add_memory(aconn, "Kopi tubruk kesukaan Muqsith", importance=0.9)
    assert await memory.retrieve(aconn, FakeEmbedder(), "   ", now=START) == []


async def test_memory_query_falls_back_to_the_expectation_about(aconn):
    await _add_memory(aconn, "Kopi tubruk kesukaan Muqsith", importance=0.9)
    kernel = Kernel(aconn, clock=FakeClock(START), soul=SOUL, embedder=FakeEmbedder())
    lines = await kernel._memories(Event(id=None, ts=START, kind="expectation.fired",
                                         payload={"about": "kopi"}))
    assert lines and "Kopi tubruk" in lines[0]
