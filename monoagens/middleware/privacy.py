"""Privacy: who is present, what an audience may see, consent to share, and the withheld note.

The core knows only opaque ids; this module gives them meaning. It runs as the absolute
second line of every hook: content from a place whose audience this context does not cover
never enters the desk, and sharing needs the consent of someone who was part of it.
"""

from __future__ import annotations

import json
import re
from collections.abc import Mapping, Sequence
from dataclasses import replace
from typing import Any

from monoagens.core.context import working
from monoagens.core.expectations import matches_cue
from monoagens.core.hooks import Continue, Middleware, Replace, Stop
from monoagens.core.memory.retrieval import recent_lines
from monoagens.infra.prior import constants, load_prior
from monoagens.infra.store import append_event, payload_text, read_state, write_state
from monoagens.types import Event

OWNER = "person:owner"
UNKNOWN = "*unknown*"
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
        "SELECT payload->>'place_kind' FROM events WHERE kind = 'percept' AND place = %s "
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


async def _place_members(conn, place: str, before: int | None,
                         mapping: dict[str, str]) -> tuple[set[str], bool]:
    """Return `(mapped_member_ids, known)` from `place_info.members` or current `place:` state."""
    raw_members: Any = None
    has_raw = False
    if before is not None:
        row = await (await conn.execute(
            "SELECT payload->'place_info'->'members' FROM events "
            "WHERE place = %s AND id <= %s AND payload->'place_info' ? 'members' "
            "ORDER BY id DESC LIMIT 1",
            (place, int(before)),
        )).fetchone()
        if row is not None:
            raw_members, has_raw = row[0], True
    if not has_raw:
        ev_row = await (await conn.execute(
            "SELECT id, payload->'place_info'->'members' FROM events "
            "WHERE place = %s AND payload->'place_info' ? 'members' "
            "ORDER BY id DESC LIMIT 1",
            (place,),
        )).fetchone()
        st_row = await (await conn.execute(
            "SELECT value, updated_by_event FROM state WHERE key = %s",
            (f"place:{place}",),
        )).fetchone()
        ev_id = int(ev_row[0]) if ev_row is not None else -1
        st_has = bool(st_row and isinstance(st_row[0], dict) and "members" in st_row[0])
        st_id = int(st_row[1]) if (st_has and st_row[1] is not None) else -1
        if st_has and (ev_row is None or st_id >= ev_id):
            raw_members = st_row[0].get("members")
        elif ev_row is not None:
            raw_members = ev_row[1]
    if isinstance(raw_members, (list, tuple)):
        ids = {mapping.get(item, item) for item in raw_members if isinstance(item, str) and item}
        if ids:
            return ids, True
    return set(), False


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
             "AND kind = 'percept'")
    params: list[Any] = [place]
    if before is not None:
        query, params = query + " AND id <= %s", params + [before]
    rows = await (await conn.execute(query, params)).fetchall()
    mapping = await aliases(conn)
    speakers = {mapping.get(who, who) for (who,) in rows}
    members, members_known = await _place_members(conn, place, before, mapping)
    people = speakers | members
    if await direct(conn, place):
        others = people - {OWNER}
        base = frozenset(others) if len(others) == 1 else frozenset(people)
        return base | delegated_actors
    shared_people = set(people) | set(delegated_actors)
    if not members_known:
        shared_people.add(UNKNOWN)
    return frozenset(shared_people)


async def _share_grants(conn, place: str, before: int | None = None, *,
                        event_id: int | None = None) -> tuple[frozenset[str], frozenset[str]]:
    """Return `(consented_persons, consented_target_places)` from scoped `share.granted` events."""
    del place
    if event_id is None:
        return frozenset(), frozenset()
    query = (
        "SELECT payload FROM events WHERE kind = 'share.granted' AND ("
        "payload->>'episode' = %s OR (payload->'episodes') @> %s::jsonb)"
    )
    params: list[Any] = [str(int(event_id)), json.dumps([int(event_id)])]
    if before is not None:
        query, params = query + " AND id <= %s", params + [int(before)]
    rows = await (await conn.execute(query, params)).fetchall()
    people = {who for (payload,) in rows for who in (payload or {}).get("persons") or () if who}
    places = {str(payload["to"]) for (payload,) in rows if (payload or {}).get("to")}
    return frozenset(people), frozenset(places)


