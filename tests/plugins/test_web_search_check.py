"""Web Search, end to end: with the plugin the soul can search; without it, it says so."""

from __future__ import annotations

import asyncio
import pathlib
import shutil
from datetime import datetime, timezone

from monoagens.boot import build
from monoagens.core.reasoning import ToolCall
from monoagens.infra.clock import FakeClock
from monoagens.infra.store import append_event
from monoagens.middleware.loader import Loader
from monoagens.types import Event
from tests.support import settings

ROOT = pathlib.Path(__file__).resolve().parent.parent.parent
START = datetime(2026, 1, 1, 3, 0, tzinfo=timezone.utc)
PLACE = "space:tg:1001"


class Voice:
    """A talking adapter that records what it sent."""

    name = "telegram"

    def __init__(self) -> None:
        self.actions = []

    def capabilities(self):
        return ({"name": "say", "description": "Speak here.",
                 "schema": {"type": "object", "properties": {"text": {"type": "string"}},
                            "required": ["text"]},
                 "communicative": True, "long_running": False, "identity_changing": False,
                 "facts": (), "permission": None},)

    async def act(self, action) -> None:
        self.actions.append(action)


class Scripted:
    """A reasoning that answers with one call list, then nothing more."""

    def __init__(self, calls) -> None:
        self.calls = calls
        self.contexts = []
        self.last_usage = {}

    async def decide(self, context):
        self.contexts.append(context)
        if len(self.contexts) > 1:
            return []
        return [ToolCall(name, dict(arguments)) for name, arguments in self.calls]

    async def aclose(self):
        return None


def _stub_ddgs(monkeypatch) -> None:
    """A ddgs module that answers with nothing, so the test needs no network or package."""
    import sys
    import types

    class _DDGS:
        def __init__(self, *args, **kwargs) -> None:
            return None

        def text(self, query, max_results=5):
            return []

    monkeypatch.setitem(sys.modules, "ddgs", types.SimpleNamespace(DDGS=_DDGS))


async def _run(aconn, tmp_path, *, with_plugin: bool, calls) -> tuple[list[str], object]:
    root = tmp_path
    (root / "plugins").mkdir(exist_ok=True)
    if with_plugin:
        shutil.copytree(ROOT / "plugins" / "web-search", root / "plugins" / "web-search")
    values = settings(plugins={"web-search": {"enabled": True}} if with_plugin else {})
    loader = Loader(root, env={})
    loaded = loader.load(values)
    pfc = Scripted(calls)
    kernel = build(aconn, clock=FakeClock(START), pfc=pfc, settings=values)
    kernel.register_adapter(Voice())
    await loader.install(kernel, loaded)
    put = asyncio.create_task(kernel.put(Event(
        id=None, ts=START, kind="percept", who="person:owner", place=PLACE, body="telegram",
        payload={"description": "siapa presiden sekarang?", "place_kind": "direct",
                 "addressed": True})))
    await asyncio.sleep(0)
    await kernel.step()
    await put
    await kernel.run_until_idle()
    cur = await aconn.execute("SELECT kind FROM events ORDER BY id")
    return [kind for (kind,) in await cur.fetchall()], kernel


async def test_with_the_plugin_a_search_runs_and_is_recorded(aconn, tmp_path, monkeypatch):
    _stub_ddgs(monkeypatch)
    kinds, kernel = await _run(aconn, tmp_path, with_plugin=True, calls=[
        ("act", {"capability": "web-search.web_search",
                 "arguments": {"query": "presiden indonesia sekarang"}})])
    cur = await aconn.execute(
        "SELECT payload FROM events WHERE kind = 'act.out' ORDER BY id DESC LIMIT 1")
    act = (await cur.fetchone())[0]

    assert "web-search.web_search" in kernel.tools.capabilities
    assert act["capability"] == "web-search.web_search"
    assert "model.call" in kinds


async def test_without_the_plugin_the_tool_is_absent_and_the_answer_is_honest(
        aconn, tmp_path, monkeypatch):
    _stub_ddgs(monkeypatch)
    honest = "I cannot check current facts; as of my knowledge the president is Prabowo."
    kinds, kernel = await _run(aconn, tmp_path, with_plugin=False, calls=[
        ("act", {"capability": "telegram.say", "arguments": {"text": honest}})])
    cur = await aconn.execute("SELECT payload FROM events WHERE kind = 'act.out'")
    act = (await cur.fetchone())[0]

    assert "web-search.web_search" not in kernel.tools.capabilities
    assert act["arguments"]["text"] == honest
    assert "model.call" in kinds
