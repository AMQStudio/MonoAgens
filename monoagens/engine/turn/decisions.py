"""One decision contract for every route: `/approve`, `/deny`, and a gateway gesture.

ADR-0013 §4.4: a decision is data (`Decision`), never text parsed back out of chat. The core
validates every one of them the same way: the row exists, the person may decide it in this
room, the row is still pending and unexpired, then the action runs (or is cancelled).
"""


import logging
import time

from .. import io
from ..i18n import t
from ..inbound import Address, Decision, Percept

log = logging.getLogger(__name__)


async def decide(
    agent,
    approval,
    approve: bool,
    *,
    sender: str,
    roles,
    gateway,
    thread_id: str,
    mode: str = "",
    reason: str = "",
) -> tuple[str, bool]:
    """Decide one pending row: `(chat text, did an action run)`.

    Shared by the typed commands and every gesture. `mode` is the relay choice only the typed
    `/approve <id> <mode>` carries; `reason` is the text only `/deny` records.
    """
    # Local import: `commands.approve` imports this module at load time.
    from ..commands.approve import decision_refusal, relay_decision, run_approval

    lang = str(agent.soul.language)
    if approval is None:
        return t("command.approve.not_found", lang, id=""), False
    if decision_refusal(approval, thread_id=thread_id, sender=sender, roles=roles):
        return t("command.approve.foreign", lang), False
    if approval.status != "pending":
        return t("command.approve.already", lang, id=approval.id, status=approval.status), False
    if approval.expires_at < time.time():
        await io.run(agent.approvals.mark, approval.id, "expired", sender)
        return t("command.approve.expired", lang, id=approval.id), False
    if approve:
        if approval.tool == "relay":
            text = await relay_decision(
                agent, approval, extra=mode, sender=sender, lang=lang
            )
            return text, False
        return await run_approval(
            agent,
            approval,
            sender=sender,
            roles=roles,
            gateway=gateway,
            thread_id=thread_id,
        )
    if approval.kind in ("privacy", "disclosure"):
        # Declining a privacy row forgets the memory; it is not a tool cancellation.
        text = await agent.privacy.decide(approval, approved=False, sender=sender)
        return f"{t('command.deny.done', lang, id=approval.id)}\n{text}", False
    await io.run(agent.approvals.mark, approval.id, "denied", sender, str(reason or "").strip())
    return t("command.deny.done", lang, id=approval.id), False


class Decisions:
    """The Agent's decision seam: resolve the person, decide the row, send one receipt."""

    def __init__(self, agent) -> None:
        self.agent = agent

    async def decide(
        self,
        approval,
        approve: bool,
        *,
        sender: str,
        roles,
        gateway,
        thread_id: str,
        mode: str = "",
        reason: str = "",
    ) -> tuple[str, bool]:
        return await decide(
            self.agent,
            approval,
            approve,
            sender=sender,
            roles=roles,
            gateway=gateway,
            thread_id=thread_id,
            mode=mode,
            reason=reason,
        )

    async def receive(self, inbound: Percept, gateway) -> None:
        """A `decision` percept: no turn, no memory candidate, no model call (ADR-0013 §4.4)."""
        agent = self.agent
        found = inbound.hints.get("decision")
        if not isinstance(found, Decision):
            return
        source = str(inbound.source or gateway.name)
        raw = (
            str(inbound.actor or "")
            or str(getattr(inbound.payload, "sender_id", "") or "")
            or str(inbound.thread)
        )
        try:
            person = agent.identity.resolve(source, raw)
        except Exception:
            log.exception("decision identity resolve failed source=%s", source)
            return
        roles = agent.config.roles.for_person(
            person,
            room=f"{source}:{inbound.thread}",
            participants=agent._room_participants,
        )
        thread = str(inbound.thread)
        approval = await io.run(agent.approvals.get, found.approval_id)
        text, _ran = await decide(
            agent,
            approval,
            found.approve,
            sender=person,
            roles=roles,
            gateway=gateway,
            thread_id=thread,
        )
        await agent.delivery.text(
            thread,
            text,
            gateway=gateway,
            subject=person,
            origin=Address(str(gateway.name), thread),
            receipt_actor=person,
        )
