"""Pairing the first owner: a short code shown once at first start."""

from __future__ import annotations

import logging
import secrets
from dataclasses import replace
from datetime import datetime

from monoagens.middleware import Continue
from monoagens.store import append_event, read_state, write_state

LOGGER = logging.getLogger(__name__)
CODE_KEY = "pairing_code"
OWNER_KEY = "owner"
CREATED = "pair.created"
CLAIMED = "pair.claimed"


def new_code(draw=secrets.randbelow) -> str:
    """Return a six-digit code, split for readability (for example 123-456)."""
    digits = "".join(str(draw(10)) for _ in range(6))
    return f"{digits[:3]}-{digits[3:]}"


def _text(event) -> str:
    payload = event.payload or {}
    return str(payload.get("description") or payload.get("text") or "")


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
    LOGGER.info("pairing code: %s — send it from any organ to become the owner", code)
    return code


async def claim(conn, event, *, now: datetime):
    """Make the sender of the pairing code the owner; return the claim event."""
    if event.kind != "percept" or not event.who:
        return None
    if await read_state(conn, OWNER_KEY) is not None:
        return None
    current = await read_state(conn, CODE_KEY) or {}
    code = str(current.get("code") or "")
    if not code or code not in _text(event):
        return None
    async with conn.transaction():
        claimed = await append_event(conn, ts=now, kind=CLAIMED, who=event.who,
                                     place=event.place, body=event.body,
                                     payload={"code_used": True}, caused_by=event.id)
        await write_state(conn, OWNER_KEY, {"who": event.who}, claimed.id)
        await conn.execute("DELETE FROM state WHERE key = %s", (CODE_KEY,))
    LOGGER.info("owner paired: %s", event.who)
    return claimed


async def as_owner(conn, event):
    """Rewrite the paired owner's native id to `person:owner` at intake."""
    owner = await read_state(conn, OWNER_KEY) or {}
    if event.who and event.who == owner.get("who"):
        return replace(event, who="person:owner")
    return event


async def inbound(item, ctx):
    """Inbound hook: claim the owner when a percept carries the pairing code."""
    await claim(ctx["conn"], item, now=ctx["now"])
    return Continue()
