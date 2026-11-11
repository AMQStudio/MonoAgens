#!/usr/bin/env python3
"""Replay YAML scenarios through the real kernel to tune the cognition layer.

Usage:
    uv run python scripts/replay.py                  # every scenarios/*.yaml
    uv run python scripts/replay.py --fake           # no LLM, no embeddings
    uv run python scripts/replay.py scenarios/04_group_mention.yaml

Real mode talks to the OpenAI-compatible endpoint in .env and spends tokens;
it prints a token report at the end. Fake mode is free and only checks
appraisal decisions, so reminder/recall expectations usually fail there by design.

The replay database must end with "_replay"; its schema is rebuilt for every
scenario, so nothing here can touch the production database.
"""

from __future__ import annotations

import argparse
import asyncio
import json
import os
import pathlib
import re
import sys
from collections.abc import Callable, Sequence
from dataclasses import dataclass, field
from datetime import datetime, timedelta, timezone
from typing import Any
from zoneinfo import ZoneInfo

import httpx
import psycopg
from psycopg import sql
from pgvector.psycopg import register_vector_async
import yaml

from monoagens.bodies.cli import CAPABILITIES as CLI_CAPABILITIES
from monoagens.bodies.telegram import CAPABILITIES as TELEGRAM_CAPABILITIES
from monoagens.clock import FakeClock
from monoagens.cognition import Appraisal
from monoagens.config import (DEFAULT_ENV_PATH, load_env_file, load_llm_config,
                              load_soul)
from monoagens.expect import parse_duration
from monoagens.kernel import Kernel
from monoagens.memory import FastEmbedder, FakeEmbedder
from monoagens.migrate import migrate, migrate_url
from monoagens.pfc import PFC, PFCContext, ToolCall
from monoagens.types import Action, Event

ROOT = pathlib.Path(__file__).resolve().parent.parent
SCENARIO_DIR = ROOT / "scenarios"
REPORT_DIR = ROOT / "reports"
DEFAULT_BASE_URL = "https://api.deepinfra.com/v1/openai"
DEFAULT_MODEL = "deepseek-ai/DeepSeek-V4.1-Flash-1017"
DEFAULT_START = datetime(2026, 1, 1, tzinfo=timezone.utc)
BODY_NAME = "telegram"
CLI_BODY = "cli"
OWNER = "person:owner"
OWNER_DM = "space:tg:1001"
TIME_RE = re.compile(r"^(\d{1,2}):(\d{2})(?::(\d{2}))?$")


# ---------------------------------------------------------------- environment


def load_environment() -> None:
    """Populate os.environ from .env without overriding real variables."""
    for key, value in load_env_file(DEFAULT_ENV_PATH).items():
        os.environ.setdefault(key, value)


def replay_url() -> str:
    """Return REPLAY_DATABASE_URL, refusing anything outside a *_replay database."""
    url = os.environ.get("REPLAY_DATABASE_URL")
    if not url:
        raise RuntimeError("REPLAY_DATABASE_URL is required (see .env.example)")
    dbname = psycopg.conninfo.conninfo_to_dict(url).get("dbname") or ""
    if not dbname.endswith("_replay"):
        raise RuntimeError(f"Refusing to replay: database name {dbname!r} must end with '_replay'")
    return url


def reset_schema(url: str) -> None:
    """Drop and recreate the public schema, then apply the migrations."""
    with psycopg.connect(url, autocommit=True) as conn:
        conn.execute("DROP SCHEMA IF EXISTS public CASCADE")
        conn.execute("CREATE SCHEMA public")
    asyncio.run(migrate_url(url, backup_dir=REPORT_DIR / "backups"))


def ensure_database(url: str) -> None:
    """Create the replay database when it does not exist yet."""
    params = psycopg.conninfo.conninfo_to_dict(url)
    dbname = params["dbname"]
    admin = dict(params, dbname="postgres")
    with psycopg.connect(psycopg.conninfo.make_conninfo(**admin), autocommit=True) as conn:
        found = conn.execute("SELECT 1 FROM pg_database WHERE datname = %s", (dbname,)).fetchone()
        if found is None:
            conn.execute(sql.SQL("CREATE DATABASE {}").format(sql.Identifier(dbname)))


def llm_settings() -> dict[str, str]:
    """Resolve LLM_BASE_URL, LLM_API_KEY, and LLM_MODEL with defaults."""
    config = load_llm_config()
    base_url = config.get("base_url") or DEFAULT_BASE_URL
    model = config.get("model") or DEFAULT_MODEL
    api_key = config.get("api_key") or ""
    if not api_key:
        raise RuntimeError("LLM_API_KEY is required for a real-LLM replay (or use --fake)")
    return {"base_url": base_url, "api_key": api_key, "model": model}


