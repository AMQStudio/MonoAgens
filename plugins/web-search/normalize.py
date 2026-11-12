"""One shape for every provider: title, url, snippet and when it was published."""

from __future__ import annotations

from typing import Any


def result(title: Any, url: Any, snippet: Any = "", published: Any = None) -> dict[str, Any]:
    """Return one normalized search result."""
    return {"title": str(title or "").strip(), "url": str(url or "").strip(),
            "snippet": str(snippet or "").strip(),
            "published_at": str(published).strip() if published else None}


def searxng(payload: dict[str, Any]) -> list[dict[str, Any]]:
    """SearXNG answers `results[]` with url, title, content and publishedDate."""
    return [result(item.get("title"), item.get("url"), item.get("content"),
                   item.get("publishedDate"))
            for item in payload.get("results") or ()]


def tavily(payload: dict[str, Any]) -> list[dict[str, Any]]:
    """Tavily answers `results[]` with url, title, content and published_date."""
    return [result(item.get("title"), item.get("url"), item.get("content"),
                   item.get("published_date"))
            for item in payload.get("results") or ()]


def brave(payload: dict[str, Any]) -> list[dict[str, Any]]:
    """Brave answers `web.results[]` with url, title, description and page_age."""
    rows = (payload.get("web") or {}).get("results") or ()
    return [result(item.get("title"), item.get("url"), item.get("description"),
                   item.get("page_age")) for item in rows]


def exa(payload: dict[str, Any]) -> list[dict[str, Any]]:
    """Exa answers `results[]` with url, title, text (or summary) and publishedDate."""
    return [result(item.get("title"), item.get("url"), item.get("text") or item.get("summary"),
                   item.get("publishedDate"))
            for item in payload.get("results") or ()]


def serper(payload: dict[str, Any]) -> list[dict[str, Any]]:
    """Serper answers `organic[]` with link, title, snippet and date."""
    return [result(item.get("title"), item.get("link"), item.get("snippet"), item.get("date"))
            for item in payload.get("organic") or ()]


def google(payload: dict[str, Any]) -> list[dict[str, Any]]:
    """Google CSE answers `items[]` with link, title and snippet."""
    return [result(item.get("title"), item.get("link"), item.get("snippet"))
            for item in payload.get("items") or ()]


def ddgs(rows: list[dict[str, Any]]) -> list[dict[str, Any]]:
    """ddgs answers dicts with title, href (or url), body (or snippet) and published."""
    return [result(item.get("title"), item.get("href") or item.get("url"),
                   item.get("body") or item.get("snippet"), item.get("published"))
            for item in rows or ()]
