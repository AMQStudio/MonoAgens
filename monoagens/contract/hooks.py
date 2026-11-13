"""The three hooks: the outcome vocabulary, the slots and one registration (contract.md §3)."""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any, Protocol

SLOTS = ("first", "middle", "last")
OWNER_POLICY = "policy"
RULE_KINDS = ("guard", "best-effort")


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
    """Stop the item here and record why.

    `held=True` (the action hook only) means the action waits for someone or something,
    an approval or the end of quiet hours, instead of being refused.
    """

    reason: str
    held: bool = False


HookOutcome = Continue | Replace | Stop


class Middleware(Protocol):
    """One middleware: inspect an item in a context and return an outcome."""

    async def __call__(self, item: Any, ctx: Any) -> HookOutcome:
        """Return the outcome for one item."""
        ...


@dataclass(frozen=True, slots=True)
class Registration:
    """One middleware with its name, slot and scope (`tool=None` means shared)."""

    name: str
    rule: Middleware
    slot: str = "middle"
    tool: str | None = None
