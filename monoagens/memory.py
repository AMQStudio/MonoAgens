"""Memory: embeddings, hybrid retrieval, and sleep consolidation."""

from __future__ import annotations

import math
from collections.abc import Sequence
from datetime import datetime
from typing import Any, Protocol
from zoneinfo import ZoneInfo

import psycopg
from pgvector import Vector

from monoagens import budget
from monoagens.pfc import PFCContext
from monoagens.prior import constants, retrieval
from monoagens.store import (active_expectation_lines, append_event,
                             read_event_vector, read_expectations, read_last_events,
                             read_state, resolve_expectation, write_state)
from monoagens.types import RECENT_KINDS, Event

_ROWS = "id, text, who, place, importance, last_accessed_at, private"
_VECTOR_SQL = (f"SELECT {_ROWS} FROM memories WHERE valid_to IS NULL AND embedding IS NOT NULL "
               "ORDER BY embedding <=> %s LIMIT 20")
_TEXT_SQL = (f"SELECT {_ROWS} FROM memories WHERE valid_to IS NULL AND "
             "(tsv @@ plainto_tsquery('simple', %s) OR word_similarity(%s, text) > 0.2) "
             "ORDER BY word_similarity(%s, text) DESC LIMIT 20")
STORE_MEMORIES = {"type": "function", "function": {
    "name": "store_memories",
    "description": "Close the sleep by writing what is worth keeping.",
    "parameters": {"type": "object", "required": ["items", "core_memory"], "properties": {
        "items": {"type": "array", "items": {"type": "object", "required": ["text"],
                                             "properties": {
            "text": {"type": "string"}, "importance": {"type": "number"},
            "who": {"type": "string"}, "place": {"type": "string"},
            "private": {"type": "boolean"}}}},
        "core_memory": {"type": "string"}}}}}
SLEEP_SYSTEM = (
    "You are {name}'s memory, asleep; nobody sees this. Summarize the events into "
    "durable memories in {language}, one item per fact, preference, or promise worth "
    "keeping, each with importance 0..1. Rewrite core_memory as the short "
    "always-visible summary of who the owner is and what matters now. Use "
    "store_memories. Never invent facts."
)


class Embedder(Protocol):
    """Anything that turns texts into vectors."""

    async def embed(self, texts: Sequence[str]) -> list[list[float]]:
        """Return one vector per input text."""


def _payload_text(payload: dict[str, Any]) -> str:
    """Return the human-readable text of one event payload."""
    for key in ("description", "text", "reason", "about"):
        if payload.get(key):
            return str(payload[key])
    return ""


def cosine(a: Sequence[float], b: Sequence[float]) -> float:
    """Return the cosine similarity of two vectors (0.0 for a zero vector)."""
    dot = sum(x * y for x, y in zip(a, b))
    norm = math.sqrt(sum(x * x for x in a)) * math.sqrt(sum(y * y for y in b))
    return dot / norm if norm else 0.0


class MemoryHit(str):
    """A retrieved memory line that carries its place and privacy flag."""

    def __new__(cls, line: str, *, place: str | None, private: bool) -> MemoryHit:
        """Build one line the context policy can filter."""
        hit = super().__new__(cls, line)
        hit.place, hit.private = place, private
        return hit


