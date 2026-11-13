"""Configuration: config.yaml, the birth certificate, the tiny .env loader, LLM settings."""

from __future__ import annotations

import logging
import os
import pathlib
import re
from collections.abc import Callable, Mapping
from typing import Any

import psycopg
import yaml

DEFAULT_ROOT = pathlib.Path(__file__).resolve().parent.parent.parent
DEFAULT_SOUL_PATH = DEFAULT_ROOT / "soul.yaml"
DEFAULT_CONFIG_PATH = DEFAULT_ROOT / "config.yaml"
DEFAULT_ENV_PATH = DEFAULT_ROOT / ".env"
DEFAULT_TICK_SECONDS = 30.0
EFFORTS = ("low", "medium", "high", "max")
CONFIG_VERSION = 2
SOUL_VERSION = 1
PRIOR_VERSION = 1
LOGGER = logging.getLogger(__name__)
_LLM_KEYS = (("base_url", "LLM_BASE_URL"), ("api_key", "LLM_API_KEY"),
             ("model", "LLM_MODEL"), ("effort", "LLM_REASONING_EFFORT"))
OFF_VALUES = ("false", "0")


def read_yaml(path: pathlib.Path) -> dict[str, Any]:
    """Load one YAML file as a dict (empty when the file is missing or blank)."""
    path = pathlib.Path(path)
    if not path.is_file():
        return {}
    with open(path, encoding="utf-8") as handle:
        return yaml.safe_load(handle) or {}


def note(notes: list[str] | None, line: str) -> None:
    """Report one boot-report line; without a sink it is a warning in the log."""
    if notes is None:
        LOGGER.warning("%s", line)
    else:
        notes.append(line)


def read_versioned(path: pathlib.Path, *, key: str, supported: int, label: str,
                   upcasters: Mapping[int, Callable[[dict[str, Any]], dict[str, Any]]],
                   notes: list[str] | None = None) -> dict[str, Any]:
    """Read one versioned YAML file, upcasting an older format in memory (D58).

    The file is never rewritten. A newer version than supported fails at boot; a file
    without the key is read as version 1 with a note.
    """
    path = pathlib.Path(path)
    if not path.is_file():
        return {}
    data = read_yaml(path)
    if not data:
        return {}
    if key not in data:
        version = 1
        note(notes, f"{label} {path.name}: no {key}: read as version 1")
    else:
        version = data[key]
        if isinstance(version, bool) or not isinstance(version, int) or version < 1:
            raise ValueError(f"{label} {path.name}: invalid {key} {version!r}")
    if version > supported:
        raise ValueError(f"{label} {path.name}: format version {version} is newer than "
                         f"supported {supported}")
    if version < supported:
        note(notes, f"{label} {path.name}: upcast from version {version} to {supported}")
        for step in range(version, supported):
            upcast = upcasters.get(step)
            if upcast is None:
                raise ValueError(f"{label} {path.name}: no upcast from version {step}")
            data = upcast(data)
    return data


def _config_v1_to_v2(config: dict[str, Any]) -> dict[str, Any]:
    """Version 1 listed plugins as names; version 2 maps each name to its settings."""
    plugins = config.get("plugins")
    if isinstance(plugins, list):
        return {**config, "plugins": {str(name): {"enabled": True} for name in plugins}}
    return config


CONFIG_UPCASTERS: dict[int, Callable[[dict[str, Any]], dict[str, Any]]] = {
    1: _config_v1_to_v2,
}


def load_config(path: pathlib.Path = DEFAULT_CONFIG_PATH,
                notes: list[str] | None = None) -> dict[str, Any]:
    """Load config.yaml: this deployment's non-secret settings, at the current version."""
    return read_versioned(path, key="version", supported=CONFIG_VERSION, label="config.yaml",
                          upcasters=CONFIG_UPCASTERS, notes=notes)


def load_soul(path: pathlib.Path = DEFAULT_SOUL_PATH,
              notes: list[str] | None = None) -> dict[str, Any]:
    """Load soul.yaml: the birth certificate, read once at birth."""
    return read_versioned(path, key="version", supported=SOUL_VERSION, label="soul.yaml",
                          upcasters={}, notes=notes)


def tick_seconds(settings: dict[str, Any]) -> float:
    """Return the clock tick period from the runtime settings."""
    runtime = settings.get("runtime") or {}
    return float(runtime.get("tick_seconds") or DEFAULT_TICK_SECONDS)


def load_env_file(path: pathlib.Path = DEFAULT_ENV_PATH) -> dict[str, str]:
    """Read a simple KEY=VALUE .env file into a dict."""
    env_path = pathlib.Path(path)
    if not env_path.is_file():
        return {}
    lines = (line.strip() for line in env_path.read_text(encoding="utf-8").splitlines())
    return {key.strip(): value.strip().strip("'\"") for key, value in
            (line.split("=", 1) for line in lines
             if line and not line.startswith("#") and "=" in line)}


def load_env(root: pathlib.Path = DEFAULT_ROOT) -> dict[str, str]:
    """Return os.environ plus .env values (os.environ wins) without printing them."""
    values = load_env_file(pathlib.Path(root) / ".env")
    return {**values, **os.environ}


def load_llm_config(env_path: pathlib.Path = DEFAULT_ENV_PATH) -> dict[str, str]:
    """Resolve the LLM settings from the environment, then the .env file."""
    file_values = load_env_file(env_path)
    config = {name: os.environ.get(key) or file_values.get(key) or ""
              for name, key in _LLM_KEYS}
    config["effort"] = config["effort"].strip().lower()
    if config["effort"] and config["effort"] not in EFFORTS:
        raise ValueError(f"LLM_REASONING_EFFORT must be one of {', '.join(EFFORTS)}")
    parallel = (os.environ.get("LLM_PARALLEL_TOOL_CALLS")
                or file_values.get("LLM_PARALLEL_TOOL_CALLS") or "true")
    config["parallel_tool_calls"] = parallel.strip().lower() not in OFF_VALUES
    return config


def load_provider_config(env_path: pathlib.Path = DEFAULT_ENV_PATH) -> dict[str, Any]:
    """Resolve provider selection and fallback list on top of load_llm_config()."""
    file_values = load_env_file(env_path)
    config: dict[str, Any] = dict(load_llm_config(env_path))
    provider = (os.environ.get("LLM_PROVIDER") or file_values.get("LLM_PROVIDER") or "").strip()
    config["provider"] = provider.lower() or "openai-compatible"
    raw_fb = os.environ.get("LLM_FALLBACKS") or file_values.get("LLM_FALLBACKS") or ""
    config["fallbacks"] = [item.strip().lower() for item in raw_fb.split(",") if item.strip()]
    return config


_PASSWORD_TOKEN = re.compile(
    r"(?i)\b(password|passfile|sslpassword)\s*=\s*(?:'[^']*'|\"[^\"]*\"|\S+)")
_URL_CREDENTIALS = re.compile(r"([a-z][a-z0-9+.-]*://[^:/@\s]*:)[^@\s]*@")


def private_dsn(conn: Any) -> str:
    """Rebuild a connection string that carries its password, for a private connection."""
    info = conn.info
    return psycopg.conninfo.make_conninfo(info.dsn, password=info.password or None)


def redact_connection_secrets(text: str, conn: Any) -> str:
    """Return text with a connection's password, DSN and URL credentials masked."""
    info = conn.info
    for secret in (private_dsn(conn), info.dsn, info.password):
        if secret:
            text = text.replace(secret, "***")
    text = _PASSWORD_TOKEN.sub(r"\1=***", text)
    return _URL_CREDENTIALS.sub(r"\1***@", text)
