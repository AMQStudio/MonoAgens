"""Where a person may read and write: their own workspace and the folders they share."""

from __future__ import annotations

import pathlib
import re
from typing import Any

SAFE_ID = re.compile(r"[^A-Za-z0-9._-]+")


def person_folder(who: str) -> str:
    """Return the folder name of one person: its id with the colons made safe."""
    return SAFE_ID.sub("_", str(who)).strip("_") or "unknown"


def root_of(api: Any) -> pathlib.Path:
    """Return the workspace root: the option, or <deployment>/var/workspaces."""
    given = str(api.options.get("root") or "")
    return pathlib.Path(given) if given else pathlib.Path(api.deployment_root) / "var" / "workspaces"


def roots(api: Any, who: str) -> dict[str, pathlib.Path]:
    """Return every folder one person may use: `{"": own}` plus each shared name."""
    base = root_of(api)
    found = {"": base / person_folder(who)}
    shared = api.role_settings(who).get("workspace.shared") or []
    for name in [shared] if isinstance(shared, str) else list(shared or ()):
        found[str(name)] = base / "shared" / str(name)
    return found


def resolve(api: Any, who: str, path: str) -> pathlib.Path:
    """Resolve one path inside the person's folders; anything outside is refused.

    `shared/<name>/…` names a shared folder; every other path belongs to the person's own
    workspace. `..`, an absolute path and a symlink that leaves the folder are refused.
    """
    text = str(path or "").strip().replace("\\", "/")
    if text.startswith("/") or re.match(r"^[A-Za-z]:", text):
        raise PermissionError(f"{path} is outside your workspace")
    folders = roots(api, who)
    name, _, rest = text.partition("/")
    if name in folders and name:
        base, relative = folders[name], rest
    else:
        base, relative = folders[""], text
    candidate = (base / relative).resolve()
    if candidate != base.resolve() and not candidate.is_relative_to(base.resolve()):
        raise PermissionError(f"{path} is outside your workspace")
    return candidate


def display(api: Any, who: str, path: pathlib.Path) -> str:
    """Return the path as the person wrote it (`""` for their own workspace)."""
    for name, base in roots(api, who).items():
        resolved = base.resolve()
        if path == resolved or path.is_relative_to(resolved):
            relative = str(path.relative_to(resolved)).replace("\\", "/")
            return f"{name}/{relative}".strip("/") if relative else name
    return str(path)
