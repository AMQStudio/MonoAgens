"""Telegram inbound updates as the contract's percepts (§2.4.2, §5.4).

Everything the Bot API delivers becomes one `Percept`: text, replies and quotes (`about`),
mentions (`involves`), edits and deletions (`about` the earlier one), reactions by others,
stickers, locations, contact cards, polls and votes, media, calls and place changes. The
description is short natural language for reasoning; the facts are written only when certain.
"""

from __future__ import annotations

from collections.abc import Mapping, Sequence
from typing import Any

from monoagens.infra.clock import Clock
from monoagens.adapters import support
from monoagens.contract import Answer, Percept, describe_attachments

UNKNOWN_ACTOR = "unknown"


def place_id(chat: Mapping[str, Any], message: Mapping[str, Any]) -> str:
    """Return one Telegram chat or forum topic as a place id."""
    topic = message.get("message_thread_id")
    if topic is not None and str(chat.get("type")) != "private":
        return f"space:tg:{chat['id']}:{topic}"
    return f"space:tg:{chat['id']}"


def audience_of(chat: Mapping[str, Any], place: str, *, actor: str) -> Any:
    """Return who can perceive this chat, folding Telegram's rule into one fact (§2.4.3).

    A private chat is its two people; a group's newcomers do not see the past, yet the Bot API
    cannot list its members, so the group and the channel both report *unknown* rather than a
    guess.
    """
    from monoagens.contract import Audience

    if str(chat.get("type")) == "private":
        found = frozenset({actor}) if actor and actor != UNKNOWN_ACTOR else frozenset()
        return Audience(actors=found, unknown=not found)
    del place
    return Audience(actors=frozenset(), unknown=True)


def has_content(message: Mapping[str, Any]) -> bool:
    """Return whether a Telegram message carries text or any structured Bot API payload."""
    if message.get("text") or message.get("caption"):
        return True
    keys = (
        "document", "photo", "audio", "video", "voice", "animation", "video_note",
        "sticker", "location", "venue", "contact", "poll",
        "new_chat_members", "left_chat_member", "new_chat_title", "forum_topic_created",
        "pinned_message",
    )
    return any(message.get(k) for k in keys)


def _place_change(message: Mapping[str, Any], sender_label: str) -> tuple[dict[str, Any] | None, str]:
    """Return the place change one message carries and the line reasoning reads for it."""
    if message.get("new_chat_members"):
        names = ", ".join(str(u.get("first_name") or u.get("id"))
                          for u in message["new_chat_members"])
        return {"kind": "member_joined",
                "members": [str(u.get("id")) for u in message["new_chat_members"]]}, (
            f"{names} joined the chat")
    if message.get("left_chat_member"):
        who = message["left_chat_member"]
        label = str(who.get("first_name") or who.get("id") or "someone")
        return {"kind": "member_left", "member": str(who.get("id"))}, f"{label} left the chat"
    if message.get("new_chat_title"):
        title = str(message["new_chat_title"])
        return {"kind": "renamed", "name": title}, f"{sender_label} renamed the chat to {title}"
    if message.get("forum_topic_created"):
        topic = message["forum_topic_created"]
        return {"kind": "topic_created", "name": topic.get("name", "")}, (
            f"{sender_label} opened topic {topic.get('name', '')}".strip())
    if message.get("pinned_message"):
        return {"kind": "pinned"}, f"{sender_label} pinned a message"
    return None, ""


def _who(chat: Mapping[str, Any], message: Mapping[str, Any]) -> str:
    """Return one stable actor id for the message's sender, or *unknown* when certain only."""
    sender = message.get("from") or message.get("sender_chat") or {}
    sender_id = sender.get("id")
    if sender_id is not None:
        return f"telegram:{sender_id}"
    if str(chat.get("type")) == "channel":
        return UNKNOWN_ACTOR
    return f"telegram:chat:{chat['id']}"


