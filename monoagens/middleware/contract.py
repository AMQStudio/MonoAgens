"""The middleware contract: the core's hook outcomes, composition, and the action context.

The core defines `Continue`, `Replace`, `Stop`, `Middleware` and `Registration` in
`monoagens/core/hooks.py`; the middleware speaks them and composes its rules from them.

The action hook carries one act in a context dict. The keys the core sets are fixed:

| ctx key | set by | meaning for the core |
|---|---|---|
| `conn`, `event`, `now` | core | the connection, the trigger event, the clock |
| `capability`, `descriptor` | core | the capability name and its descriptor |
| `origin_actor` | core | who the chain started with |
| `arguments` | core | the raw arguments the model wrote (may include `to`/`via`) |
| `to`, `via` | core | already split out of `arguments`; `None` when absent |
| `long_running`, `identity_changing` | core | from the descriptor |
| `place` | middleware | the place resolved for this act; `None` means the current place |
| `refusal` | middleware | a reason: the core records it as this act's `act.result` error |
| `handled` | middleware | the middleware performed the act itself; the core records nothing |
| `labels` | middleware | neutral values the core stores and passes on, unread |
"""

from __future__ import annotations

from typing import Any

from monoagens.core.hooks import (OWNER_POLICY, SLOTS, Continue, HookOutcome, Middleware,
                                  Registration, Replace, Stop)

# What a tool states about itself (middleware.md §4). A tool without facts fails at boot.
FACTS = ("reads", "changes", "runs_code", "network")
CALL_FACTS = FACTS + ("sensitive",)  # `sensitive` is added to one call, never declared

# What an adapter can show besides a message (middleware.md §9).
SHOWS = ("typing", "edit", "buttons", "reactions")

# The one marker for a cut tool output; a tool never invents a second one.
OUTPUT_MARKER = "\n...[truncated: output exceeds {limit} chars; next: continue from character {n}]"

__all__ = ["CALL_FACTS", "FACTS", "OUTPUT_MARKER", "OWNER_POLICY", "SHOWS", "SLOTS",
           "Continue", "HookOutcome", "Middleware", "Registration", "Replace", "Stop",
           "compose"]


def compose(*rules: Middleware) -> Middleware:
    """Thread several rules into one: a Stop ends them, a Replace hands on the new item."""

    async def middleware(item: Any, ctx: Any) -> HookOutcome:
        changed = False
        for rule in rules:
            outcome = await rule(item, ctx)
            if isinstance(outcome, Stop):
                return outcome
            if isinstance(outcome, Replace):
                item, changed = outcome.item, True
        return Replace(item) if changed else Continue()

    return middleware