# ------------------------------------------------------------------- usage


@dataclass
class Usage:
    """Cumulative LLM call and token counters."""

    calls: int = 0
    prompt: int = 0
    cached: int = 0
    completion: int = 0
    total: int = 0

    def add_payload(self, payload: dict[str, Any]) -> None:
        """Add one chat completion response body's usage to the counters."""
        usage = payload.get("usage") or {}
        details = usage.get("prompt_tokens_details") or {}
        prompt = int(usage.get("prompt_tokens") or 0)
        completion = int(usage.get("completion_tokens") or 0)
        self.prompt += prompt
        self.cached += int(details.get("cached_tokens") or 0)
        self.completion += completion
        self.total += int(usage.get("total_tokens") or (prompt + completion))

    def minus(self, other: Usage) -> Usage:
        """Return the counter difference (for per-scenario usage)."""
        return Usage(self.calls - other.calls, self.prompt - other.prompt,
                     self.cached - other.cached, self.completion - other.completion,
                     self.total - other.total)


@dataclass
class Tracker:
    """Shared usage counters and captured prompts."""

    usage: Usage = field(default_factory=Usage)
    prompts: list[tuple[int, str]] = field(default_factory=list)
    step_index: int = 0

    def record_prompt(self, content: str) -> None:
        """Store one prompt with the step that produced it."""
        self.prompts.append((self.step_index, content))


class UsageTransport(httpx.AsyncBaseTransport):
    """An HTTP transport that records requests and token usage in transit."""

    def __init__(self, inner: httpx.AsyncBaseTransport, tracker: Tracker) -> None:
        """Wrap the real transport and share the tracker."""
        self._inner = inner
        self._tracker = tracker

    async def handle_async_request(self, request: httpx.Request) -> httpx.Response:
        """Forward the request, read the body, and record usage."""
        response = await self._inner.handle_async_request(request)
        await response.aread()
        self._tracker.usage.calls += 1
        try:
            payload = json.loads(request.content or b"{}")
        except ValueError:
            payload = {}
        messages = payload.get("messages") or []
        if messages:
            self._tracker.record_prompt(str(messages[-1].get("content") or ""))
        try:
            self._tracker.usage.add_payload(json.loads(response.content or b"{}"))
        except ValueError:
            pass
        return response

    async def aclose(self) -> None:
        """Close the wrapped transport."""
        await self._inner.aclose()


# -------------------------------------------------------------- fake agents


def _fake_context_text(context: PFCContext) -> str:
    """Render a fake-mode prompt from the pieces the kernel handed over."""
    parts: list[str] = [context.core_memory, *context.memories]
    parts.extend(str(event.payload.get("description") or "") for event in context.recent)
    if context.trigger is not None:
        parts.append(str(context.trigger.payload.get("description") or ""))
    return "\n".join(part for part in parts if part)


def _fake_sleep_arguments(messages: Sequence[dict[str, Any]]) -> dict[str, Any]:
    """Turn the last sleep event line into one memory item."""
    content = str(messages[-1].get("content") or "")
    block = content.split("<events>", 1)[-1].split("</events>", 1)[0]
    lines = [line for line in block.splitlines() if line.strip()]
    summary = lines[-1] if lines else "percakapan replay"
    text = summary.split(": ", 1)[-1] or summary
    return {"items": [{"text": text, "importance": 0.5}],
            "core_memory": f"Replay: {text}"}


class FakePFC:
    """A scripted PFC: always say "ok" when woken, keep one memory on sleep."""

    def __init__(self, tracker: Tracker) -> None:
        """Share the tracker so fake calls are counted."""
        self._tracker = tracker

    async def decide(self, context: PFCContext) -> list[ToolCall]:
        """Always answer with say(text="ok")."""
        self._tracker.usage.calls += 1
        self._tracker.record_prompt(_fake_context_text(context))
        return [ToolCall("act", {"capability": "telegram.say", "arguments": {"text": "ok"}})]

    async def call(self, messages: list[dict[str, Any]], tools: list[dict[str, Any]], *,
                   tool_choice: str = "required") -> list[ToolCall]:
        """Answer sleep with store_memories and everything else with say("ok")."""
        self._tracker.usage.calls += 1
        self._tracker.record_prompt(str(messages[-1].get("content") or ""))
        names = {tool["function"]["name"] for tool in tools}
        if "store_memories" in names:
            return [ToolCall("store_memories", _fake_sleep_arguments(messages))]
        return [ToolCall("act", {"capability": "telegram.say", "arguments": {"text": "ok"}})]

    async def aclose(self) -> None:
        """Nothing to close."""


