"""WhatsApp outbound sending: text, attachments, replies, mentions, stickers, choices, location."""

from __future__ import annotations

from typing import Any

from monoagens.adapters.whatsapp.identity import (
    WhatsAppDirectory,
    chat_jid_from_place,
    digits_of,
)
from monoagens.adapters.whatsapp.media import classify_send_bytes


def jid_of(chat_jid: str) -> Any:
    """Build a neonize JID object from `<user>@<server>`."""
    from neonize.utils.jid import build_jid

    user, _, server = str(chat_jid or "").partition("@")
    return build_jid(user, server or "s.whatsapp.net")


def _msg_id(resp: Any, fallback: str = "") -> str:
    return str(getattr(resp, "ID", "") or getattr(resp, "id", "") or fallback)


def _mention_jids(targets: Any, directory: WhatsAppDirectory) -> list[str]:
    found: list[str] = []
    for raw in targets or ():
        text = str(raw or "").strip()
        if not text:
            continue
        jid = text if "@" in text else directory.jid_for_account(text)
        if jid and jid not in found:
            found.append(jid)
    return found


def _with_context(text: str, *, mentions: list[str], reply_to: str = "",
                  directory: WhatsAppDirectory | None = None) -> Any:
    from neonize.proto.waE2E.WAWebProtobufsE2E_pb2 import ContextInfo, ExtendedTextMessage, Message

    if not mentions and not reply_to:
        return text
    rendered = text
    for jid in mentions:
        tag = f"@{digits_of(jid)}"
        if tag not in rendered:
            rendered = f"{rendered} {tag}".strip()
    ctx = ContextInfo(
        mentionedJID=mentions,
        **({"stanzaID": reply_to} if reply_to else {}),
    )
    return Message(extendedTextMessage=ExtendedTextMessage(text=rendered, contextInfo=ctx))


async def send_say(client: Any, directory: WhatsAppDirectory, files: Any, place: str,
                   payload: dict[str, Any]) -> dict[str, Any]:
    """Send text, files, sticker, choices, thread or location to one WhatsApp chat."""
    if client is None:
        raise RuntimeError("WhatsApp client is not connected")
    chat_jid = chat_jid_from_place(place)
    jid = jid_of(chat_jid)
    text = str(payload.get("text") or "")
    thread = str(payload.get("thread") or "").strip()
    if thread:
        text = f"*{thread}*\n{text}".strip() if text else f"*{thread}*"
    reply_to = str(payload.get("reply_to") or "")
    mentions = _mention_jids(payload.get("mentions"), directory)
    voice = bool(payload.get("voice"))
    sent_ids: list[str] = []

    sticker_ref = str(payload.get("sticker") or "").strip()
    if sticker_ref:
        data, _ = await files.open(sticker_ref, context=place)
        resp = await client.send_sticker(jid, data)
        if mid := _msg_id(resp):
            sent_ids.append(mid)

    location = payload.get("location")
    if isinstance(location, dict) and "lat" in location and "lon" in location:
        loc_id = await _send_location(client, jid, location)
        if loc_id:
            sent_ids.append(loc_id)

    choices = [str(c) for c in (payload.get("choices") or ()) if str(c).strip()]
    choices_prompt: tuple[str, list[str]] | None = None
    if choices:
        prompt_id = await _send_choices(
            client, jid, text or "Choose one:", choices, multiple=bool(payload.get("multiple")))
        if prompt_id:
            sent_ids.append(prompt_id)
            choices_prompt = (prompt_id, choices)
        text = ""

    opened: list[tuple[bytes, dict[str, Any]]] = []
    for ref in payload.get("attachments") or ():
        opened.append(await files.open(str(ref), context=place))

    if text:
        msg_payload = _with_context(text, mentions=mentions, reply_to=reply_to, directory=directory)
        resp = await client.send_message(jid, msg_payload)
        if mid := _msg_id(resp):
            sent_ids.append(mid)

    for data, info in opened:
        name = str(info.get("name") or "file")
        mime = str(info.get("mime") or "")
        kind = classify_send_bytes(data, name, mime, voice=voice)
        if kind == "sticker":
            resp = await client.send_sticker(jid, data)
        elif kind == "photo":
            resp = await client.send_image(jid, data, caption=None)
        elif kind in ("voice", "audio"):
            resp = await client.send_audio(jid, data, ptt=(kind == "voice"))
        elif kind == "video":
            resp = await client.send_video(jid, data, caption=None)
        else:
            resp = await client.send_document(jid, data, filename=name, mimetype=mime or None)
        if mid := _msg_id(resp):
            sent_ids.append(mid)

    result: dict[str, Any] = {}
    if sent_ids:
        result["message_id"] = sent_ids[0]
        if len(sent_ids) > 1:
            result["message_ids"] = sent_ids
    if opened:
        result["sent"] = len(opened)
    if choices_prompt is not None:
        result["choices_prompt"] = {"message_id": choices_prompt[0], "options": choices_prompt[1]}
    return result


