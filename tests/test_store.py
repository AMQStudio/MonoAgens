"""Tests for Step 1: store (The Log, cursor, read/write state, rebuild state)."""

from __future__ import annotations

from datetime import datetime, timezone

import pytest

from monoagens.store import (
    append_event,
    cursor,
    read_last_events,
    read_last_events_elsewhere,
    read_state,
    read_state_entry,
    rebuild_state,
    write_state,
)
from monoagens.types import Event


async def test_append_event_requires_ts_and_kind(aconn):
    with pytest.raises(ValueError, match="ts and kind must be explicitly provided"):
        await append_event(aconn, kind="msg.in", ts=None)

    with pytest.raises(ValueError, match="ts and kind must be explicitly provided"):
        await append_event(aconn, kind=None, ts=datetime.now(timezone.utc))


async def test_events_ordering(aconn):
    t0 = datetime(2026, 11, 10, 10, 0, 0, tzinfo=timezone.utc)
    t1 = datetime(2026, 11, 10, 10, 1, 0, tzinfo=timezone.utc)
    t2 = datetime(2026, 11, 10, 10, 2, 0, tzinfo=timezone.utc)

    e1 = await append_event(
        aconn,
        ts=t0,
        kind="msg.in",
        who="person:owner",
        place="space:alpha",
        payload={"msg": "one"},
    )
    e2 = await append_event(
        aconn,
        ts=t1,
        kind="msg.in",
        who="person:owner",
        place="space:alpha",
        payload={"msg": "two"},
    )
    e3 = await append_event(
        aconn,
        ts=t2,
        kind="msg.in",
        who="person:owner",
        place="space:alpha",
        payload={"msg": "three"},
    )

    # Monotonically increasing IDs and exact ts
    assert e1.id < e2.id < e3.id
    assert e1.ts == t0
    assert e2.ts == t1
    assert e3.ts == t2

    # Chronological last 2 events (oldest -> newest among the last 2)
    last_two = await read_last_events(aconn, "space:alpha", limit=2)
    assert [e.id for e in last_two] == [e2.id, e3.id]

    # Reverse-chronological when requested
    last_two_rev = await read_last_events(aconn, "space:alpha", limit=2, chronological=False)
    assert [e.id for e in last_two_rev] == [e3.id, e2.id]

    # Different place should not leak
    other = await read_last_events(aconn, "space:beta", limit=10)
    assert other == []


async def test_read_last_events_filters_by_kind(aconn):
    """A kinds filter keeps internal events from displacing conversational ones."""
    t = datetime(2026, 11, 10, 10, 0, 0, tzinfo=timezone.utc)
    conversational = await append_event(
        aconn, ts=t, kind="msg.in", who="person:owner", place="space:alpha",
        payload={"text": "first"},
    )
    for _ in range(25):
        await append_event(aconn, ts=t, kind="decision.silent", place="space:alpha")

    events = await read_last_events(
        aconn, "space:alpha", limit=5, kinds=("msg.in", "msg.out", "expectation.fired")
    )

    assert [event.id for event in events] == [conversational.id]


async def test_read_last_events_elsewhere(aconn):
    """Elsewhere events belong to one person, skip this place, and keep only recent kinds."""
    t = datetime(2026, 11, 10, 10, 0, 0, tzinfo=timezone.utc)
    here = await append_event(aconn, ts=t, kind="msg.in", who="person:owner",
                              place="space:here", payload={"text": "here"})
    first = await append_event(aconn, ts=t, kind="msg.in", who="person:owner",
                                place="space:other", payload={"text": "first"})
    await append_event(aconn, ts=t, kind="decision.silent", who="person:owner",
                       place="space:other", payload={"reason": "internal"})
    second = await append_event(aconn, ts=t, kind="msg.out", who="person:owner",
                                place="space:other", payload={"text": "second"})
    await append_event(aconn, ts=t, kind="msg.in", who="person:alice",
                       place="space:other", payload={"text": "stranger"})
    homeless = await append_event(aconn, ts=t, kind="msg.in", who="person:owner",
                                  place=None, payload={"text": "no place"})

    events = await read_last_events_elsewhere(aconn, "person:owner", "space:here")

    assert [event.id for event in events] == [first.id, second.id, homeless.id]
    assert here.id not in [event.id for event in events]
    assert events[0].payload == {"text": "first"}

    newest = await read_last_events_elsewhere(
        aconn, "person:owner", "space:here", limit=1, chronological=False
    )
    assert [event.id for event in newest] == [homeless.id]

    assert await read_last_events_elsewhere(aconn, "person:owner", "space:here",
                                            kinds=("msg.out",)) == [second]
    assert await read_last_events_elsewhere(aconn, "", "space:here") == []


