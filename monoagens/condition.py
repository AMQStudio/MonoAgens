"""Conditions: exact fast paths and the free-text wake gate (docs/core.md §5)."""

from __future__ import annotations

import re
from datetime import timedelta

from monoagens.embedding import cosine
from monoagens.store import append_event, payload_text, read_event_vector

_UNITS = {"s": 1, "m": 60, "h": 3600, "d": 86400}
_RE = re.compile(r"^(\d+(?:\.\d+)?)\s*([smhd])$")


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


async def reminded(kernel, row, event):
    """Return whether the line reminds the condition: cosine reaches `lo` from the prior."""
    query = payload_text(event.payload)
    if kernel is None or kernel.pfc is None or kernel.embedder is None or not query:
        return False
    left = await read_event_vector(kernel.conn, event.id) if event.id is not None else None
    if left is None:
        left = (await kernel.embedder.embed([query]))[0]
    condition_text = row["condition_text"] or text(row["condition"])
    right = (await kernel.embedder.embed([condition_text]))[0]
    lo = float((kernel.settings.get("calibration") or {})["wake_gate_lo"])
    return cosine(left, right) >= lo


async def reminder(conn, row, event, now):
    """Record the reminder wake: the line and the Expectation are its trigger."""
    payload = {"expectation_id": row["id"], "about": row["about"],
               "text": payload_text(event.payload), "reminder": True}
    fired = await append_event(conn, ts=now, kind="expectation.fired", who=row["who"],
                               place=row["place"], body=row["source_body"],
                               caused_by=row["source_event"], payload=payload)
    return {**row, "fired_event": fired.id, "trigger": fired}
