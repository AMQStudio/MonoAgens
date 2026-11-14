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

import asyncio

from monoagens.contract import composing as composing_event
from monoagens.contract import progress as progress_event
from monoagens.contract import seen as seen_event
from monoagens.contract import thought as thought_event
from monoagens.core.hooks import Continue, Middleware, Replace
from monoagens.infra.store import append_event, read_state, write_state
from monoagens.infra.prior import constants
from monoagens.middleware import contract, identity, loader, privacy, registry
from monoagens.types import Event

_LANG_DIR = pathlib.Path(__file__).resolve().parent / "lang"

DEFAULTS: dict[str, str] = {
    "thinking": "owner_direct",
    "steps": "long",
    "approvals": "owner",
}
# What the `/show` command may set: the middleware's own decisions only (no body's options).
ALLOWED_SHOW: dict[str, tuple[str, ...]] = {
    "thinking": ("off", "owner_direct", "all"),
    "steps": ("off", "long", "all"),
    "approvals": ("owner", "off"),
}

OWNER = "person:owner"
REQUEST = "approval.requested"
ANSWERED = "approval.answered"
CONSUMED = "approval.consumed"
EXPIRED = "approval.expired"
APPROVAL_TTL = timedelta(minutes=30)
BRAKE_HELD = "brake.held"
BRAKE_RESUMED = "brake.resumed"
QUESTION_ASKED = "question.asked"
QUESTION_ANSWERED = "question.answered"
QUESTION_EXPIRED = "question.expired"
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


async def question_ttl(kernel: Any) -> timedelta:
    """Return how long a question stays open before it expires (§2.4.6)."""
    values = constants(getattr(kernel, "settings", None) or {})
    return timedelta(seconds=float(values.get("question_ttl_seconds", 900)))


async def ask(kernel: Any, trigger: Event, place: str, question: str, choices: list[str], *,
              kind: str, request: int) -> str:
    """Ask one place its own way and record the question (§2.4.6).

    The body's own `ask(place, question, choices)` draws it when the body has one (buttons, a
    poll, a menu); numbered lines are the one fallback, so a question is never lost. The ref it
    answers is what an answer comes back with.
    """
    body = await _body_of(kernel.conn, place)
    adapter = kernel.tools.adapters.get(body)
    drawn = getattr(adapter, "ask", None)
    ref = ""
    if callable(drawn):
        try:
            ref = str(await drawn(place, question, list(choices)) or "")
        except Exception as exc:  # best-effort: recorded, then the text fallback (§2.4.5)
            await kernel._record_error(trigger, exc)
    lines = "\n".join(f"{number}. {choice}" for number, choice in enumerate(choices, 1))
    args: dict[str, Any] = {"text": f"{question}\n{lines}" if not ref else question,
                            "choices": list(choices)}
    act_out = await kernel._emit(trigger, "act.out", {
        "capability": f"{body}.say", "arguments": args, "communicative": True}, place=place)
    result: Any = None
    if adapter is not None:
        out_event = Event(id=None, ts=kernel.clock.now(), kind="act.out", who=OWNER,
                          place=place, body=body, caused_by=act_out.id)
        receipt = await kernel.tools.act(registry.own_action(out_event, f"{body}.say", args))
        result = receipt.result if receipt.state == "done" else None
        if result is not None:
            await kernel._emit(act_out, "act.result", {
                "capability": f"{body}.say", "result": result}, place=place)
    message_id = str(result.get("message_id") or "") if isinstance(result, Mapping) else ""
    recorded = _normalize_choices(result.get("choices") if isinstance(result, Mapping) else None,
                                  list(choices))
    await kernel._emit(act_out, QUESTION_ASKED, {
        "ref": ref or message_id, "request": int(request), "kind": kind, "place": place,
        "question": question, "choices": recorded,
    }, place=place, description=question)
    return ref or message_id


