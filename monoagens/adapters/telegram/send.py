"""Sending: messages, reactions, what the soul shows besides them, and place facts."""

from __future__ import annotations

from collections.abc import Awaitable, Callable, Sequence
from typing import Any

Call = Callable[..., Awaitable[Any]]


def chat_id(place: str) -> str:
    """Return Telegram's chat id for one neutral place id."""
    return str(place).removeprefix("space:tg:")


async def say(call: Call, place: str, text: str) -> Any:
    """Send one text message to one place."""
    return await call("sendMessage", chat_id=chat_id(place), text=text)


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


async def place_info(call: Call, place: str) -> dict[str, Any]:
    """Return what Telegram reports about one chat: its name and how many members it has."""
    chat = await call("getChat", chat_id=chat_id(place)) or {}
    info: dict[str, Any] = {"name": chat.get("title") or chat.get("first_name")
                            or chat.get("username") or "", "member_count": None,
                            "role_facts": []}
    try:
        info["member_count"] = await call("getChatMemberCount", chat_id=chat_id(place))
    except Exception:  # a chat that reports no count simply has none
        info["member_count"] = None
    return info


async def document(http: Any, base_url: str, token: str, place: str, data: bytes,
                   info: dict[str, Any]) -> Any:
    """Send one stored file: a photo for images, a document for everything else."""
    mime = str(info.get("mime") or "application/octet-stream")
    method = "sendPhoto" if mime.startswith("image/") else "sendDocument"
    field = "photo" if method == "sendPhoto" else "document"
    response = await http.post(
        f"{base_url}/bot{token}/{method}",
        data={"chat_id": chat_id(place)},
        files={field: (str(info.get("name") or "file"), data, mime)})
    response.raise_for_status()
    payload = response.json()
    if not payload.get("ok"):
        raise RuntimeError(f"Telegram {method} failed: {payload.get('description')}")
    return payload.get("result")
