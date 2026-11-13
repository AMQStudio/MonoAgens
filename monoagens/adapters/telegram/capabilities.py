"""Telegram capabilities, neutral `say` fields, optional tools, and D67 coverage."""

from __future__ import annotations

from typing import Any

# An adapter's talking capabilities declare no permission (middleware.md §4, D59).
CAPABILITIES: tuple[dict[str, Any], ...] = (
    {
        "name": "say",
        "description": (
            "Send a message in a Telegram chat or forum topic, with files, reply, "
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
                "reply_to": {"type": "string"},
                "mentions": {"type": "array", "items": {"type": "string"}},
                "sticker": {"type": "string"},
                "choices": {"type": "array", "items": {"type": "string"}},
                "multiple": {"type": "boolean"},
                "thread": {"type": "string"},
                "location": {
                    "type": "object",
                    "properties": {
                        "lat": {"type": "number"},
                        "lon": {"type": "number"},
                        "name": {"type": "string"},
                    },
                    "required": ["lat", "lon"],
                },
                "voice": {"type": "boolean"},
            },
            "required": ["text"],
        },
        "communicative": True,
        "long_running": False,
        "identity_changing": False,
        "facts": (),
        "permission": None,
    },
    {
        "name": "react",
        "description": "React to a Telegram message with one emoji.",
        "schema": {
            "type": "object",
            "properties": {"emoji": {"type": "string"}, "message_id": {"type": "integer"}},
            "required": ["emoji"],
        },
        "communicative": False,
        "long_running": False,
        "identity_changing": False,
        "facts": (),
        "permission": None,
    },
    {
        "name": "edit",
        "description": "Edit one of the soul's own Telegram messages.",
        "schema": {
            "type": "object",
            "properties": {"message_id": {"type": "integer"}, "text": {"type": "string"}},
            "required": ["message_id", "text"],
        },
        "communicative": False,
        "long_running": False,
        "identity_changing": False,
        "facts": (),
        "permission": None,
    },
)

OPTIONAL_CAPABILITIES: dict[str, dict[str, Any]] = {
    "pin": {
        "name": "pin",
        "description": "Pin a message in the current Telegram chat.",
        "schema": {
            "type": "object",
            "properties": {"message_id": {"type": "integer"}, "silent": {"type": "boolean"}},
            "required": ["message_id"],
        },
        "communicative": False,
        "long_running": False,
        "identity_changing": False,
        "facts": ("changes",),
        "permission": "telegram.pin",
    },
    "unpin": {
        "name": "unpin",
        "description": "Unpin a message in the current Telegram chat.",
        "schema": {
            "type": "object",
            "properties": {"message_id": {"type": "integer"}},
            "required": ["message_id"],
        },
        "communicative": False,
        "long_running": False,
        "identity_changing": False,
        "facts": ("changes",),
        "permission": "telegram.pin",
    },
}

READS = (
    "text", "reply", "mention", "edit", "delete", "reaction", "sticker",
    "location", "contact", "poll", "files", "place_event", "voice_note",
    "choices_answer",
)

# Keeps "typing, edit, buttons, reactions" first for existing boot-report substring checks.
SHOWS = (
    "typing", "edit", "buttons", "reactions", "delete", "reply",
    "menus", "polls", "stickers", "threads", "locations",
)

GAPS: dict[str, str] = {
    "view_once": "the Telegram Bot API does not expose self-destruct view-once flags on updates",
    "call": "the Telegram Bot API delivers no voice or video call events",
    "read_receipts": "the Telegram Bot API has no method to send read receipts",
    "embeds": "Telegram uses text formatting and inline keyboards rather than separate embed cards",
    "history": "the Telegram Bot API provides no chat history fetch API beyond updates",
}
