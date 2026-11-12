"""Recall: the self's search over the Log and memories, and a task thread's desk."""

from __future__ import annotations

from collections.abc import Sequence
from datetime import datetime, timezone
from typing import Any, Mapping

from monoagens import reach
from monoagens.store import read_last_events
from monoagens.types import Event, RECENT_KINDS

RECALL = {
    "name": "recall",
    "description": "Search the soul's own Log and memories: by time range, by lineage "
                   "from one event, or by text. Reports what was searched and found.",
    "schema": {"type": "object", "additionalProperties": False, "properties": {
        "query": {"type": "string", "description": "Text to search event payloads for."},
        "event_id": {"type": "integer", "description": "Follow this event's lineage."},
        "place": {"type": "string", "description": "Limit to one place."},
        "from": {"type": "string", "description": "ISO start, inclusive."},
        "to": {"type": "string", "description": "ISO end, exclusive."}}},
    "communicative": False, "long_running": False, "identity_changing": False,
}


async def search(conn, action) -> dict[str, Any]:
    """Search events and memories, obeying the audience of the current context."""
    from monoagens import prior  # local: identity imports recall at module level

    args, context = dict(action.payload), action.place or None
    query = str(args.get("query") or "").strip()
    start, end = _stamp(args.get("from")), _stamp(args.get("to"))
    limit = int(prior.constants(prior.load_settings())["recall_results"])
    clause, params = ["kind = ANY(%s)"], [list(RECENT_KINDS)]
    for value, condition in ((args.get("place"), "place = %s"), (start, "ts >= %s"),
                             (end, "ts < %s")):
        if value is not None:
            clause, params = [*clause, condition], [*params, value]
    if query:
        clause, params = [*clause, "payload::text ILIKE %s"], [*params, f"%{query}%"]
    sql = ("SELECT id, ts, kind, who, place, payload FROM events WHERE "
           + " AND ".join(clause) + " ORDER BY id DESC LIMIT %s")
    if args.get("event_id") is not None:
        sql = ("SELECT id, ts, kind, who, place, payload FROM events WHERE id IN "
               "(WITH RECURSIVE chain AS (SELECT id, caused_by FROM events WHERE id = %s "
               "UNION ALL SELECT e.id, e.caused_by FROM events e JOIN chain c "
               "ON e.id = c.caused_by) SELECT id FROM chain) ORDER BY id")
        params = [int(args["event_id"])]
    lineage = args.get("event_id") is not None
    rows = await _rows(conn, sql, params if lineage else (*params, limit))
    found = [{"kind": "event", "event_id": row[0], "ts": row[1].isoformat(),
              "place": row[4], "text": _payload_text(row[5])}
             for row in rows
             if await reach.visible(conn, context=context, source=row[4], before=row[0])]
    memory_sql = ("SELECT id, text, place, derived_from FROM memories "
                  "WHERE valid_to IS NULL" + (" AND text ILIKE %s" if query else "")
                  + " ORDER BY id DESC LIMIT %s")
    for memory_id, text, place, derived in await _rows(
            conn, memory_sql, ((f"%{query}%",) if query else ()) + (limit * 4,)):
        details = [{"event_id": row[0], "ts": row[1].isoformat(),
                    "text": _payload_text(row[4])}
                   for row in await _rows(
                       conn, "SELECT id, ts, kind, place, payload FROM events "
                             "WHERE id = ANY(%s) ORDER BY id", (derived,))
                   if _inside(row[1], start, end)
                   and await reach.visible(conn, context=context, source=row[3])]
        allowed = await reach.visible(conn, context=context, source=place)
        if allowed and (details or not (start or end)):
            found.append({"kind": "memory", "memory_id": memory_id, "text": text,
                          "place": place, "details": details})
    found = found[:limit]
    return {"searched": {"query": query or None, "event_id": args.get("event_id"),
                         "place": args.get("place"),
                         "from": start.isoformat() if start else None,
                         "to": end.isoformat() if end else None},
            "scope": f"events and memories visible from {context}",
            "count": len(found), "found": found,
            "note": None if found else "nothing found"}


async def _rows(conn, sql: str, params: Sequence[Any]) -> list[Any]:
    return list(await (await conn.execute(sql, params)).fetchall())


async def doing_lines(conn, place: str, task: Mapping[str, Any],
                      values: Mapping[str, Any]) -> tuple[str, ...]:
    """Show a task thread's work: goal, one line per step, latest result verbatim."""
    since, limit = int(task.get("since") or 0), int(values["max_task_steps"]) * 3
    events = await read_last_events(conn, place, limit=limit,
                                    kinds=("task.opened", "act.out", "act.result"))
    parent = str(task.get("parent") or "")
    if parent:
        events += await read_last_events(conn, parent, limit=limit,
                                         kinds=("act.out", "act.result"))
    events = sorted((seen for seen in events if (seen.id or 0) >= since),
                    key=lambda seen: seen.id or 0)
    lines = [f"goal: {task.get('goal') or '(unknown)'}"]
    lines += [f"step: {(seen.payload.get('arguments') or {}).get('text')}"
              for seen in events if seen.kind == "act.out"]
    results = [(seen.id, _result_text(seen)) for seen in events if seen.kind == "act.result"]
    lines += [f"earlier result #{event_id} (recall it)" for event_id, _ in results[:-1]]
    if results:
        lines.append(f"result: {results[-1][1][: int(values['thread_result_tokens']) * 4 or None]}")
    return tuple(lines)


def _result_text(event: Event) -> str:
    result = event.payload.get("result")
    return str(result.get("text") if isinstance(result, dict) else result or "")


def _inside(ts: datetime, start: datetime | None, end: datetime | None) -> bool:
    return (start is None or ts >= start) and (end is None or ts < end)


def _stamp(value: Any) -> datetime | None:
    if value in (None, ""):
        return None
    stamp = datetime.fromisoformat(str(value))
    return stamp if stamp.tzinfo else stamp.replace(tzinfo=timezone.utc)


def _payload_text(payload: Mapping[str, Any]) -> str:
    text = next((payload[key] for key in ("description", "text", "reason", "about")
                 if payload.get(key)), "")
    return str(text or (payload.get("arguments") or {}).get("text") or "")