async def shared(conn, place: str, before: int | None = None, *,
                 event_id: int | None = None) -> frozenset[str]:
    """Return the people a `share.granted` added to a place or one episode up to `before`."""
    people, _ = await _share_grants(conn, place, before, event_id=event_id)
    return people


async def _shared_info(conn, place: str, cache: dict | None = None,
                       event_id: int | None = None) -> tuple[frozenset[str], frozenset[str]]:
    """Cached `_share_grants` lookup per wake."""
    if cache is None:
        return await _share_grants(conn, place, event_id=event_id)
    key = ("shared_info", place, event_id)
    if key not in cache:
        cache[key] = await _share_grants(conn, place, event_id=event_id)
    return cache[key]


async def _task_parent(conn, place: str | None, cache: dict | None = None) -> str | None:
    """Return the parent place of a `space:task:*` thread, if any."""
    if not place or not str(place).startswith("space:task:"):
        return None
    if cache is not None and ("task_parent", place) in cache:
        return cache[("task_parent", place)]
    task = await read_state(conn, f"task:{place}")
    parent = str(task.get("parent") or "") if isinstance(task, dict) and task.get("parent") else None
    if cache is not None:
        cache[("task_parent", place)] = parent
    return parent


async def owner_direct(conn, place: str | None, cache: dict | None = None) -> bool:
    """Return whether this context is a direct chat with only the owner present."""
    parent = await _task_parent(conn, place, cache)
    if parent:
        return await owner_direct(conn, parent, cache)
    return (bool(place)
            and await direct(conn, place, cache)
            and await _audience(conn, place, None, cache) == frozenset({OWNER}))


async def _covered(conn, *, context: str, need: set[str], source: str,
                   before: int | None, cache: dict | None,
                   same_place: bool = False) -> bool:
    """Return whether `source` (at `before`) covers `need` for `context`, failing closed on UNKNOWN."""
    present = set(await _audience(conn, source, before, cache))
    consented, consented_places = await _shared_info(conn, source, cache, event_id=before)
    known_need = need - {UNKNOWN}
    known_have = (present - {OWNER, UNKNOWN}) | (consented - {UNKNOWN})
    if not known_need <= known_have:
        return False
    if UNKNOWN in need:
        return source == context or same_place or context in consented_places
    return True


async def visible(conn, *, context: str | None, source: str | None,
                  before: int | None = None, cache: dict | None = None) -> bool:
    """Return whether content from `source` may enter a context at `context` (D45, D50, D85)."""
    if not context or not source:
        return False
    if await owner_direct(conn, context, cache):
        return True
    ctx_aud = await _audience(conn, context, None, cache)
    if not ctx_aud:
        return False
    need = set(ctx_aud) - {OWNER}
    parent = await _task_parent(conn, context, cache)
    same_place = source == context or (parent is not None and source == parent)
    if not need and not same_place:
        return False
    if source == context and before is None:
        return True
    return await _covered(conn, context=context, need=need, source=source,
                          before=before, cache=cache, same_place=same_place)


async def memory_visible(conn, *, context: str | None, sources: Any = None,
                         cache: dict | None = None) -> bool:
    """Return whether every source of a memory was said to the people here (D53, D50, D85)."""
    if not context:
        return False
    if await owner_direct(conn, context, cache):
        return True
    if not sources:
        return False
    ctx_aud = await _audience(conn, context, None, cache)
    if not ctx_aud:
        return False
    need = set(ctx_aud) - {OWNER}
    parent = await _task_parent(conn, context, cache)
    source_ids = [int(s) for s in sources]
    rows = await (await conn.execute(
        "SELECT id, place FROM events WHERE id = ANY(%s) AND place IS NOT NULL",
        (source_ids,))).fetchall()
    if len(rows) != len(set(source_ids)):
        return False
    for source, place in rows:
        same_place = place == context or (parent is not None and place == parent)
        if not need and not same_place:
            return False
        if not await _covered(conn, context=context, need=need, source=place,
                              before=source, cache=cache, same_place=same_place):
            return False
    return True


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
    """Return the label of the source one withheld item came from (D56)."""
    need = set(await _audience(conn, context, None, cache)) - {OWNER}
    rows = []
    if sources:
        rows = await (await conn.execute(
            "SELECT id, place FROM events WHERE id = ANY(%s) AND place IS NOT NULL",
            (list(sources),))).fetchall()
    for source, source_place in rows:
        if not context or not await _covered(conn, context=context, need=need,
                                             source=source_place, before=source, cache=cache):
            return await source_label(conn, source_place, source, cache)
    return await source_label(conn, None, None, cache)


