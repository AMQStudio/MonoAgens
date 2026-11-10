"""The PFC: the LLM side of the self, reached only through the gate.

One wake = one chat completion with tool calling. The prompt is layered
(stable identity -> core_memory -> volatile context) so it can be cached.
"""

from __future__ import annotations

import json
from dataclasses import dataclass
from datetime import datetime
from typing import Any, Sequence
from zoneinfo import ZoneInfo

import httpx

from monoagens.config import load_soul
from monoagens.types import Event

SYSTEM_TAIL = "You act only through the tools. Staying silent is a valid choice."

def _tool(name: str, description: str, argument: str, argument_description: str) -> dict[str, Any]:
    """One function tool whose single string argument is required."""
    return {
        "type": "function",
        "function": {
            "name": name,
            "description": description,
            "parameters": {
                "type": "object",
                "properties": {argument: {"type": "string", "description": argument_description}},
                "required": [argument],
                "additionalProperties": False,
            },
        },
    }


TOOLS: list[dict[str, Any]] = [
    _tool("say", "Say something in the current place.", "text", "What to say."),
    _tool("react", "React with an emoji to the message that woke you.", "emoji", "One emoji."),
    _tool("stay_silent", "Deliberately do not respond.", "reason", "Why silence."),
]

TOOL_NAMES = frozenset(tool["function"]["name"] for tool in TOOLS)


@dataclass(frozen=True, slots=True)
class ToolCall:
    """One parsed tool call: an action the PFC chose."""

    name: str
    arguments: dict[str, Any]


@dataclass(frozen=True, slots=True)
class PFCContext:
    """Everything volatile about one wake; the PFC turns it into a prompt."""

    now: datetime
    place: str
    place_kind: str
    reason: str
    core_memory: str = ""
    expectations: Sequence[str] = ()
    memories: Sequence[str] = ()
    recent: Sequence[Event] = ()
    trigger: Event | None = None
    last_interaction: datetime | None = None


class PFC:
    """OpenAI-compatible chat completions client with tool calling."""

    def __init__(
        self,
        base_url: str,
        api_key: str,
        model: str,
        *,
        soul: dict[str, Any] | None = None,
        transport: httpx.AsyncBaseTransport | None = None,
        client: httpx.AsyncClient | None = None,
        timeout: float = 30.0,
    ) -> None:
        self.base_url = base_url.rstrip("/")
        self.api_key = api_key
        self.model = model
        self.soul = soul if soul is not None else load_soul()
        self.timezone = ZoneInfo(str(self.soul.get("timezone") or "UTC"))
        self.name = str(self.soul.get("name") or "Mona")
        self._transport = transport
        self._client = client
        self._timeout = timeout

    def _http(self) -> httpx.AsyncClient:
        if self._client is None:
            self._client = httpx.AsyncClient(transport=self._transport, timeout=self._timeout)
        return self._client

    async def aclose(self) -> None:
        """Close the HTTP client, if one was opened."""
        if self._client is not None:
            await self._client.aclose()
            self._client = None

    async def decide(self, context: PFCContext) -> list[ToolCall]:
        """Ask the PFC what to do; returns the tool calls it chose."""
        headers = {"Authorization": f"Bearer {self.api_key}"} if self.api_key else {}
        response = await self._http().post(
            f"{self.base_url}/chat/completions",
            json={
                "model": self.model,
                "messages": self.build_messages(context),
                "tools": TOOLS,
                "tool_choice": "required",
            },
            headers=headers,
        )
        response.raise_for_status()
        try:
            message = response.json()["choices"][0]["message"]
            raw_calls = message.get("tool_calls") or []
        except (KeyError, IndexError, TypeError) as exc:
            raise ValueError(f"Malformed chat completion response: {exc}") from exc
        return [
            ToolCall(call["function"]["name"], json.loads(call["function"]["arguments"] or "{}"))
            for call in raw_calls
        ]

    def build_messages(self, context: PFCContext) -> list[dict[str, Any]]:
        """The three prompt layers: stable, core_memory, volatile."""
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
        parts.append(f"Language: {self.soul.get('language') or 'en'}.")
        parts.append(SYSTEM_TAIL)
        return "\n\n".join(part for part in parts if part)

    def _volatile_prompt(self, context: PFCContext) -> str:
        local_now = context.now.astimezone(self.timezone)
        recent = self._recent_lines(context)
        sections = [
            f"<now>{local_now:%Y-%m-%d %H:%M}, {local_now:%A}, {self.timezone.key}, "
            f"time since last interaction: {_duration(context.now, context.last_interaction)}</now>",
            f"<place>{context.place} ({context.place_kind})</place>",
            f"<why_awake>{context.reason}</why_awake>",
            f"<expectations>{_listed(context.expectations)}</expectations>",
            f"<memories>{_listed(context.memories)}</memories>",
            f"<recent>\n{chr(10).join(recent)}\n</recent>",
            f"<event>{self._event_text(context.trigger)}</event>",
        ]
        return "\n".join(sections)

    def _recent_lines(self, context: PFCContext) -> list[str]:
        lines: list[str] = []
        for event in context.recent:
            text = event.payload.get("text")
            if not text:
                continue
            speaker = self.name if event.kind == "msg.out" else str(event.who or "?").removeprefix("person:")
            stamp = event.ts.astimezone(self.timezone)
            lines.append(f"{stamp:%H:%M} {speaker}: {text}")
        return lines or ["(nothing yet)"]

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
    seconds = int((now - then).total_seconds())
    if seconds < 60:
        return "less than a minute"
    minutes, seconds = divmod(seconds, 60)
    hours, minutes = divmod(minutes, 60)
    days, hours = divmod(hours, 24)
    if days:
        return f"{days}d {hours}h"
    return f"{hours}h {minutes}m" if hours else f"{minutes}m"


__all__ = ["PFC", "PFCContext", "TOOLS", "TOOL_NAMES", "ToolCall"]
