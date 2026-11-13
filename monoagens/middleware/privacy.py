"""Privacy: who is present, what an audience may see, consent to share, and the withheld note.

The core knows only opaque ids; this module gives them meaning. It runs as the absolute
second line of every hook: content from a place whose audience this context does not cover
never enters the desk, and sharing needs the consent of someone who was part of it.
"""

from __future__ import annotations

import re
from collections.abc import Sequence
from dataclasses import replace
from typing import Any

from monoagens.core.expectations import matches_cue
from monoagens.core.hooks import Continue, Middleware, Replace, Stop
from monoagens.infra.config import leak_candidates
from monoagens.infra.store import payload_text, read_state, write_state
from monoagens.types import Event

OWNER = "person:owner"
SHARE = "self.share"
SHARE_CAPABILITY = {
    "name": "share",
    "description": "Share private content with a person or a place; only if you were part of it. "
                   "Content from another place needs this before it can be repeated where its "
                   "audience may not see it.",
    "schema": {"type": "object", "properties": {
        "item": {"type": "string"},
        "episode": {"type": "string",
                    "description": "The event id of the content's source, digits only."},
        "to": {"type": "string", "description": "person:<id> or a place id."}},
        "required": ["to"]},
    "communicative": False, "long_running": False, "identity_changing": False,
}


def register(registry: Any) -> None:
    """Declare the middleware's own `self.share` capability on the kernel's registry."""
    registry.add_capability(SHARE, SHARE_CAPABILITY, facts=(), permission="privacy.share",
                            result=False)  # the guard records the grant itself


async def aliases(conn) -> dict[str, str]:
    """Return the alias map that middleware keeps in `state: alias:*`."""
    cur = await conn.execute("SELECT key, value FROM state WHERE key LIKE 'alias:%'")
    return {key[6:]: str((value or {}).get("who") or value) for key, value in await cur.fetchall()}


async def place_kind(conn, place: str | None) -> str:
    """Return the kind the adapter reported for a place; unknown counts as shared."""
    if not place:
        return "shared"
    row = await (await conn.execute(
        "SELECT payload->>'place_kind' FROM events WHERE place = %s "
        "AND payload ? 'place_kind' ORDER BY id DESC LIMIT 1", (place,))).fetchone()
    return str(row[0] or "shared") if row else "shared"


async def direct(conn, place: str | None, cache: dict | None = None) -> bool:
    """Return whether a place is a 1:1 chat, by the kind its adapter reported."""
    return await _kind(conn, place, cache) == "direct"


async def _kind(conn, place: str | None, cache: dict | None = None) -> str:
    """Return one place's kind, remembered for one wake when a cache is given."""
    if cache is None:
        return await place_kind(conn, place)
    key = ("kind", place)
    if key not in cache:
        cache[key] = await place_kind(conn, place)
    return cache[key]


async def _delegated(conn, event_id: int | None,
                     cache: dict | None = None) -> dict[str, Any]:
    """Return delegated actors and origin places attached to one percept event (D80)."""
    if event_id is None:
        return {"actors": frozenset(), "source_places": ()}
    if cache is not None and ("delegated", event_id) in cache:
        return cache[("delegated", event_id)]
    stored = await read_state(conn, f"delegated:{int(event_id)}") or {}
    entry = {
        "actors": frozenset(str(who) for who in stored.get("actors") or () if who),
        "source_places": tuple(str(place) for place in stored.get("source_places") or () if place),
    }
    if cache is not None:
        cache[("delegated", event_id)] = entry
    return entry


async def _audience(conn, place: str | None, before: int | None = None,
                    cache: dict | None = None) -> frozenset[str]:
    """Return one place's audience, remembered for one wake when a cache is given."""
    if cache is None:
        return await audience(conn, place, before)
    key = ("people", place, before)
    if key not in cache:
        cache[key] = await audience(conn, place, before)
    return cache[key]


async def _shared(conn, place: str, cache: dict | None = None,
                  event_id: int | None = None) -> frozenset[str]:
    """Return what `share.granted` added to one place or episode, remembered for one wake."""
    if cache is None:
        return await shared(conn, place, event_id=event_id)
    key = ("shared", place, event_id)
    if key not in cache:
        cache[key] = await shared(conn, place, event_id=event_id)
    return cache[key]


async def audience(conn, place: str | None, before: int | None = None) -> frozenset[str]:
    """Return the person ids present at a place (at `before`, after the alias map)."""
    if not place:
        return frozenset()
    delegated_actors: frozenset[str] = frozenset()
    if before is not None:
        delegated_actors = (await _delegated(conn, int(before)))["actors"]
    task = await read_state(conn, f"task:{place}")
    if task:
        return frozenset(task.get("audience") or ()) | delegated_actors
    query = ("SELECT who FROM events WHERE place = %s AND who IS NOT NULL "
             "AND kind IN ('percept', 'act.out')")
    params = [place]
    if before is not None:
        query, params = query + " AND id <= %s", params + [before]
    rows = await (await conn.execute(query, params)).fetchall()
    mapping = await aliases(conn)
    people = {mapping.get(who, who) for (who,) in rows}
    if await direct(conn, place):
        others = people - {OWNER}
        base = frozenset(others) if len(others) == 1 else frozenset(people)
        return base | delegated_actors
    return frozenset(people) | delegated_actors