class FakeBody:
    """A body that records every Action dispatched to it."""

    def __init__(self, name: str = BODY_NAME) -> None:
        """Start with an empty action list and the given body name."""
        self.name = name
        self.actions: list[Action] = []

    def capabilities(self) -> Sequence[dict[str, Any]]:
        """Declare the same capabilities as the real body of that name."""
        return CLI_CAPABILITIES if self.name == CLI_BODY else TELEGRAM_CAPABILITIES

    async def act(self, action: Action) -> None:
        """Record the action instead of sending it."""
        self.actions.append(action)


class RecordingKernel(Kernel):
    """A kernel that keeps the appraisals of the step being executed."""

    def __init__(self, *args, **kwargs) -> None:
        """Start with an empty appraisal list."""
        super().__init__(*args, **kwargs)
        self.appraisals: list[Appraisal] = []

    async def _appraise(self, event: Event) -> Appraisal:
        """Appraise as usual and remember the decision for the report."""
        appraisal = await super()._appraise(event)
        self.appraisals.append(appraisal)
        return appraisal

    async def put(self, event: Event) -> None:
        """Pin the replayed clock to each event's timestamp before it is processed."""
        self.clock.set_time(event.ts)
        await super().put(event)


# ------------------------------------------------------------------ scenario


@dataclass
class Scenario:
    """One YAML scenario file."""

    path: pathlib.Path
    name: str
    description: str
    steps: list[dict[str, Any]]
    expectations: list[Any]
    zone: ZoneInfo


@dataclass
class StepRecord:
    """Everything the replay observed while executing one step."""

    index: int
    kind: str
    at: datetime
    who: str
    place: str
    text: str
    addressed: bool
    appraisals: list[Appraisal]
    replies: list[tuple[str, str]]
    events: list[dict[str, Any]]


@dataclass
class Reply:
    """One act.out produced by the kernel."""

    step: int
    ts: datetime
    place: str
    text: str


@dataclass
class CheckResult:
    """One expectation verdict."""

    label: str
    passed: bool
    detail: str


@dataclass
class ScenarioResult:
    """Everything a scenario produced, ready for checks and reporting."""

    scenario: Scenario
    steps: list[StepRecord]
    replies: list[Reply]
    prompts: list[tuple[int, str]]
    events: list[dict[str, Any]]
    memories: int
    usage: Usage
    checks: list[CheckResult] = field(default_factory=list)


def load_scenario(path: pathlib.Path, zone: ZoneInfo) -> Scenario:
    """Read and minimally validate one scenario YAML file."""
    data = yaml.safe_load(path.read_text(encoding="utf-8")) or {}
    steps = data.get("steps") or []
    if not isinstance(steps, list) or not steps:
        raise ValueError(f"{path}: steps must be a non-empty list")
    for index, step in enumerate(steps, 1):
        if not isinstance(step, dict):
            raise ValueError(f"{path}: step {index} must be a mapping")
        if not any(key in step for key in ("text", "advance", "restart")):
            raise ValueError(f"{path}: step {index} needs text, advance, or restart")
    return Scenario(path=path, name=str(data.get("name") or path.stem),
                    description=str(data.get("description") or ""), steps=steps,
                    expectations=list(data.get("expectations") or []), zone=zone)


def parse_at(value: Any, clock: FakeClock, zone: ZoneInfo) -> datetime:
    """Parse an ISO timestamp or a local HH:MM clock time into UTC."""
    text = str(value).strip()
    match = TIME_RE.match(text)
    if match:
        local = clock.now().astimezone(zone)
        stamp = local.replace(hour=int(match.group(1)), minute=int(match.group(2)),
                              second=int(match.group(3) or 0), microsecond=0)
        return stamp.astimezone(timezone.utc)
    stamp = datetime.fromisoformat(text)
    if stamp.tzinfo is None:
        stamp = stamp.replace(tzinfo=zone)
    return stamp.astimezone(timezone.utc)


def place_kind(place: str) -> str:
    """Guess direct vs group from the neutral Telegram place id."""
    return "group" if place.startswith("space:tg:-") else "direct"


def _event_row(row: Sequence[Any]) -> dict[str, Any]:
    payload = row[5] or {}
    text = payload.get("description")
    if text is None:
        text = (payload.get("arguments") or {}).get("text")
    return {"id": row[0], "ts": row[1], "kind": row[2], "who": row[3], "place": row[4],
            "text": text, "payload": payload}


# ------------------------------------------------------------------- runner


