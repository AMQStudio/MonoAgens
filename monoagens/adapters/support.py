"""The plumbing every body shares: one receipt per action, splitting, cutting (§2.4.4).

A body answers exactly one receipt for an action id and never sends the same one twice, splits one
act's items by what one message takes, and cuts text at its own limit. That code lives here so no
adapter invents a second convention. It imports nothing from the middleware or the core.
"""

from __future__ import annotations

import logging
from collections.abc import Awaitable, Callable, Sequence
from typing import Any

from monoagens.contract import Receipt

LOGGER = logging.getLogger(__name__)


def claim(body: Any, action: Any) -> Receipt | None:
    """Return the receipt this body already answered for `action`, or None to carry it out.

    The memory is the body's own (`_receipts`), so an id this process never saw answers `unknown`
    and is not sent again: the contract's rule for a repeated action after a restart (§2.3).
    """
    found = getattr(body, "_receipts", None)
    if found is None:
        found = {}
        setattr(body, "_receipts", found)
    known = found.get(str(action.id))
    if known is not None:
        return known
    return None


def remember(body: Any, action: Any, receipt: Receipt) -> Receipt:
    """Store the one receipt of one action id and answer it."""
    found = getattr(body, "_receipts", None)
    if found is None:
        found = {}
        setattr(body, "_receipts", found)
    found[str(action.id)] = receipt
    return receipt


async def answer(body: Any, action: Any, *, capability: str,
                 function: Callable[[], Awaitable[Any]]) -> Receipt:
    """Carry one action out once and answer its receipt (§2.4.4).

    A returned value is `done`; `PermissionError` is `refused` with its reason; anything else is
    `failed` naming only the exception type, never its message (it may carry a secret).
    """
    known = claim(body, action)
    if known is not None:
        return known
    try:
        result = await function()
    except PermissionError as exc:
        return remember(body, action, Receipt("refused", reason=str(exc)))
    except Exception as exc:
        LOGGER.error("%s failed (%s)", capability, type(exc).__name__)
        return remember(body, action,
                        Receipt("failed", reason=f"{capability} failed ({type(exc).__name__})"))
    return remember(body, action, Receipt("done", result=result))


def split(items: Sequence[Any], per_message: int) -> list[list[Any]]:
    """Split one act's items into the batches a body sends per message."""
    step = max(1, int(per_message or 1))
    found: list[list[Any]] = []
    for start in range(0, len(items), step):
        found.append(list(items[start:start + step]))
    return found or [[]]


def cut(text: str, limit: int) -> str:
    """Cut one text at the body's own length limit, never in the middle of a character."""
    if limit <= 0 or len(text) <= limit:
        return text
    return text[:limit]
