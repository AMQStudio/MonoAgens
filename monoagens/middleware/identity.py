"""Identity: the alias map, pairing the first owner, the person cards and the routes."""

from __future__ import annotations

import asyncio
import json
import logging
import re
import secrets
import weakref
from collections.abc import Mapping
from dataclasses import replace
from datetime import datetime, timedelta, timezone
from typing import Any

from monoagens.contract import ContextItem
from monoagens.core.hooks import Continue, Middleware, Replace, Stop
from monoagens.core.memory.cards import canonical, merge_map
from monoagens.infra.clock import Clock
from monoagens.infra.store import append_event, payload_text, read_state, write_state
from monoagens.middleware import privacy
from monoagens.middleware.registry import CommandEntry
from monoagens.types import Event

LOGGER = logging.getLogger(__name__)
OWNER = "person:owner"
CONTEXT_CAP = 200  # one role's or person's context line on a card (middleware.md §8)
CODE_KEY = "pairing_code"
OWNER_KEY = "owner"
CREATED = "pair.created"
CLAIMED = "pair.claimed"
REGISTERED = "identity.registered"
MERGED = "identity.merged"
UNLINKED = "identity.unlinked"
LINK_PREFIX = "link_code:"
LINK_TRIES_PREFIX = "link_tries:"
PAIR_TRIES_PREFIX = "pair_tries:"
LINK_TTL = timedelta(minutes=10)
LINK_TRIES = 3
_CONN_LOCKS: weakref.WeakKeyDictionary[Any, asyncio.Lock] = weakref.WeakKeyDictionary()


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


async def _tries_blocked(conn: Any, key: str, *, now: datetime,
                         limit: int = LINK_TRIES) -> bool:
    """Return True while ``key`` is locked out after hitting ``limit`` wrong guesses."""
    state = await read_state(conn, key)
    if not state:
        return False
    reset_at = datetime.fromisoformat(str(state["reset_at"]))
    if reset_at <= now:
        await conn.execute("DELETE FROM state WHERE key = %s", (key,))
        return False
    return int(state.get("tries") or 0) >= limit


async def _record_wrong_try(conn: Any, key: str, cause_id: int, *, now: datetime,
                            ttl: timedelta = LINK_TTL,
                            limit: int = LINK_TRIES) -> bool:
    """Record one wrong guess for ``key`` and return True once it reaches ``limit``."""
    state = await read_state(conn, key)
    tries = 0
    reset_at = now + ttl
    if state:
        existing_reset = datetime.fromisoformat(str(state["reset_at"]))
        if existing_reset > now:
            tries = int(state.get("tries") or 0)
            reset_at = existing_reset
    tries += 1
    await write_state(
        conn, key, {"tries": tries, "reset_at": reset_at.isoformat()}, cause_id)
    return tries >= limit


async def claim_owner(conn, event, *, now: datetime):
    """Make the sender of the pairing code the owner; return the claim event."""
    if event.kind != "percept" or not event.who:
        return None
    if (event.payload or {}).get("imported"):
        return None
    if await read_state(conn, OWNER_KEY) is not None:
        return None
    row = await (await conn.execute(
        "SELECT value, updated_by_event FROM state WHERE key = %s",
        (CODE_KEY,))).fetchone()
    if row is None:
        return None
    current, code_event_id = dict(row[0] or {}), int(row[1])
    code = str(current.get("code") or "")
    text = payload_text(event.payload).strip()
    if not code or not text:
        return None
    try_key = f"{PAIR_TRIES_PREFIX}{event.who}"
    if await _tries_blocked(conn, try_key, now=now):
        return None
    if code not in text:
        cause_id = event.id if event.id is not None else code_event_id
        await _record_wrong_try(conn, try_key, cause_id, now=now)
        return None
    async with conn.transaction():
        claimed = await append_event(conn, ts=now, kind=CLAIMED, who=event.who,
                                     place=event.place, body=event.body,
                                     payload={"code_used": True}, caused_by=event.id)
        await write_state(conn, OWNER_KEY, {"who": event.who}, claimed.id)
        await conn.execute(
            "UPDATE state SET value = %s, updated_by_event = %s "
            "WHERE key LIKE 'alias:%%' AND value->>'who' = %s",
            (json.dumps({"who": OWNER}), claimed.id, event.who))
        await conn.execute("DELETE FROM state WHERE key = %s", (CODE_KEY,))
        await conn.execute("DELETE FROM state WHERE key LIKE %s", (f"{PAIR_TRIES_PREFIX}%",))
    LOGGER.info("owner paired: %s", event.who)
    return claimed


