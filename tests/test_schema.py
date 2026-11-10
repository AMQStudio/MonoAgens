"""Step 0: the schema is installed and the Log is append-only."""

from __future__ import annotations

import psycopg
import pytest

TABLES = {"events", "state", "expectations", "memories"}


def test_extensions_installed(conn):
    rows = conn.execute(
        "SELECT extname, extversion FROM pg_extension WHERE extname IN ('vector', 'pg_trgm')"
    ).fetchall()
    extensions = {name: tuple(int(part) for part in version.split(".")) for name, version in rows}
    assert set(extensions) == {"vector", "pg_trgm"}
    assert extensions["vector"] >= (0, 8, 1)


def test_tables_exist(conn):
    rows = conn.execute(
        """
        SELECT table_name
        FROM information_schema.tables
        WHERE table_schema = current_schema()
        """
    ).fetchall()
    assert TABLES <= {name for (name,) in rows}


def test_expectations_next_check_at_index_exists(conn):
    exists = conn.execute(
        "SELECT 1 FROM pg_indexes WHERE schemaname = current_schema()"
        " AND indexname = 'idx_expectations_next_check_at'"
    ).fetchone()
    assert exists is not None


def test_insert_event_succeeds(conn):
    row = conn.execute(
        "INSERT INTO events (kind, who, place, body) VALUES ('msg.in', 'person:owner', 'space:cli', 'hello')"
        " RETURNING id, ts, payload"
    ).fetchone()
    event_id, ts, payload = row
    assert event_id > 0
    assert ts is not None
    assert payload == {}


def test_update_event_is_rejected(conn):
    event_id = conn.execute(
        "INSERT INTO events (kind) VALUES ('decision.silent') RETURNING id"
    ).fetchone()[0]
    with pytest.raises(psycopg.Error, match="events table is append-only"):
        conn.execute("UPDATE events SET kind = 'msg.out' WHERE id = %s", (event_id,))


def test_delete_event_is_rejected(conn):
    event_id = conn.execute(
        "INSERT INTO events (kind) VALUES ('decision.note') RETURNING id"
    ).fetchone()[0]
    with pytest.raises(psycopg.Error, match="events table is append-only"):
        conn.execute("DELETE FROM events WHERE id = %s", (event_id,))
