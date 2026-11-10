"""Deterministic envelope labels for a stored message (ADR-0012 §4.1, ADR-0018 P1).

The archive writes these labels once, at storage time, from facts that are certain then; a later
reader never re-derives who spoke or what the row is. The value sets mirror the CHECK constraints
on `messages` (migration `0025`): a label outside its set must never reach the row.
"""

import re

from .access.permissions import normalize

# The allowed value sets, mirroring `ck_messages_speaker`, `ck_messages_kind`, `ck_messages_trust`.
SPEAKER_RECEIPT = "system_receipt"  # the harness answered the command, not the Soul
SPEAKERS = frozenset({"self", "sender", "other_person", SPEAKER_RECEIPT, "unknown"})
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


def replies_to_self(message, handles, found) -> bool:
    """Whether the message quotes one of the Soul's own earlier messages.

    `handles` is the body's `self_handles()` set; `found` is the stored message named by the
    quote, when the caller could look it up. A body that reports the quote author is trusted
    first; only then does the stored author decide.
    """
    quote = getattr(message, "quote", None)
    if quote is None:
        return False
    target = normalize(getattr(quote, "sender_id", "") or "")
    if target and target in {normalize(h) for h in (handles or ()) if h}:
        return True
    if not getattr(quote, "id", "") or found is None:
        return False
    from .turn.context import AUTHOR_BOT

    return getattr(found, "author", "") == AUTHOR_BOT


def merge_labels(old, new) -> dict:
    """The envelope labels to keep when two rows of the same message merge.

    A label is sticky: a re-delivered duplicate that carries no signal never downgrades one
    already stored.
    """
    return {
        "speaker": new.speaker if new.speaker != UNKNOWN else old.speaker,
        "kind": new.kind if new.kind != UNKNOWN else old.kind,
        "trust": new.trust if new.trust != UNKNOWN else old.trust,
        "addressed_to": new.addressed_to if new.addressed_to != UNKNOWN else old.addressed_to,
        "actor": new.actor or old.actor,
        "reply_to_self": bool(new.reply_to_self or old.reply_to_self),
        "valid_until": new.valid_until or old.valid_until,
        "label_meta": new.label_meta if new.speaker != UNKNOWN else old.label_meta,
    }


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
