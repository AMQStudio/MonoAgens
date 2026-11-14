"""Tests for Step 7: embeddings, hybrid retrieval, sleep, and memories in the prompt."""

from __future__ import annotations

import asyncio
import json
import math
from datetime import datetime, timedelta, timezone

import httpx
import pytest

from monoagens.core import expectations, memory
from monoagens.infra.clock import FakeClock
from monoagens.core.kernel import Kernel
from monoagens.infra.embedding import FakeEmbedder
from monoagens.core.memory import base_level, decay_d
from monoagens.core.reasoning import Reasoning, ToolCall
from monoagens.infra.providers import OpenAICompatibleProvider
from monoagens.infra.store import append_event, insert_event_vector, read_state
from monoagens.types import Event
from monoagens.boot import build
from tests.support import settings

PLACE = "space:tg:1"
START = datetime(2026, 11, 10, 8, 0, tzinfo=timezone.utc)
SETTINGS = settings()
VALUES = SETTINGS["constants"]
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
    """A sleep-only Reasoning: answers store_memories and records the request."""

    def __init__(self, arguments: dict) -> None:
        self.arguments = arguments
        self.requests: list[tuple[list[dict], list[dict]]] = []
        self.model = "fake-model"
        self.last_usage: dict[str, int] = {}

    async def call(self, messages, tools, *, tool_choice="required"):
        self.requests.append((messages, tools))
        self.last_usage = {"prompt_tokens": 7, "cached_tokens": 2, "completion_tokens": 3,
                           "total_tokens": 10}
        return [ToolCall("store_memories", self.arguments)]


class FailingPFC:
    """A Reasoning whose every call fails, like a broken LLM endpoint."""

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


def _pfc(fake: FakeLLM) -> Reasoning:
    return Reasoning(
        OpenAICompatibleProvider("https://llm.test/v1", "key", "fake",
                                 transport=httpx.MockTransport(fake.handler)),
        settings=SETTINGS)


async def _add_memory(conn, text: str, *, importance=0.5, accessed=START, valid_to=None,
                      created=None, uses=0) -> int:
    vector = (await FakeEmbedder().embed([text]))[0]
    cur = await conn.execute(
        "INSERT INTO memories (who, place, text, embedding, importance, last_accessed_at, "
        "valid_to, created_at, use_count) VALUES (%s, %s, %s, %s, %s, %s, %s, %s, %s) RETURNING id",
        ("person:owner", PLACE, text, vector, importance, accessed, valid_to,
         created or accessed, uses))
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
    await kernel.run_until_idle()
    await asyncio.wait_for(put_task, 1.0)
    return stored


async def _add_vector_memory(conn, text: str, vector: list[float], *, importance=0.9,
                             created=START, uses=0) -> int:
    cur = await conn.execute(
        "INSERT INTO memories (who, place, text, embedding, importance, last_accessed_at, "
        "created_at, use_count) VALUES (%s, %s, %s, %s, %s, %s, %s, %s) RETURNING id",
        ("person:owner", PLACE, text, vector, importance, created, created, uses))
    return (await cur.fetchone())[0]


def _dot(a, b) -> float:
    return sum(x * y for x, y in zip(a, b))


async def test_fake_embedder_is_deterministic_and_word_sensitive():
    embedder = FakeEmbedder()
    first, again, other = await embedder.embed(["kopi tubruk", "kopi tubruk", "anjing bruno"])
    assert first == again and len(first) == 384 and first != other
    message_vector = await embedder.embed(["kopi tubruk manis"])
    assert _dot(message_vector[0], first) > _dot(first, other)


