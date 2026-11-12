"""The desk: what the PFC sees in one wake, rebuilt for every wake (core.md §7)."""

from __future__ import annotations

from dataclasses import dataclass
from datetime import datetime
from typing import Any, Sequence

from monoagens.guidance import work_guidance
from monoagens.prior import constants
from monoagens.types import RECENT_KINDS, Event


@dataclass(frozen=True, slots=True)
class PFCContext:
    """Everything the PFC sees for one wake."""

    now: datetime
    place: str
    place_kind: str
    reason: str
    core_memory: str = ""
    people: Sequence[str] = ()
    expectations: Sequence[str] = ()
    memories: Sequence[str] = ()
    recent: Sequence[Event] = ()
    recent_elsewhere: Sequence[Event] = ()
    doing: Sequence[str] = ()
    trigger: Event | None = None
    last_interaction: datetime | None = None
    capabilities: Sequence[dict[str, Any]] = ()
    results: Sequence[str] = ()


def messages(soul: Any, context: PFCContext) -> list[dict[str, Any]]:
    """Build the layered prompt: stable, core memory with person cards, volatile."""
    return [
        {"role": "system", "content": stable(soul)},
        {"role": "system", "content": _semi_stable(context)},
        {"role": "user", "content": volatile(soul, context)},
    ]


def _semi_stable(context: PFCContext) -> str:
    """Return the semi-stable message: core memory, then the person cards (D55)."""
    block = f"<core_memory>{context.core_memory}</core_memory>"
    if context.people:
        block += "\n<people>\n" + "\n".join(context.people) + "\n</people>"
    return block


def stable(soul: Any) -> str:
    """Return the stable prompt: who the soul is and how it works."""
    parts = [f"You are {soul.name}.", str(soul.identity.get("persona") or "").strip()]
    principles = [str(p) for p in soul.identity.get("principles") or []]
    if principles:
        parts.append("Principles:\n" + "\n".join(f"- {p}" for p in principles))
    parts += [f"Language: {soul.identity.get('language') or 'en'}.", work_guidance()]
    return "\n\n".join(part for part in parts if part)


def volatile(soul: Any, context: PFCContext) -> str:
    """Return the volatile prompt: what is true now, trimmed to its token budgets."""
    local_now = context.now.astimezone(soul.timezone)
    values = constants(soul.settings)
    doing = _fit(list(context.doing), int(values["thread_result_tokens"]), keep_first=True)
    memories = _fit([str(item) for item in context.memories], int(values["memory_tokens"]))
    recent = _fit(_recent_lines(soul, context), int(values["recent_tokens"]))

    def build() -> list[str]:
        return [
            f"<now>{local_now:%Y-%m-%d %H:%M}, {local_now:%A}, {soul.timezone.key}, "
            "time since last interaction: "
            f"{_duration(context.now, context.last_interaction)}</now>",
            f"<place>{context.place} ({context.place_kind})</place>",
            f"<why_awake>{context.reason}</why_awake>",
            *_results(context),
            f"<doing>\n{_listed(doing)}\n</doing>",
            f"<expectations>{_listed(context.expectations)}</expectations>",
            f"<memories>\n{_listed(memories)}\n</memories>",
            f"<recent>\n{chr(10).join(recent)}\n</recent>",
            f"<event>{_event_text(context.trigger)}</event>",
        ]

    limit, parts = int(values["volatile_tokens_max"]), build()
    for bucket in (recent, memories):
        while _tokens("\n".join(parts)) > limit and len(bucket) > 1:
            bucket.pop(0)
            parts = build()
    return "\n".join(parts)


def _results(context: PFCContext) -> list[str]:
    """Return the previous rounds' results, shown only when the model already thought."""
    return [f"<results>\n{_listed(context.results)}\n</results>"] if context.results else []


def _recent_lines(soul: Any, context: PFCContext) -> list[str]:
    lines = [_recent_line(soul, event) for event in context.recent]
    lines += [_recent_line(soul, event, f"[{event.place}] ")
              for event in context.recent_elsewhere]
    return [line for line in lines if line] or ["(nothing yet)"]


def _recent_line(soul: Any, event: Event, prefix: str = "") -> str:
    if event.kind not in RECENT_KINDS:
        return ""
    speaker, text = _recent_parts(soul, event)
    return (f"{prefix}{event.ts.astimezone(soul.timezone):%H:%M} "
            f"{speaker}: {text}") if text else ""


def _recent_parts(soul: Any, event: Event) -> tuple[str, Any]:
    if event.kind == "act.out":
        return soul.name, (event.payload.get("arguments") or {}).get("text")
    if event.kind == "expectation.fired":
        return "expectation", (event.payload.get("text") or event.payload.get("about"))
    return str(event.who or "?"), event.payload.get("description")


def _event_text(event: Event | None) -> str:
    if event is None:
        return "(no trigger event)"
    text = event.payload.get("description") or event.payload.get("text")
    who = str(event.who or "?")
    line = f"{who}: {text}" if text else f"[{event.kind}]"
    notes = event.payload.get("notes")
    if notes:
        each = [notes] if isinstance(notes, str) else notes
        line += "\n" + "\n".join(f"note: {note}" for note in each)
    return line


def _listed(items: Sequence[str]) -> str:
    return "\n".join(str(item) for item in items) if items else "none"


def _tokens(text: str) -> int:
    """Approximate one section's token count from its characters."""
    return max(1, len(text) // 4)


def _fit(lines: list[str], limit: int, *, keep_first: bool = False) -> list[str]:
    """Drop the oldest lines until the section fits its token budget."""
    lines = list(lines)
    while len(lines) > 1 and sum(map(_tokens, lines)) > limit:
        if keep_first and len(lines) > 2:
            lines.pop(1)
        else:
            lines.pop(0)
    return lines


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
