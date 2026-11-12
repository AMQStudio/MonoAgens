"""CLI adapter: an in-process Unix socket; one percept per line typed by the owner."""

from __future__ import annotations

import asyncio
import os
from collections.abc import Awaitable, Callable
from functools import partial

from monoagens.infra.clock import Clock
from monoagens.types import Action, Event

DEFAULT_SOCKET = "/tmp/monoagens.sock"
Put = Callable[[Event], Awaitable[None]]

CAPABILITIES = (
    {"name": "say", "description": "Write a text line to the terminal.",
     "schema": {"type": "object", "properties": {"text": {"type": "string"}},
                "required": ["text"]},
     "communicative": True, "long_running": False, "identity_changing": False},
    {"name": "react", "description": "Write one emoji to the terminal.",
     "schema": {"type": "object",
                "properties": {"emoji": {"type": "string"},
                               "message_id": {"type": "integer"}},
                "required": ["emoji"]},
     "communicative": False, "long_running": False, "identity_changing": False},
)


def socket_path() -> str:
    """Return CLI_SOCKET, falling back to the default socket path."""
    return os.environ.get("CLI_SOCKET") or DEFAULT_SOCKET


class CLIBody:
    """Eyes and hands for the local terminal over a Unix socket."""

    name = "cli"

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
        self._unlink()
        server = await asyncio.start_unix_server(partial(self._handle, put), path=self.path)
        os.chmod(self.path, 0o600)
        try:
            async with server:
                await server.serve_forever()
        finally:
            self._unlink()

    async def _handle(self, put: Put, reader: asyncio.StreamReader,
                      writer: asyncio.StreamWriter) -> None:
        self._writers.add(writer)
        try:
            while raw := await reader.readline():
                await put(self._event(raw.decode().rstrip("\r\n")))
        except (ConnectionError, asyncio.CancelledError):
            pass
        finally:
            self._writers.discard(writer)
            writer.close()

    async def act(self, action: Action) -> None:
        """Write one rendered line to every connected client."""
        if action.kind == "say":
            line = f"{action.payload.get('text', '')}\n"
        elif action.kind == "react":
            line = f"[{action.payload.get('emoji', '')}]\n"
        else:
            raise ValueError(f"Unsupported action kind: {action.kind!r}")
        for writer in list(self._writers):
            try:
                writer.write(line.encode())
                await writer.drain()
            except (ConnectionError, RuntimeError):
                self._writers.discard(writer)
                writer.close()

    def _event(self, text: str) -> Event:
        return Event(id=None, ts=self._clock.now(), kind="percept",
                     who="person:owner", place="space:cli", body=self.name,
                     payload={"description": text, "addressed": True, "place_kind": "direct"})

    def _unlink(self) -> None:
        try:
            os.unlink(self.path)
        except FileNotFoundError:
            pass
