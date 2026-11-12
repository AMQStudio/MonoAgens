"""Conditions: exact fast paths and the free-text wake gate (docs/core.md §5)."""

from __future__ import annotations

import math
import re
from datetime import timedelta

from pgvector import Vector

from monoagens import budget
from monoagens.store import (fire_expectation, read_event_vector, read_expectations,
                             read_state, write_state)

_UNITS = {"s": 1, "m": 60, "h": 3600, "d": 86400}
_RE = re.compile(r"^(\d+(?:\.\d+)?)\s*([smhd])$")
BANDS = "condition:bands"
JUDGE = {"type": "function", "function": {
    "name": "classify", "description": "Answer whether the event fulfils the condition.",
    "parameters": {"type": "object", "required": ["verdict"], "properties": {
        "verdict": {"type": "string", "enum": ["yes", "no"]}}}}}
SYSTEM = ("You judge one event against one condition. Answer with the classify tool: "
          "yes only when the event itself fulfils or matches the condition, no otherwise.")


def parse_duration(value):
    """Parse a duration like "2d", "6h", "30m", or "45s" into a timedelta."""
    if (m := _RE.match(str(value).strip().lower())) is None:
        raise ValueError(f"invalid duration: {value!r}")
    return timedelta(seconds=float(m[1]) * _UNITS[m[2]])


def operator(expect):
    """Return one `expect` condition as its exact fast path or its free text."""
    if "none" in expect:
        return {"none": True}
    if "event_seen" in expect:
        seen = expect["event_seen"] if isinstance(expect["event_seen"], dict) else {}
        pattern = {key: value for key, value in dict(seen.get("pattern") or seen).items()
                   if key != "within"}
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


def _limits(settings):
    """Return the prior's edges, bounds, steps and the rule-of-three count."""
    values, calibration = settings["constants"], settings.get("calibration") or {}
    return (float(calibration["wake_gate_lo"]), float(calibration["wake_gate_hi"]),
            float(values["wake_gate_floor"]), float(values["wake_gate_ceiling"]),
            float(values["wake_gate_margin"]), float(values["wake_gate_min_gray"]),
            float(values["wake_gate_step"]), float(values["wake_gate_daily"]),
            int(math.ceil(3.0 / float(values["rule_of_three_epsilon"]))))


def _edges(settings, stored):
    """Return the learned (lo, hi), falling back to the calibration."""
    lo, hi = _limits(settings)[:2]
    return (float((stored or {}).get("lo") or lo), float((stored or {}).get("hi") or hi))


def _cosine(left, right):
    """Return the cosine of two vectors, 0.0 when either is empty."""
    if not left or not right:
        return 0.0
    norm = math.sqrt(sum(x * x for x in left)) * math.sqrt(sum(x * x for x in right))
    return sum(a * b for a, b in zip(left, right)) / norm if norm else 0.0


def _event_text(event):
    """Return the text of an event a condition is judged against."""
    payload = event.payload or {}
    return str(payload.get("description") or payload.get("text") or "")


async def _vectors(conn, row, event, embedder):
    """Return the event and condition vectors, embedding and storing what is missing."""
    if embedder is None or not _event_text(event):
        return None, None
    left = await read_event_vector(conn, event.id) if event.id is not None else None
    left = left or (await embedder.embed([_event_text(event)]))[0]
    right = row.get("condition_vector")
    if right is None:
        right = (await embedder.embed([row["condition_text"]]))[0]
        await conn.execute("UPDATE expectations SET condition_vector = %s WHERE id = %s",
                           (Vector(right), row["id"]))
    right = right.to_list() if hasattr(right, "to_list") else right
    return left, [float(x) for x in right]


async def reached(kernel, row, event):
    """Return whether a free-text condition fires at once; else start its one classifier call."""
    if kernel.pfc is None:
        return False
    left, right = await _vectors(kernel.conn, row, event, kernel.embedder)
    m = _cosine(left, right)
    lo, hi = _edges(kernel.settings, await read_state(kernel.conn, BANDS))
    if m < lo:
        return False
    if m >= hi:
        return True
    await kernel.jobs.start("classify", f"classify:{row['id']}:{event.id}", event,
                            lambda: _classify(kernel, row, event, m), place=event.place or "")
    return False


