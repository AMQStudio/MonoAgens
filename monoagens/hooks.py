"""The three hooks: owner policy first and last, scoped slots, and the watchdog.

Every middleware call runs under `hook_timeout_ms` (docs/core.md §3, D42): a hook
that exceeds it counts as Stop and writes `error`; the loop goes on.
"""

from __future__ import annotations

import asyncio
from collections.abc import Awaitable, Callable, Iterator
from dataclasses import replace
from typing import Any

from monoagens import reach
from monoagens.middleware import (OWNER_POLICY, SLOTS, Continue, HookOutcome, Middleware,
                                  Registration, Replace, Stop, owner_policy)

DEFAULT_HOOK_TIMEOUT_MS = 500


def audience_policy() -> Middleware:
    """Absolute line 2 (D45): private content enters only an allowed audience."""

    async def policy(item: Any, ctx: Any) -> HookOutcome:
        """Keep only the memories and lines this context's audience may see."""
        conn, place = ctx["conn"], item.place

        async def allowed(source: str | None, before: int | None = None) -> bool:
            return await reach.visible(conn, context=place, source=source, before=before)

        memories = [memory for memory in item.memories
                    if not getattr(memory, "private", False)
                    or await allowed(getattr(memory, "place", None))]
        lines = [line for line in item.recent_elsewhere
                 if await allowed(getattr(line, "place", None), getattr(line, "id", None))]
        recent = [line for line in item.recent
                  if await allowed(getattr(line, "place", None), getattr(line, "id", None))]
        return Replace(replace(item, memories=tuple(memories), recent_elsewhere=tuple(lines),
                               recent=tuple(recent)))

    return policy


def policy_for(hook: str) -> Middleware:
    """Return the core policy of one hook: the audience rule for context, else owner policy."""
    return audience_policy() if hook == "context" else owner_policy(hook)


class HookPipeline:
    """One hook: owner policy first and last, scoped middleware in between."""

    def __init__(self, name: str, timeout: float = DEFAULT_HOOK_TIMEOUT_MS / 1000) -> None:
        """Create an empty pipeline; the name is one of the three core hooks."""
        if name not in ("inbound", "context", "action"):
            raise ValueError(f"unknown hook: {name!r}")
        self.name = name
        self.timeout = timeout
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
        return tuple(reg.middleware for reg in ordered if reg.organ in (None, organ))

    async def run(self, item: Any, ctx: Any, *, organ: str | None = None,
                  on_stop: Callable[[str], Awaitable[None]],
                  on_error: Callable[[Exception], Awaitable[None]]) -> Any | None:
        """Run the chain in order under the watchdog; None means the item stopped."""
        scoped = list(self.middlewares(organ))
        chain = [self.policy, *scoped, self.policy] if self.policy is not None else scoped
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
