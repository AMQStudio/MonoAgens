"""The composition root of the middleware: the owner policy, the intake, the tools, the limits.

`install(kernel)` is the only place that decides what is installed and in which order. The
core holds the four injected values it is given here and imports none of this.

The policy is always present and runs first and last in every hook: it carries the two
absolute lines, the daily model budget and quiet hours, and it turns a person's roles and a
tool's facts into pass, ask or refuse.
"""

from __future__ import annotations

import ipaddress
import pathlib
from collections.abc import Awaitable, Callable, Mapping
from datetime import datetime, time, timedelta, timezone
from typing import Any
from zoneinfo import ZoneInfo

from monoagens.core.hooks import Continue, Middleware, Stop
from monoagens.core.identity import SelfTool
from monoagens.infra.store import append_event, read_state
from monoagens.middleware import identity, interaction, loader, privacy, registry, roles
from monoagens.middleware.contract import compose
from monoagens.types import Event

IDENTITY_SET = "identity.set"  # the owner's alone: a permission without a base

# The addresses nothing outside may reach: private, loopback, link-local, the container
# ranges and the metadata service. One table for `web_fetch` and the sandbox (plugins.md §5).
BLOCKED_NETWORKS = tuple(ipaddress.ip_network(value) for value in (
    "10.0.0.0/8", "172.16.0.0/12", "192.168.0.0/16", "100.64.0.0/10", "127.0.0.0/8",
    "169.254.0.0/16", "0.0.0.0/8", "::1/128", "fc00::/7", "fe80::/10"))


def is_blocked_address(address: str) -> bool:
    """Return whether one address is private, reserved or otherwise off limits (fail closed)."""
    try:
        value = ipaddress.ip_address(str(address).strip("[]").split("%")[0])
    except ValueError:
        return True
    if isinstance(value, ipaddress.IPv6Address) and value.ipv4_mapped is not None:
        value = value.ipv4_mapped
    if (value.is_private or value.is_loopback or value.is_link_local or value.is_multicast
            or value.is_reserved or value.is_unspecified):
        return True
    return any(network.version == value.version and value in network
               for network in BLOCKED_NETWORKS)


def install(kernel: Any) -> None:
    """Give the kernel its registry, its intake, and the owner policy in all three hooks."""
    kernel.tools = registry.ToolRegistry(kernel=kernel)
    kernel.tools.register(SelfTool(kernel.conn, kernel.clock,
                                   visible=privacy.visible,
                                   memory_visible=privacy.memory_visible),
                          permissions={"recall": "self.recall",
                                       "set_identity": IDENTITY_SET})
    privacy.register(kernel.tools)
    registry.install_own(kernel.tools)
    kernel.roles = roles.RoleBook.empty()
    kernel.prior_knobs = prior_knobs(kernel.settings)
    kernel.knob_bounds = {}
    kernel.intake = identity.intake(kernel.conn, clock=kernel.clock)
    kernel.budget_limit = daily_limit(kernel.settings)
    kernel.after_model_call = enforce(kernel)
    for pipeline in kernel.hooks:
        pipeline.set_policy(policy_for(pipeline.name, "first", kernel),
                            policy_for(pipeline.name, "last", kernel))
        for name, middleware in modules_for(pipeline.name, kernel):
            pipeline.add(name, middleware)
    registry.install_commands(kernel)
    roles.install_commands(kernel)
    identity.install_commands(kernel)
    install_commands(kernel)
    interaction.install_commands(kernel)
    kernel.after_model_call = enforce(kernel)


class _LiveRoles:
    """Delegate attribute access to `kernel.roles` so post-boot reloads reach cards."""

    def __init__(self, kernel: Any) -> None:
        self._kernel = kernel

    def __bool__(self) -> bool:
        return bool(getattr(self._kernel, "roles", None))

    def __getattr__(self, name: str) -> Any:
        current = getattr(self._kernel, "roles", None)
        if current is None:
            raise AttributeError(name)
        return getattr(current, name)


