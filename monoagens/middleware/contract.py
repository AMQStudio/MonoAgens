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

import logging
from collections.abc import Mapping, Sequence
from typing import Any

from monoagens.infra.store import EVENT_COLUMNS as _EVENT_COLUMNS
from monoagens.types import Event

from monoagens.contract.hooks import (OWNER_POLICY, RULE_KINDS, SLOTS, Continue, HookOutcome,
                                      Middleware, Registration, Replace, Stop)

LOGGER = logging.getLogger(__name__)

# What a tool states about itself (middleware.md §4). A tool without facts fails at boot.
FACTS = ("reads", "changes", "runs_code", "network")
CALL_FACTS = FACTS + ("sensitive",)  # `sensitive` is added to one call, never declared

# What an adapter can show besides a message (adapters.md §4, middleware.md §9).
SHOWS = (
    "typing", "read_receipts", "edit", "delete", "reply", "reactions",
    "buttons", "menus", "polls", "stickers", "embeds", "threads", "locations",
)

# What an adapter can read from incoming platform events (adapters.md §4).
READS = (
    "text", "reply", "mention", "edit", "delete", "reaction", "sticker",
    "location", "contact", "poll", "view_once", "files", "call",
    "place_event", "voice_note", "choices_answer",
)


def check_coverage(name: str, adapter: Any) -> tuple[str, ...]:
    """Validate an adapter's declared reads, shows and gaps against D67 vocabulary.

    Returns coverage lines suitable for the boot report. Adapters that have not yet
    declared `reads` or `gaps` only have their `shows` checked.
    """
    shows = tuple(getattr(adapter, "shows", ()) or ())
    unknown_shows = [kind for kind in shows if kind not in SHOWS]
    if unknown_shows:
        raise ValueError(f"adapter {name}: cannot show {unknown_shows[0]!r}")

    has_reads = hasattr(adapter, "reads")
    has_gaps = hasattr(adapter, "gaps")
    if not (has_reads or has_gaps):
        return ()

    reads = tuple(getattr(adapter, "reads", ()) or ())
    unknown_reads = [kind for kind in reads if kind not in READS]
    if unknown_reads:
        raise ValueError(f"adapter {name}: unknown read kind {unknown_reads[0]!r}")

    gaps = getattr(adapter, "gaps", None)
    if not isinstance(gaps, Mapping):
        raise ValueError(f"adapter {name}: missing coverage gaps map")

    expected_gaps = [k for k in READS if k not in reads] + [k for k in SHOWS if k not in shows]
    if getattr(adapter, "history", None) is None and "history" not in expected_gaps:
        expected_gaps.append("history")
    missing = [k for k in expected_gaps if k not in gaps or not str(gaps[k] or "").strip()]
    if missing:
        raise ValueError(f"adapter {name}: undeclared coverage gap {missing[0]!r}")

    lines = [
        f"adapter {name} coverage: reads {', '.join(reads) or 'none'}",
        f"adapter {name} coverage: shows {', '.join(shows) or 'none'}",
    ]
    for kind in sorted(gaps):
        lines.append(f"adapter {name} gap {kind}: {gaps[kind]}")
    lines.extend(adapter_fallbacks(name, adapter))
    return tuple(lines)


# D70/D81 fallback chains (adapters.md §4, middleware.md §9).
FALLBACKS: dict[str, tuple[tuple[str | None, str], ...]] = {
    "approval": (("buttons", "buttons"), ("menus", "menus"), ("reactions", "reactions"),
                 ("polls", "poll"), (None, "numbered list")),
    "choices": (("buttons", "buttons"), ("menus", "menus"), ("reactions", "reactions"),
                ("polls", "poll"), (None, "numbered list")),
    "thread": (("threads", "new thread"), (None, "reply with the title")),
    "location": (("locations", "native"), (None, "text")),
    "sticker": (("stickers", "sticker"), (None, "emoji")),
    "progress": (("edit", "edit"), (None, "new message")),
}


def fallback_choice(feature: str, shows: Sequence[str] | set[str]) -> tuple[str | None, str]:
    """Return `(required_show_cap, label)` for `feature`, never picking an unshown capability."""
    show_set = set(shows)
    for cap, label in FALLBACKS[feature]:
        if cap is None or cap in show_set:
            if cap is not None and cap not in SHOWS:
                raise ValueError(f"unknown fallback capability {cap!r} for {feature}")
            return cap, label
    raise ValueError(f"fallback chain for {feature!r} has no plain-text terminal step")


def adapter_fallbacks(name: str, adapter: Any) -> tuple[str, ...]:
    """Return the fallback choice per D70 chain for `adapter` so the boot report shows it."""
    shows = set(getattr(adapter, "shows", ()) or ())
    lines: list[str] = []
    for feature, chain in FALLBACKS.items():
        cap, chosen = fallback_choice(feature, shows)
        if cap is not None and cap not in shows:
            raise ValueError(f"adapter {name} fallback {feature} chose unshown {cap!r}")
        top = chain[0][1]
        note = chosen if chosen == top else f"{chosen} (fallback from {top})"
        lines.append(f"adapter {name} fallback {feature}: {note}")
    return tuple(lines)

