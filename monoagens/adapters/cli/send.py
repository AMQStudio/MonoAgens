"""Sending: lines to every connected terminal client, and what the soul shows besides them."""

from __future__ import annotations

import asyncio
from collections.abc import Awaitable, Callable
from typing import Any

Write = Callable[[str], Awaitable[None]]


async def broadcast(writers: set[asyncio.StreamWriter], line: str) -> None:
    """Write one rendered line to every connected client; a dead one is dropped."""
    for writer in list(writers):
        try:
            writer.write(line.encode())
            await writer.drain()
        except (ConnectionError, RuntimeError):
            writers.discard(writer)
            writer.close()


async def file(writers: set[asyncio.StreamWriter], info: dict[str, Any], path: str) -> None:
    """Write one attachment line: its name and where it sits on disk."""
    await broadcast(writers, f"[file] {info.get('name') or 'file'} {path}\n")


async def show(writers: set[asyncio.StreamWriter], kind: str, **data: Any) -> None:
    """Render one non-message thing as a line: typing, a step, or an approval question."""
    if kind == "typing":
        await broadcast(writers, "[typing…]\n")
    elif kind in ("edit", "buttons", "steps"):
        await broadcast(writers, f"{data.get('text') or ''}\n")
    elif kind == "reactions":
        await broadcast(writers, f"[{data.get('emoji') or ''}]\n")
    else:
        raise ValueError(f"cannot show {kind!r}")
