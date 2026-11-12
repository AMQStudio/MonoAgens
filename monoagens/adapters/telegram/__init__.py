"""Telegram adapter: long polling over httpx; neutral ids only (who/place)."""

from __future__ import annotations

import asyncio
from collections.abc import Awaitable, Callable
from typing import Any

import httpx

from monoagens.adapters.telegram import receive, send
from monoagens.adapters.telegram.capabilities import CAPABILITIES, SHOWS
from monoagens.adapters.telegram.format import addressed, event
from monoagens.infra.clock import Clock
from monoagens.types import Action, Event

Put = Callable[[Event], Awaitable[None]]
GetCursor = Callable[[], Awaitable[Any | None]]
REQUIRES = ("TELEGRAM_BOT_TOKEN",)


def create(options: dict[str, Any], env: dict[str, str]) -> "TelegramBody":
    """Build the adapter from config.yaml's options and the environment (D49)."""
    token = str(env.get("TELEGRAM_BOT_TOKEN") or "")
    if not token:
        raise RuntimeError("telegram: TELEGRAM_BOT_TOKEN is not set")
    return TelegramBody(
        token,
        api_base=str(options.get("api_base") or "https://api.telegram.org"),
        poll_timeout=float(options.get("poll_timeout") or 30.0),
    )


class TelegramBody:
    """Eyes and hands for Telegram: getUpdates in, sendMessage and reactions out."""

    name = "telegram"
    shows = SHOWS

    def __init__(self, token: str, *,
                 get_cursor: GetCursor | None = None, poll_timeout: float = 30.0,
                 transport: httpx.AsyncBaseTransport | None = None,
                 api_base: str = "https://api.telegram.org", retry_base: float = 1.0,
                 clock: Clock | None = None) -> None:
        """Store the token and the polling settings."""
        self._token = token
        self._clock = clock or Clock()
        self.get_cursor = get_cursor
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
        """Poll Telegram forever, putting one event per incoming message."""
        try:
            me = await self._call("getMe")
            self.bot_id, self.bot_username = me["id"], me.get("username")
            cursor = await self.get_cursor() if self.get_cursor is not None else None
            offset = cursor + 1 if cursor is not None else None
            backoff = self._retry_base
            while True:
                try:
                    found = await receive.updates(self._call, offset, self._poll_timeout)
                except (httpx.HTTPError, RuntimeError):
                    # A flaky network or a Telegram hiccup never kills the adapter.
                    await asyncio.sleep(backoff)
                    backoff = min(backoff * 2, 30.0)
                    continue
                backoff = self._retry_base
                for update in found:
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

    async def act(self, action: Action) -> Any:
        """Perform one say or react action in a Telegram chat."""
        if action.kind == "say":
            return await send.say(self._call, action.place, str(action.payload["text"]))
        if action.kind == "react":
            return await send.react(self._call, action.place, str(action.payload["emoji"]),
                                    int(action.payload.get("message_id") or 0))
        raise ValueError(f"Unsupported action kind: {action.kind!r}")

    async def show(self, place: str, kind: str, **data: Any) -> Any:
        """Show one non-message thing where the adapter can; unknown kinds are refused."""
        return await send.show(self._call, place, kind, **data)

    async def place_info(self, place: str) -> dict[str, Any]:
        """Return what Telegram reports about one chat."""
        return await send.place_info(self._call, place)

    def _event(self, update_id: int, message: dict[str, Any], text: str) -> Event:
        return event(update_id, message, text, clock=self._clock, name=self.name,
                     bot_id=self.bot_id, bot_username=self.bot_username)

    def _addressed(self, chat: dict[str, Any], message: dict[str, Any], text: str) -> bool:
        return addressed(chat, message, text, bot_id=self.bot_id,
                         bot_username=self.bot_username)