# The one marker for a cut tool output; a tool never invents a second one.
OUTPUT_MARKER = (
    "\n...[truncated: output exceeds {limit} chars; "
    "full in {ref}; continue from character {n}]"
)

__all__ = ["CALL_FACTS", "FACTS", "FALLBACKS", "OUTPUT_MARKER", "OWNER_POLICY", "READS",
           "RULE_KINDS", "SHOWS", "SLOTS", "AttachmentDoor", "LogView", "adapter_fallbacks",
           "attachment_origins", "best_effort", "check_coverage", "find_attachment",
           "mark_rule", "open_attachment", "Continue", "HookOutcome", "Middleware",
           "Registration", "Replace", "Stop", "compose"]


def mark_rule(rule: Middleware, *, name: str, kind: str = "guard",
              show: str = "") -> Middleware:
    """Attach rule kind ('guard' default, or 'best-effort') and show metadata to a rule."""
    if kind not in RULE_KINDS:
        raise ValueError(f"unknown rule kind: {kind!r}")
    setattr(rule, "rule_kind", kind)
    setattr(rule, "rule_name", name)
    if show:
        setattr(rule, "show_kind", show)
    return rule


def best_effort(rule: Middleware, *, kernel: Any = None, name: str = "",
                show: str = "") -> Middleware:
    """Wrap one best-effort rule: failures record `error` and the hook continues."""
    rule_name = name or str(getattr(rule, "rule_name", "") or getattr(rule, "__name__", "rule"))
    show_kind = show or str(getattr(rule, "show_kind", "") or "")

    async def wrapped(item: Any, ctx: Any) -> HookOutcome:
        try:
            return await rule(item, ctx)
        except Exception as exc:
            event = ctx.get("event") if isinstance(ctx, Mapping) else None
            if event is None:
                event = item
            labels = ctx.get("labels") if isinstance(ctx, Mapping) else None
            adapter = str((labels or {}).get("body") or getattr(event, "body", "") or "-")
            detail = (ctx.get("show_kind") if isinstance(ctx, Mapping) else None) or show_kind
            parts = [f"rule={rule_name}", f"adapter={adapter}"]
            if detail:
                parts.append(f"show={detail}")
            message = f"best-effort rule failed ({', '.join(parts)}): {exc}"
            rec = getattr(kernel, "_record_error", None)
            if rec is not None and isinstance(event, Event):
                await rec(event, message)
            else:
                LOGGER.warning("%s", message)
            return Continue()

    return mark_rule(wrapped, name=rule_name, kind="best-effort", show=show_kind)


def compose(*rules: Middleware, kernel: Any = None) -> Middleware:
    """Thread several rules into one: best-effort rules continue on error; guards fail closed."""
    wrapped_rules = tuple(
        best_effort(rule, kernel=kernel)
        if getattr(rule, "rule_kind", "guard") == "best-effort"
        else rule
        for rule in rules
    )

    async def middleware(item: Any, ctx: Any) -> HookOutcome:
        changed = False
        for rule in wrapped_rules:
            outcome = await rule(item, ctx)
            if isinstance(outcome, Stop):
                return outcome
            if isinstance(outcome, Replace):
                item, changed = outcome.item, True
        return Replace(item) if changed else Continue()

    return middleware


EVENT_COLUMNS = ", ".join(_EVENT_COLUMNS)


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

    async def put_deferred(self, *, key: str, name: str,
                           mime: str = "application/octet-stream", size: int = 0,
                           body: str = "", locator: Mapping[str, Any] | None = None
                           ) -> dict[str, Any]:
        """Record one deferred attachment reference without downloading its bytes yet."""
        return await self.store.put_deferred(key=key, name=name, mime=mime, size=size,
                                             body=body, locator=locator)

    def register_fetcher(self, body: str, fn: Any) -> None:
        """Register the adapter callback that materializes deferred refs for `body`."""
        self.store.register_fetcher(body, fn)

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
        meta = await store.read_deferred(ref) if hasattr(store, "read_deferred") else None
        if meta is not None:
            fetch = store.fetcher(str(meta.get("body") or "")) if hasattr(store, "fetcher") else None
            if fetch is None:
                raise FileNotFoundError(f"no fetcher registered for deferred file {ref}")
            data = await fetch(ref, meta)
            target = await store.materialize(ref, data, name=str(meta.get("name") or "attachment"))
            return data, {"ref": ref, "name": target.name, "size": len(data)}
        raise FileNotFoundError(f"no stored file for {ref}")
    return await store.read(ref), {"ref": ref, "name": found.name,
                                   "size": found.stat().st_size}
