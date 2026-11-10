"""Thin CLI client: stdin lines to the main process, replies back to stdout."""

from __future__ import annotations

import asyncio
import os
import sys

DEFAULT_SOCKET = "/tmp/monoagens.sock"
NOT_RUNNING = "MonoAgens main process is not running."


async def _print_replies(reader: asyncio.StreamReader) -> None:
    while line := await reader.readline():
        print(line.decode(), end="", flush=True)


async def main() -> int:
    """Connect to the socket, forward stdin, print replies; 1 when not running."""
    path = os.environ.get("CLI_SOCKET") or DEFAULT_SOCKET
    try:
        reader, writer = await asyncio.open_unix_connection(path)
    except OSError:
        print(NOT_RUNNING, file=sys.stderr)
        return 1
    replies = asyncio.create_task(_print_replies(reader))
    try:
        while typed := await asyncio.to_thread(sys.stdin.readline):
            writer.write((typed if typed.endswith("\n") else typed + "\n").encode())
            await writer.drain()
        writer.write_eof()
        await replies  # the server closes the socket after it sees our EOF
    finally:
        writer.close()
        replies.cancel()
    return 0


if __name__ == "__main__":
    raise SystemExit(asyncio.run(main()))
