"""Appraise one event: reflexes first, then signals, threshold, and draw."""

from __future__ import annotations

from datetime import datetime
from typing import Any

from monoagens import budget
from monoagens.cognition.appraisal import Appraisal
from monoagens.cognition.rng import seeded_rng
from monoagens.cognition.signals import SIGNALS
from monoagens.cognition.situation import Situation, assemble_situation
from monoagens.config import load_soul
from monoagens.types import Event


async def appraise(event: Event, *, conn: Any = None, situation: Situation | None = None,
                   now: datetime | None = None,
                   soul: dict[str, Any] | None = None) -> Appraisal:
    """Return the decision for one event (no LLM, no writes)."""
    soul = soul if soul is not None else load_soul()
    now = now or event.ts
    config = soul.get("cognition") or {}
    u = seeded_rng(config.get("seed", 1), event.id).random()
    signals = {name: 0.0 for name in SIGNALS}
    if situation is None and conn is not None:
        situation = await assemble_situation(conn, event, now=now, soul=soul)
    if event.payload.get("addressed") is True:
        return Appraisal("wake", "addressed message", "addressed", signals, 1.0, 0.0, 1.0, u)
    if event.kind == "expectation.fired":
        return Appraisal("wake", "fired expectation", "pfc_expectation", signals,
                         1.0, 0.0, 1.0, u)
    moment = now or (situation.now if situation is not None else event.ts)
    limit = budget.daily_limit(soul)
    state = situation.state if situation is not None else {}
    used = budget.used_fraction(state.get(budget.key_for(moment, soul)), limit)
    threshold = config.get("threshold") or {}
    theta = float(threshold.get("budget") or 0.0) * used
    return Appraisal("silent", "gate score below silent threshold", None, signals,
                     0.0, theta, 0.0, u)
