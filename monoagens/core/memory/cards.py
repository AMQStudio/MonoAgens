"""The soul's own memory of a person: what it learned about a source, under that id."""

from __future__ import annotations

from collections.abc import Mapping
from datetime import datetime
from typing import Any

from monoagens.infra.store import read_state, write_state


def _resolve(mapping: Mapping[str, str], who: str) -> str:
    """Follow a `merged:` map transitively, stopping if a cycle ever appears."""
    seen: set[str] = set()
    cur = str(who)
    while cur in mapping and cur not in seen:
        seen.add(cur)
        cur = str(mapping[cur])
    return cur


async def merge_map(conn: Any) -> dict[str, str]:
    """Return every `{from: into}` recorded under `state: merged:*`, flattened."""
    cur = await conn.execute("SELECT key, value FROM state WHERE key LIKE 'merged:%'")
    raw = {str(key)[len("merged:"):]: str((value or {}).get("into") or "")
           for key, value in await cur.fetchall()
           if isinstance(value, Mapping) and value.get("into")}
    return {src: _resolve(raw, src) for src in raw}


async def canonical(conn: Any, who: str | None) -> str | None:
    """Return the surviving id for `who` by following `state: merged:*`."""
    if not who:
        return who
    seen: set[str] = set()
    cur = str(who)
    while cur not in seen:
        seen.add(cur)
        row = await read_state(conn, f"merged:{cur}")
        nxt = str((row or {}).get("into") or "") if isinstance(row, Mapping) else ""
        if not nxt:
            break
        cur = nxt
    return cur


async def equivalent_sources(conn: Any, who: str) -> tuple[str, ...]:
    """Return the surviving id and every older source id merged into it."""
    target = await canonical(conn, who) or str(who)
    mapping = await merge_map(conn)
    return tuple(sorted({target, *(src for src, dst in mapping.items() if dst == target)}))


def _merge_counts(dst: Any, src: Any) -> Any:
    """Add reliability counters or nested domains recursively without mutating inputs."""
    if isinstance(dst, Mapping) and isinstance(src, Mapping):
        merged = dict(dst)
        for key, value in src.items():
            merged[key] = _merge_counts(merged[key], value) if key in merged else value
        return merged
    if isinstance(dst, (int, float)) and isinstance(src, (int, float)):
        total = dst + src
        return int(total) if isinstance(dst, int) and isinstance(src, int) else float(total)
    return dst if dst is not None else src


def _merge_person_state(dst: Mapping[str, Any], src: Mapping[str, Any]) -> dict[str, Any]:
    """Combine two `person:<id>` cards into one: higher closeness, added reliability."""
    merged = {**src, **dst}
    if "closeness" in dst or "closeness" in src:
        merged["closeness"] = max(float(dst.get("closeness") or 0.0),
                                  float(src.get("closeness") or 0.0))
    if "reliability" in dst or "reliability" in src:
        merged["reliability"] = _merge_counts(dst.get("reliability"), src.get("reliability"))
    if dst.get("name") or src.get("name"):
        merged["name"] = dst.get("name") or src.get("name")
    return merged


async def apply_merge(conn: Any, src: str, dst: str, event_id: int) -> str:
    """Apply `source.merged {from, into}` to all core books without touching the Log."""
    target = await canonical(conn, dst) or str(dst)
    source = str(src)
    if not source or not target or source == target:
        return target
    await write_state(conn, f"merged:{source}", {"into": target}, event_id)
    cur = await conn.execute(
        "SELECT key FROM state WHERE key LIKE 'merged:%%' AND value->>'into' = %s", (source,))
    for (key,) in await cur.fetchall():
        if str(key) != f"merged:{target}":
            await write_state(conn, str(key), {"into": target}, event_id)

    src_card = await read_state(conn, f"person:{source}")
    dst_card = await read_state(conn, f"person:{target}")
    if src_card is not None:
        combined = _merge_person_state(dst_card or {}, src_card)
        await write_state(conn, f"person:{target}", combined, event_id)
        await conn.execute("DELETE FROM state WHERE key = %s", (f"person:{source}",))

    suffix = f":{source}"
    for key, val in await (await conn.execute(
            "SELECT key, value FROM state WHERE key LIKE %s", (f"habit:%:{source}",))).fetchall():
        key_s = str(key)
        if not key_s.endswith(suffix):
            continue
        dst_key = key_s[: -len(suffix)] + f":{target}"
        existing = await read_state(conn, dst_key) or {}
        incoming = val or {}
        winner = existing if float(existing.get("h") or 0.0) >= float(incoming.get("h") or 0.0) \
            else incoming
        await write_state(conn, dst_key, winner, event_id)
        await conn.execute("DELETE FROM state WHERE key = %s", (key_s,))

    src_seen = await read_state(conn, f"last_seen:{source}")
    if src_seen is not None:
        dst_seen = await read_state(conn, f"last_seen:{target}")
        keep = src_seen
        if dst_seen and dst_seen.get("ts") and src_seen.get("ts"):
            if datetime.fromisoformat(str(dst_seen["ts"])) >= datetime.fromisoformat(
                    str(src_seen["ts"])):
                keep = dst_seen
        elif dst_seen and not src_seen.get("ts"):
            keep = dst_seen
        await write_state(conn, f"last_seen:{target}", keep, event_id)
        await conn.execute("DELETE FROM state WHERE key = %s", (f"last_seen:{source}",))

    for key, val in await (await conn.execute(
            "SELECT key, value FROM state WHERE key LIKE 'task:%'")).fetchall():
        task = dict(val or {})
        if task.get("status") != "open":
            continue
        if task.get("waiting_for") == source:
            task["waiting_for"] = target
            await write_state(conn, str(key), task, event_id)

    await conn.execute("UPDATE memories SET who = %s WHERE who = %s", (target, source))
    await conn.execute("UPDATE expectations SET who = %s WHERE who = %s", (target, source))
    await conn.execute(
        "UPDATE expectations SET origin_actor = %s WHERE origin_actor = %s", (target, source))
    return target


async def closeness(conn, who: str | None) -> float:
    """Return the closeness the soul learned towards one person, clamped to [0, 1].

    The key `state: person:<id>` is frozen data (docs/core.md §3): the name belongs to the
    Log, not to any layer, and the value is the soul's own memory of a source, so it stays in
    the core while the middleware only adds the card that shows it.
    """
    resolved = await canonical(conn, who)
    if not resolved:
        return 0.0
    state = await read_state(conn, f"person:{resolved}") or {}
    return max(0.0, min(1.0, float(state.get("closeness") or 0.0)))