async def retrieve(conn: psycopg.AsyncConnection[Any], embedder: Any, query: str, *,
                   now: datetime, k: int, decay: float, rrf_k: float,
                   event_id: int | None = None) -> list[str]:
    """Retrieve up to k memories by hybrid RRF search with importance and decay."""
    if not query.strip():
        return []
    vector = await read_event_vector(conn, event_id) if event_id is not None else None
    if vector is None:
        vector = (await embedder.embed([query]))[0]
    vec_rows = await (await conn.execute(_VECTOR_SQL, (Vector(vector),))).fetchall()
    txt_rows = await (await conn.execute(_TEXT_SQL, (query, query, query))).fetchall()
    vec = {row[0]: row for row in vec_rows}
    txt = {row[0]: row for row in txt_rows}
    scores: dict[int, float] = {}
    for ranked in (list(vec), list(txt)):
        for rank, row_id in enumerate(ranked, 1):
            scores[row_id] = scores.get(row_id, 0.0) + 1.0 / (rrf_k + rank)
    rows = {**vec, **txt}
    scored = []
    for row_id, score in scores.items():
        hours = max(0.0, (now - rows[row_id][5]).total_seconds() / 3600.0)
        scored.append((score * float(rows[row_id][4]) * decay ** hours, row_id))
    chosen = [rows[row_id] for _, row_id in sorted(scored, reverse=True)[:k]]
    if chosen:
        await conn.execute("UPDATE memories SET last_accessed_at = %s WHERE id = ANY(%s)",
                           (now, [row[0] for row in chosen]))
    return [MemoryHit(f"- {row[1]} ({row[2] or '?'} at {row[3] or '?'}; importance {row[4]:.2f})",
                      place=row[3], private=bool(row[6])) for row in chosen]


async def candidates(conn: psycopg.AsyncConnection[Any], embedder: Any, event: Event, *,
                     now: datetime, settings: dict[str, Any]) -> tuple[str, ...]:
    """Return the memory candidates for one event, using the prior's retrieval constants."""
    query = str(event.payload.get("description") or event.payload.get("text")
                or event.payload.get("about") or "")
    if embedder is None or not query.strip():
        return ()
    return tuple(await retrieve(conn, embedder, query, now=now, event_id=event.id,
                                **retrieval(settings)))


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
    return [Event(id=row[0], ts=row[1], kind=row[2], who=row[3], place=row[4], body=row[5],
                  payload=row[6] or {}, caused_by=row[7], origin_actor=row[8],
                  schema_version=row[9]) for row in await cur.fetchall()]


async def build_context(conn: psycopg.AsyncConnection[Any], event: Event, place: str,
                        reason: str, *, now: datetime, settings: dict[str, Any],
                        memories: Sequence[str], capabilities: Sequence[dict[str, Any]],
                        ) -> PFCContext:
    """Lay out the desk for one wake: recent events, memories, state and expectations."""
    values = constants(settings)
    recent = await read_last_events(conn, place, limit=int(values["recent_events"]),
                                    kinds=RECENT_KINDS)
    recent_elsewhere = await recent_lines(conn, place, limit=int(values["recent_elsewhere"]))
    core = await read_state(conn, "core_memory")
    last = None
    for key in (f"last_spoke:{place}", f"last_seen:{event.who}"):
        value = await read_state(conn, key)
        if value and value.get("ts"):
            last = datetime.fromisoformat(value["ts"])
            break
    return PFCContext(
        now=now, place=place, place_kind=str(event.payload.get("place_kind") or "direct"),
        reason=reason, core_memory=str((core or {}).get("text") or ""),
        expectations=await active_expectation_lines(conn, place), memories=memories,
        recent=[seen for seen in recent if seen.id != event.id],
        recent_elsewhere=recent_elsewhere, trigger=event, last_interaction=last,
        capabilities=capabilities)


async def sleep_due(conn: psycopg.AsyncConnection[Any], now: datetime,
                    settings: dict[str, Any]) -> bool:
    """Return whether the self should sleep now."""
    config = (settings.get("runtime") or {}).get("sleep") or {}
    zone = ZoneInfo(str(settings.get("timezone") or "UTC"))
    last_id = int((await read_state(conn, "sleep:last_event") or {}).get("id") or 0)
    cur = await conn.execute(
        "SELECT max(id) FILTER (WHERE kind IN ('percept', 'act.out')), "
        "max(ts) FILTER (WHERE kind = 'sleep.failed'), "
        "max(ts) FILTER (WHERE kind IN ('percept', 'act.out')), "
        "max(ts) FILTER (WHERE kind = 'sleep.done') FROM events")
    newest, last_failed, idle_since, slept = await cur.fetchone()
    if newest is None or int(newest) <= last_id:
        return False
    retry = float(config.get("retry_minutes", 30)) * 60.0
    if last_failed and (now - last_failed).total_seconds() < retry:
        return False
    idle_limit = float(config.get("idle_minutes", 30)) * 60
    if idle_since and (now - idle_since).total_seconds() >= idle_limit:
        return True
    if now.astimezone(zone).hour != int(config.get("hour", 2)):
        return False
    return not (slept and slept.astimezone(zone).date() == now.astimezone(zone).date())


