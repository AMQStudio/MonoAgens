"""Expectations: cues, the three `expect` operators, firing, settlement, recovery."""
from __future__ import annotations
import re
from datetime import datetime, timedelta
from zoneinfo import ZoneInfo

from psycopg.types.json import Jsonb

from monoagens.store import (append_event, fire_expectation, insert_expectation, read_expectations, read_state,
                             resolve_expectation)

_UNITS = {"s": 1, "m": 60, "h": 3600, "d": 86400}
_RE = re.compile(r"^(\d+(?:\.\d+)?)\s*([smhd])$")
_OPS = ("eq", "gt", "lt")

def tz(soul):
    return ZoneInfo(str(soul.get("timezone") or "UTC"))

def parse_duration(value):
    m = _RE.match(str(value).strip().lower())
    if m is None: raise ValueError(f"invalid duration: {value!r}")
    return timedelta(seconds=float(m[1]) * _UNITS[m[2]])

def next_check(cue, now, zone):
    if not cue: return None
    if "at" in cue:
        at = datetime.fromisoformat(str(cue["at"]))
        return at if at.tzinfo else at.replace(tzinfo=zone)
    if "every" in cue: return now + parse_duration(cue["every"])
    if "daily" in cue:
        h, m = map(int, str(cue["daily"]).split(":")[:2])
        at = now.astimezone(zone).replace(hour=h, minute=m, second=0, microsecond=0)
        return at if at > now else at + timedelta(days=1)
    raise ValueError(f"unknown cue_time: {cue!r}")

def matches_cue(pattern, event):
    if not pattern: return False
    if any(pattern.get(f) is not None and pattern[f] != getattr(event, f) for f in ("kind", "who", "place")):
        return False
    needle = pattern.get("text_like")
    return needle is None or str(needle).lower() in str(event.payload.get("text") or "").lower()

def _operator(expect):
    if "none" in expect: return {"none": True}
    if "event_seen" in expect:
        seen = expect["event_seen"] if isinstance(expect["event_seen"], dict) else {}
        pattern = {k: v for k, v in dict(seen.get("pattern") or seen).items() if k != "within"}
        within = expect.get("within") or seen.get("within")
        if not pattern or not within: raise ValueError("event_seen needs a pattern and a within window")
        parse_duration(within)
        return {"event_seen": pattern, "within": str(within)}
    if "state" in expect or "key" in expect:
        key, op = expect.get("state") or expect.get("key"), expect.get("op")
        if not isinstance(key, str) or op not in _OPS or "value" not in expect:
            raise ValueError("state needs a key, an op in eq|gt|lt and a value")
        return {"state": key, "op": str(op), "value": expect["value"]}
    raise ValueError(f"unknown expect operator: {expect!r}")

def validate(data, *, now, soul):
    about, raw = str(data.get("about") or "").strip(), data.get("expect")
    if not about or not isinstance(raw, dict) or not raw: raise ValueError("expect needs about and a condition")
    weight, policy = float(data.get("weight", 0.5)), data.get("policy") or {"once": True}
    if not 0.0 <= weight <= 1.0: raise ValueError(f"weight out of range: {weight}")
    if not isinstance(policy, dict) or not (policy.get("once") is True or policy.get("cooldown")):
        raise ValueError("policy must be {once: true} or {cooldown: duration}")
    if policy.get("cooldown"): parse_duration(policy["cooldown"])
    cue_time, cue_event, expires = data.get("cue_time"), data.get("cue_event"), data.get("expires_at")
    if not cue_time and not cue_event: raise ValueError("expect needs cue_time or cue_event")
    days = float(soul["expectations"]["default_expires_days"])
    return {"who": data.get("who"), "place": data.get("place"), "about": about, "cue_time": cue_time,
            "cue_event": cue_event, "expect": _operator(raw), "intent": data.get("intent"), "weight": weight,
            "policy": dict(policy), "next_check_at": next_check(cue_time, now, tz(soul)),
            "expires_at": next_check({"at": str(expires)}, now, tz(soul)) if expires else now + timedelta(days=days)}

