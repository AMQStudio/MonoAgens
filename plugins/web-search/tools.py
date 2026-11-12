"""The plugin's two tools: search the web, and read one public page."""

from __future__ import annotations

import asyncio
from typing import Any

import httpx

from . import fetch, providers

MAX_RESULTS = 10
SEARCH = {
    "name": "web_search",
    "description": "Search the web and report titles, links and snippets.",
    "schema": {"type": "object",
               "properties": {"query": {"type": "string"},
                              "max_results": {"type": "integer", "minimum": 1,
                                              "maximum": MAX_RESULTS}},
               "required": ["query"]},
    "communicative": False, "long_running": False, "identity_changing": False,
}
FETCH = {
    "name": "web_fetch",
    "description": "Read one public web page as text.",
    "schema": {"type": "object",
               "properties": {"url": {"type": "string"},
                              "max_chars": {"type": "integer", "minimum": 200}},
               "required": ["url"]},
    "communicative": False, "long_running": False, "identity_changing": False,
}


class WebSearchTool:
    """One tool with the search chain and the guarded page reader."""

    name = "web-search"

    def __init__(self, api: Any, *, transport: httpx.AsyncBaseTransport | None = None) -> None:
        """Keep the plugin API (options, env, the blocked table) and an optional transport."""
        self.api = api
        self.transport = transport

    def capabilities(self) -> tuple[dict[str, Any], ...]:
        """Declare search and fetch."""
        return (SEARCH, FETCH)

    async def act(self, action: Any) -> dict[str, Any]:
        """Run one of the two tools."""
        if action.kind == "web_search":
            return await self.search(str(action.payload.get("query") or ""),
                                     int(action.payload.get("max_results") or 5))
        if action.kind == "web_fetch":
            return await self.fetch(str(action.payload.get("url") or ""),
                                    int(action.payload.get("max_chars") or 12000))
        return {"error": f"unknown tool: {action.kind}"}

    async def search(self, query: str, max_results: int = 5) -> dict[str, Any]:
        """Search through the provider chain; an empty query is refused."""
        query = query.strip()
        if not query:
            return {"error": "empty query"}
        limit = max(1, min(MAX_RESULTS, int(max_results or 5)))
        found = await providers.search(self.api.options, self.api.env, query, limit,
                                       transport=self.transport)
        return found

    async def fetch(self, url: str, max_chars: int = 12000) -> dict[str, Any]:
        """Read one page off the loop, with its addresses checked."""
        if not url.strip():
            return {"error": "empty url"}
        options = self.api.options
        ports = tuple(int(port) for port in options.get("allow_ports") or (80, 443))
        return await asyncio.to_thread(
            fetch.fetch, url.strip(), ports=ports,
            max_redirects=int(options.get("max_redirects") or fetch.MAX_REDIRECTS),
            timeout=float(options.get("timeout_seconds") or 20),
            limit=max(200, int(max_chars or 12000)))
