"""Shared runtime settings for the tests: fixed values, no file or database needed."""

from __future__ import annotations

from typing import Any

IDENTITY = {
    "name": "Mona",
    "language": "id",
    "persona": "Calm and honest.",
    "principles": ["Never lie."],
}

CONSTANTS = {
    "seed": 1,
    "own_name_weight": 0.5,
    "info_gap_weight": 0.35,
    "feeling_of_knowing_weight": 0.35,
    "relevance_weight": 0.2,
    "social_cost_weight": -0.4,
    "budget_weight": 0.3,
    "retrieve_k": 5,
    "recent_events": 20,
    "recent_messages": 20,
    "recent_elsewhere": 10,
    "rrf_k": 60,
    "decay": 0.995,
    "persistence_decay_d": [0.7, 0.3],
    "context_weight": 1.0,
    "importance_weight": 0.5,
    "default_expires_days": 30,
    "max_active": 200,
    "job_timeout_seconds": 5.0,
}


def settings(**sections: dict[str, Any]) -> dict[str, Any]:
    """Return one settings view, overriding whole sections by name."""
    value: dict[str, Any] = {
        "identity": dict(IDENTITY),
        "timezone": "Asia/Jakarta",
        "owner_settings": {"daily_token_budget": 0},
        "runtime": {"tick_seconds": 30,
                    "sleep": {"idle_minutes": 30, "retry_minutes": 30, "hour": 2}},
        "bodies": {},
        "plugins": [],
        "knobs": {"memory_persistence": 0.5},
        "calibration": {},
        "constants": dict(CONSTANTS),
        "references": {},
        "lessons": [],
        "procedures": [],
        "manifest": {},
    }
    for name, patch in sections.items():
        value[name] = {**value.get(name, {}), **patch}
    return value
