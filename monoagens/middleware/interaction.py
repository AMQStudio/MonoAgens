"""Interaction: approvals, D70 switches, fallbacks, read receipts, typing, thinking & progress."""

from __future__ import annotations

import json
import pathlib
import re
from collections.abc import Mapping
from dataclasses import replace
from typing import Any

import yaml

from monoagens.core.hooks import Continue, Middleware, Replace
from monoagens.infra.store import append_event, read_state
from monoagens.middleware import contract, identity, loader, privacy, registry
from monoagens.types import Event

DEFAULTS: dict[str, str] = {
    "thinking": "owner_direct",
    "steps": "long",
    "approvals": "owner",
    "typing": "on",
    "read_receipts": "on",
    "progress": "edit",
    "stickers": "on",
}
ALLOWED_SHOW: dict[str, tuple[str, ...]] = {
    "thinking": ("off", "owner_direct", "all"),
    "steps": ("off", "long", "all"),
    "approvals": ("owner", "off"),
    "typing": ("on", "off"),
    "read_receipts": ("on", "off"),
    "progress": ("edit", "new"),
    "stickers": ("on", "off"),
}
CAPABILITY_REQUIRED: dict[tuple[str, str], str] = {
    ("typing", "on"): "typing",
    ("read_receipts", "on"): "read_receipts",
    ("progress", "edit"): "edit",
    ("stickers", "on"): "stickers",
}
FALLBACKS = contract.FALLBACKS
OWNER = "person:owner"
REQUEST = "approval.requested"
ANSWERED = "approval.answered"
BRAKE_HELD = "brake.held"
BRAKE_RESUMED = "brake.resumed"
YES = ("yes", "ya", "👍")
NO = ("no", "tidak", "👎")
SHOWN = "interaction.shown"
SPLIT = "interaction.split"
STEP_CHARS = 200


async def _ask_prompt(kernel: Any, body: str, place: str, text: str, request_id: int,
                      *, prefix: str = "approval",
                      yes_label: str = "Yes", no_label: str = "No") -> None:
    """Deliver an approval/continue prompt via buttons -> reactions -> text (D70)."""
    adapter = kernel.tools.adapters.get(body)
    if adapter is None:
        return
    caps = shows(adapter)
    if "buttons" in caps:
        await adapter.show(place, "buttons", text=text, buttons=[
            (yes_label, f"{prefix}:yes:{request_id}"),
            (no_label, f"{prefix}:no:{request_id}"),
        ])
        return
    trigger = Event(id=None, ts=kernel.clock.now(), kind="act.out", who=OWNER,
                    place=place, body=body)
    result = await kernel.tools.act(trigger, f"{body}.say", {"text": text})
    msg_id = str(result.get("message_id") or "") if isinstance(result, dict) else ""
    if "reactions" in caps and msg_id:
        for emoji in ("👍", "👎"):
            await adapter.show(place, "reactions", message_id=msg_id, emoji=emoji)
        prompts = kernel.__dict__.setdefault("_approval_reaction_prompts", {})
        prompts[(place, msg_id)] = (prefix, request_id)


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
    await _ask_prompt(kernel, body, place, text, request_id, prefix="approval")
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
            "Reply yes to continue, or no to leave it stopped.")
    await kernel._emit(brake, "act.out", {
        "capability": f"{body}.say" if adapter is not None else "",
        "arguments": {"text": text}, "communicative": True}, place=direct)
    if adapter is None:
        return brake
    assert brake.id is not None
    await _ask_prompt(kernel, body, direct, text, int(brake.id),
                      prefix="brake", yes_label="Continue", no_label="Stop")
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


def _match_reaction_approval(kernel: Any, item: Event) -> tuple[str, int, bool] | None:
    data = (item.payload or {}).get("data") or {}
    react = data.get("reaction") or {}
    if not isinstance(react, Mapping):
        return None
    emoji = str(react.get("emoji") or "").strip()
    if emoji not in ("👍", "👎"):
        return None
    prompts = kernel.__dict__.get("_approval_reaction_prompts") or {}
    target_id = str(react.get("message_id") or "")
    match = prompts.get((str(item.place or ""), target_id))
    if match is None:
        return None
    prefix, req_id = match
    return prefix, req_id, emoji == "👍"


