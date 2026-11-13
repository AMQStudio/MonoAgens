"""Core-facing types: the contract's event, and the store's state row.

`Event` and the schema version come from `monoagens.contract` (contract.md §7). `Action`
is still the version 1 shape the adapters and the tool registry build; Part 4 switches them
to `monoagens.contract.Action`, which is why it is spelled here for now.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any

from monoagens.contract import SCHEMA_VERSION as CURRENT_SCHEMA_VERSION
from monoagens.contract import Event

__all__ = ["CURRENT_SCHEMA_VERSION", "Action", "Event", "StateEntry"]


@dataclass(frozen=True, slots=True)
class Action:
    """One thing an adapter should do in the world (version 1 shape, until Part 4)."""

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
