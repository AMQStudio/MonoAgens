"""The Workspace tools: read, write, edit, grep, glob and a sandboxed run."""

from __future__ import annotations

import asyncio
import contextlib
import os
import pathlib
import re
import secrets
import time
from typing import Any

from . import guard, paths, sandbox

MAX_LIMIT = 2000
# Bound what one run may hold in memory; the registry's output cap and marker then apply.
RUN_OUTPUT_BYTES = 50 * 1024 * 1024
DEFAULT_LINES = 400
GREP_TIMEOUT_SECONDS = 10.0
BINARY = re.compile(rb"[\x00-\x08\x0b\x0c\x0e-\x1f]")

READ = {"name": "read", "description": "Read a text file by line range, or one attachment.",
        "schema": {"type": "object",
                   "properties": {"path": {"type": "string"},
                                  "offset": {"type": "integer", "minimum": 0},
                                  "limit": {"type": "integer", "minimum": 1},
                                  "ref": {"type": "string"}},
                   "required": []},
        "communicative": False, "long_running": False, "identity_changing": False}
WRITE = {"name": "write", "description": "Write a whole text file, creating it if needed.",
         "schema": {"type": "object", "properties": {"path": {"type": "string"},
                                                    "text": {"type": "string"}},
                    "required": ["path", "text"]},
         "communicative": False, "long_running": False, "identity_changing": False}
EDIT = {"name": "edit", "description": "Replace one exact text in a file.",
        "schema": {"type": "object", "properties": {"path": {"type": "string"},
                                                   "old": {"type": "string"},
                                                   "new": {"type": "string"}},
                   "required": ["path", "old", "new"]},
        "communicative": False, "long_running": False, "identity_changing": False}
GREP = {"name": "grep", "description": "Search text files with a regular expression.",
        "schema": {"type": "object",
                   "properties": {"pattern": {"type": "string"},
                                  "path": {"type": "string"},
                                  "glob": {"type": "string"},
                                  "mode": {"type": "string",
                                           "enum": ["matches", "count", "files"]},
                                  "limit": {"type": "integer", "minimum": 1}},
                   "required": ["pattern"]},
        "communicative": False, "long_running": False, "identity_changing": False}
GLOB = {"name": "glob", "description": "List the paths that match a pattern.",
        "schema": {"type": "object", "properties": {"pattern": {"type": "string"},
                                                   "limit": {"type": "integer", "minimum": 1}},
                   "required": ["pattern"]},
        "communicative": False, "long_running": False, "identity_changing": False}
RUN = {"name": "run", "description": "Run a shell command in the sandbox and report its output.",
       "schema": {"type": "object", "properties": {"command": {"type": "string"},
                                                  "timeout": {"type": "integer", "minimum": 1},
                                                  "cwd": {"type": "string"}},
                  "required": ["command"]},
       "communicative": False, "long_running": True, "identity_changing": False}


