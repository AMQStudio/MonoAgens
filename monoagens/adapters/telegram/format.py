"""Telegram inbound updates to neutral events: messages, edits, reactions, polls, places."""

from __future__ import annotations

from collections.abc import Mapping, Sequence
from typing import Any

from monoagens.infra.clock import Clock
from monoagens.contract import describe_attachments
from monoagens.types import Event


def chat_place(chat: Mapping[str, Any], message: Mapping[str, Any]) -> tuple[str, str, str | None, str]:
    """Return `(place, form, parent, place_kind)` for a Telegram chat or forum topic."""
    chat_type = str(chat.get("type") or "")
    place_kind = "direct" if chat_type == "private" else "shared"
    topic = message.get("message_thread_id")
    if topic is not None and chat_type != "private":
        return (f"space:tg:{chat['id']}:{topic}", "thread",
                f"space:tg:{chat['id']}", place_kind)
    form = "dm" if chat_type == "private" else ("broadcast" if chat_type == "channel" else "group")
    return f"space:tg:{chat['id']}", form, None, place_kind


def place_meta(chat: Mapping[str, Any], form: str, parent: str | None) -> dict[str, Any]:
    """Return D68 place_info `{form, parent, can_post, name}` for a Telegram chat dict."""
    can_post = not (chat.get("type") == "channel" and chat.get("can_post") is False)
    if (chat.get("permissions") or {}).get("can_send_messages") is False:
        can_post = False
    meta: dict[str, Any] = {"form": form, "parent": parent, "can_post": can_post}
    name = str(chat.get("title") or chat.get("first_name") or chat.get("username") or "")
    if name:
        meta["name"] = name
    return meta


_chat_place = chat_place
_place_meta = place_meta


def _match_choice(text: str, reply_id: str,
                  choices_prompts: Mapping[str, list[str]]) -> tuple[str, str] | None:
    stripped = text.strip()
    if not stripped or not stripped.isdigit():
        return None
    idx = int(stripped)
    if idx < 1:
        return None
    if reply_id and reply_id in choices_prompts:
        opts = choices_prompts[reply_id]
        if idx <= len(opts):
            return opts[idx - 1], reply_id
    return None


def _place_event_summary(message: Mapping[str, Any], sender_label: str) -> tuple[dict[str, Any] | None, str]:
    if message.get("new_chat_members"):
        names = ", ".join(str(u.get("first_name") or u.get("id"))
                          for u in message["new_chat_members"])
        return {"kind": "member_joined", "members": list(message["new_chat_members"])}, (
            f"{names} joined the chat"
        )
    if message.get("left_chat_member"):
        who = message["left_chat_member"]
        label = str(who.get("first_name") or who.get("id") or "someone")
        return {"kind": "member_left", "member": dict(who)}, f"{label} left the chat"
    if message.get("new_chat_title"):
        title = str(message["new_chat_title"])
        return {"kind": "renamed", "name": title}, f"{sender_label} renamed the chat to {title}"
    if message.get("forum_topic_created"):
        topic = message["forum_topic_created"]
        return {"kind": "topic_created", "name": topic.get("name", "")}, (
            f"{sender_label} opened topic {topic.get('name', '')}".strip()
        )
    return None, ""


def has_content(message: Mapping[str, Any]) -> bool:
    """Return whether a Telegram message carries text or any structured Bot API payload."""
    if message.get("text") or message.get("caption"):
        return True
    keys = (
        "document", "photo", "audio", "video", "voice", "animation", "video_note",
        "sticker", "location", "venue", "contact", "poll",
        "new_chat_members", "left_chat_member", "new_chat_title", "forum_topic_created",
    )
    return any(message.get(k) for k in keys)


