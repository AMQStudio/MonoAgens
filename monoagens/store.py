"""The Log and Status Board persistence (events, state, expectations)."""

from __future__ import annotations

from collections.abc import Sequence
from datetime import datetime
from typing import Any
from zoneinfo import ZoneInfo

from pgvector import Vector
from psycopg.types.json import Jsonb

from monoagens.types import RECENT_KINDS, Event, StateEntry

EXPECTATION_COLUMNS = (
    "id", "kind", "who", "place", "about", "cue_time", "cue_event", "condition",
    "condition_text", "intent", "weight", "policy", "status", "next_check_at",
    "last_fired_at", "fire_count", "source_event", "derived_from", "origin_actor",
    "expires_at", "source_body",
)
_EXPECTATIONS = ("SELECT " + ", ".join("ex." + column for column in EXPECTATION_COLUMNS[:-1])
                 + ", s.body AS source_body FROM expectations ex"
                 " LEFT JOIN events s ON s.id = ex.source_event")
_INSERT_EXPECTATION = (
    "INSERT INTO expectations (kind, who, place, about, cue_time, cue_event, condition, intent, "
    "weight, policy, status, next_check_at, source_event, derived_from, origin_actor, "
    "expires_at) VALUES "
    "(%s, %s, %s, %s, %s, %s, %s, %s, %s, %s, 'active', %s, %s, %s, %s, %s) RETURNING id"
)
_EVENT_COLS = "id, ts, kind, who, place, body, payload, caused_by, origin_actor"


def _row_to_event(row):
    payload = row[6] if row[6] is not None else {}
    return Event(row[0], row[1], row[2], row[3], row[4], row[5], payload, row[7], row[8])


async def append_event(conn, *, ts, kind, who=None, place=None, body=None, payload=None,
                       caused_by=None, origin_actor=None) -> Event:
    """Append one event to the immutable Log; origin inherits along caused_by."""
    if ts is None or kind is None:
        raise ValueError("ts and kind must be explicitly provided")
    query = ("INSERT INTO events (ts, kind, who, place, body, payload, caused_by, origin_actor) "
             "VALUES (%s, %s, %s, %s, %s, %s, %s, COALESCE(%s, "
             "(SELECT origin_actor FROM events WHERE id = %s), %s)) "
             f"RETURNING {_EVENT_COLS}")
    cur = await conn.execute(query, (ts, kind, who, place, body, Jsonb(payload or {}), caused_by,
                                    origin_actor, caused_by, who))
    return _row_to_event(await cur.fetchone())


async def insert_event_vector(conn, event_id: int, embedding: Sequence[float],
                              derived_from: Sequence[int] | None = None) -> None:
    """Write the one embedding of an event; never overwrite an existing one."""
    lineage = list(derived_from) if derived_from is not None else [event_id]
    await conn.execute(
        "INSERT INTO event_vectors (event_id, embedding, derived_from) VALUES (%s, %s, %s) "
        "ON CONFLICT (event_id) DO NOTHING",
        (event_id, Vector(list(embedding)), lineage),
    )


async def read_event_vector(conn, event_id: int) -> list[float] | None:
    """Return the embedding already stored for an event, if any."""
    cur = await conn.execute("SELECT embedding FROM event_vectors WHERE event_id = %s",
                            (event_id,))
    row = await cur.fetchone()
    if row is None:
        return None
    vector = row[0]
    return [float(x) for x in (vector.to_list() if hasattr(vector, "to_list") else vector)]


async def _recent(conn, where: str, params: list[Any], limit: int,
                  chronological: bool) -> list[Event]:
    query = f"SELECT {_EVENT_COLS} FROM events WHERE {where} ORDER BY id DESC LIMIT %s"
    cur = await conn.execute(query, [*params, limit])
    events = [_row_to_event(row) for row in await cur.fetchall()]
    return list(reversed(events)) if chronological else events


async def read_last_events(conn, place, limit=20, *, chronological=True,
                           kinds: Sequence[str] | None = None) -> list[Event]:
    """Return the last events for a place, optionally restricted to some kinds."""
    where, params = "place = %s", [place]
    if kinds is not None:
        where, params = f"{where} AND kind = ANY(%s)", [place, list(kinds)]
    return await _recent(conn, where, params, limit, chronological)


async def read_last_events_elsewhere(conn, who, exclude_place, limit=10,
                                     kinds: Sequence[str] | None = RECENT_KINDS,
                                     chronological=True) -> list[Event]:
    """Return one person's last events in places other than exclude_place."""
    if not who:
        return []
    where, params = "who = %s AND (place IS NULL OR place != %s)", [who, exclude_place]
    if kinds is not None:
        where, params = f"{where} AND kind = ANY(%s)", [who, exclude_place, list(kinds)]
    return await _recent(conn, where, params, limit, chronological)


async def count_recent_events(conn, place, *, kind, since) -> int:
    """Count events of one kind in a place since a timestamp."""
    cur = await conn.execute(
        "SELECT count(*) FROM events WHERE place = %s AND kind = %s AND ts >= %s",
        (place, kind, since),
    )
    row = await cur.fetchone()
    return int(row[0]) if row else 0


async def cursor(conn, body) -> Any | None:
    """Return the last cursor payload recorded by a body, if any."""
    cur = await conn.execute("SELECT payload->'cursor' FROM events WHERE body = %s "
                             "AND payload ? 'cursor' ORDER BY id DESC LIMIT 1", (body,))
    row = await cur.fetchone()
    return row[0] if row else None