def modules_for(hook: str, kernel: Any) -> tuple[tuple[str, Middleware], ...]:
    """Return the scoped middleware of one hook, in the order it runs."""
    return {"inbound": (("identity-pairing", identity.claim(kernel.conn)),
                        ("audience-size", privacy.audience_size(kernel)),
                        ("place-owner", registry.place_owner(kernel.tools)),
                        ("approval-answer", interaction.answer(kernel)),
                        ("commands", registry.commands(kernel)),
                        ("privacy-intent", privacy.intent_words(kernel)),
                        ("narrate", interaction.narrate(kernel)),
                        ("read-receipts", compose(interaction.read_receipts_inbound(kernel),
                                                  kernel=kernel)),
                        ("attachment-sweep", attachment_sweep(kernel))),
            "context": (("identity-cards", identity.cards(kernel.conn, roles=_LiveRoles(kernel))),
                        ("self-block", registry.self_block(kernel))),
            "action": (("identity-route", identity.route(kernel.conn)),
                       ("attachments-split", interaction.split(kernel)))}.get(hook, ())


def attachment_sweep(kernel: Any) -> Middleware:
    """Inbound rule: hard forgetting deletes the files no surviving event carries."""

    async def rule(item: Any, ctx: Any) -> Any:
        """Sweep the attachment store when a `forget.hard` arrives."""
        store = getattr(kernel, "attachments", None)
        if getattr(item, "kind", "") == "forget.hard" and store is not None:
            await store.sweep(kernel.conn)
        return Continue()

    return rule


def policy_for(hook: str, when: str, kernel: Any) -> Middleware:
    """Return the absolute lines of one hook: identity, permissions, privacy, approvals, brakes."""
    if hook == "context":
        if when == "first":
            return compose(identity.identity_line(), brake_line(kernel), kernel=kernel)
        return compose(
            privacy.items_rule(kernel),
            kernel=kernel,
        )
    if hook == "action":
        if when == "first":
            return compose(identity.identity_line(), permission_line(kernel),
                           interaction.typing(kernel), privacy.guard(kernel, "first"),
                           kernel=kernel)
        return compose(privacy.guard(kernel, "last"), quiet_line(kernel),
                       approval_line(kernel), interaction.read_receipts_action(kernel),
                       kernel=kernel)
    return compose(identity.identity_line(), privacy.delegation(kernel), kernel=kernel)


def daily_limit(settings: Mapping[str, Any]) -> float:
    """Return the owner's daily token budget (0 or less means no cap)."""
    owner = settings.get("owner_settings") or {}
    return float(owner.get("daily_token_budget") or 0)


def brake_limits(settings: Mapping[str, Any]) -> tuple[int, int]:
    """Return `(max_task_steps, max_wakes_per_hour)` from owner settings or prior constants."""
    owner = settings.get("owner_settings") or {}
    constants = settings.get("constants") or {}
    steps = owner.get("max_task_steps")
    if steps is None:
        steps = constants.get("max_task_steps", 30)
    wakes = owner.get("max_wakes_per_hour")
    if wakes is None:
        wakes = constants.get("max_wakes_per_hour", 20)
    return int(steps), int(wakes)


