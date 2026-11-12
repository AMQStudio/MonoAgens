"""The registry: the tools, commands and skills present now, and the routing of one act.

One list of names with short descriptions, the full entry on request, one namespace with
collisions failing at boot (middleware.md §4). Every entry names **who uses it**: a tool or
a skill is for the soul, a command is for a person, and every entry declares a permission.

A capability descriptor keeps its frozen key `organ`; the value is the name of the adapter
or plugin that declares the capability (docs/core.md §8). The core sees only the descriptor;
the facts, the permission and the output cap live here, next to it.
"""

from __future__ import annotations

from collections.abc import Awaitable, Callable, Mapping
from dataclasses import dataclass, replace
from typing import Any

from monoagens.core.context.parts import VOLATILE, Part
from monoagens.core.hooks import Continue, Middleware, Replace
from monoagens.infra.store import payload_text
from monoagens.middleware.contract import OUTPUT_MARKER
from monoagens.types import Action, Event

ROUTING = ("to", "via")
DESCRIBE = "registry.describe"
DEFAULT_OUTPUT_CAP = 8000
SKILL_WORD = 4
RULE_BLOCK_ORDER = 5  # before the core's `now` block (order 10)

DESCRIBE_CAPABILITY = {
    "name": "describe",
    "description": "Show the full schema of one tool, or the body of one skill.",
    "schema": {"type": "object", "properties": {"name": {"type": "string"}},
               "required": ["name"]},
    "communicative": False, "long_running": False, "identity_changing": False,
}


@dataclass(frozen=True, slots=True)
class ToolEntry:
    """One capability the soul may use, with what it costs and what it may touch."""

    name: str  # the full "<owner>.<capability>"
    owner: str
    descriptor: dict[str, Any]
    facts: frozenset[str]
    permission: str  # empty only for an adapter's talking capability
    output_cap: int
    result: bool = True  # False for a capability the middleware handles itself


@dataclass(frozen=True, slots=True)
class CommandEntry:
    """One command a person may type; handled in the inbound hook, never by the model."""

    name: str  # without the slash
    description: str
    permission: str
    handler: Callable[[Any, Event, list[str]], Awaitable[str | None]]
    owner: str = ""  # the plugin that registered it; empty for the middleware's own


@dataclass(frozen=True, slots=True)
class SkillEntry:
    """One installed SKILL.md: its name and description enter the prompt, its body on ask."""

    name: str
    description: str
    body: str
    owner: str = ""