class ReplayRunner:
    """Runs scenarios against a real kernel on a throwaway database."""

    def __init__(self, url: str, soul: dict[str, Any], fake: bool, tracker: Tracker,
                 settings: dict[str, str]) -> None:
        """Wire the runner to the database, soul, and LLM settings."""
        self.url, self.soul, self.fake, self.tracker = url, soul, fake, tracker
        self.settings = settings
        self.zone = ZoneInfo(str(soul.get("timezone") or "UTC"))
        self.embedder: Any = FakeEmbedder() if fake else FastEmbedder()
        self.clock = FakeClock()
        self.conn: psycopg.AsyncConnection[Any] | None = None
        self.kernel: RecordingKernel | None = None
        self._last_id = 0
        self._message_id = 0

    def _make_pfc(self) -> Any:
        """Build a fake PFC or a real HTTP-tracked PFC."""
        if self.fake:
            return FakePFC(self.tracker)
        client = httpx.AsyncClient(
            transport=UsageTransport(httpx.AsyncHTTPTransport(), self.tracker), timeout=90.0)
        return PFC(self.settings["base_url"], self.settings["api_key"],
                   self.settings["model"], soul=self.soul, client=client, timeout=90.0)

    def _make_kernel(self) -> Kernel:
        """Build a fresh kernel that records its appraisals."""
        pfc = self._make_pfc()
        kernel = RecordingKernel(self.conn, clock=self.clock, pfc=pfc, soul=self.soul,
                                 embedder=self.embedder)
        kernel.register_body(FakeBody(BODY_NAME))
        kernel.register_body(FakeBody(CLI_BODY))
        self.kernel = kernel
        return kernel

    async def run(self, scenario: Scenario) -> ScenarioResult:
        """Reset the schema and replay one scenario from the first step."""
        start_usage = Usage(**vars(self.tracker.usage))
        self.tracker.prompts.clear()
        self._last_id = 0
        self._message_id = 0
        self.clock = FakeClock(initial_time=self._initial_time(scenario))
        async with await psycopg.AsyncConnection.connect(self.url, autocommit=True) as conn:
            self.conn = conn
            await conn.execute("DROP SCHEMA IF EXISTS public CASCADE")
            await conn.execute("CREATE SCHEMA public")
            await migrate(conn, backup_dir=REPORT_DIR / "backups")
            await register_vector_async(conn)
            self._make_kernel()
            await self.kernel.recover()
            await self.kernel.run_until_idle()
            steps = [await self._run_step(index, raw)
                     for index, raw in enumerate(scenario.steps, 1)]
            result = ScenarioResult(
                scenario=scenario, steps=steps,
                replies=self._replies(steps), prompts=list(self.tracker.prompts),
                events=[event for step in steps for event in step.events],
                memories=await self._memory_count(),
                usage=self.tracker.usage.minus(start_usage))
        self.conn = None
        result.checks = evaluate_all(result)
        return result

    def _initial_time(self, scenario: Scenario) -> datetime:
        """Use the first explicit timestamp as the starting clock, else 2026-01-01."""
        for step in scenario.steps:
            if step.get("at"):
                return parse_at(step["at"], FakeClock(DEFAULT_START), self.zone)
        return DEFAULT_START

    async def _run_step(self, index: int, raw: dict[str, Any]) -> StepRecord:
        """Execute one step and collect its appraisals, events, and replies."""
        self.tracker.step_index = index
        self.kernel.appraisals.clear()
        if raw.get("at"):
            self.clock.set_time(parse_at(raw["at"], self.clock, self.zone))
        start_id = await self._max_id()
        kind = "restart" if raw.get("restart") else "advance" if raw.get("advance") else "message"
        who = str(raw.get("who") or OWNER)
        place = str(raw.get("place") or "space:tg:1001")
        text = str(raw.get("text") or "")
        body = str(raw.get("body") or "") or None
        addressed = bool(raw.get("addressed", False))
        if kind == "message":
            await self._feed(who=who, place=place, text=text, addressed=addressed, body=body)
        elif kind == "advance":
            self.clock.advance(parse_duration(raw["advance"]))
            await self.clock.tick(self.kernel.put)
            await self.kernel.step()
            await self.kernel.run_until_idle()
        else:
            pfc = self.kernel.pfc
            if pfc is not None and hasattr(pfc, "aclose"):
                await pfc.aclose()
            self._make_kernel()
            await self.kernel.recover()
            await self.kernel.run_until_idle()
        if kind == "message":
            label = text
        elif kind == "advance":
            label = f"advance {raw['advance']}"
        else:
            label = "restart"
        events = await self._new_events(start_id)
        return StepRecord(index=index, kind=kind, at=self.clock.now(), who=who, place=place,
                          text=label, addressed=addressed,
                          appraisals=list(self.kernel.appraisals),
                          replies=[(event["place"] or "", str(event["text"]))
                                   for event in events if event["kind"] == "act.out"],
                          events=events)

    async def _feed(self, *, who: str, place: str, text: str, addressed: bool,
                    body: str | None = None) -> None:
        """Put one percept and let the single consumer process it to completion."""
        self._message_id += 1
        name = CLI_BODY if body == CLI_BODY or place.startswith("space:cli") else BODY_NAME
        kind = "direct" if name == CLI_BODY else place_kind(place)
        event = Event(id=None, ts=self.clock.now(), kind="percept", who=who, place=place,
                      body=name, payload={"description": text, "addressed": addressed,
                                          "place_kind": kind,
                                          "data": {"message_id": self._message_id}})
        task = asyncio.create_task(self.kernel.put(event))
        await asyncio.sleep(0)
        await self.kernel.step()
        await asyncio.wait_for(task, 120.0)
        await self.kernel.run_until_idle()

    async def _max_id(self) -> int:
        """Return the newest event id in the Log."""
        row = await (await self.conn.execute("SELECT coalesce(max(id), 0) FROM events")).fetchone()
        return int(row[0])

    async def _new_events(self, after_id: int) -> list[dict[str, Any]]:
        """Read every event appended after the step started."""
        cur = await self.conn.execute(
            "SELECT id, ts, kind, who, place, payload FROM events WHERE id > %s ORDER BY id",
            (after_id,))
        return [_event_row(row) for row in await cur.fetchall()]

    async def _memory_count(self) -> int:
        """Return how many memories the scenario's sleep produced."""
        row = await (await self.conn.execute("SELECT count(*) FROM memories")).fetchone()
        return int(row[0])

    def _replies(self, steps: Sequence[StepRecord]) -> list[Reply]:
        """Flatten every act.out with the step that produced it."""
        return [Reply(step.index, event["ts"], event["place"] or "", str(event["text"] or ""))
                for step in steps for event in step.events if event["kind"] == "act.out"]


