"""Discord outbound rendering: messages, attachments, buttons, select menus, threads, edits."""

from __future__ import annotations

import contextlib
import io
from collections.abc import Awaitable, Callable, Sequence
from typing import Any

import discord

ResolveChannel = Callable[[str], Awaitable[Any]]

MAX_BUTTONS = 5
MAX_SELECT_OPTIONS = 25


def numbered_choices(choices: Sequence[str]) -> str:
    """Format choices as a numbered list fallback when > 25 options exist."""
    return "\n".join(f"{i}. {item}" for i, item in enumerate(choices, start=1))


def format_location_text(location: dict[str, Any]) -> str:
    """Render a location object cleanly as text with a map link."""
    lat = location.get("lat")
    lon = location.get("lon")
    title = str(location.get("name") or "Location")
    return f"{title} ({lat}, {lon})\nhttps://maps.google.com/?q={lat},{lon}"


def _build_component_view(items: Sequence[tuple[str, str]], *,
                          multiple: bool = False) -> discord.ui.View:
    """Render `(label, custom_id)` entries as Buttons (<=5, single) or a Select menu."""
    view = discord.ui.View(timeout=None)
    if len(items) <= MAX_BUTTONS and not multiple:
        for label, cid in items:
            view.add_item(discord.ui.Button(
                label=str(label)[:80],
                custom_id=str(cid)[:100],
                style=discord.ButtonStyle.secondary,
            ))
        return view
    opts = [
        discord.SelectOption(label=str(label)[:100], value=str(label)[:100])
        for label, _ in list(items)[:MAX_SELECT_OPTIONS]
    ]
    max_v = len(opts) if multiple else 1
    view.add_item(discord.ui.Select(
        custom_id="choice:menu",
        placeholder="Pick an option…",
        min_values=1,
        max_values=max_v,
        options=opts,
    ))
    return view


def _mention_prefix(text: str, mentions: Sequence[str]) -> str:
    tokens = []
    for raw in mentions:
        uid = str(raw).removeprefix("discord:").strip()
        if not uid:
            continue
        tag = f"<@{uid}>"
        if tag not in text and f"<@!{uid}>" not in text:
            tokens.append(tag)
    if not tokens:
        return text
    prefix = " ".join(tokens)
    return f"{prefix} {text}".strip() if text else prefix


