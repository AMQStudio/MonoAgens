"""Appraise one event: the circuit breaker first, then signals, threshold and draw."""

from __future__ import annotations

import math
from dataclasses import dataclass, field
from datetime import datetime
from typing import Any, Mapping

from monoagens.core import memory
from monoagens.core.gate import formula
from monoagens.core.gate.rng import seeded_rng
from monoagens.core.gate.signals import SIGNALS
from monoagens.core.gate.situation import (Situation, assemble_situation,
                                           effective_salience)
from monoagens.infra import budget
from monoagens.infra.prior import constants, load_settings
from monoagens.infra.store import read_expectations
from monoagens.types import Event


@dataclass(frozen=True, slots=True)
class Appraisal:
    """One cognition decision, ready to be written as a decision event."""

    decision: str  # "silent" | "note" | "wake"
    reason: str
    reflex: str | None = None
    signals: dict[str, float] = field(default_factory=dict)
    s: float = 0.0
    theta: float = 0.0
    p: float = 0.0
    u: float = 0.0
    s_eff: float = 0.0
    tau: float = 0.0
    habituation: float = 0.0
    talk_rate: float = 0.0
    budget_used: float = 0.0
    promise_debt: float = 0.0
    familiarity: float = 0.0

    def record(self) -> dict[str, Any]:
        """Return the decision payload that is stored in the Log."""
        return {
            "decision": self.decision,
            "reason": self.reason,
            "reflex": self.reflex,
            "signals": self.signals,
            "s": self.s,
            "s_eff": self.s_eff,
            "theta": self.theta,
            "tau": self.tau,
            "p": self.p,
            "u": self.u,
            "habituation": self.habituation,
            "talk_rate": self.talk_rate,
            "budget_used": self.budget_used,
            "promise_debt": self.promise_debt,
            "familiarity": self.familiarity,
        }


def _addressed(event: Event) -> bool:
    """Return whether the turn is given to the soul (a back-fill batch says so too)."""
    return bool(event.features.get("addressed"))


async def _engaged(event: Event, conn: Any) -> bool:
    """Return whether this event answers an active Expectation."""
    from monoagens.core import expectations  # here: prior -> identity -> prior is a cycle
    rows = await read_expectations(conn, "ex.status = 'active' AND ex.kind <> 'guard' "
                                         "AND ex.cue_event IS NOT NULL")
    # A free-text condition is judged on its reminder wake, never by the cue alone.
    return any(expectations.matches_cue(row["cue_event"], event)
               and not (row["condition"] or {}).get("text") for row in rows)


async def reflex(event: Event, conn: Any = None) -> tuple[str, str] | None:
    """Return (reason, reflex) when a commitment bypasses the draw (docs/)."""
    if event.kind == "expectation.fired":
        return "fired expectation", "pfc_expectation"
    if event.kind == "expectation.reminded":
        return "reminded", "pfc_expectation"
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
                   settings: dict[str, Any] | None = None,
                   daily_limit: float | None = None) -> Appraisal:
    """Return the decision for one event (no LLM call).

    `daily_limit` is the daily cap on tokens, injected by the kernel; 0 means no
    cap at all, so the gate's budget term stays out.
    """
    settings = settings if settings is not None else load_settings()
    values, params = constants(settings), formula.gate_params(settings)
    now = now or event.ts
    u = seeded_rng(values["seed"], event.id).random()
    if event.kind in ("expectation.fired", "expectation.reminded"):
        reason = "reminded" if event.kind == "expectation.reminded" else "fired expectation"
        return _reflexive("wake", reason, "pfc_expectation", u, params.tau)
    signals = {name: 0.0 for name in SIGNALS}
    if situation is None and conn is not None:
        situation = await assemble_situation(conn, event, now=now, embedder=embedder,
                                             settings=settings)
    if situation is not None:
        signals = {name: fn(situation) for name, fn in SIGNALS.items()}
    if event.backfill:
        happened = event.occurred_at or event.ts
        age = max(0.0, (now - happened).total_seconds() / 3600)
        decay = math.exp(-memory.decay_d(settings) * age)
        signals = {name: value * decay for name, value in signals.items()}
    tag = await reflex(event, conn)
    if tag is not None:
        return Appraisal(decision="wake", reason=tag[0], reflex=tag[1], signals=signals,
                         s=1.0, s_eff=1.0, p=1.0, u=u, tau=params.tau)
    moment = now or (situation.now if situation is not None else event.ts)
    state = situation.state if situation is not None else {}
    used = budget.used_fraction(state.get(budget.key_for(moment, settings)),
                                daily_limit or 0.0)
    talk_rate = situation.talk_rate if situation is not None else 0.0
    debt = situation.promise_debt if situation is not None else 0.0
    known = situation.familiarity if situation is not None else 0.0
    h = situation.habituation if situation is not None else 0.0
    s_value = _salience(signals, params.weights)
    s_eff = effective_salience(s_value, h)
    theta_value = formula.theta(params, talk_rate=talk_rate, budget_used=used,
                             promise_debt=debt, familiarity=known)
    p = formula.probability(s_eff, theta_value, params.tau)
    numbers = {"signals": signals, "s": s_value, "s_eff": s_eff, "theta": theta_value,
               "p": p, "u": u, "tau": params.tau, "habituation": h, "talk_rate": talk_rate,
               "budget_used": used, "promise_debt": debt, "familiarity": known}
    decision, reason = formula.decide(u, p, params.note_band)
    return Appraisal(decision=decision, reason=reason, **numbers)