# ------------------------------------------------------------------- checks


def _target_place(text: str) -> str:
    """Resolve a reply target name into a place id."""
    target = text.strip().strip("\"'").strip()
    if target.lower().startswith("space:"):
        return target
    if "owner" in target.lower() or " dm" in target.lower() or target.lower().startswith("dm"):
        return OWNER_DM
    return target


def _string_spec(text: str) -> dict[str, Any]:
    """Turn a plain-language expectation into a check dictionary."""
    lower = text.lower().strip()
    if lower.startswith("exactly one message to "):
        target = text.strip()[len("exactly one message to "):]
        return {"type": "reply_in", "place": _target_place(target), "count": 1,
                "label": text}
    for prefix in ("exactly one reminder at ", "remind at "):
        if lower.startswith(prefix):
            return {"type": "remind_at", "at": text.strip()[len(prefix):].strip(" \"'"),
                    "label": text}
    if lower.startswith("reply mentions "):
        return {"type": "mentions", "text": text.strip()[len("reply mentions "):].strip(" \"'"),
                "label": text}
    if lower in ("no reply", "no replies"):
        return {"type": "no_reply", "label": text}
    if lower in ("reply", "replies"):
        return {"type": "reply", "label": text}
    return {"type": "unknown", "label": text}


def normalize_check(raw: Any) -> dict[str, Any]:
    """Normalize a string or dict expectation into one check dictionary."""
    if isinstance(raw, dict):
        spec = dict(raw)
        spec["type"] = str(spec.get("type") or spec.get("check") or "unknown")
        spec.setdefault("label", spec.get("label") or spec["type"])
        return spec
    return _string_spec(str(raw))


def evaluate_all(result: ScenarioResult) -> list[CheckResult]:
    """Run every scenario expectation and return its verdicts."""
    return [evaluate(normalize_check(raw), result) for raw in result.scenario.expectations]


def _scoped_replies(spec: dict[str, Any], result: ScenarioResult) -> list[Reply]:
    """Filter replies to the steps the check names."""
    steps = spec.get("steps")
    if steps is not None:
        wanted = {int(step) for step in steps}
        return [reply for reply in result.replies if reply.step in wanted]
    step = spec.get("step")
    if step is None:
        return result.replies
    return [reply for reply in result.replies if reply.step == int(step)]


