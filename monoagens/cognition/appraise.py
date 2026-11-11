"""Appraise one event: reflexes first, then signals, threshold, and draw."""

from __future__ import annotations

import math
from datetime import datetime
from typing import Any

from monoagens import budget, memory
from monoagens.cognition.appraisal import Appraisal
from monoagens.cognition.rng import seeded_rng
from monoagens.cognition.signals import SIGNALS
from monoagens.cognition.situation import Situation, assemble_situation
from monoagens.prior import constants, load_settings
from monoagens.types import Event


def _addressed(event: Event) -> bool:
    """Return whether the event, or any message in a back-fill batch, is addressed."""
    return event.payload.get("addressed") is True or any(
        bool(message.get("addressed")) for message in event.payload.get("messages") or ())


async def appraise(event: Event, *, conn: Any = None, situation: Situation | None = None,
                   now: datetime | None = None, embedder: Any = None,
                   settings: dict[str, Any] | None = None) -> Appraisal:
    """Return the decision for one event (no LLM call)."""
    settings = settings if settings is not None else load_settings()
    values = constants(settings)
    now = now or event.ts
    u = seeded_rng(values["seed"], event.id).random()
    signals = {name: 0.0 for name in SIGNALS}
    if situation is None and conn is not None:
        situation = await assemble_situation(conn, event, now=now, embedder=embedder,
                                             settings=settings)
    if event.kind == "expectation.fired":
        return Appraisal("wake", "fired expectation", "pfc_expectation", signals,
                         1.0, 0.0, 1.0, u)
    if situation is not None:
        signals = {name: fn(situation) for name, fn in SIGNALS.items()}
    if event.payload.get("backfill"):
        sent = event.payload.get("sent_at")
        age = max(0.0, (now - datetime.fromisoformat(str(sent))).total_seconds() / 3600) \
            if sent else 0.0
        decay = math.exp(-memory.decay_d(settings) * age)
        signals = {name: value * decay for name, value in signals.items()}
    if _addressed(event):
        return Appraisal("wake", "addressed message", "addressed", signals, 1.0, 0.0, 1.0, u)
    moment = now or (situation.now if situation is not None else event.ts)
    limit = budget.daily_limit(settings)
    state = situation.state if situation is not None else {}
    used = budget.used_fraction(state.get(budget.key_for(moment, settings)), limit)
    theta = float(values["budget_weight"]) * used
    return Appraisal("silent", "gate score below silent threshold", None, signals,
                     0.0, theta, 0.0, u)
