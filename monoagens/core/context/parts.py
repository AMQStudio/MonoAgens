"""The context parts: every prompt block declared once, with its layer and its order.

A part is either `stable` (the cached prefix: who the soul is and how it works), `semi`
(the rarely changing block: core memory and the person cards) or `volatile` (what is true
now). The middleware and plugins add volatile blocks through the context hook; nothing
else may touch the prefix.
"""

from __future__ import annotations

from collections.abc import Callable, Sequence
from dataclasses import dataclass, replace
from datetime import datetime
from typing import Any

from monoagens.core.context.layout import PFCContext, withheld_note
from monoagens.core.principles import work_guidance
from monoagens.infra.prior import constants
from monoagens.core.kinds import RECENT_KINDS
from monoagens.types import Event

STABLE, SEMI, VOLATILE = "stable", "semi", "volatile"
_SEPARATORS = {STABLE: "\n\n", SEMI: "\n", VOLATILE: "\n"}


@dataclass(frozen=True, slots=True)
class Part:
    """One block of the prompt: its layer, its order, and how it produces its text."""

    name: str
    layer: str
    order: int
    produce: Callable[[Any, PFCContext], str] | None = None
    text: str = ""

    @property
    def stable(self) -> bool:
        """Return whether this part belongs to the cached prefix."""
        return self.layer == STABLE


def _identity(soul: Any, context: PFCContext) -> str:
    """Return who the soul is: its name, persona, principles and language."""
    parts = [f"You are {soul.name}.", str(soul.identity.get("persona") or "").strip()]
    principles = [str(item) for item in soul.identity.get("principles") or []]
    if principles:
        parts.append("Principles:\n" + "\n".join(f"- {item}" for item in principles))
    parts.append(f"Language: {soul.identity.get('language') or 'en'}.")
    return "\n\n".join(part for part in parts if part)


def _principles(soul: Any, context: PFCContext) -> str:
    """Return the general instructions a soul gets with every prompt."""
    return work_guidance()


def _core_memory(soul: Any, context: PFCContext) -> str:
    """Return the always-visible core memory."""
    return f"<core_memory>{context.core_memory}</core_memory>"


def _people(soul: Any, context: PFCContext) -> str:
    """Return the person cards the middleware attached, if there are any (D55)."""
    if not context.people:
        return ""
    return "\n<people>\n" + "\n".join(context.people) + "\n</people>"


def _now(soul: Any, context: PFCContext) -> str:
    """Return the local time, the zone and the time since the last interaction."""
    local = context.now.astimezone(soul.timezone)
    return (f"<now>{local:%Y-%m-%d %H:%M}, {local:%A}, {soul.timezone.key}, "
            "time since last interaction: "
            f"{_duration(context.now, context.last_interaction)}</now>")


def _place(soul: Any, context: PFCContext) -> str:
    """Return the place this wake belongs to and the kind the middleware attached."""
    return f"<place>{context.place} ({context.place_kind})</place>"


def _why_awake(soul: Any, context: PFCContext) -> str:
    """Return why this wake happened."""
    return f"<why_awake>{context.reason}</why_awake>"


def _results(soul: Any, context: PFCContext) -> str:
    """Return the previous rounds' results, shown only when the model already thought."""
    if not context.results:
        return ""
    return f"<results>\n{_listed(context.results)}\n</results>"


def _doing(soul: Any, context: PFCContext) -> str:
    """Return every chain in progress (D52)."""
    return f"<doing>\n{_listed(context.doing)}\n</doing>"


def _expectations(soul: Any, context: PFCContext) -> str:
    """Return the active expectations of this place."""
    return f"<expectations>{_listed(context.expectations)}</expectations>"


def _memories(soul: Any, context: PFCContext) -> str:
    """Return the memories retrieved for this wake."""
    return f"<memories>\n{_listed(context.memories)}\n</memories>"


def _withheld(soul: Any, context: PFCContext) -> str:
    """Return the withheld note as one section, if anything was withheld."""
    note = withheld_note(context)
    return f"<withheld>{note}</withheld>" if note else ""


def _recent(soul: Any, context: PFCContext) -> str:
    """Return the recent lines of this place and of the others."""
    return f"<recent>\n{chr(10).join(_recent_lines(soul, context))}\n</recent>"


def _event(soul: Any, context: PFCContext) -> str:
    """Return the trigger event as the line that woke the soul."""
    return f"<event>{_event_text(context.trigger)}</event>"


CORE_PARTS: tuple[Part, ...] = (
    Part("identity", STABLE, 10, _identity),
    Part("principles", STABLE, 20, _principles),
    Part("core_memory", SEMI, 10, _core_memory),
    Part("people", SEMI, 20, _people),
    Part("now", VOLATILE, 10, _now),
    Part("place", VOLATILE, 20, _place),
    Part("why_awake", VOLATILE, 30, _why_awake),
    Part("results", VOLATILE, 40, _results),
    Part("doing", VOLATILE, 50, _doing),
    Part("expectations", VOLATILE, 60, _expectations),
    Part("memories", VOLATILE, 70, _memories),
    Part("withheld", VOLATILE, 80, _withheld),
    Part("recent", VOLATILE, 90, _recent),
    Part("event", VOLATILE, 100, _event),
)


