"""Action dispatch: the door between one `act` call and the tool that carries it out.

The door checks the arguments against the descriptor, asks the action hook (which resolves
the route and applies the absolute lines), records the act and its action in the ledger
before it happens, hands it to the adapter or plugin, and records exactly one receipt. It
reads only opaque ids, times and the outcome the hook returned.
"""

from __future__ import annotations

import asyncio
from dataclasses import dataclass, replace
from typing import Any

from monoagens import contract
from monoagens.core import arguments, threads
from monoagens.core.context import working
from monoagens.infra.store import project_event
from monoagens.types import Event


@dataclass(frozen=True, slots=True)
class JobRef:
    """Which reasoning step is acting: the job it belongs to and its round."""

    key: str
    started_event_id: int


@dataclass(frozen=True, slots=True)
class Dispatch:
    """What one `act` did: whether it acted, where it landed, and its receipt."""

    acted: bool
    place: str = ""
    receipt: contract.Receipt | None = None


@dataclass(frozen=True, slots=True)
class Act:
    """One recorded act: its action, its `act.out` event, and whether it is the act that speaks."""

    action: contract.Action
    out: Event
    communicative: bool = False


def action_for(job: JobRef | None, event: Event, round_index: int, n: int,
               capability: str, place: str, passed: dict[str, Any],
               caused_by: int | None) -> contract.Action:
    """Return the action one call becomes, with the id its body deduplicates on."""
    ref = job if job is not None else JobRef("hook", int(event.id or 0))
    return contract.Action(id=contract.action_id(ref.key, ref.started_event_id,
                                                 round_index, n),
                           capability=capability, place=place, arguments=dict(passed),
                           caused_by=caused_by)


def as_receipt(capability: str, raw: Any) -> contract.Receipt:
    """Answer with the receipt one tool result means (contract.md §3)."""
    if isinstance(raw, contract.Receipt):
        return raw
    if isinstance(raw, dict) and raw.get("error"):
        return contract.Receipt("failed", reason=str(raw["error"]))
    return contract.Receipt("done", result=raw)


async def dispatch(kernel: Any, event: Event, args: dict[str, Any], *,
                   said: set[str] | None = None, job: JobRef | None = None,
                   round_index: int = 0, n: int = 0) -> Dispatch:
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
        receipt = contract.Receipt("refused", reason=f"invalid arguments for {capability}: "
                                                     f"{arguments.refusal(broken)}")
        act = await record(kernel, event, job, round_index, n, capability,
                           event.place or "", raw, cap)
        await kernel._settle_action(event, act, receipt)
        return Dispatch(acted=True, receipt=receipt)
    routing = kernel.tools.routing_keys(cap)
    passed = {key: value for key, value in raw.items() if key not in routing}
    asked = dict(passed)  # what reasoning asked for, before the hook had its say
    ctx = {"conn": kernel.conn, "event": event, "now": kernel.clock.now(),
           "capability": capability, "descriptor": cap, "origin_actor": event.origin_actor,
           "arguments": dict(raw), "to": raw.get("to"), "via": raw.get("via"),
           "place": None, "refusal": None, "handled": False, "held": False,
           "long_running": bool(cap.get("long_running")),
           "identity_changing": bool(cap.get("identity_changing"))}
    payload = await kernel._run_hooks("action", passed, ctx, event,
                                      tool=capability.partition(".")[0])
    if ctx.get("refusal"):
        receipt = contract.Receipt("held" if ctx.get("held") else "refused",
                                   reason=str(ctx["refusal"]))
        act = await record(kernel, event, job, round_index, n, capability,
                           event.place or "", dict(payload if payload else passed), cap,
                           to=ctx["to"])
        await kernel._settle_action(event, act, receipt)
        return Dispatch(acted=True, receipt=receipt)
    if payload is None or ctx.get("handled"):
        return Dispatch(acted=True, place=ctx.get("place") or "")
    place, to = ctx.get("place") or event.place, ctx.get("to")
    if cap.get("communicative") and said is not None and place in said:
        receipt = contract.Receipt(
            "refused", reason=f"already said something to {to or place} this turn")
        act = await record(kernel, event, job, round_index, n, capability, place, passed,
                           cap, to=to)
        await kernel._settle_action(event, act, receipt)
        return Dispatch(acted=True, place=place, receipt=receipt)
    if place != event.place:
        await _intend(kernel, event, capability, dict(payload), asked, to, place)
        if said is not None:
            said.add(place)
        return Dispatch(acted=True, place=place)
    act = await record(kernel, event, job, round_index, n, capability, place,
                       dict(payload), cap, to=to)
    if cap.get("communicative") and said is not None:
        said.add(place)
    await kernel._encode(act.out, str(passed.get("text") or "") or None)
    if cap.get("long_running"):
        await threads.open_for(kernel, event)
    receipt = await carry(kernel, act)
    await kernel._settle_action(event, act, receipt)
    if (cap.get("identity_changing") and receipt.state == "done"
            and isinstance(receipt.result, dict)
            and isinstance(receipt.result.get("identity"), dict)):
        await _project_identity(kernel, event, act, receipt.result["identity"])
    return Dispatch(acted=True, place=ctx.get("place") or "", receipt=receipt)


