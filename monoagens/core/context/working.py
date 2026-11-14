"""Working memory: the desk's `<doing>` for every chain in progress."""

from __future__ import annotations

import json
from collections.abc import Mapping, Sequence
from typing import Any

from monoagens.infra.store import actions_of, read_last_events
from monoagens.types import Event


async def doing_entries(conn, place: str, task: Mapping[str, Any] | None,
                        values: Mapping[str, Any], trigger: Event | None = None,
                        ) -> tuple[tuple[str, str | None, int | None], ...]:
    """Return `(line, source_place, source_event_id)` for every chain item in progress."""
    if task:
        return await _task_entries(conn, place, task, values)
    root, root_place, goal = await chain_root_info(
        conn, trigger.id if trigger is not None else None)
    events = await _chain_events(conn, root, place) if root is not None else []
    if not any(seen.kind == "act.out" for seen in events):
        return ()
    ledger = await _ledger(conn, events)
    if _spoke(ledger):
        return ()
    return _line_entries(events, goal, ledger, values, goal_source=(root_place or place, root))


async def doing_lines(conn, place: str, task: Mapping[str, Any] | None,
                      values: Mapping[str, Any], trigger: Event | None = None,
                      ) -> tuple[str, ...]:
    """Show every chain in progress: the goal, one line per step, the latest result."""
    return tuple(line for line, _, _ in await doing_entries(conn, place, task, values, trigger))


async def chain_root_info(conn, event_id: int | None) -> tuple[int | None, str | None, str]:
    """Return `(root_id, root_place, goal_text)` for the chain an event belongs to."""
    if event_id is None:
        return None, None, ""
    cur = await conn.execute(
        "WITH RECURSIVE up AS ("
        "SELECT id, caused_by, place, description FROM events WHERE id = %s "
        "UNION ALL SELECT e.id, e.caused_by, e.place, e.description FROM events e JOIN up "
        "ON e.id = up.caused_by) "
        "SELECT id, place, description FROM up WHERE caused_by IS NULL ORDER BY id LIMIT 1",
        (event_id,))
    row = await cur.fetchone()
    return (int(row[0]), row[1], str(row[2] or "")) if row else (None, None, "")


async def chain_root(conn, event_id: int | None) -> tuple[int | None, str]:
    """Return the first event of the chain an event belongs to, and its goal text."""
    root, _, goal = await chain_root_info(conn, event_id)
    return root, goal


async def step_spoke(conn, event: Event) -> bool:
    """Return whether the act that produced this result already sent a message (12.16).

    The rule is per act, not per wake (F.4): the result of the message that spoke does not
    wake reasoning again, while a tool's result is kept even when another act of the same
    wake spoke, because that result is what reasoning has not seen yet.
    """
    if event.caused_by is None:
        return False
    cur = await conn.execute(
        "SELECT 1 FROM events act WHERE act.id = %s AND act.kind = 'act.out' "
        "AND act.payload->>'communicative' = 'true' "
        "AND COALESCE(act.payload->>'refused', 'false') <> 'true' LIMIT 1",
        (event.caused_by,))
    return await cur.fetchone() is not None


async def _task_entries(conn, place: str, task: Mapping[str, Any],
                        values: Mapping[str, Any]) -> tuple[tuple[str, str | None, int | None], ...]:
    """Return a task thread's lines paired with their source place and event id."""
    since, limit = int(task.get("since") or 0), int(values["max_task_steps"]) * 3
    events = await read_last_events(conn, place, limit=limit,
                                    kinds=("task.opened", "act.out", "act.result"))
    parent = str(task.get("parent") or "")
    if parent:
        events += await read_last_events(conn, parent, limit=limit,
                                         kinds=("act.out", "act.result"))
    events = [seen for seen in events if (seen.id or 0) >= since]
    goal_src = (str(task.get("source_place") or parent or place),
                int(since) if parent and since else None)
    ledger = await _ledger(conn, events)
    return _line_entries(events, str(task.get("goal") or "(unknown)"), ledger, values,
                         goal_source=goal_src)


