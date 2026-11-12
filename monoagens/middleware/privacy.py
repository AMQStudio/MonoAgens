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

from monoagens.core.hooks import Continue, Middleware, Replace, Stop
from monoagens.infra.store import payload_text, read_state
from monoagens.types import Event

OWNER = "person:owner"
SHARE = "self.share"
SHARE_CAPABILITY = {
    "name": "share",
    "description": "Share private content with a person or a place; only if you were part of it. "
                   "Content from another place needs this before it can be repeated where its "
                   "audience may not see it.",
    "schema": {"type": "object", "properties": {
        "item": {"type": "string"}, "episode": {"type": "integer"},
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


async def _audience(conn, place: str | None, before: int | None = None,
                    cache: dict | None = None) -> frozenset[str]:
    """Return one place's audience, remembered for one wake when a cache is given."""
    if cache is None:
        return await audience(conn, place, before)
    key = ("people", place, before)
    if key not in cache:
        cache[key] = await audience(conn, place, before)
    return cache[key]


async def _shared(conn, place: str, cache: dict | None = None) -> frozenset[str]:
    """Return what `share.granted` added to one place, remembered for one wake."""
    if cache is None:
        return await shared(conn, place)
    key = ("shared", place)
    if key not in cache:
        cache[key] = await shared(conn, place)
    return cache[key]


async def audience(conn, place: str | None, before: int | None = None) -> frozenset[str]:
    """Return the person ids present at a place (at `before`, after the alias map)."""
    if not place:
        return frozenset()
    task = await read_state(conn, f"task:{place}")
    if task:
        return frozenset(task.get("audience") or ())
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
        return frozenset(others) if len(others) == 1 else frozenset(people)
    return frozenset(people)


async def shared(conn, place: str, before: int | None = None) -> frozenset[str]:
    """Return the people a `share.granted` added to a place's audience, up to `before`."""
    query, params = ("SELECT payload FROM events WHERE kind = 'share.granted' "
                     "AND payload->>'source_place' = %s"), [place]
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
    consented = set(await _shared(conn, source, cache))
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
        consented = set(await _shared(conn, place, cache))
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
        consented = set(await _shared(conn, source_place, cache))
        if not need <= (present | consented):
            return await source_label(conn, source_place, source, cache)
    return await source_label(conn, None, None, cache)


async def leaks(conn, target: str | None, text: str) -> bool:
    """Return whether a message would carry private content the target may not see (D45)."""
    if not text or not target:
        return False
    cur = await conn.execute("SELECT text, derived_from FROM memories WHERE valid_to IS NULL")
    for line, sources in await cur.fetchall():
        if line and line in text and not await memory_visible(
                conn, context=target, sources=sources):
            return True
    return False


async def consent(conn, event: Event, arguments: dict[str, Any]) -> tuple[bool, str | None]:
    """Return whether the chain's actor may share this content, and the content's place."""
    source = event.place
    if arguments.get("episode"):
        row = await (await conn.execute(
            "SELECT place FROM events WHERE id = %s", (int(arguments["episode"]),))).fetchone()
        source = (row or (None,))[0] or source
    return event.origin_actor in await audience(conn, source), source


async def grant(kernel, event: Event, arguments: dict[str, Any], to: str | None,
                source: str | None) -> None:
    """Record consent for the people a `self.share` names (D46)."""
    people = ({str(to)} if str(to).startswith("person:")
              else await audience(kernel.conn, str(to)))
    await kernel._emit(event, "share.granted", {
        "item": arguments.get("item"), "episode": arguments.get("episode"), "to": to,
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
                allowed, source = await consent(ctx["conn"], ctx["event"], item)
                if not allowed:
                    ctx["refusal"] = "only someone who was part of it can share it"
                    return Stop("only someone who was part of it can share it")
                ctx["grant_source"] = source
                return Continue()
            if ctx.get("refusal") or ctx.get("handled"):
                return Continue()
            await grant(kernel, ctx["event"], item, ctx.get("to"), ctx.get("grant_source"))
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