async def test_retrieval_ranks_by_act_r_activation_and_counts_each_use(aconn):
    created = START - timedelta(days=10)
    used = await _add_memory(aconn, "Kopi tubruk tanpa gula", importance=0.30,
                             accessed=created, created=created, uses=20)
    fresh = await _add_memory(aconn, "Kopi tubruk tanpa gula", importance=0.31,
                              accessed=START - timedelta(hours=1),
                              created=START - timedelta(hours=1))
    await _add_memory(aconn, "Anjing bernama Bruno", importance=1.0,
                      created=START - timedelta(hours=1))
    await _add_memory(aconn, "Kopi hitam", importance=1.0,
                      created=START - timedelta(hours=1),
                      valid_to=START - timedelta(hours=1))

    # B(used) = ln(21/0.5) - d*ln(240h) = 1.00; B(fresh unused) = ln(1.5) = 0.41.
    results = await memory.retrieve(aconn, FakeEmbedder(), "kopi", now=START, k=2,
                                    settings=SETTINGS)

    assert len(results) == 2
    assert "importance 0.30" in results[0]           # frequent use outranks a newer unused memory
    assert "importance 0.31" in results[1]
    assert not any("Kopi hitam" in line for line in results)  # valid_to IS NULL only
    assert results[0].activation > results[1].activation
    rows = await (await aconn.execute(
        "SELECT id, last_accessed_at, use_count FROM memories ORDER BY id")).fetchall()
    assert [(row[1], row[2]) for row in rows if row[0] == used] == [(START, 21)]
    assert [(row[1], row[2]) for row in rows if row[0] == fresh] == [(START, 1)]


def test_base_level_is_reproducible_under_a_fake_clock():
    clock = FakeClock(START)
    d = decay_d(SETTINGS)
    age = (clock.now() - (START - timedelta(hours=5))).total_seconds() / 3600.0
    expected = math.log(4 / (1 - d)) - d * math.log(5.0)
    assert base_level(3, age, d) == base_level(3, age, d) == pytest.approx(expected)
    clock.advance(timedelta(hours=5))
    older = (clock.now() - (START - timedelta(hours=5))).total_seconds() / 3600.0
    assert base_level(3, older, d) < base_level(3, age, d)  # the same item, later, is weaker
    assert decay_d(settings(knobs={"memory_persistence": 0})) == pytest.approx(0.7)
    assert decay_d(settings(knobs={"memory_persistence": 1})) == pytest.approx(0.3)


class FixedEmbedder:
    """An embedder that returns vectors chosen by the test."""

    def __init__(self, vectors: dict[str, list[float]]) -> None:
        self.vectors = vectors

    async def embed(self, texts):
        return [self.vectors[text] for text in texts]


async def test_candidates_are_top_40_by_vector_union_top_20_by_text(aconn):
    query = [1.0] + [0.0] * 383
    for index in range(24):
        vector = [0.9, math.sqrt(1 - 0.9**2)] + [0.0] * 382
        vector[2] = float(index) * 0.0001
        await _add_vector_memory(aconn, f"kunci cadangan {index}", vector, uses=0,
                                 created=START - timedelta(days=1))
    # Vector rank 25 of 40: outside the old top-20 window and no text match.
    target = [0.5, math.sqrt(1 - 0.5**2)] + [0.0] * 382
    await _add_vector_memory(aconn, "barang penting di meja kerja", target, uses=40,
                             created=START - timedelta(hours=10))

    results = await memory.retrieve(aconn, FixedEmbedder({"cari": query}), "cari",
                                    now=START, k=1, settings=SETTINGS)

    assert len(results) == 1 and "meja kerja" in results[0]


async def test_best_activation_covers_candidates_and_recent_messages(aconn):
    created = START - timedelta(days=1)
    query = [1.0] + [0.0] * 383
    await _add_vector_memory(aconn, "Kunci cadangan di meja kerja", query, importance=0.5,
                             created=created, uses=20)
    message = await append_event(aconn, ts=START - timedelta(minutes=1), kind="percept",
                                 who="person:owner", place=PLACE, body="telegram",
                                 payload={"description": "Kunci cadangan di meja kerja"})
    await insert_event_vector(aconn, message.id, query)
    cue = await append_event(aconn, ts=START, kind="percept", who="person:owner", place=PLACE,
                             body="telegram", payload={"description": "kunci cadangan"})
    await insert_event_vector(aconn, cue.id, query)
    event = Event(id=cue.id, ts=START, kind="percept", who="person:owner", place=PLACE,
                  payload={"description": "kunci cadangan"}, description="kunci cadangan")

    peak = await memory.best_activation(aconn, FixedEmbedder({"kunci cadangan": query}), event,
                                        now=START, settings=SETTINGS)

    d = decay_d(SETTINGS)
    memory_a = base_level(20, 24.0, d) + 1.0 + 0.5 * 0.5
    message_a = base_level(0, 1 / 60, d) + 1.0
    assert peak == pytest.approx(max(memory_a, message_a), abs=0.02)


