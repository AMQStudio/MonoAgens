"""Identity: the alias map, pairing the first owner, the person cards and the routes."""

from __future__ import annotations

import logging
import re
import secrets
from collections.abc import Mapping
from dataclasses import replace
from datetime import datetime, timezone
from typing import Any

from monoagens.core.hooks import Continue, Middleware, Replace, Stop
from monoagens.infra.store import append_event, payload_text, read_state, write_state
from monoagens.middleware import privacy
from monoagens.types import Event

LOGGER = logging.getLogger(__name__)
OWNER = "person:owner"
CONTEXT_CAP = 200  # one role's or person's context line on a card (middleware.md §8)
CODE_KEY = "pairing_code"
OWNER_KEY = "owner"
CREATED = "pair.created"
CLAIMED = "pair.claimed"


def is_owner(who: Any) -> bool:
    """Return whether this opaque id is the owner's."""
    return str(who) == OWNER


def is_person(who: Any) -> bool:
    """Return whether this opaque id names a person rather than a place."""
    return str(who).startswith("person:")


def new_code(draw=secrets.randbelow) -> str:
    """Return a six-digit code, split for readability (for example 123-456)."""
    digits = "".join(str(draw(10)) for _ in range(6))
    return f"{digits[:3]}-{digits[3:]}"


async def ensure(conn, *, now: datetime) -> str | None:
    """Create and show the pairing code when the soul has no owner yet."""
    if await read_state(conn, OWNER_KEY) is not None:
        return None
    current = await read_state(conn, CODE_KEY) or {}
    if current.get("code"):
        return str(current["code"])
    code = new_code()
    async with conn.transaction():
        event = await append_event(conn, ts=now, kind=CREATED, payload={"created": True})
        await write_state(conn, CODE_KEY, {"code": code}, event.id)
    LOGGER.info("pairing code: %s — send it from any adapter to become the owner", code)
    return code


async def claim_owner(conn, event, *, now: datetime):
    """Make the sender of the pairing code the owner; return the claim event."""
    if event.kind != "percept" or not event.who:
        return None
    if await read_state(conn, OWNER_KEY) is not None:
        return None
    current = await read_state(conn, CODE_KEY) or {}
    code = str(current.get("code") or "")
    if not code or code not in payload_text(event.payload):
        return None
    async with conn.transaction():
        claimed = await append_event(conn, ts=now, kind=CLAIMED, who=event.who,
                                     place=event.place, body=event.body,
                                     payload={"code_used": True}, caused_by=event.id)
        await write_state(conn, OWNER_KEY, {"who": event.who}, claimed.id)
        await conn.execute("DELETE FROM state WHERE key = %s", (CODE_KEY,))
    LOGGER.info("owner paired: %s", event.who)
    return claimed


async def as_owner(conn, event: Event) -> Event:
    """Rewrite the paired owner's native id to `person:owner` at intake."""
    owner = await read_state(conn, OWNER_KEY) or {}
    if event.who and event.who == owner.get("who"):
        return replace(event, who=OWNER)
    return event


def intake(conn):
    """Return the intake callable the composition root installs on the kernel."""

    async def accept(event: Event) -> Event:
        """Take one percept from an adapter and give it the id the middleware knows."""
        return await as_owner(conn, event)

    return accept


def claim(conn) -> Middleware:
    """Inbound rule: claim the owner when a percept carries the pairing code."""

    async def rule(item: Any, ctx: Any) -> Any:
        await claim_owner(conn, item, now=ctx["now"])
        return Continue()

    return rule


def cards(conn, *, roles: Any = None) -> Middleware:
    """Context rule: add one card per person present here or named in the trigger (D55)."""

    async def rule(item: Any, ctx: Any) -> Any:
        return Replace(replace(item, people=await person_cards(conn, item.place, item.trigger,
                                                               roles=roles)))

    return rule


def identity_line() -> Middleware:
    """Absolute line 1: identity and privacy decisions belong to the owner alone."""

    async def rule(item: Any, ctx: Any) -> Any:
        if ctx.get("identity_changing") and not is_owner(ctx.get("origin_actor")):
            return Stop("only the owner may invoke identity_changing capabilities")
        return Continue()

    return rule


def route(conn) -> Middleware:
    """Action rule: resolve `to:`/`via:` to a place and attach the routing labels."""

    async def rule(item: Any, ctx: Any) -> Any:
        if ctx.get("refusal") or ctx.get("handled") or not ctx["descriptor"].get("communicative"):
            return Continue()
        to, via = ctx.get("to"), ctx.get("via")
        if not to:
            return Continue()  # the current place
        place, error = await resolve_to(conn, str(to), via)
        if place is None and str(to) == ctx["event"].who:
            place, error = ctx["event"].place, None  # the person is here: answer them here
        if (place is None and is_person(to)
                and str(to) in await privacy.audience(conn, ctx["event"].place)):
            place, error = ctx["event"].place, None  # the person is here: answer them here
        ctx["to"] = str(to)
        if error:
            ctx["refusal"] = error
            return Continue()
        ctx["place"] = place or ctx["event"].place
        labels = ctx["labels"]
        labels["place_kind"] = await privacy.place_kind(conn, ctx["place"])
        labels["hold_for"] = str(to) if is_owner(to) else None
        labels["delivers"] = bool(to) and not is_person(to)
        return Continue()

    return rule