async def _ledger(conn, events: Sequence[Event]) -> dict[int, dict[str, Any]]:
    """Return the action ledger row of every `act.out` event in one chain."""
    return await actions_of(conn, [seen.id for seen in events if seen.kind == "act.out"])


def _spoke(ledger: Mapping[int, dict[str, Any]]) -> bool:
    """Return whether an act of this chain already spoke: its receipt is not a refusal."""
    for row in ledger.values():
        receipt = row.get("receipt") or {}
        if row.get("communicative") and receipt.get("state") not in ("refused", "failed"):
            return True
    return False


async def _chain_events(conn, root: int, place: str) -> list[Event]:
    """Return every event of one chain that happened in this place, in order."""
    cur = await conn.execute(
        "WITH RECURSIVE chain AS ("
        "SELECT id, ts, kind, who, place, description, caused_by FROM events WHERE id = %s "
        "UNION ALL SELECT e.id, e.ts, e.kind, e.who, e.place, e.description, e.caused_by "
        "FROM events e JOIN chain c ON e.caused_by = c.id) "
        "SELECT id, ts, kind, who, place, description, caused_by FROM chain "
        "WHERE place = %s ORDER BY id", (root, place))
    return [Event(id=row[0], ts=row[1], kind=row[2], who=row[3], place=row[4],
                  description=row[5] or "", caused_by=row[6]) for row in await cur.fetchall()]


def _line_entries(events: Sequence[Event], goal: str, ledger: Mapping[int, dict[str, Any]],
                  values: Mapping[str, Any], *,
                  goal_source: tuple[str | None, int | None] = (None, None),
                  ) -> tuple[tuple[str, str | None, int | None], ...]:
    """Render one chain with source `(place, event_id)` attached to every line."""
    events = sorted(events, key=lambda seen: seen.id or 0)
    trigger_of = {seen.id: seen.caused_by for seen in events
                  if seen.kind == "act.out" and seen.id is not None}
    lines: list[tuple[str, str | None, int | None]] = [
        (f"goal: {goal or '(unknown)'}", goal_source[0], goal_source[1]),
    ]
    lines += [(f"step: {_step_text(seen, ledger.get(seen.id))}", seen.place, seen.id)
              for seen in events if seen.kind == "act.out"]
    results = [(seen.id, seen.caused_by, seen.place, _result_text(seen, ledger))
               for seen in events if seen.kind == "act.result"]
    if not results:
        return tuple(lines)
    latest_trigger = trigger_of.get(results[-1][1])
    cut = len(results) - 1
    if latest_trigger is not None:
        cut = next((i for i, (_, cb, _, _) in enumerate(results)
                    if trigger_of.get(cb) == latest_trigger), len(results) - 1)
    cap = int(values["thread_result_tokens"]) * 4
    lines += [(f"earlier result #{event_id} (recall it)", ev_place, event_id)
              for event_id, _, ev_place, _ in results[:cut]]
    each = max(200, cap // max(1, len(results) - cut))
    lines += [(f"result: {text[:each]}", ev_place, event_id)
              for event_id, _, ev_place, text in results[cut:]]
    return tuple(lines)


def _step_text(event: Event, row: Mapping[str, Any] | None) -> str:
    """Render one of the soul's acts from its ledger row, else from its own description."""
    if not row:
        return event.description
    arguments = dict(row.get("arguments") or {})
    return f"{row.get('capability') or '?'} {json.dumps(arguments, ensure_ascii=False)}"


def _result_text(event: Event, ledger: Mapping[int, dict[str, Any]]) -> str:
    """Render one act's result from its action's receipt, else from its own description."""
    receipt = (ledger.get(event.caused_by) or {}).get("receipt") or {}
    result = receipt.get("result")
    if isinstance(result, dict):
        found = str(result.get("text") or json.dumps(result, ensure_ascii=False, default=str))
    else:
        found = str(result or "")
    return found or str(receipt.get("reason") or receipt.get("state") or event.description or "")
