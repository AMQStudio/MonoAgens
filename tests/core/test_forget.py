"""Tests for lineage (`derived_from`) and the two dedicated vector-deletion paths."""

from __future__ import annotations

from datetime import datetime, timedelta, timezone

import pytest
from psycopg.types.json import Jsonb

from monoagens.core import expectations
from monoagens.core.memory.forget import fade, forget_hard
from monoagens.infra.store import append_event, insert_event_vector
from tests.support import settings

PLACE = "space:tg:1"
START = datetime(2026, 11, 10, 8, 0, tzinfo=timezone.utc)
SETTINGS = settings()


async def _event(conn, *, ts, kind="percept"):
    return await append_event(conn, ts=ts, kind=kind, who="person:owner", place=PLACE,
                              payload={"text": "hello"})


async def _memory(conn, event_id):
    await conn.execute(
        "INSERT INTO memories (text, derived_from) VALUES ('a memory', %s)", ([event_id],))


async def _expectation(conn, event_id):
    await conn.execute(
        "INSERT INTO expectations (about, condition, source_event, derived_from, expires_at) "
        "VALUES ('a promise', %s, %s, %s, %s)",
        (Jsonb({}), event_id, [event_id], START + timedelta(days=1)))


async def test_lineage_is_recorded_on_vectors_memories_and_expectations(aconn):
    source = await _event(aconn, ts=START)
    await insert_event_vector(aconn, source.id, [0.0] * 384)
    cur = await aconn.execute("SELECT derived_from FROM event_vectors WHERE event_id = %s",
                              (source.id,))
    assert (await cur.fetchone())[0] == [source.id]

    await _memory(aconn, source.id)
    cur = await aconn.execute("SELECT derived_from FROM memories")
    assert (await cur.fetchone())[0] == [source.id]

    await expectations.create(aconn, {"about": "coffee", "cue_time": {"every": "1d"},
                                "expect": {"none": True}}, now=START, settings=SETTINGS, source=source)
    cur = await aconn.execute("SELECT derived_from FROM expectations")
    assert (await cur.fetchone())[0] == [source.id]


async def test_fade_releases_only_old_unreferenced_vectors_and_records_it(aconn):
    old = await _event(aconn, ts=START)
    unreferenced = await _event(aconn, ts=START + timedelta(minutes=1))
    recent = await _event(aconn, ts=START + timedelta(hours=2))
    for event in (old, unreferenced, recent):
        await insert_event_vector(aconn, event.id, [0.0] * 384)
    await _memory(aconn, old.id)

    event = await fade(aconn, now=START + timedelta(hours=3),
                       before=START + timedelta(hours=1))
    assert event is not None and event.kind == "fade.done"
    assert event.payload["released"] == 1
    remaining = [row[0] for row in await (await aconn.execute(
        "SELECT event_id FROM event_vectors ORDER BY event_id")).fetchall()]
    assert remaining == [old.id, recent.id]


async def test_fade_writes_nothing_when_no_vector_is_eligible(aconn):
    new = await _event(aconn, ts=START)
    await insert_event_vector(aconn, new.id, [0.0] * 384)
    assert await fade(aconn, now=START, before=START - timedelta(days=1)) is None
    kinds = [row[0] for row in await (await aconn.execute(
        "SELECT kind FROM events ORDER BY id")).fetchall()]
    assert kinds == ["percept"]


async def test_forget_hard_scrubs_payloads_and_deletes_the_lineage(aconn):
    forgotten = await _event(aconn, ts=START)
    kept = await _event(aconn, ts=START + timedelta(minutes=1), kind="act.out")
    await insert_event_vector(aconn, forgotten.id, [0.0] * 384)
    await insert_event_vector(aconn, kept.id, [0.0] * 384)
    await _memory(aconn, forgotten.id)
    await _expectation(aconn, forgotten.id)

    event = await forget_hard(aconn, [forgotten.id], now=START + timedelta(hours=1),
                              who="person:owner", place=PLACE)
    assert event.kind == "forget.hard"
    assert event.payload == {"forgotten_events": 1}

    row = await (await aconn.execute(
        "SELECT kind, who, place, body, payload FROM events WHERE id = %s",
        (forgotten.id,))).fetchone()
    assert row == ("percept", "person:owner", PLACE, None, {"forgotten": True})
    assert (await (await aconn.execute("SELECT count(*) FROM memories")).fetchone())[0] == 0
    assert (await (await aconn.execute("SELECT count(*) FROM expectations")).fetchone())[0] == 0
    remaining = [row[0] for row in await (await aconn.execute(
        "SELECT event_id FROM event_vectors")).fetchall()]
    assert remaining == [kept.id]


async def test_forget_hard_deletes_vectors_derived_from_a_forgotten_event(aconn):
    forgotten = await _event(aconn, ts=START)
    derived = await _event(aconn, ts=START + timedelta(minutes=1))
    await insert_event_vector(aconn, derived.id, [0.0] * 384, derived_from=[forgotten.id])

    await forget_hard(aconn, [forgotten.id], now=START + timedelta(hours=1))
    assert (await (await aconn.execute("SELECT count(*) FROM event_vectors")).fetchone())[0] == 0


