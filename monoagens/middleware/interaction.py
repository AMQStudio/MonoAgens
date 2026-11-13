"""Interaction: approvals asked and answered, and what the soul shows besides its messages.

The core records everything needed (`job.started`, `think`, `act.out`, `act.result`); this
module decides what is shown, to whom and how much (middleware.md §9). An approval is an
ordinary event pair: `approval.requested` asks, `approval.answered` answers, and a `yes`
wakes the reasoning in the request's own place with an `intent` that carries the approval.
"""

from __future__ import annotations

import json
import pathlib
from dataclasses import replace
from typing import Any

from monoagens.core.hooks import Continue, Middleware
from monoagens.infra.store import append_event
from monoagens.middleware import identity, loader, privacy, registry
from monoagens.types import Event

DEFAULTS = {"thinking": "owner_direct", "steps": "long", "approvals": "owner"}
OWNER = "person:owner"
REQUEST = "approval.requested"
ANSWERED = "approval.answered"
BRAKE_HELD = "brake.held"
BRAKE_RESUMED = "brake.resumed"
YES = ("yes", "ya")


async def request(kernel: Any, trigger: Event, request_id: int, capability: str,
                  arguments: dict[str, Any], reason: str) -> str | None:
    """Ask the owner to allow one call; return the place the question went to, if any."""
    place = await identity.direct_place(kernel.conn, OWNER)
    if place is None:
        return None
    body = await _body_of(kernel.conn, place)
    adapter = kernel.tools.adapters.get(body)
    text = (f"Approval needed: {capability} "
            f"{json.dumps(arguments, ensure_ascii=False, sort_keys=True)}\n"
            f"Reason: {reason}. Reply yes or no.")
    await kernel._emit(trigger, "act.out", {
        "capability": f"{body}.say" if adapter is not None else "",
        "arguments": {"text": text}, "communicative": True}, place=place)
    if adapter is None:
        return None
    if "buttons" in tuple(getattr(adapter, "shows", ()) or ()):
        await adapter.show(place, "buttons", text=text, buttons=[
            ("Yes", f"approval:yes:{request_id}"), ("No", f"approval:no:{request_id}")])
    else:
        await kernel.tools.act(Event(id=None, ts=trigger.ts, kind="act.out", who=OWNER,
                                     place=place, body=body),
                               f"{body}.say", {"text": text})
    return place


async def _body_of(conn: Any, place: str) -> str:
    """Return the adapter that last spoke in one place."""
    row = await (await conn.execute(
        "SELECT body FROM events WHERE place = %s AND body IS NOT NULL "
        "ORDER BY id DESC LIMIT 1", (place,))).fetchone()
    return str(row[0]) if row else ""


async def pending(conn: Any) -> dict[str, Any] | None:
    """Return the newest approval request nobody answered yet, with its payload."""
    row = await (await conn.execute(
        "SELECT id, payload FROM events WHERE kind = %s AND id NOT IN ("
        "  SELECT (payload->>'request')::bigint FROM events WHERE kind = %s"
        "  AND payload ? 'request') ORDER BY id DESC LIMIT 1", (REQUEST, ANSWERED))).fetchone()
    if row is None:
        return None
    return {"id": int(row[0]), **dict(row[1] or {})}


async def by_id(conn: Any, request_id: int) -> dict[str, Any] | None:
    """Return one approval request by the id of its event."""
    row = await (await conn.execute(
        "SELECT payload FROM events WHERE id = %s AND kind = %s", (request_id, REQUEST))
    ).fetchone()
    return dict(row[0] or {}) if row else None


async def pending_brake(conn: Any) -> dict[str, Any] | None:
    """Return the newest held brake nobody answered yet, with its payload."""
    row = await (await conn.execute(
        "SELECT id, payload FROM events WHERE kind = %s AND id NOT IN ("
        "  SELECT (payload->>'brake')::bigint FROM events WHERE kind = %s"
        "  AND payload ? 'brake') ORDER BY id DESC LIMIT 1", (BRAKE_HELD, BRAKE_RESUMED))
    ).fetchone()
    return {"id": int(row[0]), **dict(row[1] or {})} if row is not None else None


async def brake_by_id(conn: Any, brake_id: int) -> dict[str, Any] | None:
    """Return one held brake event by its id."""
    row = await (await conn.execute(
        "SELECT payload FROM events WHERE id = %s AND kind = %s", (brake_id, BRAKE_HELD))
    ).fetchone()
    return dict(row[0] or {}) if row else None


