"""Background jobs: slow work leaves the loop, results return as events.

Covers T8: PFC, embedding and sleep run as jobs under one timeout; one PFC job
per place is in flight; a wake that arrives meanwhile re-wakes the place once.
"""

from __future__ import annotations

import asyncio
from datetime import datetime, timedelta, timezone

from monoagens.clock import FakeClock
from monoagens.kernel import Kernel
from monoagens.memory import FakeEmbedder
from monoagens.pfc import ToolCall
from monoagens.store import append_event, read_event_vector, read_state
from monoagens.types import Event

START = datetime(2026, 11, 10, 8, 0, tzinfo=timezone.utc)
PLACE = "space:tg:1"
WHO = "person:owner"
SOUL = {
    "name": "Mona",
    "timezone": "Asia/Jakarta",
    "cognition": {"seed": 1, "weights": {"name_mentioned": 0.5, "info_gap": 0.35,
                                         "feeling_of_knowing": 0.35, "relevance": 0.2,
                                         "social_cost": -0.4}},
    "sleep": {"idle_minutes": 30, "retry_minutes": 30, "hour": 2},
    "memory": {"retrieve_k": 5},
    "expectations": {"default_expires_days": 30, "max_active": 200},
    "jobs": {"timeout_seconds": 5.0},
}
SLEEP_ARGS = {
    "items": [{"text": "Muqsith suka kopi tubruk tanpa gula", "importance": 0.9,
               "who": WHO, "place": PLACE}],
    "core_memory": "Muqsith tinggal di Bandung.",
}


class GatePFC:
    """A PFC whose decision waits on a gate, so the test controls the flight."""

    model = "gate-model"

    def __init__(self) -> None:
        self.gate = asyncio.Event()
        self.contexts: list = []
        self.last_usage = {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15}

    async def decide(self, context):
        self.contexts.append(context)
        await self.gate.wait()
        return [ToolCall("think", {"thought": "noted", "motivation": 1, "confidence": 1})]


class SleepPFC:
    """A PFC that answers the sleep consolidation call (and nothing else)."""

    model = "sleep-model"

    def __init__(self, arguments: dict) -> None:
        self.arguments = arguments
        self.last_usage = {"prompt_tokens": 7, "completion_tokens": 3, "total_tokens": 10}

    async def call(self, messages, tools, *, tool_choice="required"):
        return [ToolCall("store_memories", self.arguments)]


def _msg(text: str, *, message_id: int = 1) -> Event:
    return Event(id=None, ts=START, kind="percept", who=WHO, place=PLACE, body="telegram",
                 payload={"description": text, "addressed": True, "place_kind": "direct",
                          "data": {"message_id": message_id}})


async def _put(kernel: Kernel, event: Event) -> Event:
    """Record and process one event, without waiting for its background jobs."""
    put_task = asyncio.create_task(kernel.put(event))
    while True:
        await asyncio.sleep(0)
        if put_task.done():
            return put_task.result()
        await kernel.step()


async def _rows(conn, kind: str) -> list[dict]:
    return [row[0] for row in await (await conn.execute(
        "SELECT payload FROM events WHERE kind = %s ORDER BY id", (kind,))).fetchall()]


async def test_pfc_and_embedding_run_as_jobs_and_record_results(aconn):
    pfc = GatePFC()
    pfc.gate.set()
    kernel = Kernel(aconn, clock=FakeClock(START), pfc=pfc, soul=SOUL,
                    embedder=FakeEmbedder())

    stored = await _put(kernel, _msg("halo"))
    assert kernel.jobs.busy(f"pfc:{PLACE}")  # the result is not applied in the turn
    assert await _rows(aconn, "decision.note") == []

    await kernel.run_until_idle()
    starts = await _rows(aconn, "job.started")
    assert {"job": "pfc", "place": PLACE} in starts
    assert {"job": "encode", "place": PLACE} in starts

    results = await _rows(aconn, "job.result")
    assert any(item["job"] == "pfc" and item["result"]["calls"] for item in results)
    encoded = next(item for item in results if item["job"] == "encode")
    assert encoded["result"]["event_id"] == stored.id
    assert encoded["result"]["embedded"] is True and "vector" not in encoded["result"]

    assert [row["act"] for row in await _rows(aconn, "decision.note")] == ["think"]
    assert (await _rows(aconn, "model.call"))[0]["purpose"] == "pfc.decide"
    assert await read_event_vector(aconn, stored.id) is not None

    cur = await aconn.execute("SELECT caused_by FROM events WHERE kind = 'job.result' "
                             "AND payload->>'job' = 'pfc'")
    (caused_by,) = await cur.fetchone()
    cur = await aconn.execute("SELECT id FROM events WHERE kind = 'job.started' "
                             "AND payload->>'job' = 'pfc'")
    assert caused_by == (await cur.fetchone())[0]


async def test_one_pfc_per_place_and_one_re_wake_with_the_newest_event(aconn):
    pfc = GatePFC()
    kernel = Kernel(aconn, clock=FakeClock(START), pfc=pfc, soul=SOUL,
                    embedder=FakeEmbedder())

    first = await _put(kernel, _msg("first"))
    second = await _put(kernel, _msg("second", message_id=2))
    third = await _put(kernel, _msg("third", message_id=3))
    assert [key for key in kernel.jobs.in_flight if key.startswith("pfc:")] \
        == [f"pfc:{PLACE}"]  # still one PFC job for the place

    pfc.gate.set()
    await kernel.run_until_idle()

    assert len(pfc.contexts) == 2  # the first wake plus exactly one re-wake
    assert pfc.contexts[0].trigger.id == first.id
    assert pfc.contexts[1].trigger.id == third.id  # newest event wins; second collapsed
    assert not kernel.jobs.in_flight
    assert len(await _rows(aconn, "decision.note")) == 2


async def test_embedding_job_timeout_returns_a_job_result_error(aconn):
    class HangingEmbedder:
        async def embed(self, texts):
            await asyncio.sleep(10)
            return [[0.0] * 384 for _ in texts]

    soul = {**SOUL, "jobs": {"timeout_seconds": 0.01}}
    kernel = Kernel(aconn, clock=FakeClock(START), soul=soul, embedder=HangingEmbedder())
    stored = await _put(kernel, _msg("slow"))
    await kernel.run_until_idle()

    result = (await _rows(aconn, "job.result"))[0]
    assert result["job"] == "encode" and "timed out" in result["result"]["error"]
    assert await read_event_vector(aconn, stored.id) is None
    assert (await _rows(aconn, "error"))[0]["error"].startswith("encode timed out")


async def test_sleep_runs_as_a_job_and_records_sleep_done(aconn):
    await append_event(aconn, ts=START, kind="percept", who=WHO, place=PLACE, body="telegram",
                       payload={"text": "Muqsith suka kopi tubruk"})
    clock = FakeClock(START + timedelta(minutes=45))
    kernel = Kernel(aconn, clock=clock, pfc=SleepPFC(SLEEP_ARGS), soul=SOUL,
                    embedder=FakeEmbedder())

    await kernel.put(Event(id=None, ts=clock.now(), kind="tick"))
    await kernel.step()
    assert kernel.jobs.busy("sleep")
    await kernel.run_until_idle()

    assert [row["job"] for row in await _rows(aconn, "job.started")] == ["sleep"]
    assert await _rows(aconn, "sleep.done")
    assert (await read_state(aconn, "core_memory"))["text"] == SLEEP_ARGS["core_memory"]
    memories = await (await aconn.execute("SELECT text FROM memories")).fetchall()
    assert [row[0] for row in memories] == [item["text"] for item in SLEEP_ARGS["items"]]