def _trim_wake_sources(store: dict[tuple[int, str], tuple[tuple[str | None, int | None], ...]],
                       max_entries: int = 256) -> None:
    """Keep the cached wake source map bounded."""
    while len(store) > max_entries:
        store.pop(next(iter(store)))


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
        payload = item.payload or {}
        if payload.get("imported") or not item.place:
            return Continue()
        conn = ctx["conn"]
        now = ctx["now"]
        mapping = await aliases(conn)
        canonical_who = mapping.get(item.who, item.who) if item.who else item.who
        candidates = (item, replace(item, who=canonical_who)) if canonical_who != item.who else (item,)
        explicit_reply = _reply_target(payload) is not None
        matched_reply = await _reply_act_out(conn, item)
        reply_act_id = matched_reply[0] if matched_reply is not None else None
        actors: set[str] = set()
        source_places: set[str] = set()

        rows = await (await conn.execute(
            "SELECT id, source_event, origin_actor, place, cue_event FROM expectations "
            "WHERE kind = 'wait' AND status = 'active' AND cue_event IS NOT NULL "
            "AND (expires_at IS NULL OR expires_at > %s) ORDER BY id",
            (now,))).fetchall()
        for exp_id, source_event, exp_origin, exp_place, cue_event in rows:
            if explicit_reply and reply_act_id is None:
                continue
            if not any(matches_cue(cue_event, candidate) for candidate in candidates):
                continue
            if await read_state(conn, f"delegated_exp:{int(exp_id)}") is not None:
                continue
            if source_event is None:
                continue
            root_place, sends = await _chain_delegated_sends(conn, int(source_event))
            matching_sends = [
                send for send in sends
                if send[1].startswith("person:") and send[2] == item.place
                and (not explicit_reply or send[0] == reply_act_id)
            ]
            unclaimed = [send for send in matching_sends
                         if await read_state(conn, f"delegated_act:{send[0]}") is None]
            if not unclaimed:
                continue
            for act_id, actor, _act_place in unclaimed:
                actors.add(actor)
                await write_state(conn, f"delegated_act:{act_id}",
                                  {"reply_event": item.id}, item.id)
            if exp_origin and str(exp_origin).startswith("person:"):
                actors.add(str(exp_origin))
            for place_ref in (exp_place, root_place):
                if place_ref:
                    source_places.add(str(place_ref))
            await write_state(conn, f"delegated_exp:{int(exp_id)}",
                              {"reply_event": item.id}, item.id)

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


_SHARE_STOPWORDS = frozenset({
    "yang", "dan", "dari", "the", "and", "for", "with", "itu", "ini",
    "ada", "sudah", "kita", "ke", "di", "yg",
})


def _share_tokens(text: str) -> set[str]:
    """Tokenize share item descriptions into significant lowercase terms."""
    raw = set(re.findall(r"[a-z0-9_.@/-]{2,}", str(text or "").lower()))
    filtered = raw - _SHARE_STOPWORDS
    return filtered or raw


