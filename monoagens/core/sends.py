"""The send ledger: an outgoing act is confirmed in the Log, never in a new table.

The ledger is **at-least-once**: a crash between the record (`act.out`) and the
confirmation (`act.confirmed`) can send the same message once more after the restart; it
never sends a confirmed message again and never drops a pending one silently. A tool that
produces no result and then raises closes its row with an `error`, so a failing adapter
cannot loop.
"""

from __future__ import annotations

from typing import Any

from monoagens.infra.store import event_from_row
from monoagens.types import Event

CONFIRMED = "act.confirmed"
CLOSING = (CONFIRMED, "act.result", "error")
_COLUMNS = ("id, ts, kind, who, place, body, payload, caused_by, origin_actor, schema_version")


async def confirm(kernel: Any, act: Event, capability: str, to: str | None) -> Event:
    """Append the confirmation that closes one recorded outgoing act."""
    payload = {"capability": capability, **({"to": to} if to else {})}
    return await kernel._emit(act, CONFIRMED, payload)


async def pending(conn) -> list[Event]:
    """Return the communicative acts recorded but neither confirmed nor closed."""
    cur = await conn.execute(
        f"SELECT {_COLUMNS} FROM events act WHERE act.kind = 'act.out' "
        "AND act.payload->>'communicative' = 'true' AND NOT EXISTS ("
        "SELECT 1 FROM events done WHERE done.caused_by = act.id "
        "AND done.kind = ANY(%s)) ORDER BY act.id", (list(CLOSING),))
    return [event_from_row(row) for row in await cur.fetchall()]


async def resume(kernel: Any) -> int:
    """Hand every pending act to its tool once, then close it."""
    count = 0
    for act in await pending(kernel.conn):
        capability = str(act.payload.get("capability") or "")
        arguments = dict(act.payload.get("arguments") or {})
        try:
            result = await kernel.tools.act(act, capability, arguments, event_id=act.id)
        except Exception as exc:  # the row is closed so a failing adapter cannot loop
            await kernel._record_error(act, exc)
            continue
        await confirm(kernel, act, capability, act.payload.get("to"))
        if result is not None:
            kernel.enqueue(Event(id=None, ts=kernel.clock.now(), kind="act.result", who=act.who,
                                 place=act.place, body=act.body, caused_by=act.id,
                                 origin_actor=act.origin_actor,
                                 payload={"capability": capability, "result": result}))
        count += 1
    return count
