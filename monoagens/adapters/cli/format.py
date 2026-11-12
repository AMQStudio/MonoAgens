"""One typed line becomes one neutral event; the terminal is the owner's direct chat."""

from __future__ import annotations

from collections.abc import Mapping, Sequence
from typing import Any

from monoagens.infra.clock import Clock
from monoagens.middleware.contract import describe_attachments
from monoagens.types import Event

PLACE = "space:cli"
OWNER = "person:owner"


def event(text: str, *, clock: Clock, name: str = "cli",
          attachments: Sequence[Mapping[str, Any]] = ()) -> Event:
    """Build the percept of one typed line (the owner typing in their own terminal)."""
    payload: dict[str, Any] = {"description": text, "addressed": True, "place_kind": "direct"}
    if attachments:
        payload["data"] = {"attachments": [dict(item) for item in attachments]}
        summary = describe_attachments(attachments, sent_by=OWNER)
        payload["description"] = f"{text} {summary}".strip() if text else summary
    return Event(id=None, ts=clock.now(), kind="percept", who=OWNER, place=PLACE, body=name,
                 payload=payload)


def split_line(line: str) -> tuple[list[str], str]:
    """Split `@a.txt|@b.txt|please read` into its paths and its text."""
    if not str(line).startswith("@"):
        return [], str(line)
    paths, words = [], []
    for part in str(line).split("|"):
        piece = part.strip()
        if piece.startswith("@"):
            paths.append(piece[1:].strip())
        elif piece:
            words.append(piece)
    return paths, " ".join(words)
