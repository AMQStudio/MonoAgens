"""The Log and Status Board persistence (events, state, expectations)."""
from __future__ import annotations
from datetime import datetime
from typing import Any
from psycopg.types.json import Jsonb
from monoagens.types import Event, StateEntry

EXPECTATION_COLUMNS = ("id", "who", "place", "about", "cue_time", "cue_event", "expect", "intent", "weight",
                       "policy", "status", "next_check_at", "last_fired_at", "fire_count", "source_event",
                       "expires_at", "source_body")
_EXPECTATIONS = ("SELECT " + ", ".join("ex." + c for c in EXPECTATION_COLUMNS[:-1]) +
                 ", s.body AS source_body FROM expectations ex LEFT JOIN events s ON s.id = ex.source_event")
_INSERT_EXPECTATION = ("INSERT INTO expectations (who, place, about, cue_time, cue_event, expect, intent, weight, "
                       "policy, status, next_check_at, source_event, expires_at) VALUES (%s, %s, %s, %s, %s, %s, "
                       "%s, %s, %s, 'active', %s, %s, %s) RETURNING id")
_EVENT_COLS = "id, ts, kind, who, place, body, payload, caused_by"

def _row_to_event(r):
    return Event(r[0], r[1], r[2], r[3], r[4], r[5], r[6] if r[6] is not None else {}, r[7])

async def append_event(conn, event=None, *, ts=None, kind=None, who=None, place=None, body=None,
                       payload=None, caused_by=None) -> Event:
    """Append one event to the immutable Log."""
    if event is not None:
        ts, kind, who, place, body = event.ts, event.kind, event.who, event.place, event.body
        payload, caused_by = event.payload, event.caused_by
    if ts is None or kind is None:
        raise ValueError("ts and kind must be explicitly provided")
    cur = await conn.execute("INSERT INTO events (ts, kind, who, place, body, payload, caused_by) VALUES "
                             f"(%s, %s, %s, %s, %s, %s, %s) RETURNING {_EVENT_COLS}",
                             (ts, kind, who, place, body, Jsonb(payload or {}), caused_by))
    row = await cur.fetchone()
    if row is None:
        raise RuntimeError("Failed to insert event")
    return _row_to_event(row)

async def read_last_events(conn, place, limit=20, *, chronological=True) -> list[Event]:
    cur = await conn.execute(f"SELECT {_EVENT_COLS} FROM events WHERE place = %s ORDER BY id DESC LIMIT %s",
                             (place, limit))
    events = [_row_to_event(r) for r in await cur.fetchall()]
    if chronological:
        events.reverse()
    return events

async def count_recent_events(conn, place, *, kind, since) -> int:
    row = await (await conn.execute("SELECT count(*) FROM events WHERE place = %s AND kind = %s AND ts >= %s",
                                    (place, kind, since))).fetchone()
    return int(row[0]) if row else 0

async def cursor(conn, body) -> Any | None:
    row = await (await conn.execute("SELECT payload->'cursor' FROM events WHERE body = %s AND payload ? 'cursor' "
                                    "ORDER BY id DESC LIMIT 1", (body,))).fetchone()
    return row[0] if row else None

async def read_state(conn, key) -> dict[str, Any] | None:
    row = await (await conn.execute("SELECT value FROM state WHERE key = %s", (key,))).fetchone()
    return row[0] if row else None

async def read_state_entry(conn, key) -> StateEntry | None:
    row = await (await conn.execute("SELECT key, value, updated_by_event FROM state WHERE key = %s",
                                    (key,))).fetchone()
    return StateEntry(row[0], row[1], row[2]) if row else None

async def write_state(conn, key, value, updated_by_event) -> None:
    await conn.execute("INSERT INTO state (key, value, updated_by_event) VALUES (%s, %s, %s) ON CONFLICT (key) DO "
                       "UPDATE SET value = EXCLUDED.value, updated_by_event = EXCLUDED.updated_by_event",
                       (key, Jsonb(value), updated_by_event))

async def project_event(conn, event: Event) -> None:
    """Project one event onto the Status Board."""
    if event.id is None:
        raise ValueError("Cannot project an event without an id")
    if event.who is not None:
        await write_state(conn, f"last_seen:{event.who}",
                          {"ts": event.ts.isoformat(), "place": event.place, "body": event.body}, event.id)
    if event.kind == "msg.out" and event.place is not None:
        await write_state(conn, f"last_spoke:{event.place}", {"ts": event.ts.isoformat()}, event.id)
    if event.kind == "sleep.done":
        if "core_memory" in event.payload:
            await write_state(conn, "core_memory", {"text": event.payload["core_memory"]}, event.id)
        await write_state(conn, "sleep:last_event", {"id": event.payload.get("last_event_id", event.id)}, event.id)

async def rebuild_state(conn) -> None:
    await conn.execute("DELETE FROM state")
    cur = await conn.execute(f"SELECT {_EVENT_COLS} FROM events ORDER BY id ASC")
    for row in await cur.fetchall():
        await project_event(conn, _row_to_event(row))

async def read_expectations(conn, where="TRUE", params=()) -> list[dict[str, Any]]:
    cur = await conn.execute(f"{_EXPECTATIONS} WHERE {where} ORDER BY ex.id", params)
    return [dict(zip(EXPECTATION_COLUMNS, row)) for row in await cur.fetchall()]

async def insert_expectation(conn, values) -> int:
    row = await (await conn.execute(_INSERT_EXPECTATION, values)).fetchone()
    if row is None:
        raise RuntimeError("Failed to insert expectation")
    return int(row[0])

async def resolve_expectation(conn, row, now: datetime, status: str) -> Event:
    async with conn.transaction():
        await conn.execute("UPDATE expectations SET status = %s, next_check_at = NULL WHERE id = %s",
                           (status, row["id"]))
        return await append_event(conn, ts=now, kind="expectation.resolved", who=row["who"], place=row["place"],
                                  body=row.get("source_body"), caused_by=row.get("fired_event"),
                                  payload={"expectation_id": row["id"], "about": row["about"], "status": status})

async def fire_expectation(conn, row, now: datetime) -> dict[str, Any]:
    async with conn.transaction():
        fired = await append_event(conn, ts=now, kind="expectation.fired", who=row["who"], place=row["place"],
                                   body=row["source_body"], caused_by=row["source_event"],
                                   payload={"expectation_id": row["id"], "about": row["about"],
                                            "intent": row["intent"], "weight": row["weight"]})
        await conn.execute("UPDATE expectations SET status = 'fired', fire_count = fire_count + 1, "
                           "last_fired_at = %s WHERE id = %s", (now, row["id"]))
    return {**row, "fired_event": fired.id, "trigger": fired}

async def active_expectation_lines(conn, place) -> list[str]:
    cur = await conn.execute("SELECT id, about, intent, next_check_at FROM expectations WHERE status = 'active' "
                             "AND (place = %s OR place IS NULL) ORDER BY id LIMIT 20", (place,))
    return [f"#{i} {about} ({intent or 'no intent stated'}{f', next check {at:%Y-%m-%d %H:%M}' if at else ''})"
            for i, about, intent, at in await cur.fetchall()]
