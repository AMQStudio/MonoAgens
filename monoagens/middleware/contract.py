"""The middleware contract: the core's hook outcomes, composition, and the action context.

The core defines `Continue`, `Replace`, `Stop`, `Middleware` and `Registration` in
`monoagens/core/hooks.py`; the middleware speaks them and composes its rules from them.

The action hook carries one act in a context dict. The keys the core sets are fixed:

| ctx key | set by | meaning for the core |
|---|---|---|
| `conn`, `event`, `now` | core | the connection, the trigger event, the clock |
| `capability`, `descriptor` | core | the capability name and its descriptor |
| `origin_actor` | core | who the chain started with |
| `arguments` | core | the raw arguments the model wrote (may include `to`/`via`) |
| `to`, `via` | core | already split out of `arguments`; `None` when absent |
| `long_running`, `identity_changing` | core | from the descriptor |
| `place` | middleware | the place resolved for this act; `None` means the current place |
| `refusal` | middleware | a reason: the core records it as this act's `act.result` error |
| `handled` | middleware | the middleware performed the act itself; the core records nothing |
| `labels` | middleware | neutral values the core stores and passes on, unread |
"""

from __future__ import annotations

from collections.abc import Mapping, Sequence
from typing import Any

from monoagens.types import Event

from monoagens.core.hooks import (OWNER_POLICY, SLOTS, Continue, HookOutcome, Middleware,
                                  Registration, Replace, Stop)

# What a tool states about itself (middleware.md §4). A tool without facts fails at boot.
FACTS = ("reads", "changes", "runs_code", "network")
CALL_FACTS = FACTS + ("sensitive",)  # `sensitive` is added to one call, never declared

# What an adapter can show besides a message (middleware.md §9).
SHOWS = ("typing", "edit", "buttons", "reactions")

# The one marker for a cut tool output; a tool never invents a second one.
OUTPUT_MARKER = (
    "\n...[truncated: output exceeds {limit} chars; "
    "full in {ref}; continue from character {n}]"
)

__all__ = ["CALL_FACTS", "FACTS", "OUTPUT_MARKER", "OWNER_POLICY", "SHOWS", "SLOTS",
           "AttachmentDoor", "LogView", "attachment_origins", "describe_attachments",
           "find_attachment", "open_attachment",
           "Continue", "HookOutcome", "Middleware", "Registration", "Replace", "Stop",
           "compose"]


def compose(*rules: Middleware) -> Middleware:
    """Thread several rules into one: a Stop ends them, a Replace hands on the new item."""

    async def middleware(item: Any, ctx: Any) -> HookOutcome:
        changed = False
        for rule in rules:
            outcome = await rule(item, ctx)
            if isinstance(outcome, Stop):
                return outcome
            if isinstance(outcome, Replace):
                item, changed = outcome.item, True
        return Replace(item) if changed else Continue()

    return middleware


EVENT_COLUMNS = ("id, ts, kind, who, place, body, payload, caused_by, origin_actor, "
                 "schema_version")


class LogView:
    """A read-only view of the Log and the state board for plugins (D43).

    It has no write method and never hands out the connection, so a plugin can read but
    never change the Log. The view returns raw content: **any plugin tool that shows a
    view line or event to people must first pass it through** `privacy.visible(conn,
    context=place, source=line.place, before=line.id)` (and `privacy.memory_visible` for
    memories); a filtered-out line is replaced by the withheld count, never shown.
    """

    def __init__(self, conn: Any = None) -> None:
        """Hold the connection privately; the loader binds it when the kernel is up."""
        self._conn = conn

    def _bind(self, conn: Any) -> None:
        """Bind the connection once the kernel exists (called by the loader, never a plugin)."""
        self._conn = conn

    def _connection(self) -> Any:
        """Return the bound connection, or refuse when the view was never bound."""
        if self._conn is None:
            raise RuntimeError("the log view is not bound to a kernel yet")
        return self._conn

    async def recent(self, place: str | None = None, *, limit: int = 20,
                     kinds: Sequence[str] | None = None) -> list[Event]:
        """Return the newest events of one place (or of every place) in time order."""
        query = f"SELECT {EVENT_COLUMNS} FROM events"
        where, params = [], []
        if place is not None:
            where.append("place = %s")
            params.append(place)
        if kinds:
            where.append("kind = ANY(%s)")
            params.append(list(kinds))
        if where:
            query += " WHERE " + " AND ".join(where)
        query += " ORDER BY id DESC LIMIT %s"
        return await self._rows(query, (*params, int(limit)), newest_first=True)

    async def by_kind(self, kind: str, *, after_id: int = 0, limit: int = 100) -> list[Event]:
        """Return the events of one kind after one id, oldest first."""
        return await self._rows(
            f"SELECT {EVENT_COLUMNS} FROM events "
            "WHERE kind = %s AND id > %s ORDER BY id LIMIT %s",
            (kind, int(after_id), int(limit)))

    async def state(self, key: str) -> dict[str, Any] | None:
        """Return one state value, or None when the key is absent."""
        row = await (await self._connection().execute(
            "SELECT value FROM state WHERE key = %s", (key,))).fetchone()
        return dict(row[0] or {}) if row else None

    async def state_prefix(self, prefix: str) -> dict[str, dict[str, Any]]:
        """Return every state value whose key begins with `prefix`."""
        cur = await self._connection().execute(
            "SELECT key, value FROM state WHERE key LIKE %s ORDER BY key", (prefix + "%",))
        return {str(key): dict(value or {}) for key, value in await cur.fetchall()}

    async def search(self, text: str, *, limit: int = 20) -> list[Event]:
        """Return the newest events whose readable text contains this text (case-blind)."""
        pattern = f"%{text}%"
        return await self._rows(
            f"SELECT {EVENT_COLUMNS} FROM events "
            "WHERE payload->>'description' ILIKE %s OR payload->>'text' ILIKE %s "
            "ORDER BY id DESC LIMIT %s", (pattern, pattern, int(limit)), newest_first=True)

    async def _rows(self, query: str, params: Any, *, newest_first: bool = False
                    ) -> list[Event]:
        """Read one query into events (the log's columns only), oldest first."""
        from monoagens.infra.store import event_from_row  # local: store imports nothing here

        cur = await self._connection().execute(query, params)
        rows = await cur.fetchall()
        found = [event_from_row(row) for row in rows]
        return found if not newest_first else list(reversed(found))


