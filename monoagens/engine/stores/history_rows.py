"""Row shape for the transcript archive: the write values and the merge of a duplicate.

The archive stores one row per (thread, message id) and keeps the richer of two copies; these
two functions are the whole row contract, so `history.py` holds the store and its queries.
"""

import time

from sqlalchemy import Text, case, cast, func, literal, or_
from sqlalchemy.dialects.postgresql import JSONB

from ..envelope import KINDS, SPEAKERS, TRUSTS, UNKNOWN, label_value, stored_labels
from ..guard.patterns import redact_spans
from ..schema import messages
from ..turn.context import AUTHOR_RANK, Message, clamp_visibility
from .history_codec import FACT_KEYS, _data


def message_row(thread_id: str, message: Message) -> dict:
    """The row values for one message, labels and redaction included."""
    text, redaction = redact_spans(message.text or "")
    labels = stored_labels(message)
    meta = dict(labels["label_meta"] or {})
    if redaction["count"]:
        # ADR-0012 §4.6: the marker records that a secret was removed, never the secret.
        meta["redaction"] = redaction
    return {
        "thread_id": thread_id,
        "msg_id": message.id or f"tmp-{time.time_ns()}",
        "ts": message.when.timestamp(),
        "sender_id": message.sender_id or "",
        "sender_name": message.sender_name or "",
        "sender_address": message.sender_address or "",
        # L5 material is never persisted: the archive keeps the redacted shape, the live
        # turn keeps the raw text in its in-memory window.
        "text": text,
        "outgoing": bool(message.outgoing),
        "author": message.author,
        "visibility": clamp_visibility(message.visibility),
        "deleted": bool(message.deleted),
        "deleted_by": message.deleted_by or "",
        "deleted_admin": bool(message.deleted_admin),
        "data": _data(message),
        # ADR-0012 §4.1: the envelope labels, written once here; defaults never vary.
        "speaker": label_value(labels["speaker"], SPEAKERS),
        "actor": str(message.actor or ""),
        "addressed_to": str(message.addressed_to or UNKNOWN) or UNKNOWN,
        "reply_to_self": bool(message.reply_to_self),
        "kind": label_value(labels["kind"], KINDS),
        "trust": label_value(message.trust, TRUSTS),
        "valid_until": message.valid_until,
        "label_meta": meta,
    }


def content_data(excluded):
    """The incoming `data` without its fact keys; NULL when the row carries no content."""
    stripped = cast(excluded.data, JSONB)
    for key in FACT_KEYS:
        stripped = stripped.op("-")(cast(literal(key), Text))
    return func.nullif(cast(stripped, Text), "{}")


def merge_update(stmt) -> dict:
    """The values an upsert keeps from the stored row and the values the new copy refreshes."""
    table = messages
    excluded = stmt.excluded
    data = case((content_data(excluded).is_(None), table.c.data), else_=excluded.data)
    new_rank = case(AUTHOR_RANK, value=excluded.author, else_=0)
    old_rank = case(AUTHOR_RANK, value=table.c.author, else_=0)
    keep_new = new_rank >= old_rank
    removed = excluded.deleted.is_(True)
    return {
        "sender_name": case(
            (
                keep_new,
                func.coalesce(func.nullif(excluded.sender_name, ""), table.c.sender_name),
            ),
            else_=table.c.sender_name,
        ),
        "sender_id": case(
            (keep_new, func.coalesce(func.nullif(excluded.sender_id, ""), table.c.sender_id)),
            else_=table.c.sender_id,
        ),
        "sender_address": case(
            (
                keep_new,
                func.coalesce(func.nullif(excluded.sender_address, ""), table.c.sender_address),
            ),
            else_=table.c.sender_address,
        ),
        "author": case((keep_new, excluded.author), else_=table.c.author),
        "visibility": case(
            (excluded.visibility > table.c.visibility, excluded.visibility),
            else_=table.c.visibility,
        ),
        "text": func.coalesce(func.nullif(excluded.text, ""), table.c.text),
        "deleted": or_(table.c.deleted, excluded.deleted),
        "deleted_by": case((removed, excluded.deleted_by), else_=table.c.deleted_by),
        "deleted_admin": case((removed, excluded.deleted_admin), else_=table.c.deleted_admin),
        "data": data,
        # ADR-0012 §4.2: a re-delivered or edited message never downgrades a label.
        **{
            name: case((excluded[name] != UNKNOWN, excluded[name]), else_=table.c[name])
            for name in ("speaker", "kind", "trust", "addressed_to")
        },
        "actor": func.coalesce(func.nullif(excluded.actor, ""), table.c.actor),
        "reply_to_self": or_(table.c.reply_to_self, excluded.reply_to_self),
        "valid_until": func.coalesce(excluded.valid_until, table.c.valid_until),
        # Provenance merges key by key: an arriving copy without an addressing entry never
        # drops the one the stored row carries (ADR-0012 §4.7).
        "label_meta": case(
            (
                excluded.speaker != UNKNOWN,
                cast(table.c.label_meta, JSONB).op("||")(cast(excluded.label_meta, JSONB)),
            ),
            else_=table.c.label_meta,
        ),
    }
