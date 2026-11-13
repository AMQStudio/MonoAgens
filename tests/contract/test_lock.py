"""The lock on `monoagens/contract/` (contract.md §7).

`contract.lock` holds one aggregate hash of the package. Its `# file` lines repeat the
per-file hashes so a failure can name the file that moved; only the aggregate line is
compared. Rewrite the file with `uv run python -m tests.contract.test_lock --write`.
"""

from __future__ import annotations

import hashlib
import pathlib
import sys

CONTRACT_DIR = pathlib.Path(__file__).resolve().parents[2] / "monoagens" / "contract"
LOCK_FILE = CONTRACT_DIR.parents[1] / "contract.lock"
HEADER = ('# sha256 over monoagens/contract/: sorted "<relpath> <sha256 of bytes>" lines, '
          "LF-separated")
ENTRY_PREFIX = "# file "
AGGREGATE = "sha256"
SKIP = {"__pycache__"}


def entries(root: pathlib.Path = CONTRACT_DIR) -> dict[str, str]:
    """Return `{relpath: sha256}` for every file of the contract package."""
    found = {}
    for path in sorted(root.rglob("*")):
        if not path.is_file():
            continue
        parts = path.relative_to(root).parts
        if SKIP.intersection(parts):
            continue
        found[path.relative_to(root).as_posix()] = hashlib.sha256(path.read_bytes()).hexdigest()
    return found


def digest(root: pathlib.Path = CONTRACT_DIR) -> str:
    """Return the one hash of the package, in the form `sha256:<64 hex>`."""
    body = "\n".join(f"{name} {value}" for name, value in sorted(entries(root).items()))
    return f"{AGGREGATE}:{hashlib.sha256(body.encode('utf-8')).hexdigest()}"


def render(root: pathlib.Path = CONTRACT_DIR) -> str:
    """Return the whole lock file's text."""
    lines = [HEADER, digest(root)]
    lines.extend(f"{ENTRY_PREFIX}{name} {value}"
                 for name, value in sorted(entries(root).items()))
    return "\n".join(lines) + "\n"


def read_lock() -> str:
    """Return the aggregate line of the committed lock file."""
    for line in LOCK_FILE.read_text(encoding="utf-8").splitlines():
        if line.startswith(f"{AGGREGATE}:"):
            return line.strip()
    raise AssertionError(f"{LOCK_FILE} holds no {AGGREGATE}: line")


def read_entries() -> dict[str, str]:
    """Return the per-file hashes the lock file records."""
    found = {}
    for line in LOCK_FILE.read_text(encoding="utf-8").splitlines():
        if not line.startswith(ENTRY_PREFIX):
            continue
        name, _, value = line[len(ENTRY_PREFIX):].rpartition(" ")
        found[name] = value.strip()
    return found


def test_contract_lock_matches() -> None:
    """The contract package is frozen: any change needs the owner and a new lock."""
    current = entries()
    recorded = read_entries()
    stale = sorted(name for name, value in current.items()
                   if name in recorded and recorded[name] != value)
    added = sorted(set(current) - set(recorded))
    gone = sorted(set(recorded) - set(current))
    assert digest() == read_lock(), (
        f"contract.lock is stale: changed={stale} added={added} missing={gone}; "
        "only the owner commits a new hash (contract.md §9)")


def test_contract_package_imports_nothing_from_monoagens() -> None:
    """The contract is pure: it imports the standard library only."""
    for path in sorted(CONTRACT_DIR.rglob("*.py")):
        for number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
            text = line.strip()
            if text.startswith(("import monoagens", "from monoagens")):
                raise AssertionError(f"{path}:{number}: the contract imports {text!r}")


def main() -> None:
    """Write `contract.lock` from the package as it stands."""
    if "--write" not in sys.argv:
        print(render())
        return
    LOCK_FILE.write_text(render(), encoding="utf-8")
    print(f"wrote {LOCK_FILE} {digest()}")


if __name__ == "__main__":
    main()
