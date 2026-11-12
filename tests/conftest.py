"""Shared test fixtures: a real Postgres, no mocks.

The suite runs ONLY against TEST_DATABASE_URL, and only if that database name
ends with '_test'. Anything else refuses to run. The numbered migrations in
sql/migrations/ are applied once per session.
"""

from __future__ import annotations

import asyncio
import os
import pathlib

from pgvector.psycopg import register_vector_async
import psycopg
import pytest

from monoagens.infra.migrate import migrate

ROOT = pathlib.Path(__file__).resolve().parent.parent
REFUSAL = (
    "Refusing to run tests: TEST_DATABASE_URL is required and database name "
    "must end with '_test'"
)


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
    """Read TEST_DATABASE_URL and refuse anything that is not a *_test database."""
    _load_env_file()
    url = os.environ.get("TEST_DATABASE_URL")
    if not url:
        raise RuntimeError(REFUSAL)
    dbname = psycopg.conninfo.conninfo_to_dict(url).get("dbname") or ""
    if not dbname.endswith("_test"):
        raise RuntimeError(REFUSAL)
    return url


DATABASE_URL = _database_url()


@pytest.fixture(scope="session")
def database_url() -> str:
    return DATABASE_URL


@pytest.fixture(scope="session", autouse=True)
def _install_schema(database_url: str, tmp_path_factory):
    async def apply() -> None:
        async with await psycopg.AsyncConnection.connect(database_url,
                                                         autocommit=True) as conn:
            await migrate(conn, backup_dir=tmp_path_factory.mktemp("backups"))

    asyncio.run(apply())
    yield


@pytest.fixture
def conn(database_url: str):
    """A connection with clean tables for test isolation."""
    with psycopg.connect(database_url, autocommit=True) as connection:
        connection.execute("TRUNCATE TABLE events, state, expectations, memories CASCADE")
        yield connection
        connection.execute("TRUNCATE TABLE events, state, expectations, memories CASCADE")


@pytest.fixture
async def aconn(database_url: str):
    """An async connection with clean tables for test isolation."""
    async with await psycopg.AsyncConnection.connect(database_url, autocommit=True) as connection:
        await register_vector_async(connection)
        await connection.execute("TRUNCATE TABLE events, state, expectations, memories CASCADE")
        yield connection
        await connection.execute("TRUNCATE TABLE events, state, expectations, memories CASCADE")