async def create(conn, data, *, now, soul, source):
    row = validate(data, now=now, soul=soul)
    limit = int(soul["expectations"]["max_active"])
    if (await (await conn.execute("SELECT count(*) FROM expectations WHERE status = 'active'")).fetchone())[0] >= limit:
        raise ValueError("too many active expectations")
    who, place = row["who"] or source.who, row["place"] or source.place
    async with conn.transaction():
        values = (who, place, row["about"], Jsonb(row["cue_time"]), Jsonb(row["cue_event"]), Jsonb(row["expect"]),
                  row["intent"], row["weight"], Jsonb(row["policy"]), row["next_check_at"], source.id, row["expires_at"])
        exp_id = await insert_expectation(conn, values)
        payload = {"expectation_id": exp_id, "about": row["about"], "weight": row["weight"],
                   "policy": row["policy"], "expires_at": row["expires_at"].isoformat()}
        return await append_event(conn, ts=now, kind="expectation.created", who=who, place=place,
                                  body=source.body, payload=payload, caused_by=source.id)

async def resolve(conn, exp_id, status, *, now):
    row = (await read_expectations(conn, "ex.id = %s", (exp_id,)) or [None])[0]
    if row is None or status not in ("satisfied", "dropped"):
        raise ValueError(f"invalid resolve: {exp_id} {status!r}")
    await resolve_expectation(conn, {**row, "fired_event": None}, now, status)

async def is_met(conn, condition, now):
    if "none" in condition: return False
    if "event_seen" in condition:
        pattern, clauses, params = condition["event_seen"], [], []
        for field in ("kind", "who", "place"):
            if pattern.get(field) is not None: clauses.append(f"{field} = %s"); params.append(pattern[field])
        if pattern.get("text_like") is not None:
            clauses.append("position(lower(%s) in lower(coalesce(payload->>'text', ''))) > 0")
            params.append(str(pattern["text_like"]))
        params.extend([now - parse_duration(condition["within"]), now])
        sql = "SELECT 1 FROM events WHERE " + " AND ".join([*clauses, "ts BETWEEN %s AND %s"]) + " LIMIT 1"
        return (await (await conn.execute(sql, params)).fetchone()) is not None
    stored = await read_state(conn, str(condition["state"]))
    got = (stored.get("value", stored) if isinstance(stored, dict) else stored) if stored is not None else None
    if got is None: return False
    try:
        if condition["op"] == "eq": return got == condition["value"]
        return float(got) > float(condition["value"]) if condition["op"] == "gt" else float(got) < float(condition["value"])
    except (TypeError, ValueError): return False

async def due(conn, event, *, now, soul):
    where, params = (("ex.status = 'active' AND ex.next_check_at <= %s", (now,)) if event.kind == "tick"
                     else ("ex.status = 'active' AND ex.cue_event IS NOT NULL", ()))
    fired = []
    for row in await read_expectations(conn, where, params):
        if event.kind != "tick" and not matches_cue(row["cue_event"], event): continue
        if not await is_met(conn, row["expect"], now): fired.append(await fire_expectation(conn, row, now))
        elif (row["policy"] or {}).get("once") is True: await resolve_expectation(conn, row, now, "satisfied")
        else: await conn.execute("UPDATE expectations SET next_check_at = %s WHERE id = %s",
                                 (next_check(row["cue_time"], now, tz(soul)), row["id"]))
    return fired

async def settle(conn, exp_id, *, now, soul):
    row = (await read_expectations(conn, "ex.id = %s", (exp_id,)) or [None])[0]
    if row is None or row["status"] != "fired": return
    policy, max_fires = row["policy"] or {}, (row["policy"] or {}).get("max_fires")
    if policy.get("once") is True or (max_fires is not None and row["fire_count"] >= int(max_fires)):
        return await resolve_expectation(conn, {**row, "fired_event": None}, now,
                                         "satisfied" if policy.get("once") is True else "dropped")
    nxt = now + parse_duration(policy["cooldown"]) if policy.get("cooldown") else next_check(row["cue_time"], now, tz(soul))
    await conn.execute("UPDATE expectations SET status = 'active', next_check_at = %s WHERE id = %s", (nxt, exp_id))

async def recovery(conn, *, now):
    wakes = []
    for row in await read_expectations(conn, "ex.status = 'fired'"):
        hit = await (await conn.execute("SELECT id FROM events WHERE kind = 'expectation.fired' "
                                        "AND payload->>'expectation_id' = %s ORDER BY id DESC LIMIT 1",
                                        (str(row["id"]),))).fetchone()
        fired_id = hit[0] if hit else None
        answered = fired_id is not None and await (await conn.execute(
            "SELECT 1 FROM events WHERE kind = 'msg.out' AND caused_by = %s LIMIT 1", (fired_id,))).fetchone()
        if answered: await resolve_expectation(conn, {**row, "fired_event": fired_id}, now, "satisfied")
        else: wakes.append({**row, "fired_event": fired_id, "trigger": None})
    return wakes
