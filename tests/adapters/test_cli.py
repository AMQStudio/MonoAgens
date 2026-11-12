"""Tests for the CLI body (in-process Unix socket) and its thin client."""

from __future__ import annotations

import asyncio
import os
import pathlib
import stat
import subprocess
import sys

import pytest

from monoagens.adapters.cli import CLIBody
from monoagens.types import Action, Event


async def _wait_for(path: str, predicate, message: str, timeout: float = 5.0) -> None:
    loop = asyncio.get_running_loop()
    deadline = loop.time() + timeout
    while not predicate():
        if loop.time() > deadline:
            raise AssertionError(message)
        await asyncio.sleep(0.01)


async def test_socket_round_trip_with_cli_body(tmp_path):
    path = str(tmp_path / "monoagens.sock")
    pathlib.Path(path).write_text("stale socket file")
    body = CLIBody(path)
    posted: list[Event] = []
    posted_event = asyncio.Event()

    async def put(event: Event) -> None:
        posted.append(event)
        posted_event.set()

    task = asyncio.create_task(body.run(put))
    try:
        await _wait_for(path, lambda: os.path.exists(path)
                        and stat.S_ISSOCK(os.stat(path).st_mode),
                        f"socket {path} was never created")
        await _wait_for(path, lambda: stat.S_IMODE(os.stat(path).st_mode) == 0o600,
                        f"socket {path} was not made owner-only")

        reader, writer = await asyncio.open_unix_connection(path)
        writer.write(b"halo\n")
        await writer.drain()
        await asyncio.wait_for(posted_event.wait(), timeout=5.0)

        event = posted[0]
        assert event.kind == "percept"
        assert event.body == "cli"
        assert event.who == "person:owner"
        assert event.place == "space:cli"
        assert event.payload == {"description": "halo", "addressed": True,
                                 "place_kind": "direct"}

        await body.act(Action(kind="say", place="space:cli", payload={"text": "hai"}))
        assert await asyncio.wait_for(reader.readline(), timeout=5.0) == b"hai\n"

        await body.act(Action(kind="react", place="space:cli", payload={"emoji": "👍"}))
        assert await asyncio.wait_for(reader.readline(), timeout=5.0) == "[👍]\n".encode()
        writer.close()
    finally:
        task.cancel()
        with pytest.raises(asyncio.CancelledError):
            await task

    assert not os.path.exists(path)


def test_client_reports_main_process_not_running(tmp_path):
    env = {**os.environ, "CLI_SOCKET": str(tmp_path / "missing.sock")}

    result = subprocess.run(
        [sys.executable, "-m", "monoagens.cli"],
        stdin=subprocess.DEVNULL,
        capture_output=True,
        text=True,
        env=env,
        timeout=30,
    )

    assert result.returncode == 1
    assert "MonoAgens main process is not running." in result.stderr


def test_capability_descriptors():
    """The CLI declares the same communicative and non-communicative capabilities."""
    caps = {cap["name"]: cap for cap in CLIBody("/tmp/unused.sock").capabilities()}
    assert set(caps) == {"say", "react"}
    assert caps["say"]["communicative"] is True
    assert caps["react"]["communicative"] is False


def test_a_typed_command_becomes_a_command_event():
    from monoagens.adapters.cli.format import command_event
    from monoagens.infra.clock import FakeClock

    clock = FakeClock()
    slash = command_event("/mode professional", clock=clock)
    colon = command_event(":help", clock=clock)

    assert slash.kind == "command" and slash.payload["command"] == "mode"
    assert slash.payload["arguments"] == ["professional"]
    assert colon.payload["command"] == "help"
    assert command_event("halo", clock=clock) is None


async def test_the_cli_body_sends_a_command_as_a_command():
    from monoagens.adapters.cli import CLIBody

    body = CLIBody("/tmp/monoagens-cli-test.sock")
    event = await body._event("/whoami")

    assert event.kind == "command" and event.payload["command"] == "whoami"