async def shared(conn, place: str, before: int | None = None, *,
                 event_id: int | None = None) -> frozenset[str]:
    """Return the people a `share.granted` added to a place or one episode up to `before`."""
    if event_id is not None:
        query = ("SELECT payload FROM events WHERE kind = 'share.granted' AND ("
                 "payload->>'episode' = %s OR ("
                 "payload->>'source_place' = %s AND "
                 "(NOT (payload ? 'episode') OR payload->>'episode' IS NULL)))")
        params: list[Any] = [str(int(event_id)), place]
    else:
        query = ("SELECT payload FROM events WHERE kind = 'share.granted' "
                 "AND payload->>'source_place' = %s AND "
                 "(NOT (payload ? 'episode') OR payload->>'episode' IS NULL)")
        params = [place]
    if before is not None:
        query, params = query + " AND id <= %s", params + [before]
    cur = await conn.execute(query, params)
    return frozenset(who for (payload,) in await cur.fetchall()
                     for who in (payload or {}).get("persons") or ())


async def owner_direct(conn, place: str | None, cache: dict | None = None) -> bool:
    """Return whether this context is a direct chat with the owner."""
    return (await direct(conn, place, cache)
            and await _audience(conn, place, None, cache) <= frozenset({OWNER}))


async def visible(conn, *, context: str | None, source: str | None,
                  before: int | None = None, cache: dict | None = None) -> bool:
    """Return whether content from `source` may enter a context at `context` (D45, D50)."""
    if not source or await owner_direct(conn, context, cache):
        return True
    need = set(await _audience(conn, context, None, cache)) - {OWNER}
    if not need or (source == context and before is None):
        return True
    present = set(await _audience(conn, source, before, cache))
    consented = set(await _shared(conn, source, cache, event_id=before))
    return need <= (present - {OWNER}) | consented


async def memory_visible(conn, *, context: str | None, sources: Any = None,
                         cache: dict | None = None) -> bool:
    """Return whether every source of a memory was said to the people here (D53, D50)."""
    if not sources:
        return await owner_direct(conn, context, cache)
    need = set(await _audience(conn, context, None, cache)) - {OWNER}
    rows = await (await conn.execute(
        "SELECT id, place FROM events WHERE id = ANY(%s) AND place IS NOT NULL",
        (list(sources),))).fetchall()
    audiences = []
    for source, place in rows:
        present = set(await _audience(conn, place, source, cache))
        consented = set(await _shared(conn, place, cache, event_id=source))
        audiences.append((present - {OWNER}) | consented)
    return all(need <= have for have in audiences)


async def source_label(conn, place: str | None, before: int | None = None,
                       cache: dict | None = None) -> str:
    """Return the D55-safe label of a source place: a private chat's people, or the place."""
    if not place:
        return f"{OWNER} private chat"
    if await direct(conn, place, cache):
        people = sorted(await _audience(conn, place, before, cache))
        if people:
            return f"{' and '.join(people)} private chat"
    return f"the chat at {place}"


async def withheld_source(conn, *, context: str | None, sources: Any = None,
                          cache: dict | None = None) -> str:
    """Return the label of the source one withheld item came from (D56).

    A memory or core memory names the first source place whose audience this
    context does not cover. A memory without a known source is private to the
    owner, so its label says so.
    """
    need = set(await _audience(conn, context, None, cache)) - {OWNER}
    rows = []
    if sources:
        rows = await (await conn.execute(
            "SELECT id, place FROM events WHERE id = ANY(%s) AND place IS NOT NULL",
            (list(sources),))).fetchall()
    for source, source_place in rows:
        present = set(await _audience(conn, source_place, source, cache)) - {OWNER}
        consented = set(await _shared(conn, source_place, cache, event_id=source))
        if not need <= (present | consented):
            return await source_label(conn, source_place, source, cache)
    return await source_label(conn, None, None, cache)


def _normalize_text(text: str) -> str:
    """Return whitespace-collapsed lower-case text for similarity checks."""
    return re.sub(r"\s+", " ", str(text or "")).strip().lower()


def _shingles(text: str, width: int = 4) -> set[tuple[str, ...]]:
    """Return consecutive content-word n-grams for catching partial private quotes."""
    tokens = re.findall(r"[0-9a-zA-ZÀ-ÿ_]{3,}", str(text or "").lower())
    if len(tokens) < width:
        return set()
    return {tuple(tokens[i: i + width]) for i in range(len(tokens) - width + 1)}


