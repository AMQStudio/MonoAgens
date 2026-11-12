"""Core types: Event, Action, and StateEntry."""

from __future__ import annotations

from dataclasses import dataclass, field
from datetime import datetime
from typing import Any

# The only kinds the PFC's <recent> section may show; internal bookkeeping stays out.
RECENT_KINDS = ("percept", "act.out", "expectation.fired")

# The payload schema every event is written with; readers upcast older ones (core.md §3).
CURRENT_SCHEMA_VERSION = 1


@dataclass(frozen=True, slots=True)
class Event:
    """One immutable entry in the Log."""

    id: int | None
    ts: datetime
    kind: str
    who: str | None = None
    place: str | None = None
    body: str | None = None
    payload: dict[str, Any] = field(default_factory=dict)
    caused_by: int | None = None
    origin_actor: str | None = None
    schema_version: int = CURRENT_SCHEMA_VERSION


@dataclass(frozen=True, slots=True)
class Action:
    """One thing an adapter should do in the world."""

    kind: str
    place: str
    payload: dict[str, Any] = field(default_factory=dict)
    adapter: str | None = None
    event_id: int | None = field(default=None, compare=False)


@dataclass(frozen=True, slots=True)
class StateEntry:
    """One Status Board row with the event that last wrote it."""

    key: str
    value: dict[str, Any]
    updated_by_event: int
