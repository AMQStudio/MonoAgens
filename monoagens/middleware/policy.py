"""The composition root of the middleware: the owner policy, the intake, the tools, the limits.

`install(kernel)` is the only place that decides what is installed and in which order. The
core holds the four injected values it is given here and imports none of this.

The policy is always present and runs first and last in every hook: it carries the two
absolute lines, the daily model budget and quiet hours, and it turns a person's roles and a
tool's facts into pass, ask or refuse.
"""

from __future__ import annotations

import ipaddress
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

# The addresses nothing outside may reach: private, loopback, link-local, the container
# ranges and the metadata service. One table for `web_fetch` and the sandbox (plugins.md §5).
BLOCKED_NETWORKS = tuple(ipaddress.ip_network(value) for value in (
    "10.0.0.0/8", "172.16.0.0/12", "192.168.0.0/16", "100.64.0.0/10", "127.0.0.0/8",
    "169.254.0.0/16", "0.0.0.0/8", "::1/128", "fc00::/7", "fe80::/10"))


def is_blocked_address(address: str) -> bool:
    """Return whether one address is private, reserved or otherwise off limits (fail closed)."""
    try:
        value = ipaddress.ip_address(str(address).strip("[]").split("%")[0])
    except ValueError:
        return True
    if isinstance(value, ipaddress.IPv6Address) and value.ipv4_mapped is not None:
        value = value.ipv4_mapped
    if (value.is_private or value.is_loopback or value.is_link_local or value.is_multicast
            or value.is_reserved or value.is_unspecified):
        return True
    return any(network.version == value.version and value in network
               for network in BLOCKED_NETWORKS)


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
