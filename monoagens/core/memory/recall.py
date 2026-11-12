"""Recall: the self's search over the Log and memories."""

from __future__ import annotations

from collections.abc import Sequence
from datetime import datetime, timezone
from typing import Any

from monoagens.infra.store import payload_text
from monoagens.types import RECENT_KINDS

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


async def search(conn, action, *, visible, memory_visible) -> dict[str, Any]:
    """Search events and memories; `visible` and `memory_visible` are the audience rule."""
    from monoagens.infra import prior  # local: identity imports recall at module level

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
              "place": row[4], "text": payload_text(row[5])}
             for row in rows
             if await visible(conn, context=context, source=row[4], before=row[0])]
    memory_sql = ("SELECT id, text, place, derived_from FROM memories "
                  "WHERE valid_to IS NULL" + (" AND text ILIKE %s" if query else "")
                  + " ORDER BY id DESC LIMIT %s")
    for memory_id, text, place, derived in await _rows(
            conn, memory_sql, ((f"%{query}%",) if query else ()) + (limit * 4,)):
        details = [{"event_id": row[0], "ts": row[1].isoformat(),
                    "text": payload_text(row[4])}
                   for row in await _rows(
                       conn, "SELECT id, ts, kind, place, payload FROM events "
                             "WHERE id = ANY(%s) ORDER BY id", (derived,))
                   if _inside(row[1], start, end)
                   and await visible(conn, context=context, source=row[3])]
        allowed = await memory_visible(conn, context=context, sources=derived)
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


def _inside(ts: datetime, start: datetime | None, end: datetime | None) -> bool:
    return (start is None or ts >= start) and (end is None or ts < end)


def _stamp(value: Any) -> datetime | None:
    if value in (None, ""):
        return None
    stamp = datetime.fromisoformat(str(value))
    return stamp if stamp.tzinfo else stamp.replace(tzinfo=timezone.utc)
