"""The kernel loop: one queue, one consumer, one event at a time."""

from __future__ import annotations

import asyncio
from collections.abc import Awaitable, Callable, Mapping
from dataclasses import dataclass, replace
from functools import partial
from typing import Any

from monoagens.core import dispatch, expectations, hooks, jobs, memory, sends, threads
from monoagens.core.context import layout, working
from monoagens.core.gate import Appraisal, appraise, validate_signals
from monoagens.core.reasoning import Reasoning, ToolCall
from monoagens.infra import budget
from monoagens.infra.clock import Clock
from monoagens.infra.prior import load_settings, signal_weights
from monoagens.infra.store import (append_event, cursor as store_cursor, insert_event_vector,
                                   project_event)
from monoagens.types import Event

NOT_INSTALLED = ("the middleware is not installed: build the kernel with "
                 "monoagens.boot.build()")


@dataclass(frozen=True, slots=True)
class _Queued:
    event: Event
    ack: asyncio.Future[Event] | None


class Kernel:
    """The one loop that records events, wakes the reasoning, and acts through tools."""

    def __init__(self, conn, clock: Clock | None = None, pfc: Reasoning | None = None,
                 settings: dict[str, Any] | None = None, embedder: Any = None) -> None:
        """Wire the kernel to its database connection and collaborators.

        The four values the middleware owns start empty: `monoagens.boot.build`
        installs them, and the loop refuses to run until it has.
        """
        self.conn, self.clock, self.pfc = conn, clock or Clock(), pfc
        self.settings = dict(settings) if settings is not None else load_settings()
        self.embedder = embedder
        self.queue: asyncio.Queue[_Queued] = asyncio.Queue()
        self.jobs = jobs.JobRunner(self)
        self.tools: Any | None = None
        self.intake: Callable[[Event], Awaitable[Event]] | None = None
        self.budget_limit: float = 0.0
        self.after_model_call: Callable[[int, Mapping[str, int]], Awaitable[None]] | None = None
        self.hooks = hooks.HookRegistry(self.settings)
        validate_signals(signal_weights(self.settings))

    def require_installed(self) -> None:
        """Refuse to run half-wired: no registry, no intake or no owner policy is fatal."""
        if self.tools is None or self.intake is None:
            raise RuntimeError(NOT_INSTALLED)
        self.hooks.require_policy()

    async def put(self, event: Event) -> Event | None:
        """Accept one event from an adapter or the clock (ticks are not acknowledged)."""
        if self.intake is None:
            raise RuntimeError(NOT_INSTALLED)
        event = await self.intake(event)
        ack = None if event.kind == "tick" else asyncio.get_running_loop().create_future()
        await self.queue.put(_Queued(event, ack))
        return await ack if ack is not None else None

    async def cursor(self, body: str) -> Any | None:
        """Return the last recorded cursor for an adapter."""
        return await store_cursor(self.conn, body)

    def enqueue(self, event: Event) -> None:
        """Queue an internal event without waiting for an intake acknowledgement."""
        self.queue.put_nowait(_Queued(event, None))

    def register_adapter(self, adapter: Any) -> None:
        """Register an adapter and the capabilities it declares."""
        if self.tools is None:
            raise RuntimeError("the tool registry is not installed")
        self.tools.register(adapter)

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
                                               tool=stored.body)
                if stored is None:
                    return item.event
            await self._wake(stored)
        except Exception as exc:
            await self._record_error(stored, exc)
        finally:
            self.queue.task_done()
        return stored

    async def _run_hooks(self, name: str, item: Any, ctx: Any, trigger: Event, *,
                         tool: str | None = None) -> Any | None:
        async def stopped(reason: str) -> None:
            await self._emit(trigger, "guard.stopped", {"hook": name, "reason": reason})

        async def failed(exc: Exception) -> None:
            await self._record_error(trigger, exc)
            await stopped(str(exc))

        return await self.hooks.get(name).run(item, ctx, tool=tool, on_stop=stopped,
                                              on_error=failed)

    async def _appraise(self, event: Event) -> Appraisal:
        return await appraise(event, conn=self.conn, now=self.clock.now(),
                              settings=self.settings, embedder=self.embedder,
                              daily_limit=self.budget_limit)

    async def _wake(self, event: Event) -> None:
        if event.kind == "job.result":
            await self._finish_job(event)
            return
        if event.kind == "intent":
            if self.pfc is not None:
                await self._think(event, str(event.payload.get("description") or ""),
                                  event.place or "")
            return
        fired = await expectations.due(self.conn, event, now=self.clock.now(),
                                  settings=self.settings, pfc=self.pfc,
                                  embedder=self.embedder)
        for item in fired:
            await self._expect_wake(item)
        if event.kind == "tick" and self.pfc is not None and self.embedder is not None:
            now = self.clock.now()
            if await memory.sleep_due(self.conn, now, self.settings):
                await self.jobs.start("sleep", "sleep", event,
                                      partial(self.jobs.sleep, now))
        if fired or self.pfc is None or event.kind not in ("percept", "act.result"):
            return
        if event.kind == "act.result" and await working.step_spoke(self.conn, event):
            await self._emit(event, "decision.silent",
                             {"reason": "a message already ended this step"})
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
            if await expectations.create_wait(self.conn, event, now=self.clock.now(),
                                        settings=self.settings):
                return
        await self._think(event, appraisal.reason, event.place or "")

    async def _think(self, event: Event, reason: str, place: str) -> None:
        """Lay out the desk, pass the context hook, start one reasoning job per place."""
        place = await threads.place_for(self.conn, event, place) or place
        context = await layout.build_context(
            self.conn, event, place, reason, now=self.clock.now(), settings=self.settings,
            memories=await memory.candidates(self.conn, self.embedder, event,
                                             now=self.clock.now(), settings=self.settings),
            capabilities=self.tools.descriptors())
        ctx = {"conn": self.conn, "event": event, "now": self.clock.now()}
        context = await self._run_hooks("context", context, ctx, event, tool=event.body)
        if context is None:
            return
        key = f"pfc:{place}"
        rounds = int(self.settings["constants"]["max_thought_rounds"])
        if not await self.jobs.start("pfc", key, event,
                                     partial(jobs.decide, self.pfc, context, rounds,
                                             self.clock, self.settings),
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
                call, totals = await budget.record_call(
                    self.conn, job.trigger.id, self.pfc, purpose="pfc.decide",
                    now=self.clock.now(), settings=self.settings, usage=item.get("usage"))
                if call is not None and self.after_model_call is not None:
                    await self.after_model_call(call.id, totals)
            calls = [call for item in rounds for call in item.get("calls") or []]
            acts = await jobs.apply_calls(self, job.trigger, calls)
            await threads.after_step(self, job, event, acts)
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
        reminded = trigger.kind == "expectation.reminded"
        place = str(item.get("place") or trigger.place or "")
        woken = replace(trigger, place=place) if reminded else trigger
        appraisal = await self._appraise(trigger)
        if appraisal.decision != "wake":
            await self._emit(trigger, f"decision.{appraisal.decision}", appraisal.payload())
        else:
            if reminded:
                condition = str(item.get("condition_text") or "")
                reason = (f"{appraisal.reason}: {item['about']} (condition: {condition}); "
                          "does this line fulfil it?")
            else:
                reason = (f"expectation missed: {item['about']} "
                          f"(intent: {item['intent'] or 'none'}); {appraisal.reason}")
            await self._think(woken, reason, place)
        if not reminded:
            await expectations.settle(self.conn, item["id"], now=now, settings=self.settings)

    async def recover(self) -> None:
        """Resend what was recorded but never confirmed, then wake for fired expectations."""
        await sends.resume(self)
        for item in await expectations.recovery(self.conn, now=self.clock.now()):
            await self._expect_wake(item)

    async def _apply(self, event: Event, call: ToolCall,
                     said: set[str] | None = None) -> dispatch.Dispatch:
        args = call.arguments
        if call.name == "think":
            await self._emit(event, "decision.note", {"act": "think", **args})
        elif call.name == "act":
            return await dispatch.dispatch(self, event, args, said=said)
        elif call.name == "expect":
            await expectations.create(self.conn, args, now=self.clock.now(),
                                settings=self.settings, source=event)
        elif call.name == "resolve":
            await expectations.resolve(self.conn, int(args["id"]), str(args["status"]),
                                 now=self.clock.now())
        elif call.name == "revise":
            await self._emit(event, "decision.note", {"act": "revise", **args})
        else:
            await self._record_error(event, f"invalid tool call: {call.name} {args}")
        return dispatch.Dispatch(acted=False)

    async def _emit(self, trigger: Event, kind: str, payload: dict[str, Any], *,
                    place: str | None = None) -> Event:
        return await self._append(trigger, kind, payload, caused_by=trigger.id, place=place)

    async def _record_error(self, event: Event, exc: BaseException | str) -> None:
        try:
            text = jobs.describe_failure(exc) if isinstance(exc, BaseException) else str(exc)
            await self._append(event, "error", {"error": text, "event_id": event.id},
                               caused_by=event.id, project=False)
        except Exception:
            pass  # the Log itself is unreachable; nothing left to record.

    async def step(self) -> Event:
        """Process exactly one queued event."""
        self.require_installed()
        return await self._settle(await self.queue.get())

    async def run_until_idle(self) -> int:
        """Process every queued event and every in-flight background job."""
        self.require_installed()
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
        self.require_installed()
        await self.recover()
        while not stop_event.is_set():
            try:
                await self._settle(await asyncio.wait_for(self.queue.get(), timeout=0.1))
            except Exception:
                continue  # an empty queue or one broken event never kills the loop.
