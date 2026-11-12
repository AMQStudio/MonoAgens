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

from collections.abc import Sequence
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
OUTPUT_MARKER = "\n...[truncated: output exceeds {limit} chars; next: continue from character {n}]"

__all__ = ["CALL_FACTS", "FACTS", "OUTPUT_MARKER", "OWNER_POLICY", "SHOWS", "SLOTS", "LogView",
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
