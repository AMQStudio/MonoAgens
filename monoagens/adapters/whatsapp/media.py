"""WhatsApp media helpers: proto unwrapping, text extraction, deferred storage, and re-upload."""

from __future__ import annotations

import base64
import contextlib
import mimetypes
import pathlib
from typing import Any

MEDIA_FIELDS = ("imageMessage", "videoMessage", "audioMessage", "documentMessage", "stickerMessage")
WRAPPERS = (
    "viewOnceMessage",
    "viewOnceMessageV2",
    "viewOnceMessageV2Extension",
    "ephemeralMessage",
    "documentWithCaptionMessage",
    "associatedChildMessage",
    "editedMessage",
)
VIEW_ONCE_WRAPPERS = ("viewOnceMessage", "viewOnceMessageV2", "viewOnceMessageV2Extension")
STRUCTURED_FIELDS = (
    "locationMessage",
    "liveLocationMessage",
    "contactMessage",
    "contactsArrayMessage",
    "pollCreationMessage",
    "pollCreationMessageV2",
    "pollCreationMessageV3",
    "pollCreationMessageV4",
    "pollCreationMessageV5",
)
FALLBACK_MIME = {
    "image": "image/jpeg",
    "video": "video/mp4",
    "audio": "audio/ogg",
    "document": "application/octet-stream",
    "sticker": "image/webp",
}
MIME_ALIASES = {"image/jpg": "image/jpeg", "image/pjpeg": "image/jpeg", "image/x-png": "image/png"}
EXT_FOR_MIME = {
    "image/jpeg": ".jpg",
    "image/png": ".png",
    "image/webp": ".webp",
    "image/gif": ".gif",
    "application/pdf": ".pdf",
    "audio/ogg": ".ogg",
    "audio/mpeg": ".mp3",
    "audio/mp4": ".m4a",
    "video/mp4": ".mp4",
    "text/plain": ".txt",
    "application/json": ".json",
    "application/zip": ".zip",
}
PHOTO_MAGIC = (b"\xff\xd8\xff", b"\x89PNG\r\n\x1a\n")
VOICE_EXT = frozenset({".ogg", ".opus"})
AUDIO_EXT = frozenset({".mp3", ".m4a", ".wav", ".aac", ".flac"})
VIDEO_EXT = frozenset({".mp4", ".mov", ".webm", ".mkv"})
STICKER_MAX_SIDE = 512


def views_once(message: Any) -> bool:
    """Return whether `message` uses a view-once wrapper."""
    for name in VIEW_ONCE_WRAPPERS:
        wrapper = getattr(message, name, None)
        if wrapper is not None and _has_fields(wrapper):
            return True
    return False


def _has_fields(proto: Any) -> bool:
    list_fields = getattr(proto, "ListFields", None)
    return bool(list_fields()) if callable(list_fields) else bool(proto)


def unwrap_message(message: Any) -> Any:
    """Strip ephemeral, view-once, caption, and association wrappers down to the inner message."""
    current = message
    for _ in range(6):
        inner = None
        for name in WRAPPERS:
            wrapper = getattr(current, name, None)
            if wrapper is not None and _has_fields(wrapper):
                inner = getattr(wrapper, "message", None)
                if inner is not None:
                    break
        if inner is None:
            break
        current = inner
    return current


def contexts(message: Any):
    """Yield every populated `contextInfo` inside an unwrapped message."""
    body = unwrap_message(message)
    for field_name in ("extendedTextMessage", *MEDIA_FIELDS, *STRUCTURED_FIELDS):
        content = getattr(body, field_name, None)
        if content is None or not _has_fields(content):
            continue
        ctx = getattr(content, "contextInfo", None)
        if ctx is not None and _has_fields(ctx):
            yield ctx


def _extract_plain(body: Any) -> str:
    conv = getattr(body, "conversation", "") or ""
    if conv:
        return str(conv)
    ext = getattr(body, "extendedTextMessage", None)
    if ext is not None and getattr(ext, "text", ""):
        return str(ext.text)
    for field_name in MEDIA_FIELDS:
        item = getattr(body, field_name, None)
        cap = getattr(item, "caption", "") if item is not None else ""
        if cap:
            return str(cap)
    return ""


def location_info(body: Any) -> dict[str, Any] | None:
    """Extract `{lat, lon, name}` from a WhatsApp location message, if present."""
    for field_name in ("locationMessage", "liveLocationMessage"):
        content = getattr(body, field_name, None)
        if content is None or not _has_fields(content):
            continue
        lat = float(getattr(content, "degreesLatitude", 0.0) or 0.0)
        lon = float(getattr(content, "degreesLongitude", 0.0) or 0.0)
        name = str(getattr(content, "name", "") or getattr(content, "address", "") or "").strip()
        info: dict[str, Any] = {"lat": lat, "lon": lon}
        if name:
            info["name"] = name
        if field_name == "liveLocationMessage":
            info["live"] = True
        return info
    return None