async def say(resolve_channel: ResolveChannel, files_door: Any, place: str, *,
              text: str = "", attachments: Sequence[str] = (),
              reply_to: str | None = None, mentions: Sequence[str] = (),
              sticker: str | None = None, choices: Sequence[str] = (),
              multiple: bool = False, thread: str | None = None,
              location: dict[str, Any] | None = None,
              remember_sent: Callable[[str], None] | None = None,
              remember_choices: Callable[[str, list[str]], None] | None = None,
              ) -> dict[str, Any]:
    """Send a Discord message supporting files, reply, mentions, sticker, choices, thread & geo."""
    channel = await resolve_channel(place)
    if channel is None:
        return {"error": f"Discord place not found: {place}"}

    body = _mention_prefix(text, mentions)
    if location:
        loc_line = format_location_text(location)
        body = f"{body}\n{loc_line}".strip() if body else loc_line

    opened_files: list[discord.File] = []
    for ref in attachments:
        try:
            raw, info = await files_door.open(str(ref), context=place)
        except Exception as exc:
            return {"error": f"cannot attach {ref}: {exc}"}
        opened_files.append(discord.File(io.BytesIO(raw), filename=str(info.get("name") or "file")))

    if sticker and files_door is not None:
        try:
            raw, info = await files_door.open(str(sticker), context=place)
            opened_files.append(discord.File(io.BytesIO(raw),
                                             filename=str(info.get("name") or "sticker.png")))
        except Exception as exc:
            if not body:
                body = f"[sticker: {sticker}]"
            else:
                return {"error": f"cannot attach sticker {sticker}: {exc}"}

    view: discord.ui.View | None = None
    choice_list = [str(c) for c in choices if str(c)]
    if choice_list:
        if len(choice_list) > MAX_SELECT_OPTIONS:
            body = f"{body}\n{numbered_choices(choice_list)}".strip()
        else:
            items = [(opt, f"choice:{i}:{opt[:40]}") for i, opt in enumerate(choice_list, 1)]
            view = _build_component_view(items, multiple=multiple)

    ref_obj: discord.MessageReference | None = None
    if reply_to and str(reply_to).isdigit() and hasattr(channel, "id"):
        with contextlib.suppress(ValueError, TypeError):
            ref_obj = discord.MessageReference(message_id=int(str(reply_to)),
                                               channel_id=int(channel.id))

    kwargs: dict[str, Any] = {}
    if opened_files:
        kwargs["files"] = opened_files
    if ref_obj is not None:
        kwargs["reference"] = ref_obj
    if view is not None:
        kwargs["view"] = view

    sent_msg: Any = None
    created_thread_id: str | None = None
    if thread:
        ctype = getattr(channel, "type", None)
        if ctype in (discord.ChannelType.forum, discord.ChannelType.media):
            created = await channel.create_thread(name=thread[:100], content=body or thread,
                                                  **kwargs)
            sent_msg = getattr(created, "message", None) or created
            created_thread_id = str(getattr(getattr(created, "thread", created), "id", ""))
        else:
            sent_msg = await channel.send(body or thread, **kwargs)
            if hasattr(sent_msg, "create_thread"):
                with contextlib.suppress(Exception):
                    thr = await sent_msg.create_thread(name=thread[:100])
                    created_thread_id = str(getattr(thr, "id", ""))
    else:
        sent_msg = await channel.send(body or "\u200b", **kwargs)

    msg_id = str(getattr(sent_msg, "id", "") or "")
    if msg_id and remember_sent is not None:
        remember_sent(msg_id)
    if msg_id and choice_list and remember_choices is not None:
        remember_choices(msg_id, choice_list)
    result: dict[str, Any] = {"message_id": msg_id, "place": place}
    if created_thread_id:
        result["thread_place"] = f"space:dc:{created_thread_id}"
    if opened_files:
        result["sent"] = len(opened_files)
    return result


async def react(resolve_channel: ResolveChannel, place: str, emoji: str,
                message_id: str) -> dict[str, Any]:
    """Add an emoji reaction to one Discord message."""
    channel = await resolve_channel(place)
    if channel is None:
        return {"error": f"Discord place not found: {place}"}
    msg = await channel.fetch_message(int(str(message_id)))
    await msg.add_reaction(emoji)
    return {"message_id": str(message_id), "emoji": emoji}


async def edit(resolve_channel: ResolveChannel, place: str, message_id: str,
               text: str) -> dict[str, Any]:
    """Edit one of the soul's own Discord messages."""
    channel = await resolve_channel(place)
    if channel is None:
        return {"error": f"Discord place not found: {place}"}
    msg = await channel.fetch_message(int(str(message_id)))
    await msg.edit(content=text)
    return {"message_id": str(message_id), "edited": True}


async def show(resolve_channel: ResolveChannel, place: str, kind: str,
               **data: Any) -> Any:
    """Render non-message interaction hints: typing, edit, buttons/menus, or reactions."""
    channel = await resolve_channel(place)
    if channel is None:
        raise ValueError(f"Discord place not found: {place}")
    if kind == "typing":
        if hasattr(channel, "trigger_typing"):
            return await channel.trigger_typing()
        async with channel.typing():
            return None
    if kind == "edit":
        msg = await channel.fetch_message(int(str(data.get("message_id"))))
        return await msg.edit(content=str(data.get("text") or ""))
    if kind == "buttons":
        buttons: Sequence[tuple[str, str]] = data.get("buttons") or ()
        items = [(str(label), str(val)) for label, val in buttons]
        view = _build_component_view(items, multiple=bool(data.get("multiple")))
        return await channel.send(str(data.get("text") or "\u200b"), view=view)
    if kind == "reactions":
        msg = await channel.fetch_message(int(str(data.get("message_id"))))
        return await msg.add_reaction(str(data.get("emoji") or ""))
    raise ValueError(f"cannot show {kind!r}")
