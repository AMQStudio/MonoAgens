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


async def open_for(kernel, event, audience) -> dict[str, Any] | None:
    """Open a task place for one chain when it does not already have one."""
    conn, place = kernel.conn, event.place
    if place and await read_state(conn, KEY + str(place)):
        return None
    tasks = await open_tasks(conn)
    if any(task.get("parent") == place for task in tasks):
        return None
    if len(tasks) >= int(constants(kernel.settings).get("max_open_tasks", 3)):
        return None
    task_place = f"space:task:{event.id}"
    opened = await kernel._emit(event, "task.opened",
                               {"parent": place, "audience": sorted(audience)}, place=task_place)
    goal = str(event.payload.get("description") or event.payload.get("text") or "")
    task = {"place": task_place, "parent": place, "audience": sorted(audience),
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


async def after_step(kernel, job, event,
                      acts: Sequence[tuple[dict[str, Any], Mapping[str, Any]]]) -> None:
    """Count one step, keep a thread open while it asks, and close it when done.

    `acts` is one `(call, labels)` per dispatched act; the labels say whether this act
    left the thread waiting for someone, or delivered into the place it waited for.
    """
    conn, place = kernel.conn, str(job.place)
    task = await read_state(conn, KEY + place)
    if not task or task.get("status") != OPEN:
        return
    steps = int(task.get("steps") or 0) + len(acts)
    held = next((labels.get("hold_for") for _, labels in acts if labels.get("hold_for")), None)
    if held:
        return await _save(kernel, task, event, steps=steps, waiting_for=held)
    asking = bool(task.get("waiting_for"))
    if asking and any(labels.get("delivers") for _, labels in acts):
        return await close(kernel, task, event, "done")
    if steps >= int(constants(kernel.settings).get("max_task_steps", 30)):
        return await close(kernel, task, event, "max_task_steps")
    await _save(kernel, task, event, steps=steps)


async def close(kernel, task: dict[str, Any], event, reason: str) -> None:
    """Report the thread's end to the parent and close it."""
    closed = await kernel._emit(event, "task.closed",
                                {"parent": task.get("parent"), "reason": reason},
                                place=str(task["place"]))
    await _save(kernel, task, closed, status="closed")


async def _save(kernel, task: dict[str, Any], event, **changes) -> None:
    value = {**task, **changes}
    await write_state(kernel.conn, KEY + str(task["place"]), value, event.id)
