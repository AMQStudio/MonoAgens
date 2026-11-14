"""The plumbing every body shares: splitting one act's items and cutting one text (§2.4.4).

A body splits, cuts and retries by itself; this module is where that one code lives, so no
adapter invents a second convention. It imports nothing from the middleware or the core.
"""

from __future__ import annotations

from collections.abc import Sequence
from typing import Any


def split(items: Sequence[Any], per_message: int) -> list[list[Any]]:
    """Split one act's items into the batches a body sends per message.

    `per_message` below one means one item per message: a body that cannot say how many it may
    send still never exceeds what the contract's files fact allows.
    """
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
