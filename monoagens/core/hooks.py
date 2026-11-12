"""The three hooks: the outcome vocabulary, the owner policy first and last, and the watchdog.

Every middleware call runs under `hook_timeout_ms` (docs/core.md §3, D42): a hook
that exceeds it counts as Stop and writes `error`; the loop goes on.
"""

from __future__ import annotations

import asyncio
from collections.abc import Awaitable, Callable, Iterator
from dataclasses import dataclass, replace
from typing import Any, Protocol

from monoagens.middleware import privacy

DEFAULT_HOOK_TIMEOUT_MS = 500
SLOTS = ("first", "middle", "last")
OWNER_POLICY = "policy"
OWNER = "person:owner"


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


def owner_policy(hook: str) -> Middleware:
    """Return the absolute-line owner policy for one hook."""

    async def policy(item: Any, ctx: Any) -> HookOutcome:
        """Enforce the identity and privacy lines deterministically."""
        if hook == "action" and ctx.get("identity_changing") \
                and ctx.get("origin_actor") != OWNER:
            return Stop("only the owner may invoke identity_changing capabilities")
        return Continue()

    return policy


def audience_policy() -> Middleware:
    """Absolute line 2 (D45): private content enters only an allowed audience."""

    async def policy(item: Any, ctx: Any) -> HookOutcome:
        """Keep only what the audience may see, and note the relevant private items withheld."""
        conn, place = ctx["conn"], item.place
        counts = dict(item.withheld)

        async def keep(lines: Any, *, note_withheld: bool) -> tuple[Any, ...]:
            kept = []
            for line in lines:
                if await privacy.visible(conn, context=place, source=line.place, before=line.id):
                    kept.append(line)
                elif note_withheld:
                    label = await privacy.source_label(conn, line.place, line.id)
                    counts[label] = counts.get(label, 0) + 1
            return tuple(kept)

        memories = []
        for memory in item.memories:
            if await privacy.memory_visible(conn, context=place,
                                            sources=getattr(memory, "sources", ())):
                memories.append(memory)
            else:
                label = await privacy.withheld_source(
                    conn, context=place, sources=getattr(memory, "sources", ()))
                counts[label] = counts.get(label, 0) + 1
        elsewhere = await keep(item.recent_elsewhere, note_withheld=True)
        recent = await keep(item.recent, note_withheld=False)
        return Replace(replace(item, memories=tuple(memories), recent_elsewhere=elsewhere,
                               recent=recent, withheld=tuple(sorted(counts.items()))))

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
        """Return the shared and matching tool-own middleware in slot order."""
        ordered = sorted(self._registrations, key=lambda item: SLOTS.index(item.slot))
        return tuple(reg.middleware for reg in ordered if reg.tool in (None, tool))

    async def run(self, item: Any, ctx: Any, *, tool: str | None = None,
                  on_stop: Callable[[str], Awaitable[None]],
                  on_error: Callable[[Exception], Awaitable[None]]) -> Any | None:
        """Run the chain in order under the watchdog; None means the item stopped."""
        scoped = list(self.middlewares(tool))
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
