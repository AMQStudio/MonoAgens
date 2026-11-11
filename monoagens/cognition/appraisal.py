"""The Appraisal: one decision and every number behind it."""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any


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

    def payload(self) -> dict[str, Any]:
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
