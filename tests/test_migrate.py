"""Tests for numbered SQL migrations, the version table and the backup."""

from __future__ import annotations

import json
import pathlib

import pytest

from monoagens.migrate import discover, migrate
from monoagens.store import append_event, read_state


async def _table_exists(conn, name: str) -> bool:
    cur = await conn.execute("SELECT to_regclass(%s)", (name,))
    return (await cur.fetchone())[0] is not None


async def _versions(conn) -> list[tuple[int, str]]:
    cur = await conn.execute("SELECT version, name FROM schema_version ORDER BY version")
    return [(int(row[0]), row[1]) for row in await cur.fetchall()]


def _write(directory: pathlib.Path, name: str, sql: str) -> None:
    (directory / name).write_text(sql, encoding="utf-8")


@pytest.fixture(autouse=True)
async def _clean_test_versions(aconn):
    yield
    await aconn.execute("DELETE FROM schema_version WHERE version >= 900")


async def test_initial_schema_is_a_recorded_migration(aconn):
    rows = await _versions(aconn)
    assert (1, "initial") in rows
    assert await _table_exists(aconn, "event_vectors")


async def test_migrations_apply_in_order_and_records_versions(aconn, tmp_path):
    migrations = tmp_path / "migrations"
    backups = tmp_path / "backups"
    migrations.mkdir()
    _write(migrations, "900_alpha.sql", "CREATE TABLE t9_alpha (id integer PRIMARY KEY);")
    _write(migrations, "901_beta.sql",
           "INSERT INTO t9_alpha (id) VALUES (1);"
           "CREATE TABLE t9_beta (id integer PRIMARY KEY);")
    try:
        applied = await migrate(aconn, directory=migrations, backup_dir=backups)
        assert applied == [900, 901]
        rows = await _versions(aconn)
        assert rows[-2:] == [(900, "alpha"), (901, "beta")]
        assert await _table_exists(aconn, "t9_alpha")
        assert await _table_exists(aconn, "t9_beta")
        cur = await aconn.execute("SELECT id FROM t9_alpha")
        assert await cur.fetchone() == (1,)
    finally:
        await aconn.execute("DROP TABLE IF EXISTS t9_beta, t9_alpha")


async def test_backup_is_written_before_migrating(aconn, tmp_path):
    event = await append_event(aconn, ts="2026-01-01T00:00:00+00:00", kind="percept",
                               who="person:1", place="space:tg:1",
                               payload={"description": "before the migration"})
    migrations = tmp_path / "migrations"
    backups = tmp_path / "backups"
    migrations.mkdir()
    _write(migrations, "910_gamma.sql", "CREATE TABLE t9_gamma (id integer PRIMARY KEY);")
    try:
        assert await migrate(aconn, directory=migrations, backup_dir=backups) == [910]
    finally:
        await aconn.execute("DROP TABLE IF EXISTS t9_gamma")
    files = sorted(backups.glob("before-910*.json"))
    assert len(files) == 1
    snapshot = json.loads(files[0].read_text(encoding="utf-8"))
    assert [row["id"] for row in snapshot["events"]] == [event.id]
    assert snapshot["events"][0]["payload"]["description"] == "before the migration"


async def test_applied_migrations_are_not_reapplied_and_need_no_backup(aconn, tmp_path):
    migrations = tmp_path / "migrations"
    backups = tmp_path / "backups"
    migrations.mkdir()
    _write(migrations, "920_delta.sql", "CREATE TABLE t9_delta (id integer PRIMARY KEY);")
    try:
        assert await migrate(aconn, directory=migrations, backup_dir=backups) == [920]
        assert await migrate(aconn, directory=migrations, backup_dir=backups) == []
    finally:
        await aconn.execute("DROP TABLE IF EXISTS t9_delta")
    assert [path.name for path in backups.glob("before-*")] == ["before-920.json"]


def test_discover_rejects_bad_names_and_duplicates(tmp_path):
    _write(tmp_path, "1_bad.sql", "SELECT 1;")
    with pytest.raises(ValueError, match="NNN_name.sql"):
        discover(tmp_path)
    (tmp_path / "1_bad.sql").unlink()
    _write(tmp_path, "930_one.sql", "SELECT 1;")
    _write(tmp_path, "930_two.sql", "SELECT 1;")
    with pytest.raises(ValueError, match="duplicate"):
        discover(tmp_path)


async def test_owner_state_survives_migration(aconn, tmp_path):
    from monoagens.store import write_state
    event = await append_event(aconn, ts="2026-01-01T00:00:00+00:00", kind="pair.claimed",
                               who="person:1")
    await write_state(aconn, "owner", {"who": "person:1"}, event.id)
    migrations = tmp_path / "migrations"
    migrations.mkdir()
    _write(migrations, "940_epsilon.sql", "SELECT 1;")
    await migrate(aconn, directory=migrations, backup_dir=tmp_path / "backups")
    assert await read_state(aconn, "owner") == {"who": "person:1"}
