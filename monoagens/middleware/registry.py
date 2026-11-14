"""The registry: the tools, commands and skills present now, and the routing of one act.

One list of names with short descriptions, the full entry on request, one namespace with
collisions failing at boot (middleware.md §4). Every entry names **who uses it**: a tool or
a skill is for the soul, a command is for a person, and every entry declares a permission.

A capability descriptor keeps its frozen key `organ`; the value is the name of the adapter
or plugin that declares the capability (docs/core.md §8). The core sees only the descriptor;
the facts, the permission and the output cap live here, next to it.
"""

from __future__ import annotations

import asyncio
import logging
from collections.abc import Awaitable, Callable, Mapping
from dataclasses import dataclass, replace
from typing import Any

from monoagens import contract
from monoagens.core.hooks import Continue, Middleware, Replace
from monoagens.infra.store import payload_text
from monoagens.contract import OUTPUT_MARKER
from monoagens.middleware.contract import open_attachment
from monoagens.contract import Action
from monoagens.types import Event

ROUTING = ("to", "via")
LOGGER = logging.getLogger(__name__)
DESCRIBE = "registry.describe"
MORE = "registry.more"
DEFAULT_OUTPUT_CAP = 8000
MORE_OUTPUT_CAP = 16000

DESCRIBE_CAPABILITY = {
    "name": "describe",
    "description": "Show the full schema of one tool, or the body of one skill.",
    "schema": {"type": "object", "properties": {"name": {"type": "string"}},
               "required": ["name"]},
    "communicative": False, "long_running": False, "identity_changing": False,
}