def event(update_id: int, message: dict[str, Any], text: str, *, clock: Clock,
          name: str = "telegram", bot_id: int | None = None,
          bot_username: str | None = None,
          attachments: Sequence[Mapping[str, Any]] = (),
          choices_prompts: Mapping[str, list[str]] | None = None,
          edited: bool = False) -> Event:
    """Build the percept of one incoming Telegram message or edited message."""
    chat = message["chat"]
    sender = message.get("from") or {}
    sender_id = sender.get("id")
    who = (f"telegram:{sender_id}" if sender_id is not None
           else f"telegram:chat:{chat['id']}")
    sender_label = str(sender.get("first_name") or "someone")
    place, form, parent, place_kind = _chat_place(chat, message)
    place_meta = _place_meta(chat, form, parent)

    msg_id = message.get("message_id")
    data: dict[str, Any] = {"message_id": msg_id}
    if edited and msg_id is not None:
        data["edits"] = str(msg_id)
    reply_msg = message.get("reply_to_message") or {}
    reply_id = str(reply_msg.get("message_id") or "") if reply_msg else ""
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

    effective_text = text or str(message.get("caption") or "")
    loc = message.get("location") or (message.get("venue") or {}).get("location")
    if isinstance(loc, Mapping):
        venue = message.get("venue") or {}
        loc_info: dict[str, Any] = {"lat": loc.get("latitude"), "lon": loc.get("longitude")}
        if venue.get("title"):
            loc_info["name"] = str(venue["title"])
        data["location"] = loc_info
    if message.get("contact"):
        c = message["contact"]
        cname = " ".join(p for p in (c.get("first_name"), c.get("last_name")) if p)
        data["contacts"] = [{"name": cname, "phone": str(c.get("phone_number") or "")}]
    if message.get("poll"):
        p = message["poll"]
        opts = [str(o.get("text") or "") for o in p.get("options") or ()]
        data["poll"] = {"id": str(p.get("id") or ""), "question": str(p.get("question") or ""),
                        "options": opts, "multiple": bool(p.get("allows_multiple_answers"))}
    if message.get("sticker"):
        st = message["sticker"]
        st_info: dict[str, Any] = {
            "emoji": str(st.get("emoji") or ""),
            "set_name": str(st.get("set_name") or ""),
            "animated": bool(st.get("is_animated") or st.get("is_video")),
        }
        if attachments:
            st_info["ref"] = attachments[0].get("ref")
        data["sticker"] = st_info
    if attachments:
        data["attachments"] = [dict(item) for item in attachments]

    place_ev, place_ev_desc = _place_event_summary(message, sender_label)
    if place_ev is not None:
        data["place_event"] = place_ev

    choice_matched = _match_choice(effective_text, reply_id, choices_prompts or {})
    if choice_matched is not None:
        val, q_id = choice_matched
        clean = effective_text.strip()
        data["choice"] = val
        data["answer"] = clean if clean.isdigit() else val
        data.setdefault("reply_to", {"message_id": int(q_id) if q_id.isdigit() else q_id})

    if place_ev_desc:
        description = place_ev_desc
    elif edited:
        description = f"{sender_label} edited: {effective_text}".strip()
    elif "sticker" in data and not effective_text:
        st = data["sticker"]
        pack = f" from '{st['set_name']}'" if st.get("set_name") else ""
        description = f"{sender_label} sent a sticker {st.get('emoji', '')}{pack}".strip()
    elif "location" in data and not effective_text:
        loc_info = data["location"]
        label = loc_info.get("name") or f"{loc_info.get('lat')}, {loc_info.get('lon')}"
        description = f"{sender_label} shared location: {label}"
    elif "contacts" in data and not effective_text:
        c = data["contacts"][0]
        description = f"{sender_label} shared contact: {c.get('name', '')} {c.get('phone', '')}".strip()
    elif "poll" in data and not effective_text:
        p = data["poll"]
        description = f"{sender_label} created poll '{p['question']}': {', '.join(p['options'])}"
    elif attachments:
        summary = describe_attachments(attachments, sent_by=sender_label)
        description = f"{effective_text} {summary}".strip() if effective_text else summary
    else:
        description = effective_text

    is_addressed = (
        addressed(chat, message, effective_text, bot_id=bot_id, bot_username=bot_username)
        or choice_matched is not None
    )
    return Event(
        id=None, ts=clock.now(), kind="percept", who=who, place=place, body=name,
        payload={
            "cursor": update_id,
            "data": data,
            "description": description,
            "place_kind": place_kind,
            "addressed": is_addressed,
        },
    )