async def read_state(conn, key) -> dict[str, Any] | None:
    """Return the JSON value of a state key, if present."""
    cur = await conn.execute("SELECT value FROM state WHERE key = %s", (key,))
    row = await cur.fetchone()
    return row[0] if row else None


async def read_state_entry(conn, key) -> StateEntry | None:
    """Return a state row together with the event that wrote it."""
    cur = await conn.execute(
        "SELECT key, value, updated_by_event FROM state WHERE key = %s", (key,)
    )
    row = await cur.fetchone()
    return StateEntry(row[0], row[1], row[2]) if row else None


async def write_state(conn, key, value, updated_by_event) -> None:
    """Upsert one state key with the event that produced it."""
    query = ("INSERT INTO state (key, value, updated_by_event) VALUES (%s, %s, %s) "
             "ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value, "
             "updated_by_event = EXCLUDED.updated_by_event")
    await conn.execute(query, (key, Jsonb(value), updated_by_event))


async def add_budget_usage(conn, *, now: datetime, timezone: str, usage: dict[str, int],
                           updated_by_event: int) -> dict[str, int]:
    """Add one LLM call's token usage to the budget key of its local date; return totals."""
    key = f"budget:{now.astimezone(ZoneInfo(timezone)).date().isoformat()}"
    current = await read_state(conn, key) or {}
    totals = {name: int(current.get(name) or 0) + int(usage.get(name) or 0)
              for name in ("prompt_tokens", "cached_tokens", "completion_tokens",
                           "total_tokens")}
    await write_state(conn, key, totals, updated_by_event)
    return totals


async def project_event(conn, event: Event) -> None:
    """Project one event onto the Status Board."""
    if event.id is None:
        raise ValueError("Cannot project an event without an id")
    if event.who is not None:
        value = {"ts": event.ts.isoformat(), "place": event.place, "body": event.body}
        await write_state(conn, f"last_seen:{event.who}", value, event.id)
    if event.kind == "act.out" and event.place is not None:
        await write_state(conn, f"last_spoke:{event.place}", {"ts": event.ts.isoformat()}, event.id)
    if event.kind == "sleep.done":
        if "core_memory" in event.payload:
            core = {"text": event.payload["core_memory"]}
            await write_state(conn, "core_memory", core, event.id)
        last = {"id": event.payload.get("last_event_id", event.id)}
        await write_state(conn, "sleep:last_event", last, event.id)


async def rebuild_state(conn) -> None:
    """Drop state and replay the whole Log into it."""
    await conn.execute("DELETE FROM state")
    cur = await conn.execute(f"SELECT {_EVENT_COLS} FROM events ORDER BY id ASC")
    for row in await cur.fetchall():
        await project_event(conn, _row_to_event(row))


async def read_expectations(conn, where="TRUE", params=()) -> list[dict[str, Any]]:
    """Read expectation rows joined with their source body, filtered by where."""
    cur = await conn.execute(f"{_EXPECTATIONS} WHERE {where} ORDER BY ex.id", params)
    return [dict(zip(EXPECTATION_COLUMNS, row)) for row in await cur.fetchall()]


async def insert_expectation(conn, values) -> int:
    """Insert one expectation row and return its id."""
    cur = await conn.execute(_INSERT_EXPECTATION, values)
    return int((await cur.fetchone())[0])


async def resolve_expectation(conn, row, now: datetime, status: str) -> Event:
    """Resolve one expectation and record the event, in one transaction."""
    async with conn.transaction():
        await conn.execute(
            "UPDATE expectations SET status = %s, next_check_at = NULL WHERE id = %s",
            (status, row["id"]),
        )
        payload = {"expectation_id": row["id"], "about": row["about"], "status": status}
        return await append_event(conn, ts=now, kind="expectation.resolved", who=row["who"],
                                  place=row["place"], body=row.get("source_body"),
                                  caused_by=row.get("fired_event"), payload=payload)


async def fire_expectation(conn, row, now: datetime) -> dict[str, Any]:
    """Mark one expectation fired, record it, and return it with the trigger."""
    async with conn.transaction():
        payload = {"expectation_id": row["id"], "about": row["about"],
                   "intent": row["intent"], "weight": row["weight"]}
        fired = await append_event(conn, ts=now, kind="expectation.fired", who=row["who"],
                                   place=row["place"], body=row["source_body"],
                                   caused_by=row["source_event"], payload=payload)
        await conn.execute("UPDATE expectations SET status = 'fired', "
                           "fire_count = fire_count + 1, last_fired_at = %s WHERE id = %s",
                           (now, row["id"]))
    return {**row, "fired_event": fired.id, "trigger": fired}


async def active_expectation_lines(conn, place) -> list[str]:
    """Render the active expectations for a place as prompt lines."""
    cur = await conn.execute("SELECT id, about, intent, next_check_at FROM expectations "
                             "WHERE status = 'active' AND (place = %s OR place IS NULL) "
                             "ORDER BY id LIMIT 20", (place,))
    return [f"#{i} {about} ({intent or 'no intent stated'}"
            f"{f', next check {at:%Y-%m-%d %H:%M}' if at else ''})"
            for i, about, intent, at in await cur.fetchall()]
