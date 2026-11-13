"""Sending: lines to every connected terminal client, and what the soul shows besides them."""

from __future__ import annotations

import asyncio
from collections.abc import Awaitable, Callable, Sequence
from typing import Any

Write = Callable[[str], Awaitable[None]]


def format_choices(choices: Sequence[str]) -> str:
    """Render choices as a terminal numbered list (`1. ...`)."""
    return "\n".join(f"{i}. {item}" for i, item in enumerate(choices, start=1))


def format_location(location: dict[str, Any]) -> str:
    """Render location coordinates as a readable terminal line."""
    lat = location.get("lat")
    lon = location.get("lon")
    title = str(location.get("name") or "Location")
    return f"[location] {title} ({lat}, {lon})"


def format_say_lines(*, text: str = "", reply_to: str | None = None,
                     mentions: Sequence[str] = (), sticker: str | None = None,
                     choices: Sequence[str] = (), thread: str | None = None,
                     location: dict[str, Any] | None = None) -> list[str]:
    """Build ordered output lines for CLI `say` with neutral fallbacks."""
    lines: list[str] = []
    if thread:
        lines.append(f"[{thread}]")
    body = text
    if mentions:
        tag = " ".join(f"@{m}" for m in mentions)
        body = f"{tag} {body}".strip() if body else tag
    if reply_to:
        body = f"(re #{reply_to}) {body}".strip()
    if body:
        lines.append(body)
    if sticker:
        lines.append(f"[sticker: {sticker}]")
    if location:
        lines.append(format_location(location))
    if choices:
        lines.append(format_choices(choices))
    return lines


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
    """Render one non-message status/fallback line in the terminal."""
    if kind == "typing":
        await broadcast(writers, "[typing…]\n")
    elif kind == "steps":
        await broadcast(writers, f"[step] {data.get('text') or ''}\n")
    elif kind == "reply":
        await broadcast(writers, f"{data.get('text') or ''}\n")
    elif kind == "reactions":
        await broadcast(writers, f"[{data.get('emoji') or ''}]\n")
    elif kind == "locations":
        loc = data.get("location") if isinstance(data.get("location"), dict) else data
        await broadcast(writers, f"{format_location(loc)}\n")
    else:
        raise ValueError(f"cannot show {kind!r}")
