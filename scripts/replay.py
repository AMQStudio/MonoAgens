#!/usr/bin/env python3
"""Replay YAML scenarios through the real kernel to tune the cognition layer.

Usage:
    uv run python scripts/replay.py                  # every scenarios/<function>/*.yaml
    uv run python scripts/replay.py --fake           # no LLM, no embeddings
    uv run python scripts/replay.py scenarios/turn-taking/04_group_mention.yaml

Real mode talks to the OpenAI-compatible endpoint in .env and spends tokens;
it prints a token report at the end. Fake mode is free: each scenario carries
recorded model answers (`model_answers`) that the scripted Reasoning replays, so the
reminder and recall behaviors are deterministic without an LLM.

The replay database must end with "_replay"; its schema is rebuilt for every
scenario, so nothing here can touch the production database.

A message step may declare `should: answer` or `should: silent`: that is the
ground truth of the missed-help and false-speak metrics. Without it, an
addressed message should be answered and an unaddressed one should stay silent.
A communicative act counts as honest when every quotation of 20 characters or
more in it appears somewhere in the scenario's Log. Absolute-line violations count every
private memory or line on a desk whose audience does not cover it, and every reply that
quotes content its place may not see.
"""

from __future__ import annotations

import argparse
import asyncio
import json
import os
import pathlib
import re
import sys
from collections.abc import Callable, Mapping, Sequence
from dataclasses import dataclass, field
from datetime import datetime, timedelta, timezone
from typing import Any
from zoneinfo import ZoneInfo

import httpx
import psycopg
from psycopg import sql
from pgvector.psycopg import register_vector_async
import yaml

from monoagens.middleware import privacy
from monoagens.adapters.cli import CAPABILITIES as CLI_CAPABILITIES
from monoagens.adapters.telegram import CAPABILITIES as TELEGRAM_CAPABILITIES
from monoagens.infra.clock import FakeClock
from monoagens.core.gate import Appraisal
from monoagens.infra.config import DEFAULT_ENV_PATH, EFFORTS, load_env_file, load_llm_config
from monoagens.infra.prior import constants, load_settings
from monoagens.infra.duration import parse_duration
from monoagens.core.kernel import Kernel
from monoagens.infra.embedding import FastEmbedder, FakeEmbedder
from monoagens.infra.migrate import migrate, migrate_url
from monoagens.core.context.layout import PFCContext, withheld_note
from monoagens.core.reasoning import Reasoning, ToolCall
from monoagens.types import Action, Event

ROOT = pathlib.Path(__file__).resolve().parent.parent
SCENARIO_DIR = ROOT / "scenarios"
REPORT_DIR = ROOT / "reports"
LATEST_NAME = "latest.md"
REPORT_KEEP = 10
REPORT_FILE_RE = re.compile(r"^(?:\d{4}-\d{2}-\d{2}-\d{4}-(?:real|fake)-.+|replay-\d{8}-\d{6})\.md$")
REPORT_STAMP_RE = re.compile(r"^(\d{4}-\d{2}-\d{2})-(\d{4})-(?:real|fake)-.+\.md$")
LEGACY_STAMP_RE = re.compile(r"^replay-(\d{8})-(\d{6})\.md$")
DEFAULT_BASE_URL = "https://api.deepinfra.com/v1/openai"
DEFAULT_MODEL = "deepseek-ai/DeepSeek-V4.1-Flash-1017"
DEFAULT_START = datetime(2026, 1, 1, tzinfo=timezone.utc)
SCENARIO_FUNCTIONS = ("cross-body", "honesty", "learning", "memory", "privacy",
                      "promises", "silence", "turn-taking")
BODY_NAME = "telegram"
CLI_BODY = "cli"
GAME_BODY = "game"
WHATSAPP_BODY = "whatsapp"
DEV_BODY = "dev"
OWNER = "person:owner"
OWNER_DM = "space:tg:1001"
TIME_RE = re.compile(r"^(\d{1,2}):(\d{2})(?::(\d{2}))?$")

# Neutral places used by the test organs; the prefix names the organ.
PLACE_BODIES = (("space:game", GAME_BODY), ("space:wa", WHATSAPP_BODY),
                ("space:cli", CLI_BODY))

# A test organ only needs the two chat capabilities of a real body.
CHAT_CAPABILITIES = (
    {"name": "say", "description": "Send a text message in the current place.",
     "schema": {"type": "object", "properties": {"text": {"type": "string"}},
                "required": ["text"]},
     "communicative": True, "long_running": False, "identity_changing": False},
    {"name": "react", "description": "React to a message with one emoji.",
     "schema": {"type": "object", "properties": {"emoji": {"type": "string"}},
                "required": ["emoji"]},
     "communicative": False, "long_running": False, "identity_changing": False},
)

# `dev` is the owner's workspace organ: it chats and runs the owner's code.
DEV_CAPABILITIES = (*CHAT_CAPABILITIES,
                    {"name": "run",
                     "description": "Run a command in the owner's workspace and report its "
                                    "output: files changed, tests, and the result text.",
                     "schema": {"type": "object", "properties": {"command": {"type": "string"}},
                                "required": ["command"]},
                     "communicative": False, "long_running": True,
                     "identity_changing": False})

TEST_CAPABILITIES = {GAME_BODY: CHAT_CAPABILITIES, WHATSAPP_BODY: CHAT_CAPABILITIES,
                     DEV_BODY: DEV_CAPABILITIES}

# Action kinds a test organ reports as a finished long action (an `act.result`).
TEST_LONG_RUNNING = {DEV_BODY: frozenset({"run"})}


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
    """Resolve LLM_BASE_URL, LLM_API_KEY, LLM_MODEL and the effort with defaults."""
    config = load_llm_config()
    base_url = config.get("base_url") or DEFAULT_BASE_URL
    model = config.get("model") or DEFAULT_MODEL
    api_key = config.get("api_key") or ""
    if not api_key:
        raise RuntimeError("LLM_API_KEY is required for a real-LLM replay (or use --fake)")
    return {"base_url": base_url, "api_key": api_key, "model": model,
            "effort": config.get("effort") or ""}


# ------------------------------------------------------------------- usage


@dataclass
class Usage:
    """Cumulative LLM call and token counters."""

    calls: int = 0
    prompt: int = 0
    cached: int = 0
    completion: int = 0
    reasoning: int = 0
    total: int = 0

    def add_payload(self, payload: dict[str, Any]) -> None:
        """Add one chat completion response body's usage to the counters."""
        usage = payload.get("usage") or {}
        details = usage.get("prompt_tokens_details") or {}
        reasoning = usage.get("completion_tokens_details") or {}
        prompt = int(usage.get("prompt_tokens") or 0)
        completion = int(usage.get("completion_tokens") or 0)
        self.prompt += prompt
        self.cached += int(details.get("cached_tokens") or 0)
        self.completion += completion
        self.reasoning += int(reasoning.get("reasoning_tokens") or 0)
        self.total += int(usage.get("total_tokens") or (prompt + completion))

    def minus(self, other: Usage) -> Usage:
        """Return the counter difference (for per-scenario usage)."""
        return Usage(self.calls - other.calls, self.prompt - other.prompt,
                     self.cached - other.cached, self.completion - other.completion,
                     self.reasoning - other.reasoning, self.total - other.total)


def _prompt_place(content: str) -> str | None:
    """Read the place a real prompt names in its `<place>` section."""
    match = re.search(r"<place>(.*?) \(", content)
    return match.group(1) if match else None


@dataclass(frozen=True)
class DeskItem:
    """One item on a wake's desk and the place or sources it came from."""

    source: str | None
    before: int | None
    sources: tuple[int, ...] | None = None


@dataclass(frozen=True)
class Desk:
    """One desk as the Reasoning received it: its place, its trigger and its items."""

    step: int
    place: str | None
    trigger: int | None
    items: tuple[DeskItem, ...]


