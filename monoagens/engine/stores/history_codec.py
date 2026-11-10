"""Row codec for the transcript archive: message envelopes and media references.

The archive stores the envelope as one JSONB `data` column plus the attachment bytes; these
functions turn a `Message` into those values and back. They hold no connection and no SQL, so
`history.py` can keep the store itself focused on the write and read paths.
"""


import re

from ..guard.patterns import redact_credentials
from ..turn.context import Media, Message

# Facts ride in the `data` blob but never make a row richer: the upsert keeps the old content
# when the incoming row carries nothing but these keys.
FACT_KEYS = ("sender_kind",)


def _media_json(media: Media) -> dict:
    return {
        "kind": media.kind,
        "mimetype": media.mimetype,
        "filename": media.filename,
        "size": int(media.size or 0),
    }


def _media_from(value, ref) -> Media | None:
    if not isinstance(value, dict):
        return None
    return Media(
        kind=str(value.get("kind") or ""),
        mimetype=str(value.get("mimetype") or ""),
        filename=str(value.get("filename") or ""),
        size=int(value.get("size") or 0),
        proto=None,
        ref=bytes(ref) if ref else None,
    )


def _data(message: Message) -> dict:
    data: dict = {}
    if message.attachments:
        # The old single-attachment key stays for readers of an older archive; the envelope is
        # the list every attachment of this message is decoded from.
        data["media"] = _media_json(message.attachments[0])
        data["attachments"] = [_media_json(item) for item in message.attachments]
    if message.quote is not None:
        quote = {
            "id": message.quote.id,
            "sender_name": message.quote.sender_name,
            "sender_id": message.quote.sender_id,
            "text": redact_credentials(message.quote.text or ""),
            "when": message.quote.when.timestamp() if message.quote.when else None,
        }
        if message.quote.media is not None:
            quote["media"] = _media_json(message.quote.media)
        data["quote"] = quote
    if message.view_once:
        data["view_once"] = True
    if message.unreadable:
        data["unreadable"] = True
    if message.sender_kind:
        data["sender_kind"] = message.sender_kind
    return data


def _media_rows(thread_id: str, message: Message) -> list[dict]:
    """The bytes that no longer live on the row (ADR-0002 §4.5), numbered per message.

    Every attachment gets a row even when the body carried no bytes, so the row set mirrors the
    envelope and `seq` is the attachment's place in it. A quote's bytes use the next free number:
    `kind` says what the row is, `seq` is the identity.
    """
    rows: list[dict] = []
    for seq, media in enumerate(message.attachments):
        if media is None:
            continue
        rows.append(
            {
                "thread_id": thread_id,
                "message_id": message.id,
                "seq": seq,
                "kind": "media",
                "mime": str(media.mimetype or ""),
                "bytes_ref": _blob(media),
            }
        )
    quote = message.quote
    quote_media = quote.media if quote is not None else None
    blob = _blob(quote_media)
    if blob is not None:
        rows.append(
            {
                "thread_id": thread_id,
                "message_id": message.id,
                "seq": len(message.attachments),
                "kind": "quote",
                "mime": str(getattr(quote_media, "mimetype", "") or ""),
                "bytes_ref": blob,
            }
        )
    return rows


def _blob(media: Media | None) -> bytes | None:
    if media is None:
        return None
    if media.ref:
        return bytes(media.ref)
    proto = getattr(media, "proto", None)
    if proto is None:
        return None
    try:
        return proto.SerializeToString()
    except (AttributeError, TypeError):
        return None


def _like_terms(query: str) -> list[str]:
    return re.findall(r"\w+", query, re.UNICODE) or [query.strip()]
