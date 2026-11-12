"""Conditions: exact fast paths and the free-text wake gate (docs/core.md §5)."""

from __future__ import annotations

from monoagens.duration import parse_duration
from monoagens.embedding import cosine
from monoagens.store import append_event, payload_text, read_event_vector


def operator(expect):
    """Return one `expect` condition as its exact fast path or its free text."""
    if "none" in expect:
        return {"none": True}
    if "event_seen" in expect:
        seen = expect["event_seen"] if isinstance(expect["event_seen"], dict) else {}
        stated = dict(seen.get("pattern") or seen)
        pattern = {key: value for key, value in stated.items() if key != "within"}
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
    query = payload_text(event.payload)
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
               "text": payload_text(event.payload), "line_place": event.place,
               "reminder": True}
    reminded = await append_event(conn, ts=now, kind="expectation.reminded", who=event.who,
                                  place=event.place, body=event.body, caused_by=event.id,
                                  payload=payload)
    return {**row, "trigger": reminded}