MORE_CAPABILITY = {
    "name": "more",
    "description": "Read a slice of a truncated tool output by its attachment ref.",
    "schema": {
        "type": "object",
        "properties": {
            "ref": {"type": "string"},
            "offset": {"type": "integer"},
            "limit": {"type": "integer"},
        },
        "required": ["ref"],
    },
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
    body: bool = False  # a sense or a hand of one body, not a tool a person asks for


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

    def __init__(self, *, kernel: Any = None) -> None:
        """Start with no tool, no command and no skill present."""
        self.kernel = kernel
        self.adapters: dict[str, Any] = {}
        self.place_body: dict[str, str] = {}  # the body that senses and acts in each place
        self.capabilities: dict[str, dict[str, Any]] = {}
        self.entries: dict[str, ToolEntry] = {}
        self.rules: list[str] = []
        self._commands: dict[str, CommandEntry] = {}
        self._skills: dict[str, SkillEntry] = {}

    # ------------------------------------------------------------- tools

    def register(self, adapter: Any, *, permissions: Mapping[str, str] | None = None,
                 body: bool = False) -> None:
        """Register one adapter-like owner and every capability it declares (§4.1).

        `permissions` names the permission of a capability whose descriptor states none: the
        middleware's own tools declare theirs here, beside the role catalog that carries it.
        `body` marks the capabilities of a body (a sense or a hand) instead of a tool a person
        asks for by name, which `/help` therefore does not list.
        """
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
            permission = given.get(short) or self._permission(full, capability)
            self._add(full, name, capability,
                      facts=tuple(capability.get("facts") or ()), permission=permission,
                      output_cap=int(capability.get("output_cap") or DEFAULT_OUTPUT_CAP),
                      body=body)

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
            permission = str(spec.get("permission") or "")
            if not permission:
                raise ValueError(f"{full}: an entry without a permission")
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
             permission: str, output_cap: int, result: bool = True,
             body: bool = False) -> None:
        if full in self.capabilities:
            raise ValueError(f"Capability collision: {full}")
        descriptor = {"organ": owner, **capability}
        self.capabilities[full] = descriptor
        self.entries[full] = ToolEntry(full, owner, descriptor, frozenset(facts),
                                       permission, output_cap, result, body)

    @staticmethod
    def _permission(full: str, capability: Mapping[str, Any]) -> str:
        """Return an entry's permission: every entry declares one (contract.md §3)."""
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

    def produces_result(self, full: str) -> bool:
        """Return whether a call to one capability yields an `act.result` (unknown: yes)."""
        entry = self.entries.get(full)
        return True if entry is None else entry.result

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

    async def act(self, action: contract.Action) -> contract.Receipt:
        """Carry out one action and answer with one receipt (§3, §4.4).

        A body's own capability goes to the body that senses its place; a tool (a plugin's or
        the middleware's own) runs where its owner is. A place another body reaches is refused
        with that body's name, never retried behind its back.
        """
        entry = self.entries.get(action.capability)
        if entry is None:
            return contract.Receipt("failed", reason=f"nothing declares {action.capability}")
        if entry.body:
            here = await self.place_body_of(str(action.place))
            if here is not None and here != entry.owner:
                # A place another body senses is reached through it, never behind its back.
                return contract.Receipt(
                    "refused", reason=f"this place is reached through {here}")
        body = self.adapters.get(entry.owner)
        if body is None:
            return contract.Receipt("refused",
                                    reason=f"nothing carries {action.capability}")
        short = action.capability.split(".", 1)[1]
        try:
            answer = await body.act(replace(action, capability=short))
        except PermissionError as exc:
            # A place the body may not post in answers `refused` with its reason (§4.4).
            return contract.Receipt("refused", reason=str(exc))
        except Exception as exc:
            kind = type(exc).__name__
            LOGGER.error("tool %s failed (%s)", action.capability, kind)
            return contract.Receipt("failed", reason=f"{action.capability} failed ({kind})")
        receipt = (answer if isinstance(answer, contract.Receipt)
                   else contract.Receipt("done", result=answer))
        if receipt.state == "held":
            LOGGER.error("a body cannot hold an action: %s", action.capability)
            return contract.Receipt(
                "failed", reason=f"{action.capability}: a body cannot hold an action")
        if receipt.state != "done":
            return receipt
        if isinstance(receipt.result, Mapping) and receipt.result.get("error"):
            return contract.Receipt("failed", reason=str(receipt.result["error"]))
        capped = await self._cap_result(receipt.result, self.output_cap(action.capability))
        return replace(receipt, result=capped)

    async def place_body_of(self, place: str) -> str | None:
        """Return the body that senses a place: the live cache, else the Log's newest percept."""
        known = self.place_body.get(place)
        if known is not None or not place:
            return known
        conn = getattr(self.kernel, "conn", None)
        if conn is None:
            return None
        row = await (await conn.execute(
            "SELECT body FROM events WHERE place = %s AND kind = 'percept' "
            "AND body IS NOT NULL ORDER BY id DESC LIMIT 1", (place,))).fetchone()
        if row is None:
            return None
        self.place_body[place] = str(row[0])
        return str(row[0])

    async def more(self, ref: str, offset: int = 0, limit: int = DEFAULT_OUTPUT_CAP,
                   *, context: str | None = None) -> dict[str, Any]:
        """Read one character slice from an archived full tool output."""
        if not ref:
            return {"error": "ref is required"}
        if not context:
            return {"error": "context place is required", "ref": ref}
        attachments = getattr(self.kernel, "attachments", None)
        conn = getattr(self.kernel, "conn", None)
        if attachments is None or conn is None:
            return {"error": "attachment store is not configured", "ref": ref}
        try:
            raw, meta = await open_attachment(conn, attachments, ref, context=context)
        except (PermissionError, FileNotFoundError) as exc:
            return {"error": str(exc), "ref": ref}
        capped_limit = min(max(1, int(limit)), MORE_OUTPUT_CAP)
        sliced = await asyncio.to_thread(_slice_utf8, raw, offset, capped_limit)
        return {"ref": ref, "size": meta["size"], **sliced}

    async def _cap_result(self, value: Any, cap: int) -> Any:
        """Cap a tool result and archive overflowing text into the attachment store."""
        text_key = _overflow_key(value, cap)
        if text_key is None:
            return _capped(value, cap)
        raw_text = value if isinstance(value, str) else value[text_key]
        archived = await self._archive_text(raw_text)
        if archived is None:
            return _capped(value, cap)
        full = {"ref": archived["ref"], "size": archived["size"]}
        cut_text = _cut(raw_text, cap, ref=full["ref"])
        if isinstance(value, str):
            return {"text": cut_text, "full": full}
        return {**value, text_key: cut_text, "full": full}

    async def _archive_text(self, text: str) -> dict[str, Any] | None:
        """Write full output text to the kernel's attachment store when configured."""
        attachments = getattr(self.kernel, "attachments", None)
        if attachments is None:
            return None
        data = await asyncio.to_thread(text.encode, "utf-8")
        return await attachments.put(data, name="output.txt", mime="text/plain; charset=utf-8")

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
    """The middleware's own `registry` tool: `describe` and `more` (middleware.md §4)."""

    name = "registry"

    def __init__(self, registry: ToolRegistry) -> None:
        """Keep the registry this tool reads."""
        self.registry = registry

    def capabilities(self) -> tuple[dict[str, Any], ...]:
        """Declare `describe` and `more`."""
        return (DESCRIBE_CAPABILITY, MORE_CAPABILITY)

    async def act(self, action: Any) -> dict[str, Any]:
        """Return a tool/skill entry (`describe`) or a slice of archived output (`more`)."""
        payload = action.arguments or {}
        if action.capability == "more":
            limit = payload.get("limit")
            return await self.registry.more(
                str(payload.get("ref") or ""),
                offset=int(payload.get("offset") or 0),
                limit=DEFAULT_OUTPUT_CAP if limit is None else int(limit),
                context=action.place or None,
            )
        return self.registry.describe(str(payload.get("name") or ""))


