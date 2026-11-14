"""Expectations: cues, the wake gate, firing, settlement, recovery."""

from __future__ import annotations

from datetime import datetime, timedelta
from typing import Any
from zoneinfo import ZoneInfo

from psycopg.types.json import Jsonb

from monoagens.infra.duration import parse_duration
from monoagens.infra.embedding import cosine
from monoagens.infra.prior import constants
from monoagens.infra.store import (append_event, fire_expectation, fired_for,
                                   insert_expectation, read_event_vector, read_expectations,
                                   read_state, resolve_expectation)

KINDS = ("promise", "wait", "predict", "guard")


def tz(settings):
    """Return the configured zoneinfo from config.yaml."""
    return ZoneInfo(str(settings.get("timezone") or "UTC"))


def next_check(cue, now, zone):
    """Return the next datetime a cue_time fires, or None when there is no cue."""
    if not cue:
        return None
    if "at" in cue:
        at = datetime.fromisoformat(str(cue["at"]))
        return at if at.tzinfo else at.replace(tzinfo=zone)
    if "every" in cue:
        return now + parse_duration(cue["every"])
    if "daily" in cue:
        h, m = map(int, str(cue["daily"]).split(":")[:2])
        at = now.astimezone(zone).replace(hour=h, minute=m, second=0, microsecond=0)
        return at if at > now else at + timedelta(days=1)
    raise ValueError(f"unknown cue_time: {cue!r}")


def matches_cue(pattern, event):
    """Return whether an event matches a cue_event pattern: facts only, never words."""
    if not pattern:
        return False
    fields = ("kind", "who", "place")
    if any(pattern.get(f) is not None and pattern[f] != getattr(event, f) for f in fields):
        return False
    return pattern.get("not_who") is None or pattern["not_who"] != event.who


def _refuse_words(pattern: Any, label: str = "cue_event") -> None:
    """Refuse a cue that carries anything but facts: words belong to `expect.text`."""
    if not isinstance(pattern, dict):
        return
    unknown = sorted(str(key) for key in pattern
                     if key not in ("kind", "who", "place", "not_who"))
    if unknown:
        raise ValueError(f"{label} takes facts only (kind, who, place, not_who), not "
                         f"{', '.join(unknown)}; use expect: {{text: ...}} for words")


def validate(data, *, now, settings):
    """Validate one `expect` tool call and return the row values to insert."""
    about = str(data.get("about") or "").strip()
    raw = data.get("condition", data.get("expect"))
    if not about or not isinstance(raw, dict) or not raw:
        raise ValueError("expect needs about and a condition")
    kind = str(data.get("kind") or "promise")
    if kind not in KINDS:
        raise ValueError(f"unknown expectation kind: {kind!r}")
    weight = float(data.get("weight", 0.5))
    policy = data.get("policy") or {"once": True}
    if not 0.0 <= weight <= 1.0:
        raise ValueError(f"weight out of range: {weight}")
    if not isinstance(policy, dict) or not (policy.get("once") is True or policy.get("cooldown")):
        raise ValueError("policy must be {once: true} or {cooldown: duration}")
    if policy.get("cooldown"):
        parse_duration(policy["cooldown"])
    cue_time, cue_event = data.get("cue_time"), data.get("cue_event")
    _refuse_words(cue_event)
    expires = data.get("expires_at")
    if not cue_time and not cue_event:
        raise ValueError("expect needs cue_time or cue_event")
    values, zone = constants(settings), tz(settings)
    days = float(values["default_expires_days"])
    condition_value = operator(raw)
    return {
        "kind": kind, "who": data.get("who"), "place": data.get("place"), "about": about,
        "cue_time": cue_time, "cue_event": cue_event, "condition": condition_value,
        "condition_text": text(condition_value),
        "intent": data.get("intent"), "weight": weight, "policy": dict(policy),
        "next_check_at": next_check(cue_time, now, zone),
        "expires_at": next_check({"at": str(expires)}, now, zone) if expires
        else now + timedelta(days=days),
    }


async def create(conn, data, *, now, settings, source):
    """Validate and store an expectation, recording its creation event."""
    row = validate(data, now=now, settings=settings)
    cur = await conn.execute("SELECT count(*) FROM expectations WHERE status = 'active'")
    if (await cur.fetchone())[0] >= int(constants(settings)["max_active"]):
        raise ValueError("too many active expectations")
    who, place = row["who"] or source.who, row["place"] or source.place
    async with conn.transaction():
        values = (row["kind"], who, place, row["about"], Jsonb(row["cue_time"]),
                  Jsonb(row["cue_event"]), Jsonb(row["condition"]), row["condition_text"],
                  row["intent"], row["weight"],
                  Jsonb(row["policy"]), row["next_check_at"], source.id, [source.id],
                  source.origin_actor or who, row["expires_at"])
        exp_id = await insert_expectation(conn, values)
        payload = {"expectation_id": exp_id, "kind": row["kind"], "about": row["about"],
                   "weight": row["weight"], "policy": row["policy"],
                   "expires_at": row["expires_at"].isoformat()}
        return await append_event(conn, ts=now, kind="expectation.created", who=who,
                                  place=place, body=source.body, payload=payload,
                                  caused_by=source.id)


