"""Signals: pure functions from a Situation to a number in [0, 1]."""

from __future__ import annotations

from collections.abc import Callable, Mapping
from typing import Any

from monoagens.cognition.situation import Situation

Signal = Callable[[Situation], float]
SIGNALS: dict[str, Signal] = {}


def register_signal(name: str) -> Callable[[Signal], Signal]:
    """Register a signal under its name in cognition.weights."""

    def decorator(fn: Signal) -> Signal:
        SIGNALS[name] = fn
        return fn

    return decorator


def validate_signals(weights: Mapping[str, float]) -> None:
    """Fail at boot unless every signal has a weight and vice versa."""
    if set(SIGNALS) != set(weights):
        missing = sorted(set(SIGNALS) - set(weights))
        unknown = sorted(set(weights) - set(SIGNALS))
        raise ValueError(f"cognition.weights mismatch: missing {missing}, unknown {unknown}")


@register_signal("name_mentioned")
def name_mentioned(situation: Situation) -> float:
    """Score whether the self's name appears without a tag (stub for Track A)."""
    return 0.0


@register_signal("info_gap")
def info_gap(situation: Situation) -> float:
    """Score whether the message is an open question or something forgotten."""
    return 0.0


@register_signal("feeling_of_knowing")
def feeling_of_knowing(situation: Situation) -> float:
    """Score "I think I know this" from memory activation (stub for Track A)."""
    return 0.0


@register_signal("relevance")
def relevance(situation: Situation) -> float:
    """Score how much the message matches what the owner cares about now."""
    return 0.0


@register_signal("social_cost")
def social_cost(situation: Situation) -> float:
    """Score what speaking here would cost socially (stub for Track A)."""
    return 0.0