def install_own(registry: ToolRegistry) -> None:
    """Declare the middleware's own `registry.describe` and `registry.more` tools."""
    registry.add_tool(RegistryTool(registry), {
        "describe": {"facts": ["reads"], "permission": DESCRIBE, "output_cap": 4000},
        "more": {"facts": ["reads"], "permission": MORE, "output_cap": MORE_OUTPUT_CAP},
    })


def may_use(permission: str, who: Any, roles: Any, place: str | None = None) -> bool:
    """Return whether one requester may use an entry with this permission (fail closed)."""
    if not permission:
        return False  # every entry declares one (§3); one that does not is never used
    return roles is not None and roles.can(who, permission, place)


def _overflow_key(value: Any, cap: int) -> str | None:
    """Return the string key exceeding `cap` (or `""` for a bare string), else None."""
    if cap <= 0:
        return None
    if isinstance(value, str):
        return "" if len(value) > cap else None
    if isinstance(value, dict):
        for key in ("text", "content", "output", "stdout"):
            item = value.get(key)
            if isinstance(item, str) and len(item) > cap:
                return key
    return None


def _slice_utf8(raw: bytes, offset: int, limit: int) -> dict[str, Any]:
    """Decode archived UTF-8 output and return one character slice."""
    text = raw.decode("utf-8", errors="replace")
    total = len(text)
    start = max(0, min(offset, total))
    end = min(total, start + max(1, limit))
    remaining = total - end
    return {
        "text": text[start:end],
        "offset": start,
        "next_offset": end if remaining > 0 else None,
        "remaining": remaining,
        "total_chars": total,
    }


def _capped(value: Any, cap: int, *, ref: str = "none") -> Any:
    """Cut one tool result to its output cap with the one continue-from-here marker."""
    if isinstance(value, str):
        return _cut(value, cap, ref=ref)
    if isinstance(value, dict):
        for key in ("text", "content", "output", "stdout"):
            if isinstance(value.get(key), str):
                return {**value, key: _cut(value[key], cap, ref=ref)}
    return value


def _cut(text: str, cap: int, *, ref: str = "none") -> str:
    """Keep the first characters of a text and mark where the next read continues."""
    if cap <= 0 or len(text) <= cap:
        return text
    keep = max(0, cap - len(OUTPUT_MARKER.format(limit=cap, ref=ref, n=cap)))
    for _ in range(3):
        marker = OUTPUT_MARKER.format(limit=cap, ref=ref, n=keep)
        next_keep = max(0, cap - len(marker))
        if next_keep == keep:
            break
        keep = next_keep
    marker = OUTPUT_MARKER.format(limit=cap, ref=ref, n=keep)
    if keep + len(marker) > cap:
        keep = max(0, cap - len(marker))
        marker = OUTPUT_MARKER.format(limit=cap, ref=ref, n=keep)
    return text[:keep] + marker