@dataclass
class Tracker:
    """Shared usage counters and captured prompts."""

    usage: Usage = field(default_factory=Usage)
    prompts: list[tuple[int, str | None, str]] = field(default_factory=list)
    desks: list[Desk] = field(default_factory=list)
    step_index: int = 0
    answers_used: dict[tuple[int, str], int] = field(default_factory=dict)
    conn: Any = None

    def record_prompt(self, content: str, place: str | None = None) -> None:
        """Store one prompt with the step and place that produced it."""
        self.prompts.append((self.step_index, place or _prompt_place(content), content))

    async def record_desk(self, context: PFCContext) -> None:
        """Store every item's source and line id for one wake, for the privacy metric."""
        items = [DeskItem(line.place, line.id)
                 for line in (*context.recent, *context.recent_elsewhere)]
        items += [DeskItem(getattr(memory, "place", None), None,
                           tuple(getattr(memory, "sources", ()) or ()))
                  for memory in context.memories]
        if context.core_memory:
            items.append(DeskItem(None, None, await self._core_sources()))
        trigger = context.trigger.id if context.trigger is not None else None
        self.desks.append(Desk(self.step_index, context.place, trigger, tuple(items)))

    async def _core_sources(self) -> tuple[int, ...]:
        """Return the Log events the current core memory was built from."""
        if self.conn is None:
            return ()
        row = await (await self.conn.execute(
            "SELECT value FROM state WHERE key = 'core_memory'")).fetchone()
        return tuple((row[0] if row else {}).get("derived_from") or ())


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
    parts: list[str] = [context.core_memory, *context.people, *context.memories,
                        *context.doing]
    note = withheld_note(context)
    if note:
        parts.append(note)
    parts.extend(str(event.payload.get("description") or "") for event in context.recent)
    parts.extend(str(event.payload.get("description") or "") for event in context.recent_elsewhere)
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
    """A scripted Reasoning: replay recorded answers, else say "ok" and keep one memory."""

    def __init__(self, tracker: Tracker,
                 answers: dict[tuple[int, str], list[list[ToolCall]]] | None = None) -> None:
        """Share the tracker and the scenario's recorded answers."""
        self._tracker = tracker
        self._answers = answers or {}

    def _recorded(self, tool: str) -> list[ToolCall] | None:
        """Return the next recorded call list for this step, or None when none is due."""
        key = (self._tracker.step_index, tool)
        queue = self._answers.get(key)
        if not queue:
            return None
        used = self._tracker.answers_used.get(key, 0)
        self._tracker.answers_used[key] = used + 1
        return list(queue[used]) if used < len(queue) else None

    async def decide(self, context: PFCContext) -> list[ToolCall]:
        """Return the recorded answer for this step, else say(text="ok") in the first round."""
        self._tracker.usage.calls += 1
        self._tracker.record_prompt(_fake_context_text(context), context.place)
        if not context.results:
            await self._tracker.record_desk(context)
        recorded = self._recorded("decide")
        if recorded is not None:
            return recorded
        if context.results:
            return []  # a scripted model has nothing more to add after its thought round
        return [ToolCall("act", {"capability": "telegram.say", "arguments": {"text": "ok"}})]

    async def call(self, messages: list[dict[str, Any]], tools: list[dict[str, Any]], *,
                   tool_choice: str = "required") -> list[ToolCall]:
        """Return the recorded answer for this step, else store one sleep memory."""
        self._tracker.usage.calls += 1
        self._tracker.record_prompt(str(messages[-1].get("content") or ""))
        recorded = self._recorded("call")
        if recorded is not None:
            return recorded
        names = {tool["function"]["name"] for tool in tools}
        if "store_memories" in names:
            return [ToolCall("store_memories", _fake_sleep_arguments(messages))]
        return [ToolCall("act", {"capability": "telegram.say", "arguments": {"text": "ok"}})]

    async def aclose(self) -> None:
        """Nothing to close."""


class TrackedPFC(Reasoning):
    """A real Reasoning that records the desk of every wake for the privacy metric."""

    def __init__(self, tracker: Tracker, *args: Any, **kwargs: Any) -> None:
        """Keep the tracker beside the real Reasoning."""
        super().__init__(*args, **kwargs)
        self._tracker = tracker

    async def decide(self, context: PFCContext) -> list[ToolCall]:
        """Record the desk, then ask the model as usual."""
        if not context.results:
            await self._tracker.record_desk(context)
        return await super().decide(context)


class FakeBody:
    """A body that records every Action dispatched to it."""

    def __init__(self, name: str = BODY_NAME) -> None:
        """Start with an empty action list and the given body name."""
        self.name = name
        self.actions: list[Action] = []

    def capabilities(self) -> Sequence[dict[str, Any]]:
        """Declare the same capabilities as the real body of that name."""
        if self.name == CLI_BODY:
            return CLI_CAPABILITIES
        if self.name == BODY_NAME:
            return TELEGRAM_CAPABILITIES
        return TEST_CAPABILITIES[self.name]

    async def act(self, action: Action) -> Any:
        """Record the action; a long action reports the output it produced."""
        self.actions.append(action)
        if action.kind in TEST_LONG_RUNNING.get(self.name, frozenset()):
            return {"ok": True, "finished": True, "text": self._output(action)}

    @staticmethod
    def _output(action: Action) -> str:
        """Render a command's observable result: what changed and what the tests said."""
        command = str(action.payload.get("command") or "")
        return (f"$ {command}\n"
                "src/parser.py | 9 ++++----- (the failing branch now returns the parsed node)\n"
                "tests/test_parser.py | 2 +-\n"
                "3 files changed, 41 insertions(+), 9 deletions(-)\n"
                "pytest: 12 passed in 0.51s")


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
    answers: dict[tuple[int, str], list[list[ToolCall]]]
    zone: ZoneInfo
    function: str = ""
    capability: str = ""


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
    should: str
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
    prompts: list[tuple[int, str | None, str]]
    events: list[dict[str, Any]]
    memories: int
    usage: Usage
    desks: list[Desk] = field(default_factory=list)
    checks: list[CheckResult] = field(default_factory=list)
    violations: list[str] = field(default_factory=list)
    log_texts: list[str] = field(default_factory=list)


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
        should = str(step.get("should") or "").strip().lower()
        if should and should not in (*ANSWER_WORDS, *SILENT_WORDS):
            raise ValueError(f"{path}: step {index} has an unknown should value")
    function = str(data.get("function") or "").strip()
    if function not in SCENARIO_FUNCTIONS:
        raise ValueError(f"{path}: function must be one of {sorted(SCENARIO_FUNCTIONS)}")
    if path.parent.name != function:
        raise ValueError(f"{path}: function {function!r} does not match its folder")
    capability = str(data.get("capability") or "").strip()
    if not capability or "\n" in capability:
        raise ValueError(f"{path}: capability must be one line naming what it proves")
    return Scenario(path=path, name=str(data.get("name") or path.stem),
                    description=str(data.get("description") or ""), steps=steps,
                    expectations=list(data.get("expectations") or []),
                    answers=load_answers(path, data), zone=zone,
                    function=function, capability=capability)


def load_answers(path: pathlib.Path,
                 data: dict[str, Any]) -> dict[tuple[int, str], list[list[ToolCall]]]:
    """Read the recorded model answers, keyed by step and tool (`decide` or `call`)."""
    answers: dict[tuple[int, str], list[list[ToolCall]]] = {}
    for index, entry in enumerate(data.get("model_answers") or [], 1):
        if not isinstance(entry, dict):
            raise ValueError(f"{path}: model_answers[{index}] must be a mapping")
        step, tool = int(entry.get("step") or 0), str(entry.get("tool") or "decide")
        calls = [ToolCall(str(raw["name"]), dict(raw.get("arguments") or {}))
                 for raw in entry.get("calls") or []]
        answers.setdefault((step, tool), []).append(calls)
    return answers


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


