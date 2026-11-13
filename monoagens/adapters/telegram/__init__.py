"""Telegram adapter: Bot API long polling over httpx; full native coverage."""

from __future__ import annotations

import asyncio
import contextlib
from collections.abc import Awaitable, Callable, Sequence
from typing import Any

import httpx

from monoagens.adapters.telegram import receive, send
from monoagens.adapters.telegram.capabilities import (
    CAPABILITIES,
    GAPS,
    OPTIONAL_CAPABILITIES,
    READS,
    SHOWS,
)
from monoagens.adapters.telegram.format import (
    addressed,
    chat_place,
    event as format_event,
    has_content,
    place_meta,
    poll_answer_event,
    reaction_event,
)
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
    tools = [str(t) for t in (options.get("tools") or ()) if t]
    unknown = [t for t in tools if t not in OPTIONAL_CAPABILITIES]
    if unknown:
        raise ValueError(f"telegram: unknown optional tool {unknown[0]!r}")
    return TelegramBody(
        token,
        api_base=str(options.get("api_base") or "https://api.telegram.org"),
        poll_timeout=float(options.get("poll_timeout") or 30.0),
        enabled_tools=tools,
    )


class TelegramBody:
    """Eyes and hands for Telegram: full Bot API updates in, native `say`/`react`/`edit` out."""

    name = "telegram"
    reads = READS
    shows = SHOWS
    gaps = GAPS

    def __init__(self, token: str, *,
                 get_cursor: GetCursor | None = None, poll_timeout: float = 30.0,
                 transport: httpx.AsyncBaseTransport | None = None,
                 api_base: str = "https://api.telegram.org", retry_base: float = 1.0,
                 clock: Clock | None = None, enabled_tools: Sequence[str] = (),
                 commands: Sequence[dict[str, str]] = ()) -> None:
        """Store token, polling configuration, enabled platform tools, and prompt tracking."""
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
        self.enabled_tools = tuple(t for t in enabled_tools if t in OPTIONAL_CAPABILITIES)
        self._commands = list(commands)
        self._sent: set[str] = set()
        self._choices_prompts: dict[str, list[str]] = {}
        self._poll_prompts: dict[str, tuple[str, str, list[str]]] = {}

    def capabilities(self) -> tuple[dict[str, Any], ...]:
        """Return declared capabilities plus any platform tools enabled in `bodies.telegram.tools`."""
        extras = tuple(OPTIONAL_CAPABILITIES[name] for name in self.enabled_tools)
        return CAPABILITIES + extras

    def _http(self) -> httpx.AsyncClient:
        if self._client is None:
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

    async def sync_commands(self, commands: Sequence[dict[str, str]]) -> Any:
        """Publish slash commands to Telegram's native command menu."""
        self._commands = list(commands)
        if not commands:
            return None
        return await send.sync_commands(self._call, commands)

    async def run(self, put: Put) -> None:
        """Poll Telegram forever, emitting events for messages, edits, reactions, polls & clicks."""
        try:
            me = await self._call("getMe")
            self.bot_id, self.bot_username = me["id"], me.get("username")
            if self._commands:
                with contextlib.suppress(Exception):
                    await send.sync_commands(self._call, self._commands)
            cursor = await self.get_cursor() if self.get_cursor is not None else None
            offset = cursor + 1 if cursor is not None else None
            backoff = self._retry_base
            groups: dict[str, list[dict[str, Any]]] = {}
            opened: dict[str, float] = {}
            while True:
                try:
                    found = await receive.updates(self._call, offset, self._poll_timeout)
                except (httpx.HTTPError, RuntimeError):
                    await asyncio.sleep(backoff)
                    backoff = min(backoff * 2, 30.0)
                    continue
                backoff = self._retry_base
                for update in found:
                    uid = update["update_id"]
                    if update.get("callback_query"):
                        await put(await self._handle_callback(update["callback_query"]))
                        offset = uid + 1
                        continue
                    if update.get("message_reaction"):
                        ev = reaction_event(uid, update["message_reaction"],
                                            clock=self._clock, own_sent=self._sent,
                                            name=self.name)
                        if ev is not None:
                            await put(ev)
                        offset = uid + 1
                        continue
                    if update.get("poll_answer"):
                        ev = poll_answer_event(uid, update["poll_answer"],
                                               clock=self._clock, poll_prompts=self._poll_prompts,
                                               name=self.name)
                        if ev is not None:
                            await put(ev)
                        offset = uid + 1
                        continue
                    edited = update.get("edited_message") or update.get("edited_channel_post")
                    if edited and edited.get("chat") and has_content(edited):
                        await put(await self._message_event(uid, edited, edited=True))
                        offset = uid + 1
                        continue
                    message = update.get("message") or update.get("channel_post")
                    if not message or not has_content(message):
                        offset = uid + 1
                        continue
                    group = receive.group(message)
                    if group:
                        groups.setdefault(group, []).append(update)
                        opened.setdefault(group, asyncio.get_running_loop().time())
                        continue
                    await put(await self._message_event(uid, message))
                    offset = uid + 1
                now = asyncio.get_running_loop().time()
                for group in [name for name, at in opened.items()
                              if now - at >= GROUP_COLLECT_SECONDS]:
                    batch = groups.pop(group, [])
                    opened.pop(group, None)
                    if batch:
                        await put(await self._batch_event(batch))
                        offset = max(offset or 0, max(item["update_id"] for item in batch) + 1)
                await asyncio.sleep(0.05 if groups else 0)
        finally:
            await self.aclose()

    async def _handle_callback(self, query: dict[str, Any]) -> Event:
        qid = query.get("id")
        if qid:
            with contextlib.suppress(Exception):
                await self._call("answerCallbackQuery", callback_query_id=qid)
        return self._callback(query)

    async def _message_event(self, update_id: int, message: dict[str, Any], *,
                             edited: bool = False) -> Event:
        """Turn one message or edit into a percept, storing every file/voice/sticker it carries."""
        text = str(message.get("text") or message.get("caption") or "")
        specs = receive.specs(message)
        attachments = await self._store(specs)
        return format_event(update_id, message, text, clock=self._clock, name=self.name,
                            bot_id=self.bot_id, bot_username=self.bot_username,
                            attachments=attachments, choices_prompts=self._choices_prompts,
                            edited=edited)

    async def _batch_event(self, batch: list[dict[str, Any]]) -> Event:
        """Turn a media group into one percept carrying every file it holds."""
        message = batch[0]["message"]
        text = next((str(item["message"].get("text") or item["message"].get("caption") or "")
                     for item in batch
                     if item["message"].get("text") or item["message"].get("caption")), "")
        specs = [spec for item in batch for spec in receive.specs(item["message"])]
        attachments = await self._store(specs)
        return format_event(batch[-1]["update_id"], message, text, clock=self._clock,
                            name=self.name, bot_id=self.bot_id,
                            bot_username=self.bot_username, attachments=attachments,
                            choices_prompts=self._choices_prompts)

    async def _store(self, specs: list[dict[str, Any]]) -> list[dict[str, Any]]:
        """Store every received file through the door and return its attachments."""
        if not specs or self.files is None:
            return []
        found = []
        for spec in specs[:FILES["per_message"]]:
            data = await receive.download(self._http(), self._api_base, self._token,
                                          str(spec["file_id"]))
            found.append(await self.files.put(data, name=str(spec["name"]),
                                              mime=str(spec["mime"])))
        return found

    async def act(self, action: Action) -> Any:
        """Perform `say`, `react`, `edit`, or enabled optional Telegram actions."""
        p = action.payload or {}
        if action.kind == "say":
            return await self._say(action)
        if action.kind == "react":
            return await send.react(self._call, action.place, str(p["emoji"]),
                                    int(p.get("message_id") or 0))
        if action.kind == "edit":
            return await send.edit(self._call, action.place,
                                   int(p.get("message_id") or 0), str(p.get("text") or ""))
        if action.kind == "pin" and "pin" in self.enabled_tools:
            return await send.pin(self._call, action.place, int(p.get("message_id") or 0),
                                  silent=bool(p.get("silent", False)))
        if action.kind == "unpin" and "unpin" in self.enabled_tools:
            return await send.unpin(self._call, action.place, int(p.get("message_id") or 0))
        raise ValueError(f"Unsupported action kind: {action.kind!r}")

    async def _say(self, action: Action) -> Any:
        """Send text/choices/thread/location and attachments in a Telegram chat."""
        p = action.payload or {}
        text = str(p.get("text") or "")
        opened = []
        for ref in p.get("attachments") or ():
            try:
                data, info = await self.files.open(str(ref), context=action.place)
            except Exception as exc:
                return {"error": f"cannot attach {ref}: {exc}"}
            opened.append((data, info))

        choices = [str(c) for c in (p.get("choices") or ()) if str(c)]
        loc = p.get("location") if isinstance(p.get("location"), dict) else None
        sticker = str(p["sticker"]) if p.get("sticker") else None
        mentions = list(p.get("mentions") or ())
        reply_to = str(p["reply_to"]) if p.get("reply_to") else None
        thread = str(p["thread"]) if p.get("thread") else None

        sent_res = None
        if sticker and sticker.startswith("att_") and self.files is not None:
            try:
                s_data, s_info = await self.files.open(sticker, context=action.place)
            except Exception as exc:
                return {"error": f"cannot send sticker {sticker}: {exc}"}
            sent_res = await send.sticker_file(self._http(), self._api_base, self._token,
                                               action.place, s_data, s_info)
            if isinstance(sent_res, dict) and sent_res.get("message_id"):
                self._sent.add(str(sent_res["message_id"]))
            sticker = None
        if text or choices or loc or sticker or mentions or thread:
            sent_res = await send.say(
                self._call, action.place, text,
                reply_to=reply_to, mentions=mentions, sticker=sticker,
                choices=choices, multiple=bool(p.get("multiple", False)),
                thread=thread, location=loc,
            )
            if isinstance(sent_res, dict):
                mid = str(sent_res.get("message_id") or "")
                if mid:
                    self._sent.add(mid)
                    if choices:
                        self._choices_prompts[mid] = choices
                poll_obj = sent_res.get("poll")
                if isinstance(poll_obj, dict) and poll_obj.get("id") and choices:
                    self._poll_prompts[str(poll_obj["id"])] = (action.place, mid, choices)

        for data, info in opened:
            doc_res = await send.document(self._http(), self._api_base, self._token,
                                          action.place, data, info)
            if isinstance(doc_res, dict) and doc_res.get("message_id"):
                self._sent.add(str(doc_res["message_id"]))
        result: dict[str, Any] = {}
        if opened:
            result["sent"] = len(opened)
        if isinstance(sent_res, dict) and sent_res.get("message_id"):
            result["message_id"] = str(sent_res["message_id"])
        if choices:
            if isinstance(sent_res, dict) and sent_res.get("poll"):
                result["choices"] = [{"label": opt, "answer": opt[:100]} for opt in choices]
            elif len(choices) <= send.MAX_INLINE_CHOICES and not bool(p.get("multiple", False)):
                result["choices"] = [{"label": opt, "answer": f"choice:{i}:{opt[:32]}"}
                                     for i, opt in enumerate(choices, 1)]
            else:
                result["choices"] = [{"label": opt, "answer": str(i)}
                                     for i, opt in enumerate(choices, 1)]
        return result or None

    async def show(self, place: str, kind: str, **data: Any) -> Any:
        """Show one non-message thing natively; unknown kinds are refused."""
        return await send.show(self._call, place, kind, **data)

    async def place_info(self, place: str) -> dict[str, Any]:
        """Return what Telegram reports about one chat or forum topic."""
        return await send.place_info(self._call, place, bot_id=self.bot_id)

    def _callback(self, query: dict[str, Any]) -> Event:
        """Turn one button callback into a `data.choice` + `data.answer` percept."""
        raw = str(query.get("data") or "")
        parts = raw.split(":")
        msg = query.get("message") or {}
        chat = msg.get("chat") or {}
        msg_id = str(msg.get("message_id") or "")
        place, form, parent, place_kind = chat_place(chat, msg)
        sender_id = (query.get("from") or {}).get("id")
        selected = raw
        if len(parts) >= 3 and parts[0] == "choice":
            opts = self._choices_prompts.get(msg_id)
            try:
                idx = int(parts[1]) - 1
                selected = opts[idx] if opts and 0 <= idx < len(opts) else ":".join(parts[2:])
            except ValueError:
                selected = ":".join(parts[2:])
        label = str((query.get("from") or {}).get("first_name") or "someone")
        data: dict[str, Any] = {"choice": selected, "answer": raw, "custom_id": raw}
        if msg_id:
            data["reply_to"] = {"message_id": msg_id}
        return Event(
            id=None, ts=self._clock.now(), kind="percept",
            who=f"telegram:{sender_id}", place=place, body=self.name,
            payload={"description": f"{label} chose {selected}",
                     "place_kind": place_kind,
                     "place_info": place_meta(chat, form, parent),
                     "addressed": True, "data": data},
        )

    def _event(self, update_id: int, message: dict[str, Any], text: str) -> Event:
        return format_event(update_id, message, text, clock=self._clock, name=self.name,
                            bot_id=self.bot_id, bot_username=self.bot_username,
                            choices_prompts=self._choices_prompts)

    def _addressed(self, chat: dict[str, Any], message: dict[str, Any], text: str) -> bool:
        return addressed(chat, message, text, bot_id=self.bot_id,
                         bot_username=self.bot_username)
