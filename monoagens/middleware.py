"""The middleware contract: the core's three hooks and their ordered slots."""

from __future__ import annotations

from collections.abc import Awaitable, Callable, Iterator
from dataclasses import dataclass
from typing import Any, Protocol

SLOTS = ("first", "middle", "last")
OWNER_POLICY = "policy"


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


class HookPipeline:
    """One hook: owner policy first and last, scoped middleware in between."""

    def __init__(self, name: str) -> None:
        """Create an empty pipeline; the name is one of the three core hooks."""
        if name not in ("inbound", "context", "action"):
            raise ValueError(f"unknown hook: {name!r}")
        self.name = name
        self.policy: Middleware | None = None
        self._registrations: list[Registration] = []
        self._names: set[str] = {OWNER_POLICY}

    def set_policy(self, middleware: Middleware) -> None:
        """Install the owner policy, which runs first and last and cannot be removed."""
        if self.policy is not None:
            raise ValueError(f"{self.name}: owner policy already set")
        self.policy = middleware

    def add(self, name: str, middleware: Middleware, *, slot: str = "middle",
            organ: str | None = None) -> None:
        """Register a middleware; a duplicate name or an unknown slot fails at boot."""
        if not name:
            raise ValueError(f"{self.name}: a middleware needs a name")
        if name in self._names:
            raise ValueError(f"{self.name}: name collision: {name!r}")
        if slot not in SLOTS:
            raise ValueError(f"{self.name}: unknown slot: {slot!r}")
        self._registrations.append(Registration(name, middleware, slot, organ))
        self._names.add(name)

    def middlewares(self, organ: str | None = None) -> tuple[Middleware, ...]:
        """Return the shared and matching organ-own middleware in slot order."""
        ordered = sorted(self._registrations, key=lambda item: SLOTS.index(item.slot))
        return tuple(item.middleware for item in ordered
                     if item.organ is None or item.organ == organ)

    async def run(self, item: Any, ctx: Any, *, organ: str | None = None,
                  on_stop: Callable[[str], Awaitable[None]],
                  on_error: Callable[[Exception], Awaitable[None]]) -> Any | None:
        """Run the chain in order; return the final item, or None when it stopped."""
        chain = list(self.middlewares(organ))
        if self.policy is not None:
            chain = [self.policy, *chain, self.policy]
        for middleware in chain:
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


class HookRegistry:
    """The three core hooks, each with its own ordered and scoped slots."""

    def __init__(self) -> None:
        """Create the inbound, context and action pipelines."""
        self.inbound = HookPipeline("inbound")
        self.context = HookPipeline("context")
        self.action = HookPipeline("action")

    def __iter__(self) -> Iterator[HookPipeline]:
        """Iterate the three pipelines in core order."""
        return iter((self.inbound, self.context, self.action))

    def get(self, name: str) -> HookPipeline:
        """Return the pipeline with this hook name."""
        for pipeline in self:
            if pipeline.name == name:
                return pipeline
        raise ValueError(f"unknown hook: {name!r}")
