"""Interaction: approvals, D70 switches, fallbacks, read receipts, typing, thinking & progress."""

from __future__ import annotations

import json
import pathlib
import re
from collections.abc import Mapping
from dataclasses import replace
from datetime import datetime, timedelta
from functools import lru_cache
from typing import Any

import yaml

from monoagens.core.hooks import Continue, Middleware, Replace
from monoagens.infra.store import append_event, read_state
from monoagens.middleware import contract, identity, loader, privacy, registry
from monoagens.types import Event

_LANG_DIR = pathlib.Path(__file__).resolve().parent / "lang"

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
OWNER = "person:owner"
REQUEST = "approval.requested"
ANSWERED = "approval.answered"
CONSUMED = "approval.consumed"
EXPIRED = "approval.expired"
APPROVAL_TTL = timedelta(minutes=30)
BRAKE_HELD = "brake.held"
BRAKE_RESUMED = "brake.resumed"
SHOWN = "interaction.shown"
SPLIT = "interaction.split"
STEP_CHARS = 200


@lru_cache(maxsize=16)
def _load_lang(code: str) -> dict[str, Any]:
    """Load one message catalog file from `lang/<code>.yaml`, with `en.yaml` fallback."""
    en_path = _LANG_DIR / "en.yaml"
    base = yaml.safe_load(en_path.read_text(encoding="utf-8")) or {}
    clean = str(code or "en").strip().lower().replace("_", "-").split("-", 1)[0]
    path = _LANG_DIR / f"{clean}.yaml"
    if not path.exists() or path == en_path:
        return dict(base)
    over = yaml.safe_load(path.read_text(encoding="utf-8")) or {}
    merged = {key: (dict(val) if isinstance(val, Mapping) else val) for key, val in base.items()}
    for key, val in over.items():
        if isinstance(val, Mapping) and isinstance(merged.get(key), dict):
            merged[key] = {**merged[key], **val}
        else:
            merged[key] = val
    return merged


def catalog(settings: Mapping[str, Any] | None = None) -> dict[str, Any]:
    """Return the message catalog in the soul's configured language."""
    ident = (settings or {}).get("identity") or {}
    code = ident.get("language") if isinstance(ident, Mapping) else "en"
    return _load_lang(str(code or "en"))


def _normalize_choices(raw: Any, labels: list[str]) -> list[dict[str, str]]:
    """Return `[{label, answer}, ...]` from an adapter's `act.result` or ordinal fallback."""
    if isinstance(raw, list) and len(raw) >= len(labels):
        parsed = [{"label": str(item.get("label") or ""), "answer": str(item.get("answer") or "")}
                  for item in raw[:len(labels)] if isinstance(item, Mapping)]
        if len(parsed) == len(labels) and all(c["answer"] for c in parsed):
            return parsed
    return [{"label": label, "answer": str(index + 1)} for index, label in enumerate(labels)]


async def _send_prompt(kernel: Any, trigger: Event, place: str, text: str, *,
                       prompt_kind: str, prompt_id: int,
                       choices: list[str]) -> None:
    """Deliver a neutral `choices` prompt via `say` and record its returned tokens in the Log."""
    body = await _body_of(kernel.conn, place)
    adapter = kernel.tools.adapters.get(body)
    args: dict[str, Any] = {"text": text, "choices": list(choices)}
    act_out = await kernel._emit(trigger, "act.out", {
        "capability": f"{body}.say" if adapter is not None else "",
        "arguments": args, "communicative": True}, place=place)
    result: Any = None
    if adapter is not None:
        out_event = Event(id=None, ts=kernel.clock.now(), kind="act.out", who=OWNER,
                          place=place, body=body, caused_by=act_out.id)
        receipt = await kernel.tools.act(registry.own_action(out_event, f"{body}.say", args))
        result = receipt.result if receipt.state == "done" else None
        if result is not None:
            await kernel._emit(act_out, "act.result", {
                "capability": f"{body}.say", "result": result}, place=place)
    msg_id = str(result.get("message_id") or "") if isinstance(result, Mapping) else ""
    recorded = _normalize_choices(result.get("choices") if isinstance(result, Mapping) else None,
                                  choices)
    await kernel._emit(act_out, SHOWN, {
        "kind": "prompt", "prompt_kind": prompt_kind, "prompt_id": prompt_id,
        "message_id": msg_id, "choices": recorded,
        "positive": choices[0], "negative": choices[1],
    }, place=place)


