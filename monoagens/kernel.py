"""The kernel loop: one queue, one consumer, one event at a time."""
from __future__ import annotations

import asyncio
from dataclasses import dataclass
from datetime import datetime
from typing import Any

import psycopg

from monoagens import expect
from monoagens.clock import Clock
from monoagens.config import load_soul
from monoagens.gate import Gate
from monoagens.pfc import PFC, PFCContext, ToolCall
from monoagens.store import (active_expectation_lines, append_event, cursor as store_cursor, project_event,
                             read_last_events, read_state)
from monoagens.types import Action, Event

@dataclass(frozen=True, slots=True)
class _Queued:
    event: Event
    ack: asyncio.Future[Event] | None


class Kernel:
    """The single consumer. Body-blind; records first, then acts."""

    def __init__(self, conn, clock: Clock | None = None, pfc: PFC | None = None, gate: Gate | None = None,
                 soul: dict[str, Any] | None = None) -> None:
        self.conn = conn
        self.clock = clock or Clock()
        self.pfc = pfc
        self.soul = dict(soul) if soul is not None else load_soul()
        self.gate = gate or Gate.from_soul(self.soul)
        self.queue: asyncio.Queue[_Queued] = asyncio.Queue()
        self.bodies: dict[str, Any] = {}

    async def put(self, event: Event) -> None:
        """Enqueue; non-tick events return only once committed and projected."""
        if event.kind == "tick":
            await self.queue.put(_Queued(event, None))
            return
        ack: asyncio.Future[Event] = asyncio.get_running_loop().create_future()
        await self.queue.put(_Queued(event, ack))
        await ack

    async def cursor(self, body: str) -> Any | None:
        return await store_cursor(self.conn, body)

    def register_body(self, body: Any) -> None:
        name = getattr(body, "name", None)
        if not name:
            raise ValueError("A body must expose a name")
        self.bodies[name] = body

    async def _append(self, event: Event, kind: str, payload: dict[str, Any], *,
                      caused_by: int | None = None, project: bool = True) -> Event:
        async with self.conn.transaction():
            stored = await append_event(self.conn, ts=self.clock.now(), kind=kind, who=event.who,
                                        place=event.place, body=event.body, payload=payload, caused_by=caused_by)
            if project:
                await project_event(self.conn, stored)
        return stored

    async def _settle(self, item: _Queued) -> Event:
        """Record, acknowledge the source, then let expectations and the gate decide."""
        try:
            stored = (item.event if item.event.kind == "tick" else
                      await self._append(item.event, item.event.kind, item.event.payload,
                                         caused_by=item.event.caused_by))
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
        """Fire due expectations first; a plain message reaches the gate only after."""
        fired = await expect.due(self.conn, event, now=self.clock.now(), soul=self.soul)
        for item in fired:
            await self._expect_wake(item)
        if fired or self.pfc is None or event.kind != "msg.in":
            return
        verdict = await self.gate.judge(self.conn, event, now=self.clock.now())
        if verdict.decision != "wake":
            await self._emit(event, f"decision.{verdict.decision}", verdict.payload())
            return
        await self._think(event, verdict.reason, event.place or "")

    async def _think(self, event: Event, reason: str, place: str) -> None:
        recent = await read_last_events(self.conn, place, limit=20)
        context = PFCContext(
            now=self.clock.now(), place=place, place_kind=str(event.payload.get("place_kind") or "direct"),
            reason=reason, core_memory=await self._core_memory(),
            expectations=await active_expectation_lines(self.conn, place),
            recent=[e for e in recent if e.id != event.id], trigger=event,
            last_interaction=await self._last_interaction(event))
        for call in await self.pfc.decide(context):
            await self._apply(event, call)

    async def _expect_wake(self, item: dict[str, Any]) -> None:
        """Wake the PFC once for one fired expectation, then settle it."""
        now = self.clock.now()
        trigger = item.get("trigger") or Event(
            id=item["fired_event"], ts=now, kind="expectation.fired", who=item["who"], place=item["place"],
            body=item["source_body"], payload={"expectation_id": item["id"], "about": item["about"]})
        if self.pfc is None:
            return
        verdict = await self.gate.judge(self.conn, trigger, now=now, surprise=float(item["weight"]))
        if verdict.decision != "wake":
            await self._emit(trigger, f"decision.{verdict.decision}", verdict.payload())
        else:
            reason = (f"expectation missed: {item['about']} "
                      f"(intent: {item['intent'] or 'none'}); {verdict.reason}")
            await self._think(trigger, reason, trigger.place or "")
        await expect.settle(self.conn, item["id"], now=now, soul=self.soul)

    async def recover(self) -> None:
        """Startup: resolve answered fired expectations, wake the PFC once for the rest."""
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
        elif call.name == "expect":
            await expect.create(self.conn, args, now=self.clock.now(), soul=self.soul, source=event)
        elif call.name == "resolve":
            await expect.resolve(self.conn, int(args["id"]), str(args["status"]), now=self.clock.now())
        else:
            await self._record_error(event, f"invalid tool call: {call.name} {args}")

    async def _emit(self, trigger: Event, kind: str, payload: dict[str, Any]) -> Event:
        return await self._append(trigger, kind, payload, caused_by=trigger.id)

    async def _act(self, event: Event, kind: str, payload: dict[str, Any]) -> None:
        body = self.bodies.get(event.body or "")
        if body is None:
            raise RuntimeError(f"No body registered for {event.body!r}")
        await body.act(Action(kind=kind, place=event.place or "", payload=payload, body=event.body))

    async def _record_error(self, event: Event, exc: BaseException | str) -> None:
        try:
            await self._append(event, "error", {"error": str(exc), "event_id": event.id},
                               caused_by=event.id, project=False)
        except Exception:
            pass  # the Log itself is unreachable; nothing left to record.

    async def step(self) -> Event:
        return await self._settle(await self.queue.get())

    async def run_until_idle(self) -> int:
        count = 0
        while not self.queue.empty():
            await self.step()
            count += 1
        return count

    async def run(self, stop_event: asyncio.Event) -> None:
        """Run the single consumer loop until stopped."""
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
