"""Expectations: cues, the wake gate, firing, settlement, recovery."""

from __future__ import annotations

from datetime import datetime, timedelta
from zoneinfo import ZoneInfo

from psycopg.types.json import Jsonb

from monoagens import condition
from monoagens.condition import parse_duration
from monoagens.prior import constants
from monoagens.store import (append_event, fire_expectation, insert_expectation,
                             read_expectations, read_state, resolve_expectation)


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
    """Return whether an event matches a cue_event pattern."""
    if not pattern:
        return False
    fields = ("kind", "who", "place")
    if any(pattern.get(f) is not None and pattern[f] != getattr(event, f) for f in fields):
        return False
    if pattern.get("not_who") is not None and pattern["not_who"] == event.who:
        return False
    needle = pattern.get("text_like")
    haystack = str(event.payload.get("description") or event.payload.get("text") or "").lower()
    return needle is None or str(needle).lower() in haystack


def validate(data, *, now, settings):
    """Validate one `expect` tool call and return the row values to insert."""
    about, raw = str(data.get("about") or "").strip(), data.get("condition", data.get("expect"))
    if not about or not isinstance(raw, dict) or not raw:
        raise ValueError("expect needs about and a condition")
    kind = str(data.get("kind") or "promise")
    if kind not in ("promise", "wait", "predict", "guard"):
        raise ValueError(f"unknown expectation kind: {kind!r}")
    weight, policy = float(data.get("weight", 0.5)), data.get("policy") or {"once": True}
    if not 0.0 <= weight <= 1.0:
        raise ValueError(f"weight out of range: {weight}")
    if not isinstance(policy, dict) or not (policy.get("once") is True or policy.get("cooldown")):
        raise ValueError("policy must be {once: true} or {cooldown: duration}")
    if policy.get("cooldown"):
        parse_duration(policy["cooldown"])
    cue_time, cue_event = data.get("cue_time"), data.get("cue_event")
    expires = data.get("expires_at")
    if not cue_time and not cue_event:
        raise ValueError("expect needs cue_time or cue_event")
    values, zone = constants(settings), tz(settings)
    days = float(values["default_expires_days"])
    condition_value = condition.operator(raw)
    return {
        "kind": kind, "who": data.get("who"), "place": data.get("place"), "about": about,
        "cue_time": cue_time, "cue_event": cue_event, "condition": condition_value,
        "condition_text": condition.text(condition_value),
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
    if event.payload.get("addressed") is True or not event.place:
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
        if pattern.get("text_like") is not None:
            clauses.append("position(lower(%s) in lower(coalesce(payload->>'description', "
                           "payload->>'text', ''))) > 0")
            params.append(str(pattern["text_like"]))
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


async def due(conn, event, *, now, settings, kernel=None):
    """Fire or resolve expectations that are due for this event, after the wake gate."""
    if event.kind == "tick":
        where = "ex.status = 'active' AND (ex.next_check_at <= %s OR ex.expires_at <= %s)"
        params = (now, now)
    else:
        where, params = "ex.status = 'active' AND ex.cue_event IS NOT NULL", ()
    fired = []
    for row in await read_expectations(conn, where, params):
        if row["expires_at"] is not None and row["expires_at"] <= now:
            await resolve_expectation(conn, row, now, "dropped")  # stale
            continue
        if row["kind"] == "guard" or (event.kind != "tick"
                                      and not matches_cue(row["cue_event"], event)):
            continue
        if event.kind != "tick" and row["kind"] == "wait" and row["cue_event"].get("not_who"):
            await resolve_expectation(conn, row, now, "satisfied")  # patience held
            continue
        if condition.text(row["condition"]):
            if await condition.reminded(kernel, row, event):
                fired.append(await condition.reminder(conn, row, event, now))
            continue
        if not await is_met(conn, row["condition"], now):
            fired.append(await fire_expectation(conn, row, now))
        elif (row["policy"] or {}).get("once") is True:
            await resolve_expectation(conn, row, now, "satisfied")
        else:
            query = "UPDATE expectations SET next_check_at = %s WHERE id = %s"
            await conn.execute(query, (next_check(row["cue_time"], now, tz(settings)), row["id"]))
    return fired


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
    nxt = now + parse_duration(policy["cooldown"]) if policy.get("cooldown") else next_check(
        row["cue_time"], now, tz(settings))
    query = "UPDATE expectations SET status = 'active', next_check_at = %s WHERE id = %s"
    await conn.execute(query, (nxt, exp_id))


async def recovery(conn, *, now):
    """Find fired expectations left unanswered, resolving answered ones."""
    wakes = []
    for row in await read_expectations(conn, "ex.status = 'fired'"):
        hit = await (await conn.execute(
            "SELECT id FROM events WHERE kind = 'expectation.fired' "
            "AND payload->>'expectation_id' = %s ORDER BY id DESC LIMIT 1",
            (str(row["id"]),))).fetchone()
        fired_id = hit[0] if hit else None
        answered = fired_id is not None and await (await conn.execute(
            "SELECT 1 FROM events WHERE kind = 'act.out' AND caused_by = %s LIMIT 1",
            (fired_id,))).fetchone() is not None
        if answered:
            await resolve_expectation(conn, {**row, "fired_event": fired_id}, now, "satisfied")
        else:
            wakes.append({**row, "fired_event": fired_id, "trigger": None})
    return wakes
