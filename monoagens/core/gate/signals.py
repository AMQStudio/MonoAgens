"""Signals: pure functions from a Situation to a number in [0, 1]."""

from __future__ import annotations

import math
from collections.abc import Callable, Mapping, Sequence

from monoagens.core.gate.situation import Situation
from monoagens.infra.embedding import cosine

Signal = Callable[[Situation], float]
SIGNALS: dict[str, Signal] = {}


def register_signal(name: str) -> Callable[[Signal], Signal]:
    """Register a signal under the name its prior weight uses."""

    def decorator(fn: Signal) -> Signal:
        SIGNALS[name] = fn
        return fn

    return decorator


def validate_signals(weights: Mapping[str, float]) -> None:
    """Fail at boot unless every signal has a weight and vice versa."""
    if set(SIGNALS) != set(weights):
        missing = sorted(set(SIGNALS) - set(weights))
        unknown = sorted(set(weights) - set(SIGNALS))
        raise ValueError(f"signal weights mismatch: missing {missing}, unknown {unknown}")


def _clamp(value: float) -> float:
    """Return a signal value clipped to [0, 1]."""
    return max(0.0, min(1.0, float(value)))


def _best_cosine(vector: Sequence[float] | None,
                 others: Sequence[Sequence[float]]) -> float:
    """Return the highest cosine between `vector` and any of `others`."""
    return max((cosine(vector, other) for other in others), default=0.0)


@register_signal("own_name")
def own_name(situation: Situation) -> float:
    """Score 1 when the soul's name was said without the turn being given to it."""
    if situation.event.features.get("addressed"):
        return 0.0
    return 1.0 if situation.event.features.get("named") else 0.0


@register_signal("info_gap")
def info_gap(situation: Situation) -> float:
    """Score the cosine to the centroid (mean vector) of the reference sentences.

    The centroid answers "is this the kind of line that asks about something forgotten",
    so one reference that happens to share a word cannot carry the value alone.
    """
    vectors = [vector for vector in situation.references if vector]
    if not vectors or not situation.vector:
        return 0.0
    centroid = [sum(column) / len(vectors) for column in zip(*vectors)]
    return _clamp(cosine(situation.vector, centroid))


@register_signal("feeling_of_knowing")
def feeling_of_knowing(situation: Situation) -> float:
    """Score metamemory: sigma((A* - fok_threshold) / fok_scale)."""
    scale = situation.fok_scale if situation.fok_scale > 0 else 1.0
    score = max(-60.0, min(60.0, (situation.best_activation - situation.fok_threshold) / scale))
    return _clamp(1.0 / (1.0 + math.exp(-score)))


@register_signal("relevance")
def relevance(situation: Situation) -> float:
    """Score the best cosine to active promises and the core memory."""
    vectors: list[Sequence[float]] = list(situation.promise_vectors)
    if situation.core_vector:
        vectors.append(situation.core_vector)
    return _clamp(_best_cosine(situation.vector, vectors))


@register_signal("social_cost")
def social_cost(situation: Situation) -> float:
    """Score a floor that belongs to someone else, lowered by closeness to the speaker."""
    if situation.event.features.get("addressed"):
        return 0.0
    taken = bool(situation.event.features.get("addressed_other")) or situation.soul_answered
    return _clamp(float(taken) * (1.0 - _clamp(situation.closeness)))