def _requester_name(kernel: Any, trigger: Event) -> str:
    """Return a readable requester label (`Label (id)` or `id`) for approval prompts."""
    who = str(trigger.origin_actor or trigger.who or "")
    roles = getattr(kernel, "roles", None)
    label = roles.label(who) if who and roles is not None else None
    if label and label != who:
        return f"{label} ({who})"
    return who or "unknown"


async def request(kernel: Any, trigger: Event, request_id: int, capability: str,
                  arguments: dict[str, Any], reason: str) -> str | None:
    """Ask the owner to allow one call via neutral `choices`; return the place asked, if any."""
    place = await identity.direct_place(kernel.conn, OWNER)
    if place is None:
        return None
    cat = catalog(getattr(kernel, "settings", None))["approval"]
    args_json = json.dumps(arguments, ensure_ascii=False, sort_keys=True)
    text = str(cat["prompt"]).format(
        capability=capability, arguments=args_json,
        requester=_requester_name(kernel, trigger), reason=reason)
    choices = [str(cat["yes_label"]), str(cat["no_label"])]
    await _send_prompt(kernel, trigger, place, text, prompt_kind="approval",
                       prompt_id=request_id, choices=choices)
    return place


async def _body_of(conn: Any, place: str) -> str:
    """Return the adapter that last spoke in one place."""
    row = await (await conn.execute(
        "SELECT body FROM events WHERE place = %s AND body IS NOT NULL "
        "ORDER BY id DESC LIMIT 1", (place,))).fetchone()
    return str(row[0]) if row else ""


async def is_consumed(conn: Any, request_id: int) -> bool:
    """Return whether an approved request has already executed its single allowed act."""
    row = await (await conn.execute(
        "SELECT 1 FROM events WHERE kind = %s AND payload->>'request' = %s LIMIT 1",
        (CONSUMED, str(request_id)))).fetchone()
    return row is not None


async def _is_closed(conn: Any, request_id: int) -> bool:
    """Return whether `request_id` has already been answered or expired."""
    row = await (await conn.execute(
        "SELECT 1 FROM events WHERE kind IN (%s, %s) AND payload->>'request' = %s LIMIT 1",
        (ANSWERED, EXPIRED, str(request_id)))).fetchone()
    return row is not None


async def expire_stale(conn: Any, *, now: datetime, trigger: Event | None = None) -> int:
    """Mark unanswered approval requests older than `APPROVAL_TTL` as `approval.expired`."""
    cutoff = now - APPROVAL_TTL
    cur = await conn.execute(
        "SELECT id, ts, who, place, body, origin_actor, payload FROM events "
        "WHERE kind = %s AND ts < %s AND id NOT IN ("
        "  SELECT (payload->>'request')::bigint FROM events WHERE kind IN (%s, %s)"
        "  AND payload ? 'request') ORDER BY id ASC",
        (REQUEST, cutoff, ANSWERED, EXPIRED))
    rows = await cur.fetchall()
    for req_id, _ts, who, place, body, origin_actor, payload in rows:
        data = dict(payload or {})
        await append_event(
            conn, ts=now, kind=EXPIRED, who=who, place=place, body=body,
            origin_actor=origin_actor or data.get("who") or who,
            caused_by=trigger.id if trigger is not None else int(req_id),
            payload={"request": int(req_id),
                     "who": data.get("who") or origin_actor or who,
                     "capability": data.get("capability"),
                     "reason": "expired"})
    return len(rows)


