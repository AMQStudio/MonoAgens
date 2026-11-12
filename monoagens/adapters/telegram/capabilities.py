"""The capabilities this adapter offers and what it can show besides a message."""

from __future__ import annotations

from typing import Any

# An adapter's talking capabilities declare no permission (middleware.md §4, D59).
CAPABILITIES: tuple[dict[str, Any], ...] = (
    {"name": "say", "description": "Send a text message in the current place, with files.",
     "schema": {"type": "object",
                "properties": {"text": {"type": "string"},
                               "attachments": {"type": "array", "items": {"type": "string"},
                                               "description": "Attachment references."}},
                "required": ["text"]},
     "communicative": True, "long_running": False, "identity_changing": False,
     "facts": (), "permission": None},
    {"name": "react", "description": "React to a message with one emoji.",
     "schema": {"type": "object",
                "properties": {"emoji": {"type": "string"},
                               "message_id": {"type": "integer"}},
                "required": ["emoji"]},
     "communicative": False, "long_running": False, "identity_changing": False,
     "facts": (), "permission": None},
)

SHOWS = ("typing", "edit", "buttons", "reactions")
