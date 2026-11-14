"""The kernel loop: one queue, one consumer, one event at a time."""

from __future__ import annotations

import asyncio
import contextlib
import logging
from collections import deque
from collections.abc import Awaitable, Callable, Mapping
from dataclasses import dataclass, replace
from datetime import datetime, timedelta
from functools import partial
from typing import Any

from monoagens.core import dispatch, expectations, hooks, jobs, memory, threads
from monoagens.core.context import layout, working
from monoagens.core.gate import Appraisal, appraise, validate_signals
from monoagens.core.reasoning import Reasoning, ToolCall
from monoagens.infra import budget, config, observability, versioning
from monoagens.infra.clock import Clock
from monoagens.infra.prior import load_settings, signal_weights
from monoagens import contract
from monoagens.contract import Percept, to_event
from monoagens.infra.store import (append_envelope, append_event, close_action,
                                   cursor as store_cursor, insert_event_vector,
                                   insert_event_vectors, merge_source, project_event,
                                   unembedded_imported_percepts, unresolved_actions,
                                   write_action)
from monoagens.types import Event

NOT_INSTALLED = ("the middleware is not installed: build the kernel with "
                 "monoagens.boot.build()")
LOGGER = logging.getLogger("kernel")

def _receipt_line(receipt: Any) -> str:
    """Return the one short human line a receipt is recorded with (DEC-4)."""
    return str(receipt.reason or _result_text(receipt.result) or receipt.state)


def _result_text(result: Any) -> str:
    """Return the words of one tool result, when it has any."""
    if isinstance(result, dict):
        return str(result.get("text") or "")
    return str(result or "")


def _clean_text(value: str | None) -> str | None:
    """Strip Postgres-rejected NUL bytes from error and identity fields."""
    if value is None:
        return None
    return str(value).replace("\x00", "")