def _matches_hidden(text: str, hidden: Sequence[tuple[str, int]],
                    visible_texts: Sequence[str]) -> bool:
    """Return whether `text` matches or quotes only non-visible private content."""
    norm_text = _normalize_text(text)
    if not norm_text or not hidden:
        return False
    visible_blob = "\n".join(_normalize_text(item) for item in visible_texts if item)
    visible_shingles: set[tuple[str, ...]] = set()
    for item in visible_texts:
        visible_shingles |= _shingles(item)
    text_shingles = _shingles(norm_text)
    for raw_line, min_len in hidden:
        norm_line = _normalize_text(raw_line)
        if len(norm_line) < min_len:
            continue
        if norm_line in norm_text and norm_line not in visible_blob:
            return True
        if min_len >= 8 and ((_shingles(norm_line) - visible_shingles) & text_shingles):
            return True
    return False


def _privacy_tsqueries(text: str) -> tuple[str, str]:
    """Build `simple` tsquery strings for candidate events (>=10 chars) and memories (>=4)."""
    tokens = re.findall(r"[0-9a-zA-ZÀ-ÿ_]+", str(text or "").lower())
    content = [token for token in tokens if len(token) >= 3]
    ev_phrases = [" <5> ".join(content[i: i + 4]) for i in range(max(0, len(content) - 3))]
    for i in range(len(tokens)):
        length = 0
        for j in range(i, min(len(tokens), i + 5)):
            length += len(tokens[j]) + (1 if j > i else 0)
            if length >= 7:
                ev_phrases.append(" <-> ".join(tokens[i: j + 1]))
                break
    mem_phrases: list[str] = []
    for i, token in enumerate(tokens):
        if len(token) >= 4:
            mem_phrases.append(token)
        elif i + 1 < len(tokens) and len(token) + 1 + len(tokens[i + 1]) >= 4:
            mem_phrases.append(f"{token} <-> {tokens[i + 1]}")
    return (" | ".join(dict.fromkeys(ev_phrases)), " | ".join(dict.fromkeys(mem_phrases)))


_CANDIDATES_SQL = """
    WITH ev_trgm AS (
        SELECT id, place, event_line_text(kind, payload) AS line,
               similarity(event_line_text(kind, payload), %s) AS rank_score,
               (place = %s) AS is_target
        FROM events
        WHERE kind IN ('percept', 'act.out', 'act.result')
          AND place IS NOT NULL
          AND event_line_text(kind, payload) %%> %s
        ORDER BY (place = %s) DESC, rank_score DESC, id DESC
        LIMIT %s
    ),
    ev_tsv AS (
        SELECT id, place, event_line_text(kind, payload) AS line,
               ts_rank(to_tsvector('simple', privacy_clean_text(event_line_text(kind, payload))),
                       to_tsquery('simple', %s), 16) AS rank_score,
               (place = %s) AS is_target
        FROM events
        WHERE kind IN ('percept', 'act.out', 'act.result')
          AND place IS NOT NULL
          AND %s <> ''
          AND to_tsvector('simple', privacy_clean_text(event_line_text(kind, payload)))
              @@ to_tsquery('simple', %s)
        ORDER BY (place = %s) DESC, rank_score DESC, id DESC
        LIMIT %s
    ),
    ev_union AS (
        SELECT DISTINCT ON (id) id, place, line, rank_score, is_target
        FROM (SELECT * FROM ev_trgm UNION ALL SELECT * FROM ev_tsv) s
        ORDER BY id, is_target DESC, rank_score DESC
    ),
    mem_trgm AS (
        SELECT id, place, text AS line, derived_from,
               similarity(text, %s) AS rank_score,
               (place = %s) AS is_target
        FROM memories
        WHERE valid_to IS NULL AND text %%> %s
        ORDER BY (place = %s) DESC, rank_score DESC, id DESC
        LIMIT %s
    ),
    mem_tsv AS (
        SELECT id, place, text AS line, derived_from,
               ts_rank(to_tsvector('simple', privacy_clean_text(text)),
                       to_tsquery('simple', %s), 16) AS rank_score,
               (place = %s) AS is_target
        FROM memories
        WHERE valid_to IS NULL
          AND %s <> ''
          AND to_tsvector('simple', privacy_clean_text(text))
              @@ to_tsquery('simple', %s)
        ORDER BY (place = %s) DESC, rank_score DESC, id DESC
        LIMIT %s
    ),
    mem_union AS (
        SELECT DISTINCT ON (id) id, place, line, derived_from, rank_score, is_target
        FROM (SELECT * FROM mem_trgm UNION ALL SELECT * FROM mem_tsv) s
        ORDER BY id, is_target DESC, rank_score DESC
    ),
    combined AS (
        SELECT 'event'::text AS src_kind, id, place, line, NULL::bigint[] AS derived_from,
               rank_score, is_target
        FROM ev_union
        UNION ALL
        SELECT 'memory'::text AS src_kind, id, place, line, derived_from,
               rank_score, is_target
        FROM mem_union
    )
    SELECT src_kind, id, place, line, derived_from
    FROM combined
    ORDER BY is_target DESC, rank_score DESC, id DESC
    LIMIT %s
"""