async def pending(conn: Any, *, now: datetime | None = None) -> dict[str, Any] | None:
    """Return the newest `approval.requested` only if it is open and within `APPROVAL_TTL`."""
    row = await (await conn.execute(
        "SELECT id, ts, who, origin_actor, payload FROM events WHERE kind = %s "
        "ORDER BY id DESC LIMIT 1", (REQUEST,))).fetchone()
    if row is None:
        return None
    req_id, ts, who, origin_actor, payload = row
    if await _is_closed(conn, int(req_id)):
        return None
    if now is not None and now - ts > APPROVAL_TTL:
        return None
    data = dict(payload or {})
    return {"id": int(req_id), "ts": ts, **data,
            "who": data.get("who") or origin_actor or who}


async def by_id(conn: Any, request_id: int) -> dict[str, Any] | None:
    """Return one approval request by the id of its event."""
    row = await (await conn.execute(
        "SELECT ts, who, origin_actor, payload FROM events WHERE id = %s AND kind = %s",
        (request_id, REQUEST))).fetchone()
    if row is None:
        return None
    ts, who, origin_actor, payload = row
    data = dict(payload or {})
    return {"ts": ts, **data, "who": data.get("who") or origin_actor or who}


async def pending_brake(conn: Any) -> dict[str, Any] | None:
    """Return the newest held brake nobody answered yet, with its payload."""
    row = await (await conn.execute(
        "SELECT id, who, origin_actor, payload FROM events WHERE kind = %s AND id NOT IN ("
        "  SELECT (payload->>'brake')::bigint FROM events WHERE kind = %s"
        "  AND payload ? 'brake') ORDER BY id DESC LIMIT 1", (BRAKE_HELD, BRAKE_RESUMED))
    ).fetchone()
    if row is None:
        return None
    bid, who, origin_actor, payload = row
    data = dict(payload or {})
    if not data.get("origin_actor") and (origin_actor or who):
        data["origin_actor"] = origin_actor or who
    return {"id": int(bid), **data}


async def brake_by_id(conn: Any, brake_id: int) -> dict[str, Any] | None:
    """Return one held brake event by its id."""
    row = await (await conn.execute(
        "SELECT who, origin_actor, payload FROM events WHERE id = %s AND kind = %s",
        (brake_id, BRAKE_HELD))).fetchone()
    if row is None:
        return None
    who, origin_actor, payload = row
    data = dict(payload or {})
    if not data.get("origin_actor") and (origin_actor or who):
        data["origin_actor"] = origin_actor or who
    return data


async def _brake_origin(conn: Any, held: Mapping[str, Any]) -> str | None:
    """Return the held chain's `origin_actor` (from `held` or its root event), never owner."""
    origin = str(held.get("origin_actor") or "").strip()
    if origin:
        return origin
    root_id = held.get("root")
    if root_id is not None:
        row = await (await conn.execute(
            "SELECT origin_actor, who FROM events WHERE id = %s", (int(root_id),))).fetchone()
        if row is not None:
            resolved = str(row[0] or row[1] or "").strip()
            if resolved:
                return resolved
    return None


async def _prompt_by_reply(conn: Any, place: str, message_id: str,
                           *, now: datetime | None = None) -> dict[str, Any] | None:
    """Find the newest open approval or brake prompt rendered with `message_id` in `place`."""
    cur = await conn.execute(
        "SELECT payload FROM events WHERE kind = %s AND place = %s "
        "AND payload->>'kind' = 'prompt' AND payload->>'message_id' = %s "
        "ORDER BY id DESC LIMIT 5",
        (SHOWN, place, message_id))
    for row in await cur.fetchall():
        info = dict(row[0] or {})
        kind = str(info.get("prompt_kind") or "")
        prompt_id = int(info.get("prompt_id") or 0)
        if not prompt_id:
            continue
        if kind == "approval":
            asked = await by_id(conn, prompt_id)
            if asked is not None and not await _is_closed(conn, prompt_id):
                if now is None or now - asked["ts"] <= APPROVAL_TTL:
                    return info
        elif kind == "brake":
            row_ans = await (await conn.execute(
                "SELECT 1 FROM events WHERE kind = %s AND payload->>'brake' = %s LIMIT 1",
                (BRAKE_RESUMED, str(prompt_id)))).fetchone()
            if row_ans is None and await brake_by_id(conn, prompt_id) is not None:
                return info
    return None


