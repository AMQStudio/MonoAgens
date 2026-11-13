"""Sending: messages, replies, mentions, stickers, choices/polls, threads, locations, edits."""

from __future__ import annotations

import contextlib
from collections.abc import Awaitable, Callable, Sequence
from typing import Any

Call = Callable[..., Awaitable[Any]]

MAX_INLINE_CHOICES = 4
MAX_POLL_CHOICES = 10


def chat_id(place: str) -> str:
    """Return Telegram's chat id for one neutral place id (`space:tg:<chat>` or topic)."""
    raw = str(place).removeprefix("space:tg:")
    return raw.partition(":")[0]


def thread_id(place: str) -> int | None:
    """Return the forum topic id when `place` names a thread (`space:tg:<chat>:<thread>`)."""
    raw = str(place).removeprefix("space:tg:")
    _, sep, topic = raw.partition(":")
    return int(topic) if sep and topic.lstrip("-").isdigit() else None


def numbered_choices(choices: Sequence[str]) -> str:
    """Format choices as a numbered list fallback."""
    return "\n".join(f"{i}. {opt}" for i, opt in enumerate(choices, start=1))


def format_location_text(location: dict[str, Any]) -> str:
    """Fallback text when a message already uses sendMessage alongside location."""
    name = str(location.get("name") or "Location")
    return f"{name} ({location.get('lat')}, {location.get('lon')})"


def _with_mentions(text: str, mentions: Sequence[str]) -> str:
    tags = []
    for raw in mentions:
        uid = str(raw).removeprefix("telegram:").strip()
        if not uid:
            continue
        tags.append(uid if uid.startswith("@") else f"@{uid}")
    if not tags:
        return text
    prefix = " ".join(tags)
    return f"{prefix} {text}".strip() if text else prefix


async def say(call: Call, place: str, text: str = "", *,
              reply_to: str | None = None, mentions: Sequence[str] = (),
              sticker: str | None = None, choices: Sequence[str] = (),
              multiple: bool = False, thread: str | None = None,
              location: dict[str, Any] | None = None) -> Any:
    """Send a Telegram message cleanly preserving plain calls while supporting neutral fields."""
    cid = chat_id(place)
    extra: dict[str, Any] = {}
    topic = thread_id(place)
    if topic is not None:
        extra["message_thread_id"] = topic
    if thread:
        with contextlib.suppress(Exception):
            created = await call("createForumTopic", chat_id=cid, name=thread[:128])
            if isinstance(created, dict) and created.get("message_thread_id"):
                extra["message_thread_id"] = int(created["message_thread_id"])
            else:
                text = f"[{thread}]\n{text}".strip() if text else f"[{thread}]"
    if reply_to:
        mid = int(reply_to) if str(reply_to).lstrip("-").isdigit() else reply_to
        extra["reply_parameters"] = {"message_id": mid}

    rendered = _with_mentions(text, mentions)
    choice_list = [str(c) for c in choices if str(c)]

    if sticker and not rendered and not choice_list and not location:
        return await call("sendSticker", chat_id=cid, sticker=sticker, **extra)

    if location and not rendered and not choice_list:
        return await call("sendLocation", chat_id=cid,
                          latitude=float(location["lat"]), longitude=float(location["lon"]),
                          **extra)

    if location and rendered:
        rendered = f"{rendered}\n{format_location_text(location)}"

    if choice_list:
        if len(choice_list) <= MAX_INLINE_CHOICES and not multiple:
            keyboard = [[{"text": opt, "callback_data": f"choice:{i}:{opt[:32]}"}]
                        for i, opt in enumerate(choice_list, start=1)]
            extra["reply_markup"] = {"inline_keyboard": keyboard}
        elif len(choice_list) <= MAX_POLL_CHOICES:
            poll_args: dict[str, Any] = {
                "chat_id": cid,
                "question": (rendered or "Choose one:")[:300],
                "options": [{"text": opt[:100]} for opt in choice_list],
                "allows_multiple_answers": bool(multiple),
                "is_anonymous": False,
                **extra,
            }
            return await call("sendPoll", **poll_args)
        else:
            rendered = f"{rendered}\n{numbered_choices(choice_list)}".strip()

    if sticker and rendered:
        with contextlib.suppress(Exception):
            await call("sendSticker", chat_id=cid, sticker=sticker, **extra)

    return await call("sendMessage", chat_id=cid, text=rendered, **extra)


async def react(call: Call, place: str, emoji: str, message_id: int) -> Any:
    """React to one message with one emoji."""
    return await call("setMessageReaction", chat_id=chat_id(place), message_id=message_id,
                      reaction=[{"type": "emoji", "emoji": emoji}])


async def edit(call: Call, place: str, message_id: int, text: str) -> Any:
    """Edit one of the soul's own Telegram messages."""
    return await call("editMessageText", chat_id=chat_id(place),
                      message_id=message_id, text=text)


async def pin(call: Call, place: str, message_id: int, *, silent: bool = False) -> Any:
    """Pin one message in a Telegram chat (optional tool)."""
    return await call("pinChatMessage", chat_id=chat_id(place), message_id=message_id,
                      disable_notification=silent)


async def unpin(call: Call, place: str, message_id: int) -> Any:
    """Unpin one message in a Telegram chat (optional tool)."""
    return await call("unpinChatMessage", chat_id=chat_id(place), message_id=message_id)


async def sync_commands(call: Call, commands: Sequence[dict[str, str]]) -> Any:
    """Publish the soul's slash commands menu via Bot API `setMyCommands`."""
    entries = [{"command": str(c["command"]).lstrip("/")[:32],
                "description": str(c.get("description") or c["command"])[:256]}
               for c in commands if c.get("command")]
    return await call("setMyCommands", commands=entries)


