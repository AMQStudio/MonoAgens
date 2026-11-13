"""The capabilities the WhatsApp adapter offers and what it can show besides a message."""

from __future__ import annotations

from typing import Any

# An adapter's talking capabilities declare no permission (middleware.md §4, D59).
CAPABILITIES: tuple[dict[str, Any], ...] = (
    {
        "name": "say",
        "description": "Send a message in a WhatsApp chat, with files, reply, mentions, sticker, choices or location.",
        "schema": {
            "type": "object",
            "properties": {
                "text": {"type": "string"},
                "attachments": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Attachment references.",
                },
                "reply_to": {"type": "string", "description": "Message id to quote/reply to."},
                "mentions": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Account or person ids to mention.",
                },
                "sticker": {"type": "string", "description": "Attachment reference of a sticker."},
                "choices": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Options to offer as a poll or numbered list.",
                },
                "multiple": {"type": "boolean", "description": "Allow picking several choices."},
                "thread": {"type": "string", "description": "Topic/thread title."},
                "location": {
                    "type": "object",
                    "properties": {
                        "lat": {"type": "number"},
                        "lon": {"type": "number"},
                        "name": {"type": "string"},
                    },
                    "required": ["lat", "lon"],
                },
                "voice": {"type": "boolean", "description": "Send audio attachment as a voice note."},
            },
        },
        "communicative": True,
        "long_running": False,
        "identity_changing": False,
        "facts": (),
        "permission": None,
    },
    {
        "name": "react",
        "description": "React to a WhatsApp message with one emoji.",
        "schema": {
            "type": "object",
            "properties": {"emoji": {"type": "string"}, "message_id": {"type": "string"}},
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
        "description": "Edit one of the soul's own recent WhatsApp messages.",
        "schema": {
            "type": "object",
            "properties": {"message_id": {"type": "string"}, "text": {"type": "string"}},
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
        "description": "Pin a message in the current WhatsApp chat.",
        "schema": {
            "type": "object",
            "properties": {"message_id": {"type": "string"}, "seconds": {"type": "integer"}},
            "required": ["message_id"],
        },
        "communicative": False,
        "long_running": False,
        "identity_changing": False,
        "facts": ("changes",),
        "permission": "whatsapp.pin",
    },
    "reject_call": {
        "name": "reject_call",
        "description": "Reject an incoming WhatsApp call.",
        "schema": {
            "type": "object",
            "properties": {"call_id": {"type": "string"}, "caller": {"type": "string"}},
            "required": ["call_id"],
        },
        "communicative": False,
        "long_running": False,
        "identity_changing": False,
        "facts": ("changes",),
        "permission": "whatsapp.call",
    },
}

READS = (
    "text", "reply", "mention", "edit", "delete", "reaction", "sticker",
    "location", "contact", "poll", "view_once", "files", "call",
    "place_event", "voice_note", "choices_answer",
)

SHOWS = (
    "typing", "read_receipts", "edit", "delete", "reply", "reactions",
    "polls", "stickers", "locations",
)

GAPS: dict[str, str] = {
    "buttons": "WhatsApp consumer accounts over neonize cannot send interactive reply buttons",
    "menus": "WhatsApp consumer accounts cannot send list menus; choices use polls or numbered lists",
    "embeds": "WhatsApp has no separate embed card object beyond link previews",
    "threads": "WhatsApp chats have no sub-thread primitive; thread falls back to a titled reply",
    "history": "WhatsApp pushes pairing HistorySync events automatically instead of on-demand pull",
}
