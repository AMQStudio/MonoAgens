"""WhatsApp inbound event formatter: messages, edits, revokes, reactions, albums, calls, groups."""

from __future__ import annotations

import hashlib
from collections.abc import Iterable, Mapping, Sequence
from dataclasses import replace
from datetime import datetime, timezone
from typing import Any

from monoagens.adapters.whatsapp.identity import (
    WhatsAppDirectory,
    digits_of,
    jid_text,
    place_id,
)
from monoagens.adapters.whatsapp.media import (
    contacts_info,
    contexts,
    location_info,
    message_text,
    poll_info,
    unwrap_message,
    views_once,
)
from monoagens.adapters import support
from monoagens.contract import Answer, Audience, Percept, describe_attachments
from monoagens.types import Event


def percept_of(*, actor: str, place: str, occurred_at: datetime, description: str,
               features: Mapping[str, float], data: Mapping[str, Any],
               ref: str = "", about: str = "", involves: Sequence[str] = (),
               attachments: Sequence[Mapping[str, Any]] = (), answer: Answer | None = None,
               imported: bool = False, backfill: bool = False) -> Percept:
    """Return one percept with its `data` scrubbed to plain JSON (§2.4.2)."""
    return Percept(actor=actor, place=place, occurred_at=occurred_at,
                   description=clean_text(description), ref=clean_text(ref),
                   about=clean_text(about), involves=tuple(clean_text(item)
                                                          for item in involves),
                   attachments=_clean_attachments(attachments),
                   features=dict(features), answer=answer, backfill=backfill,
                   imported=imported, data=clean_json(dict(data)))


def _clean_attachments(items: Sequence[Any]) -> tuple[Any, ...]:
    """Return the contract's attachment objects with Postgres-safe text fields."""
    return tuple(
        replace(item, name=clean_text(item.name), mime=clean_text(item.mime))
        for item in support.attachments_of(items))


def audience_of(place: str, directory: WhatsAppDirectory, *, actor: str) -> Audience:
    """Return who can perceive one WhatsApp place, with its own rule folded in (§2.4.3).

    A private chat is the other person; a group is its members now (newcomers do not see the
    past); a channel only broadcasts, so anyone may read it later: *unknown*.
    """
    meta = directory.place_meta(place)
    form = str(meta.get("form") or "")
    if form == "broadcast":
        return Audience(actors=frozenset(), unknown=True)
    if form == "group":
        members = frozenset(str(member) for member in meta.get("members") or ())
        return Audience(actors=members, unknown=not members)
    found = frozenset({actor}) if actor and actor != "unknown" else frozenset()
    return Audience(actors=found, unknown=not found)


def clean_text(value: Any) -> str:
    """Return string stripped of NUL bytes rejected by Postgres text/jsonb."""
    return str(value).replace("\x00", "")


def clean_json(value: Any) -> Any:
    """Coerce nested values into Postgres-safe JSON types with no NUL bytes."""
    if value is None or isinstance(value, (bool, int, float)):
        return value
    if isinstance(value, str):
        return value.replace("\x00", "")
    if isinstance(value, (bytes, bytearray, memoryview)):
        return bytes(value).decode("utf-8", errors="replace").replace("\x00", "")
    if isinstance(value, Mapping):
        return {clean_text(k): clean_json(v) for k, v in value.items()}
    if isinstance(value, (list, tuple, set, frozenset)):
        return [clean_json(item) for item in value]
    if isinstance(value, Iterable):
        return [clean_json(item) for item in value]
    return clean_text(value)


def clean_event(event: Event) -> Event:
    """Normalize event fields and payload so Postgres jsonb/text never reject a valid message."""
    return replace(
        event,
        who=clean_text(event.who) if event.who is not None else None,
        place=clean_text(event.place) if event.place is not None else None,
        body=clean_text(event.body) if event.body is not None else None,
        origin_actor=clean_text(event.origin_actor) if event.origin_actor is not None else None,
        payload=clean_json(dict(event.payload or {})),
    )


def parse_timestamp(raw_ts: Any) -> datetime | None:
    """Parse a seconds or milliseconds epoch into a UTC datetime, or None when unset."""
    try:
        value = int(raw_ts or 0)
    except (TypeError, ValueError):
        return None
    if value > 10**12:
        value //= 1000
    if value <= 0:
        return None
    try:
        return datetime.fromtimestamp(value, tz=timezone.utc)
    except (OverflowError, OSError, ValueError):
        return None


