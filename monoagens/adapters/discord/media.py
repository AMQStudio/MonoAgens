"""Discord attachment and sticker capture (immediate or deferred fetch)."""

from __future__ import annotations

import mimetypes
from typing import Any

import httpx


def _guess_mime(att: Any) -> str:
    ctype = str(getattr(att, "content_type", "") or "").split(";", 1)[0].strip()
    if ctype:
        return ctype
    name = str(getattr(att, "filename", "") or "")
    guessed, _ = mimetypes.guess_type(name)
    return guessed or "application/octet-stream"


async def collect_attachments(message: Any, files: Any, *, place: str,
                              per_message: int = 10,
                              deferred: bool = False) -> list[dict[str, Any]]:
    """Store every attachment on `message` through `AttachmentDoor` (`put` or `put_deferred`)."""
    del place
    items = list(getattr(message, "attachments", None) or ())[:per_message]
    if not items or files is None:
        return []
    stored: list[dict[str, Any]] = []
    for att in items:
        name = str(getattr(att, "filename", None) or f"file_{getattr(att, 'id', 'bin')}")
        mime = _guess_mime(att)
        url = str(getattr(att, "url", "") or getattr(att, "proxy_url", "") or "")
        size = int(getattr(att, "size", 0) or 0)
        att_id = str(getattr(att, "id", "") or url)
        locator = {
            "url": url,
            "attachment_id": att_id,
            "channel_id": str(getattr(getattr(message, "channel", None), "id", "")),
            "message_id": str(getattr(message, "id", "")),
        }
        if deferred and url and hasattr(files, "put_deferred"):
            meta = await files.put_deferred(
                key=f"discord:{att_id}", name=name, mime=mime, size=size,
                body="discord", locator=locator,
            )
            stored.append(meta)
            continue
        reader = getattr(att, "read", None)
        if callable(reader):
            try:
                raw = await reader()
                meta = await files.put(raw, name=name, mime=mime)
                stored.append(meta)
                continue
            except Exception:
                pass
        if url and hasattr(files, "put_deferred"):
            meta = await files.put_deferred(
                key=f"discord:{att_id}", name=name, mime=mime, size=size,
                body="discord", locator=locator,
            )
            stored.append(meta)
    return stored


async def collect_sticker(message: Any, files: Any, *, place: str) -> dict[str, Any] | None:
    """Capture the first sticker on `message`, storing its asset when downloadable."""
    del place
    stickers = list(getattr(message, "stickers", None) or ())
    if not stickers:
        return None
    item = stickers[0]
    info: dict[str, Any] = {
        "id": str(getattr(item, "id", "") or ""),
        "name": str(getattr(item, "name", "") or ""),
        "format": str(getattr(item, "format", "") or ""),
    }
    url = str(getattr(item, "url", "") or "")
    if url:
        info["url"] = url
    if files is not None:
        reader = getattr(item, "read", None)
        if callable(reader):
            try:
                raw = await reader()
                meta = await files.put(raw, name=f"sticker_{info['id'] or 'img'}.png",
                                       mime="image/png")
                info["ref"] = meta.get("ref")
            except Exception:
                pass
    return info


async def fetch_deferred(meta: dict[str, Any], *,
                         http_client: httpx.AsyncClient | None = None) -> tuple[bytes, str]:
    """Download deferred Discord CDN media described by `meta['locator']['url']`."""
    locator = meta.get("locator") or meta.get("source") or {}
    url = str(locator.get("url") or "")
    if not url:
        raise FileNotFoundError("Discord deferred attachment has no source URL")
    mime = str(meta.get("mime") or "application/octet-stream")
    if http_client is not None:
        resp = await http_client.get(url)
        resp.raise_for_status()
        ct = str(resp.headers.get("content-type") or "").split(";", 1)[0].strip()
        return resp.content, ct or mime
    async with httpx.AsyncClient(timeout=20.0) as client:
        resp = await client.get(url)
        resp.raise_for_status()
        ct = str(resp.headers.get("content-type") or "").split(";", 1)[0].strip()
        return resp.content, ct or mime