async def person_cards(conn, place: str | None, event: Event | None,
                       roles: Any = None) -> tuple[str, ...]:
    """Return one card per person present here or named in the trigger (D55)."""
    present = set(await privacy.audience(conn, place)) if place else set()
    known = await _known_people(conn)
    text = payload_text(event.payload) if event is not None else ""
    named = {who for who, card in known.items()
             if card["name"] and _named(card["name"], text)}
    people = sorted((present | named) - {OWNER})
    return tuple([await _card(conn, who, known.get(who) or {"name": None, "closeness": 0.0,
                                                            "natives": {who}}, roles=roles)
                  for who in people])


async def name_of(conn, who: str) -> str | None:
    """Return the name the soul knows for one person id, if it knows one."""
    known = await _known_people(conn)
    return (known.get(str(who)) or {}).get("name")


async def accounts_of(conn, who: str) -> tuple[str, ...]:
    """Return the native accounts linked to one person id, as `<body>:<native id>`."""
    mapping = await privacy.aliases(conn)
    found = [account for account, person in mapping.items() if person == str(who)]
    if not found and str(who).startswith("person:"):
        native = str(who).removeprefix("person:")
        if native != "owner":
            found.append(native)
    return tuple(sorted(found))


async def find_person(conn, query: str, *, labels: Mapping[str, str] | None = None,
                      ) -> str | None:
    """Return the person id matching an account, a person id or a known name."""
    text = str(query).strip()
    if not text:
        return None
    mapping = await privacy.aliases(conn)
    if text in mapping:
        return mapping[text]
    known = await _known_people(conn)
    if text in known:
        return text
    for who, label in (labels or {}).items():
        if str(label).lower() == text.lower():
            return who
    for who, card in known.items():
        if card["name"] and str(card["name"]).lower() == text.lower():
            return who
    return None


async def _known_people(conn) -> dict[str, dict[str, Any]]:
    """Return the card data of every person the alias map or the Log has seen."""
    mapping = await privacy.aliases(conn)
    cur = await conn.execute("SELECT DISTINCT who FROM events WHERE who IS NOT NULL")
    rows = await cur.fetchall()
    states = await _person_states(conn)
    known = {}
    for who in {mapping.get(native, native) for (native,) in rows} | set(mapping.values()):
        state = states.get(who) or {}
        known[who] = {"name": _name(who, state),
                      "closeness": float(state.get("closeness") or 0.0),
                      "natives": {who}}
    for native, who in mapping.items():
        known.setdefault(who, {"name": None, "closeness": 0.0, "natives": set()})
        known[who]["natives"].add(native)
    return known


async def _person_states(conn) -> dict[str, dict[str, Any]]:
    """Return the `state: person:*` entries by person id."""
    cur = await conn.execute("SELECT key, value FROM state WHERE key LIKE 'person:%'")
    return {str(key): value or {} for key, value in await cur.fetchall()}


def _name(who: str, state: dict[str, Any]) -> str | None:
    """Return the name a card knows: the state's, else the last id segment if it is a word."""
    name = str(state.get("name") or "").strip()
    if name:
        return name
    part = who.removeprefix("person:").rpartition(":")[2]
    return part if part.isalpha() else None


def _named(name: str, text: str) -> bool:
    """Return whether a card name appears in the trigger as a whole word."""
    return bool(text) and re.search(rf"(?<!\w){re.escape(name)}(?!\w)", text,
                                    re.IGNORECASE) is not None


async def _card(conn, who: str, card: dict[str, Any], *, roles: Any = None) -> str:
    """Render one person card: name, person id, reachable adapters, closeness, role context."""
    places = await _direct_places(conn, card["natives"])
    reach = "; ".join(f"{adapter} at {place} (last {seen.astimezone(timezone.utc):%Y-%m-%d %H:%M})"
                      for adapter, (place, seen) in sorted(places.items()))
    label = f"{card['name']} ({who})" if card["name"] else who
    line = f"- {label}: {reach or 'no direct place known yet'}; closeness {card['closeness']:.2f}"
    contexts = [str(context)[:CONTEXT_CAP] for context in (roles.contexts(who) if roles else ())]
    return "\n".join([line, *[f"  {context}" for context in contexts if context]])


async def _direct_places(conn, natives: set[str]) -> dict[str, tuple[str, datetime]]:
    """Return the direct place, and when it was last active, per adapter for one person."""
    if not natives:
        return {}
    cur = await conn.execute(
        "SELECT DISTINCT ON (body) body, place, ts FROM events WHERE kind = 'percept' "
        "AND place IS NOT NULL AND who = ANY(%s) AND payload->>'place_kind' = 'direct' "
        "ORDER BY body, id DESC", (sorted(natives),))
    return {str(body or "?"): (place, seen) for body, place, seen in await cur.fetchall()}


async def direct_place(conn, who: str, via: str | None = None) -> str | None:
    """Return the 1:1 place where one person was last active, optionally on one adapter."""
    mapping = await privacy.aliases(conn)
    cur = await conn.execute(
        "SELECT who, place, body, payload->>'place_kind' FROM events "
        "WHERE kind = 'percept' AND place IS NOT NULL AND who IS NOT NULL "
        "ORDER BY id DESC")
    for native, place, body, kind in await cur.fetchall():
        if mapping.get(native, native) != who or (via and str(body or "").lower() != via.lower()):
            continue
        if kind == "direct":
            return place
    return None


async def resolve_to(conn, to: str, via: str | None = None) -> tuple[str | None, str | None]:
    """Resolve an act's `to:` into a target place; a missing route is an honest error."""
    if not is_person(to):
        found = await (await conn.execute(
            "SELECT 1 FROM events WHERE place = %s LIMIT 1", (str(to),))).fetchone()
        return (str(to), None) if found else (None, f"no such place: {to}")
    place = await direct_place(conn, str(to), via)
    if place is None:
        return None, f"no way to reach {str(to).removeprefix('person:')} yet"
    return place, None