_BATCH_VISIBILITY_SQL = """
    WITH mem_sources AS (
        SELECT e.id AS event_id, e.place
        FROM events e
        WHERE e.id = ANY(%s::bigint[])
          AND e.place IS NOT NULL
    ),
    targets AS (
        SELECT cid AS event_id, cplace AS place
        FROM unnest(%s::bigint[], %s::text[]) AS t(cid, cplace)
        UNION
        SELECT event_id, place FROM mem_sources
    ),
    alias_map AS (
        SELECT substring(key FROM 7) AS native,
               CASE
                   WHEN jsonb_typeof(value) = 'object' AND NULLIF(value->>'who', '') IS NOT NULL
                       THEN value->>'who'
                   ELSE value #>> '{}'
               END AS person
        FROM state
        WHERE key LIKE 'alias:%%'
    ),
    share_events AS (
        SELECT DISTINCT t.event_id, p.who
        FROM targets t
        JOIN events sg
          ON sg.kind = 'share.granted'
         AND (sg.payload->>'episode' = t.event_id::text
              OR (sg.payload->>'source_place' = t.place
                  AND (NOT (sg.payload ? 'episode') OR sg.payload->>'episode' IS NULL)))
        CROSS JOIN LATERAL jsonb_array_elements_text(
            coalesce(sg.payload->'persons', '[]'::jsonb)) AS p(who)
    ),
    delegated_actors AS (
        SELECT t.event_id, p.who
        FROM targets t
        JOIN state st ON st.key = 'delegated:' || t.event_id::text
        CROSS JOIN LATERAL jsonb_array_elements_text(
            coalesce(st.value->'actors', '[]'::jsonb)) AS p(who)
    ),
    task_audiences AS (
        SELECT t.event_id, p.who
        FROM targets t
        JOIN state st ON st.key = 'task:' || t.place
        CROSS JOIN LATERAL jsonb_array_elements_text(
            coalesce(st.value->'audience', '[]'::jsonb)) AS p(who)
        WHERE st.value <> '{}'::jsonb AND st.value <> 'null'::jsonb
    ),
    place_first_who AS (
        SELECT p.place, ev.who, min(ev.id) AS first_id
        FROM (
            SELECT DISTINCT t.place
            FROM targets t
            LEFT JOIN state st ON st.key = 'task:' || t.place
            WHERE st.key IS NULL OR st.value = '{}'::jsonb OR st.value = 'null'::jsonb
        ) p
        JOIN events ev
          ON ev.place = p.place
         AND ev.who IS NOT NULL
         AND ev.kind IN ('percept', 'act.out')
        GROUP BY p.place, ev.who
    ),
    chat_audiences AS (
        SELECT DISTINCT t.event_id, coalesce(am.person, pfw.who) AS who
        FROM targets t
        JOIN place_first_who pfw
          ON pfw.place = t.place AND pfw.first_id <= t.event_id
        LEFT JOIN alias_map am ON am.native = pfw.who
    ),
    all_visible_people AS (
        SELECT event_id, who FROM share_events
        UNION
        SELECT event_id, who FROM delegated_actors
        UNION
        SELECT event_id, who FROM task_audiences
        UNION
        SELECT event_id, who FROM chat_audiences
    )
    SELECT t.event_id
    FROM targets t
    WHERE NOT EXISTS (
        SELECT 1
        FROM unnest(%s::text[]) AS req(who)
        WHERE req.who <> %s
          AND NOT EXISTS (
              SELECT 1 FROM all_visible_people avp
              WHERE avp.event_id = t.event_id AND avp.who = req.who
          )
    )
"""


async def _leak_candidates(conn, target: str, text: str,
                           *, limit: int) -> list[tuple[str, int, str, tuple[int, ...]]]:
    """Return top `limit` candidate event and memory texts similar to `text`."""
    ev_tsq, mem_tsq = _privacy_tsqueries(text)
    params = (
        text, target, text, target, limit,
        ev_tsq or "a", target, ev_tsq, ev_tsq or "a", target, limit,
        text, target, text, target, limit,
        mem_tsq or "a", target, mem_tsq, mem_tsq or "a", target, limit,
        limit,
    )
    rows = await (await conn.execute(_CANDIDATES_SQL, params)).fetchall()
    return [(str(kind), int(cid), str(place or ""), str(line), tuple(int(s) for s in (src or ())))
            for kind, cid, place, line, src in rows if line]


async def _batch_visible_event_ids(conn, need: set[str], ev_ids: list[int],
                                   ev_places: list[str], mem_src_ids: list[int]) -> set[int]:
    """Return the subset of candidate/source event ids visible to `need` in one SQL query."""
    if not ev_ids and not mem_src_ids:
        return set()
    if not need:
        return set(ev_ids) | set(mem_src_ids)
    rows = await (await conn.execute(
        _BATCH_VISIBILITY_SQL,
        (mem_src_ids, ev_ids, ev_places, sorted(need), OWNER),
    )).fetchall()
    return {int(row[0]) for row in rows}


