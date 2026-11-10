"""Shared test fixtures: a real Postgres, no mocks.

The suite connects to TEST_DATABASE_URL, then DATABASE_URL, then the local
development default (port 5433). sql/schema.sql is applied once per session.
"""

from __future__ import annotations

import os
import pathlib

from pgvector.psycopg import register_vector_async
import psycopg
import pytest

ROOT = pathlib.Path(__file__).resolve().parent.parent
SCHEMA_PATH = ROOT / "sql" / "schema.sql"
DEFAULT_DATABASE_URL = "postgresql://monoagens:monoagens@localhost:5433/monoagens_test"


def _load_env_file() -> None:
    env_path = ROOT / ".env"
    if not env_path.is_file():
        return
    for raw_line in env_path.read_text(encoding="utf-8").splitlines():
        line = raw_line.strip()
        if not line or line.startswith("#") or "=" not in line:
            continue
        key, value = line.split("=", 1)
        os.environ.setdefault(key.strip(), value.strip().strip("'\""))


def _database_url() -> str:
    _load_env_file()
    return (
        os.environ.get("TEST_DATABASE_URL")
        or os.environ.get("DATABASE_URL")
        or DEFAULT_DATABASE_URL
    )


@pytest.fixture(scope="session")
def database_url() -> str:
    return _database_url()


@pytest.fixture(scope="session", autouse=True)
def _install_schema(database_url: str):
    sql = SCHEMA_PATH.read_text(encoding="utf-8")
    with psycopg.connect(database_url, autocommit=True) as conn:
        conn.execute(sql)
    yield


@pytest.fixture
def conn(database_url: str):
    """A connection whose work is rolled back after each test."""
    with psycopg.connect(database_url) as connection:
        yield connection
        connection.rollback()


@pytest.fixture
async def aconn(database_url: str):
    """An async connection whose work is rolled back after each test."""
    async with await psycopg.AsyncConnection.connect(database_url) as connection:
        await register_vector_async(connection)
        yield connection
        await connection.rollback()