async def _chain_brake_state(conn: Any, trigger: Event, now: datetime) -> tuple[int, int, int, str, bool]:
    """Return `(root_id, steps, wakes_in_hour, work_label, already_held)` for one chain."""
    anchor = trigger.id or trigger.caused_by
    if anchor is None:
        return 0, 0, 0, "", False
    cur = await conn.execute(
        "WITH RECURSIVE up AS ("
        "SELECT id, caused_by, payload FROM events WHERE id = %s "
        "UNION ALL SELECT e.id, e.caused_by, e.payload FROM events e JOIN up u ON e.id = u.caused_by"
        ") SELECT id, payload FROM up ORDER BY id ASC LIMIT 1",
        (int(anchor),),
    )
    root_row = await cur.fetchone()
    if root_row is None:
        return 0, 0, 0, "", False
    root_id = int(root_row[0])
    root_payload = dict(root_row[1] or {})
    work = str(root_payload.get("description") or root_payload.get("text")
               or trigger.place or f"chain #{root_id}")[:80]
    resumed = await (await conn.execute(
        "SELECT id, ts FROM events WHERE kind = %s AND payload->>'root' = %s "
        "ORDER BY id DESC LIMIT 1", (interaction.BRAKE_RESUMED, str(root_id)))).fetchone()
    after_id = int(resumed[0]) if resumed is not None else 0
    window_start = now - timedelta(hours=1)
    if resumed is not None and resumed[1] > window_start:
        window_start = resumed[1]
    held = await (await conn.execute(
        "SELECT 1 FROM events WHERE kind = %s AND id > %s AND payload->>'root' = %s LIMIT 1",
        (interaction.BRAKE_HELD, after_id, str(root_id)))).fetchone()
    counts = await (await conn.execute(
        "WITH RECURSIVE down AS ("
        "SELECT id, kind, ts, payload FROM events WHERE id = %s "
        "UNION ALL SELECT e.id, e.kind, e.ts, e.payload FROM events e JOIN down d ON e.caused_by = d.id"
        ") SELECT "
        "COUNT(*) FILTER (WHERE kind = 'act.out' AND id > %s), "
        "COUNT(*) FILTER (WHERE kind = 'job.started' AND payload->>'job' = 'pfc' "
        "  AND id > %s AND ts >= %s) "
        "FROM down",
        (root_id, after_id, after_id, window_start),
    )).fetchone()
    return root_id, int(counts[0] or 0), int(counts[1] or 0), work, held is not None


def brake_line(kernel: Any) -> Middleware:
    """Context pass: stop a chain at `max_task_steps` or `max_wakes_per_hour` and tell the owner."""

    async def rule(item: Any, ctx: Any) -> Any:
        """Stop an endless or oversized chain before starting another model call."""
        trigger = ctx.get("event")
        if trigger is None:
            return Continue()
        max_steps, max_wakes = brake_limits(kernel.settings)
        if max_steps <= 0 and max_wakes <= 0:
            return Continue()
        root_id, steps, wakes, work, already_held = await _chain_brake_state(
            kernel.conn, trigger, kernel.clock.now())
        if root_id == 0:
            return Continue()
        place = str(getattr(item, "place", None) or trigger.place or "")
        first_task_step = False
        if place.startswith("space:task:"):
            task = await read_state(kernel.conn, f"task:{place}")
            first_task_step = bool(task and int(task.get("steps") or 0) == 0)
        reason, limit_name, count, cap = "", "", 0, 0
        if max_wakes > 0 and wakes >= max_wakes:
            limit_name, count, cap = "max_wakes_per_hour", wakes, max_wakes
            reason = f"brake: {limit_name} limit reached ({wakes}/{max_wakes})"
        elif max_steps > 0 and steps >= max_steps and not first_task_step:
            limit_name, count, cap = "max_task_steps", steps, max_steps
            reason = f"brake: {limit_name} limit reached ({steps}/{max_steps})"
        if not reason:
            return Continue()
        if not already_held:
            await interaction.notify_brake(
                kernel, trigger, root_id=root_id, place=place or str(trigger.place or ""),
                work=work, reason=reason, limit=limit_name, value=count, cap=cap)
        return Stop(reason)

    return rule


def enforce(kernel: Any) -> Callable[[int, Mapping[str, int]], Awaitable[None]]:
    """Return the watcher that writes `budget.exhausted` once per local day."""

    async def watch(call_id: int, totals: Mapping[str, int]) -> None:
        """Record the day the budget ran out, once, caused by the call that spent it."""
        limit = kernel.budget_limit
        used = int(totals.get("total_tokens") or 0)
        if limit <= 0 or used < limit:
            return
        now = kernel.clock.now()
        local = now.astimezone(ZoneInfo(str(kernel.settings.get("timezone") or "UTC")))
        midnight = datetime.combine(local.date(), time.min, tzinfo=local.tzinfo)
        cur = await kernel.conn.execute("SELECT 1 FROM events WHERE kind = 'budget.exhausted' "
                                        "AND ts >= %s LIMIT 1", (midnight,))
        if await cur.fetchone() is None:
            await append_event(kernel.conn, ts=now, kind="budget.exhausted", caused_by=call_id,
                               payload={"date": local.date().isoformat(), "used": used,
                                        "budget": limit})

    return watch


