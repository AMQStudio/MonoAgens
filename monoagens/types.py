"""Core types: Event, Action, and StateEntry."""
from __future__ import annotations
from dataclasses import dataclass, field
from datetime import datetime
from typing import Any

@dataclass(frozen=True, slots=True)
class Event:
    id: int | None
    ts: datetime
    kind: str
    who: str | None = None
    place: str | None = None
    body: str | None = None
    payload: dict[str, Any] = field(default_factory=dict)
    caused_by: int | None = None

@dataclass(frozen=True, slots=True)
class Action:
    kind: str
    place: str
    payload: dict[str, Any] = field(default_factory=dict)
    body: str | None = None

@dataclass(frozen=True, slots=True)
class StateEntry:
    key: str
    value: dict[str, Any]
    updated_by_event: int
