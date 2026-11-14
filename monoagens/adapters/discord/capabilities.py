"""The capabilities the Discord adapter offers and what it can show besides a message."""

from __future__ import annotations

from typing import Any

# An adapter's talking capabilities declare no permission (middleware.md §4, D59).
CAPABILITIES: tuple[dict[str, Any], ...] = (
    {
        "name": "say",
        "description": (
            "Send a message in a Discord channel, thread or DM, with files, reply, "
            "mentions, sticker, choices, thread or location."
        ),
        "schema": {
            "type": "object",
            "properties": {
                "text": {"type": "string"},
                "attachments": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Attachment references.",
                },
                "reply_to": {"type": "string", "description": "Message id to reply to."},
                "mentions": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Account or person ids to mention.",
                },
                "sticker": {"type": "string", "description": "Attachment or sticker id/ref."},
                "choices": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Options offered via buttons, a select menu, or numbered list.",
                },
                "multiple": {"type": "boolean", "description": "Allow picking several choices."},
                "thread": {"type": "string", "description": "Title of a new Discord thread."},
                "location": {
                    "type": "object",
                    "properties": {
                        "lat": {"type": "number"},
                        "lon": {"type": "number"},
                        "name": {"type": "string"},
                    },
                    "required": ["lat", "lon"],
                },
                "voice": {"type": "boolean", "description": "Speak aloud if supported."},
            },
        },
        "communicative": True,
        "long_running": False,
        "identity_changing": False,
        "facts": (),
        "permission": "discord.say",
    },
    {
        "name": "react",
        "description": "React to a Discord message with one emoji.",
        "schema": {
            "type": "object",
            "properties": {"emoji": {"type": "string"}, "message_id": {"type": "string"}},
            "required": ["emoji"],
        },
        "communicative": False,
        "long_running": False,
        "identity_changing": False,
        "facts": (),
        "permission": "discord.react",
    },
    {
        "name": "edit",
        "description": "Edit one of the soul's own Discord messages.",
        "schema": {
            "type": "object",
            "properties": {"message_id": {"type": "string"}, "text": {"type": "string"}},
            "required": ["message_id", "text"],
        },
        "communicative": False,
        "long_running": False,
        "identity_changing": False,
        "facts": (),
        "permission": "discord.edit",
    },
)

OPTIONAL_CAPABILITIES: dict[str, dict[str, Any]] = {
    "pin": {
        "name": "pin",
        "description": "Pin a message in the current Discord channel or thread.",
        "schema": {
            "type": "object",
            "properties": {"message_id": {"type": "string"}},
            "required": ["message_id"],
        },
        "communicative": False,
        "long_running": False,
        "identity_changing": False,
        "facts": ("changes",),
        "permission": "discord.pin",
    },
    "manage_role": {
        "name": "manage_role",
        "description": "Add or remove a Discord role on a server member.",
        "schema": {
            "type": "object",
            "properties": {
                "user_id": {"type": "string"},
                "role_id": {"type": "string"},
                "action": {"type": "string", "enum": ["add", "remove"]},
            },
            "required": ["user_id", "role_id", "action"],
        },
        "communicative": False,
        "long_running": False,
        "identity_changing": False,
        "facts": ("changes",),
        "permission": "discord.roles",
    },
}

READS = (
    "text", "reply", "mention", "edit", "delete", "reaction", "sticker",
    "location", "poll", "files", "call", "place_event", "voice_note",
    "choices_answer",
)

SHOWS = (
    "typing", "edit", "delete", "reply", "reactions", "buttons", "menus",
    "polls", "stickers", "embeds", "threads",
)

GAPS: dict[str, str] = {
    "view_once": "Discord has no view-once self-destructing media flag",
    "contact": "Discord has no native vCard contact card attachment type",
    "read_receipts": "Discord bot accounts cannot send per-message read receipts",
    "locations": "Discord has no native map pin widget; location renders as text coordinates and link",
}