async def notify_brake(kernel: Any, trigger: Event, *, root_id: int, place: str, work: str,
                       reason: str, limit: str, value: int, cap: int) -> Event:
    """Record a held brake and tell the owner once in the owner's direct chat."""
    brake = await kernel._emit(trigger, BRAKE_HELD, {
        "root": root_id, "place": place, "work": work, "reason": reason,
        "limit": limit, "value": value, "cap": cap,
        "place_kind": (trigger.payload or {}).get("place_kind"),
    }, place=place)
    direct = await identity.direct_place(kernel.conn, OWNER)
    if direct is None:
        return brake
    body = await _body_of(kernel.conn, direct)
    adapter = kernel.tools.adapters.get(body)
    text = (f"Stopped work ({work}): {reason}. "
            f"Reply yes to continue, or no to leave it stopped.")
    await kernel._emit(brake, "act.out", {
        "capability": f"{body}.say" if adapter is not None else "",
        "arguments": {"text": text}, "communicative": True}, place=direct)
    if adapter is None:
        return brake
    if "buttons" in tuple(getattr(adapter, "shows", ()) or ()):
        await adapter.show(direct, "buttons", text=text, buttons=[
            ("Continue", f"approval:yes:{brake.id}"), ("Stop", f"approval:no:{brake.id}")])
    else:
        await kernel.tools.act(Event(id=None, ts=trigger.ts, kind="act.out", who=OWNER,
                                     place=direct, body=body),
                               f"{body}.say", {"text": text})
    return brake


async def _answer_brake(kernel: Any, item: Event, brake_id: int, held: dict[str, Any],
                        allowed: bool, answerer: str) -> None:
    """Record a brake decision and enqueue a continuation intent when the owner allows it."""
    work = str(held.get("work") or held.get("place") or "stopped work")
    target = str(held.get("place") or item.place or "")
    await append_event(kernel.conn, ts=kernel.clock.now(), kind=BRAKE_RESUMED,
                       who=OWNER, place=target, body=item.body, caused_by=item.id,
                       payload={"brake": brake_id, "root": held.get("root"),
                                "allowed": allowed, "by": answerer})
    if allowed:
        kernel.enqueue(Event(
            id=None, ts=kernel.clock.now(), kind="intent", who=OWNER,
            place=target, body=item.body,
            payload={"description": f"the owner chose to continue the stopped work: {work}",
                     "root": held.get("root"), "brake": brake_id,
                     "place_kind": held.get("place_kind")}))


def answer(kernel: Any) -> Middleware:
    """Inbound rule: the owner's yes or no answers the newest pending approval or brake."""

    async def rule(item: Any, ctx: Any) -> Any:
        """Turn one answer into `approval.answered` / `brake.resumed`, and yes into an intent."""
        if getattr(item, "kind", "") != "percept":
            return Continue()
        payload = item.payload or {}
        answerer = str(item.origin_actor or item.who)
        owner = answerer == OWNER
        request_id, allowed = None, False
        if payload.get("approval") is not None:
            if not (owner or kernel.roles.can(answerer, "approval.answer")):
                return Continue()
            request_id, allowed = int(payload["approval"]), bool(payload.get("answer"))
        elif payload.get("brake") is not None:
            if not owner:
                return Continue()
            held = await brake_by_id(kernel.conn, int(payload["brake"]))
            if held is not None:
                await _answer_brake(kernel, item, int(payload["brake"]), held,
                                    bool(payload.get("answer")), answerer)
            return Continue()
        else:
            text = str(payload.get("description") or "").strip().lower()
            if owner and text in (*YES, "no"):
                found_approval = await pending(kernel.conn)
                found_brake = await pending_brake(kernel.conn)
                if found_brake is not None and (
                        found_approval is None or found_brake["id"] > found_approval["id"]):
                    await _answer_brake(kernel, item, int(found_brake["id"]), found_brake,
                                        text in YES, answerer)
                    return Continue()
                if found_approval is None:
                    return Continue()
                request_id, allowed = int(found_approval["id"]), text in YES
        if request_id is None:
            return Continue()
        asked = await by_id(kernel.conn, request_id)
        if asked is None:
            held = await brake_by_id(kernel.conn, request_id) if owner else None
            if held is not None:
                await _answer_brake(kernel, item, request_id, held, allowed, answerer)
            return Continue()
        await append_event(kernel.conn, ts=kernel.clock.now(), kind=ANSWERED,
                           who=asked.get("who"), place=asked.get("place"), body=item.body,
                           caused_by=item.id,
                           payload={"request": request_id, "allowed": allowed,
                                    "who": asked.get("who"), "by": answerer})
        if allowed:
            kernel.enqueue(Event(
                id=None, ts=kernel.clock.now(), kind="intent", who=OWNER,
                place=str(asked.get("place") or item.place), body=item.body,
                payload={
                    "description": f"the owner approved {asked.get('capability')} with these "
                                   f"arguments; carry it out now: "
                                   f"{json.dumps(asked.get('arguments') or {}, ensure_ascii=False, sort_keys=True)}",
                    "capability": asked.get("capability"),
                    "arguments": asked.get("arguments") or {},
                    "intent": request_id, "approved": request_id,
                    "place_kind": asked.get("place_kind")}))
        return Continue()

    return rule


