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
    "talkativeness_theta_base": [0.85, 0.35],
    "spontaneity_tau": [0.02, 0.2],
    "talk_rate_window_minutes": 10,
    "promise_debt_weight": 0.1,
    "note_band": 0.2,
    "retrieve_k": 5,
    "recent_events": 20,
    "recent_messages": 20,
    "recent_elsewhere": 10,
    "recent_tokens": 2000,
    "memory_tokens": 1500,
    "thread_result_tokens": 3000,
    "volatile_tokens_max": 8000,
    "recall_results": 10,
    "max_task_steps": 30,
    "max_open_tasks": 3,
    "max_thought_rounds": 3,
    "model_call_retries": 1,
    "rrf_k": 60,
    "decay": 0.995,
    "persistence_decay_d": [0.7, 0.3],
    "boredom_gamma_per_minute": [0.99, 0.8],
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
        "knobs": {"memory_persistence": 0.5, "boredom_with_repeats": 0.5,
                  "talkativeness": 0.45, "restraint_after_speaking": 0.5, "spontaneity": 0.2,
                  "reserve_when_unfamiliar": 0.3, "patience_seconds": 90},
        "calibration": {"wake_gate_lo": 0.35},
        "constants": dict(CONSTANTS),
        "references": {},
        "lessons": [],
        "procedures": [],
        "manifest": {},
    }
    for name, patch in sections.items():
        value[name] = {**value.get(name, {}), **patch}
    return value
