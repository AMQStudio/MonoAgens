"""Privacy: who can perceive a place, the one context check, and consent to share.

The core knows only opaque ids; this module reads the one fact that gives them meaning, the
audience a body reported for a place ([]), and applies the second absolute line
in exactly one place: the context hook, over the context's items.
"""

from __future__ import annotations

import json
from collections.abc import Mapping, Sequence
from dataclasses import replace
from typing import Any

from monoagens.contract import ContextItem
from monoagens.core.hooks import Continue, Middleware, Replace, Stop
from monoagens.infra.store import append_event, read_event_row, read_state, write_state
from monoagens.types import Event

OWNER = "person:owner"
UNKNOWN = "*unknown*"
SHARE = "self.share"
SHARE_CAPABILITY = {
    "name": "share",
    "description": "Share private content with a person or a place; only if you were part of it. "
                   "Name the event ids you mean. Content from another place needs this before "
                   "it can be repeated where its audience may not see it.",
    "schema": {"type": "object", "properties": {
        "items": {"type": "array", "items": {"type": "integer"},
                  "description": "The event ids of the content you mean."},
        "episode": {"type": "integer",
                    "description": "One event id, alone."},
        "to": {"type": "string", "description": "person:<id> or a place id."}},
        "required": ["to"]},
    "facts": (), "communicative": False, "long_running": False, "identity_changing": False,
}


def register(registry: Any) -> None:
    """Declare the middleware's own `self.share` capability on the kernel's registry."""
    registry.add_capability(SHARE, SHARE_CAPABILITY, facts=(), permission="privacy.share",
                            result=False)  # the guard records the grant itself


async def aliases(conn) -> dict[str, str]:
    """Return the alias map that middleware keeps in `state: alias:*`."""
    cur = await conn.execute("SELECT key, value FROM state WHERE key LIKE 'alias:%'")
    return {key[6:]: str((value or {}).get("who") or value) for key, value in await cur.fetchall()}


# ------------------------------------------------------- the one fact: an audience


async def _reported(conn, place: str, cache: dict | None = None
                    ) -> tuple[tuple[int, frozenset[str] | None], ...]:
    """Return every audience one place ever reported, oldest first, as `(event id, audience)`.

    A `place.audience` event is the one fact: it is never copied into place
    state, so the newest row at or before a moment is that place's audience then. An event that
    says `unknown` or names nobody gives None, which lets no private item in.
    """
    key = ("history", place)
    if cache is not None and key in cache:
        return cache[key]
    cur = await conn.execute(
        "SELECT id, payload FROM events WHERE kind = 'place.audience' AND place = %s ORDER BY id",
        (place,))
    rows = await cur.fetchall()
    mapping = await aliases(conn)
    history = tuple((int(found), _members(payload, mapping)) for found, payload in rows)
    if cache is not None:
        cache[key] = history
    return history


def _members(payload: Mapping[str, Any] | None, mapping: Mapping[str, str]
             ) -> frozenset[str] | None:
    """Return one reported audience as person ids, or None when the body could not know it."""
    stored = payload or {}
    if stored.get("unknown"):
        return None
    actors = [str(actor) for actor in stored.get("actors") or () if actor]
    if not actors:
        return None
    return frozenset(mapping.get(actor, actor) for actor in actors)


async def _parent_place(conn, place: str) -> str | None:
    """Return the place a task thread belongs to; a thread's audience is its parent's."""
    if not place.startswith("space:task:"):
        return None
    task = await read_state(conn, f"task:{place}")
    return str(task.get("parent") or "") or None if isinstance(task, dict) else None


async def audience(conn, place: str | None, before: int | None = None,
                   cache: dict | None = None) -> frozenset[str] | None:
    """Return who can perceive a place now, or at `before`; None when that is not known."""
    if not place:
        return None
    parent = await _parent_place(conn, place)
    if parent is not None:
        return await audience(conn, parent, before, cache)
    history = await _reported(conn, place, cache)
    found = None
    for event_id, members in history:
        if before is not None and event_id > before:
            break
        found = members
    return found


async def owner_direct(conn, place: str | None, cache: dict | None = None) -> bool:
    """Return whether a place's audience is only the owner, its direct place."""
    return await audience(conn, place, None, cache) == frozenset({OWNER})