def is_account(value: Any) -> bool:
    """Return whether one id is a native adapter account (`channel:123`), not a person id."""
    text = str(value or "")
    return (bool(text) and ":" in text and not text.startswith("person:")
            and not text.startswith("space:"))


def _conn_lock(conn: Any) -> asyncio.Lock:
    """Return one serialization lock for shared async connections during intake."""
    lock = _CONN_LOCKS.get(conn)
    if lock is None:
        lock = asyncio.Lock()
        _CONN_LOCKS[conn] = lock
    return lock


async def resolve_account(conn: Any, account: str, *, now: datetime,
                          place: str | None = None, body: str | None = None,
                          cause_id: int | None = None) -> str:
    """Map one native account to its person id, registering unknown accounts atomically."""
    if not is_account(account):
        return account
    key = f"alias:{account}"
    async with _conn_lock(conn):
        owner = await read_state(conn, OWNER_KEY) or {}
        if owner.get("who") == account:
            return OWNER
        existing = await read_state(conn, key)
        if existing is not None:
            person = str((existing or {}).get("who") or existing)
            return OWNER if owner.get("who") == person else person
        person = f"person:p_{secrets.token_hex(4)}"
        async with conn.transaction():
            await conn.execute("SELECT pg_advisory_xact_lock(hashtext(%s))", (key,))
            found = await read_state(conn, key)
            if found is not None:
                resolved = str((found or {}).get("who") or found)
                return OWNER if owner.get("who") == resolved else resolved
            registered = await append_event(
                conn, ts=now, kind=REGISTERED, who=person, place=place, body=body,
                payload={"account": account, "person": person}, caused_by=cause_id)
            await write_state(conn, key, {"who": person}, registered.id)
        return person


async def as_owner(conn, event: Event) -> Event:
    """Rewrite the paired owner's native id to `person:owner` at intake."""
    owner = await read_state(conn, OWNER_KEY) or {}
    if event.who and event.who == owner.get("who"):
        return replace(event, who=OWNER)
    return event


async def resolve_event(conn, event: Event, *, now: datetime) -> Event:
    """Map native accounts in `who`, `origin_actor`, `data.mentions`, and `data.reply_to` atomically."""
    if event.kind not in ("percept", "command"):
        return await as_owner(conn, event)
    async with conn.transaction():
        who = event.who
        if who and is_account(who):
            who = await resolve_account(conn, who, now=now, place=event.place, body=event.body)
        origin = event.origin_actor
        if origin and is_account(origin):
            origin = await resolve_account(conn, origin, now=now, place=event.place, body=event.body)
        payload = dict(event.payload or {})
        data = payload.get("data")
        if isinstance(data, Mapping):
            updated_data = dict(data)
            changed = False
            mentions = updated_data.get("mentions")
            if isinstance(mentions, (list, tuple)):
                mapped_mentions = [
                    (await resolve_account(conn, str(item), now=now, place=event.place,
                                           body=event.body)
                     if is_account(item) else item)
                    for item in mentions
                ]
                if list(mentions) != mapped_mentions:
                    updated_data["mentions"] = mapped_mentions
                    changed = True
            reply_to = updated_data.get("reply_to")
            if isinstance(reply_to, Mapping) and reply_to.get("who"):
                rwho = str(reply_to["who"])
                mapped_rwho = (
                    await resolve_account(conn, rwho, now=now, place=event.place, body=event.body)
                    if is_account(rwho) else rwho
                )
                if mapped_rwho != rwho:
                    updated_data["reply_to"] = {**reply_to, "who": mapped_rwho}
                    changed = True
            if changed:
                payload["data"] = updated_data
        resolved = replace(event, who=who, origin_actor=origin, payload=payload)
        return await as_owner(conn, resolved)


