"""Discord inbound event formatter: messages, edits, deletions, reactions, interactions, places."""

from __future__ import annotations

import contextlib
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
from monoagens.adapters import support
from monoagens.contract import Answer, Audience, Percept, describe_attachments

UNKNOWN_ACTOR = "unknown"


def _event_ts(raw_dt: datetime | None, fallback: datetime) -> datetime:
    if raw_dt is None:
        return fallback
    if raw_dt.tzinfo is None:
        return raw_dt.replace(tzinfo=timezone.utc)
    return raw_dt.astimezone(timezone.utc)


def audience_of(channel: Any, *, actor: str) -> Audience:
    """Return who can perceive one Discord place, with its own rule folded in (§2.4.3).

    A DM is the other person; a guild channel is its members — but when anyone who joins later
    can read the history, nobody's past words were private, so the answer is *unknown*.
    """
    guild = getattr(channel, "guild", None)
    if guild is None:
        found = frozenset({actor}) if actor and actor != UNKNOWN_ACTOR else frozenset()
        return Audience(actors=found, unknown=not found)
    default = getattr(guild, "default_role", None)
    permissions_for = getattr(channel, "permissions_for", None)
    if default is not None and callable(permissions_for):
        with contextlib.suppress(Exception):
            perms = permissions_for(default)
            if getattr(perms, "read_message_history", False):
                return Audience(actors=frozenset(), unknown=True)
    members = frozenset(
        f"discord:{getattr(member, 'id', '')}"
        for member in (getattr(channel, "members", None) or ())
        if getattr(member, "id", None) is not None)
    return Audience(actors=members, unknown=not members)


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


def format_message(message: Any, *, now: datetime, bot_id: str, bot_user: Any = None,
                   directory: DiscordDirectory,
                   attachments: Sequence[Mapping[str, Any]] = (),
                   sticker_info: Mapping[str, Any] | None = None,
                   own_sent: set[str] | None = None,
                   backfill: bool = False, imported: bool = False,
                   name: str = "discord") -> Percept | None:
    """Turn one Discord message into a percept of the contract, skipping the soul's own echoes."""
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
    msg_id = str(getattr(message, "id", "") or "")
    text = str(getattr(message, "content", "") or "")
    embeds = list(getattr(message, "embeds", None) or ())
    if not text and embeds:
        parts = [str(getattr(e, "title", "") or getattr(e, "description", "") or "")
                 for e in embeds]
        text = " | ".join(p for p in parts if p)

    msg_type = getattr(message, "type", None)
    is_call = msg_type == discord.MessageType.call

    data: dict[str, Any] = {"message_id": msg_id, "place_name": p_info.get("name", "")}
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
    spoken_to = bool(dm or mentioned_bot or replied_bot)
    data["account"] = str(getattr(author, "name", "") or sender_label)
    del name
    return Percept(
        actor=f"discord:{author_id}",
        place=place,
        occurred_at=_event_ts(getattr(message, "created_at", None), now),
        description=desc,
        ref=msg_id,
        about=reply_id,
        involves=tuple(mention_ids),
        attachments=support.attachments_of(attachments),
        features={"addressed": 1 if spoken_to else 0,
                  "named": 1 if mentioned_bot else 0,
                  "addressed_other": 1 if len(mention_ids) > 1 or (
                      mention_ids and not mentioned_bot) else 0},
        backfill=backfill,
        imported=imported,
        data=data,
    )


def format_edit(channel: Any, author: Any, msg_id: str, new_text: str, *,
                now: datetime, bot_id: str, directory: DiscordDirectory,
                name: str = "discord") -> Percept | None:
    """Turn an edited Discord message into the percept of that edit (§5.4)."""
    del name
    author_id = str(getattr(author, "id", "") or "")
    if not author_id or (bot_id and author_id == str(bot_id)) or channel is None:
        return None
    label = directory.remember_user(author)
    directory.remember_channel(channel, author=author)
    dm = is_dm_channel(channel)
    return Percept(
        actor=f"discord:{author_id}",
        place=place_id(channel),
        occurred_at=now,
        description=(f"{label} edited: {new_text}" if new_text
                     else f"{label} edited {msg_id}"),
        ref=msg_id,
        about=msg_id,
        features={"addressed": 1 if dm else 0, "named": 0, "addressed_other": 0},
        data={"message_id": msg_id, "edits": msg_id, "text": new_text},
    )


