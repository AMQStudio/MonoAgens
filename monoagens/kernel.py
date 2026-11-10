"""The kernel loop: one queue, one consumer, one event at a time."""

from __future__ import annotations

import asyncio
from dataclasses import dataclass
from datetime import datetime
from typing import Any

from monoagens import expect, memory
from monoagens.clock import Clock
from monoagens.config import load_soul
from monoagens.gate import Gate
from monoagens.pfc import PFC, PFCContext, ToolCall
from monoagens.store import (active_expectation_lines, append_event,
                             cursor as store_cursor, project_event, read_last_events,
                             read_state)
from monoagens.types import RECENT_KINDS, Action, Event


@dataclass(frozen=True, slots=True)
class _Queued:
    event: Event
    ack: asyncio.Future[Event] | None


class Kernel:
    """The one loop that records events, wakes the PFC, and acts through bodies."""

    def __init__(self, conn, clock: Clock | None = None, pfc: PFC | None = None,
                 gate: Gate | None = None, soul: dict[str, Any] | None = None,
                 embedder: Any = None) -> None:
        """Wire the kernel to its database connection and collaborators."""
        self.conn, self.clock, self.pfc = conn, clock or Clock(), pfc
        self.soul = dict(soul) if soul is not None else load_soul()
        self.gate = gate or Gate.from_soul(self.soul)
        self.embedder = embedder
        self.queue: asyncio.Queue[_Queued] = asyncio.Queue()
        self.bodies: dict[str, Any] = {}

    async def put(self, event: Event) -> None:
        """Accept one event from a body or the clock (ticks are not acknowledged)."""
        if event.kind == "tick":
            await self.queue.put(_Queued(event, None))
            return
        ack: asyncio.Future[Event] = asyncio.get_running_loop().create_future()
        await self.queue.put(_Queued(event, ack))
        await ack

    async def cursor(self, body: str) -> Any | None:
        """Return the last recorded cursor for a body."""
        return await store_cursor(self.conn, body)

    def register_body(self, body: Any) -> None:
        """Register a body under its name so actions can reach it."""
        name = getattr(body, "name", None)
        if not name:
            raise ValueError("A body must expose a name")
        self.bodies[name] = body

    async def _append(self, event: Event, kind: str, payload: dict[str, Any], *,
                      caused_by: int | None = None, project: bool = True) -> Event:
        async with self.conn.transaction():
            stored = await append_event(self.conn, ts=self.clock.now(), kind=kind, who=event.who,
                                        place=event.place, body=event.body, payload=payload,
                                        caused_by=caused_by)
            if project:
                await project_event(self.conn, stored)
        return stored

    async def _settle(self, item: _Queued) -> Event:
        try:
            if item.event.kind == "tick":
                stored = item.event
            else:
                stored = await self._append(item.event, item.event.kind, item.event.payload,
                                            caused_by=item.event.caused_by)
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
        fired = await expect.due(self.conn, event, now=self.clock.now(), soul=self.soul)
        for item in fired:
            await self._expect_wake(item)
        if event.kind == "tick":
            await self._maybe_sleep()
        if fired or self.pfc is None or event.kind != "msg.in":
            return
        verdict = await self.gate.judge(self.conn, event, now=self.clock.now())
        if verdict.decision != "wake":
            await self._emit(event, f"decision.{verdict.decision}", verdict.payload())
            return
        await self._think(event, verdict.reason, event.place or "")

    async def _think(self, event: Event, reason: str, place: str) -> None:
        recent = await read_last_events(self.conn, place, limit=20, kinds=RECENT_KINDS)
        context = PFCContext(
            now=self.clock.now(), place=place,
            place_kind=str(event.payload.get("place_kind") or "direct"), reason=reason,
            core_memory=await self._core_memory(),
            expectations=await active_expectation_lines(self.conn, place),
            memories=await self._memories(event), recent=[e for e in recent if e.id != event.id],
            trigger=event, last_interaction=await self._last_interaction(event))
        for call in await self.pfc.decide(context):
            await self._apply(event, call)

    async def _memories(self, event: Event) -> tuple[str, ...]:
        query = str(event.payload.get("text") or event.payload.get("about") or "")
        if self.embedder is None or not query.strip():
            return ()
        k = int((self.soul.get("memory") or {}).get("retrieve_k", 5))
        return tuple(await memory.retrieve(self.conn, self.embedder, query, k=k,
                                           now=self.clock.now()))

    async def _maybe_sleep(self) -> None:
        if self.pfc is None or self.embedder is None:
            return
        now = self.clock.now()
        if await memory.sleep_due(self.conn, now, self.soul):
            await memory.sleep(self.conn, pfc=self.pfc, embedder=self.embedder, now=now,
                               soul=self.soul)

    async def _expect_wake(self, item: dict[str, Any]) -> None:
        now = self.clock.now()
        trigger = item.get("trigger") or Event(
            id=item["fired_event"], ts=now, kind="expectation.fired", who=item["who"],
            place=item["place"], body=item["source_body"],
            payload={"expectation_id": item["id"], "about": item["about"]})
        if self.pfc is None:
            return
        verdict = await self.gate.judge(self.conn, trigger, now=now,
                                        surprise=float(item["weight"]))
        if verdict.decision != "wake":
            await self._emit(trigger, f"decision.{verdict.decision}", verdict.payload())
        else:
            reason = (f"expectation missed: {item['about']} "
                      f"(intent: {item['intent'] or 'none'}); {verdict.reason}")
            await self._think(trigger, reason, trigger.place or "")
        await expect.settle(self.conn, item["id"], now=now, soul=self.soul)

    async def recover(self) -> None:
        """Wake once more for expectations left fired across a restart."""
        for item in await expect.recovery(self.conn, now=self.clock.now()):
            await self._expect_wake(item)

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
        elif call.name == "expect":
            await expect.create(self.conn, args, now=self.clock.now(), soul=self.soul,
                                source=event)
        elif call.name == "resolve":
            await expect.resolve(self.conn, int(args["id"]), str(args["status"]),
                                 now=self.clock.now())
        else:
            await self._record_error(event, f"invalid tool call: {call.name} {args}")

    async def _emit(self, trigger: Event, kind: str, payload: dict[str, Any]) -> Event:
        return await self._append(trigger, kind, payload, caused_by=trigger.id)

    async def _act(self, event: Event, kind: str, payload: dict[str, Any]) -> None:
        body = self.bodies.get(event.body or "")
        if body is None:
            raise RuntimeError(f"No body registered for {event.body!r}")
        await body.act(Action(kind=kind, place=event.place or "", payload=payload,
                              body=event.body))

    async def _record_error(self, event: Event, exc: BaseException | str) -> None:
        try:
            await self._append(event, "error", {"error": str(exc), "event_id": event.id},
                               caused_by=event.id, project=False)
        except Exception:
            pass  # the Log itself is unreachable; nothing left to record.

    async def step(self) -> Event:
        """Process exactly one queued event."""
        return await self._settle(await self.queue.get())

    async def run_until_idle(self) -> int:
        """Process every queued event and return how many were handled."""
        count = 0
        while not self.queue.empty():
            await self.step()
            count += 1
        return count

    async def run(self, stop_event: asyncio.Event) -> None:
        """Run the consumer loop until the stop event is set."""
        await self.recover()
        while not stop_event.is_set():
            try:
                item = await asyncio.wait_for(self.queue.get(), timeout=0.1)
            except asyncio.TimeoutError:
                continue
            try:
                await self._settle(item)
            except Exception:
                continue  # one broken event never kills the loop.
