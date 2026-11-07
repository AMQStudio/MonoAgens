"""Claim detection: an inability is a tool result, never a transcript line or a memory.

`note.md` §10: a model that repeats "I cannot read that room" from its own history, from a
stored memory, or from an unverified guess must be corrected before the reply is sent. There is
no phrase list here: a refusal can be worded in millions of ways in any language, so the check
is one light yes/no model call over the draft, bounded by the caller's retry budget.

The judge sees the sender's request and what each tool of this turn returned, so it can tell an
ordinary impossible plan ("I cannot attend the meeting tomorrow") from a claim about access the
turn never checked.
"""

import re

from . import i18n

# The one system correction the guard adds before it re-asks (prompt scaffolding, English).
NOTE = (
    "Your draft said something is impossible without a tool result. Call the relevant tool "
    "(thread_history, info, memory, or the body tool that owns the target) and answer from what "
    "it returns. Never state an inability, a missing archive, or somebody's membership before "
    "checking it in this turn."
)

# The judge: one yes/no answer over the draft plus this turn's evidence.
JUDGE = (
    "You check one assistant draft. Answer YES only when the draft tells the sender that the "
    "assistant cannot read, send, reach, or access something, that an archive does not exist, "
    "or that somebody is not a member of a room, and no tool result in this turn already "
    "answered it. A refusal the sender asked for, an ordinary plan that is impossible, or a "
    "statement about a third person is NO."
)
JUDGE_CHARS = 1200
REQUEST_CHARS = 300
TOOL_LIMIT = 10

# A refusal the code itself produced is authoritative: the guard never re-asks it.
OFFICIAL = (
    "agent.access_refused",
    "agent.cross_chat_blocked",
    "agent.cross_room_blocked",
    "agent.research_abstain",
)


def classifier_cap(llm) -> int:
    """The measured completion cap for a one-answer call; `0` means no explicit `max_tokens`."""
    config = getattr(llm, "config", None)
    llm_config = getattr(config, "llm", None)
    return max(0, int(getattr(llm_config, "classifier_max_tokens", 0) or 0))


def cap_kwargs(llm) -> dict:
    """`max_tokens` only when the operator measured a cap; otherwise the client default."""
    cap = classifier_cap(llm)
    return {"max_tokens": cap} if cap > 0 else {}


async def detect(
    llm, text: str, *, language: str = "", request: str = "", tools=()
) -> bool:
    """True when the text asserts an inability about access, capability, or membership.

    One small tool-free call, never a keyword match: the wording and the language are the
    model's to judge. `tools` is `[(name, refused), ...]` for this turn, so the judge knows what
    the code already answered. A missing model or a failed call answers "not a claim", so
    detection can only ever cost the retry it guards, never a turn.
    """
    body = " ".join(str(text or "").split())[:JUDGE_CHARS]
    if llm is None or not body:
        return False
    asked = " ".join(str(request or "").split())[:REQUEST_CHARS] or "-"
    seen = ", ".join(
        f"{name}: {'refused' if refused else 'ok'}"
        for name, refused in list(tools)[:TOOL_LIMIT]
    )
    try:
        answer = await llm.complete(
            JUDGE,
            f"Language: {language or 'unknown'}\n"
            f"Sender asked: {asked}\n"
            f"Tools run this turn: {seen or 'none'}\n"
            f"Draft:\n{body}",
            purpose="claim_guard",
            **cap_kwargs(llm),
        )
    except Exception:
        return False
    # Loose parse: a model may preamble ("Sure — YES, it does."), and an empty answer (the
    # budget went to reasoning) is not a claim, so it never spends a re-ask.
    words = re.findall(r"[A-Z]+", str(answer or "").upper())
    return "YES" in words and "NO" not in words


def official(text: str, language: str) -> bool:
    """True when the text is (or quotes) a refusal the code produced, not a model claim."""
    body = " ".join(str(text or "").split())
    if not body:
        return False
    for key in OFFICIAL:
        message = " ".join(i18n.t(key, language).split())
        if message and message in body:
            return True
    return False