def reaction_event(update_id: int, update: dict[str, Any], *, clock: Clock,
                   own_sent: set[str] | None = None, name: str = "telegram") -> Event | None:
    """Turn a Telegram `message_reaction` update into a percept."""
    chat = update.get("chat") or {}
    actor = update.get("user") or update.get("actor_chat") or {}
    if not chat or not actor.get("id"):
        return None
    msg_id = str(update.get("message_id") or "")
    emojis = [str(r.get("emoji") or r.get("custom_emoji_id") or "")
              for r in update.get("new_reaction") or ()]
    emoji = next((e for e in emojis if e), "")
    if not emoji:
        return None
    label = str(actor.get("first_name") or actor.get("title") or "someone")
    place, form, parent, place_kind = _chat_place(chat, update)
    is_own = bool(own_sent and msg_id in own_sent)
    return Event(
        id=None, ts=clock.now(), kind="percept",
        who=f"telegram:{actor['id']}", place=place, body=name,
        payload={
            "cursor": update_id,
            "data": {
                "message_id": msg_id,
                "reaction": {"message_id": msg_id, "emoji": emoji},
                "answer": emoji,
                **({"reply_to": {"message_id": msg_id}} if msg_id else {}),
            },
            "description": f"{label} reacted {emoji} to {msg_id}",
            "place_kind": place_kind,
            "addressed": is_own or place_kind == "direct",
        },
    )


def poll_answer_event(update_id: int, answer: dict[str, Any], *, clock: Clock,
                      poll_prompts: Mapping[str, tuple[str, str, list[str]]],
                      name: str = "telegram") -> Event | None:
    """Turn a Telegram `poll_answer` update into a percept with `data.choice` & `data.reply_to`."""
    poll_id = str(answer.get("poll_id") or "")
    user = answer.get("user") or answer.get("voter_chat") or {}
    if not poll_id or not user.get("id"):
        return None
    meta = poll_prompts.get(poll_id)
    if not meta:
        return None
    place, msg_id, options = meta
    option_ids = [int(i) for i in answer.get("option_ids") or ()]
    picked = [options[i] for i in option_ids if 0 <= i < len(options)]
    if not picked:
        return None
    choice_val: str | list[str] = picked[0] if len(picked) == 1 else picked
    label = str(user.get("first_name") or "someone")
    return Event(
        id=None, ts=clock.now(), kind="percept",
        who=f"telegram:{user['id']}", place=place, body=name,
        payload={
            "cursor": update_id,
            "data": {"choice": choice_val, "answer": choice_val,
                     "reply_to": {"message_id": msg_id}, "poll_id": poll_id},
            "description": f"{label} chose {choice_val}",
            "place_kind": "direct" if place.count(":") == 2 and not place.split(":")[2].startswith("-") else "shared",
            "addressed": True,
        },
    )


def addressed(chat: dict[str, Any], message: Mapping[str, Any], text: str, *,
              bot_id: int | None, bot_username: str | None) -> bool:
    """Return whether the message speaks to the soul: private chat, reply, or @mention."""
    if chat.get("type") == "private":
        return True
    reply_from = (message.get("reply_to_message") or {}).get("from") or {}
    if bot_id is not None and reply_from.get("id") == bot_id:
        return True
    if not bot_username:
        return False
    handle = f"@{bot_username}".lower()
    entities = (*(message.get("entities") or ()), *(message.get("caption_entities") or ()))
    return any(
        entity.get("type") == "mention"
        and text[entity["offset"]:entity["offset"] + entity["length"]].lower() == handle
        for entity in entities
    )
