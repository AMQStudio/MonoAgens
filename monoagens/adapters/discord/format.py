"""Discord inbound event formatter: messages, edits, deletions, reactions, interactions, places."""

from __future__ import annotations

from collections.abc import Mapping, Sequence
from datetime import datetime, timezone
from typing import Any

import discord

from monoagens.adapters.discord.identity import (
    DiscordDirectory,
    is_dm_channel,
    member_roles,
    place_id,
)
from monoagens.middleware.contract import describe_attachments
from monoagens.types import Event


def _event_ts(raw_dt: datetime | None, fallback: datetime) -> datetime:
    if raw_dt is None:
        return fallback
    if raw_dt.tzinfo is None:
        return raw_dt.replace(tzinfo=timezone.utc)
    return raw_dt.astimezone(timezone.utc)


def _match_choice(text: str, reply_id: str,
                  choices_prompts: Mapping[str, list[str]]) -> tuple[str, str] | None:
    stripped = text.strip()
    if not stripped or not stripped.isdigit():
        return None
    idx = int(stripped)
    if idx < 1:
        return None
    if reply_id and reply_id in choices_prompts:
        opts = choices_prompts[reply_id]
        if idx <= len(opts):
            return opts[idx - 1], reply_id
    return None


def _extract_poll(poll: Any) -> dict[str, Any] | None:
    if poll is None:
        return None
    question = getattr(poll, "question", None)
    q_text = getattr(question, "text", None) or str(question or "")
    answers = []
    for ans in getattr(poll, "answers", None) or ():
        a_text = getattr(ans, "text", None) or str(ans)
        if a_text:
            answers.append(str(a_text))
    return {
        "question": str(q_text),
        "options": answers,
        "allow_multiselect": bool(getattr(poll, "allow_multiselect", False)),
    }


def _reply_info(message: Any) -> tuple[dict[str, Any] | None, str | None]:
    ref = getattr(message, "reference", None)
    if ref is None or getattr(ref, "message_id", None) is None:
        return None, None
    ref_id = str(ref.message_id)
    info: dict[str, Any] = {"message_id": ref_id}
    resolved = getattr(ref, "resolved", None)
    ref_author = getattr(resolved, "author", None)
    ref_author_id = getattr(ref_author, "id", None)
    if ref_author_id is not None:
        info["who"] = f"discord:{ref_author_id}"
        return info, str(ref_author_id)
    return info, None


def _base_guild_data(channel: Any, author: Any) -> dict[str, Any]:
    guild = getattr(channel, "guild", None) or getattr(author, "guild", None)
    out: dict[str, Any] = {"discord_roles": member_roles(author) if author is not None else []}
    if guild is not None and getattr(guild, "id", None) is not None:
        out["guild_id"] = str(guild.id)
    return out


