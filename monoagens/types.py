"""Core-facing types: the contract's event and action, and the store's state row.

`Event`, `Action` and the schema version come from `monoagens.contract` (§2.3): the core, the
middleware, the adapters and the plugins speak those types and nothing else.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any

from monoagens.contract import SCHEMA_VERSION as CURRENT_SCHEMA_VERSION
from monoagens.contract import Action, Event

__all__ = ["CURRENT_SCHEMA_VERSION", "Action", "Event", "StateEntry"]


@dataclass(frozen=True, slots=True)
class StateEntry:
    """One Status Board row with the event that last wrote it."""

    key: str
    value: dict[str, Any]
    updated_by_event: int
