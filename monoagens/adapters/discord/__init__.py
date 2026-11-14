"""Discord adapter (discord.py): servers, channels, threads, DMs, buttons, menus, roles."""

from __future__ import annotations

import asyncio
import contextlib
from collections.abc import Awaitable, Callable
from typing import Any

import discord

from monoagens.adapters.discord import send
from monoagens.adapters.discord.capabilities import (
    CAPABILITIES,
    GAPS,
    OPTIONAL_CAPABILITIES,
    READS,
    SHOWS,
)
from monoagens.adapters.discord.format import (
    format_delete,
    format_edit,
    format_interaction,
    format_message,
    format_reaction,
)
from monoagens.adapters.discord.identity import (
    DiscordDirectory,
    channel_form,
    is_dm_channel,
    member_roles,
    place_id,
    raw_channel_id,
)
from monoagens.adapters.discord.media import (
    collect_attachments,
    collect_sticker,
    fetch_deferred,
)
from monoagens.infra.clock import Clock
from monoagens.contract import Action
from monoagens.types import Event

Put = Callable[[Event], Awaitable[None]]
REQUIRES: tuple[str, ...] = ("DISCORD_BOT_TOKEN",)


def create(options: dict[str, Any], env: dict[str, str]) -> "DiscordBody":
    """Build the Discord adapter from `bodies.discord` in config.yaml and `.env`."""
    token_env = str(options.get("bot_token_env") or "DISCORD_BOT_TOKEN")
    token = str(env.get(token_env) or "")
    defer_media = bool(options.get("defer_media", False))
    tools = [str(t) for t in (options.get("tools") or ()) if t]
    unknown = [t for t in tools if t not in OPTIONAL_CAPABILITIES]
    if unknown:
        raise ValueError(f"discord: unknown optional tool {unknown[0]!r}")
    return DiscordBody(token=token, defer_media=defer_media, enabled_tools=tools)