def format_message(message: Any, *, now: datetime, bot_id: str, bot_user: Any = None,
                   directory: DiscordDirectory,
                   attachments: Sequence[Mapping[str, Any]] = (),
                   sticker_info: Mapping[str, Any] | None = None,
                   choices_prompts: Mapping[str, list[str]] | None = None,
                   own_sent: set[str] | None = None,
                   backfill: bool = False, imported: bool = False,
                   name: str = "discord") -> Event | None:
    """Turn one Discord message into a neutral `Event(kind='percept')`, skipping own echoes."""
    author = getattr(message, "author", None)
    author_id = str(getattr(author, "id", "") or "")
    if not author_id or (bot_id and author_id == str(bot_id)):
        return None
    channel = getattr(message, "channel", None)
    if channel is None:
        return None

    sender_label = directory.remember_user(author)
    p_info = directory.remember_channel(channel, bot_user=bot_user, author=author)
    place = place_id(channel)
    dm = is_dm_channel(channel)
    place_kind = "direct" if dm else "shared"
    msg_id = str(getattr(message, "id", "") or "")
    text = str(getattr(message, "content", "") or "")
    embeds = list(getattr(message, "embeds", None) or ())
    if not text and embeds:
        parts = [str(getattr(e, "title", "") or getattr(e, "description", "") or "")
                 for e in embeds]
        text = " | ".join(p for p in parts if p)

    msg_type = getattr(message, "type", None)
    is_call = msg_type == discord.MessageType.call

    data: dict[str, Any] = {
        "message_id": msg_id,
        **_base_guild_data(channel, author),
    }
    reply_info, reply_author_id = _reply_info(message)
    reply_id = str((reply_info or {}).get("message_id") or "")
    if reply_info:
        data["reply_to"] = reply_info

    raw_mentions = list(getattr(message, "mentions", None) or ())
    mention_ids = [f"discord:{getattr(u, 'id', '')}" for u in raw_mentions
                   if getattr(u, "id", None) is not None and str(u.id) != str(bot_id)]
    if mention_ids:
        data["mentions"] = list(dict.fromkeys(mention_ids))

    poll = _extract_poll(getattr(message, "poll", None))
    if poll:
        data["poll"] = poll
    if attachments:
        data["attachments"] = [dict(item) for item in attachments]
    if sticker_info:
        data["sticker"] = dict(sticker_info)
    if is_call:
        data["call"] = {"status": "incoming"}

    choice_matched = _match_choice(text, reply_id, choices_prompts or {})
    if choice_matched is not None:
        val, q_id = choice_matched
        data["choice"] = val
        data.setdefault("reply_to", {"message_id": q_id})
        data["reply_to"].setdefault("message_id", q_id)

    if is_call:
        desc = f"{sender_label} started a call"
    elif sticker_info and not text and not attachments:
        s_name = str(sticker_info.get("name") or "sticker")
        desc = f"{sender_label} sent a sticker :{s_name}:"
    elif attachments:
        summary = describe_attachments(attachments, sent_by=sender_label)
        desc = f"{sender_label}: {text} {summary}".strip() if text else summary
    elif poll and not text:
        opts = ", ".join(poll.get("options") or ())
        desc = f"{sender_label} created poll '{poll.get('question', '')}': {opts}".strip()
    else:
        desc = f"{sender_label}: {text}".strip()

    if not desc and not data.get("attachments") and not sticker_info and not poll:
        return None

    mentioned_bot = bool(bot_id and (
        any(str(getattr(u, "id", "")) == str(bot_id) for u in raw_mentions)
        or f"<@{bot_id}>" in text or f"<@!{bot_id}>" in text
    ))
    replied_bot = bool(
        (reply_author_id and bot_id and reply_author_id == str(bot_id))
        or (reply_id and own_sent and reply_id in own_sent)
    )
    addressed = bool(dm or mentioned_bot or replied_bot or choice_matched is not None)

    ts = _event_ts(getattr(message, "created_at", None), now)
    payload: dict[str, Any] = {
        "source_name": sender_label,
        "account": str(getattr(author, "name", "") or sender_label),
        "place_kind": place_kind,
        "place_info": p_info,
        "addressed": addressed,
        "description": desc,
        "data": data,
    }
    if backfill:
        payload["backfill"] = True
    if imported:
        payload["imported"] = True
        payload["sent_at"] = ts.isoformat()
    return Event(id=None, ts=ts, kind="percept", who=f"discord:{author_id}",
                 place=place, body=name, payload=payload)


def format_edit(channel: Any, author: Any, msg_id: str, new_text: str, *,
                now: datetime, bot_id: str, directory: DiscordDirectory,
                name: str = "discord") -> Event | None:
    """Turn an edited Discord message into a percept with `data.edits`."""
    author_id = str(getattr(author, "id", "") or "")
    if not author_id or (bot_id and author_id == str(bot_id)) or channel is None:
        return None
    label = directory.remember_user(author)
    p_info = directory.remember_channel(channel, author=author)
    dm = is_dm_channel(channel)
    return Event(
        id=None, ts=now, kind="percept", who=f"discord:{author_id}",
        place=place_id(channel), body=name,
        payload={
            "source_name": label,
            "place_kind": "direct" if dm else "shared",
            "place_info": p_info,
            "addressed": dm,
            "description": f"{label} edited: {new_text}" if new_text else f"{label} edited {msg_id}",
            "data": {"message_id": msg_id, "edits": msg_id,
                     **_base_guild_data(channel, author)},
        },
    )


def format_delete(channel: Any, msg_id: str, *, author: Any = None, now: datetime,
                  bot_id: str, directory: DiscordDirectory,
                  name: str = "discord") -> Event | None:
    """Turn a deleted Discord message into a percept with `data.deletes`."""
    if channel is None or not msg_id:
        return None
    author_id = str(getattr(author, "id", "") or "")
    if author_id and bot_id and author_id == str(bot_id):
        return None
    label = directory.remember_user(author) if author is not None else "someone"
    who = f"discord:{author_id}" if author_id else f"discord:place:{getattr(channel, 'id', '')}"
    p_info = directory.remember_channel(channel, author=author)
    dm = is_dm_channel(channel)
    return Event(
        id=None, ts=now, kind="percept", who=who, place=place_id(channel), body=name,
        payload={
            "source_name": label,
            "place_kind": "direct" if dm else "shared",
            "place_info": p_info,
            "addressed": False,
            "description": f"{label} deleted a message ({msg_id})",
            "data": {"message_id": msg_id, "deletes": msg_id,
                     **_base_guild_data(channel, author)},
        },
    )


