"""The middleware contract: four hook points around the kernel's work."""

from __future__ import annotations

from collections.abc import Awaitable, Callable, Sequence
from dataclasses import dataclass
from typing import Any, Protocol


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


async def run_hooks(hooks: Sequence[Middleware], item: Any, ctx: Any, *,
                    on_stop: Callable[[str], Awaitable[None]],
                    on_error: Callable[[Exception], Awaitable[None]]) -> Any | None:
    """Run middleware in order; return the final item, or None when it stopped."""
    for middleware in hooks:
        try:
            outcome = await middleware(item, ctx)
        except Exception as exc:
            await on_error(exc)
            return None
        if isinstance(outcome, Stop):
            await on_stop(outcome.reason)
            return None
        if isinstance(outcome, Replace):
            item = outcome.item
        elif not isinstance(outcome, Continue):
            await on_error(ValueError(f"invalid hook outcome: {outcome!r}"))
            return None
    return item
