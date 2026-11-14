"""Workspace checks (4.8): the approval modes, a guest, and a task thread with a promise."""

from __future__ import annotations

import asyncio
import os
import pathlib
import shutil
from datetime import datetime, timezone
from typing import Any

import pytest

from monoagens.boot import build
from monoagens.core.reasoning import ToolCall
from tests.support import last_receipt as _last_receipt, receipt_result as _receipt
from monoagens.infra.clock import FakeClock
from monoagens.infra.store import append_event, read_state
from monoagens.middleware.loader import Loader
from monoagens.types import Action, Event
from tests.support import settings

ROOT = pathlib.Path(__file__).resolve().parent.parent.parent
START = datetime(2026, 1, 1, 3, 0, tzinfo=timezone.utc)
PLACE = "space:tg:1001"
BUDI = "space:wa:628110001"


class FileTest:
    """A test adapter that can send a text and a file."""

    name = "filetest"
    FILES = {"per_message": 2, "max_bytes": 50_000_000, "mimes": ()}

    def __init__(self) -> None:
        self.actions: list[Action] = []

    def capabilities(self):
        return ({"name": "say", "description": "Send a message.",
                 "schema": {"type": "object", "properties": {"text": {"type": "string"}},
                            "required": ["text"]},
                 "communicative": True, "long_running": False, "identity_changing": False,
                 "facts": (), "permission": None},
                {"name": "send_file", "description": "Send a file with a note.",
                 "schema": {"type": "object",
                            "properties": {"text": {"type": "string"},
                                           "attachments": {"type": "array",
                                                           "items": {"type": "string"}}},
                            "required": ["text"]},
                 "communicative": True, "long_running": False, "identity_changing": False,
                 "facts": (), "permission": "filetest.send"})

    async def act(self, action: Action) -> Any:
        self.actions.append(action)
        return None


class Scripted:
    """A reasoning that plays one scripted round per wake, then stops."""

    def __init__(self, rounds: list[list[tuple[str, dict]]]) -> None:
        self.rounds = list(rounds)
        self.last_usage: dict[str, int] = {}

    async def decide(self, context: Any):
        if context.results or not self.rounds:
            return []
        found = self.rounds.pop(0)
        return [ToolCall(name, dict(arguments)) for name, arguments in found]

    async def aclose(self):
        return None


def _deployment(tmp_path: pathlib.Path) -> pathlib.Path:
    """A root with the workspace plugin and its folders."""
    root = tmp_path / "deployment"
    (root / "plugins").mkdir(parents=True, exist_ok=True)
    shutil.copytree(ROOT / "plugins" / "workspace", root / "plugins" / "workspace")
    return root


async def _kernel(aconn, tmp_path, *, mode: str, pfc=None):
    root = _deployment(tmp_path)
    values = settings(plugins={"workspace": {"enabled": True}})
    values["roles"] = {"fallback": "unverified", "unverified": {"base": "guest"},
                       "runner": {"base": "trusted", "approval": mode}}
    values["people"] = {"person:owner": {"roles": ["runner"]},
                        "person:budi": {"roles": ["runner"]}}
    loader = Loader(root, env={})
    loaded = loader.load(values)
    kernel = build(aconn, clock=FakeClock(START), pfc=pfc, settings=values)
    adapter = FileTest()
    kernel.register_adapter(adapter)
    await loader.install(kernel, loaded)
    return kernel, adapter, root


async def _say(kernel, aconn, command: str, *, who="person:owner"):
    trigger = await append_event(aconn, ts=START, kind="percept", who=who, place=PLACE,
                                 body="filetest",
                                 payload={"description": "jalankan", "place_kind": "direct",
                                          "addressed": True})
    await kernel._apply(trigger, ToolCall("act", {"capability": "workspace.run",
                                                  "arguments": {"command": command}}))
    await kernel.run_until_idle()
    return _receipt(await _last_receipt(aconn))


@pytest.mark.parametrize("mode, asked", [("default", True), ("yolo", False),
                                         ("strict", True)])
async def test_the_approval_mode_decides_whether_run_asks(aconn, tmp_path, mode, asked):
    kernel, _adapter, _root = await _kernel(aconn, tmp_path, mode=mode)

    result = await _say(kernel, aconn, "ls")

    if asked:
        assert result["error"] == "waiting for the owner's approval"
    else:
        assert "error" not in result or "unavailable" in str(result.get("error"))


async def test_a_guest_is_refused_run_and_is_not_offered_it(aconn, tmp_path):
    kernel, _adapter, _root = await _kernel(aconn, tmp_path, mode="yolo")

    result = await _say(kernel, aconn, "ls", who="person:guest")

    assert result["error"] == "you may not use workspace.run"
    assert "workspace.run" not in kernel.tools.allowed("person:guest", kernel.roles)


async def test_finish_my_code_then_send_it_opens_a_task_thread(aconn, tmp_path):
    rounds = [
        [("act", {"capability": "workspace.run", "arguments": {"command": "python3 -m pytest"}})],
        [("act", {"capability": "filetest.send_file",
                  "arguments": {"text": "sudah selesai", "to": "person:budi"}})],
        [("act", {"capability": "filetest.send_file",
                  "arguments": {"text": "sudah selesai"}})],
    ]
    pfc = Scripted(rounds)
    kernel, adapter, _root = await _kernel(aconn, tmp_path, mode="yolo", pfc=pfc)
    await append_event(aconn, ts=START, kind="percept", who="person:budi", place=BUDI,
                       body="filetest",
                       payload={"description": "halo", "place_kind": "direct"})
    put = asyncio.create_task(kernel.put(Event(
        id=None, ts=START, kind="percept", who="person:owner", place=PLACE, body="filetest",
        payload={"description": "selesaikan kodenya lalu kirim ke Budi",
                 "place_kind": "direct", "addressed": True}, description="selesaikan kodenya lalu kirim ke Budi")))
    await asyncio.sleep(0)
    await kernel.step()
    await put
    await kernel.run_until_idle()

    cur = await aconn.execute("SELECT kind, place, payload FROM events ORDER BY id")
    rows = [(kind, place, payload) for kind, place, payload in await cur.fetchall()]
    kinds = [row[0] for row in rows]

    assert "task.opened" in kinds
    assert any(row[0] == "act.result" and "unavailable" in str(row[2].get("result"))
               or row[0] == "act.result" for row in rows)
    sent = [row for row in rows if row[0] == "act.out" and row[1] == BUDI]
    assert sent and adapter.actions


@pytest.mark.skipif(not os.environ.get("MONOAGENS_SANDBOX_E2E"),
                    reason="the sandbox end-to-end run needs a prepared base (4.8)")
async def test_the_sandbox_hides_the_host(aconn, tmp_path):
    kernel, _adapter, root = await _kernel(aconn, tmp_path, mode="yolo")
    workspace = root / "var" / "workspaces" / "person_owner"
    workspace.mkdir(parents=True, exist_ok=True)

    for command, expectation in (
            ("ls /", "work"),
            ("cat /home/muqsith/.env", "No such file"),
            ("python3 -c \"import os; print(sorted(os.environ))\"", "LLM_API_KEY")):
        result = await _say(kernel, aconn, command)
        text = str(result.get("output") or "") + str(result.get("error") or "")
        if expectation == "LLM_API_KEY":
            assert expectation not in text
        elif expectation == "No such file":
            assert expectation in text or "No such" in text