class DiscordBody:
    """Discord gateway adapter on discord.py: DMs, guild channels, threads, components & roles."""

    name = "discord"
    FILES = {"per_message": 10, "max_bytes": 25_000_000, "mimes": ()}
    PERMISSIONS = {"say": "guest", "react": "guest", "edit": "guest", "pin": "guest",
                   "manage_role": "admin"}
    reads = READS
    shows = SHOWS
    gaps = GAPS

    def __init__(self, token: str = "", *, clock: Clock | None = None,
                 client: Any = None, client_factory: Callable[..., Any] | None = None,
                 defer_media: bool = False,
                 http_client: Any = None,
                 enabled_tools: list[str] | tuple[str, ...] = ()) -> None:
        self._token = token
        self._clock = clock or Clock()
        self._client = client
        self._client_factory = client_factory
        self._defer_media = defer_media
        self._http_client = http_client
        self._files: Any = None
        self._put: Put | None = None
        self.directory = DiscordDirectory()
        self.bot_id: str = ""
        self.enabled_tools = tuple(t for t in enabled_tools if t in OPTIONAL_CAPABILITIES)
        self._sent: set[str] = set()
        self._choices_prompts: dict[str, list[str]] = {}
        self._stop_event = asyncio.Event()

    @property
    def files(self) -> Any:
        """Return the bound AttachmentDoor."""
        return self._files

    @files.setter
    def files(self, door: Any) -> None:
        """Bind `AttachmentDoor` and register our deferred-media fetcher."""
        self._files = door
        if door is not None and hasattr(door, "register_fetcher"):
            door.register_fetcher(self.name, self.fetch_attachment)

    async def fetch_attachment(self, meta: dict[str, Any]) -> tuple[bytes, str]:
        """Download a deferred Discord CDN attachment when first opened."""
        return await fetch_deferred(meta, http_client=self._http_client)

    def capabilities(self) -> tuple[dict[str, Any], ...]:
        """Return declared capabilities plus any platform tools enabled in `bodies.discord.tools`."""
        extras = tuple(OPTIONAL_CAPABILITIES[name] for name in self.enabled_tools)
        return CAPABILITIES + extras

    def remember_sent(self, message_id: str) -> None:
        """Track outgoing Discord message ids so replies and reactions know what is ours."""
        if message_id:
            self._sent.add(str(message_id))

    def remember_choices(self, message_id: str, options: list[str]) -> None:
        """Track choices offered on a sent prompt so interaction clicks or numbers map back."""
        if message_id and options:
            self._choices_prompts[str(message_id)] = list(options)

    def _build_client(self) -> Any:
        if self._client is not None:
            return self._client
        if self._client_factory is not None:
            self._client = self._client_factory()
            return self._client
        intents = discord.Intents.default()
        intents.message_content = True
        intents.members = True
        intents.reactions = True
        intents.guilds = True
        intents.messages = True
        intents.dm_messages = True
        self._client = _GatewayClient(self, intents=intents)
        return self._client

    async def _resolve_channel(self, place: str) -> Any:
        client = self._client
        if client is None:
            return None
        cid_text = raw_channel_id(place)
        cid: Any = int(cid_text) if cid_text.isdigit() else cid_text
        ch = client.get_channel(cid)
        if ch is not None:
            return ch
        if hasattr(client, "fetch_channel"):
            with contextlib.suppress(Exception):
                return await client.fetch_channel(cid)
        return None

    async def run(self, put: Put) -> None:
        """Connect to Discord and forward incoming percepts until cancelled or closed."""
        self._stop_event.clear()
        self._put = put
        client = self._build_client()
        if getattr(client, "user", None) is not None:
            self.bot_id = str(client.user.id)
        if hasattr(client, "bind_adapter"):
            client.bind_adapter(self)
        start_fn = getattr(client, "start", None)
        if callable(start_fn) and self._token:
            try:
                await start_fn(self._token)
            finally:
                await self.aclose()
            return
        await self._stop_event.wait()

    async def stop(self) -> None:
        """Stop the gateway connection and close the client."""
        await self.aclose()

    async def render(self, event: Any) -> None:
        """Render one lifecycle event natively (§2.4.5): typing, an edit or a read mark."""
        kind = "typing" if event.name == "composing" else event.name
        await self.show(str(event.place), kind, text=str(event.data.get("text") or ""))

    def prompt_section(self) -> str:
        """Return what reasoning should know about Discord (≤1200 characters)."""
        return ("Discord: one message per say, with optional files, reply, mentions, choices, "
                "thread or location; react adds one emoji; edit rewrites a sent message. A DM "
                "is a direct place; a server channel is shared and its members are the role's.")

    def self_actor(self) -> str:
        """Return the actor id this body acts as: its own bot account."""
        return f"discord:{self.bot_id}" if self.bot_id else "discord:unknown"

    async def aclose(self) -> None:
        """Close the Discord client cleanly."""
        self._stop_event.set()
        if self._client is not None and hasattr(self._client, "close"):
            with contextlib.suppress(Exception):
                await self._client.close()

    async def on_ready(self) -> None:
        """Record bot user id when the gateway connects."""
        user = getattr(self._client, "user", None)
        if user is not None:
            self.bot_id = str(user.id)

    async def handle_message(self, message: Any, *, backfill: bool = False) -> Event | None:
        """Convert one `discord.Message` into a percept and enqueue it if configured."""
        channel = getattr(message, "channel", None)
        place = place_id(channel) if channel is not None else "space:dc:unknown"
        atts = await collect_attachments(
            message, self._files, place=place,
            per_message=self.FILES["per_message"], deferred=self._defer_media,
        )
        sticker = await collect_sticker(message, self._files, place=place)
        ev = format_message(
            message, now=self._clock.now(), bot_id=self.bot_id,
            bot_user=getattr(self._client, "user", None),
            directory=self.directory, attachments=atts, sticker_info=sticker,
            choices_prompts=self._choices_prompts, own_sent=self._sent,
            backfill=backfill, name=self.name,
        )
        if ev is not None and self._put is not None:
            await self._put(ev)
        return ev

    async def handle_interaction(self, interaction: Any) -> Event | None:
        """Defer an interaction immediately (<3 s) inside the adapter, then emit its percept."""
        response = getattr(interaction, "response", None)
        if response is not None and not bool(getattr(response, "is_done", lambda: False)()):
            itype = getattr(interaction, "type", None)
            thinking = itype == discord.InteractionType.application_command
            with contextlib.suppress(Exception):
                await response.defer(thinking=thinking)
        msg = getattr(interaction, "message", None)
        data = getattr(interaction, "data", None) or {}
        if msg is not None and str(data.get("custom_id") or "").startswith("approval:"):
            with contextlib.suppress(Exception):
                await msg.edit(view=None)
        ev = format_interaction(
            interaction, now=self._clock.now(), bot_id=self.bot_id,
            directory=self.directory, choices_prompts=self._choices_prompts, name=self.name,
        )
        if ev is not None and self._put is not None:
            await self._put(ev)
        return ev

    async def handle_message_edit(self, before: Any, after: Any) -> Event | None:
        """Emit an edit percept unless the content did not change."""
        if getattr(before, "content", None) == getattr(after, "content", None):
            return None
        ev = format_edit(
            getattr(after, "channel", None), getattr(after, "author", None),
            str(getattr(after, "id", "")), str(getattr(after, "content", "") or ""),
            now=self._clock.now(), bot_id=self.bot_id, directory=self.directory,
            name=self.name,
        )
        if ev is not None and self._put is not None:
            await self._put(ev)
        return ev

    async def handle_message_delete(self, message: Any) -> Event | None:
        """Emit a delete percept for a removed message."""
        ev = format_delete(
            getattr(message, "channel", None), str(getattr(message, "id", "")),
            author=getattr(message, "author", None),
            now=self._clock.now(), bot_id=self.bot_id, directory=self.directory,
            name=self.name,
        )
        if ev is not None and self._put is not None:
            await self._put(ev)
        return ev

    async def handle_reaction_add(self, reaction: Any, user: Any) -> Event | None:
        """Emit a reaction percept, addressed if the reaction lands on the soul's own message."""
        msg = getattr(reaction, "message", None)
        target_author = str(getattr(getattr(msg, "author", None), "id", "") or "")
        ev = format_reaction(
            getattr(msg, "channel", None), user,
            str(getattr(msg, "id", "")), getattr(reaction, "emoji", ""),
            now=self._clock.now(), bot_id=self.bot_id, directory=self.directory,
            own_sent=self._sent, target_author_id=target_author, name=self.name,
        )
        if ev is not None and self._put is not None:
            await self._put(ev)
        return ev

    async def handle_raw_reaction_add(self, payload: Any) -> Event | None:
        """Emit a reaction percept from a raw gateway reaction payload when uncached."""
        ch = await self._resolve_channel(f"space:dc:{getattr(payload, 'channel_id', '')}")
        user = getattr(payload, "member", None)
        if user is None and self._client is not None:
            user = self._client.get_user(getattr(payload, "user_id", None))
        if user is None:
            user = type("_User", (), {"id": getattr(payload, "user_id", ""),
                                      "name": "someone", "roles": ()})()
        ev = format_reaction(
            ch, user, str(getattr(payload, "message_id", "")),
            getattr(payload, "emoji", ""),
            now=self._clock.now(), bot_id=self.bot_id, directory=self.directory,
            own_sent=self._sent, name=self.name,
        )
        if ev is not None and self._put is not None:
            await self._put(ev)
        return ev

    async def act(self, action: Action) -> Any:
        """Perform `say`, `react`, or `edit` on Discord."""

        return await support.answer(
            self, action, capability=f"{self.name}.{action.capability}",
            function=lambda: self._carry(action))

    async def _carry(self, action: Action) -> Any:
        """Carry one action out; `support.answer` turns what comes back into the receipt."""
        p = action.arguments or {}
        if action.capability == "say":
            return await send.say(
                self._resolve_channel, self._files, action.place,
                text=str(p.get("text") or ""),
                attachments=list(p.get("attachments") or ()),
                reply_to=str(p["reply_to"]) if p.get("reply_to") else None,
                mentions=list(p.get("mentions") or ()),
                sticker=str(p["sticker"]) if p.get("sticker") else None,
                choices=list(p.get("choices") or ()),
                multiple=bool(p.get("multiple", False)),
                thread=str(p["thread"]) if p.get("thread") else None,
                location=p.get("location") if isinstance(p.get("location"), dict) else None,
                remember_sent=self.remember_sent,
                remember_choices=self.remember_choices,
            )
        if action.capability == "react":
            return await send.react(
                self._resolve_channel, action.place,
                str(p.get("emoji") or ""), str(p.get("message_id") or ""),
            )
        if action.capability == "edit":
            return await send.edit(
                self._resolve_channel, action.place,
                str(p.get("message_id") or ""), str(p.get("text") or ""),
            )
        if action.capability == "pin" and "pin" in self.enabled_tools:
            ch = await self._resolve_channel(action.place)
            if ch is None:
                return {"error": f"Discord place not found: {action.place}"}
            msg = await ch.fetch_message(int(str(p.get("message_id") or "")))
            if hasattr(msg, "pin"):
                await msg.pin()
            return {"pinned": True, "message_id": str(p.get("message_id") or "")}
        if action.capability == "manage_role" and "manage_role" in self.enabled_tools:
            ch = await self._resolve_channel(action.place)
            guild = getattr(ch, "guild", None)
            if guild is None:
                return {"error": "manage_role requires a Discord server channel"}
            member = guild.get_member(int(str(p.get("user_id"))))
            role = guild.get_role(int(str(p.get("role_id"))))
            if member is None or role is None:
                return {"error": "member or role not found"}
            if p.get("action") == "remove":
                await member.remove_roles(role)
            else:
                await member.add_roles(role)
            return {"ok": True, "action": p.get("action"), "role_id": str(role.id)}
        raise ValueError(f"Unsupported action kind: {action.capability!r}")

    async def show(self, place: str, kind: str, **data: Any) -> Any:
        """Show typing, edit, buttons/menus or reactions via the adapter."""
        return await send.show(self._resolve_channel, place, kind, **data)

    async def place_info(self, place: str) -> dict[str, Any]:
        """Return D68 structured metadata `{form, parent, can_post, name, members, roles}`."""
        cached = self.directory.place_info(place)
        if cached and ("name" in cached or cached.get("parent") is not None):
            return cached
        ch = await self._resolve_channel(place)
        if ch is not None:
            return self.directory.remember_channel(ch, bot_user=getattr(self._client, "user", None))
        return cached

    async def history(self, place: str, since: Any = None, *, limit: int = 50) -> list[Event]:
        """Back-fill recent messages in `place` missed while offline (`backfill: true`)."""
        ch = await self._resolve_channel(place)
        if ch is None or not hasattr(ch, "history"):
            return []
        events: list[Event] = []
        kwargs: dict[str, Any] = {"limit": limit}
        if since is not None:
            kwargs["after"] = since
        async for msg in ch.history(**kwargs):
            ev = format_message(
                msg, now=self._clock.now(), bot_id=self.bot_id,
                bot_user=getattr(self._client, "user", None),
                directory=self.directory, backfill=True, own_sent=self._sent, name=self.name,
            )
            if ev is not None:
                events.append(ev)
        return events

    def get_cursor(self) -> Any:
        """Return current timestamp cursor for back-fill."""
        return self._clock.now()


class _GatewayClient(discord.Client):
    """Thin `discord.Client` bridge delegating events to `DiscordBody`."""

    def __init__(self, adapter: DiscordBody, **kwargs: Any) -> None:
        super().__init__(**kwargs)
        self._adapter = adapter

    async def on_ready(self) -> None:
        await self._adapter.on_ready()

    async def on_message(self, message: discord.Message) -> None:
        await self._adapter.handle_message(message)

    async def on_interaction(self, interaction: discord.Interaction) -> None:
        await self._adapter.handle_interaction(interaction)

    async def on_message_edit(self, before: discord.Message, after: discord.Message) -> None:
        await self._adapter.handle_message_edit(before, after)

    async def on_message_delete(self, message: discord.Message) -> None:
        await self._adapter.handle_message_delete(message)

    async def on_reaction_add(self, reaction: discord.Reaction, user: Any) -> None:
        await self._adapter.handle_reaction_add(reaction, user)
