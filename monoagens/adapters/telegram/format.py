"""One Telegram update becomes one neutral event: ids, the text, and whether it is addressed."""

from __future__ import annotations

from collections.abc import Mapping, Sequence
from typing import Any

from monoagens.infra.clock import Clock
from monoagens.middleware.contract import describe_attachments
from monoagens.types import Event


def event(update_id: int, message: dict[str, Any], text: str, *, clock: Clock,
          name: str = "telegram", bot_id: int | None = None,
          bot_username: str | None = None,
          attachments: Sequence[Mapping[str, Any]] = ()) -> Event:
    """Build the percept of one incoming message; ids stay opaque, nothing is interpreted."""
    chat = message["chat"]
    sender = message.get("from") or {}
    sender_id = sender.get("id")
    who = (f"telegram:{sender_id}" if sender_id is not None
           else f"telegram:chat:{chat['id']}")
    sender_label = str(sender.get("first_name") or "someone")
    chat_type = str(chat.get("type") or "")
    place_kind = "direct" if chat_type == "private" else "shared"
    topic = message.get("message_thread_id")
    if topic is not None and chat_type != "private":
        place = f"space:tg:{chat['id']}:{topic}"
        form, parent = "thread", f"space:tg:{chat['id']}"
    else:
        place = f"space:tg:{chat['id']}"
        form = "dm" if chat_type == "private" else ("broadcast" if chat_type == "channel"
                                                    else "group")
        parent = None
    place_name = str(chat.get("title") or chat.get("first_name") or chat.get("username") or "")
    can_post = not (chat_type == "channel" and chat.get("can_post") is False)
    if (chat.get("permissions") or {}).get("can_send_messages") is False:
        can_post = False
    data: dict[str, Any] = {"message_id": message["message_id"]}
    reply_msg = message.get("reply_to_message") or {}
    if reply_msg:
        reply_from = (reply_msg.get("from") or {}).get("id")
        reply_info: dict[str, Any] = {"message_id": reply_msg.get("message_id")}
        if reply_from is not None:
            reply_info["who"] = f"telegram:{reply_from}"
        data["reply_to"] = reply_info
    mentions = [
        f"telegram:{entity['user']['id']}"
        for entity in (*(message.get("entities") or ()),
                       *(message.get("caption_entities") or ()))
        if entity.get("type") == "text_mention" and (entity.get("user") or {}).get("id")
    ]
    if mentions:
        data["mentions"] = list(dict.fromkeys(mentions))
    description = text
    if attachments:
        data["attachments"] = [dict(item) for item in attachments]
        summary = describe_attachments(attachments, sent_by=sender_label)
        description = f"{text} {summary}".strip() if text else summary
    place_meta: dict[str, Any] = {"form": form, "parent": parent, "can_post": can_post}
    if place_name:
        place_meta["name"] = place_name
    return Event(id=None, ts=clock.now(), kind="percept", who=who,
                 place=place, body=name,
                 payload={"cursor": update_id, "data": data, "description": description,
                          "place_kind": place_kind, "place_info": place_meta,
                          "addressed": addressed(chat, message, text, bot_id=bot_id,
                                                 bot_username=bot_username)})


def addressed(chat: dict[str, Any], message: dict[str, Any], text: str, *,
              bot_id: int | None, bot_username: str | None) -> bool:
    """Return whether the message speaks to the soul: a private chat, a reply, or a mention."""
    if chat.get("type") == "private":
        return True
    reply_from = (message.get("reply_to_message") or {}).get("from") or {}
    if reply_from.get("id") == bot_id:
        return True
    if not bot_username:
        return False
    handle = f"@{bot_username}".lower()
    return any(
        entity.get("type") == "mention"
        and text[entity["offset"]:entity["offset"] + entity["length"]].lower() == handle
        for entity in message.get("entities") or []
    )