SHOULD_ANSWER = "answer"
SHOULD_SILENT = "silent"
ANSWER_WORDS = ("answer", "reply", "speak")
SILENT_WORDS = ("silent", "quiet", "stay_silent")


def should_field(raw: dict[str, Any]) -> str:
    """Ground truth for one message step: should the soul answer or stay silent."""
    value = str(raw.get("should") or "").strip().lower()
    if value in ANSWER_WORDS:
        return SHOULD_ANSWER
    if value in SILENT_WORDS:
        return SHOULD_SILENT
    return SHOULD_ANSWER if bool(raw.get("addressed", False)) else SHOULD_SILENT


def place_kind(place: str) -> str:
    """Guess direct vs shared from the neutral place id (test organs only)."""
    if ":dm:" in place:
        return "direct"
    if place.startswith("space:tg:-") or place.endswith("@g.us"):
        return "shared"
    if ":group:" in place or place.startswith(f"space:{GAME_BODY}:"):
        return "shared"
    return "direct"


def body_for(place: str, explicit: str | None = None) -> str:
    """Resolve which organ a percept came from: the step's body, else its place prefix."""
    if explicit:
        return explicit
    for prefix, body in PLACE_BODIES:
        if place.startswith(prefix):
            return body
    return BODY_NAME


def _event_row(row: Sequence[Any]) -> dict[str, Any]:
    payload = row[6] or {}
    text = payload.get("description")
    if text is None:
        text = (payload.get("arguments") or {}).get("text")
    return {"id": row[0], "ts": row[1], "kind": row[2], "who": row[3], "place": row[4],
            "text": text, "body": row[5], "payload": payload}


def said(event: dict[str, Any], capabilities: Mapping[str, Any]) -> bool:
    """Return whether one act.out is a message a person received (12.18)."""
    capability = str((event.get("payload") or {}).get("capability") or "")
    return bool((capabilities.get(capability) or {}).get("communicative", False))


# ------------------------------------------------------------------- runner


class ReplayRunner:
    """Runs scenarios against a real kernel on a throwaway database."""

    def __init__(self, url: str, settings: dict[str, Any], fake: bool, tracker: Tracker,
                 llm: dict[str, str]) -> None:
        """Wire the runner to the database, the settings, and the LLM settings."""
        self.url, self.settings, self.fake, self.tracker = url, settings, fake, tracker
        self.llm = llm
        self.zone = ZoneInfo(str(settings.get("timezone") or "UTC"))
        self.embedder: Any = FakeEmbedder() if fake else FastEmbedder()
        self.clock = FakeClock()
        self.conn: psycopg.AsyncConnection[Any] | None = None
        self.kernel: RecordingKernel | None = None
        self.answers: dict[tuple[int, str], list[list[ToolCall]]] = {}
        self._last_id = 0
        self._message_id = 0

    def _make_pfc(self) -> Any:
        """Build a fake Reasoning with the scenario's answers, or a real HTTP-tracked Reasoning."""
        if self.fake:
            return FakePFC(self.tracker, self.answers)
        client = httpx.AsyncClient(
            transport=UsageTransport(httpx.AsyncHTTPTransport(), self.tracker), timeout=90.0)
        return TrackedPFC(self.tracker, self.llm["base_url"], self.llm["api_key"],
                          self.llm["model"], effort=self.llm.get("effort") or "",
                          settings=self.settings, client=client, timeout=90.0)

    def _make_kernel(self) -> Kernel:
        """Build a fresh kernel that records its appraisals."""
        pfc = self._make_pfc()
        kernel = RecordingKernel(self.conn, clock=self.clock, pfc=pfc,
                                 settings=self.settings, embedder=self.embedder)
        for name in (BODY_NAME, CLI_BODY, GAME_BODY, WHATSAPP_BODY, DEV_BODY):
            kernel.register_adapter(FakeBody(name))
        self.kernel = kernel
        return kernel

    async def run(self, scenario: Scenario) -> ScenarioResult:
        """Reset the schema and replay one scenario from the first step."""
        start_usage = Usage(**vars(self.tracker.usage))
        self.tracker.prompts.clear()
        self.tracker.desks.clear()
        self.tracker.answers_used.clear()
        self.answers = scenario.answers
        self._last_id = 0
        self._message_id = 0
        self.clock = FakeClock(initial_time=self._initial_time(scenario))
        async with await psycopg.AsyncConnection.connect(self.url, autocommit=True) as conn:
            self.conn = self.tracker.conn = conn
            await conn.execute("DROP SCHEMA IF EXISTS public CASCADE")
            await conn.execute("CREATE SCHEMA public")
            await migrate(conn, backup_dir=REPORT_DIR / "backups")
            await register_vector_async(conn)
            self._make_kernel()
            await self.kernel.recover()
            await self.kernel.run_until_idle()
            steps = [await self._run_step(index, raw)
                     for index, raw in enumerate(scenario.steps, 1)]
            replies = self._replies(steps)
            desks = list(self.tracker.desks)
            result = ScenarioResult(
                scenario=scenario, steps=steps,
                replies=replies, prompts=list(self.tracker.prompts), desks=desks,
                events=[event for step in steps for event in step.events],
                memories=await self._memory_count(),
                usage=self.tracker.usage.minus(start_usage),
                violations=(await self._leak_violations(replies)
                            + await desk_violations(self.conn, desks)),
                log_texts=await self._log_texts(steps))
        self.conn = self.tracker.conn = None
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
                          should=should_field(raw) if kind == "message" else "",
                          appraisals=list(self.kernel.appraisals),
                          replies=[(event["place"] or "", str(event["text"]))
                                   for event in events
                                   if event["kind"] == "act.out" and self._said(event)],
                          events=events)

    async def _feed(self, *, who: str, place: str, text: str, addressed: bool,
                    body: str | None = None) -> None:
        """Put one percept and let the single consumer process it to completion."""
        self._message_id += 1
        name = body_for(place, body)
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
            "SELECT id, ts, kind, who, place, body, payload FROM events "
            "WHERE id > %s ORDER BY id", (after_id,))
        events = [_event_row(row) for row in await cur.fetchall()]
        await self._attach_expectations(events)
        return events

    async def _attach_expectations(self, events: Sequence[dict[str, Any]]) -> None:
        """Add the cue and condition behind every Expectation a created event names."""
        ids = [int((event["payload"] or {}).get("expectation_id") or 0) for event in events
               if event["kind"] == "expectation.created"]
        ids = sorted({found for found in ids if found})
        if not ids:
            return
        cur = await self.conn.execute(
            "SELECT id, cue_event, condition FROM expectations WHERE id = ANY(%s)", (ids,))
        rows = {int(row[0]): (row[1] or {}, row[2] or {}) for row in await cur.fetchall()}
        for event in events:
            if event["kind"] != "expectation.created":
                continue
            found = rows.get(int((event["payload"] or {}).get("expectation_id") or 0))
            if found:
                event["cue_event"], event["condition"] = found

    async def _memory_count(self) -> int:
        """Return how many memories the scenario's sleep produced."""
        row = await (await self.conn.execute("SELECT count(*) FROM memories")).fetchone()
        return int(row[0])

    def _replies(self, steps: Sequence[StepRecord]) -> list[Reply]:
        """Flatten every communicative act.out with the step that produced it."""
        return [Reply(step.index, event["ts"], event["place"] or "", str(event["text"] or ""))
                for step in steps for event in step.events
                if event["kind"] == "act.out" and self._said(event)]

    def _said(self, event: dict[str, Any]) -> bool:
        """Return whether one act.out is a message, judged by its capability."""
        capabilities = self.kernel.tools.capabilities if self.kernel is not None else {}
        return said(event, capabilities)

    async def _leak_violations(self, replies: Sequence[Reply]) -> list[str]:
        """Return every reply that carries a private line its audience may not see."""
        cur = await self.conn.execute(
            "SELECT text, derived_from FROM memories WHERE valid_to IS NULL AND text IS NOT NULL")
        private = [(str(text), sources) for text, sources in await cur.fetchall()]
        found: list[str] = []
        for reply in replies:
            for text, sources in private:
                if text in reply.text and not await privacy.memory_visible(
                        self.conn, context=reply.place, sources=sources):
                    found.append(f"step {reply.step}: a private memory was said "
                                 f"in `{reply.place}`")
        return found

    async def _log_texts(self, steps: Sequence[StepRecord]) -> list[str]:
        """Return every text the scenario's Log holds, for the honesty scan."""
        cur = await self.conn.execute(
            "SELECT payload->>'description' FROM events WHERE payload ? 'description'")
        texts = [str(row[0]) for row in await cur.fetchall() if row[0]]
        cur = await self.conn.execute("SELECT text FROM memories WHERE text IS NOT NULL")
        texts += [str(row[0]) for row in await cur.fetchall() if row[0]]
        texts += [step.text for step in steps if step.kind == "message"]
        return texts


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