# ------------------------------------------------------------ attachments


def describe_attachments(attachments: Sequence[Mapping[str, Any]], *,
                         sent_by: str = "someone") -> str:
    """Render the one line reasoning sees: `Budi sent 3 files: report.pdf (att_ab12…)`.

    The reference is part of the line because it is the handle the soul reads the file
    with (`workspace.read(ref=…)`); a name alone would leave the file unreadable.
    """

    def label(item: Mapping[str, Any]) -> str:
        name = str(item.get("name") or f"{int(item.get('size') or 0)} bytes")
        ref = str(item.get("ref") or "")
        return f"{name} ({ref})" if ref else name

    found = list(attachments or ())
    names = [label(item) for item in found]
    if not names:
        return f"{sent_by} sent no file"
    plural = "" if len(names) == 1 else "s"
    return f"{sent_by} sent {len(names)} file{plural}: " + ", ".join(names)


class AttachmentDoor:
    """The one door adapters and plugins use: `put` a file, `open` one they may see."""

    def __init__(self, store: Any, conn: Any) -> None:
        """Keep the store and the connection the audience check runs on."""
        self.store = store
        self._conn = conn

    async def put(self, data: bytes, *, name: str,
                  mime: str = "application/octet-stream") -> dict[str, Any]:
        """Store one received file and return its attachment."""
        return await self.store.put(data, name=name, mime=mime)

    async def open(self, ref: str, *, context: str | None) -> tuple[bytes, dict[str, Any]]:
        """Return one file's bytes when this context may see where it came from."""
        return await open_attachment(self._conn, self.store, ref, context=context)


async def attachment_origins(conn: Any, ref: str) -> list[tuple[int, str]]:
    """Return every non-forgotten event that carried or archived one attachment, oldest first."""
    cur = await conn.execute(
        "SELECT id, place FROM events WHERE ("
        "payload @> jsonb_build_object('data', jsonb_build_object("
        "'attachments', jsonb_build_array(jsonb_build_object('ref', %s::text)))) "
        "OR payload @> jsonb_build_object('result', jsonb_build_object("
        "'full', jsonb_build_object('ref', %s::text)))"
        ") AND payload <> '{\"forgotten\": true}'::jsonb ORDER BY id", (ref, ref))
    return [(int(found), str(place or "")) for found, place in await cur.fetchall()]


async def find_attachment(conn: Any, ref: str, *, context: str | None
                          ) -> tuple[int, str] | None:
    """Return the first event carrying one attachment whose place this context may see."""
    from monoagens.middleware import privacy  # local: privacy reads the Log, not this module

    for event_id, place in await attachment_origins(conn, ref):
        if await privacy.visible(conn, context=context, source=place, before=event_id):
            return event_id, place
    return None


async def open_attachment(conn: Any, store: Any, ref: str, *, context: str | None
                          ) -> tuple[bytes, dict[str, Any]]:
    """Return one file and its name when the requester's context may see where it came from."""
    if await find_attachment(conn, ref, context=context) is None:
        raise PermissionError("that file came with content this place may not see")
    found = store.find(ref)
    if found is None:
        raise FileNotFoundError(f"no stored file for {ref}")
    return await store.read(ref), {"ref": ref, "name": found.name,
                                   "size": found.stat().st_size}
