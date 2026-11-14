"""The context as items: one builder per section, each carrying the events it quotes.

The context reaches the reasoning as items, never as text, until the last hook has run
(contract.md §3). A middleware rule drops or annotates an item; the core renders the text of
what is left. Every builder here reads the envelope and the ledger only.
"""

from __future__ import annotations

from collections.abc import Mapping, Sequence
from typing import Any

from monoagens.contract import ContextItem
from monoagens.core.kinds import RECENT_KINDS
from monoagens.types import Event

SHOWN_KINDS = (*RECENT_KINDS, "intent")  # an intent stays pending on that place's desk


def recent_items(events: Sequence[Event], *, zone: Any, name: str,
                 section: str = "recent") -> tuple[ContextItem, ...]:
    """Return one item per line of a place's recent events, newest last."""
    items = []
    for event in events:
        line = line_of(event, zone=zone, name=name)
        if not line:
            continue
        prefix = f"[{event.place}] " if section == "elsewhere" and event.place else ""
        items.append(ContextItem(section=section, text=f"{prefix}{line}",
                                 place=str(event.place or ""),
                                 sources=(int(event.id),) if event.id else (),
                                 kind=event.kind))
    return tuple(items)


def line_of(event: Event, *, zone: Any, name: str) -> str:
    """Return one recent line: its local time, its speaker and its description."""
    if event.kind not in SHOWN_KINDS or not event.description:
        return ""
    return f"{event.ts.astimezone(zone):%H:%M} {speaker_of(event, name)}: {event.description}"


def speaker_of(event: Event, name: str) -> str:
    """Return who the line is attributed to, from the envelope alone."""
    if event.kind == "act.out":
        return name
    if event.kind == "expectation.fired":
        return "expectation"
    if event.kind == "intent":
        return "intent"
    return str(event.who or "?")


def memory_items(memories: Sequence[Any]) -> tuple[ContextItem, ...]:
    """Return one item per retrieved memory, sourced from the events it was derived from."""
    return tuple(ContextItem(section="memories", text=str(memory),
                             place=str(getattr(memory, "place", "") or ""),
                             sources=tuple(int(item) for item in getattr(memory, "sources", ()) or ()
                                           if item is not None))
                 for memory in memories if str(memory))


def core_memory_items(memory: Mapping[str, Any] | None) -> tuple[ContextItem, ...]:
    """Return the core memory as one item, sourced from the events it summarizes."""
    text = str((memory or {}).get("text") or "")
    if not text:
        return ()
    sources = tuple(int(item) for item in (memory or {}).get("derived_from") or ()
                    if item is not None)
    return (ContextItem(section="core_memory", text=text, sources=sources),)


def doing_items(entries: Sequence[tuple[str, str | None, int | None]]) -> tuple[ContextItem, ...]:
    """Return one item per line of every chain in progress, with the event it came from."""
    return tuple(ContextItem(section="doing", text=line, place=str(place or ""),
                             sources=(int(event_id),) if event_id else (), kind="chain")
                 for line, place, event_id in entries)


def expectation_items(rows: Sequence[tuple[str, int | None]]) -> tuple[ContextItem, ...]:
    """Return one item per active expectation, sourced from the event it came from."""
    return tuple(ContextItem(section="expectations", text=line, kind="expectation",
                             sources=(source,) if source else ())
                 for line, source in rows)


def trigger_item(event: Event | None) -> tuple[ContextItem, ...]:
    """Return the event that woke the soul as one item."""
    if event is None:
        return ()
    text = trigger_text(event)
    return (ContextItem(section="trigger", text=text, place=str(event.place or ""),
                        sources=(int(event.id),) if event.id else (), kind=event.kind),)


def trigger_text(event: Event | None) -> str:
    """Return the line that woke the soul, read from the envelope."""
    if event is None:
        return "(no trigger event)"
    if event.kind == "expectation.reminded":
        where = f"[{event.place}] " if event.place else ""
        return f"{where}{event.who or '?'}: {event.description}" if event.description else ""
    if event.description:
        return f"{event.who or '?'}: {event.description}"
    return f"[{event.kind}]"


__all__ = ["SHOWN_KINDS", "core_memory_items", "doing_items", "elsewhere_items",
           "expectation_items", "line_of", "memory_items", "recent_items", "speaker_of",
           "trigger_item", "trigger_text"]


def elsewhere_items(events: Sequence[Event], *, zone: Any, name: str) -> tuple[ContextItem, ...]:
    """Return one item per line of the other places, each marked with its own place."""
    return recent_items(events, zone=zone, name=name, section="elsewhere")
