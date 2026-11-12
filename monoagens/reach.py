"""Reach: who is present, consent to share, and reaching a person (D45, D46)."""

from __future__ import annotations

from typing import Any

from monoagens.store import read_state
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
