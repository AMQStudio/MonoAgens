"""Budget: every model call is recorded, counted, and bounded by the day."""

from __future__ import annotations

from datetime import datetime, time
from typing import Any, Mapping
from zoneinfo import ZoneInfo

from monoagens.store import add_budget_usage, append_event
from monoagens.types import Event

COUNTERS = ("prompt_tokens", "cached_tokens", "completion_tokens", "total_tokens")


def _zone(settings: Mapping[str, Any]) -> ZoneInfo:
    return ZoneInfo(str(settings.get("timezone") or "UTC"))


def daily_limit(settings: Mapping[str, Any]) -> float:
    """Return the owner's daily token budget (0 or less means no cap)."""
    owner = settings.get("owner_settings") or {}
    return float(owner.get("daily_token_budget") or 0)


def key_for(now: datetime, settings: Mapping[str, Any]) -> str:
    """Return the state key that holds the token usage of that local day."""
    return f"budget:{now.astimezone(_zone(settings)).date().isoformat()}"


def used_fraction(value: Mapping[str, Any] | None, limit: float) -> float:
    """Return the share of the daily budget already spent, clamped to [0, 1]."""
    if limit <= 0:
        return 0.0
    total = int((value or {}).get("total_tokens") or 0)
    return min(1.0, total / limit)


async def record_call(conn, trigger_id: int | None, pfc: Any, *, purpose: str,
                      now: datetime, settings: Mapping[str, Any],
                      usage: Mapping[str, Any] | None = None) -> Event | None:
    """Record one model call with its purpose and tokens, then count it for today."""
    if trigger_id is None:
        return None
    raw = usage if usage is not None else getattr(pfc, "last_usage", None) or {}
    totals_usage = {name: int(raw.get(name) or 0) for name in COUNTERS}
    call = await append_event(
        conn, ts=now, kind="model.call", caused_by=trigger_id,
        payload={"purpose": purpose, "model": str(getattr(pfc, "model", "") or ""),
                 "usage": totals_usage})
    totals = await add_budget_usage(
        conn, now=now, timezone=str(settings.get("timezone") or "UTC"),
        usage=totals_usage, updated_by_event=call.id)
    await _record_exhausted(conn, now, settings, totals, call.id)
    return call


async def _record_exhausted(conn, now: datetime, settings: Mapping[str, Any],
                            totals: dict[str, int], call_id: int) -> None:
    """Write budget.exhausted once per local day when the budget runs out."""
    limit = daily_limit(settings)
    used = int(totals.get("total_tokens") or 0)
    if limit <= 0 or used < limit:
        return
    local = now.astimezone(_zone(settings))
    midnight = datetime.combine(local.date(), time.min, tzinfo=local.tzinfo)
    cur = await conn.execute("SELECT 1 FROM events WHERE kind = 'budget.exhausted' "
                             "AND ts >= %s LIMIT 1", (midnight,))
    if await cur.fetchone() is None:
        await append_event(conn, ts=now, kind="budget.exhausted", caused_by=call_id,
                           payload={"date": local.date().isoformat(), "used": used,
                                    "budget": limit})
