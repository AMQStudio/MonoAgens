"""Configuration: config.yaml, the birth certificate, the tiny .env loader, LLM settings."""

from __future__ import annotations

import os
import pathlib
import re
from typing import Any

import psycopg
import yaml

DEFAULT_ROOT = pathlib.Path(__file__).resolve().parent.parent
DEFAULT_SOUL_PATH = DEFAULT_ROOT / "soul.yaml"
DEFAULT_CONFIG_PATH = DEFAULT_ROOT / "config.yaml"
DEFAULT_ENV_PATH = DEFAULT_ROOT / ".env"
DEFAULT_TICK_SECONDS = 30.0
_LLM_KEYS = (("base_url", "LLM_BASE_URL"), ("api_key", "LLM_API_KEY"),
             ("model", "LLM_MODEL"))


def read_yaml(path: pathlib.Path) -> dict[str, Any]:
    """Load one YAML file as a dict (empty when the file is missing or blank)."""
    path = pathlib.Path(path)
    if not path.is_file():
        return {}
    with open(path, encoding="utf-8") as handle:
        return yaml.safe_load(handle) or {}


def load_config(path: pathlib.Path = DEFAULT_CONFIG_PATH) -> dict[str, Any]:
    """Load config.yaml: this deployment's non-secret settings."""
    return read_yaml(path)


def load_soul(path: pathlib.Path = DEFAULT_SOUL_PATH) -> dict[str, Any]:
    """Load soul.yaml: the birth certificate, read once at birth."""
    return read_yaml(path)


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


def load_llm_config(env_path: pathlib.Path = DEFAULT_ENV_PATH) -> dict[str, str]:
    """Resolve the LLM settings from the environment, then the .env file."""
    file_values = load_env_file(env_path)
    return {name: os.environ.get(key) or file_values.get(key) or ""
            for name, key in _LLM_KEYS}


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
