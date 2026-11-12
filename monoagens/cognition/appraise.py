"""Appraise one event: the circuit breaker first, then signals, threshold and draw."""

from __future__ import annotations

import math
from datetime import datetime
from typing import Any, Mapping

from monoagens import budget, memory
from monoagens.cognition import gate
from monoagens.cognition.appraisal import Appraisal
from monoagens.cognition.rng import seeded_rng
from monoagens.cognition.signals import SIGNALS
from monoagens.cognition.situation import (Situation, assemble_situation,
                                           effective_salience)
from monoagens.prior import constants, load_settings
from monoagens.store import read_expectations
from monoagens.types import Event


def _addressed(event: Event) -> bool:
    """Return whether the event, or any message in a back-fill batch, is addressed."""
    return event.payload.get("addressed") is True or any(
        bool(message.get("addressed")) for message in event.payload.get("messages") or ())


async def _engaged(event: Event, conn: Any) -> bool:
    """Return whether this event answers an active Expectation."""
    from monoagens import expect  # imported here: prior -> identity -> prior is a cycle
    rows = await read_expectations(conn, "ex.status = 'active' AND ex.kind <> 'guard' "
                                         "AND ex.cue_event IS NOT NULL")
    # A free-text condition is judged on its reminder wake, never by the cue alone.
    return any(expect.matches_cue(row["cue_event"], event)
               and not (row["condition"] or {}).get("text") for row in rows)


async def reflex(event: Event, conn: Any = None) -> tuple[str, str] | None:
    """Return (reason, reflex) when a commitment bypasses the draw (docs/core.md §6)."""
    if event.kind == "expectation.fired":
        return "fired expectation", "pfc_expectation"
    if _addressed(event):
        return "addressed message", "addressed"
    if event.kind == "act.result":
        return "goal continued", "goal"
    if conn is not None and await _engaged(event, conn):
        return "expectation engaged", "expectation"
    return None


def _salience(signals: Mapping[str, float], weights: Mapping[str, float]) -> float:
    """Return S = clamp(sum w_k * s_k) over the signals the prior weights."""
    total = sum(weights.get(name, 0.0) * value for name, value in signals.items())
    return max(0.0, min(1.0, total))


def _reflexive(appraisal: str, reason: str, tag: str, u: float, tau: float) -> Appraisal:
    """Return the wake that bypasses the draw; the gate's terms are not used."""
    return Appraisal(decision=appraisal, reason=reason, reflex=tag, signals={},
                     s=1.0, s_eff=1.0, p=1.0, u=u, tau=tau)


async def appraise(event: Event, *, conn: Any = None, situation: Situation | None = None,
                   now: datetime | None = None, embedder: Any = None,
                   settings: dict[str, Any] | None = None) -> Appraisal:
    """Return the decision for one event (no LLM call)."""
    settings = settings if settings is not None else load_settings()
    values, params = constants(settings), gate.gate_params(settings)
    now = now or event.ts
    u = seeded_rng(values["seed"], event.id).random()
    if event.kind == "expectation.fired":
        return _reflexive("wake", "fired expectation", "pfc_expectation", u, params.tau)
    signals = {name: 0.0 for name in SIGNALS}
    if situation is None and conn is not None:
        situation = await assemble_situation(conn, event, now=now, embedder=embedder,
                                             settings=settings)
    if situation is not None:
        signals = {name: fn(situation) for name, fn in SIGNALS.items()}
    if event.payload.get("backfill"):
        sent = event.payload.get("sent_at")
        age = max(0.0, (now - datetime.fromisoformat(str(sent))).total_seconds() / 3600) \
            if sent else 0.0
        decay = math.exp(-memory.decay_d(settings) * age)
        signals = {name: value * decay for name, value in signals.items()}
    tag = await reflex(event, conn)
    if tag is not None:
        return Appraisal(decision="wake", reason=tag[0], reflex=tag[1], signals=signals,
                         s=1.0, s_eff=1.0, p=1.0, u=u, tau=params.tau)
    moment = now or (situation.now if situation is not None else event.ts)
    limit = budget.daily_limit(settings)
    state = situation.state if situation is not None else {}
    used = budget.used_fraction(state.get(budget.key_for(moment, settings)), limit)
    talk_rate = situation.talk_rate if situation is not None else 0.0
    debt = situation.promise_debt if situation is not None else 0.0
    known = situation.familiarity if situation is not None else 0.0
    h = situation.habituation if situation is not None else 0.0
    s_value = _salience(signals, params.weights)
    s_eff = effective_salience(s_value, h)
    theta_value = gate.theta(params, talk_rate=talk_rate, budget_used=used,
                             promise_debt=debt, familiarity=known)
    p = gate.probability(s_eff, theta_value, params.tau)
    numbers = {"signals": signals, "s": s_value, "s_eff": s_eff, "theta": theta_value,
               "p": p, "u": u, "tau": params.tau, "habituation": h, "talk_rate": talk_rate,
               "budget_used": used, "promise_debt": debt, "familiarity": known}
    decision, reason = gate.decide(u, p, params.note_band)
    return Appraisal(decision=decision, reason=reason, **numbers)
