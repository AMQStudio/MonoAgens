"""Soul Tuning: modes and example conversations that tune how the soul speaks.

The mode is state, not configuration: the latest `soul-tuning.mode_changed` wins, then the
`mode` option, then `daily`. `/mode` records the change, records one bounded knob offset per
knob, and asks the middleware to recompute the effective knobs at once.
"""

from __future__ import annotations

from dataclasses import replace
from typing import Any

from monoagens.contract import ContextItem
from monoagens.core.hooks import Continue, Replace
from monoagens.infra.store import append_event

from . import modes

# An event kind holds only lower-case words and dots, so the hyphen becomes an underscore;
# the payload still names the plugin as `soul-tuning` (that is the source, not a kind).
CHANGED = "soul_tuning.mode_changed"
UPDATED = "disposition.updated"
BLOCK = "soul-tuning"
BLOCK_ORDER = 1
PERMISSION = "soul-tuning.mode"


def register(api: Any) -> None:
    """Add the `<soul_tuning>` block and the owner's `/mode` command."""
    modes.from_options(getattr(api, "options", {}) or {})  # a bad option fails at boot
    api.add_middleware("context", "tuning", block(api))
    api.add_command("mode", "Switch how the soul speaks: /mode daily|professional.",
                    command(api), permission=PERMISSION)


def block(api: Any):
    """Return the context rule that renders the current mode, its rule and its examples."""

    async def rule(item: Any, ctx: Any) -> Any:
        """Add one volatile block: the mode first, then its rule, then the examples."""
        mode = await current_mode(ctx.get("conn"), api)
        examples = modes.from_options(getattr(api, "options", {}) or {})
        note = ContextItem(section="notes", kind=BLOCK, text=render(mode, examples))
        return Replace(replace(item, items=(*item.items, note)))

    return rule


def render(mode: str, examples: dict[str, list[tuple[str, str]]] | None = None) -> str:
    """Render the block: mode first, its rule, what the examples are, then the examples."""
    found = examples or modes.EXAMPLES
    pairs = found.get(mode) or found[modes.DEFAULT_MODE]
    lines = ["<soul_tuning>", f"mode: {mode}", modes.MODES[mode]["rule"],
             "These examples show the style, not the language: answer in the speaker's "
             "language.", "Examples:"]
    lines += [f"{speaker}: {text}" for speaker, text in pairs]
    lines.append("</soul_tuning>")
    return "\n".join(lines)


async def current_mode(conn: Any, api: Any = None) -> str:
    """Return the current mode: the last change, then the option, then `daily`."""
    if conn is not None:
        row = await (await conn.execute(
            "SELECT payload->>'mode' FROM events WHERE kind = %s ORDER BY id DESC LIMIT 1",
            (CHANGED,))).fetchone()
        if row and row[0] in modes.MODES:
            return str(row[0])
    option = str((getattr(api, "options", {}) or {}).get("mode") or "")
    return option if option in modes.MODES else modes.DEFAULT_MODE


def command(api: Any):
    """Return the `/mode` handler the registry calls in the inbound hook."""

    async def handler(kernel: Any, event: Any, arguments: list[str]) -> str:
        """Show the mode, or switch it and record the offsets."""
        wanted = arguments[0].strip().lower() if arguments else ""
        if not wanted:
            mode = await current_mode(kernel.conn, api)
            return f"mode: {mode} ({'|'.join(modes.NAMES)})"
        if wanted not in modes.MODES:
            return f"mode must be {' or '.join(modes.NAMES)}"
        who = str(event.origin_actor or event.who)
        await append_event(kernel.conn, ts=kernel.clock.now(), kind=CHANGED, who=who,
                           place=event.place, body=event.body, caused_by=event.id,
                           payload={"mode": wanted, "by": who})
        for knob, offset in modes.MODES[wanted]["offsets"].items():
            before = float((kernel.settings.get("knobs") or {}).get(knob) or 0.0)
            await append_event(
                kernel.conn, ts=kernel.clock.now(), kind=UPDATED, who=who, place=event.place,
                body=event.body, caused_by=event.id,
                payload={"source": BLOCK, "knob": knob, "offset": offset, "from": before,
                         "to": max(0.0, min(1.0, before + offset))})
        from monoagens.middleware import policy  # local: the middleware imports the loader

        await policy.apply_offsets(kernel)
        return f"mode: {await current_mode(kernel.conn, api)}"

    return handler


__all__ = ["Continue", "block", "command", "current_mode", "register", "render"]
