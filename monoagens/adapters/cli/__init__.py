"""CLI adapter: an in-process Unix socket; one percept per line typed by the owner."""

from __future__ import annotations

import asyncio
import os
import pathlib
from collections.abc import Awaitable, Callable, Sequence
from typing import Any

from monoagens.adapters.cli import receive, send
from monoagens.adapters.cli.capabilities import (
    CAPABILITIES,
    GAPS,
    OPTIONAL_CAPABILITIES,
    READS,
    SHOWS,
)
from monoagens.adapters.cli.format import command_event, event, split_line
from monoagens.infra.clock import Clock
from monoagens.types import Action, Event

DEFAULT_SOCKET = "/tmp/monoagens.sock"
Put = Callable[[Event], Awaitable[None]]
REQUIRES: tuple[str, ...] = ()
FILES = {"per_message": 1, "max_bytes": 50_000_000, "mimes": ()}


def socket_path() -> str:
    """Return CLI_SOCKET, falling back to the default socket path."""
    return os.environ.get("CLI_SOCKET") or DEFAULT_SOCKET


def create(options: dict[str, Any], env: dict[str, str]) -> "CLIBody":
    """Build the adapter from config.yaml's options and the environment (D49)."""
    del env
    tools = [str(t) for t in (options.get("tools") or ()) if t]
    unknown = [t for t in tools if t not in OPTIONAL_CAPABILITIES]
    if unknown:
        raise ValueError(f"cli: unknown optional tool {unknown[0]!r}")
    return CLIBody(str(options.get("socket") or "") or None, enabled_tools=tools)


class CLIBody:
    """Eyes and hands for the local terminal over a Unix socket."""

    name = "cli"
    reads = READS
    shows = SHOWS
    gaps = GAPS

    def __init__(self, path: str | None = None, clock: Clock | None = None, *,
                 enabled_tools: Sequence[str] = ()) -> None:
        """Store socket path, enabled platform tools, and prompt tracking state."""
        self.path = path or socket_path()
        self._clock = clock or Clock()
        self._writers: set[asyncio.StreamWriter] = set()
        self.files: Any = None
        self.enabled_tools = tuple(t for t in enabled_tools if t in OPTIONAL_CAPABILITIES)
        self._seq = 0
        self._last_prompt: tuple[str, list[str]] | None = None

    def capabilities(self) -> tuple[dict[str, Any], ...]:
        """Return declared capabilities plus any platform tools enabled in `bodies.cli.tools`."""
        extras = tuple(OPTIONAL_CAPABILITIES[name] for name in self.enabled_tools)
        return CAPABILITIES + extras

    async def place_info(self, place: str) -> dict[str, Any]:
        """Return structured place info for the local owner terminal."""
        del place
        return {"form": "dm", "parent": None, "can_post": True, "name": "Terminal"}

    async def run(self, put: Put) -> None:
        """Serve typed lines from local clients until the task is cancelled."""

        async def accept(text: str) -> None:
            await put(await self._event(text))

        await receive.serve(self.path, accept, self._writers)

    async def act(self, action: Action) -> Any:
        """Write one rendered line or optional platform action to connected clients."""
        p = action.payload or {}
        if action.kind == "say":
            self._seq += 1
            msg_id = f"cli:{self._seq}"
            choices = [str(c) for c in (p.get("choices") or ()) if str(c)]
            if choices:
                self._last_prompt = (msg_id, choices)
            lines = send.format_say_lines(
                text=str(p.get("text") or ""),
                reply_to=str(p["reply_to"]) if p.get("reply_to") else None,
                mentions=list(p.get("mentions") or ()),
                sticker=str(p["sticker"]) if p.get("sticker") else None,
                choices=choices,
                thread=str(p["thread"]) if p.get("thread") else None,
                location=p.get("location") if isinstance(p.get("location"), dict) else None,
            )
            for line in lines:
                await send.broadcast(self._writers, f"{line}\n")
            for ref in p.get("attachments") or ():
                try:
                    data, info = await self.files.open(str(ref), context=action.place)
                except Exception as exc:
                    return {"error": f"cannot attach {ref}: {exc}"}
                await send.file(self._writers, info, await self._where(ref, data))
            res: dict[str, Any] = {"message_id": msg_id}
            if choices:
                res["choices"] = [{"label": opt, "answer": str(idx)}
                                  for idx, opt in enumerate(choices, 1)]
            return res
        if action.kind == "react":
            await send.broadcast(self._writers, f"[{p.get('emoji', '')}]\n")
            return {"emoji": p.get("emoji", "")}
        if action.kind == "pin" and "pin" in self.enabled_tools:
            await send.broadcast(self._writers, f"[pinned] {p.get('text') or ''}\n")
            return {"pinned": True}
        raise ValueError(f"Unsupported action kind: {action.kind!r}")

    async def show(self, place: str, kind: str, **data: Any) -> None:
        """Show one non-message thing as a status/fallback line; unknown kinds are refused."""
        del place
        if kind in ("buttons", "menus") and data.get("buttons"):
            self._seq += 1
            msg_id = f"cli:{self._seq}"
            self._last_prompt = (msg_id, [str(lbl) for lbl, _ in data["buttons"]])
        await send.show(self._writers, kind, **data)

    async def _where(self, ref: str, data: bytes) -> str:
        """Return the path a written file has on disk, for the terminal line."""
        del data
        found = self.files.store.find(str(ref))
        return str(found) if found is not None else str(ref)

    async def _event(self, line: str) -> Event:
        """Turn one typed line into a command or a percept, storing any files it names."""
        found = command_event(line, clock=self._clock, name=self.name)
        if found is not None:
            return found
        paths, text = split_line(line)
        attachments = []
        for path in paths:
            try:
                data = await asyncio.to_thread(pathlib.Path(path).read_bytes)
            except OSError as exc:
                text = f"{text} [cannot read {path}: {exc}]".strip()
                continue
            attachments.append(await self.files.put(data, name=pathlib.Path(path).name))
        return event(text, clock=self._clock, name=self.name,
                     attachments=attachments, last_prompt=self._last_prompt)
