"""Telegram body: long polling over httpx.

Two ports: run(put) turns updates into Events, act(action) performs say/react.
Neutral ids only: who = person:owner | person:tg:<id>, place = space:tg:<chat_id>.
"""

from __future__ import annotations

import asyncio
from collections.abc import Awaitable, Callable
from datetime import datetime, timezone
from typing import Any

import httpx

from monoagens.types import Action, Event

Put = Callable[[Event], Awaitable[None]]
GetCursor = Callable[[], Awaitable[Any | None]]


class TelegramBody:
    """Long-polling Telegram body. Advanced only after put() acknowledges."""

    def __init__(
        self,
        token: str,
        *,
        owner_ids: set[int] | frozenset[int] = frozenset(),
        get_cursor: GetCursor | None = None,
        poll_timeout: float = 30.0,
        transport: httpx.AsyncBaseTransport | None = None,
        api_base: str = "https://api.telegram.org",
    ) -> None:
        self._token = token
        self._owner_ids = set(owner_ids)
        self._get_cursor = get_cursor
        self._poll_timeout = poll_timeout
        self._transport = transport
        self._api_base = api_base.rstrip("/")
        self._client: httpx.AsyncClient | None = None
        self.bot_id: int | None = None
        self.bot_username: str | None = None

    def _http(self) -> httpx.AsyncClient:
        if self._client is None:
            # HTTP timeout must outlive the long poll.
            self._client = httpx.AsyncClient(
                transport=self._transport, timeout=self._poll_timeout + 10.0
            )
        return self._client

    async def aclose(self) -> None:
        """Close the HTTP client, if one was opened."""
        if self._client is not None:
            await self._client.aclose()
            self._client = None

    async def _call(self, method: str, **params: Any) -> Any:
        url = f"{self._api_base}/bot{self._token}/{method}"
        response = await self._http().post(url, json=params)
        response.raise_for_status()
        data = response.json()
        if not data.get("ok"):
            raise RuntimeError(f"Telegram {method} failed: {data.get('description')}")
        return data.get("result")

    async def run(self, put: Put) -> None:
        """Port 1: long-poll getUpdates, put one Event per text message."""
        try:
            me = await self._call("getMe")
            self.bot_id = me["id"]
            self.bot_username = me.get("username")

            cursor = await self._get_cursor() if self._get_cursor is not None else None
            offset = cursor + 1 if cursor is not None else None

            while True:
                params: dict[str, Any] = {"timeout": self._poll_timeout}
                if offset is not None:
                    params["offset"] = offset
                for update in await self._call("getUpdates", **params):
                    update_id = update["update_id"]
                    message = update.get("message")
                    text = message.get("text") if message else None
                    if not text:
                        # Non-text updates create no event but must not be re-read.
                        offset = update_id + 1
                        continue
                    await put(self._event(update_id, message, text))
                    # Advance only after the event is safely recorded.
                    offset = update_id + 1
                # Never spin hot when a poll returns without yielding.
                await asyncio.sleep(0)
        finally:
            await self.aclose()

    async def act(self, action: Action) -> None:
        """Port 2: say -> sendMessage, react -> setMessageReaction."""
        chat_id = action.place.removeprefix("space:tg:")
        if action.kind == "say":
            await self._call("sendMessage", chat_id=chat_id, text=action.payload["text"])
        elif action.kind == "react":
            await self._call(
                "setMessageReaction",
                chat_id=chat_id,
                message_id=action.payload["message_id"],
                reaction=[{"type": "emoji", "emoji": action.payload["emoji"]}],
            )
        else:
            raise ValueError(f"Unsupported action kind: {action.kind!r}")

    def _event(self, update_id: int, message: dict[str, Any], text: str) -> Event:
        chat = message["chat"]
        sender = message.get("from") or {}
        sender_id = sender.get("id")
        who = "person:owner" if sender_id in self._owner_ids else f"person:tg:{sender_id}"
        return Event(
            id=None,
            ts=datetime.now(timezone.utc),
            kind="msg.in",
            who=who,
            place=f"space:tg:{chat['id']}",
            body="telegram",
            payload={
                "cursor": update_id,
                "message_id": message["message_id"],
                "text": text,
                "addressed": self._addressed(chat, message, text),
            },
        )

    def _addressed(self, chat: dict[str, Any], message: dict[str, Any], text: str) -> bool:
        if chat.get("type") == "private":
            return True
        reply_from = (message.get("reply_to_message") or {}).get("from") or {}
        if reply_from.get("id") == self.bot_id:
            return True
        if not self.bot_username:
            return False
        handle = f"@{self.bot_username}".lower()
        for entity in message.get("entities") or []:
            if entity.get("type") != "mention":
                continue
            start, length = entity["offset"], entity["length"]
            if text[start : start + length].lower() == handle:
                return True
        return False
