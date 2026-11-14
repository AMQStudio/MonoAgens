"""CLI adapter: an in-process Unix socket; one percept per line typed by the owner."""

from __future__ import annotations

import asyncio
import os
import pathlib
from collections.abc import Awaitable, Callable, Sequence
from typing import Any

from monoagens.adapters import support
from monoagens.adapters.cli import receive, send
from monoagens.adapters.cli.format import PLACE as CLI_PLACE
from monoagens.adapters.cli.capabilities import (
    CAPABILITIES,
    GAPS,
    OPTIONAL_CAPABILITIES,
    READS,
    SHOWS,
)
from monoagens.adapters.cli.format import command_event, event, split_line
from monoagens.infra.clock import Clock
from monoagens import contract
from monoagens.contract import Action, Percept

DEFAULT_SOCKET = "/tmp/monoagens.sock"
Put = Callable[[Any], Awaitable[None]]
REQUIRES: tuple[str, ...] = ()


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
    FILES = {"per_message": 1, "max_bytes": 50_000_000, "mimes": ()}
    PERMISSIONS = {"say": "guest", "react": "guest", "pin": "guest"}
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

    async def stop(self) -> None:
        """Stop the socket server and close every connected client."""
        for writer in list(self._writers):
            writer.close()
        self._writers.clear()

    async def render(self, event: Any) -> None:
        """Render one lifecycle event as a terminal line (§2.4.5)."""
        kind = "typing" if event.name == "composing" else event.name
        await self.show(str(event.place), kind, text=str(event.data.get("text") or ""))

    def prompt_section(self) -> str:
        """Return what reasoning should know about the local terminal (≤1200 characters)."""
        return ("The terminal: type a line to speak here; a line starting with @path sends that "
                "file; reply by the id of an earlier line; /name args runs a command.")

    def self_actor(self) -> str:
        """Return the actor id this body acts as."""
        return "cli:local"

    async def run(self, put: Put) -> None:
        """Publish this terminal's audience, then serve typed lines until cancelled (§4.1)."""
        await put(contract.audience_event(
            CLI_PLACE, contract.Audience(frozenset({"person:owner"})),
            ts=self._clock.now(), body=self.name))

        async def accept(text: str) -> None:
            await put(await self._event(text))

        await receive.serve(self.path, accept, self._writers)

    async def act(self, action: Action) -> contract.Receipt:
        """Write one line or optional platform action, and answer one receipt (§2.4.4)."""
        return await support.answer(
            self, action, capability=f"{self.name}.{action.capability}",
            function=lambda: self._carry(action))

    async def _carry(self, action: Action) -> Any:
        """Carry one action out; `support.answer` turns what comes back into the receipt."""
        p = action.arguments or {}
        if action.capability == "say":
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
        if action.capability == "react":
            await send.broadcast(self._writers, f"[{p.get('emoji', '')}]\n")
            return {"emoji": p.get("emoji", "")}
        if action.capability == "pin" and "pin" in self.enabled_tools:
            await send.broadcast(self._writers, f"[pinned] {p.get('text') or ''}\n")
            return {"pinned": True}
        raise ValueError(f"Unsupported action: {action.capability!r}")

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

    async def _event(self, line: str) -> Any:
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
