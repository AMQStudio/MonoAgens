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

    def payload(self) -> dict[str, Any]:
        """Return the decision payload that is stored in the Log."""
        return {
            "decision": self.decision,
            "reason": self.reason,
            "reflex": self.reflex,
            "signals": self.signals,
            "s": self.s,
            "theta": self.theta,
            "p": self.p,
            "u": self.u,
        }
