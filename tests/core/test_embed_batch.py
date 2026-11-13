"""Tests for Item 4 Stage 2 batched background embedding of imported percepts."""

from __future__ import annotations

import asyncio
from datetime import datetime, timezone

from monoagens.boot import build
from monoagens.core.memory import recall
from monoagens.infra import config
from monoagens.infra.clock import FakeClock
from monoagens.infra.embedding import FakeEmbedder
from monoagens.infra.store import append_event, read_event_vector
from monoagens.types import Action, Event
from tests.support import settings

START = datetime(2026, 11, 10, 8, 0, tzinfo=timezone.utc)


class _SpyEmbedder(FakeEmbedder):
    def __init__(self) -> None:
        self.calls: list[list[str]] = []

    async def embed(self, texts):
        batch = list(texts)
        self.calls.append(batch)
        return await super().embed(batch)


async def _wait_jobs_and_queue(kernel) -> None:
    await asyncio.sleep(0.02)
    for _ in range(200):
        await kernel.queue.join()
        if not kernel.jobs.in_flight and kernel.queue.empty():
            return
        await asyncio.sleep(0.01)


async def _allow(*_args, **_kwargs) -> bool:
    return True


async def test_imported_percepts_embed_in_batches_and_stay_searchable(aconn) -> None:
    """Imported percepts batch-embed off-loop, emit no per-item encode jobs, and remain searchable."""
    cfg = settings()
    cfg["runtime"] = {**config.load_config()["runtime"], "embed_batch_size": 3}
    embedder = _SpyEmbedder()
    kernel = build(aconn, clock=FakeClock(START), settings=cfg, embedder=embedder)

    stop = asyncio.Event()
    runner = asyncio.create_task(kernel.run(stop))
    imported_ids: list[int] = []
    try:
        tasks = [
            asyncio.create_task(kernel.put(Event(
                id=None, ts=START, kind="percept", who="person:alice",
                place="space:wa:100", body="whatsapp",
                payload={"text": f"hist topicalpha #{i}", "imported": True},
            )))
            for i in range(5)
        ]
        stored_imports = await asyncio.gather(*tasks)
        imported_ids = [ev.id for ev in stored_imports if ev is not None and ev.id is not None]
        await _wait_jobs_and_queue(kernel)

        live = await kernel.put(Event(
            id=None, ts=START, kind="percept", who="person:alice",
            place="space:wa:100", body="whatsapp",
            payload={"text": "live message", "imported": False},
        ))
        await _wait_jobs_and_queue(kernel)
    finally:
        stop.set()
        await runner

    rows = await (await aconn.execute(
        "SELECT kind, payload FROM events WHERE kind IN ('job.started', 'job.result') ORDER BY id ASC"
    )).fetchall()
    started_jobs = [r[1].get("job") for r in rows if r[0] == "job.started"]
    batch_results = [r[1].get("result") for r in rows if r[0] == "job.result" and r[1].get("job") == "embed_batch"]

    assert len(embedder.calls) == 3  # 2 batches for 5 imported (3 + 2) + 1 single for live
    assert [len(c) for c in embedder.calls[:2]] == [3, 2]
    assert started_jobs.count("embed_batch") == 2
    assert started_jobs.count("encode") == 1  # only the live message
    assert batch_results == [{"count": 3, "embedded": True}, {"count": 2, "embedded": True}]

    for eid in imported_ids:
        assert await read_event_vector(aconn, eid) is not None
    assert live is not None and live.id is not None
    assert await read_event_vector(aconn, live.id) is not None

    found = await recall.search(
        aconn,
        Action(kind="self.recall", place="space:wa:100", payload={"query": "topicalpha"}),
        visible=_allow,
        memory_visible=_allow,
    )
    assert found["count"] == 5
    assert {item["event_id"] for item in found["found"]} == set(imported_ids)

    # Also verify vector similarity lookup finds the batch-embedded imported events
    query_vec = (await embedder.embed(["hist topicalpha #2"]))[0]
    from pgvector import Vector
    nearest = await (await aconn.execute(
        "SELECT event_id FROM event_vectors ORDER BY embedding <=> %s LIMIT 1",
        (Vector(query_vec),),
    )).fetchone()
    assert nearest is not None and nearest[0] in imported_ids


async def test_recover_embeds_unvectorized_imported_percepts_in_batches(aconn) -> None:
    """Kernel.recover() queues any imported percepts missing from event_vectors."""
    ev1 = await append_event(
        aconn, ts=START, kind="percept", who="person:bob", place="space:wa:200", body="whatsapp",
        payload={"text": "crash orphan one", "imported": True},
    )
    ev2 = await append_event(
        aconn, ts=START, kind="percept", who="person:bob", place="space:wa:200", body="whatsapp",
        payload={"text": "crash orphan two", "imported": True},
    )
    cfg = settings()
    cfg["runtime"] = {**config.load_config()["runtime"], "embed_batch_size": 4}
    embedder = _SpyEmbedder()
    kernel = build(aconn, clock=FakeClock(START), settings=cfg, embedder=embedder)

    stop = asyncio.Event()
    runner = asyncio.create_task(kernel.run(stop))
    try:
        await _wait_jobs_and_queue(kernel)
    finally:
        stop.set()
        await runner

    assert len(embedder.calls) == 1
    assert len(embedder.calls[0]) == 2
    assert ev1.id is not None and await read_event_vector(aconn, ev1.id) is not None
    assert ev2.id is not None and await read_event_vector(aconn, ev2.id) is not None
