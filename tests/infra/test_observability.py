"""Tests for Fix 3 kernel INFO observability lines and imported-percept batching."""

from __future__ import annotations

import asyncio
import io
import json
import logging
from datetime import datetime, timezone

import httpx

from monoagens.__main__ import configure_logging
from monoagens.adapters.telegram import CAPABILITIES as TELEGRAM_CAPABILITIES
from monoagens.boot import build
from monoagens.core.hooks import Continue, Stop
from monoagens.core.reasoning import Reasoning
from monoagens.infra import config
from monoagens.infra.clock import FakeClock
from monoagens.infra.embedding import FakeEmbedder
from monoagens.infra.providers import OpenAICompatibleProvider
from monoagens.types import Event
from tests.support import settings

START = datetime(2026, 11, 10, 8, 0, tzinfo=timezone.utc)


class _ScriptedLLM:
    def __init__(self, calls: list[tuple[str, dict]]) -> None:
        self._calls = calls

    def handler(self, request: httpx.Request) -> httpx.Response:
        tool_calls = [
            {
                "id": f"call_{idx}",
                "type": "function",
                "function": {"name": name, "arguments": json.dumps(args)},
            }
            for idx, (name, args) in enumerate(self._calls)
        ]
        return httpx.Response(
            200,
            json={
                "choices": [{"message": {"role": "assistant", "tool_calls": tool_calls}}],
                "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15},
            },
        )


class _FakeAdapter:
    name = "telegram"
    shows = ("typing", "reply")

    def capabilities(self):
        return TELEGRAM_CAPABILITIES

    async def act(self, action) -> dict:
        return {"ok": True}

    async def show(self, *args, **kwargs) -> dict:
        return {}


def _info_lines(caplog) -> list[str]:
    return [
        rec.getMessage()
        for rec in caplog.records
        if rec.name == "kernel.events" and rec.levelno == logging.INFO
    ]


async def test_observability_logs_key_events_without_private_text(aconn, caplog) -> None:
    """Private text never leaks into INFO lines while key events and lengths do."""
    secret_dm = "SECRET_PRIVATE_DM_PHRASE_XYZ"
    secret_reply = "SECRET_REPLY_TEXT_ABC"
    llm = _ScriptedLLM([("act", {"capability": "telegram.say", "arguments": {"text": secret_reply}})])
    port = OpenAICompatibleProvider(
        base_url="http://llm.test/v1",
        api_key="test-key",
        model="fake",
        client=httpx.AsyncClient(transport=httpx.MockTransport(llm.handler)),
    )
    cfg = settings()
    pfc = Reasoning(port, settings=cfg)
    kernel = build(aconn, clock=FakeClock(START), pfc=pfc, settings=cfg, embedder=FakeEmbedder())
    kernel.register_adapter(_FakeAdapter())
    caplog.set_level(logging.INFO, logger="kernel")

    try:
        async def _stop_secret_word(event, ctx):
            if "BLOCK_ME" in str((event.payload or {}).get("text") or ""):
                return Stop("blocked by test guard")
            return Continue()

        kernel.hooks.get("inbound").add("test_block", _stop_secret_word)

        stop = asyncio.Event()
        runner = asyncio.create_task(kernel.run(stop))
        try:
            await kernel.put(Event(
                id=None, ts=START, kind="percept", who="person:owner",
                place="space:tg:999", body="telegram",
                payload={"text": secret_dm, "dm": True, "addressed": True},
            ))
            await kernel.put(Event(
                id=None, ts=START, kind="percept", who="person:owner",
                place="space:tg:-100123", body="telegram",
                payload={"text": "BLOCK_ME private text"},
            ))
            await kernel.put(Event(
                id=None, ts=START, kind="percept", who="person:alice",
                place="space:tg:-100123", body="telegram",
                payload={"text": "selamat pagi semua!"},
            ))
            await kernel.put(Event(
                id=None, ts=START, kind="error", who="telegram", place="", body="telegram",
                payload={"error": "timeout connecting https://user:topsecret@example.com/api"},
            ))
            await kernel.queue.join()
        finally:
            stop.set()
            await runner
    finally:
        await port.aclose()

    lines = _info_lines(caplog)
    joined = "\n".join(lines)
    assert secret_dm not in joined
    assert secret_reply not in joined
    assert "topsecret" not in joined
    assert f"kind=percept adapter=telegram place=space:tg:999 who=person:owner len={len(secret_dm)}" in lines
    assert any(line.startswith("kind=decision.wake ") and "reason=" in line for line in lines)
    assert any(line.startswith("kind=decision.silent ") and "reason=" in line for line in lines)
    assert f"kind=act.out adapter=telegram place=space:tg:999 who=person:owner len={len(secret_reply)}" in lines
    assert any(
        line.startswith("kind=guard.stopped ") and "reason=blocked by test guard" in line
        for line in lines
    )
    assert any(line.startswith("kind=error ") and "adapter=telegram" in line for line in lines)


async def test_imported_percepts_emit_summary_every_n_from_config(aconn, caplog) -> None:
    """Imported percepts batch into one INFO summary line every N items from config."""
    cfg = settings()
    cfg["runtime"] = {**config.load_config()["runtime"], "import_summary_every": 3}
    kernel = build(aconn, clock=FakeClock(START), settings=cfg, embedder=FakeEmbedder())
    kernel.register_adapter(_FakeAdapter())
    caplog.set_level(logging.INFO, logger="kernel")

    stop = asyncio.Event()
    runner = asyncio.create_task(kernel.run(stop))
    try:
        for i in range(6):
            await kernel.put(Event(
                id=None, ts=START, kind="percept", who="person:bob",
                place="space:tg:123", body="telegram",
                payload={"text": f"IMPORTED_TEXT_{i}", "imported": True},
            ))
        await kernel.queue.join()
    finally:
        stop.set()
        await runner

    lines = _info_lines(caplog)
    joined = "\n".join(lines)
    assert "IMPORTED_TEXT_" not in joined
    assert not any(line.startswith("kind=percept ") for line in lines)
    assert lines == [
        "kind=percept.imported count=3 adapter=telegram place=space:tg:123",
        "kind=percept.imported count=6 adapter=telegram place=space:tg:123",
    ]


def test_configure_logging_writes_info_lines_to_stdout_stream() -> None:
    """configure_logging writes kernel INFO events directly to stdout stream."""
    buf = io.StringIO()
    handler = configure_logging(buf)
    logger = logging.getLogger("kernel")
    prev_level = logger.level
    try:
        logging.getLogger("kernel.events").info("kind=percept adapter=cli place=cli who=owner len=4")
        assert "INFO kernel.events kind=percept adapter=cli place=cli who=owner len=4" in buf.getvalue()
    finally:
        logger.removeHandler(handler)
        logger.setLevel(prev_level)
