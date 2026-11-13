"""The gate formula: one place where the prior's knobs become theta, tau, gamma
and weights."""

from __future__ import annotations

import math
from dataclasses import dataclass
from datetime import timedelta
from typing import Any, Mapping

from monoagens.core import memory
from monoagens.infra.prior import constants, signal_weights


def between(pair: Any, knob: Any) -> float:
    """Map a knob in [0, 1] onto one constant pair (knob 0 -> first, 1 -> second)."""
    low, high = pair
    return float(low) + (float(high) - float(low)) * float(knob or 0.0)


def sigmoid(x: float) -> float:
    """Return sigma(x) = 1 / (1 + e^(-x)), clamped so no call overflows."""
    return 1.0 / (1.0 + math.exp(-max(-60.0, min(60.0, x))))


@dataclass(frozen=True, slots=True)
class GateParams:
    """Every number the gate needs, derived from the prior in one place."""

    theta_base: float
    tau: float
    talk_weight: float
    budget_weight: float
    promise_weight: float
    reserve: float
    note_band: float
    gamma: float
    talk_window_minutes: float
    weights: Mapping[str, float]


def gate_params(settings: Mapping[str, Any]) -> GateParams:
    """Map the prior's knobs and constants to the gate (docs/core-math.md §3, §15)."""
    knobs = settings.get("knobs") or {}
    values = constants(settings)
    return GateParams(
        theta_base=between(values["talkativeness_theta_base"], knobs.get("talkativeness")),
        tau=between(values["spontaneity_tau"], knobs.get("spontaneity")),
        talk_weight=float(knobs.get("restraint_after_speaking") or 0.0),
        budget_weight=float(values["budget_weight"]),
        promise_weight=float(values["promise_debt_weight"]),
        reserve=float(knobs.get("reserve_when_unfamiliar") or 0.0),
        note_band=float(values["note_band"]),
        gamma=between(values["boredom_gamma_per_minute"], knobs.get("boredom_with_repeats")),
        talk_window_minutes=float(values["talk_rate_window_minutes"]),
        weights=signal_weights(settings),
    )


def theta(params: GateParams, *, talk_rate: float, budget_used: float,
          promise_debt: float, familiarity: float) -> float:
    """Return theta = base + a*talk_rate + b*budget - c*debt + r*(1 - familiarity)."""
    return (params.theta_base + params.talk_weight * max(0.0, talk_rate)
            + params.budget_weight * budget_used - params.promise_weight * promise_debt
            + params.reserve * (1.0 - familiarity))


def probability(s_eff: float, theta_value: float, tau: float) -> float:
    """Return P = sigma((S_eff - theta) / tau), with tau floored at a tiny value."""
    return sigmoid((s_eff - theta_value) / (tau if tau > 0 else 1e-6))


def decide(u: float, p: float, note_band: float) -> tuple[str, str]:
    """Return (decision, reason) for one draw: go, silent, or a note."""
    if u < p:
        return "wake", "gate opened: u < P"
    if p < note_band:
        return "silent", "gate score below silent threshold"
    return "note", "gate score in the note band, no go"


def familiarity(activation: float) -> float:
    """Return familiarity = sigma(B_ctx) from ACT-R base-level activation."""
    return sigmoid(activation)


@dataclass(frozen=True, slots=True)
class GateInputs:
    """The three log-derived terms of theta for one event."""

    talk_rate: float = 0.0
    promise_debt: float = 0.0
    familiarity: float = 0.0


async def inputs(conn: Any, event: Any, *, now: Any,
                 settings: Mapping[str, Any]) -> GateInputs:
    """Read talk rate, promise debt and familiarity from the Log at `now`."""
    if conn is None or not event.place:
        return GateInputs()
    params = gate_params(settings)
    since = now - timedelta(minutes=params.talk_window_minutes)
    spoke, heard, people = await (await conn.execute(
        "SELECT count(*) FILTER (WHERE kind = 'act.out'), "
        "count(*) FILTER (WHERE kind = 'percept'), "
        "count(DISTINCT who) FILTER (WHERE kind = 'percept') "
        "FROM events WHERE place = %s AND ts >= %s AND kind IN ('percept', 'act.out')",
        (event.place, since))).fetchone()
    share = int(spoke or 0) / max(1, int(spoke or 0) + int(heard or 0))
    audience = int(event.features.get("audience_size") or 0) or int(people or 0) + 1
    talk_rate = max(0.0, share - 1.0 / audience)
    due, total = (await (await conn.execute(
        "SELECT count(*) FILTER (WHERE next_check_at <= %s), count(*) FROM expectations "
        "WHERE status = 'active' AND kind = 'promise'",
        (now + timedelta(hours=1),))).fetchone())
    debt = int(due or 0) / max(1, int(total or 0))
    seen, first = (await (await conn.execute(
        "SELECT count(*), min(ts) FROM events WHERE place = %s "
        "AND kind IN ('percept', 'act.out')", (event.place,))).fetchone())
    known = 0.0
    if int(seen or 0) and first is not None:
        age = max(0.0, (now - first).total_seconds() / 3600.0)
        known = familiarity(memory.base_level(int(seen) - 1, age, memory.decay_d(settings)))
    return GateInputs(talk_rate=talk_rate, promise_debt=debt, familiarity=known)
