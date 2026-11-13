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


def upcast_event(row: Mapping[str, Any]) -> Event:
    """Return one stored row as a current-version `Event`, upcasting a version 1 row.

    A version 1 row carries its meaning inside `payload` (`description`, `text`, `sent_at`,
    `addressed`, `backfill`, `imported`); those keys are read here, once, and never again.
    Stored events are never rewritten (contract.md §9). A row from a newer version than this
    reader knows is refused instead of half-read.
    """
    version = int(row.get("schema_version") or 1)
    if version > SCHEMA_VERSION:
        raise ValueError(f"event schema_version {version} is newer than {SCHEMA_VERSION}")
    ts = row["ts"]
    kind = str(row["kind"])
    payload = dict(row.get("payload") or {})
    description = str(row.get("description") or "")
    features = dict(row.get("features") or {})
    occurred_at = _as_time(row.get("occurred_at")) or ts
    backfill = bool(row.get("backfill") or False)
    imported = bool(row.get("imported") or False)
    labels = dict(row.get("labels") or {})
    if version == 1:
        description = description or _v1_description(kind, payload)
        occurred_at = _as_time(payload.get("sent_at")) or occurred_at
        if payload.get("addressed") is True:
            features["addressed"] = 1
        backfill = backfill or payload.get("backfill") is True
        imported = imported or payload.get("imported") is True
        labels = {}
    return Event(id=row.get("id"), ts=ts, kind=kind, occurred_at=occurred_at,
                 description=description, who=row.get("who"), place=row.get("place"),
                 body=row.get("body"), caused_by=row.get("caused_by"),
                 origin_actor=row.get("origin_actor"), features=features, backfill=backfill,
                 imported=imported, payload=payload, labels=labels, schema_version=version)


def _v1_description(kind: str, payload: Mapping[str, Any]) -> str:
    """Read the one description a version 1 payload holds."""
    if kind == "act.out":
        arguments = payload.get("arguments")
        text = arguments.get("text") if isinstance(arguments, Mapping) else None
        if text:
            return str(text)
    return str(payload.get("description") or payload.get("text") or "")


def _as_time(value: Any) -> datetime | None:
    """Return one stored time as a datetime, or None when it is absent or unreadable."""
    if isinstance(value, datetime):
        return value
    if isinstance(value, str) and value:
        try:
            return datetime.fromisoformat(value)
        except ValueError:
            return None
    return None