async def _send_prompt(kernel: Any, trigger: Event, place: str, text: str, *,
                       prompt_kind: str, prompt_id: int,
                       choices: list[str]) -> None:
    """Ask one question and record how its prompt is read back (`interaction.shown`)."""
    ref = await ask(kernel, trigger, place, text, list(choices), kind=prompt_kind,
                    request=prompt_id)
    await kernel._emit(trigger, SHOWN, {
        "kind": "prompt", "prompt_kind": prompt_kind, "prompt_id": prompt_id,
        "message_id": ref, "choices": _normalize_choices(None, list(choices)),
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


async def expire_stale(conn: Any, *, now: datetime, trigger: Event | None = None,
                       ttl: timedelta | None = None) -> int:
    """Mark unanswered questions older than their TTL as expired (§2.4.6)."""
    cutoff = now - (ttl or APPROVAL_TTL)
    cur = await conn.execute(
        "SELECT id, ts, who, place, body, origin_actor, payload FROM events "
        "WHERE kind = %s AND ts < %s AND id NOT IN ("
        "  SELECT (payload->>'request')::bigint FROM events WHERE kind IN (%s, %s)"
        "  AND payload ? 'request') ORDER BY id ASC",
        (REQUEST, cutoff, ANSWERED, EXPIRED))
    rows = await cur.fetchall()
    expired = 0
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
        expired += 1
    expired += await _expire_questions(conn, now=now, cutoff=cutoff, trigger=trigger)
    return expired


async def _expire_questions(conn, *, now: datetime, cutoff: datetime,
                            trigger: Event | None) -> int:
    """Record `question.expired` for every question past its TTL that nobody answered."""
    cur = await conn.execute(
        "SELECT id, ts, payload FROM events WHERE kind = %s AND ts < %s AND id NOT IN ("
        "  SELECT (payload->>'question')::bigint FROM events WHERE kind IN (%s, %s)"
        "  AND payload ? 'question') ORDER BY id ASC",
        (QUESTION_ASKED, cutoff, QUESTION_ANSWERED, QUESTION_EXPIRED))
    rows = await cur.fetchall()
    for question_id, _ts, payload in rows:
        data = dict(payload or {})
        await append_event(
            conn, ts=now, kind=QUESTION_EXPIRED,
            place=str(data.get("place") or "") or None,
            caused_by=trigger.id if trigger is not None else int(question_id),
            payload={"question": int(question_id), "ref": data.get("ref"),
                     "request": data.get("request"), "reason": "expired"})
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


async def notify_brake(kernel: Any, trigger: Event, *, root_id: int, place: str, work: str,
                       reason: str, limit: str, value: int, cap: int) -> Event:
    """Record a held brake and ask the owner once, through the one question door (§2.4.6)."""
    brake = await kernel._emit(trigger, BRAKE_HELD, {
        "root": root_id, "place": place, "work": work, "reason": reason,
        "limit": limit, "value": value, "cap": cap,
        "origin_actor": trigger.origin_actor or trigger.who,
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


async def question_by_ref(conn: Any, ref: str) -> dict[str, Any] | None:
    """Return the open question one answer's `prompt` ref names, or None (§2.4.6)."""
    if not ref:
        return None
    row = await (await conn.execute(
        "SELECT id, ts, payload FROM events WHERE kind = %s AND payload->>'ref' = %s "
        "ORDER BY id DESC LIMIT 1", (QUESTION_ASKED, ref))).fetchone()
    if row is None:
        return None
    answered = await (await conn.execute(
        "SELECT 1 FROM events WHERE kind IN (%s, %s) AND payload->>'question' = %s LIMIT 1",
        (QUESTION_ANSWERED, QUESTION_EXPIRED, str(int(row[0]))))).fetchone()
    if answered is not None:
        return None
    return {"id": int(row[0]), "ts": row[1], **dict(row[2] or {})}


async def _choice_index(choices: list[str], value: str) -> int | None:
    """Return which choice one answer's value names: its label, its 1-based position, or None."""
    if not value:
        return None
    for index, choice in enumerate(choices):
        if value == choice or value == str(index + 1):
            return index
    return None


def answer(kernel: Any) -> Middleware:
    """Inbound rule: owner or authorized reply answers pending approval/brake from the Log."""

    async def rule(item: Any, ctx: Any) -> Any:
        """Match an incoming answer against pending Log prompts and record the outcome."""
        del ctx
        kind = getattr(item, "kind", "")
        now = kernel.clock.now()
        if kind in ("percept", "tick"):
            await expire_stale(kernel.conn, now=now,
                               trigger=item if getattr(item, "id", None) else None,
                               ttl=await question_ttl(kernel))
        if kind != "percept":
            return Continue()
        payload = item.payload or {}
        answerer = str(item.origin_actor or item.who)
        owner = answerer == OWNER
        request_id, allowed = None, False
        data = payload.get("data") or {}
        answer = payload.get("answer")
        if isinstance(answer, Mapping) and answer.get("prompt"):
            question = await question_by_ref(kernel.conn, str(answer["prompt"]))
            if question is not None:
                choices = [str(item.get("label") or "")
                           for item in (question.get("choices") or ())]
                value = str(answer.get("value") or "")
                matched = _choice_index(choices, value)
                if matched is not None:
                    await kernel._emit(item, QUESTION_ANSWERED, {
                        "question": int(question["id"]), "ref": str(answer["prompt"]),
                        "request": question.get("request"), "value": value,
                        "allowed": matched == 0, "by": answerer,
                    }, place=item.place)
                    kind = str(question.get("kind") or "")
                    target = int(question.get("request") or 0)
                    if kind == "brake":
                        if not owner:
                            return Continue()
                        held = await brake_by_id(kernel.conn, target)
                        if held is not None:
                            await _answer_brake(kernel, item, target, held, matched == 0, answerer)
                        return Continue()
                    if kind == "approval":
                        if not (owner or kernel.roles.can(answerer, "approval.answer",
                                                          item.place)):
                            return Continue()
                        request_id, allowed = target, matched == 0
        if isinstance(payload.get("answer"), Mapping):
            pass  # already handled above, from the question's own ref
        elif payload.get("approval") is not None:
            if not (owner or kernel.roles.can(answerer, "approval.answer", item.place)):
                return Continue()
            request_id, allowed = int(payload["approval"]), bool(payload["answer"])
        elif payload.get("brake") is not None:
            if not owner:
                return Continue()
            held = await brake_by_id(kernel.conn, int(payload["brake"]))
            if held is not None:
                await _answer_brake(kernel, item, int(payload["brake"]), held,
                                    bool(payload["answer"]), answerer)
            return Continue()
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


def switches(kernel: Any) -> dict[str, str]:
    """Return the effective `interaction:` switches: the defaults, then config.yaml.

    These are the middleware's own decisions (whether a thought or a step is worth drawing). What
    a body draws with them is the body's own `render:` option, read by the body itself.
    """
    merged = {**DEFAULTS,
              **{str(k): str(v) for k, v in (kernel.settings.get("interaction") or {}).items()}}
    return {key: str(value) for key, value in merged.items()}


def presentation_timeout(kernel: Any) -> float:
    """Return how long one body's `render` may take, in seconds."""
    values = constants(getattr(kernel, "settings", None) or {})
    return float(values.get("presentation_timeout_ms", 500)) / 1000


async def present(kernel: Any, event: Event, kind: str, text: str) -> bool:
    """Tell the body's own `render` what the soul is doing; never break the turn (§2.4.5).

    A body that offers no `render` renders nothing, and one that raises is recorded as an `error`
    and skipped: a redraw is best-effort, always.
    """
    adapter = kernel.tools.adapters.get(str(event.body or ""))
    render = getattr(adapter, "render", None)
    if not callable(render):
        return False
    lifecycle = _lifecycle(kind, event, text)
    if lifecycle is None:
        return False
    # A slow or failing render is the calling best-effort rule's error to record (§2.4.5, D82):
    # it never breaks the turn, but it is never silent either.
    await asyncio.wait_for(render(lifecycle), presentation_timeout(kernel))
    return True


def _lifecycle(kind: str, event: Event, text: str) -> Any:
    """Return the contract lifecycle event one kind of presentation means, or None."""
    place = str(event.place or "")
    if kind == "composing":
        return composing_event(place)
    if kind == "progress":
        return progress_event(place, {"text": text})
    if kind == "thought":
        return thought_event(place, text)
    if kind == "seen":
        return seen_event(int(event.id or 0), place)
    return None


async def _present_step(kernel: Any, event: Event, line: str, *, chain: int, place: str) -> None:
    """Tell the body one step of long work, and remember it so it is never said twice."""
    del chain
    if await present(kernel, replace(event, place=place), "progress", line):
        await kernel._emit(event, SHOWN, {"kind": "progress", "note": event.id}, place=place)


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
        think_mode = switches(kernel).get("thinking", "owner_direct")
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
                if await present(kernel, replace(item, place=place), "thought", line):
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
        steps_mode = switches(kernel).get("steps", "long")
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


def composing(kernel: Any) -> Middleware:
    """Action rule: tell the body the soul is about to speak in this place (§2.4.5)."""

    async def rule(item: Any, ctx: Any) -> Any:
        """Compose only for a communicative act, once the hook let it through."""
        del item
        if ctx.get("refusal") or ctx.get("handled"):
            return Continue()
        if not (ctx.get("descriptor") or {}).get("communicative"):
            return Continue()
        await present(kernel, ctx["event"], "composing", "")
        return Continue()

    return contract.mark_rule(rule, name="composing", kind="best-effort")


def seen(kernel: Any) -> Middleware:
    """Inbound rule: tell the body the soul has taken in a live percept, once per event."""

    async def rule(item: Any, ctx: Any) -> Any:
        """Render `seen` for a live percept, and never for an imported or backfilled one."""
        del ctx
        if getattr(item, "kind", "") != "percept" or getattr(item, "id", None) is None:
            return Continue()
        if getattr(item, "imported", False) or getattr(item, "backfill", False):
            return Continue()
        key = f"seen:{int(item.id)}"
        if await read_state(kernel.conn, key) is not None:
            return Continue()
        if not await present(kernel, item, "seen", ""):
            return Continue()
        event = await kernel._emit(item, SHOWN, {"kind": "seen", "event": int(item.id)},
                                   place=item.place)
        await write_state(kernel.conn, key, {"event": int(item.id)}, event.id)
        return Continue()

    return contract.mark_rule(rule, name="seen", kind="best-effort")


def install_commands(kernel: Any) -> None:
    """Register the `/show` command on the kernel tool registry."""
    if kernel.tools.command("show") is None:
        kernel.tools.add_command(registry.CommandEntry(
            "show",
            "Switch one interaction setting: /show <name> <value>.",
            "interaction.show", show_switch, owner="interaction"))


def _resolve_show_value(what: str, raw_value: str) -> str | None:
    """Return the value `/show` should write, or None when it is not one this setting allows."""
    value = raw_value.strip().lower()
    if value == "on" and what in ("thinking", "steps"):
        return DEFAULTS[what]
    return value if value in ALLOWED_SHOW.get(what, ()) else None


async def show_switch(kernel: Any, event: Event, arguments: list[str]) -> str:
    """Write one `interaction:` setting in config.yaml and apply it at once.

    Only the middleware's own decisions are here; what a body draws with them is that body's own
    `render:` option, which the body reads itself (§2.4.5).
    """
    names = ", ".join(DEFAULTS)
    usage = f"usage: /show <{names}> <value>"
    if len(arguments) < 2:
        return usage
    what = arguments[0]
    if what not in DEFAULTS:
        return usage
    after = _resolve_show_value(what, arguments[1])
    if after is None:
        return f"invalid value {arguments[1]!r} for {what}; expected {', '.join(ALLOWED_SHOW[what])}"
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
