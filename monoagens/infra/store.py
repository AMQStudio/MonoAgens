"""The Log and Status Board persistence (events, state, expectations)."""

from __future__ import annotations

from collections.abc import Sequence
from datetime import datetime
from typing import Any
from zoneinfo import ZoneInfo

from pgvector import Vector
from psycopg.types.json import Jsonb

from monoagens.contract import SCHEMA_VERSION, Action, Event, Receipt
from monoagens.types import StateEntry
from monoagens.infra.versioning import is_valid_event_kind, upcast_event

EXPECTATION_COLUMNS = (
    "id", "kind", "who", "place", "about", "cue_time", "cue_event", "condition",
    "condition_text", "intent", "weight", "policy", "status", "next_check_at",
    "last_fired_at", "fire_count", "source_event", "derived_from", "origin_actor",
    "expires_at", "source_body",
)
_EXPECTATIONS = ("SELECT " + ", ".join("ex." + column for column in EXPECTATION_COLUMNS[:-1])
                 + ", s.body AS source_body FROM expectations ex"
                 " LEFT JOIN events s ON s.id = ex.source_event")
_INSERT_EXPECTATION = (
    "INSERT INTO expectations (kind, who, place, about, cue_time, cue_event, condition, "
    "condition_text, intent, weight, policy, status, next_check_at, source_event, derived_from, "
    "origin_actor, expires_at) VALUES "
    "(%s, %s, %s, %s, %s, %s, %s, %s, %s, %s, %s, 'active', %s, %s, %s, %s, %s) RETURNING id"
)
# The whole envelope of contract.md §3, in the order every read uses.
EVENT_COLUMNS = ("id", "ts", "occurred_at", "kind", "description", "who", "place", "body",
                 "caused_by", "origin_actor", "features", "backfill", "imported", "payload",
                 "labels", "schema_version")
_EVENT_COLS = ", ".join(EVENT_COLUMNS)


def event_from_row(row) -> Event:
    """Return the event one row of the Log's columns describes, upcast on read."""
    return upcast_event(dict(zip(EVENT_COLUMNS, row)))


def payload_text(payload: dict[str, Any]) -> str:
    """Return the human-readable text of one event payload.

    The middleware and the adapters read their own payload this way; the core reads
    `Event.description` instead (contract.md §3).
    """
    text = next((payload[key] for key in ("description", "text", "reason", "about")
                 if payload.get(key)), "")
    return str(text or (payload.get("arguments") or {}).get("text") or "")


async def append_event(conn, *, ts, kind, who=None, place=None, body=None, payload=None,
                       caused_by=None, origin_actor=None, occurred_at=None,
                       description: str | None = None, features=None, backfill: bool = False,
                       imported: bool = False, labels=None,
                       schema_version: int = SCHEMA_VERSION) -> Event:
    """Append one event to the immutable Log; origin inherits along caused_by.

    A field the caller leaves empty is read from the payload through the contract's own upcast
    rule, so no caller has to repeat the version 1 shape while it moves to the envelope.
    """
    if ts is None or kind is None:
        raise ValueError("ts and kind must be explicitly provided")
    if not is_valid_event_kind(kind):
        raise ValueError(f"invalid event kind: {kind!r}")
    if schema_version < 1:
        raise ValueError(f"schema_version must be positive: {schema_version!r}")
    translated = upcast_event({"ts": ts, "kind": kind, "payload": dict(payload or {})})
    written = description or translated.description
    occurred_at = occurred_at or translated.occurred_at
    features = {**translated.features, **dict(features or {})}
    backfill = backfill or translated.backfill
    imported = imported or translated.imported
    query = ("INSERT INTO events (ts, occurred_at, kind, description, who, place, body, "
             "caused_by, origin_actor, features, backfill, imported, payload, labels, "
             "schema_version) VALUES (%s, %s, %s, %s, %s, %s, %s, %s, COALESCE(%s, "
             "(SELECT origin_actor FROM events WHERE id = %s), %s), %s, %s, %s, %s, %s, %s) "
             f"RETURNING {_EVENT_COLS}")
    cur = await conn.execute(query, (ts, occurred_at, kind, written, who, place, body,
                                    caused_by, origin_actor, caused_by, who,
                                    Jsonb(dict(features or {})), backfill, imported,
                                    Jsonb(dict(payload or {})), Jsonb(dict(labels or {})),
                                    schema_version))
    return event_from_row(await cur.fetchone())


async def append_envelope(conn, event: Event, *, ts) -> Event:
    """Append one received event's whole envelope as it stands.

    This is the one place the envelope's opaque fields (`labels` among them) are handed to
    the Log; the core stores an event without reading what the middleware attached to it.
    """
    return await append_event(
        conn, ts=ts, kind=event.kind, who=event.who, place=event.place, body=event.body,
        payload=dict(event.payload or {}), caused_by=event.caused_by,
        origin_actor=event.origin_actor, occurred_at=event.occurred_at,
        description=event.description or None, features=event.features,
        backfill=event.backfill, imported=event.imported, labels=event.labels)


