"""Deterministic envelope labels for a stored message (ADR-0012 §4.1, ADR-0018 P1).

The archive writes these labels once, at storage time, from facts that are certain then; a later
reader never re-derives who spoke or what the row is. The value sets mirror the CHECK constraints
on `messages` (migration `0025`): a label outside its set must never reach the row.
"""

import re

# The allowed value sets, mirroring `ck_messages_speaker`, `ck_messages_kind`, `ck_messages_trust`.
SPEAKERS = frozenset({"self", "sender", "other_person", "system_receipt", "unknown"})
KINDS = frozenset({"chat", "question", "command", "tool_result", "notice", "unknown"})
TRUSTS = frozenset({"owner", "member", "untrusted_content", "unknown"})

UNKNOWN = "unknown"

# The one text rule: a leading slash command (`/approve #5`, `/help`); no keyword table.
COMMAND = re.compile(r"^/[A-Za-z][A-Za-z0-9_-]*(\s|$)")

_RULE = {"source": "rule", "version": 1}


def label_value(value: object, allowed: frozenset[str]) -> str:
    """A label inside its allowed set; `unknown` for anything else (never raise)."""
    text = str(value if value is not None else "")
    return text if text in allowed else UNKNOWN


def _speaker(message, bot: str) -> str:
    if message.outgoing:
        return "self" if message.author == bot else UNKNOWN
    return UNKNOWN if message.author == bot else "sender"


def _kind(message) -> str:
    if message.notice:
        return "notice"
    text = (message.text or "").strip()
    if not text:
        return UNKNOWN
    # A command is something a person types; the Soul quoting `/approve #5` is still chat.
    return "command" if not message.outgoing and COMMAND.match(text) else "chat"


def stored_labels(message) -> dict:
    """The labels to write: the message's own when it already carries one, else the rules."""
    if message.speaker != UNKNOWN or message.kind != UNKNOWN:
        return {
            "speaker": message.speaker,
            "kind": message.kind,
            "label_meta": message.label_meta,
        }
    return label_message(message)


def label_message(message) -> dict:
    """The labels to store for a message whose `speaker` and `kind` are still `unknown`.

    `addressed_to`, `reply_to_self`, `trust` and `actor` need gateway signals or RBAC evidence, so
    they are left to the caller and the table defaults. `label_meta` records provenance only for
    the labels this function actually set.
    """
    from .turn.context import AUTHOR_BOT

    speaker = _speaker(message, AUTHOR_BOT)
    kind = _kind(message)
    labels = {"speaker": speaker, "kind": kind}
    meta = {name: dict(_RULE) for name, value in labels.items() if value != UNKNOWN}
    return {"speaker": speaker, "kind": kind, "label_meta": meta}