async def test_forget_hard_rejects_unknown_events(aconn):
    with pytest.raises(ValueError, match="unknown event ids"):
        await forget_hard(aconn, [999_999], now=START)


async def test_forget_hard_clears_overlapping_core_memory_and_blocks_inflight_vectors(aconn):
    from monoagens.boot import build
    from monoagens.core import memory
    from monoagens.core.reasoning import ToolCall
    from monoagens.infra.clock import FakeClock
    from monoagens.infra.embedding import FakeEmbedder
    from monoagens.infra.store import read_state, rebuild_state

    class _SleepLLM:
        model = "sleep-model"
        last_usage = {"prompt_tokens": 5, "completion_tokens": 5}

        def __init__(self) -> None:
            self.seen_cores: list[str] = []

        async def call(self, messages, _tools, *, tool_choice="required"):
            content = str(messages[-1]["content"])
            self.seen_cores.append(content)
            return [ToolCall("store_memories", {
                "items": [{"text": "clean summary", "importance": 0.5}],
                "core_memory": "rebuilt clean core",
            })]

    forgotten = await _event(aconn, ts=START)
    kept = await _event(aconn, ts=START + timedelta(minutes=1))
    done_ev = await append_event(
        aconn, ts=START + timedelta(minutes=2), kind="sleep.done",
        payload={"core_memory": "secret in core", "last_event_id": kept.id,
                 "derived_from": [forgotten.id, kept.id]})
    await aconn.execute(
        "INSERT INTO state (key, value, updated_by_event) VALUES "
        "('core_memory', %s, %s), ('sleep:last_event', %s, %s) "
        "ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value, "
        "updated_by_event = EXCLUDED.updated_by_event",
        (Jsonb({"text": "secret in core", "derived_from": [forgotten.id, kept.id]}),
         done_ev.id, Jsonb({"id": kept.id}), done_ev.id))

    # 1. Hard forget clears core_memory state + scrubs sleep.done copy; rebuild_state stays clean.
    await forget_hard(aconn, [forgotten.id], now=START + timedelta(minutes=3),
                      who="person:owner", place=PLACE)
    assert await read_state(aconn, "core_memory") is None
    done_row = await (await aconn.execute(
        "SELECT payload FROM events WHERE id = %s", (done_ev.id,))).fetchone()
    assert done_row == ({"forgotten": True},)

    await rebuild_state(aconn)
    assert await read_state(aconn, "core_memory") is None

    # 2. Next sleep rebuilds core_memory from an empty core prompt.
    llm = _SleepLLM()
    rebuilt_done = await memory.sleep(aconn, pfc=llm, embedder=FakeEmbedder(),
                                      now=START + timedelta(hours=2), settings=SETTINGS)
    assert rebuilt_done is not None and rebuilt_done.kind == "sleep.done"
    assert llm.seen_cores and "<core_memory></core_memory>" in llm.seen_cores[0]
    core_after = await read_state(aconn, "core_memory")
    assert core_after is not None and core_after["text"] == "rebuilt clean core"

    # 3. In-flight encode and embed_batch results never insert vectors for forgotten events.
    kernel = build(aconn, clock=FakeClock(START), settings=SETTINGS, embedder=FakeEmbedder())
    ev_encode = await _event(aconn, ts=START + timedelta(hours=3))
    ev_batch_forget = await _event(aconn, ts=START + timedelta(hours=3, minutes=1))
    ev_batch_keep = await _event(aconn, ts=START + timedelta(hours=3, minutes=2))
    from monoagens.core.jobs import Job
    # A job's result names its job by the start it was caused by, never by a payload key.
    started1 = await append_event(aconn, ts=START, kind="job.started",
                                  payload={"job": "encode"})
    started2 = await append_event(aconn, ts=START, kind="job.started",
                                  payload={"job": "embed_batch"})
    j1 = await append_event(aconn, ts=START, kind="job.result", caused_by=started1.id)
    j2 = await append_event(aconn, ts=START, kind="job.result", caused_by=started2.id)
    kernel.jobs.in_flight[f"encode:{ev_encode.id}"] = Job(
        name="encode", key=f"encode:{ev_encode.id}", trigger=ev_encode, place=PLACE,
        outcome={"result": {"event_id": ev_encode.id, "vector": [0.1] * 384}})
    kernel.jobs.in_flight[f"encode:{ev_encode.id}"].started_event_id = started1.id
    kernel.jobs.in_flight["embed_batch"] = Job(
        name="embed_batch", key="embed_batch", trigger=ev_batch_keep, place="",
        outcome={"result": {"items": [(ev_batch_forget.id, [0.2] * 384),
                                      (ev_batch_keep.id, [0.3] * 384)]}})
    kernel.jobs.in_flight["embed_batch"].started_event_id = started2.id
    await forget_hard(aconn, [ev_encode.id, ev_batch_forget.id],
                      now=START + timedelta(hours=3, minutes=5))
    await kernel._finish_job(j1)
    await kernel._finish_job(j2)

    rows = await (await aconn.execute("SELECT event_id FROM event_vectors ORDER BY event_id")).fetchall()
    stored_ids = [int(r[0]) for r in rows]
    assert ev_encode.id not in stored_ids
    assert ev_batch_forget.id not in stored_ids
    assert ev_batch_keep.id in stored_ids

