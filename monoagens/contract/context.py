"""The context as items, never as text until the last hook has run (contract.md §3)."""

from __future__ import annotations

from dataclasses import dataclass, field
from datetime import datetime

from .envelope import Event

SECTIONS = ("recent", "elsewhere", "memories", "core_memory", "doing", "expectations",
            "trigger", "cards", "notes")


@dataclass(frozen=True, slots=True)
class ContextItem:
    """One piece of the context, with the events it was built from.

    `sources` are the event ids the item quotes; the privacy check admits an item by them,
    so an item without sources is treated as belonging to the wake's own place.
    """

    section: str
    text: str
    place: str = ""
    sources: tuple[int, ...] = ()
    kind: str = ""


@dataclass(frozen=True, slots=True)
class Context:
    """One wake's whole context, before and after the context hook."""

    now: datetime
    place: str
    reason: str
    items: tuple[ContextItem, ...] = field(default_factory=tuple)
    trigger: Event | None = None
