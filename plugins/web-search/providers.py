"""The provider chain: first success wins, every failure carries a structured reason.

A provider whose key or URL is missing is skipped with `missing_key`; an HTTP error is
`http_<status>`, a timeout is `timeout`, a transport failure `transport_error`, and any
other exception is named by its class. A success with no results is still a success.
"""

from __future__ import annotations

import asyncio
from collections.abc import Mapping
from typing import Any

import httpx

from . import normalize

USER_AGENT = "monoagens"
DEFAULT_PROVIDERS = ("searxng", "tavily", "brave", "exa", "serper", "google", "ddgs")
DDGS_ATTEMPTS = 4
DDGS_WAIT = 1.5
DDGS_TIMEOUT = 10


def _reason(exc: BaseException) -> str:
    """Return the structured reason one provider failed."""
    if isinstance(exc, httpx.TimeoutException):
        return "timeout"
    if isinstance(exc, httpx.HTTPStatusError):
        return f"http_{exc.response.status_code}"
    if isinstance(exc, httpx.TransportError):
        return "transport_error"
    return type(exc).__name__.lower()


async def search(options: Mapping[str, Any], env: Mapping[str, str], query: str, limit: int,
                 *, transport: httpx.AsyncBaseTransport | None = None) -> dict[str, Any]:
    """Ask the providers in order and return the first answer, or every reason tried."""
    chain = [str(name) for name in (options.get("providers") or DEFAULT_PROVIDERS)]
    tried: list[dict[str, str]] = []
    timeout = float(options.get("timeout_seconds") or 20)
    async with httpx.AsyncClient(transport=transport, timeout=timeout,
                                 headers={"User-Agent": USER_AGENT}) as client:
        for name in chain:
            try:
                found = await _call(name, options, env, query, limit, client)
            except _Skipped as skip:
                tried.append({"provider": name, "reason": skip.reason})
                continue
            except Exception as exc:  # a provider never crashes the search
                tried.append({"provider": name, "reason": _reason(exc)})
                continue
            if found is not None:
                return {"provider": name, "results": found}
            tried.append({"provider": name, "reason": "empty"})
    return {"error": "no provider answered", "tried": tried}


class _Skipped(Exception):
    """One provider cannot be tried: its key or URL is missing."""

    def __init__(self, reason: str) -> None:
        super().__init__(reason)
        self.reason = reason


async def _call(name: str, options: Mapping[str, Any], env: Mapping[str, str], query: str,
                limit: int, client: httpx.AsyncClient) -> list[dict[str, Any]] | None:
    """Try one provider; None means it answered with no results."""
    if name == "searxng":
        base = str(options.get("searxng_url") or env.get("SEARXNG_URL") or "")
        if not base:
            raise _Skipped("missing_key")
        response = await client.get(f"{base.rstrip('/')}/search",
                                    params={"q": query, "format": "json"})
        response.raise_for_status()
        rows = normalize.searxng(response.json())
    elif name == "tavily":
        key = _key(env, "TAVILY_API_KEY")
        response = await client.post("https://api.tavily.com/search",
                                     json={"query": query, "max_results": limit},
                                     headers={"Authorization": f"Bearer {key}"})
        response.raise_for_status()
        rows = normalize.tavily(response.json())
    elif name == "brave":
        key = _key(env, "BRAVE_API_KEY")
        response = await client.get("https://api.search.brave.com/res/v1/web/search",
                                    params={"q": query, "count": limit},
                                    headers={"X-Subscription-Token": key})
        response.raise_for_status()
        rows = normalize.brave(response.json())
    elif name == "exa":
        key = _key(env, "EXA_API_KEY")
        response = await client.post(
            "https://api.exa.ai/search",
            json={"query": query, "numResults": limit,
                  "contents": {"text": {"maxCharacters": 500}}},
            headers={"x-api-key": key})
        response.raise_for_status()
        rows = normalize.exa(response.json())
    elif name == "serper":
        key = _key(env, "SERPER_API_KEY")
        response = await client.post("https://google.serper.dev/search",
                                     json={"q": query, "num": limit},
                                     headers={"X-API-KEY": key})
        response.raise_for_status()
        rows = normalize.serper(response.json())
    elif name == "google":
        key, cx = _key(env, "GOOGLE_API_KEY"), _key(env, "GOOGLE_CX")
        response = await client.get("https://www.googleapis.com/customsearch/v1",
                                    params={"key": key, "cx": cx, "q": query, "num": limit})
        response.raise_for_status()
        rows = normalize.google(response.json())
    elif name == "ddgs":
        rows = normalize.ddgs(await _ddgs(query, limit))
    else:
        raise _Skipped("unknown_provider")
    return rows or None


def _key(env: Mapping[str, str], name: str) -> str:
    """Return one provider key or skip the provider."""
    value = str(env.get(name) or "")
    if not value:
        raise _Skipped("missing_key")
    return value


async def _ddgs(query: str, limit: int) -> list[dict[str, Any]]:
    """Ask ddgs in a worker thread, retrying a few times before giving up."""
    try:
        from ddgs import DDGS
    except ImportError as exc:
        raise _Skipped("ddgs_unavailable") from exc
    for attempt in range(DDGS_ATTEMPTS):
        try:
            return await asyncio.to_thread(
                lambda: list(DDGS(timeout=DDGS_TIMEOUT).text(query, max_results=limit)))
        except Exception:
            if attempt == DDGS_ATTEMPTS - 1:
                raise
            await asyncio.sleep(DDGS_WAIT)
    return []
