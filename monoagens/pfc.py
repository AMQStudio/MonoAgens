"""The PFC: the LLM side of the self, reached only through the gate."""

from __future__ import annotations

import json
from dataclasses import dataclass
from datetime import datetime
from typing import Any, Sequence
from zoneinfo import ZoneInfo

import httpx

from monoagens.config import load_soul
from monoagens.types import RECENT_KINDS, Event

SYSTEM_TAIL = "You act only through the tools. Staying silent is a valid choice."


def _tool(name: str, description: str, properties: dict[str, Any],
          required: Sequence[str] = ()) -> dict[str, Any]:
    return {"type": "function", "function": {
        "name": name, "description": description,
        "parameters": {"type": "object", "properties": properties, "required": list(required),
                       "additionalProperties": False}}}


def _text(description: str) -> dict[str, Any]:
    return {"type": "string", "description": description}


TOOLS: list[dict[str, Any]] = [
    _tool("say", "Say something in the current place.",
          {"text": _text("What to say.")}, ["text"]),
    _tool("react", "React with an emoji to the message that woke you.",
          {"emoji": _text("One emoji.")}, ["emoji"]),
    _tool("stay_silent", "Deliberately do not respond.",
          {"reason": _text("Why silence.")}, ["reason"]),
    _tool("expect", "Hold an expectation about the future; you are woken if it is missed.",
          {"about": _text("One sentence: what is expected."),
           "cue_time": {"type": "object",
                        "description": '{"at": ISO}, {"every": "2d"} or {"daily": "07:00"}.'},
           "cue_event": {"type": "object", "description": "Pattern: kind, who, place, text_like."},
           "expect": {"type": "object",
                      "description": '{"none": true}, {"event_seen": pattern, "within": "2d"} '
                      'or {"state": key, "op": "eq|gt|lt", "value": x}.'},
           "intent": _text("What to do if it is missed."),
           "weight": {"type": "number", "description": "Importance 0..1."},
           "policy": {"type": "object",
                      "description": '{"once": true} or {"cooldown": "6h", "max_fires": 3}.'},
           "expires_at": _text("ISO time after which it is void.")}, ["about", "expect"]),
    _tool("resolve", "Close an expectation.",
          {"id": {"type": "integer"},
           "status": {"type": "string", "enum": ["satisfied", "dropped"]}}, ["id", "status"]),
]
TOOL_NAMES = frozenset(tool["function"]["name"] for tool in TOOLS)


@dataclass(frozen=True, slots=True)
class ToolCall:
    """One tool call returned by the PFC."""

    name: str
    arguments: dict[str, Any]


@dataclass(frozen=True, slots=True)
class PFCContext:
    """Everything the PFC sees for one wake."""

    now: datetime
    place: str
    place_kind: str
    reason: str
    core_memory: str = ""
    expectations: Sequence[str] = ()
    memories: Sequence[str] = ()
    recent: Sequence[Event] = ()
    recent_elsewhere: Sequence[Event] = ()
    trigger: Event | None = None
    last_interaction: datetime | None = None