async def direct_for(conn, place: str | None, who: Any, cache: dict | None = None) -> bool:
    """Return whether a place is a 1:1 chat with one person (and the owner), by its audience."""
    found = await audience(conn, place, None, cache)
    if not found or who not in found:
        return False
    return found <= {str(who), OWNER}


# ------------------------------------------------------------ the one predicate


def admits(target: frozenset[str] | None, item: frozenset[str] | None) -> bool:
    """Return whether content one audience saw may enter a place with the other.

    An unknown audience on either side means nobody may assume: content said where the audience
    was unknown is not private to anyone and enters; content said before a known audience enters
    only a place whose audience is inside it. The owner's own direct place sees everything.
    """
    if target == frozenset({OWNER}):
        return True
    if item is None:
        return True  # said where nobody was certain: not private to anyone
    if target is None:
        return False  # this place's audience is unknown: no private item enters
    return target <= item


async def granted_people(conn, event_id: int, cache: dict | None = None) -> frozenset[str]:
    """Return the people a `share.granted` added to one event's audience."""
    key = ("granted", int(event_id))
    if cache is not None and key in cache:
        return cache[key]
    cur = await conn.execute(
        "SELECT payload FROM events WHERE kind = 'share.granted' "
        "AND payload->'episodes' @> %s::jsonb", (json.dumps([int(event_id)]),))
    found = frozenset(str(person) for (payload,) in await cur.fetchall()
                      for person in (payload or {}).get("persons") or () if person)
    if cache is not None:
        cache[key] = found
    return found


async def source_audience(conn, event_id: int | None, cache: dict | None = None
                          ) -> frozenset[str] | None:
    """Return the audience one event was said to: its place and moment, plus any grant."""
    if event_id is None:
        return None
    row = await read_event_row(conn, int(event_id))
    if row is None:
        return None
    place = str(row.get("place") or "")
    found = await audience(conn, place, int(event_id), cache)
    delegated = await read_state(conn, f"delegated:{int(event_id)}") or {}
    extra = frozenset(str(actor) for actor in delegated.get("actors") or () if actor)
    extra |= await granted_people(conn, int(event_id), cache)
    if not place and not extra:
        return None
    if found is None:
        return None if not extra else extra
    return found | extra


MEMORY_SECTIONS = ("memories", "core_memory")


async def item_audience(conn, item: ContextItem, place: str, cache: dict | None = None
                        ) -> frozenset[str] | None:
    """Return the audience one context item was said to: the union over its sources.

    A memory is judged by the events it was derived from; one with no sources has no
    known audience at all, so it enters the owner's own place and nothing else.
    """
    memory = item.section in MEMORY_SECTIONS
    if not item.sources:
        return frozenset() if memory else await audience(conn, place, None, cache)
    union: frozenset[str] | None = frozenset()
    for source in item.sources:
        if memory:
            row = await read_event_row(conn, int(source))
            if row is not None and not row.get("place"):
                return frozenset()  # a memory with a source from nowhere is unknown
        found = await source_audience(conn, source, cache)
        if found is None:
            return None
        union = (union or frozenset()) | found
    return union


async def visible(conn, *, context: str | None, source: str | None,
                  before: int | None = None, cache: dict | None = None) -> bool:
    """Return whether content from `source` may be shown in `context`."""
    if not context or not source:
        return False
    target = await audience(conn, context, None, cache)
    if before is None:
        return admits(target, await audience(conn, source, None, cache))
    return admits(target, await source_audience(conn, int(before), cache))


async def memory_visible(conn, *, context: str | None, sources: Any = (),
                         cache: dict | None = None) -> bool:
    """Return whether every source of a memory was said to the people here."""
    if not context:
        return False
    target = await audience(conn, context, None, cache)
    ids = [int(source) for source in sources or ()]
    if not ids:
        return target == frozenset({OWNER})  # a memory with no source is the owner's alone
    union: frozenset[str] | None = frozenset()
    for event_id in ids:
        found = await source_audience(conn, event_id, cache)
        if found is None:
            return admits(target, None)
        union = (union or frozenset()) | found
    return admits(target, union)


# -------------------------------------------------------- the one context check


