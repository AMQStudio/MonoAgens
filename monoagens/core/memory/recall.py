"""Recall: the self's search over the Log and memories."""

from __future__ import annotations

from collections.abc import Mapping, Sequence
from datetime import datetime, timezone
from typing import Any

from monoagens.core.memory.cards import equivalent_sources, merge_map
from monoagens.core.kinds import RECENT_KINDS
from monoagens.infra.store import escape_like

RECALL = {
    "name": "recall",
    "description": "Search the soul's own Log and memories: by time range, by lineage "
                   "from one event, or by text. Reports what was searched and found.",
    "schema": {"type": "object", "additionalProperties": False, "properties": {
        "query": {"type": "string", "description": "Text to search the Log's own words for."},
        "event_id": {"type": "integer", "description": "Follow this event's lineage."},
        "place": {"type": "string", "description": "Limit to one place."},
        "from": {"type": "string", "description": "ISO start, inclusive."},
        "to": {"type": "string", "description": "ISO end, exclusive."}}},
    "communicative": False, "long_running": False, "identity_changing": False,
}


async def search(conn, action, *, visible, memory_visible,
                 settings: Mapping[str, Any] | None = None) -> dict[str, Any]:
    """Search events and memories; `visible` and `memory_visible` are the audience rule."""
    from monoagens.infra import prior  # local: identity imports recall at module level

    values = prior.constants(settings or prior.load_settings())
    args, context = dict(action.arguments), action.place or None
    query = str(args.get("query") or "").strip()
    pattern = escape_like(query)
    start, end = _stamp(args.get("from")), _stamp(args.get("to"))
    limit = int(values["recall_results"])
    merges = await merge_map(conn)
    family = list(await equivalent_sources(conn, query)) if query else []
    clause, params = ["kind = ANY(%s)"], [list(RECENT_KINDS)]
    for value, condition in ((args.get("place"), "place = %s"), (start, "ts >= %s"),
                             (end, "ts < %s")):
        if value is not None:
            clause, params = [*clause, condition], [*params, value]
    if query:
        clause, params = [*clause, "(description ILIKE %s OR who = ANY(%s))"], [
            *params, f"%{pattern}%", family]
    sql = ("SELECT id, ts, kind, who, place, description FROM events WHERE "
           + " AND ".join(clause) + " ORDER BY id DESC LIMIT %s")
    if args.get("event_id") is not None:
        sql = ("SELECT id, ts, kind, who, place, description FROM events WHERE id IN "
               "(WITH RECURSIVE chain AS (SELECT id, caused_by FROM events WHERE id = %s "
               "UNION ALL SELECT e.id, e.caused_by FROM events e JOIN chain c "
               "ON e.id = c.caused_by) SELECT id FROM chain) ORDER BY id")
        params = [int(args["event_id"])]
    lineage = args.get("event_id") is not None
    rows = await _rows(conn, sql, params if lineage else (*params, limit))
    found = [{"kind": "event", "event_id": row[0], "ts": row[1].isoformat(),
              "who": merges.get(row[3], row[3]), "place": row[4], "text": str(row[5] or "")}
             for row in rows
             if await visible(conn, context=context, source=row[4], before=row[0])]
    mem_where = "WHERE valid_to IS NULL" + (
        " AND (text ILIKE %s OR who = ANY(%s))" if query else "")
    mem_params: tuple[Any, ...] = ((f"%{pattern}%", family) if query else ()) + (limit * 4,)
    memory_sql = f"SELECT id, text, who, place, derived_from FROM memories {mem_where} ORDER BY id DESC LIMIT %s"
    for memory_id, text, who, place, derived in await _rows(conn, memory_sql, mem_params):
        details = [{"event_id": row[0], "ts": row[1].isoformat(),
                    "who": merges.get(row[2], row[2]), "text": str(row[5] or "")}
                   for row in await _rows(
                       conn, "SELECT id, ts, who, kind, place, description FROM events "
                             "WHERE id = ANY(%s) ORDER BY id", (derived,))
                   if _inside(row[1], start, end)
                   and await visible(conn, context=context, source=row[4])]
        allowed = await memory_visible(conn, context=context, sources=derived)
        if allowed and (details or not (start or end)):
            found.append({"kind": "memory", "memory_id": memory_id, "text": text,
                          "who": merges.get(who, who), "place": place, "details": details})
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
