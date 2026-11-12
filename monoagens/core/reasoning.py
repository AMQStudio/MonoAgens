"""The reasoning: the LLM side of the self, reached only through appraisal."""

from __future__ import annotations

import json
from collections.abc import Awaitable, Callable
from dataclasses import dataclass, replace
from typing import Any, Sequence
from zoneinfo import ZoneInfo

import httpx

from monoagens.core import arguments, expectations
from monoagens.core.context import parts
from monoagens.core.context.layout import PFCContext
from monoagens.infra.prior import load_settings


def _tool(name: str, description: str, properties: dict[str, Any],
          required: Sequence[str] = ()) -> dict[str, Any]:
    return {"type": "function", "function": {
        "name": name, "description": description,
        "parameters": {"type": "object", "properties": properties, "required": list(required),
                       "additionalProperties": False}}}


def _text(description: str) -> dict[str, Any]:
    return {"type": "string", "description": description}


TOOLS: list[dict[str, Any]] = [
    _tool("think", "State a thought and how sure it is; the same wake still acts on it.",
          {"thought": _text("One sentence of inner thought."),
           "motivation": {"type": "integer", "minimum": 1, "maximum": 5,
                          "description": "How strongly this deserves to be said."},
           "confidence": {"type": "number", "minimum": 0, "maximum": 1,
                          "description": "How sure the thought is."}},
          ["thought", "motivation", "confidence"]),
    _tool("act", "Use one tool capability.",
          {"capability": _text("The tool capability name."),
           "arguments": {"type": "object", "description": "Arguments for the capability."}},
          ["capability", "arguments"]),
    _tool("expect", "Hold what must remind you later: a promise, a watch, or a wait; "
                    "its cue wakes you.",
          {"about": _text("One sentence: what is expected."),
           "kind": {"type": "string", "enum": list(expectations.KINDS),
                    "description": "promise, patience, prediction or guard (default promise)."},
           "cue_time": {"type": "object",
                        "description": '{"at": ISO}, {"every": "2d"} or {"daily": "07:00"}.'},
           "cue_event": {"type": "object",
                         "description": 'Keys, all of which must match: kind (the event kind '
                         'exactly as the desk shows it, e.g. percept for what someone says), '
                         'who (a speaker id), place (place id), not_who (a speaker id to '
                         'ignore), text_like (a plain phrase the line contains, '
                         'case-insensitive: NOT a regex and no | alternation; for several '
                         'phrasings make several expectations or pick the most distinctive '
                         'word). '
                         'who and place are ids copied exactly as the desk and the recent '
                         'lines show them, prefix included: a cue whose place does not match '
                         'the line never fires.'},
           "expect": {"type": "object",
                      "description": '{"none": true}: the cue alone wakes you. '
                      '{"text": "what you are waiting to hear"}: a watch on free text, so a '
                      'line that sounds like it reminds you wakes you once to judge it '
                      '(cue_event narrows where and who). '
                      '{"event_seen": pattern, "within": "2d"}: fires when no matching event '
                      'was seen within the window. '
                      '{"state": key, "op": "eq|gt|lt", "value": x}.'},
           "intent": _text("What to do if it is missed."),
           "weight": {"type": "number", "description": "Importance 0..1."},
           "policy": {"type": "object",
                      "description": '{"once": true} or {"cooldown": "6h", "max_fires": 3}.'},
           "expires_at": _text("ISO time after which it is void.")}, ["about", "expect"]),
    _tool("resolve", "Close an expectation.",
          {"id": {"type": "integer", "description": "The number the desk shows as #N."},
           "status": {"type": "string", "enum": ["satisfied", "dropped"]}}, ["id", "status"]),
    _tool("revise", "Correct an earlier memory; nothing is deleted.",
          {"old": _text("The memory being corrected."),
           "new": _text("The corrected memory."),
           "reason": _text("Why the correction is made.")}, ["old", "new", "reason"]),
]
TOOL_NAMES = frozenset(tool["function"]["name"] for tool in TOOLS)
THOUGHT_ACTS = frozenset({"think", "expect", "resolve", "revise"})
_PARAMETERS = {tool["function"]["name"]: tool["function"]["parameters"] for tool in TOOLS}