class ToolRegistry:
    """The adapters and plugins registered with the kernel, their capabilities and commands."""

    def __init__(self) -> None:
        """Start with no tool, no command and no skill present."""
        self.adapters: dict[str, Any] = {}
        self.capabilities: dict[str, dict[str, Any]] = {}
        self.entries: dict[str, ToolEntry] = {}
        self.rules: list[str] = []
        self._commands: dict[str, CommandEntry] = {}
        self._skills: dict[str, SkillEntry] = {}

    # ------------------------------------------------------------- tools

    def register(self, adapter: Any, *, permissions: Mapping[str, str] | None = None) -> None:
        """Register one adapter-like owner and every capability it declares."""
        name = getattr(adapter, "name", None)
        if not name:
            raise ValueError("An adapter must expose a name")
        if name in self.adapters:
            raise ValueError(f"adapter collision: {name}")
        self.adapters[name] = adapter
        given = dict(permissions or {})
        for capability in getattr(adapter, "capabilities", lambda: ())():
            short = str(capability["name"])
            full = f"{name}.{short}"
            permission = self._permission(full, capability, given.get(short))
            self._add(full, name, capability,
                      facts=tuple(capability.get("facts") or ()), permission=permission,
                      output_cap=int(capability.get("output_cap") or DEFAULT_OUTPUT_CAP))

    def add_tool(self, tool: Any, facts: Mapping[str, Any] | None = None, *,
                 owner: str | None = None) -> None:
        """Register a plugin tool: every capability needs its facts and its permission."""
        name = owner or getattr(tool, "name", None)
        if not name:
            raise ValueError("A tool must expose a name")
        if name in self.adapters:
            raise ValueError(f"adapter collision: {name}")
        self.adapters[name] = tool
        declared = dict(facts or {})
        for capability in getattr(tool, "capabilities", lambda: ())():
            short = str(capability["name"])
            full = f"{name}.{short}"
            spec = declared.get(short)
            if not spec or "facts" not in spec:
                raise ValueError(f"{full}: a tool without facts")
            permission = str(spec.get("permission") or full)
            self._add(full, name, capability, facts=tuple(spec["facts"]),
                      permission=permission,
                      output_cap=int(spec.get("output_cap") or DEFAULT_OUTPUT_CAP))

    def add_capability(self, full: str, descriptor: dict[str, Any], *, facts: Any = (),
                       permission: str, output_cap: int = DEFAULT_OUTPUT_CAP,
                       result: bool = True) -> None:
        """Declare a capability the middleware dispatches itself, like `self.share`."""
        if not permission:
            raise ValueError(f"{full}: an entry without a permission")
        self._add(full, full.partition(".")[0], descriptor, facts=tuple(facts),
                  permission=permission, output_cap=output_cap, result=result)

    def _add(self, full: str, owner: str, capability: Mapping[str, Any], *, facts: Any,
             permission: str, output_cap: int, result: bool = True) -> None:
        if full in self.capabilities:
            raise ValueError(f"Capability collision: {full}")
        descriptor = {"organ": owner, **capability}
        self.capabilities[full] = descriptor
        self.entries[full] = ToolEntry(full, owner, descriptor, frozenset(facts),
                                       permission, output_cap, result)

    @staticmethod
    def _permission(full: str, capability: Mapping[str, Any], given: str | None) -> str:
        """Return an entry's permission; only a talking capability may declare none."""
        if given is not None:
            return given
        if "permission" in capability and capability["permission"] is None:
            return ""  # an adapter's talking capability: say, react and the like
        permission = str(capability.get("permission") or "")
        if not permission:
            raise ValueError(f"{full}: an entry without a permission")
        return permission

    def discard_owner(self, owner: str) -> None:
        """Remove everything one owner declared (a plugin that left the configuration)."""
        self.adapters.pop(owner, None)
        for full in [name for name, entry in self.entries.items() if entry.owner == owner]:
            self.entries.pop(full, None)
            self.capabilities.pop(full, None)
        for name in [name for name, entry in self._commands.items() if entry.owner == owner]:
            self._commands.pop(name, None)
        for name in [name for name, entry in self._skills.items() if entry.owner == owner]:
            self._skills.pop(name, None)

    def get(self, full: str) -> dict[str, Any] | None:
        """Return one capability's descriptor, or None when nothing declares it."""
        return self.capabilities.get(full)

    def entry(self, full: str) -> ToolEntry | None:
        """Return one capability's full entry, or None."""
        return self.entries.get(full)

    def facts(self, full: str) -> frozenset[str]:
        """Return the facts one capability declares; unknown means none."""
        entry = self.entries.get(full)
        return entry.facts if entry is not None else frozenset()

    def permission(self, full: str) -> str:
        """Return one capability's permission; unknown means none."""
        entry = self.entries.get(full)
        return entry.permission if entry is not None else ""

    def output_cap(self, full: str) -> int:
        """Return one capability's output cap; unknown means the default."""
        entry = self.entries.get(full)
        return entry.output_cap if entry is not None else DEFAULT_OUTPUT_CAP

    def descriptors(self) -> tuple[dict[str, Any], ...]:
        """Return the capability descriptors sorted by name (a fixed prompt order)."""
        return tuple(self.capabilities[name] for name in sorted(self.capabilities))

    def routing_keys(self, capability: dict[str, Any]) -> tuple[str, ...]:
        """Return the arguments an act routes on, which the core strips before acting."""
        return ROUTING if capability.get("communicative") else ()

    def allowed(self, who: Any, roles: Any = None) -> frozenset[str]:
        """Return the capabilities one requester may use; no roles book means talk only."""
        return frozenset(
            full for full, entry in self.entries.items()
            if not entry.permission or (roles is not None
                                        and roles.can(who, entry.permission)))

    async def act(self, event: Event, capability: str, payload: dict[str, Any], *,
                  event_id: int | None = None) -> Any:
        """Hand one action to the adapter or plugin that declared the capability."""
        declared_by, _, name = capability.partition(".")
        adapter = self.adapters.get(declared_by)
        if adapter is None:
            raise RuntimeError(f"No adapter registered for {declared_by!r}")
        action = Action(kind=name, place=event.place or "", payload=payload,
                        adapter=declared_by, event_id=event_id)
        return _capped(await adapter.act(action), self.output_cap(capability))

    # ---------------------------------------------------------- commands

    def add_command(self, entry: CommandEntry) -> None:
        """Register one command; a duplicate name or a missing permission fails at boot."""
        if not entry.permission:
            raise ValueError(f"{entry.name}: an entry without a permission")
        if entry.name in self._commands:
            raise ValueError(f"collision: {entry.name}")
        self._commands[entry.name] = entry

    def command(self, name: str) -> CommandEntry | None:
        """Return one command by name, or None."""
        return self._commands.get(name)

    def commands(self, who: Any = None, roles: Any = None) -> tuple[CommandEntry, ...]:
        """Return the commands, sorted by name, optionally only those one person may use."""
        found = [self._commands[name] for name in sorted(self._commands)]
        if who is None and roles is None:
            return tuple(found)
        return tuple(entry for entry in found if may_use(entry.permission, who, roles))

    # ------------------------------------------------------------ skills

    def add_skill(self, entry: SkillEntry) -> None:
        """Register one installed skill; a duplicate name fails at boot."""
        if entry.name in self._skills:
            raise ValueError(f"collision: {entry.name}")
        self._skills[entry.name] = entry

    def skills(self) -> tuple[SkillEntry, ...]:
        """Return the installed skills, sorted by name."""
        return tuple(self._skills[name] for name in sorted(self._skills))

    def skill(self, name: str) -> SkillEntry | None:
        """Return one skill by name, or None."""
        return self._skills.get(name)

    def add_rule(self, text: str) -> None:
        """Add one standing rule, shown in `<self>` in install order."""
        if text.strip():
            self.rules.append(text.strip())

    # ------------------------------------------------------------ lookup

    def describe(self, name: str) -> dict[str, Any]:
        """Return the full entry of one tool or the body of one skill (ADR 0015)."""
        entry = self.entries.get(name)
        if entry is not None:
            return {"tool": {**entry.descriptor, "facts": sorted(entry.facts),
                             "permission": entry.permission, "output_cap": entry.output_cap}}
        skill = self._skills.get(name)
        if skill is not None:
            return {"skill": {"name": skill.name, "description": skill.description,
                              "body": skill.body}}
        return {"error": f"unknown: {name}"}