async def show(call: Call, place: str, kind: str, **data: Any) -> Any:
    """Show one non-message thing natively where Telegram's Bot API supports it."""
    cid = chat_id(place)
    topic = thread_id(place)
    t_extra = {"message_thread_id": topic} if topic is not None else {}
    if kind == "typing":
        return await call("sendChatAction", chat_id=cid, action="typing", **t_extra)
    if kind == "edit":
        mid = data.get("message_id")
        if not mid:
            return await call("sendMessage", chat_id=cid, text=str(data.get("text") or ""),
                              **t_extra)
        return await call("editMessageText", chat_id=cid,
                          message_id=mid, text=str(data.get("text") or ""))
    if kind == "delete":
        return await call("deleteMessage", chat_id=cid, message_id=data.get("message_id"))
    if kind in ("buttons", "menus"):
        buttons: Sequence[tuple[str, str]] = data.get("buttons") or ()
        keyboard = [[{"text": label, "callback_data": value}] for label, value in buttons]
        return await call("sendMessage", chat_id=cid, text=str(data.get("text") or ""),
                          reply_markup={"inline_keyboard": keyboard}, **t_extra)
    if kind == "reactions":
        return await react(call, place, str(data.get("emoji") or ""),
                           int(data.get("message_id") or 0))
    if kind == "reply":
        return await call("sendMessage", chat_id=cid, text=str(data.get("text") or ""),
                          reply_parameters={"message_id": data.get("message_id")}, **t_extra)
    if kind == "polls":
        opts = [{"text": str(o)[:100]} for o in (data.get("options") or ())]
        return await call("sendPoll", chat_id=cid,
                          question=str(data.get("question") or data.get("text") or "Vote"),
                          options=opts, allows_multiple_answers=bool(data.get("multiple")),
                          is_anonymous=False, **t_extra)
    if kind == "stickers":
        if data.get("sticker") or data.get("file_id"):
            return await call("sendSticker", chat_id=cid,
                              sticker=str(data.get("sticker") or data.get("file_id")), **t_extra)
        return await call("sendMessage", chat_id=cid, text=str(data.get("emoji") or "🏷️"),
                          **t_extra)
    if kind == "threads":
        return await call("createForumTopic", chat_id=cid,
                          name=str(data.get("title") or data.get("text") or "Topic")[:128])
    if kind == "locations":
        loc = data.get("location") if isinstance(data.get("location"), dict) else data
        return await call("sendLocation", chat_id=cid,
                          latitude=float(loc.get("lat") or 0.0),
                          longitude=float(loc.get("lon") or 0.0), **t_extra)
    raise ValueError(f"cannot show {kind!r}")


def _chat_form(chat: dict[str, Any], place: str) -> tuple[str, str | None]:
    """Return `(form, parent)` for a Telegram chat or forum topic place."""
    if thread_id(place) is not None:
        return "thread", f"space:tg:{chat_id(place)}"
    kind = str(chat.get("type") or "")
    if kind == "private":
        return "dm", None
    if kind == "channel":
        return "broadcast", None
    return "group", None


async def _can_post(call: Call, raw_chat_id: str, chat: dict[str, Any], form: str,
                    bot_id: int | None) -> tuple[bool, list[str]]:
    """Return `(can_post, role_facts)` from chat permissions and bot membership."""
    perms = chat.get("permissions") or {}
    if perms.get("can_send_messages") is False or chat.get("can_post") is False:
        return False, ["read-only"]
    facts: list[str] = []
    if bot_id is not None and form in ("broadcast", "group", "thread"):
        try:
            member = await call("getChatMember", chat_id=raw_chat_id, user_id=bot_id)
        except Exception:
            member = None
        if isinstance(member, dict):
            status = str(member.get("status") or "")
            if status:
                facts.append(status)
            if status in ("left", "kicked", "member") and form == "broadcast":
                return False, facts
            if status == "restricted" and member.get("can_send_messages") is False:
                return False, facts
            if (form == "broadcast" and status == "administrator"
                    and member.get("can_post_messages") is False):
                return False, facts
    return True, facts


async def place_info(call: Call, place: str, *, bot_id: int | None = None) -> dict[str, Any]:
    """Return structured place info `{form, parent, can_post, name, members, roles}`."""
    raw_chat = chat_id(place)
    chat = await call("getChat", chat_id=raw_chat) or {}
    if not isinstance(chat, dict):
        chat = {}
    form, parent = _chat_form(chat, place)
    can_post, roles = await _can_post(call, raw_chat, chat, form, bot_id)
    members: int | None = None
    try:
        count = await call("getChatMemberCount", chat_id=raw_chat)
        if isinstance(count, int):
            members = count
    except Exception:
        members = None
    name = str(chat.get("title") or chat.get("first_name") or chat.get("username") or "")
    info: dict[str, Any] = {"form": form, "parent": parent, "can_post": can_post, "roles": roles}
    if name:
        info["name"] = name
    if members is not None:
        info["members"] = members
    return info


async def document(http: Any, base_url: str, token: str, place: str,
                   data: bytes, info: dict[str, Any]) -> Any:
    """Upload one attachment cleanly to Telegram (`sendSticker` for webp stickers if marked)."""
    files = {"document": (str(info.get("name") or "file"), data,
                          str(info.get("mime") or "application/octet-stream"))}
    params: dict[str, Any] = {"chat_id": chat_id(place)}
    topic = thread_id(place)
    if topic is not None:
        params["message_thread_id"] = str(topic)
    response = await http.post(f"{base_url}/bot{token}/sendDocument", data=params, files=files)
    response.raise_for_status()
    payload = response.json()
    if not payload.get("ok"):
        raise RuntimeError(f"Telegram sendDocument failed: {payload.get('description')}")
    return payload.get("result")