def contacts_info(body: Any) -> list[dict[str, str]]:
    """Extract contact cards (`contactMessage` / `contactsArrayMessage`)."""
    found: list[dict[str, str]] = []
    single = getattr(body, "contactMessage", None)
    if single is not None and _has_fields(single):
        name = str(getattr(single, "displayName", "") or "").strip()
        vcard = str(getattr(single, "vcard", "") or "")
        found.append({"name": name, **({"vcard": vcard} if vcard else {})})
    multi = getattr(body, "contactsArrayMessage", None)
    if multi is not None and _has_fields(multi):
        for item in getattr(multi, "contacts", ()) or ():
            name = str(getattr(item, "displayName", "") or "").strip()
            vcard = str(getattr(item, "vcard", "") or "")
            found.append({"name": name, **({"vcard": vcard} if vcard else {})})
    return found


def poll_info(body: Any) -> dict[str, Any] | None:
    """Extract poll creation details (`question`, `options`, `selectable_count`)."""
    for field_name in ("pollCreationMessage", "pollCreationMessageV2", "pollCreationMessageV3",
                       "pollCreationMessageV4", "pollCreationMessageV5"):
        content = getattr(body, field_name, None)
        if content is None or not _has_fields(content):
            continue
        question = str(getattr(content, "name", "") or "").strip()
        options = [str(getattr(opt, "optionName", "") or "").strip()
                   for opt in getattr(content, "options", ()) or ()]
        return {
            "question": question,
            "options": [opt for opt in options if opt],
            "multiple": int(getattr(content, "selectableOptionsCount", 1) or 1) != 1,
        }
    return None


def message_text(message: Any) -> str:
    """Return human-readable text plus summaries of location, contact, and poll stanzas."""
    body = unwrap_message(message)
    parts = [_extract_plain(body)]
    loc = location_info(body)
    if loc:
        label = f" — {loc['name']}" if loc.get("name") else ""
        parts.append(f"[location: {loc['lat']:.6f}, {loc['lon']:.6f}{label}]")
    contacts = contacts_info(body)
    if contacts:
        names = ", ".join(c.get("name") or "contact" for c in contacts)
        parts.append(f"[contact: {names}]" if len(contacts) == 1 else f"[contacts: {names}]")
    poll = poll_info(body)
    if poll:
        opts = " / ".join(poll["options"])
        parts.append(f"[poll: {poll['question']} — {opts}]" if opts else f"[poll: {poll['question']}]")
    return " ".join(part.strip() for part in parts if part and part.strip())


def describe_media(message: Any, message_id: str) -> dict[str, Any] | None:
    """Return deferred-attachment metadata for a media protobuf stanza, or None."""
    body = unwrap_message(message)
    for field_name in MEDIA_FIELDS:
        content = getattr(body, field_name, None)
        if content is None or not _has_fields(content):
            continue
        kind = field_name[: -len("Message")].lower()
        declared = str(getattr(content, "mimetype", "") or "").split(";")[0].strip().lower()
        raw_name = str(getattr(content, "fileName", "") or getattr(content, "title", "") or "")
        name = pathlib.PurePath(raw_name).name if raw_name else f"{message_id}_{kind}"
        mime = declared or mimetypes.guess_type(name)[0] or FALLBACK_MIME.get(kind, "")
        mime = MIME_ALIASES.get(mime, mime) or "application/octet-stream"
        if not pathlib.PurePath(name).suffix:
            name = f"{name}{EXT_FOR_MIME.get(mime, '')}"
        size = int(getattr(content, "fileLength", 0) or 0)
        serialize = getattr(message, "SerializeToString", None)
        proto_b64 = base64.b64encode(serialize()).decode("ascii") if callable(serialize) else ""
        emoji = str(getattr(content, "accessibilityLabel", "") or "")
        return {
            "kind": kind,
            "name": name,
            "mime": mime,
            "size": size,
            "animated": bool(getattr(content, "isAnimated", False)) if kind == "sticker" else False,
            "emoji": emoji,
            "ptt": bool(getattr(content, "PTT", False)) if kind == "audio" else False,
            "locator": {
                "message_id": str(message_id),
                "kind": kind,
                "url": str(getattr(content, "URL", "") or getattr(content, "url", "") or ""),
                "direct_path": str(getattr(content, "directPath", "") or ""),
                "proto_b64": proto_b64,
            },
        }
    return None