class WorkspaceTool:
    """One tool with the six capabilities, over the caller's own folders."""

    name = "workspace"

    def __init__(self, api: Any) -> None:
        """Keep the plugin API (options, role settings, the attachment door)."""
        self.api = api

    def capabilities(self) -> tuple[dict[str, Any], ...]:
        """Declare the six tools."""
        return (READ, WRITE, EDIT, GREP, GLOB, RUN)

    async def act(self, action: Any) -> dict[str, Any]:
        """Run one tool for the person the act started with."""
        who = str(action.arguments.get("_who") or "")
        handlers = {"read": self.read, "write": self.write, "edit": self.edit,
                    "grep": self.grep, "glob": self.glob, "run": self.run}
        handler = handlers.get(action.capability)
        if handler is None:
            return {"error": f"unknown tool: {action.capability}"}
        try:
            return await handler(who, action)
        except PermissionError as refused:
            return {"error": str(refused)}
        except FileNotFoundError as missing:
            return {"error": f"no such file: {missing.filename or ''}".strip()}
        except IsADirectoryError:
            return {"error": "that path is a folder"}
        except (OSError, ValueError) as exc:
            return {"error": f"{type(exc).__name__.lower()}: {exc}"}

    # ------------------------------------------------------------- reading

    async def read(self, who: str, action: Any) -> dict[str, Any]:
        """Read lines of a file, or one attachment by its reference."""
        if action.arguments.get("ref"):
            return await self._read_attachment(who, action)
        raw_path = str(action.arguments.get("path") or "")
        target = paths.resolve(self.api, who, raw_path)
        ws_root = paths.root_for(self.api, who, raw_path)
        if not paths.inside(ws_root, target):
            raise PermissionError(f"{raw_path} is outside your workspace")
        data = await asyncio.to_thread(target.read_bytes)
        if BINARY.search(data[:4096]):
            return {"error": "that file is not text"}
        offset = max(0, int(action.arguments.get("offset") or 0))
        limit = max(1, min(MAX_LIMIT, int(action.arguments.get("limit") or DEFAULT_LINES)))
        lines = data.decode("utf-8", "replace").splitlines()
        window = lines[offset:offset + limit]
        return {"path": paths.display(self.api, who, target), "offset": offset,
                "lines": len(lines), "text": "\n".join(window)}

    async def _read_attachment(self, who: str, action: Any) -> dict[str, Any]:
        """Read one attachment through the door, then through a reader by its suffix."""
        data, info = await self.api.open_file(str(action.arguments["ref"]),
                                              context=action.place)
        name = str(info.get("name") or "attachment")
        suffix = pathlib.PurePath(name).suffix.lower()
        reader = self.api.point("reader").get(suffix)
        if reader is not None:
            return {"name": name, "text": str(await asyncio.to_thread(reader, data))}
        if suffix and suffix not in ("", ".txt", ".md", ".py", ".json", ".csv", ".tsv",
                                     ".yml", ".yaml", ".html", ".css", ".js", ".sh"):
            return {"error": f"no reader for {suffix}: install one for this file type"}
        if BINARY.search(data[:4096]):
            return {"error": f"no reader for {suffix or 'binary'}: install one"}
        return {"name": name, "text": data.decode("utf-8", "replace")}

    # ------------------------------------------------------------- writing

    async def write(self, who: str, action: Any) -> dict[str, Any]:
        """Write one whole file inside the person's folders."""
        target = paths.resolve(self.api, who, str(action.arguments.get("path") or ""))
        text = str(action.arguments.get("text") or "")
        await asyncio.to_thread(_write, target, text)
        return {"path": paths.display(self.api, who, target), "bytes": len(text.encode())}

    async def edit(self, who: str, action: Any) -> dict[str, Any]:
        """Replace exactly one occurrence of the old text."""
        target = paths.resolve(self.api, who, str(action.arguments.get("path") or ""))
        old, new = str(action.arguments.get("old") or ""), str(action.arguments.get("new") or "")
        text = await asyncio.to_thread(target.read_text, "utf-8")
        found = text.count(old)
        if found != 1:
            return {"error": f"old text is not unique ({found} matches)"}
        await asyncio.to_thread(_write, target, text.replace(old, new, 1))
        return {"path": paths.display(self.api, who, target), "bytes": len(text.encode())}

    # ------------------------------------------------------------ searching

    async def grep(self, who: str, action: Any) -> dict[str, Any]:
        """Search the person's files; deterministic order, binary and big files skipped."""
        pattern = str(action.arguments.get("pattern") or "")
        if not pattern:
            return {"error": "empty pattern"}
        try:
            compiled = re.compile(pattern)
        except re.error as exc:
            return {"error": f"invalid regex: {exc}"}
        raw_path = str(action.arguments.get("path") or "")
        base = paths.resolve(self.api, who, raw_path)
        ws_root = paths.root_for(self.api, who, raw_path)
        mask = str(action.arguments.get("glob") or "*")
        mode = str(action.arguments.get("mode") or "matches")
        limit = max(1, min(500, int(action.arguments.get("limit") or 50)))
        cap = int(self.api.options.get("max_file_bytes") or 2_000_000)
        timeout = float(self.api.options.get("grep_timeout_seconds") or GREP_TIMEOUT_SECONDS)
        try:
            return await asyncio.wait_for(
                asyncio.to_thread(
                    _grep, self.api, who, compiled, base, mask, mode, limit, cap,
                    ws_root=ws_root, deadline=time.monotonic() + timeout,
                ),
                timeout,
            )
        except asyncio.TimeoutError:
            return {"error": f"grep timed out after {timeout:g}s"}

    async def glob(self, who: str, action: Any) -> dict[str, Any]:
        """List the paths matching one pattern, sorted and capped."""
        pattern = str(action.arguments.get("pattern") or "*")
        limit = max(1, min(500, int(action.arguments.get("limit") or 200)))
        found = await asyncio.to_thread(_glob, self.api, who, pattern, limit)
        return {"paths": found, "count": len(found)}

    # -------------------------------------------------------------- running

    async def run(self, who: str, action: Any) -> dict[str, Any]:
        """Screen, then run one command in the sandbox."""
        command = str(action.arguments.get("command") or "")
        verdict, reason = guard.screen(command)
        if verdict == guard.DENY:
            return {"error": f"sandbox command blocked: {reason}"}
        quota = self.api.role_settings(who).get("workspace.quota") or "0"
        used = await asyncio.to_thread(_layer_bytes, self.api, who)
        if _over(used, str(quota)):
            return {"error": f"workspace quota exceeded: {used} of {quota}; /workspace reset"}
        writable = self.api.roles_can(who, "workspace.write")
        folders = paths.roots(self.api, who)
        cwd = str(action.arguments.get("cwd") or "")
        shared = {name: folder for name, folder in folders.items() if name}
        result = await sandbox.run(self.api, who, command, workspace=folders[""],
                                   shared=shared, writable=writable, cwd=cwd,
                                   timeout=action.arguments.get("timeout"))
        if isinstance(result.get("output"), str) and len(result["output"]) > RUN_OUTPUT_BYTES:
            result["output"] = result["output"][-RUN_OUTPUT_BYTES:]
        return result