# ------------------------------------------------------------ pass or refuse


def permission_line(kernel: Any) -> Middleware:
    """Absolute line: a call passes only for a requester who holds its permission."""

    async def rule(item: Any, ctx: Any) -> Any:
        """Refuse a capability the requester's roles do not give them."""
        if ctx.get("refusal") or ctx.get("handled"):
            return Continue()
        capability = str(ctx.get("capability") or "")
        permission = kernel.tools.permission(capability)
        who = ctx.get("origin_actor")
        if permission and not kernel.roles.can(who, permission, ctx["event"].place):
            ctx["refusal"] = f"you may not use {capability}"
            return Stop(f"you may not use {capability}")
        return Continue()

    return rule


async def _needs_approval(kernel: Any, ctx: Any, facts: set[str]) -> str:
    """Return 'pass', 'ask' or 'refused' for one call: an approval covers only its own call."""
    event = ctx["event"]
    approved = event.payload.get("approved") if event.kind == "intent" else None
    if approved is not None:
        asked = await interaction.by_id(kernel.conn, int(approved))
        asked_who = asked.get("who") if asked else None
        same_actor = (not asked_who) or (str(asked_who) == str(ctx.get("origin_actor") or ""))
        same = (asked is not None
                and same_actor
                and str(asked.get("capability")) == str(ctx.get("capability"))
                and (asked.get("arguments") or {}) == (ctx.get("arguments") or {}))
        if same:
            if await interaction.is_consumed(kernel.conn, int(approved)):
                return "ask"
            return "pass"
        if await _asked_again(kernel.conn, int(approved)):
            return "refused"
        return "ask"
    if "sensitive" in facts:
        return "ask"
    mode = kernel.roles.approval(ctx.get("origin_actor"))
    if mode == "yolo":
        return "pass"
    if mode == "strict":
        return "ask"
    return "ask" if "runs_code" in facts else "pass"


async def _asked_again(conn: Any, request_id: int) -> bool:
    """Return whether this approval already led to a second question (never a third)."""
    asked = await interaction.by_id(conn, request_id)
    if asked and asked.get("retried_from"):
        return True  # this answer approved the second question: the last chance is spent
    row = await (await conn.execute(
        "SELECT 1 FROM events WHERE kind = %s AND payload->>'retried_from' = %s LIMIT 1",
        (interaction.REQUEST, str(request_id)))).fetchone()
    return row is not None


def _reason(facts: set[str], mode: str, ctx: Any) -> str:
    """Name why the call needs the owner, in words the owner can judge."""
    if ctx.get("approval_reason"):
        return str(ctx["approval_reason"])
    if "sensitive" in facts:
        return "the call is marked sensitive"
    if mode == "strict":
        return f"the approval mode is {mode}"
    return "the call runs code"


def approval_line(kernel: Any) -> Middleware:
    """Last pass: pass, ask the owner, or refuse, from the facts and the approval mode."""

    async def rule(item: Any, ctx: Any) -> Any:
        """Ask where the approval mode says so, and pass a call the owner already allowed."""
        if ctx.get("refusal") or ctx.get("handled"):
            return Continue()
        capability = str(ctx.get("capability") or "")
        facts = set(kernel.tools.facts(capability)) | set(ctx.get("call_facts") or ())
        decision = await _needs_approval(kernel, ctx, facts)
        if decision == "pass":
            approved = (ctx["event"].payload.get("approved")
                        if ctx["event"].kind == "intent" else None)
            if approved is not None:
                await kernel._emit(ctx["event"], interaction.CONSUMED, {
                    "request": int(approved), "capability": capability,
                    "who": ctx.get("origin_actor")})
            return Continue()
        if decision == "refused":
            ctx["refusal"] = "the approved call was different; ask the owner again"
            return Continue()
        mode = kernel.roles.approval(ctx.get("origin_actor"))
        reason = _reason(facts, mode, ctx)
        stored = await kernel._emit(ctx["event"], interaction.REQUEST, {
            "capability": capability, "arguments": dict(ctx.get("arguments") or {}),
            "who": ctx.get("origin_actor"), "place": ctx["event"].place,
            "reason": reason,
            **({"retried_from": ctx["event"].payload.get("approved")}
               if ctx["event"].kind == "intent" and ctx["event"].payload.get("approved")
               else {})})
        ctx["refusal"] = "waiting for the owner's approval"
        ctx["held"] = True
        await interaction.request(kernel, ctx["event"], int(stored.id), capability,
                                  dict(ctx.get("arguments") or {}), reason)
        return Continue()

    return rule