def iso_timestamp(raw_ts: Any, *, fallback: datetime | None = None) -> str:
    """Convert a seconds/milliseconds transport epoch into an ISO UTC timestamp string."""
    parsed = parse_timestamp(raw_ts)
    if parsed is None:
        return (fallback or datetime.now(timezone.utc)).isoformat()
    return parsed.isoformat()


def album_parent_id(message: Any) -> str | None:
    """Return the album's parent message id if `message` belongs to a WhatsApp media album."""
    ctx = getattr(message, "messageContextInfo", None)
    if ctx is None:
        return None
    assoc = getattr(ctx, "messageAssociation", None)
    if assoc is None:
        return None
    parent_key = getattr(assoc, "parentMessageKey", None)
    parent_id = str(getattr(parent_key, "ID", "") or "")
    if not parent_id:
        return None
    assoc_type = int(getattr(assoc, "associationType", 0) or 0)
    # MessageAssociation.MEDIA_ALBUM == 1 in WhatsApp E2E protobufs
    return parent_id if assoc_type == 1 else None


def album_expected_count(message: Any) -> int:
    """Return expected album child count for an `albumMessage` stanza, else 0."""
    body = unwrap_message(message)
    album = getattr(body, "albumMessage", None)
    if album is None:
        return 0
    return int(getattr(album, "expectedImageCount", 0) or 0) + int(
        getattr(album, "expectedVideoCount", 0) or 0
    )


def _sender_and_chat(source: Any, directory: WhatsAppDirectory, *,
                     self_account: str | None = None,
                     fallback_chat: str = "",
                     participant: Any = None) -> tuple[str, str, bool]:
    chat_raw = jid_text(getattr(source, "Chat", None)) or jid_text(
        getattr(source, "remoteJID", None)) or fallback_chat
    alt = getattr(source, "SenderAlt", None)
    chat_jid = directory.canonical_chat(chat_raw, alt=alt)
    from_me = bool(getattr(source, "IsFromMe", False) or getattr(source, "fromMe", False))
    if from_me and self_account:
        sender_acc = self_account
    else:
        sender_obj = getattr(source, "Sender", None) or participant
        sender_raw = jid_text(sender_obj)
        if not sender_raw and not chat_jid.endswith(("@g.us", "@newsletter")):
            sender_raw = chat_raw
        sender_acc = directory.canonical_account(sender_raw, alt=alt)
    return chat_jid, sender_acc, from_me


def _is_self_addressed(message: Any, chat_jid: str, *, from_me: bool,
                       directory: WhatsAppDirectory, own_digits: str,
                       own_sent: Iterable[str] = (), reply_msg_id: str = "") -> bool:
    if from_me:
        return bool(own_digits and not chat_jid.endswith(("@g.us", "@newsletter"))
                    and directory.canonical_digits(chat_jid) == own_digits)
    if not chat_jid.endswith(("@g.us", "@newsletter")):
        return True
    if reply_msg_id and reply_msg_id in own_sent:
        return True
    if not own_digits:
        return False
    for ctx in contexts(message):
        if any(directory.canonical_digits(jid) == own_digits
               for jid in (getattr(ctx, "mentionedJID", ()) or ())):
            return True
        part = getattr(ctx, "participant", "") or ""
        if part and directory.canonical_digits(part) == own_digits:
            return True
    return False


def _context_links(message: Any, directory: WhatsAppDirectory, *,
                   own_digits: str = "") -> tuple[dict[str, Any] | None, list[str]]:
    reply_info: dict[str, Any] | None = None
    mentions: list[str] = []
    for ctx in contexts(message):
        stanza_id = str(getattr(ctx, "stanzaID", "") or "")
        participant = str(getattr(ctx, "participant", "") or "")
        if stanza_id and reply_info is None:
            reply_info = {"message_id": stanza_id}
            if participant:
                reply_info["who"] = directory.canonical_account(participant)
        for jid in getattr(ctx, "mentionedJID", ()) or ():
            if own_digits and directory.canonical_digits(jid) == own_digits:
                continue
            acc = directory.canonical_account(jid)
            if acc and acc not in mentions:
                mentions.append(acc)
    return reply_info, mentions


