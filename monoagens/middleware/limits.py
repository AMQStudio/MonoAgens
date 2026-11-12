"""Limits: the owner's daily token budget and the one event it writes when it runs out."""

from __future__ import annotations

from collections.abc import Awaitable, Callable, Mapping
from datetime import datetime, time
from typing import Any
from zoneinfo import ZoneInfo

from monoagens.infra.store import append_event


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
