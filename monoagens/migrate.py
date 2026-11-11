"""Numbered SQL migrations: a version table and a backup before applying."""

from __future__ import annotations

import asyncio
import json
import os
import re
from pathlib import Path

import psycopg
from pgvector.psycopg import register_vector_async

ROOT = Path(__file__).resolve().parent.parent
MIGRATIONS_DIR = ROOT / "sql" / "migrations"
BACKUP_DIR = Path(os.environ.get("MONOAGENS_BACKUP_DIR") or ROOT / "backups")
NAME = re.compile(r"^(\d{3})_([a-z0-9_]+)\.sql$")
VERSION_DDL = """CREATE TABLE IF NOT EXISTS schema_version (
    version integer PRIMARY KEY,
    name text NOT NULL,
    applied_at timestamptz NOT NULL DEFAULT now()
)"""
TABLES = ("events", "event_vectors", "state", "expectations", "memories")


def discover(directory=MIGRATIONS_DIR) -> list[tuple[int, str, Path]]:
    """Return the numbered migration files in version order."""
    found = []
    for path in sorted(Path(directory).glob("*.sql")):
        match = NAME.match(path.name)
        if match is None:
            raise ValueError(f"migration name must be NNN_name.sql: {path.name!r}")
        found.append((int(match[1]), match[2], path))
    versions = [version for version, _, _ in found]
    if len(versions) != len(set(versions)):
        raise ValueError(f"duplicate migration version in {directory}")
    return found


async def _applied(conn) -> set[int]:
    cur = await conn.execute("SELECT to_regclass('schema_version')")
    if (await cur.fetchone())[0] is None:
        return set()
    cur = await conn.execute("SELECT version FROM schema_version")
    return {int(row[0]) for row in await cur.fetchall()}


async def _exists(conn, table: str) -> bool:
    cur = await conn.execute("SELECT to_regclass(%s)", (table,))
    return (await cur.fetchone())[0] is not None


async def backup(conn, directory=BACKUP_DIR, *, first_version: int) -> Path:
    """Snapshot every book as JSON before the first pending migration runs."""
    directory = Path(directory)
    await asyncio.to_thread(directory.mkdir, parents=True, exist_ok=True)
    target = directory / f"before-{first_version:03d}.json"
    counter = 0
    while target.exists():
        counter += 1
        target = directory / f"before-{first_version:03d}-{counter}.json"
    snapshot = {}
    for table in TABLES:
        if await _exists(conn, table):
            cur = await conn.execute(f"SELECT to_jsonb(t) FROM {table} AS t")
            snapshot[table] = [row[0] for row in await cur.fetchall()]
    text = json.dumps(snapshot, ensure_ascii=False, default=str)
    await asyncio.to_thread(target.write_text, text, "utf-8")
    return target


async def migrate(conn, *, directory=MIGRATIONS_DIR, backup_dir=BACKUP_DIR) -> list[int]:
    """Apply the unapplied migrations in order, each in its own transaction."""
    applied = await _applied(conn)
    pending = [item for item in discover(directory) if item[0] not in applied]
    if not pending:
        return []
    await backup(conn, backup_dir, first_version=pending[0][0])
    await conn.execute(VERSION_DDL)
    for version, name, path in pending:
        sql = await asyncio.to_thread(path.read_text, encoding="utf-8")
        async with conn.transaction():
            await conn.execute(sql)
            await conn.execute(
                "INSERT INTO schema_version (version, name) VALUES (%s, %s)",
                (version, name),
            )
    return [version for version, _, _ in pending]


async def migrate_url(url: str, *, directory=MIGRATIONS_DIR,
                      backup_dir=BACKUP_DIR) -> list[int]:
    """Apply migrations on a fresh connection; used by scripts and test bootstrap."""
    async with await psycopg.AsyncConnection.connect(url, autocommit=True) as conn:
        applied = await migrate(conn, directory=directory, backup_dir=backup_dir)
        await register_vector_async(conn)
        return applied