def format_message(raw: Any, *, directory: WhatsAppDirectory, now: datetime,
                   name: str = "whatsapp", self_account: str | None = None,
                   own_sent: set[str] | frozenset[str] = frozenset(),
                   attachments: Sequence[dict[str, Any]] = (),
                   media_spec: dict[str, Any] | None = None,
                   choices_prompts: dict[str, Sequence[str]] | None = None,
                   imported: bool = False, backfill: bool = False,
                   fallback_chat: str = "", fallback_ts: int = 0) -> Event | None:
    """Turn a live MessageEv or HistorySync WebMessageInfo into one percept, or None to skip."""
    info = getattr(raw, "Info", None)
    if info is not None:
        source = info.MessageSource
        msg_id = str(info.ID or "")
        message = raw.Message
        pushname = str(getattr(info, "Pushname", "") or "")
        raw_ts = getattr(info, "Timestamp", 0) or fallback_ts
        participant = None
    else:
        source = getattr(raw, "key", None)
        msg_id = str(getattr(source, "ID", "") or "")
        message = getattr(raw, "message", None)
        pushname = str(getattr(raw, "pushName", "") or "")
        raw_ts = getattr(raw, "messageTimestamp", 0) or fallback_ts
        participant = getattr(source, "participant", "") or getattr(raw, "participant", "")
    if message is None or not _has_fields(message):
        return None
    if not msg_id:
        ser = getattr(raw, "SerializeToString", None)
        msg_id = "wa-" + hashlib.sha1(ser() if callable(ser) else b"msg",
                                      usedforsecurity=False).hexdigest()[:20]
    own_digits = digits_of(self_account or "")
    chat_jid, who, from_me = _sender_and_chat(
        source, directory, self_account=self_account,
        fallback_chat=fallback_chat, participant=participant)
    if not chat_jid or chat_jid.endswith("@broadcast"):
        return None
    if from_me and msg_id in own_sent:
        return None
    if not who:
        who = self_account or f"whatsapp:{digits_of(chat_jid)}"
    sender_label = directory.label(who, pushname=pushname)
    place = place_id(chat_jid)
    place_kind = "shared" if chat_jid.endswith(("@g.us", "@newsletter")) else "direct"
    place_meta = directory.place_meta(place)
    sent_at = iso_timestamp(raw_ts, fallback=now)
    base_payload: dict[str, Any] = {
        "place_kind": place_kind,
        "sent_at": sent_at,
        **({"self_account": self_account} if self_account else {}),
        **({"imported": True} if imported else {}),
        **({"backfill": True} if backfill else {}),
    }

    # 1. Protocol edits & deletions
    proto = getattr(message, "protocolMessage", None)
    if proto is not None and _has_fields(proto):
        ptype = int(getattr(proto, "type", 0) or 0)
        target_id = str(getattr(getattr(proto, "key", None), "ID", "") or "")
        if ptype == 0 and target_id:  # REVOKE
            return percept_of(
                actor=who, place=place, occurred_at=now,
                description=f"{sender_label} deleted a message",
                features={"addressed": 0, "named": 0, "addressed_other": 0},
                data={"message_id": msg_id, "deletes": target_id,
                      "self_account": self_account},
                ref=msg_id, about=target_id, imported=imported, backfill=backfill)
        if ptype == 14 and target_id:  # MESSAGE_EDIT
            edited = getattr(proto, "editedMessage", None)
            new_text = message_text(edited) if edited is not None else ""
            addr = _is_self_addressed(edited or message, chat_jid, from_me=from_me,
                                      directory=directory, own_digits=own_digits,
                                      own_sent=own_sent)
            return percept_of(
                actor=who, place=place, occurred_at=now,
                description=f"{sender_label} edited: {new_text}".strip(),
                features={"addressed": 1 if addr else 0, "named": 0, "addressed_other": 0},
                data={"message_id": msg_id, "edits": target_id, "text": new_text,
                      "self_account": self_account},
                ref=msg_id, about=target_id, imported=imported, backfill=backfill)

    # 2. Reactions
    reaction = getattr(message, "reactionMessage", None)
    if reaction is not None and _has_fields(reaction):
        emoji = str(getattr(reaction, "text", "") or "")
        target_id = str(getattr(getattr(reaction, "key", None), "ID", "") or "")
        if not emoji:
            desc = f"{sender_label} removed reaction on {target_id}".strip()
        else:
            desc = f"{sender_label} reacted {emoji}"
        addr = bool(target_id and target_id in own_sent) and not from_me
        return percept_of(
            actor=who, place=place, occurred_at=now, description=desc,
            features={"addressed": 1 if addr else 0, "named": 0,
                      "addressed_other": 0 if addr else 1},
            data={"message_id": msg_id,
                  "reaction": {"emoji": emoji, "message_id": target_id},
                  "self_account": self_account},
            ref=msg_id, about=target_id, imported=imported, backfill=backfill)

    # 3. Poll vote update
    poll_upd = getattr(message, "pollUpdateMessage", None)
    if poll_upd is not None and _has_fields(poll_upd):
        poll_id = str(getattr(getattr(poll_upd, "pollCreationMessageKey", None), "ID", "") or "")
        addr = bool(poll_id and poll_id in own_sent) and not from_me
        option = poll_vote_option(poll_upd)
        return percept_of(
            actor=who, place=place, occurred_at=now,
            description=(f"{sender_label} chose {option}" if option
                         else f"{sender_label} voted on poll"),
            features={"addressed": 1, "named": 0, "addressed_other": 0},
            data={"message_id": msg_id, "poll_vote": {"poll_id": poll_id},
                  "self_account": self_account},
            ref=msg_id, about=poll_id,
            answer=Answer(prompt=poll_id, value=option) if (poll_id and option) else None,
            imported=imported, backfill=backfill)

    # 4. Content message (text, attachments, sticker, location, contact, poll)
    text = message_text(message)
    if not text and not attachments and not media_spec:
        return None
    reply_info, mentions = _context_links(message, directory, own_digits=own_digits)
    reply_id = str((reply_info or {}).get("message_id") or "")
    data: dict[str, Any] = {"message_id": msg_id}
    if reply_info:
        data["reply_to"] = reply_info
    if mentions:
        data["mentions"] = mentions
    if views_once(message):
        data["view_once"] = True
    body = unwrap_message(message)
    loc = location_info(body)
    if loc:
        data["location"] = loc
    contacts = contacts_info(body)
    if contacts:
        data["contacts"] = contacts
    poll = poll_info(body)
    if poll:
        data["poll"] = poll
    if attachments:
        data["attachments"] = [dict(item) for item in attachments]
    if media_spec and media_spec.get("kind") == "sticker" and attachments:
        emoji = media_spec.get("emoji") or ""
        data["sticker"] = {"ref": attachments[0].get("ref"), "emoji": emoji,
                           "animated": bool(media_spec.get("animated"))}
        desc = f"{sender_label} sent a sticker {emoji}".strip()
        if len(attachments) > 1:
            desc += f" ({describe_attachments(attachments, sent_by=sender_label)})"
    elif attachments:
        summary = describe_attachments(attachments, sent_by=sender_label)
        desc = f"{sender_label}: {text} {summary}".strip() if text else summary
    else:
        desc = f"{sender_label}: {text}".strip()

    del choices_prompts  # free words are reasoning's; a poll or a button is the answer
    addressed = _is_self_addressed(
        message, chat_jid, from_me=from_me, directory=directory, own_digits=own_digits,
        own_sent=own_sent, reply_msg_id=reply_id)
    if self_account:
        data["self_account"] = self_account
    return percept_of(
        actor=who, place=place, occurred_at=parse_timestamp(raw_ts) or now, description=desc,
        features={"addressed": 1 if addressed else 0, "named": 0,
                  "addressed_other": 0 if addressed else 1},
        data=data, ref=msg_id, about=reply_id, involves=mentions,
        attachments=attachments, imported=imported, backfill=backfill)


