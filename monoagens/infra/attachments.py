"""Attachments: the files that arrive with a message or leave with one (middleware.md §10).

Files live on disk under `var/attachments`, content-addressed: the same bytes always get one
ref, so a file sent in several places costs one copy. Nothing about an attachment is stored
in a table: its audience comes from the events that carried it (see `contract`), and the file
is deleted when no such event remains.
"""

from __future__ import annotations

import asyncio
import hashlib
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
_UNSAFE = re.compile(r"[^A-Za-z0-9._ -]+")


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

    async def put(self, data: bytes, *, name: str, mime: str = DEFAULT_MIME) -> dict[str, Any]:
        """Write one file and return its attachment: ref, name, mime and size."""
        if len(data) > MAX_BYTES:
            raise ValueError(f"file too large: {len(data)}")
        ref = REF_PREFIX + hashlib.sha256(data).hexdigest()[:REF_DIGITS]
        target = self.path(ref, name)
        if not target.is_file():
            await asyncio.to_thread(self._write, target, data)
        return {"ref": ref, "name": target.name, "mime": mime, "size": len(data)}

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
        files = sorted(found for found in folder.iterdir() if found.is_file())
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