def _match_choice_token(prompt: Mapping[str, Any], token: Any,
                        text: str, settings: Mapping[str, Any] | None) -> bool | None:
    """Match an opaque `answer` token or typed label against recorded choices (`[pos, neg]`)."""
    choices = [item for item in (prompt.get("choices") or ()) if isinstance(item, Mapping)]
    if token is not None and str(token):
        tok = str(token)
        for index, choice in enumerate(choices):
            if tok == str(choice.get("answer") or ""):
                return index == 0
        low = tok.strip().lower()
        for index, choice in enumerate(choices):
            if low == str(choice.get("label") or "").strip().lower() or low == str(index + 1):
                return index == 0
    if text:
        low = text.strip().lower()
        for index, choice in enumerate(choices):
            if low == str(choice.get("label") or "").strip().lower() or low == str(index + 1):
                return index == 0
        cat = catalog(settings)
        app_cat, brake_cat = cat["approval"], cat["brake"]
        pos = {str(w).lower() for w in (app_cat.get("yes_words") or ())} | {
            str(app_cat.get("yes_label") or "").lower(),
            str(brake_cat.get("continue_label") or "").lower(),
            str(prompt.get("positive") or "").lower(),
        }
        neg = {str(w).lower() for w in (app_cat.get("no_words") or ())} | {
            str(app_cat.get("no_label") or "").lower(),
            str(brake_cat.get("stop_label") or "").lower(),
            str(prompt.get("negative") or "").lower(),
        }
        if low in pos:
            return True
        if low in neg:
            return False
    return None


async def notify_brake(kernel: Any, trigger: Event, *, root_id: int, place: str, work: str,
                       reason: str, limit: str, value: int, cap: int) -> Event:
    """Record a held brake and ask the owner once via neutral `choices`."""
    brake = await kernel._emit(trigger, BRAKE_HELD, {
        "root": root_id, "place": place, "work": work, "reason": reason,
        "limit": limit, "value": value, "cap": cap,
        "origin_actor": trigger.origin_actor or trigger.who,
        "place_kind": (trigger.payload or {}).get("place_kind"),
    }, place=place)
    direct = await identity.direct_place(kernel.conn, OWNER)
    if direct is None:
        return brake
    cat = catalog(getattr(kernel, "settings", None))["brake"]
    text = str(cat["prompt"]).format(work=work, reason=reason, limit=limit, value=value, cap=cap)
    choices = [str(cat["continue_label"]), str(cat["stop_label"])]
    assert brake.id is not None
    await _send_prompt(kernel, brake, direct, text, prompt_kind="brake",
                       prompt_id=int(brake.id), choices=choices)
    return brake


async def _answer_brake(kernel: Any, item: Event, brake_id: int, held: dict[str, Any],
                        allowed: bool, answerer: str) -> None:
    """Record a brake decision and enqueue a continuation intent with the held chain's origin."""
    work = str(held.get("work") or held.get("place") or "stopped work")
    target = str(held.get("place") or item.place or "")
    origin = await _brake_origin(kernel.conn, held)
    await append_event(kernel.conn, ts=kernel.clock.now(), kind=BRAKE_RESUMED,
                       who=origin, origin_actor=origin, place=target, body=item.body,
                       caused_by=item.id,
                       payload={"brake": brake_id, "root": held.get("root"),
                                "allowed": allowed, "by": answerer})
    if allowed:
        kernel.enqueue(Event(
            id=None, ts=kernel.clock.now(), kind="intent", who=origin,
            origin_actor=origin, place=target, body=item.body,
            payload={"description": f"the owner chose to continue the stopped work: {work}",
                     "root": held.get("root"), "brake": brake_id,
                     "place_kind": held.get("place_kind")}))


def _reply_message_id(data: Mapping[str, Any]) -> str:
    """Extract `data.reply_to`'s message id (falling back to reaction target id)."""
    reply_to = data.get("reply_to")
    if isinstance(reply_to, Mapping):
        mid = str(reply_to.get("message_id") or reply_to.get("id") or "")
        if mid:
            return mid
    elif isinstance(reply_to, str) and reply_to:
        return reply_to
    react = data.get("reaction")
    if isinstance(react, Mapping):
        return str(react.get("message_id") or "")
    return ""


