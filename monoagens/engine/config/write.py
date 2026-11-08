"""Write a host file the operator may have bind-mounted (ADR-0002 §4.1, ADR-0007 §4.1).

`config.yaml`, `roles.yaml`, and `soul.yaml` are mounted into the container as single files. A
single-file bind mount refuses `rename(2)` onto the mount point with `EBUSY`, so the atomic path
(a sibling temp file plus `os.replace`) falls back to truncating and rewriting the same inode,
which the mount does see. Every engine writer of an operator file goes through this function, so
no path can keep an older contract and fail only on a compose deployment.
"""


import errno
import os
from pathlib import Path

DEFAULT_MODE = 0o644


def _in_place(target: Path, text: str) -> None:
    """Rewrite the same inode: what a bind-mounted file needs when rename is refused."""
    with open(target, "r+" if target.exists() else "w", encoding="utf-8") as handle:
        handle.write(text)
        handle.truncate()
        handle.flush()
        os.fsync(handle.fileno())


def write_text(path: str | Path, text: str, *, mode: int | None = None) -> None:
    """Validate-free atomic write; an `EBUSY` rename falls back to an in-place write.

    `mode` applies only to a file that does not exist yet; an existing file keeps its mode, so
    a `0600` roles file stays `0600`.
    """
    target = Path(path)
    target.parent.mkdir(parents=True, exist_ok=True)
    target_mode = (
        target.stat().st_mode & 0o777 if target.exists() else (mode or DEFAULT_MODE)
    )
    temp = target.with_name(f".{target.name}.tmp")
    try:
        with open(temp, "w", encoding="utf-8") as handle:
            handle.write(text)
            handle.flush()
            os.fsync(handle.fileno())
        os.chmod(temp, target_mode)
        os.replace(temp, target)
    except OSError as exc:
        if exc.errno != errno.EBUSY:
            raise
        _in_place(target, text)
    finally:
        temp.unlink(missing_ok=True)
