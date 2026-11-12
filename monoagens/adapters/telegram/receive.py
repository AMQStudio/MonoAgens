"""Receiving: one getUpdates call. The polling loop and its backoff live in the adapter."""

from __future__ import annotations

from collections.abc import Awaitable, Callable
from typing import Any

Call = Callable[..., Awaitable[Any]]


async def updates(call: Call, offset: int | None, timeout: float) -> list[dict[str, Any]]:
    """Ask Telegram for the updates after `offset`; an empty answer is normal."""
    params: dict[str, Any] = {"timeout": timeout}
    if offset is not None:
        params["offset"] = offset
    found = await call("getUpdates", **params)
    if not isinstance(found, list):
        raise RuntimeError("telegram: getUpdates answered without a list")
    return found
