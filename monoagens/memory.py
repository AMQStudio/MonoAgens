"""Memory: embeddings, hybrid retrieval, and sleep consolidation."""

from __future__ import annotations

import asyncio
import hashlib
import math
from collections.abc import Sequence
from datetime import datetime, timezone
from typing import Any, Protocol
from zoneinfo import ZoneInfo

import psycopg
from pgvector import Vector

from monoagens.store import (append_event, read_expectations, read_state,
                             resolve_expectation, write_state)
from monoagens.types import Event

DIM = 384
MODEL = "sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2"
RRF_K, DECAY = 60.0, 0.995
_ROWS = "id, text, who, place, importance, last_accessed_at"
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
            "who": {"type": "string"}, "place": {"type": "string"}}}},
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


def cosine(a: Sequence[float], b: Sequence[float]) -> float:
    """Return the cosine similarity of two vectors (0.0 for a zero vector)."""
    dot = sum(x * y for x, y in zip(a, b))
    norm = math.sqrt(sum(x * x for x in a)) * math.sqrt(sum(y * y for y in b))
    return dot / norm if norm else 0.0


class FakeEmbedder:
    """A deterministic hashed bag-of-words embedder for tests."""

    def _vector(self, text: str) -> list[float]:
        vec = [0.0] * DIM
        for token in str(text).lower().split():
            token = "".join(ch for ch in token if ch.isalnum())
            if not token:
                continue
            digest = hashlib.sha256(token.encode()).digest()
            vec[int.from_bytes(digest[:4], "big") % DIM] += 1.0
            vec[int.from_bytes(digest[4:8], "big") % DIM] -= 0.5
        norm = math.sqrt(sum(x * x for x in vec))
        return [x / norm for x in vec] if norm else vec

    async def embed(self, texts: Sequence[str]) -> list[list[float]]:
        """Return one hashed vector per text."""
        return [self._vector(text) for text in texts]


class FastEmbedder:
    """The real embedder, lazily loaded and run off the event loop."""

    def __init__(self, model_name: str = MODEL) -> None:
        """Remember the model name; the model itself loads on first use."""
        self.model_name = model_name
        self._model: Any = None

    async def embed(self, texts: Sequence[str]) -> list[list[float]]:
        """Return one embedding per text, computed in a worker thread."""
        if self._model is None:
            from fastembed import TextEmbedding

            self._model = await asyncio.to_thread(TextEmbedding, model_name=self.model_name)
        return await asyncio.to_thread(
            lambda: [[float(x) for x in v] for v in self._model.embed(list(texts))])


async def retrieve(conn: psycopg.AsyncConnection[Any], embedder: Any, query: str, *,
                   k: int = 5, now: datetime | None = None) -> list[str]:
    """Retrieve up to k memories by hybrid RRF search with importance and decay."""
    if not query.strip():
        return []
    now = now or datetime.now(timezone.utc)
    vector = (await embedder.embed([query]))[0]
    vec_rows = await (await conn.execute(_VECTOR_SQL, (Vector(vector),))).fetchall()
    txt_rows = await (await conn.execute(_TEXT_SQL, (query, query, query))).fetchall()
    vec = {row[0]: row for row in vec_rows}
    txt = {row[0]: row for row in txt_rows}
    scores: dict[int, float] = {}
    for ranked in (list(vec), list(txt)):
        for rank, row_id in enumerate(ranked, 1):
            scores[row_id] = scores.get(row_id, 0.0) + 1.0 / (RRF_K + rank)
    rows = {**vec, **txt}
    scored = []
    for row_id, score in scores.items():
        hours = max(0.0, (now - rows[row_id][5]).total_seconds() / 3600.0)
        scored.append((score * float(rows[row_id][4]) * DECAY ** hours, row_id))
    chosen = [rows[row_id] for _, row_id in sorted(scored, reverse=True)[:k]]
    if chosen:
        await conn.execute("UPDATE memories SET last_accessed_at = %s WHERE id = ANY(%s)",
                           (now, [row[0] for row in chosen]))
    return [f"- {row[1]} ({row[2] or '?'} at {row[3] or '?'}; importance {row[4]:.2f})"
            for row in chosen]