class RegistryTool:
    """The middleware's own `describe` tool: the full entry on request (ADR 0015)."""

    name = "registry"

    def __init__(self, registry: ToolRegistry) -> None:
        """Keep the registry this tool reads."""
        self.registry = registry

    def capabilities(self) -> tuple[dict[str, Any], ...]:
        """Declare the one `describe` capability."""
        return (DESCRIBE_CAPABILITY,)

    async def act(self, action: Action) -> dict[str, Any]:
        """Return the full entry of the named tool or skill."""
        return self.registry.describe(str(action.payload.get("name") or ""))


def install_own(registry: ToolRegistry) -> None:
    """Declare the middleware's own `registry.describe` tool on the registry."""
    registry.add_tool(RegistryTool(registry), {
        "describe": {"facts": ["reads"], "permission": DESCRIBE, "output_cap": 4000}})


def may_use(permission: str, who: Any, roles: Any, place: str | None = None) -> bool:
    """Return whether one requester may use an entry with this permission (fail closed)."""
    if not permission:
        return True  # a talking capability of an adapter
    return roles is not None and roles.can(who, permission, place)


def _capped(value: Any, cap: int) -> Any:
    """Cut one tool result to its output cap with the one continue-from-here marker."""
    if isinstance(value, str):
        return _cut(value, cap)
    if isinstance(value, dict):
        for key in ("text", "content", "output", "stdout"):
            if isinstance(value.get(key), str):
                return {**value, key: _cut(value[key], cap)}
    return value


def _cut(text: str, cap: int) -> str:
    """Keep the first characters of a text and mark where the next read continues."""
    if cap <= 0 or len(text) <= cap:
        return text
    marker = OUTPUT_MARKER.format(limit=cap, n=cap)
    return text[:max(0, cap - len(marker))] + marker


# ---------------------------------------------------------------- commands


async def _help(kernel: Any, event: Event, arguments: list[str]) -> str:
    """Render `/help` for one requester: the commands and tools they may use."""
    who = event.origin_actor or event.who
    return help_text(kernel.tools, who, _roles(kernel))


def _roles(kernel: Any) -> Any:
    """Return the kernel's roles book, or None before one is installed."""
    return getattr(kernel, "roles", None)


def help_text(registry: ToolRegistry, who: Any, roles: Any = None) -> str:
    """Return the one `/help` body: the commands and the tools one person may use."""
    commands = "\n".join(f"/{entry.name} — {entry.description}"
                         for entry in registry.commands(who, roles))
    tools = "\n".join(_tool_line(entry)
                      for entry in sorted(registry.entries.values(), key=lambda e: e.name)
                      if entry.permission and may_use(entry.permission, who, roles))
    return f"Commands:\n{commands or 'none'}\n\nTools:\n{tools or 'none'}"


def _tool_line(entry: ToolEntry) -> str:
    """Render one tool as a help line: its name, its arguments and its description."""
    schema = entry.descriptor.get("schema") or {}
    names = list((schema.get("properties") or {}))
    names += ["to?", "via?"] if entry.descriptor.get("communicative") else []
    return f"{entry.name}({', '.join(names)}): {entry.descriptor.get('description') or ''}"


