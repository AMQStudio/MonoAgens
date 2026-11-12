"""The composition root of the middleware: the owner policy, the intake, the tools, the limits.

`install(kernel)` is the only place that decides what is installed and in which order. The
core holds the four injected values it is given here and imports none of this.

The policy is always present and runs first and last in every hook: it carries the two
absolute lines, the daily model budget and quiet hours, and it turns a person's roles and a
tool's facts into pass, ask or refuse.
"""

from __future__ import annotations

from collections.abc import Awaitable, Callable, Mapping
from datetime import datetime, time
from typing import Any
from zoneinfo import ZoneInfo

from monoagens.core.hooks import Middleware
from monoagens.core.identity import SelfTool
from monoagens.infra.store import append_event
from monoagens.middleware import identity, privacy, registry, roles
from monoagens.middleware.contract import compose

IDENTITY_SET = "identity.set"  # the owner's alone: a permission without a base


def install(kernel: Any) -> None:
    """Give the kernel its registry, its intake, and the owner policy in all three hooks."""
    kernel.tools = registry.ToolRegistry()
    kernel.tools.register(SelfTool(kernel.conn, kernel.clock,
                                   visible=privacy.visible,
                                   memory_visible=privacy.memory_visible),
                          permissions={"recall": "self.recall",
                                       "set_identity": IDENTITY_SET})
    privacy.register(kernel.tools)
    registry.install_own(kernel.tools)
    kernel.roles = roles.RoleBook.empty()
    kernel.intake = identity.intake(kernel.conn)
    kernel.budget_limit = daily_limit(kernel.settings)
    kernel.after_model_call = enforce(kernel)
    for pipeline in kernel.hooks:
        pipeline.set_policy(policy_for(pipeline.name, "first", kernel),
                            policy_for(pipeline.name, "last", kernel))
        for name, middleware in modules_for(pipeline.name, kernel):
            pipeline.add(name, middleware)
    registry.install_commands(kernel)


def modules_for(hook: str, kernel: Any) -> tuple[tuple[str, Middleware], ...]:
    """Return the scoped middleware of one hook, in the order it runs."""
    return {"inbound": (("identity-pairing", identity.claim(kernel.conn)),
                        ("commands", registry.commands(kernel))),
            "context": (("identity-cards", identity.cards(kernel.conn)),
                        ("self-block", registry.self_block(kernel))),
            "action": (("identity-route", identity.route(kernel.conn)),)}.get(hook, ())


def policy_for(hook: str, when: str, kernel: Any) -> Middleware:
    """Return the absolute lines of one hook, run before and after the scoped rules."""
    rules = [identity.identity_line()]
    if hook == "context":
        rules.append(privacy.filter_context(kernel.conn))
    if hook == "action":
        rules.append(privacy.guard(kernel, when=when))
    return compose(*rules)


def daily_limit(settings: Mapping[str, Any]) -> float:
    """Return the owner's daily token budget (0 or less means no cap)."""
    owner = settings.get("owner_settings") or {}
    return float(owner.get("daily_token_budget") or 0)


def enforce(kernel: Any) -> Callable[[int, Mapping[str, int]], Awaitable[None]]:
    """Return the watcher that writes `budget.exhausted` once per local day."""

    async def watch(call_id: int, totals: Mapping[str, int]) -> None:
        """Record the day the budget ran out, once, caused by the call that spent it."""
        limit = kernel.budget_limit
        used = int(totals.get("total_tokens") or 0)
        if limit <= 0 or used < limit:
            return
        now = kernel.clock.now()
        local = now.astimezone(ZoneInfo(str(kernel.settings.get("timezone") or "UTC")))
        midnight = datetime.combine(local.date(), time.min, tzinfo=local.tzinfo)
        cur = await kernel.conn.execute("SELECT 1 FROM events WHERE kind = 'budget.exhausted' "
                                        "AND ts >= %s LIMIT 1", (midnight,))
        if await cur.fetchone() is None:
            await append_event(kernel.conn, ts=now, kind="budget.exhausted", caused_by=call_id,
                               payload={"date": local.date().isoformat(), "used": used,
                                        "budget": limit})

    return watch
