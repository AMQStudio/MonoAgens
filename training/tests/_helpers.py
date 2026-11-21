"""Shared helpers for pytest discovery across main checkouts and linked git worktrees."""

from __future__ import annotations

import subprocess
from pathlib import Path


def resolve_data_repo_root(default_root: Path) -> Path:
    """Returns `default_root` when `lab/datasets.manifest.toml` is present, else the main git repo root."""
    if (default_root / "lab" / "datasets.manifest.toml").is_file():
        return default_root
    try:
        common_dir = subprocess.check_output(
            ["git", "rev-parse", "--git-common-dir"],
            cwd=str(default_root),
            text=True,
            stderr=subprocess.DEVNULL,
        ).strip()
        candidate = Path(common_dir).resolve().parent
        if (candidate / "lab" / "datasets.manifest.toml").is_file():
            return candidate
    except (OSError, subprocess.CalledProcessError):
        pass
    return default_root
