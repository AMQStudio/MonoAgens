"""The context hook stays inside its watchdog on a long desk (1.0c).

`privacy.filter_context` is the expensive part of the context hook: it checks one line
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


async def test_filter_context_fits_the_hook_watchdog(aconn):
    await _world(aconn)
    trigger = await append_event(aconn, ts=START, kind="percept", who="person:tg:1",
                                 place=DESK, body="telegram",
                                 payload={"description": "halo", "place_kind": "shared"})
    hits = [memory.MemoryHit(f"memory {index} (person:owner private chat)", place=None,
                             sources=()) for index in range(5)]
    context = await layout.build_context(aconn, trigger, DESK, "test", now=START,
                                         settings=SETTINGS, memories=hits, capabilities=())
    assert len(context.recent) + len(context.recent_elsewhere) >= 10

    started = time.perf_counter()
    outcome = await privacy.filter_context(aconn)(context, {"conn": aconn, "now": START})
    elapsed = time.perf_counter() - started

    filtered = outcome.item if isinstance(outcome, Replace) else context
    assert filtered.place_kind == "shared"
    assert filtered.withheld  # the owner-only memories came from a private chat
    print(f"\nfilter_context on {EVENTS} events, {MEMORIES} memories, {PLACES} places: "
          f"{elapsed * 1000:.1f} ms")
    assert elapsed < 0.5 * HOOK_TIMEOUT_SECONDS
