"""Telegram adapter: long polling over httpx; neutral ids only (who/place)."""

from __future__ import annotations

import asyncio
from collections.abc import Awaitable, Callable
from typing import Any

import httpx

from monoagens.adapters.telegram import receive, send
from monoagens.adapters.telegram.capabilities import CAPABILITIES, SHOWS
from monoagens.adapters.telegram.format import addressed, event as format_event
from monoagens.infra.clock import Clock
from monoagens.types import Action, Event

Put = Callable[[Event], Awaitable[None]]
GetCursor = Callable[[], Awaitable[Any | None]]
REQUIRES = ("TELEGRAM_BOT_TOKEN",)
FILES = {"per_message": 10, "max_bytes": 50_000_000, "mimes": ()}
GROUP_COLLECT_SECONDS = 1.0


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
        self.files: Any = None
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
        """Poll Telegram forever, putting one event per message, files included."""
        try:
            me = await self._call("getMe")
            self.bot_id, self.bot_username = me["id"], me.get("username")
            cursor = await self.get_cursor() if self.get_cursor is not None else None
            offset = cursor + 1 if cursor is not None else None
            backoff = self._retry_base
            groups: dict[str, list[dict[str, Any]]] = {}
            opened: dict[str, float] = {}
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
                    if update.get("callback_query"):
                        await put(self._callback(update["callback_query"]))
                        offset = update["update_id"] + 1
                        continue
                    message = update.get("message") or update.get("channel_post")
                    if not message:
                        offset = update["update_id"] + 1
                        continue
                    if not (message.get("text") or receive.specs(message)):
                        offset = update["update_id"] + 1  # nothing to make a percept of
                        continue
                    group = receive.group(message)
                    if group:
                        groups.setdefault(group, []).append(update)
                        opened.setdefault(group, asyncio.get_running_loop().time())
                        continue
                    await put(await self._message_event(update["update_id"], message))
                    # Advance only after the event is safely recorded.
                    offset = update["update_id"] + 1
                now = asyncio.get_running_loop().time()
                for group in [name for name, at in opened.items()
                              if now - at >= GROUP_COLLECT_SECONDS]:
                    batch = groups.pop(group, [])
                    opened.pop(group, None)
                    if batch:
                        await put(await self._batch_event(batch))
                        offset = max(offset or 0, max(item["update_id"] for item in batch) + 1)
                # never spin hot, and give a media group its second
                await asyncio.sleep(0.05 if groups else 0)
        finally:
            await self.aclose()

    async def _message_event(self, update_id: int, message: dict[str, Any]) -> Event:
        """Turn one message into one percept, storing every file it carries."""
        text = str(message.get("text") or "")
        specs = receive.specs(message)
        attachments = await self._store(specs)
        return format_event(update_id, message, text, clock=self._clock, name=self.name,
                            bot_id=self.bot_id, bot_username=self.bot_username,
                            attachments=attachments)

    async def _batch_event(self, batch: list[dict[str, Any]]) -> Event:
        """Turn a media group into one percept carrying every file it holds."""
        message = batch[0]["message"]
        text = next((str(item["message"].get("text")) for item in batch
                     if item["message"].get("text")), "")
        specs = [spec for item in batch for spec in receive.specs(item["message"])]
        attachments = await self._store(specs)
        return format_event(batch[-1]["update_id"], message, text, clock=self._clock,
                            name=self.name, bot_id=self.bot_id,
                            bot_username=self.bot_username, attachments=attachments)

    async def _store(self, specs: list[dict[str, Any]]) -> list[dict[str, Any]]:
        """Store every received file through the door and return its attachments."""
        found = []
        for spec in specs[:FILES["per_message"]]:
            data = await receive.download(self._http(), self._api_base, self._token,
                                          str(spec["file_id"]))
            found.append(await self.files.put(data, name=str(spec["name"]),
                                              mime=str(spec["mime"])))
        return found

    async def act(self, action: Action) -> Any:
        """Perform one say or react action in a Telegram chat, files included."""
        if action.kind == "say":
            return await self._say(action)
        if action.kind == "react":
            return await send.react(self._call, action.place, str(action.payload["emoji"]),
                                    int(action.payload.get("message_id") or 0))
        raise ValueError(f"Unsupported action kind: {action.kind!r}")

    async def _say(self, action: Action) -> Any:
        """Send the text and every attachment this act carries (middleware.md §10)."""
        text = str(action.payload.get("text") or "")
        opened = []
        for ref in action.payload.get("attachments") or ():
            try:
                data, info = await self.files.open(str(ref), context=action.place)
            except Exception as exc:
                return {"error": f"cannot attach {ref}: {exc}"}
            opened.append((data, info))
        if text:
            await send.say(self._call, action.place, text)
        for data, info in opened:
            await send.document(self._http(), self._api_base, self._token, action.place,
                                data, info)
        return {"sent": len(opened)} if opened else None

    async def show(self, place: str, kind: str, **data: Any) -> Any:
        """Show one non-message thing where the adapter can; unknown kinds are refused."""
        return await send.show(self._call, place, kind, **data)

    async def place_info(self, place: str) -> dict[str, Any]:
        """Return what Telegram reports about one chat."""
        return await send.place_info(self._call, place, bot_id=self.bot_id)

    def _callback(self, query: dict[str, Any]) -> Event:
        """Turn one button press into an ordinary percept that answers an approval."""
        data = str(query.get("data") or "")
        parts = data.split(":")
        answer = len(parts) == 3 and parts[0] == "approval"
        return Event(id=None, ts=self._clock.now(), kind="percept",
                     who=f"telegram:{(query.get('from') or {}).get('id')}",
                     place=f"space:tg:{(query.get('message') or {}).get('chat', {}).get('id')}",
                     body=self.name,
                     payload={"description": parts[1] if answer else data,
                              "place_kind": "shared", "addressed": True,
                              **({"approval": int(parts[2]), "answer": parts[1] == "yes"}
                                 if answer else {})})

    def _event(self, update_id: int, message: dict[str, Any], text: str) -> Event:
        return format_event(update_id, message, text, clock=self._clock, name=self.name,
                            bot_id=self.bot_id, bot_username=self.bot_username)

    def _addressed(self, chat: dict[str, Any], message: dict[str, Any], text: str) -> bool:
        return addressed(chat, message, text, bot_id=self.bot_id,
                         bot_username=self.bot_username)