async def leaks(conn, target: str | None, text: str, *, limit: int | None = None) -> bool:
    """Return whether a message would carry private content the target may not see (D45)."""
    if not text or not target:
        return False
    cache: dict = {}
    if await owner_direct(conn, target, cache):
        return False
    top_k = int(limit if limit is not None else leak_candidates())
    candidates = await _leak_candidates(conn, target, text, limit=top_k)
    if not candidates:
        return False
    need = set(await _audience(conn, target, None, cache)) - {OWNER}
    ev_ids = [cid for kind, cid, _, _, _ in candidates if kind == "event"]
    ev_places = [place for kind, _, place, _, _ in candidates if kind == "event"]
    mem_src_ids = list(dict.fromkeys(
        src_id for kind, _, _, _, sources in candidates if kind == "memory" for src_id in sources
    ))
    visible_ids = await _batch_visible_event_ids(conn, need, ev_ids, ev_places, mem_src_ids)
    hidden: list[tuple[str, int]] = []
    seen_visible: list[str] = []
    for kind, cid, _, line, sources in candidates:
        if kind == "event":
            if not need or cid in visible_ids:
                seen_visible.append(line)
            else:
                hidden.append((line, 10))
        elif sources and all(src_id in visible_ids for src_id in sources):
            seen_visible.append(line)
        else:
            hidden.append((line, 4))
    return _matches_hidden(text, hidden, seen_visible)


def episode_error(arguments: dict[str, Any]) -> str:
    """Return why `episode` is wrong, or "" when it is absent or all digits (coerced in place)."""
    raw = arguments.get("episode")
    if raw is None:
        return ""
    text = str(raw).strip()
    if not text.isdigit():
        return f"episode: expected an event id made of digits, got {raw!r}"
    arguments["episode"] = int(text)
    return ""


async def _chain_delegated_sends(conn, event_id: int) -> tuple[str | None, list[tuple[int, str, str | None]]]:
    """Return `(root_place, [(act_id, actor, place), ...])` for communicative sends on this chain."""
    query = """
    WITH RECURSIVE anc AS (
        SELECT id, caused_by, place FROM events WHERE id = %s
        UNION ALL
        SELECT e.id, e.caused_by, e.place FROM events e JOIN anc a ON e.id = a.caused_by
    ),
    root AS (SELECT id, place FROM anc WHERE caused_by IS NULL LIMIT 1),
    descendants AS (
        SELECT id, kind, place, origin_actor, payload FROM events WHERE id = (SELECT id FROM root)
        UNION ALL
        SELECT e.id, e.kind, e.place, e.origin_actor, e.payload
        FROM events e JOIN descendants d ON e.caused_by = d.id
    )
    SELECT (SELECT place FROM root), id, origin_actor, place
    FROM descendants
    WHERE kind = 'act.out' AND (payload->>'communicative') = 'true'
      AND COALESCE(payload->>'refused', 'false') <> 'true'
    ORDER BY id
    """
    rows = await (await conn.execute(query, (event_id,))).fetchall()
    root_place = str(rows[0][0]) if rows and rows[0][0] else None
    sends = [(int(act_id), str(actor or ""), str(place) if place else None)
             for _, act_id, actor, place in rows if actor]
    return root_place, sends


def _reply_target(payload: dict[str, Any]) -> str | None:
    """Return the referenced native or Log message id when a percept replies to one."""
    raw = payload.get("reply_to") or payload.get("reply_to_message_id")
    if raw is None and isinstance(payload.get("reply_to_message"), dict):
        raw = payload["reply_to_message"].get("message_id")
    return str(raw).strip() if raw is not None and str(raw).strip() else None


async def _reply_act_out(conn, item: Event) -> tuple[int, str, str | None, str | None] | None:
    """Return `(act_id, origin_actor, act_place, root_place)` for a percept's `reply_to`."""
    target = _reply_target(item.payload or {})
    if target is None or not item.place:
        return None
    row = None
    if target.isdigit():
        row = await (await conn.execute(
            "SELECT id, origin_actor, place, caused_by FROM events "
            "WHERE id = %s AND kind = 'act.out' AND place = %s "
            "AND (payload->>'communicative') = 'true' "
            "AND COALESCE(payload->>'refused', 'false') <> 'true'",
            (int(target), item.place))).fetchone()
    if row is None:
        row = await (await conn.execute(
            "SELECT act.id, act.origin_actor, act.place, act.caused_by "
            "FROM events msg JOIN events act ON msg.caused_by = act.id "
            "WHERE act.kind = 'act.out' AND act.place = %s "
            "AND (act.payload->>'communicative') = 'true' "
            "AND COALESCE(act.payload->>'refused', 'false') <> 'true' "
            "AND msg.kind IN ('act.confirmed', 'act.result') "
            "AND (msg.payload->>'message_id' = %s OR msg.payload->'result'->>'message_id' = %s) "
            "ORDER BY msg.id DESC LIMIT 1",
            (item.place, target, target))).fetchone()
    if row is None:
        row = await (await conn.execute(
            "SELECT id, origin_actor, place, caused_by FROM events "
            "WHERE kind = 'act.out' AND place = %s "
            "AND (payload->>'communicative') = 'true' "
            "AND COALESCE(payload->>'refused', 'false') <> 'true' "
            "AND payload->>'message_id' = %s "
            "ORDER BY id DESC LIMIT 1",
            (item.place, target))).fetchone()
    if row is None or not str(row[1] or "").startswith("person:"):
        return None
    root_place, _ = await _chain_delegated_sends(conn, int(row[0]))
    return int(row[0]), str(row[1]), (str(row[2]) if row[2] else None), root_place