def withheld_line(counts: Mapping[str, int]) -> str:
    """Return the D56 line: how many relevant private items are absent, and from where.

    It names only the count and the source label; it never names the content.
    """
    parts = [f"{count} thing{'s' if count != 1 else ''} you know from {label}"
             for label, count in sorted(counts.items())]
    total = sum(counts.values())
    verb = "are" if total != 1 else "is"
    return f"{' and '.join(parts)} {verb} not shown here"


def audience_size(kernel: Any) -> Middleware:  # noqa: ARG001 - one rule shape for every hook
    """Inbound rule: the middleware's one label — how many known people can perceive the event.

    The number is the contract's own feature `audience_size`, so it reaches the
    gate inside `Situation` and is never a label the core would have to read.
    """

    async def rule(item: Any, ctx: Any) -> Any:
        """Write the count of known people here, once, from the place's own audience."""
        if not isinstance(item, Event) or item.kind != "percept" or item.imported:
            return Continue()
        found = await audience(ctx["conn"], item.place)
        size = len([who for who in found or () if who not in (UNKNOWN, OWNER)])
        if not size:
            return Continue()
        return Replace(replace(item, features={**dict(item.features or {}),
                                               "audience_size": size}))

    return rule


def items_rule(kernel: Any = None) -> Middleware:
    """Absolute line 2: one check over every context item, in the last slot."""

    async def rule(item: Any, ctx: Any) -> Any:
        """Keep what the place's audience may see, and say how many items were withheld."""
        conn = ctx["conn"]
        cache: dict = {}  # one history per place, not one query per item
        target = await audience(conn, item.place, None, cache)
        kept: list[ContextItem] = []
        withheld: dict[str, int] = {}
        for one in item.items:
            if one.section == "cards" or admits(
                    target, await item_audience(conn, one, str(item.place), cache)):
                kept.append(one)
                continue
            label = one.place or "another place"
            withheld[label] = withheld.get(label, 0) + 1
        if item.trigger is not None and not any(
                one.section == "trigger" for one in kept) and item.place:
            return Stop("the event that woke this place is not visible here")
        if withheld:
            kept.append(ContextItem(section="notes", text=withheld_line(withheld)))
        filtered = replace(item, items=tuple(kept))
        if kernel is not None:
            await _record_wake_sources(kernel, conn, filtered, cache)
        return Replace(filtered) if filtered.items != item.items else Continue()

    return rule


async def _record_wake_sources(kernel: Any, conn: Any, item: Any, cache: dict) -> None:
    """Remember the private sources a verified wake held, keyed by `(trigger.id, place)`."""
    if item.trigger is None or item.trigger.id is None:
        return
    sources: list[tuple[str, int]] = []
    for one in item.items:
        for source in one.sources:
            row = await read_event_row(conn, int(source))
            if row is None or not row.get("place"):
                continue
            pair = (str(row["place"]), int(source))
            if pair not in sources:
                sources.append(pair)
    store = getattr(kernel, "_wake_private_sources", None)
    if store is None:
        store = {}
        setattr(kernel, "_wake_private_sources", store)
    store[(int(item.trigger.id), str(item.place))] = tuple(sources)
    while len(store) > 256:
        store.pop(next(iter(store)))


# ------------------------------------------------------------ consent to share


async def consent(conn, event: Event, arguments: Mapping[str, Any],
                  *, to: Any = None) -> tuple[bool, str | None, tuple[int, ...]]:
    """Return whether the chain's actor may share the content it names, its place and the ids."""
    del to
    named = [int(item) for item in arguments.get("items") or ()]
    if arguments.get("episode") is not None:
        named = [int(arguments["episode"])]
    if not named:
        return False, event.place, ()
    ids = tuple(sorted(set(named)))
    actor = event.origin_actor
    if not actor:
        return False, event.place, ids
    place: str | None = event.place
    for event_id in ids:
        row = await read_event_row(conn, event_id)
        if row is None:
            return False, event.place, ids
        source = str(row.get("place") or event.place or "")
        if actor not in (await audience(conn, source, event_id) or frozenset()):
            return False, source or event.place, ids
        place = source or place
    return True, place, ids