# ------------------------------------------------------ what the soul shows

SHOWN = "interaction.shown"
STEP_CHARS = 200


def switches(kernel: Any) -> dict[str, str]:
    """Return the interaction switches: the defaults, overridden by `config.yaml`."""
    return {**DEFAULTS, **(kernel.settings.get("interaction") or {})}


def shows(adapter: Any) -> tuple[str, ...]:
    """Return what one adapter declared it can show, in the contract's vocabulary."""
    return tuple(getattr(adapter, "shows", ()) or ())


async def present(kernel: Any, event: Event, kind: str, text: str) -> bool:
    """Show one line where the adapter can, with the fallback the contract names."""
    adapter = kernel.tools.adapters.get(str(event.body))
    if adapter is None:
        return False
    if kind in shows(adapter):
        await adapter.show(str(event.place), kind, text=text)
        return True
    if kind == "typing":
        return False  # nothing to show, and nothing lost
    await kernel.tools.act(event, f"{event.body}.say", {"text": text})
    return True


async def _last_shown(conn: Any, place: str) -> int:
    """Return the id of the last event of this place that reasoning already showed."""
    row = await (await conn.execute(
        "SELECT max((payload->>'note')::bigint) FROM events "
        "WHERE kind = %s AND place = %s", (SHOWN, place))).fetchone()
    return int((row or (0,))[0] or 0)


async def _notes(conn: Any, chain: int, after: int) -> list[tuple[int, str, str]]:
    """Return the thoughts of one chain that were not shown yet, oldest first."""
    cur = await conn.execute(
        "SELECT id, place, payload FROM events WHERE kind = 'decision.note' "
        "AND caused_by = %s AND id > %s AND payload->>'act' = 'think' ORDER BY id",
        (chain, after))
    return [(int(row[0]), str(row[1] or ""), str((row[2] or {}).get("thought") or ""))
            for row in await cur.fetchall()]


def narrate(kernel: Any) -> Middleware:
    """Inbound rule: show the owner a thought, and a step of work that takes long."""

    async def rule(item: Any, ctx: Any) -> Any:
        """Show what the switches allow, only after the audience check."""
        place = str(item.place or "")
        if not place:
            return Continue()
        settings_now = switches(kernel)
        chain = int(item.caused_by or item.id or 0)
        thoughts = await _notes(kernel.conn, chain, 0) if settings_now.get("thinking") != "off" \
            else []
        if thoughts and await privacy.owner_direct(kernel.conn, place):
            shown = await _last_shown(kernel.conn, place)
            for note, note_place, thought in [note for note in thoughts if note[0] > shown]:
                if not thought or not await privacy.visible(kernel.conn, context=place,
                                                            source=note_place, before=note):
                    continue
                if await present(kernel, replace(item, place=place), "edit",
                                 f"(thinking) {thought}"):
                    await kernel._emit(item, SHOWN, {"place": place, "kind": "thinking",
                                                     "note": note}, place=place)
        if settings_now.get("steps") != "off" and item.kind == "act.result":
            capability = str((item.payload or {}).get("capability") or "")
            declared = kernel.tools.get(capability) or {}
            if declared.get("long_running"):
                short = _short(interaction_text((item.payload or {}).get("result")))
                await present(kernel, item, "edit", f"(step) {capability}: {short}")
        return Continue()

    return rule


def interaction_text(result: Any) -> str:
    """Return the text of one tool result, whichever key carries it."""
    if isinstance(result, str):
        return result
    if isinstance(result, dict):
        for key in ("text", "content", "output", "stdout", "error"):
            if isinstance(result.get(key), str):
                return str(result[key])
        return json.dumps(result, ensure_ascii=False)
    return "" if result is None else str(result)


def _short(text: str) -> str:
    """Return the first characters of one step's result."""
    return text if len(text) <= STEP_CHARS else text[:STEP_CHARS] + "…"