def delegation(kernel: Any) -> Middleware:
    """Inbound rule (D80): add delegator X to the audience of the one reply answering X's send."""

    async def rule(item: Any, ctx: Any) -> Any:
        """Record `delegated:<id>` once for a matched `wait` expectation or `reply_to`."""
        if not isinstance(item, Event) or item.kind != "percept" or item.id is None:
            return Continue()
        conn = ctx["conn"]
        now = ctx["now"]
        mapping = await aliases(conn)
        canonical_who = mapping.get(item.who, item.who) if item.who else item.who
        candidates = (item, replace(item, who=canonical_who)) if canonical_who != item.who else (item,)
        actors: set[str] = set()
        source_places: set[str] = set()

        rows = await (await conn.execute(
            "SELECT id, source_event, origin_actor, place, cue_event FROM expectations "
            "WHERE kind = 'wait' AND status = 'active' AND cue_event IS NOT NULL "
            "AND (expires_at IS NULL OR expires_at > %s) ORDER BY id",
            (now,))).fetchall()
        for exp_id, source_event, exp_origin, exp_place, cue_event in rows:
            if not any(matches_cue(cue_event, candidate) for candidate in candidates):
                continue
            if await read_state(conn, f"delegated_exp:{int(exp_id)}") is not None:
                continue
            if source_event is None:
                continue
            root_place, sends = await _chain_delegated_sends(conn, int(source_event))
            matching_sends = [send for send in sends if send[1].startswith("person:")]
            if not matching_sends:
                continue
            for act_id, actor, act_place in matching_sends:
                actors.add(actor)
                if item.place is None or act_place == item.place:
                    await write_state(conn, f"delegated_act:{act_id}",
                                      {"reply_event": item.id}, item.id)
            if exp_origin and str(exp_origin).startswith("person:"):
                actors.add(str(exp_origin))
            for place_ref in (exp_place, root_place):
                if place_ref:
                    source_places.add(str(place_ref))
            await write_state(conn, f"delegated_exp:{int(exp_id)}",
                              {"reply_event": item.id}, item.id)

        matched_reply = await _reply_act_out(conn, item)
        if matched_reply is not None:
            act_id, actor, _act_place, root_place = matched_reply
            if await read_state(conn, f"delegated_act:{act_id}") is None:
                actors.add(actor)
                if root_place:
                    source_places.add(root_place)
                await write_state(conn, f"delegated_act:{act_id}",
                                  {"reply_event": item.id}, item.id)

        if actors:
            await write_state(conn, f"delegated:{item.id}", {
                "actors": sorted(actors),
                "source_places": sorted(source_places),
            }, item.id)
        return Continue()

    return rule


async def _delegated_share_source(conn, event: Event, arguments: dict[str, Any],
                                  target: Any = None) -> tuple[str | None, int | None]:
    """Resolve an unshared D80 delegated reply event for `event.origin_actor` when no episode is given."""
    if not event.origin_actor:
        return None, None
    to = str(target if target is not None else (arguments.get("to") or ""))
    target_people: set[str] = set()
    if to.startswith("person:"):
        target_people = {to}
    elif to:
        target_people = set(await audience(conn, to))
    cur = await conn.execute(
        "SELECT key, value FROM state WHERE key LIKE 'delegated:%' ORDER BY updated_by_event DESC")
    rows = await cur.fetchall()
    parsed: list[tuple[int, dict[str, Any]]] = []
    for key, value in rows:
        suffix = str(key)[10:]
        if suffix.isdigit() and isinstance(value, dict):
            parsed.append((int(suffix), value))
    parsed.sort(key=lambda pair: pair[0], reverse=True)
    for ev_id, data in parsed:
        if event.origin_actor not in (data.get("actors") or ()):
            continue
        src_places = set(str(p) for p in data.get("source_places") or () if p)
        if str(event.place or "") not in src_places and to not in src_places:
            continue
        row = await (await conn.execute("SELECT place FROM events WHERE id = %s", (ev_id,))).fetchone()
        ev_place = (row or (None,))[0]
        if not ev_place:
            continue
        if target_people:
            have = ((set(await audience(conn, ev_place, before=ev_id)) - {OWNER})
                    | set(await shared(conn, ev_place, event_id=ev_id)))
            if (target_people - {OWNER}) <= have:
                continue
        return str(ev_place), ev_id
    return None, None


