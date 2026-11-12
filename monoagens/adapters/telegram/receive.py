"""Receiving: one getUpdates call. The polling loop and its backoff live in the adapter."""

from __future__ import annotations

from collections.abc import Awaitable, Callable
from typing import Any

Call = Callable[..., Awaitable[Any]]


async def updates(call: Call, offset: int | None, timeout: float) -> list[dict[str, Any]]:
    """Ask Telegram for the updates after `offset`; an empty answer is normal."""
    params: dict[str, Any] = {"timeout": timeout}
    if offset is not None:
        params["offset"] = offset
    found = await call("getUpdates", **params)
    if not isinstance(found, list):
        raise RuntimeError("telegram: getUpdates answered without a list")
    return found


DEFAULT_MIME = "application/octet-stream"


def specs(message: dict[str, Any]) -> list[dict[str, Any]]:
    """Return one spec per file in one message: document, photo, audio, video or voice."""
    found: list[dict[str, Any]] = []
    document = message.get("document") or {}
    if document:
        found.append({"file_id": document.get("file_id"), "name": document.get("file_name")
                      or "document", "mime": document.get("mime_type") or DEFAULT_MIME,
                      "size": int(document.get("file_size") or 0)})
    if message.get("photo"):
        largest = max(message["photo"], key=lambda size: size.get("file_size") or 0)
        found.append({"file_id": largest.get("file_id"), "name": "photo.jpg",
                      "mime": "image/jpeg", "size": int(largest.get("file_size") or 0)})
    for key, name, mime in (("audio", "audio", "audio/mpeg"), ("video", "video", "video/mp4"),
                            ("voice", "voice", "audio/ogg")):
        item = message.get(key) or {}
        if item:
            found.append({"file_id": item.get("file_id"),
                          "name": item.get("file_name") or name,
                          "mime": item.get("mime_type") or mime,
                          "size": int(item.get("file_size") or 0)})
    return [spec for spec in found if spec["file_id"]]


def group(message: dict[str, Any]) -> str | None:
    """Return the media group one message belongs to, when it belongs to one."""
    found = message.get("media_group_id")
    return str(found) if found else None


async def download(http: Any, base_url: str, token: str,
                   file_id: str) -> bytes:
    """Ask for one file's path, then download its bytes."""
    response = await http.post(f"{base_url}/bot{token}/getFile", json={"file_id": file_id})
    response.raise_for_status()
    payload = response.json()
    if not payload.get("ok"):
        raise RuntimeError(f"telegram getFile failed: {payload.get('description')}")
    path = (payload.get("result") or {}).get("file_path")
    if not path:
        raise RuntimeError("telegram getFile answered without a path")
    found = await http.get(f"{base_url}/file/bot{token}/{path}")
    found.raise_for_status()
    return found.content
