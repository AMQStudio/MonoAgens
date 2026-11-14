"""The event envelope every layer shares (contract.md §3).

Nothing in this module decides what an event means; it carries the fields the core may
read (ids, times, kind, description, features, flags) and the two opaque mappings the
middleware and the adapters use for their own data.
"""

from __future__ import annotations

from collections.abc import Mapping
from dataclasses import dataclass, field
from datetime import datetime
from typing import Any

SCHEMA_VERSION = 2

# The only feature names the core's signals read (contract.md §3).
FEATURES = ("addressed", "named", "addressed_other", "audience_size")


@dataclass(frozen=True, slots=True)
class Event:
    """One immutable entry in the Log."""

    id: int | None
    ts: datetime
    kind: str
    occurred_at: datetime | None = None
    description: str = ""
    who: str | None = None
    place: str | None = None
    body: str | None = None
    caused_by: int | None = None
    origin_actor: str | None = None
    features: Mapping[str, float] = field(default_factory=dict)
    backfill: bool = False
    imported: bool = False
    payload: Mapping[str, Any] = field(default_factory=dict)
    labels: Mapping[str, Any] = field(default_factory=dict)
    schema_version: int = SCHEMA_VERSION

    def __post_init__(self) -> None:
        """Treat a missing `occurred_at` as "it happened when it was recorded"."""
        if self.occurred_at is None:
            object.__setattr__(self, "occurred_at", self.ts)