async def create_wait(conn, event, *, now, settings, source=None):
    """Create the patience Expectation for a go on a turn not allocated to the soul."""
    if event.features.get("addressed") or not event.place:
        return None
    seconds = float(settings.get("knobs", {})["patience_seconds"])
    data = {"kind": "wait", "about": "waiting for someone else to take the floor",
            "cue_time": {"at": (now + timedelta(seconds=seconds)).isoformat()},
            "cue_event": {"kind": "percept", "place": event.place, "not_who": event.who},
            "expect": {"none": True}}
    return await create(conn, data, now=now, settings=settings, source=source or event)


async def resolve(conn, exp_id, status, *, now):
    """Resolve an expectation by id as satisfied or dropped."""
    found = await read_expectations(conn, "ex.id = %s", (exp_id,))
    if not found or status not in ("satisfied", "dropped"):
        raise ValueError(f"invalid resolve: {exp_id} {status!r}")
    await resolve_expectation(conn, {**found[0], "fired_event": None}, now, status)


async def is_met(conn, condition, now):
    """Return whether an exact `expect` condition currently holds."""
    if not condition or "none" in condition or condition.get("text"):
        return False
    if "event_seen" in condition:
        pattern = condition["event_seen"]
        fields = [f for f in ("kind", "who", "place") if pattern.get(f) is not None]
        clauses, params = [f"{f} = %s" for f in fields], [pattern[f] for f in fields]
        if pattern.get("not_who") is not None:
            clauses.append("who <> %s")
            params.append(pattern["not_who"])
        params.extend([now - parse_duration(condition["within"]), now])
        sql = ("SELECT 1 FROM events WHERE "
               + " AND ".join([*clauses, "ts BETWEEN %s AND %s"]) + " LIMIT 1")
        return (await (await conn.execute(sql, params)).fetchone()) is not None
    stored = await read_state(conn, str(condition["state"]))
    got = stored.get("value", stored) if isinstance(stored, dict) else stored
    if got is None:
        return False
    try:
        if condition["op"] == "eq":
            return got == condition["value"]
        left, right = float(got), float(condition["value"])
        return left > right if condition["op"] == "gt" else left < right
    except (TypeError, ValueError):
        return False


async def due(conn, event, *, now, settings, pfc=None, embedder=None, on_error=None):
    """Fire, remind or resolve the expectations that are due for this event.

    `on_error` receives one row's failure so a malformed expectation loses no other fired
    item (F.5); without it the failure propagates.
    """
    if event.kind == "tick":
        where = "ex.status = 'active' AND (ex.next_check_at <= %s OR ex.expires_at <= %s)"
        params = (now, now)
    else:
        where, params = "ex.status = 'active' AND ex.cue_event IS NOT NULL", ()
    wakes = []
    for row in await read_expectations(conn, where, params):
        try:
            wakes.extend(await _due_one(conn, row, event, now=now, settings=settings,
                                        pfc=pfc, embedder=embedder))
        except Exception as exc:  # one malformed row never aborts the others (F.5)
            if on_error is None:
                raise
            await on_error(exc)
    return wakes


async def _due_one(conn, row, event, *, now, settings, pfc=None, embedder=None):
    """Return the wakes one expectation earns for this event, and record what it settles."""
    if row["expires_at"] is not None and row["expires_at"] <= now:
        await resolve_expectation(conn, row, now, "dropped")  # stale
        return ()
    if row["kind"] == "guard":
        return ()
    if event.kind != "tick" and not matches_cue(row["cue_event"], event):
        return ()
    if event.kind != "tick" and row["kind"] == "wait" and row["cue_event"].get("not_who"):
        await resolve_expectation(conn, row, now, "satisfied")  # patience held
        return ()
    if text(row["condition"]):
        if await reminded(conn, row, event, pfc=pfc, embedder=embedder, settings=settings):
            return (await reminder(conn, row, event, now),)
        return ()
    if not await is_met(conn, row["condition"], now):
        return (await fire_expectation(conn, row, now),)
    if (row["policy"] or {}).get("once") is True:
        await resolve_expectation(conn, row, now, "satisfied")
        return ()
    await conn.execute("UPDATE expectations SET next_check_at = %s WHERE id = %s",
                       (next_check(row["cue_time"], now, tz(settings)), row["id"]))
    return ()


