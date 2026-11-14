"""Attachments: the files that arrive with a message or leave with one.

Files live on disk under `var/attachments`, content-addressed: the same bytes always get one
ref, so a file sent in several places costs one copy. Nothing about an attachment is stored
in a table: its audience comes from the events that carried it (see `contract`), and the file
is deleted when no such event remains.
"""

from __future__ import annotations

import asyncio
from collections.abc import Awaitable, Callable, Mapping
import hashlib
import json
import os
import pathlib
import re
from typing import Any

ATTACHMENT_KEYS = ("ref", "name", "mime", "size")
MAX_BYTES = 50 * 1024 * 1024
DEFAULT_MIME = "application/octet-stream"
REF_PREFIX = "att_"
REF_DIGITS = 24
MAX_NAME = 120
DEFERRED_META = ".deferred.json"
_UNSAFE = re.compile(r"[^A-Za-z0-9._ -]+")
Fetcher = Callable[[str, dict[str, Any]], Awaitable[bytes]]


def safe_filename(name: str) -> str:
    """Return a name that is safe on disk: its basename, printable and short."""
    base = pathlib.PurePath(str(name or "")).name
    cleaned = _UNSAFE.sub("_", base).strip(" .")[:MAX_NAME]
    return cleaned or "attachment"


class AttachmentStore:
    """The one place files are written and read; content-addressed, atomic, bounded."""

    def __init__(self, root: pathlib.Path) -> None:
        """Store files under this root (created on the first put)."""
        self.root = pathlib.Path(root)
        self._fetchers: dict[str, Fetcher] = {}

    def register_fetcher(self, body: str, fn: Fetcher) -> None:
        """Register the async callback an adapter uses to materialize deferred refs."""
        self._fetchers[str(body)] = fn

    def fetcher(self, body: str) -> Fetcher | None:
        """Return the registered deferred-attachment fetcher for one adapter, if any."""
        return self._fetchers.get(str(body))

    async def put(self, data: bytes, *, name: str, mime: str = DEFAULT_MIME) -> dict[str, Any]:
        """Write one file and return its attachment: ref, name, mime and size."""
        if len(data) > MAX_BYTES:
            raise ValueError(f"file too large: {len(data)}")
        ref = REF_PREFIX + hashlib.sha256(data).hexdigest()[:REF_DIGITS]
        target = self.path(ref, name)
        if not target.is_file():
            await asyncio.to_thread(self._write, target, data)
        return {"ref": ref, "name": target.name, "mime": mime, "size": len(data)}

    async def put_deferred(self, *, key: str, name: str, mime: str = DEFAULT_MIME,
                           size: int = 0, body: str = "",
                           locator: Mapping[str, Any] | None = None) -> dict[str, Any]:
        """Record metadata and a platform locator without downloading bytes yet."""
        digest = hashlib.sha256(f"{body}:{key}".encode("utf-8")).hexdigest()[:REF_DIGITS]
        ref = f"{REF_PREFIX}{digest}"
        safe = safe_filename(name)
        meta_path = self._meta_path(ref)
        if self.find(ref) is None and not meta_path.is_file():
            payload = {"name": safe, "mime": mime, "size": int(size or 0),
                       "body": str(body), "locator": dict(locator or {})}
            await asyncio.to_thread(self._write, meta_path, json.dumps(payload).encode("utf-8"))
        return {"ref": ref, "name": safe, "mime": mime, "size": int(size or 0), "deferred": True}

    def _meta_path(self, ref: str) -> pathlib.Path:
        """Return the deferred locator sidecar path for one ref."""
        return self.root / ref[len(REF_PREFIX):len(REF_PREFIX) + 2] / ref / DEFERRED_META

    async def read_deferred(self, ref: str) -> dict[str, Any] | None:
        """Return the deferred locator metadata for `ref`, if present."""
        path = self._meta_path(ref)
        if not path.is_file():
            return None
        raw = await asyncio.to_thread(path.read_text, encoding="utf-8")
        return dict(json.loads(raw) or {})

    async def materialize(self, ref: str, data: bytes, *, name: str) -> pathlib.Path:
        """Store downloaded bytes for a deferred ref and remove its sidecar manifest."""
        if len(data) > MAX_BYTES:
            raise ValueError(f"file too large: {len(data)}")
        target = self.path(ref, name)
        await asyncio.to_thread(self._write, target, data)
        meta = self._meta_path(ref)
        if meta.is_file():
            await asyncio.to_thread(meta.unlink)
        return target

    def _write(self, target: pathlib.Path, data: bytes) -> None:
        """Write one file atomically (.tmp then os.replace); never half a file."""
        target.parent.mkdir(parents=True, exist_ok=True)
        temporary = target.with_name(target.name + ".tmp")
        temporary.write_bytes(data)
        os.replace(temporary, target)

    def path(self, ref: str, name: str) -> pathlib.Path:
        """Return where one attachment lives: root/<2 hex>/<ref>/<safe name>."""
        safe = safe_filename(name)
        return self.root / ref[len(REF_PREFIX):len(REF_PREFIX) + 2] / ref / safe

    def find(self, ref: str) -> pathlib.Path | None:
        """Return the stored file of one ref, whichever name it was written under."""
        folder = self.root / ref[len(REF_PREFIX):len(REF_PREFIX) + 2] / ref
        if not folder.is_dir():
            return None
        files = sorted(found for found in folder.iterdir()
                       if found.is_file() and found.name != DEFERRED_META)
        return files[0] if files else None

    async def read(self, ref: str) -> bytes:
        """Return one attachment's bytes."""
        found = self.find(ref)
        if found is None:
            raise FileNotFoundError(f"no stored file for {ref}")
        return await asyncio.to_thread(found.read_bytes)

    async def digest(self, ref: str) -> str | None:
        """Return the sha256 of one stored file, or None when it is gone."""
        found = self.find(ref)
        if found is None:
            return None
        return await asyncio.to_thread(
            lambda: hashlib.sha256(found.read_bytes()).hexdigest())

    async def sweep(self, conn: Any) -> int:
        """Delete every file no non-forgotten event carries any more; return how many."""
        from monoagens.middleware import contract  # local: contract reads the store

        if not self.root.is_dir():
            return 0
        removed = 0
        for bucket in sorted(self.root.iterdir()):
            if not bucket.is_dir():
                continue
            for ref in sorted(part for part in bucket.iterdir() if part.is_dir()):
                if await contract.attachment_origins(conn, ref.name):
                    continue
                for found in ref.iterdir():
                    if found.is_file():
                        await asyncio.to_thread(found.unlink)
                        removed += 1
                await asyncio.to_thread(ref.rmdir)
            if not any(bucket.iterdir()):
                await asyncio.to_thread(bucket.rmdir)
        return removed