def intake(conn, *, clock: Clock | None = None):
    """Return the intake callable the composition root installs on the kernel."""
    clock_now = clock or Clock()

    async def accept(event: Event) -> Event:
        """Take one percept from an adapter and give it the id the middleware knows."""
        return await resolve_event(conn, event, now=clock_now.now())

    return accept


def claim(conn) -> Middleware:
    """Inbound rule: claim the owner when a percept carries the pairing code."""

    async def rule(item: Any, ctx: Any) -> Any:
        if (getattr(item, "payload", None) or {}).get("imported") is True:
            return Continue()
        await claim_owner(conn, item, now=ctx["now"])
        return Continue()

    return rule


async def shared_account_items(conn: Any) -> tuple[ContextItem, ...]:
    """Return one card per adapter whose own actor belongs to the owner (D66)."""
    cur = await conn.execute(
        "SELECT key, value FROM state WHERE key LIKE 'self_account:%' ORDER BY key")
    rows = await cur.fetchall()
    mapping = await privacy.aliases(conn)
    merges = await merge_map(conn)
    items: list[ContextItem] = []
    for key, value in rows:
        body = str(key).removeprefix("self_account:")
        account = str((value or {}).get("account") or "")
        person = str((value or {}).get("person") or "") or mapping.get(account, account)
        person = merges.get(person, person)
        if not body or not account:
            continue
        if person != OWNER and account != OWNER:
            continue
        items.append(ContextItem(
            section="cards", kind="shared_account",
            text=(f"- Shared account ({body}): speaking in someone else's chat on {body} "
                  f"speaks through the owner's account ({account}).")))
    return tuple(items)


async def record_self_account(conn: Any, body: str, native: str, *, now: datetime) -> None:
    """Record one adapter's own actor, once at install, so a card can say what it speaks as."""
    if not body or not native:
        return
    person = await resolve_account(conn, native, now=now, body=body)
    key = f"self_account:{body}"
    current = await read_state(conn, key) or {}
    if current.get("account") == native and current.get("person") == person:
        return
    async with conn.transaction():
        event = await append_event(conn, ts=now, kind="identity.self", body=body,
                                   payload={"account": native, "person": person})
        await write_state(conn, key, {"account": native, "person": person}, event.id)


def cards(conn, *, roles: Any = None) -> Middleware:
    """Context rule: add one card per person here, and the shared-account notes (D55, D66)."""

    async def rule(item: Any, ctx: Any) -> Any:
        cards = await person_cards(conn, item.place, item.trigger, roles=roles)
        items = tuple(ContextItem(section="cards", text=text, kind="person")
                      for text in cards)
        items += await shared_account_items(conn)
        if not items:
            return Continue()
        return Replace(replace(item, items=(*item.items, *items)))

    return rule


def identity_line() -> Middleware:
    """Absolute line 1: identity and privacy decisions belong to the owner alone."""

    async def rule(item: Any, ctx: Any) -> Any:
        if ctx.get("identity_changing") and not is_owner(ctx.get("origin_actor")):
            return Stop("only the owner may invoke identity_changing capabilities")
        return Continue()

    return rule


def route(conn) -> Middleware:
    """Action rule: resolve `to:`/`via:` to the place an act lands in (D46, D57)."""

    async def rule(item: Any, ctx: Any) -> Any:
        """Resolve a target that is not a place; no target means the current place."""
        if ctx.get("refusal") or ctx.get("handled") or not ctx["descriptor"].get("communicative"):
            return Continue()
        to, via = ctx.get("to"), ctx.get("via")
        if not to:
            return Continue()
        place, error = await resolve_to(conn, str(to), via)
        if place is None and str(to) == ctx["event"].who:
            place, error = ctx["event"].place, None  # the person is here: answer them here
        if place is None and is_person(to) and str(to) in (
                await privacy.audience(conn, ctx["event"].place) or frozenset()):
            place, error = ctx["event"].place, None  # the person is here: answer them here
        ctx["to"] = str(to)
        if error:
            ctx["refusal"] = error
            return Continue()
        ctx["place"] = place or ctx["event"].place
        return Continue()

    return rule