def _args(cap: dict[str, Any]) -> str:
    names = list((cap.get("schema") or {}).get("properties") or {})
    names += ["to? (a speaker or a place)", "via? (a tool)"] if cap.get("communicative") else []
    return "(" + ", ".join(names) + ")"


def tools_for(capabilities: Sequence[dict[str, Any]]) -> list[dict[str, Any]]:
    """Return the fixed thought acts with `act` listing the capabilities present now."""
    names = [f"{cap['organ']}.{cap['name']}" for cap in capabilities]
    lines = [f"{name}{_args(cap)}: {cap.get('description') or ''}"
             for name, cap in zip(names, capabilities)]
    capability = {"type": "string", "description": "The tool capability name."}
    if names:
        capability["enum"] = names
    act = _tool("act", "Use one tool capability.\n" + "\n".join(lines),
                {"capability": capability,
                 "arguments": {"type": "object", "description": "Arguments for the capability."}},
                ["capability", "arguments"])
    return [act if tool["function"]["name"] == "act" else tool for tool in TOOLS]


@dataclass(frozen=True, slots=True)
class ToolCall:
    """One tool call returned by the reasoning."""

    name: str
    arguments: dict[str, Any]


class Reasoning:
    """The LLM client: layered prompts and tool calls over chat completions."""

    def __init__(self, base_url: str, api_key: str, model: str, *, effort: str = "",
                 settings: dict[str, Any] | None = None,
                 identity: dict[str, Any] | None = None,
                 transport: httpx.AsyncBaseTransport | None = None,
                 client: httpx.AsyncClient | None = None, timeout: float = 30.0,
                 parallel_tool_calls: bool = True) -> None:
        """Store the endpoint, the key, the model, the optional effort, and the identity."""
        self.base_url, self.api_key, self.model = base_url.rstrip("/"), api_key, model
        self.effort = effort.strip().lower()
        self.settings = settings if settings is not None else load_settings()
        self.identity = dict(identity if identity is not None
                             else self.settings.get("identity") or {})
        self.timezone = ZoneInfo(str(self.settings.get("timezone") or "UTC"))
        self.name = str(self.identity.get("name") or "")
        self.last_usage: dict[str, int] = {}
        self.parallel_tool_calls = parallel_tool_calls
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

    async def _post(self, body: dict[str, Any], headers: dict[str, str]) -> httpx.Response:
        """POST one completion, retrying a transport failure as often as the prior allows."""
        retries = int(self.settings["constants"]["model_call_retries"])
        for attempt in range(retries + 1):
            try:
                return await self._http().post(f"{self.base_url}/chat/completions",
                                               json=body, headers=headers)
            except httpx.TransportError:
                if attempt == retries:
                    raise
        raise RuntimeError("unreachable: the retry loop always returns or raises")

    async def call(self, messages: list[dict[str, Any]], tools: list[dict[str, Any]], *,
                   tool_choice: str = "required") -> list[ToolCall]:
        """Make one chat completion call and return its tool calls."""
        headers = {"Authorization": f"Bearer {self.api_key}"} if self.api_key else {}
        body = {"model": self.model, "messages": messages, "tools": tools,
                "tool_choice": tool_choice}
        if self.parallel_tool_calls:
            body["parallel_tool_calls"] = True
        if self.effort:
            body["reasoning_effort"] = self.effort
        response = await self._post(body, headers)
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

    def build_messages(self, context: PFCContext) -> list[dict[str, Any]]:
        """Build the layered prompt: stable, core memory, volatile."""
        return parts.messages(self, context)

    async def decide(self, context: PFCContext) -> list[ToolCall]:
        """Ask the model to act on one context; after a thought round no tool is forced."""
        return await self.call(self.build_messages(context), tools_for(context.capabilities),
                               tool_choice="auto" if context.results else "required")