def format_reaction(channel: Any, user: Any, msg_id: str, emoji: Any, *,
                    now: datetime, bot_id: str, directory: DiscordDirectory,
                    own_sent: set[str] | None = None, target_author_id: str = "",
                    name: str = "discord") -> Event | None:
    """Turn a Discord reaction add into a percept (`addressed` when on the soul's own message)."""
    user_id = str(getattr(user, "id", "") or "")
    if not user_id or (bot_id and user_id == str(bot_id)) or channel is None:
        return None
    label = directory.remember_user(user)
    p_info = directory.remember_channel(channel, author=user)
    dm = is_dm_channel(channel)
    emoji_text = str(emoji)
    on_own = bool(
        (target_author_id and bot_id and str(target_author_id) == str(bot_id))
        or (own_sent and str(msg_id) in own_sent)
    )
    return Event(
        id=None, ts=now, kind="percept", who=f"discord:{user_id}",
        place=place_id(channel), body=name,
        payload={
            "source_name": label,
            "place_kind": "direct" if dm else "shared",
            "place_info": p_info,
            "addressed": on_own or dm,
            "description": f"{label} reacted {emoji_text} to {msg_id}",
            "data": {"message_id": str(msg_id),
                     "reaction": {"message_id": str(msg_id), "emoji": emoji_text},
                     **_base_guild_data(channel, user)},
        },
    )


def format_interaction(interaction: Any, *, now: datetime, bot_id: str,
                       directory: DiscordDirectory,
                       choices_prompts: Mapping[str, list[str]] | None = None,
                       name: str = "discord") -> Event | None:
    """Turn a button press or select-menu interaction into a percept (approval or `data.choice`)."""
    user = getattr(interaction, "user", None)
    user_id = str(getattr(user, "id", "") or "")
    if not user_id or (bot_id and user_id == str(bot_id)):
        return None
    channel = getattr(interaction, "channel", None)
    if channel is None:
        return None
    label = directory.remember_user(user)
    p_info = directory.remember_channel(channel, author=user)
    dm = is_dm_channel(channel)
    raw_data = dict(getattr(interaction, "data", None) or {})
    custom_id = str(raw_data.get("custom_id") or "")
    msg = getattr(interaction, "message", None)
    msg_id = str(getattr(msg, "id", "") or "")
    guild_data = _base_guild_data(channel, user)

    parts = custom_id.split(":")
    if len(parts) == 3 and parts[0] == "approval":
        choice = parts[1]
        data: dict[str, Any] = {**guild_data}
        if msg_id:
            data["reply_to"] = {"message_id": msg_id}
        return Event(
            id=None, ts=now, kind="percept", who=f"discord:{user_id}",
            place=place_id(channel), body=name,
            payload={
                "source_name": label,
                "place_kind": "direct" if dm else "shared",
                "place_info": p_info,
                "addressed": True,
                "description": choice,
                "approval": int(parts[2]),
                "answer": choice == "yes",
                "data": data,
            },
        )

    values = list(raw_data.get("values") or ())
    selected: str | list[str]
    if values:
        selected = values[0] if len(values) == 1 else [str(v) for v in values]
    elif len(parts) >= 3 and parts[0] == "choice":
        opts = (choices_prompts or {}).get(msg_id)
        try:
            idx = int(parts[1]) - 1
            selected = opts[idx] if opts and 0 <= idx < len(opts) else ":".join(parts[2:])
        except ValueError:
            selected = ":".join(parts[2:])
    else:
        selected = custom_id or str(raw_data.get("name") or "")

    data = {"custom_id": custom_id, "choice": selected, **guild_data}
    if msg_id:
        data["reply_to"] = {"message_id": msg_id}
    return Event(
        id=None, ts=now, kind="percept", who=f"discord:{user_id}",
        place=place_id(channel), body=name,
        payload={
            "source_name": label,
            "place_kind": "direct" if dm else "shared",
            "place_info": p_info,
            "addressed": True,
            "description": f"{label} chose {selected}",
            "data": data,
        },
    )
