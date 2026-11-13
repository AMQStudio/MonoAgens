"""Discord place ids, parent server chains, permissions and member role extraction."""

from __future__ import annotations

from typing import Any

import discord


def place_id(channel: Any) -> str:
    """Return the neutral place id for a Discord DM, guild channel or thread."""
    if isinstance(channel, str):
        if channel.startswith("space:dc:"):
            return channel
        return f"space:dc:{channel}"
    return f"space:dc:{getattr(channel, 'id', '')}"


def guild_place_id(guild: Any) -> str:
    """Return the neutral place id for a Discord server (guild)."""
    gid = getattr(guild, "id", guild)
    raw = str(gid or "")
    return raw if raw.startswith("space:dc:guild:") else f"space:dc:guild:{raw}"


def raw_channel_id(place: str) -> str:
    """Extract the numeric/raw Discord channel or thread id from a neutral place id."""
    text = str(place or "")
    for prefix in ("space:dc:guild:", "space:dc:thread:", "space:dc:channel:", "space:dc:"):
        if text.startswith(prefix):
            return text.removeprefix(prefix)
    return text


def is_dm_channel(channel: Any) -> bool:
    """Return whether `channel` is a Discord private DM or group DM."""
    ctype = getattr(channel, "type", None)
    if ctype in (discord.ChannelType.private, discord.ChannelType.group):
        return True
    return isinstance(channel, discord.DMChannel) and getattr(channel, "guild", None) is None


def channel_form(channel: Any) -> str:
    """Map a Discord channel object to the D68 place `form` vocabulary."""
    if is_dm_channel(channel):
        return "dm"
    ctype = getattr(channel, "type", None)
    if ctype in (
        discord.ChannelType.public_thread,
        discord.ChannelType.private_thread,
        discord.ChannelType.news_thread,
    ) or hasattr(channel, "parent_id") and getattr(channel, "parent", None) is not None and (
        ctype not in (discord.ChannelType.text, discord.ChannelType.news,
                      discord.ChannelType.voice, discord.ChannelType.stage_voice,
                      discord.ChannelType.forum, discord.ChannelType.media,
                      discord.ChannelType.category)
    ):
        return "thread"
    if ctype == discord.ChannelType.news:
        return "broadcast"
    if ctype in (discord.ChannelType.forum, discord.ChannelType.media):
        return "forum"
    return "group"


def can_post_in(channel: Any, *, bot_user: Any = None) -> bool:
    """Return whether the bot has permission to send messages in `channel`."""
    if is_dm_channel(channel):
        return True
    perms_fn = getattr(channel, "permissions_for", None)
    if not callable(perms_fn):
        return True
    guild = getattr(channel, "guild", None)
    me = bot_user or (getattr(guild, "me", None) if guild is not None else None)
    if me is None:
        return True
    try:
        perms = perms_fn(me)
    except Exception:
        return True
    form = channel_form(channel)
    if form == "thread":
        send_thread = getattr(perms, "send_messages_in_threads", None)
        if send_thread is not None:
            return bool(send_thread)
    send_msg = getattr(perms, "send_messages", None)
    return True if send_msg is None else bool(send_msg)


def member_roles(author: Any) -> list[str]:
    """Return non-default Discord role ids held by `author` as strings."""
    roles = getattr(author, "roles", None) or ()
    out: list[str] = []
    seen: set[str] = set()
    for role in roles:
        rid = getattr(role, "id", None)
        if rid is None:
            continue
        is_default = getattr(role, "is_default", None)
        if callable(is_default) and is_default():
            continue
        if getattr(role, "name", None) == "@everyone":
            continue
        text = str(rid)
        if text not in seen:
            seen.add(text)
            out.append(text)
    return out


