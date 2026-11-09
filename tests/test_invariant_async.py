"""ADR-0017 §4.1 / ADR-0018 P0: no blocking work inline in an `async def` (AGENTS.md §2).

Async paths must hand synchronous DB queries, embeddings and blocking I/O to the bounded
executor (`await io.run(...)`). This is a static AST check: no database, no network.
"""

import ast
from pathlib import Path
from typing import NamedTuple

ROOT = Path(__file__).resolve().parents[1]
ENGINE = ROOT / "monoagens" / "engine"

# Synchronous SQLAlchemy handles the engine must not touch on the loop.
SYNC_HANDLES = frozenset({"conn", "connection", "con", "engine"})
SUBPROCESS_CALLS = frozenset(
    {
        "subprocess.run",
        "subprocess.call",
        "subprocess.check_call",
        "subprocess.check_output",
        "subprocess.Popen",
    }
)


class BlockingCall(NamedTuple):
    """One blocking call made inline by an `async def`."""

    function: str
    lineno: int
    call: str


def _dotted(node: ast.AST) -> str:
    """The dotted source name of an expression, or "" when it is not a plain name chain."""
    parts: list[str] = []
    while isinstance(node, ast.Attribute):
        parts.append(node.attr)
        node = node.value
    if not isinstance(node, ast.Name):
        return ""
    parts.append(node.id)
    return ".".join(reversed(parts))


def _is_offload(call: ast.Call) -> bool:
    """`io.run(...)`, `asyncio.to_thread(...)`, `loop.run_in_executor(...)` and friends."""
    func = call.func
    if not isinstance(func, ast.Attribute):
        return False
    if func.attr == "run_in_executor":
        return True
    owner = _dotted(func.value)
    if func.attr in {"run", "submit"} and owner == "io":
        return True
    return func.attr == "to_thread" and owner == "asyncio"


def _is_blocking(call: ast.Call) -> bool:
    """A call the event loop must not make inline (ADR-0017 §4.1)."""
    func = call.func
    if isinstance(func, ast.Name):
        return func.id == "open"
    dotted = _dotted(func)
    if dotted == "time.sleep" or dotted == "urllib.request.urlopen":
        return True
    if dotted.startswith("requests.") or dotted in SUBPROCESS_CALLS:
        return True
    return (
        isinstance(func, ast.Attribute)
        and func.attr == "execute"
        and isinstance(func.value, ast.Name)
        and func.value.id in SYNC_HANDLES
    )


class _BodyScan:
    """Walks one `async def` body, except the bodies of callables handed to the executor."""

    def __init__(self) -> None:
        self.hits: list[tuple[int, str]] = []

    def scan(self, body: list[ast.stmt]) -> list[tuple[int, str]]:
        for stmt in body:
            self._walk(stmt, offloaded=False)
        return self.hits

    def _walk(self, node: ast.AST, *, offloaded: bool) -> None:
        # A nested plain def or lambda is a unit of work, not inline code; a nested
        # `async def` is scanned on its own by the module walk.
        if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef, ast.Lambda)):
            return
        if isinstance(node, ast.Call):
            if _is_offload(node):
                for arg in [*node.args, *(kw.value for kw in node.keywords)]:
                    self._walk(arg, offloaded=True)
                return
            if not offloaded and _is_blocking(node):
                self.hits.append((node.lineno, " ".join(ast.unparse(node).split())))
        for child in ast.iter_child_nodes(node):
            self._walk(child, offloaded=offloaded)


def _scan(source: str, path: str) -> list[BlockingCall]:
    """Every blocking call made inline by an `async def` in `source`."""
    found: list[BlockingCall] = []
    for node in ast.walk(ast.parse(source, filename=path)):
        if isinstance(node, ast.AsyncFunctionDef):
            for lineno, call in _BodyScan().scan(node.body):
                found.append(BlockingCall(node.name, lineno, call))
    return found


def _violation_keys() -> set[str]:
    keys: set[str] = set()
    for file in sorted(ENGINE.rglob("*.py")):
        relative = file.relative_to(ROOT).as_posix()
        for hit in _scan(file.read_text(encoding="utf-8"), relative):
            keys.add(f"{relative}::{hit.function}: {hit.call}")
    return keys


# Dated 2026-11-09. This set may only shrink; it is owned by ADR-0018 P0 / ADR-0017 §4.1.
KNOWN_BLOCKING: frozenset[str] = frozenset()


def test_no_new_blocking_calls_in_async() -> None:
    added = sorted(_violation_keys() - KNOWN_BLOCKING)
    assert not added, "Blocking calls inside `async def`; offload through `io.run`:\n" + "\n".join(
        added
    )


def test_known_blocking_entries_still_exist() -> None:
    stale = sorted(KNOWN_BLOCKING - _violation_keys())
    assert not stale, "No longer blocking; delete from KNOWN_BLOCKING:\n" + "\n".join(stale)


def test_scanner_detects_blocking_call() -> None:
    blocked = _scan("import time\n\nasync def f():\n    time.sleep(1)\n", "inline.py")
    assert blocked == [BlockingCall("f", 4, "time.sleep(1)")]

    offloaded = _scan(
        "from monoagens.engine import io\nimport time\n\n"
        "async def g():\n    await io.run(lambda: time.sleep(1))\n",
        "inline.py",
    )
    assert offloaded == []
