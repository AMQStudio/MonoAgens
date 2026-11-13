"""What a body is doing, told in generic terms (contract.md §4.5)."""

from __future__ import annotations

from collections.abc import Mapping
from dataclasses import dataclass, field

LIFECYCLE = ("composing", "seen", "progress", "thought")


@dataclass(frozen=True, slots=True)
class Lifecycle:
    """One thing the soul is doing, for a body to render its own way."""

    name: str
    place: str
    data: Mapping[str, object] = field(default_factory=dict)


def composing(place: str) -> Lifecycle:
    """Return the event a body shows while a reply is being written in `place`."""
    return Lifecycle("composing", place)


def seen(event_id: int, place: str) -> Lifecycle:
    """Return the event a body shows when the soul has taken in event `event_id`."""
    return Lifecycle("seen", place, {"event": event_id})


def progress(place: str, step: Mapping[str, object]) -> Lifecycle:
    """Return the event a body shows for one step of long work."""
    return Lifecycle("progress", place, dict(step))


def thought(place: str, text: str) -> Lifecycle:
    """Return the event a body shows for one thought."""
    return Lifecycle("thought", place, {"text": text})