async def _classify(kernel, row, event, m):
    """Ask the model one general question: does this event fulfil this condition?"""
    user = f"condition: {row['condition_text']}\nevent: {_event_text(event)}"
    calls = await kernel.pfc.call([{"role": "system", "content": SYSTEM},
                                   {"role": "user", "content": user}], [JUDGE])
    verdict = str(calls[0].arguments.get("verdict") or "no") if calls else "no"
    return {"expectation_id": row["id"], "m": m, "verdict": verdict,
            "usage": dict(getattr(kernel.pfc, "last_usage", None) or {})}


async def finish(kernel, job):
    """Record one verdict, learn the bands from it, and let a yes fire its expectation."""
    result = job.outcome.get("result") or {}
    now, m = kernel.clock.now(), float(result.get("m") or 0.0)
    yes = result.get("verdict") == "yes"
    await budget.record_call(kernel.conn, job.trigger.id, kernel.pfc, purpose="classifier",
                             now=now, settings=kernel.settings, usage=result.get("usage"))
    verdict = await kernel._emit(job.trigger, "classifier.verdict",
                                 {"expectation_id": result.get("expectation_id"), "m": m,
                                  "verdict": "yes" if yes else "no"})
    await learn(kernel, verdict, m, yes)
    rows = await read_expectations(kernel.conn, "ex.id = %s", (result.get("expectation_id"),))
    if yes and rows and rows[0]["status"] == "active":
        await kernel._expect_wake(await fire_expectation(kernel.conn, rows[0], now))


async def learn(kernel, verdict, m, yes):
    """Move the two edges by the bounded-update rule; record each change with its delta."""
    lo, hi, floor, ceiling, margin, gray, step, daily, enough = _limits(kernel.settings)
    stored = await read_state(kernel.conn, BANDS) or {"lo": lo, "hi": hi, "yes": [], "no": []}
    bucket = "yes" if yes else "no"
    stored[bucket] = (list(stored.get(bucket) or []) + [round(m, 4)])[-200:]
    today = verdict.ts.date().isoformat()
    stored["moved"] = float(stored.get("moved") or 0.0) if stored.get("day") == today else 0.0
    stored["day"] = today
    before, target, changed = (stored["lo"], stored["hi"]), None, False
    if yes and len(stored["yes"]) >= enough:
        target = min(stored["yes"]) - margin
        bounded = max(floor, min(target, stored["hi"] - gray, ceiling))
        spare = max(0.0, daily - stored["moved"])
        move = max(-spare, min(spare, max(-step, min(step, bounded - stored["lo"]))))
        stored["lo"] = round(max(floor, stored["lo"] + move), 4)
        stored["moved"] = round(stored["moved"] + abs(move), 4)
        changed = stored["lo"] != before[0]
    top = sorted([(value, True) for value in stored["yes"]]
                 + [(value, False) for value in stored["no"]], reverse=True)[:enough]
    if len(top) >= enough and top[-1][0] >= stored["lo"] + gray:
        stored["hi"] = min(ceiling, round(top[-1][0], 4)) if all(ok for _, ok in top) else 1.0
        stored["lo"] = min(stored["lo"], stored["hi"] - gray)
        changed = changed or (stored["lo"], stored["hi"]) != before
    frozen = bool(target is not None and (target < floor or target > ceiling)
                  and stored.get("frozen") != today and not changed)
    if not changed and not frozen:
        return await write_state(kernel.conn, BANDS, stored, verdict.id)
    stored["frozen"] = today
    change = await kernel._emit(verdict, "disposition.updated",
                                {"key": BANDS, "lo": stored["lo"], "hi": stored["hi"],
                                 "frozen": frozen,
                                 "delta": [round(stored["lo"] - before[0], 4),
                                           round(stored["hi"] - before[1], 4)]})
    await write_state(kernel.conn, BANDS, stored, change.id)
