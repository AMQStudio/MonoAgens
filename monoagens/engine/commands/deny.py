"""Command /deny: cancel a pending approval without running it."""


from ..i18n import t
from ..registry import Handler
from . import CommandContext
from .approve import decision_refusal


async def _deny(ctx: CommandContext) -> str:
    agent = ctx.agent
    lang = ctx.lang
    raw = ctx.args.strip()
    key, _, reason = raw.partition(" ")
    if key.lower() in {"last", "terakhir", ""}:
        approval = agent.approvals.latest(ctx.thread_id, ctx.sender)
    else:
        approval = agent.approvals.get(key.lstrip("#"))
    if approval is None:
        return t("command.deny.none", lang)
    if decision_refusal(
        approval, thread_id=ctx.thread_id, sender=ctx.sender, roles=ctx.roles
    ):
        return t("command.deny.foreign", lang)
    if approval.status != "pending":
        return t("command.deny.already", lang, id=approval.id, status=approval.status)
    if approval.kind in ("privacy", "disclosure"):
        # Declining a privacy row forgets the memory; it is not a tool cancellation.
        text = await agent.privacy.decide(approval, approved=False, sender=ctx.sender)
        return f"{t('command.deny.done', lang, id=approval.id)}\n{text}"
    agent.approvals.mark(approval.id, "denied", ctx.sender, reason.strip())
    return t("command.deny.done", lang, id=approval.id)


COMMAND = Handler(
    name="deny",
    description="Cancel an action that is waiting for approval.",
    fn=_deny,
    usage="[id|last] [reason]",
)