async def grant(kernel, event: Event, arguments: Mapping[str, Any], to: str | None,
                source: str | None, episode: int | Sequence[int] | None = None) -> None:
    """Record consent for the specific events and people a `self.share` names."""
    people = ({str(to)} if str(to).startswith("person:")
              else (await audience(kernel.conn, str(to)) or frozenset()))
    raw = arguments.get("items") if arguments.get("episode") is None else episode
    if raw is None:
        raw = [arguments["episode"]] if arguments.get("episode") is not None else []
    episodes = sorted({int(item) for item in (raw or ())})
    await kernel._emit(event, "share.granted", {
        "episodes": episodes, "to": to, "source_place": source,
        "persons": sorted(person for person in people if person != UNKNOWN)})


# ------------------------------------------------ asking on someone's behalf


async def _send_answered(conn, event: Event) -> tuple[int, str] | None:
    """Return the soul's send a percept answers, from the percept's own `about` ref."""
    ref = str(((event.payload or {}).get("about")) or "")
    if not ref or not event.place:
        return None
    row = await (await conn.execute(
        "SELECT id, origin_actor FROM events WHERE kind = 'act.out' AND place = %s "
        "AND id::text = %s AND (payload->>'communicative') = 'true' LIMIT 1",
        (event.place, ref))).fetchone()
    if row is None or not str(row[1] or "").startswith("person:"):
        return None
    return int(row[0]), str(row[1])


def delegation(kernel: Any) -> Middleware:
    """Inbound rule: the one answer to the soul's send carries its requester's audience."""

    async def rule(item: Any, ctx: Any) -> Any:
        """Record `delegated:<id>` for the send this percept answers, once."""
        if not isinstance(item, Event) or item.kind != "percept" or item.id is None:
            return Continue()
        if item.imported or not item.place:
            return Continue()
        matched = await _send_answered(kernel.conn, item)
        if matched is None:
            return Continue()
        act_id, actor = matched
        if await read_state(kernel.conn, f"delegated_act:{act_id}") is not None:
            return Continue()
        await write_state(kernel.conn, f"delegated_act:{act_id}",
                          {"reply_event": item.id}, item.id)
        await write_state(kernel.conn, f"delegated:{item.id}", {"actors": [actor]}, item.id)
        return Continue()

    return rule


# --------------------------------------------------- speaking elsewhere


def intent_words(kernel: Any) -> Middleware:
    """Inbound rule: an intent quotes its words only where their source may be shown."""

    async def rule(item: Any, ctx: Any) -> Any:
        """Replace an intent's words with the neutral line when they may not be shown here."""
        if not isinstance(item, Event) or item.kind != "intent" or not item.place:
            return Continue()
        source = await _intent_source(kernel.conn, item)
        if source is None or not await _words_hidden(kernel, source, str(item.place)):
            return Continue()
        target = str((item.payload or {}).get("to") or item.place)
        return Replace(replace(item, description=(
            f"you set out to send a message to {target}; its words cannot appear here "
            f"(from {source.place})")))

    return rule


async def _intent_source(conn, intent: Event) -> Event | None:
    """Return the event whose wake set one intent out, from the intent's own chain."""
    if intent.caused_by is None:
        return None
    row = await (await conn.execute(
        "SELECT src.id, src.place FROM events asked "
        "JOIN events src ON src.id = asked.caused_by WHERE asked.id = %s",
        (int(intent.caused_by),))).fetchone()
    if row is None:
        return None
    return Event(id=int(row[0]), ts=intent.ts, kind="act.out", place=row[1])


async def _words_hidden(kernel: Any, event: Event, target_place: str) -> bool:
    """Return whether anything private in the source wake's view is hidden from `target_place`."""
    cache: dict = {}
    if await owner_direct(kernel.conn, target_place, cache):
        return False
    store = getattr(kernel, "_wake_private_sources", None) or {}
    key = (int(event.id), str(event.place or "")) if event.id is not None else None
    sources = store.get(key) if key is not None else None
    if sources is None:
        sources = (((str(event.place or ""), int(event.id)),) if event.id is not None
                   and event.place else ())
    for source_place, source_id in sources:
        if not source_place or not await visible(kernel.conn, context=target_place,
                                                 source=source_place, before=source_id,
                                                 cache=cache):
            return True
    return False


# ------------------------------------------------------------ the action hook