async def consent(conn, event: Event, arguments: dict[str, Any],
                  *, to: Any = None) -> tuple[bool, str | None, int | None]:
    """Return whether the chain's actor may share this content, its place, and optional episode."""
    source = event.place
    if arguments.get("episode"):
        ep_id = int(arguments["episode"])
        row = await (await conn.execute(
            "SELECT place FROM events WHERE id = %s", (ep_id,))).fetchone()
        source = (row or (None,))[0] or source
        return event.origin_actor in await audience(conn, source, before=ep_id), source, ep_id
    delegated_place, delegated_ep = await _delegated_share_source(conn, event, arguments, to)
    if delegated_ep is not None and delegated_place is not None:
        return (event.origin_actor in await audience(conn, delegated_place, before=delegated_ep),
                delegated_place, delegated_ep)
    return event.origin_actor in await audience(conn, source), source, None


async def grant(kernel, event: Event, arguments: dict[str, Any], to: str | None,
                source: str | None, episode: int | None = None) -> None:
    """Record consent for the people a `self.share` names (D46)."""
    people = ({str(to)} if str(to).startswith("person:")
              else await audience(kernel.conn, str(to)))
    ep_value = arguments.get("episode") if arguments.get("episode") is not None else episode
    await kernel._emit(event, "share.granted", {
        "item": arguments.get("item"), "episode": ep_value, "to": to,
        "persons": sorted(people), "source_place": source})


def filter_context(conn) -> Middleware:
    """Absolute line 2 (D45): private content enters only an allowed audience."""

    async def rule(item: Any, ctx: Any) -> Any:
        """Keep only what the audience may see, and note the private items withheld."""
        if ctx.get("filtered") is item:
            return Continue()  # the desk this rule produced, still unfiltered by anyone else
        place = item.place
        counts = dict(item.withheld)
        cache: dict = {}  # one audience lookup per place, not one per line

        async def keep(lines: Any, *, note_withheld: bool) -> tuple[Any, ...]:
            kept = []
            for line in lines:
                if await visible(conn, context=place, source=line.place, before=line.id,
                                 cache=cache):
                    kept.append(line)
                elif note_withheld:
                    label = await source_label(conn, line.place, line.id, cache)
                    counts[label] = counts.get(label, 0) + 1
            return tuple(kept)

        memories = []
        for memory in item.memories:
            if await memory_visible(conn, context=place,
                                    sources=getattr(memory, "sources", ()), cache=cache):
                memories.append(memory)
            else:
                label = await withheld_source(
                    conn, context=place, sources=getattr(memory, "sources", ()), cache=cache)
                counts[label] = counts.get(label, 0) + 1
        filtered = replace(
            item, place_kind=await _kind(conn, place, cache), memories=tuple(memories),
            recent_elsewhere=await keep(item.recent_elsewhere, note_withheld=True),
            recent=await keep(item.recent, note_withheld=False))
        core = await read_state(conn, "core_memory")
        sources = (core or {}).get("derived_from")
        if filtered.core_memory and not await memory_visible(conn, context=place,
                                                             sources=sources, cache=cache):
            label = await withheld_source(conn, context=place, sources=sources, cache=cache)
            counts[label] = counts.get(label, 0) + 1
            filtered = replace(filtered, core_memory="")
        filtered = replace(filtered, withheld=tuple(sorted(counts.items())))
        ctx["filtered"] = filtered
        return Replace(filtered)

    return rule


def guard(kernel: Any, when: str) -> Middleware:
    """Absolute line 3 (D45, D46): consent to share, and no private words in a place.

    On the way in it asks consent for `self.share`; on the way out, once the route is
    resolved, it records the grant and refuses a message whose words the target place
    may not see.
    """
    refused = "that place may not see this content; say it where it belongs"

    async def rule(item: Any, ctx: Any) -> Any:
        """Apply the sharing and leaking lines to one act."""
        if ctx.get("capability") == SHARE:
            if when == "first":
                wrong = episode_error(item)
                if wrong:
                    ctx["refusal"] = wrong
                    return Stop(wrong)
                allowed, source, grant_ep = await consent(
                    ctx["conn"], ctx["event"], item, to=ctx.get("to"))
                if not allowed:
                    ctx["refusal"] = "only someone who was part of it can share it"
                    return Stop("only someone who was part of it can share it")
                ctx["grant_source"] = source
                ctx["grant_episode"] = grant_ep
                return Continue()
            if ctx.get("refusal") or ctx.get("handled"):
                return Continue()
            await grant(kernel, ctx["event"], item, ctx.get("to"), ctx.get("grant_source"),
                        episode=ctx.get("grant_episode"))
            ctx["handled"] = True
            return Continue()
        if when == "first":
            leaving_refusal = await leaving(kernel, ctx, item)
            if leaving_refusal:
                ctx["refusal"] = leaving_refusal
                return Stop(leaving_refusal)
            if await forward_needs_confirmation(ctx["conn"], ctx, item):
                ctx["call_facts"] = set(ctx.get("call_facts") or ()) | {"sensitive"}
                ctx["approval_reason"] = FORWARDING
            return Continue()
        place = ctx.get("place") or ctx["event"].place
        if ctx.get("long_running"):
            ctx["labels"]["audience"] = sorted(await audience(ctx["conn"], ctx["event"].place))
        hidden = await leaks(ctx["conn"], place, str(item.get("text") or ""),
                             limit=leak_candidates(getattr(kernel, "settings", None)))
        ctx["labels"]["words_hidden"] = hidden
        if hidden and place == ctx["event"].place:
            ctx["refusal"] = refused
            return Stop(refused)
        return Continue()

    return rule