def _quiet_window(hours: Mapping[str, Any]) -> tuple[str, str] | None:
    """Return the quiet window as two `HH:MM` strings, or None when it is off."""
    start = str((hours or {}).get("from") or "").strip()
    end = str((hours or {}).get("to") or "").strip()
    return (start, end) if start and end else None


def in_quiet_hours(now: datetime, hours: Mapping[str, Any],
                   zone: str = "UTC") -> tuple[bool, str]:
    """Return whether the local time is inside the quiet window, and when it ends."""
    window = _quiet_window(hours)
    if window is None:
        return False, ""
    start, end = window
    local = now.astimezone(ZoneInfo(zone)).time()
    try:
        begins, ends = time.fromisoformat(start), time.fromisoformat(end)
    except ValueError:
        return False, ""
    inside = (begins <= local < ends) if begins <= ends else (local >= begins or local < ends)
    return inside, end


def quiet_line(kernel: Any) -> Middleware:
    """Last pass: silence in every place but the owner's own chat while quiet hours last."""

    async def rule(item: Any, ctx: Any) -> Any:
        """Refuse a communicative act during the owner's quiet hours."""
        if ctx.get("refusal") or ctx.get("handled"):
            return Continue()
        if not (ctx.get("descriptor") or {}).get("communicative"):
            return Continue()
        place = ctx.get("place") or ctx["event"].place
        if await privacy.owner_direct(kernel.conn, place):
            return Continue()
        owner = kernel.settings.get("owner_settings") or {}
        inside, until = in_quiet_hours(kernel.clock.now(), owner.get("quiet_hours") or {},
                                       str(kernel.settings.get("timezone") or "UTC"))
        if not inside:
            return Continue()
        ctx["refusal"] = f"quiet hours: the owner asked for silence until {until}"
        ctx["held"] = True
        return Continue()

    return rule


# ------------------------------------------------------------ owner commands


def install_commands(kernel: Any) -> None:
    """Register `/budget` and `/quiet`: the owner settings that write their own line."""
    for name, description, permission, handler in (
            ("budget", "Show today's token budget; /budget <n> sets it.", "budget.manage",
             _budget),
            ("quiet", "Set the quiet hours: /quiet 22:00 06:00, or /quiet off.",
             "quiet.manage", _quiet)):
        if kernel.tools.command(name) is None:
            kernel.tools.add_command(registry.CommandEntry(name, description, permission,
                                                           handler, owner="policy"))


def _config_text(patch: Callable[[str], str], kind: str, argument: str) -> str:
    """Return the config.yaml text with one owner setting set."""
    return patch


async def _budget(kernel: Any, event: Event, arguments: list[str]) -> str:
    """Show today's tokens, or set the owner's daily cap in config.yaml."""
    from monoagens.infra import budget as budget_module

    used = int((await read_state(kernel.conn, budget_module.key_for(
        kernel.clock.now(), kernel.settings)) or {}).get("total_tokens") or 0)
    limit = int(kernel.budget_limit)
    if not arguments:
        return f"budget: {used} of {limit or 'no cap'} tokens today"
    try:
        value = int(arguments[0])
        if value < 0:
            raise ValueError
    except ValueError:
        return "usage: /budget <tokens>"
    before = limit
    loader.edit_config(_config_path(kernel), lambda text: loader.set_block_entry(
        text, "owner_settings", "daily_token_budget", value), now=kernel.clock.now())
    kernel.budget_limit = float(value)
    kernel.settings.setdefault("owner_settings", {})["daily_token_budget"] = value
    await loader.record_config_change(kernel, event, plugin="policy",
                                      key="owner_settings.daily_token_budget",
                                      before=before, after=value)
    return f"budget: {used} of {value} tokens today"


