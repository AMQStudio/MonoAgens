"""The kernel loop: one queue, one consumer, one event at a time."""

from __future__ import annotations

import asyncio
from dataclasses import dataclass
from datetime import datetime
from typing import Any

from monoagens import expect, memory
from monoagens.clock import Clock
from monoagens.cognition import Appraisal, appraise, validate_signals
from monoagens.config import load_soul
from monoagens.middleware import HookRegistry
from monoagens.organs import OrganRegistry
from monoagens.pfc import PFC, PFCContext, ToolCall
from monoagens.store import (active_expectation_lines, add_budget_usage, append_event,
                             cursor as store_cursor, insert_event_vector, project_event,
                             read_last_events, read_last_events_elsewhere, read_state)
from monoagens.types import RECENT_KINDS, Event


@dataclass(frozen=True, slots=True)
class _Queued:
    event: Event
    ack: asyncio.Future[Event] | None


class Kernel:
    """The one loop that records events, wakes the PFC, and acts through bodies."""

    def __init__(self, conn, clock: Clock | None = None, pfc: PFC | None = None,
                 soul: dict[str, Any] | None = None, embedder: Any = None) -> None:
        """Wire the kernel to its database connection and collaborators."""
        self.conn, self.clock, self.pfc = conn, clock or Clock(), pfc
        self.soul = dict(soul) if soul is not None else load_soul()
        self.embedder = embedder
        self.queue: asyncio.Queue[_Queued] = asyncio.Queue()
        self.organs = OrganRegistry()
        self.hooks = HookRegistry()
        validate_signals((self.soul.get("cognition") or {}).get("weights") or {})

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
        """Register a body and the capabilities it declares."""
        self.organs.register(body)

    async def _append(self, event: Event, kind: str, payload: dict[str, Any], *,
                      caused_by: int | None = None, project: bool = True) -> Event:
        async with self.conn.transaction():
            stored = await append_event(self.conn, ts=self.clock.now(), kind=kind, who=event.who,
                                        place=event.place, body=event.body, payload=payload,
                                        caused_by=caused_by)
            if project:
                await project_event(self.conn, stored)
        return stored

    async def _encode(self, event: Event, text: str | None = None) -> None:
        text = text or str(event.payload.get("description") or event.payload.get("text") or "")
        if self.embedder is None or event.id is None or not text.strip():
            return
        vector = (await self.embedder.embed([text]))[0]
        await insert_event_vector(self.conn, event.id, vector)

    async def _settle(self, item: _Queued) -> Event:
        try:
            stored = item.event if item.event.kind == "tick" else await self._append(
                item.event, item.event.kind, item.event.payload, caused_by=item.event.caused_by)
        except BaseException as exc:
            if item.ack is not None and not item.ack.done():
                item.ack.set_exception(exc)
            self.queue.task_done()
            raise
        if item.ack is not None and not item.ack.done():
            item.ack.set_result(stored)
        try:
            await self._encode(stored)
            if stored.kind != "tick":
                ctx = {"conn": self.conn, "now": self.clock.now()}
                stored = await self._run_hooks("inbound", stored, ctx, stored,
                                               organ=stored.body)
                if stored is None:
                    return item.event
            await self._wake(stored)
        except Exception as exc:
            await self._record_error(stored, exc)
        finally:
            self.queue.task_done()
        return stored

    async def _run_hooks(self, name: str, item: Any, ctx: Any, trigger: Event, *,
                         organ: str | None = None) -> Any | None:
        async def stopped(reason: str) -> None:
            await self._emit(trigger, "guard.stopped", {"hook": name, "reason": reason})

        async def failed(exc: Exception) -> None:
            await self._record_error(trigger, exc)
            await stopped(str(exc))

        return await self.hooks.get(name).run(item, ctx, organ=organ, on_stop=stopped,
                                              on_error=failed)

    async def _appraise(self, event: Event) -> Appraisal:
        """Ask the cognition layer for one decision (no LLM)."""
        return await appraise(event, conn=self.conn, now=self.clock.now(), soul=self.soul)

    async def _wake(self, event: Event) -> None:
        fired = await expect.due(self.conn, event, now=self.clock.now(), soul=self.soul)
        for item in fired:
            await self._expect_wake(item)
        if event.kind == "tick" and self.pfc is not None and self.embedder is not None:
            now = self.clock.now()
            if await memory.sleep_due(self.conn, now, self.soul):
                await memory.sleep(self.conn, pfc=self.pfc, embedder=self.embedder,
                                   now=now, soul=self.soul)
        if fired or self.pfc is None or event.kind != "percept":
            return
        appraisal = await self._appraise(event)
        if appraisal.decision != "wake":
            await self._emit(event, f"decision.{appraisal.decision}", appraisal.payload())
            return
        await self._think(event, appraisal.reason, event.place or "")

    async def _think(self, event: Event, reason: str, place: str) -> None:
        memory_soul = self.soul.get("memory") or {}
        recent = await read_last_events(self.conn, place,
                                        limit=int(memory_soul.get("recent_events", 20)),
                                        kinds=RECENT_KINDS)
        recent_elsewhere = await read_last_events_elsewhere(
            self.conn, event.who, place, limit=int(memory_soul.get("recent_elsewhere", 10)),
            kinds=RECENT_KINDS) if event.who else []
        core = await read_state(self.conn, "core_memory")
        last = None
        for key in (f"last_spoke:{place}", f"last_seen:{event.who}"):
            value = await read_state(self.conn, key)
            if value and value.get("ts"):
                last = datetime.fromisoformat(value["ts"])
                break
        context = PFCContext(
            now=self.clock.now(), place=place,
            place_kind=str(event.payload.get("place_kind") or "direct"), reason=reason,
            core_memory=str((core or {}).get("text") or ""),
            expectations=await active_expectation_lines(self.conn, place),
            memories=await self._memories(event),
            recent=[seen for seen in recent if seen.id != event.id],
            recent_elsewhere=recent_elsewhere, trigger=event, last_interaction=last,
            capabilities=self.organs.descriptors())
        ctx = {"conn": self.conn, "event": event, "now": self.clock.now()}
        context = await self._run_hooks("context", context, ctx, event, organ=event.body)
        if context is None:
            return
        calls = await self.pfc.decide(context)
        usage = getattr(self.pfc, "last_usage", None)
        if usage and any(usage.values()) and event.id is not None:
            await add_budget_usage(self.conn, now=self.clock.now(),
                                   timezone=str(self.soul.get("timezone") or "UTC"),
                                   usage=usage, updated_by_event=event.id)
        acted, thought = False, ""
        for call in calls:
            acted = await self._apply(event, call) or acted
            if call.name == "think":
                thought = str(call.arguments.get("thought") or "")
        if not acted:
            await self._emit(event, "decision.silent", {"reason": thought or "no act"})

    async def _memories(self, event: Event) -> tuple[str, ...]:
        query = str(event.payload.get("description") or event.payload.get("text")
                    or event.payload.get("about") or "")
        if self.embedder is None or not query.strip():
            return ()
        k = int((self.soul.get("memory") or {}).get("retrieve_k", 5))
        return tuple(await memory.retrieve(self.conn, self.embedder, query, k=k,
                                           now=self.clock.now(), event_id=event.id))

    async def _expect_wake(self, item: dict[str, Any]) -> None:
        now = self.clock.now()
        trigger = item.get("trigger") or Event(
            id=item["fired_event"], ts=now, kind="expectation.fired", who=item["who"],
            place=item["place"], body=item["source_body"],
            payload={"expectation_id": item["id"], "about": item["about"]})
        if self.pfc is None:
            return
        appraisal = await self._appraise(trigger)
        if appraisal.decision != "wake":
            await self._emit(trigger, f"decision.{appraisal.decision}", appraisal.payload())
        else:
            reason = (f"expectation missed: {item['about']} "
                      f"(intent: {item['intent'] or 'none'}); {appraisal.reason}")
            await self._think(trigger, reason, trigger.place or "")
        await expect.settle(self.conn, item["id"], now=now, soul=self.soul)

    async def recover(self) -> None:
        """Wake once more for expectations left fired across a restart."""
        for item in await expect.recovery(self.conn, now=self.clock.now()):
            await self._expect_wake(item)

    async def _apply(self, event: Event, call: ToolCall) -> bool:
        args = call.arguments
        if call.name == "think":
            await self._emit(event, "decision.note", {"act": "think", **args})
        elif call.name == "act":
            return await self._act_call(event, args)
        elif call.name == "expect":
            await expect.create(self.conn, args, now=self.clock.now(), soul=self.soul,
                                source=event)
        elif call.name == "resolve":
            await expect.resolve(self.conn, int(args["id"]), str(args["status"]),
                                 now=self.clock.now())
        elif call.name == "revise":
            await self._emit(event, "decision.note", {"act": "revise", **args})
        else:
            await self._record_error(event, f"invalid tool call: {call.name} {args}")
        return False

    async def _act_call(self, event: Event, args: dict[str, Any]) -> bool:
        capability = str(args.get("capability") or "")
        if capability not in self.organs.capabilities:
            await self._record_error(event, f"unknown capability: {capability!r}")
            return False
        arguments = args.get("arguments")
        arguments = arguments if isinstance(arguments, dict) else {}
        stored = await self._emit(event, "act.out",
                                  {"capability": capability, "arguments": arguments})
        await self._encode(stored, str(arguments.get("text") or "") or None)
        ctx = {"conn": self.conn, "event": event, "now": self.clock.now(),
               "capability": capability}
        payload = await self._run_hooks("action", arguments, ctx, event,
                                        organ=capability.partition(".")[0])
        if payload is not None:
            result = await self.organs.act(event, capability, payload)
            if result is not None:
                await self._emit(event, "act.result",
                                 {"capability": capability, "result": result})
        return True

    async def _emit(self, trigger: Event, kind: str, payload: dict[str, Any]) -> Event:
        return await self._append(trigger, kind, payload, caused_by=trigger.id)

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
                await self._settle(await asyncio.wait_for(self.queue.get(), timeout=0.1))
            except Exception:
                continue  # an empty queue or one broken event never kills the loop.