def answer(kernel: Any) -> Middleware:
    """Inbound rule: owner yes/no, button click or reaction answers pending approval/brake."""

    async def rule(item: Any, ctx: Any) -> Any:
        """Turn one answer into `approval.answered` / `brake.resumed`, and yes into an intent."""
        del ctx
        if getattr(item, "kind", "") != "percept":
            return Continue()
        payload = item.payload or {}
        answerer = str(item.origin_actor or item.who)
        owner = answerer == OWNER
        request_id, allowed = None, False
        if payload.get("approval") is not None:
            if not (owner or kernel.roles.can(answerer, "approval.answer", item.place)):
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
        elif (rmatch := _match_reaction_approval(kernel, item)) is not None:
            prefix, request_id, allowed = rmatch
            if not (owner or (prefix == "approval"
                              and kernel.roles.can(answerer, "approval.answer", item.place))):
                return Continue()
            if prefix == "brake":
                held = await brake_by_id(kernel.conn, request_id)
                if held is not None:
                    await _answer_brake(kernel, item, request_id, held, allowed, answerer)
                return Continue()
        else:
            text = str(payload.get("description") or "").strip().lower()
            if owner and text in (*YES, *NO):
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
                    "description": (
                        f"the owner approved {asked.get('capability')} with these arguments; "
                        f"carry it out now: {json.dumps(asked.get('arguments') or {}, ensure_ascii=False, sort_keys=True)}"
                    ),
                    "capability": asked.get("capability"),
                    "arguments": asked.get("arguments") or {},
                    "intent": request_id, "approved": request_id,
                    "place_kind": asked.get("place_kind"),
                },
            ))
        return Continue()

    return rule


# ------------------------------------------------------ what the soul shows


def switches(kernel: Any, *, body: str | None = None) -> dict[str, str]:
    """Return effective interaction switches: defaults -> `interaction:` -> `bodies.<body>.show:`."""
    merged = {**DEFAULTS, **{str(k): str(v) for k, v in (kernel.settings.get("interaction") or {}).items()}}
    if body:
        body_cfg = (kernel.settings.get("bodies") or {}).get(body) or {}
        per_body = body_cfg.get("show") or {}
        for k, v in per_body.items():
            merged[str(k)] = _normalize_bool_str(str(v))
    return {k: _normalize_bool_str(str(v)) for k, v in merged.items()}


def _normalize_bool_str(val: str) -> str:
    low = val.strip().lower()
    if low == "true":
        return "on"
    if low == "false":
        return "off"
    return low


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
        return False
    await kernel.tools.act(event, f"{event.body}.say", {"text": text})
    return True


async def _present_step(kernel: Any, event: Event, line: str, *, chain: int, place: str) -> None:
    """Show a progress step either by editing one message in place or sending a new message."""
    adapter = kernel.tools.adapters.get(str(event.body))
    if adapter is None:
        return
    mode = switches(kernel, body=str(event.body)).get("progress", "edit")
    if mode == "edit" and "edit" in shows(adapter):
        progress = kernel.__dict__.setdefault("_progress_messages", {})
        key = (str(event.body), place, chain)
        mid = progress.get(key)
        kwargs: dict[str, Any] = {"text": line}
        if mid:
            kwargs["message_id"] = mid
        res = await adapter.show(place, "edit", **kwargs)
        if isinstance(res, dict) and res.get("message_id"):
            progress[key] = res["message_id"]
        return
    await kernel.tools.act(event, f"{event.body}.say", {"text": line})


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
    """Inbound rule: show allowed thoughts (`off|owner_direct|all`) and steps (`edit|new`)."""

    async def rule(item: Any, ctx: Any) -> Any:
        """Show what the switches allow, only after the `privacy.visible` audience check."""
        del ctx
        place = str(item.place or "")
        if not place:
            return Continue()
        settings_now = switches(kernel, body=str(item.body or ""))
        think_mode = settings_now.get("thinking", "owner_direct")
        chain = int(item.caused_by or item.id or 0)
        thoughts = await _notes(kernel.conn, chain, 0) if think_mode != "off" else []
        eligible = think_mode == "all" or (
            think_mode == "owner_direct" and await privacy.owner_direct(kernel.conn, place)
        )
        if thoughts and eligible:
            shown = await _last_shown(kernel.conn, place)
            for note, note_place, thought in [n for n in thoughts if n[0] > shown]:
                if not thought or not await privacy.visible(kernel.conn, context=place,
                                                            source=note_place, before=note):
                    continue
                if await present(kernel, replace(item, place=place), "edit",
                                 f"(thinking) {thought}"):
                    await kernel._emit(item, SHOWN, {"place": place, "kind": "thinking",
                                                     "note": note}, place=place)
        steps_mode = settings_now.get("steps", "long")
        if steps_mode != "off" and item.kind == "act.result":
            capability = str((item.payload or {}).get("capability") or "")
            declared = kernel.tools.get(capability) or {}
            if steps_mode == "all" or declared.get("long_running"):
                short = _short(interaction_text((item.payload or {}).get("result")))
                await _present_step(kernel, item, f"(step) {capability}: {short}",
                                    chain=chain, place=place)
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
    """Action rule: show typing ONLY when the soul is going to speak (`communicative`)."""

    async def rule(item: Any, ctx: Any) -> Any:
        """Show typing for a communicative act when enabled and supported; silent/tools skip."""
        del item
        if ctx.get("refusal") or ctx.get("handled"):
            return Continue()
        descriptor = ctx.get("descriptor") or {}
        if not descriptor.get("communicative"):
            return Continue()
        body = str(ctx["event"].body or "")
        if switches(kernel, body=body).get("typing", "on") == "off":
            return Continue()
        adapter = kernel.tools.adapters.get(body)
        if adapter is None or "typing" not in shows(adapter):
            return Continue()
        await adapter.show(str(ctx["event"].place), "typing")
        return Continue()

    return rule


