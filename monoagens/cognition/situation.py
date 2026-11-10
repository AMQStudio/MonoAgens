"""The Situation: the single read-only view a signal is allowed to see."""

from __future__ import annotations

from dataclasses import dataclass, field
from datetime import datetime, timezone
from typing import Any, Mapping, Sequence
from zoneinfo import ZoneInfo

from monoagens.store import read_event_vector, read_last_events, read_state
from monoagens.types import RECENT_KINDS, Event


@dataclass(frozen=True, slots=True)
class Situation:
    """One event with its vector, its recent context, and relevant state values."""

    event: Event
    vector: list[float] | None = None
    recent: Sequence[Event] = ()
    memory_candidates: Sequence[Any] = ()
    state: Mapping[str, Any] = field(default_factory=dict)
    now: datetime | None = None


async def assemble_situation(conn, event: Event, *, embedder: Any = None,
                             now: datetime | None = None,
                             soul: dict[str, Any] | None = None) -> Situation:
    """Assemble once what the signals may read about this event."""
    soul = soul or {}
    now = now or datetime.now(timezone.utc)
    zone = ZoneInfo(str(soul.get("timezone") or "UTC"))
    memory_soul = soul.get("memory") or {}
    recent = await read_last_events(conn, event.place,
                                    limit=int(memory_soul.get("recent_events", 20)),
                                    kinds=RECENT_KINDS) if event.place else []
    vector = await read_event_vector(conn, event.id) if event.id is not None else None
    keys = ["cognition:weights"]
    if event.place:
        keys.append(f"habit:{event.place}:{event.who}")
    keys.append(f"budget:{now.astimezone(zone).date().isoformat()}")
    state: dict[str, Any] = {}
    for key in keys:
        value = await read_state(conn, key)
        if value is not None:
            state[key] = value
    return Situation(event=event, vector=vector, recent=tuple(recent), state=state, now=now)
