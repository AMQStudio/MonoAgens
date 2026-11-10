"""The access gate: one refusal per body, silent in shared rooms (ADR-0013 §4.4).

The gateway answers whether this person may talk here; the core renders the same notice for
every body and decides where a notice is allowed at all.
"""


import logging

from ..access.permissions import command_allowed
from ..i18n import t
from ..inbound import Address, GatewayCore, Percept
from .commands import command_body

log = logging.getLogger(__name__)


def command_permitted(agent, inbound: Percept, gateway: GatewayCore) -> bool:
    """A command the sender's role may run skips the gateway's conversation policy."""
    body = command_body(
        str(getattr(inbound.payload, "text", "") or ""),
        agent.soul.name,
        gateway.self_handles(),
    )
    if not body.startswith("/"):
        return False
    name = body[1:].partition(" ")[0].strip().lower()
    if not name:
        return False
    try:
        person = agent.identity.resolve(
            inbound.source or gateway.name,
            inbound.actor or agent._sender(inbound),
        )
    except Exception:
        log.exception("identity resolve failed source=%s", inbound.source or gateway.name)
        return False
    roles = agent.config.roles.for_person(
        person,
        room=f"{inbound.source or gateway.name}:{inbound.thread}",
        participants=agent._room_participants,
    )
    handler = agent.commands.get(name)
    if handler is not None and command_allowed(roles, handler.name):
        return True
    recipe = agent.recipes.get(name) if agent.recipes is not None else None
    return bool(recipe is not None and {role.name for role in roles} & set(recipe.roles))


def _person(agent, inbound: Percept, gateway: GatewayCore) -> str:
    """The canonical person for the notice window; the transport id when identity fails."""
    source = str(inbound.source or gateway.name)
    raw = str(inbound.actor or "") or agent._sender(inbound)
    try:
        return str(agent.identity.resolve(source, raw) or raw)
    except Exception:
        log.debug("access notice identity failed source=%s", source, exc_info=True)
        return raw


async def refused(
    agent, inbound: Percept, gateway: GatewayCore, *, claimed: dict | None
) -> bool:
    """One refusal for every body: the gateway decides, the core sends the same reply."""
    check = getattr(gateway, "access_refusal", None)
    if claimed is not None or not callable(check):
        return False
    message = inbound.payload
    if getattr(message, "outgoing", False) or bool(inbound.hints.get("self")):
        return False
    if command_permitted(agent, inbound, gateway):
        return False
    try:
        reason = str(check(inbound) or "")
    except Exception:
        log.exception(
            "access check failed gateway=%s chat=%s; refusing",
            getattr(gateway, "name", ""),
            inbound.thread,
        )
        reason = "check_failed"
    if not reason:
        return False
    if not bool(inbound.hints.get("dm")):
        # A shared room never hears a refusal; the gate keeps its conversation silent.
        agent.intake.observe_group(inbound, gateway, reason)
        return True
    person = _person(agent, inbound, gateway)
    if not agent.access_notices.claim(str(getattr(gateway, "name", "") or ""), person):
        # Inside the window the turn stays refused; only the notice is withheld.
        return True
    try:
        await agent.delivery.text(
            inbound.thread,
            t("agent.access_refused", agent.soul.language),
            gateway=gateway,
            origin=Address(str(getattr(gateway, "name", "") or ""), str(inbound.thread)),
        )
    except Exception:
        log.warning(
            "access refusal notice failed gateway=%s chat=%s",
            getattr(gateway, "name", ""),
            inbound.thread,
            exc_info=True,
        )
    return True
