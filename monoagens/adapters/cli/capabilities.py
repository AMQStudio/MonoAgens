"""The capabilities the terminal offers and what it can show besides a line of text."""

from __future__ import annotations

from typing import Any

# An adapter's talking capabilities declare no permission (middleware.md §4, D59).
CAPABILITIES: tuple[dict[str, Any], ...] = (
    {"name": "say", "description": "Write a text line to the terminal.",
     "schema": {"type": "object", "properties": {"text": {"type": "string"}},
                "required": ["text"]},
     "communicative": True, "long_running": False, "identity_changing": False,
     "facts": (), "permission": None},
    {"name": "react", "description": "Write one emoji to the terminal.",
     "schema": {"type": "object",
                "properties": {"emoji": {"type": "string"},
                               "message_id": {"type": "integer"}},
                "required": ["emoji"]},
     "communicative": False, "long_running": False, "identity_changing": False,
     "facts": (), "permission": None},
)

SHOWS = ("typing",)
