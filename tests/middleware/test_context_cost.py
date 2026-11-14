"""The context hook stays inside its watchdog on a long desk (1.0c).

`privacy.items_rule` is the expensive part of the context hook: it checks one line
and one memory at a time against the audience of the place. This test builds a desk as
large as the biggest replay scenario (500 events, 200 memories, 50 places) and measures
the rule; the number is recorded under the task's Notes.
"""

from __future__ import annotations

import time
from datetime import datetime, timezone

from monoagens.core import memory
from monoagens.core.context import layout
from monoagens.core.hooks import Replace
from monoagens.infra.store import append_event
from monoagens.middleware import privacy
from tests.support import settings

START = datetime(2026, 1, 1, tzinfo=timezone.utc)
SETTINGS = settings()
PLACES = 50
EVENTS = 500
MEMORIES = 200
DESK = "space:tg:-500"
HOOK_TIMEOUT_SECONDS = 0.5


async def _world(conn) -> None:
    """Fill the log with events in PLACES shared places and MEMORIES owner memories."""
    await conn.execute(
        "INSERT INTO events (ts, kind, who, place, body, payload) "
        "SELECT %s, 'percept', 'person:tg:' || (i %% 97), 'space:tg:' || (10000 + i %% %s), "
        "'telegram', jsonb_build_object('description', 'line ' || i, "
        "'place_kind', 'shared') FROM generate_series(0, %s) AS i",
        (START, PLACES, EVENTS - 1))
    await conn.execute(
        "INSERT INTO events (ts, kind, payload) "
        "SELECT %s, 'memory.stored', jsonb_build_object('text', 'memory ' || i) "
        "FROM generate_series(0, %s) AS i", (START, MEMORIES - 1))


async def test_the_items_check_fits_the_hook_watchdog(aconn):
    await _world(aconn)
    await append_event(aconn, ts=START, kind="place.audience", place=DESK,
                       payload={"place": DESK, "actors": ["person:owner", "person:tg:7"]})
    await append_event(aconn, ts=START, kind="place.audience", place="space:tg:10001",
                       payload={"place": "space:tg:10001", "actors": ["person:owner"]})
    trigger = await append_event(aconn, ts=START, kind="percept", who="person:tg:1",
                                 place=DESK, body="telegram",
                                 payload={"description": "halo"})
    private = "space:tg:dm:owner"
    await append_event(aconn, ts=START, kind="place.audience", place=private,
                       payload={"place": private, "actors": ["person:owner"]})
    source = await append_event(aconn, ts=START, kind="percept", who="person:owner",
                                place=private, body="telegram", description="rahasia")
    hits = [memory.MemoryHit(f"memory {index} (person:owner private chat)", place=private,
                             sources=(source.id,)) for index in range(5)]
    context = await layout.build_context(aconn, trigger, DESK, "test", now=START,
                                         settings=SETTINGS, memories=hits, capabilities=())
    assert len([item for item in context.items
            if item.section in ("recent", "elsewhere")]) >= 10

    started = time.perf_counter()
    outcome = await privacy.items_rule()(context, {"conn": aconn, "now": START})
    elapsed = time.perf_counter() - started

    filtered = outcome.item if isinstance(outcome, Replace) else context
    assert filtered.place == DESK
    assert [item for item in filtered.items if item.section == "notes"]  # withheld
    print(f"\nthe items check on {EVENTS} events, {MEMORIES} memories, {PLACES} places: "
          f"{elapsed * 1000:.1f} ms")
    assert elapsed < 0.5 * HOOK_TIMEOUT_SECONDS