def episode_error(arguments: dict[str, Any]) -> str:
    """Return why `episode` is wrong, or "" when it is absent or an event id."""
    raw = arguments.get("episode")
    if raw is None:
        return ""
    if isinstance(raw, bool) or not isinstance(raw, int) or raw <= 0:
        return f"episode: expected an event id, got {raw!r}"
    return ""


def guard(kernel: Any, when: str) -> Middleware:
    """Absolute line 3: consent to share and the drafted words of an intent."""

    async def rule(item: Any, ctx: Any) -> Any:
        """Apply sharing consent, and hide an intent's words where they may not appear."""
        if ctx.get("capability") == SHARE:
            if when == "first":
                wrong = episode_error(item)
                if wrong:
                    ctx["refusal"] = wrong
                    return Stop(wrong)
                if not (item.get("items") or item.get("episode") is not None):
                    reason = "which event ids do you want to share?"
                    ctx["refusal"] = reason
                    return Stop(reason)
                allowed, source, ids = await consent(ctx["conn"], ctx["event"], item,
                                                    to=ctx.get("to"))
                if not allowed:
                    reason = "only someone who was part of it can share it"
                    ctx["refusal"] = reason
                    return Stop(reason)
                ctx["grant_source"], ctx["grant_episode"] = source, ids
                return Continue()
            if ctx.get("refusal") or ctx.get("handled"):
                return Continue()
            await grant(kernel, ctx["event"], item, ctx.get("to"), ctx.get("grant_source"),
                        episode=ctx.get("grant_episode"))
            ctx["handled"] = True
            return Continue()
        if when == "first":
            if await forward_needs_confirmation(ctx["conn"], ctx, item):
                ctx["call_facts"] = set(ctx.get("call_facts") or ()) | {"sensitive"}
                ctx["approval_reason"] = FORWARDING
            return Continue()
        place = ctx.get("place") or ctx["event"].place
        if place and place != ctx["event"].place and await _words_hidden(
                kernel, ctx["event"], str(place)):
            # D45: the intent's line never quotes words the other place may not see; the
            # arguments reasoning asked for stay in the Log, so nothing is lost.
            return Replace({key: value for key, value in item.items() if key != "text"})
        return Continue()

    return rule


FORWARDING = "forwarding a file"


def carries_file(arguments: Any, descriptor: Any) -> bool:
    """Return whether one communicative act carries a stored file (DEC-9)."""
    if not (descriptor or {}).get("communicative"):
        return False
    return any(refs_in(value) for value in (arguments or {}).values())


def refs_in(value: Any) -> bool:
    """Return whether one argument value is a stored attachment reference, or holds one."""
    if isinstance(value, str):
        return bool(value) and _STORE.find(value) is not None
    if isinstance(value, (list, tuple)):
        return any(refs_in(item) for item in value)
    if isinstance(value, Mapping):
        return any(refs_in(item) for item in value.values())
    return False


class _StoreProbe:
    """The attachment store the action hook asks whether one string is a stored file."""

    store: Any = None

    def find(self, ref: str) -> Any:
        """Return the stored file for one reference, or None."""
        return self.store.find(ref) if self.store is not None else None


_STORE = _StoreProbe()


def bind_store(store: Any) -> None:
    """Bind the attachment store the forwarding check asks about (the loader does this once)."""
    _STORE.store = store


async def forward_needs_confirmation(conn: Any, ctx: Any, arguments: Any) -> bool:
    """Return whether forwarding these files reaches someone who was not part of them."""
    if not carries_file(arguments, ctx.get("descriptor")):
        return False
    to = str(ctx.get("to") or "")
    if not to:
        return False  # the same place: the audience already has it
    source = await audience(conn, ctx["event"].place) or frozenset()
    if to.startswith("person:"):
        return to not in source
    return not set(await audience(conn, to) or ()) <= set(source)


async def leaving(kernel: Any, ctx: Any, arguments: Any) -> str | None:
    """Return None."""
    del kernel, ctx, arguments
    return None


__all__ = ["OWNER", "SHARE", "SHARE_CAPABILITY", "UNKNOWN", "admits", "aliases", "audience",
           "audience_size",
           "bind_store", "carries_file", "consent", "delegation", "episode_error", "grant",
           "guard", "intent_words", "item_audience", "items_rule", "memory_visible",
           "direct_for", "owner_direct", "register", "source_audience", "visible", "withheld_line"]