async def person_cards(conn, place: str | None, event: Event | None,
                       roles: Any = None) -> tuple[str, ...]:
    """Return one card per person here or involved in the trigger (D55)."""
    present = set(await privacy.audience(conn, place) or frozenset()) if place else set()
    involved = {str(actor) for actor in ((event.payload or {}).get("involves") or ())} \
        if event is not None else set()
    known = await _known_people(conn)
    people = sorted((present | involved) - {OWNER, privacy.UNKNOWN})
    return tuple([await _card(conn, who, known.get(who) or {"name": None, "closeness": 0.0,
                                                            "natives": {who}},
                              roles=roles, place=place)
                  for who in people])


async def name_of(conn, who: str) -> str | None:
    """Return the name the soul knows for one person id, if it knows one."""
    known = await _known_people(conn)
    return (known.get(str(who)) or {}).get("name")


async def accounts_of(conn, who: str) -> tuple[str, ...]:
    """Return the native accounts linked to one person id, as `<body>:<native id>`."""
    mapping = await privacy.aliases(conn)
    target = await canonical(conn, str(who)) or str(who)
    found = [account for account, person in mapping.items()
             if person == target and is_account(account)]
    if not found and target.startswith("person:"):
        native = target.removeprefix("person:")
        if native != "owner" and not native.startswith("p_"):
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
        return await canonical(conn, mapping[text])
    resolved = await canonical(conn, text)
    known = await _known_people(conn)
    if resolved in known:
        return resolved
    for who, label in (labels or {}).items():
        if str(label) == text:
            return await canonical(conn, who)
    for who, card in known.items():
        if card["name"] and str(card["name"]) == text:
            return who
    return None


async def _known_people(conn) -> dict[str, dict[str, Any]]:
    """Return the card data of every person the alias map or the Log has seen."""
    mapping = await privacy.aliases(conn)
    merges = await merge_map(conn)
    cur = await conn.execute("SELECT DISTINCT who FROM events WHERE who IS NOT NULL")
    rows = await cur.fetchall()
    states = await _person_states(conn)
    known: dict[str, dict[str, Any]] = {}
    resolve = lambda raw: merges.get(mapping.get(raw, raw), mapping.get(raw, raw))
    for who in {resolve(native) for (native,) in rows} | {resolve(v) for v in mapping.values()}:
        state = states.get(who) or {}
        known[who] = {"name": _name(who, state),
                      "closeness": float(state.get("closeness") or 0.0),
                      "natives": {who}}
    for native, person in mapping.items():
        who = merges.get(person, person)
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


async def _card(conn, who: str, card: dict[str, Any], *, roles: Any = None,
                place: str | None = None) -> str:
    """Render one person card: name, person id, reachable adapters, closeness, role context."""
    places = await _direct_places(conn, card["natives"])
    reach = "; ".join(f"{adapter} at {place} (last {seen.astimezone(timezone.utc):%Y-%m-%d %H:%M})"
                      for adapter, (place, seen) in sorted(places.items()))
    label = f"{card['name']} ({who})" if card["name"] else who
    line = f"- {label}: {reach or 'no direct place known yet'}; closeness {card['closeness']:.2f}"
    raw_contexts = (roles.contexts(who, place) if place else roles.contexts(who)) if roles else ()
    contexts = [str(context)[:CONTEXT_CAP] for context in raw_contexts]
    return "\n".join([line, *[f"  {context}" for context in contexts if context]])