def _detail(replies: Sequence[Reply]) -> str:
    """Render replies for a check's detail line."""
    if not replies:
        return "no act.out"
    return "; ".join(f"[{reply.place}] {reply.text}" for reply in replies)


def _time_matches(expected: str, ts: datetime, zone: ZoneInfo) -> bool:
    """Compare an HH:MM or ISO time against an event timestamp."""
    match = TIME_RE.match(expected.strip())
    if match:
        local = ts.astimezone(zone)
        return (local.hour, local.minute) == (int(match.group(1)), int(match.group(2)))
    try:
        target = datetime.fromisoformat(expected.strip())
    except ValueError:
        return False
    if target.tzinfo is None:
        return False
    left = ts.astimezone(timezone.utc).replace(second=0, microsecond=0)
    return left == target.astimezone(timezone.utc).replace(second=0, microsecond=0)


def _check_no_reply(spec: dict[str, Any], result: ScenarioResult) -> CheckResult:
    replies = _scoped_replies(spec, result)
    return CheckResult(str(spec["label"]), not replies,
                       "no act.out" if not replies else _detail(replies))


def _check_reply(spec: dict[str, Any], result: ScenarioResult) -> CheckResult:
    replies = _scoped_replies(spec, result)
    return CheckResult(str(spec["label"]), bool(replies), _detail(replies))


def _check_mentions(spec: dict[str, Any], result: ScenarioResult) -> CheckResult:
    replies = _scoped_replies(spec, result)
    needle = str(spec.get("text") or "").lower()
    hits = [reply for reply in replies if needle in reply.text.lower()]
    return CheckResult(str(spec["label"]), bool(hits), _detail(hits) if hits else _detail(replies))


def _check_remind_at(spec: dict[str, Any], result: ScenarioResult) -> CheckResult:
    expected = str(spec.get("at") or "")
    replies = _scoped_replies(spec, result)
    hits = [reply for reply in replies if _time_matches(expected, reply.ts, result.scenario.zone)]
    return CheckResult(str(spec["label"]), len(hits) == 1,
                       f"{len(hits)} at {expected}: {_detail(hits)}")


def _check_min_replies(spec: dict[str, Any], result: ScenarioResult) -> CheckResult:
    replies = _scoped_replies(spec, result)
    want = int(spec.get("count") or 1)
    return CheckResult(str(spec["label"]), len(replies) >= want,
                       f"{len(replies)} >= {want}: {_detail(replies)}")


def _check_reply_in(spec: dict[str, Any], result: ScenarioResult) -> CheckResult:
    place = str(spec.get("place") or "")
    replies = _scoped_replies(spec, result)
    hits = [reply for reply in replies if reply.place == place]
    count = spec.get("count")
    if count is None:
        return CheckResult(str(spec["label"]), bool(hits),
                           _detail(hits) if hits else _detail(replies))
    want = int(count)
    return CheckResult(str(spec["label"]), len(hits) == want,
                       f"{len(hits)} == {want}: {_detail(hits) if hits else _detail(replies)}")


def _check_prompt_contains(spec: dict[str, Any], result: ScenarioResult) -> CheckResult:
    needle = str(spec.get("text") or "").lower()
    step = spec.get("step")
    prompts = [(index, content) for index, content in result.prompts
               if step is None or index == int(step)]
    hits = [index for index, content in prompts if needle in content.lower()]
    return CheckResult(str(spec["label"]), bool(hits),
                       f"prompt steps with {needle!r}: {hits or 'none'}")


def _check_sleep_done(spec: dict[str, Any], result: ScenarioResult) -> CheckResult:
    hits = [event for event in result.events if event["kind"] == "sleep.done"]
    return CheckResult(str(spec["label"]), bool(hits), f"{len(hits)} sleep.done")


def _check_memories(spec: dict[str, Any], result: ScenarioResult) -> CheckResult:
    want = int(spec.get("count") or 1)
    return CheckResult(str(spec["label"]), result.memories >= want,
                       f"{result.memories} memories stored (need {want})")


CHECKS: dict[str, Callable[[dict[str, Any], ScenarioResult], CheckResult]] = {
    "no_reply": _check_no_reply,
    "reply": _check_reply,
    "mentions": _check_mentions,
    "remind_at": _check_remind_at,
    "min_replies": _check_min_replies,
    "reply_in": _check_reply_in,
    "prompt_contains": _check_prompt_contains,
    "sleep_done": _check_sleep_done,
    "memories_stored": _check_memories,
}


def register_check(name: str, fn: Callable[[dict[str, Any], ScenarioResult], CheckResult]) -> None:
    """Register a custom check function for dict expectations with that type."""
    CHECKS[name] = fn


