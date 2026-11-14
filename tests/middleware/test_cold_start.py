"""Cold start: the slow parts are loaded before an adapter may deliver a percept (1.0a).

The embedder loads its model on the first call, which takes far longer than the 500 ms a
hook may run; `boot.warm` does that before any adapter starts, so the first message of a
cold start — often the pairing code — is handled inside the watchdog like any other.
"""

from __future__ import annotations

import asyncio
import time
from datetime import datetime, timezone

from monoagens.boot import build, warm
from monoagens.infra.clock import FakeClock
from monoagens.infra.embedding import FakeEmbedder
from monoagens.middleware import identity
from monoagens.types import Event
from tests.support import settings

START = datetime(2026, 11, 12, 8, 0, tzinfo=timezone.utc)
HOOK_TIMEOUT_SECONDS = 0.5  # the prior's hook_timeout_ms default (core/hooks.py)


class SlowModelEmbedder(FakeEmbedder):
    """An embedder that loads its model on the first call, as `FastEmbedder` does."""

    def __init__(self, load_seconds: float = 0.6) -> None:
        """Start unloaded; the first call takes `load_seconds`."""
        self.load_seconds = load_seconds
        self.loaded = False
        self.calls = 0

    async def embed(self, texts):
        """Sleep once as if loading the model, then embed like the fake embedder."""
        self.calls += 1
        if not self.loaded:
            await asyncio.sleep(self.load_seconds)
            self.loaded = True
        return await super().embed(texts)


def _percept(text: str) -> Event:
    return Event(id=None, ts=START, kind="percept", who="person:tg:7001",
                 place="space:tg:7001", body="telegram",
                 payload={"description": text, "place_kind": "direct", "addressed": True}, description=text)


async def _kinds(conn) -> list[str]:
    cur = await conn.execute("SELECT kind FROM events ORDER BY id")
    return [kind for (kind,) in await cur.fetchall()]


async def test_a_slow_embedder_is_warmed_before_the_first_percept(aconn):
    embedder = SlowModelEmbedder()
    kernel = build(aconn, clock=FakeClock(START), settings=settings(), embedder=embedder)
    code = await identity.ensure(aconn, now=START)

    warming = time.monotonic()
    await warm(kernel)
    warmed_in = time.monotonic() - warming

    assert embedder.loaded
    assert warmed_in >= embedder.load_seconds  # the model load happened here, not later

    processing = time.monotonic()
    put = asyncio.create_task(kernel.put(_percept(code)))
    await asyncio.sleep(0)
    await kernel.step()
    await asyncio.wait_for(put, 5.0)
    await kernel.run_until_idle()
    elapsed = time.monotonic() - processing

    kinds = await _kinds(aconn)
    assert "percept" in kinds
    assert identity.CLAIMED in kinds
    assert "error" not in kinds
    assert elapsed < HOOK_TIMEOUT_SECONDS