async def record(kernel: Any, event: Event, job: JobRef | None, round_index: int, n: int,
                 capability: str, place: str, passed: dict[str, Any], cap: Any, *,
                 to: str | None = None) -> Act:
    """Record one act and its ledger row before its body sees it (D-F)."""
    index = n if job is not None else kernel.next_hook_act(event.id)
    action = action_for(job, event, round_index, index, capability, place, passed,
                        caused_by=event.id)
    record_payload = {"capability": capability, "arguments": dict(passed),
                      "communicative": bool(cap.get("communicative"))}
    out = await kernel._emit_act(event, {**record_payload, "to": to} if to else record_payload,
                                 action=action, communicative=record_payload["communicative"],
                                 place=place, description=str(passed.get("text") or ""))
    return Act(action=action, out=out, communicative=record_payload["communicative"])


async def carry(kernel: Any, act: Act) -> contract.Receipt:
    """Hand one recorded action to its body under the job watchdog (F.2).

    A tool that overruns is a `failed` receipt with its timeout named, never a hung turn.
    """
    trigger = replace(act.out, place=act.action.place)
    timeout = getattr(getattr(kernel, "jobs", None), "timeout", None)
    try:
        coro = kernel.tools.act(trigger, act.action.capability,
                                dict(act.action.arguments), event_id=act.out.id,
                                action_id=act.action.id)
        raw = await (asyncio.wait_for(coro, timeout) if timeout else coro)
    except asyncio.TimeoutError:
        return contract.Receipt(
            "failed", reason=f"{act.action.capability} timed out after {timeout:g}s")
    except Exception as exc:
        return contract.Receipt(
            "failed", reason=f"{act.action.capability} failed ({type(exc).__name__}): {exc}")
    return as_receipt(act.action.capability, raw)


async def _project_identity(kernel: Any, event: Event, act: Act,
                            new_identity: dict[str, Any]) -> None:
    """Project an identity change one act carries, once per act."""
    row = await (await kernel.conn.execute(
        "SELECT 1 FROM events WHERE kind = 'identity.updated' AND caused_by = %s LIMIT 1",
        (act.out.id,))).fetchone()
    if row is None:
        updated = await kernel._emit(act.out, "identity.updated",
                                     {"identity": new_identity}, place=act.action.place)
        await project_event(kernel.conn, updated)
    if kernel.pfc:
        kernel.pfc.identity = new_identity
        kernel.pfc.name = str(new_identity.get("name") or "")
    kernel.settings = {**kernel.settings, "identity": new_identity}


async def _intend(kernel: Any, event: Event, capability: str, quoted: dict[str, Any],
                  recorded: dict[str, Any], to: str | None, place: str) -> None:
    """Record what to say elsewhere and wake the reasoning there with the intent itself (D48).

    The line the other place's desk shows quotes only what the action hook let through
    (`quoted`); the intent's own arguments are the ones reasoning asked for, so the Log keeps
    the drafted words even when the desk hides them (D45, 2.8).
    """
    intent = await kernel._emit(event, "act.intent", {
        "capability": capability, "arguments": dict(recorded), "to": to, "target": place})
    words = str(quoted.get("text") or quoted.get("item") or "")
    reason = (f"you set out to say {words!r} to {to or place}" if words else
              f"you set out to send a message to {to or place}; its words cannot appear here")
    kernel.enqueue(Event(
        id=None, ts=kernel.clock.now(), kind="intent", who=None, place=place,
        caused_by=intent.id, origin_actor=event.origin_actor,
        description=f"{reason} (from {event.place})",
        payload={"capability": capability, "to": to, "intent": intent.id}))