async def test_cursor_tracking(aconn):
    t = datetime(2026, 11, 10, 12, 0, 0, tzinfo=timezone.utc)

    # Unknown body has no cursor
    assert await cursor(aconn, "body:nonexistent") is None

    # Append first event with cursor in payload
    await append_event(
        aconn,
        ts=t,
        kind="msg.in",
        body="body:stream",
        payload={"cursor": 101},
    )
    assert await cursor(aconn, "body:stream") == 101

    # Append subsequent event with advanced cursor
    await append_event(
        aconn,
        ts=t,
        kind="msg.in",
        body="body:stream",
        payload={"cursor": 102},
    )
    assert await cursor(aconn, "body:stream") == 102

    # A subsequent msg.out for the same body WITHOUT a cursor must NOT clobber the cursor
    await append_event(
        aconn,
        ts=t,
        kind="msg.out",
        body="body:stream",
        payload={"text": "outgoing response without cursor"},
    )
    assert await cursor(aconn, "body:stream") == 102


async def test_read_write_state(aconn):
    t = datetime(2026, 11, 10, 13, 0, 0, tzinfo=timezone.utc)
    ev = await append_event(aconn, ts=t, kind="decision.note")

    # Read nonexistent key
    assert await read_state(aconn, "nonexistent") is None
    assert await read_state_entry(aconn, "nonexistent") is None

    # Write and read
    await write_state(aconn, "foo:bar", {"count": 42}, ev.id)
    assert await read_state(aconn, "foo:bar") == {"count": 42}

    entry = await read_state_entry(aconn, "foo:bar")
    assert entry is not None
    assert entry.key == "foo:bar"
    assert entry.value == {"count": 42}
    assert entry.updated_by_event == ev.id

    # Upsert with new event
    ev2 = await append_event(aconn, ts=t, kind="decision.note")
    await write_state(aconn, "foo:bar", {"count": 43}, ev2.id)
    assert await read_state(aconn, "foo:bar") == {"count": 43}
    entry2 = await read_state_entry(aconn, "foo:bar")
    assert entry2.updated_by_event == ev2.id


async def test_rebuild_state_from_events(aconn):
    t1 = datetime(2026, 11, 10, 8, 0, 0, tzinfo=timezone.utc)
    t2 = datetime(2026, 11, 10, 8, 5, 0, tzinfo=timezone.utc)
    t3 = datetime(2026, 11, 10, 8, 10, 0, tzinfo=timezone.utc)
    t4 = datetime(2026, 11, 10, 8, 15, 0, tzinfo=timezone.utc)
    t5 = datetime(2026, 11, 10, 8, 20, 0, tzinfo=timezone.utc)

    e1 = await append_event(
        aconn,
        ts=t1,
        kind="msg.in",
        who="person:owner",
        place="space:room1",
        body="body:one",
        payload={"text": "hello"},
    )
    e2 = await append_event(
        aconn,
        ts=t2,
        kind="msg.out",
        place="space:room1",
        payload={"text": "hello back"},
    )
    e3 = await append_event(
        aconn,
        ts=t3,
        kind="msg.in",
        who="person:alice",
        place="space:room2",
        body="body:two",
        payload={"text": "hi"},
    )
    e4 = await append_event(
        aconn,
        ts=t4,
        kind="msg.in",
        who="person:owner",
        place="space:room2",
        body="body:one",
        payload={"text": "joined room2"},
    )
    e5 = await append_event(
        aconn,
        ts=t5,
        kind="sleep.done",
        payload={"core_memory": "owner moved to room2", "last_event_id": e4.id},
    )

    # Build the state projection from the log
    await rebuild_state(aconn)

    # Verify last_seen for owner reflects the latest event (e4)
    owner_seen = await read_state_entry(aconn, "last_seen:person:owner")
    assert owner_seen is not None
    assert owner_seen.value["place"] == "space:room2"
    assert owner_seen.value["body"] == "body:one"
    assert owner_seen.updated_by_event == e4.id

    # Verify last_seen for alice reflects e3
    alice_seen = await read_state_entry(aconn, "last_seen:person:alice")
    assert alice_seen is not None
    assert alice_seen.value["place"] == "space:room2"
    assert alice_seen.updated_by_event == e3.id

    # Verify last_spoke for room1 reflects e2
    room1_spoke = await read_state_entry(aconn, "last_spoke:space:room1")
    assert room1_spoke is not None
    assert room1_spoke.updated_by_event == e2.id

    # Verify sleep projections
    core_mem = await read_state_entry(aconn, "core_memory")
    assert core_mem is not None
    assert core_mem.value["text"] == "owner moved to room2"

    sleep_ev = await read_state_entry(aconn, "sleep:last_event")
    assert sleep_ev is not None
    assert sleep_ev.value["id"] == e4.id

    # Drop state completely and rebuild again
    await aconn.execute("DELETE FROM state")
    assert await read_state(aconn, "last_seen:person:owner") is None

    await rebuild_state(aconn)
    restored = await read_state_entry(aconn, "last_seen:person:owner")
    assert restored is not None
    assert restored.value["place"] == "space:room2"
    assert restored.updated_by_event == e4.id
