"""Receiving: one getUpdates call, media specs, and Telegram Bot API file downloads."""

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
    """Return one spec per file in a message: doc, photo, audio, video, voice or sticker."""
    found: list[dict[str, Any]] = []
    document = message.get("document") or {}
    if document:
        found.append({"file_id": document.get("file_id"),
                      "name": document.get("file_name") or "document",
                      "mime": document.get("mime_type") or DEFAULT_MIME,
                      "size": int(document.get("file_size") or 0),
                      "kind": "document"})
    if message.get("photo"):
        largest = max(message["photo"], key=lambda item: item.get("file_size") or 0)
        found.append({"file_id": largest.get("file_id"), "name": "photo.jpg",
                      "mime": "image/jpeg", "size": int(largest.get("file_size") or 0),
                      "kind": "photo"})
    for key, name, mime in (
        ("audio", "audio.mp3", "audio/mpeg"),
        ("video", "video.mp4", "video/mp4"),
        ("animation", "animation.mp4", "video/mp4"),
        ("video_note", "video_note.mp4", "video/mp4"),
        ("voice", "voice.ogg", "audio/ogg"),
    ):
        item = message.get(key) or {}
        if item:
            found.append({"file_id": item.get("file_id"),
                          "name": item.get("file_name") or name,
                          "mime": item.get("mime_type") or mime,
                          "size": int(item.get("file_size") or 0),
                          "kind": key})
    sticker = message.get("sticker") or {}
    if sticker and sticker.get("file_id"):
        ext = "webm" if sticker.get("is_video") else ("tgs" if sticker.get("is_animated") else "webp")
        mime = ("video/webm" if sticker.get("is_video")
                else ("application/x-tgsticker" if sticker.get("is_animated") else "image/webp"))
        found.append({"file_id": sticker.get("file_id"),
                      "name": f"sticker.{ext}",
                      "mime": mime,
                      "size": int(sticker.get("file_size") or 0),
                      "kind": "sticker",
                      "emoji": sticker.get("emoji") or "",
                      "set_name": sticker.get("set_name") or ""})
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
