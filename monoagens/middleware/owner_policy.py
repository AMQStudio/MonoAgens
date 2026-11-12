"""The composition root of the middleware: the owner policy, the intake, the tools, the limits.

`install(kernel)` is the only place that decides what is installed and in which order. The
core holds the four injected values it is given here and imports none of this.
"""

from __future__ import annotations

from typing import Any

from monoagens.core.hooks import Middleware
from monoagens.core.identity import SelfTool
from monoagens.middleware import identity, limits, privacy, tools
from monoagens.middleware.contract import compose


def install(kernel: Any) -> None:
    """Give the kernel its registry, its intake, and the owner policy in all three hooks."""
    kernel.tools = tools.ToolRegistry()
    kernel.tools.register(SelfTool(kernel.conn, kernel.clock,
                                   visible=privacy.visible,
                                   memory_visible=privacy.memory_visible))
    privacy.register(kernel.tools)
    kernel.intake = identity.intake(kernel.conn)
    kernel.budget_limit = limits.daily_limit(kernel.settings)
    kernel.after_model_call = limits.enforce(kernel)
    for pipeline in kernel.hooks:
        pipeline.set_policy(policy_for(pipeline.name, "first", kernel),
                            policy_for(pipeline.name, "last", kernel))
        for name, middleware in modules_for(pipeline.name, kernel):
            pipeline.add(name, middleware)


def modules_for(hook: str, kernel: Any) -> tuple[tuple[str, Middleware], ...]:
    """Return the scoped middleware of one hook, in the order it runs."""
    return {"inbound": (("identity-pairing", identity.claim(kernel.conn)),),
            "context": (("identity-cards", identity.cards(kernel.conn)),),
            "action": (("identity-route", identity.route(kernel.conn)),)}.get(hook, ())


def policy_for(hook: str, when: str, kernel: Any) -> Middleware:
    """Return the absolute lines of one hook, run before and after the scoped rules."""
    rules = [identity.identity_line()]
    if hook == "context":
        rules.append(privacy.filter_context(kernel.conn))
    if hook == "action":
        rules.append(privacy.guard(kernel, when=when))
    return compose(*rules)
