"""Command /deny: cancel a pending approval without running it."""


from ..i18n import t
from ..registry import Handler
from . import CommandContext


async def _deny(ctx: CommandContext) -> str:
    from ..turn.decisions import decide

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
    text, _ran = await decide(
        agent,
        approval,
        False,
        sender=ctx.sender,
        roles=ctx.roles,
        gateway=ctx.gateway,
        thread_id=ctx.thread_id,
        reason=reason,
    )
    return text


COMMAND = Handler(
    name="deny",
    description="Cancel an action that is waiting for approval.",
    fn=_deny,
    usage="[id|last] [reason]",
)