def format_album(events: Sequence[Percept], *, album_id: str,
                 directory: WhatsAppDirectory) -> Percept | None:
    """Collapse a batch of album child percepts into ONE percept carrying all attachments."""
    if not events:
        return None
    if len(events) == 1:
        return events[0]
    first = events[0]
    all_atts: list[dict[str, Any]] = []
    captions: list[str] = []
    msg_ids: list[str] = []
    addressed = any(float(ev.features.get("addressed") or 0) for ev in events)
    sender_label = directory.label(first.actor or "")
    for ev in events:
        d = ev.data or {}
        if d.get("message_id"):
            msg_ids.append(str(d["message_id"]))
        for att in d.get("attachments") or ():
            all_atts.append(dict(att))
        raw_desc = str(ev.description or "")
        # Keep a real caption, not the boilerplate the album items share
        if raw_desc and not raw_desc.startswith(f"{sender_label} sent "):
            captions.append(raw_desc)
    summary = describe_attachments(all_atts, sent_by=sender_label)
    desc = f"{captions[0]} {summary}".strip() if captions else summary
    data = dict(first.data or {})
    data["message_id"] = album_id
    data["message_ids"] = msg_ids
    data["attachments"] = all_atts
    joined = tuple(dict.fromkeys(
        actor for ev in events for actor in (ev.involves or ())))
    return percept_of(
        actor=first.actor, place=first.place, occurred_at=first.occurred_at,
        description=desc,
        features={"addressed": 1 if addressed else 0, "named": 0,
                  "addressed_other": 0 if addressed else 1},
        data=data, ref=album_id, involves=joined, attachments=all_atts,
        imported=first.imported, backfill=first.backfill)


