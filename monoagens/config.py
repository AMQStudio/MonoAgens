"""Configuration: soul.yaml, the tiny .env loader, and LLM settings."""
from __future__ import annotations
import os
import pathlib
from typing import Any
import yaml

DEFAULT_SOUL_PATH = pathlib.Path(__file__).resolve().parent.parent / "soul.yaml"
DEFAULT_ENV_PATH = pathlib.Path(__file__).resolve().parent.parent / ".env"
DEFAULT_TICK_SECONDS = 30.0
_LLM_KEYS = (("base_url", "LLM_BASE_URL", "PFC_BASE_URL"), ("api_key", "LLM_API_KEY", "PFC_API_KEY"),
             ("model", "LLM_MODEL", "PFC_MODEL"))

def load_soul(path=DEFAULT_SOUL_PATH) -> dict[str, Any]:
    with open(path, encoding="utf-8") as handle:
        return yaml.safe_load(handle) or {}

def load_owner_ids(path=DEFAULT_SOUL_PATH) -> set[int]:
    owner = load_soul(path).get("owner") or {}
    return {int(tid) for tid in owner.get("telegram_ids") or []}

def load_clock_tick_seconds(path=DEFAULT_SOUL_PATH) -> float:
    return float((load_soul(path).get("clock") or {}).get("tick_seconds", DEFAULT_TICK_SECONDS))

def load_env_file(path=DEFAULT_ENV_PATH) -> dict[str, str]:
    values: dict[str, str] = {}
    env_path = pathlib.Path(path)
    if not env_path.is_file():
        return values
    for raw_line in env_path.read_text(encoding="utf-8").splitlines():
        line = raw_line.strip()
        if not line or line.startswith("#") or "=" not in line:
            continue
        key, value = line.split("=", 1)
        values[key.strip()] = value.strip().strip("'\"")
    return values

def load_llm_config(env_path=DEFAULT_ENV_PATH) -> dict[str, str]:
    file_values = load_env_file(env_path)
    return {name: os.environ.get(primary) or file_values.get(primary) or os.environ.get(alias)
            or file_values.get(alias) or "" for name, primary, alias in _LLM_KEYS}
