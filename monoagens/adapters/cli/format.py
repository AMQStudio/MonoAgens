"""One typed line becomes one neutral event; the terminal is the owner's direct chat."""

from __future__ import annotations

from monoagens.infra.clock import Clock
from monoagens.types import Event

PLACE = "space:cli"
OWNER = "person:owner"


def event(text: str, *, clock: Clock, name: str = "cli") -> Event:
    """Build the percept of one typed line (the owner typing in their own terminal)."""
    return Event(id=None, ts=clock.now(), kind="percept", who=OWNER, place=PLACE, body=name,
                 payload={"description": text, "addressed": True, "place_kind": "direct"})