async def _direct_places(conn, natives: set[str]) -> dict[str, tuple[str, datetime]]:
    """Return the direct place, and when it was last active, per adapter for one person.

    A place is direct for a person when the audience it reported is that person alone, or that
    person and the owner (middleware.md §3): the one fact, never a platform's own field.
    """
    if not natives:
        return {}
    mapping = await privacy.aliases(conn)
    merges = await merge_map(conn)
    people = {merges.get(mapping.get(native, native), mapping.get(native, native))
              for native in natives}
    cur = await conn.execute(
        "SELECT DISTINCT ON (place) place, payload FROM events "
        "WHERE kind = 'place.audience' AND place IS NOT NULL ORDER BY place, id DESC")
    direct: list[str] = []
    for place, payload in await cur.fetchall():
        members = {str(actor) for actor in (payload or {}).get("actors") or ()}
        members = {merges.get(mapping.get(actor, actor), mapping.get(actor, actor))
                   for actor in members}
        if not members or not members & people or members - {OWNER} - people:
            continue
        direct.append(str(place))
    if not direct:
        return {}
    cur = await conn.execute(
        "SELECT DISTINCT ON (body, place) body, place, ts FROM events WHERE kind = 'percept' "
        "AND place = ANY(%s) AND who = ANY(%s) ORDER BY body, place, id DESC",
        (sorted(direct), sorted(people)))
    places: dict[str, tuple[str, datetime]] = {}
    for body, place, seen in await cur.fetchall():
        places.setdefault(str(body or "?"), (str(place), seen))
    return places


async def direct_place(conn, who: str, via: str | None = None) -> str | None:
    """Return the 1:1 place where one person was last active, optionally on one adapter."""
    target = await canonical(conn, str(who)) or str(who)
    known = await _known_people(conn)
    natives = (known.get(target) or {}).get("natives") or {target}
    places = await _direct_places(conn, set(natives))
    if via:
        for body, (place, _) in places.items():
            if str(body) == str(via):
                return place
        return None
    if places:
        return max(places.values(), key=lambda pair: pair[1])[0]
    return None


async def resolve_to(conn, to: str, via: str | None = None) -> tuple[str | None, str | None]:
    """Resolve an act's `to:` into a target place; a missing route is an honest error."""
    if not is_person(to):
        found = await (await conn.execute(
            "SELECT 1 FROM events WHERE place = %s LIMIT 1", (str(to),))).fetchone()
        if found:
            return str(to), None
        return None, f"no such place: {to}"
    place = await direct_place(conn, str(to), via)
    if place is None:
        return None, f"no way to reach {str(to).removeprefix('person:')} yet"
    return place, None


async def _first_seen_id(conn, person: str) -> int:
    """Return the earliest event id introducing or sent by `person`, or a large sentinel."""
    row = await (await conn.execute(
        "SELECT min(id) FROM events WHERE who = %s "
        "OR (kind = %s AND payload->>'person' = %s)",
        (person, REGISTERED, person))).fetchone()
    return int(row[0]) if row and row[0] is not None else 10**15


async def _older_person(conn, first: str, second: str) -> tuple[str, str]:
    """Return `(from_id, into_id)`: `person:owner` always wins, else the older person id."""
    a = await canonical(conn, first) or first
    b = await canonical(conn, second) or second
    if a == OWNER:
        return b, a
    if b == OWNER:
        return a, b
    id_a, id_b = await _first_seen_id(conn, a), await _first_seen_id(conn, b)
    if id_a < id_b or (id_a == id_b and a <= b):
        return b, a
    return a, b


async def merge_people(kernel: Any, first: str, second: str, *, by: str, now: datetime,
                       place: str | None = None, body: str | None = None,
                       caused_by: int | None = None,
                       trigger: Event | None = None) -> tuple[str, str]:
    """Merge two person ids into the older surviving one, emitting `source.merged` (D65)."""
    conn = kernel.conn
    src, dst = await _older_person(conn, first, second)
    if src == dst:
        return src, dst
    async with conn.transaction():
        merged = await append_event(
            conn, ts=now, kind=MERGED, who=dst, place=place, body=body,
            payload={"from": src, "into": dst, "by": by}, caused_by=caused_by)
        await conn.execute(
            "UPDATE state SET value = %s, updated_by_event = %s "
            "WHERE key LIKE 'alias:%%' AND value->>'who' = %s",
            (json.dumps({"who": dst}), merged.id, src))
        await write_state(conn, f"alias:{src}", {"who": dst}, merged.id)
    base = trigger or Event(id=merged.id, ts=now, kind=MERGED, who=dst, place=place, body=body)
    await kernel._append(base, "source.merged", {"from": src, "into": dst}, caused_by=merged.id)
    if hasattr(kernel, "roles") and kernel.roles is not None:
        kernel.roles = kernel.roles.with_merges({src: dst})
    return src, dst


