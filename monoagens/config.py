"""Configuration: loading soul.yaml (persona, owner ids, clock)."""

from __future__ import annotations

import pathlib
from typing import Any

import yaml

DEFAULT_SOUL_PATH = pathlib.Path(__file__).resolve().parent.parent / "soul.yaml"
DEFAULT_TICK_SECONDS = 30.0


def load_soul(path: str | pathlib.Path = DEFAULT_SOUL_PATH) -> dict[str, Any]:
    """Load the soul configuration from YAML."""
    with open(path, encoding="utf-8") as handle:
        return yaml.safe_load(handle) or {}


def load_owner_ids(path: str | pathlib.Path = DEFAULT_SOUL_PATH) -> set[int]:
    """Read owner.telegram_ids from the soul configuration."""
    owner = load_soul(path).get("owner") or {}
    return {int(tid) for tid in owner.get("telegram_ids") or []}


def load_clock_tick_seconds(path: str | pathlib.Path = DEFAULT_SOUL_PATH) -> float:
    """Read clock.tick_seconds from the soul configuration."""
    clock = load_soul(path).get("clock") or {}
    return float(clock.get("tick_seconds", DEFAULT_TICK_SECONDS))
