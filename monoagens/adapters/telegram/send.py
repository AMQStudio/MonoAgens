"""Sending: messages, reactions, what the soul shows besides them, and place facts."""

from __future__ import annotations

from collections.abc import Awaitable, Callable, Sequence
from typing import Any

Call = Callable[..., Awaitable[Any]]


def chat_id(place: str) -> str:
    """Return Telegram's chat id for one neutral place id (`space:tg:<chat>` or topic)."""
    raw = str(place).removeprefix("space:tg:")
    return raw.partition(":")[0]


def thread_id(place: str) -> int | None:
    """Return the forum topic id when `place` names a thread (`space:tg:<chat>:<thread>`)."""
    raw = str(place).removeprefix("space:tg:")
    _, sep, topic = raw.partition(":")
    return int(topic) if sep and topic.lstrip("-").isdigit() else None


async def say(call: Call, place: str, text: str) -> Any:
    """Send one text message to one place (including forum topics)."""
    extra: dict[str, Any] = {}
    topic = thread_id(place)
    if topic is not None:
        extra["message_thread_id"] = topic
    return await call("sendMessage", chat_id=chat_id(place), text=text, **extra)


async def react(call: Call, place: str, emoji: str, message_id: int) -> Any:
    """React to one message with one emoji."""
    return await call("setMessageReaction", chat_id=chat_id(place), message_id=message_id,
                      reaction=[{"type": "emoji", "emoji": emoji}])


async def show(call: Call, place: str, kind: str, **data: Any) -> Any:
    """Show one non-message thing: typing, an edited message or buttons (middleware.md §9)."""
    if kind == "typing":
        return await call("sendChatAction", chat_id=chat_id(place), action="typing")
    if kind == "edit":
        return await call("editMessageText", chat_id=chat_id(place),
                          message_id=data.get("message_id"), text=str(data.get("text") or ""))
    if kind == "buttons":
        buttons: Sequence[tuple[str, str]] = data.get("buttons") or ()
        keyboard = [[{"text": label, "callback_data": value}] for label, value in buttons]
        return await call("sendMessage", chat_id=chat_id(place), text=str(data.get("text") or ""),
                          reply_markup={"inline_keyboard": keyboard})
    if kind == "reactions":
        return await react(call, place, str(data.get("emoji") or ""),
                           int(data.get("message_id") or 0))
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
    can_post, role_facts = await _can_post(call, raw_chat, chat, form, bot_id)
    info: dict[str, Any] = {
        "form": form,
        "parent": parent,
        "can_post": can_post,
        "name": chat.get("title") or chat.get("first_name") or chat.get("username") or "",
        "members": [],
        "roles": list(role_facts),
        "member_count": None,
        "role_facts": role_facts,
    }
    try:
        count = await call("getChatMemberCount", chat_id=raw_chat)
        info["member_count"] = count if isinstance(count, int) and not isinstance(count, bool) else None
    except Exception:  # a chat that reports no count simply has none
        info["member_count"] = None
    return info


async def document(http: Any, base_url: str, token: str, place: str, data: bytes,
                   info: dict[str, Any]) -> Any:
    """Send one stored file: a photo for images, a document for everything else."""
    mime = str(info.get("mime") or "application/octet-stream")
    method = "sendPhoto" if mime.startswith("image/") else "sendDocument"
    field = "photo" if method == "sendPhoto" else "document"
    form_data: dict[str, str] = {"chat_id": chat_id(place)}
    topic = thread_id(place)
    if topic is not None:
        form_data["message_thread_id"] = str(topic)
    response = await http.post(
        f"{base_url}/bot{token}/{method}",
        data=form_data,
        files={field: (str(info.get("name") or "file"), data, mime)})
    response.raise_for_status()
    payload = response.json()
    if not payload.get("ok"):
        raise RuntimeError(f"Telegram {method} failed: {payload.get('description')}")
    return payload.get("result")
