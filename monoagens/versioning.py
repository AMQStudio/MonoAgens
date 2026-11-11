"""The stability contract: event schema versions and event-kind namespaces (core.md §3)."""

from __future__ import annotations

import re
from collections.abc import Callable
from dataclasses import replace
from typing import Any

from monoagens.types import CURRENT_SCHEMA_VERSION, Event

# One translator per version: it turns a payload of version N into version N + 1.
# The first payload change registers version 1 here; until then nothing is translated.
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
    """True for a lower-case dotted kind (`percept`, `act.out`, `telegram.say`)."""
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