async def insert_event_vector(conn, event_id: int, embedding: Sequence[float],
                              derived_from: Sequence[int] | None = None) -> None:
    """Write the one embedding of an event; never overwrite an existing one."""
    lineage = list(derived_from) if derived_from is not None else [event_id]
    await conn.execute(
        "INSERT INTO event_vectors (event_id, embedding, derived_from) VALUES (%s, %s, %s) "
        "ON CONFLICT (event_id) DO NOTHING",
        (event_id, Vector(list(embedding)), lineage),
    )


async def insert_event_vectors(conn, items: Sequence[tuple[int, Sequence[float]]]) -> None:
    """Batch-insert event embeddings without overwriting existing vectors."""
    if not items:
        return
    rows = [(int(eid), Vector(list(vec)), [int(eid)]) for eid, vec in items]
    async with conn.cursor() as cur:
        await cur.executemany(
            "INSERT INTO event_vectors (event_id, embedding, derived_from) "
            "VALUES (%s, %s, %s) ON CONFLICT (event_id) DO NOTHING",
            rows,
        )


async def unembedded_imported_percepts(conn) -> list[tuple[int, str]]:
    """Return imported percept `(id, text)` pairs that still lack an embedding vector."""
    cur = await conn.execute(
        "SELECT e.id, e.description FROM events e "
        "LEFT JOIN event_vectors v ON v.event_id = e.id "
        "WHERE e.kind = 'percept' AND e.imported = true AND v.event_id IS NULL "
        "  AND BTRIM(e.description) <> '' ORDER BY e.id ASC"
    )
    return [(int(eid), str(text)) for eid, text in await cur.fetchall()]


async def read_event_vector(conn, event_id: int) -> list[float] | None:
    """Return the embedding already stored for an event, if any."""
    cur = await conn.execute("SELECT embedding FROM event_vectors WHERE event_id = %s",
                            (event_id,))
    row = await cur.fetchone()
    if row is None:
        return None
    vector = row[0]
    return [float(x) for x in (vector.to_list() if hasattr(vector, "to_list") else vector)]


async def _recent(conn, where: str, params: list[Any], limit: int,
                  chronological: bool) -> list[Event]:
    query = f"SELECT {_EVENT_COLS} FROM events WHERE {where} ORDER BY id DESC LIMIT %s"
    cur = await conn.execute(query, [*params, limit])
    events = [event_from_row(row) for row in await cur.fetchall()]
    return list(reversed(events)) if chronological else events


async def read_last_events(conn, place, limit=20, *, chronological=True,
                           kinds: Sequence[str] | None = None) -> list[Event]:
    """Return the last events for a place, optionally restricted to some kinds."""
    where, params = "place = %s", [place]
    if kinds is not None:
        where, params = f"{where} AND kind = ANY(%s)", [place, list(kinds)]
    return await _recent(conn, where, params, limit, chronological)


async def read_event_row(conn, event_id: int) -> dict[str, Any] | None:
    """Return one event's envelope as a mapping, or None when the Log has no such id."""
    row = await (await conn.execute(
        f"SELECT {_EVENT_COLS} FROM events WHERE id = %s", (int(event_id),))).fetchone()
    return dict(zip(EVENT_COLUMNS, row)) if row else None


async def cursor(conn, body) -> Any | None:
    """Return the last cursor payload recorded by a body, if any."""
    cur = await conn.execute("SELECT payload->'cursor' FROM events WHERE body = %s "
                             "AND payload ? 'cursor' ORDER BY id DESC LIMIT 1", (body,))
    row = await cur.fetchone()
    return row[0] if row else None


async def read_state(conn, key) -> dict[str, Any] | None:
    """Return the JSON value of a state key, if present."""
    cur = await conn.execute("SELECT value FROM state WHERE key = %s", (key,))
    row = await cur.fetchone()
    return row[0] if row else None


async def read_state_entry(conn, key) -> StateEntry | None:
    """Return a state row together with the event that wrote it."""
    cur = await conn.execute(
        "SELECT key, value, updated_by_event FROM state WHERE key = %s", (key,)
    )
    row = await cur.fetchone()
    return StateEntry(row[0], row[1], row[2]) if row else None


async def write_state(conn, key, value, updated_by_event) -> None:
    """Upsert one state key with the event that produced it."""
    query = ("INSERT INTO state (key, value, updated_by_event) VALUES (%s, %s, %s) "
             "ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value, "
             "updated_by_event = EXCLUDED.updated_by_event")
    await conn.execute(query, (key, Jsonb(value), updated_by_event))


