"""The soft quota of one person's writable layer, and resetting it."""

from __future__ import annotations

import pathlib
import shutil
from typing import Any

from . import paths, sandbox


def layer(api: Any, who: str) -> pathlib.Path:
    """Return one person's overlay layer folder."""
    return sandbox.sandbox_root(api) / "layers" / paths.person_folder(who)


def used(api: Any, who: str) -> int:
    """Return how many bytes sit in one layer's upper folder."""
    upper = layer(api, who) / "upper"
    if not upper.is_dir():
        return 0
    return sum(path.stat().st_size for path in upper.rglob("*") if path.is_file())


def reset(api: Any, who: str) -> int:
    """Remove one person's upper and work folders; return how many files went."""
    folder = layer(api, who)
    files = sum(1 for path in folder.rglob("*") if path.is_file()) if folder.is_dir() else 0
    for name in ("upper", "work"):
        target = folder / name
        if target.is_dir():
            shutil.rmtree(target)
    return files