async def unlink_account(conn: Any, account: str, *, actor: str, now: datetime,
                         place: str | None = None, body: str | None = None,
                         caused_by: int | None = None) -> tuple[str, str]:
    """Give `account` a new person id from now on without splitting merged history (D65)."""
    mapping = await privacy.aliases(conn)
    if not is_account(account) or account not in mapping:
        raise ValueError(f"unknown account: {account}")
    old_person = await canonical(conn, mapping[account]) or mapping[account]
    new_person = f"person:p_{secrets.token_hex(4)}"
    async with conn.transaction():
        unlinked = await append_event(
            conn, ts=now, kind=UNLINKED, who=new_person, place=place, body=body,
            payload={"account": account, "from": old_person, "to": new_person, "by": actor},
            caused_by=caused_by)
        await write_state(conn, f"alias:{account}", {"who": new_person}, unlinked.id)
    return old_person, new_person


def _norm_code(raw: str) -> str:
    return "".join(ch for ch in str(raw) if ch.isdigit())


def _role_labels(kernel: Any) -> dict[str, str]:
    book = getattr(kernel, "roles", None)
    if book is None:
        return {}
    return {who: str(p.label) for who, p in book.people.items() if p.label}


async def _issue_link_code(conn: Any, caller: str, event: Event, now: datetime) -> str:
    code = new_code()
    expires = (now + LINK_TTL).isoformat()
    key = f"{LINK_PREFIX}{caller}"
    async with conn.transaction():
        ev = await append_event(conn, ts=now, kind="link.offered", who=caller,
                                place=event.place, body=event.body,
                                payload={"person": caller, "expires_at": expires},
                                caused_by=event.id)
        await write_state(conn, key, {
            "code": _norm_code(code), "person": caller, "expires_at": expires, "tries": 0},
            ev.id)
    return (f"/link code (from the other account, send /link {code}): {code} — "
            "single use, expires in 10 minutes (3 tries). Merging combines memory and private "
            "context permanently; unlinking later gives a new id without splitting memory.")


async def _consume_link_code(kernel: Any, event: Event, caller: str, guess: str,
                             now: datetime) -> str:
    conn = kernel.conn
    try_key = f"{LINK_TRIES_PREFIX}{caller}"
    if await _tries_blocked(conn, try_key, now=now):
        return "too many wrong /link codes; run /link again for a new code"
    digits = _norm_code(guess)
    rows = await (await conn.execute(
        "SELECT key, value, updated_by_event FROM state WHERE key LIKE %s",
        (f"{LINK_PREFIX}%",))).fetchall()
    matched = None
    fallback_id = event.id
    expired_match = False
    for key, raw, upd_id in rows:
        if fallback_id is None:
            fallback_id = int(upd_id)
        data = dict(raw or {})
        expires = datetime.fromisoformat(str(data.get("expires_at")))
        if expires <= now:
            await conn.execute("DELETE FROM state WHERE key = %s", (key,))
            if digits and data.get("code") == digits:
                expired_match = True
            continue
        if digits and data.get("code") == digits:
            matched = (str(key), data)
    if matched is not None:
        key, data = matched
        target = str(data.get("person") or "")
        if not target or target == caller:
            await conn.execute("DELETE FROM state WHERE key = %s", (key,))
            return f"that account already belongs to {caller}"
        if is_owner(caller) and not is_owner(target):
            return ("cannot link another person's code into the owner; "
                    "run /link from the owner account instead")
        await conn.execute("DELETE FROM state WHERE key = %s", (key,))
        await conn.execute("DELETE FROM state WHERE key = %s", (try_key,))
        src, dst = await merge_people(
            kernel, caller, target, by=caller, now=now, place=event.place,
            body=event.body, caused_by=event.id, trigger=event)
        return (f"linked: {src} merged into {dst}. Remembered history stays together "
                "(unlinking later only starts a new id from then on).")
    if fallback_id is None:
        row = await (await conn.execute("SELECT MAX(id) FROM events")).fetchone()
        fallback_id = int(row[0]) if row and row[0] is not None else None
    locked = False
    if fallback_id is not None:
        locked = await _record_wrong_try(conn, try_key, fallback_id, now=now)
    if expired_match:
        return "that /link code has expired; run /link again for a new code"
    if locked:
        return "too many wrong /link codes; run /link again for a new code"
    return "invalid or expired /link code"