def evaluate(spec: dict[str, Any], result: ScenarioResult) -> CheckResult:
    """Run one normalized check, failing unknown types loudly."""
    fn = CHECKS.get(str(spec.get("type")))
    if fn is None:
        return CheckResult(str(spec["label"]), False, f"unknown check type: {spec.get('type')!r}")
    return fn(spec, result)


# ------------------------------------------------------------------- report


def _cell(value: Any) -> str:
    """Make a value safe inside one markdown table cell."""
    return str(value).replace("|", "\\|").replace("\n", " ").strip()


def _fmt_time(stamp: datetime, zone: ZoneInfo) -> str:
    """Render a timestamp in the soul's timezone."""
    return stamp.astimezone(zone).strftime("%Y-%m-%d %H:%M")


def _decision_summary(step: StepRecord) -> tuple[str, Appraisal | None]:
    """Pick the strongest appraisal and an aggregate decision label."""
    decisions = [appraisal.decision for appraisal in step.appraisals]
    if "wake" in decisions:
        decision = "wake"
    elif "note" in decisions:
        decision = "note"
    elif "silent" in decisions:
        decision = "silent"
    elif step.kind == "advance":
        decision = "tick"
    elif step.kind == "restart":
        decision = "restart"
    else:
        decision = "-"
    preferred = next((a for a in step.appraisals if a.decision == decision), None)
    return decision, preferred


def _step_row(step: StepRecord, zone: ZoneInfo) -> str:
    """Render one step as a markdown table row."""
    decision, appraisal = _decision_summary(step)
    reply = " / ".join(text for _, text in step.replies) or "-"
    event_kinds = ", ".join(dict.fromkeys(event["kind"] for event in step.events)) or "-"
    cells = [
        step.index, _fmt_time(step.at, zone), step.who if step.kind == "message" else "-",
        step.place if step.kind == "message" else "-",
        step.text if step.kind == "message" else step.kind, decision,
        appraisal.reflex if appraisal and appraisal.reflex else "-",
        f"{appraisal.s:.2f}" if appraisal else "-",
        f"{appraisal.theta:.2f}" if appraisal else "-",
        f"{appraisal.p:.2f}" if appraisal else "-",
        f"{appraisal.u:.2f}" if appraisal else "-",
        appraisal.reason if appraisal else "-", reply, event_kinds,
    ]
    return "| " + " | ".join(_cell(cell) for cell in cells) + " |"


def _scenario_section(result: ScenarioResult, zone: ZoneInfo) -> list[str]:
    """Render one scenario's table, checks, and outcome."""
    scenario = result.scenario
    failed = [check for check in result.checks if not check.passed]
    status = "PASS" if not failed else "FAIL"
    lines = [f"## {scenario.name}", ""]
    if scenario.description:
        lines += [scenario.description, ""]
    lines += ["| # | Time | Who | Place | Input | Decision | Reflex | S | Theta | P | u "
              "| Reason | Reply | Events |",
              "|---|---|---|---|---|---|---|---|---|---|---|---|---|---|"]
    lines += [_step_row(step, zone) for step in result.steps]
    lines += ["", f"**Expectations: {status}**", ""]
    if not result.checks:
        lines += ["- (no expectations declared)", ""]
    for check in result.checks:
        mark = "PASS" if check.passed else "FAIL"
        lines.append(f"- **{mark}** — {check.label}: {check.detail}")
    usage = result.usage
    lines += ["", f"LLM calls: {usage.calls} · prompt {usage.prompt} · cached "
              f"{usage.cached} · completion {usage.completion} · total {usage.total} tokens", ""]
    return lines


def _summary_row(result: ScenarioResult) -> str:
    """Render one scenario as a summary table row."""
    passed = sum(1 for check in result.checks if check.passed)
    total = len(result.checks)
    status = "PASS" if passed == total else "FAIL"
    usage = result.usage
    cells = [result.scenario.name, len(result.steps), usage.calls, usage.prompt,
             usage.cached, usage.completion, usage.total, f"{passed}/{total}", status]
    return "| " + " | ".join(_cell(cell) for cell in cells) + " |"