async def test_best_activation_ignores_the_cue_vector_so_fok_cannot_saturate(aconn):
    cue = await append_event(aconn, ts=START, kind="percept", who="person:owner", place=PLACE,
                             body="telegram", payload={"description": "kunci cadangan"})
    query = [1.0] + [0.0] * 383
    await insert_event_vector(aconn, cue.id, query)  # the cue's own vector is already stored
    message = await append_event(aconn, ts=START - timedelta(minutes=10), kind="percept",
                                 who="person:owner", place=PLACE, body="telegram",
                                 payload={"description": "barang di meja"})
    await insert_event_vector(aconn, message.id, [0.5, math.sqrt(1 - 0.5**2)] + [0.0] * 382)
    event = Event(id=cue.id, ts=START, kind="percept", who="person:owner", place=PLACE,
                  payload={"description": "kunci cadangan"}, description="kunci cadangan")

    peak = await memory.best_activation(aconn, FixedEmbedder({"kunci cadangan": query}), event,
                                        now=START, settings=SETTINGS)

    d = decay_d(SETTINGS)
    expected = base_level(0, 10 / 60, d) + 0.5 * float(VALUES["context_weight"])
    assert peak == pytest.approx(expected, abs=0.01)
    # A self-match would add base_level(0, 0) + context_weight and push FOK towards 1.
    assert peak < base_level(0, 0.0, d) + float(VALUES["context_weight"])


async def test_sleep_writes_memories_core_memory_and_never_touches_raw_events(aconn):
    max_before = await _seed_events(aconn)
    before = await (await aconn.execute(
        "SELECT id, ts, kind, who, place, body, payload FROM events ORDER BY id")).fetchall()

    pfc = FakePFC(SLEEP_ARGS)
    done = await memory.sleep(aconn, pfc=pfc, embedder=FakeEmbedder(),
                              now=START + timedelta(hours=1), settings=SETTINGS)
    assert done is not None and done.kind == "sleep.done"
    assert [tool["function"]["name"] for tool in pfc.requests[0][1]] == ["store_memories"]
    assert "Muqsith suka kopi tubruk" in pfc.requests[0][0][1]["content"]

    after = await (await aconn.execute(
        "SELECT id, ts, kind, who, place, body, payload FROM events WHERE id <= %s ORDER BY id",
        (max_before,))).fetchall()
    assert after == before  # the Log is untouched
    assert done.payload["last_event_id"] == max_before

    rows = await (await aconn.execute(
        "SELECT text, embedding, importance, derived_from FROM memories ORDER BY id")).fetchall()
    assert [row[0] for row in rows] == [item["text"] for item in SLEEP_ARGS["items"]]
    assert all(len(row[1].to_list()) == 384 for row in rows)
    assert all(max_before in row[3] for row in rows)
    assert (await read_state(aconn, "core_memory"))["text"] == SLEEP_ARGS["core_memory"]
    assert (await read_state(aconn, "sleep:last_event"))["id"] == max_before

    call = await (await aconn.execute(
        "SELECT payload, caused_by FROM events WHERE kind = 'model.call'")).fetchone()
    assert call[0] == {"purpose": "sleep.consolidate", "model": "fake-model",
                       "usage": {"prompt_tokens": 7, "cached_tokens": 2,
                                 "completion_tokens": 3, "reasoning_tokens": 0,
                                 "total_tokens": 10}}
    assert call[1] == max_before

    # Nothing new since the last sleep: sleep does nothing.
    assert await memory.sleep(aconn, pfc=FakePFC(SLEEP_ARGS), embedder=FakeEmbedder(),
                              now=START + timedelta(hours=2), settings=SETTINGS) is None


async def test_sleep_drops_expired_expectations(aconn):
    source_id = await _seed_events(aconn)
    source = Event(id=source_id, ts=START, kind="percept", who="person:owner",
                   place=PLACE, body="telegram", payload={"text": "remind me"}, description="remind me")
    await expectations.create(aconn, {"about": "rapat pagi", "cue_time": {"every": "1d"},
                                "expect": {"none": True},
                                "expires_at": (START - timedelta(days=1)).isoformat()},
                        now=START, settings=SETTINGS, source=source)

    await memory.sleep(aconn, pfc=FakePFC(SLEEP_ARGS), embedder=FakeEmbedder(),
                       now=START + timedelta(hours=1), settings=SETTINGS)

    status = await (await aconn.execute("SELECT status FROM expectations")).fetchone()
    assert status[0] == "dropped"
    payload = await (await aconn.execute(
        "SELECT payload FROM events WHERE kind = 'expectation.resolved'")).fetchone()
    assert payload[0]["status"] == "dropped"