class PFC:
    """The LLM client: layered prompts and tool calls over chat completions."""

    def __init__(self, base_url: str, api_key: str, model: str, *,
                 soul: dict[str, Any] | None = None,
                 transport: httpx.AsyncBaseTransport | None = None,
                 client: httpx.AsyncClient | None = None, timeout: float = 30.0) -> None:
        """Store the endpoint, the key, the model, and the persona."""
        self.base_url, self.api_key, self.model = base_url.rstrip("/"), api_key, model
        self.soul = soul if soul is not None else load_soul()
        self.timezone = ZoneInfo(str(self.soul.get("timezone") or "UTC"))
        self.name = str(self.soul.get("name") or "Mona")
        self._transport, self._client, self._timeout = transport, client, timeout

    def _http(self) -> httpx.AsyncClient:
        if self._client is None:
            self._client = httpx.AsyncClient(transport=self._transport, timeout=self._timeout)
        return self._client

    async def aclose(self) -> None:
        """Close the underlying HTTP client."""
        if self._client is not None:
            await self._client.aclose()
            self._client = None

    async def call(self, messages: list[dict[str, Any]], tools: list[dict[str, Any]], *,
                   tool_choice: str = "required") -> list[ToolCall]:
        """Make one chat completion call and return its tool calls."""
        headers = {"Authorization": f"Bearer {self.api_key}"} if self.api_key else {}
        response = await self._http().post(
            f"{self.base_url}/chat/completions",
            json={"model": self.model, "messages": messages, "tools": tools,
                  "tool_choice": tool_choice},
            headers=headers)
        response.raise_for_status()
        try:
            raw_calls = response.json()["choices"][0]["message"].get("tool_calls") or []
        except (KeyError, IndexError, TypeError) as exc:
            raise ValueError(f"Malformed chat completion response: {exc}") from exc
        return [
            ToolCall(call["function"]["name"], json.loads(call["function"]["arguments"] or "{}"))
            for call in raw_calls
        ]

    async def decide(self, context: PFCContext) -> list[ToolCall]:
        """Ask the model to act on one context."""
        return await self.call(self.build_messages(context), TOOLS)

    def build_messages(self, context: PFCContext) -> list[dict[str, Any]]:
        """Build the layered prompt: stable, core memory, volatile."""
        return [
            {"role": "system", "content": self._stable_prompt()},
            {"role": "system", "content": f"<core_memory>{context.core_memory}</core_memory>"},
            {"role": "user", "content": self._volatile_prompt(context)},
        ]

    def _stable_prompt(self) -> str:
        parts = [f"You are {self.name}.", str(self.soul.get("persona") or "").strip()]
        principles = [str(p) for p in self.soul.get("principles") or []]
        if principles:
            parts.append("Principles:\n" + "\n".join(f"- {p}" for p in principles))
        parts += [f"Language: {self.soul.get('language') or 'en'}.", SYSTEM_TAIL]
        return "\n\n".join(part for part in parts if part)

    def _volatile_prompt(self, context: PFCContext) -> str:
        local_now = context.now.astimezone(self.timezone)
        sections = [
            f"<now>{local_now:%Y-%m-%d %H:%M}, {local_now:%A}, {self.timezone.key}, "
            "time since last interaction: "
            f"{_duration(context.now, context.last_interaction)}</now>",
            f"<place>{context.place} ({context.place_kind})</place>",
            f"<why_awake>{context.reason}</why_awake>",
            f"<expectations>{_listed(context.expectations)}</expectations>",
            f"<memories>{_listed(context.memories)}</memories>",
            f"<recent>\n{chr(10).join(self._recent_lines(context))}\n</recent>",
            f"<event>{self._event_text(context.trigger)}</event>",
        ]
        return "\n".join(sections)

    def _recent_lines(self, context: PFCContext) -> list[str]:
        lines: list[str] = []
        for event in context.recent:
            if event.kind not in RECENT_KINDS:
                continue
            speaker, text = self._recent_parts(event)
            if text:
                stamp = event.ts.astimezone(self.timezone)
                lines.append(f"{stamp:%H:%M} {speaker}: {text}")
        for event in context.recent_elsewhere:
            if event.kind not in RECENT_KINDS:
                continue
            speaker, text = self._recent_parts(event)
            if text:
                stamp = event.ts.astimezone(self.timezone)
                lines.append(f"[{event.place}] {stamp:%H:%M} {speaker}: {text}")
        return lines or ["(nothing yet)"]

    def _recent_parts(self, event: Event) -> tuple[str, Any]:
        if event.kind == "msg.out":
            return self.name, event.payload.get("text")
        if event.kind == "expectation.fired":
            return "expectation", (event.payload.get("text") or event.payload.get("about"))
        return str(event.who or "?").removeprefix("person:"), event.payload.get("text")

    def _event_text(self, event: Event | None) -> str:
        if event is None:
            return "(no trigger event)"
        text = event.payload.get("text")
        who = str(event.who or "?").removeprefix("person:")
        return f"{who}: {text}" if text else f"[{event.kind}]"


def _listed(items: Sequence[str]) -> str:
    return "\n".join(str(item) for item in items) if items else "none"


def _duration(now: datetime, then: datetime | None) -> str:
    if then is None:
        return "first interaction"
    minutes = int((now - then).total_seconds() // 60)
    if minutes < 1:
        return "less than a minute"
    days, rest = divmod(minutes, 1440)
    hours, mins = divmod(rest, 60)
    if days:
        return f"{days}d {hours}h"
    return f"{hours}h {mins}m" if hours else f"{mins}m"


__all__ = ["PFC", "PFCContext", "TOOLS", "TOOL_NAMES", "ToolCall"]
