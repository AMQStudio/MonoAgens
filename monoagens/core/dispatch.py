"""Action dispatch: resolve, check and hand one `act` call to the tool that declares it."""

from __future__ import annotations

from dataclasses import replace
from typing import Any

from monoagens.core import arguments, threads
from monoagens.core.context import working
from monoagens.middleware import privacy
from monoagens.types import Event


async def dispatch(kernel: Any, event: Event, args: dict[str, Any], *,
                   said: set[str] | None = None) -> bool:
    """Resolve, guard and hand one `act` call to the tool that declares it (D2, D46).

    `said` holds the places this turn already spoke to: a turn says at most one
    message per place, so a second communicative act there is refused (D51).
    """
    capability = str(args.get("capability") or "")
    cap = kernel.tools.get(capability)
    if cap is None:
        await kernel._record_error(event, f"unknown capability: {capability!r}")
        return False
    body = args.get("arguments")
    broken = arguments.capability_errors(cap, body)
    if broken:
        await kernel._emit(event, "act.result", {
            "capability": capability,
            "result": {"error": f"invalid arguments for {capability}: "
                                f"{arguments.refusal(broken)}"}})
        return True
    body = dict(body)
    routed = cap.get("communicative") or capability == privacy.SHARE
    to, via = (body.pop("to", None), body.pop("via", None)) if routed else (None, None)
    ctx = {"conn": kernel.conn, "event": event, "now": kernel.clock.now(),
           "capability": capability, "origin_actor": event.origin_actor, "to": to,
           "identity_changing": cap.get("identity_changing")}
    if capability == privacy.SHARE:
        allowed, source = await privacy.consent(kernel.conn, event, body)
        if not allowed:
            await kernel._emit(event, "act.result", {"capability": capability, "to": to,
                               "result": {"error": "only someone who was part of it can "
                                                   "share it"}})
            return True
        payload = await kernel._run_hooks("action", body, ctx, event, tool="self")
        if payload is not None:
            await privacy.grant(kernel, event, payload, to, source)
        return True
    target, error = await privacy.resolve_to(kernel.conn, str(to), via) if to else (None, None)
    if to and target is None and str(to) == event.who:
        target, error = event.place, None  # the person is here: answer them here
    if error:
        await kernel._emit(event, "act.result",
                           {"capability": capability, "to": to, "result": {"error": error}})
        return True
    place = target or event.place
    if cap.get("communicative") and said is not None and place in said:
        await kernel._emit(event, "act.result", {"capability": capability, "to": to, "result": {
            "error": f"already said something to {to or place} this turn"}})
        return True
    if place != event.place and cap.get("communicative"):
        await _intend(kernel, event, capability, body, to, place)
        if said is not None:
            said.add(place)
        return True
    if await privacy.leaks(kernel.conn, place, str(body.get("text") or "")):
        await kernel._emit(event, "act.result", {"capability": capability, "to": to, "result": {
            "error": "that place may not see this content; say it where it belongs"}})
        return True
    record = {"capability": capability, "arguments": body,
              "communicative": bool(cap.get("communicative"))}
    stored = await kernel._emit(event, "act.out", {**record, "to": to} if to else record,
                                place=place)
    if cap.get("communicative") and said is not None:
        said.add(place)
    await kernel._encode(stored, str(body.get("text") or "") or None)
    payload = await kernel._run_hooks("action", body, ctx, event,
                                      tool=capability.partition(".")[0])
    if payload is None:
        return True
    if cap.get("long_running"):
        await threads.open_for(kernel, event, await privacy.audience(kernel.conn, event.place))
    result = await kernel.tools.act(replace(event, place=place), capability, payload,
                                    event_id=stored.id)
    if result is not None:
        kernel.enqueue(Event(id=None, ts=kernel.clock.now(), kind="act.result", who=event.who,
                             place=place, body=event.body, caused_by=stored.id,
                             origin_actor=event.origin_actor,
                             payload={"capability": capability, "result": result}))
    if capability == "self.set_identity" and isinstance(result, dict) and kernel.pfc:
        kernel.pfc.identity = result["identity"]
        kernel.settings = {**kernel.settings, "identity": result["identity"]}
    return True


async def _intend(kernel: Any, event: Event, capability: str, body: dict[str, Any],
                  to: str | None, place: str) -> None:
    """Record what to say elsewhere and wake the reasoning there with the intent itself (D48)."""
    _, goal = await working.chain_root(kernel.conn, event.id)
    said = str(body.get("text") or body.get("item") or "")
    intent = await kernel._emit(event, "act.intent", {
        "capability": capability, "arguments": body, "to": to, "target": place,
        "why": goal})
    reason = f"you set out to say {said!r} to {to or place}"
    if await privacy.leaks(kernel.conn, place, f"{said}\n{goal}"):
        reason = f"you set out to send a message to {to or place}; its words cannot appear here"
    elif goal:
        reason += f", because {goal}"
    kernel.enqueue(Event(
        id=None, ts=kernel.clock.now(), kind="intent", who=None, place=place,
        caused_by=intent.id, origin_actor=event.origin_actor,
        payload={"description": f"{reason} (from {event.place})",
                 "capability": capability, "to": to, "intent": intent.id,
                 "place_kind": await privacy.place_kind(kernel.conn, place)}))
