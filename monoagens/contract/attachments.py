"""Attachments as the contract states them (contract.md §4.2)."""

from __future__ import annotations

from collections.abc import Mapping, Sequence
from dataclasses import dataclass
from typing import Any

# The contract's word for "the body cannot tell who this is" (contract.md §4.2).
UNKNOWN = "unknown"


@dataclass(frozen=True, slots=True)
class Attachment:
    """One file a percept carries, by reference into the attachment store."""

    ref: str
    name: str = ""
    mime: str = ""
    size: int = 0


def describe_attachments(attachments: Sequence[Attachment | Mapping[str, Any]], *,
                         sent_by: str = "someone") -> str:
    """Render the one line reasoning sees: `Budi sent 3 files: report.pdf (att_ab12…)`.

    The reference is part of the line because it is the handle the soul reads the file
    with (`workspace.read(ref=…)`); a name alone would leave the file unreadable.
    """
    found = list(attachments or ())
    names = [_label(item) for item in found]
    if not names:
        return f"{sent_by} sent no file"
    plural = "" if len(names) == 1 else "s"
    return f"{sent_by} sent {len(names)} file{plural}: " + ", ".join(names)


def _label(item: Attachment | Mapping[str, Any]) -> str:
    """Name one attachment the way the reasoning reads it."""
    name = str(_read(item, "name") or f"{int(_read(item, 'size') or 0)} bytes")
    ref = str(_read(item, "ref") or "")
    return f"{name} ({ref})" if ref else name


def _read(item: Attachment | Mapping[str, Any], field: str) -> Any:
    """Read one attachment field from either shape the layers use."""
    if isinstance(item, Mapping):
        return item.get(field)
    return getattr(item, field, None)