def install_commands(kernel: Any) -> None:
    """Register the middleware's own commands; `/help` is the same on every adapter."""
    kernel.tools.add_command(CommandEntry(
        name="help", description="List the commands and tools you may use.",
        permission="help", handler=_help))


def commands(kernel: Any) -> Middleware:
    """Inbound rule: handle a `command` event in the registry, without a model call."""

    async def rule(item: Any, ctx: Any) -> Any:
        """Answer one typed command through the adapter that delivered it."""
        if getattr(item, "kind", "") != "command":
            return Continue()
        payload = item.payload or {}
        name = str(payload.get("command") or "").strip().lstrip("/:").partition("@")[0].lower()
        arguments = [str(argument) for argument in payload.get("arguments") or ()]
        entry = kernel.tools.command(name)
        who, roles = item.origin_actor or item.who, _roles(kernel)
        if entry is None:
            text = f"no command /{name}; /help lists them"
        elif not may_use(entry.permission, who, roles, item.place):
            text = f"/{name} is not allowed for you"
        else:
            text = await entry.handler(kernel, item, arguments)
        if text:
            await _answer(kernel, item, text)
        return Continue()

    return rule


async def _answer(kernel: Any, event: Event, text: str) -> None:
    """Send one command's reply through the adapter that delivered the command."""
    capability = f"{event.body}.say"
    stored = await kernel._emit(event, "act.out", {
        "capability": capability, "arguments": {"text": text}, "communicative": True},
        place=event.place)
    await kernel.tools.act(event, capability, {"text": text}, event_id=stored.id)
    await kernel._emit(event, "act.result", {"capability": capability, "result": None})


# --------------------------------------------------------------- the block


def self_block(kernel: Any) -> Middleware:
    """Context rule: offer this turn only what its requester may use, behind `<self>`."""

    async def rule(item: Any, ctx: Any) -> Any:
        """Filter the offered capabilities and add the `<self>` block."""
        trigger = item.trigger
        who = ((trigger.origin_actor or trigger.who) if trigger is not None else None)
        roles = _roles(kernel)
        allowed = kernel.tools.allowed(who, roles)
        text = _self_text(kernel.tools, item, who, roles)
        blocks = (*item.blocks, Part("self", VOLATILE, RULE_BLOCK_ORDER, text=text))
        capabilities = tuple(capability for full, capability in item_capabilities(item)
                             if full in allowed)
        return Replace(replace(item, blocks=blocks, capabilities=capabilities))

    return rule


def item_capabilities(item: Any) -> tuple[tuple[str, dict[str, Any]], ...]:
    """Pair every offered capability descriptor with its full name."""
    return tuple((f"{capability['organ']}.{capability['name']}", capability)
                 for capability in item.capabilities)


def _self_text(registry: ToolRegistry, item: Any, who: Any, roles: Any) -> str:
    """Render the `<self>` block: the commands, the relevant skills and the rules."""
    lines = ["<self>"]
    commands_line = ", ".join(f"/{entry.name}" for entry in registry.commands(who, roles))
    if commands_line:
        lines.append(f"Commands: {commands_line}")
    skills_line = ", ".join(f"{entry.name} — {entry.description}"
                            for entry in relevant_skills(registry, item))
    if skills_line:
        lines.append(f"Skills: {skills_line}")
    if registry.rules:
        lines.append("Rules:")
        lines.extend(f"- {rule}" for rule in registry.rules)
    lines.append("</self>")
    return "\n".join(lines)


def relevant_skills(registry: ToolRegistry, item: Any) -> tuple[SkillEntry, ...]:
    """Return the skills whose name or description words appear in this wake (deterministic)."""
    haystack = " ".join([payload_text((item.trigger.payload if item.trigger else {}) or {}),
                         *(str(line) for line in (item.recent or ())[-10:]),
                         *(str(line) for line in (item.recent_elsewhere or ())[-10:])])
    words = {word for word in _words(haystack) if len(word) >= SKILL_WORD}
    return tuple(entry for entry in registry.skills()
                 if words & {word for word in _words(entry.name + " " + entry.description)
                             if len(word) >= SKILL_WORD})


def _words(text: str) -> set[str]:
    """Return the lowercase words of a text."""
    return {word.strip(".,;:!?()[]\"'").lower() for word in str(text).split()}


__all__ = ["CommandEntry", "SkillEntry", "ToolEntry", "ToolRegistry", "commands",
           "help_text", "install_commands", "install_own", "may_use", "self_block"]
