"""Identity: born from the birth certificate, then kept in `state: identity`."""

from __future__ import annotations

from datetime import datetime, timezone
from collections.abc import Sequence
from typing import Any, Mapping

from monoagens import reach
from monoagens.prior import constants, load_settings
from monoagens.store import append_event, project_event, read_last_events, read_state
from monoagens.types import RECENT_KINDS

KEY = "identity"
BORN = "identity.born"
UPDATED = "identity.updated"
FIELDS = ("name", "language", "persona", "principles")
CAPABILITIES = (
    {"name": "set_identity",
     "description": "Change who the soul is: name, language, persona or principles. "
                    "Only the owner may invoke it.",
     "schema": {"type": "object", "additionalProperties": False, "properties": {
         "name": {"type": "string", "description": "The new name."},
         "language": {"type": "string", "description": "The language it speaks with the owner."},
         "persona": {"type": "string", "description": "The persona, in any language."},
         "principles": {"type": "array", "items": {"type": "string"},
                        "description": "The principles it must never break."}}},
     "communicative": False, "long_running": False, "identity_changing": True},
    {"name": "recall",
     "description": "Search the soul's own Log and memories: by time range, by lineage "
                    "from one event, or by text. Reports what was searched and found.",
     "schema": {"type": "object", "additionalProperties": False, "properties": {
         "query": {"type": "string", "description": "Text to search event payloads for."},
         "event_id": {"type": "integer", "description": "Follow this event's lineage."},
         "place": {"type": "string", "description": "Limit to one place."},
         "from": {"type": "string", "description": "ISO start, inclusive."},
         "to": {"type": "string", "description": "ISO end, exclusive."}}},
     "communicative": False, "long_running": False, "identity_changing": False},
)


def from_certificate(certificate: Mapping[str, Any]) -> dict[str, Any]:
    """Return the identity a birth certificate describes."""
    identity = {field: certificate.get(field) for field in FIELDS}
    identity["principles"] = [str(item) for item in certificate.get("principles") or []]
    return identity


async def load(conn, certificate: Mapping[str, Any]) -> dict[str, Any]:
    """Return `state: identity`, falling back to the birth certificate before birth."""
    stored = await read_state(conn, KEY)
    return dict(stored) if stored is not None else from_certificate(certificate)


async def ensure(conn, certificate: Mapping[str, Any], *, now: datetime) -> dict[str, Any]:
    """Write the birth certificate into `state: identity` the first time only."""
    stored = await read_state(conn, KEY)
    if stored is not None:
        return dict(stored)
    identity = from_certificate(certificate)
    async with conn.transaction():
        event = await append_event(conn, ts=now, kind=BORN, payload={"identity": identity})
        await project_event(conn, event)
    return identity


def _update(current: Mapping[str, Any], arguments: Mapping[str, Any]) -> dict[str, Any]:
    """Merge one set_identity call into the current identity, rejecting bad fields."""
    unknown = sorted(set(arguments) - set(FIELDS))
    if unknown:
        raise ValueError(f"set_identity: unknown fields {unknown}")
    update = {field: arguments[field] for field in FIELDS if field in arguments}
    for field in ("name", "language", "persona"):
        if field in update and not str(update[field]).strip():
            raise ValueError(f"set_identity: {field} cannot be empty")
    if "principles" in update:
        if not isinstance(update["principles"], list):
            raise ValueError("set_identity: principles must be a list")
        update["principles"] = [str(item) for item in update["principles"]]
    if not update:
        raise ValueError("set_identity: nothing to change")
    return {**current, **update}


class SelfOrgan:
    """The one organ inside the core: it changes the soul's own identity."""

    name = "self"

    def __init__(self, conn, clock) -> None:
        """Store the connection and the injected clock."""
        self.conn, self.clock = conn, clock

    def capabilities(self) -> tuple[dict[str, Any], ...]:
        """Return the capability descriptors this organ declares."""
        return CAPABILITIES

    async def act(self, action) -> dict[str, Any]:
        """Apply an owner-approved identity change or answer a recall."""
        if action.kind == "recall":
            return await self._recall(action)
        if action.kind != "set_identity":
            raise ValueError(f"Unsupported action kind: {action.kind!r}")
        if action.event_id is None:
            raise ValueError("set_identity needs the act.out event it belongs to")
        current = await read_state(self.conn, KEY) or {}
        identity = _update(current, action.payload)
        async with self.conn.transaction():
            event = await append_event(self.conn, ts=self.clock.now(), kind=UPDATED,
                                       caused_by=action.event_id,
                                       payload={"identity": identity})
            await project_event(self.conn, event)
        return {"identity": identity}

    async def _recall(self, action) -> dict[str, Any]:
        """Search events and memories, obeying the audience of the current context."""
        args, context = dict(action.payload), action.place or None
        query = str(args.get("query") or "").strip()
        start, end = _stamp(args.get("from")), _stamp(args.get("to"))
        limit = int(constants(load_settings())["recall_results"])
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
        rows = await self._rows(sql, params if lineage else (*params, limit))
        found = [{"kind": "event", "event_id": row[0], "ts": row[1].isoformat(),
                  "place": row[4], "text": _payload_text(row[5])}
                 for row in rows
                 if await reach.visible(self.conn, context=context, source=row[4], before=row[0])]
        memory_sql = ("SELECT id, text, place, private, derived_from FROM memories "
                      "WHERE valid_to IS NULL" + (" AND text ILIKE %s" if query else "")
                      + " ORDER BY id DESC LIMIT %s")
        for memory_id, text, place, private, derived in await self._rows(
                memory_sql, ((f"%{query}%",) if query else ()) + (limit * 4,)):
            details = [{"event_id": row[0], "ts": row[1].isoformat(),
                        "text": _payload_text(row[4])}
                       for row in await self._rows(
                           "SELECT id, ts, kind, place, payload FROM events "
                           "WHERE id = ANY(%s) ORDER BY id", (derived,))
                       if _inside(row[1], start, end)
                       and await reach.visible(self.conn, context=context, source=row[3])]
            allowed = not private or await reach.visible(self.conn, context=context,
                                                         source=place)
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

    async def _rows(self, sql: str, params: Sequence[Any]) -> list[Any]:
        return list(await (await self.conn.execute(sql, params)).fetchall())


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
