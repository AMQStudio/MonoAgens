"""The reasoning: the LLM side of the self, reached only through appraisal."""

from __future__ import annotations

import json
from collections.abc import Awaitable, Callable
from dataclasses import dataclass, replace
from typing import Any, Protocol, Sequence
from zoneinfo import ZoneInfo

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
                         "description": 'Facts only, all of which must match: kind (the event '
                         'kind exactly as the desk shows it, e.g. percept for what someone '
                         'says), who (a speaker id), place (place id), not_who (a speaker id '
                         'to ignore). No words: what a line says is what expect.text is for. '
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


class ModelPort(Protocol):
    """Model port called by Reasoning; provider drivers live outside the core."""

    async def decide(self, messages: list[dict[str, Any]], tools: list[dict[str, Any]], *,
                     tool_choice: str = "required") -> tuple[list[ToolCall], dict[str, Any]]:
        """Run one model decision and return (tool_calls, usage)."""
        ...

    async def aclose(self) -> None:
        """Close any underlying transport resources."""
        ...


class Reasoning:
    """The LLM side of the self: layered prompts and tool calls over an injected model port."""

    def __init__(self, port: ModelPort | None = None, *,
                 settings: dict[str, Any] | None = None,
                 identity: dict[str, Any] | None = None) -> None:
        """Store the model port, settings, and identity."""
        self._port = port
        self.settings = settings if settings is not None else load_settings()
        self.identity = dict(identity if identity is not None
                             else self.settings.get("identity") or {})
        self.timezone = ZoneInfo(str(self.settings.get("timezone") or "UTC"))
        self.name = str(self.identity.get("name") or "")
        self.last_usage: dict[str, Any] = {}

    @property
    def model(self) -> str:
        """Return the active model name reported by the injected port."""
        return str(getattr(self._port, "model", "") or "")

    async def aclose(self) -> None:
        """Close the injected model port if present."""
        if self._port is not None:
            await self._port.aclose()

    async def call(self, messages: list[dict[str, Any]], tools: list[dict[str, Any]], *,
                   tool_choice: str = "required") -> list[ToolCall]:
        """Make one model decision through the port and return its tool calls."""
        if self._port is None:
            raise RuntimeError("ModelPort is not configured")
        calls, usage = await self._port.decide(messages, tools, tool_choice=tool_choice)
        self.last_usage = dict(usage)
        return calls

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