def answer(kernel: Any) -> Middleware:
    """Inbound rule: owner or authorized reply answers pending approval/brake from the Log."""

    async def rule(item: Any, ctx: Any) -> Any:
        """Match an incoming answer against pending Log prompts and record the outcome."""
        del ctx
        kind = getattr(item, "kind", "")
        now = kernel.clock.now()
        if kind in ("percept", "tick"):
            await expire_stale(kernel.conn, now=now,
                               trigger=item if getattr(item, "id", None) else None)
        if kind != "percept":
            return Continue()
        payload = item.payload or {}
        answerer = str(item.origin_actor or item.who)
        owner = answerer == OWNER
        request_id, allowed = None, False
        data = payload.get("data") or {}
        reply_mid = _reply_message_id(data) if isinstance(data, Mapping) else ""
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
        elif reply_mid:
            prompt = await _prompt_by_reply(kernel.conn, str(item.place or ""), reply_mid, now=now)
            if prompt is not None:
                token = data.get("answer")
                if token is None:
                    token = data.get("choice")
                if token is None and isinstance(data.get("reaction"), Mapping):
                    token = data["reaction"].get("emoji")
                matched = _match_choice_token(
                    prompt, token, str(payload.get("description") or ""),
                    getattr(kernel, "settings", None))
                if matched is not None:
                    pkind = str(prompt.get("prompt_kind") or "")
                    pid = int(prompt.get("prompt_id") or 0)
                    if pkind == "brake":
                        if not owner:
                            return Continue()
                        held = await brake_by_id(kernel.conn, pid)
                        if held is not None:
                            await _answer_brake(kernel, item, pid, held, matched, answerer)
                        return Continue()
                    if pkind == "approval":
                        if not (owner or kernel.roles.can(
                                answerer, "approval.answer", item.place)):
                            return Continue()
                        request_id, allowed = pid, matched
        else:
            text = str(payload.get("description") or "").strip().lower()
            direct = await identity.direct_place(kernel.conn, OWNER) if owner and text else None
            if owner and text and direct is not None and item.place == direct:
                cat = catalog(getattr(kernel, "settings", None))
                app_cat, brake_cat = cat["approval"], cat["brake"]
                pos_app = {str(w).lower() for w in (app_cat.get("yes_words") or ())} | {
                    str(app_cat.get("yes_label") or "").lower()}
                neg_app = {str(w).lower() for w in (app_cat.get("no_words") or ())} | {
                    str(app_cat.get("no_label") or "").lower()}
                pos_brk = pos_app | {str(brake_cat.get("continue_label") or "").lower()}
                neg_brk = neg_app | {str(brake_cat.get("stop_label") or "").lower()}
                found_approval = await pending(kernel.conn, now=now)
                found_brake = await pending_brake(kernel.conn)
                if found_brake is not None and (
                        found_approval is None or found_brake["id"] > found_approval["id"]):
                    if text in pos_brk or text in neg_brk:
                        await _answer_brake(kernel, item, int(found_brake["id"]), found_brake,
                                            text in pos_brk, answerer)
                    return Continue()
                if found_approval is not None and (text in pos_app or text in neg_app):
                    request_id, allowed = int(found_approval["id"]), text in pos_app
        if request_id is None:
            return Continue()
        asked = await by_id(kernel.conn, request_id)
        if asked is None:
            held = await brake_by_id(kernel.conn, request_id) if owner else None
            if held is not None:
                await _answer_brake(kernel, item, request_id, held, allowed, answerer)
            return Continue()
        if await _is_closed(kernel.conn, request_id) or now - asked["ts"] > APPROVAL_TTL:
            return Continue()
        requester = str(asked.get("who") or "") or None
        await append_event(kernel.conn, ts=now, kind=ANSWERED,
                           who=requester, origin_actor=requester,
                           place=asked.get("place"), body=item.body, caused_by=item.id,
                           payload={"request": request_id, "allowed": allowed,
                                    "who": requester, "by": answerer})
        if allowed:
            kernel.enqueue(Event(
                id=None, ts=now, kind="intent", who=requester, origin_actor=requester,
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
    await kernel.tools.act(registry.own_action(event, f"{event.body}.say", {"text": text}))
    return True


async def _present_step(kernel: Any, event: Event, line: str, *, chain: int, place: str) -> None:
    """Show a progress step either by editing one message in place or sending a new message."""
    adapter = kernel.tools.adapters.get(str(event.body))
    if adapter is None:
        return
    body = str(event.body)
    mode = switches(kernel, body=body).get("progress", "edit")
    if mode == "edit" and "edit" in shows(adapter):
        row = await (await kernel.conn.execute(
            "SELECT payload->>'message_id' FROM events WHERE kind = %s AND place = %s "
            "AND body = %s AND payload->>'kind' = 'progress' AND payload->>'chain' = %s "
            "AND COALESCE(payload->>'message_id', '') <> '' ORDER BY id DESC LIMIT 1",
            (SHOWN, place, body, str(chain)))).fetchone()
        kwargs: dict[str, Any] = {"text": line}
        if row and row[0]:
            kwargs["message_id"] = str(row[0])
        res = await adapter.show(place, "edit", **kwargs)
        if isinstance(res, Mapping) and res.get("message_id"):
            await kernel._emit(event, SHOWN, {
                "kind": "progress", "chain": str(chain),
                "message_id": str(res["message_id"])}, place=place)
        return
    await kernel.tools.act(registry.own_action(event, f"{body}.say", {"text": line}))


async def _last_shown(conn: Any, place: str) -> int:
    """Return the id of the last event of this place that reasoning already showed."""
    row = await (await conn.execute(
        "SELECT max((payload->>'note')::bigint) FROM events "
        "WHERE kind = %s AND place = %s AND payload ? 'note'", (SHOWN, place))).fetchone()
    return int((row or (0,))[0] or 0)


async def _notes(conn: Any, chain: int, after: int) -> list[tuple[int, str, str]]:
    """Return the thoughts of one chain that were not shown yet, oldest first."""
    cur = await conn.execute(
        "SELECT id, place, payload FROM events WHERE kind = 'decision.note' "
        "AND caused_by = %s AND id > %s AND payload->>'act' = 'think' ORDER BY id",
        (chain, after))
    return [(int(row[0]), str(row[1] or ""), str((row[2] or {}).get("thought") or ""))
            for row in await cur.fetchall()]


def thinking_show(kernel: Any) -> Middleware:
    """Inbound best-effort rule: show allowed thoughts (`off|owner_direct|all`)."""

    async def rule(item: Any, ctx: Any) -> Any:
        """Show thoughts when privacy allows."""
        del ctx
        place = str(item.place or "")
        if not place:
            return Continue()
        cat = catalog(getattr(kernel, "settings", None))
        think_prefix = str((cat.get("thinking") or {}).get("prefix") or "")
        think_mode = switches(kernel, body=str(item.body or "")).get("thinking", "owner_direct")
        if think_mode == "off":
            return Continue()
        chain = int(item.caused_by or item.id or 0)
        thoughts = await _notes(kernel.conn, chain, 0)
        eligible = think_mode == "all" or (
            think_mode == "owner_direct" and await privacy.owner_direct(kernel.conn, place)
        )
        if thoughts and eligible:
            shown = await _last_shown(kernel.conn, place)
            for note, note_place, thought in [n for n in thoughts if n[0] > shown]:
                if not thought or not await privacy.visible(kernel.conn, context=place,
                                                            source=note_place, before=note):
                    continue
                line = f"{think_prefix} {thought}".strip()
                if await present(kernel, replace(item, place=place), "edit", line):
                    await kernel._emit(item, SHOWN, {"place": place, "kind": "thinking",
                                                     "note": note}, place=place)
        return Continue()

    return contract.mark_rule(rule, name="narrate.thinking", kind="best-effort")


async def _step_visible(conn: Any, item: Any, *, place: str) -> bool:
    """Return whether one step's source place and causal ancestors are visible at `place`."""
    payload = item.payload if isinstance(getattr(item, "payload", None), Mapping) else {}
    source_place = str(payload.get("source_place") or "")
    before = item.id or item.caused_by
    if source_place and not await privacy.visible(conn, context=place, source=source_place,
                                                  before=before):
        return False
    start = item.caused_by or item.id
    if start is None:
        return True
    cur = await conn.execute(
        "WITH RECURSIVE up AS ("
        "SELECT id, caused_by, place FROM events WHERE id = %s "
        "UNION ALL SELECT e.id, e.caused_by, e.place FROM events e JOIN up "
        "ON e.id = up.caused_by) "
        "SELECT id, place FROM up WHERE COALESCE(place, '') <> ''",
        (int(start),))
    for anc_id, anc_place in await cur.fetchall():
        if not await privacy.visible(conn, context=place, source=str(anc_place),
                                     before=int(anc_id)):
            return False
    return True


def progress_show(kernel: Any) -> Middleware:
    """Inbound best-effort rule: show act progress steps (`off|long|all`)."""

    async def rule(item: Any, ctx: Any) -> Any:
        """Show step progress when enabled and visible at the destination place."""
        del ctx
        place = str(item.place or "")
        if not place or item.kind != "act.result":
            return Continue()
        steps_mode = switches(kernel, body=str(item.body or "")).get("steps", "long")
        if steps_mode == "off":
            return Continue()
        capability = str((item.payload or {}).get("capability") or "")
        declared = kernel.tools.get(capability) or {}
        if (steps_mode == "all" or declared.get("long_running")) and await _step_visible(
            kernel.conn, item, place=place
        ):
            cat = catalog(getattr(kernel, "settings", None))
            step_prefix = str((cat.get("step") or {}).get("prefix") or "")
            receipt = item.payload or {}
            short = _short(interaction_text(receipt.get("result") or receipt.get("reason")))
            line = f"{step_prefix} {capability}: {short}".strip()
            chain = int(item.caused_by or item.id or 0)
            await _present_step(kernel, item, line, chain=chain, place=place)
        return Continue()

    return contract.mark_rule(rule, name="narrate.progress", kind="best-effort")


def narrate(kernel: Any) -> Middleware:
    """Inbound rule: show allowed thoughts and steps under best-effort error handling."""
    return contract.compose(thinking_show(kernel), progress_show(kernel), kernel=kernel)


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

    return contract.mark_rule(rule, name="typing", kind="best-effort")


async def _is_shared_with_owner(conn: Any, body: str) -> bool:
    """True when `body`'s self-account is bound to `person:owner` (D70 shared account)."""
    acc = (await read_state(conn, f"self_account:{body}") or {}).get("account")
    if not acc:
        return False
    alias = await read_state(conn, f"alias:{acc}")
    return bool(alias and (alias.get("who") or alias.get("person")) == OWNER)


async def _is_read_marked(conn: Any, place: str, msg_id: str) -> bool:
    """True when `interaction.shown` (`kind='read'`) already recorded `msg_id` in `place`."""
    row = await (await conn.execute(
        "SELECT 1 FROM events WHERE kind = %s AND place = %s "
        "AND payload->>'kind' = 'read' AND payload->>'message_id' = %s LIMIT 1",
        (SHOWN, place, msg_id))).fetchone()
    return row is not None


async def _unread_messages(conn: Any, place: str, body: str) -> list[str]:
    """Return un-marked non-owner percept message ids in `place`, oldest first."""
    cur = await conn.execute(
        "SELECT e.payload->'data'->>'message_id' FROM events e "
        "WHERE e.kind = 'percept' AND e.place = %s AND e.body = %s "
        "AND COALESCE((e.payload->>'imported')::boolean, false) = false "
        "AND COALESCE((e.payload->>'backfill')::boolean, false) = false "
        "AND COALESCE(e.origin_actor, e.who, '') <> %s "
        "AND COALESCE(e.payload->'data'->>'message_id', '') <> '' "
        "AND NOT EXISTS ("
        "  SELECT 1 FROM events s WHERE s.kind = %s AND s.place = e.place "
        "  AND s.payload->>'kind' = 'read' "
        "  AND s.payload->>'message_id' = e.payload->'data'->>'message_id') "
        "ORDER BY e.id ASC",
        (place, body, OWNER, SHOWN))
    seen: set[str] = set()
    mids: list[str] = []
    for row in await cur.fetchall():
        mid = str(row[0] or "")
        if mid and mid not in seen:
            seen.add(mid)
            mids.append(mid)
    return mids


def read_receipts_inbound(kernel: Any) -> Middleware:
    """Inbound rule: mark a live percept read on non-shared adapters and record the mark."""

    async def rule(item: Any, ctx: Any) -> Any:
        """Mark the percept read unless imported/backfilled, disabled, or shared with owner."""
        del ctx
        if getattr(item, "kind", "") != "percept":
            return Continue()
        payload = item.payload or {}
        if payload.get("imported") is True or payload.get("backfill") is True:
            return Continue()
        place = str(item.place or "")
        body = str(item.body or "")
        if not place or not body:
            return Continue()
        if switches(kernel, body=body).get("read_receipts", "on") == "off":
            return Continue()
        adapter = kernel.tools.adapters.get(body)
        if adapter is None or "read_receipts" not in shows(adapter):
            return Continue()
        msg_id = (payload.get("data") or {}).get("message_id")
        if not msg_id or await _is_shared_with_owner(kernel.conn, body):
            return Continue()
        if await _is_read_marked(kernel.conn, place, str(msg_id)):
            return Continue()
        await adapter.show(place, "read_receipts", message_id=msg_id, message_ids=[msg_id])
        await kernel._emit(item, SHOWN, {"kind": "read", "message_id": str(msg_id)}, place=place)
        return Continue()

    return contract.mark_rule(rule, name="read_receipts_inbound", kind="best-effort")


def read_receipts_action(kernel: Any) -> Middleware:
    """Action rule: on an owner-shared account, mark earlier unread messages read on speak."""

    async def rule(item: Any, ctx: Any) -> Any:
        """Mark unread non-owner messages in `place` read when an allowed communicative act fires."""
        del item
        if ctx.get("refusal"):
            return Continue()
        descriptor = ctx.get("descriptor") or {}
        if not descriptor.get("communicative"):
            return Continue()
        event = ctx["event"]
        place = str(ctx.get("place") or event.place or "")
        if not place or place != str(event.place or ""):
            return Continue()
        body = str(event.body or "")
        if not body or switches(kernel, body=body).get("read_receipts", "on") == "off":
            return Continue()
        adapter = kernel.tools.adapters.get(body)
        if adapter is None or "read_receipts" not in shows(adapter):
            return Continue()
        if not await _is_shared_with_owner(kernel.conn, body):
            return Continue()
        mids = await _unread_messages(kernel.conn, place, body)
        if not mids:
            return Continue()
        await adapter.show(place, "read_receipts", message_id=mids[-1], message_ids=mids)
        for mid in mids:
            await kernel._emit(event, SHOWN, {"kind": "read", "message_id": mid}, place=place)
        return Continue()

    return contract.mark_rule(rule, name="read_receipts_action", kind="best-effort")


def install_commands(kernel: Any) -> None:
    """Register the `/show` command on the kernel tool registry."""
    if kernel.tools.command("show") is None:
        kernel.tools.add_command(registry.CommandEntry(
            "show",
            "Switch interaction display: /show <name> <value> [adapter].",
            "interaction.show", show_switch, owner="interaction"))


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

        # Sticker fallback: stickers: off (or unsupported) -> fallback text
        sticker_mode = switches(kernel, body=body).get("stickers", "on")
        if arguments.get("sticker") and (sticker_mode == "off" or "stickers" not in shows(adapter)):
            emoji = str(arguments.pop("sticker_emoji", "") or "[sticker]")
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
            await kernel.tools.act(registry.own_action(ctx["event"], str(ctx["capability"]),
                                                       passed))
        ctx["handled"] = True
        return Continue()

    return rule