def _check_mentions_any(spec: dict[str, Any], result: ScenarioResult) -> CheckResult:
    replies = _scoped_replies(spec, result)
    needles = [str(item).lower() for item in spec.get("texts") or ()]
    hits = [reply for reply in replies if any(item in reply.text.lower() for item in needles)]
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
    replies = _scoped_replies(spec, result)
    hits = [reply for reply in replies if _place_matches(spec, reply.place)]
    want = spec.get("count")
    if want is None and spec.get("min") is not None:
        floor = int(spec["min"])
        return CheckResult(str(spec["label"]), len(hits) >= floor,
                           f"{len(hits)} >= {floor}: {_detail(hits) if hits else _detail(replies)}")
    if want is None:
        return CheckResult(str(spec["label"]), bool(hits),
                           _detail(hits) if hits else _detail(replies))
    want = int(want)
    return CheckResult(str(spec["label"]), len(hits) == want,
                       f"{len(hits)} == {want}: {_detail(hits) if hits else _detail(replies)}")


def _place_matches(spec: dict[str, Any], place: str) -> bool:
    """Match a reply place against an exact place, a prefix, or a list of places."""
    places = spec.get("place_any")
    if places is not None:
        return place in {str(item) for item in places}
    prefix = spec.get("place_prefix")
    if prefix is not None:
        return place.startswith(str(prefix))
    return place == str(spec.get("place") or "")


def _scoped_events(spec: dict[str, Any], result: ScenarioResult) -> list[dict[str, Any]]:
    """Filter events to the steps the check names, or to every step."""
    steps = spec.get("steps")
    if steps is not None:
        wanted = {int(step) for step in steps}
        return [event for step in result.steps if step.index in wanted for event in step.events]
    step = spec.get("step")
    if step is not None:
        wanted_step = int(step)
        return [event for step in result.steps if step.index == wanted_step
                for event in step.events]
    return list(result.events)


def _matches_event(spec: dict[str, Any], event: dict[str, Any]) -> bool:
    """Return whether one event matches a check's kind, actors and payload fields."""
    if event["kind"] != str(spec.get("kind") or ""):
        return False
    for field in ("who", "place", "body"):
        if spec.get(field) is not None and event.get(field) != spec[field]:
            return False
    for key, value in (spec.get("payload") or {}).items():
        if str((event.get("payload") or {}).get(key)) != str(value):
            return False
    needle = spec.get("text_contains")
    if needle is not None:
        haystack = json.dumps(event, default=str).lower()
        if str(needle).lower() not in haystack:
            return False
    return True


def _check_event_seen(spec: dict[str, Any], result: ScenarioResult) -> CheckResult:
    hits = [event for event in _scoped_events(spec, result) if _matches_event(spec, event)]
    want = int(spec.get("count") or 1)
    return CheckResult(str(spec["label"]), len(hits) >= want,
                       f"{len(hits)} {spec.get('kind')} >= {want}")


def _check_no_event(spec: dict[str, Any], result: ScenarioResult) -> CheckResult:
    hits = [event for event in _scoped_events(spec, result) if _matches_event(spec, event)]
    allowed = int(spec.get("count") or 0)
    return CheckResult(str(spec["label"]), len(hits) <= allowed,
                       f"{len(hits)} {spec.get('kind')} <= {allowed}")


def _first_event_id(spec: dict[str, Any], kind: str,
                    result: ScenarioResult) -> int | None:
    hits = [event for event in _scoped_events(spec, result) if event["kind"] == kind]
    return min((int(event["id"]) for event in hits), default=None)


def _check_event_order(spec: dict[str, Any], result: ScenarioResult) -> CheckResult:
    first = _first_event_id(spec, str(spec.get("first") or ""), result)
    then = _first_event_id(spec, str(spec.get("then") or ""), result)
    started = first is not None and then is not None and first < then
    detail = f"{spec.get('first')}#{first} before {spec.get('then')}#{then}"
    return CheckResult(str(spec["label"]), started, detail)


def _scoped_prompts(spec: dict[str, Any],
                    result: ScenarioResult) -> list[tuple[int, str | None, str]]:
    """Filter recorded prompts to the steps and the place the check names."""
    steps = spec.get("steps")
    wanted = {int(step) for step in steps} if steps is not None else None
    scoped = [item for item in result.prompts if wanted is None or item[0] in wanted]
    if spec.get("step") is not None:
        scoped = [item for item in scoped if item[0] == int(spec["step"])]
    if any(key in spec for key in ("place", "place_prefix", "place_any")):
        scoped = [item for item in scoped if _place_matches(spec, item[1] or "")]
    return scoped


def _check_prompt_absent(spec: dict[str, Any], result: ScenarioResult) -> CheckResult:
    needle = str(spec.get("text") or "").lower()
    hits = [index for index, _, content in _scoped_prompts(spec, result)
            if needle in content.lower()]
    return CheckResult(str(spec["label"]), not hits,
                       f"prompt steps with {needle!r}: {hits or 'none'}")


def _check_prompt_contains(spec: dict[str, Any], result: ScenarioResult) -> CheckResult:
    needle = str(spec.get("text") or "").lower()
    hits = [index for index, _, content in _scoped_prompts(spec, result)
            if needle in content.lower()]
    return CheckResult(str(spec["label"]), bool(hits),
                       f"prompt steps with {needle!r}: {hits or 'none'}")


def _check_sleep_done(spec: dict[str, Any], result: ScenarioResult) -> CheckResult:
    hits = [event for event in result.events if event["kind"] == "sleep.done"]
    return CheckResult(str(spec["label"]), bool(hits), f"{len(hits)} sleep.done")


def _check_memories(spec: dict[str, Any], result: ScenarioResult) -> CheckResult:
    want = int(spec.get("count") or 1)
    return CheckResult(str(spec["label"]), result.memories >= want,
                       f"{result.memories} memories stored (need {want})")


def _check_max_calls(spec: dict[str, Any], result: ScenarioResult) -> CheckResult:
    """Count the act.out calls of one capability in the named chain."""
    name = spec.get("capability")
    calls = [event for event in _scoped_events(spec, result) if event["kind"] == "act.out"
             and (name is None or (event.get("payload") or {}).get("capability") == name)]
    want = int(spec.get("count") or 1)
    return CheckResult(str(spec["label"]), len(calls) <= want, f"{len(calls)} calls <= {want}")


def _check_no_repeat_call(spec: dict[str, Any], result: ScenarioResult) -> CheckResult:
    """Fail when a chain calls one capability with the same arguments more than once."""
    seen: set[tuple[Any, str]] = set()
    repeats: list[str] = []
    for event in _scoped_events(spec, result):
        if event["kind"] != "act.out":
            continue
        payload = event.get("payload") or {}
        key = (payload.get("capability"),
               json.dumps(payload.get("arguments") or {}, sort_keys=True, default=str))
        if key in seen:
            repeats.append(f"#{event['id']} {key[0]}")
        seen.add(key)
    return CheckResult(str(spec["label"]), not repeats,
                       f"repeated: {', '.join(repeats)}" if repeats else "no repeat")