def messages(soul: Any, context: PFCContext) -> list[dict[str, Any]]:
    """Build the three messages from the parts: stable, semi-stable, volatile."""
    blocks = _extension_blocks(context.blocks)
    return [
        {"role": "system", "content": _SEPARATORS[STABLE].join(_texts(soul, context, STABLE))},
        {"role": "system", "content": _SEPARATORS[SEMI].join(_texts(soul, context, SEMI))},
        {"role": "user", "content": _volatile(soul, context, blocks)},
    ]


def _extension_blocks(blocks: Sequence[Part]) -> tuple[Part, ...]:
    """Return the middleware's blocks; a prefix block or a taken name is refused."""
    names = {part.name for part in CORE_PARTS}
    found = []
    for block in blocks:
        if block.layer != VOLATILE:
            raise ValueError(f"an extension block must be volatile: {block.name!r}")
        if block.name in names:
            raise ValueError(f"duplicate block name: {block.name!r}")
        names.add(block.name)
        found.append(block)
    return tuple(sorted(found, key=lambda part: part.order))


def _texts(soul: Any, context: PFCContext, layer: str,
           blocks: Sequence[Part] = ()) -> list[str]:
    """Return the non-empty texts of one layer's parts, in order."""
    parts = [part for part in CORE_PARTS if part.layer == layer]
    if layer == VOLATILE:
        parts += list(blocks)
    return [text for text in (_render(part, soul, context) for part in parts) if text]


def _render(part: Part, soul: Any, context: PFCContext) -> str:
    """Return one part's text: its producer's, or its own literal text."""
    return part.produce(soul, context) if part.produce is not None else part.text


def _volatile(soul: Any, context: PFCContext, blocks: Sequence[Part]) -> str:
    """Return the volatile message: every volatile part, trimmed to its token budgets."""
    values = constants(soul.settings)
    view = replace(
        context,
        doing=tuple(_fit(list(context.doing), int(values["thread_result_tokens"]),
                         keep_first=True)),
        memories=tuple(_fit([str(item) for item in context.memories],
                            int(values["memory_tokens"]))))
    view = _trim_recent(soul, view, int(values["recent_tokens"]))
    limit = int(values["volatile_tokens_max"])
    texts = _texts(soul, view, VOLATILE, blocks)
    for name in ("recent", "memories"):
        while _tokens("\n".join(texts)) > limit and _count(view, name) > 1:
            view = _drop_oldest(view, name)
            texts = _texts(soul, view, VOLATILE, blocks)
    return "\n".join(texts)


def _count(view: PFCContext, name: str) -> int:
    """Return how many items one trimmed section holds."""
    if name == "recent":
        return len(view.recent) + len(view.recent_elsewhere)
    return len(view.memories)


def _drop_oldest(view: PFCContext, name: str) -> PFCContext:
    """Return the view with the oldest item of one section dropped."""
    if name == "memories":
        return replace(view, memories=tuple(view.memories[1:]))
    if view.recent:
        return replace(view, recent=tuple(view.recent[1:]))
    return replace(view, recent_elsewhere=tuple(view.recent_elsewhere[1:]))


def _trim_recent(soul: Any, view: PFCContext, limit: int) -> PFCContext:
    """Drop the oldest recent lines until the section fits its own budget."""
    while _count(view, "recent") > 1 and \
            sum(map(_tokens, _recent_lines(soul, view))) > limit:
        view = _drop_oldest(view, "recent")
    return view


def _recent_lines(soul: Any, context: PFCContext) -> list[str]:
    lines = [_recent_line(soul, event) for event in context.recent]
    lines += [_recent_line(soul, event, f"[{event.place}] ")
              for event in context.recent_elsewhere]
    return [line for line in lines if line] or ["(nothing yet)"]


def _recent_line(soul: Any, event: Event, prefix: str = "") -> str:
    if event.kind not in SHOWN_KINDS:
        return ""
    speaker, text = _recent_parts(soul, event)
    return (f"{prefix}{event.ts.astimezone(soul.timezone):%H:%M} "
            f"{speaker}: {text}") if text else ""


SHOWN_KINDS = (*RECENT_KINDS, "intent")  # an intent stays pending on that place's desk


def _recent_parts(soul: Any, event: Event) -> tuple[str, Any]:
    if event.kind == "act.out":
        return soul.name, (event.payload.get("arguments") or {}).get("text")
    if event.kind == "expectation.fired":
        return "expectation", (event.payload.get("text") or event.payload.get("about"))
    if event.kind == "intent":
        return "intent", event.payload.get("description")
    return str(event.who or "?"), event.payload.get("description")


def _event_text(event: Event | None) -> str:
    if event is None:
        return "(no trigger event)"
    if event.kind == "expectation.reminded":
        return _reminder_text(event)
    text = event.payload.get("description") or event.payload.get("text")
    who = str(event.who or "?")
    line = f"{who}: {text}" if text else f"[{event.kind}]"
    notes = event.payload.get("notes")
    if notes:
        each = [notes] if isinstance(notes, str) else notes
        line += "\n" + "\n".join(f"note: {note}" for note in each)
    return line


def _reminder_text(event: Event) -> str:
    """Render a reminder as the line with its speaker and place, then its question."""
    payload = event.payload
    place = payload.get("line_place") or event.place
    where = f"[{place}] " if place else ""
    said = f"{where}{event.who or '?'}: {payload.get('text')}" if payload.get("text") else ""
    question = (f"this line may relate to your expectation: {payload.get('about') or '?'}; "
                f"condition: {payload.get('condition') or '?'}; "
                "does this line fulfil it?")
    return f"{said}\n{question}" if said else question


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