# ------------------------------------------------------- narrower rules


TOPIC_LABEL = "a topic you keep out of this mode"
LEAVING = "that would send private content off the machine"
FORWARDING = "forwarding a file"


def topic_rule(kernel: Any, topics: Sequence[str]) -> Middleware:
    """Return a context rule that drops lines and memories about these topics.

    A plugin may only narrow: the rule removes what it matches and adds one withheld
    count; it never puts anything back.
    """
    return _narrowing(topics)


def narrowing(kernel: Any, topics: Sequence[str]) -> Middleware:
    """The same rule under the name plugin code uses when it registers a middleware."""
    return _narrowing(topics)


def _narrowing(topics: Sequence[str]) -> Middleware:
    """Build the one topic rule; a topic must be a word to match, never a pattern."""
    words = [str(topic).strip().lower() for topic in topics if str(topic).strip()]

    async def rule(item: Any, ctx: Any) -> Any:
        """Drop what mentions a topic, and count what was dropped."""
        if not words:
            return Continue()
        recent = tuple(line for line in item.recent if not _mentions(line.payload, words))
        elsewhere = tuple(line for line in item.recent_elsewhere
                          if not _mentions(line.payload, words))
        memories = tuple(memory for memory in item.memories
                         if not _mentions_text(str(memory), words))
        dropped = ((len(item.recent) - len(recent))
                   + (len(item.recent_elsewhere) - len(elsewhere))
                   + (len(item.memories) - len(memories)))
        if not dropped:
            return Continue()
        counts = dict(item.withheld)
        counts[TOPIC_LABEL] = counts.get(TOPIC_LABEL, 0) + dropped
        return Replace(replace(item, recent=recent, recent_elsewhere=elsewhere,
                               memories=memories, withheld=tuple(sorted(counts.items()))))

    return rule


def _mentions(payload: Any, words: Sequence[str]) -> bool:
    """Return whether one event's text mentions a topic word."""
    return _mentions_text(payload_text(payload or {}), words)


def _mentions_text(text: str, words: Sequence[str]) -> bool:
    """Return whether a text holds a topic word as a whole word."""
    lowered = str(text).lower()
    return any(re.search(rf"(?<!\w){re.escape(word)}(?!\w)", lowered) for word in words)


def _strings(value: Any) -> list[str]:
    """Return every string inside one argument value, at any depth."""
    if isinstance(value, str):
        return [value]
    if isinstance(value, dict):
        return [found for item in value.values() for found in _strings(item)]
    if isinstance(value, (list, tuple)):
        return [found for item in value for found in _strings(item)]
    return []


def carries_file(arguments: Any, descriptor: Any) -> bool:
    """Return whether one act carries a file attachment with a communicative capability."""
    if not (descriptor or {}).get("communicative"):
        return False
    values = arguments or {}
    return any(values.get(key) for key in ("attachment", "attachments", "file"))


async def leaving(kernel: Any, ctx: Any, arguments: Any) -> str | None:
    """Return the refusal when a network call would carry content this place may not see."""
    if "network" not in kernel.tools.facts(str(ctx.get("capability") or "")):
        return None
    top_k = leak_candidates(getattr(kernel, "settings", None))
    for value in _strings(arguments):
        if await leaks(kernel.conn, ctx["event"].place, value, limit=top_k):
            return LEAVING
    return None


async def forward_needs_confirmation(conn: Any, ctx: Any, arguments: Any) -> bool:
    """Return whether forwarding this file reaches someone who was not part of it (D46)."""
    if not carries_file(arguments, ctx.get("descriptor")):
        return False
    to = str(ctx.get("to") or "")
    if not to:
        return False  # the same place: the audience already has it
    source = set(await audience(conn, ctx["event"].place))
    if to.startswith("person:"):
        return to not in source
    return not set(await audience(conn, to)) <= source