def _check_signal(spec: dict[str, Any], result: ScenarioResult) -> CheckResult:
    """Compare the highest value of one signal over the named steps."""
    name = str(spec.get("signal") or "")
    wanted = spec.get("steps")
    if wanted is not None:
        wanted = {int(step) for step in wanted}
    if spec.get("step") is not None:
        wanted = {int(spec["step"])}
    values = [appraisal.signals.get(name, 0.0) for step in result.steps
              if wanted is None or step.index in wanted for appraisal in step.appraisals]
    peak = max(values, default=0.0)
    floor, ceiling = spec.get("min"), spec.get("max")
    ok = bool(values) and (floor is None or peak >= float(floor)) \
        and (ceiling is None or peak <= float(ceiling))
    return CheckResult(str(spec["label"]), ok,
                       f"{name} max {peak:.3f} over {len(values)} appraisals")


CHECKS: dict[str, Callable[[dict[str, Any], ScenarioResult], CheckResult]] = {
    "no_reply": _check_no_reply,
    "reply": _check_reply,
    "mentions": _check_mentions,
    "mentions_any": _check_mentions_any,
    "remind_at": _check_remind_at,
    "min_replies": _check_min_replies,
    "reply_in": _check_reply_in,
    "prompt_contains": _check_prompt_contains,
    "prompt_absent": _check_prompt_absent,
    "event_seen": _check_event_seen,
    "no_event": _check_no_event,
    "event_order": _check_event_order,
    "sleep_done": _check_sleep_done,
    "memories_stored": _check_memories,
    "signal": _check_signal,
    "max_calls": _check_max_calls,
    "no_repeat_call": _check_no_repeat_call,
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


DETAIL_LIMIT = 300
EXPECTATION_LIMIT = 400
SECRET_NAME_RE = re.compile(r"(_KEY|_TOKEN|_SECRET|_PASSWORD)$", re.IGNORECASE)
SECRET_MIN_LENGTH = 8


def _redact(text: str) -> str:
    """Replace any configured secret that leaked into a payload text."""
    for name, value in os.environ.items():
        if len(value) >= SECRET_MIN_LENGTH and SECRET_NAME_RE.search(name):
            text = text.replace(value, "***")
    return text


def _detail_text(value: Any, limit: int = DETAIL_LIMIT) -> str:
    """Flatten one payload field into one redacted, truncated report line."""
    return _clamp(_redact(" ".join(str(value).split())), limit)


def _clamp(text: str, limit: int) -> str:
    """Cut a report line at `limit` characters, marking the cut with an ellipsis."""
    return text if len(text) <= limit else text[:limit - 1] + "…"


def _json_field(value: Any) -> str:
    """Render one JSON payload field compactly and in a stable order."""
    return json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":"))


def _expectation_created(event: dict[str, Any]) -> str:
    """Render what an Expectation was told to watch: its kind, cue, condition and about."""
    payload = event.get("payload") or {}
    line = (f"expectation.created: kind={payload.get('kind') or 'unknown'} "
            f"cue_event={_json_field(event.get('cue_event') or {})} "
            f"condition={_json_field(event.get('condition') or {})} "
            f"about={payload.get('about') or '(none)'}")
    return _detail_text(line, EXPECTATION_LIMIT)


def _event_detail(event: dict[str, Any]) -> str | None:
    """Return the one line a step detail shows for an event, or None for other kinds."""
    kind = str(event.get("kind") or "")
    payload = event.get("payload") or {}
    if kind == "error":
        return f"error: {_detail_text(payload.get('error') or '(no message)')}"
    if kind == "decision.silent":
        reason = payload.get("reason")
        return f"decision.silent: {_detail_text(reason)}" if reason else None
    if kind == "decision.note":
        act = str(payload.get("act") or "note")
        text = payload.get("thought") or payload.get("reason") or payload.get("new")
        return f"decision.note ({act}): {_detail_text(text)}" if text else None
    if kind == "expectation.created":
        return _expectation_created(event)
    if kind == "expectation.fired":
        text = payload.get("text") or payload.get("about") or _json_field(payload)
        return f"expectation.fired: {_detail_text(text)}"
    if kind == "expectation.reminded":
        text = payload.get("text") or payload.get("about") or _json_field(payload)
        return f"expectation.reminded: {_detail_text(text)}"
    if kind == "expectation.resolved":
        return f"expectation.resolved: {_detail_text(payload.get('status') or 'unknown')}"
    return None


def _step_details(step: StepRecord) -> list[str]:
    """Render one line per error or reason the step's events carry."""
    return [f"- step {step.index} — {detail}" for detail in
            (_event_detail(event) for event in step.events) if detail]


def _honesty_details(result: ScenarioResult) -> list[str]:
    """Render every spoken act that quotes text the Log does not hold."""
    lines = []
    for reply in result.replies:
        quote = _fabricated_quote(reply.text, result.log_texts)
        if quote is not None:
            lines.append(f"- step {reply.step} — honesty: {_detail_text(reply.text)}; "
                         f"not in the log: {_detail_text(quote)}")
    return lines


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
    if scenario.function or scenario.capability:
        lines += [f"**Function:** `{scenario.function}` — {scenario.capability}", ""]
    lines += ["| # | Time | Who | Place | Input | Decision | Reflex | S | Theta | P | u "
              "| Reason | Reply | Events |",
              "|---|---|---|---|---|---|---|---|---|---|---|---|---|---|"]
    lines += [_step_row(step, zone) for step in result.steps]
    details = [line for step in result.steps for line in _step_details(step)]
    details += _honesty_details(result)
    if details:
        lines += ["", "**Step details**", "", *details]
    lines += ["", f"**Expectations: {status}**", ""]
    if not result.checks:
        lines += ["- (no expectations declared)", ""]
    for check in result.checks:
        mark = "PASS" if check.passed else "FAIL"
        lines.append(f"- **{mark}** — {check.label}: {check.detail}")
    usage = result.usage
    lines += ["", f"LLM calls: {usage.calls} · prompt {usage.prompt} · cached "
              f"{usage.cached} · completion {usage.completion} (reasoning {usage.reasoning})"
              f" · total {usage.total} tokens", ""]
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


# ----------------------------------------------------------------- metrics

QUOTED_RE = re.compile(r'"([^"\n]{20,})"|“([^”\n]{20,})”')
DAY_SECONDS = 86400.0


@dataclass
class Metrics:
    """The five headline numbers of one replay run, in their reported order."""

    should_answer: int = 0
    answered: int = 0
    should_silent: int = 0
    false_speaks: int = 0
    tokens: int = 0
    simulated_days: float = 0.0
    scenarios: int = 0
    violations: list[str] = field(default_factory=list)
    speaking_turns: int = 0
    honest_turns: int = 0

    @staticmethod
    def _rate(part: int, whole: int) -> float | None:
        """Return part/whole, or None when there is nothing to divide."""
        return part / whole if whole else None

    def missed_help_rate(self) -> float | None:
        """Share of messages that should be answered but did not pass the gate."""
        return self._rate(self.should_answer - self.answered, self.should_answer)

    def false_speak_rate(self) -> float | None:
        """Share of messages that should have stayed silent that got a reply."""
        return self._rate(self.false_speaks, self.should_silent)

    def tokens_per_day(self) -> float | None:
        """Tokens spent per simulated day across every scenario."""
        return self.tokens / self.simulated_days if self.simulated_days else None

    def honesty_compliance(self) -> float | None:
        """Share of communicative acts that quote only text the log holds."""
        return self._rate(self.honest_turns, self.speaking_turns)


def _simulated_days(result: ScenarioResult) -> float:
    """Return how many days one scenario's steps simulate."""
    stamps = [step.at for step in result.steps]
    if len(stamps) < 2:
        return 0.0
    return (max(stamps) - min(stamps)).total_seconds() / DAY_SECONDS