async def _send_location(client: Any, jid: Any, location: dict[str, Any]) -> str:
    from neonize.proto.waE2E.WAWebProtobufsE2E_pb2 import LocationMessage, Message

    lat, lon = float(location["lat"]), float(location["lon"])
    name = str(location.get("name") or "").strip()
    msg = Message(locationMessage=LocationMessage(
        degreesLatitude=lat, degreesLongitude=lon, name=name))
    resp = await client.send_message(jid, msg)
    return _msg_id(resp)


async def _send_choices(client: Any, jid: Any, title: str, choices: list[str], *,
                        multiple: bool = False) -> str:
    builder = getattr(client, "build_poll_vote_creation", None)
    if callable(builder) and 2 <= len(choices) <= 12:
        from neonize.utils.enum import VoteType

        poll_msg = builder(title, choices, VoteType.MULTIPLE if multiple else VoteType.SINGLE)
        if hasattr(poll_msg, "__await__"):
            poll_msg = await poll_msg
        resp = await client.send_message(jid, poll_msg)
        return _msg_id(resp)
    lines = [title, *[f"{idx}. {opt}" for idx, opt in enumerate(choices, 1)]]
    resp = await client.send_message(jid, "\n".join(lines))
    return _msg_id(resp)


async def send_react(client: Any, place: str, emoji: str, message_id: str) -> dict[str, Any]:
    """React to one WhatsApp message with an emoji."""
    if client is None or not message_id or not emoji:
        return {}
    jid = jid_of(chat_jid_from_place(place))
    me = getattr(client, "me", None) or await client.get_me()
    built = await client.build_reaction(jid, me.JID, str(message_id), str(emoji))
    resp = await client.send_message(jid, built)
    mid = _msg_id(resp)
    return {"message_id": mid} if mid else {}


async def send_edit(client: Any, place: str, message_id: str, text: str) -> dict[str, Any]:
    """Edit one WhatsApp message in place."""
    from neonize.proto.waE2E.WAWebProtobufsE2E_pb2 import Message

    if client is None or not message_id:
        return {}
    jid = jid_of(chat_jid_from_place(place))
    resp = await client.edit_message(jid, str(message_id), Message(conversation=text))
    mid = _msg_id(resp, str(message_id))
    return {"message_id": mid}


async def send_show(client: Any, place: str, kind: str, **data: Any) -> Any:
    """Show typing, read receipts, edit, delete, reply, reactions, polls, stickers or geo."""
    if kind == "typing":
        if client is None:
            return None
        from neonize.utils.enum import ChatPresence, ChatPresenceMedia

        jid = jid_of(chat_jid_from_place(place))
        return await client.send_chat_presence(
            jid, ChatPresence.CHAT_PRESENCE_COMPOSING, ChatPresenceMedia.CHAT_PRESENCE_MEDIA_TEXT)
    if kind == "read_receipts":
        if client is None:
            return None
        jid = jid_of(chat_jid_from_place(place))
        ids = [str(x) for x in (data.get("message_ids") or [data.get("message_id")] or ()) if x]
        return await client.mark_read(*ids, chat=jid)
    if kind == "edit":
        mid = str(data.get("message_id") or "")
        if not mid:
            return await send_say(client, None, None, place, {"text": str(data.get("text") or "")})
        return await send_edit(client, place, mid, str(data.get("text") or ""))
    if kind == "delete":
        if client is None or not data.get("message_id"):
            return None
        jid = jid_of(chat_jid_from_place(place))
        return await client.revoke_message(jid, jid, str(data["message_id"]))
    if kind == "reply":
        return await send_say(client, None, None, place,
                              {"text": str(data.get("text") or ""),
                               "reply_to": data.get("message_id")})
    if kind == "reactions":
        return await send_react(client, place, str(data.get("emoji") or ""),
                                str(data.get("message_id") or ""))
    if kind == "polls":
        return await send_say(client, None, None, place,
                              {"text": str(data.get("question") or data.get("text") or "Vote"),
                               "choices": list(data.get("options") or ()),
                               "multiple": bool(data.get("multiple"))})
    if kind == "stickers":
        return await send_say(client, None, None, place,
                              {"text": str(data.get("emoji") or ""),
                               "sticker": data.get("sticker")})
    if kind == "locations":
        loc = data.get("location") if isinstance(data.get("location"), dict) else data
        return await send_say(client, None, None, place, {"location": loc})
    raise ValueError(f"cannot show {kind!r}")
