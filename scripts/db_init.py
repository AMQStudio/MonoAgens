#!/usr/bin/env -S uv run python
"""Apply the numbered SQL migrations to the MonoAgens database.

Usage:
    uv run python scripts/db_init.py [DATABASE_URL]

The connection URL is resolved in this order: command-line argument, the
DATABASE_URL environment variable, the local .env file, then the built-in
development default.
"""

from __future__ import annotations

import asyncio
import os
import pathlib
import sys

import psycopg

from monoagens.migrate import migrate_url

ROOT = pathlib.Path(__file__).resolve().parent.parent
DEFAULT_URL = "postgresql://monoagens:monoagens@localhost:5433/monoagens"


def load_env_file(path: pathlib.Path) -> None:
    """Populate os.environ from a simple KEY=VALUE file, without overriding."""
    if not path.is_file():
        return
    for raw_line in path.read_text(encoding="utf-8").splitlines():
        line = raw_line.strip()
        if not line or line.startswith("#") or "=" not in line:
            continue
        key, value = line.split("=", 1)
        key = key.strip()
        value = value.strip().strip("'\"")
        if key:
            os.environ.setdefault(key, value)


def resolve_url(argv: list[str]) -> str:
    if len(argv) > 1:
        return argv[1]
    load_env_file(ROOT / ".env")
    return os.environ.get("DATABASE_URL", DEFAULT_URL)


def main(argv: list[str]) -> int:
    url = resolve_url(argv)
    applied = asyncio.run(migrate_url(url))
    with psycopg.connect(url, autocommit=True) as conn:
        row = conn.execute("SELECT current_database(), version()").fetchone()
    print(f"migrations applied: {applied or 'none (already current)'}")
    print(f"schema on {row[0]}")
    print(row[1])
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