def _fabricated_quote(text: str, corpus: Sequence[str]) -> str | None:
    """Return the first quoted span of `text` that appears nowhere in the log."""
    for match in QUOTED_RE.finditer(text):
        span = match.group(1) or match.group(2)
        if not any(span in source for source in corpus):
            return span
    return None


async def desk_violations(conn, desks: Sequence[Desk]) -> list[str]:
    """Return every desk item whose own audience does not cover that desk (D45, D50, D53).

    Independent of `privacy.visible`: it restates absolute line 2 from the Log, so a bug in
    the context hook or in `visible` itself cannot pass the metric.
    """
    found: list[str] = []
    for desk in desks:
        if not desk.place or await privacy.owner_direct(conn, desk.place):
            continue
        need = set(await privacy.audience(conn, desk.place, desk.trigger)) - {OWNER}
        reported: set[tuple[str | None, tuple[int, ...] | None]] = set()
        for item in desk.items:
            key = (item.source, item.sources)
            if not need or key in reported:
                continue
            if item.sources is not None:
                rows = await (await conn.execute(
                    "SELECT id, place FROM events WHERE id = ANY(%s) AND place IS NOT NULL",
                    (list(item.sources),))).fetchall()
                audiences = [((set(await privacy.audience(conn, place, source)) - {OWNER})
                              | set(await privacy.shared(conn, place, desk.trigger)))
                             for source, place in rows]
                covered = bool(item.sources) and all(need <= have for have in audiences)
                label = f"sources {list(item.sources)}" if item.sources else "no known source"
            else:
                covered = bool(item.source) and need <= (
                    (set(await privacy.audience(conn, item.source, item.before)) - {OWNER})
                    | set(await privacy.shared(conn, item.source, desk.trigger)))
                label = item.source or "no known source"
            if not covered:
                reported.add(key)
                found.append(f"step {desk.step}: an item from `{label}` was on the "
                             f"`{desk.place}` desk, whose audience it does not cover")
    return found


def _line_violations(result: ScenarioResult) -> list[str]:
    """Return every absolute-line breach the scenario's own checks recorded."""
    found = list(result.violations)
    specs = [normalize_check(raw) for raw in result.scenario.expectations]
    for spec, check in zip(specs, result.checks):
        if spec.get("type") == "prompt_absent" and not check.passed:
            found.append(f"{result.scenario.name}: {check.label} ({check.detail})")
    return found


def compute_metrics(results: Sequence[ScenarioResult], usage: Usage) -> Metrics:
    """Aggregate the five headline metrics over a finished replay run."""
    metrics = Metrics(tokens=usage.total, scenarios=len(results))
    for result in results:
        metrics.simulated_days += _simulated_days(result)
        spoken = {reply.step for reply in result.replies}
        for step in result.steps:
            if step.kind != "message" or step.should == "":
                continue
            if step.should == SHOULD_ANSWER:
                metrics.should_answer += 1
                if any(item.decision == "wake" for item in step.appraisals):
                    metrics.answered += 1
            else:
                metrics.should_silent += 1
                if step.index in spoken:
                    metrics.false_speaks += 1
        metrics.violations += _line_violations(result)
        for reply in result.replies:
            metrics.speaking_turns += 1
            if _fabricated_quote(reply.text, result.log_texts) is None:
                metrics.honest_turns += 1
    return metrics


def _share(value: float | None) -> str:
    """Render a share as a percentage, or n/a when there was nothing to measure."""
    return "n/a" if value is None else f"{value * 100:.1f}%"


def metric_lines(metrics: Metrics, *, markdown: bool = False) -> list[str]:
    """Render the five metrics in their fixed order, missed-help first."""
    bold = (lambda name: f"**{name}**") if markdown else str
    missed, answered, should = (metrics.missed_help_rate(), metrics.answered,
                                metrics.should_answer)
    tokens_day = metrics.tokens_per_day()
    honesty = metrics.honesty_compliance()
    per_day = "n/a" if tokens_day is None else f"{tokens_day:.1f}"
    days = "n/a" if not metrics.simulated_days else f"{metrics.simulated_days:.2f}"
    lines = [
        f"1. {bold('Missed-help rate: ' + _share(missed))} — "
        f"{should - answered} of {should} messages that should have been answered did not "
        f"pass the gate ({answered}/{should} helped).",
        f"2. {bold('False-speak rate: ' + _share(metrics.false_speak_rate()))} — "
        f"{metrics.false_speaks} of {metrics.should_silent} messages that should have stayed "
        f"silent got a reply.",
        f"3. {bold('Tokens per simulated day: ' + per_day)} — {metrics.tokens} tokens over "
        f"{days} simulated days across {metrics.scenarios} scenarios.",
        f"4. {bold('Absolute-line violations: ' + str(len(metrics.violations)))} — must be 0"
        + (f"; {'; '.join(metrics.violations)}." if metrics.violations else "."),
        f"5. {bold('Honesty-principle compliance: ' + _share(honesty))} — "
        f"{metrics.honest_turns} of {metrics.speaking_turns} communicative acts quote only "
        f"text the log holds (no fabricated quotation).",
    ]
    return lines


def render_report(results: Sequence[ScenarioResult], usage: Usage, mode: str,
                  model: str, settings: dict[str, Any], when: datetime,
                  zone: ZoneInfo, effort: str = "") -> str:
    """Build the full markdown report, with the model and the effort that answered."""
    lines = [f"# Replay report — {when:%Y-%m-%d %H:%M}", "",
             f"- Mode: **{mode}**", f"- Model: `{model}`",
             f"- Effort: `{effort or 'default'}`", f"- Timezone: `{zone.key}`",
             f"- Scenarios: {len(results)}", f"- Report generated: {when:%Y-%m-%d %H:%M:%S}",
             ""]
    metrics = compute_metrics(results, usage)
    lines += ["## Headline metrics", "", *metric_lines(metrics, markdown=True), ""]
    values = constants(settings)
    lines += ["## Cognition configuration", "",
              f"- Knobs: `{json.dumps(settings.get('knobs') or {}, sort_keys=True)}`",
              f"- Calibration: `{json.dumps(settings.get('calibration') or {}, sort_keys=True)}`",
              f"- Constants: `{json.dumps(values, sort_keys=True)}`",
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
              f"{usage.completion} (reasoning {usage.reasoning}) · total {usage.total}", ""]
    return "\n".join(lines)


def report_label(paths: Sequence[pathlib.Path]) -> str:
    """Name a report by the scenarios it covers: `all`, or their numbers in order."""
    if len(paths) == len(scenario_paths([])):
        return "all"
    return "-".join(path.name.split("_", 1)[0] for path in paths)


def write_report(text: str, when: datetime, *, fake: bool, label: str) -> pathlib.Path:
    """Write the report and a copy under latest.md, prune old ones, return the path."""
    REPORT_DIR.mkdir(parents=True, exist_ok=True)
    path = REPORT_DIR / f"{when:%Y-%m-%d-%H%M}-{'fake' if fake else 'real'}-{label}.md"
    path.write_text(text, encoding="utf-8")
    (REPORT_DIR / LATEST_NAME).write_text(text, encoding="utf-8")
    _prune_reports()
    return path


def _prune_reports(keep: int = REPORT_KEEP) -> None:
    """Delete every report beyond the newest `keep`; latest.md and backups stay."""
    reports = [path for path in REPORT_DIR.glob("*.md") if REPORT_FILE_RE.match(path.name)]
    reports.sort(key=lambda path: path.stat().st_mtime, reverse=True)
    for path in reports[keep:]:
        path.unlink()


# ------------------------------------------------------- capability record

