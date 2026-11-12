"""Sleep: consolidating new events into memories and a fresh core memory."""

from __future__ import annotations

from datetime import datetime
from typing import Any
from zoneinfo import ZoneInfo

import psycopg
from pgvector import Vector

from monoagens.infra import budget
from monoagens.infra.store import (append_event, payload_text, read_expectations, read_state,
                                   resolve_expectation, write_state)
from monoagens.types import Event

STORE_MEMORIES = {"type": "function", "function": {
    "name": "store_memories",
    "description": "Close the sleep by writing what is worth keeping.",
    "parameters": {"type": "object", "required": ["items", "core_memory"], "properties": {
        "items": {"type": "array", "items": {"type": "object", "required": ["text"],
                                             "properties": {
            "text": {"type": "string"}, "importance": {"type": "number"},
            "who": {"type": "string"}}}},
        "core_memory": {"type": "string"}}}}}
SLEEP_SYSTEM = (
    "You are {name}'s memory, asleep; nobody sees this. Summarize the events into durable "
    "memories in {language}, one item per fact, preference, or promise worth keeping, each "
    "with importance 0..1. Rewrite core_memory as the short always-visible summary of who "
    "the people are and what matters now. Use store_memories. Never invent facts."
)


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
    sources = [row[0] for row in rows]
    places = {row[4] for row in rows if row[4]}
    place = next(iter(places)) if len(places) == 1 else None
    zone = ZoneInfo(str(settings.get("timezone") or "UTC"))
    identity = settings.get("identity") or {}
    lines = [f"#{r[0]} {r[1].astimezone(zone):%Y-%m-%d %H:%M} [{r[2]}] {r[3] or '?'} @ "
             f"{r[4] or '?'}: {payload_text(r[5])}"
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
    payload = {"core_memory": core, "last_event_id": sources[-1], "derived_from": sources}
    async with conn.transaction():
        for item, vector in zip(items, vectors):
            await conn.execute(
                "INSERT INTO memories (who, place, text, embedding, importance, private, "
                "valid_from, last_accessed_at, derived_from) "
                "VALUES (%s, %s, %s, %s, %s, %s, %s, %s, %s)",
                (item.get("who") or None, place, str(item["text"]),
                 Vector(vector), float(item.get("importance", 0.5)),
                 bool(places), now, now, sources))
        for row in await read_expectations(conn, "ex.status = 'active' AND ex.expires_at <= %s",
                                           (now,)):
            await resolve_expectation(conn, row, now, "dropped")
        done = await append_event(conn, ts=now, kind="sleep.done", payload=payload,
                                  caused_by=payload["last_event_id"])
    await write_state(conn, "core_memory", {"text": core, "derived_from": sources}, done.id)
    await write_state(conn, "sleep:last_event", {"id": payload["last_event_id"]}, done.id)
    return done