def format_call(status: str, meta: Any, *, reason: str = "", is_video: bool = False,
                directory: WhatsAppDirectory, now: datetime, name: str = "whatsapp",
                self_account: str | None = None) -> Event | None:
    """Turn incoming, missed, or ended WhatsApp call events into neutral percepts."""
    caller_obj = getattr(meta, "from", None) or getattr(meta, "callCreator", None)
    alt_obj = getattr(meta, "callCreatorAlt", None)
    caller_jid = directory.canonical_chat(caller_obj, alt=alt_obj)
    who = directory.canonical_account(caller_obj, alt=alt_obj)
    if not who or not caller_jid:
        return None
    label = directory.label(who)
    call_id = str(getattr(meta, "callID", "") or "")
    place = place_id(caller_jid)
    kind_word = "video call" if is_video else "call"
    if status == "incoming":
        desc = f"{label} is calling ({kind_word})"
    elif status == "missed":
        desc = f"{label} called, not answered ({kind_word})"
    else:
        desc = f"Call with {label} ended" + (f" ({reason})" if reason else "")
    del name
    spoken = status in ("incoming", "missed")
    return percept_of(
        actor=who, place=place,
        occurred_at=parse_timestamp(getattr(meta, "timestamp", 0)) or now,
        description=desc,
        features={"addressed": 1 if spoken else 0, "named": 0,
                  "addressed_other": 0 if spoken else 1},
        data={"call": {"status": status, "call_id": call_id, "video": is_video,
                       **({"reason": reason} if reason else {})},
              "self_account": self_account})


def poll_vote_option(vote: Any) -> str:
    """Return the option one decrypted poll vote names, or an empty string (§5.4)."""
    selected = getattr(vote, "selectedOptions", None) or ()
    names = [str(getattr(option, "name", "") or "") for option in selected]
    return next((name for name in names if name), "")


def _has_fields(proto: Any) -> bool:
    fn = getattr(proto, "ListFields", None)
    return bool(fn()) if callable(fn) else bool(proto)


def format_undecryptable(ev: Any, *, directory: WhatsAppDirectory, now: datetime,
                         name: str = "whatsapp", self_account: str | None = None,
                         backfill: bool = False) -> Event | None:
    """Turn a failed-decryption `UndecryptableMessage` into an honest percept."""
    info = getattr(ev, "Info", None)
    if info is None:
        return None
    source = getattr(info, "MessageSource", None)
    msg_id = str(getattr(info, "ID", "") or "")
    pushname = str(getattr(info, "Pushname", "") or "")
    raw_ts = getattr(info, "Timestamp", 0)
    chat_jid, who, _from_me = _sender_and_chat(source, directory, self_account=self_account)
    if not chat_jid or chat_jid.endswith("@broadcast"):
        return None
    if not who:
        who = self_account or f"whatsapp:{digits_of(chat_jid)}"
    sender_label = directory.label(who, pushname=pushname)
    place = place_id(chat_jid)
    direct = not chat_jid.endswith(("@g.us", "@newsletter"))
    del name
    return percept_of(
        actor=who, place=place, occurred_at=parse_timestamp(raw_ts) or now,
        description=f"{sender_label}: a message could not be read",
        features={"addressed": 1 if direct else 0, "named": 0, "addressed_other": 0},
        data={"message_id": msg_id, "undecryptable": True,
              "unavailable": bool(getattr(ev, "IsUnavailable", False)),
              "self_account": self_account},
        ref=msg_id, backfill=backfill)
