"""The desk: what the reasoning sees in one wake, rebuilt for every wake.

The desk is a tuple of items with their sources, never text: the context hook may drop or
annotate one, and the parts render whatever is left.
"""

from __future__ import annotations

from dataclasses import dataclass, replace
from datetime import datetime
from typing import Any, Sequence

import psycopg

from monoagens.contract import ContextItem
from monoagens.core.context import items as builders
from monoagens.core.context.working import doing_entries
from monoagens.core.expectations import tz
from monoagens.core.memory.cards import canonical, merge_map
from monoagens.core.memory.retrieval import recent_lines
from monoagens.core.kinds import RECENT_KINDS
from monoagens.infra.prior import constants
from monoagens.infra.store import active_expectations, read_last_events, read_state
from monoagens.types import Event


@dataclass(frozen=True, slots=True)
class PFCContext:
    """Everything the reasoning sees for one wake."""

    now: datetime
    place: str
    reason: str
    items: tuple[ContextItem, ...] = ()
    self_text: str = ""
    trigger: Event | None = None
    last_interaction: datetime | None = None
    capabilities: Sequence[dict[str, Any]] = ()
    results: Sequence[str] = ()


async def build_context(conn: psycopg.AsyncConnection[Any], event: Event, place: str,
                        reason: str, *, now: datetime, settings: dict[str, Any],
                        memories: Sequence[Any], capabilities: Sequence[dict[str, Any]],
                        ) -> PFCContext:
    """Lay out the desk for one wake: recent items, memories, done work and expectations."""
    values = constants(settings)
    zone = tz(settings)
    name = str((settings.get("identity") or {}).get("name") or "")
    merges = await merge_map(conn)
    relabel = lambda item: replace(item, who=merges[item.who]) if item.who in merges else item
    recent = [relabel(seen) for seen in await read_last_events(
        conn, place, limit=int(values["recent_events"]), kinds=(*RECENT_KINDS, "intent"))]
    elsewhere = [relabel(seen) for seen in await recent_lines(
        conn, place, limit=int(values["recent_elsewhere"]))]
    core = await read_state(conn, "core_memory")
    task = await read_state(conn, "task:" + place) if place.startswith("space:task:") else None
    source_who = await canonical(conn, event.who)
    shown_event = relabel(event)
    doing = await doing_entries(conn, place, task, values, trigger=shown_event)
    last = None
    for key in (f"last_spoke:{place}", f"last_seen:{source_who}"):
        value = await read_state(conn, key)
        if value and value.get("ts"):
            last = datetime.fromisoformat(value["ts"])
            break
    shown = [seen for seen in recent if seen.id != shown_event.id]
    return PFCContext(
        now=now, place=place, reason=reason,
        items=(*builders.core_memory_items(core),
               *builders.doing_items(doing),
               *builders.expectation_items(await active_expectations(conn, place)),
               *builders.memory_items(memories),
               *builders.recent_items(shown, zone=zone, name=name),
               *builders.elsewhere_items(elsewhere, zone=zone, name=name),
               *builders.trigger_item(shown_event)),
        trigger=shown_event, last_interaction=last, capabilities=capabilities)
