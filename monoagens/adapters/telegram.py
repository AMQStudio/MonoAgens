"""Telegram adapter: long polling over httpx; neutral ids only (who/place)."""

from __future__ import annotations

import asyncio
from collections.abc import Awaitable, Callable
from typing import Any

import httpx

from monoagens.infra.clock import Clock
from monoagens.types import Action, Event

Put = Callable[[Event], Awaitable[None]]
GetCursor = Callable[[], Awaitable[Any | None]]

CAPABILITIES = (
    {"name": "say", "description": "Send a text message in the current place.",
     "schema": {"type": "object", "properties": {"text": {"type": "string"}},
                "required": ["text"]},
     "communicative": True, "long_running": False, "identity_changing": False,
     "permission": None},
    {"name": "react", "description": "React to a message with one emoji.",
     "schema": {"type": "object",
                "properties": {"emoji": {"type": "string"},
                               "message_id": {"type": "integer"}},
                "required": ["emoji"]},
     "communicative": False, "long_running": False, "identity_changing": False,
     "permission": None},
)


class TelegramBody:
    """Eyes and hands for Telegram: getUpdates in, sendMessage and reactions out."""

    name = "telegram"

    def __init__(self, token: str, *,
                 get_cursor: GetCursor | None = None, poll_timeout: float = 30.0,
                 transport: httpx.AsyncBaseTransport | None = None,
                 api_base: str = "https://api.telegram.org", retry_base: float = 1.0,
                 clock: Clock | None = None) -> None:
        """Store the token and the polling settings."""
        self._token = token
        self._clock = clock or Clock()
        self._get_cursor = get_cursor
        self._poll_timeout = poll_timeout
        self._transport = transport
        self._api_base = api_base.rstrip("/")
        self._retry_base = retry_base
        self._client: httpx.AsyncClient | None = None
        self.bot_id: int | None = None
        self.bot_username: str | None = None

    def capabilities(self) -> tuple[dict[str, Any], ...]:
        """Return the capability descriptors this adapter declares."""
        return CAPABILITIES

    def _http(self) -> httpx.AsyncClient:
        if self._client is None:
            # the timeout must outlive the long poll
            self._client = httpx.AsyncClient(
                transport=self._transport, timeout=self._poll_timeout + 10.0
            )
        return self._client

    async def aclose(self) -> None:
        """Close the underlying HTTP client."""
        if self._client is not None:
            await self._client.aclose()
            self._client = None

    async def _call(self, method: str, **params: Any) -> Any:
        response = await self._http().post(
            f"{self._api_base}/bot{self._token}/{method}", json=params
        )
        response.raise_for_status()
        data = response.json()
        if not data.get("ok"):
            raise RuntimeError(f"Telegram {method} failed: {data.get('description')}")
        return data.get("result")

    async def run(self, put: Put) -> None:
        """Poll Telegram forever, putting one event per text message."""
        try:
            me = await self._call("getMe")
            self.bot_id, self.bot_username = me["id"], me.get("username")
            cursor = await self._get_cursor() if self._get_cursor is not None else None
            offset = cursor + 1 if cursor is not None else None
            backoff = self._retry_base
            while True:
                params: dict[str, Any] = {"timeout": self._poll_timeout}
                params.update({"offset": offset} if offset is not None else {})
                try:
                    updates = await self._call("getUpdates", **params)
                except (httpx.HTTPError, RuntimeError):
                    # A flaky network or a Telegram hiccup never kills the adapter.
                    await asyncio.sleep(backoff)
                    backoff = min(backoff * 2, 30.0)
                    continue
                backoff = self._retry_base
                for update in updates:
                    update_id = update["update_id"]
                    message = update.get("message")
                    text = message.get("text") if message else None
                    if text:
                        await put(self._event(update_id, message, text))
                    # Advance only after the event is safely recorded.
                    offset = update_id + 1
                # never spin hot when a poll returns without yielding
                await asyncio.sleep(0)
        finally:
            await self.aclose()

    async def act(self, action: Action) -> None:
        """Perform one say or react action in a Telegram chat."""
        chat_id = action.place.removeprefix("space:tg:")
        if action.kind == "say":
            await self._call("sendMessage", chat_id=chat_id, text=action.payload["text"])
        elif action.kind == "react":
            await self._call("setMessageReaction", chat_id=chat_id,
                             message_id=action.payload["message_id"],
                             reaction=[{"type": "emoji", "emoji": action.payload["emoji"]}])
        else:
            raise ValueError(f"Unsupported action kind: {action.kind!r}")

    def _event(self, update_id: int, message: dict[str, Any], text: str) -> Event:
        chat = message["chat"]
        sender_id = (message.get("from") or {}).get("id")
        who = f"person:tg:{sender_id}"
        place_kind = "direct" if chat.get("type") == "private" else "shared"
        return Event(id=None, ts=self._clock.now(), kind="percept", who=who,
                     place=f"space:tg:{chat['id']}", body=self.name,
                     payload={"cursor": update_id,
                              "data": {"message_id": message["message_id"]},
                              "description": text, "place_kind": place_kind,
                              "addressed": self._addressed(chat, message, text)})

    def _addressed(self, chat: dict[str, Any], message: dict[str, Any], text: str) -> bool:
        if chat.get("type") == "private":
            return True
        reply_from = (message.get("reply_to_message") or {}).get("from") or {}
        if reply_from.get("id") == self.bot_id:
            return True
        if not self.bot_username:
            return False
        handle = f"@{self.bot_username}".lower()
        return any(
            entity.get("type") == "mention"
            and text[entity["offset"]:entity["offset"] + entity["length"]].lower() == handle
            for entity in message.get("entities") or []
        )
