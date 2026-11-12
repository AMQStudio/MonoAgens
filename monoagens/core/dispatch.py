"""Action dispatch: the door between one `act` call and the tool that carries it out.

The door checks the arguments against the descriptor, asks the action hook (which resolves
the route and applies the absolute lines), records the act before it happens, hands it to
the adapter or plugin, and records the result. It reads only opaque ids and the labels the
middleware attached.
"""

from __future__ import annotations

from dataclasses import dataclass, field, replace
from typing import Any

from monoagens.core import arguments, threads
from monoagens.core.context import working
from monoagens.types import Event


@dataclass(frozen=True, slots=True)
class Dispatch:
    """What one `act` did, with the labels the middleware attached to it."""

    acted: bool
    labels: dict[str, Any] = field(default_factory=dict)


async def dispatch(kernel: Any, event: Event, args: dict[str, Any], *,
                   said: set[str] | None = None) -> Dispatch:
    """Resolve, guard and hand one `act` call to the tool that declares it (D2, D46).

    `said` holds the places this turn already spoke to: a turn says at most one
    message per place, so a second communicative act there is refused (D51).
    """
    capability = str(args.get("capability") or "")
    cap = kernel.tools.get(capability)
    if cap is None:
        await kernel._record_error(event, f"unknown capability: {capability!r}")
        return Dispatch(acted=False)
    raw = dict(args.get("arguments") or {})
    broken = arguments.capability_errors(cap, raw)
    if broken:
        await kernel._emit(event, "act.result", {
            "capability": capability,
            "result": {"error": f"invalid arguments for {capability}: "
                                f"{arguments.refusal(broken)}"}})
        return Dispatch(acted=True)
    routing = kernel.tools.routing_keys(cap)
    passed = {key: value for key, value in raw.items() if key not in routing}
    ctx = {"conn": kernel.conn, "event": event, "now": kernel.clock.now(),
           "capability": capability, "descriptor": cap, "origin_actor": event.origin_actor,
           "arguments": dict(raw), "to": raw.get("to"), "via": raw.get("via"),
           "place": None, "refusal": None, "handled": False, "labels": {},
           "long_running": bool(cap.get("long_running")),
           "identity_changing": bool(cap.get("identity_changing"))}
    payload = await kernel._run_hooks("action", passed, ctx, event,
                                      tool=capability.partition(".")[0])
    if ctx.get("refusal"):
        refusal = {"capability": capability, "result": {"error": str(ctx["refusal"])}}
        if ctx["to"] is not None:
            refusal = {"capability": capability, "to": ctx["to"],
                       "result": {"error": str(ctx["refusal"])}}
        await kernel._emit(event, "act.result", refusal)
        return Dispatch(acted=True)
    if payload is None or ctx.get("handled"):
        return Dispatch(acted=True, labels=ctx["labels"])
    place, to = ctx.get("place") or event.place, ctx.get("to")
    if cap.get("communicative") and said is not None and place in said:
        await kernel._emit(event, "act.result", {"capability": capability, "to": to, "result": {
            "error": f"already said something to {to or place} this turn"}})
        return Dispatch(acted=True)
    if place != event.place:
        await _intend(kernel, event, capability, passed, to, place, ctx["labels"])
        if said is not None:
            said.add(place)
        return Dispatch(acted=True, labels=ctx["labels"])
    record = {"capability": capability, "arguments": passed,
              "communicative": bool(cap.get("communicative"))}
    stored = await kernel._emit(event, "act.out", {**record, "to": to} if to else record,
                                place=place)
    if cap.get("communicative") and said is not None:
        said.add(place)
    await kernel._encode(stored, str(passed.get("text") or "") or None)
    if cap.get("long_running"):
        await threads.open_for(kernel, event, ctx["labels"].get("audience") or ())
    result = await kernel.tools.act(replace(event, place=place), capability, payload,
                                    event_id=stored.id)
    if result is not None:
        kernel.enqueue(Event(id=None, ts=kernel.clock.now(), kind="act.result", who=event.who,
                             place=place, body=event.body, caused_by=stored.id,
                             origin_actor=event.origin_actor,
                             payload={"capability": capability, "result": result}))
    if isinstance(result, dict) and isinstance(result.get("identity"), dict) and kernel.pfc:
        kernel.pfc.identity = result["identity"]
        kernel.settings = {**kernel.settings, "identity": result["identity"]}
    return Dispatch(acted=True, labels=ctx["labels"])


async def _intend(kernel: Any, event: Event, capability: str, passed: dict[str, Any],
                  to: str | None, place: str, labels: dict[str, Any]) -> None:
    """Record what to say elsewhere and wake the reasoning there with the intent itself (D48)."""
    _, goal = await working.chain_root(kernel.conn, event.id)
    said = str(passed.get("text") or passed.get("item") or "")
    intent = await kernel._emit(event, "act.intent", {
        "capability": capability, "arguments": passed, "to": to, "target": place,
        "why": goal})
    reason = f"you set out to say {said!r} to {to or place}"
    if labels.get("words_hidden"):
        reason = f"you set out to send a message to {to or place}; its words cannot appear here"
    elif goal:
        reason += f", because {goal}"
    kernel.enqueue(Event(
        id=None, ts=kernel.clock.now(), kind="intent", who=None, place=place,
        caused_by=intent.id, origin_actor=event.origin_actor,
        payload={"description": f"{reason} (from {event.place})",
                 "capability": capability, "to": to, "intent": intent.id,
                 "place_kind": labels.get("place_kind")}))
