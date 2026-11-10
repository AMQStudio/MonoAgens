"""The gate: decides whether an event is worth waking the PFC (no LLM)."""

from __future__ import annotations

from dataclasses import dataclass
from datetime import datetime, timedelta
from typing import Any, Mapping

import psycopg

from monoagens.config import load_soul
from monoagens.store import count_recent_events, read_state
from monoagens.types import Event

CROWD_FULL_MESSAGES = 10.0  # count at which the crowd feature saturates


@dataclass(frozen=True, slots=True)
class GateConfig:
    weights: Mapping[str, float]
    silent_band: float
    wake_band: float
    cooldown_minutes: float
    crowd_window_minutes: float

    @classmethod
    def from_soul(cls, soul: Mapping[str, Any]) -> GateConfig:
        gate = soul.get("gate") or {}
        try:
            return cls({k: float(v) for k, v in gate["weights"].items()}, float(gate["band"]["silent"]),
                       float(gate["band"]["wake"]), float(gate["cooldown_minutes"]),
                       float(gate["crowd_window_minutes"]))
        except (KeyError, TypeError, AttributeError) as exc:
            raise ValueError(f"soul.yaml gate section is incomplete: {exc}") from exc


@dataclass(frozen=True, slots=True)
class Verdict:
    decision: str  # "silent" | "note" | "wake"
    score: float
    breakdown: dict[str, Any]
    reason: str

    def payload(self) -> dict[str, Any]:
        return {"score": self.score, "breakdown": self.breakdown, "reason": self.reason}


class Gate:
    def __init__(self, config: GateConfig) -> None:
        self.config = config

    @classmethod
    def from_soul(cls, soul: Mapping[str, Any] | None = None) -> Gate:
        return cls(GateConfig.from_soul(soul if soul is not None else load_soul()))

    async def judge(self, conn: psycopg.AsyncConnection[Any], event: Event, *, now: datetime,
                    surprise: float = 0.0) -> Verdict:
        """Score one event. Addressed events always wake and pay no penalties."""
        if event.payload.get("addressed") is True:
            breakdown = {"addressed": 1.0, "surprise": 0.0, "cooldown_penalty": 0.0,
                         "crowd_penalty": 0.0, "score": 1.0}
            return Verdict("wake", 1.0, breakdown, "addressed message")
        features = {"surprise": _clamp(surprise),
                    "cooldown_penalty": await self._cooldown(conn, event, now),
                    "crowd_penalty": await self._crowd(conn, event, now)}
        contributions = {name: _clamp(value) * float(self.config.weights.get(name, 0.0))
                         for name, value in features.items()}
        score = _clamp(sum(contributions.values()))
        breakdown = {"addressed": 0.0, **features, "contributions": contributions, "score": score}
        if score < self.config.silent_band:
            return Verdict("silent", score, breakdown, "gate score below silent threshold")
        if score < self.config.wake_band:
            return Verdict("note", score, breakdown, "gate score in note band")
        return Verdict("wake", score, breakdown, f"gate score {score:.2f} >= {self.config.wake_band}")

    async def _cooldown(self, conn: psycopg.AsyncConnection[Any], event: Event, now: datetime) -> float:
        """1.0 right after speaking in this place, falling to 0.0 at the window edge."""
        value = await read_state(conn, f"last_spoke:{event.place}")
        if not value or not value.get("ts"):
            return 0.0
        elapsed = (now - datetime.fromisoformat(str(value["ts"]))).total_seconds()
        window = self.config.cooldown_minutes * 60.0
        return _clamp(1.0 - elapsed / window) if window > 0 else 0.0

    async def _crowd(self, conn: psycopg.AsyncConnection[Any], event: Event, now: datetime) -> float:
        """How busy this place is: 10 recent messages already saturate it."""
        since = now - timedelta(minutes=self.config.crowd_window_minutes)
        return min(await count_recent_events(conn, event.place or "", kind="msg.in", since=since)
                   / CROWD_FULL_MESSAGES, 1.0)


def _clamp(value: float) -> float:
    return max(0.0, min(1.0, float(value)))


__all__ = ["CROWD_FULL_MESSAGES", "Gate", "GateConfig", "Verdict"]