# ---------------------------------------------------------------- commands


def place_owner(tools: ToolRegistry) -> Middleware:
    """Inbound rule: remember which body senses and acts in each place (§4.4)."""

    async def rule(item: Any, ctx: Any) -> Any:
        """Record the body that delivered a percept in its place (§2.4.4)."""
        del ctx
        place = str(getattr(item, "place", None) or "")
        body = str(getattr(item, "body", None) or "")
        if (place and body and getattr(item, "kind", "") == "percept"
                and not getattr(item, "imported", False)):
            tools.place_body[place] = body
        return Continue()

    return rule


def own_action(event: Event, capability: str, arguments: Mapping[str, Any]) -> contract.Action:
    """Return the action one act the middleware itself decided on becomes (§3).

    The id is derived from the event it belongs to, so a retried internal act carries the same
    id a body already answered.
    """
    return contract.Action(
        id=contract.action_id("middleware", int(event.id or 0), 0, 0), capability=capability,
        place=str(event.place or ""), arguments=dict(arguments), caused_by=event.id)


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
                      if not entry.body and may_use(entry.permission, who, roles))
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
        """Answer one typed command, from the `command` a body put in its percept (§2.4.7)."""
        payload = dict(getattr(item, "payload", None) or {})
        command = payload.get("command")
        if not isinstance(command, Mapping) or not command.get("name"):
            return Continue()
        name = str(command["name"])
        arguments = [str(argument) for argument in command.get("args") or ()]
        entry = kernel.tools.command(name)
        who, roles = item.origin_actor or item.who, _roles(kernel)
        if entry is None:
            text = f"no command /{name}; /help lists them"
        elif not may_use(entry.permission, who, roles, item.place):
            text = f"/{name} is not allowed for you"
        else:
            try:
                text = await entry.handler(kernel, item, arguments)
            except Exception as exc:
                await kernel._record_error(item, f"/{name} failed ({type(exc).__name__})")
                text = f"/{name} failed: {type(exc).__name__}"
        if text:
            await _answer(kernel, item, text)
        return Continue()

    return rule


async def _answer(kernel: Any, event: Event, text: str) -> None:
    """Answer one command through the action hook, like every other act (F.12)."""
    capability = f"{event.body}.say"
    await kernel.act_via_hooks(event, capability, {"text": text})


# --------------------------------------------------------------- the block


def self_block(kernel: Any) -> Middleware:
    """Context rule: offer this turn only what its requester may use, and say what is installed."""

    async def rule(item: Any, ctx: Any) -> Any:
        """Narrow the offered capabilities and set the `<self>` block."""
        trigger = item.trigger
        who = ((trigger.origin_actor or trigger.who) if trigger is not None else None)
        roles = _roles(kernel)
        allowed = kernel.tools.allowed(who, roles)
        text = self_text(kernel.tools, who, roles)
        capabilities = tuple(capability for full, capability in item_capabilities(item)
                             if full in allowed)
        return Replace(replace(item, self_text=text, capabilities=capabilities))

    return rule


def item_capabilities(item: Any) -> tuple[tuple[str, dict[str, Any]], ...]:
    """Pair every offered capability descriptor with its full name."""
    return tuple((f"{capability['organ']}.{capability['name']}", capability)
                 for capability in item.capabilities)


def self_text(registry: ToolRegistry, who: Any, roles: Any) -> str:
    """Render `<self>`: the commands, every installed skill and the standing rules (D-I)."""
    lines = ["<self>"]
    commands_line = ", ".join(f"/{entry.name}" for entry in registry.commands(who, roles))
    if commands_line:
        lines.append(f"Commands: {commands_line}")
    skills_line = ", ".join(f"{entry.name} — {entry.description}"
                            for entry in registry.skills())
    if skills_line:
        lines.append(f"Skills: {skills_line}")
    if registry.rules:
        lines.append("Rules:")
        lines.extend(f"- {rule}" for rule in registry.rules)
    lines.append("</self>")
    return "\n".join(lines)


__all__ = ["CommandEntry", "SkillEntry", "ToolEntry", "ToolRegistry", "commands",
           "help_text", "install_commands", "install_own", "may_use", "self_block",
           "self_text"]
