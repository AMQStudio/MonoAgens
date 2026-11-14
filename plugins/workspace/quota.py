"""The soft quota of one person's writable layer, and resetting it."""

from __future__ import annotations

import os
import pathlib
import shutil
from typing import Any

from. import paths, sandbox


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
            _make_writable(target)
            shutil.rmtree(target)
    return files


def _make_writable(path: pathlib.Path) -> None:
    """Make one layer folder removable, even where bubblewrap left a mode-000 work dir.

    The layer belongs to the person, so chmod on an unreadable folder of theirs is always
    allowed; a folder that still refuses is left for the next sweep rather than hidden.
    """
    try:
        os.chmod(path, 0o700)
        entries = list(os.scandir(path))
    except OSError:
        return
    for entry in entries:
        if entry.is_dir(follow_symlinks=False):
            _make_writable(pathlib.Path(entry.path))