async def merge_source(event: Event) -> tuple[str, str] | None:
    """Return the `(from, into)` source ids of a merge event, or None when it names none.

    A merge carries two ids in one event and the envelope has one `place` column, so this is
    the one place the core lets the store read a payload for it (D-B).
    """
    payload = event.payload or {}
    src, dst = str(payload.get("from") or ""), str(payload.get("into") or "")
    return (src, dst) if src and dst else None


async def write_action(conn, action: Action, event_id: int, *,
                       communicative: bool = False) -> None:
    """Record one action in the ledger before its body sees it, and point at it by event.

    The row is the fact (D-F); the `act:<event id>` pointer lets a later `act.result`
    find its action without reading a payload.
    """
    await write_state(conn, f"action:{action.id}", {
        "capability": action.capability,
        "place": action.place,
        "arguments": dict(action.arguments),
        "caused_by": action.caused_by,
        "communicative": bool(communicative),
        "event": int(event_id),
        "receipt": None,
    }, event_id)
    await write_state(conn, f"act:{int(event_id)}", {"id": action.id}, event_id)


async def close_action(conn, action_id: str, receipt: Receipt, event_id: int) -> None:
    """Write one action's receipt into its ledger row."""
    key = f"action:{action_id}"
    current = await read_state(conn, key)
    if current is None:
        return
    await write_state(conn, key, {**current, "receipt": {
        "state": receipt.state,
        "reason": receipt.reason,
        "result": receipt.result,
    }}, event_id)


async def action_of(conn, act_out_id: int) -> dict[str, Any] | None:
    """Return the action ledger row one `act.out` event records, if it has one."""
    return (await actions_of(conn, [act_out_id])).get(int(act_out_id))


async def actions_of(conn, act_out_ids: Sequence[int]) -> dict[int, dict[str, Any]]:
    """Return the action ledger row of many `act.out` events, keyed by the event id."""
    ids = [int(item) for item in act_out_ids if item is not None]
    if not ids:
        return {}
    cur = await conn.execute(
        "SELECT p.key, a.value FROM state p JOIN state a "
        "ON a.key = 'action:' || (p.value->>'id') WHERE p.key = ANY(%s)",
        ([f"act:{item}" for item in ids],))
    return {int(str(key).split(":", 1)[1]): dict(value or {})
            for key, value in await cur.fetchall()}


async def unresolved_actions(conn) -> list[dict[str, Any]]:
    """Return every ledger row whose action still has no receipt, oldest first."""
    cur = await conn.execute(
        "SELECT key, value FROM state WHERE key LIKE 'action:%' "
        "AND (value->>'receipt') IS NULL ORDER BY key")
    found = []
    for key, value in await cur.fetchall():
        ledger = dict(value or {})
        found.append({
            "id": str(key)[len("action:"):],
            "capability": str(ledger.get("capability") or ""),
            "place": str(ledger.get("place") or ""),
            "arguments": dict(ledger.get("arguments") or {}),
            "caused_by": ledger.get("caused_by"),
            "act_out": ledger.get("event"),
        })
    return found


async def add_budget_usage(conn, *, now: datetime, timezone: str, usage: dict[str, int],
                           updated_by_event: int) -> dict[str, int]:
    """Add one LLM call's token usage to the budget key of its local date; return totals."""
    key = f"budget:{now.astimezone(ZoneInfo(timezone)).date().isoformat()}"
    current = await read_state(conn, key) or {}
    totals = {name: int(current.get(name) or 0) + int(usage.get(name) or 0)
              for name in ("prompt_tokens", "cached_tokens", "completion_tokens",
                           "reasoning_tokens", "total_tokens")}
    await write_state(conn, key, totals, updated_by_event)
    return totals


async def project_event(conn, event: Event) -> None:
    """Project one event onto the Status Board."""
    if event.id is None:
        raise ValueError("Cannot project an event without an id")
    if event.who is not None:
        value = {"ts": event.ts.isoformat(), "place": event.place, "body": event.body}
        await write_state(conn, f"last_seen:{event.who}", value, event.id)
    if event.kind == "act.out" and event.place is not None:
        await write_state(conn, f"last_spoke:{event.place}", {"ts": event.ts.isoformat()}, event.id)
    if event.kind == "sleep.done":
        if "core_memory" in event.payload:
            core = {"text": event.payload["core_memory"],
                    "derived_from": event.payload.get("derived_from") or []}
            await write_state(conn, "core_memory", core, event.id)
        last = {"id": event.payload.get("last_event_id", event.id)}
        await write_state(conn, "sleep:last_event", last, event.id)
    if event.kind in ("identity.born", "identity.updated"):
        if "identity" in event.payload:
            await write_state(conn, "identity", event.payload["identity"], event.id)


