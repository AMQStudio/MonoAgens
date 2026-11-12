"""Interaction: approvals asked and answered, and what the soul shows besides its messages.

The core records everything needed (`job.started`, `think`, `act.out`, `act.result`); this
module decides what is shown, to whom and how much (middleware.md §9). An approval is an
ordinary event pair: `approval.requested` asks, `approval.answered` answers, and a `yes`
wakes the reasoning in the request's own place with an `intent` that carries the approval.
"""

from __future__ import annotations

import json
from typing import Any

from monoagens.core.hooks import Continue, Middleware
from monoagens.infra.store import append_event
from monoagens.middleware import identity
from monoagens.types import Event

DEFAULTS = {"thinking": "owner_direct", "steps": "long", "approvals": "owner"}
OWNER = "person:owner"
REQUEST = "approval.requested"
ANSWERED = "approval.answered"
YES = ("yes", "ya")


async def request(kernel: Any, trigger: Event, request_id: int, capability: str,
                  arguments: dict[str, Any], reason: str) -> str | None:
    """Ask the owner to allow one call; return the place the question went to, if any."""
    place = await identity.direct_place(kernel.conn, OWNER)
    if place is None:
        return None
    body = await _body_of(kernel.conn, place)
    adapter = kernel.tools.adapters.get(body)
    text = (f"Approval needed: {capability} "
            f"{json.dumps(arguments, ensure_ascii=False, sort_keys=True)}\n"
            f"Reason: {reason}. Reply yes or no.")
    await kernel._emit(trigger, "act.out", {
        "capability": f"{body}.say" if adapter is not None else "",
        "arguments": {"text": text}, "communicative": True}, place=place)
    if adapter is None:
        return None
    if "buttons" in tuple(getattr(adapter, "shows", ()) or ()):
        await adapter.show(place, "buttons", text=text, buttons=[
            ("Yes", f"approval:yes:{request_id}"), ("No", f"approval:no:{request_id}")])
    else:
        await kernel.tools.act(Event(id=None, ts=trigger.ts, kind="act.out", who=OWNER,
                                     place=place, body=body),
                               f"{body}.say", {"text": text})
    return place


async def _body_of(conn: Any, place: str) -> str:
    """Return the adapter that last spoke in one place."""
    row = await (await conn.execute(
        "SELECT body FROM events WHERE place = %s AND body IS NOT NULL "
        "ORDER BY id DESC LIMIT 1", (place,))).fetchone()
    return str(row[0]) if row else ""


async def pending(conn: Any) -> dict[str, Any] | None:
    """Return the newest approval request nobody answered yet, with its payload."""
    row = await (await conn.execute(
        "SELECT id, payload FROM events WHERE kind = %s AND id NOT IN ("
        "  SELECT (payload->>'request')::bigint FROM events WHERE kind = %s"
        "  AND payload ? 'request') ORDER BY id DESC LIMIT 1", (REQUEST, ANSWERED))).fetchone()
    if row is None:
        return None
    return {"id": int(row[0]), **dict(row[1] or {})}


async def by_id(conn: Any, request_id: int) -> dict[str, Any] | None:
    """Return one approval request by the id of its event."""
    row = await (await conn.execute(
        "SELECT payload FROM events WHERE id = %s AND kind = %s", (request_id, REQUEST))
    ).fetchone()
    return dict(row[0] or {}) if row else None


def answer(kernel: Any) -> Middleware:
    """Inbound rule: the owner's yes or no answers the pending approval, once."""

    async def rule(item: Any, ctx: Any) -> Any:
        """Turn one answer into `approval.answered`, and a yes into a retry intent."""
        if getattr(item, "kind", "") != "percept":
            return Continue()
        payload = item.payload or {}
        answerer = str(item.origin_actor or item.who)
        owner = answerer == OWNER
        request_id, allowed = None, False
        if payload.get("approval") is not None:
            if not (owner or kernel.roles.can(answerer, "approval.answer")):
                return Continue()
            request_id, allowed = int(payload["approval"]), bool(payload.get("answer"))
        else:
            text = str(payload.get("description") or "").strip().lower()
            if owner and text in (*YES, "no"):
                found = await pending(kernel.conn)
                if found is None:
                    return Continue()
                request_id, allowed = int(found["id"]), text in YES
        if request_id is None:
            return Continue()
        asked = await by_id(kernel.conn, request_id)
        if asked is None:
            return Continue()
        await append_event(kernel.conn, ts=kernel.clock.now(), kind=ANSWERED,
                           who=asked.get("who"), place=asked.get("place"), body=item.body,
                           caused_by=item.id,
                           payload={"request": request_id, "allowed": allowed,
                                    "who": asked.get("who"), "by": answerer})
        if allowed:
            kernel.enqueue(Event(
                id=None, ts=kernel.clock.now(), kind="intent", who=OWNER,
                place=str(asked.get("place") or item.place), body=item.body,
                payload={
                    "description": f"the owner approved {asked.get('capability')} with these "
                                   f"arguments; carry it out now: "
                                   f"{json.dumps(asked.get('arguments') or {}, ensure_ascii=False, sort_keys=True)}",
                    "capability": asked.get("capability"),
                    "arguments": asked.get("arguments") or {},
                    "intent": request_id, "approved": request_id,
                    "place_kind": asked.get("place_kind")}))
        return Continue()

    return rule