def _event_share_text(payload: Any) -> str:
    """Collect searchable text from one event payload, attachments and tool results."""
    if not isinstance(payload, Mapping):
        return str(payload or "")
    parts = [payload_text(payload)]
    data = payload.get("data")
    if isinstance(data, Mapping):
        for att in data.get("attachments") or ():
            if isinstance(att, Mapping):
                parts.extend(str(att.get(k) or "") for k in ("name", "ref"))
    if "result" in payload:
        parts.append(json.dumps(payload["result"], default=str))
    if "arguments" in payload:
        parts.append(json.dumps(payload["arguments"], default=str))
    return " ".join(p for p in parts if p)


async def _resolve_share_events(
    conn, event: Event, item_text: str
) -> tuple[bool, str | None, tuple[int, ...]]:
    """Resolve `item` text to matching non-forgotten event ids the actor may share."""
    lowered = " ".join(item_text.lower().split())
    tokens = _share_tokens(item_text)
    if not lowered and not tokens:
        return False, event.place, ()
    max_id = int(event.id) if event.id is not None else None
    ev_query = (
        "SELECT id, place, payload FROM events "
        "WHERE kind IN ('percept', 'act.out', 'act.result', 'decision.note') "
        "AND payload <> '{\"forgotten\": true}'::jsonb"
    )
    ev_params: list[Any] = []
    if max_id is not None:
        ev_query += " AND id <= %s"
        ev_params.append(max_id)
    ev_query += " ORDER BY id DESC LIMIT 200"
    ev_rows = await (await conn.execute(ev_query, ev_params)).fetchall()
    ev_place_by_id = {int(eid): (str(eplace) if eplace else None) for eid, eplace, _ in ev_rows}

    mem_rows = await (await conn.execute(
        "SELECT id, text, place, derived_from FROM memories "
        "WHERE valid_to IS NULL ORDER BY id DESC LIMIT 100"
    )).fetchall()
    missing_ids = {
        int(sid)
        for _, _, _, derived in mem_rows
        for sid in (derived or ())
        if int(sid) not in ev_place_by_id
    }
    if missing_ids:
        extra = await (await conn.execute(
            "SELECT id, place FROM events WHERE id = ANY(%s) "
            "AND payload <> '{\"forgotten\": true}'::jsonb",
            (sorted(missing_ids),),
        )).fetchall()
        for eid, eplace in extra:
            ev_place_by_id[int(eid)] = str(eplace) if eplace else None

    min_overlap = max(1, (len(tokens) + 1) // 2) if tokens else 1
    candidates: list[tuple[tuple[int, int, int], str | None, tuple[int, ...]]] = []
    for eid, eplace, payload in ev_rows:
        cand_text = _event_share_text(payload)
        cand_lower = " ".join(cand_text.lower().split())
        exact = 1 if (lowered and lowered in cand_lower) else 0
        overlap = len(tokens & _share_tokens(cand_text)) if tokens else 0
        if exact or overlap >= min_overlap:
            candidates.append(((exact, overlap, 0), eplace, (int(eid),)))
    for _, text, mplace, derived in mem_rows:
        src_ids = tuple(int(s) for s in (derived or ()) if int(s) in ev_place_by_id)
        if not src_ids:
            continue
        cand_lower = " ".join(str(text or "").lower().split())
        exact = 1 if (lowered and lowered in cand_lower) else 0
        overlap = len(tokens & _share_tokens(text)) if tokens else 0
        if exact or overlap >= min_overlap:
            src_place = mplace or ev_place_by_id.get(src_ids[0])
            candidates.append(((exact, overlap, 1), src_place, src_ids))

    if not candidates:
        return False, event.place, ()

    actor = event.origin_actor
    allowed_candidates: list[tuple[tuple[int, int, int], str | None, tuple[int, ...]]] = []
    for score, src_place, ep_ids in candidates:
        ok = bool(actor)
        for sid in ep_ids:
            p = ev_place_by_id.get(sid) or src_place
            if not actor or actor not in await audience(conn, p, before=sid):
                ok = False
                break
        if ok:
            allowed_candidates.append((score, src_place, ep_ids))

    if not allowed_candidates:
        return False, candidates[0][1] or event.place, candidates[0][2]

    best_score = max(c[0][:2] for c in allowed_candidates)
    chosen_ids: list[int] = []
    chosen_place: str | None = None
    for score, src_place, ep_ids in allowed_candidates:
        if score[:2] == best_score:
            if chosen_place is None and src_place:
                chosen_place = src_place
            for sid in ep_ids:
                if sid not in chosen_ids:
                    chosen_ids.append(sid)
    return True, chosen_place or event.place, tuple(sorted(chosen_ids))


async def consent(conn, event: Event, arguments: dict[str, Any],
                  *, to: Any = None) -> tuple[bool, str | None, int | tuple[int, ...] | None]:
    """Return whether the chain's actor may share this content, its place, and scoped episode(s)."""
    source = event.place
    if arguments.get("episode") is not None:
        ep_id = int(arguments["episode"])
        row = await (await conn.execute(
            "SELECT place FROM events WHERE id = %s", (ep_id,))).fetchone()
        source = (row or (None,))[0] or source
        return event.origin_actor in await audience(conn, source, before=ep_id), source, ep_id
    delegated_place, delegated_ep = await _delegated_share_source(conn, event, arguments, to)
    if delegated_ep is not None and delegated_place is not None:
        return (event.origin_actor in await audience(conn, delegated_place, before=delegated_ep),
                delegated_place, delegated_ep)
    item_text = str(arguments.get("item") or "").strip()
    if not item_text:
        return False, source, ()
    allowed, resolved_place, matched_ids = await _resolve_share_events(conn, event, item_text)
    return allowed, resolved_place, matched_ids


async def grant(kernel, event: Event, arguments: dict[str, Any], to: str | None,
                source: str | None, episode: int | Sequence[int] | None = None) -> None:
    """Record consent for the specific events and people a `self.share` names (D46)."""
    people = ({str(to)} if str(to).startswith("person:")
              else await audience(kernel.conn, str(to)))
    raw_ep = arguments.get("episode") if arguments.get("episode") is not None else episode
    if isinstance(raw_ep, (list, tuple, set)):
        episodes = sorted({int(ep) for ep in raw_ep})
    elif raw_ep is not None:
        episodes = [int(raw_ep)]
    else:
        episodes = []
    ep_scalar = episodes[0] if len(episodes) == 1 else (episodes[0] if episodes else None)
    await kernel._emit(event, "share.granted", {
        "item": arguments.get("item"), "episode": ep_scalar, "episodes": episodes,
        "to": to, "persons": sorted(p for p in people if p != UNKNOWN), "source_place": source})


_EXP_RE = re.compile(r"^#(\d+)\b")
_PERSON_CARD_RE = re.compile(r"^- (?:(?P<name>.+?) \((?P<wrapped>person:[^\s)]+)\)|(?P<bare>person:[^\s:]+)):")


async def _trigger_allowed(conn, place: str, trigger: Event | None, cache: dict) -> bool:
    """Return whether `trigger` (including a reminder line source) is visible at `place`."""
    if trigger is None:
        return True
    if trigger.kind == "expectation.reminded":
        src_place = trigger.payload.get("line_place") or trigger.place
        src_before = trigger.payload.get("line_event") or trigger.caused_by
    else:
        src_place = trigger.place
        src_before = trigger.id
    return await visible(conn, context=place, source=src_place, before=src_before, cache=cache)


async def _filter_expectations(conn, place: str, lines: Sequence[str], cache: dict,
                               ) -> tuple[tuple[str, ...], list[str]]:
    """Keep only expectation lines whose source/place is visible at `place`."""
    if not lines:
        return (), []
    if await owner_direct(conn, place, cache):
        return tuple(lines), []
    ids: list[int] = []
    for line in lines:
        m = _EXP_RE.match(str(line))
        if m:
            ids.append(int(m.group(1)))
    by_id: dict[int, tuple[str | None, int | None, str | None]] = {}
    if ids:
        rows = await (await conn.execute(
            "SELECT e.id, e.place, e.source_event, ev.place FROM expectations e "
            "LEFT JOIN events ev ON ev.id = e.source_event WHERE e.id = ANY(%s)",
            (ids,),
        )).fetchall()
        by_id = {int(eid): (exp_place, src_ev, ev_place)
                 for eid, exp_place, src_ev, ev_place in rows}
    kept: list[str] = []
    dropped: list[str] = []
    for line in lines:
        m = _EXP_RE.match(str(line))
        entry = by_id.get(int(m.group(1))) if m else None
        if entry is None:
            dropped.append("expectation without visible source")
            continue
        exp_place, src_ev, ev_place = entry
        src = ev_place if (src_ev is not None and ev_place) else exp_place
        before = int(src_ev) if (src_ev is not None and ev_place) else None
        if src and await visible(conn, context=place, source=src, before=before, cache=cache):
            kept.append(line)
        else:
            dropped.append("expectation source not visible at place")
    return tuple(kept), dropped


async def _filter_doing(conn, place: str, lines: Sequence[str], trigger: Event | None,
                        values: Mapping[str, Any], cache: dict,
                        ) -> tuple[tuple[str, ...], list[str]]:
    """Keep only `<doing>` lines whose source event (or task) is visible at `place`."""
    if not lines:
        return (), []
    if await owner_direct(conn, place, cache):
        return tuple(lines), []
    task = await read_state(conn, f"task:{place}") if place.startswith("space:task:") else None
    entries = await working.doing_entries(conn, place, task, values, trigger=trigger)
    sources_by_line: dict[str, list[tuple[str | None, int | None]]] = {}
    for line, src_place, src_id in entries:
        sources_by_line.setdefault(line, []).append((src_place, src_id))
    kept: list[str] = []
    dropped: list[str] = []
    for line in lines:
        queue = sources_by_line.get(line)
        if not queue:
            dropped.append("doing line without visible source")
            continue
        src_place, src_id = queue.pop(0)
        if src_place and await visible(conn, context=place, source=src_place,
                                       before=src_id, cache=cache):
            kept.append(line)
        else:
            dropped.append("doing line source not visible at place")
    return tuple(kept), dropped


async def _filter_people(conn, place: str, cards: Sequence[str], trigger: Event | None,
                         cache: dict) -> tuple[tuple[str, ...], list[str]]:
    """Keep only person cards belonging to the place audience or named in the trigger (D55)."""
    if not cards:
        return (), []
    if await owner_direct(conn, place, cache):
        return tuple(cards), []
    aud = set(await _audience(conn, place, None, cache))
    trig_text = payload_text(trigger.payload) if trigger is not None else ""
    kept: list[str] = []
    dropped: list[str] = []
    for card in cards:
        first_line = str(card).splitlines()[0] if str(card) else ""
        m = _PERSON_CARD_RE.match(first_line)
        if not m:
            dropped.append("person card without identifiable person id")
            continue
        who = m.group("wrapped") or m.group("bare")
        if who == UNKNOWN:
            dropped.append("person card for unknown member")
            continue
        if who in aud:
            kept.append(card)
            continue
        name = (m.group("name") or "").strip()
        if not name:
            st = await read_state(conn, f"person:{who}")
            name = str((st or {}).get("name") or "").strip()
        named_in_trigger = bool(trig_text) and (
            who in trig_text
            or (bool(name) and re.search(rf"(?<!\w){re.escape(name)}(?!\w)",
                                         trig_text, re.IGNORECASE) is not None)
        )
        if named_in_trigger:
            kept.append(card)
        else:
            dropped.append("person card not present or named in trigger")
    return tuple(kept), dropped


async def _filter_core_memory(conn, place: str, core_text: str, cache: dict,
                              ) -> tuple[bool, Any]:
    """Return `(visible_ok, sources)` for `core_memory`."""
    if not core_text:
        return True, ()
    core = await read_state(conn, "core_memory")
    sources = (core or {}).get("derived_from")
    return await memory_visible(conn, context=place, sources=sources, cache=cache), sources


def _context_values(settings: Any) -> Any:
    """Return prior constants, loading defaults when settings has none."""
    if isinstance(settings, Mapping) and settings.get("constants"):
        return constants(settings)
    return load_prior()["constants"]


def filter_context(conn, *, kernel: Any = None, settings: Any = None) -> Middleware:
    """Absolute line 2 (D45, D84): single context privacy check in the last context slot."""

    async def rule(item: Any, ctx: Any) -> Any:
        """Keep only what the audience may see, stop hidden reminders, and note what was withheld."""
        place = item.place
        counts = dict(item.withheld)
        cache: dict = {}  # one audience lookup per place, not one per line

        trigger_ok = await _trigger_allowed(conn, place, item.trigger, cache)
        if not trigger_ok and item.trigger is not None and item.trigger.kind == "expectation.reminded":
            return Stop("reminder line is not visible at this place")
        trigger_kept = item.trigger if trigger_ok else None

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

        recent = await keep(item.recent, note_withheld=False)
        elsewhere = await keep(item.recent_elsewhere, note_withheld=True)
        memories = []
        for memory in item.memories:
            if await memory_visible(conn, context=place,
                                    sources=getattr(memory, "sources", ()), cache=cache):
                memories.append(memory)
            else:
                label = await withheld_source(
                    conn, context=place, sources=getattr(memory, "sources", ()), cache=cache)
                counts[label] = counts.get(label, 0) + 1
        active_kernel = kernel or ctx.get("kernel")
        active_settings = settings or getattr(active_kernel, "settings", None)
        doing_kept, _ = await _filter_doing(
            conn, place, item.doing, trigger_kept, _context_values(active_settings), cache)
        exps_kept, _ = await _filter_expectations(conn, place, item.expectations, cache)
        people_kept, _ = await _filter_people(conn, place, item.people, trigger_kept, cache)
        core_text = item.core_memory
        core_ok, core_sources = await _filter_core_memory(conn, place, core_text, cache)
        if core_text and not core_ok:
            label = await withheld_source(conn, context=place, sources=core_sources, cache=cache)
            counts[label] = counts.get(label, 0) + 1
            core_text = ""
        kind = await _kind(conn, place, cache)
        filtered = replace(
            item, place_kind=kind, memories=tuple(memories),
            recent_elsewhere=elsewhere, recent=recent,
            doing=doing_kept, expectations=exps_kept, core_memory=core_text,
            trigger=trigger_kept, people=people_kept,
            withheld=tuple(sorted(counts.items())))
        if active_kernel is not None:
            await _record_wake_sources(active_kernel, conn, filtered, cache)
        if filtered == item:
            return Continue()
        return Replace(filtered)

    return rule


async def _source_refs_for_ids(conn, ids: Sequence[int]) -> list[tuple[str | None, int | None]]:
    """Resolve `derived_from` event ids into `(place, event_id)` tuples."""
    clean = [int(i) for i in ids if i is not None]
    if not clean:
        return [(None, None)]
    rows = await (await conn.execute(
        "SELECT id, place FROM events WHERE id = ANY(%s)", (clean,))).fetchall()
    by_id = {int(row[0]): row[1] for row in rows}
    return [(by_id.get(i), i) if i in by_id else (None, None) for i in clean]


async def _context_private_sources(conn, place: str, item: Any, cache: dict,
                                   ) -> tuple[tuple[str | None, int | None], ...]:
    """Return every private `(source_place, source_id)` held in this wake's verified desk (D84)."""
    is_owner_dm = await owner_direct(conn, place, cache)
    collected: list[tuple[str | None, int | None]] = []

    def add(src_place: str | None, src_id: int | None) -> None:
        if is_owner_dm and src_place == place:
            return
        pair = (src_place, int(src_id) if src_id is not None else None)
        if pair not in collected:
            collected.append(pair)

    for line in (*item.recent, *item.recent_elsewhere):
        add(line.place, line.id)
    if item.trigger is not None:
        if item.trigger.kind == "expectation.reminded":
            add(item.trigger.payload.get("line_place") or item.trigger.place,
                item.trigger.payload.get("line_event") or item.trigger.caused_by)
        else:
            add(item.trigger.place, item.trigger.id)
    for mem in item.memories:
        refs = await _source_refs_for_ids(conn, getattr(mem, "sources", ()) or ())
        for src_place, src_id in refs:
            add(src_place, src_id)
    if item.core_memory:
        core = await read_state(conn, "core_memory")
        for src_place, src_id in await _source_refs_for_ids(conn, (core or {}).get("derived_from") or ()):
            add(src_place, src_id)
    return tuple(collected)


async def _record_wake_sources(kernel: Any, conn: Any, item: Any, cache: dict) -> None:
    """Remember the private sources a verified wake context held, keyed by `(trigger.id, place)`."""
    if item.trigger is None or item.trigger.id is None:
        return
    sources = await _context_private_sources(conn, item.place, item, cache)
    store = getattr(kernel, "_wake_private_sources", None)
    if store is None:
        store = {}
        setattr(kernel, "_wake_private_sources", store)
    store[(int(item.trigger.id), str(item.place))] = sources
    _trim_wake_sources(store)


async def _words_hidden(kernel: Any, event: Event, target_place: str) -> bool:
    """Return whether anything private in the source wake's view is hidden from `target_place`."""
    conn = kernel.conn
    cache: dict = {}
    if await owner_direct(conn, target_place, cache):
        return False
    store = getattr(kernel, "_wake_private_sources", None) or {}
    key = (int(event.id), str(event.place or "")) if event.id is not None else None
    if key is not None and key in store:
        sources = store[key]
    else:
        origin = event.place
        is_owner_dm = await owner_direct(conn, origin, cache)
        sources_list: list[tuple[str | None, int | None]] = []
        if not is_owner_dm:
            sources_list.append((origin, event.id))
        if origin:
            limit = int(_context_values(getattr(kernel, "settings", None))["recent_elsewhere"])
            for line in await recent_lines(conn, origin, limit=limit):
                if await visible(conn, context=origin, source=line.place, before=line.id,
                                 cache=cache):
                    sources_list.append((line.place, line.id))
        core = await read_state(conn, "core_memory")
        if core and core.get("text"):
            core_refs = core.get("derived_from") or ()
            if await memory_visible(conn, context=origin, sources=core_refs, cache=cache):
                for src_place, src_id in await _source_refs_for_ids(conn, core_refs):
                    if not (is_owner_dm and src_place == origin):
                        sources_list.append((src_place, src_id))
        sources = tuple(dict.fromkeys(sources_list))
    for src_place, src_id in sources:
        if not src_place or not await visible(conn, context=target_place, source=src_place,
                                             before=src_id, cache=cache):
            return True
    return False


def guard(kernel: Any, when: str) -> Middleware:
    """Absolute line 3 (D45, D46, D84): consent to share, and record deterministic cross-place flag."""

    async def rule(item: Any, ctx: Any) -> Any:
        """Apply sharing consent and cross-place drafted-words check to one act."""
        if ctx.get("capability") == SHARE:
            if when == "first":
                wrong = episode_error(item)
                if wrong:
                    ctx["refusal"] = wrong
                    return Stop(wrong)
                has_episode = item.get("episode") is not None
                has_item = bool(str(item.get("item") or "").strip())
                if not has_episode and not has_item:
                    reason = "what item or episode do you want to share?"
                    ctx["refusal"] = reason
                    return Stop(reason)
                allowed, source, grant_ep = await consent(
                    ctx["conn"], ctx["event"], item, to=ctx.get("to"))
                if not grant_ep:
                    reason = "which specific item or episode do you want to share?"
                    ctx["refusal"] = reason
                    return Stop(reason)
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
        ctx["labels"]["words_hidden"] = (
            await _words_hidden(kernel, ctx["event"], str(place))
            if place and place != ctx["event"].place else False
        )
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


def carries_file(arguments: Any, descriptor: Any) -> bool:
    """Return whether one act carries a file attachment with a communicative capability."""
    if not (descriptor or {}).get("communicative"):
        return False
    values = arguments or {}
    return any(values.get(key) for key in ("attachment", "attachments", "file"))


async def leaving(kernel: Any, ctx: Any, arguments: Any) -> str | None:
    """Return None (D84 removed text-similarity checking; plugins narrow deterministically)."""
    del kernel, ctx, arguments
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
