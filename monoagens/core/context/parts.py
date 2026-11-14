"""The three messages, rendered from the context's items after the last hook has run.

The stable prefix is who the soul is and how it works; the semi-stable one is core memory, the
person cards and what is installed; the volatile one is what is true now. No hook ever sees this
text: it renders whatever items survived the context hook (contract.md §3).
"""

from __future__ import annotations

from collections.abc import Sequence
from datetime import datetime
from typing import Any

from monoagens.contract import ContextItem
from monoagens.core.context.layout import PFCContext
from monoagens.core.principles import work_guidance
from monoagens.infra.prior import constants

# The order the volatile sections render in, after the clock, the place and the reason.
VOLATILE_SECTIONS = ("doing", "expectations", "memories", "notes", "recent", "trigger")

_WRAPPERS = {
    "doing": ("<doing>\n", "\n</doing>"),
    "expectations": ("<expectations>", "</expectations>"),
    "memories": ("<memories>\n", "\n</memories>"),
    "notes": ("<notes>\n", "\n</notes>"),
    "core_memory": ("<core_memory>", "</core_memory>"),
    "trigger": ("<event>", "</event>"),
}

NOTHING_YET = "(nothing yet)"

RECENT_WRAPPER = ("<recent>\n", "\n</recent>")

EMPTY = "none"


def messages(soul: Any, context: PFCContext) -> list[dict[str, Any]]:
    """Build the three messages from the context: stable, semi-stable, volatile."""
    return [
        {"role": "system", "content": "\n\n".join(_stable(soul, context))},
        {"role": "system", "content": "\n".join(_semi(soul, context))},
        {"role": "user", "content": _volatile(soul, context)},
    ]


def _stable(soul: Any, context: PFCContext) -> list[str]:
    """Return the cached prefix: who the soul is and how it works."""
    return [text for text in (_identity(soul, context), work_guidance()) if text]


def _semi(soul: Any, context: PFCContext) -> list[str]:
    """Return the rarely changing block: core memory, the person cards and what is installed."""
    sections = _grouped(context.items)
    parts = [_wrap("core_memory", [item.text for item in sections.get("core_memory") or ()])
             if sections.get("core_memory") else ""]
    cards = [item.text for item in sections.get("cards") or ()]
    if cards:
        parts.append("\n<people>\n" + "\n".join(cards) + "\n</people>")
    parts.append(context.self_text)
    return [text for text in parts if text]


def _identity(soul: Any, context: PFCContext) -> str:
    """Return who the soul is: its name, persona, principles and language."""
    parts = [f"You are {soul.name}.", str(soul.identity.get("persona") or "").strip()]
    principles = [str(item) for item in soul.identity.get("principles") or []]
    if principles:
        parts.append("Principles:\n" + "\n".join(f"- {item}" for item in principles))
    parts.append(f"Language: {soul.identity.get('language') or 'en'}.")
    return "\n\n".join(part for part in parts if part)


def _volatile(soul: Any, context: PFCContext) -> str:
    """Return the volatile message: what is true now, trimmed to its token budgets."""
    sections = _fit_sections(soul, _grouped(context.items))
    texts = [
        f"<now>{_clock(soul, context)}</now>",
        f"<place>{context.place}</place>",
        f"<why_awake>{context.reason}</why_awake>",
    ]
    if context.results:
        texts.append(f"<results>\n{_listed(context.results)}\n</results>")
    texts.extend(_render_sections(sections))
    return "\n".join(text for text in texts if text)


def _clock(soul: Any, context: PFCContext) -> str:
    """Return the local time, the zone and the time since the last interaction."""
    local = context.now.astimezone(soul.timezone)
    return (f"{local:%Y-%m-%d %H:%M}, {local:%A}, {soul.timezone.key}, "
            "time since last interaction: "
            f"{_duration(context.now, context.last_interaction)}")


def _render_sections(sections: dict[str, list[ContextItem]]) -> list[str]:
    """Return every volatile section's text, in the fixed order."""
    texts: list[str] = []
    for name in VOLATILE_SECTIONS:
        held = [item.text for item in sections.get(name) or ()]
        if name == "recent":
            texts.append(_render_recent(sections))
        elif name == "notes":
            texts.append(_notes(held))
        else:
            texts.append(_wrap(name, held))
    return [text for text in texts if text]