def _write(target: pathlib.Path, text: str) -> None:
    """Write one file atomically in `target.parent` without following any symlink."""
    target.parent.mkdir(parents=True, exist_ok=True)
    flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW
    for _ in range(8):
        temporary = target.with_name(f".{target.name}.{secrets.token_hex(6)}.tmp")
        try:
            fd = os.open(temporary, flags, 0o600)
            break
        except FileExistsError:
            continue
    else:
        raise OSError("could not create unique temporary file")
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as handle:
            handle.write(text)
        os.replace(temporary, target)
    except BaseException:
        with contextlib.suppress(OSError):
            os.unlink(temporary)
        raise


def _grep(api: Any, who: str, compiled: re.Pattern, base: pathlib.Path, mask: str,
          mode: str, limit: int, cap: int, *, ws_root: pathlib.Path | None = None,
          deadline: float | None = None) -> dict[str, Any]:
    """Walk one tree and collect matches, counts or file names (staying inside workspace)."""
    root = ws_root or base
    files = [base] if base.is_file() else sorted(base.rglob(mask))
    rows: list[str] = []
    counts: dict[str, int] = {}
    for path in files:
        if deadline is not None and time.monotonic() > deadline:
            raise TimeoutError("grep deadline exceeded")
        if mode == "matches" and len(rows) >= limit:
            break
        if not paths.inside(root, path):
            continue
        if not path.is_file() or path.stat().st_size > cap:
            continue
        data = path.read_bytes()
        if BINARY.search(data[:4096]):
            continue
        shown = paths.display(api, who, path)
        for number, line in enumerate(data.decode("utf-8", "replace").splitlines(), 1):
            if deadline is not None and number % 256 == 0 and time.monotonic() > deadline:
                raise TimeoutError("grep deadline exceeded")
            if compiled.search(line):
                counts[shown] = counts.get(shown, 0) + 1
                if mode == "matches":
                    rows.append(f"{shown}:{number}: {line[:400]}")
                if mode == "matches" and len(rows) >= limit:
                    break
    if mode == "files":
        return {"files": sorted(counts), "count": len(counts)}
    if mode == "count":
        return {"counts": dict(sorted(counts.items()))}
    return {"matches": rows[:limit], "count": len(rows)}


def _glob(api: Any, who: str, pattern: str, limit: int) -> list[str]:
    """Return sorted paths matching one pattern inside the person's folders (no symlink escape)."""
    clean = str(pattern or "*").replace("\\", "/")
    if clean.startswith("/") or ".." in pathlib.PurePosixPath(clean).parts:
        raise PermissionError(f"{pattern} is outside your workspace")
    found: set[str] = set()
    folders = paths.roots(api, who)
    for name, base in folders.items():
        if not base.is_dir():
            continue
        for path in base.glob(clean):
            if not paths.inside(base, path):
                continue
            relative = str(path.relative_to(base)).replace("\\", "/")
            found.add(f"{name}/{relative}".strip("/") if name else relative)
    return sorted(found)[:limit]


def _layer_bytes(api: Any, who: str) -> int:
    """Return how many bytes sit in one person's writable layer."""
    folder = sandbox.sandbox_root(api) / "layers" / paths.person_folder(who) / "upper"
    if not folder.is_dir():
        return 0
    return sum(path.stat().st_size for path in folder.rglob("*")
               if not path.is_symlink() and path.is_file())


def _over(used: int, quota: str) -> bool:
    """Return whether a person's layer is over their soft quota."""
    size = _parse_size(quota)
    return size is not None and size > 0 and used > size


def _parse_size(text: str) -> int | None:
    """Parse a quota (`1G`, `512M`, `1024`, `0`) into bytes; None means no quota."""
    found = re.fullmatch(r"\s*(\d+)\s*([KMG]?)\s*", str(text or ""))
    if not found:
        return None
    scale = {"": 1, "K": 1024, "M": 1024 ** 2, "G": 1024 ** 3}[found.group(2)]
    return int(found.group(1)) * scale
