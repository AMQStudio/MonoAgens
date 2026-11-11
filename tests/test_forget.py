"""Tests for lineage (`derived_from`) and the two dedicated vector-deletion paths."""

from __future__ import annotations

from datetime import datetime, timedelta, timezone

import pytest
from psycopg.types.json import Jsonb

from monoagens import expect
from monoagens.forget import fade, forget_hard
from monoagens.store import append_event, insert_event_vector

PLACE = "space:tg:1"
START = datetime(2026, 11, 10, 8, 0, tzinfo=timezone.utc)
SOUL = {"timezone": "Asia/Jakarta",
        "expectations": {"default_expires_days": 30, "max_active": 200}}


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

    await expect.create(aconn, {"about": "coffee", "cue_time": {"every": "1d"},
                                "expect": {"none": True}}, now=START, soul=SOUL, source=source)
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
