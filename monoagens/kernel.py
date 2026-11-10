"""The kernel loop: one queue, one consumer, one event at a time."""

from __future__ import annotations

import asyncio
from dataclasses import dataclass
from datetime import datetime
from typing import Any

import psycopg

from monoagens.clock import Clock
from monoagens.pfc import PFC, PFCContext, ToolCall
from monoagens.store import append_event, cursor as store_cursor
from monoagens.store import project_event, read_last_events, read_state
from monoagens.types import Action, Event

WAKE_REASON = "a message addressed to you"


@dataclass(frozen=True, slots=True)
class _Queued:
    event: Event
    ack: asyncio.Future[Event] | None


class Kernel:
    """The central loop of MonoAgens.

    Invariant 1: Body-blind. Never imports or references specific bodies.
    Invariant 3: One queue, one consumer.
    Invariant 4: Record first, then act.
    """

    def __init__(self, conn: psycopg.AsyncConnection[Any], clock: Clock | None = None,
                 pfc: PFC | None = None) -> None:
        self.conn = conn
        self.clock = clock or Clock()
        self.pfc = pfc
        self.queue: asyncio.Queue[_Queued] = asyncio.Queue()
        self.bodies: dict[str, Any] = {}

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
        """Register a body adapter under its `name`."""
        name = getattr(body, "name", None)
        if not name:
            raise ValueError("A body must expose a name")
        self.bodies[name] = body

    async def start_body(self, body: Any) -> asyncio.Task[None]:
        """Start a body adapter listening and putting events into the queue."""
        self.register_body(body)
        return asyncio.create_task(body.run(self.put))

    async def _append(self, event: Event, kind: str, payload: dict[str, Any], *,
                      caused_by: int | None = None, project: bool = True) -> Event:
        """Append one event and, unless it is a diagnostic, project it."""
        async with self.conn.transaction():
            stored = await append_event(
                self.conn,
                ts=self.clock.now(),
                kind=kind,
                who=event.who,
                place=event.place,
                body=event.body,
                payload=payload,
                caused_by=caused_by,
            )
            if project:
                await project_event(self.conn, stored)
        return stored

    async def _process(self, event: Event) -> Event:
        """Record one event, stamped with the kernel clock (single time source)."""
        if event.kind == "tick":
            return event
        return await self._append(event, event.kind, event.payload, caused_by=event.caused_by)

    async def _settle(self, item: _Queued) -> Event:
        """Record, acknowledge the source, then let the PFC act."""
        try:
            stored = await self._process(item.event)
        except BaseException as exc:
            if item.ack is not None and not item.ack.done():
                item.ack.set_exception(exc)
            self.queue.task_done()
            raise
        try:
            if item.ack is not None and not item.ack.done():
                item.ack.set_result(stored)
            await self._wake(stored)
        except Exception as exc:
            await self._record_error(stored, exc)
        finally:
            self.queue.task_done()
        return stored

    async def _wake(self, event: Event) -> None:
        """Wake the PFC for an addressed message (the gate arrives in Step 5)."""
        if self.pfc is None or event.kind != "msg.in" or not event.payload.get("addressed"):
            return
        recent = await read_last_events(self.conn, event.place or "", limit=20)
        context = PFCContext(
            now=self.clock.now(),
            place=event.place or "",
            place_kind=str(event.payload.get("place_kind") or "direct"),
            reason=WAKE_REASON,
            core_memory=await self._core_memory(),
            recent=[e for e in recent if e.id != event.id],
            trigger=event,
            last_interaction=await self._last_interaction(event),
        )
        for call in await self.pfc.decide(context):
            await self._apply(event, call)

    async def _core_memory(self) -> str:
        value = await read_state(self.conn, "core_memory")
        return str(value.get("text") or "") if value else ""

    async def _last_interaction(self, event: Event) -> datetime | None:
        for key in (f"last_spoke:{event.place}", f"last_seen:{event.who}"):
            value = await read_state(self.conn, key)
            if value and value.get("ts"):
                return datetime.fromisoformat(value["ts"])
        return None

    async def _apply(self, event: Event, call: ToolCall) -> None:
        """Write the action as an event, then hand it to the body."""
        args = call.arguments
        if call.name == "say" and isinstance(args.get("text"), str):
            await self._emit(event, "msg.out", {"text": args["text"]})
            await self._act(event, "say", {"text": args["text"]})
        elif call.name == "react" and isinstance(args.get("emoji"), str):
            payload = {"emoji": args["emoji"], "message_id": event.payload.get("message_id")}
            await self._emit(event, "react.out", payload)
            await self._act(event, "react", payload)
        elif call.name == "stay_silent":
            await self._emit(event, "decision.silent", {"reason": str(args.get("reason", ""))})
        else:
            await self._record_error(event, f"invalid tool call: {call.name} {args}")

    async def _emit(self, trigger: Event, kind: str, payload: dict[str, Any]) -> Event:
        return await self._append(trigger, kind, payload, caused_by=trigger.id)

    async def _act(self, event: Event, kind: str, payload: dict[str, Any]) -> None:
        body = self.bodies.get(event.body or "")
        if body is None:
            raise RuntimeError(f"No body registered for {event.body!r}")
        action = Action(kind=kind, place=event.place or "", payload=payload, body=event.body)
        await body.act(action)

    async def _record_error(self, event: Event, exc: BaseException | str) -> None:
        try:
            payload = {"error": str(exc), "event_id": event.id}
            await self._append(event, "error", payload, caused_by=event.id, project=False)
        except Exception:
            pass  # the Log itself is unreachable; nothing left to record.

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
            try:
                await self._settle(item)
            except Exception:
                continue  # one broken event never kills the loop.
