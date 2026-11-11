"""The Situation: the single read-only view a signal is allowed to see."""

from __future__ import annotations

from dataclasses import dataclass, field
from datetime import datetime
from typing import Any, Mapping, Sequence
from zoneinfo import ZoneInfo

from monoagens import memory
from monoagens.prior import constants
from monoagens.store import (read_event_vector, read_expectations, read_last_events,
                             read_state, write_state)
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
    self_name: str = ""
    references: Sequence[Sequence[float]] = ()
    core_vector: list[float] | None = None
    promise_vectors: Sequence[Sequence[float]] = ()
    best_activation: float = 0.0
    closeness: float = 0.0
    habituation: float = 0.0
    other_selected: bool = False
    soul_answered: bool = False
    fok_threshold: float = 0.0
    fok_scale: float = 1.0

    @property
    def text(self) -> str:
        """Return the human-readable text of the triggering event."""
        payload = self.event.payload
        return str(payload.get("description") or payload.get("text") or "")


def boredom_gamma(settings: Mapping[str, Any]) -> float:
    """Map boredom_with_repeats to gamma per minute (0 -> 0.99, 1 -> 0.8)."""
    low, high = constants(settings)["boredom_gamma_per_minute"]
    knob = float((settings.get("knobs") or {}).get("boredom_with_repeats") or 0.0)
    return float(low) + (float(high) - float(low)) * knob


def habituate(stored: Mapping[str, Any] | None, now: datetime, gamma: float) -> float:
    """Return the next h for one (place, who): h * gamma^minutes + 1."""
    if not stored:
        return 1.0
    minutes = 0.0
    if stored.get("ts"):
        elapsed = now - datetime.fromisoformat(str(stored["ts"]))
        minutes = max(0.0, elapsed.total_seconds() / 60.0)
    return float(stored.get("h") or 0.0) * gamma**minutes + 1.0


def effective_salience(s: float, h: float) -> float:
    """Return S / (1 + h), the salience left after habituation."""
    return max(0.0, min(1.0, s)) / (1.0 + max(0.0, h))


def _floor_flags(event: Event, recent: Sequence[Event]) -> tuple[bool, bool]:
    """Return whether another person was selected and whether the soul just answered."""
    before = [seen for seen in recent if seen.id != event.id]
    soul_answered = bool(before) and before[-1].kind == "act.out"
    payload = event.payload
    targets = payload.get("addressed_to") or (payload.get("data") or {}).get("mentions") or ()
    if isinstance(targets, str):
        targets = (targets,)
    other_selected = any(str(target) != str(event.who) for target in targets)
    return other_selected, soul_answered


async def _vectors(conn: Any, settings: Mapping[str, Any],
                   embedder: Any) -> tuple[list[list[float]], list[list[float]],
                                           list[list[float]]]:
    """Embed core memory, active promises and reference sentences in one call."""
    language = str((settings.get("identity") or {}).get("language") or "en")
    references = [str(sentence) for sentence in
                  (settings.get("references") or {}).get(language) or []]
    core = str(((await read_state(conn, "core_memory")) or {}).get("text") or "")
    rows = await read_expectations(conn, "ex.status = 'active' AND ex.kind = 'promise'")
    promises = [str(row.get("about") or "") for row in rows]
    promises = [about for about in promises if about.strip()]
    texts = ([core] if core.strip() else []) + promises + references
    if embedder is None or not texts:
        return [], [], []
    vectors = await embedder.embed(texts)
    start = 1 if core.strip() else 0
    return (vectors[start:start + len(promises)], vectors[start + len(promises):],
            [vectors[0]] if core.strip() else [])


async def assemble_situation(conn, event: Event, *, now: datetime,
                             embedder: Any = None,
                             settings: dict[str, Any] | None = None) -> Situation:
    """Assemble once what the signals may read about this event (at `now`)."""
    settings = settings or {}
    values = constants(settings)
    zone = ZoneInfo(str(settings.get("timezone") or "UTC"))
    recent = await read_last_events(conn, event.place,
                                    limit=int(values["recent_events"]),
                                    kinds=RECENT_KINDS) if event.place else []
    vector = await read_event_vector(conn, event.id) if event.id is not None else None
    state: dict[str, Any] = {}
    keys = ["cognition:weights"]
    if event.place:
        keys.append(f"habit:{event.place}:{event.who}")
    keys.append(f"budget:{now.astimezone(zone).date().isoformat()}")
    for key in keys:
        value = await read_state(conn, key)
        if value is not None:
            state[key] = value
    habituation = 0.0
    if event.place:
        habit_key = f"habit:{event.place}:{event.who}"
        habituation = habituate(state.get(habit_key), now, boredom_gamma(settings))
        state[habit_key] = {"h": habituation, "ts": now.isoformat()}
        if event.id is not None:
            await write_state(conn, habit_key, state[habit_key], event.id)
    person = await read_state(conn, f"person:{event.who}") if event.who else None
    closeness = max(0.0, min(1.0, float((person or {}).get("closeness") or 0.0)))
    promise_vectors, reference_vectors, core_vectors = await _vectors(
        conn, settings, embedder)
    activation = await memory.best_activation(conn, embedder, event, now=now,
                                              settings=settings) if event.id else 0.0
    other_selected, soul_answered = _floor_flags(event, recent)
    calibration = settings.get("calibration") or {}
    return Situation(
        event=event, vector=vector, recent=tuple(recent), state=state, now=now,
        self_name=str((settings.get("identity") or {}).get("name") or ""),
        references=tuple(reference_vectors), core_vector=core_vectors[0] if core_vectors else None,
        promise_vectors=tuple(promise_vectors), best_activation=activation,
        closeness=closeness, habituation=habituation, other_selected=other_selected,
        soul_answered=soul_answered,
        fok_threshold=float(calibration.get("fok_threshold") or 0.0),
        fok_scale=float(calibration.get("fok_scale") or 1.0),
    )