async def test_sleep_due_needs_new_events_and_idleness(aconn):
    clock = FakeClock(START)
    assert await memory.sleep_due(aconn, clock.now(), SETTINGS) is False  # no events at all
    await _seed_events(aconn)
    assert await memory.sleep_due(aconn, START + timedelta(minutes=10), SETTINGS) is False
    assert await memory.sleep_due(aconn, START + timedelta(minutes=31), SETTINGS) is True


async def test_mvp_d_sleep_then_a_differently_worded_question_finds_the_memory(aconn):
    """PLAN.md Step 7 MVP (d): after sleep there are memories and core_memory, the
    raw events are untouched, and a question phrased differently finds the memory."""
    max_before = await _seed_events(aconn)
    before = await (await aconn.execute(
        "SELECT id, kind, payload FROM events WHERE id <= %s ORDER BY id", (max_before,))).fetchall()
    clock = FakeClock(START)
    fake = FakeLLM([[("store_memories", SLEEP_ARGS)], []])
    kernel = build(aconn, clock=clock, pfc=_pfc(fake), settings=SETTINGS, embedder=FakeEmbedder())

    # Idle for 45 minutes: the tick path runs sleep.
    clock.set_time(START + timedelta(minutes=45))
    await kernel.put(Event(id=None, ts=clock.now(), kind="tick"))
    await kernel.step()
    await kernel.run_until_idle()

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
                                                        "place_kind": "direct"}, description="Apa kopi favorit saya?"))
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
    assert await memory.sleep_due(aconn, night, SETTINGS) is True  # the hour, though not idle
    await memory.sleep(aconn, pfc=FakePFC(SLEEP_ARGS), embedder=FakeEmbedder(), now=night, settings=SETTINGS)

    await append_event(aconn, ts=night + timedelta(minutes=5), kind="percept", who="person:owner",
                       place=PLACE, body="telegram", payload={"text": "pagi"})
    assert await memory.sleep_due(aconn, night + timedelta(minutes=10), SETTINGS) is False  # already slept today
    tomorrow = night + timedelta(days=1)
    await append_event(aconn, ts=tomorrow - timedelta(minutes=1), kind="percept", who="person:owner",
                       place=PLACE, body="telegram", payload={"text": "begadang lagi"})
    assert await memory.sleep_due(aconn, tomorrow, SETTINGS) is True


async def test_failing_sleep_retries_at_most_once_per_retry_window(aconn):
    await _seed_events(aconn)
    clock = FakeClock(START + timedelta(minutes=31))
    pfc = FailingPFC()
    kernel = build(aconn, clock=clock, pfc=pfc, settings=SETTINGS, embedder=FakeEmbedder())

    for _ in range(20):  # 20 ticks, 30 seconds apart: 10 minutes in total.
        await kernel.put(Event(id=None, ts=clock.now(), kind="tick"))
        await kernel.step()
        await kernel.run_until_idle()
        clock.advance(30)

    assert pfc.calls == 1
    rows = await (await aconn.execute(
        "SELECT ts, payload FROM events WHERE kind = 'sleep.failed'")).fetchall()
    assert len(rows) == 1 and "llm down" in rows[0][1]["error"]
    assert await read_state(aconn, "sleep:last_event") is None  # nothing consolidated

    clock.set_time(rows[0][0] + timedelta(minutes=31))  # the retry window has passed
    await kernel.put(Event(id=None, ts=clock.now(), kind="tick"))
    await kernel.step()
    await kernel.run_until_idle()
    assert pfc.calls == 2


