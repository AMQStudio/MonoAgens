"""Core types: Event, Action, and StateEntry."""

from __future__ import annotations

from dataclasses import dataclass, field
from datetime import datetime
from typing import Any

# The only kinds the PFC's <recent> section may show; internal bookkeeping stays out.
RECENT_KINDS = ("percept", "act.out", "expectation.fired")


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


@dataclass(frozen=True, slots=True)
class Action:
    """One thing a body should do in the world."""

    kind: str
    place: str
    payload: dict[str, Any] = field(default_factory=dict)
    body: str | None = None


@dataclass(frozen=True, slots=True)
class StateEntry:
    """One Status Board row with the event that last wrote it."""

    key: str
    value: dict[str, Any]
    updated_by_event: int
