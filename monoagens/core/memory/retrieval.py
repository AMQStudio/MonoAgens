"""Memory retrieval: embeddings, ACT-R activation and the candidates of one wake."""

from __future__ import annotations

import math
from collections.abc import Sequence
from datetime import datetime
from typing import Any

import psycopg
from pgvector import Vector

from monoagens.infra.prior import constants, retrieval
from monoagens.infra.store import event_from_row, payload_text, read_event_vector
from monoagens.types import RECENT_KINDS, Event

_ROWS = ("id, text, who, place, importance, last_accessed_at, private, use_count, created_at, "
         "derived_from, coalesce(1 - (embedding <=> %s), 0) AS similarity")
_VECTOR_SQL = (f"SELECT {_ROWS} FROM memories WHERE valid_to IS NULL AND embedding IS NOT NULL "
               "ORDER BY embedding <=> %s LIMIT 40")
_TEXT_SQL = (f"SELECT {_ROWS} FROM memories WHERE valid_to IS NULL AND "
             "(tsv @@ plainto_tsquery('simple', %s) OR word_similarity(%s, text) > 0.2) "
             "ORDER BY word_similarity(%s, text) DESC LIMIT 20")


def decay_d(settings: dict[str, Any]) -> float:
    """Return ACT-R's d from memory_persistence (0 -> 0.7, 1 -> 0.3)."""
    low, high = constants(settings)["persistence_decay_d"]
    return float(low) + (float(high) - float(low)) * float(settings["knobs"]["memory_persistence"])


def base_level(uses: int, age_hours: float, d: float) -> float:
    """Return ACT-R base-level activation ln(n/(1-d)) - d*ln(L), L in hours."""
    return math.log((max(0, uses) + 1) / (1.0 - d)) - d * math.log(max(age_hours, 1e-3))


class MemoryHit(str):
    """A retrieved memory line that carries its place, sources and activation."""
    def __new__(cls, line: str, *, place: str | None, sources: Any = (),
                activation: float = 0.0) -> MemoryHit:
        hit = super().__new__(cls, line)
        hit.place, hit.sources, hit.activation = place, tuple(sources), activation
        return hit


async def retrieve(conn: psycopg.AsyncConnection[Any], embedder: Any, query: str, *,
                   now: datetime, k: int, settings: dict[str, Any],
                   event_id: int | None = None) -> list[str]:
    """Retrieve up to k memories by total ACT-R activation, and count each use."""
    if not query.strip():
        return []
    vector = await read_event_vector(conn, event_id) if event_id is not None else None
    if vector is None:
        vector = (await embedder.embed([query]))[0]
    vec_rows = await (await conn.execute(_VECTOR_SQL, (Vector(vector), Vector(vector)))).fetchall()
    txt_rows = await (await conn.execute(_TEXT_SQL,
                                         (Vector(vector), query, query, query))).fetchall()
    rows = {row[0]: row for row in (*vec_rows, *txt_rows)}
    values, d = constants(settings), decay_d(settings)
    scored = [(base_level(int(row[7]), max(0.0, (now - row[8]).total_seconds() / 3600.0), d)
               + float(values["context_weight"]) * float(row[10])
               + float(values["importance_weight"]) * float(row[4]), row) for row in rows.values()]
    ranked = sorted(scored, key=lambda pair: (pair[0], pair[1][0]), reverse=True)[:k]
    if ranked:
        await conn.execute("UPDATE memories SET last_accessed_at = %s, use_count = use_count + 1 "
                           "WHERE id = ANY(%s)", (now, [row[0] for _, row in ranked]))
    return [MemoryHit(f"- {row[1]} ({row[2] or '?'} at {row[3] or '?'}; importance {row[4]:.2f})",
                      place=row[3], sources=row[9], activation=activation)
            for activation, row in ranked]


async def candidates(conn: psycopg.AsyncConnection[Any], embedder: Any, event: Event, *,
                     now: datetime, settings: dict[str, Any]) -> tuple[str, ...]:
    """Return the memory candidates for one event, using the prior's retrieval constants."""
    query = payload_text(event.payload)
    if embedder is None or not query.strip():
        return ()
    return tuple(await retrieve(conn, embedder, query, now=now, event_id=event.id,
                                settings=settings, k=int(retrieval(settings)["k"])))


async def best_activation(conn: psycopg.AsyncConnection[Any], embedder: Any, event: Event, *,
                          now: datetime, settings: dict[str, Any]) -> float:
    """Return the highest activation among candidates, recent messages included (FOK)."""
    lines = await candidates(conn, embedder, event, now=now, settings=settings)
    peak = max((line.activation for line in lines), default=0.0)
    vector = await read_event_vector(conn, event.id) if event.id is not None else None
    if vector is None or not event.place:
        return peak
    cur = await conn.execute(
        "SELECT 1 - (v.embedding <=> %s), e.ts FROM event_vectors v "
        "JOIN events e ON e.id = v.event_id WHERE e.place = %s AND e.kind = ANY(%s) "
        "AND e.id <> %s ORDER BY e.id DESC LIMIT %s",
        (Vector(vector), event.place, list(RECENT_KINDS), event.id,
         int(constants(settings)["recent_messages"])))
    values, d = constants(settings), decay_d(settings)
    scores = [base_level(0, max(0.0, (now - ts).total_seconds() / 3600.0), d)
              + float(values["context_weight"]) * float(similarity)
              for similarity, ts in await cur.fetchall()]
    return max([peak, *scores])


async def recent_lines(conn: psycopg.AsyncConnection[Any], exclude_place: str,
                       limit: int = 10) -> list[Event]:
    """Return the latest conversational lines of the other places, per place."""
    cur = await conn.execute(
        "WITH places AS (SELECT place, max(id) AS newest FROM events "
        "WHERE place IS NOT NULL AND place <> %s AND kind = ANY(%s) "
        "GROUP BY place ORDER BY newest DESC LIMIT %s), "
        "lines AS (SELECT e.*, row_number() OVER (PARTITION BY e.place ORDER BY e.id DESC) "
        "AS rn FROM events e JOIN places p ON p.place = e.place WHERE e.kind = ANY(%s)) "
        "SELECT id, ts, kind, who, place, body, payload, caused_by, origin_actor, "
        "schema_version FROM lines WHERE rn <= 3 ORDER BY id",
        (exclude_place, list(RECENT_KINDS), max(1, limit // 2), list(RECENT_KINDS)))
    return [event_from_row(row) for row in await cur.fetchall()]