async def test_internal_events_never_make_sleep_due(aconn):
    await _seed_events(aconn)
    await memory.sleep(aconn, pfc=FakePFC(SLEEP_ARGS), embedder=FakeEmbedder(),
                       now=START + timedelta(hours=1), settings=SETTINGS)
    for offset, kind in enumerate(("decision.silent", "error", "expectation.fired")):
        await append_event(aconn, ts=START + timedelta(hours=2, minutes=offset), kind=kind,
                           who="person:owner", place=PLACE, body="telegram",
                           payload={"reason": "quiet"})

    # Idle long enough, but only bookkeeping happened since sleep: not due.
    assert await memory.sleep_due(aconn, START + timedelta(hours=3), SETTINGS) is False

    await append_event(aconn, ts=START + timedelta(hours=3), kind="percept", who="person:owner",
                       place=PLACE, body="telegram", payload={"text": "halo"})
    assert await memory.sleep_due(aconn, START + timedelta(hours=3, minutes=31), SETTINGS) is True


async def test_retrieval_ignores_an_empty_query(aconn):
    await _add_memory(aconn, "Kopi tubruk kesukaan Muqsith", importance=0.9)
    assert await memory.retrieve(aconn, FakeEmbedder(), "   ", now=START,
                                 k=VALUES["retrieve_k"],
                                 settings=SETTINGS) == []


async def test_memory_query_falls_back_to_the_expectation_about(aconn):
    await _add_memory(aconn, "Kopi tubruk kesukaan Muqsith", importance=0.9)
    lines = await memory.candidates(aconn, FakeEmbedder(),
                                    Event(id=None, ts=START, kind="expectation.fired",
                                          description="kopi"),
                                    now=START, settings=SETTINGS)
    assert lines and "Kopi tubruk" in lines[0]


async def test_private_memories_follow_people_not_places(aconn):
    source = await append_event(aconn, ts=START, kind="percept", who="person:fulan",
                                place="space:tg:9", body="telegram",
                                payload={"place_kind": "direct", "description": "tato rahasia"})
    await _add_memory(aconn, "Muqsith punya tato rahasia", importance=0.9)
    await aconn.execute("UPDATE memories SET private = true, place = 'space:tg:9', "
                        "derived_from = %s", ([source.id],))
    fake = FakeLLM([[]])
    kernel = build(aconn, clock=FakeClock(START), pfc=_pfc(fake), settings=SETTINGS,
                    embedder=FakeEmbedder())

    async def said(who: str, place: str, kind: str) -> Event:
        await append_event(aconn, ts=START, kind="percept", who=who, place=place,
                           body="telegram", payload={"place_kind": kind})
        return Event(id=None, ts=START, kind="percept", who=who, place=place, body="telegram",
                     payload={"description": "tato rahasia", "addressed": True,
                              "place_kind": kind}, description="tato rahasia")

    def memories() -> str:
        prompt = fake.requests[-1]["messages"][2]["content"]
        return prompt.split("<memories>", 1)[1].split("</memories>", 1)[0]

    # The memory lives in a private chat with Fulan; the owner was never part of it.
    await said("person:fulan", "space:tg:9", "direct")
    await said("person:charlie", "space:tg:2", "shared")
    await _feed(kernel, await said("person:owner", "space:tg:2", "shared"))
    assert "tato rahasia" not in memories()

    # The owner's direct chat is a view: allowed.
    await _feed(kernel, await said("person:owner", "space:tg:1", "direct"))
    assert "tato rahasia" in memories()

    # Fulan's own private chat on any body is allowed.
    await _feed(kernel, await said("person:fulan", "space:tg:9", "direct"))
    assert "tato rahasia" in memories()


def test_the_sleep_tool_does_not_ask_for_place_or_private():
    """D53: the core derives a memory's audience; the model never fills it in."""
    item = memory.STORE_MEMORIES["function"]["parameters"]["properties"]["items"]["items"]
    assert set(item["properties"]) == {"text", "importance", "who"}