async def link_command(kernel: Any, event: Event, arguments: list[str]) -> str:
    """Link two accounts as one person (`/link`, `/link <code>`, or owner `/link <a> <b>`)."""
    conn, now = kernel.conn, kernel.clock.now()
    caller = await canonical(conn, str(event.origin_actor or event.who or "")) or ""
    if not arguments:
        return await _issue_link_code(conn, caller, event, now)
    if len(arguments) == 1:
        return await _consume_link_code(kernel, event, caller, arguments[0], now)
    if len(arguments) != 2:
        return "usage: /link | /link <code> | /link <account|person|name> <account|person|name>"
    if not is_owner(caller):
        return "/link with two targets belongs to the owner; send /link for a one-time code"
    labels = _role_labels(kernel)
    first = await find_person(conn, arguments[0], labels=labels)
    second = await find_person(conn, arguments[1], labels=labels)
    if first is None or second is None:
        missing = arguments[0] if first is None else arguments[1]
        return f"unknown person or account: {missing}"
    if first == second:
        return f"already the same person: {first}"
    src, dst = await merge_people(
        kernel, first, second, by=caller, now=now, place=event.place,
        body=event.body, caused_by=event.id, trigger=event)
    return (f"linked: {src} merged into {dst}. Remembered history stays together "
            "(unlinking later only starts a new id from then on).")


async def unlink_command(kernel: Any, event: Event, arguments: list[str]) -> str:
    """Give an account a new person id from now on without splitting merged history."""
    conn, now = kernel.conn, kernel.clock.now()
    caller = await canonical(conn, str(event.origin_actor or event.who or "")) or ""
    mapping = await privacy.aliases(conn)
    if arguments:
        account = str(arguments[0]).strip()
    else:
        body_accounts = [acc for acc in await accounts_of(conn, caller)
                         if acc.startswith(f"{event.body}:")]
        if len(body_accounts) != 1:
            return "usage: /unlink <account>"
        account = body_accounts[0]
    if not is_account(account) or account not in mapping:
        return f"unknown account: {account}"
    owner_of_account = await canonical(conn, mapping[account]) or mapping[account]
    if not is_owner(caller) and owner_of_account != caller:
        return f"you may only unlink your own account ({account} belongs elsewhere)"
    remaining = await accounts_of(conn, owner_of_account)
    if owner_of_account == OWNER and len(remaining) <= 1:
        return "cannot unlink the owner's only remaining account"
    old_person, new_person = await unlink_account(
        conn, account, actor=caller, now=now, place=event.place,
        body=event.body, caused_by=event.id)
    return (f"unlinked {account}: new id {new_person} from now on "
            f"(earlier merged memory remains under {old_person})")


def install_commands(kernel: Any) -> None:
    """Register `/link` and `/unlink` (human/owner commands only; never soul tools)."""
    tools = kernel.tools
    for name, description, permission, handler in (
            ("link", "Link accounts into one person (/link, /link <code>, /link <a> <b>).",
             "identity.link", link_command),
            ("unlink", "Unlink an account to a fresh person id from now on: /unlink <account>.",
             "identity.unlink", unlink_command)):
        if tools.command(name) is None:
            tools.add_command(CommandEntry(name, description, permission, handler,
                                           owner="identity"))
