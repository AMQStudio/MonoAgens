"""Background jobs: slow work off the loop; every result returns as an event.

The kernel loop only records and decides. The PFC, the classifier, embeddings and
sleep run here, each under one timeout. A job's result goes back into the kernel
queue as `job.result`, so it is recorded and applied by the one consumer.
"""

from __future__ import annotations

import asyncio
from collections.abc import Awaitable, Callable
from dataclasses import dataclass, field
from datetime import datetime
from typing import Any

import psycopg
from pgvector.psycopg import register_vector_async

from monoagens import config, memory
from monoagens.pfc import ToolCall, turn_check, turns
from monoagens.types import Event

INTERNAL_KINDS = ("job.started", "job.result")
JobWork = Callable[[], Awaitable[Any]]


@dataclass(slots=True)
class Job:
    """One background job in flight."""

    name: str
    key: str
    trigger: Event
    place: str
    re_wake: tuple[Event, str, str] | None = None
    outcome: dict[str, Any] = field(default_factory=dict)


class JobRunner:
    """Owns the background tasks and answers their results on the kernel queue."""

    def __init__(self, kernel: Any) -> None:
        """Read the timeout from the prior and start with nothing in flight."""
        self.kernel = kernel
        values = (kernel.settings.get("constants") or {})
        self.timeout = float(values["job_timeout_seconds"])
        self.in_flight: dict[str, Job] = {}
        self.tasks: set[asyncio.Task[None]] = set()

    def busy(self, key: str) -> bool:
        """Return whether a job with that key is still in flight."""
        return key in self.in_flight

    def ask_re_wake(self, key: str, trigger: Event, reason: str, place: str) -> None:
        """Remember the newest wake for a busy place; it fires once at the end."""
        job = self.in_flight.get(key)
        if job is not None:
            job.re_wake = (trigger, reason, place)

    async def start(self, name: str, key: str, trigger: Event, work: JobWork, *,
                    place: str = "") -> bool:
        """Start one job unless a job with the same key is still in flight."""
        if key in self.in_flight:
            return False
        job = Job(name, key, trigger, place)
        self.in_flight[key] = job
        try:
            started = await self.kernel._emit(trigger, "job.started",
                                              {"job": name, "place": place})
        except Exception:
            self.in_flight.pop(key, None)
            raise
        task = asyncio.create_task(self._run(job, started, work))
        self.tasks.add(task)
        task.add_done_callback(self.tasks.discard)
        return True

    async def _run(self, job: Job, started: Event, work: JobWork) -> None:
        try:
            job.outcome = {"result": await asyncio.wait_for(work(), self.timeout)}
        except asyncio.TimeoutError:
            job.outcome = {"error": f"{job.name} timed out after {self.timeout:g}s"}
        except Exception as exc:  # a broken job reports it; the loop lives on
            error = config.redact_connection_secrets(str(exc), self.kernel.conn)
            job.outcome = {"error": error}
        payload = {"job": job.name, "place": job.place, "key": job.key,
                   "started": started.id, "result": _view(job)}
        self.kernel.enqueue(Event(
            id=None, ts=self.kernel.clock.now(), kind="job.result",
            who=job.trigger.who, place=job.trigger.place, body=job.trigger.body,
            origin_actor=job.trigger.origin_actor, caused_by=started.id,
            payload=payload))

    def finish(self, event: Event) -> Job | None:
        """Remove and return the job that a `job.result` event closes."""
        return self.in_flight.pop(str(event.payload.get("key") or ""), None)

    async def sleep(self, now: datetime) -> dict[str, Any]:
        """Consolidate on a private connection, so the consumer never waits on it."""
        conn = await psycopg.AsyncConnection.connect(config.private_dsn(self.kernel.conn),
                                                     autocommit=True)
        try:
            await register_vector_async(conn)
            done = await memory.sleep(conn, pfc=self.kernel.pfc,
                                      embedder=self.kernel.embedder, now=now,
                                      settings=self.kernel.settings)
            return {"sleep_event": done.id if done is not None else None}
        finally:
            await conn.close()


async def decide(pfc: Any, context: Any, rounds: int, clock: Any,
                  settings: dict[str, Any]) -> dict[str, Any]:
    """Run one wake's PFC rounds in the background, each keeping its own usage (D51)."""
    check = turn_check(context, clock.now(), settings)
    made = await turns(pfc, context, rounds, check=check)
    calls = [call for item in made for call in item["calls"]]
    return {"calls": calls, "rounds": made}


async def encode(embedder: Any, event_id: int, text: str) -> dict[str, Any]:
    """Embed one text in the background; the kernel stores the vector."""
    return {"event_id": event_id, "vector": (await embedder.embed([text]))[0]}


async def apply_calls(kernel: Any, trigger: Event, calls: list[dict[str, Any]]) -> None:
    """Apply the tool calls of a finished PFC job; silence is recorded when none acts."""
    acted, thought = False, ""
    said: set[str] = set()
    for raw in calls:
        if raw.get("refused"):
            continue
        call = ToolCall(str(raw.get("name")), dict(raw.get("arguments") or {}))
        acted = await kernel._apply(trigger, call, said) or acted
        if call.name == "think":
            thought = str(call.arguments.get("thought") or "")
    if not acted:
        await kernel._emit(trigger, "decision.silent", {"reason": thought or "no act"})


def _view(job: Job) -> dict[str, Any]:
    """Return the log-safe view of a result; a vector never enters the Log."""
    if "error" in job.outcome:
        return job.outcome
    result = job.outcome.get("result") or {}
    if job.name == "encode":
        vector = result.get("vector") or []
        return {"event_id": result.get("event_id"), "embedded": bool(vector),
                "dim": len(vector)}
    return result if isinstance(result, dict) else {"value": result}