def _notes(lines: Sequence[str]) -> str:
    """Return the notes section, which exists only when a rule added one."""
    start, end = _WRAPPERS["notes"]
    kept = [line for line in lines if line]
    return f"{start}{chr(10).join(kept)}{end}" if kept else ""


def _render_recent(sections: dict[str, list[ContextItem]]) -> str:
    """Return this place's recent lines together with the ones from elsewhere."""
    lines = [item.text for name in ("recent", "elsewhere")
             for item in sections.get(name) or ()]
    start, end = RECENT_WRAPPER
    return f"{start}{chr(10).join(lines) if lines else NOTHING_YET}{end}"


def _wrap(name: str, texts: Sequence[str]) -> str:
    """Wrap one section's lines, or return "" when the section has no lines."""
    if name not in _WRAPPERS:
        lines = [text for text in texts if text]
        return "\n".join(lines)
    start, end = _WRAPPERS[name]
    return f"{start}{_listed(texts)}{end}"


def _grouped(items: Sequence[ContextItem]) -> dict[str, list[ContextItem]]:
    """Group the context's items by their section, keeping their order."""
    grouped: dict[str, list[ContextItem]] = {}
    for item in items:
        grouped.setdefault(item.section, []).append(item)
    return grouped


def _fit_sections(soul: Any, sections: dict[str, list[ContextItem]],
                  ) -> dict[str, list[ContextItem]]:
    """Trim every section to its own token budget and the volatile message to its own."""
    values = constants(soul.settings)
    sections = dict(sections)
    sections["doing"] = _fit(sections.get("doing") or [],
                             int(values["thread_result_tokens"]), keep_first=True)
    sections["memories"] = _fit(sections.get("memories") or [], int(values["memory_tokens"]))
    sections = _fit_recent(sections, int(values["recent_tokens"]))
    limit = int(values["volatile_tokens_max"])
    while _tokens("\n".join(_render_sections(sections))) > limit:
        if not _drop_oldest(sections, "recent"):
            break
    return sections


def _fit_recent(sections: dict[str, list[ContextItem]], limit: int,
                ) -> dict[str, list[ContextItem]]:
    """Drop the oldest recent lines until the section fits its own budget."""
    sections = dict(sections)
    while (len(sections.get("recent") or ()) + len(sections.get("elsewhere") or ())) > 1 and \
            sum(map(_tokens, (item.text for item in
                              (sections.get("recent") or []) + (sections.get("elsewhere") or [])))) > limit:
        _drop_oldest(sections, "recent")
    return sections


def _drop_oldest(sections: dict[str, list[ContextItem]], name: str) -> bool:
    """Drop the oldest item of one section, then the other place's, deepest section first."""
    order = ("recent", "elsewhere") if name == "recent" else (name, "memories")
    for key in order:
        held = sections.get(key) or []
        if len(held) > 1 or (key != name and held):
            sections[key] = held[1:]
            return True
    return False


def _listed(lines: Sequence[str]) -> str:
    return "\n".join(str(line) for line in lines) if lines else EMPTY


def _tokens(text: str) -> int:
    """Approximate one section's token count from its characters."""
    return max(1, len(text) // 4)


def _fit(items: list[ContextItem], limit: int, *, keep_first: bool = False) -> list[ContextItem]:
    """Drop the oldest items until the section fits its token budget."""
    items = list(items)
    while len(items) > 1 and sum(map(_tokens, (item.text for item in items))) > limit:
        if keep_first and len(items) > 2:
            items.pop(1)
        else:
            items.pop(0)
    return items


def _duration(now: datetime, then: datetime | None) -> str:
    if then is None:
        return "first interaction"
    minutes = max(0, int((now - then).total_seconds() // 60))
    days, rest = divmod(minutes, 1440)
    hours, mins = divmod(rest, 60)
    if days:
        return f"{days}d {hours}h"
    if hours:
        return f"{hours}h {mins}m"
    return f"{minutes}m"
