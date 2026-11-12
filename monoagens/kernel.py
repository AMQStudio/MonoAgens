"""The kernel loop: one queue, one consumer, one event at a time."""

from __future__ import annotations

import asyncio
from dataclasses import dataclass
from functools import partial
from typing import Any

from monoagens import budget, expect, hooks, identity, jobs, memory, organs, pairing, reach, threads
from monoagens.clock import Clock
from monoagens.cognition import Appraisal, appraise, validate_signals
from monoagens.prior import load_settings, signal_weights
from monoagens.organs import OrganRegistry
from monoagens.pfc import PFC, ToolCall
from monoagens.store import (append_event, cursor as store_cursor, insert_event_vector,
                             project_event)
from monoagens.types import Event


@dataclass(frozen=True, slots=True)
class _Queued:
    event: Event
    ack: asyncio.Future[Event] | None


class Kernel:
    """The one loop that records events, wakes the PFC, and acts through bodies."""

    def __init__(self, conn, clock: Clock | None = None, pfc: PFC | None = None,
                 settings: dict[str, Any] | None = None, embedder: Any = None) -> None:
        """Wire the kernel to its database connection and collaborators."""
        self.conn, self.clock, self.pfc = conn, clock or Clock(), pfc
        self.settings = dict(settings) if settings is not None else load_settings()
        self.embedder = embedder
        self.queue: asyncio.Queue[_Queued] = asyncio.Queue()
        self.jobs = jobs.JobRunner(self)
        self.organs = OrganRegistry()
        self.organs.register(identity.SelfOrgan(conn, self.clock))
        self.organs.add_capability(reach.SHARE, reach.SHARE_CAPABILITY)
        self.hooks = hooks.HookRegistry(self.settings)
        for pipeline in self.hooks:
            pipeline.set_policy(hooks.policy_for(pipeline.name))
        validate_signals(signal_weights(self.settings))

    async def put(self, event: Event) -> Event | None:
        """Accept one event from a body or the clock (ticks are not acknowledged)."""
        event = await pairing.as_owner(self.conn, event)
        ack = None if event.kind == "tick" else asyncio.get_running_loop().create_future()
        await self.queue.put(_Queued(event, ack))
        return await ack if ack is not None else None

    async def cursor(self, body: str) -> Any | None:
        """Return the last recorded cursor for a body."""
        return await store_cursor(self.conn, body)

    def enqueue(self, event: Event) -> None:
        """Queue an internal event without waiting for an intake acknowledgement."""
        self.queue.put_nowait(_Queued(event, None))

    def register_body(self, body: Any) -> None:
        """Register a body and the capabilities it declares."""
        self.organs.register(body)

    async def _append(self, event: Event, kind: str, payload: dict[str, Any], *,
                      caused_by: int | None = None, project: bool = True,
                      place: str | None = None) -> Event:
        async with self.conn.transaction():
            stored = await append_event(self.conn, ts=self.clock.now(), kind=kind, who=event.who,
                                        place=place or event.place, body=event.body,
                                        payload=payload, caused_by=caused_by,
                                        origin_actor=event.origin_actor)
            if project:
                await project_event(self.conn, stored)
        return stored

    async def _encode(self, event: Event, text: str | None = None) -> None:
        text = text or str(event.payload.get("description") or event.payload.get("text") or "")
        if self.embedder is None or event.id is None or not text.strip():
            return
        await self.jobs.start("encode", f"encode:{event.id}", event,
                              partial(jobs.encode, self.embedder, event.id, text),
                              place=event.place or "")

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
            if stored.kind != "tick" and stored.kind not in jobs.INTERNAL_KINDS:
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
        return await appraise(event, conn=self.conn, now=self.clock.now(),
                              settings=self.settings, embedder=self.embedder)

    async def _wake(self, event: Event) -> None:
        if event.kind == "job.result":
            await self._finish_job(event)
            return
        if event.kind == "intent":
            if self.pfc is not None:
                await self._think(event, str(event.payload.get("description") or ""),
                                  event.place or "")
            return
        fired = await expect.due(self.conn, event, now=self.clock.now(),
                                  settings=self.settings, kernel=self)
        for item in fired:
            await self._expect_wake(item)
        if event.kind == "tick" and self.pfc is not None and self.embedder is not None:
            now = self.clock.now()
            if await memory.sleep_due(self.conn, now, self.settings):
                await self.jobs.start("sleep", "sleep", event,
                                      partial(self.jobs.sleep, now))
        if fired or self.pfc is None or event.kind not in ("percept", "act.result"):
            return
        appraisal = await self._appraise(event)
        if event.kind == "act.result":
            if appraisal.decision == "wake":
                await self._think(event, appraisal.reason, event.place or "")
            return
        if appraisal.decision != "wake":
            await self._emit(event, f"decision.{appraisal.decision}", appraisal.payload())
            return
        if appraisal.reflex is None:
            await self._emit(event, "decision.wake", appraisal.payload())
            if await expect.create_wait(self.conn, event, now=self.clock.now(),
                                        settings=self.settings):
                return
        await self._think(event, appraisal.reason, event.place or "")

    async def _think(self, event: Event, reason: str, place: str) -> None:
        """Lay out the desk, pass the context hook, start one PFC job per place."""
        place = await threads.place_for(self.conn, event, place) or place
        context = await memory.build_context(
            self.conn, event, place, reason, now=self.clock.now(), settings=self.settings,
            memories=await memory.candidates(self.conn, self.embedder, event,
                                             now=self.clock.now(), settings=self.settings),
            capabilities=self.organs.descriptors())
        ctx = {"conn": self.conn, "event": event, "now": self.clock.now()}
        context = await self._run_hooks("context", context, ctx, event, organ=event.body)
        if context is None:
            return
        key = f"pfc:{place}"
        rounds = int(self.settings["constants"]["max_thought_rounds"])
        if not await self.jobs.start("pfc", key, event,
                                     partial(jobs.decide, self, self.pfc, context, rounds),
                                     place=place):
            self.jobs.ask_re_wake(key, event, reason, place)

    async def _finish_job(self, event: Event) -> None:
        """Apply one job result, then re-wake a place that asked while it ran."""
        job = self.jobs.finish(event)
        if job is None:
            return
        result = job.outcome.get("result") or {}
        if "error" in job.outcome:
            await self._record_error(job.trigger, str(job.outcome["error"]))
            if job.name == "sleep":
                await self._emit(job.trigger, "sleep.failed", {"error": str(job.outcome["error"])})
        elif job.name == "pfc":
            rounds = result.get("rounds") or []
            for item in rounds:
                await budget.record_call(self.conn, job.trigger.id, self.pfc,
                                         purpose="pfc.decide", now=self.clock.now(),
                                         settings=self.settings, usage=item.get("usage"))
            calls = [call for item in rounds for call in item.get("calls") or []]
            await jobs.apply_calls(self, job.trigger, calls)
            await threads.after_step(self, job, event, calls)
        elif job.name == "encode" and isinstance(result.get("vector"), list):
            await insert_event_vector(self.conn, int(result["event_id"]), result["vector"])
        if job.re_wake is not None:
            await self._think(*job.re_wake)

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
        await expect.settle(self.conn, item["id"], now=now, settings=self.settings)

    async def recover(self) -> None:
        """Wake once more for expectations left fired across a restart."""
        for item in await expect.recovery(self.conn, now=self.clock.now()):
            await self._expect_wake(item)

    async def _apply(self, event: Event, call: ToolCall) -> bool:
        args = call.arguments
        if call.name == "think":
            await self._emit(event, "decision.note", {"act": "think", **args})
        elif call.name == "act":
            return await organs.dispatch(self, event, args)
        elif call.name == "expect":
            await expect.create(self.conn, args, now=self.clock.now(),
                                settings=self.settings, source=event)
        elif call.name == "resolve":
            await expect.resolve(self.conn, int(args["id"]), str(args["status"]),
                                 now=self.clock.now())
        elif call.name == "revise":
            await self._emit(event, "decision.note", {"act": "revise", **args})
        else:
            await self._record_error(event, f"invalid tool call: {call.name} {args}")
        return False

    async def _emit(self, trigger: Event, kind: str, payload: dict[str, Any], *,
                    place: str | None = None) -> Event:
        return await self._append(trigger, kind, payload, caused_by=trigger.id, place=place)

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
        """Process every queued event and every in-flight background job."""
        count = 0
        while True:
            while not self.queue.empty():
                await self.step()
                count += 1
            if not self.jobs.in_flight:
                return count
            pending = [task for task in self.jobs.tasks if not task.done()]
            await (asyncio.wait(pending) if pending else asyncio.sleep(0))

    async def run(self, stop_event: asyncio.Event) -> None:
        """Run the consumer loop until the stop event is set."""
        await self.recover()
        while not stop_event.is_set():
            try:
                await self._settle(await asyncio.wait_for(self.queue.get(), timeout=0.1))
            except Exception:
                continue  # an empty queue or one broken event never kills the loop.