async def sleep_due(conn: psycopg.AsyncConnection[Any], now: datetime,
                    soul: dict[str, Any]) -> bool:
    """Return whether the self should sleep now."""
    config, zone = soul.get("sleep") or {}, ZoneInfo(str(soul.get("timezone") or "UTC"))
    last_id = int((await read_state(conn, "sleep:last_event") or {}).get("id") or 0)
    cur = await conn.execute(
        "SELECT max(id) FILTER (WHERE kind IN ('msg.in', 'msg.out')), "
        "max(ts) FILTER (WHERE kind = 'sleep.failed'), "
        "max(ts) FILTER (WHERE kind IN ('msg.in', 'msg.out')), "
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
                soul: dict[str, Any]) -> Event | None:
    """Summarize new events into memories, or record sleep.failed and wait to retry."""
    try:
        return await _consolidate(conn, pfc=pfc, embedder=embedder, now=now, soul=soul)
    except Exception as exc:
        return await append_event(conn, ts=now, kind="sleep.failed", payload={"error": str(exc)})


async def _consolidate(conn: psycopg.AsyncConnection[Any], *, pfc: Any, embedder: Any,
                       now: datetime, soul: dict[str, Any]) -> Event | None:
    """Summarize new events into memories, refresh core_memory, record sleep.done."""
    last_id = int((await read_state(conn, "sleep:last_event") or {}).get("id") or 0)
    cur = await conn.execute("SELECT id, ts, kind, who, place, payload FROM events WHERE id > %s "
                             "AND kind NOT IN ('sleep.done', 'tick') ORDER BY id", (last_id,))
    rows = await cur.fetchall()
    if not rows:
        return None
    zone = ZoneInfo(str(soul.get("timezone") or "UTC"))
    lines = [f"#{r[0]} {r[1].astimezone(zone):%Y-%m-%d %H:%M} [{r[2]}] {r[3] or '?'} @ "
             f"{r[4] or '?'}: {r[5].get('text') or r[5].get('reason') or r[5].get('about') or ''}"
             for r in rows]
    old_core = str((await read_state(conn, "core_memory") or {}).get("text") or "")
    messages = [
        {"role": "system", "content": SLEEP_SYSTEM.format(name=soul.get("name") or "Mona",
                                                          language=soul.get("language") or "en")},
        {"role": "user", "content": f"<core_memory>{old_core}</core_memory>\n<events>\n"
                                    + "\n".join(lines) + "\n</events>"}]
    calls = await pfc.call(messages, [STORE_MEMORIES])
    args = next((call.arguments for call in calls if call.name == "store_memories"), {})
    items = [item for item in args.get("items") or []
             if isinstance(item, dict) and item.get("text")]
    vectors = await embedder.embed([str(item["text"]) for item in items]) if items else []
    core = str(args.get("core_memory") or old_core)
    payload = {"core_memory": core, "last_event_id": int(rows[-1][0]), "memories": len(items)}
    async with conn.transaction():
        for item, vector in zip(items, vectors):
            await conn.execute(
                "INSERT INTO memories (who, place, text, embedding, importance, valid_from, "
                "last_accessed_at, source_events) VALUES (%s, %s, %s, %s, %s, %s, %s, %s)",
                (item.get("who") or None, item.get("place") or None, str(item["text"]),
                 Vector(vector), float(item.get("importance", 0.5)), now, now,
                 [r[0] for r in rows]))
        for row in await read_expectations(conn, "ex.status = 'active' AND ex.expires_at <= %s",
                                           (now,)):
            await resolve_expectation(conn, row, now, "dropped")
        done = await append_event(conn, ts=now, kind="sleep.done", payload=payload,
                                  caused_by=payload["last_event_id"])
    await write_state(conn, "core_memory", {"text": core}, done.id)
    await write_state(conn, "sleep:last_event", {"id": payload["last_event_id"]}, done.id)
    return done

