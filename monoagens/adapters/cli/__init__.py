"""CLI adapter: an in-process Unix socket; one percept per line typed by the owner."""

from __future__ import annotations

import asyncio
import os
from collections.abc import Awaitable, Callable
from typing import Any

from monoagens.adapters.cli import receive, send
from monoagens.adapters.cli.capabilities import CAPABILITIES, SHOWS
from monoagens.adapters.cli.format import event
from monoagens.infra.clock import Clock
from monoagens.types import Action, Event

DEFAULT_SOCKET = "/tmp/monoagens.sock"
Put = Callable[[Event], Awaitable[None]]
REQUIRES: tuple[str, ...] = ()


def socket_path() -> str:
    """Return CLI_SOCKET, falling back to the default socket path."""
    return os.environ.get("CLI_SOCKET") or DEFAULT_SOCKET


def create(options: dict[str, Any], env: dict[str, str]) -> "CLIBody":
    """Build the adapter from config.yaml's options and the environment (D49)."""
    return CLIBody(str(options.get("socket") or "") or None)


class CLIBody:
    """Eyes and hands for the local terminal over a Unix socket."""

    name = "cli"
    shows = SHOWS

    def __init__(self, path: str | None = None, clock: Clock | None = None) -> None:
        """Store the socket path; the file is created when the server starts."""
        self.path = path or socket_path()
        self._clock = clock or Clock()
        self._writers: set[asyncio.StreamWriter] = set()

    def capabilities(self) -> tuple[dict[str, Any], ...]:
        """Return the capability descriptors this adapter declares."""
        return CAPABILITIES

    async def run(self, put: Put) -> None:
        """Serve typed lines from local clients until the task is cancelled."""

        async def accept(text: str) -> None:
            await put(self._event(text))

        await receive.serve(self.path, accept, self._writers)

    async def act(self, action: Action) -> None:
        """Write one rendered line to every connected client."""
        if action.kind == "say":
            await send.broadcast(self._writers, f"{action.payload.get('text', '')}\n")
        elif action.kind == "react":
            await send.broadcast(self._writers, f"[{action.payload.get('emoji', '')}]\n")
        else:
            raise ValueError(f"Unsupported action kind: {action.kind!r}")

    async def show(self, place: str, kind: str, **data: Any) -> None:
        """Show one non-message thing as a line; unknown kinds are refused."""
        await send.show(self._writers, kind, **data)

    def _event(self, text: str) -> Event:
        return event(text, clock=self._clock, name=self.name)