async def settle(conn, exp_id, *, now, settings):
    """Apply the firing policy of a fired expectation after it was handled."""
    rows = await read_expectations(conn, "ex.id = %s", (exp_id,))
    row = rows[0] if rows else None
    if row is None or row["status"] != "fired":
        return
    policy = row["policy"] or {}
    max_fires = policy.get("max_fires")
    maxed = max_fires is not None and row["fire_count"] >= int(max_fires)
    if policy.get("once") is True or maxed:
        status = "satisfied" if policy.get("once") is True else "dropped"
        return await resolve_expectation(conn, {**row, "fired_event": None}, now, status)
    if policy.get("cooldown"):
        nxt = now + parse_duration(policy["cooldown"])
    else:
        nxt = next_check(row["cue_time"], now, tz(settings))
    query = "UPDATE expectations SET status = 'active', next_check_at = %s WHERE id = %s"
    await conn.execute(query, (nxt, exp_id))


async def recovery(conn, *, now):
    """Find fired expectations left unanswered, resolving answered ones."""
    wakes = []
    for row in await read_expectations(conn, "ex.status = 'fired'"):
        hit = await fired_for(conn, int(row["id"]))
        fired_id = hit[0] if hit else None
        answered = False
        if fired_id is not None:
            cur = await conn.execute("SELECT 1 FROM events WHERE kind = 'act.out' "
                                     "AND caused_by = %s LIMIT 1", (fired_id,))
            answered = await cur.fetchone() is not None
        if answered:
            await resolve_expectation(conn, {**row, "fired_event": fired_id}, now, "satisfied")
        else:
            wakes.append({**row, "fired_event": fired_id, "trigger": None})
    return wakes


def operator(expect):
    """Return one `expect` condition as its exact fast path or its free text."""
    if "none" in expect:
        return {"none": True}
    if "event_seen" in expect:
        seen = expect["event_seen"] if isinstance(expect["event_seen"], dict) else {}
        stated = dict(seen.get("pattern") or seen)
        pattern = {key: value for key, value in stated.items() if key != "within"}
        _refuse_words(pattern, "event_seen")
        within = expect.get("within") or seen.get("within")
        if not pattern or not within:
            raise ValueError("event_seen needs a pattern and a within window")
        parse_duration(within)
        return {"event_seen": pattern, "within": str(within)}
    if "state" in expect or "key" in expect:
        key, op = expect.get("state") or expect.get("key"), expect.get("op")
        if not isinstance(key, str) or op not in ("eq", "gt", "lt") or "value" not in expect:
            raise ValueError("state needs a key, an op in eq|gt|lt and a value")
        return {"state": key, "op": str(op), "value": expect["value"]}
    if "text" in expect or "condition_text" in expect:
        text = str(expect.get("text") or expect.get("condition_text") or "").strip()
        if not text:
            raise ValueError("a free-text condition needs text")
        return {"text": text}
    raise ValueError(f"unknown expect operator: {expect!r}")


def text(condition):
    """Return the free text of a condition, or None for an exact fast path."""
    return str((condition or {}).get("text") or "").strip() or None


async def reminded(conn, row, event, *, pfc, embedder, settings):
    """Return whether the line reminds the condition: cosine reaches `lo` from the prior."""
    query = event.description
    if pfc is None or embedder is None or not query:
        return False
    left = await read_event_vector(conn, event.id) if event.id is not None else None
    if left is None:
        left = (await embedder.embed([query]))[0]
    condition_text = row["condition_text"] or text(row["condition"])
    right = (await embedder.embed([condition_text]))[0]
    lo = float((settings.get("calibration") or {})["wake_gate_lo"])
    return cosine(left, right) >= lo


async def reminder(conn, row, event, now):
    """Record the reminder wake: the line, its speaker and the Expectation are its trigger.

    It is not a fulfilment: the Expectation keeps its status and its fire count.
    """
    payload = {"expectation_id": row["id"], "about": row["about"],
               "condition": row["condition_text"] or text(row["condition"]) or "",
               "text": event.description, "line_place": event.place,
               "reminder": True}
    reminded_event = await append_event(conn, ts=now, kind="expectation.reminded",
                                        who=event.who, place=event.place, body=event.body,
                                        origin_actor=row["origin_actor"],
                                        caused_by=event.id, description=event.description,
                                        payload=payload)
    return {**row, "trigger": reminded_event}
