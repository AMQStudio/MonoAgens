"""The stability contract: event schema versions, upcasting and kind namespaces"""

from __future__ import annotations

from datetime import datetime, timezone

import pytest

from monoagens.infra import versioning
from monoagens.infra.store import append_event, read_last_events
from monoagens.types import CURRENT_SCHEMA_VERSION

START = datetime(2026, 11, 11, 9, 0, tzinfo=timezone.utc)
PLACE = "space:stability"
CORE_KINDS = (
    "percept", "command", "act.out", "act.result", "decision.silent", "decision.note",
    "decision.wake", "model.call", "budget.exhausted", "expectation.created",
    "expectation.fired", "expectation.reminded", "expectation.resolved", "job.started",
    "job.result",
    "classifier.verdict", "credence.updated", "disposition.updated", "share.granted",
    "task.opened", "task.closed", "guard.stopped", "forget.soft", "forget.hard",
    "fade.done", "sleep.done", "sleep.failed", "error",
)


async def _stored_row(conn, event_id):
    cur = await conn.execute("SELECT payload, schema_version FROM events WHERE id = %s",
                            (event_id,))
    return await cur.fetchone()


def test_events_table_has_the_version_column(conn):
    cur = conn.execute(
        "SELECT data_type, is_nullable, column_default FROM information_schema.columns "
        "WHERE table_name = 'events' AND column_name = 'schema_version'"
    )
    assert cur.fetchone() == ("integer", "NO", "1")


async def test_an_event_carries_its_schema_version(aconn):
    payload = {"description": "halo"}
    event = await append_event(aconn, ts=START, kind="percept", place=PLACE, payload=payload)
    assert event.schema_version == CURRENT_SCHEMA_VERSION
    assert await _stored_row(aconn, event.id) == (payload, CURRENT_SCHEMA_VERSION)
    assert (await read_last_events(aconn, PLACE))[-1].schema_version == CURRENT_SCHEMA_VERSION


async def test_upcasting_on_read_leaves_the_stored_row_untouched(aconn, monkeypatch):
    stored = await append_event(aconn, ts=START, kind="percept", place=PLACE,
                                payload={"description": "halo"}, schema_version=1)
    seen = []

    def add_tokens(payload):
        seen.append(dict(payload))
        return {**payload, "tokens": 0}

    monkeypatch.setattr(versioning, "CURRENT_SCHEMA_VERSION", 2)
    monkeypatch.setitem(versioning.UPCASTERS, 1, add_tokens)
    read = (await read_last_events(aconn, PLACE))[-1]
    assert read.schema_version == 2
    assert read.payload == {"description": "halo", "tokens": 0}
    assert seen == [{"description": "halo"}]
    assert await _stored_row(aconn, stored.id) == ({"description": "halo"}, 1)


async def test_an_old_version_without_an_upcaster_fails_loudly(aconn, monkeypatch):
    await append_event(aconn, ts=START, kind="percept", place=PLACE, schema_version=1)
    monkeypatch.setattr(versioning, "CURRENT_SCHEMA_VERSION", 2)
    with pytest.raises(ValueError, match="no upcaster from event schema version 1"):
        await read_last_events(aconn, PLACE)


async def test_readers_ignore_unknown_payload_fields(aconn):
    payload = {"description": "halo", "topic": "cuaca", "future_extension": {"n": 1}}
    await append_event(aconn, ts=START, kind="percept", place=PLACE, payload=payload)
    read = (await read_last_events(aconn, PLACE))[-1]
    assert read.payload == payload


def test_every_core_kind_is_a_valid_kind():
    assert all(versioning.is_valid_event_kind(kind) for kind in CORE_KINDS)
    for bad in ("Percept", "act..out", ".act.out", "act.out.", "", "act out", None):
        assert not versioning.is_valid_event_kind(bad)


def test_plugin_kinds_live_under_their_namespace():
    kind = versioning.plugin_kind("telegram", "reaction_sent")
    assert kind == "telegram.reaction_sent"
    assert versioning.is_namespaced_event_kind(kind)
    assert not versioning.is_namespaced_event_kind("percept")
    for plugin, name in (("telegram.extra", "x"), ("Telegram", "x"),
                         ("telegram", "telegram.say"), ("telegram", "Not A Kind")):
        with pytest.raises(ValueError):
            versioning.plugin_kind(plugin, name)


async def test_append_event_rejects_a_malformed_kind(aconn):
    with pytest.raises(ValueError, match="invalid event kind"):
        await append_event(aconn, ts=START, kind="Not A Kind")