async def _quiet(kernel: Any, event: Event, arguments: list[str]) -> str:
    """Set or clear the owner's quiet hours in config.yaml."""
    owner = kernel.settings.setdefault("owner_settings", {})
    before = dict(owner.get("quiet_hours") or {})
    if not arguments:
        window = _quiet_window(before)
        return "quiet hours: off" if window is None else \
            f"quiet hours: {window[0]} to {window[1]}"
    off_words = tuple(interaction.catalog(kernel.settings).get("quiet", {}).get("off_words")
                      or ("off", "none"))
    if len(arguments) == 1 and arguments[0].lower() in off_words:
        after: dict[str, Any] = {}
        rendered: Any = {}
    elif len(arguments) == 2:
        try:
            time.fromisoformat(arguments[0])
            time.fromisoformat(arguments[1])
        except ValueError:
            return "usage: /quiet 22:00 06:00, or /quiet off"
        after = {"from": arguments[0], "to": arguments[1]}
        rendered = after
    else:
        return "usage: /quiet 22:00 06:00, or /quiet off"
    loader.edit_config(_config_path(kernel), lambda text: loader.set_block_entry(
        text, "owner_settings", "quiet_hours", rendered), now=kernel.clock.now())
    owner["quiet_hours"] = after
    await loader.record_config_change(kernel, event, plugin="policy",
                                      key="owner_settings.quiet_hours",
                                      before=before or None, after=after or None)
    return "quiet hours: off" if not after else f"quiet hours: {after['from']} to {after['to']}"


def _config_path(kernel: Any) -> Any:
    """Return the config.yaml a running kernel writes; a test kernel may have none."""
    root = getattr(kernel, "deployment_root", None)
    if root is None:
        raise RuntimeError("this kernel has no deployment root to write to")
    return pathlib.Path(root) / "config.yaml"


# ------------------------------------------------------------- knob offsets


def prior_knobs(settings: Mapping[str, Any]) -> dict[str, float]:
    """Return the prior's own knob values, before any plugin offset."""
    return {str(name): float(value) for name, value in (settings.get("knobs") or {}).items()
            if isinstance(value, (int, float)) and not isinstance(value, bool)}


async def offsets(conn: Any, plugin: str) -> dict[str, float]:
    """Return the latest offset per knob one plugin recorded in `disposition.updated`."""
    cur = await conn.execute(
        "SELECT payload FROM events WHERE kind = 'disposition.updated' "
        "AND payload->>'source' = %s ORDER BY id", (plugin,))
    found: dict[str, float] = {}
    for (payload,) in await cur.fetchall():
        knob = str((payload or {}).get("knob") or "")
        if knob:
            found[knob] = float((payload or {}).get("offset") or 0.0)
    return found


def _clamp(value: float, low: float, high: float) -> float:
    """Return one value kept inside two bounds."""
    return max(low, min(high, value))


async def apply_offsets(kernel: Any) -> None:
    """Recompute the effective knobs from the prior plus every installed plugin's offsets.

    One pass, never a snapshot: the bounds come from the installed manifests (a plugin that
    left has none), each offset is clamped to its manifest's bounds and to [-1, 1], the
    offsets are summed and the total is clamped to [0, 1]. The prior stays the base, never an
    effective value, so nothing feeds back on itself.
    """
    base = dict(getattr(kernel, "prior_knobs", None) or prior_knobs(kernel.settings))
    total = dict(base)
    for plugin, bounds in (getattr(kernel, "knob_bounds", None) or {}).items():
        for knob, offset in (await offsets(kernel.conn, plugin)).items():
            low, high = (bounds.get(knob) or (-1.0, 1.0))[:2]
            total[knob] = total.get(knob, 0.0) + _clamp(offset, max(-1.0, low),
                                                       min(1.0, high))
    kernel.settings["knobs"] = {
        knob: _clamp(value, 0.0, 1.0) if 0.0 <= base.get(knob, 0.0) <= 1.0
        else max(0.0, value)
        for knob, value in total.items()}
