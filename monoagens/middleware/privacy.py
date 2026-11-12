"""Privacy: who is present, consent to share, and reaching a person (D45, D46)."""

from __future__ import annotations

import re
from typing import Any

from monoagens.infra.store import payload_text, read_state
from monoagens.types import Event

OWNER = "person:owner"
SHARE = "self.share"
SHARE_CAPABILITY = {
    "name": "share",
    "description": "Share private content with a person or a place; only if you were part of it.",
    "schema": {"type": "object", "properties": {
        "item": {"type": "string"}, "episode": {"type": "integer"},
        "to": {"type": "string", "description": "person:<id> or a place id."}},
        "required": ["to"]},
    "communicative": False, "long_running": False, "identity_changing": False,
}


async def aliases(conn) -> dict[str, str]:
    """Return the alias map that middleware keeps in `state: alias:*`."""
    cur = await conn.execute("SELECT key, value FROM state WHERE key LIKE 'alias:%'")
    return {key[6:]: str((value or {}).get("who") or value) for key, value in await cur.fetchall()}


async def place_kind(conn, place: str | None) -> str:
    """Return the kind the organ reported for a place; unknown counts as shared."""
    if not place:
        return "shared"
    row = await (await conn.execute(
        "SELECT payload->>'place_kind' FROM events WHERE place = %s "
        "AND payload ? 'place_kind' ORDER BY id DESC LIMIT 1", (place,))).fetchone()
    return str(row[0] or "shared") if row else "shared"


async def direct(conn, place: str | None) -> bool:
    """Return whether a place is a 1:1 chat, by the kind its organ reported."""
    return await place_kind(conn, place) == "direct"


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


async def owner_direct(conn, place: str | None) -> bool:
    """Return whether this context is a direct chat with the owner."""
    return await direct(conn, place) and await audience(conn, place) <= frozenset({OWNER})


async def visible(conn, *, context: str | None, source: str | None,
                  before: int | None = None) -> bool:
    """Return whether content from `source` may enter a context at `context` (D45, D50)."""
    if not source or await owner_direct(conn, context):
        return True
    need = set(await audience(conn, context)) - {OWNER}
    if not need or (source == context and before is None):
        return True
    present = set(await audience(conn, source, before))
    consented = set(await shared(conn, source))
    return need <= (present - {OWNER}) | consented


async def memory_visible(conn, *, context: str | None, sources: Any = None) -> bool:
    """Return whether every source of a memory was said to the people here (D53, D50)."""
    if not sources:
        return await owner_direct(conn, context)
    need = set(await audience(conn, context)) - {OWNER}
    rows = await (await conn.execute(
        "SELECT id, place FROM events WHERE id = ANY(%s) AND place IS NOT NULL",
        (list(sources),))).fetchall()
    audiences = []
    for source, place in rows:
        present = set(await audience(conn, place, before=source))
        consented = set(await shared(conn, place))
        audiences.append((present - {OWNER}) | consented)
    return all(need <= have for have in audiences)


async def source_label(conn, place: str | None, before: int | None = None) -> str:
    """Return the D55-safe label of a source place: a private chat's people, or the place."""
    if not place:
        return f"{OWNER} private chat"
    if await direct(conn, place):
        people = sorted(await audience(conn, place, before))
        if people:
            return f"{' and '.join(people)} private chat"
    return f"the chat at {place}"


async def withheld_source(conn, *, context: str | None, sources: Any = None) -> str:
    """Return the label of the source one withheld item came from (D56).

    A memory or core memory names the first source place whose audience this
    context does not cover. A memory without a known source is private to the
    owner, so its label says so.
    """
    need = set(await audience(conn, context)) - {OWNER}
    rows = []
    if sources:
        rows = await (await conn.execute(
            "SELECT id, place FROM events WHERE id = ANY(%s) AND place IS NOT NULL",
            (list(sources),))).fetchall()
    for source, source_place in rows:
        present = set(await audience(conn, source_place, before=source)) - {OWNER}
        consented = set(await shared(conn, source_place))
        if not need <= (present | consented):
            return await source_label(conn, source_place, source)
    return await source_label(conn, None)


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


async def person_cards(conn, place: str | None, event: Event | None) -> tuple[str, ...]:
    """Return one card per person present here or named in the trigger (D55)."""
    present = set(await audience(conn, place)) if place else set()
    known = await _known_people(conn)
    text = payload_text(event.payload) if event is not None else ""
    named = {who for who, card in known.items()
             if card["name"] and _named(card["name"], text)}
    people = sorted((present | named) - {OWNER})
    return tuple([await _card(conn, who, known.get(who) or {"name": None, "closeness": 0.0,
                                                            "natives": {who}})
                  for who in people])


async def _known_people(conn) -> dict[str, dict[str, Any]]:
    """Return the card data of every person the alias map or the Log has seen."""
    mapping = await aliases(conn)
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


async def _card(conn, who: str, card: dict[str, Any]) -> str:
    """Render one person card: name, person id, reachable bodies, closeness."""
    places = await _direct_places(conn, card["natives"])
    reach = "; ".join(f"{body} at {place}" for body, place in sorted(places.items()))
    label = f"{card['name']} ({who})" if card["name"] else who
    return f"- {label}: {reach or 'no direct place known yet'}; closeness {card['closeness']:.2f}"


async def _direct_places(conn, natives: set[str]) -> dict[str, str]:
    """Return the direct place where each body last saw one of a person's ids."""
    if not natives:
        return {}
    cur = await conn.execute(
        "SELECT body, place FROM events WHERE kind = 'percept' AND place IS NOT NULL "
        "AND who = ANY(%s) AND payload->>'place_kind' = 'direct' ORDER BY id DESC",
        (sorted(natives),))
    places: dict[str, str] = {}
    for body, place in await cur.fetchall():
        places.setdefault(str(body or "?"), place)
    return places


async def direct_place(conn, who: str, via: str | None = None) -> str | None:
    """Return the 1:1 place where one person was last active, optionally on one body."""
    mapping = await aliases(conn)
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
    if not str(to).startswith("person:"):
        found = await (await conn.execute(
            "SELECT 1 FROM events WHERE place = %s LIMIT 1", (str(to),))).fetchone()
        return (str(to), None) if found else (None, f"no such place: {to}")
    place = await direct_place(conn, str(to), via)
    if place is None:
        return None, f"no way to reach {str(to).removeprefix('person:')} yet"
    return place, None


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