async def store_deferred(files: Any, *, body: str, chat_jid: str, message_id: str,
                         message: Any) -> tuple[list[dict[str, Any]], dict[str, Any] | None]:
    """Register one media stanza as a deferred ref on `files`; returns `(attachments, spec)`."""
    spec = describe_media(message, message_id)
    if spec is None:
        return [], None
    locator = {**spec["locator"], "chat": chat_jid}
    key = f"{chat_jid}:{message_id}:{spec['kind']}"
    if files is not None and hasattr(files, "put_deferred"):
        ref = await files.put_deferred(
            key=key, name=spec["name"], mime=spec["mime"], size=spec["size"],
            body=body, locator=locator)
    else:
        ref = {"ref": "", "name": spec["name"], "mime": spec["mime"],
               "size": spec["size"], "deferred": True}
    return [ref], spec


async def fetch_media(client: Any, ref: str, meta: dict[str, Any]) -> bytes:
    """Download a deferred WhatsApp attachment, issuing a re-upload request if expired."""
    if client is None:
        raise RuntimeError(f"WhatsApp client is not running to fetch {ref}")
    locator = dict(meta.get("locator") or {})
    proto_b64 = str(locator.get("proto_b64") or "")
    proto: Any = None
    if proto_b64:
        from neonize.proto.waE2E.WAWebProtobufsE2E_pb2 import Message
        proto = Message.FromString(base64.b64decode(proto_b64))
    else:
        proto = locator
    try:
        data = await _download_proto(client, proto)
        if data:
            return data
    except Exception as exc:
        if not _is_expired_error(exc):
            raise RuntimeError(f"WhatsApp media download failed for {ref}: {exc}") from exc
    requested = await _request_reupload(client, locator, proto)
    if requested:
        data = await _download_proto(client, proto)
        if data:
            return data
    raise RuntimeError(
        f"WhatsApp media link expired for {ref}"
        + ("; re-upload requested from phone" if requested else "; unavailable")
    )


async def _download_proto(client: Any, proto: Any) -> bytes:
    data = await client.download_any(proto)
    return bytes(data) if data is not None else b""


def _is_expired_error(exc: Exception) -> bool:
    text = f"{type(exc).__name__}: {exc}".lower()
    return any(token in text for token in ("404", "410", "expired", "not found", "gone"))


async def _request_reupload(client: Any, locator: dict[str, Any], proto: Any) -> bool:
    chat = str(locator.get("chat") or "")
    msg_id = str(locator.get("message_id") or "")
    for method in ("request_media_reupload", "send_media_retry_receipt", "get_message_for_retry"):
        fn = getattr(client, method, None)
        if not callable(fn):
            continue
        with contextlib.suppress(Exception):
            if method == "request_media_reupload":
                res = fn(chat, msg_id, proto)
            elif method == "send_media_retry_receipt":
                res = fn(chat, msg_id)
            else:
                res = fn(chat, msg_id, msg_id)
            if hasattr(res, "__await__"):
                await res
            return True
    return False


def _webp_shape(head: bytes) -> tuple[int, int, bool] | None:
    if len(head) < 30 or head[:4] != b"RIFF" or head[8:12] != b"WEBP":
        return None
    chunk = head[12:16]
    if chunk == b"VP8X":
        return (int.from_bytes(head[24:27], "little") + 1,
                int.from_bytes(head[27:30], "little") + 1, bool(head[20] & 0x12))
    if chunk == b"VP8 ":
        return (int.from_bytes(head[26:28], "little") & 0x3FFF,
                int.from_bytes(head[28:30], "little") & 0x3FFF, False)
    if chunk == b"VP8L":
        bits = int.from_bytes(head[21:25], "little")
        return (bits & 0x3FFF) + 1, ((bits >> 14) & 0x3FFF) + 1, bool(bits >> 28)
    return None


def classify_send_bytes(data: bytes, name: str, mime: str = "", *, voice: bool = False) -> str:
    """Classify outbound attachment bytes as sticker, photo, voice, audio, video or file."""
    if voice:
        return "voice"
    head = data[:30]
    if head.startswith(PHOTO_MAGIC) or mime.startswith("image/jpeg") or mime.startswith("image/png"):
        return "photo"
    shape = _webp_shape(head) if head else None
    if shape is not None:
        width, height, marked = shape
        return "sticker" if marked and max(width, height) <= STICKER_MAX_SIDE else "photo"
    suffix = pathlib.PurePath(str(name or "")).suffix.lower()
    if suffix in VOICE_EXT or mime in ("audio/ogg", "audio/opus"):
        return "voice"
    if suffix in AUDIO_EXT or mime.startswith("audio/"):
        return "audio"
    if suffix in VIDEO_EXT or mime.startswith("video/"):
        return "video"
    return "file"