def event(update_id: int, message: dict[str, Any], text: str, *, clock: Clock,
          name: str = "telegram", bot_id: int | None = None,
          bot_username: str | None = None,
          attachments: Sequence[Mapping[str, Any]] = (),
          edited: bool = False) -> Percept:
    """Build the percept of one incoming Telegram message or edited message."""
    del name
    chat = message["chat"]
    sender = message.get("from") or {}
    actor = _who(chat, message)
    sender_label = str(sender.get("first_name") or sender.get("title") or "someone")
    place = place_id(chat, message)

    msg_id = str(message.get("message_id") or "")
    data: dict[str, Any] = {"cursor": update_id, "chat_id": chat["id"], "chat": dict(chat)}
    if msg_id:
        data["message_id"] = msg_id
    about = ""
    reply_msg = message.get("reply_to_message") or {}
    if reply_msg.get("message_id") is not None:
        about = str(reply_msg["message_id"])
        reply_from = (reply_msg.get("from") or {}).get("id")
        data["reply_to"] = {"message_id": reply_msg["message_id"]}
        if reply_from is not None:
            data["reply_to"]["who"] = f"telegram:{reply_from}"
    if edited and msg_id:
        about = msg_id
        data["edits"] = msg_id

    involves = list(dict.fromkeys(
        f"telegram:{entity['user']['id']}"
        for entity in (*(message.get("entities") or ()), *(message.get("caption_entities") or ()))
        if entity.get("type") == "text_mention" and (entity.get("user") or {}).get("id")))
    reply_from_id = (reply_msg.get("from") or {}).get("id")
    if reply_from_id is not None and f"telegram:{reply_from_id}" not in involves:
        involves.append(f"telegram:{reply_from_id}")

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
        data["poll"] = {"id": str(p.get("id") or ""), "question": str(p.get("question") or ""),
                        "options": [str(o.get("text") or "") for o in p.get("options") or ()],
                        "multiple": bool(p.get("allows_multiple_answers"))}
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
    if message.get("voice"):
        data["voice"] = True
    if message.get("video_note"):
        data["video_note"] = True

    change, change_line = _place_change(message, sender_label)
    if change is not None:
        data["place_event"] = change
    if edited:
        data.setdefault("before", effective_text)

    if change_line:
        description = change_line
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
        description = (f"{sender_label} shared contact: {c.get('name', '')} "
                       f"{c.get('phone', '')}").strip()
    elif "poll" in data and not effective_text:
        p = data["poll"]
        description = f"{sender_label} created poll '{p['question']}': {', '.join(p['options'])}"
    elif attachments:
        summary = describe_attachments(attachments, sent_by=sender_label)
        description = f"{effective_text} {summary}".strip() if effective_text else summary
    else:
        description = f"{sender_label}: {effective_text}" if actor != UNKNOWN_ACTOR \
            else effective_text

    spoken_to = addressed(chat, message, effective_text, bot_id=bot_id,
                          bot_username=bot_username)
    named = named_soul(message, effective_text, bot_username=bot_username)
    return Percept(
        actor=actor,
        place=place,
        occurred_at=clock.now(),
        description=description,
        ref=msg_id,
        about=about,
        involves=tuple(involves),
        attachments=support.attachments_of(attachments),
        features={"addressed": 1 if spoken_to else 0, "named": named,
                  "addressed_other": 0},
        data=data,
    )


def reaction_event(update_id: int, update: dict[str, Any], *, clock: Clock,
                   own_sent: set[str] | None = None, name: str = "telegram",
                   bot_id: int | None = None) -> Percept | None:
    """Turn a Telegram `message_reaction` update into the percept of one reaction by someone."""
    del name
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
    place = place_id(chat, update)
    is_own = bool(own_sent and msg_id in own_sent)
    direct = str(chat.get("type")) == "private"
    return Percept(
        actor=f"telegram:{actor['id']}",
        place=place,
        occurred_at=clock.now(),
        description=f"{label} reacted {emoji} to {msg_id}",
        about=msg_id,
        features={"addressed": 1 if (is_own or direct) else 0, "named": 0,
                  "addressed_other": 1 if not is_own else 0},
        data={"cursor": update_id, "chat_id": chat["id"], "message_id": msg_id,
              "reaction": {"message_id": msg_id, "emoji": emoji},
              "own": is_own, "bot_id": bot_id},
    )


def poll_answer_event(update_id: int, answer: dict[str, Any], *, clock: Clock,
                      poll_prompts: Mapping[str, tuple[str, str, list[str]]],
                      name: str = "telegram") -> Percept | None:
    """Turn a Telegram `poll_answer` into the percept that answers the question it followed."""
    del name
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
    label = str(user.get("first_name") or "someone")
    value = picked[0] if len(picked) == 1 else " | ".join(picked)
    return Percept(
        actor=f"telegram:{user['id']}",
        place=place,
        occurred_at=clock.now(),
        description=f"{label} chose {value}",
        about=msg_id,
        features={"addressed": 1, "named": 0, "addressed_other": 0},
        answer=Answer(prompt=msg_id, value=value),
        data={"cursor": update_id, "poll_id": poll_id, "choice": value},
    )


def command_event(update_id: int, message: Mapping[str, Any], *, clock: Clock) -> Percept | None:
    """Return the percept of one typed `/command args`, or None (§2.4.7)."""
    text = str(message.get("text") or message.get("caption") or "").strip()
    if not text.startswith("/"):
        return None
    parts = text[1:].split()
    if not parts:
        return None
    chat = message["chat"]
    return Percept(
        actor=_who(chat, message),
        place=place_id(chat, message),
        occurred_at=clock.now(),
        description=text,
        ref=str(message.get("message_id") or ""),
        features={"addressed": 1, "named": 0, "addressed_other": 0},
        data={"cursor": update_id, "chat_id": chat["id"], "chat": dict(chat)},
    )


def addressed(chat: dict[str, Any], message: Mapping[str, Any], text: str, *,
              bot_id: int | None, bot_username: str | None) -> bool:
    """Return whether the message speaks to the soul: private chat, reply, or @mention."""
    if chat.get("type") == "private":
        return True
    reply_from = (message.get("reply_to_message") or {}).get("from") or {}
    if bot_id is not None and reply_from.get("id") == bot_id:
        return True
    return bool(named_soul(message, text, bot_username=bot_username))


def named_soul(message: Mapping[str, Any], text: str, *, bot_username: str | None) -> bool:
    """Return whether the message names the soul by its handle on this platform."""
    if not bot_username:
        return False
    handle = f"@{bot_username}".lower()
    entities = (*(message.get("entities") or ()), *(message.get("caption_entities") or ()))
    return any(
        entity.get("type") == "mention"
        and text[entity.get("offset", 0):entity.get("offset", 0) + entity.get("length", 0)].lower()
        == handle
        for entity in entities
    )
