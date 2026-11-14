"""The stability contract: event schema versions and event-kind namespaces (core.md §3).

The one version 1 -> 2 upcast lives here, outside the lock: a fix to how an old row is read
is a fix to this module, never a change to the contract (contract.md §7).
"""

from __future__ import annotations

import re
from collections.abc import Callable, Mapping
from dataclasses import replace
from datetime import datetime
from typing import Any

from monoagens.contract import SCHEMA_VERSION, Event
from monoagens.types import CURRENT_SCHEMA_VERSION

UPCASTERS: dict[int, Callable[[dict[str, Any]], dict[str, Any]]] = {}

_KIND = re.compile(r"^[a-z][a-z0-9_]*(\.[a-z][a-z0-9_]*)*$")


def upcast(event: Event) -> Event:
    """Return the event at the current schema version; the stored row is never rewritten."""
    version, payload = event.schema_version, event.payload
    while version < CURRENT_SCHEMA_VERSION:
        translator = UPCASTERS.get(version)
        if translator is None:
            raise ValueError(f"no upcaster from event schema version {version}")
        # A translator only adds or renames fields; unknown fields are carried along.
        payload = translator(dict(payload))
        version += 1
    if version == event.schema_version:
        return event
    return replace(event, payload=payload, schema_version=version)


def is_valid_event_kind(kind: Any) -> bool:
    """True for a lower-case dotted kind (`percept`, `act.out`, `plugin.hint`)."""
    return isinstance(kind, str) and _KIND.match(kind) is not None


def is_namespaced_event_kind(kind: Any) -> bool:
    """True for a plugin kind, which must carry its namespace (`<plugin>.<kind>`)."""
    return is_valid_event_kind(kind) and "." in kind


def plugin_kind(plugin: str, kind: str) -> str:
    """Compose the namespaced kind a plugin must publish its events under."""
    if not is_valid_event_kind(plugin) or "." in plugin:
        raise ValueError(f"plugin name must be one lower-case segment: {plugin!r}")
    if not is_valid_event_kind(kind) or kind.startswith(f"{plugin}."):
        raise ValueError(f"plugin kind must be a bare lower-case name: {kind!r}")
    return f"{plugin}.{kind}"


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
                 imported=imported, payload=payload, labels=labels,
                 schema_version=SCHEMA_VERSION)


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