@dataclass(frozen=True, slots=True)
class _Queued:
    event: Event
    ack: asyncio.Future[Event] | None
    recorded: bool = False


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
        self._loop: asyncio.AbstractEventLoop | None = None
        with contextlib.suppress(RuntimeError):
            self._loop = asyncio.get_running_loop()
        self.jobs = jobs.JobRunner(self)
        self.tools: Any | None = None
        self.intake: Callable[[Event], Awaitable[Event]] | None = None
        self._roles: Any = None
        self._body_bases: dict[str, str] = {}
        self.budget_limit: float = 0.0
        self.after_model_call: Callable[[int, Mapping[str, int]], Awaitable[None]] | None = None
        self.hooks = hooks.HookRegistry(self.settings)
        self._batches: dict[tuple[int, str], dict[str, Any]] = {}
        self._backlogs: dict[str, dict[str, Any]] = {}
        self._results_this_round = 0
        self._hook_acts: dict[int, int] = {}
        self._pending_embeds: deque[tuple[int, str]] = deque()
        self._embed_trigger: Event | None = None
        self._embed_batch_size: int = config.embed_batch_size(self.settings)
        self._backfill_settle_seconds: float = config.backfill_settle_seconds(self.settings)
        self._observer = observability.EventObserver(
            summary_every=config.import_summary_every(self.settings),
            conn=self.conn,
        )
        validate_signals(signal_weights(self.settings))

    def require_installed(self) -> None:
        """Refuse to run half-wired: no registry, no intake or no owner policy is fatal."""
        if self.tools is None or self.intake is None:
            raise RuntimeError(NOT_INSTALLED)
        self.hooks.require_policy()

    async def put(self, item: Percept | Event, *, body: str | None = None) -> Event | None:
        """Accept one percept or event from an adapter or the clock (ticks are not acked).

        A percept becomes a version 2 event here; an adapter that still builds a version 1
        event goes through the contract's one upcast (D-M). `body` names the adapter, which
        the loader binds per adapter from Part 4 on.
        """
        if self.intake is None:
            raise RuntimeError(NOT_INSTALLED)
        loop = asyncio.get_running_loop()
        self._loop = loop
        event = self._incoming(item, body)
        ack = None if event.kind == "tick" else loop.create_future()
        await self.queue.put(_Queued(event, ack))
        return await ack if ack is not None else None

    def _incoming(self, item: Percept | Event, body: str | None) -> Event:
        """Return the version 2 event one incoming percept or event becomes (contract.md §9)."""
        if isinstance(item, Percept):
            return to_event(item, body=body or "unknown", ts=self.clock.now())
        return versioning.upcast_incoming(item)

    async def act_via_hooks(self, trigger: Event, capability: str,
                            arguments: Mapping[str, Any]) -> dispatch.Dispatch:
        """Ask the action hook for one act the middleware itself decided to make (D-H).

        Command answers, approval prompts and brake prompts go through this one door, so the
        authority stays in the action hook and is never derived a second time.
        """
        return await dispatch.dispatch(self, trigger,
                                       {"capability": capability,
                                        "arguments": dict(arguments)})

    async def cursor(self, body: str) -> Any | None:
        """Return the last recorded cursor for an adapter."""
        return await store_cursor(self.conn, body)

    def enqueue(self, event: Event) -> None:
        """Queue an internal event safely from any thread without waiting for an intake ack.

        An event that already carries an id is already in the Log, so the consumer wakes on
        it without recording it a second time.
        """
        item = _Queued(event, None, recorded=event.id is not None)
        loop = self._loop
        if loop is not None and not loop.is_closed():
            try:
                running = asyncio.get_running_loop()
            except RuntimeError:
                running = None
            if running is not loop:
                loop.call_soon_threadsafe(self.queue.put_nowait, item)
                return
        self.queue.put_nowait(item)

    def put_threadsafe(self, event: Event) -> Any:
        """Submit one event onto the kernel queue from another thread cleanly."""
        loop = self._loop
        if loop is None or loop.is_closed():
            raise RuntimeError("kernel loop is not running")
        return asyncio.run_coroutine_threadsafe(self.put(event), loop)

    @property
    def roles(self) -> Any:
        """The role book, carrying every registered body's declared base tier (§3)."""
        return self._roles

    @roles.setter
    def roles(self, book: Any) -> None:
        self._roles = book.with_bases(self._body_bases) if book is not None else book

    def register_adapter(self, adapter: Any) -> None:
        """Register a body, the capabilities it declares and the base tier of each (§3)."""
        if self.tools is None:
            raise RuntimeError("the tool registry is not installed")
        self.tools.register(adapter, body=True)
        declared = getattr(adapter, "PERMISSIONS", None) or {}
        self._body_bases = {**self._body_bases,
                            **{f"{adapter.name}.{short}": str(tier)
                               for short, tier in declared.items()}}
        self.roles = self._roles

    async def _append(self, event: Event, kind: str, payload: dict[str, Any], *,
                      caused_by: int | None = None, project: bool = True,
                      place: str | None = None, description: str | None = None,
                      features: Mapping[str, float] | None = None,
                      backfill: bool = False, imported: bool = False,
                      labels: Mapping[str, Any] | None = None,
                      occurred_at: datetime | None = None,
                      action: Any = None, close: tuple[Any, Any] | None = None,
                      communicative: bool = False) -> Event:
        async with self.conn.transaction():
            stored = await append_event(self.conn, ts=self.clock.now(), kind=kind, who=event.who,
                                        place=place or event.place, body=event.body,
                                        payload=payload, caused_by=caused_by,
                                        origin_actor=event.origin_actor,
                                        occurred_at=occurred_at, description=description,
                                        features=features, backfill=backfill, imported=imported,
                                        labels=labels)
            if action is not None:
                await write_action(self.conn, action, int(stored.id), communicative=communicative)
            if close is not None:
                await close_action(self.conn, close[0].id, close[1], int(stored.id))
            await self._project(stored, project=project)
        self._observer.observe(stored)
        return stored

    async def _append_received(self, event: Event) -> Event:
        """Write one received event exactly as its adapter and the inbound hook left it."""
        async with self.conn.transaction():
            stored = await append_envelope(self.conn, event, ts=self.clock.now())
            await self._project(stored, project=True)
        self._observer.observe(stored)
        return stored

    async def _project(self, stored: Event, *, project: bool) -> None:
        """Project one stored event onto the Status Board, and apply a merge it records."""
        if not project:
            return
        await project_event(self.conn, stored)
        if stored.kind == "source.merged" and stored.id is not None:
            merged = await merge_source(stored)
            if merged is not None:
                await memory.apply_merge(self.conn, merged[0], merged[1], stored.id)

    async def _encode(self, event: Event, text: str | None = None) -> None:
        text = text or event.description
        if self.embedder is None or event.id is None or not text.strip():
            return
        if event.kind == "percept" and event.imported:
            self._pending_embeds.append((event.id, text))
            self._embed_trigger = event
            if len(self._pending_embeds) >= self._embed_batch_size:
                await self._flush_embed_batch()
            return
        await self.jobs.start("encode", f"encode:{event.id}", event,
                              partial(jobs.encode, self.embedder, event.id, text),
                              place=event.place or "")

    async def _flush_embed_batch(self, *, force: bool = False) -> None:
        if self.embedder is None or not self._pending_embeds or self.jobs.busy("embed_batch"):
            return
        if not force and len(self._pending_embeds) < self._embed_batch_size:
            return
        count = min(self._embed_batch_size, len(self._pending_embeds))
        batch = [self._pending_embeds.popleft() for _ in range(count)]
        trigger = self._embed_trigger or Event(
            id=batch[0][0], ts=self.clock.now(), kind="percept", who="", place="",
        )
        await self.jobs.start(
            "embed_batch",
            "embed_batch",
            trigger,
            partial(jobs.embed_batch, self.embedder, batch),
            place="",
        )

    async def _settle(self, item: _Queued) -> Event:
        if item.recorded:
            try:
                ctx = {"conn": self.conn, "now": self.clock.now()}
                await self._run_hooks("inbound", item.event, ctx, item.event,
                                      tool=item.event.body)
                await self._wake(item.event)
            except Exception as exc:
                await self._record_error(item.event, exc)
            finally:
                self.queue.task_done()
            return item.event
        resolved = item.event
        try:
            if item.event.kind == "tick":
                stored = item.event
            else:
                if self.intake is not None:
                    resolved = await self.intake(item.event)
                stored = await self._append_received(resolved)
        except BaseException as exc:
            if isinstance(exc, Exception) and item.event.kind != "error":
                await self._record_error(resolved, exc)
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
            if self._pending_embeds and self.queue.empty():
                await self._flush_embed_batch(force=True)
        except Exception as exc:
            await self._record_error(stored, exc)
        finally:
            self.queue.task_done()
        return stored

    async def _run_hooks(self, name: str, item: Any, ctx: Any, trigger: Event, *,
                         tool: str | None = None) -> Any | None:
        async def stopped(reason: str, held: bool = False) -> None:
            await self._emit(trigger, "guard.stopped",
                             {"hook": name, "reason": reason, "held": held})

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
        if event.kind == "act.result" and await self._batch_holds(event):
            return
        await self._flush_batches(None if event.kind == "tick" else (event.place or ""))
        if event.kind == "job.result":
            await self._finish_job(event)
            return
        if event.kind == "intent":
            if self.pfc is not None:
                await self._think(event, event.description, event.place or "")
            return
        if event.kind == "percept" and event.imported:
            return
        fired = await expectations.due(self.conn, event, now=self.clock.now(),
                                  settings=self.settings, pfc=self.pfc,
                                  embedder=self.embedder,
                                  on_error=partial(self._record_error, event))
        for item in fired:
            try:
                await self._expect_wake(item)
            except Exception as exc:
                await self._record_error(event, exc)
        if event.kind == "tick" and self.pfc is not None and self.embedder is not None:
            now = self.clock.now()
            if await memory.sleep_due(self.conn, now, self.settings):
                await self.jobs.start("sleep", "sleep", event,
                                      partial(self.jobs.sleep, now))
        if fired or self.pfc is None or event.kind not in ("percept", "act.result"):
            if event.kind == "tick" and self._backlogs:
                await self._flush_backlogs(due_only=True)
            return
        if event.kind == "percept" and event.backfill:
            self._queue_backfill(event)
            return
        if event.kind == "percept":
            await self._flush_backlogs(place=event.place or "")
        if event.kind == "act.result":
            if await self._decided(event):
                return
            if await threads.closed_reason(self.conn, event) in ("max_task_steps", "stopped"):
                await self._emit(event, "decision.silent",
                                 {"reason": "the task chain already stopped"})
                return
        await self._appraise_and_wake(event)

    async def _appraise_and_wake(self, event: Event) -> None:
        appraisal = await self._appraise(event)
        if event.kind == "act.result":
            if appraisal.decision == "wake":
                await self._think(event, appraisal.reason, event.place or "")
            return
        if appraisal.decision != "wake":
            await self._emit(event, f"decision.{appraisal.decision}", appraisal.record())
            return
        if appraisal.reflex is None:
            await self._emit(event, "decision.wake", appraisal.record())
            if await expectations.create_wait(self.conn, event, now=self.clock.now(),
                                        settings=self.settings):
                return
        else:
            self._observer.observe_decision(event, "decision.wake", appraisal.reason)
        await self._think(event, appraisal.reason, event.place or "")

    def _queue_backfill(self, event: Event) -> None:
        key = event.place or ""
        entry = self._backlogs.setdefault(key, {"events": [], "idle_polls": 0,
                                                "deadline": self.clock.now()})
        entry["events"].append(event)
        entry["idle_polls"] = 0
        entry["deadline"] = self.clock.now() + timedelta(seconds=self._backfill_settle_seconds)

    @staticmethod
    def _collapse_backlog(events: list[Event]) -> Event:
        """Collapse one place's back-fill into the one event the gate appraises (D37)."""
        addressed = [seen for seen in events if seen.features.get("addressed")]
        rep = addressed[-1] if addressed else events[-1]
        lines = [f"{seen.who or '?'}: {seen.description}" for seen in events]
        features = dict(rep.features)
        features["addressed"] = 1 if addressed else 0
        return replace(rep, description=" + ".join(lines) if len(lines) == 1
                       else "\n".join(lines), features=features, backfill=True)

    async def _flush_backlogs(self, *, place: str | None = None, due_only: bool = False,
                              idle_polls: int = 0) -> None:
        if not self._backlogs:
            return
        now = self.clock.now()
        poll_due = idle_polls * 0.1 >= self._backfill_settle_seconds
        keys = [
            key for key, entry in list(self._backlogs.items())
            if (place is None or key == place)
            and (not due_only or now >= entry["deadline"] or poll_due)
        ]
        if self.pfc is None:
            for key in keys:
                self._backlogs.pop(key, None)
            return
        for key in keys:
            entry = self._backlogs.pop(key, None)
            if not entry or not entry["events"]:
                continue
            collapsed = self._collapse_backlog(entry["events"])
            try:
                await self._appraise_and_wake(collapsed)
            except Exception as exc:
                await self._record_error(collapsed, exc)

    async def _batch_holds(self, event: Event) -> bool:
        """Hold one result of a round whose other results have not arrived yet.

        A round's results are awaited together, keyed by the round (the event that started
        the reasoning job and its place), so one round wakes reasoning once. A result from
        another round, place or thread never matches a key, so it never counts here.
        """
        if event.caused_by is None or not self._batches:
            return False
        candidates = [int(event.caused_by)]
        row = await (await self.conn.execute(
            "SELECT caused_by FROM events WHERE id = %s", (event.caused_by,))).fetchone()
        if row is not None and row[0] is not None:
            candidates.append(int(row[0]))
        key, batch = None, None
        for trigger_id in candidates:
            direct = (trigger_id, event.place or "")
            if direct in self._batches:
                key, batch = direct, self._batches[direct]
                break
            match = next(((k, b) for k, b in self._batches.items() if k[0] == trigger_id), None)
            if match is not None:
                key, batch = match
                break
        if batch is None or key is None:
            return False
        batch["remaining"] -= 1
        if batch["remaining"] > 0 and self.clock.now() < batch["deadline"]:
            return True
        self._batches.pop(key, None)
        return False

    async def _flush_batches(self, place: str | None = None) -> None:
        """Wake once for a round whose results are late; a result cannot stall a place.

        A tick carries no place of its own, so it flushes every place (a round whose result
        never arrives must not also wait for an unrelated event in its own place).
        """
        now = self.clock.now()
        due = [key for key, batch in self._batches.items()
               if (place is None or place in (key[1], batch["trigger"].place))
               and now >= batch["deadline"]]
        for key in due:
            batch = self._batches.pop(key)
            target = str(key[1])
            if target.startswith("space:task:") and not await threads.is_open(self.conn, target):
                continue
            await self._think(batch["trigger"], "the results of one round are in", target)

    def _track_batch(self, job: Any, acts: list[tuple[dict[str, Any], Any]]) -> None:
        """Remember how many results one finished round still awaits, and until when.

        Only a call that produces a result counts: a slow tool (it reports later by design
        and wakes as it always did), a message (complete once sent) and a capability the
        middleware handles itself (`share` records its own outcome) never hold a round open,
        so no place ever waits on a result that cannot arrive.
        """
        fast, slow = 0, 0
        for raw, _outcome in acts:
            capability = str((raw.get("arguments") or {}).get("capability") or "")
            declared = self.tools.get(capability)
            if declared is None:
                continue  # no such tool, so no result will come
            if not self.tools.produces_result(capability):
                continue  # the middleware handled it and recorded its own outcome
            if declared.get("communicative"):
                continue
            if declared.get("long_running"):
                slow += 1
            else:
                fast += 1
        results = fast + slow
        if results == 0 or (results == 1 and slow == 1):
            return
        values = self.settings.get("constants") or {}
        milliseconds = int(values.get("batch_wake_timeout_ms") or 2000)
        self._batches[(int(job.trigger.id or 0), str(job.place))] = {
            "trigger": job.trigger, "remaining": results,
            "deadline": self.clock.now() + timedelta(milliseconds=milliseconds)}

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
            acts = await jobs.apply_calls(self, job.trigger, rounds,
                                          dispatch.JobRef(job.key, job.started_event_id))
            self._track_batch(job, acts)
            await threads.after_step(self, job, event, acts)
        elif job.name == "encode" and isinstance(result.get("vector"), list):
            event_id = int(result["event_id"])
            async with self.conn.transaction():
                row = await (await self.conn.execute(
                    "SELECT 1 FROM events WHERE id = %s "
                    "AND payload <> '{\"forgotten\": true}'::jsonb FOR SHARE",
                    (event_id,))).fetchone()
                if row is not None:
                    await insert_event_vector(self.conn, event_id, result["vector"])
        elif job.name == "embed_batch":
            items = result.get("items") or []
            if items:
                batch_ids = sorted({int(eid) for eid, _ in items})
                async with self.conn.transaction():
                    rows = await (await self.conn.execute(
                        "SELECT id FROM events WHERE id = ANY(%s) "
                        "AND payload <> '{\"forgotten\": true}'::jsonb FOR SHARE",
                        (batch_ids,))).fetchall()
                    alive = {int(r[0]) for r in rows}
                    kept = [(eid, vec) for eid, vec in items if int(eid) in alive]
                    if kept:
                        await insert_event_vectors(self.conn, kept)
            if self._pending_embeds:
                await self._flush_embed_batch(force=self.queue.empty())
        if job.re_wake is not None:
            re_place = str(job.re_wake[2])
            if not (re_place.startswith("space:task:") and not await threads.is_open(
                    self.conn, re_place)):
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
            await self._emit(trigger, f"decision.{appraisal.decision}", appraisal.record())
        else:
            if reminded:
                condition = str(item.get("condition_text") or "")
                # The line names neither the expectation's words nor its source: this place's
                # audience decides, through the desk, what it may be told (D45).
                reason = (f"this line may relate to an expectation of yours; "
                          f"condition: {condition}; does this line fulfil it?")
            else:
                # The reason names the expectation, never its words: this place's audience may
                # not be allowed to see them, so the desk decides, not this line (D45).
                reason = (f"expectation #{item['id']} is due here; what it was about is on the "
                          f"desk only if this place may see it; {appraisal.reason}")
            await self._think(woken, reason, place)
        if not reminded:
            await expectations.settle(self.conn, item["id"], now=now, settings=self.settings)

    async def recover(self) -> None:
        """Resend what was recorded but never confirmed, then wake for fired expectations."""
        now = self.clock.now()
        try:
            await self._resume_actions(now)
        except Exception as exc:
            await self._record_error(Event(id=None, ts=now, kind="tick"), exc)
        try:
            recovered = await expectations.recovery(self.conn, now=now)
        except Exception as exc:
            await self._record_error(Event(id=None, ts=now, kind="tick"), exc)
            recovered = []
        for item in recovered:
            try:
                await self._expect_wake(item)
            except Exception as exc:
                err_ev = item.get("trigger") or Event(
                    id=item.get("fired_event"), ts=now, kind="expectation.fired",
                    who=item.get("who"), place=item.get("place"),
                    body=item.get("source_body"))
                await self._record_error(err_ev, exc)
        if self.embedder is not None:
            try:
                known = {eid for eid, _ in self._pending_embeds}
                missing = [item for item in await unembedded_imported_percepts(self.conn)
                           if item[0] not in known]
                if missing:
                    self._pending_embeds.extend(missing)
                    await self._flush_embed_batch(force=True)
            except Exception as exc:
                await self._record_error(Event(id=None, ts=now, kind="tick"), exc)

    async def _resume_actions(self, now: datetime) -> int:
        """Hand every action without a receipt back to its body with the same id (DEC-6)."""
        count = 0
        for row in await unresolved_actions(self.conn):
            placed = str(row.get("place") or "")
            act = dispatch.Act(
                action=contract.Action(id=str(row["id"]),
                                       capability=str(row.get("capability") or ""),
                                       place=placed,
                                       arguments=dict(row.get("arguments") or {}),
                                       caused_by=row.get("caused_by")),
                out=Event(id=row.get("act_out"), ts=now, kind="act.out", place=placed),
                communicative=bool(
                    (self.tools.get(str(row.get("capability") or "")) or {})
                    .get("communicative")))
            trigger = Event(id=row.get("caused_by"), ts=now, kind="act.out", place=placed)
            if act.out.id is None:
                continue
            receipt = await dispatch.carry(self, act)
            await self._settle_action(trigger, act, receipt)
            count += 1
        return count

    async def _apply(self, event: Event, call: ToolCall, said: set[str] | None = None, *,
                     job: dispatch.JobRef | None = None, round_index: int = 0,
                     n: int = 0) -> dispatch.Dispatch:
        args = call.arguments
        if call.name == "think":
            await self._emit(event, "decision.note", {"act": "think", **args})
        elif call.name == "act":
            return await dispatch.dispatch(self, event, args, said=said, job=job,
                                           round_index=round_index, n=n)
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
                    place: str | None = None, description: str | None = None) -> Event:
        return await self._append(trigger, kind, payload, caused_by=trigger.id, place=place,
                                  description=description)

    def next_hook_act(self, event_id: int | None) -> int:
        """Return how many acts this event already made through the hook door (D-H)."""
        key = int(event_id or 0)
        self._hook_acts[key] = self._hook_acts.get(key, 0) + 1
        return self._hook_acts[key] - 1

    async def _emit_act(self, trigger: Event, payload: dict[str, Any], *,
                        action: Any, place: str | None = None, description: str | None = None,
                        communicative: bool = False) -> Event:
        """Record one `act.out` and its action in the ledger, in one transaction (D-F)."""
        return await self._append(trigger, "act.out", payload, caused_by=trigger.id,
                                  place=place, description=description, action=action,
                                  communicative=communicative)

    async def _settle_action(self, trigger: Event, act: Any,
                             receipt: Any) -> Event:
        """Record one action's one receipt, close its ledger row, and wake or stay silent.

        A held action is recorded and never wakes reasoning; a message that already ended
        the step records the silence instead of waking (DEC-4, DEC-5).
        """
        silent = (receipt.state == "done" and act.communicative
                  and str(act.action.place) == str(trigger.place))
        payload = {"capability": act.action.capability, "state": receipt.state,
                   "reason": receipt.reason, "result": receipt.result}
        stored = await self._append(act.out, "act.result", payload,
                                    caused_by=act.out.id, place=act.action.place,
                                    description=_receipt_line(receipt),
                                    close=(act.action, receipt))
        if receipt.state == "held":
            return stored
        if silent:
            await self._emit(stored, "decision.silent",
                             {"reason": "a message already ended this step"})
        elif receipt.state == "done" and not receipt.reason and receipt.result is None:
            return stored  # an empty answer carries no news, so it costs no model call
        self.enqueue(stored)
        return stored

    async def _decided(self, event: Event) -> bool:
        """Return whether a decision was already recorded for this event (DEC-5)."""
        if event.id is None:
            return False
        row = await (await self.conn.execute(
            "SELECT 1 FROM events WHERE caused_by = %s AND kind LIKE 'decision.%%' LIMIT 1",
            (event.id,))).fetchone()
        return row is not None

    async def _record_error(self, event: Event, exc: BaseException | str) -> None:
        raw = jobs.describe_failure(exc) if isinstance(exc, BaseException) else str(exc)
        text = config.redact_connection_secrets(_clean_text(raw) or "unknown error", self.conn)
        LOGGER.error("kernel error on %s (%s): %s", event.kind, event.body or "-", text)
        safe_event = Event(
            id=None,
            ts=event.ts,
            kind="error",
            who=_clean_text(event.who),
            place=_clean_text(event.place),
            body=_clean_text(event.body),
            origin_actor=_clean_text(event.origin_actor),
        )
        try:
            await self._append(safe_event, "error", {"error": text, "event_id": event.id},
                               caused_by=event.id, project=False)
        except Exception as rec_exc:
            LOGGER.error("failed to record error event: %s", rec_exc)

    async def step(self) -> Event:
        """Process exactly one queued event."""
        self.require_installed()
        self._loop = asyncio.get_running_loop()
        return await self._settle(await self.queue.get())

    async def run_until_idle(self) -> int:
        """Process every queued event and every in-flight background job."""
        self.require_installed()
        self._loop = asyncio.get_running_loop()
        count = 0
        while True:
            while not self.queue.empty():
                await self.step()
                count += 1
            if self._backlogs:
                try:
                    await self._flush_backlogs()
                except Exception as exc:  # one place's backlog never stops the loop (F.5)
                    await self._record_error(
                        Event(id=None, ts=self.clock.now(), kind="tick"), exc)
                continue
            if not self.jobs.in_flight:
                return count
            pending = [task for task in self.jobs.tasks if not task.done()]
            await (asyncio.wait(pending) if pending else asyncio.sleep(0))

    async def run(self, stop_event: asyncio.Event) -> None:
        """Run the consumer loop until the stop event is set."""
        self.require_installed()
        self._loop = asyncio.get_running_loop()
        try:
            await self.recover()
        except Exception as exc:  # a broken recovery never stops the loop (F.5)
            await self._record_error(Event(id=None, ts=self.clock.now(), kind="tick"), exc)
        idle_polls = 0
        while not stop_event.is_set():
            try:
                item = await asyncio.wait_for(self.queue.get(), timeout=0.1)
            except TimeoutError:
                idle_polls += 1
                if self._backlogs:
                    try:
                        await self._flush_backlogs(due_only=True, idle_polls=idle_polls)
                    except Exception as exc:  # one place's backlog never stops the loop (F.5)
                        await self._record_error(
                            Event(id=None, ts=self.clock.now(), kind="tick"), exc)
                continue
            idle_polls = 0
            try:
                await self._settle(item)
            except Exception as exc:
                LOGGER.error("kernel settle failed: %s", jobs.describe_failure(exc))
                continue  # one broken event never kills the loop.
