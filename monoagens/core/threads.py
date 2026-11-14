"""Task threads: one chain of work in its own place, with its own PFC slot (D47)."""

from __future__ import annotations

from collections.abc import Mapping
from typing import Any, Sequence

from monoagens.infra.prior import constants
from monoagens.infra.store import read_state, write_state

KEY = "task:"
OPEN = "open"
CONTINUATIONS = ("expectation.fired", "act.result")


async def open_tasks(conn) -> list[dict[str, Any]]:
    """Return every open task thread."""
    cur = await conn.execute("SELECT key, value FROM state WHERE key LIKE 'task:%' ORDER BY key")
    tasks = [{"place": key[len(KEY):], **value} for key, value in await cur.fetchall()]
    return [task for task in tasks if task.get("status") == OPEN]


async def is_open(conn, place: str) -> bool:
    """Return whether one task thread exists and is still open."""
    task = await read_state(conn, KEY + place)
    return bool(task and task.get("status") == OPEN)


async def closed_reason(conn, event) -> str | None:
    """Return the close reason if this event comes from a task thread that is now closed."""
    anchor = event.id or event.caused_by
    if anchor is None:
        return None
    cur = await conn.execute(
        "WITH RECURSIVE chain AS ("
        "SELECT id, caused_by FROM events WHERE id = %s "
        "UNION ALL SELECT e.id, e.caused_by FROM events e JOIN chain c ON e.id = c.caused_by"
        ") SELECT s.value->>'reason' FROM chain c "
        "JOIN state s ON s.key = 'task:space:task:' || c.id::text "
        "WHERE s.value->>'status' <> %s ORDER BY c.id DESC LIMIT 1",
        (int(anchor), OPEN),
    )
    row = await cur.fetchone()
    if row is not None:
        return str(row[0] or "closed")
    if getattr(event, "kind", None) == "act.result" and event.caused_by is not None and event.place:
        cur = await conn.execute(
            "SELECT value->>'reason' FROM state WHERE key LIKE 'task:%%' "
            "AND value->>'status' <> %s AND value->>'parent' = %s "
            "AND COALESCE((value->>'since')::bigint, 0) <= %s "
            "AND COALESCE((value->>'closed_event')::bigint, 0) > %s LIMIT 1",
            (OPEN, str(event.place), int(event.caused_by), int(event.caused_by)),
        )
        row = await cur.fetchone()
        if row is not None:
            return str(row[0] or "closed")
    return None


async def _goal_for(conn, event) -> str:
    """Return the chain's triggering description or text when opening on a follow-up act."""
    if event.description or event.caused_by is None:
        return event.description
    cur = await conn.execute(
        "WITH RECURSIVE chain AS ("
        "SELECT id, caused_by, description FROM events WHERE id = %s "
        "UNION ALL SELECT e.id, e.caused_by, e.description FROM events e JOIN chain c "
        "ON e.id = c.caused_by) SELECT description FROM chain ORDER BY id",
        (int(event.caused_by),),
    )
    for (found,) in await cur.fetchall():
        if found:
            return str(found)
    return ""


async def open_for(kernel, event) -> dict[str, Any] | None:
    """Open a task place for one chain when it does not already have one."""
    conn, place = kernel.conn, event.place
    if place and await read_state(conn, KEY + str(place)):
        return None
    if event.kind == "act.result" and await closed_reason(conn, event) is not None:
        return None
    tasks = await open_tasks(conn)
    if any(task.get("parent") == place for task in tasks):
        return None
    if len(tasks) >= int(constants(kernel.settings).get("max_open_tasks", 3)):
        return None
    task_place = f"space:task:{event.id}"
    opened = await kernel._emit(event, "task.opened", {"parent": place}, place=task_place)
    goal = await _goal_for(conn, event)
    task = {"place": task_place, "parent": place,
            "status": OPEN, "steps": 0, "waiting_for": None, "opened_event": opened.id,
            "goal": goal, "since": event.id}
    await write_state(conn, KEY + task_place, task, opened.id)
    return task


async def place_for(conn, event, place: str | None) -> str | None:
    """Return the task place that this event belongs to, if any."""
    if not place:
        return None
    for task in await open_tasks(conn):
        if task.get("parent") != place:
            continue
        if event.kind in CONTINUATIONS:
            return str(task["place"])
        if event.kind == "percept" and event.who and task.get("waiting_for") == event.who:
            return str(task["place"])
    return None


async def after_step(kernel, job, event, acts: Sequence[tuple[dict[str, Any], Any]]) -> None:
    """Count one step, keep a thread open while it asks, and close it when done.

    `acts` is one `(call, dispatch)` per act. An act answered to a person (`to:` a person id)
    leaves the thread waiting for that person; a later act that lands in a place while it
    waits delivers and closes the thread (DEC-8). The arguments and the receipt states are
    the facts; no label is read.
    """
    conn, place = kernel.conn, str(job.place)
    task = await read_state(conn, KEY + place)
    if not task or task.get("status") != OPEN:
        return
    steps = int(task.get("steps") or 0) + len(acts)
    held = next((found for raw, outcome in acts
                 if (found := _person_target(raw, outcome))), None)
    if held:
        return await _save(kernel, task, event, steps=steps, waiting_for=held)
    asking = bool(task.get("waiting_for"))
    if asking and any(_delivers(raw, outcome) for raw, outcome in acts):
        return await close(kernel, task, event, "done")
    if steps >= int(constants(kernel.settings).get("max_task_steps", 30)):
        return await close(kernel, task, event, "max_task_steps")
    await _save(kernel, task, event, steps=steps)


def _state(outcome: Any) -> str:
    """Return the state of one act's receipt, or "" when the act made none."""
    receipt = getattr(outcome, "receipt", None)
    return str(getattr(receipt, "state", "") or "")


def _target(raw: Any) -> str:
    """Return the `to:` one `act` call names, or "" when it names none."""
    outer = raw.get("arguments") or {}
    return str((outer.get("arguments") or {}).get("to") or "")


def _person_target(raw: Any, outcome: Any) -> str:
    """Return the person an act left the thread waiting for, or "" when it left it waiting for none."""
    target = _target(raw)
    return target if target.startswith("person:") and _state(outcome) in ("done", "held") else ""


def _delivers(raw: Any, outcome: Any) -> bool:
    """Return whether one act went out to a place while the thread waited for a person."""
    target = _target(raw)
    return bool(target) and not target.startswith("person:") \
        and _state(outcome) not in ("refused", "failed")


async def close(kernel, task: dict[str, Any], event, reason: str) -> None:
    """Report the thread's end to the parent and close it."""
    closed = await kernel._emit(event, "task.closed",
                                {"parent": task.get("parent"), "reason": reason},
                                place=str(task["place"]))
    await _save(kernel, task, closed, status="closed", reason=reason, closed_event=closed.id)


async def _save(kernel, task: dict[str, Any], event, **changes) -> None:
    value = {**task, **changes}
    await write_state(kernel.conn, KEY + str(task["place"]), value, event.id)
