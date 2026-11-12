"""Tests for Step 2: The kernel loop.

One queue, one consumer, clock ticks not stored, last_seen projection,
and 100 events from two fake bodies recorded in order.
"""

from __future__ import annotations

import asyncio
from collections.abc import Awaitable, Callable
from datetime import datetime, timedelta, timezone

import pytest

from monoagens.infra.clock import FakeClock
from monoagens.core.kernel import Kernel
from monoagens.infra.store import read_state_entry
from monoagens.types import Action, Event
from monoagens.boot import build
from tests.support import settings


SETTINGS = settings()


class FakeBody:
    """Fake body implementing the two body ports: run(put) and act(action)."""

    def __init__(self, name: str, count: int, start_seq: int = 0) -> None:
        self.name = name
        self.count = count
        self.start_seq = start_seq
        self.actions: list[Action] = []

    async def run(self, put: Callable[[Event], Awaitable[None]]) -> None:
        base_time = datetime(2026, 11, 10, 10, 0, 0, tzinfo=timezone.utc)
        for i in range(self.count):
            seq = self.start_seq + i
            event = Event(
                id=None,
                ts=base_time + timedelta(seconds=seq),
                kind="percept",
                who=f"person:{self.name}",
                place="space:lobby",
                body=self.name,
                payload={"seq": seq, "body": self.name, "cursor": seq},
            )
            await put(event)
            # Yield control so bodies can interleave
            await asyncio.sleep(0)

    async def act(self, action: Action) -> None:
        self.actions.append(action)


async def test_fake_clock_advance_and_tick(aconn):
    start = datetime(2026, 11, 10, 8, 0, 0, tzinfo=timezone.utc)
    clock = FakeClock(initial_time=start, tick_seconds=30.0)
    assert clock.now() == start

    clock.advance(15)
    assert clock.now() == start + timedelta(seconds=15)

    kernel = build(aconn, clock=clock, settings=SETTINGS)
    tick_event = await clock.tick(kernel.put)
    assert tick_event.kind == "tick"
    assert tick_event.ts == clock.now()

    # Step the kernel on the tick: ticks are NOT stored
    processed = await kernel.step()
    assert processed.kind == "tick"
    assert processed.id is None

    cur = await aconn.execute("SELECT count(*) FROM events WHERE kind = 'tick'")
    count = (await cur.fetchone())[0]
    assert count == 0


async def test_one_hundred_events_from_two_fake_bodies(aconn):
    """PLAN.md Step 2 acceptance test:

    Done when: 100 events from two fake bodies are recorded in order;
    ticks are not stored.
    """
    clock = FakeClock(initial_time=datetime(2026, 11, 10, 10, 0, 0, tzinfo=timezone.utc))
    kernel = build(aconn, clock=clock, settings=SETTINGS)

    body_a = FakeBody(name="fake_a", count=50, start_seq=0)
    body_b = FakeBody(name="fake_b", count=50, start_seq=50)

    stop_event = asyncio.Event()

    # Producer task for body A and body B
    async def run_bodies() -> None:
        await asyncio.gather(
            body_a.run(kernel.put),
            body_b.run(kernel.put),
        )

    # Periodic ticks injected while bodies run
    async def run_ticks() -> None:
        while not stop_event.is_set():
            await clock.tick(kernel.put)
            await asyncio.sleep(0.005)

    # Consumer task running the kernel loop
    consumer_task = asyncio.create_task(kernel.run(stop_event))
    ticks_task = asyncio.create_task(run_ticks())

    # Run the bodies to completion
    await run_bodies()

    # Wait until all queued items are processed
    await kernel.queue.join()

    # Stop the background tick injector and kernel consumer
    stop_event.set()
    ticks_task.cancel()
    try:
        await ticks_task
    except asyncio.CancelledError:
        pass
    await consumer_task

    # Verify database contents
    cur = await aconn.execute(
        """
        SELECT id, kind, who, place, body, payload
        FROM events
        ORDER BY id ASC
        """
    )
    rows = await cur.fetchall()

    # Exactly 100 non-tick events recorded
    assert len(rows) == 100

    # Ticks are NOT stored
    cur_ticks = await aconn.execute("SELECT count(*) FROM events WHERE kind = 'tick'")
    ticks_count = (await cur_ticks.fetchone())[0]
    assert ticks_count == 0

    # All events are strictly ordered by id
    ids = [r[0] for r in rows]
    assert ids == sorted(ids)
    assert len(set(ids)) == 100

    # Events from both bodies are recorded
    bodies_seen = {r[4] for r in rows}
    assert bodies_seen == {"fake_a", "fake_b"}

    # Projections: last_seen for both persons exists and points to their latest processed event
    seen_a = await read_state_entry(aconn, "last_seen:person:fake_a")
    seen_b = await read_state_entry(aconn, "last_seen:person:fake_b")
    assert seen_a is not None
    assert seen_b is not None
    assert seen_a.value["body"] == "fake_a"
    assert seen_b.value["body"] == "fake_b"

    # Last seen points to the highest id recorded for that person
    max_id_a = max(r[0] for r in rows if r[2] == "person:fake_a")
    max_id_b = max(r[0] for r in rows if r[2] == "person:fake_b")
    assert seen_a.updated_by_event == max_id_a
    assert seen_b.updated_by_event == max_id_b


async def test_atomic_transaction_per_event(aconn):
    """Architect note: append_event and project_event in one transaction per event."""
    kernel = build(aconn, settings=SETTINGS)
    event = Event(
        id=None,
        ts=datetime(2026, 11, 10, 11, 0, 0, tzinfo=timezone.utc),
        kind="percept",
        who="person:atomic",
        place="space:test",
        body="tester",
    )
    # put() now waits for the intake acknowledgement, so a consumer must run
    # concurrently with it.
    put_task = asyncio.create_task(kernel.put(event))
    await asyncio.sleep(0)
    processed = await kernel.step()
    await asyncio.wait_for(put_task, 1.0)
    assert processed.id is not None

    # State was projected and committed together with the event
    entry = await read_state_entry(aconn, "last_seen:person:atomic")
    assert entry is not None
    assert entry.updated_by_event == processed.id


async def test_put_returns_only_after_event_is_committed(aconn):
    """Intake acknowledgement: put() returns after the event is in the Log.

    The event must already be queryable from `events` and projected into
    `state` when `await kernel.put(event)` returns.
    """
    clock = FakeClock(initial_time=datetime(2026, 11, 10, 12, 0, 0, tzinfo=timezone.utc))
    kernel = build(aconn, clock=clock, settings=SETTINGS)
    event = Event(
        id=None,
        ts=datetime(2020, 1, 1, tzinfo=timezone.utc),  # overridden by the kernel clock
        kind="percept",
        who="person:ack",
        place="space:ack",
        body="tester",
    )
    observed: list[object] = []

    async def source() -> None:
        await kernel.put(event)
        cur = await aconn.execute("SELECT id, ts, who FROM events ORDER BY id")
        observed.append(await cur.fetchall())
        observed.append(await read_state_entry(aconn, "last_seen:person:ack"))

    put_task = asyncio.create_task(source())
    await asyncio.sleep(0)
    assert not put_task.done(), "put() must not return before the event is processed"

    await kernel.step()
    await asyncio.wait_for(put_task, 1.0)

    rows, entry = observed
    assert len(rows) == 1
    event_id, stored_ts, who = rows[0]
    assert who == "person:ack"
    # Single time source: the kernel stamp wins over the source-supplied ts.
    assert stored_ts == clock.now()
    assert entry is not None
    assert entry.updated_by_event == event_id
