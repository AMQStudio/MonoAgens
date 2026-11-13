"""Tests for core `imported` handling in sleep consolidation (D69)."""

from __future__ import annotations

from datetime import datetime, timedelta, timezone

from monoagens.core.memory import sleep, sleep_due
from monoagens.core.reasoning import ToolCall
from monoagens.infra.clock import FakeClock
from monoagens.infra.store import append_event

START = datetime(2026, 11, 13, 10, 0, tzinfo=timezone.utc)


class _DummyEmbedder:
    async def embed(self, texts: list[str]) -> list[list[float]]:
        return [[0.1] * 384 for _ in texts]


class _ScriptedSleepLLM:
    def __init__(self) -> None:
        self.prompts: list[str] = []
        self.usage = {"prompt_tokens": 10, "completion_tokens": 5}

    async def call(self, messages, _tools):
        self.prompts.append(str(messages[-1]["content"]))
        return [ToolCall(name="store_memories", arguments={
            "items": [{"text": "Active chat fact", "importance": 0.7}],
            "core_memory": "summary",
        })]


async def test_sleep_skips_inactive_imported_chats_until_they_become_active(aconn):
    chat_a = "space:wa:active@s.whatsapp.net"
    chat_b = "space:wa:inactive@s.whatsapp.net"

    # Import history into both A and B
    await append_event(aconn, ts=START, kind="percept", who="person:p_a", place=chat_a,
                       body="whatsapp", payload={"description": "Imported A hello", "imported": True})
    await append_event(aconn, ts=START, kind="percept", who="person:p_b", place=chat_b,
                       body="whatsapp", payload={"description": "Imported B secret", "imported": True})

    # Imported history alone does NOT trigger sleep
    assert await sleep_due(aconn, START + timedelta(hours=2), {}) is False

    # Chat A becomes active with a live message
    await append_event(aconn, ts=START + timedelta(hours=2), kind="percept", who="person:p_a",
                       place=chat_a, body="whatsapp", payload={"description": "Live A ping"})
    assert await sleep_due(aconn, START + timedelta(hours=3), {}) is True

    llm = _ScriptedSleepLLM()
    done1 = await sleep(aconn, pfc=llm, embedder=_DummyEmbedder(),
                        now=START + timedelta(hours=3), settings={})
    assert done1 is not None and done1.kind == "sleep.done"
    assert len(llm.prompts) == 1
    assert "Imported A hello" in llm.prompts[0]
    assert "Live A ping" in llm.prompts[0]
    assert "Imported B secret" not in llm.prompts[0]

    # Sleep is not due again while B stays inactive
    assert await sleep_due(aconn, START + timedelta(hours=4), {}) is False

    # Later, chat B becomes active -> sleep now includes B's earlier imported history too
    await append_event(aconn, ts=START + timedelta(hours=5), kind="percept", who="person:p_b",
                       place=chat_b, body="whatsapp", payload={"description": "Live B wake"})
    assert await sleep_due(aconn, START + timedelta(hours=6), {}) is True

    done2 = await sleep(aconn, pfc=llm, embedder=_DummyEmbedder(),
                        now=START + timedelta(hours=6), settings={})
    assert done2 is not None and done2.kind == "sleep.done"
    assert len(llm.prompts) == 2
    assert "Imported B secret" in llm.prompts[1]
    assert "Live B wake" in llm.prompts[1]
