"""Web Search: `web_search` through a provider chain, and a guarded `web_fetch`."""

from __future__ import annotations

from typing import Any

from .tools import WebSearchTool


def register(api: Any) -> None:
    """Add the search and fetch tools; the manifest declares their facts and permissions."""
    api.add_tool(WebSearchTool(api))
