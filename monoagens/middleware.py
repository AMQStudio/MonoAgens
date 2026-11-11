"""The middleware contract: outcomes, owner policy and registration.

The three pipelines, their ordered slots and the watchdog live in `hooks.py`.
"""

from __future__ import annotations

from dataclasses import dataclass, replace
from typing import Any, Protocol

SLOTS = ("first", "middle", "last")
OWNER_POLICY = "policy"
OWNER = "person:owner"


def owner_policy(hook: str) -> Middleware:
    """Return the absolute-line owner policy for one hook."""

    async def policy(item: Any, ctx: Any) -> HookOutcome:
        """Enforce the identity and privacy lines deterministically."""
        if hook == "action" and ctx.get("identity_changing") \
                and ctx.get("origin_actor") != OWNER:
            return Stop("only the owner may invoke identity_changing capabilities")
        if hook == "context":
            keep = tuple(m for m in item.memories if not getattr(m, "private", False)
                         or m.place == item.place or (item.place_kind == "direct"
                         and getattr(item.trigger, "who", None) == OWNER))
            return Replace(replace(item, memories=keep))
        return Continue()

    return policy


@dataclass(frozen=True, slots=True)
class Continue:
    """Let the item through unchanged."""

    pass


@dataclass(frozen=True, slots=True)
class Replace:
    """Let a different item through."""

    item: Any


@dataclass(frozen=True, slots=True)
class Stop:
    """Stop the item here and record why."""

    reason: str


HookOutcome = Continue | Replace | Stop


class Middleware(Protocol):
    """One middleware: inspect an item in a context and return an outcome."""

    async def __call__(self, item: Any, ctx: Any) -> HookOutcome:
        """Return what should happen to this item."""
        ...


@dataclass(frozen=True, slots=True)
class Registration:
    """One middleware with its name, slot and scope (`organ=None` means shared)."""

    name: str
    middleware: Middleware
    slot: str = "middle"
    organ: str | None = None

