"""Read one PDF with pypdf, natively: page count first, then the text of every page."""

from __future__ import annotations

from pathlib import Path


def read_pdf(source: bytes | str | Path) -> str:
    """Return `pages: N` and then the text of every page, separated by form feeds."""
    from pypdf import PdfReader  # lazy: the library is installed only with this plugin

    from io import BytesIO

    reader = PdfReader(BytesIO(source) if isinstance(source, bytes) else str(source))
    pages = [page.extract_text() or "" for page in reader.pages]
    return f"pages: {len(pages)}\n" + "\n\f\n".join(pages)