async def _is_shared_with_owner(conn: Any, body: str) -> bool:
    acc = (await read_state(conn, f"self_account:{body}") or {}).get("account")
    if not acc:
        return False
    alias = await read_state(conn, f"alias:{acc}")
    return bool(alias and alias.get("person") == OWNER)


async def _soul_spoke_since_message(conn: Any, place: str, event_id: int) -> bool:
    """True when the soul sent a communicative act in `place` after `event_id`."""
    if event_id <= 0:
        return False
    row_soul = await (await conn.execute(
        "SELECT COALESCE(MAX(id), 0) FROM events WHERE kind = 'act.out' AND place = %s "
        "AND COALESCE((payload->>'communicative')::boolean, false)",
        (place,),
    )).fetchone()
    last_soul = int((row_soul or (0,))[0] or 0)
    return last_soul > event_id


async def mark_read_for(kernel: Any, trigger: Event) -> bool:
    """Mark one percept read after its decision is recorded, respecting D69 & D66."""
    if trigger is None or trigger.kind != "percept":
        return False
    payload = trigger.payload or {}
    if payload.get("imported") is True or payload.get("backfill") is True:
        return False
    place = str(trigger.place or "")
    body = str(trigger.body or "")
    if not place or not body:
        return False
    if switches(kernel, body=body).get("read_receipts", "on") == "off":
        return False
    adapter = kernel.tools.adapters.get(body)
    if adapter is None or "read_receipts" not in shows(adapter):
        return False
    msg_id = (payload.get("data") or {}).get("message_id")
    if not msg_id:
        return False
    marked = kernel.__dict__.setdefault("_read_marked", set())
    dedup_key = (body, place, str(msg_id))
    if dedup_key in marked:
        return False
    if await _is_shared_with_owner(kernel.conn, body):
        if str(trigger.origin_actor or trigger.who) == OWNER:
            return False
        if not await _soul_spoke_since_message(kernel.conn, place, int(trigger.id or 0)):
            return False
    marked.add(dedup_key)
    await adapter.show(place, "read_receipts", message_id=msg_id, message_ids=[msg_id])
    return True


def install_commands(kernel: Any) -> None:
    """Register `/show` and hook decision emission for D70 read receipts."""
    if kernel.tools.command("show") is None:
        kernel.tools.add_command(registry.CommandEntry(
            "show",
            "Switch interaction display: /show <name> <value> [adapter].",
            "interaction.show", show_switch, owner="interaction"))
    if not kernel.__dict__.get("_interaction_emit_wrapped") and hasattr(kernel, "_emit"):
        orig_emit = kernel._emit

        async def _emit_with_receipt(trigger: Event, kind: str, payload: dict[str, Any], *,
                                     place: str | None = None) -> Event:
            stored = await orig_emit(trigger, kind, payload, place=place)
            if kind.startswith("decision.") or (kind == "act.out" and payload.get("communicative")):
                await mark_read_for(kernel, trigger)
            return stored

        kernel.__dict__["_interaction_emit_wrapped"] = True
        kernel._emit = _emit_with_receipt