async def test_sleep_core_memory_derived_from_unions_old_and_new_sources(aconn):
    """P.6: core_memory.derived_from unions prior core_memory sources with new batch."""
    first = await append_event(
        aconn, ts=START, kind="percept", who="person:fulan", place="space:tg:9",
        body="telegram", payload={"text": "rahasia lama", "place_kind": "direct"})
    await memory.sleep(aconn, pfc=FakePFC({"items": [], "core_memory": "Core v1"}),
                       embedder=FakeEmbedder(), now=START + timedelta(hours=1), settings=SETTINGS)
    second = await append_event(
        aconn, ts=START + timedelta(hours=2), kind="percept", who="person:owner",
        place=PLACE, body="telegram", payload={"text": "kabar baru", "place_kind": "direct"})
    await memory.sleep(aconn, pfc=FakePFC({"items": [], "core_memory": "Core v2"}),
                       embedder=FakeEmbedder(), now=START + timedelta(hours=3), settings=SETTINGS)

    core_state = await read_state(aconn, "core_memory")
    assert set(core_state["derived_from"]) == {first.id, second.id}


async def test_sleep_runs_once_per_place_uses_sent_at_and_is_atomic(aconn, monkeypatch):
    """P.7: per-place consolidation, sent_at timestamps in lines, transactional state writes."""
    sent = START - timedelta(days=3)
    ev1 = await append_event(
        aconn, ts=START, kind="percept", who="person:owner", place="space:tg:1",
        body="telegram", payload={"text": "suka teh", "sent_at": sent.isoformat()})
    ev2 = await append_event(
        aconn, ts=START + timedelta(minutes=1), kind="percept", who="person:fulan",
        place="space:tg:2", body="telegram", payload={"text": "suka matcha"})

    class PerPlacePFC:
        def __init__(self) -> None:
            self.requests: list[tuple[list[dict], list[dict]]] = []
            self.model = "fake-model"
            self.last_usage = {"prompt_tokens": 5, "cached_tokens": 0,
                               "completion_tokens": 2, "total_tokens": 7}

        async def call(self, messages, tools, *, tool_choice="required"):
            self.requests.append((messages, tools))
            body = messages[1]["content"]
            if "suka teh" in body:
                return [ToolCall("store_memories", {
                    "items": [{"text": "Memori tempat 1", "importance": 0.7, "who": "person:owner"}],
                    "core_memory": "Core after place 1",
                })]
            return [ToolCall("store_memories", {
                "items": [{"text": "Memori tempat 2", "importance": 0.6, "who": "person:fulan"}],
                "core_memory": "Core after place 2",
            })]

    # First prove transactional rollback when write_state fails after sleep.done.
    import importlib
    from zoneinfo import ZoneInfo

    sleep_mod = importlib.import_module("monoagens.core.memory.sleep")

    async def boom_write_state(*args, **kwargs):
        raise RuntimeError("disk full on state write")

    monkeypatch.setattr(sleep_mod, "write_state", boom_write_state)
    failed = await memory.sleep(aconn, pfc=PerPlacePFC(), embedder=FakeEmbedder(),
                                now=START + timedelta(hours=1), settings=SETTINGS)
    assert failed is not None and failed.kind == "sleep.failed"
    assert await (await aconn.execute("SELECT count(*) FROM memories")).fetchone() == (0,)
    assert await (await aconn.execute(
        "SELECT count(*) FROM events WHERE kind = 'sleep.done'")).fetchone() == (0,)
    monkeypatch.undo()

    pfc = PerPlacePFC()
    done = await memory.sleep(aconn, pfc=pfc, embedder=FakeEmbedder(),
                              now=START + timedelta(hours=2), settings=SETTINGS)
    assert done is not None and done.kind == "sleep.done"
    assert len(pfc.requests) == 2
    sent_local = sent.astimezone(ZoneInfo(str(SETTINGS.get("timezone") or "UTC")))
    assert f"#{ev1.id} {sent_local:%Y-%m-%d %H:%M}" in pfc.requests[0][0][1]["content"]
    assert "suka matcha" not in pfc.requests[0][0][1]["content"]
    assert "Core after place 1" in pfc.requests[1][0][1]["content"]
    assert "suka teh" not in pfc.requests[1][0][1]["content"]

    mem_rows = await (await aconn.execute(
        "SELECT place, text, derived_from, private FROM memories ORDER BY id")).fetchall()
    assert mem_rows == [
        ("space:tg:1", "Memori tempat 1", [ev1.id], True),
        ("space:tg:2", "Memori tempat 2", [ev2.id], True),
    ]
    core_state = await read_state(aconn, "core_memory")
    assert core_state["text"] == "Core after place 2"
    assert set(core_state["derived_from"]) == {ev1.id, ev2.id}

