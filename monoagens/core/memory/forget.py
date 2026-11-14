"""Forgetting: the only paths by which vectors leave the Log's index."""

from __future__ import annotations

from collections.abc import Sequence
from datetime import datetime

import psycopg
from psycopg.types.json import Jsonb

from monoagens.infra.store import append_event
from monoagens.types import Event


async def fade(conn: psycopg.AsyncConnection, *, now: datetime, before: datetime,
               limit: int = 100) -> Event | None:
    """Release old vectors that no memory or expectation derives from; record fade.done."""
    async with conn.transaction():
        await conn.execute("SELECT set_config('monoagens.allowed_vector_delete', 'on', true)")
        cur = await conn.execute(
            "DELETE FROM event_vectors v WHERE v.event_id IN ("
            " SELECT v2.event_id FROM event_vectors v2 JOIN events e ON e.id = v2.event_id"
            " WHERE e.ts < %s"
            " AND NOT EXISTS (SELECT 1 FROM memories m WHERE e.id = ANY(m.derived_from))"
            " AND NOT EXISTS (SELECT 1 FROM expectations x WHERE e.id = ANY(x.derived_from))"
            " ORDER BY v2.event_id LIMIT %s) RETURNING v.event_id",
            (before, limit))
        released = [row[0] for row in await cur.fetchall()]
        if not released:
            return None
        return await append_event(conn, ts=now, kind="fade.done", payload={
            "released": len(released), "before": before.isoformat()})


async def forget_hard(conn: psycopg.AsyncConnection, event_ids: Sequence[int], *,
                      now: datetime, who: str | None = None,
                      place: str | None = None) -> Event:
    """Scrub an event's content and delete the lineage derived from it.

    The payload becomes ``{"forgotten": true}``, the description and the features
    are wiped: those are the three fields that hold what was forgotten.
    """
    ids = sorted({int(event_id) for event_id in event_ids})
    if not ids:
        raise ValueError("forget_hard needs at least one event id")
    cur = await conn.execute("SELECT id FROM events WHERE id = ANY(%s)", (ids,))
    if len(await cur.fetchall()) != len(ids):
        raise ValueError(f"unknown event ids: {ids}")
    async with conn.transaction():
        await conn.execute("SELECT set_config('monoagens.forgetting', 'on', true)")
        await conn.execute("SELECT set_config('monoagens.allowed_vector_delete', 'on', true)")
        await conn.execute("DELETE FROM event_vectors WHERE event_id = ANY(%s) "
                           "OR derived_from && %s::bigint[]", (ids, ids))
        await conn.execute("DELETE FROM memories WHERE derived_from && %s::bigint[]", (ids,))
        await conn.execute("DELETE FROM expectations WHERE derived_from && %s::bigint[]", (ids,))
        core_row = await (await conn.execute(
            "SELECT value FROM state WHERE key = 'core_memory'")).fetchone()
        core_sources = (
            {int(src) for src in (core_row[0].get("derived_from") or [])}
            if core_row and isinstance(core_row[0], dict) else set()
        )
        if core_sources & set(ids):
            await conn.execute("DELETE FROM state WHERE key = 'core_memory'")
        await conn.execute(
            "UPDATE events SET payload = %s, description = '', features = '{}'::jsonb "
            "WHERE id = ANY(%s) OR (kind = 'sleep.done' AND EXISTS ("
            "SELECT 1 FROM jsonb_array_elements_text("
            "COALESCE(payload->'derived_from', '[]'::jsonb)) elem "
            "WHERE elem::bigint = ANY(%s)))",
            (Jsonb({"forgotten": True}), ids, ids))
        return await append_event(conn, ts=now, kind="forget.hard", who=who, place=place,
                                  payload={"forgotten_events": len(ids)})
