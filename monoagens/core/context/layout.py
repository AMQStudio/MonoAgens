"""The desk: what the reasoning sees in one wake, rebuilt for every wake (core.md §7)."""

from __future__ import annotations

from dataclasses import dataclass, replace
from datetime import datetime
from typing import TYPE_CHECKING, Any, Sequence

import psycopg

from monoagens.core.context.working import doing_lines
from monoagens.core.memory.cards import canonical, merge_map
from monoagens.core.memory.retrieval import recent_lines
from monoagens.infra.prior import constants
from monoagens.infra.store import active_expectation_lines, read_last_events, read_state
from monoagens.core.kinds import RECENT_KINDS
from monoagens.types import Event

if TYPE_CHECKING:  # the parts render the desk; importing them here would be a cycle
    from monoagens.core.context.parts import Part


@dataclass(frozen=True, slots=True)
class PFCContext:
    """Everything the reasoning sees for one wake."""

    now: datetime
    place: str
    reason: str
    core_memory: str = ""
    people: Sequence[str] = ()
    expectations: Sequence[str] = ()
    memories: Sequence[str] = ()
    withheld: Sequence[tuple[str, int]] = ()
    recent: Sequence[Event] = ()
    recent_elsewhere: Sequence[Event] = ()
    doing: Sequence[str] = ()
    trigger: Event | None = None
    last_interaction: datetime | None = None
    capabilities: Sequence[dict[str, Any]] = ()
    results: Sequence[str] = ()
    blocks: Sequence[Part] = ()  # the middleware's volatile additions, in `order`


def withheld_note(context: PFCContext) -> str:
    """Return the D56 line: how many relevant private items are absent, and from where.

    It names only the count and the source label the middleware attached; it never names
    the content.
    """
    if not context.withheld:
        return ""
    parts = [f"{count} thing{'s' if count != 1 else ''} you know from {label}"
             for label, count in context.withheld]
    total = sum(count for _, count in context.withheld)
    verb = "are" if total != 1 else "is"
    return f"{' and '.join(parts)} {verb} not shown here"


async def build_context(conn: psycopg.AsyncConnection[Any], event: Event, place: str,
                        reason: str, *, now: datetime, settings: dict[str, Any],
                        memories: Sequence[str], capabilities: Sequence[dict[str, Any]],
                        ) -> PFCContext:
    """Lay out the desk for one wake: recent events, memories, state and expectations."""
    values = constants(settings)
    merges = await merge_map(conn)
    relabel = lambda item: replace(item, who=merges[item.who]) if item.who in merges else item
    recent = [relabel(seen) for seen in await read_last_events(
        conn, place, limit=int(values["recent_events"]), kinds=(*RECENT_KINDS, "intent"))]
    recent_elsewhere = [relabel(seen) for seen in await recent_lines(
        conn, place, limit=int(values["recent_elsewhere"]))]
    core = await read_state(conn, "core_memory")
    task = await read_state(conn, "task:" + place) if place.startswith("space:task:") else None
    source_who = await canonical(conn, event.who)
    shown_event = relabel(event)
    doing = await doing_lines(conn, place, task, values, trigger=shown_event)
    last = None
    for key in (f"last_spoke:{place}", f"last_seen:{source_who}"):
        value = await read_state(conn, key)
        if value and value.get("ts"):
            last = datetime.fromisoformat(value["ts"])
            break
    return PFCContext(
        now=now, place=place, reason=reason,
        core_memory=str((core or {}).get("text") or ""),
        expectations=await active_expectation_lines(conn, place), memories=memories,
        recent=[seen for seen in recent if seen.id != shown_event.id],
        recent_elsewhere=recent_elsewhere, doing=doing, trigger=shown_event,
        last_interaction=last, capabilities=capabilities)
