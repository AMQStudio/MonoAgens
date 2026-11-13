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


def _event_line(kind: str, payload: Any) -> str:
    """Return the readable text carried by a Log event."""
    data = payload if isinstance(payload, dict) else {}
    if kind == "act.out":
        return str((data.get("arguments") or {}).get("text") or "")
    if kind == "act.result":
        result = data.get("result")
        return payload_text(result) if isinstance(result, dict) else ""
    return payload_text(data)


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


async def leaks(conn, target: str | None, text: str) -> bool:
    """Return whether a message would carry private content the target may not see (D45)."""
    if not text or not target:
        return False
    cache: dict = {}
    if await owner_direct(conn, target, cache):
        return False
    hidden: list[tuple[str, int]] = []
    seen_visible: list[str] = []
    cur = await conn.execute("SELECT text, derived_from FROM memories WHERE valid_to IS NULL")
    for line, sources in await cur.fetchall():
        if not line:
            continue
        if await memory_visible(conn, context=target, sources=sources, cache=cache):
            seen_visible.append(str(line))
        else:
            hidden.append((str(line), 4))
    ev_cur = await conn.execute(
        "SELECT id, kind, place, payload FROM events "
        "WHERE kind IN ('percept', 'act.out', 'act.result') AND place IS NOT NULL ORDER BY id")
    for event_id, kind, place, payload in await ev_cur.fetchall():
        line = _event_line(str(kind), payload)
        if not line:
            continue
        if await visible(conn, context=target, source=str(place), before=int(event_id),
                         cache=cache):
            seen_visible.append(line)
        else:
            hidden.append((line, 10))
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
            "AND (payload->>'communicative') = 'true'",
            (int(target), item.place))).fetchone()
    if row is None:
        row = await (await conn.execute(
            "SELECT act.id, act.origin_actor, act.place, act.caused_by "
            "FROM events msg JOIN events act ON msg.caused_by = act.id "
            "WHERE act.kind = 'act.out' AND act.place = %s "
            "AND (act.payload->>'communicative') = 'true' "
            "AND msg.kind IN ('act.confirmed', 'act.result') "
            "AND (msg.payload->>'message_id' = %s OR msg.payload->'result'->>'message_id' = %s) "
            "ORDER BY msg.id DESC LIMIT 1",
            (item.place, target, target))).fetchone()
    if row is None:
        row = await (await conn.execute(
            "SELECT id, origin_actor, place, caused_by FROM events "
            "WHERE kind = 'act.out' AND place = %s "
            "AND (payload->>'communicative') = 'true' AND payload->>'message_id' = %s "
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
        hidden = await leaks(ctx["conn"], place, str(item.get("text") or ""))
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
    for value in _strings(arguments):
        if await leaks(kernel.conn, ctx["event"].place, value):
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
