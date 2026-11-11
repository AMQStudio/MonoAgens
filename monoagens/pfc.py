"""The PFC: the LLM side of the self, reached only through appraisal."""

from __future__ import annotations

import json
from dataclasses import dataclass
from datetime import datetime
from typing import Any, Sequence
from zoneinfo import ZoneInfo

import httpx

from monoagens.guidance import work_guidance
from monoagens.prior import constants, load_settings
from monoagens.types import RECENT_KINDS, Event


def _tool(name: str, description: str, properties: dict[str, Any],
          required: Sequence[str] = ()) -> dict[str, Any]:
    return {"type": "function", "function": {
        "name": name, "description": description,
        "parameters": {"type": "object", "properties": properties, "required": list(required),
                       "additionalProperties": False}}}


def _text(description: str) -> dict[str, Any]:
    return {"type": "string", "description": description}


TOOLS: list[dict[str, Any]] = [
    _tool("think", "Think before acting; state the thought and how sure you are.",
          {"thought": _text("One sentence of inner thought."),
           "motivation": {"type": "integer", "minimum": 1, "maximum": 5,
                          "description": "How strongly this deserves to be said."},
           "confidence": {"type": "number", "minimum": 0, "maximum": 1,
                          "description": "How sure the thought is."}},
          ["thought", "motivation", "confidence"]),
    _tool("act", "Use one organ capability.",
          {"capability": _text("The organ capability name."),
           "arguments": {"type": "object", "description": "Arguments for the capability."}},
          ["capability", "arguments"]),
    _tool("expect", "Hold an expectation about the future; you are woken if it is missed.",
          {"about": _text("One sentence: what is expected."),
           "kind": {"type": "string", "enum": ["promise", "wait", "predict", "guard"],
                    "description": "promise, patience, prediction or guard (default promise)."},
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
    _tool("revise", "Correct an earlier memory; nothing is deleted.",
          {"old": _text("The memory being corrected."),
           "new": _text("The corrected memory."),
           "reason": _text("Why the correction is made.")}, ["old", "new", "reason"]),
]
TOOL_NAMES = frozenset(tool["function"]["name"] for tool in TOOLS)


def _args(cap: dict[str, Any]) -> str:
    return "(" + ", ".join((cap.get("schema") or {}).get("properties") or {}) + ")"


def tools_for(capabilities: Sequence[dict[str, Any]]) -> list[dict[str, Any]]:
    """Return the fixed thought acts with `act` listing the capabilities present now."""
    names = [f"{cap['organ']}.{cap['name']}" for cap in capabilities]
    lines = [f"{name}{_args(cap)}: {cap.get('description') or ''}"
             for name, cap in zip(names, capabilities)]
    capability = {"type": "string", "description": "The organ capability name."}
    if names:
        capability["enum"] = names
    act = _tool("act", "Use one organ capability.\n" + "\n".join(lines),
                {"capability": capability,
                 "arguments": {"type": "object", "description": "Arguments for the capability."}},
                ["capability", "arguments"])
    return [act if tool["function"]["name"] == "act" else tool for tool in TOOLS]


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
    doing: Sequence[str] = ()
    trigger: Event | None = None
    last_interaction: datetime | None = None
    capabilities: Sequence[dict[str, Any]] = ()


class PFC:
    """The LLM client: layered prompts and tool calls over chat completions."""

    def __init__(self, base_url: str, api_key: str, model: str, *,
                 settings: dict[str, Any] | None = None,
                 identity: dict[str, Any] | None = None,
                 transport: httpx.AsyncBaseTransport | None = None,
                 client: httpx.AsyncClient | None = None, timeout: float = 30.0) -> None:
        """Store the endpoint, the key, the model, and the identity."""
        self.base_url, self.api_key, self.model = base_url.rstrip("/"), api_key, model
        self.settings = settings if settings is not None else load_settings()
        self.identity = dict(identity if identity is not None
                             else self.settings.get("identity") or {})
        self.timezone = ZoneInfo(str(self.settings.get("timezone") or "UTC"))
        self.name = str(self.identity.get("name") or "")
        self.last_usage: dict[str, int] = {}
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
        body = {"model": self.model, "messages": messages, "tools": tools,
                "tool_choice": tool_choice}
        response = await self._http().post(f"{self.base_url}/chat/completions",
                                           json=body, headers=headers)
        response.raise_for_status()
        try:
            payload = response.json()
            raw_calls = payload["choices"][0]["message"].get("tool_calls") or []
        except (KeyError, IndexError, TypeError) as exc:
            raise ValueError(f"Malformed chat completion response: {exc}") from exc
        self.last_usage = _usage(payload.get("usage") or {})
        return [
            ToolCall(call["function"]["name"], json.loads(call["function"]["arguments"] or "{}"))
            for call in raw_calls
        ]

    async def decide(self, context: PFCContext) -> list[ToolCall]:
        """Ask the model to act on one context."""
        return await self.call(self.build_messages(context), tools_for(context.capabilities))

    def build_messages(self, context: PFCContext) -> list[dict[str, Any]]:
        """Build the layered prompt: stable, core memory, volatile."""
        return [
            {"role": "system", "content": self._stable_prompt()},
            {"role": "system", "content": f"<core_memory>{context.core_memory}</core_memory>"},
            {"role": "user", "content": self._volatile_prompt(context)},
        ]

    def _stable_prompt(self) -> str:
        parts = [f"You are {self.name}.", str(self.identity.get("persona") or "").strip()]
        principles = [str(p) for p in self.identity.get("principles") or []]
        if principles:
            parts.append("Principles:\n" + "\n".join(f"- {p}" for p in principles))
        parts += [f"Language: {self.identity.get('language') or 'en'}.", work_guidance()]
        return "\n\n".join(part for part in parts if part)

    def _volatile_prompt(self, context: PFCContext) -> str:
        local_now = context.now.astimezone(self.timezone)
        values = constants(self.settings)
        doing = _fit(list(context.doing), int(values["thread_result_tokens"]), keep_first=True)
        memories = _fit([str(item) for item in context.memories], int(values["memory_tokens"]))
        recent = _fit(self._recent_lines(context), int(values["recent_tokens"]))

        def build() -> list[str]:
            return [
                f"<now>{local_now:%Y-%m-%d %H:%M}, {local_now:%A}, {self.timezone.key}, "
                "time since last interaction: "
                f"{_duration(context.now, context.last_interaction)}</now>",
                f"<place>{context.place} ({context.place_kind})</place>",
                f"<why_awake>{context.reason}</why_awake>",
                f"<doing>\n{_listed(doing)}\n</doing>",
                f"<expectations>{_listed(context.expectations)}</expectations>",
                f"<memories>\n{_listed(memories)}\n</memories>",
                f"<recent>\n{chr(10).join(recent)}\n</recent>",
                f"<event>{self._event_text(context.trigger)}</event>",
            ]

        limit, parts = int(values["volatile_tokens_max"]), build()
        for bucket in (recent, memories):
            while _tokens("\n".join(parts)) > limit and len(bucket) > 1:
                bucket.pop(0)
                parts = build()
        return "\n".join(parts)

    def _recent_lines(self, context: PFCContext) -> list[str]:
        lines = [self._recent_line(event) for event in context.recent]
        lines += [self._recent_line(event, f"[{event.place}] ")
                  for event in context.recent_elsewhere]
        return [line for line in lines if line] or ["(nothing yet)"]

    def _recent_line(self, event: Event, prefix: str = "") -> str:
        if event.kind not in RECENT_KINDS:
            return ""
        speaker, text = self._recent_parts(event)
        return (f"{prefix}{event.ts.astimezone(self.timezone):%H:%M} "
                f"{speaker}: {text}") if text else ""

    def _recent_parts(self, event: Event) -> tuple[str, Any]:
        if event.kind == "act.out":
            return self.name, (event.payload.get("arguments") or {}).get("text")
        if event.kind == "expectation.fired":
            return "expectation", (event.payload.get("text") or event.payload.get("about"))
        return str(event.who or "?").removeprefix("person:"), event.payload.get("description")

    def _event_text(self, event: Event | None) -> str:
        if event is None:
            return "(no trigger event)"
        text = event.payload.get("description") or event.payload.get("text")
        who = str(event.who or "?").removeprefix("person:")
        line = f"{who}: {text}" if text else f"[{event.kind}]"
        notes = event.payload.get("notes")
        if notes:
            each = [notes] if isinstance(notes, str) else notes
            line += "\n" + "\n".join(f"note: {note}" for note in each)
        return line


def _usage(raw: dict[str, Any]) -> dict[str, int]:
    """Normalize one response's token usage into the four tracked counters."""
    details, prompt = raw.get("prompt_tokens_details") or {}, int(raw.get("prompt_tokens") or 0)
    completion = int(raw.get("completion_tokens") or 0)
    return {"prompt_tokens": prompt, "cached_tokens": int(details.get("cached_tokens") or 0),
            "completion_tokens": completion,
            "total_tokens": int(raw.get("total_tokens") or prompt + completion)}


def _listed(items: Sequence[str]) -> str:
    return "\n".join(str(item) for item in items) if items else "none"


def _tokens(text: str) -> int:
    """Approximate one section's token count from its characters."""
    return max(1, len(text) // 4)


def _fit(lines: list[str], limit: int, *, keep_first: bool = False) -> list[str]:
    """Drop the oldest lines until the section fits its token budget."""
    lines = list(lines)
    while len(lines) > 1 and sum(map(_tokens, lines)) > limit:
        lines.pop(1 if keep_first and len(lines) > 2 else 0)
    return lines


def _duration(now: datetime, then: datetime | None) -> str:
    if then is None:
        return "first interaction"
    minutes = max(0, int((now - then).total_seconds() // 60))
    days, rest = divmod(minutes, 1440)
    hours, mins = divmod(rest, 60)
    if days:
        return f"{days}d {hours}h"
    return f"{hours}h {mins}m" if hours else f"{minutes}m"

