"""The three hooks: the outcome vocabulary, the owner policy first and last, and the watchdog.

The core defines the vocabulary; the middleware speaks it. The owner policy is installed
from outside and runs first and last in every hook, so no scoped rule can widen it.
Every middleware call runs under `hook_timeout_ms` (docs/core.md §3, D42): a hook
that exceeds it counts as Stop and writes `error`; the loop goes on.
"""

from __future__ import annotations

import asyncio
from collections.abc import Awaitable, Callable, Iterator
from dataclasses import dataclass
from typing import Any, Protocol

DEFAULT_HOOK_TIMEOUT_MS = 500
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
    """One middleware with its name, slot and scope (`tool=None` means shared)."""

    name: str
    middleware: Middleware
    slot: str = "middle"
    tool: str | None = None


class HookPipeline:
    """One hook: owner policy first and last, scoped middleware in between."""

    def __init__(self, name: str, timeout: float = DEFAULT_HOOK_TIMEOUT_MS / 1000) -> None:
        """Create an empty pipeline; the name is one of the three core hooks."""
        if name not in ("inbound", "context", "action"):
            raise ValueError(f"unknown hook: {name!r}")
        self.name = name
        self.timeout = timeout
        self.first_policy: Middleware | None = None
        self.last_policy: Middleware | None = None
        self._registrations: list[Registration] = []
        self._names: set[str] = {OWNER_POLICY}

    def set_policy(self, first: Middleware, last: Middleware | None = None) -> None:
        """Install the owner policy: first and last, and never removable."""
        if self.first_policy is not None or self.last_policy is not None:
            raise ValueError(f"{self.name}: owner policy already set")
        self.first_policy, self.last_policy = first, last if last is not None else first

    def add(self, name: str, middleware: Middleware, *, slot: str = "middle",
            tool: str | None = None) -> None:
        """Register a middleware; a duplicate name or an unknown slot fails at boot."""
        if not name:
            raise ValueError(f"{self.name}: a middleware needs a name")
        if name in self._names:
            raise ValueError(f"{self.name}: name collision: {name!r}")
        if slot not in SLOTS:
            raise ValueError(f"{self.name}: unknown slot: {slot!r}")
        self._registrations.append(Registration(name, middleware, slot, tool))
        self._names.add(name)

    def middlewares(self, tool: str | None = None) -> tuple[Middleware, ...]:
        """Return the shared and the matching tool-own middleware in slot order."""
        ordered = sorted(self._registrations, key=lambda item: SLOTS.index(item.slot))
        return tuple(reg.middleware for reg in ordered if reg.tool in (None, tool))

    async def run(self, item: Any, ctx: Any, *, tool: str | None = None,
                  on_stop: Callable[[str], Awaitable[None]],
                  on_error: Callable[[Exception], Awaitable[None]]) -> Any | None:
        """Run the chain in order under the watchdog; None means the item stopped."""
        scoped = list(self.middlewares(tool))
        chain = (scoped if self.first_policy is None
                 else [self.first_policy, *scoped, self.last_policy])
        for middleware in chain:
            try:
                outcome = await asyncio.wait_for(middleware(item, ctx), self.timeout)
            except asyncio.TimeoutError:
                await on_error(TimeoutError(
                    f"{self.name} hook timed out after {self.timeout:g}s"))
                return None
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

    def __init__(self, settings: Any = None) -> None:
        """Create the three pipelines with the hook timeout from the prior."""
        values = (settings or {}).get("constants") or {}
        timeout = float(values.get("hook_timeout_ms") or DEFAULT_HOOK_TIMEOUT_MS) / 1000
        self.inbound = HookPipeline("inbound", timeout)
        self.context = HookPipeline("context", timeout)
        self.action = HookPipeline("action", timeout)

    def __iter__(self) -> Iterator[HookPipeline]:
        """Iterate the three pipelines in core order."""
        return iter((self.inbound, self.context, self.action))

    def get(self, name: str) -> HookPipeline:
        """Return the pipeline with this hook name."""
        for pipeline in self:
            if pipeline.name == name:
                return pipeline
        raise ValueError(f"unknown hook: {name!r}")

    def require_policy(self) -> None:
        """Fail unless the owner policy is installed in all three hooks (fail-closed)."""
        for pipeline in self:
            if pipeline.first_policy is None or pipeline.last_policy is None:
                raise RuntimeError(
                    f"the owner policy is not installed in the {pipeline.name} hook")
