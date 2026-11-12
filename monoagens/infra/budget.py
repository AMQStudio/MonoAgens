"""Budget: every model call is recorded and counted for the day. The cap is middleware.

The core counts (invariant 5); `monoagens/middleware/limits.py` owns the limit and the
one event it writes when the day's budget runs out.
"""

from __future__ import annotations

from datetime import datetime
from typing import Any, Mapping
from zoneinfo import ZoneInfo

from monoagens.infra.store import add_budget_usage, append_event
from monoagens.types import Event

COUNTERS = ("prompt_tokens", "cached_tokens", "completion_tokens", "reasoning_tokens",
            "total_tokens")


def key_for(now: datetime, settings: Mapping[str, Any]) -> str:
    """Return the state key that holds the token usage of that local day."""
    zone = ZoneInfo(str(settings.get("timezone") or "UTC"))
    return f"budget:{now.astimezone(zone).date().isoformat()}"


def used_fraction(value: Mapping[str, Any] | None, limit: float) -> float:
    """Return the share of the daily budget already spent, clamped to [0, 1]."""
    if limit <= 0:
        return 0.0
    total = int((value or {}).get("total_tokens") or 0)
    return min(1.0, total / limit)


async def record_call(conn, trigger_id: int | None, pfc: Any, *, purpose: str,
                      now: datetime, settings: Mapping[str, Any],
                      usage: Mapping[str, Any] | None = None,
                      ) -> tuple[Event | None, dict[str, int]]:
    """Record one model call with its purpose and tokens, then count it for today."""
    if trigger_id is None:
        return None, {}
    raw = usage if usage is not None else getattr(pfc, "last_usage", None) or {}
    totals_usage = {name: int(raw.get(name) or 0) for name in COUNTERS}
    call = await append_event(
        conn, ts=now, kind="model.call", caused_by=trigger_id,
        payload={"purpose": purpose, "model": str(getattr(pfc, "model", "") or ""),
                 "usage": totals_usage})
    totals = await add_budget_usage(
        conn, now=now, timezone=str(settings.get("timezone") or "UTC"),
        usage=totals_usage, updated_by_event=call.id)
    return call, totals