async def rebuild_state(conn) -> None:
    """Drop state and replay the whole Log into it."""
    await conn.execute("DELETE FROM state")
    cur = await conn.execute(f"SELECT {_EVENT_COLS} FROM events ORDER BY id ASC")
    for row in await cur.fetchall():
        await project_event(conn, event_from_row(row))


async def read_expectations(conn, where="TRUE", params=()) -> list[dict[str, Any]]:
    """Read expectation rows joined with their source body, filtered by where.

    A stored `text_like` cue is upcast here, in memory, into a free-text condition: the
    words it matched are what the condition compares, and the cue keeps facts only (D-K).
    The stored row is never rewritten.
    """
    cur = await conn.execute(f"{_EXPECTATIONS} WHERE {where} ORDER BY ex.id", params)
    return [_upcast_cue(dict(zip(EXPECTATION_COLUMNS, row))) for row in await cur.fetchall()]


def _upcast_cue(row: dict[str, Any]) -> dict[str, Any]:
    """Rewrite one stored `text_like` cue as a free-text condition, or return it unchanged.

    A cue stored before the contract spoke only facts may still carry the phrase it matched;
    that phrase becomes the condition's words, and the stored row is left alone (D-K).
    """
    cue = row.get("cue_event")
    if not isinstance(cue, dict):
        return row
    phrase = cue.get("text_like")
    cleaned = {key: value for key, value in cue.items() if key != "text_like"}
    if phrase is None:
        seen = cue.get("event_seen")
        if not isinstance(seen, dict) or seen.get("text_like") is None:
            seen = (row.get("condition") or {}).get("event_seen")
            if not isinstance(seen, dict) or seen.get("text_like") is None:
                return row
        phrase = seen["text_like"]
        cleaned = {**cleaned, "event_seen": {key: value for key, value in seen.items()
                                             if key != "text_like"}}
    condition = dict(row.get("condition") or {})
    if not condition.get("text"):
        condition = {"text": str(phrase)}
    return {**row, "cue_event": cleaned, "condition": condition,
            "condition_text": str(condition.get("text") or row.get("condition_text") or "")}



async def insert_expectation(conn, values) -> int:
    """Insert one expectation row and return its id."""
    cur = await conn.execute(_INSERT_EXPECTATION, values)
    return int((await cur.fetchone())[0])


async def resolve_expectation(conn, row, now: datetime, status: str) -> Event:
    """Resolve one expectation and record the event, in one transaction."""
    async with conn.transaction():
        await conn.execute(
            "UPDATE expectations SET status = %s, next_check_at = NULL WHERE id = %s",
            (status, row["id"]),
        )
        payload = {"expectation_id": row["id"], "about": row["about"], "status": status}
        return await append_event(conn, ts=now, kind="expectation.resolved", who=row["who"],
                                  place=row["place"], body=row.get("source_body"),
                                  caused_by=row.get("fired_event"), payload=payload)


async def fired_for(conn, expectation_id: int) -> tuple[int, ...] | None:
    """Return the newest `expectation.fired` event of one expectation, as a row tuple.

    The store wrote that payload's `expectation_id`, so the store is the one place that
    reads it back; the core asks this question in ids only.
    """
    return await (await conn.execute(
        "SELECT id FROM events WHERE kind = 'expectation.fired' "
        "AND payload->>'expectation_id' = %s ORDER BY id DESC LIMIT 1",
        (str(int(expectation_id)),))).fetchone()


async def fire_expectation(conn, row, now: datetime) -> dict[str, Any]:
    """Mark one expectation fired, record it, and return it with the trigger."""
    async with conn.transaction():
        payload = {"expectation_id": row["id"], "about": row["about"],
                   "intent": row["intent"], "weight": row["weight"]}
        fired = await append_event(conn, ts=now, kind="expectation.fired", who=row["who"],
                                   place=row["place"], body=row["source_body"],
                                   caused_by=row["source_event"],
                                   description=str(row["about"] or ""), payload=payload)
        await conn.execute("UPDATE expectations SET status = 'fired', "
                           "fire_count = fire_count + 1, last_fired_at = %s WHERE id = %s",
                           (now, row["id"]))
    return {**row, "fired_event": fired.id, "trigger": fired}


async def active_expectation_lines(conn, place) -> list[str]:
    """Render the active expectations for a place as prompt lines."""
    cur = await conn.execute("SELECT id, about, intent, next_check_at FROM expectations "
                             "WHERE status = 'active' AND (place = %s OR place IS NULL) "
                             "ORDER BY id LIMIT 20", (place,))
    return [f"#{i} {about} ({intent or 'no intent stated'}"
            f"{f', next check {at:%Y-%m-%d %H:%M}' if at else ''})"
            for i, about, intent, at in await cur.fetchall()]
