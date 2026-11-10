"""Deterministic addressing score: is this message meant for the Soul? (ADR-0012 §4.7).

Pure and total: the caller supplies the transport signals and the conversation state, the
weights and bands come from `engine.addressing`, and the result names every signal that moved
the score. A group message is answered only when the score clears `answer_at`; a wrong answer
costs more than a missed one, so the band between the two thresholds stays silent.
"""

from dataclasses import dataclass, field

from ..config import AddressingConfig

ANSWER = "answer"
IGNORE = "ignore"
UNSURE = "unsure"
DECISIONS = frozenset({ANSWER, IGNORE, UNSURE})

SELF = "self"
GROUP = "group"
UNKNOWN = "unknown"

# A direct chat is decisive: the ADR gives it no weight because no signal outranks it.
DIRECT_POINTS = 100


@dataclass(frozen=True)
class Signals:
    """What the gateway reported and what the core knows about this one message."""

    direct: bool = False
    mentions_self: bool = False
    mentions_other: bool = False
    reply_to_self: bool = False
    reply_to_other: bool = False
    continuity: bool = False
    pending_request: bool = False
    name_in_text: bool = False
    # The person the negative signals point to, when the body named one.
    other_target: str = ""


@dataclass(frozen=True)
class Result:
    score: int
    decision: str
    addressed_to: str
    reasons: list[tuple[str, int]] = field(default_factory=list)


def _points(signals: Signals, weights) -> list[tuple[str, int]]:
    """The signals that fired, in the table order of the ADR, with their points."""
    found: list[tuple[str, int]] = []
    if signals.mentions_self:
        found.append(("mention_self", weights.mention_self))
    # "Mention of another person, NOT the Soul": a message that names the Soul as well is still
    # for the Soul (the mention passes alone), so the other person only counts without it.
    tagging_other = signals.mentions_other and not signals.mentions_self
    if signals.reply_to_self:
        # Quoting the Soul to tag someone is the one row that covers both signals: the reply row
        # replaces the mention row, so the same tag is never counted twice.
        if tagging_other:
            found.append(("reply_to_self_tagging_other", weights.reply_to_self_tagging_other))
        else:
            found.append(("reply_to_self", weights.reply_to_self))
    elif tagging_other:
        found.append(("mention_other", weights.mention_other))
    if signals.reply_to_other:
        found.append(("reply_to_other", weights.reply_to_other))
    if signals.continuity:
        found.append(("continuity", weights.continuity))
    if signals.pending_request:
        found.append(("pending_request", weights.pending_request))
    if signals.name_in_text:
        found.append(("name_in_text", weights.name_in_text))
    return [(name, points) for name, points in found if points]


def _band(total: int, cfg: AddressingConfig, skepticism: str) -> str:
    shift = int(getattr(cfg.skepticism, str(skepticism or "normal"), 0) or 0)
    if total >= cfg.answer_at + shift:
        return ANSWER
    if total < cfg.ignore_below + shift:
        return IGNORE
    return UNSURE


def _addressed_to(signals: Signals, decision: str) -> str:
    """Who the message is for; the negative signals name a person only when they have one."""
    if decision == ANSWER:
        return SELF
    other = str(signals.other_target or "").strip()
    if other and (signals.mentions_other or signals.reply_to_other):
        return f"person:{other}"
    return UNKNOWN if signals.direct else GROUP


def score(signals: Signals, cfg: AddressingConfig, *, skepticism: str = "normal") -> Result:
    """Score one message and explain it; never raises on a value the config carries."""
    if signals.direct:
        return Result(DIRECT_POINTS, ANSWER, SELF, [("direct", DIRECT_POINTS)])
    reasons = _points(signals, cfg.advanced.weights)
    total = sum(points for _name, points in reasons)
    decision = _band(total, cfg, skepticism)
    return Result(total, decision, _addressed_to(signals, decision), reasons)
