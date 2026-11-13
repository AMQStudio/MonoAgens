"""Workspace: read, write, edit, grep, glob and run in a sandboxed folder per person."""

from __future__ import annotations

import shutil
from typing import Any

from monoagens.core.hooks import Continue, Replace, Stop

from . import base, guard, quota, sandbox, tools


def register(api: Any) -> None:
    """Add the tools, the two action rules, the workspace command and the `reader` point."""
    tool = tools.WorkspaceTool(api)
    api.add_tool(tool)
    api.add_middleware("action", "who", who_rule())
    api.add_middleware("action", "guard", guard_rule(api))
    api.add_command("workspace", "Reset your workspace: /workspace reset.",
                    workspace_command(api), permission="workspace.run")
    api.add_rule(
        "For multi-step workspace work, keep `progress.md` in your workspace (goal, done, "
        "next step, files involved); update it after each step and read it first when resuming."
    )
    sandbox.resolvers(api)  # a private or loopback resolver is refused at boot
    ready, reason = sandbox.available(api)
    if not ready:
        api.notes.append(f"workspace: run unavailable ({reason})")
        api.add_rule(f"The workspace sandbox is unavailable ({reason}); "
                     "reading and writing files still work.")
    elif shutil.which(sandbox.PASTA) is None:
        api.notes.append("workspace: network off (no pasta)")
        api.add_rule("The sandbox has no network: pasta is missing, so run only sees itself.")


def who_rule():
    """Action rule: tell the tools who is acting, as a key the tool pops before it acts."""

    async def rule(item: Any, ctx: Any) -> Any:
        """Annotate a workspace call with the chain's origin; every other tool is untouched."""
        if not str(ctx.get("capability") or "").startswith("workspace."):
            return Continue()
        who = str(ctx.get("origin_actor") or "")
        return Replace({**(item or {}), "_who": who})

    return rule


def guard_rule(api: Any):
    """Action rule: deny a blocked command and mark a hidden or wide one `sensitive`."""

    async def rule(item: Any, ctx: Any) -> Any:
        """Apply the two deterministic stages of the guard to `workspace.run`."""
        if ctx.get("capability") != "workspace.run" or ctx.get("refusal"):
            return Continue()
        verdict, reason = guard.screen(str((item or {}).get("command") or ""))
        if verdict == guard.DENY:
            ctx["refusal"] = f"sandbox command blocked: {reason}"
            return Stop(ctx["refusal"])
        if verdict == guard.PENDING:
            ctx["call_facts"] = set(ctx.get("call_facts") or ()) | {"sensitive"}
            ctx["approval_reason"] = "the command is not plainly safe"
        return Continue()

    return rule


def workspace_command(api: Any):
    """`/workspace reset` removes the caller's own sandbox layer."""

    async def handler(kernel: Any, event: Any, arguments: list[str]) -> str:
        """Reset the caller's layer, or explain the usage."""
        if not arguments or arguments[0].lower() != "reset":
            return "usage: /workspace reset"
        who = str(event.origin_actor or event.who)
        removed = quota.reset(api, who)
        return (f"workspace reset: {removed} files removed" if removed
                else "workspace reset: nothing to remove")

    return handler


__all__ = ["base", "guard", "quota", "register", "sandbox", "tools"]