async def sleep(conn: psycopg.AsyncConnection[Any], *, pfc: Any, embedder: Any, now: datetime,
                settings: dict[str, Any]) -> Event | None:
    """Summarize new events into memories, or record sleep.failed and wait to retry."""
    try:
        return await _consolidate(conn, pfc=pfc, embedder=embedder, now=now, settings=settings)
    except Exception as exc:
        return await append_event(conn, ts=now, kind="sleep.failed", payload={"error": str(exc)})


async def _consolidate(conn: psycopg.AsyncConnection[Any], *, pfc: Any, embedder: Any,
                       now: datetime, settings: dict[str, Any]) -> Event | None:
    """Summarize new events into memories, refresh core_memory, record sleep.done."""
    last_id = int((await read_state(conn, "sleep:last_event") or {}).get("id") or 0)
    cur = await conn.execute("SELECT id, ts, kind, who, place, payload FROM events WHERE id > %s "
                             "AND kind NOT IN ('sleep.done', 'tick', 'model.call', "
                             "'budget.exhausted', 'job.started', 'job.result') "
                             "ORDER BY id", (last_id,))
    rows = await cur.fetchall()
    if not rows:
        return None
    zone = ZoneInfo(str(settings.get("timezone") or "UTC"))
    identity = settings.get("identity") or {}
    lines = [f"#{r[0]} {r[1].astimezone(zone):%Y-%m-%d %H:%M} [{r[2]}] {r[3] or '?'} @ "
             f"{r[4] or '?'}: {_payload_text(r[5])}"
             for r in rows]
    old_core = str((await read_state(conn, "core_memory") or {}).get("text") or "")
    messages = [
        {"role": "system", "content": SLEEP_SYSTEM.format(name=identity.get("name") or "Mona",
                                                          language=identity.get("language")
                                                          or "en")},
        {"role": "user", "content": f"<core_memory>{old_core}</core_memory>\n<events>\n"
                                    + "\n".join(lines) + "\n</events>"}]
    calls = await pfc.call(messages, [STORE_MEMORIES])
    await budget.record_call(conn, int(rows[-1][0]), pfc, purpose="sleep.consolidate",
                             now=now, settings=settings)
    args = next((call.arguments for call in calls if call.name == "store_memories"), {})
    items = [item for item in args.get("items") or []
             if isinstance(item, dict) and item.get("text")]
    vectors = await embedder.embed([str(item["text"]) for item in items]) if items else []
    core = str(args.get("core_memory") or old_core)
    payload = {"core_memory": core, "last_event_id": int(rows[-1][0]), "memories": len(items)}
    async with conn.transaction():
        for item, vector in zip(items, vectors):
            await conn.execute(
                "INSERT INTO memories (who, place, text, embedding, importance, private, "
                "valid_from, last_accessed_at, derived_from) "
                "VALUES (%s, %s, %s, %s, %s, %s, %s, %s, %s)",
                (item.get("who") or None, item.get("place") or None, str(item["text"]),
                 Vector(vector), float(item.get("importance", 0.5)),
                 bool(item.get("private", False)), now, now, [r[0] for r in rows]))
        for row in await read_expectations(conn, "ex.status = 'active' AND ex.expires_at <= %s",
                                           (now,)):
            await resolve_expectation(conn, row, now, "dropped")
        done = await append_event(conn, ts=now, kind="sleep.done", payload=payload,
                                  caused_by=payload["last_event_id"])
    await write_state(conn, "core_memory", {"text": core}, done.id)
    await write_state(conn, "sleep:last_event", {"id": payload["last_event_id"]}, done.id)
    return done

