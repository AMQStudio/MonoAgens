"""The kernel loop: one queue, one consumer, one event at a time."""

from __future__ import annotations

import asyncio
from dataclasses import dataclass
from typing import Any

import psycopg

from monoagens.clock import Clock
from monoagens.store import append_event, cursor as store_cursor, project_event
from monoagens.types import Event


@dataclass(frozen=True, slots=True)
class _Queued:
    """A queue item plus the intake acknowledgement awaited by its source."""

    event: Event
    ack: asyncio.Future[Event] | None


class Kernel:
    """The central loop of MonoAgens.

    Invariant 1: Body-blind. Never imports or references specific bodies.
    Invariant 3: One queue, one consumer.
    Invariant 4: Record first, then act.
    """

    def __init__(
        self,
        conn: psycopg.AsyncConnection[Any],
        clock: Clock | None = None,
    ) -> None:
        self.conn = conn
        self.clock = clock or Clock()
        self.queue: asyncio.Queue[_Queued] = asyncio.Queue()
        self.bodies: list[Any] = []

    async def put(self, event: Event) -> None:
        """Enqueue an event from any source (bodies, clock).

        Non-tick events return only after they are committed to the Log and
        projected onto the Status Board (intake acknowledgement). Ticks are
        fire-and-forget: they are never persisted.
        """
        if event.kind == "tick":
            await self.queue.put(_Queued(event, None))
            return
        ack: asyncio.Future[Event] = asyncio.get_running_loop().create_future()
        await self.queue.put(_Queued(event, ack))
        await ack

    async def cursor(self, body: str) -> Any | None:
        """Query the last recorded cursor for a body adapter."""
        return await store_cursor(self.conn, body)

    def register_body(self, body: Any) -> None:
        """Register a body adapter with the kernel."""
        self.bodies.append(body)

    async def start_body(self, body: Any) -> asyncio.Task[None]:
        """Start a body adapter listening and putting events into the queue."""
        self.register_body(body)
        return asyncio.create_task(body.run(self.put))

    async def _process(self, event: Event) -> Event:
        """Record one event, stamped with the kernel clock (single time source).

        Ticks are discarded from persistence.
        All other events are persisted and projected in one transaction.
        """
        if event.kind == "tick":
            return event

        async with self.conn.transaction():
            stored = await append_event(
                self.conn,
                ts=self.clock.now(),
                kind=event.kind,
                who=event.who,
                place=event.place,
                body=event.body,
                payload=event.payload,
                caused_by=event.caused_by,
            )
            await project_event(self.conn, stored)
        return stored

    async def _settle(self, item: _Queued) -> Event:
        """Process one queued item and resolve its intake acknowledgement."""
        try:
            stored = await self._process(item.event)
        except BaseException as exc:
            if item.ack is not None and not item.ack.done():
                item.ack.set_exception(exc)
            raise
        else:
            if item.ack is not None and not item.ack.done():
                item.ack.set_result(stored)
            return stored
        finally:
            self.queue.task_done()

    async def step(self) -> Event:
        """Process one event from the queue."""
        item = await self.queue.get()
        return await self._settle(item)

    async def run_until_idle(self) -> int:
        """Process all queued events until the queue is empty."""
        count = 0
        while not self.queue.empty():
            await self.step()
            count += 1
        return count

    async def run(self, stop_event: asyncio.Event) -> None:
        """Run the single consumer loop until stopped."""
        while not stop_event.is_set():
            try:
                item = await asyncio.wait_for(self.queue.get(), timeout=0.1)
            except asyncio.TimeoutError:
                continue
            await self._settle(item)