CAPABILITY_PATH = SCENARIO_DIR / "README.md"
RECORD_RUNS = 3
GENERATED_MARK = "<!-- Generated by scripts/replay.py; do not edit by hand. -->"
REPORT_HEADING_RE = re.compile(r"^## (.+)$")
REPORT_MODE_RE = re.compile(r"^- Mode: \*\*(.+)\*\*$")
REPORT_MODEL_RE = re.compile(r"^- Model: `([^`]+)`$")
REPORT_EFFORT_RE = re.compile(r"^- Effort: `([^`]+)`$")
REPORT_WHEN_RE = re.compile(r"^- Report generated: (\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2})")
REPORT_CHECK_RE = re.compile(r"^- \*\*(PASS|FAIL)\*\* — ")
REPORT_SKIP = {"Headline metrics", "Cognition configuration", "Summary"}


@dataclass(frozen=True)
class Capability:
    """One scenario's claim: its brain function and the one line it proves."""

    name: str
    function: str
    capability: str


@dataclass
class CapabilityVerdict:
    """The checks one scenario passed, out of the checks it declared, in one run."""

    passed: int
    total: int


@dataclass
class ReportRun:
    """One replay run: its mode, model, effort, time, and per-scenario verdicts."""

    path: pathlib.Path
    when: datetime
    mode: str
    model: str
    results: dict[str, CapabilityVerdict]
    effort: str = ""


REAL_RECORD_HEADING = "## Real-model record"
REAL_RECORD_ROWS = 3
REAL_ROW = re.compile(r"^\| ([^|]+?) \| (PASS|FAIL) \| ([^|]*?) \| ([^|]*?) \| ([^|]*?) "
                      r"\| ([^|]*?) \| ([^|]*?) \|$")


@dataclass(frozen=True)
class RealRecord:
    """One scenario's stored real-model verdict, kept in the capability record itself."""

    scenario: str
    verdict: str        # PASS or FAIL
    checks: str         # "12/12", or "-" when the evidence is a note
    model: str
    effort: str
    date: str           # YYYY-MM-DD
    report: str         # a report filename, or notes:<part>.<item>


def _report_when(path: pathlib.Path, lines: Sequence[str]) -> datetime:
    """Read a report's time from its header, else its filename, else its mtime."""
    for line in lines:
        match = REPORT_WHEN_RE.match(line)
        if match:
            stamp = datetime.strptime(match.group(1), "%Y-%m-%d %H:%M:%S")
            return stamp.replace(tzinfo=timezone.utc)
    match = REPORT_STAMP_RE.match(path.name)
    if match:
        stamp = datetime.strptime(match.group(1) + match.group(2), "%Y-%m-%d%H%M")
        return stamp.replace(tzinfo=timezone.utc)
    legacy = LEGACY_STAMP_RE.match(path.name)
    if legacy:
        stamp = datetime.strptime(legacy.group(1) + legacy.group(2), "%Y%m%d%H%M%S")
        return stamp.replace(tzinfo=timezone.utc)
    return datetime.fromtimestamp(path.stat().st_mtime, timezone.utc)


def parse_report(path: pathlib.Path) -> ReportRun | None:
    """Read one saved replay report; None when it holds no scenario results."""
    lines = path.read_text(encoding="utf-8").splitlines()
    mode, model, effort, scenario = "fake", "unknown", "", ""
    results: dict[str, CapabilityVerdict] = {}
    for line in lines:
        header = REPORT_MODE_RE.match(line)
        if header:
            mode = "real" if "real" in header.group(1).lower() else "fake"
            continue
        model_line = REPORT_MODEL_RE.match(line)
        if model_line and model == "unknown":
            model = model_line.group(1)
            continue
        effort_line = REPORT_EFFORT_RE.match(line)
        if effort_line:
            effort = "" if effort_line.group(1) == "default" else effort_line.group(1)
            continue
        heading = REPORT_HEADING_RE.match(line)
        if heading:
            scenario = "" if heading.group(1).strip() in REPORT_SKIP else heading.group(1)
            continue
        check = REPORT_CHECK_RE.match(line)
        if check and scenario:
            verdict = results.setdefault(scenario, CapabilityVerdict(0, 0))
            verdict.total += 1
            if check.group(1) == "PASS":
                verdict.passed += 1
    if not results:
        return None
    return ReportRun(path=path, when=_report_when(path, lines), mode=mode,
                     model=model, results=results, effort=effort)


def saved_reports(reports_dir: pathlib.Path, before: datetime) -> list[ReportRun]:
    """Read the saved reports older than this run, newest first."""
    runs = []
    for path in sorted(reports_dir.glob("*.md")):
        if not REPORT_FILE_RE.match(path.name):
            continue
        run = parse_report(path)
        if run is not None and run.when < before:
            runs.append(run)
    return sorted(runs, key=lambda run: run.when, reverse=True)


def current_run(results: Sequence[ScenarioResult], mode: str,
                model: str, when: datetime, effort: str = "") -> ReportRun:
    """Turn this run's finished scenarios into a report run of its own."""
    verdicts = {}
    for result in results:
        passed = sum(1 for check in result.checks if check.passed)
        verdicts[result.scenario.name] = CapabilityVerdict(passed, len(result.checks))
    return ReportRun(path=pathlib.Path("(this run)"), when=when,
                     mode="real" if "real" in mode.lower() else "fake",
                     model=model, results=verdicts, effort=effort)


def _record_pool(runs: Sequence[ReportRun], mode: str, name: str) -> list[ReportRun]:
    """Return the newest runs of one mode that actually replayed this scenario."""
    found = [run for run in runs if run.mode == mode and name in run.results]
    return found[:RECORD_RUNS]


def _record_cell(pool: Sequence[ReportRun], name: str) -> str:
    """Render one cell: verdict, passes over runs, model, and the latest date."""
    if not pool:
        return "not run"
    passes = sum(1 for run in pool
                 if run.results[name].passed == run.results[name].total)
    verdict = "PASS" if passes == len(pool) else "FAIL"
    if pool[0].mode == "fake":
        return f"{verdict} {passes}/{len(pool)} · {pool[0].when:%Y-%m-%d}"
    models = ", ".join(dict.fromkeys(
        f"{run.model} (effort {run.effort or 'default'})" if run.effort else run.model
        for run in pool))
    return f"{verdict} {passes}/{len(pool)} · {_cell(models)} · {pool[0].when:%Y-%m-%d}"


def stored_records(previous: str) -> list[RealRecord]:
    """Read the ledger the capability record keeps, so a pruned report leaves no hole."""
    found: list[RealRecord] = []
    inside = False
    for line in previous.splitlines():
        if line.strip() == REAL_RECORD_HEADING:
            inside = True
            continue
        if not inside:
            continue
        if line.startswith("## "):
            break
        match = REAL_ROW.match(line.strip())
        if match:
            found.append(RealRecord(*(cell.strip() for cell in match.groups())))
    return found


def merge_records(stored: Sequence[RealRecord], runs: Sequence[ReportRun],
                  current: ReportRun | None) -> dict[str, list[RealRecord]]:
    """Return each scenario's recorded real runs, newest first, at most REAL_RECORD_ROWS."""
    records = list(stored)
    reports = [run for run in runs if run.mode == "real"]
    if current is not None and current.mode == "real":
        reports.append(current)
    for run in reports:
        for name, verdict in run.results.items():
            records.append(RealRecord(
                scenario=name, verdict="PASS" if verdict.passed == verdict.total else "FAIL",
                checks=f"{verdict.passed}/{verdict.total}", model=run.model,
                effort=run.effort, date=f"{run.when:%Y-%m-%d}", report=run.path.name))
    ordered = sorted(records, reverse=True,
                     key=lambda record: (record.date,
                                         1 if record.report.startswith("notes:") else 0,
                                         record.report))
    kept: dict[str, list[RealRecord]] = {}
    seen: set[tuple[str, str]] = set()
    for record in ordered:
        if (record.scenario, record.report) in seen:
            continue
        seen.add((record.scenario, record.report))
        kept.setdefault(record.scenario, []).append(record)
    return {name: items[:REAL_RECORD_ROWS] for name, items in kept.items()}


