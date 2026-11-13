"""Who can perceive a place, as one fact (contract.md §4.3)."""

from __future__ import annotations

from dataclasses import dataclass, field
from datetime import datetime

from .envelope import Event

PLACE_AUDIENCE = "place.audience"


@dataclass(frozen=True, slots=True)
class Audience:
    """The actors who can perceive a place right now, and whether that is uncertain."""

    actors: frozenset[str] = field(default_factory=frozenset)
    unknown: bool = False


def audience_of(event: Event) -> Audience | None:
    """Return the audience one `place.audience` event reports, or None for any other event."""
    if event.kind != PLACE_AUDIENCE:
        return None
    payload = event.payload or {}
    if "actors" not in payload and "unknown" not in payload:
        return None
    actors = frozenset(str(actor) for actor in payload.get("actors") or ())
    return Audience(actors=actors, unknown=bool(payload.get("unknown")))


def audience_event(place: str, audience: Audience, *, ts: datetime,
                   body: str | None = None) -> Event:
    """Return the event one body writes when it first sees a place or its audience changes."""
    return Event(id=None, ts=ts, kind=PLACE_AUDIENCE, place=place, body=body,
                 payload={"actors": sorted(audience.actors), "unknown": audience.unknown})