def _set_body_show_entry(text: str, adapter_name: str, key: str, value: str) -> str:
    """Update `bodies.<adapter>.show.<key>: <value>` in config.yaml cleanly."""
    doc = yaml.safe_load(text) if text.strip() else {}
    if not isinstance(doc, dict):
        doc = {}
    bodies = doc.setdefault("bodies", {})
    adapter_cfg = bodies.setdefault(adapter_name, {})
    show_cfg = adapter_cfg.setdefault("show", {})
    show_cfg[key] = value
    return yaml.safe_dump(doc, sort_keys=False, allow_unicode=True)


def _resolve_show_value(what: str, raw_val: str) -> str | None:
    low = raw_val.strip().lower()
    if low == "on" and what in ("thinking", "steps"):
        return DEFAULTS[what]
    allowed = ALLOWED_SHOW.get(what, ())
    return low if low in allowed else None


async def show_switch(kernel: Any, event: Event, arguments: list[str]) -> str:
    """Write `interaction:` or `bodies.<adapter>.show:` in config.yaml and apply at once."""
    usage = "usage: /show <thinking|steps|approvals|typing|read_receipts|progress|stickers> <value> [adapter]"
    if len(arguments) < 2:
        return usage
    what = arguments[0].lower()
    if what not in DEFAULTS:
        return usage
    after = _resolve_show_value(what, arguments[1])
    if after is None:
        return f"invalid value {arguments[1]!r} for {what}; expected {', '.join(ALLOWED_SHOW[what])}"
    adapter_name = arguments[2].lower() if len(arguments) > 2 else ""

    if adapter_name:
        adapter = kernel.tools.adapters.get(adapter_name)
        if adapter is None:
            return f"refused: unknown adapter {adapter_name!r}"
        needed = CAPABILITY_REQUIRED.get((what, after))
        if needed and needed not in shows(adapter):
            reason = (getattr(adapter, "gaps", {}) or {}).get(needed) or f"{adapter_name} cannot show {needed}"
            return f"refused for {adapter_name}: {reason}"
        before = switches(kernel, body=adapter_name).get(what)
        path = _config_path(kernel)
        if path is not None:
            loader.edit_config(
                path, lambda text: _set_body_show_entry(text, adapter_name, what, after),
                now=kernel.clock.now())
            await loader.record_config_change(
                kernel, event, plugin="interaction",
                key=f"bodies.{adapter_name}.show.{what}", before=before, after=after)
        kernel.settings.setdefault("bodies", {}).setdefault(adapter_name, {}).setdefault(
            "show", {})[what] = after
        return f"{adapter_name}.{what} is now {after}"

    before = switches(kernel).get(what)
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


# -------------------------------------------------------------- attachments & fallbacks


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
    """Action rule: handle sticker fallback (`stickers: off`) and split multi-file sends."""

    async def rule(item: Any, ctx: Any) -> Any:
        """Rewrite sticker fallback to emoji when off, or split large attachment lists."""
        if ctx.get("refusal") or ctx.get("handled"):
            return Continue()
        descriptor = ctx.get("descriptor") or {}
        arguments = dict(item or {})
        body = str(ctx["event"].body or "")
        adapter = kernel.tools.adapters.get(body)
        if adapter is None or not descriptor.get("communicative"):
            return Continue()

        # Sticker fallback: stickers: off (or unsupported) -> emoji text
        sticker_mode = switches(kernel, body=body).get("stickers", "on")
        if arguments.get("sticker") and (sticker_mode == "off" or "stickers" not in shows(adapter)):
            emoji = str(arguments.pop("sticker_emoji", "") or "🏷️")
            arguments.pop("sticker", None)
            existing = str(arguments.get("text") or "").strip()
            arguments["text"] = f"{existing} {emoji}".strip() if existing else emoji
            if not arguments.get("attachments"):
                return Replace(arguments)

        attachments = list(arguments.get("attachments") or ())
        if not attachments:
            return Replace(arguments) if arguments != dict(item or {}) else Continue()
        limit = int((getattr(adapter, "FILES", {}) or {}).get("per_message") or 0)
        if not limit or len(attachments) <= limit:
            return Replace(arguments) if arguments != dict(item or {}) else Continue()
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
