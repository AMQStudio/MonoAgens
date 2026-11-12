"""Receiving: a Unix-socket server that hands every typed line to the kernel."""

from __future__ import annotations

import asyncio
import os
from collections.abc import Awaitable, Callable
from functools import partial

Put = Callable[[str], Awaitable[None]]


async def serve(path: str, put: Put, writers: set[asyncio.StreamWriter]) -> None:
    """Serve lines from local clients until the task is cancelled; owner-only socket."""
    _unlink(path)
    server = await asyncio.start_unix_server(partial(handle, put, writers), path=path)
    os.chmod(path, 0o600)
    try:
        async with server:
            await server.serve_forever()
    finally:
        _unlink(path)


async def handle(put: Put, writers: set[asyncio.StreamWriter], reader: asyncio.StreamReader,
                 writer: asyncio.StreamWriter) -> None:
    """Read one client's lines until it disconnects."""
    writers.add(writer)
    try:
        while raw := await reader.readline():
            await put(raw.decode().rstrip("\r\n"))
    except (ConnectionError, asyncio.CancelledError):
        pass
    finally:
        writers.discard(writer)
        writer.close()


def _unlink(path: str) -> None:
    """Remove a stale socket file, if any."""
    try:
        os.unlink(path)
    except FileNotFoundError:
        pass
