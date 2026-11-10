"""The Log and Status Board persistence (events and state)."""

from __future__ import annotations

from datetime import datetime
from typing import Any

from psycopg.types.json import Jsonb
import psycopg

from monoagens.types import Event, StateEntry


def _row_to_event(r: tuple[Any, ...]) -> Event:
    return Event(r[0], r[1], r[2], r[3], r[4], r[5], r[6] if r[6] is not None else {}, r[7])


async def append_event(
    conn: psycopg.AsyncConnection[Any],
    event: Event | None = None,
    *,
    ts: datetime | None = None,
    kind: str | None = None,
    who: str | None = None,
    place: str | None = None,
    body: str | None = None,
    payload: dict[str, Any] | None = None,
    caused_by: int | None = None,
) -> Event:
    """Append one event to the immutable Log."""
    if event is not None:
        ts, kind, who = event.ts, event.kind, event.who
        place, body, payload, caused_by = event.place, event.body, event.payload, event.caused_by
    if ts is None or kind is None:
        raise ValueError("ts and kind must be explicitly provided")

    cur = await conn.execute(
        "INSERT INTO events (ts, kind, who, place, body, payload, caused_by) "
        "VALUES (%s, %s, %s, %s, %s, %s, %s) "
        "RETURNING id, ts, kind, who, place, body, payload, caused_by",
        (ts, kind, who, place, body, Jsonb(payload or {}), caused_by),
    )
    row = await cur.fetchone()
    if row is None:
        raise RuntimeError("Failed to insert event")
    return _row_to_event(row)


async def read_last_events(
    conn: psycopg.AsyncConnection[Any],
    place: str,
    limit: int = 20,
    *,
    chronological: bool = True,
) -> list[Event]:
    """Read the last N events for a place, chronological by default."""
    cur = await conn.execute(
        "SELECT id, ts, kind, who, place, body, payload, caused_by "
        "FROM events WHERE place = %s ORDER BY id DESC LIMIT %s",
        (place, limit),
    )
    rows = await cur.fetchall()
    events = [_row_to_event(r) for r in rows]
    if chronological:
        events.reverse()
    return events


async def cursor(
    conn: psycopg.AsyncConnection[Any],
    body: str,
) -> Any | None:
    """Read the last recorded cursor for an input body from events."""
    cur = await conn.execute(
        "SELECT payload->'cursor' FROM events "
        "WHERE body = %s AND payload ? 'cursor' "
        "ORDER BY id DESC LIMIT 1",
        (body,),
    )
    row = await cur.fetchone()
    return row[0] if row else None


async def read_state(conn: psycopg.AsyncConnection[Any], key: str) -> dict[str, Any] | None:
    """Read the value for a state key."""
    cur = await conn.execute("SELECT value FROM state WHERE key = %s", (key,))
    row = await cur.fetchone()
    return row[0] if row else None


async def read_state_entry(conn: psycopg.AsyncConnection[Any], key: str) -> StateEntry | None:
    """Read a full state entry (key, value, updated_by_event)."""
    cur = await conn.execute("SELECT key, value, updated_by_event FROM state WHERE key = %s", (key,))
    row = await cur.fetchone()
    return StateEntry(row[0], row[1], row[2]) if row else None


async def write_state(
    conn: psycopg.AsyncConnection[Any],
    key: str,
    value: dict[str, Any],
    updated_by_event: int,
) -> None:
    """Upsert one key in the state projection."""
    await conn.execute(
        "INSERT INTO state (key, value, updated_by_event) VALUES (%s, %s, %s) "
        "ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value, updated_by_event = EXCLUDED.updated_by_event",
        (key, Jsonb(value), updated_by_event),
    )


async def project_event(conn: psycopg.AsyncConnection[Any], event: Event) -> None:
    """Project one event onto the Status Board."""
    if event.id is None:
        raise ValueError("Cannot project an event without an id")
    if event.who is not None:
        await write_state(
            conn, f"last_seen:{event.who}",
            {"ts": event.ts.isoformat(), "place": event.place, "body": event.body},
            event.id,
        )
    if event.kind == "msg.out" and event.place is not None:
        await write_state(conn, f"last_spoke:{event.place}", {"ts": event.ts.isoformat()}, event.id)
    if event.kind == "sleep.done":
        if "core_memory" in event.payload:
            await write_state(conn, "core_memory", {"text": event.payload["core_memory"]}, event.id)
        last_id = event.payload.get("last_event_id", event.id)
        await write_state(conn, "sleep:last_event", {"id": last_id}, event.id)


async def rebuild_state(conn: psycopg.AsyncConnection[Any]) -> None:
    """Rebuild the entire state projection from the immutable Log."""
    await conn.execute("DELETE FROM state")
    cur = await conn.execute(
        "SELECT id, ts, kind, who, place, body, payload, caused_by FROM events ORDER BY id ASC"
    )
    for row in await cur.fetchall():
        await project_event(conn, _row_to_event(row))
