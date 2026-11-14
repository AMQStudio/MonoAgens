"""Read one PDF with pypdf, natively: page count first, then the text of every page."""

from __future__ import annotations

from pathlib import Path

MAX_BYTES = 20_000_000
MAX_PAGES = 200


def read_pdf(source: bytes | str | Path) -> str:
    """Return `pages: N` and then the text of the first pages, separated by form feeds.

    One file is read within a size and a page cap; beyond them the answer says what it left out
    rather than spending minutes on a book nobody asked to read whole.
    """
    from pypdf import PdfReader  # lazy: the library is installed only with this plugin

    from io import BytesIO

    if isinstance(source, bytes) and len(source) > MAX_BYTES:
        return f"error: the file is larger than {MAX_BYTES} bytes"
    reader = PdfReader(BytesIO(source) if isinstance(source, bytes) else str(source))
    total = len(reader.pages)
    pages = [page.extract_text() or "" for page in reader.pages[:MAX_PAGES]]
    head = f"pages: {total}" + (f" (read the first {MAX_PAGES})" if total > MAX_PAGES else "")
    return head + "\n" + "\n\f\n".join(pages)
