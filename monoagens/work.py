"""Working memory: the desk's `<doing>` for every chain in progress (D52)."""

from __future__ import annotations

import json
from collections.abc import Mapping, Sequence
from typing import Any

from monoagens.store import payload_text, read_last_events
from monoagens.types import Event


async def doing_lines(conn, place: str, task: Mapping[str, Any] | None,
                      values: Mapping[str, Any], trigger: Event | None = None,
                      ) -> tuple[str, ...]:
    """Show every chain in progress: the goal, one line per step, the latest result."""
    if task:
        return await _task_lines(conn, place, task, values)
    root, goal = await chain_root(conn, trigger.id if trigger is not None else None)
    events = await _chain_events(conn, root, place) if root is not None else []
    acts = [seen for seen in events if seen.kind == "act.out"]
    if not acts or any(seen.payload.get("communicative") for seen in acts):
        return ()
    return _lines(events, goal, values)


async def chain_root(conn, event_id: int | None) -> tuple[int | None, str]:
    """Return the first event of the chain an event belongs to, and its goal text."""
    if event_id is None:
        return None, ""
    cur = await conn.execute(
        "WITH RECURSIVE up AS ("
        "SELECT id, caused_by, payload FROM events WHERE id = %s "
        "UNION ALL SELECT e.id, e.caused_by, e.payload FROM events e JOIN up "
        "ON e.id = up.caused_by) "
        "SELECT id, payload FROM up WHERE caused_by IS NULL ORDER BY id LIMIT 1", (event_id,))
    row = await cur.fetchone()
    return (int(row[0]), payload_text(row[1] or {})) if row else (None, "")


async def step_spoke(conn, event: Event) -> bool:
    """Return whether the wake that produced this result already sent a message (12.16)."""
    if event.caused_by is None:
        return False
    cur = await conn.execute(
        "SELECT 1 FROM events act JOIN events said ON said.caused_by = act.caused_by "
        "WHERE act.id = %s AND said.kind = 'act.out' "
        "AND said.payload->>'communicative' = 'true' LIMIT 1", (event.caused_by,))
    return await cur.fetchone() is not None


async def _task_lines(conn, place: str, task: Mapping[str, Any],
                      values: Mapping[str, Any]) -> tuple[str, ...]:
    """Show a task thread's work from its own place and the parent that started it."""
    since, limit = int(task.get("since") or 0), int(values["max_task_steps"]) * 3
    events = await read_last_events(conn, place, limit=limit,
                                    kinds=("task.opened", "act.out", "act.result"))
    parent = str(task.get("parent") or "")
    if parent:
        events += await read_last_events(conn, parent, limit=limit,
                                         kinds=("act.out", "act.result"))
    events = [seen for seen in events if (seen.id or 0) >= since]
    return _lines(events, str(task.get("goal") or "(unknown)"), values)


async def _chain_events(conn, root: int, place: str) -> list[Event]:
    """Return every event of one chain that happened in this place, in order."""
    cur = await conn.execute(
        "WITH RECURSIVE chain AS ("
        "SELECT id, ts, kind, who, place, payload, caused_by FROM events WHERE id = %s "
        "UNION ALL SELECT e.id, e.ts, e.kind, e.who, e.place, e.payload, e.caused_by "
        "FROM events e JOIN chain c ON e.caused_by = c.id) "
        "SELECT id, ts, kind, who, place, payload, caused_by FROM chain "
        "WHERE place = %s ORDER BY id", (root, place))
    return [Event(id=row[0], ts=row[1], kind=row[2], who=row[3], place=row[4],
                  payload=row[5] or {}, caused_by=row[6]) for row in await cur.fetchall()]


def _lines(events: Sequence[Event], goal: str,
           values: Mapping[str, Any]) -> tuple[str, ...]:
    """Render one chain: the goal, a line per step, the latest result verbatim (D52)."""
    events = sorted(events, key=lambda seen: seen.id or 0)
    lines = [f"goal: {goal or '(unknown)'}"]
    lines += [f"step: {_step_text(seen)}" for seen in events if seen.kind == "act.out"]
    results = [(seen.id, _result_text(seen)) for seen in events if seen.kind == "act.result"]
    lines += [f"earlier result #{event_id} (recall it)" for event_id, _ in results[:-1]]
    if results:
        lines.append(f"result: {results[-1][1][: int(values['thread_result_tokens']) * 4]}")
    return tuple(lines)


def _step_text(event: Event) -> str:
    """Render one of the soul's acts, its arguments included, communicative or not."""
    arguments = dict(event.payload.get("arguments") or {})
    capability = event.payload.get("capability") or "?"
    return f"{capability} {json.dumps(arguments, ensure_ascii=False)}"


def _result_text(event: Event) -> str:
    """Render one act's result: its text, or the whole result when it has none."""
    result = event.payload.get("result")
    if isinstance(result, dict):
        return str(result.get("text") or json.dumps(result, ensure_ascii=False, default=str))
    return str(result or "")