def format_delete(channel: Any, msg_id: str, *, author: Any = None, now: datetime,
                  bot_id: str, directory: DiscordDirectory,
                  name: str = "discord") -> Percept | None:
    """Turn a deleted Discord message into the percept of that deletion (§5.4)."""
    del name
    if channel is None or not msg_id:
        return None
    author_id = str(getattr(author, "id", "") or "")
    if author_id and bot_id and author_id == str(bot_id):
        return None
    label = directory.remember_user(author) if author is not None else "someone"
    actor = (f"discord:{author_id}" if author_id
             else f"discord:place:{getattr(channel, 'id', '')}")
    directory.remember_channel(channel, author=author)
    dm = is_dm_channel(channel)
    return Percept(
        actor=actor,
        place=place_id(channel),
        occurred_at=now,
        description=f"{label} deleted a message ({msg_id})",
        ref=msg_id,
        about=msg_id,
        features={"addressed": 1 if dm else 0, "named": 0, "addressed_other": 0},
        data={"message_id": msg_id, "deletes": msg_id},
    )


def format_reaction(channel: Any, user: Any, msg_id: str, emoji: Any, *,
                    now: datetime, bot_id: str, directory: DiscordDirectory,
                    own_sent: set[str] | None = None, target_author_id: str = "",
                    name: str = "discord") -> Percept | None:
    """Turn a Discord reaction add into the percept of that reaction (§5.4)."""
    del name
    user_id = str(getattr(user, "id", "") or "")
    if not user_id or (bot_id and user_id == str(bot_id)) or channel is None:
        return None
    label = directory.remember_user(user)
    directory.remember_channel(channel, author=user)
    dm = is_dm_channel(channel)
    emoji_text = str(emoji)
    on_own = bool(
        (target_author_id and bot_id and str(target_author_id) == str(bot_id))
        or (own_sent and str(msg_id) in own_sent)
    )
    return Percept(
        actor=f"discord:{user_id}",
        place=place_id(channel),
        occurred_at=now,
        description=f"{label} reacted {emoji_text} to {msg_id}",
        ref=str(msg_id),
        about=str(msg_id),
        features={"addressed": 1 if (on_own or dm) else 0, "named": 0,
                  "addressed_other": 1 if not on_own else 0},
        data={"message_id": str(msg_id), "reaction": {"message_id": str(msg_id),
                                                      "emoji": emoji_text},
              "own": on_own},
    )


def format_interaction(interaction: Any, *, now: datetime, bot_id: str,
                       directory: DiscordDirectory,
                       choices_prompts: Mapping[str, list[str]] | None = None,
                       name: str = "discord") -> Percept | None:
    """Turn a button press into the structured answer it was drawn for (§2.4.6)."""
    user = getattr(interaction, "user", None)
    user_id = str(getattr(user, "id", "") or "")
    if not user_id or (bot_id and user_id == str(bot_id)):
        return None
    channel = getattr(interaction, "channel", None)
    if channel is None:
        return None
    del name
    label = directory.remember_user(user)
    directory.remember_channel(channel, author=user)
    raw_data = dict(getattr(interaction, "data", None) or {})
    custom_id = str(raw_data.get("custom_id") or "")
    msg = getattr(interaction, "message", None)
    msg_id = str(getattr(msg, "id", "") or "")

    parts = custom_id.split(":")
    values = list(raw_data.get("values") or ())
    selected: str | list[str]
    answer_val: str | list[str]
    if values:
        selected = values[0] if len(values) == 1 else [str(v) for v in values]
        answer_val = selected
    elif len(parts) >= 3 and parts[0] == "choice":
        opts = (choices_prompts or {}).get(msg_id)
        try:
            idx = int(parts[1]) - 1
            selected = opts[idx] if opts and 0 <= idx < len(opts) else ":".join(parts[2:])
        except ValueError:
            selected = ":".join(parts[2:])
        answer_val = custom_id
    else:
        selected = custom_id or str(raw_data.get("name") or "")
        answer_val = custom_id or str(selected)

    value = selected if isinstance(selected, str) else " | ".join(selected)
    return Percept(
        actor=f"discord:{user_id}",
        place=place_id(channel),
        occurred_at=now,
        description=f"{label} chose {selected}",
        ref=msg_id,
        about=msg_id,
        features={"addressed": 1, "named": 0, "addressed_other": 0},
        answer=Answer(prompt=msg_id, value=value) if msg_id else None,
        data={"custom_id": custom_id, "choice": selected, "answer": answer_val,
              "message_id": msg_id},
    )
