"""Organs present now, the capabilities they declare, and action dispatch."""

from __future__ import annotations

from dataclasses import replace
from typing import Any

from monoagens import reach, threads
from monoagens.types import Action, Event


class OrganRegistry:
    """The bodies registered with the kernel and their capability descriptors."""

    def __init__(self) -> None:
        """Start with no organ present."""
        self.bodies: dict[str, Any] = {}
        self.capabilities: dict[str, dict[str, Any]] = {}

    def register(self, body: Any) -> None:
        """Register one body and every capability it declares."""
        name = getattr(body, "name", None)
        if not name:
            raise ValueError("A body must expose a name")
        self.bodies[name] = body
        for capability in getattr(body, "capabilities", lambda: ())():
            full = f"{name}.{capability['name']}"
            if full in self.capabilities:
                raise ValueError(f"Capability collision: {full}")
            self.capabilities[full] = {"organ": name, **capability}

    def add_capability(self, full: str, descriptor: dict[str, Any]) -> None:
        """Declare a capability the core itself dispatches, like `self.share`."""
        if full in self.capabilities:
            raise ValueError(f"Capability collision: {full}")
        self.capabilities[full] = {"organ": full.partition(".")[0], **descriptor}

    def descriptors(self) -> tuple[dict[str, Any], ...]:
        """Return the capability descriptors in registration order."""
        return tuple(self.capabilities.values())

    async def act(self, event: Event, capability: str, payload: dict[str, Any], *,
                  event_id: int | None = None) -> Any:
        """Hand one action to the organ that declared the capability."""
        organ, _, name = capability.partition(".")
        body = self.bodies.get(organ)
        if body is None:
            raise RuntimeError(f"No body registered for {organ!r}")
        action = Action(kind=name, place=event.place or "", payload=payload, body=organ,
                        event_id=event_id)
        return await body.act(action)


async def dispatch(kernel: Any, event: Event, args: dict[str, Any]) -> bool:
    """Resolve, guard and hand one `act` call to the organ that declares it (D2, D46)."""
    capability = str(args.get("capability") or "")
    cap = kernel.organs.capabilities.get(capability)
    if cap is None:
        await kernel._record_error(event, f"unknown capability: {capability!r}")
        return False
    arguments = args.get("arguments") if isinstance(args.get("arguments"), dict) else {}
    arguments = dict(arguments)
    routed = cap.get("communicative") or capability == reach.SHARE
    to, via = (arguments.pop("to", None), arguments.pop("via", None)) if routed else (None, None)
    if capability == reach.SHARE:
        allowed, source = await reach.consent(kernel.conn, event, arguments)
        if not allowed:
            await kernel._emit(event, "act.result", {"capability": capability, "to": to,
                               "result": {"error": "only someone who was part of it can "
                                                   "share it"}})
            return True
        ctx = {"conn": kernel.conn, "event": event, "now": kernel.clock.now(),
               "capability": capability, "origin_actor": event.origin_actor, "to": to,
               "identity_changing": cap.get("identity_changing")}
        payload = await kernel._run_hooks("action", arguments, ctx, event, organ="self")
        if payload is not None:
            await reach.grant(kernel, event, payload, to, source)
        return True
    target, error = await reach.resolve_to(kernel.conn, str(to), via) if to else (None, None)
    if to and target is None and str(to) == event.who:
        target, error = event.place, None  # the person is here: answer them here
    if error:
        await kernel._emit(event, "act.result",
                           {"capability": capability, "to": to, "result": {"error": error}})
        return True
    place = target or event.place
    if place != event.place and cap.get("communicative"):
        await _intend(kernel, event, capability, arguments, to, place)
        return True
    if await reach.leaks(kernel.conn, event.place, place, str(arguments.get("text") or "")):
        await kernel._emit(event, "act.result", {"capability": capability, "to": to, "result": {
            "error": "that place may not see this content; say it where it belongs"}})
        return True
    record = {"capability": capability, "arguments": arguments,
              "communicative": bool(cap.get("communicative"))}
    stored = await kernel._emit(event, "act.out", {**record, "to": to} if to else record,
                                place=place)
    await kernel._encode(stored, str(arguments.get("text") or "") or None)
    ctx = {"conn": kernel.conn, "event": event, "now": kernel.clock.now(),
           "capability": capability, "origin_actor": event.origin_actor, "to": to,
           "identity_changing": cap.get("identity_changing")}
    payload = await kernel._run_hooks("action", arguments, ctx, event,
                                      organ=capability.partition(".")[0])
    if payload is None:
        return True
    if cap.get("long_running"):
        await threads.open_for(kernel, event, await reach.audience(kernel.conn, event.place))
    result = await kernel.organs.act(replace(event, place=place), capability, payload,
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


async def _intend(kernel: Any, event: Event, capability: str, arguments: dict[str, Any],
                  to: str | None, place: str) -> None:
    """Record what to say elsewhere and wake the PFC there, with that place's desk (D48)."""
    intent = await kernel._emit(event, "act.intent", {
        "capability": capability, "arguments": arguments, "to": to, "target": place,
        "why": event.payload.get("text") or event.payload.get("description")})
    kernel.enqueue(Event(
        id=None, ts=kernel.clock.now(), kind="intent", who=None, place=place,
        caused_by=intent.id, origin_actor=event.origin_actor,
        payload={"description": f"carry out intent #{intent.id}: a message for "
                                f"{to or place}",
                 "capability": capability, "to": to, "intent": intent.id,
                 "place_kind": await reach.place_kind(kernel.conn, place)}))