def typing(kernel: Any) -> Middleware:
    """Action rule (first pass): say "typing…" where the act is work or a message."""

    async def rule(item: Any, ctx: Any) -> Any:
        """Show typing for a message or a long action, if the adapter can show it."""
        if ctx.get("refusal") or ctx.get("handled"):
            return Continue()
        descriptor = ctx.get("descriptor") or {}
        if not (descriptor.get("communicative") or ctx.get("long_running")):
            return Continue()
        adapter = kernel.tools.adapters.get(str(ctx["event"].body))
        if adapter is None or "typing" not in shows(adapter):
            return Continue()
        await adapter.show(str(ctx["event"].place), "typing")
        return Continue()

    return rule


def install_commands(kernel: Any) -> None:
    """Register `/show`: the owner switches what the soul shows besides its messages."""
    if kernel.tools.command("show") is None:
        kernel.tools.add_command(registry.CommandEntry(
            "show", "Switch thinking, steps or approvals: /show <name> <on|off>.",
            "interaction.show", show_switch, owner="interaction"))


async def show_switch(kernel: Any, event: Event, arguments: list[str]) -> str:
    """Write one `interaction:` line in config.yaml and apply it at once."""
    usage = "usage: /show thinking|steps|approvals <on|off>"
    if len(arguments) < 2:
        return usage
    what, state = arguments[0].lower(), arguments[1].lower()
    if what not in DEFAULTS or state not in ("on", "off"):
        return usage
    before = switches(kernel).get(what)
    after = DEFAULTS[what] if state == "on" else "off"
    path = _config_path(kernel)
    if path is not None:
        loader.edit_config(path, lambda text: loader.set_block_entry(
            text, "interaction", what, after), now=kernel.clock.now())
        await loader.record_config_change(kernel, event, plugin="interaction",
                                          key=f"interaction.{what}", before=before,
                                          after=after)
    kernel.settings.setdefault("interaction", {})[what] = after
    return f"{what} is now {after}"


def _config_path(kernel: Any) -> Any:
    """Return the config.yaml a running kernel writes; a test kernel may have none."""
    root = getattr(kernel, "deployment_root", None)
    if root is None:
        return None
    return pathlib.Path(root) / "config.yaml"


# -------------------------------------------------------------- attachments

SPLIT = "interaction.split"


def split_attachments(adapter: Any, arguments: dict[str, Any]) -> list[dict[str, Any]]:
    """Return one argument dict per message the adapter can send."""
    limit = int((getattr(adapter, "FILES", {}) or {}).get("per_message") or 0) or 1
    found = [str(ref) for ref in arguments.get("attachments") or ()]
    total = -(-len(found) // limit)
    parts: list[dict[str, Any]] = []
    for index in range(total):
        chunk = found[index * limit:(index + 1) * limit]
        part: dict[str, Any] = {**{key: value for key, value in arguments.items()
                                   if key != "attachments"}, "attachments": chunk}
        if index == 0:
            if arguments.get("text"):
                part["text"] = arguments["text"]
        else:
            part["text"] = f"(part {index + 1}/{total})"
        parts.append(part)
    return parts


def split(kernel: Any) -> Middleware:
    """Action rule: send the files in as many messages as the adapter needs."""

    async def rule(item: Any, ctx: Any) -> Any:
        """Record one `act.out` per part and hand each part to the adapter."""
        if ctx.get("refusal") or ctx.get("handled"):
            return Continue()
        descriptor = ctx.get("descriptor") or {}
        arguments = dict(item or {})
        attachments = list(arguments.get("attachments") or ())
        adapter = kernel.tools.adapters.get(str(ctx["event"].body))
        if adapter is None or not descriptor.get("communicative") or not attachments:
            return Continue()
        limit = int((getattr(adapter, "FILES", {}) or {}).get("per_message") or 0)
        if not limit or len(attachments) <= limit:
            return Continue()
        parts = split_attachments(adapter, arguments)
        await kernel._emit(ctx["event"], SPLIT, {
            "capability": ctx["capability"], "parts": len(parts),
            "attachments": len(attachments)})
        for part in parts:
            passed = {key: value for key, value in part.items() if key not in registry.ROUTING}
            await kernel._emit(ctx["event"], "act.out", {
                "capability": ctx["capability"], "arguments": passed,
                "communicative": True}, place=ctx["event"].place)
            await kernel.tools.act(ctx["event"], str(ctx["capability"]), passed)
        ctx["handled"] = True
        return Continue()

    return rule