async def turns(pfc: Any, context: PFCContext, limit: int,
                check: Callable[[ToolCall], Awaitable[str | None]] | None = None,
                ) -> list[dict[str, Any]]:
    """Run one wake's rounds: a thought act's result returns to the model (D51)."""
    taken, made = [], []
    while len(made) < max(1, int(limit)):
        calls = await pfc.decide(replace(context, results=tuple(taken)))
        raw, notes = [], []
        for call in calls:
            item = {"name": call.name, "arguments": call.arguments}
            refused = await check(call) if check is not None else None
            if refused:
                item["refused"] = refused
                notes.append(f"{call.name} refused: {refused}")
            else:
                notes.append(_recorded([item])[0])
            raw.append(item)
        made.append({"calls": raw, "usage": dict(getattr(pfc, "last_usage", None) or {})})
        refused = any(item.get("refused") for item in raw)
        acted = any(item["name"] not in THOUGHT_ACTS and not item.get("refused")
                    for item in raw)
        # A refusal must return as a result even when the same round carried out an act.
        if not raw or (acted and not refused):
            break
        taken += notes
    return made


def turn_check(context: PFCContext, now: Any, settings: dict[str, Any]) -> Callable[..., Any]:
    """Return the check that refuses a bad thought act with its reason (D51)."""

    async def check(call: ToolCall) -> str | None:
        if call.name == "act":
            return _check_act(context, call.arguments)
        parameters = _PARAMETERS.get(call.name)
        if parameters is None:
            return f"unknown tool: {call.name}"
        found = arguments.errors(parameters, call.arguments)
        if found:
            return arguments.refusal(found)
        if call.name == "expect":
            try:
                expectations.validate(call.arguments, now=now, settings=settings)
            except (TypeError, ValueError, KeyError) as exc:
                return str(exc)
        if call.name == "resolve":
            return _check_resolve(context, call.arguments)
        return None

    return check


def _check_act(context: PFCContext, given: dict[str, Any]) -> str | None:
    """Refuse an act whose own or whose capability's arguments break their schema."""
    found = arguments.errors(_PARAMETERS["act"], given)
    if found:
        return arguments.refusal(found)
    name = str(given.get("capability") or "")
    cap = next((item for item in context.capabilities
                if f"{item['organ']}.{item['name']}" == name), None)
    if cap is None:
        return f"unknown capability: {name}"
    found = arguments.capability_errors(cap, given["arguments"])
    return arguments.refusal(found) if found else None


def _check_resolve(context: PFCContext, given: dict[str, Any]) -> str | None:
    """Refuse a resolve that names no expectation on the desk, or a status outside two."""
    status = str(given.get("status") or "")
    if status not in ("satisfied", "dropped"):
        return "status must be satisfied or dropped"
    exp_id = given["id"]
    shown = {int(line[1:].split(" ", 1)[0]) for line in context.expectations
             if line.startswith("#") and line[1:2].isdigit()}
    return None if exp_id in shown else f"no expectation #{exp_id} on the desk"


def _recorded(calls: list[dict[str, Any]]) -> list[str]:
    """Render what each thought act recorded, as the next round's results."""
    return [f"{call['name']}: {json.dumps(call['arguments'], ensure_ascii=False)}"
            for call in calls]


def _usage(raw: dict[str, Any]) -> dict[str, int]:
    """Normalize one response's token usage into the five tracked counters."""
    details, prompt = raw.get("prompt_tokens_details") or {}, int(raw.get("prompt_tokens") or 0)
    reasoning = raw.get("completion_tokens_details") or {}
    completion = int(raw.get("completion_tokens") or 0)
    return {"prompt_tokens": prompt, "cached_tokens": int(details.get("cached_tokens") or 0),
            "completion_tokens": completion,
            "reasoning_tokens": int(reasoning.get("reasoning_tokens") or 0),
            "total_tokens": int(raw.get("total_tokens") or prompt + completion)}
