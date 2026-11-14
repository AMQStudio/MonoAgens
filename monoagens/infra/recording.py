"""What a body sensed and would send, recorded as JSON lines (§4.7)."""

from __future__ import annotations

import json
import logging
import pathlib
from typing import Any

LOGGER = logging.getLogger(__name__)


class Recorder:
    """Append one body's events and would-be actions to a file, one JSON object per line."""

    def __init__(self, path: str | pathlib.Path) -> None:
        """Keep the path; the file is created on the first entry."""
        self.path = pathlib.Path(path)

    def write(self, entry: dict[str, Any]) -> None:
        """Append one entry; a caller records through `support.record`, which never raises."""
        self.path.parent.mkdir(parents=True, exist_ok=True)
        with self.path.open("a", encoding="utf-8") as handle:
            handle.write(json.dumps(entry, ensure_ascii=False, default=str) + "\n")
