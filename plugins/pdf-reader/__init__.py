"""PDF Reader: fills the Workspace `reader` point for `.pdf` files."""

from __future__ import annotations

from typing import Any

from .reader import read_pdf


def register(api: Any) -> None:
    """Fill `.pdf` with the native reader; a missing pypdf is the loader's note."""
    api.fill_point("reader", ".pdf", read_pdf)