def render_report(results: Sequence[ScenarioResult], usage: Usage, mode: str,
                  model: str, soul: dict[str, Any], when: datetime,
                  zone: ZoneInfo) -> str:
    """Build the full markdown report."""
    lines = [f"# Replay report — {when:%Y-%m-%d %H:%M}", "",
             f"- Mode: **{mode}**", f"- Model: `{model}`", f"- Timezone: `{zone.key}`",
             f"- Scenarios: {len(results)}", f"- Report generated: {when:%Y-%m-%d %H:%M:%S}",
             ""]
    cognition = soul.get("cognition") or {}
    lines += ["## Cognition configuration", "",
              f"- Weights: `{json.dumps(cognition.get('weights') or {}, sort_keys=True)}`",
              f"- Seed: `{cognition.get('seed', 1)}`",
              f"- Band: `{json.dumps(cognition.get('band') or {}, sort_keys=True)}`",
              f"- Threshold: `{json.dumps(cognition.get('threshold') or {}, sort_keys=True)}`",
              ""]
    for result in results:
        lines += _scenario_section(result, zone)
    lines += ["## Summary", "",
              "| Scenario | Steps | LLM calls | Prompt | Cached | Completion | Total tokens "
              "| Checks | Result |",
              "|---|---|---|---|---|---|---|---|---|"]
    lines += [_summary_row(result) for result in results]
    lines += ["", f"**Total LLM calls:** {usage.calls}", "",
              f"**Tokens:** prompt {usage.prompt} · cached {usage.cached} · completion "
              f"{usage.completion} · total {usage.total}", ""]
    return "\n".join(lines)


def write_report(text: str, when: datetime) -> pathlib.Path:
    """Write the report under reports/ and return its path."""
    REPORT_DIR.mkdir(parents=True, exist_ok=True)
    path = REPORT_DIR / f"replay-{when:%Y%m%d-%H%M%S}.md"
    path.write_text(text, encoding="utf-8")
    return path


# ---------------------------------------------------------------------- main


def scenario_paths(args: Sequence[str]) -> list[pathlib.Path]:
    """Resolve explicit scenario paths or every scenarios/*.yaml in order."""
    if args:
        paths = [pathlib.Path(item) for item in args]
        missing = [str(path) for path in paths if not path.is_file()]
        if missing:
            raise RuntimeError(f"scenario not found: {', '.join(missing)}")
        return paths
    paths = sorted(SCENARIO_DIR.glob("*.yaml"))
    if not paths:
        raise RuntimeError(f"no scenarios found in {SCENARIO_DIR}")
    return paths


async def run_all(paths: Sequence[pathlib.Path], fake: bool, url: str,
                  soul: dict[str, Any],
                  settings: dict[str, str]) -> tuple[list[ScenarioResult], Usage]:
    """Reset the database and replay every scenario in order."""
    zone = ZoneInfo(str(soul.get("timezone") or "UTC"))
    tracker = Tracker()
    runner = ReplayRunner(url, soul, fake, tracker, settings)
    results: list[ScenarioResult] = []
    for path in paths:
        scenario = load_scenario(path, zone)
        print(f"replaying {scenario.name} ...", flush=True)
        result = await runner.run(scenario)
        passed = sum(1 for check in result.checks if check.passed)
        print(f"  {passed}/{len(result.checks)} checks passed, "
              f"{result.usage.calls} LLM calls", flush=True)
        results.append(result)
    return results, tracker.usage


def main(argv: Sequence[str] | None = None) -> int:
    """Parse arguments, replay scenarios, and write the report."""
    parser = argparse.ArgumentParser(description="Replay MonoAgens scenarios to tune cognition.")
    parser.add_argument("scenarios", nargs="*", help="scenario YAML files (default: all)")
    parser.add_argument("--fake", action="store_true",
                        help="use a scripted FakeLLM and FakeEmbedder (free)")
    args = parser.parse_args(argv)
    load_environment()
    url = replay_url()
    soul = load_soul()
    settings = {"base_url": DEFAULT_BASE_URL, "api_key": "", "model": DEFAULT_MODEL}
    if not args.fake:
        settings = llm_settings()
    paths = scenario_paths(args.scenarios)
    ensure_database(url)
    reset_schema(url)
    results, usage = asyncio.run(run_all(paths, args.fake, url, soul, settings))
    when = datetime.now(timezone.utc)
    zone = ZoneInfo(str(soul.get("timezone") or "UTC"))
    mode = "fake LLM (free)" if args.fake else "real LLM"
    text = render_report(results, usage, mode, settings["model"], soul, when, zone)
    path = write_report(text, when)
    failed = [check.label for result in results for check in result.checks if not check.passed]
    print(f"\nreport: {path}")
    print(f"checks: {sum(len(r.checks) for r in results) - len(failed)}/"
          f"{sum(len(r.checks) for r in results)} passed")
    if failed:
        print(f"failed: {len(failed)}")
    print(f"llm calls: {usage.calls} · tokens: prompt {usage.prompt}, cached {usage.cached}, "
          f"completion {usage.completion}, total {usage.total}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
