"""The capabilities the terminal offers, what it reads and shows, and its declared gaps."""

from __future__ import annotations

from typing import Any

# An adapter's talking capabilities declare no permission (middleware.md §4, D59).
CAPABILITIES: tuple[dict[str, Any], ...] = (
    {
        "name": "say",
        "description": (
            "Write a text line to the terminal, with optional files, reply, mentions, "
            "sticker, numbered choices, titled thread section or location."
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
        "permission": "cli.say",
    },
    {
        "name": "react",
        "description": "Write one emoji status line to the terminal.",
        "schema": {
            "type": "object",
            "properties": {"emoji": {"type": "string"}, "message_id": {"type": "integer"}},
            "required": ["emoji"],
        },
        "communicative": False,
        "long_running": False,
        "identity_changing": False,
        "facts": (),
        "permission": "cli.react",
    },
)

OPTIONAL_CAPABILITIES: dict[str, dict[str, Any]] = {
    "pin": {
        "name": "pin",
        "description": "Print a highlighted pinned notice line in the terminal.",
        "schema": {
            "type": "object",
            "properties": {"text": {"type": "string"}, "message_id": {"type": "string"}},
            "required": ["text"],
        },
        "communicative": True,
        "long_running": False,
        "identity_changing": False,
        "facts": ("changes",),
        "permission": "cli.pin",
    },
}

READS = ("text", "reply", "files", "choices_answer")
SHOWS = ("typing", "reply")
GAPS: dict[str, str] = {
    "mention": "the local terminal is a 1:1 owner stream with no user mentions",
    "edit": "line-oriented socket input is append-only; lines cannot be edited in place",
    "delete": "line-oriented socket input is append-only; lines cannot be deleted retroactively",
    "reaction": "the plain-text socket client sends text lines, not emoji reactions",
    "reactions": "the plain-text terminal stream cannot render emoji reactions; choices use numbered lines",
    "sticker": "the terminal client sends file paths via @path, not sticker packs",
    "location": "the terminal client has no GPS location picker",
    "locations": "the terminal stream has no native map picker; location falls back to text coordinates",
    "contact": "the terminal client has no contact card picker",
    "poll": "the terminal client has no interactive poll widget",
    "view_once": "local files attached over Unix socket are regular filesystem files",
    "call": "the Unix socket CLI is text-only and has no voice/video calls",
    "place_event": "the CLI is a single fixed owner direct place with no join/leave events",
    "voice_note": "the Unix socket CLI carries text and @path files, not live audio recording",
    "read_receipts": "the Unix socket stream prints lines immediately without read receipts",
    "buttons": "plain terminal lines cannot render clickable buttons; choices use numbered lines",
    "menus": "plain terminal lines cannot render select menus; choices use numbered lines",
    "polls": "plain terminal lines have no poll widget; choices use numbered lines",
    "stickers": "the terminal renders sticker fallback as text",
    "embeds": "plain terminal output renders text lines rather than rich embed cards",
    "threads": "a single terminal stream has no sub-threads; thread falls back to titled lines",
    "history": "the Unix socket stream is live-only and stores no chat scrollback",
}