def _real_cell(records: Sequence[RealRecord]) -> str:
    """Render one real-model cell: verdict, checks, model, date and the report behind it."""
    if not records:
        return "not run"
    newest = records[0]
    parts = [f"{newest.verdict} {newest.checks}" if newest.checks not in ("", "-")
             else newest.verdict]
    effort = "" if newest.effort == "-" else newest.effort
    parts.append(f"{newest.model} (effort {effort})" if effort else newest.model)
    parts += [newest.date, newest.report]
    cell = " · ".join(part for part in parts if part and part != "-")
    verdicts = {record.verdict for record in records}
    return f"flaky · {cell}" if len(verdicts) > 1 else cell


def build_capability_record(claims: Sequence[Capability], runs: Sequence[ReportRun],
                            when: datetime, previous: str = "") -> str:
    """Render scenarios/README.md: every capability with its fake and real results."""
    ordered = sorted(runs, key=lambda run: run.when, reverse=True)
    fake_runs = [run for run in ordered if run.mode == "fake"]
    real_runs = [run for run in ordered if run.mode == "real"]
    by_function: dict[str, list[Capability]] = {}
    for claim in claims:
        by_function.setdefault(claim.function, []).append(claim)
    counted = sum(len(items) for items in by_function.values())
    lines = [
        GENERATED_MARK, "",
        "# The capability record", "",
        "Each scenario proves one capability in one brain function. This file is written by",
        "`uv run python scripts/replay.py` after every replay and is never edited by hand.",
        "`PASS 2/3` means every check of that capability passed in 2 of its 3 most recent",
        "runs, followed by the model used and the date of the latest run. Fake results come",
        "from the scripted fake model; real results come from the saved reports in",
        "`reports/`. A capability that fails with the real model is listed as FAIL, not",
        "hidden.", "",
        f"Generated {when:%Y-%m-%d %H:%M} UTC from {counted} capabilities across "
        f"{len(SCENARIO_FUNCTIONS)} brain functions.",
        f"Pooled per capability: its latest runs among {len(fake_runs)} saved fake and "
        f"{len(real_runs)} saved real-model reports. The real column names the report "
        "behind its",
        "verdict and reads `flaky` when the recorded runs both passed and failed; the",
        "`Real-model record` section at the end keeps those runs, so pruning the reports "
        "never",
        "erases evidence.", "",
    ]
    ledger = merge_records(stored_records(previous), runs, None)
    for function in SCENARIO_FUNCTIONS:
        entries = sorted(by_function.get(function) or [], key=lambda claim: claim.name)
        lines += [f"## {function}", ""]
        if not entries:
            lines += ["_No scenario yet._", ""]
            continue
        lines += ["| Capability | Scenario | Fake result | Real model |",
                  "|---|---|---|---|"]
        for claim in entries:
            fake = _record_cell(_record_pool(fake_runs, "fake", claim.name), claim.name)
            real = _real_cell(ledger.get(claim.name) or ())
            lines.append(f"| {_cell(claim.capability)} | {claim.name} | {fake} | {real} |")
        lines.append("")
    lines += [REAL_RECORD_HEADING, "",
              "Kept here, so pruning `reports/` to ten never erases evidence.", "",
              "| Scenario | Verdict | Checks | Model | Effort | Date | Report |",
              "|---|---|---|---|---|---|---|"]
    for claim in claims:
        for record in ledger.get(claim.name) or ():
            lines.append(f"| {_cell(record.scenario)} | {record.verdict} | "
                         f"{_cell(record.checks)} | {_cell(record.model)} | "
                         f"{_cell(record.effort or '-')} | {_cell(record.date)} | "
                         f"{_cell(record.report)} |")
    lines.append("")
    return "\n".join(lines)


def capability_record(results: Sequence[ScenarioResult], mode: str, model: str,
                      when: datetime, effort: str = "",
                      reports_dir: pathlib.Path = REPORT_DIR, previous: str = "",
                      report_path: pathlib.Path | None = None) -> str:
    """Build the capability record from this run and the saved reports."""
    claims = [Capability(result.scenario.name, result.scenario.function,
                         result.scenario.capability) for result in results]
    known = {claim.name for claim in claims}
    for path in scenario_paths([]):
        scenario = load_scenario(path, ZoneInfo("UTC"))
        if scenario.name not in known:
            claims.append(Capability(scenario.name, scenario.function, scenario.capability))
    current = current_run(results, mode, model, when, effort)
    if report_path is not None:
        current.path = report_path
    runs = [current, *saved_reports(reports_dir, when)]
    return build_capability_record(claims, runs, when, previous)


def write_capability_record(text: str, path: pathlib.Path = CAPABILITY_PATH) -> pathlib.Path:
    """Write the capability record and return its path."""
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8")
    return path


# ---------------------------------------------------------------------- main


def scenario_paths(args: Sequence[str]) -> list[pathlib.Path]:
    """Resolve explicit scenario paths or every scenarios/<function>/*.yaml in order."""
    if args:
        paths = [pathlib.Path(item) for item in args]
        missing = [str(path) for path in paths if not path.is_file()]
        if missing:
            raise RuntimeError(f"scenario not found: {', '.join(missing)}")
        return paths
    paths = sorted(SCENARIO_DIR.rglob("*.yaml"))
    if not paths:
        raise RuntimeError(f"no scenarios found in {SCENARIO_DIR}")
    return paths


async def run_all(paths: Sequence[pathlib.Path], fake: bool, url: str,
                  settings: dict[str, Any],
                  llm: dict[str, str]) -> tuple[list[ScenarioResult], Usage]:
    """Reset the database and replay every scenario in order."""
    zone = ZoneInfo(str(settings.get("timezone") or "UTC"))
    tracker = Tracker()
    runner = ReplayRunner(url, settings, fake, tracker, llm)
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
    parser.add_argument("--model", help="override LLM_MODEL for this run")
    parser.add_argument("--effort", choices=EFFORTS,
                        help="override LLM_REASONING_EFFORT for this run")
    args = parser.parse_args(argv)
    load_environment()
    url = replay_url()
    settings = load_settings()
    llm = {"base_url": DEFAULT_BASE_URL, "api_key": "", "model": DEFAULT_MODEL, "effort": ""}
    if not args.fake:
        llm = llm_settings()
    if args.model:
        llm["model"] = args.model
    if args.effort:
        llm["effort"] = args.effort
    paths = scenario_paths(args.scenarios)
    ensure_database(url)
    reset_schema(url)
    results, usage = asyncio.run(run_all(paths, args.fake, url, settings, llm))
    when = datetime.now(timezone.utc)
    zone = ZoneInfo(str(settings.get("timezone") or "UTC"))
    mode = "fake LLM (free)" if args.fake else "real LLM"
    text = render_report(results, usage, mode, llm["model"], settings, when, zone,
                         llm["effort"])
    path = write_report(text, when, fake=args.fake, label=report_label(paths))
    previous = (CAPABILITY_PATH.read_text(encoding="utf-8")
                if CAPABILITY_PATH.is_file() else "")
    record = capability_record(results, mode, llm["model"], when, llm["effort"],
                               previous=previous, report_path=path)
    record_path = write_capability_record(record)
    failed = [check.label for result in results for check in result.checks if not check.passed]
    print(f"\nreport: {path}")
    print(f"capability record: {record_path}")
    print("\nmetrics (missed-help first):")
    for line in metric_lines(compute_metrics(results, usage)):
        print(line)
    print(f"\nchecks: {sum(len(r.checks) for r in results) - len(failed)}/"
          f"{sum(len(r.checks) for r in results)} passed")
    if failed:
        print(f"failed: {len(failed)}")
    print(f"llm calls: {usage.calls} · tokens: prompt {usage.prompt}, cached {usage.cached}, "
          f"completion {usage.completion} (reasoning {usage.reasoning}), total {usage.total}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
