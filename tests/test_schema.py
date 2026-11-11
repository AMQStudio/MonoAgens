"""Step 0: the schema is installed and the Log is append-only."""

from __future__ import annotations

import psycopg
import pytest

TABLES = {"events", "event_vectors", "state", "expectations", "memories"}


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


def test_events_place_id_index_exists(conn):
    exists = conn.execute(
        "SELECT 1 FROM pg_indexes WHERE schemaname = current_schema()"
        " AND indexname = 'idx_events_place_id'"
    ).fetchone()
    assert exists is not None


def test_insert_event_succeeds(conn):
    row = conn.execute(
        "INSERT INTO events (kind, who, place, body) VALUES ('percept', 'person:owner', 'space:cli', 'hello')"
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
        conn.execute("UPDATE events SET kind = 'act.out' WHERE id = %s", (event_id,))


def test_delete_event_is_rejected(conn):
    event_id = conn.execute(
        "INSERT INTO events (kind) VALUES ('decision.note') RETURNING id"
    ).fetchone()[0]
    with pytest.raises(psycopg.Error, match="events table is append-only"):
        conn.execute("DELETE FROM events WHERE id = %s", (event_id,))


def test_event_vectors_are_write_once(conn):
    event_id = conn.execute(
        "INSERT INTO events (kind) VALUES ('percept') RETURNING id"
    ).fetchone()[0]
    conn.execute("INSERT INTO event_vectors (event_id, embedding) VALUES (%s, %s)",
                 (event_id, [0.0] * 384))
    with pytest.raises(psycopg.Error, match="event vectors are write-once"):
        conn.execute("UPDATE event_vectors SET embedding = %s WHERE event_id = %s",
                     ([1.0] * 384, event_id))


def test_direct_vector_delete_is_rejected(conn):
    event_id = conn.execute(
        "INSERT INTO events (kind) VALUES ('percept') RETURNING id"
    ).fetchone()[0]
    conn.execute("INSERT INTO event_vectors (event_id, embedding) VALUES (%s, %s)",
                 (event_id, [0.0] * 384))
    with pytest.raises(psycopg.Error,
                       match="event vectors leave only by fading or hard forgetting"):
        conn.execute("DELETE FROM event_vectors WHERE event_id = %s", (event_id,))


def test_lineage_columns_exist(conn):
    expectations = {row[0] for row in conn.execute(
        "SELECT column_name FROM information_schema.columns "
        "WHERE table_name = 'expectations'").fetchall()}
    memories = {row[0] for row in conn.execute(
        "SELECT column_name FROM information_schema.columns "
        "WHERE table_name = 'memories'").fetchall()}
    vectors = {row[0] for row in conn.execute(
        "SELECT column_name FROM information_schema.columns "
        "WHERE table_name = 'event_vectors'").fetchall()}
    assert "derived_from" in expectations
    assert "derived_from" in memories
    assert "derived_from" in vectors
    assert "source_events" not in memories
    assert conn.execute("SELECT derived_from FROM memories LIMIT 0").description is not None
    assert conn.execute("SELECT derived_from FROM expectations LIMIT 0").description is not None
    assert conn.execute("SELECT derived_from FROM event_vectors LIMIT 0").description is not None


def test_new_cognition_columns_exist(conn):
    expectations = {row[0] for row in conn.execute(
        "SELECT column_name FROM information_schema.columns "
        "WHERE table_name = 'expectations'").fetchall()}
    memories = {row[0] for row in conn.execute(
        "SELECT column_name FROM information_schema.columns "
        "WHERE table_name = 'memories'").fetchall()}
    assert "origin" in expectations
    assert "use_count" in memories
    assert conn.execute("SELECT origin FROM expectations LIMIT 0").description is not None
    assert conn.execute("SELECT use_count FROM memories LIMIT 0").description is not None


def test_provenance_and_privacy_columns_exist(conn):
    events = {row[0] for row in conn.execute(
        "SELECT column_name FROM information_schema.columns "
        "WHERE table_name = 'events'").fetchall()}
    memories = {row[0] for row in conn.execute(
        "SELECT column_name FROM information_schema.columns "
        "WHERE table_name = 'memories'").fetchall()}
    assert "origin_actor" in events
    assert "private" in memories
    assert conn.execute("SELECT origin_actor FROM events LIMIT 0").description is not None
    assert conn.execute("SELECT private FROM memories LIMIT 0").description is not None