def extract_place_info(channel: Any, *, bot_user: Any = None,
                       author: Any = None) -> tuple[dict[str, Any], dict[str, dict[str, Any]]]:
    """Build primary D68 place_info for `channel` plus cached entries for parent & guild."""
    related: dict[str, dict[str, Any]] = {}
    form = channel_form(channel)
    guild = getattr(channel, "guild", None)
    parent: str | None = None
    if guild is not None:
        g_id = guild_place_id(guild)
        g_info: dict[str, Any] = {
            "form": "guild",
            "parent": None,
            "can_post": False,
            "server_id": str(getattr(guild, "id", "")),
        }
        g_name = getattr(guild, "name", None)
        if g_name:
            g_info["name"] = str(g_name)
        m_count = getattr(guild, "member_count", None)
        if isinstance(m_count, int):
            g_info["members"] = m_count
        related[g_id] = g_info
        parent = g_id

    if form == "thread":
        p_chan = getattr(channel, "parent", None)
        p_id = getattr(channel, "parent_id", None)
        if p_chan is not None:
            p_place = place_id(p_chan)
            p_info: dict[str, Any] = {
                "form": channel_form(p_chan),
                "parent": parent,
                "can_post": can_post_in(p_chan, bot_user=bot_user),
            }
            if getattr(p_chan, "name", None):
                p_info["name"] = str(p_chan.name)
            related[p_place] = p_info
            parent = p_place
        elif p_id is not None:
            p_place = f"space:dc:{p_id}"
            related[p_place] = {"form": "group", "parent": parent, "can_post": True}
            parent = p_place

    info: dict[str, Any] = {
        "form": form,
        "parent": parent,
        "can_post": can_post_in(channel, bot_user=bot_user),
    }
    name = getattr(channel, "name", None)
    if not name and form == "dm":
        recip = getattr(channel, "recipient", None) or author
        name = getattr(recip, "display_name", None) or getattr(recip, "name", None)
    if name:
        info["name"] = str(name)
    members = getattr(channel, "members", None)
    if members is not None and not isinstance(members, int):
        try:
            info["members"] = len(members)
        except TypeError:
            pass
    if guild is not None and author is not None:
        rids = member_roles(author)
        if rids:
            info["roles"] = [f"{guild.id}/{rid}" for rid in rids]
    related[place_id(channel)] = info
    parents = {pid: str(meta["parent"]) for pid, meta in related.items() if meta.get("parent")}
    if parents:
        info["parents"] = parents
    return info, related


class DiscordDirectory:
    """In-memory place hierarchy and sender display-name directory for the Discord adapter."""

    def __init__(self) -> None:
        self._places: dict[str, dict[str, Any]] = {}
        self._labels: dict[str, str] = {}

    def remember_user(self, user: Any) -> str:
        """Record a user's display name and return it."""
        uid = f"discord:{getattr(user, 'id', '')}"
        label = str(
            getattr(user, "display_name", None)
            or getattr(user, "global_name", None)
            or getattr(user, "name", None)
            or "someone"
        )
        self._labels[uid] = label
        return label

    def label(self, account: str) -> str:
        """Return the last known display label for `account`."""
        return self._labels.get(account) or "someone"

    def remember_channel(self, channel: Any, *, bot_user: Any = None,
                         author: Any = None) -> dict[str, Any]:
        """Cache `channel`, its parent channel, and its guild server place metadata."""
        primary, related = extract_place_info(channel, bot_user=bot_user, author=author)
        for pid, meta in related.items():
            prev = dict(self._places.get(pid) or {})
            prev.update({k: v for k, v in meta.items() if v is not None})
            self._places[pid] = prev
        return primary

    def place_info(self, place: str) -> dict[str, Any]:
        """Return cached place metadata for `place`, else safe defaults."""
        if place in self._places:
            return dict(self._places[place])
        if str(place).startswith("space:dc:guild:"):
            return {"form": "guild", "parent": None, "can_post": False,
                    "server_id": raw_channel_id(place)}
        return {"form": "group", "parent": None, "can_post": True}
